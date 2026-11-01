//! The histogram and the clipping warnings through the engine (DEV-03).
//!
//! - Each Fit frame's counts reach the sink and `histogram`, drag frames
//!   included.
//! - Criterion 4: at 1:1 the whole photo's counts come after the frame and
//!   equal the Fit ones, bin for bin; a pan asks for none, and a drag after
//!   the pass renders as one without it.
//! - Criterion 6's end of the gesture: the Alt preview ends with it, by an
//!   *end* or by a switch, and the settled frame is the normal one.
//! - J stays on across photos, with nothing open too.
//! - While Opening, the shown preview's counts come first, below the
//!   frame's generation; a file that does not decode keeps them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::develop::{Applied, Readback, Reran, Target, ViewSpec};
use genzo_engine::histogram::{Counted, Overlay, Preview};
use genzo_engine::preview::Shown;
use genzo_engine::strip::Decode;
use genzo_engine::{Engine, EngineError};
use genzo_imgio::ColorEncoding;
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::histogram::count;
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

const RED: [u8; 4] = [230, 40, 40, 255];
const BLUE: [u8; 4] = [40, 90, 230, 255];

/// What the sink received.
#[derive(Default)]
struct Counts(Mutex<Vec<Counted>>);

impl Counts {
    fn take(&self) -> Vec<Counted> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

fn engine_with(engine: Engine) -> Option<(Engine, Arc<Counts>)> {
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    let counts = Arc::new(Counts::default());
    let c = counts.clone();
    let engine = engine.with_histogram_sink(Arc::new(move |x| c.0.lock().unwrap().push(x)));
    Some((engine, counts))
}

fn engine() -> Option<(Engine, Arc<Counts>)> {
    engine_with(Engine::new())
}

fn source(size: (u32, u32)) -> RawSource {
    with_camera_profile(scene_raw(size.0, size.1))
}

fn pixels(a: &Applied) -> Vec<[u8; 4]> {
    match &a.frame.as_ref().expect("rendered").readback {
        Readback::Frame { pixels, .. } => pixels.clone(),
        r => panic!("{r:?}"),
    }
}

/// The frame as read back: tiles at 1:1.
fn readback(a: &Applied) -> &Readback {
    &a.frame.as_ref().expect("rendered").readback
}

fn generation(a: &Applied) -> u64 {
    a.frame.as_ref().expect("rendered").generation
}

/// Stage 9 alone.
const OUTPUT: Reran = Reran {
    pyramid: false,
    finished_rebuild: false,
    base_layer: false,
    sample: false,
    tone: false,
    color: false,
    output: true,
};

fn reran(a: &Applied) -> Reran {
    a.frame.as_ref().expect("rendered").reran
}

/// Enough to clip both ends.
fn clipping() -> [Change; 2] {
    [Change::Exposure(1.5), Change::Blacks(-40.0)]
}

#[test]
fn each_fit_frame_sends_its_histogram() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, counts)) = engine() else {
        return;
    };
    let photo = engine.open_source(source((1030, 777)), 1).unwrap();
    assert_eq!(engine.histogram(photo), None);
    let a = engine
        .set_view(ViewSpec::Fit((420, 310)), Target::Srgb)
        .unwrap();
    let want = Counted {
        photo,
        generation: generation(&a),
        histogram: count(&pixels(&a)),
    };
    assert_eq!(counts.take(), std::slice::from_ref(&want));
    assert_eq!(engine.histogram(photo), Some(want));
    assert_eq!(engine.histogram(photo + 1), None, "only the shown photo's");
    // A drag's frames too, at half size.
    for phase in [Phase::Start, Phase::Update] {
        let a = engine.apply(&[Change::Exposure(0.7)], phase).unwrap();
        let [c] = &counts.take()[..] else {
            panic!("one count per frame")
        };
        assert_eq!(c.generation, generation(&a));
        assert_eq!(c.histogram, count(&pixels(&a)));
    }
    engine.apply(&[], Phase::End).unwrap();
    engine.wait_histograms_idle();
    assert_eq!(counts.take().len(), 1);
}

