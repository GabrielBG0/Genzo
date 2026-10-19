//! Stages 1 to 3 on the GPU, tile by tile (DEV-01, Tiling): FixBadPixels,
//! levels and GainMaps, white balance and the highlight clip, then the RCD
//! demosaic for a Bayer image. Linear input skips the demosaic.
//!
//! Each tile computes its output rect plus an apron on each side and keeps
//! only the output. The apron is wider than RCD's support (10 px) and the
//! bad-pixel fix's (2 px), so a tiled render equals a render of the whole
//! image as one tile, bit for bit: the tests hold it to that.
//!
//! Every resource is allocated once, at the largest padded tile size, and
//! reused. The output tiles are read back here; plan step 5b2 feeds them to
//! stage 4 and the pyramid instead.

use std::collections::VecDeque;
use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use genzo_model::frame::Rect;

use super::{Gpu, GpuError};
use crate::camera_color::CameraTransform;
use crate::raw::RawSource;
use crate::reference::stages::{DESATURATE_FROM, half, luminance_row};
use crate::tiles::{TileJob, jobs};

const WG: u32 = 16;

/// Tile size and apron (DEV-01: 1024 and 24 px for Bayer and linear input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileConfig {
    pub tile: u32,
    pub apron: u32,
}

impl Default for TileConfig {
    fn default() -> Self {
        Self {
            tile: 1024,
            apron: 24,
        }
    }
}

/// The raw rect reaches this far past the padded rect: FixBadPixelsConstant
/// reads neighbors 2 px away.
const RAW_MARGIN: u32 = 2;

