//! The settled frame of a gesture at rest (DEV-01, *Interactive and
//! settled*): when a drag stops moving but is not released, the photo
//! renders settled once the input has rested for [`REST`], so a slider held
//! still shows full quality and not the drag's preview. The gesture goes
//! on: nothing is recorded, the Alt preview stays, and the next move
//! renders interactive again.
//!
//! One thread waits. Each interactive frame restarts its wait; the render
//! runs only if that frame is still the last one issued and its gesture is
//! still in progress, checked as the generation is taken.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use genzo_jobs::Generations;
use genzo_pipeline::gpu::thread::RenderThread;

use crate::develop::{Session, To};
use crate::histogram::Histograms;
use crate::{EventSink, FrameStore};

/// How long a gesture's input rests before its frame settles: the spec's
/// "about 150 ms after input stops".
pub const REST: Duration = Duration::from_millis(150);

/// What the waiting thread renders with.
pub(crate) struct Parts {
    pub gpu: Arc<RenderThread>,
    pub session: Arc<Mutex<Session>>,
    pub generations: Arc<Generations>,
    pub histograms: Arc<Histograms>,
    pub frames: Arc<Mutex<FrameStore>>,
    pub events: Option<EventSink>,
}

/// The waiting thread, started with the first interactive frame.
#[derive(Debug, Default)]
pub(crate) struct Settler {
    thread: Mutex<Option<(Sender<u64>, JoinHandle<()>)>>,
}

impl Settler {
    /// Interactive frame `generation` was just stored: it settles after
    /// `rest` unless a newer frame restarts the wait. `parts` is asked for
    /// once, when the thread starts.
    pub(crate) fn rested(&self, generation: u64, rest: Duration, parts: impl FnOnce() -> Parts) {
        let mut thread = self.thread.lock().expect("settler");
        if thread.is_none() {
            let (tx, rx) = mpsc::channel::<u64>();
            let parts = parts();
            let handle = std::thread::Builder::new()
                .name("genzo-settle".into())
                .spawn(move || {
                    // The newest frame told of, and when it settles.
                    let mut waiting: Option<(u64, Instant)> = None;
                    loop {
                        let next = match waiting {
                            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                            Some((_, at)) => {
                                rx.recv_timeout(at.saturating_duration_since(Instant::now()))
                            }
                        };
                        match next {
                            // Two renders can tell of their frames out of
                            // order: an older one neither replaces the
                            // newer nor restarts its wait.
                            Ok(generation) => {
                                if waiting.is_none_or(|(newest, _)| generation > newest) {
                                    waiting = Some((generation, Instant::now() + rest));
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                if let Some((generation, _)) = waiting.take() {
                                    parts.settle(generation);
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                });
            // Without the thread a gesture settles on its release, as before.
            let Ok(handle) = handle else { return };
            *thread = Some((tx, handle));
        }
        if let Some((tx, _)) = thread.as_ref() {
            let _ = tx.send(generation);
        }
    }

    /// Stops the thread and waits for it: it holds the render thread.
    pub(crate) fn stop(&self) {
        let thread = self.thread.lock().expect("settler").take();
        if let Some((tx, handle)) = thread {
            drop(tx);
            let _ = handle.join();
        }
    }
}

impl Parts {
    fn settle(&self, generation: u64) {
        let to = To {
            gpu: &self.gpu,
            session: &self.session,
            generations: &self.generations,
            histograms: &self.histograms,
            frames: &self.frames,
            events: self.events.as_ref(),
        };
        // A render that fails here fails again, and is said, at the release.
        let _ = crate::develop::render_at_rest(&to, generation);
    }
}
