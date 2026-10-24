//! The render thread: the only thread that touches the GPU device.
//!
//! Callers send it requests over a channel and wait on their own reply
//! channel, so nothing else ever blocks the render thread. Renders are
//! coalesced latest-wins: when several are queued, only the newest runs and
//! the others are answered with `None`.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use std::collections::VecDeque;

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase};

use super::develop::{Develop, DevelopError, Photo, Readback, Reran};
use super::raw_stages::TileConfig;
use super::spike::{BayerImage, Frame, SpikeParams, SpikePipeline};
use super::{AdapterChoice, Gpu, GpuError, GpuInfo};
use crate::reference::sampling::ViewSpec;

type Reply<T> = Sender<Result<T, GpuError>>;

enum Request {
    SetImage(Arc<BayerImage>, Reply<()>),
    Render {
        params: SpikeParams,
        bounds: (u32, u32),
        reply: Reply<Option<Frame>>,
    },
    /// Opens a photo in Develop, as photo `id`.
    Open {
        id: u64,
        photo: Box<Photo>,
        reply: Reply<()>,
    },
    Develop {
        request: Box<DevelopRequest>,
        reply: Reply<Option<DevelopFrame>>,
    },
}

/// One Develop render: the whole state, never a delta, so a newer request
/// replacing an older one loses nothing.
#[derive(Debug, Clone)]
pub struct DevelopRequest {
    pub generation: u64,
    /// The photo it is for; a request for another than the open one is
    /// dropped.
    pub photo: u64,
    pub edit: Edit,
    pub view: ViewSpec,
    pub target: Target,
    pub phase: Phase,
}

/// What a Develop render sends.
#[derive(Debug, Clone, PartialEq)]
pub struct DevelopFrame {
    pub generation: u64,
    pub readback: Readback,
    pub reran: Reran,
}

/// The receiving end of a submitted Develop render.
pub type Pending = Receiver<Result<Option<DevelopFrame>, GpuError>>;

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

    /// Opens `photo` in Develop as photo `id`, replacing the one open, and
    /// waits.
    pub fn open(&self, id: u64, photo: Photo) -> Result<(), GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::Open {
            id,
            photo: Box::new(photo),
            reply,
        })?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }

    /// Queues a Develop render without waiting. The answer is `None` when
    /// a newer Develop render replaced it before it ran, or it is for a
    /// photo that is no longer open.
    pub fn submit(&self, request: DevelopRequest) -> Result<Pending, GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::Develop {
            request: Box::new(request),
            reply,
        })?;
        Ok(rx)
    }

    /// [`Self::submit`] and wait.
    pub fn develop(&self, request: DevelopRequest) -> Result<Option<DevelopFrame>, GpuError> {
        self.submit(request)?
            .recv()
            .map_err(|_| GpuError::ThreadGone)?
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
    // Made on the first Develop request, so spike-only use (the benches)
    // holds no second GPU state.
    let mut develop: Option<Develop> = None;
    let mut open: Option<u64> = None;
    // Requests that arrived while the rest of a rebuild ran.
    let mut queued: VecDeque<Request> = VecDeque::new();
    loop {
        // Between requests: the rest of a white balance rebuild, stopped as
        // soon as a request comes (DEV-01, Generations). A failed rest
        // leaves both whites in the pyramid; the next render that moves
        // finishes it, and one that fails makes the pyramid again.
        if queued.is_empty()
            && let Some(d) = develop.as_mut().filter(|d| d.rebuilding())
        {
            let _ = guarded(&gpu, || {
                d.continue_rebuild(&gpu, || match rx.try_recv() {
                    Ok(r) => {
                        queued.push_back(r);
                        true
                    }
                    Err(_) => false,
                })
                .map_err(develop_error)
            });
        }
        let first = match queued.pop_front() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            },
        };
        // Drain what is queued, so stale renders can be skipped: the newest
        // of each kind runs.
        let mut batch: Vec<Request> = vec![first];
        batch.extend(queued.drain(..));
        batch.extend(rx.try_iter());
        let newest_render = batch
            .iter()
            .rposition(|r| matches!(r, Request::Render { .. }));
        let newest_develop = batch
            .iter()
            .rposition(|r| matches!(r, Request::Develop { .. }));
        for (i, req) in batch.into_iter().enumerate() {
            match req {
                Request::SetImage(image, reply) => {
                    let result = guarded(&gpu, || {
                        pipeline.set_image(&gpu, &image);
                        Ok(())
                    });
                    if result.is_err() {
                        // A half-allocated image cannot be rendered.
                        pipeline.clear_image();
                    }
                    let _ = reply.send(result);
                }
                Request::Render { reply, .. } if Some(i) != newest_render => {
                    let _ = reply.send(Ok(None));
                }
                Request::Render {
                    params,
                    bounds,
                    reply,
                } => {
                    let result = guarded(&gpu, || pipeline.render(&gpu, params, bounds));
                    if result.is_err() {
                        pipeline.forget_renders();
                    }
                    let _ = reply.send(result.map(Some));
                }
                Request::Open { id, photo, reply } => {
                    let d =
                        develop.get_or_insert_with(|| Develop::new(&gpu, TileConfig::default()));
                    let result = guarded(&gpu, || d.open(&gpu, *photo).map_err(develop_error));
                    open = Some(id);
                    let _ = reply.send(result);
                }
                Request::Develop { reply, request }
                    if Some(i) != newest_develop || open != Some(request.photo) =>
                {
                    let _ = reply.send(Ok(None));
                }
                Request::Develop { request, reply } => {
                    let Some(d) = develop.as_mut() else {
                        let _ = reply.send(Ok(None));
                        continue;
                    };
                    let r = &*request;
                    let result = guarded(&gpu, || {
                        let reran = d
                            .render(&gpu, &r.edit, r.view, r.target, r.phase)
                            .map_err(develop_error)?;
                        Ok(DevelopFrame {
                            generation: r.generation,
                            readback: d.read_back(&gpu)?,
                            reran,
                        })
                    });
                    let _ = reply.send(result.map(Some));
                }
            }
        }
    }
}

fn develop_error(e: DevelopError) -> GpuError {
    match e {
        DevelopError::Gpu(e) => e,
        other => GpuError::Request(other.to_string()),
    }
}

/// Runs `f` with the device's errors captured instead of left to wgpu's
/// default handler, which panics and would stop the render thread for good.
/// An out-of-memory error wins over the validation errors that follow from
/// using the resource that failed to allocate.
fn guarded<T>(gpu: &Gpu, f: impl FnOnce() -> Result<T, GpuError>) -> Result<T, GpuError> {
    let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = gpu.device.push_error_scope(wgpu::ErrorFilter::Internal);
    let oom = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let result = f();
    let oom = pollster::block_on(oom.pop());
    let internal = pollster::block_on(internal.pop());
    let validation = pollster::block_on(validation.pop());
    if oom.is_some() {
        return Err(GpuError::OutOfMemory);
    }
    if let Some(e) = internal.or(validation) {
        let first_line = e
            .to_string()
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default()
            .to_owned();
        return Err(GpuError::Request(first_line));
    }
    result
}
