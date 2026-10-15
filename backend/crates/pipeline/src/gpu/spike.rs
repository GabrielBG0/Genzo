//! The minimal raw path of spike 0.D, on RGBA16F textures:
//!
//! 1. linearize and white balance (full resolution)
//! 2. RCD demosaic, seven passes (full resolution)
//! 3. the mip pyramid of the demosaiced image
//! 4. resample to the viewport and apply the camera matrix (viewport size)
//! 5. exposure and the output transform to 8-bit sRGB or Display P3
//! 6. optionally, a half-size copy of the result for drag previews
//!
//! Each step's result is cached. A render re-runs from the earliest step whose
//! inputs changed ([`Stage`]); an exposure change only re-runs step 5.

use std::sync::mpsc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use genzo_color::Mat3;

use super::{Gpu, GpuError};

const WG: u32 = 16;

/// A raw sensor image, already unpacked to one 16-bit sample per pixel.
#[derive(Debug, Clone)]
pub struct BayerImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
    /// The CFA color (0 red, 1 green, 2 blue) at (0,0), (1,0), (0,1), (1,1).
    pub cfa: [u8; 4],
    pub black: f32,
    pub white: f32,
    pub cam_to_working: [[f32; 3]; 3],
}

/// The display encoding of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySpace {
    Srgb,
    DisplayP3,
}

/// The edit parameters the spike exposes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpikeParams {
    /// White balance gains for red, green and blue.
    pub wb: [f32; 3],
    pub exposure_ev: f32,
    pub space: DisplaySpace,
    /// A slider is being dragged. White balance is then applied after the
    /// cached demosaic, as a ratio to the white balance it was demosaiced
    /// with, instead of re-running the demosaic. The settled render (false)
    /// re-runs it.
    pub interactive: bool,
    /// Read back a half-size frame instead of the full viewport frame. The
    /// stages and their caches are unchanged; a small extra pass shrinks the
    /// finished frame, so the readback and everything after it moves a
    /// quarter of the bytes.
    pub proxy: bool,
}

impl Default for SpikeParams {
    fn default() -> Self {
        Self {
            wb: [1.0; 3],
            exposure_ev: 0.0,
            space: DisplaySpace::Srgb,
            interactive: false,
            proxy: false,
        }
    }
}

/// The step a render starts from, in pipeline order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// The image changed, or a settled render after a white balance change:
    /// everything re-runs.
    Linearize,
    /// The viewport size or, while dragging, the white balance changed:
    /// resample and the output re-run.
    Viewport,
    /// Exposure or the display space changed: only the output re-runs.
    Output,
}

/// One rendered viewport frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 in the requested display encoding.
    pub rgba: Vec<u8>,
    pub from: Stage,
    /// GPU time of the stages that ran, from timestamp queries, if the
    /// adapter has them.
    pub gpu_ms: Option<f64>,
    /// Submit to the GPU finishing, measured on the CPU.
    pub wall_ms: f64,
    /// Mapping and copying the frame out, after the GPU finished.
    pub readback_ms: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    dims: [u32; 4],
    cfa: [u32; 4],
    levels: [f32; 4],
    wb: [f32; 4],
    cam: [[f32; 4]; 3],
    out: [[f32; 4]; 3],
    drag: [f32; 4],
}

/// What a binding slot holds; binding 0 is always the uniforms.
#[derive(Clone, Copy)]
enum Slot {
    /// A texture read with `textureLoad`, of the given sample type.
    Tex(wgpu::TextureSampleType),
    /// A filterable texture read through a sampler, with all its mips.
    Sampled,
    Sampler,
    Store(wgpu::TextureFormat),
}

const FLOAT: Slot = Slot::Tex(wgpu::TextureSampleType::Float { filterable: false });
const UINT: Slot = Slot::Tex(wgpu::TextureSampleType::Uint);

