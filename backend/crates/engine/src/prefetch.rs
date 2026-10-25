//! The strip's neighbors, prefetched (LIB-09, *Prefetch*; DEV-01,
//! *Prefetched neighbors*).
//!
//! - After each switch the engine plans the two neighbors on each side, in
//!   the direction of travel: after moving right next, next + 1, previous,
//!   previous − 1; after moving left the mirror.
//! - One worker thread runs the plan, one photo at a time: it decodes the
//!   photo, has the render thread build its level 1 with its edit's white
//!   balance, waits for it, and keeps it in an LRU of 768 MB. A queued build
//!   holds its decoded input, so only one is ever queued.
//! - A photo opened from its level 1 needs its input: the worker decodes it
//!   first in the queue, and attaches it. If the last frame was
//!   provisional, it renders the photo again from level 0.
//! - A switch replaces the plan and cancels the builds of photos that are no
//!   longer neighbors. A photo already cached, or whose decode failed, is not
//!   decoded again.
//! - An entry counts as cached only while its level 1 fits the photo's edit
//!   (`PhotoInfo::level1_fits`); one that no longer does is dropped.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use genzo_cache::ByteLru;
use genzo_jobs::Generations;
use genzo_model::edit::Phase;
use genzo_pipeline::gpu::develop::{Photo, PhotoInfo, as_shot};
use genzo_pipeline::gpu::raw_stages::Level1;
use genzo_pipeline::gpu::thread::RenderThread;

use crate::EngineError;
use crate::develop::{Applied, Session};
use crate::history::PhotoEdits;
use crate::strip::{Decode, Direction};

/// The cap of the neighbors' cache (DEV-01, *Prefetched neighbors*).
const CACHE_BYTES: u64 = 768 << 20;

/// A prefetched neighbor: what `RenderThread::open_level1` takes.
#[derive(Debug)]
pub(crate) struct Cached {
    pub info: PhotoInfo,
    pub level1: Arc<Level1>,
}

/// The strip indices to prefetch around `at`, best first.
pub(crate) fn neighbors(len: usize, at: usize, direction: Direction) -> Vec<usize> {
    let next = [at + 1, at + 2].into_iter().filter(|&i| i < len);
    let previous = [at.checked_sub(1), at.checked_sub(2)].into_iter().flatten();
    match direction {
        Direction::Next => next.chain(previous).collect(),
        Direction::Previous => previous.chain(next).collect(),
    }
}

#[derive(Debug)]
enum Task {
    /// Decode the current photo, opened from its level 1, and attach it.
    Attach { id: u64, path: PathBuf },
    /// Decode a neighbor and build its level 1.
    Neighbor { id: u64, path: PathBuf },
}

impl Task {
    fn id(&self) -> u64 {
        match self {
            Self::Attach { id, .. } | Self::Neighbor { id, .. } => *id,
        }
    }
}

#[derive(Debug, Default)]
struct Plan {
    tasks: VecDeque<Task>,
    /// The current photo's neighbors, cached or not.
    wanted: Vec<u64>,
    /// The photo whose task the worker runs, and whether it is a
    /// neighbor's.
    running: Option<(u64, bool)>,
    /// Photos that did not decode: never tried again.
    failed: HashSet<u64>,
    stop: bool,
}

/// What the engine and the worker share.
#[derive(Debug)]
pub(crate) struct Shared {
    plan: Mutex<Plan>,
    /// Signalled when the plan changes, and when the worker goes idle.
    changed: Condvar,
    cache: Mutex<ByteLru<u64, Arc<Cached>>>,
    /// The worker's render from level 0 of a photo whose last frame was
    /// provisional, until the caller takes it.
    sharpened: Mutex<Option<Applied>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            plan: Mutex::default(),
            changed: Condvar::new(),
            cache: Mutex::new(ByteLru::new(CACHE_BYTES)),
            sharpened: Mutex::new(None),
        }
    }
}

/// What the worker uses of the engine.
pub(crate) struct Worker {
    pub shared: Arc<Shared>,
    pub gpu: Arc<RenderThread>,
    pub session: Arc<Mutex<Session>>,
    pub generations: Arc<Generations>,
    pub decoder: Arc<dyn Decode>,
}

impl Shared {
    /// The cached level 1 of photo `id`, if it fits `edit`. One that no
    /// longer fits is dropped.
    pub fn cached(&self, id: u64, edit: &genzo_model::edit::Edit) -> Option<Arc<Cached>> {
        let mut cache = self.cache.lock().expect("cache");
        let c = cache.get(&id)?.clone();
        if c.info.level1_fits(&c.level1, edit) {
            return Some(c);
        }
        cache.remove(&id);
        None
    }

    pub fn is_cached(&self, id: u64) -> bool {
        self.cache.lock().expect("cache").contains(&id)
    }

    /// Notes a photo that did not decode, so prefetch never tries it.
    pub fn failed(&self, id: u64) {
        self.plan.lock().expect("plan").failed.insert(id);
    }

    /// Replaces the plan: `attach` first, then each of `neighbors` (id,
    /// path, whether it is cached) not cached, failed or being run.
    /// Returns the neighbors' ids, for `keep_prefetching`.
    pub fn replan(
        &self,
        attach: Option<(u64, PathBuf)>,
        neighbors: Vec<(u64, PathBuf, bool)>,
    ) -> Vec<u64> {
        let mut plan = self.plan.lock().expect("plan");
        plan.wanted = neighbors.iter().map(|(id, ..)| *id).collect();
        plan.tasks.clear();
        if let Some((id, path)) = attach {
            plan.tasks.push_back(Task::Attach { id, path });
        }
        for (id, path, cached) in neighbors {
            // A neighbor being decoded is not queued twice; a photo whose
            // input is being attached is.
            if !cached && !plan.failed.contains(&id) && plan.running != Some((id, true)) {
                plan.tasks.push_back(Task::Neighbor { id, path });
            }
        }
        // A sharp frame of the photo before the switch is no longer wanted.
        self.sharpened.lock().expect("sharpened").take();
        self.changed.notify_all();
        plan.wanted.clone()
    }

