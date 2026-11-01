//! The histogram and the clipping warnings in the engine (DEV-03): where
//! the counts come from, and how they leave.
//!
//! - **At Fit** each frame brings its counts, drag frames included.
//! - **At other views** a settled frame's render asks the render thread
//!   for the whole photo's, counted at the Fit size once it is idle; a
//!   thread of its own waits for the answer, so the caller never does. A
//!   pan changes nothing counted, so a render whose photo, edit, target
//!   and size are those of the last whole-photo count asks for none.
//! - **While Opening** the shown preview is counted on the CPU, on the
//!   preview pool, after the preview sink has it.
//! - Each count goes to the caller's sink ([`crate::Engine::with_histogram_sink`]),
//!   which the event sink makes `HistogramReady`, and the latest is kept
//!   for `GetHistogram`. Counts come in generation order:
//!   one older than the latest is dropped, so a late answer never
//!   replaces a newer frame's.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};

use genzo_color::output::Target;
use genzo_model::edit::Edit;
use genzo_pipeline::gpu::thread::{PendingHistogram, RenderThread};
use genzo_pipeline::reference::sampling::ViewSpec;

use crate::develop::Session;
use crate::preview::ShownPreview;

pub use genzo_pipeline::histogram::{Channels, Clipping, Histogram, Overlay, Preview};

/// One count of the shown photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counted {
    pub photo: u64,
    /// The render's generation; for an Opening preview, one taken when it
    /// was shown, so it sorts before the photo's first frame.
    pub generation: u64,
    pub histogram: Histogram,
}

/// Receives each [`Counted`], in generation order, on the thread that
/// counted it. It must not call the engine's Develop or strip methods,
/// which may be waiting to send the next one.
pub type HistogramSink = Arc<dyn Fn(Counted) + Send + Sync>;

/// What a whole-photo count was for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Key {
    pub(crate) photo: u64,
    pub(crate) edit: Edit,
    pub(crate) target: Target,
    pub(crate) view: ViewSpec,
}

impl Key {
    /// Whether `other` counts the same pixels: a pan moves the region, not
    /// the photo's Fit frame.
    fn counts_as(&self, other: &Key) -> bool {
        let out = |v: ViewSpec| match v {
            ViewSpec::Fit(out) | ViewSpec::Region { out, .. } => out,
        };
        self.photo == other.photo
            && self.edit == other.edit
            && self.target == other.target
            && out(self.view) == out(other.view)
    }
}

/// A whole-photo count on its way.
struct Waiting {
    pending: PendingHistogram,
    generation: u64,
    key: Key,
}

#[derive(Default)]
pub(crate) struct Histograms {
    sink: Option<HistogramSink>,
    /// Held while one count is checked and sent, so they leave in order.
    sending: Mutex<()>,
    latest: Mutex<Option<Counted>>,
    /// The last whole-photo count's key.
    whole: Mutex<Option<Key>>,
    waiter: Mutex<Option<Sender<Waiting>>>,
    /// Counts not sent yet: whole-photo answers waited for, and previews
    /// being counted.
    busy: Mutex<usize>,
    idle: Condvar,
}

impl std::fmt::Debug for Histograms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Histograms")
            .field("latest", &self.latest)
            .finish_non_exhaustive()
    }
}

impl Histograms {
    pub(crate) fn with_sink(sink: HistogramSink) -> Self {
        Self {
            sink: Some(sink),
            ..Default::default()
        }
    }

    /// The latest count of `photo`, if it is the latest at all.
    pub(crate) fn latest(&self, photo: u64) -> Option<Counted> {
        self.latest
            .lock()
            .expect("latest")
            .clone()
            .filter(|c| c.photo == photo)
    }

    /// Keeps `c` and sends it, unless a newer count came first.
    pub(crate) fn deliver(&self, c: Counted) {
        let _sending = self.sending.lock().expect("sending");
        {
            let mut latest = self.latest.lock().expect("latest");
            if latest
                .as_ref()
                .is_some_and(|l| l.generation >= c.generation)
            {
                return;
            }
            *latest = Some(c.clone());
        }
        if let Some(sink) = &self.sink {
            sink(c);
        }
    }