#[derive(Debug)]
struct Pass {
    name: &'static str,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

fn pass(device: &wgpu::Device, name: &'static str, body: &str, slots: &[Slot]) -> Pass {
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
            Slot::Tex(sample_type) => wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            Slot::Sampled => wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            Slot::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
    let source = format!("{}\n{}", include_str!("shaders/common.wgsl"), body);
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
    Pass {
        name,
        layout,
        pipeline,
    }
}

#[derive(Debug)]
struct Passes {
    linearize: Pass,
    vh: Pass,
    lpf_pq: Pass,
    pq_dir: Pass,
    green: Pass,
    rb: Pass,
    rcd_final: Pass,
    downsample: Pass,
    viewport: Pass,
    output: Pass,
    shrink: Pass,
}

impl Passes {
    fn new(device: &wgpu::Device) -> Self {
        use wgpu::TextureFormat::{R32Float, Rg32Float, Rgba8Unorm, Rgba16Float};
        let store = Slot::Store;
        Self {
            linearize: pass(
                device,
                "linearize",
                include_str!("shaders/linearize.wgsl"),
                &[UINT, store(R32Float)],
            ),
            vh: pass(
                device,
                "rcd_vh",
                include_str!("shaders/rcd_vh.wgsl"),
                &[FLOAT, store(R32Float)],
            ),
            lpf_pq: pass(
                device,
                "rcd_lpf_pq",
                include_str!("shaders/rcd_lpf_pq.wgsl"),
                &[FLOAT, store(R32Float), store(Rg32Float)],
            ),
            pq_dir: pass(
                device,
                "rcd_pq_dir",
                include_str!("shaders/rcd_pq_dir.wgsl"),
                &[FLOAT, store(R32Float)],
            ),
            green: pass(
                device,
                "rcd_green",
                include_str!("shaders/rcd_green.wgsl"),
                &[FLOAT, FLOAT, FLOAT, store(R32Float)],
            ),
            rb: pass(
                device,
                "rcd_rb",
                include_str!("shaders/rcd_rb.wgsl"),
                &[FLOAT, FLOAT, FLOAT, store(Rg32Float)],
            ),
            rcd_final: pass(
                device,
                "rcd_final",
                include_str!("shaders/rcd_final.wgsl"),
                &[FLOAT, FLOAT, FLOAT, FLOAT, store(Rgba16Float)],
            ),
            downsample: pass(
                device,
                "downsample",
                include_str!("shaders/downsample.wgsl"),
                &[
                    Slot::Tex(wgpu::TextureSampleType::Float { filterable: true }),
                    store(Rgba16Float),
                ],
            ),
            viewport: pass(
                device,
                "viewport",
                include_str!("shaders/viewport.wgsl"),
                &[Slot::Sampled, Slot::Sampler, store(Rgba16Float)],
            ),
            output: pass(
                device,
                "output",
                include_str!("shaders/output.wgsl"),
                &[FLOAT, store(Rgba8Unorm)],
            ),
            shrink: pass(
                device,
                "shrink",
                include_str!("shaders/shrink.wgsl"),
                &[FLOAT, store(Rgba8Unorm)],
            ),
        }
    }
}

/// A bind group and the grid it is dispatched over.
#[derive(Debug)]
struct Dispatch {
    pass: &'static str,
    group: wgpu::BindGroup,
    size: (u32, u32),
}

/// A 2D texture that shaders write as storage and read back.
fn texture(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    mips: u32,
) -> wgpu::Texture {
    texture_with(
        device,
        label,
        size,
        format,
        mips,
        wgpu::TextureUsages::STORAGE_BINDING,
    )
}

fn texture_with(
    device: &wgpu::Device,
    label: &str,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
    mips: u32,
    extra: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | extra,
        view_formats: &[],
    })
}

fn r(v: &wgpu::TextureView) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(v)
}

fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&Default::default())
}

fn mip_view(t: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn bind(
    device: &wgpu::Device,
    pass: &Pass,
    uniforms: &wgpu::Buffer,
    views: &[&wgpu::BindingResource],
) -> wgpu::BindGroup {
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    for (i, r) in views.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: i as u32 + 1,
            resource: (*r).clone(),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(pass.name),
        layout: &pass.layout,
        entries: &entries,
    })
}

