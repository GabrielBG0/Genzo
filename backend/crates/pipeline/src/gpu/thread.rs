//! The render thread: the only thread that touches the GPU device.
//!
//! Callers send it requests over a channel and wait on their own reply
//! channel, so nothing else ever blocks the render thread. Renders are
//! coalesced latest-wins: when several are queued, only the newest runs and
//! the others are answered with `None`.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use super::spike::{BayerImage, Frame, SpikeParams, SpikePipeline};
use super::{AdapterChoice, Gpu, GpuError, GpuInfo};

type Reply<T> = Sender<Result<T, GpuError>>;

enum Request {
    SetImage(Arc<BayerImage>, Reply<()>),
    Render {
        params: SpikeParams,
        bounds: (u32, u32),
        reply: Reply<Option<Frame>>,
    },
}

/// A handle to the render thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct RenderThread {
    tx: Option<Sender<Request>>,
    info: GpuInfo,
    handle: Option<JoinHandle<()>>,
}

impl RenderThread {
    /// Starts the thread, opens the device on it and waits for the result.
    pub fn spawn(choice: AdapterChoice) -> Result<Self, GpuError> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("genzo-render".into())
            .spawn(move || match Gpu::open(choice) {
                Ok(gpu) => {
                    let _ = ready_tx.send(Ok(gpu.info.clone()));
                    run(gpu, rx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| GpuError::Device(e.to_string()))?;
        let info = ready_rx.recv().map_err(|_| GpuError::ThreadGone)??;
        Ok(Self {
            tx: Some(tx),
            info,
            handle: Some(handle),
        })
    }

    pub fn info(&self) -> &GpuInfo {
        &self.info
    }

    fn send(&self, req: Request) -> Result<(), GpuError> {
        self.tx
            .as_ref()
            .ok_or(GpuError::ThreadGone)?
            .send(req)
            .map_err(|_| GpuError::ThreadGone)
    }

    /// Uploads an image. Later renders use it.
    pub fn set_image(&self, image: Arc<BayerImage>) -> Result<(), GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::SetImage(image, reply))?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }

    /// Renders and waits for the frame. `None` means a newer render request
    /// replaced this one before it ran.
    pub fn render(
        &self,
        params: SpikeParams,
        bounds: (u32, u32),
    ) -> Result<Option<Frame>, GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::Render {
            params,
            bounds,
            reply,
        })?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop.
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(gpu: Gpu, rx: Receiver<Request>) {
    let mut pipeline = SpikePipeline::new(&gpu);
    while let Ok(first) = rx.recv() {
        // Drain what is queued, so stale renders can be skipped.
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let newest_render = batch
            .iter()
            .rposition(|r| matches!(r, Request::Render { .. }));
        for (i, req) in batch.into_iter().enumerate() {
            match req {
                Request::SetImage(image, reply) => {
                    pipeline.set_image(&gpu, &image);
                    let _ = reply.send(Ok(()));
                }
                Request::Render { reply, .. } if Some(i) != newest_render => {
                    let _ = reply.send(Ok(None));
                }
                Request::Render {
                    params,
                    bounds,
                    reply,
                } => {
                    let _ = reply.send(pipeline.render(&gpu, params, bounds).map(Some));
                }
            }
        }
    }
}
