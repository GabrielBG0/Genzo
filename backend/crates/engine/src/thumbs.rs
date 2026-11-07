//! The strip's thumbnails (LIB-09, *The strip*): each photo's embedded
//! preview, small, upright, in the frame's target, as one JPEG tile message.
//!
//! - **Made from the embedded preview**, never a render: the smallest
//!   stream whose long edge is at least [`EDGE`] (a camera's small preview,
//!   not its full-size one), reduced to fit in [`EDGE`] × [`EDGE`]. A file
//!   without one has no thumbnail: its cell shows the file name.
//! - **In the order asked**: the frontend names the visible cells first,
//!   then the rest. A new request replaces the order; one made already is
//!   answered at once.
//! - **Two tiers**: an LRU in memory ([`MEMORY_BYTES`]), and files in the
//!   cache directory, if the engine has one, so a folder opened again in a
//!   later session reads no photo. A disk entry is keyed by the file's
//!   canonical path, length and modification time and by the target, so a
//!   changed file is made again; the key is stored in the entry too, so a
//!   hash collision is a miss, never the wrong picture.
//! - **Behind the photo on screen**: [`WORKERS`] threads, each with a rayon
//!   pool of one thread, so they never queue work on the global pool where
//!   a decode runs. They start no thumbnail while a photo switch is in
//!   progress, nor while prefetch has work.

use std::collections::{HashSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use genzo_cache::ByteLru;
use genzo_color::output::Target;
use genzo_model::frame::{Rect, TileHeader};

use crate::preview::{convert, reduce};

/// The long edge a thumbnail fits in, in pixels: a 108 × 70 cell at a
/// device pixel ratio of 2 fills from 216 px, and a portrait one's width
/// from 320 px high.
pub const EDGE: u32 = 320;

/// JPEG quality of a thumbnail.
const QUALITY: u8 = 85;

/// The memory tier: about 1000 thumbnails of 30 KB.
pub(crate) const MEMORY_BYTES: u64 = 32 << 20;

/// What a "no thumbnail" entry counts in the memory tier.
const NONE_BYTES: u64 = 64;

/// Threads making thumbnails.
const WORKERS: usize = 2;

/// How often a worker waiting for prefetch looks again.
const PREFETCH_POLL: Duration = Duration::from_millis(20);

/// The disk entries' layout version, in their directory's name: a new
/// layout, size or encoding starts a new directory.
const DISK_DIR: &str = "thumbs-v1";

/// The start of each disk entry.
const DISK_MAGIC: &[u8; 4] = b"GZTH";

/// Receives each made thumbnail: the photo, and whether it has one.
pub type ThumbSink = Arc<dyn Fn(u64, bool) + Send + Sync>;

/// A made thumbnail: its tile message, or `None` when the file has none.
#[derive(Debug, Clone)]
struct Made {
    target: Target,
    message: Option<Arc<Vec<u8>>>,
}

#[derive(Debug)]
struct State {
    queue: VecDeque<(u64, PathBuf)>,
    target: Target,
    /// Photos being made, so a request does not queue them twice.
    running: HashSet<u64>,
    /// Photo switches in progress.
    paused: usize,
    started: bool,
    stop: bool,
}

/// The thumbnails' queue, tiers and workers.
pub(crate) struct Thumbs {
    state: Mutex<State>,
    changed: Condvar,
    made: Mutex<ByteLru<u64, Made>>,
    dir: Option<PathBuf>,
    sink: Mutex<Option<ThumbSink>>,
    /// Whether prefetch has work, which goes first.
    busy: Box<dyn Fn() -> bool + Send + Sync>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbs").field("dir", &self.dir).finish()
    }
}

/// Held while a photo switch runs: no thumbnail starts meanwhile.
pub(crate) struct Paused<'a>(&'a Thumbs);

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.0.state.lock().expect("thumbs").paused -= 1;
        self.0.changed.notify_all();
    }
}

