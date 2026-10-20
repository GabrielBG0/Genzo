//! The base layer of Highlights and Shadows on the GPU (DEV-01, stage 6,
//! step 2), as `reference::base` computes it: the pyramid sampled at the
//! working size (long edge 1536 px) into `t`, then the guided filter's four
//! box passes (`base_box.wgsl`), leaving one plane of `(Ā, b̄)` as f16 pairs.
//! Stage 6 reads it bilinearly at any view (`ViewStages::tone`).
//!
//! It depends on the pyramid and the frame (the orientation, turns, flip
//! and crop) only, never on the view: it covers the cropped image, sampled
//! from the pyramid's levels, not from level 0, so Fit, 1:1 and export read
//! the same coefficients; a tight crop is sampled from level 1 upsampled.
//! Exposure, Highlights and Shadows do not rebuild it; white balance and
//! the geometry do (the caller calls [`BaseLayer::build`] again).
//!
//! Memory: two planes of 4 bytes per working pixel while it builds (18.9 MB
//! for a square image, `budget::BASE_LAYER`); the coefficients' plane alone
//! after.

use bytemuck::{Pod, Zeroable};

use super::raw_stages::{Pass, Pyramid, Slot, pass_with};
use super::view::{SampleUniforms, map, readback};
use super::{Gpu, GpuError};
use crate::reference::base::{EPSILON, T_FLOOR, radius, working_size};
use crate::reference::sampling::{Frame, Sampling, ViewSpec, plan_view_without_level0};