/// Where a render stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// After stages 1 and 2: the mosaic for a Bayer image (one value per
    /// pixel), camera RGB for linear input.
    Stage2,
    /// After the demosaic: camera RGB.
    Stage3,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileUniforms {
    size: [u32; 4],
    origin: [u32; 4],
    raw: [u32; 4],
    info: [u32; 4],
    levels: [u32; 4],
    bayer: [u32; 4],
    wb: [f32; 4],
    scale: [f32; 4],
    deltas: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Stage4Uniforms {
    m: [[f32; 4]; 3],
    luma: [f32; 4],
    params: [f32; 4],
    texels: [u32; 4],
    level: [u32; 4],
}

/// Tiles in flight at once while building the pyramid: each holds its
/// upload staging until the GPU is done with it (6.9 MB for a linear DNG
/// tile; DEV-01's upload line counts two).
const IN_FLIGHT: usize = 2;

/// How many pyramid levels a first level of `size` gets: down to the first
/// whose long edge is 256 px or less.
pub fn level_count(size: (u32, u32)) -> u32 {
    let mut n = 1;
    let mut s = size;
    while s.0.max(s.1) > 256 {
        s = half(s);
        n += 1;
    }
    n
}

/// The bytes of a pyramid whose first level is `size` (RGBA16F).
pub fn pyramid_bytes(size: (u32, u32)) -> u64 {
    let mut s = size;
    let mut total = 0;
    for _ in 0..level_count(size) {
        total += u64::from(s.0) * u64::from(s.1) * 8;
        s = half(s);
    }
    total
}

/// The image-wide mip pyramid (DEV-01, Tiling): level 1 (half size) and up,
/// RGBA16F, in the working space. Level 0 is never kept.
#[derive(Debug)]
pub struct Pyramid {
    texture: wgpu::Texture,
    /// Level 1's width and height.
    pub size: (u32, u32),
    /// Levels 1 to `levels`.
    pub levels: u32,
    /// What stages 2 and 4 used. A white balance drag applies its change on
    /// these values as the matrix `M_new · diag(r) · M_old⁻¹` (DEV-01).
    pub transform: CameraTransform,
}

impl Pyramid {
    /// The size of `level` (1 is half size).
    pub fn level_size(&self, level: u32) -> (u32, u32) {
        let mut s = self.size;
        for _ in 1..level {
            s = half(s);
        }
        s
    }

    pub fn bytes(&self) -> u64 {
        pyramid_bytes(self.size)
    }

    /// Reads `level` back as RGB, row by row: for tests, and for keeping a
    /// prefetched photo's pyramid in CPU memory.
    pub fn read_level(&self, gpu: &Gpu, level: u32) -> Result<Vec<[f32; 3]>, GpuError> {
        assert!((1..=self.levels).contains(&level), "level {level}");
        let (w, h) = self.level_size(level);
        let row = padded_row(w, 8);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pyramid readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: level - 1,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            let line: &[u16] = bytemuck::cast_slice(&mapped[start..start + w as usize * 8]);
            for px in line.as_chunks::<4>().0 {
                out.push([0, 1, 2].map(|c| ::half::f16::from_bits(px[c]).to_f32()));
            }
        }
        Ok(out)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapUniform {
    area: [u32; 4],
    pitch: [u32; 4],
    points: [u32; 4],
    geo: [f32; 4],
}

#[derive(Clone, Copy)]
enum Slot {
    Uniform,
    Float,
    Uint,
    Storage,
    Store(wgpu::TextureFormat),
}

#[derive(Debug)]
struct Pass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

fn pass(device: &wgpu::Device, name: &str, body: &str, slots: &[Slot]) -> Pass {
    pass_with(device, name, body, slots, true)
}

/// A pass from `body`, after the tile block (`tile_common.wgsl`) when
/// `common`; binding 0 is a uniform buffer either way.
fn pass_with(device: &wgpu::Device, name: &str, body: &str, slots: &[Slot], common: bool) -> Pass {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    for (i, slot) in slots.iter().enumerate() {
        let ty = match *slot {
            Slot::Float => wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            Slot::Uniform => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            Slot::Uint => wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            Slot::Storage => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            Slot::Store(format) => wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: i as u32 + 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        });
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(name),
        entries: &entries,
    });
    let source = if common {
        format!("{}\n{}", include_str!("shaders/tile_common.wgsl"), body)
    } else {
        body.to_owned()
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(name),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    Pass { layout, pipeline }
}

#[derive(Debug)]
struct Passes {
    stage1: Pass,
    vh: Pass,
    lpf_pq: Pass,
    pq_dir: Pass,
    green: Pass,
    rb: Pass,
    rcd_final: Pass,
    stage4: Pass,
    downsample: Pass,
}

impl Passes {
    fn new(device: &wgpu::Device) -> Self {
        use Slot::{Float, Storage, Store, Uint, Uniform};
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba16Float, Rgba32Float};
        Self {
            stage1: pass(
                device,
                "tile stage 1",
                include_str!("shaders/tile_stage1.wgsl"),
                &[
                    Uint,
                    Storage,
                    Storage,
                    Storage,
                    Store(R32Float),
                    Store(Rgba32Float),
                ],
            ),
            vh: pass(
                device,
                "tile rcd vh",
                include_str!("shaders/tile_rcd_vh.wgsl"),
                &[Float, Store(R32Float)],
            ),
            lpf_pq: pass(
                device,
                "tile rcd lpf pq",
                include_str!("shaders/tile_rcd_lpf_pq.wgsl"),
                &[Float, Store(R32Float), Store(Rg32Float)],
            ),
            pq_dir: pass(
                device,
                "tile rcd pq dir",
                include_str!("shaders/tile_rcd_pq_dir.wgsl"),
                &[Float, Store(R32Float)],
            ),
            green: pass(
                device,
                "tile rcd green",
                include_str!("shaders/tile_rcd_green.wgsl"),
                &[Float, Float, Float, Store(R32Float)],
            ),
            rb: pass(
                device,
                "tile rcd rb",
                include_str!("shaders/tile_rcd_rb.wgsl"),
                &[Float, Float, Float, Store(Rg32Float)],
            ),
            rcd_final: pass(
                device,
                "tile rcd final",
                include_str!("shaders/tile_rcd_final.wgsl"),
                &[Float, Float, Float, Float, Store(Rgba32Float)],
            ),
            stage4: pass(
                device,
                "tile stage 4",
                include_str!("shaders/tile_stage4.wgsl"),
                &[Uniform, Float, Store(Rgba16Float)],
            ),
            downsample: pass_with(
                device,
                "pyramid downsample",
                include_str!("shaders/tile_downsample.wgsl"),
                &[Float, Store(Rgba16Float)],
                false,
            ),
        }
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn storage(device: &wgpu::Device, label: &str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        // wgpu refuses empty bindings, and a binding must hold at least one
        // element (a GainMap is 64 bytes).
        size: bytes.max(64).next_multiple_of(4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The per-tile textures: allocated once at the largest padded size.
#[derive(Debug)]
struct Tiles {
    size: (u32, u32),
    /// The raw rect: one 16-bit sample per texel, `samples` texels per pixel.
    raw: wgpu::Texture,
    cfa: wgpu::Texture,
    vh: wgpu::Texture,
    lpf: wgpu::Texture,
    pq: wgpu::Texture,
    pq_dir: wgpu::Texture,
    green: wgpu::Texture,
    rb: wgpu::Texture,
    rgb: wgpu::Texture,
}

impl Tiles {
    fn new(device: &wgpu::Device, size: (u32, u32), samples: u32) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba32Float};
        let (w, h) = (size.0 + 2 * RAW_MARGIN, size.1 + 2 * RAW_MARGIN);
        Self {
            size,
            raw: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("tile raw"),
                size: wgpu::Extent3d {
                    width: w * samples,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            cfa: texture(device, "tile cfa", size, R32Float),
            vh: texture(device, "tile vh", size, R32Float),
            lpf: texture(device, "tile lpf", size, R32Float),
            pq: texture(device, "tile pq", size, Rg32Float),
            pq_dir: texture(device, "tile pq dir", size, R32Float),
            green: texture(device, "tile green", size, R32Float),
            rb: texture(device, "tile rb", size, Rg32Float),
            rgb: texture(device, "tile rgb", size, Rgba32Float),
        }
    }
}

fn padded_row(width: u32, bytes_per_pixel: u32) -> u32 {
    (width * bytes_per_pixel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&Default::default())
}

/// Stages 1 to 3 on one device. Only the render thread holds it.
#[derive(Debug)]
pub struct RawStages {
    config: TileConfig,
    passes: Passes,
    uniforms: wgpu::Buffer,
    stage4_uniforms: wgpu::Buffer,
    tiles: Option<Tiles>,
    /// For [`RawStages::render`] only, made on first use.
    readback: Option<wgpu::Buffer>,
    pyramid: Option<Pyramid>,
    /// For the memory test: sample the allocator report after each tile's
    /// submit in [`RawStages::render_pyramid`], keeping the largest total.
    #[doc(hidden)]
    pub sample_memory: bool,
    #[doc(hidden)]
    pub peak_allocated: Option<u64>,
}

/// The image-wide inputs of a render, uploaded once.
struct ImageBuffers {
    black: wgpu::Buffer,
    maps: wgpu::Buffer,
    gains: wgpu::Buffer,
    deltas: [u32; 4],
    map_count: u32,
}

impl RawStages {
    pub fn new(gpu: &Gpu, config: TileConfig) -> Self {
        Self {
            config,
            passes: Passes::new(&gpu.device),
            uniforms: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tile uniforms"),
                size: std::mem::size_of::<TileUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            stage4_uniforms: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stage 4 uniforms"),
                size: std::mem::size_of::<Stage4Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            tiles: None,
            readback: None,
            pyramid: None,
            sample_memory: false,
            peak_allocated: None,
        }
    }

    /// The bytes of one tile's working textures (DEV-01's "stage 1 to 4
    /// scratch"). Zero before the first render.
    pub fn scratch_bytes(&self) -> u64 {
        self.tiles.as_ref().map_or(0, |t| {
            let px = u64::from(t.size.0) * u64::from(t.size.1);
            // cfa, vh, lpf, pq_dir, green: 4 B; pq, rb: 8 B; rgb: 16 B.
            px * (5 * 4 + 2 * 8 + 16)
        })
    }

    /// The bytes of the raw tile the upload lands in (DEV-01's "upload
    /// staging for one tile"), sized for three samples per pixel.
    pub fn upload_bytes(&self) -> u64 {
        self.tiles.as_ref().map_or(0, |t| {
            u64::from(t.raw.width()) * u64::from(t.raw.height()) * 2
        })
    }

    /// The pyramid of the last [`RawStages::render_pyramid`].
    pub fn pyramid(&self) -> Option<&Pyramid> {
        self.pyramid.as_ref()
    }

    /// The tiles of `src`, with the per-tile resources allocated to fit.
    fn prepare(&mut self, gpu: &Gpu, src: &RawSource) -> Vec<TileJob> {
        let (w, h) = src.size();
        let margin = if src.fix_bad_pixels.is_empty() {
            0
        } else {
            RAW_MARGIN
        };
        let jobs = jobs(
            (w, h),
            (src.active.left, src.active.top),
            src.stored,
            self.config.tile,
            self.config.apron,
            margin,
        );
        let largest = jobs.iter().fold((0, 0), |(a, b), j| {
            (a.max(j.padded.width), b.max(j.padded.height))
        });
        let fits = self.tiles.as_ref().is_some_and(|t| {
            t.size.0 >= largest.0
                && t.size.1 >= largest.1
                && t.raw.width() >= (largest.0 + 2 * RAW_MARGIN) * src.samples
                && t.raw.height() >= largest.1 + 2 * RAW_MARGIN
        });
        if !fits {
            self.tiles = None;
            self.readback = None;
            let full = self.config.tile + 2 * self.config.apron;
            // Allocated at the configured size, so every later image fits;
            // smaller only when this image is.
            let size = (full.min(w).max(largest.0), full.min(h).max(largest.1));
            self.tiles = Some(Tiles::new(&gpu.device, size, 3));
        }
        jobs
    }

    /// Renders `src` through stages 1 to `until` with the white balance
    /// `multipliers` (stage 2; the smallest should be 1, it is the clip
    /// level). Returns the active area row by row: one value per pixel for a
    /// Bayer mosaic at [`Until::Stage2`], else RGB.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        multipliers: [f32; 3],
        until: Until,
    ) -> Result<Vec<f32>, GpuError> {
        let (w, h) = src.size();
        let jobs = self.prepare(gpu, src);
        if self.readback.is_none() {
            let size = self.tiles.as_ref().expect("prepared").size;
            self.readback = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tile readback"),
                size: u64::from(padded_row(size.0, 16)) * u64::from(size.1),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let image = self.upload_image(gpu, src);
        let mosaic = src.bayer.is_some() && until == Until::Stage2;
        let channels = if mosaic { 1 } else { 3 };
        let mut out = vec![0.0f32; w as usize * h as usize * channels];
        for job in &jobs {
            self.render_tile(
                gpu,
                src,
                &image,
                job,
                multipliers,
                until,
                channels,
                &mut out,
                w,
            )?;
        }
        Ok(out)
    }

    fn upload_image(&self, gpu: &Gpu, src: &RawSource) -> ImageBuffers {
        let lin = &src.linearization;
        let mut black = lin.pattern.clone();
        let mut deltas = [0u32; 4];
        if !lin.delta_h.is_empty() {
            deltas[0] = black.len() as u32;
            black.extend(lin.delta_h.iter().flatten());
        }
        if !lin.delta_v.is_empty() {
            deltas[1] = black.len() as u32;
            black.extend(lin.delta_v.iter().flatten());
        }
        let (w, h) = src.size();
        let mut maps = Vec::new();
        let mut gains: Vec<f32> = Vec::new();
        for gm in &src.gain_maps {
            // The area intersected with the image, from its own top left
            // (reference::opcodes::overlap).
            let a = &gm.area;
            let (t, l, b, r) = if a.top >= a.bottom || a.left >= a.right {
                (0, 0, h, w)
            } else {
                (a.top, a.left, a.bottom.min(h), a.right.min(w))
            };
            if t >= b || l >= r || gm.map_planes == 0 {
                continue;
            }
            let end = a.plane.saturating_add(a.planes).min(src.samples);
            maps.push(MapUniform {
                area: [t, l, b, r],
                pitch: [a.plane, end, a.row_pitch.max(1), a.col_pitch.max(1)],
                points: [gm.points_v, gm.points_h, gm.map_planes, gains.len() as u32],
                geo: [
                    gm.spacing_v as f32,
                    gm.spacing_h as f32,
                    gm.origin_v as f32,
                    gm.origin_h as f32,
                ],
            });
            gains.extend_from_slice(&gm.gains);
        }
        let device = &gpu.device;
        let buffer = |label, bytes: &[u8]| {
            let b = storage(device, label, bytes.len() as u64);
            gpu.queue.write_buffer(&b, 0, bytes);
            b
        };
        ImageBuffers {
            black: buffer("black levels", bytemuck::cast_slice(&black)),
            maps: buffer("gain maps", bytemuck::cast_slice(&maps)),
            gains: buffer("gains", bytemuck::cast_slice(&gains)),
            deltas,
            map_count: maps.len() as u32,
        }
    }

    /// Uploads one tile's uniforms and raw rect and encodes stages 1 and 2,
    /// and the demosaic when `demosaic`.
    fn encode_tile(
        &self,
        gpu: &Gpu,
        src: &RawSource,
        image: &ImageBuffers,
        job: &TileJob,
        multipliers: [f32; 3],
        demosaic: bool,
    ) -> wgpu::CommandEncoder {
        let t = self.tiles.as_ref().expect("prepared");
        let device = &gpu.device;
        let (w, h) = src.size();
        let lin = &src.linearization;
        let p = job.padded;
        let fbp = src.fix_bad_pixels.first();
        let scale = |i: usize| {
            lin.scale
                .get(i)
                .or(lin.scale.first())
                .copied()
                .unwrap_or(1.0)
        };
        let clip = multipliers[0].min(multipliers[1]).min(multipliers[2]);
        let uniforms = TileUniforms {
            size: [p.width, p.height, w, h],
            origin: [p.x, p.y, src.active.left, src.active.top],
            raw: [job.raw.x, job.raw.y, job.raw.width, job.raw.height],
            info: [
                src.samples,
                image.map_count,
                u32::from(fbp.is_some()),
                fbp.map_or(0, |f| f.bayer_phase),
            ],
            levels: [
                fbp.map_or(0, |f| f.constant),
                lin.rows,
                lin.cols,
                u32::from(!lin.delta_h.is_empty()) | u32::from(!lin.delta_v.is_empty()) << 1,
            ],
            bayer: src.bayer.map_or([255; 4], |b| b.map(u32::from)),
            wb: [multipliers[0], multipliers[1], multipliers[2], clip],
            scale: [scale(0), scale(1), scale(2), 0.0],
            deltas: image.deltas,
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        // The raw rect straight from the stored image, at its row stride.
        let n = src.samples as usize;
        let stride = src.stored.0 as usize * n;
        let start = job.raw.y as usize * stride + job.raw.x as usize * n;
        let end = start + (job.raw.height as usize - 1) * stride + job.raw.width as usize * n;
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.raw,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&src.data[start..end]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride as u32 * 2),
                rows_per_image: Some(job.raw.height),
            },
            wgpu::Extent3d {
                width: job.raw.width * src.samples,
                height: job.raw.height,
                depth_or_array_layers: 1,
            },
        );

        let u = self.uniforms.as_entire_binding();
        let bind = |pass: &Pass, resources: Vec<wgpu::BindingResource>| {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: u.clone(),
            }];
            for (i, r) in resources.into_iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: r,
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pass.layout,
                entries: &entries,
            })
        };
        let views = TileViews::new(t);
        let ps = &self.passes;
        let mut groups = vec![(
            &ps.stage1,
            bind(
                &ps.stage1,
                vec![
                    views.r(&views.raw),
                    image.black.as_entire_binding(),
                    image.maps.as_entire_binding(),
                    image.gains.as_entire_binding(),
                    views.r(&views.cfa),
                    views.r(&views.rgb),
                ],
            ),
        )];
        if demosaic && src.bayer.is_some() {
            let v = &views;
            groups.push((&ps.vh, bind(&ps.vh, vec![v.r(&v.cfa), v.r(&v.vh)])));
            groups.push((
                &ps.lpf_pq,
                bind(&ps.lpf_pq, vec![v.r(&v.cfa), v.r(&v.lpf), v.r(&v.pq)]),
            ));
            groups.push((
                &ps.pq_dir,
                bind(&ps.pq_dir, vec![v.r(&v.pq), v.r(&v.pq_dir)]),
            ));
            groups.push((
                &ps.green,
                bind(
                    &ps.green,
                    vec![v.r(&v.cfa), v.r(&v.lpf), v.r(&v.vh), v.r(&v.green)],
                ),
            ));
            groups.push((
                &ps.rb,
                bind(
                    &ps.rb,
                    vec![v.r(&v.cfa), v.r(&v.green), v.r(&v.pq_dir), v.r(&v.rb)],
                ),
            ));
            groups.push((
                &ps.rcd_final,
                bind(
                    &ps.rcd_final,
                    vec![
                        v.r(&v.cfa),
                        v.r(&v.green),
                        v.r(&v.rb),
                        v.r(&v.vh),
                        v.r(&v.rgb),
                    ],
                ),
            ));
        }

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("raw stages"),
                timestamp_writes: None,
            });
            for (pass, group) in &groups {
                cpass.set_pipeline(&pass.pipeline);
                cpass.set_bind_group(0, group, &[]);
                cpass.dispatch_workgroups(p.width.div_ceil(WG), p.height.div_ceil(WG), 1);
            }
        }
        encoder
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tile(
        &self,
        gpu: &Gpu,
        src: &RawSource,
        image: &ImageBuffers,
        job: &TileJob,
        multipliers: [f32; 3],
        until: Until,
        channels: usize,
        out: &mut [f32],
        width: u32,
    ) -> Result<(), GpuError> {
        let t = self.tiles.as_ref().expect("prepared");
        let readback = self.readback.as_ref().expect("made in render");
        let device = &gpu.device;
        let p = job.padded;
        let mut encoder =
            self.encode_tile(gpu, src, image, job, multipliers, until == Until::Stage3);
        // The output rect within the padded one.
        let keep = Rect {
            x: job.out.x - p.x,
            y: job.out.y - p.y,
            width: job.out.width,
            height: job.out.height,
        };
        let mosaic = channels == 1;
        let (source, bpp) = if mosaic { (&t.cfa, 4) } else { (&t.rgb, 16) };
        let row = padded_row(keep.width, bpp);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: source,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: keep.x,
                    y: keep.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(keep.height),
                },
            },
            wgpu::Extent3d {
                width: keep.width,
                height: keep.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        {
            let mapped = readback
                .get_mapped_range(..)
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            for y in 0..keep.height as usize {
                let line: &[f32] = bytemuck::cast_slice(
                    &mapped
                        [y * row as usize..y * row as usize + keep.width as usize * bpp as usize],
                );
                let dst_row = job.out.y as usize + y;
                for x in 0..keep.width as usize {
                    let dst = (dst_row * width as usize + job.out.x as usize + x) * channels;
                    if mosaic {
                        out[dst] = line[x];
                    } else {
                        out[dst..dst + 3].copy_from_slice(&line[x * 4..x * 4 + 3]);
                    }
                }
            }
        }
        readback.unmap();
        Ok(())
    }
}