impl Thumbs {
    /// Thumbnails kept on disk under `dir`, if given, made only while
    /// `busy` (prefetch's) says false.
    pub(crate) fn new(dir: Option<PathBuf>, busy: Box<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                target: Target::Srgb,
                running: HashSet::new(),
                paused: 0,
                started: false,
                stop: false,
            }),
            changed: Condvar::new(),
            made: Mutex::new(ByteLru::new(MEMORY_BYTES)),
            dir: dir.map(|d| d.join(DISK_DIR)),
            sink: Mutex::new(None),
            busy,
            workers: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn set_sink(&self, sink: ThumbSink) {
        *self.sink.lock().expect("thumb sink") = Some(sink);
    }

    fn send(&self, photo: u64, found: bool) {
        let sink = self.sink.lock().expect("thumb sink").clone();
        if let Some(sink) = sink {
            sink(photo, found);
        }
    }

    /// No thumbnail starts until the guard drops.
    pub(crate) fn pause(&self) -> Paused<'_> {
        self.state.lock().expect("thumbs").paused += 1;
        Paused(self)
    }

    /// Makes `photos`' thumbnails for `target`, in this order, replacing
    /// the order asked before. Those made already are sent at once.
    pub(crate) fn request(self: &Arc<Self>, photos: Vec<(u64, PathBuf)>, target: Target) {
        let mut ready = Vec::new();
        {
            let mut made = self.made.lock().expect("thumbs made");
            let mut state = self.state.lock().expect("thumbs");
            state.target = target;
            state.queue.clear();
            let mut seen = HashSet::new();
            for (id, path) in photos {
                if !seen.insert(id) || state.running.contains(&id) {
                    continue;
                }
                match made.get(&id) {
                    Some(m) if m.target == target => ready.push((id, m.message.is_some())),
                    _ => state.queue.push_back((id, path)),
                }
            }
            if !state.queue.is_empty() && !state.started {
                state.started = true;
                self.start();
            }
        }
        self.changed.notify_all();
        for (id, found) in ready {
            self.send(id, found);
        }
    }

    fn start(self: &Arc<Self>) {
        let mut workers = self.workers.lock().expect("thumb workers");
        for i in 0..WORKERS {
            let me = Arc::clone(self);
            let spawned = std::thread::Builder::new()
                .name(format!("genzo-thumb-{i}"))
                .spawn(move || me.run());
            if let Ok(handle) = spawned {
                workers.push(handle);
            }
        }
    }

    /// The next photo to make, once nothing goes before it; `None` to stop.
    fn next(&self) -> Option<(u64, PathBuf, Target)> {
        let mut state = self.state.lock().expect("thumbs");
        loop {
            if state.stop {
                return None;
            }
            if state.paused == 0 && !state.queue.is_empty() {
                if !(self.busy)() {
                    let (id, path) = state.queue.pop_front().expect("not empty");
                    state.running.insert(id);
                    return Some((id, path, state.target));
                }
                // Prefetch does not signal here: look again soon.
                state = self
                    .changed
                    .wait_timeout(state, PREFETCH_POLL)
                    .expect("thumbs")
                    .0;
            } else {
                state = self.changed.wait(state).expect("thumbs");
            }
        }
    }

    fn run(self: Arc<Self>) {
        let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(1).build() else {
            return;
        };
        while let Some((id, path, target)) = self.next() {
            let message = pool.install(|| self.make(&path, target));
            let _evicted = self.made.lock().expect("thumbs made").insert(
                id,
                Made {
                    target,
                    message: message.clone(),
                },
                message.as_ref().map_or(NONE_BYTES, |m| m.len() as u64),
            );
            self.state.lock().expect("thumbs").running.remove(&id);
            self.changed.notify_all();
            self.send(id, message.is_some());
        }
    }

    /// Photo `id`'s thumbnail message, made for `target`: from memory, else
    /// from disk if `path` is given. `None` when it was not made, or the
    /// file has none.
    pub(crate) fn get(&self, id: u64, target: Target, path: Option<&Path>) -> Option<Arc<Vec<u8>>> {
        if let Some(m) = self.made.lock().expect("thumbs made").get(&id)
            && m.target == target
        {
            return m.message.clone();
        }
        let key = disk_key(path?, target)?;
        self.read_disk(&key).flatten().map(Arc::new)
    }

    /// The thumbnail of the file at `path` for `target`: from disk, else
    /// made and written there.
    fn make(&self, path: &Path, target: Target) -> Option<Arc<Vec<u8>>> {
        let key = disk_key(path, target);
        if let Some(found) = key.as_deref().and_then(|k| self.read_disk(k)) {
            return found.map(Arc::new);
        }
        let message = make(path, target);
        // A file that could not be read is tried again next session.
        if let (Some(key), Some(message)) = (&key, &message) {
            self.write_disk(key, message.as_ref());
        }
        message.flatten().map(Arc::new)
    }

    fn entry(&self, key: &str) -> Option<PathBuf> {
        Some(
            self.dir
                .as_ref()?
                .join(format!("{:016x}.gzth", fnv1a(key.as_bytes()))),
        )
    }

    /// `Some(None)` for a file stored as having no thumbnail.
    fn read_disk(&self, key: &str) -> Option<Option<Vec<u8>>> {
        let bytes = std::fs::read(self.entry(key)?).ok()?;
        let rest = bytes.strip_prefix(DISK_MAGIC)?;
        let len = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let stored = rest.get(4..4 + len)?;
        if stored != key.as_bytes() {
            return None;
        }
        let message = &rest[4 + len..];
        Some((!message.is_empty()).then(|| message.to_vec()))
    }

    /// Written to a file of its own, then renamed, so a reader never sees
    /// half an entry. A failure leaves the thumbnail in memory only.
    fn write_disk(&self, key: &str, message: Option<&Vec<u8>>) {
        let Some(entry) = self.entry(key) else {
            return;
        };
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(entry.parent().expect("in the cache directory"))?;
            let tmp = entry.with_extension(format!("tmp{}", std::process::id()));
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(DISK_MAGIC)?;
            f.write_all(&(key.len() as u32).to_le_bytes())?;
            f.write_all(key.as_bytes())?;
            f.write_all(message.map_or(&[][..], |m| m.as_slice()))?;
            drop(f);
            std::fs::rename(&tmp, &entry).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        };
        let _ = write();
    }

    /// Drops the memory tier.
    pub(crate) fn forget(&self) {
        *self.made.lock().expect("thumbs made") = ByteLru::new(MEMORY_BYTES);
    }

    /// Stops the workers after the thumbnail each is making.
    pub(crate) fn stop(&self) {
        self.state.lock().expect("thumbs").stop = true;
        self.changed.notify_all();
        for handle in self.workers.lock().expect("thumb workers").drain(..) {
            let _ = handle.join();
        }
    }

    /// Waits until every requested thumbnail is made: for the tests and
    /// `cli bench`.
    pub(crate) fn wait_idle(&self) {
        let mut state = self.state.lock().expect("thumbs");
        while !state.stop && (!state.queue.is_empty() || !state.running.is_empty()) {
            state = self
                .changed
                .wait_timeout(state, PREFETCH_POLL)
                .expect("thumbs")
                .0;
        }
    }
}

