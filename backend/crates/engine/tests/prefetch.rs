//! The strip's neighbors, prefetched (LIB-09 criterion 4; DEV-01,
//! *Prefetched neighbors*).
//!
//! A fake decoder records which files it is asked for, in order, and can
//! hold one until the test releases it, so every order here is
//! deterministic. The files are stubs; each name decodes to a synthetic
//! photo of its own size.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
use genzo_engine::preview::Shown;
use genzo_engine::strip::Decode;
use genzo_engine::{Engine, EngineError};
use genzo_imgio::{ColorEncoding, Preview};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
struct Calls {
    names: Vec<String>,
    /// Every name asked for, never drained.
    seen: Vec<String>,
    held: HashSet<String>,
    /// Every name whose preview was asked for, never drained.
    previews: Vec<String>,
    /// Names that fail as missing, from the decodes started after.
    gone: HashSet<String>,
}

/// Names with "broken" in them fail.
#[derive(Debug, Default)]
struct Fake {
    calls: Mutex<Calls>,
    changed: Condvar,
}

impl Fake {
    /// The names asked for since the last call.
    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.calls.lock().unwrap().names)
    }

    /// Holds the decode of `name` until [`Self::release`].
    fn hold(&self, name: &str) {
        self.calls.lock().unwrap().held.insert(name.into());
    }

    fn release(&self, name: &str) {
        self.calls.lock().unwrap().held.remove(name);
        self.changed.notify_all();
    }

    /// Makes the decodes of `name` started from now on fail as missing.
    fn delete(&self, name: &str) {
        self.calls.lock().unwrap().gone.insert(name.into());
    }

    fn release_all(&self) {
        self.calls.lock().unwrap().held.clear();
        self.changed.notify_all();
    }

    /// Waits until the decode of `name` has started.
    fn wait_started(&self, name: &str) {
        let mut c = self.calls.lock().unwrap();
        while !c.seen.iter().any(|n| n == name) {
            c = self.changed.wait(c).unwrap();
        }
    }
}

fn source_for(name: &str) -> RawSource {
    let n = name.bytes().map(u32::from).sum::<u32>() % 7;
    with_camera_profile(scene_raw(300 + 20 * n, 220 + 10 * n))
}

impl Decode for Fake {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut c = self.calls.lock().unwrap();
        c.names.push(name.clone());
        c.seen.push(name.clone());
        self.changed.notify_all();
        if c.gone.contains(&name) {
            return Err(EngineError::Missing(name));
        }
        while c.held.contains(&name) {
            c = self.changed.wait(c).unwrap();
        }
        drop(c);
        if name.contains("broken") {
            return Err(EngineError::Decode(format!("{name} is broken")));
        }
        Ok((source_for(&name), 1))
    }

    /// A 4 × 2 sRGB preview whose codes depend on the name; none for names
    /// with "nopreview" in them. A broken file has one, as unsupported
    /// files do.
    fn preview(&self, path: &Path) -> Result<Option<Preview>, EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        self.calls.lock().unwrap().previews.push(name.clone());
        if name.contains("nopreview") {
            return Ok(None);
        }
        let v = name.bytes().fold(0u8, u8::wrapping_add);
        Ok(Some(Preview {
            width: 4,
            height: 2,
            rgba: [v, 2, 3, 255].repeat(8),
            orientation: 1,
            color: ColorEncoding::Srgb,
        }))
    }
}

/// Releases every held decode when dropped, before the engine is: the
/// engine's drop waits for the worker, which may wait in a held decode.
struct ReleaseAll(Arc<Fake>);

impl Drop for ReleaseAll {
    fn drop(&mut self) {
        self.0.release_all();
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str, names: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-prefetch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"stub").unwrap();
        }
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn engine(fake: &Arc<Fake>) -> Option<Engine> {
    let engine = Engine::new().with_decoder(fake.clone());
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some(engine)
}

const NINE: [&str; 9] = [
    "p0.jpg", "p1.jpg", "p2.jpg", "p3.jpg", "p4.jpg", "p5.jpg", "p6.jpg", "p7.jpg", "p8.jpg",
];

