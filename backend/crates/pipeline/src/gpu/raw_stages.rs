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
use genzo_color::encoding::InputEncoding;
use genzo_model::frame::Rect;

use super::{Gpu, GpuError};
use crate::camera_color::CameraTransform;
use crate::raw::{Cfa, RawSource};
use crate::reference::sampling::Sampling;
use crate::reference::stages::{DESATURATE_FROM, half, luminance_row, span};
use crate::tiles::{TileJob, jobs_in};

const WG: u32 = 16;

/// Tile sizes and apron (DEV-01: 1024 px for Bayer and linear input, 512
/// for X-Trans, whose demosaic keeps 8 directional images; 24 px of apron).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileConfig {
    pub tile: u32,
    pub xtrans_tile: u32,
    pub apron: u32,
}

impl Default for TileConfig {
    fn default() -> Self {
        Self {
            tile: 1024,
            xtrans_tile: 512,
            apron: 24,
        }
    }
}

/// Per pixel of a 3 x 3 cell, its 8 hexagon offsets (x, y).
pub type Hexagons = [[[(i32, i32); 8]; 3]; 3];

/// The green hexagon around each pixel of a 3 x 3 cell of an X-Trans
/// pattern (from its origin) as (x, y) offsets, and the solitary green's
/// (x, y) in the cell: darktable's `allhex` and `sgrow`/`sgcol`.
pub fn xtrans_hexagons(x: &[[u8; 6]; 6]) -> (Hexagons, (i32, i32)) {
    const ORTH: [i32; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];
    const PATT: [[i32; 16]; 2] = [
        [0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0],
        [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1],
    ];
    let fc = |r: i32, c: i32| x[r.rem_euclid(6) as usize][c.rem_euclid(6) as usize];
    let mut hex = [[[(0, 0); 8]; 3]; 3];
    let mut sgreen = (0, 0);
    for row in 0..3 {
        for col in 0..3 {
            let mut ng = 0;
            for d in (0..10).step_by(2) {
                let g = usize::from(fc(row, col) == 1);
                if fc(row + ORTH[d], col + ORTH[d + 2]) == 1 {
                    ng = 0;
                } else {
                    ng += 1;
                }
                if ng == 4 {
                    sgreen = (col, row);
                }
                if ng == g + 1 {
                    for c in 0..8 {
                        let v = ORTH[d] * PATT[g][c * 2] + ORTH[d + 1] * PATT[g][c * 2 + 1];
                        let h = ORTH[d + 2] * PATT[g][c * 2] + ORTH[d + 3] * PATT[g][c * 2 + 1];
                        hex[row as usize][col as usize][c ^ ((g * 2) & d)] = (h, v);
                    }
                }
            }
        }
    }
    (hex, sgreen)
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
    /// 0: no mosaic (linear), 1: Bayer, 2: X-Trans; then 1 for a non-RAW
    /// image's curves in `black`; the rest unused. Last, as in
    /// tile_common.wgsl.
    cfa: [u32; 4],
    xtrans: [[u32; 4]; 9],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct XImageUniforms {
    hex: [[i32; 4]; 36],
    sgreen: [i32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct XRunUniforms {
    p: [i32; 4],
    q: [u32; 4],
}

/// The entry points of tile_xtrans.wgsl, in pass order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XStep {
    Copy,
    CopySet,
    MinMax,
    Green,
    Recalc,
    Solitary,
    RedBlue,
    TwoByTwo,
    Derivatives,
    Homogeneity,
    HomoSum,
    Final,
}

const XSTEPS: [(XStep, &str); 12] = [
    (XStep::Copy, "copy"),
    (XStep::CopySet, "copy_set"),
    (XStep::MinMax, "minmax"),
    (XStep::Green, "green"),
    (XStep::Recalc, "recalc"),
    (XStep::Solitary, "solitary"),
    (XStep::RedBlue, "red_blue"),
    (XStep::TwoByTwo, "two_by_two"),
    (XStep::Derivatives, "derivatives"),
    (XStep::Homogeneity, "homogeneity"),
    (XStep::HomoSum, "homo_sum"),
    (XStep::Final, "final_rgb"),
];

/// darktable's host loop for Markesteijn, 3 passes (process_markesteijn_cl):
/// each step and its run parameters.
fn xtrans_sequence() -> (Vec<(XStep, usize)>, Vec<XRunUniforms>) {
    let mut runs = vec![XRunUniforms::default()];
    let mut seq = vec![(XStep::Copy, 0), (XStep::MinMax, 0), (XStep::Green, 0)];
    let mut push = |seq: &mut Vec<(XStep, usize)>, step, run: XRunUniforms| {
        runs.push(run);
        seq.push((step, runs.len() - 1));
    };
    for pass in 0..3u32 {
        let base = if pass == 0 { 0 } else { 4 };
        if pass == 1 {
            seq.push((XStep::CopySet, 0));
        }
        if pass > 0 {
            push(
                &mut seq,
                XStep::Recalc,
                XRunUniforms {
                    p: [0; 4],
                    q: [0, base, 0, 0],
                },
            );
        }
        // The solitary green runs write images 0, 1, -, 2, -, 3 of the set
        // and read the one they write next.
        let images = [0, 1, 2, 2, 3, 3];
        for d in 0..6i32 {
            let i = if d % 2 == 0 { 1 } else { 0 };
            let hcomp = if d % 2 == 0 { 0 } else { 2 };
            push(
                &mut seq,
                XStep::Solitary,
                XRunUniforms {
                    p: [d, i, i ^ 1, hcomp],
                    q: [base + images[d as usize], base, 0, 0],
                },
            );
        }
        for d in 0..4i32 {
            push(
                &mut seq,
                XStep::RedBlue,
                XRunUniforms {
                    p: [d, 0, 0, 0],
                    q: [base + d as u32, base, 0, 0],
                },
            );
        }
        for d in (0..8i32).step_by(2) {
            push(
                &mut seq,
                XStep::TwoByTwo,
                XRunUniforms {
                    p: [d, 0, 0, 0],
                    q: [base + d as u32 / 2, base, 0, 0],
                },
            );
        }
    }
    for step in [
        XStep::Derivatives,
        XStep::Homogeneity,
        XStep::HomoSum,
        XStep::Final,
    ] {
        seq.push((step, 0));
    }
    (seq, runs)
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

/// The most bytes of level 1 [`RawStages::upload_pyramid`] stages at once:
/// with the raw tile texture beside it, within DEV-01's upload line.
const UPLOAD_BAND: usize = 8 << 20;

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
    /// A non-RAW image's encoding; None for RAW. Its white balance is a
    /// relative shift in the working space (DEV-01), not a new camera white.
    pub encoding: Option<InputEncoding>,
    /// Which render or rebuild wrote it last: a [`Rebuild`] resumes only
    /// the pyramid of its own `rebuild_first`.
    id: u64,
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

    /// The sizes of levels 1 to `levels`.
    pub fn level_sizes(&self) -> Vec<(u32, u32)> {
        (1..=self.levels).map(|l| self.level_size(l)).collect()
    }

    /// A view of one level, for sampling it.
    pub(crate) fn level_view(&self, level: u32) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: level - 1,
            mip_level_count: Some(1),
            ..Default::default()
        })
    }

    /// Reads `level` back as RGB, row by row: for tests.
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
pub(crate) enum Slot {
    Uniform,
    StorageRw,
    Float,
    Uint,
    Storage,
    Store(wgpu::TextureFormat),
}

