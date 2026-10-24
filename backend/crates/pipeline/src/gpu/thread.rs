//! The render thread: the only thread that touches the GPU device.
//!
//! Callers send it requests over a channel and wait on their own reply
//! channel, so nothing else ever blocks the render thread. Renders are
//! coalesced latest-wins: when several are queued, only the newest runs and
//! the others are answered with `None`.
//!
//! A photo opened from its prefetched level 1 has no input until
//! [`RenderThread::attach`] brings it (DEV-01, *Prefetched neighbors*). A
//! Develop render that needs it is held until then, and still answered
//! `None` if a newer one replaces it first.
//!
//! **Prefetch** (DEV-01, *Prefetched neighbors*): neighbors' level 1 are
//! built in the order they were queued, one at a time, only while no
//! request waits and after the rest of the open photo's rebuild. A build
//! stops between tiles for any request and resumes after it. Each holds its
//! photo's input until it is done or cancelled.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use std::collections::VecDeque;

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase};

use super::develop::{Develop, DevelopError, Photo, PhotoInfo, Readback, Reran};
use super::raw_stages::{Level1, Level1Build, TileConfig};
use super::spike::{BayerImage, Frame, SpikeParams, SpikePipeline};
use super::{AdapterChoice, Gpu, GpuError, GpuInfo};
use crate::camera_color::CameraTransform;
use crate::raw::RawSource;
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
    /// Opens a photo from its prefetched level 1, as photo `id`.
    OpenLevel1 {
        id: u64,
        info: Box<PhotoInfo>,
        level1: Arc<Level1>,
        reply: Reply<()>,
    },
    /// The input of photo `id`, opened from its level 1.
    Attach {
        id: u64,
        src: Box<RawSource>,
        reply: Reply<()>,
    },
    Develop {
        request: Box<DevelopRequest>,
        reply: Reply<Option<DevelopFrame>>,
    },
    /// Queues photo `id`'s level 1.
    Prefetch(Box<Queued>),
    /// Cancels the queued level 1 of every photo not in `keep`.
    KeepPrefetching {
        keep: Vec<u64>,
        reply: Reply<()>,
    },
    /// Opens photo `id` from its queued prefetch's input.
    OpenPrefetching {
        id: u64,
        reply: Reply<bool>,
    },
}

/// A neighbor's level 1, queued or being built.
struct Queued {
    id: u64,
    photo: Photo,
    transform: CameraTransform,
    reply: Reply<Option<Prefetched>>,
    /// Started on its first turn.
    build: Option<Level1Build>,
}

/// A prefetched neighbor: what [`RenderThread::open_level1`] takes.
#[derive(Debug)]
pub struct Prefetched {
    pub id: u64,
    pub info: PhotoInfo,
    pub level1: Level1,
}