    /// Asks for the whole photo's count after the settled render `key` of
    /// `generation`, unless the last one counted the same pixels. Sent
    /// when it comes, if that render is still the last one issued.
    pub(crate) fn whole(
        self: &Arc<Self>,
        gpu: &RenderThread,
        session: &Arc<Mutex<Session>>,
        generation: u64,
        key: Key,
    ) {
        if self
            .whole
            .lock()
            .expect("whole")
            .as_ref()
            .is_some_and(|k| k.counts_as(&key))
        {
            return;
        }
        let Ok(pending) = gpu.whole_histogram(key.photo, generation) else {
            return;
        };
        let mut waiter = self.waiter.lock().expect("waiter");
        if waiter.is_none() {
            let (tx, rx) = mpsc::channel::<Waiting>();
            let this = Arc::downgrade(self);
            let session = Arc::downgrade(session);
            std::thread::Builder::new()
                .name("genzo-histogram".into())
                .spawn(move || {
                    for w in rx {
                        let answer = w.pending.recv();
                        let (Some(this), Some(session)) = (this.upgrade(), session.upgrade())
                        else {
                            return;
                        };
                        if let Ok(Ok(Some(histogram))) = answer {
                            let current = {
                                let s = session.lock().expect("session");
                                s.open == Some(w.key.photo) && s.issued == w.generation
                            };
                            if current {
                                this.deliver(Counted {
                                    photo: w.key.photo,
                                    generation: w.generation,
                                    histogram,
                                });
                                *this.whole.lock().expect("whole") = Some(w.key);
                            }
                        }
                        this.done();
                    }
                })
                .expect("the histogram thread");
            *waiter = Some(tx);
        }
        self.start();
        let sent = waiter.as_ref().is_some_and(|tx| {
            tx.send(Waiting {
                pending,
                generation,
                key,
            })
            .is_ok()
        });
        if !sent {
            self.done();
        }
    }

    /// Counts photo `photo`'s Opening preview on the preview pool, and
    /// sends it as `generation`.
    pub(crate) fn preview(self: &Arc<Self>, photo: u64, generation: u64, p: Arc<ShownPreview>) {
        self.start();
        let this = self.clone();
        crate::preview::spawn(move || {
            let histogram = genzo_pipeline::histogram::count(p.rgba.as_chunks::<4>().0);
            this.deliver(Counted {
                photo,
                generation,
                histogram,
            });
            this.done();
        });
    }

    fn start(&self) {
        *self.busy.lock().expect("busy") += 1;
    }

    fn done(&self) {
        let mut busy = self.busy.lock().expect("busy");
        *busy -= 1;
        if *busy == 0 {
            self.idle.notify_all();
        }
    }

    /// Waits until every count asked for is sent or dropped.
    pub(crate) fn wait_idle(&self) {
        let mut busy = self.busy.lock().expect("busy");
        while *busy > 0 {
            busy = self.idle.wait(busy).expect("busy");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_count_never_replaces_a_newer_one() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s = sent.clone();
        let h = Histograms::with_sink(Arc::new(move |c: Counted| {
            s.lock().unwrap().push(c.generation)
        }));
        let at = |photo, generation, pixels| Counted {
            photo,
            generation,
            histogram: Histogram {
                pixels,
                ..Histogram::default()
            },
        };
        h.deliver(at(1, 5, 10));
        h.deliver(at(1, 3, 20));
        h.deliver(at(1, 5, 30));
        assert_eq!(h.latest(1), Some(at(1, 5, 10)));
        h.deliver(at(2, 6, 40));
        assert_eq!(h.latest(1), None, "another photo's is the latest");
        assert_eq!(h.latest(2), Some(at(2, 6, 40)));
        assert_eq!(*sent.lock().unwrap(), [5, 6]);
    }
}