#[derive(Debug)]
pub(crate) struct Pass {
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) pipeline: wgpu::ComputePipeline,
}

fn pass(device: &wgpu::Device, name: &str, body: &str, slots: &[Slot]) -> Pass {
    pass_with(device, name, body, slots, true)
}

/// A pass from `body`, after the tile block (`tile_common.wgsl`) when
/// `common`; binding 0 is a uniform buffer either way.
pub(crate) fn pass_with(
    device: &wgpu::Device,
    name: &str,
    body: &str,
    slots: &[Slot],
    common: bool,
) -> Pass {
    let layout = layout(device, name, slots);
    let source = if common {
        format!("{}\n{}", include_str!("shaders/tile_common.wgsl"), body)
    } else {
        body.to_owned()
    };
    let pipeline = pipeline(device, name, &layout, &source, "main");
    Pass { layout, pipeline }
}

fn pipeline(
    device: &wgpu::Device,
    name: &str,
    layout: &wgpu::BindGroupLayout,
    source: &str,
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(name),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// A bind group layout: binding 0 a uniform buffer, then `slots`.
fn layout(device: &wgpu::Device, name: &str, slots: &[Slot]) -> wgpu::BindGroupLayout {
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
            Slot::StorageRw => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
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
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(name),
        entries: &entries,
    })
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
    stage4_full: Pass,
    downsample: Pass,
    /// Markesteijn: one layout, one pipeline per entry point (XSTEPS).
    xtrans_layout: wgpu::BindGroupLayout,
    xtrans: Vec<wgpu::ComputePipeline>,
}

impl Passes {
    fn new(device: &wgpu::Device) -> Self {
        use Slot::{Float, Storage, StorageRw, Store, Uint, Uniform};
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba16Float, Rgba32Float};
        let xtrans_layout = layout(
            device,
            "tile xtrans",
            &[
                Uniform,
                Uniform,
                Float,
                StorageRw,
                StorageRw,
                StorageRw,
                Store(Rgba32Float),
            ],
        );
        let source = format!(
            "{}\n{}",
            include_str!("shaders/tile_common.wgsl"),
            include_str!("shaders/tile_xtrans.wgsl")
        );
        let xtrans = XSTEPS
            .iter()
            .map(|(_, entry)| pipeline(device, entry, &xtrans_layout, &source, entry))
            .collect();
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
                concat!(
                    include_str!("shaders/tile_stage4_fn.wgsl"),
                    include_str!("shaders/tile_stage4.wgsl")
                ),
                &[Uniform, Float, Store(Rgba16Float)],
            ),
            stage4_full: pass(
                device,
                "tile stage 4, full resolution",
                concat!(
                    include_str!("shaders/tile_stage4_fn.wgsl"),
                    include_str!("shaders/tile_stage4_full.wgsl")
                ),
                &[Uniform, Float, Store(Rgba16Float), Storage],
            ),
            downsample: pass_with(
                device,
                "pyramid downsample",
                include_str!("shaders/tile_downsample.wgsl"),
                &[Float, Store(Rgba16Float)],
                false,
            ),
            xtrans_layout,
            xtrans,
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

/// Which demosaic a tile set is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Linear,
    Bayer,
    XTrans,
}

impl Kind {
    fn of(src: &RawSource) -> Self {
        match src.cfa {
            None => Kind::Linear,
            Some(Cfa::Bayer(_)) => Kind::Bayer,
            Some(Cfa::XTrans(_)) => Kind::XTrans,
        }
    }
}