/// The receiving end of a queued prefetch: `None` when it was cancelled,
/// or its photo opened from its input instead.
pub type PendingLevel1 = Receiver<Result<Option<Prefetched>, GpuError>>;

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
    /// Drawn from level 1 where level 0 was wanted, the photo's input not
    /// attached yet ([`Develop::provisional`]): the caller renders it again
    /// after [`RenderThread::attach`].
    pub provisional: bool,
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

    /// Opens a photo from its prefetched `level1` as photo `id`, replacing
    /// the one open, and waits. Renders that need its input wait for
    /// [`Self::attach`].
    pub fn open_level1(
        &self,
        id: u64,
        info: PhotoInfo,
        level1: Arc<Level1>,
    ) -> Result<(), GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::OpenLevel1 {
            id,
            info: Box::new(info),
            level1,
            reply,
        })?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }

    /// Gives photo `id`, opened from its level 1, its decoded input, and
    /// waits. A held render then runs. Nothing happens when another photo
    /// is open by then; an input of another size is refused, and the photo
    /// keeps waiting.
    pub fn attach(&self, id: u64, src: RawSource) -> Result<(), GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::Attach {
            id,
            src: Box::new(src),
            reply,
        })?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }

    /// Queues photo `id`'s level 1 with `transform` (its edit's white
    /// balance), without waiting. Builds run in the order they were queued.
    pub fn prefetch(
        &self,
        id: u64,
        photo: Photo,
        transform: CameraTransform,
    ) -> Result<PendingLevel1, GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::Prefetch(Box::new(Queued {
            id,
            photo,
            transform,
            reply,
            build: None,
        })))?;
        Ok(rx)
    }

    /// Cancels the prefetch of every photo not in `keep`, between tiles,
    /// and waits. Each is answered `None`.
    pub fn keep_prefetching(&self, keep: &[u64]) -> Result<(), GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::KeepPrefetching {
            keep: keep.to_vec(),
            reply,
        })?;
        rx.recv().map_err(|_| GpuError::ThreadGone)?
    }

    /// Opens photo `id` in Develop from the input its queued prefetch holds,
    /// so a switch to a neighbor being built does not decode it again, and
    /// waits. The prefetch is answered `None`. False when none is queued:
    /// the caller opens it another way.
    pub fn open_prefetching(&self, id: u64) -> Result<bool, GpuError> {
        let (reply, rx) = mpsc::channel();
        self.send(Request::OpenPrefetching { id, reply })?;
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
    // The newest Develop render, held until the open photo's input comes.
    let mut held: Option<Held> = None;
    // Neighbors' level 1, first in first built.
    let mut prefetch: VecDeque<Box<Queued>> = VecDeque::new();
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
        // Then a neighbor's level 1, stopped the same way.
        if queued.is_empty()
            && let Some(q) = prefetch.front_mut()
        {
            let d = develop.get_or_insert_with(|| Develop::new(&gpu, TileConfig::default()));
            let built = guarded(&gpu, || {
                let src = &q.photo.src;
                let build = match &mut q.build {
                    Some(b) => b,
                    None => q.build.insert(d.start_level1(&gpu, src, &q.transform)),
                };
                d.continue_level1(&gpu, src, build, || match rx.try_recv() {
                    Ok(r) => {
                        queued.push_back(r);
                        true
                    }
                    Err(_) => false,
                })
            });
            if !matches!(built, Ok(false)) {
                let Queued {
                    id,
                    photo,
                    reply,
                    build,
                    ..
                } = *prefetch.pop_front().expect("front");
                // After an error the build is dropped: its buffer may still
                // be mapped.
                let answer = built.map(|_| {
                    Some(Prefetched {
                        id,
                        info: PhotoInfo::of(&photo),
                        level1: build.expect("started").finish(),
                    })
                });
                let _ = reply.send(answer);
            }
            if queued.is_empty() {
                continue;
            }
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
                    drop_held(&mut held);
                    let d =
                        develop.get_or_insert_with(|| Develop::new(&gpu, TileConfig::default()));
                    let result = guarded(&gpu, || d.open(&gpu, *photo).map_err(develop_error));
                    open = Some(id);
                    let _ = reply.send(result);
                }
                Request::OpenLevel1 {
                    id,
                    info,
                    level1,
                    reply,
                } => {
                    drop_held(&mut held);
                    let d =
                        develop.get_or_insert_with(|| Develop::new(&gpu, TileConfig::default()));
                    let result = guarded(&gpu, || {
                        d.open_level1(&gpu, *info, &level1).map_err(develop_error)
                    });
                    open = Some(id);
                    let _ = reply.send(result);
                }
                Request::Attach { id, src, reply } => {
                    let mut result = Ok(());
                    if let Some(d) = develop.as_mut().filter(|_| open == Some(id)) {
                        result = d.attach(*src).map_err(develop_error);
                        if result.is_ok()
                            && let Some(h) = held.take()
                        {
                            held = develop_render(&gpu, d, h);
                        }
                    }
                    let _ = reply.send(result);
                }
                Request::Develop { reply, request }
                    if Some(i) != newest_develop || open != Some(request.photo) =>
                {
                    let _ = reply.send(Ok(None));
                }
                Request::Prefetch(q) => prefetch.push_back(q),
                Request::KeepPrefetching { keep, reply } => {
                    let (kept, gone) = prefetch.drain(..).partition(|q| keep.contains(&q.id));
                    prefetch = kept;
                    for q in gone {
                        let _ = q.reply.send(Ok(None));
                    }
                    let _ = reply.send(Ok(()));
                }
                Request::OpenPrefetching { id, reply } => {
                    let Some(i) = prefetch.iter().position(|q| q.id == id) else {
                        let _ = reply.send(Ok(false));
                        continue;
                    };
                    let q = prefetch.remove(i).expect("found");
                    let _ = q.reply.send(Ok(None));
                    drop_held(&mut held);
                    let d =
                        develop.get_or_insert_with(|| Develop::new(&gpu, TileConfig::default()));
                    let result = guarded(&gpu, || d.open(&gpu, q.photo).map_err(develop_error));
                    open = Some(id);
                    let _ = reply.send(result.map(|()| true));
                }
                Request::Develop { request, reply } => {
                    drop_held(&mut held);
                    let Some(d) = develop.as_mut() else {
                        let _ = reply.send(Ok(None));
                        continue;
                    };
                    held = develop_render(&gpu, d, Held { request, reply });
                }
            }
        }
    }
}

/// A Develop render waiting for the open photo's input.
struct Held {
    request: Box<DevelopRequest>,
    reply: Reply<Option<DevelopFrame>>,
}

/// A held render that a newer request replaced: answered `None`.
fn drop_held(held: &mut Option<Held>) {
    if let Some(h) = held.take() {
        let _ = h.reply.send(Ok(None));
    }
}

/// Runs a Develop render and answers it, or gives it back to be held when
/// it needs the photo's input.
fn develop_render(gpu: &Gpu, d: &mut Develop, h: Held) -> Option<Held> {
    let r = &*h.request;
    let result = guarded(gpu, || {
        let reran = match d.render(gpu, &r.edit, r.view, r.target, r.phase) {
            Err(DevelopError::NeedsSource) => return Ok(None),
            reran => reran.map_err(develop_error)?,
        };
        Ok(Some(DevelopFrame {
            generation: r.generation,
            readback: d.read_back(gpu)?,
            reran,
            provisional: d.provisional(),
        }))
    });
    match result {
        Ok(None) => Some(h),
        result => {
            let _ = h.reply.send(result);
            None
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