/// A view level 1 of every photo here covers: never provisional.
const SMALL: ViewSpec = ViewSpec::Fit((120, 90));
/// A view that wants level 0.
const LARGE: ViewSpec = ViewSpec::Fit((600, 450));

fn frame(a: &Applied) -> (Vec<[u8; 4]>, bool) {
    let f = a.frame.as_ref().expect("rendered");
    match &f.readback {
        Readback::Frame { pixels, .. } => (pixels.clone(), f.provisional),
        r => panic!("{r:?}"),
    }
}

/// The frame of the photo named `name` opened from its input, at `view`.
fn fresh(engine: &Engine, name: &str, view: ViewSpec) -> Vec<[u8; 4]> {
    engine.open_source(source_for(name), 1).unwrap();
    frame(&engine.set_view(view, Target::Srgb).unwrap()).0
}

#[test]
fn prefetch_follows_the_direction_of_travel() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("order", &NINE);
    let p: Vec<u64> = engine
        .open_folder(&dir.0)
        .unwrap()
        .strip
        .photos
        .iter()
        .map(|p| p.id)
        .collect();
    engine.wait_prefetch_idle();
    // On opening, as after moving right: the photo, next, next + 1.
    assert_eq!(fake.calls(), ["p0.jpg", "p1.jpg", "p2.jpg"]);
    assert_eq!(engine.prefetched(), [p[1], p[2]]);

    // Right to a cached neighbor: its input first, then next + 1 and the
    // previous one; next is cached.
    engine.select_photo(p[1]).unwrap();
    engine.wait_prefetch_idle();
    assert_eq!(fake.calls(), ["p1.jpg", "p3.jpg", "p0.jpg"]);

    // A jump right, to a photo not prefetched: decoded, then next, and the
    // two before it.
    engine.select_photo(p[7]).unwrap();
    engine.wait_prefetch_idle();
    assert_eq!(fake.calls(), ["p7.jpg", "p8.jpg", "p6.jpg", "p5.jpg"]);

    // Left: the mirror. Previous is cached, then previous − 1, next.
    engine.select_photo(p[6]).unwrap();
    engine.wait_prefetch_idle();
    assert_eq!(fake.calls(), ["p6.jpg", "p4.jpg", "p7.jpg"]);
}

#[test]
fn a_switch_cancels_the_neighbors_no_longer_wanted() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("cancel", &NINE);
    fake.hold("p1.jpg");
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    // The worker decodes p1, held; p2 waits behind it.
    fake.wait_started("p1.jpg");
    engine.select_photo(p[5]).unwrap();
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    // p1, decoded by then, is no longer a neighbor and is not built; p2 is
    // never decoded.
    assert_eq!(
        fake.calls(),
        [
            "p0.jpg", "p1.jpg", "p5.jpg", "p6.jpg", "p7.jpg", "p4.jpg", "p3.jpg"
        ]
    );
    assert_eq!(engine.prefetched(), [p[3], p[4], p[6], p[7]]);
}