/// RCD's working textures.
#[derive(Debug)]
struct Rcd {
    vh: wgpu::Texture,
    lpf: wgpu::Texture,
    pq: wgpu::Texture,
    pq_dir: wgpu::Texture,
    green: wgpu::Texture,
    rb: wgpu::Texture,
}

/// Markesteijn's working buffers (tile_xtrans.wgsl) and a bind group per
/// run of its sequence.
#[derive(Debug)]
struct Xt {
    rgb: wgpu::Buffer,
    scratch: wgpu::Buffer,
    homo: wgpu::Buffer,
    groups: Vec<wgpu::BindGroup>,
}

/// The per-tile resources: allocated once at the largest padded size, with
/// the working set of one demosaic only.
#[derive(Debug)]
struct Tiles {
    size: (u32, u32),
    kind: Kind,
    /// The raw rect: one 16-bit sample per texel, `samples` texels per pixel.
    raw: wgpu::Texture,
    cfa: wgpu::Texture,
    rgb: wgpu::Texture,
    rcd: Option<Rcd>,
    xt: Option<Xt>,
}

impl Tiles {
    fn new(device: &wgpu::Device, size: (u32, u32), samples: u32, kind: Kind) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba32Float};
        let (w, h) = (size.0 + 2 * RAW_MARGIN, size.1 + 2 * RAW_MARGIN);
        let px = u64::from(size.0) * u64::from(size.1);
        let buffer = |label, bytes: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        Self {
            kind,
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
            rgb: texture(device, "tile rgb", size, Rgba32Float),
            rcd: (kind == Kind::Bayer).then(|| Rcd {
                vh: texture(device, "tile vh", size, R32Float),
                lpf: texture(device, "tile lpf", size, R32Float),
                pq: texture(device, "tile pq", size, Rg32Float),
                pq_dir: texture(device, "tile pq dir", size, R32Float),
                green: texture(device, "tile green", size, R32Float),
                rb: texture(device, "tile rb", size, Rg32Float),
            }),
            xt: (kind == Kind::XTrans).then(|| Xt {
                // 8 images of 4 floats; 8 values; 8 bytes per pixel.
                rgb: buffer("tile xt rgb", px * 128),
                scratch: buffer("tile xt scratch", px * 32),
                homo: buffer("tile xt homo", px * 8),
                groups: Vec::new(),
            }),
        }
    }

    /// The bytes of the working set (DEV-01's "stage 1 to 4 scratch").
    fn scratch_bytes(&self) -> u64 {
        let px = u64::from(self.size.0) * u64::from(self.size.1);
        // cfa 4 B and rgb 16 B, plus RCD's vh, lpf, pq_dir, green (4 B) and
        // pq, rb (8 B), or Markesteijn's buffers.
        let set = match self.kind {
            Kind::Linear => 0,
            Kind::Bayer => 4 * 4 + 2 * 8,
            Kind::XTrans => 128 + 32 + 8,
        };
        px * (4 + 16 + set)
    }
}

pub(crate) fn padded_row(width: u32, bytes_per_pixel: u32) -> u32 {
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
    /// Markesteijn's per-image uniforms (hexagons, solitary green) and its
    /// sequence of steps with a uniform buffer per run.
    xt_image: wgpu::Buffer,
    xt_sequence: Vec<(XStep, usize)>,
    xt_runs: Vec<wgpu::Buffer>,
    tiles: Option<Tiles>,
    /// For [`RawStages::render`] only, made on first use.
    readback: Option<wgpu::Buffer>,
    pyramid: Option<Pyramid>,
    /// The id the next pyramid render or rebuild gets.
    next_id: u64,
    /// For the memory test: sample the allocator report after each tile's
    /// submit in [`RawStages::render_pyramid`], keeping the largest total.
    #[doc(hidden)]
    pub sample_memory: bool,
    #[doc(hidden)]
    pub peak_allocated: Option<u64>,
}

/// The image-wide inputs of a render, uploaded once.
#[derive(Debug)]
struct ImageBuffers {
    black: wgpu::Buffer,
    maps: wgpu::Buffer,
    gains: wgpu::Buffer,
    deltas: [u32; 4],
    map_count: u32,
    /// The X-Trans hexagons, for the shared uniform: rewritten before each
    /// run of tiles, since another image's work may have replaced them.
    xt: Option<XImageUniforms>,
}

impl ImageBuffers {
    fn restore(&self, gpu: &Gpu, xt_image: &wgpu::Buffer) {
        if let Some(u) = &self.xt {
            gpu.queue.write_buffer(xt_image, 0, bytemuck::bytes_of(u));
        }
    }
}