#[test]
fn at_1_1_the_whole_photo_is_counted_after_the_frame() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, counts)) = engine() else {
        return;
    };
    // Level 1, 1200 x 800, covers the Fit frame: bin for bin.
    let size = (2400, 1600);
    let out = (600, 450);
    let fit = ViewSpec::Fit(out);
    let corner = ViewSpec::Region {
        out,
        zoom: 1.0,
        origin: (0.0, 0.0),
    };
    let other = ViewSpec::Region {
        out,
        zoom: 1.0,
        origin: (1800.0, 1150.0),
    };
    let photo = engine.open_source(source(size), 1).unwrap();
    engine.set_view(fit, Target::Srgb).unwrap();
    engine.apply(&[Change::Exposure(1.0)], Phase::End).unwrap();
    engine.wait_histograms_idle();
    let at_fit = engine.histogram(photo).expect("counted at Fit").histogram;
    counts.take();

    let a = engine.set_view(corner, Target::Srgb).unwrap();
    engine.wait_histograms_idle();
    let got = counts.take();
    assert_eq!(
        got,
        [Counted {
            photo,
            generation: generation(&a),
            histogram: at_fit,
        }],
        "the whole photo's, after the frame"
    );
    // A drag right after the pass: as one in a session that had none, and
    // no counts until its release.
    let drag = engine
        .apply(&[Change::Exposure(1.4)], Phase::Start)
        .unwrap();
    engine.wait_histograms_idle();
    assert_eq!(counts.take(), []);
    let released = engine.apply(&[], Phase::End).unwrap();
    engine.wait_histograms_idle();
    let [c] = &counts.take()[..] else {
        panic!("one count after the release")
    };
    assert_eq!(c.generation, generation(&released));
    // A pan counts the same pixels: no pass.
    engine.set_view(other, Target::Srgb).unwrap();
    engine.wait_histograms_idle();
    assert_eq!(counts.take(), []);

    engine.open_source(source(size), 1).unwrap();
    engine.set_view(fit, Target::Srgb).unwrap();
    engine.apply(&[Change::Exposure(1.4)], Phase::End).unwrap();
    let fresh = engine.set_view(corner, Target::Srgb).unwrap();
    assert_eq!(readback(&drag), readback(&fresh));
    assert_eq!(readback(&released), readback(&fresh));
}

#[test]
fn the_clipping_preview_ends_with_the_gesture() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, _counts)) = engine() else {
        return;
    };
    let view = ViewSpec::Fit((420, 310));
    engine.open_source(source((1030, 777)), 1).unwrap();
    engine.set_view(view, Target::Srgb).unwrap();
    let [a, b] = clipping();
    engine.apply(std::slice::from_ref(&a), Phase::End).unwrap();
    let drag = engine
        .apply(std::slice::from_ref(&b), Phase::Start)
        .unwrap();
    let plain = pixels(&drag);

    let shown = engine
        .set_clipping_preview(Preview::Highlights)
        .unwrap()
        .expect("rendered");
    assert_eq!(reran(&shown), OUTPUT);
    let highlights = Overlay {
        preview: Preview::Highlights,
        ..Overlay::default()
    };
    let want: Vec<_> = plain.iter().map(|&p| highlights.apply(p)).collect();
    assert_eq!(pixels(&shown), want);
    assert_ne!(want, plain, "something clips");
    assert!(
        engine
            .set_clipping_preview(Preview::Highlights)
            .unwrap()
            .is_none(),
        "unchanged"
    );
    // Its end: the normal settled frame, and no preview after it.
    let settled = engine.apply(&[], Phase::End).unwrap();
    assert!(
        engine
            .set_clipping_preview(Preview::Shadows)
            .unwrap()
            .is_none()
    );
    engine.open_source(source((1030, 777)), 1).unwrap();
    engine.set_view(view, Target::Srgb).unwrap();
    engine.apply(&clipping(), Phase::End).unwrap();
    let fresh = engine.set_view(view, Target::Srgb).unwrap();
    assert_eq!(pixels(&settled), pixels(&fresh));

    // A switch ends it too: the next photo's drag, its first render,
    // shows no preview.
    engine
        .apply(std::slice::from_ref(&a), Phase::Start)
        .unwrap();
    engine.set_clipping_preview(Preview::Shadows).unwrap();
    engine.open_source(source((1030, 777)), 1).unwrap();
    let next = engine.apply(&clipping(), Phase::Start).unwrap();
    assert_eq!(pixels(&next), plain);
}

