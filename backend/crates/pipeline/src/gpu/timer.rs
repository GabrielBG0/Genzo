//! GPU time of a span of submits, from timestamp queries: what `bench gpu`
//! reports as a render's GPU time.
//!
//! [`SpanTimer::start`] and [`SpanTimer::stop`] each submit an empty compute
//! pass that writes one timestamp, so the span covers every submit between
//! them, however many encoders the render uses. It is the GPU's time from
//! the first pass to the last, so a gap where the GPU waits for the CPU (a
//! readback inside the span) counts too. Without timestamp queries there is
//! no timer and no GPU time.

use std::sync::mpsc;

use super::{Gpu, GpuError};

#[derive(Debug)]
pub struct SpanTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    period_ns: f64,
    /// Whether a stopped span waits to be read.
    stopped: bool,
}

impl SpanTimer {
    /// A timer, if the device has timestamp queries.
    pub fn new(gpu: &Gpu) -> Option<Self> {
        let device = &gpu.device;
        gpu.info.timestamps.then(|| Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("span timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("span timer resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("span timer readback"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period_ns: f64::from(gpu.queue.get_timestamp_period()),
            stopped: false,
        })
    }

    /// Starts a span; a span stopped and not read is dropped.
    pub fn start(&mut self, gpu: &Gpu) {
        self.stopped = false;
        self.mark(gpu, 0, false);
    }

    /// Stops the span: its time is read by [`SpanTimer::read`].
    pub fn stop(&mut self, gpu: &Gpu) {
        self.mark(gpu, 1, true);
        self.stopped = true;
    }

    fn mark(&self, gpu: &Gpu, index: u32, resolve: bool) {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("span timer"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
        if resolve {
            encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
            encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// The stopped span's GPU time in ms, waiting for it; `None` when no
    /// span was stopped since the last read.
    pub fn read(&mut self, gpu: &Gpu) -> Result<Option<f64>, GpuError> {
        if !std::mem::take(&mut self.stopped) {
            return Ok(None);
        }
        let (tx, rx) = mpsc::channel();
        self.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        // Mapped from here: unmapped on every path, or the next span's copy
        // into it fails.
        let ms = self
            .readback
            .get_mapped_range(..)
            .map(|mapped| {
                let ts: &[u64] = bytemuck::cast_slice(&mapped);
                ts[1].saturating_sub(ts[0]) as f64 * self.period_ns / 1e6
            })
            .map_err(|e| GpuError::Readback(e.to_string()));
        self.readback.unmap();
        ms.map(Some)
    }
}