impl RawStages {
    pub fn new(gpu: &Gpu, config: TileConfig) -> Self {
        let device = &gpu.device;
        let (xt_sequence, runs) = xtrans_sequence();
        let xt_runs = runs
            .iter()
            .map(|r| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("xtrans run"),
                    size: std::mem::size_of::<XRunUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                gpu.queue.write_buffer(&b, 0, bytemuck::bytes_of(r));
                b
            })
            .collect();
        Self {
            xt_image: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xtrans image"),
                size: std::mem::size_of::<XImageUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            xt_sequence,
            xt_runs,
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
            next_id: 0,
            sample_memory: false,
            peak_allocated: None,
        }
    }

    /// The bytes of one tile's working textures (DEV-01's "stage 1 to 4
    /// scratch"). Zero before the first render.
    pub fn scratch_bytes(&self) -> u64 {
        self.tiles.as_ref().map_or(0, Tiles::scratch_bytes)
    }

    /// The bytes of the raw tile the upload lands in (DEV-01's "upload
    /// staging for one tile"), sized for three samples per pixel.
    pub fn upload_bytes(&self) -> u64 {
        self.tiles.as_ref().map_or(0, |t| {
            u64::from(t.raw.width()) * u64::from(t.raw.height()) * 2
        })
    }

    fn xt_groups(
        &self,
        device: &wgpu::Device,
        cfa: &wgpu::Texture,
        rgb: &wgpu::Texture,
        xt: &Xt,
    ) -> Vec<wgpu::BindGroup> {
        let (cfa, rgb) = (view(cfa), view(rgb));
        self.xt_runs
            .iter()
            .map(|run| {
                let entries = [
                    self.uniforms.as_entire_binding(),
                    self.xt_image.as_entire_binding(),
                    run.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&cfa),
                    xt.rgb.as_entire_binding(),
                    xt.scratch.as_entire_binding(),
                    xt.homo.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&rgb),
                ];
                let entries: Vec<_> = entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, resource)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource,
                    })
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("xtrans run"),
                    layout: &self.passes.xtrans_layout,
                    entries: &entries,
                })
            })
            .collect()
    }

    /// The pyramid of the last [`RawStages::render_pyramid`].
    pub fn pyramid(&self) -> Option<&Pyramid> {
        self.pyramid.as_ref()
    }

    /// The tiles of `src`, with the per-tile resources allocated to fit.
    fn prepare(&mut self, gpu: &Gpu, src: &RawSource) -> Vec<TileJob> {
        let (w, h) = src.size();
        let all = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        self.prepare_in(gpu, src, all)
    }

    /// The tiles of `region` (active-area coordinates, even origin), with
    /// the per-tile resources allocated to fit.
    fn prepare_in(&mut self, gpu: &Gpu, src: &RawSource, region: Rect) -> Vec<TileJob> {
        let (w, h) = src.size();
        let margin = if src.fix_bad_pixels.is_empty() {
            0
        } else {
            RAW_MARGIN
        };
        let kind = Kind::of(src);
        let tile = if kind == Kind::XTrans {
            self.config.xtrans_tile
        } else {
            self.config.tile
        };
        let jobs = jobs_in(
            region,
            (w, h),
            (src.active.left, src.active.top),
            src.stored,
            tile,
            self.config.apron,
            margin,
        );
        let largest = jobs.iter().fold((0, 0), |(a, b), j| {
            (a.max(j.padded.width), b.max(j.padded.height))
        });
        let fits = self.tiles.as_ref().is_some_and(|t| {
            t.kind == kind
                && t.size.0 >= largest.0
                && t.size.1 >= largest.1
                && t.raw.width() >= (largest.0 + 2 * RAW_MARGIN) * src.samples
                && t.raw.height() >= largest.1 + 2 * RAW_MARGIN
        });
        if !fits {
            self.tiles = None;
            self.readback = None;
            // wgpu frees dropped textures once no submitted work uses them.
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let full = tile + 2 * self.config.apron;
            // Allocated at the configured size, so every later image fits;
            // smaller only when this image is.
            let size = (full.min(w).max(largest.0), full.min(h).max(largest.1));
            let mut tiles = Tiles::new(&gpu.device, size, 3, kind);
            if let Some(xt) = tiles.xt.as_mut() {
                xt.groups = self.xt_groups(&gpu.device, &tiles.cfa, &tiles.rgb, xt);
            }
            self.tiles = Some(tiles);
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
        let mosaic = src.cfa.is_some() && until == Until::Stage2;
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
        let mut xt = None;
        if let Some(Cfa::XTrans(x)) = src.cfa {
            let (hex, sgreen) = xtrans_hexagons(&x);
            let mut u = XImageUniforms {
                hex: [[0; 4]; 36],
                sgreen: [sgreen.0, sgreen.1, 0, 0],
            };
            for (cell, entries) in hex.iter().flatten().enumerate() {
                for (k, &(dx, dy)) in entries.iter().enumerate() {
                    let e = cell * 8 + k;
                    u.hex[e / 2][(e & 1) * 2] = dx;
                    u.hex[e / 2][(e & 1) * 2 + 1] = dy;
                }
            }
            gpu.queue
                .write_buffer(&self.xt_image, 0, bytemuck::bytes_of(&u));
            xt = Some(u);
        }
        let lin = &src.linearization;
        // A non-RAW image's curves take the levels' place (tile_stage1.wgsl).
        let mut black = match &src.encoding {
            Some(e) => e.lut(),
            None => lin.pattern.clone(),
        };
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
            xt,
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
            bayer: match src.cfa {
                Some(Cfa::Bayer(b)) => b.map(u32::from),
                _ => [0; 4],
            },
            cfa: [
                match src.cfa {
                    None => 0,
                    Some(Cfa::Bayer(_)) => 1,
                    Some(Cfa::XTrans(_)) => 2,
                },
                u32::from(src.encoding.is_some()),
                0,
                0,
            ],
            xtrans: {
                let mut x = [[0u32; 4]; 9];
                if let Some(Cfa::XTrans(p)) = src.cfa {
                    for i in 0..36 {
                        x[i / 4][i % 4] = u32::from(p[i / 6][i % 6]);
                    }
                }
                x
            },
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
        if demosaic && t.kind == Kind::Bayer {
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
            if demosaic && t.kind == Kind::XTrans {
                let xt = t.xt.as_ref().expect("an X-Trans tile set");
                for &(step, run) in &self.xt_sequence {
                    let i = XSTEPS.iter().position(|(s, _)| *s == step).expect("listed");
                    cpass.set_pipeline(&ps.xtrans[i]);
                    cpass.set_bind_group(0, &xt.groups[run], &[]);
                    cpass.dispatch_workgroups(p.width.div_ceil(WG), p.height.div_ceil(WG), 1);
                }
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
    rgb: wgpu::TextureView,
    vh: wgpu::TextureView,
    lpf: wgpu::TextureView,
    pq: wgpu::TextureView,
    pq_dir: wgpu::TextureView,
    green: wgpu::TextureView,
    rb: wgpu::TextureView,
}

impl TileViews {
    fn new(t: &Tiles) -> Self {
        // Without an RCD set, its views stand in on the cfa texture; they
        // are never bound.
        let rcd = |pick: fn(&Rcd) -> &wgpu::Texture| view(t.rcd.as_ref().map_or(&t.cfa, pick));
        Self {
            raw: view(&t.raw),
            cfa: view(&t.cfa),
            rgb: view(&t.rgb),
            vh: rcd(|r| &r.vh),
            lpf: rcd(|r| &r.lpf),
            pq: rcd(|r| &r.pq),
            pq_dir: rcd(|r| &r.pq_dir),
            green: rcd(|r| &r.green),
            rb: rcd(|r| &r.rb),
        }
    }

    fn r<'a>(&self, v: &'a wgpu::TextureView) -> wgpu::BindingResource<'a> {
        wgpu::BindingResource::TextureView(v)
    }
}

/// A white balance rebuild of the held pyramid, in place (DEV-01, Settled):
/// [`RawStages::rebuild_first`] renders the tiles a view reads, and
/// [`RawStages::rebuild_rest`] the others. Until it finishes, the pyramid
/// holds the new white only where the first part reached.
#[derive(Debug)]
pub struct Rebuild {
    image: ImageBuffers,
    rest: Vec<TileJob>,
    /// The pyramid's id when it started.
    id: u64,
}

impl Rebuild {
    /// The tiles not rendered yet.
    pub fn left(&self) -> usize {
        self.rest.len()
    }
}

/// The level-0 pixels (active-area coordinates) a view sampled per `s`
/// into `out` reads through the pyramid of an image whose level 0 is
/// `level0`: the texels of [`Sampling::source_rect`], then, level by level
/// down, every texel the means behind them average. None for a view of
/// level 0, which the level-0 tiles serve, or one that reads nothing.
pub fn level0_reach(s: &Sampling, out: (u32, u32), level0: (u32, u32)) -> Option<Rect> {
    let r = s.source_rect(out);
    if s.level == 0 || r.width == 0 || r.height == 0 {
        return None;
    }
    let mut sizes = vec![level0];
    for _ in 0..s.level {
        sizes.push(half(*sizes.last().expect("level 0")));
    }
    let mut x = (r.x, r.x + r.width);
    let mut y = (r.y, r.y + r.height);
    // Levels 2 and up: downsample.wgsl's 2 x 2 box, clamped at the edge.
    for level in (2..=s.level as usize).rev() {
        let below = sizes[level - 1];
        let down = |(a, b): (u32, u32), n: u32| ((2 * a).min(n - 1), (2 * b - 1).min(n - 1) + 1);
        x = down(x, below.0);
        y = down(y, below.1);
    }
    // Level 1: tile_stage4.wgsl's spans, an odd last pixel folded in.
    let up =
        |(a, b): (u32, u32), n: u32, src: u32| (span(a, n, src).start, span(b - 1, n, src).end);
    let x = up(x, sizes[1].0, level0.0);
    let y = up(y, sizes[1].1, level0.1);
    Some(Rect {
        x: x.0,
        y: y.0,
        width: x.1 - x.0,
        height: y.1 - y.0,
    })
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
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
        self.free_pyramid(gpu);
        let jobs = self.prepare(gpu, src);
        let pyramid = self.new_pyramid(gpu, half(src.size()), transform, &src.encoding);
        let image = self.upload_image(gpu, src);
        self.update(gpu, src, &image, &pyramid, &jobs, &mut || false)?;
        Ok(self.pyramid.insert(pyramid))
    }

    /// Starts rebuilding the held pyramid of `src` with `transform` (a new
    /// white balance), in place, so no second pyramid is allocated: the
    /// tiles whose output meets `first` (level-0 pixels, active-area
    /// coordinates), then levels 2 and up from the level 1 that holds. A
    /// view whose [`level0_reach`] lies inside `first` then samples only
    /// new texels, and reads as it would from a whole new pyramid. The
    /// other tiles wait for [`RawStages::rebuild_rest`]. On an error the
    /// pyramid is dropped, since it may hold both whites:
    /// [`RawStages::render_pyramid`] makes it again.
    pub fn rebuild_first(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        transform: &CameraTransform,
        first: Option<Rect>,
    ) -> Result<Rebuild, GpuError> {
        let held = self.pyramid.as_ref().expect("render_pyramid first");
        assert_eq!(
            held.size,
            half(src.size()),
            "a rebuild is of the same image"
        );
        let jobs = self.prepare(gpu, src);
        let (now, rest): (Vec<_>, Vec<_>) = jobs
            .into_iter()
            .partition(|j| first.is_some_and(|r| overlaps(j.out, r)));
        let image = self.upload_image(gpu, src);
        let mut pyramid = self.pyramid.take().expect("held");
        pyramid.transform = transform.clone();
        // Any older Rebuild is now stale.
        pyramid.id = self.take_id();
        let id = pyramid.id;
        // Failed, the pyramid holds both whites and its transform names only
        // the new one: it goes, and the caller renders it again.
        self.update(gpu, src, &image, &pyramid, &now, &mut || false)?;
        self.pyramid = Some(pyramid);
        Ok(Rebuild { image, rest, id })
    }

    /// Renders the tiles `rebuild` has left, asking `stop` before each one.
    /// Stopped, it returns false and `rebuild` keeps the tiles not done:
    /// work for an older request ends between tiles (DEV-01, Generations).
    /// Done, it rebuilds levels 2 and up and returns true; the pyramid then
    /// equals [`RawStages::render_pyramid`]'s with the new white, bit for
    /// bit. Panics when a render or another rebuild replaced the pyramid
    /// since `rebuild`'s [`RawStages::rebuild_first`].
    pub fn rebuild_rest(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        rebuild: &mut Rebuild,
        mut stop: impl FnMut() -> bool,
    ) -> Result<bool, GpuError> {
        let held = self.pyramid.as_ref().expect("render_pyramid first");
        assert_eq!(
            held.id, rebuild.id,
            "a rebuild resumes only the pyramid it started on: a later render or rebuild replaced it"
        );
        // Other work, a neighbor's level 1, may have refitted the tiles.
        self.prepare(gpu, src);
        let pyramid = self.pyramid.take().expect("held");
        let mut jobs = std::mem::take(&mut rebuild.rest);
        let result = self.update(gpu, src, &rebuild.image, &pyramid, &jobs, &mut stop);
        self.pyramid = Some(pyramid);
        match result {
            Ok(done) => {
                rebuild.rest = jobs.split_off(done);
                Ok(rebuild.rest.is_empty())
            }
            Err(e) => {
                rebuild.rest = jobs;
                Err(e)
            }
        }
    }

    fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Frees the held pyramid, so two are never in GPU memory at once.
    fn free_pyramid(&mut self, gpu: &Gpu) {
        if self.pyramid.take().is_some() {
            // wgpu frees a dropped texture once no submitted work uses it.
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
    }

    /// A new pyramid whose level 1 is `size`, with a new id.
    fn new_pyramid(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
        transform: &CameraTransform,
        encoding: &Option<InputEncoding>,
    ) -> Pyramid {
        let levels = level_count(size);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pyramid"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Pyramid {
            texture,
            size,
            levels,
            transform: transform.clone(),
            encoding: encoding.clone(),
            id: self.take_id(),
        }
    }

    /// Stages 1 to 4 for `jobs` into `pyramid`'s level 1 with its
    /// transform, asking `stop` before each tile, then, when every tile is
    /// done, levels 2 and up. Returns how many tiles were done.
    fn update(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        image: &ImageBuffers,
        pyramid: &Pyramid,
        jobs: &[TileJob],
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<usize, GpuError> {
        image.restore(gpu, &self.xt_image);
        let transform = &pyramid.transform;
        let device = &gpu.device;
        let level1 = pyramid.level_view(1);
        let mut in_flight = VecDeque::new();
        for (done, job) in jobs.iter().enumerate() {
            if stop() {
                return Ok(done);
            }
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
            let mut encoder = self.encode_tile(gpu, src, image, job, transform.multipliers, true);
            self.encode_stage4(
                gpu,
                &mut encoder,
                transform,
                job,
                pyramid.size,
                &level1,
                false,
            );
            in_flight.push_back(gpu.queue.submit([encoder.finish()]));
            self.sample_allocated(gpu);
        }
        self.downsample(gpu, pyramid)?;
        Ok(jobs.len())
    }

    /// For the memory test: the allocator's total after a submit, while
    /// the uploads it staged still count.
    fn sample_allocated(&mut self, gpu: &Gpu) {
        if self.sample_memory
            && let Some(r) = gpu.device.generate_allocator_report()
        {
            let peak = self.peak_allocated.get_or_insert(0);
            *peak = (*peak).max(r.total_allocated_bytes);
        }
    }

    /// Encodes stage 4 for `job` into `dst`: the level 1 texels whose 2 x 2
    /// block starts in the tile, of a level 1 of `size`, stored at their
    /// place in the level, or from `dst`'s top left when `from_origin`.
    /// Returns the first of them and how many across and down.
    #[allow(clippy::too_many_arguments)]
    fn encode_stage4(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        transform: &CameraTransform,
        job: &TileJob,
        size: (u32, u32),
        dst: &wgpu::TextureView,
        from_origin: bool,
    ) -> ((u32, u32), (u32, u32)) {
        let m = transform.matrix;
        let mm = transform.multipliers;
        let clip = mm[0].min(mm[1]).min(mm[2]);
        let luma = luminance_row();
        let rgb = view(&self.tiles.as_ref().expect("prepared").rgb);
        // Tiles start on even pixels, and an odd last pixel folds into the
        // block before it, in the same tile (tiles::jobs).
        let first = (job.out.x / 2, job.out.y / 2);
        let end = (
            (job.out.x + job.out.width).div_ceil(2).min(size.0),
            (job.out.y + job.out.height).div_ceil(2).min(size.1),
        );
        let count = (end.0 - first.0, end.1 - first.1);
        let origin = if from_origin { first } else { (0, 0) };
        let u = Stage4Uniforms {
            m: m.map(|r| [r[0], r[1], r[2], 0.0]),
            luma: [luma[0], luma[1], luma[2], 0.0],
            params: [
                transform.exposure,
                clip,
                DESATURATE_FROM,
                if transform.highlight_desaturation {
                    1.0
                } else {
                    0.0
                },
            ],
            texels: [first.0, first.1, count.0, count.1],
            level: [size.0, size.1, origin.0, origin.1],
        };
        gpu.queue
            .write_buffer(&self.stage4_uniforms, 0, bytemuck::bytes_of(&u));
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: wgpu::BindingResource::TextureView(dst),
                },
            ],
        });
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("stage 4"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.passes.stage4.pipeline);
        cpass.set_bind_group(0, &group, &[]);
        cpass.dispatch_workgroups(count.0.div_ceil(WG), count.1.div_ceil(WG), 1);
        (first, count)
    }

    /// Levels 2 and up of `pyramid` from its level 1, each texel the mean of
    /// the 2 x 2 block below it; waits for the GPU.
    fn downsample(&self, gpu: &Gpu, pyramid: &Pyramid) -> Result<(), GpuError> {
        let device = &gpu.device;
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut sizes = pyramid.size;
        for level in 2..=pyramid.levels {
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
            let (src_view, dst_view) = (pyramid.level_view(level - 1), pyramid.level_view(level));
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
        Ok(())
    }
}