const WG: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WorkingUniforms {
    y_row: [f32; 4],
    floor: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoxUniforms {
    size: [u32; 4],
    eps: [f32; 4],
}

/// The bytes a base layer of working size `size` holds while it builds:
/// two planes of one u32 per pixel.
pub fn peak_bytes(size: (u32, u32)) -> u64 {
    2 * u64::from(size.0) * u64::from(size.1) * 4
}

/// The base layer's passes and its coefficients. Only the render thread
/// holds it.
#[derive(Debug)]
pub struct BaseLayer {
    sample: Pass,
    filter: Pass,
    sample_uniforms: wgpu::Buffer,
    working_uniforms: wgpu::Buffer,
    box_uniforms: [wgpu::Buffer; 4],
    /// `(Ā, b̄)` per working pixel, `pack2x16float`, row by row.
    coefficients: Option<wgpu::Buffer>,
    /// How the working image samples the pyramid.
    sampling: Option<Sampling>,
    size: (u32, u32),
}

impl BaseLayer {
    pub fn new(gpu: &Gpu) -> Self {
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
                "base layer t",
                concat!(
                    include_str!("shaders/view_sample.wgsl"),
                    include_str!("shaders/view_fetch_pyramid.wgsl"),
                    include_str!("shaders/base_store_t.wgsl")
                ),
                &[Slot::Float, Slot::StorageRw, Slot::Uniform],
                false,
            ),
            filter: pass_with(
                device,
                "base layer filter",
                include_str!("shaders/base_box.wgsl"),
                &[Slot::Storage, Slot::StorageRw],
                false,
            ),
            sample_uniforms: uniforms("base layer sample", size_of::<SampleUniforms>()),
            working_uniforms: uniforms("base layer t", size_of::<WorkingUniforms>()),
            box_uniforms: std::array::from_fn(|_| {
                uniforms("base layer filter", size_of::<BoxUniforms>())
            }),
            coefficients: None,
            sampling: None,
            size: (0, 0),
        }
    }

    /// Builds the base layer of `pyramid`, whose level 0 (the active area)
    /// is `level0`, in `frame` (an orientation, or the edit's geometry).
    /// Replaces the one held.
    pub fn build(
        &mut self,
        gpu: &Gpu,
        pyramid: &Pyramid,
        level0: (u32, u32),
        frame: impl Into<Frame>,
    ) {
        // The old plane goes first, so the new ones never add to it.
        self.coefficients = None;
        self.sampling = None;
        let frame = frame.into();
        let size = working_size(frame.pixel_size(level0));
        let s =
            plan_view_without_level0(level0, &pyramid.level_sizes(), frame, ViewSpec::Fit(size));
        let bytes = u64::from(size.0) * u64::from(size.1) * 4;
        let plane = |label| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let (main, scratch) = (plane("base layer"), plane("base layer scratch"));
        gpu.queue.write_buffer(
            &self.sample_uniforms,
            0,
            bytemuck::bytes_of(&SampleUniforms::new(&s, size, (0, 0))),
        );
        let y = genzo_color::WORKING_TO_XYZ[1];
        let floor = genzo_color::tone::MIDDLE_GRAY * T_FLOOR.exp2();
        gpu.queue.write_buffer(
            &self.working_uniforms,
            0,
            bytemuck::bytes_of(&WorkingUniforms {
                y_row: [y[0] as f32, y[1] as f32, y[2] as f32, 0.0],
                floor: [floor as f32, 0.0, 0.0, 0.0],
            }),
        );
        let r = radius(size);
        for (pass, u) in self.box_uniforms.iter().enumerate() {
            gpu.queue.write_buffer(
                u,
                0,
                bytemuck::bytes_of(&BoxUniforms {
                    size: [size.0, size.1, r, pass as u32],
                    eps: [EPSILON as f32, 0.0, 0.0, 0.0],
                }),
            );
        }
        let level = pyramid.level_view(s.level);
        let sample = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("base layer t"),
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
                    resource: main.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.working_uniforms.as_entire_binding(),
                },
            ],
        });
        // t into the main plane; then main → scratch → main, twice.
        let filters: Vec<wgpu::BindGroup> = self
            .box_uniforms
            .iter()
            .enumerate()
            .map(|(pass, u)| {
                let (from, to) = if pass % 2 == 0 {
                    (&main, &scratch)
                } else {
                    (&scratch, &main)
                };
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("base layer filter"),
                    layout: &self.filter.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: u.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: from.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: to.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let groups = (size.0.div_ceil(WG), size.1.div_ceil(WG));
        for (pass, group) in std::iter::once((&self.sample, &sample))
            .chain(filters.iter().map(|g| (&self.filter, g)))
        {
            let mut cp = encoder.begin_compute_pass(&Default::default());
            cp.set_pipeline(&pass.pipeline);
            cp.set_bind_group(0, group, &[]);
            cp.dispatch_workgroups(groups.0, groups.1, 1);
        }
        gpu.queue.submit([encoder.finish()]);
        // The scratch plane is freed once the GPU is done with it.
        drop(scratch);
        self.coefficients = Some(main);
        self.sampling = Some(s);
        self.size = size;
    }

    /// The working size of the base layer held.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// How the working image samples the pyramid; None before a build.
    pub fn sampling(&self) -> Option<&Sampling> {
        self.sampling.as_ref()
    }

    /// The bytes held between builds: the coefficients' plane.
    pub fn bytes(&self) -> u64 {
        self.coefficients.as_ref().map_or(0, wgpu::Buffer::size)
    }

    pub(crate) fn coefficients(&self) -> Option<&wgpu::Buffer> {
        self.coefficients.as_ref()
    }

    /// Frees the coefficients, for a new photo.
    pub fn clear(&mut self) {
        self.coefficients = None;
        self.sampling = None;
        self.size = (0, 0);
    }

    /// The coefficients `(Ā, b̄)`, row by row: for tests.
    pub fn read(&self, gpu: &Gpu) -> Result<Vec<[f32; 2]>, GpuError> {
        let plane = self.coefficients.as_ref().expect("build first");
        let buffer = readback(gpu, plane.size());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(plane, 0, &buffer, 0, plane.size());
        let bytes = map(gpu, encoder, &buffer)?;
        Ok(bytemuck::cast_slice::<u8, u32>(&bytes)
            .iter()
            .map(|&v| {
                let lo = ::half::f16::from_bits(v as u16).to_f32();
                let hi = ::half::f16::from_bits((v >> 16) as u16).to_f32();
                [lo, hi]
            })
            .collect())
    }
}