/// The disk key of the file at `path` for `target`; `None` when the file
/// cannot be looked at.
fn disk_key(path: &Path, target: Target) -> Option<String> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&canonical).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!(
        "{}\n{}\n{modified}\n{target:?}\n{EDGE}",
        canonical.display(),
        meta.len()
    ))
}

/// 64-bit FNV-1a: a stable name for a disk entry. The entry holds its key,
/// so a collision is a miss.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Makes the thumbnail of the file at `path`: `None` when the file cannot
/// be read (tried again later), `Some(None)` when it has no preview that
/// shows.
fn make(path: &Path, target: Target) -> Option<Option<Vec<u8>>> {
    let data = std::fs::read(path).ok()?;
    let Ok(Some(preview)) = genzo_imgio::decode_preview_near(&data, EDGE) else {
        return Some(None);
    };
    drop(data);
    Some(thumbnail(&preview, target))
}

/// `preview` as a thumbnail's tile message.
pub(crate) fn thumbnail(preview: &genzo_imgio::Preview, target: Target) -> Option<Vec<u8>> {
    let shown = convert(&reduce(preview, (EDGE, EDGE))?, target)?;
    let jpeg =
        genzo_imgio::jpeg::encode_rgba(&shown.rgba, shown.width, shown.height, QUALITY).ok()?;
    let header = TileHeader {
        generation: 0,
        rect: Rect {
            x: 0,
            y: 0,
            width: shown.width,
            height: shown.height,
        },
        mip_level: 0,
        format: crate::frames::format(target).as_jpeg(),
        image_width: shown.width,
        image_height: shown.height,
    };
    Some(header.encode(&jpeg))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// A JPEG file whose EXIF segment holds a `w` × `h` preview of one
    /// color; its own image is 64 × 48 gray.
    pub(crate) fn jpeg_with_preview(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        jpeg_with_previews(&[(w, h, rgb)])
    }

    fn solid(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
        let px: Vec<u8> = (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
        genzo_imgio::jpeg::encode_rgba(&px, w, h, 90).unwrap()
    }

    /// As [`jpeg_with_preview`], with several previews in its EXIF segment.
    fn jpeg_with_previews(previews: &[(u32, u32, [u8; 3])]) -> Vec<u8> {
        let mut app1 = b"Exif\0\0II*\0\x08\0\0\0\0\0".to_vec();
        for &(w, h, rgb) in previews {
            app1.extend_from_slice(&solid(w, h, rgb));
        }
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&solid(64, 48, [128; 3])[2..]);
        out
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("genzo-thumbs-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, data: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    type Got = Arc<Mutex<Vec<(u64, bool)>>>;

    fn thumbs(dir: Option<PathBuf>, busy: &Arc<AtomicBool>) -> (Arc<Thumbs>, Got) {
        let busy = Arc::clone(busy);
        let t = Arc::new(Thumbs::new(
            dir,
            Box::new(move || busy.load(Ordering::SeqCst)),
        ));
        let got: Got = Arc::default();
        let g = Arc::clone(&got);
        t.set_sink(Arc::new(move |id, found| {
            g.lock().unwrap().push((id, found))
        }));
        (t, got)
    }

    fn queued(t: &Thumbs) -> Vec<u64> {
        t.state
            .lock()
            .unwrap()
            .queue
            .iter()
            .map(|(id, _)| *id)
            .collect()
    }

    fn sorted(got: &Got) -> Vec<(u64, bool)> {
        let mut v = got.lock().unwrap().clone();
        v.sort_unstable();
        v
    }

    #[test]
    fn fnv1a_is_the_published_function() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn the_queue_takes_the_order_asked_and_a_new_request_replaces_it() {
        let dir = Dir::new("order");
        let f: Vec<PathBuf> = (0..5)
            .map(|i| {
                dir.file(
                    &format!("{i}.jpg"),
                    &jpeg_with_preview(400, 300, [i * 40; 3]),
                )
            })
            .collect();
        let (t, got) = thumbs(None, &Arc::default());
        let paused = t.pause();
        let ask = |ids: &[u64]| ids.iter().map(|&i| (i, f[i as usize].clone())).collect();
        t.request(ask(&[3, 1, 2, 1]), Target::Srgb);
        assert_eq!(queued(&t), [3, 1, 2], "in order, once each");
        t.request(ask(&[2, 4]), Target::Srgb);
        assert_eq!(queued(&t), [2, 4], "the newer order replaces it");
        drop(paused);
        t.wait_idle();
        assert_eq!(sorted(&got), [(2, true), (4, true)], "3 and 1 were dropped");

        // One made already is answered at once, even during a switch.
        got.lock().unwrap().clear();
        let paused = t.pause();
        t.request(ask(&[4, 0]), Target::Srgb);
        assert_eq!(*got.lock().unwrap(), [(4, true)]);
        assert_eq!(queued(&t), [0]);
        // For another target it is made again.
        t.request(ask(&[4]), Target::DisplayP3);
        assert_eq!(queued(&t), [4]);
        drop(paused);
        t.stop();
    }

    #[test]
    fn nothing_starts_during_a_switch_or_while_prefetch_has_work() {
        let dir = Dir::new("quiet");
        let a = dir.file("a.jpg", &jpeg_with_preview(400, 300, [10; 3]));
        let b = dir.file("b.jpg", &jpeg_with_preview(400, 300, [20; 3]));
        let busy = Arc::new(AtomicBool::new(true));
        let (t, got) = thumbs(None, &busy);
        t.request(vec![(1, a)], Target::Srgb);
        std::thread::sleep(Duration::from_millis(150));
        assert!(got.lock().unwrap().is_empty(), "prefetch goes first");
        busy.store(false, Ordering::SeqCst);
        t.wait_idle();
        assert_eq!(sorted(&got), [(1, true)]);

        let paused = t.pause();
        t.request(vec![(2, b)], Target::Srgb);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(sorted(&got), [(1, true)], "a switch goes first");
        drop(paused);
        t.wait_idle();
        assert_eq!(sorted(&got), [(1, true), (2, true)]);
        t.stop();
    }

    #[test]
    fn a_thumbnail_is_the_preview_reduced_in_one_jpeg_tile() {
        let dir = Dir::new("tile");
        let a = dir.file("a.jpg", &jpeg_with_preview(400, 300, [200, 100, 50]));
        let message = make(&a, Target::Srgb).unwrap().unwrap();
        let tiles = crate::split_frame(&message).unwrap();
        assert_eq!(tiles.len(), 1);
        let (h, jpeg) = &tiles[0];
        assert_eq!((h.generation, h.mip_level), (0, 0));
        assert_eq!(h.format, genzo_model::frame::PixelFormat::JpegSrgb);
        assert_eq!((h.rect.width, h.rect.height), (320, 240));
        assert_eq!((h.image_width, h.image_height), (320, 240));
        let img = genzo_imgio::decode(jpeg, Default::default()).unwrap();
        let genzo_imgio::Pixels::Rgba16 { data, .. } = img.pixels else {
            panic!("a JPEG decodes to RGBA");
        };
        let at = data.len() / 2 / 4 * 4;
        for (c, want) in [200i32, 100, 50].into_iter().enumerate() {
            let got = i32::from(data[at + c] / 257);
            assert!((got - want).abs() <= 3, "{:?}", &data[at..at + 4]);
        }
        // One already small enough is kept at its size, never enlarged.
        let b = dir.file("b.jpg", &jpeg_with_preview(200, 150, [9; 3]));
        let small = make(&b, Target::Srgb).unwrap().unwrap();
        let (h, _) = &crate::split_frame(&small).unwrap()[0];
        assert_eq!((h.rect.width, h.rect.height), (200, 150));
        // P3 frames, P3 thumbnails.
        let p3 = make(&a, Target::DisplayP3).unwrap().unwrap();
        let (h, _) = &crate::split_frame(&p3).unwrap()[0];
        assert_eq!(h.format, genzo_model::frame::PixelFormat::JpegDisplayP3);
        // The smallest preview of at least 320 px, not the camera's larger
        // one that a switch shows.
        let two = dir.file(
            "two.jpg",
            &jpeg_with_previews(&[(1600, 1200, [0, 0, 250]), (480, 360, [250, 0, 0])]),
        );
        let message = make(&two, Target::Srgb).unwrap().unwrap();
        let (_, jpeg) = &crate::split_frame(&message).unwrap()[0];
        let img = genzo_imgio::decode(jpeg, Default::default()).unwrap();
        let genzo_imgio::Pixels::Rgba16 { data, .. } = img.pixels else {
            panic!("a JPEG decodes to RGBA");
        };
        assert!(
            data[0] / 257 > 200 && data[2] / 257 < 50,
            "{:?}",
            &data[..4]
        );
        // No preview: none. Not readable: tried again later.
        let png = dir.file("c.png", b"\x89PNG\r\n\x1a\n");
        assert_eq!(make(&png, Target::Srgb), Some(None));
        assert_eq!(make(&dir.0.join("gone.jpg"), Target::Srgb), None);
    }

    #[test]
    fn the_disk_tier_serves_a_later_session_until_the_file_changes() {
        let dir = Dir::new("disk");
        let cache = dir.0.join("cache");
        let a = dir.file("a.jpg", &jpeg_with_preview(400, 300, [30; 3]));
        let png = dir.file("c.png", b"\x89PNG\r\n\x1a\n");
        let never = Arc::new(AtomicBool::new(false));

        let (t1, got) = thumbs(Some(cache.clone()), &never);
        t1.request(vec![(1, a.clone()), (2, png.clone())], Target::Srgb);
        t1.wait_idle();
        assert_eq!(sorted(&got), [(1, true), (2, false)]);
        t1.stop();
        let entries: Vec<PathBuf> = std::fs::read_dir(cache.join(DISK_DIR))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries.iter().all(|e| e.extension().unwrap() == "gzth"));

        // Mark a's entry, so a read from disk shows.
        let key = disk_key(&a, Target::Srgb).unwrap();
        let entry = t1.entry(&key).unwrap();
        let mut bytes = std::fs::read(&entry).unwrap();
        bytes.truncate(4 + 4 + key.len());
        bytes.extend_from_slice(b"from disk");
        std::fs::write(&entry, &bytes).unwrap();

        let (t2, got) = thumbs(Some(cache.clone()), &never);
        t2.request(vec![(1, a.clone()), (2, png.clone())], Target::Srgb);
        t2.wait_idle();
        assert_eq!(sorted(&got), [(1, true), (2, false)]);
        assert_eq!(
            t2.get(1, Target::Srgb, None).unwrap().as_slice(),
            b"from disk"
        );
        // Out of memory, still on disk.
        let t3 = Thumbs::new(Some(cache.clone()), Box::new(|| false));
        assert_eq!(
            t3.get(1, Target::Srgb, Some(&a)).unwrap().as_slice(),
            b"from disk"
        );
        assert_eq!(t3.get(1, Target::DisplayP3, Some(&a)), None);
        // An entry holding another key, as a hash collision would, is a
        // miss.
        let mut other = std::fs::read(&entry).unwrap();
        other[8] ^= 1;
        std::fs::write(&entry, &other).unwrap();
        assert_eq!(t3.get(1, Target::Srgb, Some(&a)), None);
        std::fs::write(&entry, &bytes).unwrap();
        t2.stop();

        // A changed file is a miss.
        std::fs::write(&a, jpeg_with_preview(600, 400, [30; 3])).unwrap();
        let (t4, _) = thumbs(Some(cache), &never);
        t4.request(vec![(1, a)], Target::Srgb);
        t4.wait_idle();
        let made = t4.get(1, Target::Srgb, None).unwrap();
        assert_eq!(crate::split_frame(&made).unwrap()[0].0.rect.width, 320);
        t4.stop();
    }
}