/// A photo's level 1 in CPU memory, RGBA16F row by row without padding:
/// what a prefetched neighbor keeps (DEV-01, Prefetched neighbors).
/// [`RawStages::upload_pyramid`] makes its pyramid from it.
#[derive(Debug, Clone)]
pub struct Level1 {
    pub size: (u32, u32),
    /// What stages 2 and 4 used, as [`Pyramid::transform`].
    pub transform: CameraTransform,
    pub encoding: Option<InputEncoding>,
    pub texels: Vec<u8>,
}

impl Level1 {
    pub fn bytes(&self) -> u64 {
        self.texels.len() as u64
    }
}

/// A [`Level1`] being rendered beside the held pyramid, tile by tile:
/// [`RawStages::start_level1`], then [`RawStages::continue_level1`] until
/// it is done. Its GPU memory is one tile's level 1 and the buffer it is
/// read back through, freed with it.
#[derive(Debug)]
pub struct Level1Build {
    level1: Level1,
    image: ImageBuffers,
    rest: Vec<TileJob>,
    scratch: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl Level1Build {
    /// The tiles not rendered yet.
    pub fn left(&self) -> usize {
        self.rest.len()
    }

    /// The bytes of its GPU memory: one tile's level 1 and its readback.
    pub fn gpu_bytes(&self) -> u64 {
        let (w, h) = (self.scratch.width(), self.scratch.height());
        u64::from(w) * u64::from(h) * 8 + self.readback.size()
    }

    /// The level 1. Panics when tiles are left.
    pub fn finish(self) -> Level1 {
        assert!(self.rest.is_empty(), "{} tiles left", self.rest.len());
        self.level1
    }
}

impl RawStages {
    /// Starts rendering `src`'s level 1 with `transform` into CPU memory,
    /// leaving the held pyramid as it is: a prefetched neighbor's (DEV-01,
    /// Prefetched neighbors). Renders no tile yet.
    pub fn start_level1(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        transform: &CameraTransform,
    ) -> Level1Build {
        let jobs = self.prepare(gpu, src);
        let size = half(src.size());
        // The most texels one tile writes, an odd last pixel folded in.
        let most = jobs.iter().fold((1, 1), |(a, b), j| {
            (
                a.max(j.out.width.div_ceil(2)),
                b.max(j.out.height.div_ceil(2)),
            )
        });
        let scratch = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("level 1 tile"),
            size: wgpu::Extent3d {
                width: most.0,
                height: most.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("level 1 tile readback"),
            size: u64::from(padded_row(most.0, 8)) * u64::from(most.1),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Level1Build {
            level1: Level1 {
                size,
                transform: transform.clone(),
                encoding: src.encoding.clone(),
                texels: vec![0; size.0 as usize * size.1 as usize * 8],
            },
            image: self.upload_image(gpu, src),
            rest: jobs,
            scratch,
            readback,
        }
    }

    /// Renders the tiles `build` has left, each read back into its level 1
    /// before the next, asking `stop` before each one. Stopped, it returns
    /// false and `build` keeps the tiles not done: the render thread's
    /// requests come first (DEV-01, Generations). Done, it returns true;
    /// the level 1 then equals [`RawStages::render_pyramid`]'s, bit for bit.
    /// The held pyramid, and a [`Rebuild`] of it, are left as they were.
    pub fn continue_level1(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        build: &mut Level1Build,
        mut stop: impl FnMut() -> bool,
    ) -> Result<bool, GpuError> {
        assert_eq!(build.level1.size, half(src.size()), "the build's image");
        // Work on the held pyramid in between may have refitted the tiles.
        self.prepare(gpu, src);
        build.image.restore(gpu, &self.xt_image);
        let dst = view(&build.scratch);
        let size = build.level1.size;
        while let Some(job) = build.rest.first() {
            if stop() {
                return Ok(false);
            }
            let transform = &build.level1.transform;
            let mut encoder =
                self.encode_tile(gpu, src, &build.image, job, transform.multipliers, true);
            let (first, count) =
                self.encode_stage4(gpu, &mut encoder, transform, job, size, &dst, true);
            let row = padded_row(count.0, 8);
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &build.scratch,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &build.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(count.1),
                    },
                },
                wgpu::Extent3d {
                    width: count.0,
                    height: count.1,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit([encoder.finish()]);
            self.sample_allocated(gpu);
            let (tx, rx) = mpsc::channel();
            build.readback.map_async(wgpu::MapMode::Read, .., move |r| {
                let _ = tx.send(r);
            });
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            rx.recv()
                .map_err(|e| GpuError::Readback(e.to_string()))?
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            {
                let mapped = build
                    .readback
                    .get_mapped_range(..)
                    .map_err(|e| GpuError::Readback(e.to_string()))?;
                let line = count.0 as usize * 8;
                for y in 0..count.1 as usize {
                    let from = y * row as usize;
                    let to = ((first.1 as usize + y) * size.0 as usize + first.0 as usize) * 8;
                    build.level1.texels[to..to + line].copy_from_slice(&mapped[from..from + line]);
                }
            }
            build.readback.unmap();
            build.rest.remove(0);
        }
        Ok(true)
    }

