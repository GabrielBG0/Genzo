//! What the view shows (DEV-01): the pyramid sampled into the view, kept as
//! the "stage 4 sampled" cache (RGBA16F, the size of the view); stage 6
//! (tone) and stage 7 (color), each into its own cache of the same size, so
//! a change re-runs from the earliest stage it touches; then stage 9 into
//! an RGBA8 frame from the latest of them. A view that skips stages 6 and 7
//! shows the sampled cache as Genzo Linear would.

use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use genzo_color::Mat3;
use genzo_color::output::{EDGE_STEPS, Target};

use super::level0::{Level0Cache, Level0Error};
use super::raw_stages::{Pass, Pyramid, RawStages, Slot, padded_row, pass_with};
use super::{Gpu, GpuError};
use crate::look::{BaseCurve, Look, Profile};
use crate::raw::RawSource;
use crate::reference::sampling::{Sampling, ViewSpec, plan, plan_view, plan_view_without_level0};

const WG: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SampleUniforms {
    across: [f32; 4],
    down: [f32; 4],
    footprint: [f32; 4],
    sizes: [u32; 4],
    grid: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OutputUniforms {
    to_target: [[f32; 4]; 3],
    to_lms: [[f32; 4]; 3],
    lms_to_lab: [[f32; 4]; 3],
    lab_to_lms: [[f32; 4]; 3],
    lms_to_rgb: [[f32; 4]; 3],
    compression: [f32; 4],
    size: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ToneUniforms {
    y_row: [f32; 4],
    to_prophoto: [[f32; 4]; 3],
    from_prophoto: [[f32; 4]; 3],
    controls: [f32; 4],
    ends: [f32; 4],
    ramp: [f32; 4],
    size: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LookUniforms {
    to_lms: [[f32; 4]; 3],
    from_lms: [[f32; 4]; 3],
    lms_to_lab: [[f32; 4]; 3],
    lab_to_lms: [[f32; 4]; 3],
    y_row: [f32; 4],
    controls: [f32; 4],
    size: [u32; 4],
}

/// Which cache holds the view's latest values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Latest {
    Sampled,
    Tone,
    Color,
}

fn rows(m: &Mat3) -> [[f32; 4]; 3] {
    m.map(|r| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0])
}

/// Whether stage 9 compresses the gamut for this pyramid (DEV-01): always
/// for RAW; for a non-RAW photo only when its source gamut is not inside
/// the target.
pub fn compresses(pyramid: &Pyramid, target: Target) -> bool {
    match &pyramid.encoding {
        None => true,
        Some(e) => !genzo_color::output::fits(&e.to_working, target),
    }
}

/// Whether the frame is dithered: not for an unedited non-RAW photo, which
/// shows its own codes (decided 2026-10-19, criterion 8); always otherwise.
pub fn dithers(pyramid: &Pyramid, edited: bool) -> bool {
    pyramid.encoding.is_none() || edited
}

/// The view's passes and buffers. Only the render thread holds it.
#[derive(Debug)]
pub struct ViewStages {
    sample: Pass,
    sample_level0: Pass,
    tone: Pass,
    color: Pass,
    output: Pass,
    sample_uniforms: wgpu::Buffer,
    tone_uniforms: wgpu::Buffer,
    color_uniforms: wgpu::Buffer,
    output_uniforms: wgpu::Buffer,
    /// The base curve's table (DEV-02): 4098 f32.
    curve: wgpu::Buffer,
    /// The stage 4 sampled cache and the frame, for the current view size.
    sampled: Option<wgpu::Texture>,
    /// The stage 6 and 7 caches, made on first use.
    toned: Option<wgpu::Texture>,
    colored: Option<wgpu::Texture>,
    latest: Latest,
    /// Whether the stage 6 cache holds stage 6 of the current sampling.
    toned_current: bool,
    frame: Option<wgpu::Buffer>,
    size: (u32, u32),
}

impl ViewStages {
    pub fn new(gpu: &Gpu) -> Self {
        use wgpu::TextureFormat::Rgba16Float;
        let device = &gpu.device;
        let uniforms = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            sample: pass_with(
                device,
                "view sample",
                concat!(
                    include_str!("shaders/view_sample.wgsl"),
                    include_str!("shaders/view_fetch_pyramid.wgsl")
                ),
                &[Slot::Float, Slot::Store(Rgba16Float)],
                false,
            ),
            sample_level0: pass_with(
                device,
                "view sample, level 0",
                concat!(
                    include_str!("shaders/view_sample.wgsl"),
                    include_str!("shaders/view_fetch_level0.wgsl")
                ),
                &[Slot::Float, Slot::Store(Rgba16Float), Slot::Storage],
                false,
            ),
            tone: pass_with(
                device,
                "view tone",
                include_str!("shaders/view_tone.wgsl"),
                &[Slot::Float, Slot::Store(Rgba16Float), Slot::Storage],
                false,
            ),
            color: pass_with(
                device,
                "view color",
                include_str!("shaders/view_look.wgsl"),
                &[Slot::Float, Slot::Store(Rgba16Float)],
                false,
            ),
            output: pass_with(
                device,
                "view output",
                include_str!("shaders/view_output.wgsl"),
                &[Slot::Float, Slot::StorageRw],
                false,
            ),
            sample_uniforms: uniforms("view sample", size_of::<SampleUniforms>()),
            tone_uniforms: uniforms("view tone", size_of::<ToneUniforms>()),
            color_uniforms: uniforms("view color", size_of::<LookUniforms>()),
            output_uniforms: uniforms("view output", size_of::<OutputUniforms>()),
            curve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("base curve table"),
                size: ((genzo_color::tone::TABLE_SIZE + 2) * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            sampled: None,
            toned: None,
            colored: None,
            latest: Latest::Sampled,
            toned_current: false,
            frame: None,
            size: (0, 0),
        }
    }

    /// The view's width and height.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The bytes of the viewport stage caches that exist (RGBA16F each):
    /// stage 4 sampled, and stages 6 and 7 once used.
    pub fn cache_bytes(&self) -> u64 {
        let caches = [&self.sampled, &self.toned, &self.colored]
            .iter()
            .filter(|t| t.is_some())
            .count() as u64;
        caches * u64::from(self.size.0) * u64::from(self.size.1) * 8
    }

    /// The bytes of the frame (RGBA8), DEV-01's output line.
    pub fn frame_bytes(&self) -> u64 {
        u64::from(self.size.0) * u64::from(self.size.1) * 4
    }

    fn cache(gpu: &Gpu, label: &str, size: (u32, u32)) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.size == size && self.sampled.is_some() {
            return;
        }
        // The old caches go first, so the new ones never add to them.
        self.sampled = None;
        self.toned = None;
        self.colored = None;
        self.frame = None;
        self.sampled = Some(Self::cache(gpu, "stage 4 sampled", size));
        self.frame = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: u64::from(size.0) * u64::from(size.1) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        self.size = size;
    }

    /// Samples `pyramid`, shown in orientation `o`, into an `out`-sized
    /// Fit view from its own levels only (no level 0): the stage 4 sampled
    /// cache.
    pub fn sample(&mut self, gpu: &Gpu, pyramid: &Pyramid, o: u8, out: (u32, u32)) -> Sampling {
        let s = plan(&pyramid.level_sizes(), o, out);
        self.sample_with(gpu, pyramid, None, &s, out);
        s
    }

    /// Shows `view` of `src` (its pyramid built, shown in orientation `o`):
    /// renders the level-0 tiles it needs into `cache` when it reads level
    /// 0 (`key` as for [`Level0Cache::ensure`]), then samples. When the
    /// cache cannot hold them all, it falls back to the pyramid's level 1.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        gpu: &Gpu,
        stages: &mut RawStages,
        cache: &mut Level0Cache,
        src: &RawSource,
        key: u64,
        o: u8,
        view: ViewSpec,
    ) -> Result<Sampling, GpuError> {
        let pyramid = stages.pyramid().expect("render_pyramid first");
        let transform = pyramid.transform.clone();
        let mut s = plan_view(src.size(), &pyramid.level_sizes(), o, view);
        let out = view.out();
        if s.level == 0 {
            let tiles = Level0Cache::tiles_for(s.source_rect(out), src.size());
            match cache.ensure(gpu, stages, src, &transform, key, &tiles) {
                Ok(()) => {}
                Err(Level0Error::Full { .. }) => {
                    let pyramid = stages.pyramid().expect("built");
                    s = plan_view_without_level0(src.size(), &pyramid.level_sizes(), o, view);
                }
                Err(Level0Error::Gpu(e)) => return Err(e),
            }
        }
        let pyramid = stages.pyramid().expect("built");
        self.sample_with(gpu, pyramid, (s.level == 0).then_some(&*cache), &s, out);
        Ok(s)
    }

    /// Samples per `s` into an `out`-sized view: from `pyramid` for levels 1
    /// and up, from `level0` (which must hold the tiles `s` reads) for 0.
    pub fn sample_with(
        &mut self,
        gpu: &Gpu,
        pyramid: &Pyramid,
        level0: Option<&Level0Cache>,
        s: &Sampling,
        out: (u32, u32),
    ) {
        self.resize(gpu, out);
        self.latest = Latest::Sampled;
        self.toned_current = false;
        let a = &s.affine;
        let grid = level0.map_or((0, 0), |c| c.grid());
        let u = SampleUniforms {
            across: [a[0][0] as f32, a[0][1] as f32, a[0][2] as f32, 0.0],
            down: [a[1][0] as f32, a[1][1] as f32, a[1][2] as f32, 0.0],
            footprint: [s.footprint[0] as f32, s.footprint[1] as f32, 0.0, 0.0],
            sizes: [s.level_size.0, s.level_size.1, out.0, out.1],
            grid: [grid.0, grid.1, 0, 0],
        };
        gpu.queue
            .write_buffer(&self.sample_uniforms, 0, bytemuck::bytes_of(&u));
        let sampled = self
            .sampled
            .as_ref()
            .expect("resized")
            .create_view(&Default::default());
        let level;
        let (pass, source, table) = match (s.level, level0) {
            (0, Some(cache)) => (&self.sample_level0, cache.atlas_view(), Some(cache.table())),
            (0, None) => panic!("level 0 needs the tile cache"),
            (l, _) => {
                level = pyramid.level_view(l);
                (&self.sample, &level, None)
            }
        };
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.sample_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&sampled),
            },
        ];
        if let Some(t) = table {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: t.as_entire_binding(),
            });
        }
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view sample"),
            layout: &pass.layout,
            entries: &entries,
        });
        self.dispatch(gpu, pass, &group, out);
    }

    fn latest_texture(&self) -> &wgpu::Texture {
        match self.latest {
            Latest::Sampled => self.sampled.as_ref(),
            Latest::Tone => self.toned.as_ref(),
            Latest::Color => self.colored.as_ref(),
        }
        .expect("sample first")
    }

    /// Stage 6 from the sampled cache into the tone cache. `base` is the
    /// photo's base curve, which Genzo Standard needs.
    pub fn tone(&mut self, gpu: &Gpu, look: &Look, base: Option<&BaseCurve>) {
        let sampled = self
            .sampled
            .as_ref()
            .expect("sample first")
            .create_view(&Default::default());
        if self.toned.is_none() {
            self.toned = Some(Self::cache(gpu, "stage 6", self.size));
        }
        let (to, from) = crate::reference::look::prophoto();
        let curve = look.profile == Profile::GenzoStandard;
        let ramp = match (curve, base) {
            (true, Some(b)) => {
                gpu.queue
                    .write_buffer(&self.curve, 0, bytemuck::cast_slice(b.table.values()));
                let r = b.ramp(look.exposure);
                let slope = 1.0 / (1.0 - r.black());
                let q = if r.radius() > 0.0 {
                    slope / (4.0 * r.radius())
                } else {
                    0.0
                };
                [r.black() as f32, r.radius() as f32, slope as f32, q as f32]
            }
            (true, None) => panic!("Genzo Standard needs its base curve"),
            (false, _) => [0.0; 4],
        };
        let y = genzo_color::WORKING_TO_XYZ[1];
        let u = ToneUniforms {
            y_row: [y[0] as f32, y[1] as f32, y[2] as f32, 0.0],
            to_prophoto: rows(&to),
            from_prophoto: rows(&from),
            controls: [
                look.exposure,
                look.whites / 100.0,
                look.blacks / 100.0,
                look.contrast / 100.0,
            ],
            ends: [
                genzo_color::tone::white_end() as f32,
                genzo_color::tone::black_end() as f32,
                genzo_color::tone::ENDS_RANGE as f32,
                genzo_color::tone::CONTRAST_GAIN as f32,
            ],
            ramp,
            size: [self.size.0, self.size.1, u32::from(curve), 0],
        };
        gpu.queue
            .write_buffer(&self.tone_uniforms, 0, bytemuck::bytes_of(&u));
        let toned = self
            .toned
            .as_ref()
            .expect("made")
            .create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view tone"),
            layout: &self.tone.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.tone_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sampled),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&toned),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.curve.as_entire_binding(),
                },
            ],
        });
        self.dispatch(gpu, &self.tone, &group, self.size);
        self.latest = Latest::Tone;
        self.toned_current = true;
    }

    /// Stage 7 from the tone cache (the sampled cache when stage 6 did not
    /// run since the last sampling) into the color cache.
    pub fn color(&mut self, gpu: &Gpu, look: &Look) {
        let input = if self.toned_current {
            self.toned.as_ref()
        } else {
            self.sampled.as_ref()
        }
        .expect("sample first")
        .create_view(&Default::default());
        if self.colored.is_none() {
            self.colored = Some(Self::cache(gpu, "stage 7", self.size));
        }
        let m = genzo_color::look::OklabMatrices::working();
        let y = genzo_color::WORKING_TO_XYZ[1];
        let lab_to_lms = genzo_color::invert(&genzo_color::output::OKLAB_M2).expect("invertible");
        let u = LookUniforms {
            to_lms: rows(&m.to_lms),
            from_lms: rows(&m.from_lms),
            lms_to_lab: rows(&genzo_color::output::OKLAB_M2),
            lab_to_lms: rows(&lab_to_lms),
            y_row: [y[0] as f32, y[1] as f32, y[2] as f32, 0.0],
            controls: [
                look.vibrance / 100.0,
                look.saturation / 100.0,
                if look.monochrome { 1.0 } else { 0.0 },
                genzo_color::look::reference_chroma() as f32,
            ],
            size: [self.size.0, self.size.1, 0, 0],
        };
        gpu.queue
            .write_buffer(&self.color_uniforms, 0, bytemuck::bytes_of(&u));
        let colored = self
            .colored
            .as_ref()
            .expect("made")
            .create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view color"),
            layout: &self.color.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.color_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&input),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&colored),
                },
            ],
        });
        self.dispatch(gpu, &self.color, &group, self.size);
        self.latest = Latest::Color;
    }

    /// Stage 9 from the latest cache (stage 7, 6 or the sampled one) into
    /// the frame.
    pub fn output(&mut self, gpu: &Gpu, target: Target, compress: bool, dither: bool) {
        assert!(self.sampled.is_some(), "sample first");
        let from = target.from_xyz_oklab();
        let c = target.compression();
        let lms = genzo_color::mat_mul(&genzo_color::output::OKLAB_M1, &target.to_xyz());
        let u = OutputUniforms {
            to_target: rows(&target.from_working()),
            to_lms: rows(&lms),
            lms_to_lab: rows(&genzo_color::output::OKLAB_M2),
            lab_to_lms: rows(&from.lab_to_lms),
            lms_to_rgb: rows(&from.lms_to_rgb),
            compression: [
                c.threshold as f32,
                c.limit as f32,
                c.power as f32,
                if compress { 1.0 } else { 0.0 },
            ],
            size: [self.size.0, self.size.1, EDGE_STEPS, u32::from(dither)],
        };
        gpu.queue
            .write_buffer(&self.output_uniforms, 0, bytemuck::bytes_of(&u));
        let sampled = self.latest_texture().create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view output"),
            layout: &self.output.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.output_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sampled),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frame.as_ref().expect("resized").as_entire_binding(),
                },
            ],
        });
        self.dispatch(gpu, &self.output, &group, self.size);
    }

    fn dispatch(&self, gpu: &Gpu, pass: &Pass, group: &wgpu::BindGroup, size: (u32, u32)) {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut cp = encoder.begin_compute_pass(&Default::default());
            cp.set_pipeline(&pass.pipeline);
            cp.set_bind_group(0, group, &[]);
            cp.dispatch_workgroups(size.0.div_ceil(WG), size.1.div_ceil(WG), 1);
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// The sampled cache, RGB row by row: for tests.
    pub fn read_sampled(&self, gpu: &Gpu) -> Result<Vec<[f32; 3]>, GpuError> {
        read_rgba16f(
            gpu,
            self.sampled.as_ref().expect("sampled"),
            (0, 0),
            self.size,
        )
    }

    /// The latest cache (stage 7, 6 or the sampled one), RGB row by row: for
    /// tests.
    pub fn read_latest(&self, gpu: &Gpu) -> Result<Vec<[f32; 3]>, GpuError> {
        read_rgba16f(gpu, self.latest_texture(), (0, 0), self.size)
    }

    /// The frame, RGBA8 row by row.
    pub fn read_frame(&self, gpu: &Gpu) -> Result<Vec<[u8; 4]>, GpuError> {
        let frame = self.frame.as_ref().expect("output first");
        let buffer = readback(gpu, frame.size());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(frame, 0, &buffer, 0, frame.size());
        let bytes = map(gpu, encoder, &buffer)?;
        Ok(bytes.as_chunks::<4>().0.to_vec())
    }
}

/// A rectangle of an RGBA16F texture as RGB, row by row.
pub(crate) fn read_rgba16f(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    origin: (u32, u32),
    (w, h): (u32, u32),
) -> Result<Vec<[f32; 3]>, GpuError> {
    let row = padded_row(w, 8);
    let buffer = readback(gpu, u64::from(row) * u64::from(h));
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin.0,
                y: origin.1,
                z: 0,
            },
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
    let bytes = map(gpu, encoder, &buffer)?;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        let start = y * row as usize;
        let line: &[u16] = bytemuck::cast_slice(&bytes[start..start + w as usize * 8]);
        for px in line.as_chunks::<4>().0 {
            out.push([0, 1, 2].map(|c| ::half::f16::from_bits(px[c]).to_f32()));
        }
    }
    Ok(out)
}

fn readback(gpu: &Gpu, size: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("view readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn map(
    gpu: &Gpu,
    encoder: wgpu::CommandEncoder,
    buffer: &wgpu::Buffer,
) -> Result<Vec<u8>, GpuError> {
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
    Ok(mapped.to_vec())
}
