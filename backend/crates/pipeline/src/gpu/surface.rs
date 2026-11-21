//! A native surface for the photo (docs/native-surface-spike.md): the frame
//! stage 9 already holds on the GPU is drawn into a window's swapchain, with
//! no readback, encode or transport. The surface is made and used on the
//! render thread only, which owns the device.
//!
//! The frame is packed RGBA8 in the output target's encoding (one `R32Uint`
//! texel per pixel, R in the low byte), so the present pass unpacks it and
//! writes it to a non-sRGB format unchanged: what stage 9 produced is what
//! the swapchain holds. Converting to the monitor's profile is not done here
//! (spike: the color path is still open).

use std::time::Instant;

use super::{Gpu, GpuError};

const SHADER: &str = r#"
struct U { origin: vec2<i32>, size: vec2<i32>, clear: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var frame: texture_2d<u32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle over the whole target.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let q = vec2<i32>(p.xy) - u.origin;
    if (q.x < 0 || q.y < 0 || q.x >= u.size.x || q.y >= u.size.y) {
        return u.clear;
    }
    return unpack4x8unorm(textureLoad(frame, q, 0).r);
}
"#;

/// How a present waits for the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// Vsync, the usual.
    Fifo,
    /// The newest frame replaces a waiting one.
    Mailbox,
    /// No wait; tears. For measuring the pipeline without the display.
    Immediate,
}

impl Pacing {
    /// `GENZO_PRESENT=fifo|immediate`; anything else is [`Pacing::Mailbox`],
    /// which keeps the photo within half a display frame of a moving layout
    /// where vsync's queue lets it trail by more (docs/native-surface-spike.md,
    /// K2). A surface that lacks it falls back to [`Pacing::Fifo`].
    pub fn from_env() -> Self {
        match std::env::var("GENZO_PRESENT").as_deref() {
            Ok("fifo") => Self::Fifo,
            Ok("immediate") => Self::Immediate,
            _ => Self::Mailbox,
        }
    }

    fn mode(self) -> wgpu::PresentMode {
        match self {
            Self::Fifo => wgpu::PresentMode::Fifo,
            Self::Mailbox => wgpu::PresentMode::Mailbox,
            Self::Immediate => wgpu::PresentMode::Immediate,
        }
    }
}

/// What one present took.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presented {
    /// Waiting for the swapchain's next image: where vsync shows.
    pub acquire_ms: f64,
    /// Recording, submitting and handing the image to the display.
    pub submit_ms: f64,
    /// False when nothing was shown this time (the swapchain was outdated and
    /// is configured again, or the window is hidden or the wait timed out):
    /// the next frame or layout shows.
    pub shown: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    origin: [i32; 2],
    size: [i32; 2],
    clear: [f32; 4],
}

pub struct NativeSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// Where the photo's frame goes in the window, in device pixels (the
    /// top left of the hole the page leaves for it).
    pub origin: (i32, i32),
    /// What the surface offers and what it was asked for, for reports.
    pub info: String,
    /// The surface was lost: drop it and attach a new one.
    pub lost: bool,
}

impl NativeSurface {
    /// Configures `surface` (made by [`super::thread::RenderThread::create_surface`]
    /// on a window the shell holds), sized `size` in device pixels. Fails if
    /// the adapter the device was opened on cannot present to it.
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        size: (u32, u32),
        pacing: Pacing,
        color_space: wgpu::SurfaceColorSpace,
    ) -> Result<Self, GpuError> {
        if !gpu.adapter.is_surface_supported(&surface) {
            return Err(GpuError::Device(
                "the adapter cannot present to this window".into(),
            ));
        }
        let caps = surface.get_capabilities(&gpu.adapter);
        // A format that does not encode: the frame already is.
        let format = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ]
        .into_iter()
        .find(|f| caps.formats.contains(f))
        .ok_or_else(|| GpuError::Device("no plain 8-bit surface format".into()))?;
        let supported = caps.color_spaces(format);
        let wanted = color_space.to_color_spaces();
        let color_note = match wanted {
            Some(w) if !supported.contains(w) => {
                format!("{color_space:?} is NOT supported for {format:?}, using Auto")
            }
            _ => format!("{color_space:?}"),
        };
        let color_space = match wanted {
            Some(w) if !supported.contains(w) => wgpu::SurfaceColorSpace::Auto,
            _ => color_space,
        };
        let info = format!(
            "formats {:?}; color spaces per format {:?}; present modes {:?}; asked for {}",
            caps.formats,
            caps.format_capabilities
                .iter()
                .map(|c| (c.format, c.color_spaces))
                .collect::<Vec<_>>(),
            caps.present_modes,
            color_note,
        );
        let present_mode = if caps.present_modes.contains(&pacing.mode()) {
            pacing.mode()
        } else {
            wgpu::PresentMode::Fifo
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            color_space,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);

        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("present"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("present"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("present"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("present"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            surface,
            config,
            pipeline,
            layout,
            uniforms,
            origin: (0, 0),
            info,
            lost: false,
        })
    }

    fn skipped(&self, since: Instant) -> Presented {
        Presented {
            acquire_ms: since.elapsed().as_secs_f64() * 1e3,
            submit_ms: 0.0,
            shown: false,
        }
    }

    /// The window's size changed.
    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let (w, h) = (size.0.max(1), size.1.max(1));
        if (w, h) != (self.config.width, self.config.height) {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&gpu.device, &self.config);
        }
    }

    /// Draws `frame`'s top left `frame_size` at [`Self::origin`] and shows
    /// it. The rest of the window is cleared to a dark gray (the page covers
    /// it).
    pub fn present(
        &mut self,
        gpu: &Gpu,
        frame: &wgpu::Texture,
        frame_size: (u32, u32),
    ) -> Result<Presented, GpuError> {
        let t0 = Instant::now();
        let image = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            // Usable, but the window changed under it: show this one, and
            // configure again for the next.
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.surface.configure(&gpu.device, &self.config);
                t
            }
            // The configuration is out of date (a resize the layout has not
            // caught up with): configure again, nothing to show now.
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(self.skipped(t0));
            }
            // Hidden or slow: nothing to show, nothing to repair.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(self.skipped(t0));
            }
            // Reconfiguring does not bring a lost surface back: the shell
            // makes a new one ([`super::thread::SurfaceMaker`]) and attaches it.
            wgpu::CurrentSurfaceTexture::Lost => {
                self.lost = true;
                return Err(GpuError::Device("the surface was lost".into()));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(GpuError::Request("surface validation error".into()));
            }
        };
        let acquire_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        gpu.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                origin: [self.origin.0, self.origin.1],
                size: [frame_size.0 as i32, frame_size.1 as i32],
                clear: [0.07, 0.07, 0.07, 1.0],
            }),
        );
        let view = frame.create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let target = image.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(image);
        Ok(Presented {
            acquire_ms,
            submit_ms: t1.elapsed().as_secs_f64() * 1e3,
            shown: true,
        })
    }
}

impl std::fmt::Debug for NativeSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSurface")
            .field("config", &self.config)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