    /// Makes the pyramid from `level1`, freeing the held one first: level 1
    /// is written in bands of at most [`UPLOAD_BAND`] bytes, then levels 2
    /// and up are computed as [`RawStages::render_pyramid`] computes them.
    /// The pyramid then equals render_pyramid's for the same photo and
    /// white, bit for bit, without decoding the photo (DEV-01, Prefetched
    /// neighbors: on a switch).
    pub fn upload_pyramid(&mut self, gpu: &Gpu, level1: &Level1) -> Result<&Pyramid, GpuError> {
        let (w, h) = level1.size;
        let row = w as usize * 8;
        assert_eq!(level1.texels.len(), row * h as usize, "level 1's texels");
        self.free_pyramid(gpu);
        let pyramid = self.new_pyramid(gpu, level1.size, &level1.transform, &level1.encoding);
        let rows = (UPLOAD_BAND / row).max(1);
        for y in (0..h as usize).step_by(rows) {
            let n = rows.min(h as usize - y);
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pyramid.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: y as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &level1.texels[y * row..(y + n) * row],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row as u32),
                    rows_per_image: Some(n as u32),
                },
                wgpu::Extent3d {
                    width: w,
                    height: n as u32,
                    depth_or_array_layers: 1,
                },
            );
            // Each band's staging is freed before the next is written.
            gpu.queue.submit([]);
            self.sample_allocated(gpu);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| GpuError::Request(e.to_string()))?;
        }
        self.downsample(gpu, &pyramid)?;
        Ok(self.pyramid.insert(pyramid))
    }
}