/// The full-resolution resources of one image.
#[derive(Debug)]
struct ImageState {
    size: (u32, u32),
    cfa: [u8; 4],
    black: f32,
    white: f32,
    cam: [[f32; 3]; 3],
    mips: wgpu::Texture,
    mip_count: u32,
    full: Vec<Dispatch>,
}

/// The viewport-size resources.
#[derive(Debug)]
struct ViewState {
    size: (u32, u32),
    mip_level: u32,
    viewport: Dispatch,
    output: Dispatch,
    display: wgpu::Texture,
    readback: wgpu::Buffer,
    padded_row: u32,
    proxy: ProxyState,
}

/// The half-size copy of the display frame, for drag previews.
#[derive(Debug)]
struct ProxyState {
    size: (u32, u32),
    shrink: Dispatch,
    display: wgpu::Texture,
    readback: wgpu::Buffer,
    padded_row: u32,
}

#[derive(Debug)]
struct Timer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    period_ns: f64,
}

/// The spike pipeline on one device. Not thread-safe by design: only the
/// render thread holds it.
#[derive(Debug)]
pub struct SpikePipeline {
    passes: Passes,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    timer: Option<Timer>,
    image: Option<ImageState>,
    view: Option<ViewState>,
    last: Option<SpikeParams>,
    /// The white balance the cached demosaic was made with.
    demosaic_wb: Option<[f32; 3]>,
}

/// How many mip levels to build: down to the first level whose long edge
/// is 256 px or less.
fn mip_count(w: u32, h: u32) -> u32 {
    let mut n = 1;
    while (w.max(h) >> (n - 1)) > 256 {
        n += 1;
    }
    n
}