    /// Waits until the worker has nothing left to do.
    pub fn wait_idle(&self) {
        let mut plan = self.plan.lock().expect("plan");
        while !plan.stop && (plan.running.is_some() || !plan.tasks.is_empty()) {
            plan = self.changed.wait(plan).expect("plan");
        }
    }

    /// Asks the worker to stop after its task.
    pub fn stop(&self) {
        self.plan.lock().expect("plan").stop = true;
        self.changed.notify_all();
    }

    pub fn take_sharpened(&self) -> Option<Applied> {
        self.sharpened.lock().expect("sharpened").take()
    }

    /// The next task, waiting for one; `None` once stopped.
    fn next(&self) -> Option<Task> {
        let mut plan = self.plan.lock().expect("plan");
        plan.running = None;
        self.changed.notify_all();
        loop {
            if plan.stop {
                return None;
            }
            if let Some(task) = plan.tasks.pop_front() {
                plan.running = Some((task.id(), matches!(task, Task::Neighbor { .. })));
                return Some(task);
            }
            plan = self.changed.wait(plan).expect("plan");
        }
    }

    /// Whether neighbor `id` is still wanted, after its decode.
    fn still_wanted(&self, id: u64) -> bool {
        let plan = self.plan.lock().expect("plan");
        !plan.stop && plan.wanted.contains(&id)
    }
}

impl Worker {
    /// Notes what a decode said about its file, for the strip's `kind`.
    fn learn<T>(&self, id: u64, decoded: &Result<T, EngineError>) {
        self.session
            .lock()
            .expect("session")
            .strip
            .learn(id, decoded);
    }

    pub fn run(self) {
        while let Some(task) = self.shared.next() {
            match task {
                Task::Attach { id, path } => self.attach(id, &path),
                Task::Neighbor { id, path } => self.neighbor(id, &path),
            }
        }
    }

    fn attach(&self, id: u64, path: &std::path::Path) {
        let decoded = self.decoder.decode(path);
        self.learn(id, &decoded);
        let Ok((src, _)) = decoded else {
            // Its level 1 shows; the views that need level 0 stay
            // provisional.
            self.shared.failed(id);
            return;
        };
        if self.session.lock().expect("session").open != Some(id) {
            return;
        }
        if self.gpu.attach(id, src).is_err() {
            return;
        }
        // Rendered again only when the last frame was provisional and
        // nothing newer was asked for since (a render held for the input
        // ran on its own), and no gesture is in progress.
        let again = |s: &Session| {
            s.open == Some(id)
                && s.provisional == Some(s.issued)
                && !s.edits.get(&id).is_some_and(PhotoEdits::in_gesture)
        };
        let Ok(Some(a)) = crate::develop::render_when(
            &self.gpu,
            &self.session,
            &self.generations,
            false,
            Phase::End,
            again,
        ) else {
            return;
        };
        if a.frame.as_ref().is_some_and(|f| !f.provisional) {
            // Kept only while the photo is still open: a switch clears the
            // slot after it changes the open photo.
            let s = self.session.lock().expect("session");
            if s.open == Some(id) {
                *self.shared.sharpened.lock().expect("sharpened") = Some(a);
            }
        }
    }

    fn neighbor(&self, id: u64, path: &std::path::Path) {
        let decoded = self.decoder.decode(path);
        // Not if a switch made it current meanwhile: that switch decoded it
        // again, and knows better.
        self.session
            .lock()
            .expect("session")
            .strip
            .learn_neighbor(id, &decoded);
        // A switch while it decoded may have made it current, or no longer
        // a neighbor.
        if !self.shared.still_wanted(id) {
            return;
        }
        let (src, orientation) = match decoded {
            Ok(d) => d,
            Err(_) => {
                self.shared.failed(id);
                return;
            }
        };
        let edit = {
            let mut s = self.session.lock().expect("session");
            let (_, kind) = as_shot(&src);
            let size = src.size();
            s.edits
                .entry(id)
                .or_insert_with(|| PhotoEdits::new(kind, size, orientation))
                .current()
                .clone()
        };
        let Ok(pending) = self.gpu.prefetch(id, Photo { src, orientation }, &edit) else {
            return;
        };
        // None: cancelled, or opened by a switch from the input it held.
        if let Ok(Ok(Some(p))) = pending.recv() {
            let bytes = p.level1.bytes();
            let cached = Arc::new(Cached {
                info: p.info,
                level1: Arc::new(p.level1),
            });
            let _ = self
                .shared
                .cache
                .lock()
                .expect("cache")
                .insert(id, cached, bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_follow_the_direction_of_travel() {
        assert_eq!(neighbors(9, 4, Direction::Next), [5, 6, 3, 2]);
        assert_eq!(neighbors(9, 4, Direction::Previous), [3, 2, 5, 6]);
        // At the ends, what there is.
        assert_eq!(neighbors(9, 0, Direction::Next), [1, 2]);
        assert_eq!(neighbors(9, 8, Direction::Next), [7, 6]);
        assert_eq!(neighbors(9, 1, Direction::Previous), [0, 2, 3]);
        assert_eq!(neighbors(1, 0, Direction::Next), Vec::<usize>::new());
    }
}
