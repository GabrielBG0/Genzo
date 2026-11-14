//! A gesture's renders in flight (DEV-01, *Re-runs, quality levels and
//! generations*): a gesture's *start* or *update* sent as a command answers
//! with the edit at once, and its frame follows by `RenderFinished`. The
//! command no longer waits for the render, the readback and the encoding,
//! so the next step can be sent while this one's frame is made; of the
//! steps that wait on the render thread together, only the newest renders.
//!
//! One thread takes each render's answer, in the order they were asked
//! for, and does what the command did before: keeps the frame, sends its
//! event and its counts, and tells the settler of it. A render that fails
//! here says nothing: the gesture's *end*, which waits for its frame, fails
//! the same way and is said.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use genzo_pipeline::gpu::thread::Pending;

use crate::develop::{Issued, To, finish};
use crate::settle::{Parts, Settler};

/// The most renders in flight. Each answer holds its frame's readback until
/// the thread takes it; past this many, the next step waits for one to be
/// finished, as every step did before, so a thread that falls behind holds
/// a few frames and never a drag's worth. The steps a newer one replaced
/// answer at once and do not count for long.
const MOST: usize = 4;

/// A render asked for and not answered yet.
struct Flying {
    pending: Pending,
    issued: Issued,
}

/// What the thread finishes a frame with.
pub(crate) struct Finisher {
    pub parts: Parts,
    /// How long a gesture's input rests before its frame settles, if it
    /// does, and what renders that frame.
    pub rest: Option<Duration>,
    pub settler: Arc<Settler>,
}

/// The waiting thread, started with the first render in flight.
#[derive(Debug, Default)]
pub(crate) struct InFlight {
    thread: Mutex<Option<(Sender<Flying>, JoinHandle<()>)>>,
    /// Renders in flight, and the signal that one is finished.
    busy: Arc<(Mutex<usize>, Condvar)>,
}

impl InFlight {
    /// Takes the render `pending` of `issued`: its frame is finished when
    /// it comes. `finisher` is asked for once, when the thread starts. An
    /// error is the thread that could not start; nothing was taken.
    pub(crate) fn take(
        &self,
        pending: Pending,
        issued: Issued,
        finisher: impl FnOnce() -> Finisher,
    ) -> std::io::Result<()> {
        let mut thread = self.thread.lock().expect("in flight");
        if thread.is_none() {
            let (tx, rx) = mpsc::channel::<Flying>();
            let finisher = finisher();
            let busy = self.busy.clone();
            let handle = std::thread::Builder::new()
                .name("genzo-frames".into())
                .spawn(move || {
                    for flying in rx {
                        finisher.finish(flying);
                        let (count, finished) = &*busy;
                        *count.lock().expect("busy") -= 1;
                        finished.notify_all();
                    }
                })?;
            *thread = Some((tx, handle));
        }
        let (tx, _) = thread.as_ref().expect("started above");
        {
            let (count, finished) = &*self.busy;
            let mut count = count.lock().expect("busy");
            while *count >= MOST {
                count = finished.wait(count).expect("busy");
            }
            *count += 1;
        }
        // The thread only ends with its sender, which is held here.
        let _ = tx.send(Flying { pending, issued });
        Ok(())
    }

    /// Waits until every render taken is finished or dropped.
    pub(crate) fn wait_idle(&self) {
        let (count, finished) = &*self.busy;
        let mut count = count.lock().expect("busy");
        while *count > 0 {
            count = finished.wait(count).expect("busy");
        }
    }

    /// Stops the thread and waits for it: it holds the render thread, which
    /// must still answer what is in flight.
    pub(crate) fn stop(&self) {
        let thread = self.thread.lock().expect("in flight").take();
        if let Some((tx, handle)) = thread {
            drop(tx);
            let _ = handle.join();
        }
    }
}

impl Finisher {
    fn finish(&self, flying: Flying) {
        // `None`: a newer render replaced it, or its photo is gone.
        let Ok(Ok(Some(frame))) = flying.pending.recv() else {
            return;
        };
        let p = &self.parts;
        let to = To {
            gpu: &p.gpu,
            session: &p.session,
            generations: &p.generations,
            histograms: &p.histograms,
            frames: &p.frames,
            events: p.events.as_ref(),
        };
        let generation = flying.issued.generation();
        finish(&to, flying.issued, Some(&frame));
        if let Some(rest) = self.rest {
            self.settler.rested(generation, rest, || p.clone());
        }
    }
}