/// Where level-0 tiles go: an RGBA16F atlas of 256 px slots and the table
/// from each cache tile of the image to its slot (`gpu::level0`).
pub(crate) struct Level0Target<'a> {
    pub atlas: &'a wgpu::TextureView,
    pub slots: &'a wgpu::Buffer,
    /// Cache tiles across the image, slots across the atlas.
    pub tiles_across: u32,
    pub slots_across: u32,
}

impl RawStages {
    /// Stages 1 to 4 at full resolution over `region` (active-area
    /// coordinates, on the 256 px cache grid), each pixel written to its
    /// cache tile's slot when it has one (DEV-01, At 1:1). The tiles are the
    /// configured ones, so a region renders as the whole image would.
    pub(crate) fn render_level0(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        transform: &CameraTransform,
        region: Rect,
        target: &Level0Target,
    ) -> Result<(), GpuError> {
        let jobs = self.prepare_in(gpu, src, region);
        let image = self.upload_image(gpu, src);
        let m = transform.matrix;
        let mm = transform.multipliers;
        let clip = mm[0].min(mm[1]).min(mm[2]);
        let luma = luminance_row();
        let rgb = view(&self.tiles.as_ref().expect("prepared").rgb);
        let device = &gpu.device;
        let mut in_flight = VecDeque::new();
        for job in &jobs {
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
            let o = job.out;
            let u = Stage4Uniforms {
                m: m.map(|r| [r[0], r[1], r[2], 0.0]),
                luma: [luma[0], luma[1], luma[2], 0.0],
                params: [
                    transform.exposure,
                    clip,
                    DESATURATE_FROM,
                    if transform.highlight_desaturation {
                        1.0
                    } else {
                        0.0
                    },
                ],
                texels: [o.x, o.y, o.width, o.height],
                level: [target.tiles_across, target.slots_across, 0, 0],
            };
            gpu.queue
                .write_buffer(&self.stage4_uniforms, 0, bytemuck::bytes_of(&u));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stage 4, full resolution"),
                layout: &self.passes.stage4_full.layout,
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
                        resource: wgpu::BindingResource::TextureView(target.atlas),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: target.slots.as_entire_binding(),
                    },
                ],
            });
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stage 4, full resolution"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&self.passes.stage4_full.pipeline);
                cpass.set_bind_group(0, &group, &[]);
                cpass.dispatch_workgroups(o.width.div_ceil(WG), o.height.div_ceil(WG), 1);
            }
            in_flight.push_back(gpu.queue.submit([encoder.finish()]));
            if self.sample_memory
                && let Some(r) = device.generate_allocator_report()
            {
                let peak = self.peak_allocated.get_or_insert(0);
                *peak = (*peak).max(r.total_allocated_bytes);
            }
        }
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Request(e.to_string()))?;
        Ok(())
    }
}
