//! What the view shows (DEV-01): the pyramid sampled into the view, kept as
//! the "stage 4 sampled" cache (RGBA16F, the size of the view), then stage
//! 9 into an RGBA8 frame. Stages 6 and 7 go between the two; until they
//! exist the view is what their defaults give, with the base tone curve of
//! DEV-02 left out (Genzo Linear: clamped at display white).

use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use genzo_color::Mat3;
use genzo_color::output::{EDGE_STEPS, Target};

use super::raw_stages::{Pass, Pyramid, Slot, padded_row, pass_with};
use super::{Gpu, GpuError};
use crate::reference::sampling::{Sampling, plan};

const WG: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SampleUniforms {
    across: [f32; 4],
    down: [f32; 4],
    footprint: [f32; 4],
    sizes: [u32; 4],
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
    output: Pass,
    sample_uniforms: wgpu::Buffer,
    output_uniforms: wgpu::Buffer,
    /// The stage 4 sampled cache and the frame, for the current view size.
    sampled: Option<wgpu::Texture>,
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
                include_str!("shaders/view_sample.wgsl"),
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
            output_uniforms: uniforms("view output", size_of::<OutputUniforms>()),
            sampled: None,
            frame: None,
            size: (0, 0),
        }
    }

    /// The view's width and height.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The bytes of the stage 4 sampled cache (RGBA16F): one of DEV-01's
    /// viewport stage caches.
    pub fn cache_bytes(&self) -> u64 {
        u64::from(self.size.0) * u64::from(self.size.1) * 8
    }

    /// The bytes of the frame (RGBA8), DEV-01's output line.
    pub fn frame_bytes(&self) -> u64 {
        u64::from(self.size.0) * u64::from(self.size.1) * 4
    }

    fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.size == size && self.sampled.is_some() {
            return;
        }
        self.sampled = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stage 4 sampled"),
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
        }));
        self.frame = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: u64::from(size.0) * u64::from(size.1) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
        self.size = size;
    }

    /// Samples `pyramid`, shown in orientation `o`, into an `out`-sized
    /// view: the stage 4 sampled cache.
    pub fn sample(&mut self, gpu: &Gpu, pyramid: &Pyramid, o: u8, out: (u32, u32)) -> Sampling {
        self.resize(gpu, out);
        let s = plan(&pyramid.level_sizes(), o, out);
        let a = &s.affine;
        let u = SampleUniforms {
            across: [a[0][0] as f32, a[0][1] as f32, a[0][2] as f32, 0.0],
            down: [a[1][0] as f32, a[1][1] as f32, a[1][2] as f32, 0.0],
            footprint: [s.footprint[0] as f32, s.footprint[1] as f32, 0.0, 0.0],
            sizes: [s.level_size.0, s.level_size.1, out.0, out.1],
        };
        gpu.queue
            .write_buffer(&self.sample_uniforms, 0, bytemuck::bytes_of(&u));
        let level = pyramid.level_view(s.level);
        let sampled = self
            .sampled
            .as_ref()
            .expect("resized")
            .create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view sample"),
            layout: &self.sample.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.sample_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&level),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&sampled),
                },
            ],
        });
        self.dispatch(gpu, &self.sample, &group, out);
        s
    }

    /// Stage 9 from the sampled cache into the frame.
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
        let sampled = self
            .sampled
            .as_ref()
            .expect("sampled")
            .create_view(&Default::default());
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
        let (w, h) = self.size;
        let row = padded_row(w, 8);
        let buffer = readback(gpu, u64::from(row) * u64::from(h));
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: self.sampled.as_ref().expect("sampled"),
                mip_level: 0,
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