#[test]
fn j_stays_on_across_photos() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, counts)) = engine() else {
        return;
    };
    let view = ViewSpec::Fit((420, 310));
    let j = Overlay {
        highlights: Some(RED),
        shadows: Some(BLUE),
        ..Overlay::default()
    };
    // With nothing open, kept for the next photo.
    assert!(
        engine
            .set_clipping_overlay(Some(BLUE), Some(RED))
            .unwrap()
            .is_none()
    );
    for size in [(1030, 777), (640, 900)] {
        engine.open_source(source(size), 1).unwrap();
        engine.set_view(view, Target::Srgb).unwrap();
        let on = engine.apply(&clipping(), Phase::End).unwrap();
        counts.take();
        let off = engine
            .set_clipping_overlay(None, None)
            .unwrap()
            .expect("rendered");
        assert_eq!(reran(&off), OUTPUT);
        let plain = pixels(&off);
        let want: Vec<_> = plain.iter().map(|&p| j.apply(p)).collect();
        assert_eq!(pixels(&on), want, "{size:?}");
        assert_ne!(want, plain, "{size:?}: something clips");
        // What is counted is the photo, never the overlay.
        let [c] = &counts.take()[..] else {
            panic!("one count")
        };
        assert_eq!(c.histogram, count(&plain));
        engine.set_clipping_overlay(Some(BLUE), Some(RED)).unwrap();
    }
}

/// Decodes a synthetic photo once let through, and gives each file a
/// preview with codes 0 to 255; "broken" files do not decode.
#[derive(Debug, Default)]
struct Gated {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gated {
    fn let_through(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }
}

impl Decode for Gated {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        if name.starts_with("broken") {
            return Err(EngineError::Damaged(format!("{name} is broken")));
        }
        Ok((source((600, 400)), 1))
    }

    fn preview(&self, _path: &Path) -> Result<Option<genzo_imgio::Preview>, EngineError> {
        let rgba = (0..64 * 32)
            .flat_map(|i: u32| {
                let v = (i % 256) as u8;
                [v, 255 - v, v / 2, 255]
            })
            .collect();
        Ok(Some(genzo_imgio::Preview {
            width: 64,
            height: 32,
            rgba,
            orientation: 1,
            color: ColorEncoding::Srgb,
        }))
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(names: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-histogram-{}", std::process::id()));
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

#[test]
fn while_opening_the_preview_is_counted() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gate = Arc::new(Gated::default());
    let Some((engine, counts)) = engine_with(Engine::new().with_decoder(gate.clone())) else {
        return;
    };
    let shown = Arc::new(Mutex::new(Vec::<Shown>::new()));
    let s = shown.clone();
    let engine = Arc::new(engine.with_preview_sink(Arc::new(move |x| s.lock().unwrap().push(x))));
    let dir = TempDir::new(&["a.jpg", "broken.jpg"]);

    // The folder's first photo opens behind the gate.
    let opening = {
        let engine = engine.clone();
        let dir = dir.0.clone();
        std::thread::spawn(move || engine.open_folder(&dir).unwrap())
    };
    let first = loop {
        if let Some(x) = shown.lock().unwrap().first().cloned() {
            break x;
        }
        std::thread::yield_now();
    };
    let preview = first.preview.expect("a preview");
    // Counted after the sink has it, off the switch's thread.
    let start = std::time::Instant::now();
    let early = loop {
        if let Some(c) = engine.histogram(first.photo) {
            break c;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            gate.let_through();
            panic!("the preview was not counted");
        }
        std::thread::yield_now();
    };
    assert_eq!(
        early.histogram,
        count(preview.rgba.as_chunks::<4>().0),
        "as shown"
    );
    assert!(early.histogram.highlights.pixels > 0 && early.histogram.shadows.pixels > 0);
    gate.let_through();
    let opened = opening.join().unwrap();
    let frame = opened.first.expect("one photo").unwrap();
    engine.wait_histograms_idle();
    let got = counts.take();
    assert_eq!(got[0], early);
    let last = got.last().unwrap();
    assert!(early.generation < last.generation);
    assert_eq!(last.generation, generation(&frame));
    assert_eq!(last.histogram, count(&pixels(&frame)));

    // A file that does not decode keeps its preview's.
    let broken = opened.strip.photos[1].id;
    assert!(engine.select_photo(broken).is_err());
    engine.wait_histograms_idle();
    let kept = engine.histogram(broken).expect("the preview's");
    assert_eq!(kept.histogram, early.histogram);
}