#[test]
fn a_neighbor_that_does_not_decode_is_not_tried_again() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("failed", &["p0.jpg", "p1-broken.jpg", "p2.jpg", "p3.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.wait_prefetch_idle();
    assert_eq!(fake.calls(), ["p0.jpg", "p1-broken.jpg", "p2.jpg"]);
    engine.select_photo(p[2]).unwrap();
    engine.wait_prefetch_idle();
    // Its input, then next; the previous one failed, previous − 1 next.
    assert_eq!(fake.calls(), ["p2.jpg", "p3.jpg", "p0.jpg"]);
    // Selecting it still tries, and shows why.
    assert!(matches!(
        engine.select_photo(p[1]),
        Err(EngineError::Decode(_))
    ));
}

#[test]
fn a_switch_to_a_prefetched_neighbor_renders_without_decoding() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("fast", &["p0.jpg", "p1.jpg", "p2.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.set_view(SMALL, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();
    // Its input's decode is held, so the switch renders from level 1 alone.
    fake.hold("p1.jpg");
    let switched = engine.select_photo(p[1]).unwrap();
    let (pixels, provisional) = frame(&switched);
    assert!(!provisional, "level 1 covers the view");
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    // Not provisional, so not rendered again.
    assert!(engine.take_sharpened().is_none());
    assert_eq!(pixels, fresh(&engine, "p1.jpg", SMALL));
}

/// A viewport wider than level 1, whose fitted frame level 1 covers: p1's
/// 360 x 250 in 260 x 100 is 144 x 100, its level 1 180 x 125. The switch
/// is final, as for a 12 MP photo at Fit in a 2560 x 1440 viewport.
#[test]
fn a_neighbor_whose_level1_covers_the_fitted_frame_is_not_provisional() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("fitted", &["p0.jpg", "p1.jpg", "p2.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    let wide = ViewSpec::Fit((260, 100));
    engine.set_view(wide, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();
    fake.hold("p1.jpg");
    let switched = engine.select_photo(p[1]).unwrap();
    let f = switched.frame.as_ref().expect("rendered");
    assert!(!f.provisional, "level 1 covers the fitted frame");
    assert!(matches!(
        f.readback,
        Readback::Frame {
            size: (144, 100),
            ..
        }
    ));
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    assert!(engine.take_sharpened().is_none());
    assert_eq!(frame(&switched).0, fresh(&engine, "p1.jpg", wide));
}

#[test]
fn a_provisional_frame_is_rendered_again_once_the_input_comes() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("sharpen", &["p0.jpg", "p1.jpg", "p2.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.set_view(LARGE, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();
    fake.hold("p1.jpg");
    let switched = engine.select_photo(p[1]).unwrap();
    assert!(
        frame(&switched).1,
        "level 1 upsampled for a view that wants level 0"
    );
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    let sharp = engine.take_sharpened().expect("rendered again");
    let (pixels, provisional) = frame(&sharp);
    assert!(!provisional);
    assert!(engine.take_sharpened().is_none(), "taken once");
    assert!(
        pixels == fresh(&engine, "p1.jpg", LARGE),
        "as from its input"
    );
}

#[test]
fn a_frame_newer_than_the_provisional_one_is_not_rendered_again() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("newer", &["p0.jpg", "p1.jpg", "p2.jpg", "p3.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.set_view(LARGE, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();

    // A view level 1 covers came after it.
    fake.hold("p1.jpg");
    assert!(frame(&engine.select_photo(p[1]).unwrap()).1);
    engine.set_view(SMALL, Target::Srgb).unwrap();
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    assert!(engine.take_sharpened().is_none(), "after a newer frame");

    // A gesture in progress.
    engine.set_view(LARGE, Target::Srgb).unwrap();
    fake.hold("p2.jpg");
    assert!(frame(&engine.select_photo(p[2]).unwrap()).1);
    let during = engine
        .apply(&[Change::Exposure(0.4)], Phase::Start)
        .unwrap();
    assert!(during.frame.unwrap().provisional);
    fake.wait_started("p2.jpg");
    fake.release("p2.jpg");
    engine.wait_prefetch_idle();
    assert!(engine.take_sharpened().is_none(), "during a gesture");
    engine.apply(&[Change::Exposure(0.4)], Phase::End).unwrap();

    // A settled new white, held on the render thread for the input: the
    // frame it gets once the input comes is the sharp one, rendered once.
    fake.hold("p3.jpg");
    assert!(frame(&engine.select_photo(p[3]).unwrap()).1);
    fake.wait_started("p3.jpg");
    let warm = Change::WhiteBalance {
        mode: genzo_engine::api::edit::WhiteBalanceMode::Custom,
        temp: 4100.0,
        tint: 8.0,
    };
    std::thread::scope(|scope| {
        let held = scope.spawn(|| engine.apply(&[warm], Phase::End).unwrap());
        // It is held until the input comes.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!held.is_finished(), "held for the input");
        fake.release("p3.jpg");
        let got = held.join().unwrap();
        assert!(!got.frame.unwrap().provisional);
    });
    engine.wait_prefetch_idle();
    assert!(engine.take_sharpened().is_none(), "after the held render");
}

#[test]
fn a_neighbor_whose_white_changed_since_is_prefetched_again() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("white", &["p0.jpg", "p1.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.set_view(SMALL, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();
    engine.select_photo(p[1]).unwrap();
    engine.wait_prefetch_idle();
    // Its cached level 1 has the white it was built with.
    let warm = Change::WhiteBalance {
        mode: genzo_engine::api::edit::WhiteBalanceMode::Custom,
        temp: 4100.0,
        tint: 8.0,
    };
    let edited = engine.apply(&[warm], Phase::End).unwrap();
    fake.calls();
    // Not the level 1 of the old white: it is decoded and built again.
    engine.select_photo(p[0]).unwrap();
    engine.wait_prefetch_idle();
    assert_eq!(
        fake.calls(),
        ["p0.jpg", "p1.jpg"],
        "built with its new white"
    );
    // And the switch back shows it at once, before its input comes.
    fake.hold("p1.jpg");
    let back = engine.select_photo(p[1]).unwrap();
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    assert_eq!(
        frame(&back).0,
        frame(&edited).0,
        "its edit, from its new level 1"
    );
}

#[test]
fn a_switch_drops_the_sharp_frame_of_the_photo_before() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("drop-sharp", &["p0.jpg", "p1.jpg", "p2.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    engine.set_view(LARGE, Target::Srgb).unwrap();
    engine.wait_prefetch_idle();
    fake.hold("p1.jpg");
    assert!(frame(&engine.select_photo(p[1]).unwrap()).1);
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    // Rendered again, not taken, and then the photo goes.
    engine.select_photo(p[0]).unwrap();
    assert!(engine.take_sharpened().is_none());
}

/// A neighbor whose decode read the file before it went, and finished
/// after the switch to it found it missing: the switch's result stands.
#[test]
fn a_neighbor_decode_older_than_the_switch_does_not_set_its_kind() {
    use genzo_engine::strip::PhotoKind;
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("stale-kind", &["p0.jpg", "p1.jpg"]);
    fake.hold("p1.jpg");
    let opened = engine.open_folder(&dir.0).unwrap();
    let p1 = opened.strip.photos[1].id;
    fake.wait_started("p1.jpg");
    fake.delete("p1.jpg");
    assert!(matches!(
        engine.select_photo(p1),
        Err(EngineError::Missing(_))
    ));
    fake.release("p1.jpg");
    engine.wait_prefetch_idle();
    assert_eq!(engine.strip().photos[1].kind, PhotoKind::Missing);
}

/// What the preview sink received, in order.
#[derive(Default)]
struct Seen {
    shown: Mutex<Vec<Shown>>,
    changed: Condvar,
}

impl Seen {
    /// Waits for, and returns, what was shown for `photo`; panics after
    /// 10 s.
    fn wait_for(&self, photo: u64) -> Shown {
        let s = self.shown.lock().unwrap();
        let (s, _) = self
            .changed
            .wait_timeout_while(s, std::time::Duration::from_secs(10), |s| {
                !s.iter().any(|x| x.photo == photo)
            })
            .unwrap();
        let found = s.iter().find(|x| x.photo == photo).cloned();
        found.unwrap_or_else(|| panic!("nothing shown for photo {photo}"))
    }

    fn take(&self) -> Vec<Shown> {
        std::mem::take(&mut self.shown.lock().unwrap())
    }
}

fn with_sink(engine: Engine) -> (Engine, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let s = seen.clone();
    let engine = engine.with_preview_sink(Arc::new(move |shown| {
        s.shown.lock().unwrap().push(shown);
        s.changed.notify_all();
    }));
    (engine, seen)
}

fn previews(fake: &Fake) -> Vec<String> {
    std::mem::take(&mut fake.calls.lock().unwrap().previews)
}

/// The red code of a shown preview's first pixel: which photo's it is.
fn code(s: &Shown) -> u8 {
    s.preview.as_ref().expect("a preview").rgba[0]
}

fn code_of(name: &str) -> u8 {
    name.bytes().fold(0u8, u8::wrapping_add)
}

#[test]
fn a_switch_shows_the_preview_before_the_photo_decodes() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let (engine, seen) = with_sink(engine);
    let engine = Arc::new(engine);
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new("first-pixels", &["p0.jpg", "p1.jpg", "p2.jpg", "p3.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let p: Vec<u64> = opened.strip.photos.iter().map(|p| p.id).collect();
    // The first photo's preview is shown on opening the folder.
    assert_eq!(code(&seen.wait_for(p[0])), code_of("p0.jpg"));
    engine.wait_prefetch_idle();
    seen.take();
    previews(&fake);

    // p3 is no neighbor: its decode is held, and its preview is already
    // shown, ready to draw.
    fake.hold("p3.jpg");
    let (e, far) = (engine.clone(), p[3]);
    let switch = std::thread::spawn(move || e.select_photo(far).map(|a| a.state.photo));
    let shown = seen.wait_for(p[3]);
    let preview = shown.preview.as_ref().unwrap();
    assert_eq!((preview.width, preview.height), (4, 2));
    assert_eq!(preview.target, Target::Srgb);
    assert_eq!(code(&shown), code_of("p3.jpg"));
    fake.wait_started("p3.jpg");
    assert!(!switch.is_finished(), "shown while its decode is held");
    fake.release("p3.jpg");
    assert_eq!(switch.join().unwrap().unwrap(), p[3]);
    engine.wait_prefetch_idle();
    seen.take();
    previews(&fake);

    // A neighbor prefetched: the preview kept with its level 1, not read
    // again on the switch.
    engine.select_photo(p[2]).unwrap();
    assert_eq!(code(&seen.wait_for(p[2])), code_of("p2.jpg"));
    assert!(!previews(&fake).contains(&"p2.jpg".into()));
    engine.wait_prefetch_idle();
    seen.take();

    // Kept for sRGB, so prepared again for Display P3 from its kept
    // pixels, without reading the file: other codes for the same color.
    engine.set_view(SMALL, Target::DisplayP3).unwrap();
    previews(&fake);
    engine.select_photo(p[1]).unwrap();
    let p3 = seen.wait_for(p[1]);
    assert_eq!(p3.preview.as_ref().unwrap().target, Target::DisplayP3);
    assert_ne!(code(&p3), code_of("p1.jpg"));
    assert!(previews(&fake).is_empty());
}

#[test]
fn a_switch_without_a_preview_says_so() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let (engine, seen) = with_sink(engine);
    let _release = ReleaseAll(fake.clone());
    let dir = TempDir::new(
        "no-preview",
        &[
            "a.jpg",
            "b_nopreview.jpg",
            "c.jpg",
            "d_broken.jpg",
            "e_nopreview.jpg",
        ],
    );
    let p: Vec<u64> = engine
        .open_folder(&dir.0)
        .unwrap()
        .strip
        .photos
        .iter()
        .map(|p| p.id)
        .collect();
    engine.wait_prefetch_idle();
    // A neighbor without one, kept as none: shown as none, so the photo
    // before does not stay.
    engine.select_photo(p[1]).unwrap();
    assert!(seen.wait_for(p[1]).preview.is_none());
    // Not prefetched, and without one.
    engine.select_photo(p[4]).unwrap();
    assert!(seen.wait_for(p[4]).preview.is_none());
    engine.wait_prefetch_idle();
    // A file that does not decode still shows its preview, beside its
    // error.
    seen.take();
    assert!(engine.select_photo(p[3]).is_err());
    assert_eq!(code(&seen.wait_for(p[3])), code_of("d_broken.jpg"));
    // Each switch shows once.
    assert_eq!(seen.take().len(), 1);
}