/// Views of the tile textures, for one tile's bind groups.
struct TileViews {
    raw: wgpu::TextureView,
    cfa: wgpu::TextureView,
    vh: wgpu::TextureView,
    lpf: wgpu::TextureView,
    pq: wgpu::TextureView,
    pq_dir: wgpu::TextureView,
    green: wgpu::TextureView,
    rb: wgpu::TextureView,
    rgb: wgpu::TextureView,
}

impl TileViews {
    fn new(t: &Tiles) -> Self {
        Self {
            raw: view(&t.raw),
            cfa: view(&t.cfa),
            vh: view(&t.vh),
            lpf: view(&t.lpf),
            pq: view(&t.pq),
            pq_dir: view(&t.pq_dir),
            green: view(&t.green),
            rb: view(&t.rb),
            rgb: view(&t.rgb),
        }
    }

    fn r<'a>(&self, v: &'a wgpu::TextureView) -> wgpu::BindingResource<'a> {
        wgpu::BindingResource::TextureView(v)
    }
}

impl RawStages {
    /// Builds the pyramid of `src` with `transform`: stages 1 to 4 tile by
    /// tile, reduced into level 1, then the levels above. The previous
    /// pyramid is freed first, so two are never in GPU memory at once.
    pub fn render_pyramid(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        transform: &CameraTransform,
    ) -> Result<&Pyramid, GpuError> {
        if self.pyramid.take().is_some() {
            // wgpu frees a dropped texture once no submitted work uses it.
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let jobs = self.prepare(gpu, src);
        let l1 = half(src.size());
        let levels = level_count(l1);
        let device = &gpu.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pyramid"),
            size: wgpu::Extent3d {
                width: l1.0,
                height: l1.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mip = |level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let image = self.upload_image(gpu, src);
        let m = transform.matrix;
        let mm = transform.multipliers;
        let clip = mm[0].min(mm[1]).min(mm[2]);
        let luma = luminance_row();
        let level1 = mip(0);
        let rgb = view(&self.tiles.as_ref().expect("prepared").rgb);
        let mut in_flight = VecDeque::new();
        for job in &jobs {
            // This tile's upload is staged as soon as it is encoded, so wait
            // for room first.
            if in_flight.len() >= IN_FLIGHT {
                let oldest = in_flight.pop_front();
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: oldest,
                        timeout: None,
                    })
                    .map_err(|e| GpuError::Request(e.to_string()))?;
            }
            let mut encoder = self.encode_tile(gpu, src, &image, job, mm, true);
            // The level 1 texels whose 2 x 2 block starts in this tile; tiles
            // start on even pixels, and an odd last pixel folds into the
            // block before it, in the same tile (tiles::jobs).
            let first = (job.out.x / 2, job.out.y / 2);
            let end = (
                (job.out.x + job.out.width).div_ceil(2).min(l1.0),
                (job.out.y + job.out.height).div_ceil(2).min(l1.1),
            );
            let count = (end.0 - first.0, end.1 - first.1);
            let u = Stage4Uniforms {
                m: m.map(|r| [r[0], r[1], r[2], 0.0]),
                luma: [luma[0], luma[1], luma[2], 0.0],
                params: [transform.exposure, clip, DESATURATE_FROM, 0.0],
                texels: [first.0, first.1, count.0, count.1],
                level: [l1.0, l1.1, 0, 0],
            };
            gpu.queue
                .write_buffer(&self.stage4_uniforms, 0, bytemuck::bytes_of(&u));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stage 4"),
                layout: &self.passes.stage4.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.stage4_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&rgb),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&level1),
                    },
                ],
            });
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stage 4"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&self.passes.stage4.pipeline);
                cpass.set_bind_group(0, &group, &[]);
                cpass.dispatch_workgroups(count.0.div_ceil(WG), count.1.div_ceil(WG), 1);
            }
            in_flight.push_back(gpu.queue.submit([encoder.finish()]));
            if self.sample_memory {
                // The uploads still staged count here: their submissions
                // have not finished.
                if let Some(r) = device.generate_allocator_report() {
                    let peak = self.peak_allocated.get_or_insert(0);
                    *peak = (*peak).max(r.total_allocated_bytes);
                }
            }
        }
        // Levels 2 and up.
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut sizes = l1;
        for level in 1..levels {
            let dst = half(sizes);
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("level sizes"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.queue.write_buffer(
                &uniform,
                0,
                bytemuck::cast_slice(&[sizes.0, sizes.1, dst.0, dst.1]),
            );
            let (src_view, dst_view) = (mip(level - 1), mip(level));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("downsample"),
                layout: &self.passes.downsample.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&dst_view),
                    },
                ],
            });
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("downsample"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.passes.downsample.pipeline);
            cpass.set_bind_group(0, &group, &[]);
            cpass.dispatch_workgroups(dst.0.div_ceil(WG), dst.1.div_ceil(WG), 1);
            drop(cpass);
            sizes = dst;
        }
        gpu.queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Request(e.to_string()))?;
        Ok(self.pyramid.insert(Pyramid {
            texture,
            size: l1,
            levels,
            transform: transform.clone(),
        }))
    }
}