/// The size of `image` scaled to fit inside `bounds`, never enlarged.
pub fn fit((w, h): (u32, u32), (bw, bh): (u32, u32)) -> (u32, u32) {
    let s = (bw as f64 / w as f64).min(bh as f64 / h as f64).min(1.0);
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

fn to_f32(m: &Mat3) -> [[f32; 3]; 3] {
    m.map(|row| row.map(|v| v as f32))
}

/// Working space to the display space's linear RGB.
fn output_matrix(space: DisplaySpace) -> [[f32; 3]; 3] {
    let to_xyz = match space {
        DisplaySpace::Srgb => genzo_color::SRGB_TO_XYZ,
        DisplaySpace::DisplayP3 => genzo_color::DISPLAY_P3_TO_XYZ,
    };
    let from_xyz = genzo_color::invert(&to_xyz).expect("display matrices are invertible");
    to_f32(&genzo_color::mat_mul(
        &from_xyz,
        &genzo_color::WORKING_TO_XYZ,
    ))
}

fn row4(r: [f32; 3]) -> [f32; 4] {
    [r[0], r[1], r[2], 0.0]
}

impl SpikePipeline {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let timer = gpu.info.timestamps.then(|| Timer {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("stage timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timer resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timer readback"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period_ns: gpu.queue.get_timestamp_period() as f64,
        });
        Self {
            passes: Passes::new(device),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("viewport"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            timer,
            image: None,
            view: None,
            last: None,
            demosaic_wb: None,
        }
    }

    /// Uploads a raw image and allocates its full-resolution textures.
    pub fn set_image(&mut self, gpu: &Gpu, img: &BayerImage) {
        let device = &gpu.device;
        let size = (img.width, img.height);
        use wgpu::TextureFormat::{R16Uint, R32Float, Rg32Float, Rgba16Float};
        let raw = texture_with(
            device,
            "raw",
            size,
            R16Uint,
            1,
            wgpu::TextureUsages::empty(),
        );
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &raw,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&img.data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img.width * 2),
                rows_per_image: Some(img.height),
            },
            wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
        );
        let cfa = texture(device, "cfa", size, R32Float, 1);
        let vh = texture(device, "vh", size, R32Float, 1);
        let lpf = texture(device, "lpf", size, R32Float, 1);
        let pq = texture(device, "pq", size, Rg32Float, 1);
        let pq_dir = texture(device, "pq_dir", size, R32Float, 1);
        let green = texture(device, "green", size, R32Float, 1);
        let rb = texture(device, "rb", size, Rg32Float, 1);
        let mip_count = mip_count(img.width, img.height);
        let mips = texture(device, "mips", size, Rgba16Float, mip_count);

        let v = |t: &wgpu::Texture| view(t);
        let (raw_v, cfa_v, vh_v, lpf_v, pq_v, pq_dir_v, green_v, rb_v) = (
            v(&raw),
            v(&cfa),
            v(&vh),
            v(&lpf),
            v(&pq),
            v(&pq_dir),
            v(&green),
            v(&rb),
        );
        let p = &self.passes;
        let u = &self.uniforms;
        let mip0 = mip_view(&mips, 0);
        let mut full = vec![
            Dispatch {
                pass: p.linearize.name,
                group: bind(device, &p.linearize, u, &[&r(&raw_v), &r(&cfa_v)]),
                size,
            },
            Dispatch {
                pass: p.vh.name,
                group: bind(device, &p.vh, u, &[&r(&cfa_v), &r(&vh_v)]),
                size,
            },
            Dispatch {
                pass: p.lpf_pq.name,
                group: bind(device, &p.lpf_pq, u, &[&r(&cfa_v), &r(&lpf_v), &r(&pq_v)]),
                size,
            },
            Dispatch {
                pass: p.pq_dir.name,
                group: bind(device, &p.pq_dir, u, &[&r(&pq_v), &r(&pq_dir_v)]),
                size,
            },
            Dispatch {
                pass: p.green.name,
                group: bind(
                    device,
                    &p.green,
                    u,
                    &[&r(&cfa_v), &r(&lpf_v), &r(&vh_v), &r(&green_v)],
                ),
                size,
            },
            Dispatch {
                pass: p.rb.name,
                group: bind(
                    device,
                    &p.rb,
                    u,
                    &[&r(&cfa_v), &r(&green_v), &r(&pq_dir_v), &r(&rb_v)],
                ),
                size,
            },
            Dispatch {
                pass: p.rcd_final.name,
                group: bind(
                    device,
                    &p.rcd_final,
                    u,
                    &[&r(&cfa_v), &r(&green_v), &r(&rb_v), &r(&vh_v), &r(&mip0)],
                ),
                size,
            },
        ];
        for level in 1..mip_count {
            let src = mip_view(&mips, level - 1);
            let dst = mip_view(&mips, level);
            let dst_size = ((img.width >> level).max(1), (img.height >> level).max(1));
            full.push(Dispatch {
                pass: p.downsample.name,
                group: bind(device, &p.downsample, u, &[&r(&src), &r(&dst)]),
                size: dst_size,
            });
        }
        self.image = Some(ImageState {
            size,
            cfa: img.cfa,
            black: img.black,
            white: img.white,
            cam: img.cam_to_working,
            mips,
            mip_count,
            full,
        });
        self.view = None;
        self.last = None;
        self.demosaic_wb = None;
    }

    fn ensure_view(&mut self, device: &wgpu::Device, bounds: (u32, u32)) -> Result<(), GpuError> {
        let image = self.image.as_ref().ok_or(GpuError::NoImage)?;
        let size = fit(image.size, bounds);
        if self.view.as_ref().is_some_and(|v| v.size == size) {
            return Ok(());
        }
        // The finest level that is still at least as large as the viewport.
        let ratio = image.size.0 as f64 / size.0 as f64;
        let mip_level = (ratio.log2().floor().max(0.0) as u32).min(image.mip_count - 1);
        let working = texture(device, "working", size, wgpu::TextureFormat::Rgba16Float, 1);
        let display = texture(device, "display", size, wgpu::TextureFormat::Rgba8Unorm, 1);
        let padded_row = (size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display readback"),
            size: padded_row as u64 * size.1 as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (mips_v, working_v, display_v) = (view(&image.mips), view(&working), view(&display));
        let proxy_size = (size.0.div_ceil(2), size.1.div_ceil(2));
        let proxy_display = texture(
            device,
            "proxy display",
            proxy_size,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        );
        let proxy_padded_row = (proxy_size.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let proxy_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("proxy readback"),
            size: proxy_padded_row as u64 * proxy_size.1 as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let proxy_v = view(&proxy_display);
        let p = &self.passes;
        let u = &self.uniforms;
        self.view = Some(ViewState {
            size,
            mip_level,
            viewport: Dispatch {
                pass: p.viewport.name,
                group: bind(
                    device,
                    &p.viewport,
                    u,
                    &[
                        &r(&mips_v),
                        &wgpu::BindingResource::Sampler(&self.sampler),
                        &r(&working_v),
                    ],
                ),
                size,
            },
            output: Dispatch {
                pass: p.output.name,
                group: bind(device, &p.output, u, &[&r(&working_v), &r(&display_v)]),
                size,
            },
            display,
            readback,
            padded_row,
            proxy: ProxyState {
                size: proxy_size,
                shrink: Dispatch {
                    pass: p.shrink.name,
                    group: bind(device, &p.shrink, u, &[&r(&display_v), &r(&proxy_v)]),
                    size: proxy_size,
                },
                display: proxy_display,
                readback: proxy_readback,
                padded_row: proxy_padded_row,
            },
        });
        Ok(())
    }

    fn pass_by_name(&self, name: &str) -> &Pass {
        let p = &self.passes;
        [
            &p.linearize,
            &p.vh,
            &p.lpf_pq,
            &p.pq_dir,
            &p.green,
            &p.rb,
            &p.rcd_final,
            &p.downsample,
            &p.viewport,
            &p.output,
            &p.shrink,
        ]
        .into_iter()
        .find(|pass| pass.name == name)
        .expect("every dispatch names a pass")
    }

    /// Renders the current image into a viewport that fits `bounds`.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        params: SpikeParams,
        bounds: (u32, u32),
    ) -> Result<Frame, GpuError> {
        let view_was = self.view.as_ref().map(|v| v.size);
        self.ensure_view(&gpu.device, bounds)?;
        let image = self.image.as_ref().ok_or(GpuError::NoImage)?;
        let view = self.view.as_ref().expect("ensure_view set it");
        let from = match (self.last, self.demosaic_wb) {
            (None, _) | (_, None) => Stage::Linearize,
            (_, Some(wb)) if wb != params.wb && !params.interactive => Stage::Linearize,
            (Some(last), _) if last.wb != params.wb || view_was != Some(view.size) => {
                Stage::Viewport
            }
            (Some(_), _) => Stage::Output,
        };
        if from == Stage::Linearize {
            self.demosaic_wb = Some(params.wb);
        }
        let demosaic_wb = self.demosaic_wb.expect("set above");
        let ratio = |i: usize| params.wb[i] / demosaic_wb[i];

        let out = output_matrix(params.space);
        let uniforms = Uniforms {
            dims: [image.size.0, image.size.1, view.size.0, view.size.1],
            cfa: image.cfa.map(u32::from),
            levels: [
                image.black,
                1.0 / (image.white - image.black),
                view.mip_level as f32,
                0.0,
            ],
            wb: [
                params.wb[0],
                params.wb[1],
                params.wb[2],
                params.exposure_ev.exp2(),
            ],
            cam: image.cam.map(row4),
            out: out.map(row4),
            drag: [ratio(0), ratio(1), ratio(2), 0.0],
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut dispatches: Vec<&Dispatch> = Vec::new();
        if from <= Stage::Linearize {
            dispatches.extend(&image.full);
        }
        if from <= Stage::Viewport {
            dispatches.push(&view.viewport);
        }
        dispatches.push(&view.output);
        if params.proxy {
            dispatches.push(&view.proxy.shrink);
        }
        // What gets read back: the full frame or the half-size one.
        let (out_tex, out_buf, out_row, (w, h)) = if params.proxy {
            let p = &view.proxy;
            (&p.display, &p.readback, p.padded_row, p.size)
        } else {
            (&view.display, &view.readback, view.padded_row, view.size)
        };

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("spike"),
                timestamp_writes: self
                    .timer
                    .as_ref()
                    .map(|t| wgpu::ComputePassTimestampWrites {
                        query_set: &t.queries,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }),
            });
            for d in &dispatches {
                cpass.set_pipeline(&self.pass_by_name(d.pass).pipeline);
                cpass.set_bind_group(0, &d.group, &[]);
                cpass.dispatch_workgroups(d.size.0.div_ceil(WG), d.size.1.div_ceil(WG), 1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: out_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(out_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if let Some(t) = &self.timer {
            encoder.resolve_query_set(&t.queries, 0..2, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback, 0, 16);
        }

        let started = Instant::now();
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        out_buf.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        if let Some(t) = &self.timer {
            t.readback.map_async(wgpu::MapMode::Read, .., move |r| {
                let _ = tx2.send(r);
            });
        } else {
            drop(tx2);
        }
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        for r in rx.iter() {
            r.map_err(|e| GpuError::Readback(e.to_string()))?;
        }

        let read_started = Instant::now();
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        {
            let mapped = out_buf
                .get_mapped_range(..)
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            for row in mapped.chunks(out_row as usize) {
                rgba.extend_from_slice(&row[..w as usize * 4]);
            }
        }
        out_buf.unmap();
        let readback_ms = read_started.elapsed().as_secs_f64() * 1000.0;

        let gpu_ms = match &self.timer {
            Some(t) => {
                let ms = {
                    let mapped = t
                        .readback
                        .get_mapped_range(..)
                        .map_err(|e| GpuError::Readback(e.to_string()))?;
                    let ts: &[u64] = bytemuck::cast_slice(&mapped);
                    ts[1].saturating_sub(ts[0]) as f64 * t.period_ns / 1e6
                };
                t.readback.unmap();
                Some(ms)
            }
            None => None,
        };

        self.last = Some(params);
        Ok(Frame {
            width: w,
            height: h,
            rgba,
            from,
            gpu_ms,
            wall_ms,
            readback_ms,
        })
    }
}

fn render_gray(gpu: &Gpu) -> Result<Frame, GpuError> {
    let mut pipeline = SpikePipeline::new(gpu);
    let img = super::synthetic::mosaic(64, 48, |_, _| [0.18, 0.18, 0.18]);
    pipeline.set_image(gpu, &img);
    pipeline.render(gpu, SpikeParams::default(), (64, 48))
}

/// Renders a flat gray patch and checks every pixel. A driver that gets this
/// wrong is not used.
pub fn self_test(gpu: &Gpu) -> Result<(), GpuError> {
    // Shader compile and validation errors would otherwise panic the thread.
    let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = gpu.device.push_error_scope(wgpu::ErrorFilter::Internal);
    let rendered = render_gray(gpu);
    for scope in [internal, validation] {
        if let Some(e) = pollster::block_on(scope.pop()) {
            let first_line = e
                .to_string()
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or_default()
                .to_owned();
            return Err(GpuError::SelfTest(first_line));
        }
    }
    let frame = rendered?;
    // The working space and sRGB share the D65 white, so neutral stays neutral.
    let want = (genzo_color::srgb_encode(0.18) * 255.0).round() as i32;
    for px in frame.rgba.chunks(4) {
        for &c in &px[..3] {
            if (c as i32 - want).abs() > 1 {
                return Err(GpuError::SelfTest(format!(
                    "gray 0.18 rendered as {px:?}, expected {want}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_and_never_enlarges() {
        assert_eq!(fit((8064, 6048), (2560, 2560)), (2560, 1920));
        assert_eq!(fit((100, 50), (2560, 1440)), (100, 50));
        assert_eq!(fit((6000, 4000), (2560, 1440)), (2160, 1440));
    }

    #[test]
    fn mips_stop_at_256() {
        assert_eq!(mip_count(256, 100), 1);
        assert_eq!(mip_count(8064, 6048), 6);
    }
}
