//! The opened folder's session (LIB-09): stable ids, switching photos in
//! Develop, and criterion 6, the edited dot across folder switches.
//!
//! The files are stubs: a fake decoder records which it is asked for and
//! returns a synthetic photo per name, so nothing reads real pixels.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
use genzo_engine::strip::{Decode, Direction, Strip};
use genzo_engine::{Engine, EngineError};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

/// Decodes any file to a synthetic photo whose size depends on its name,
/// and records the names. Names starting with "broken" fail.
#[derive(Debug, Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.calls.lock().unwrap())
    }
}

fn source_for(name: &str) -> RawSource {
    let n = name.bytes().map(u32::from).sum::<u32>() % 7;
    with_camera_profile(scene_raw(300 + 20 * n, 220 + 10 * n))
}

impl Decode for Fake {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        self.calls.lock().unwrap().push(name.clone());
        if name.starts_with("broken") {
            return Err(EngineError::Decode(format!("{name} is broken")));
        }
        Ok((source_for(&name), 1))
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str, names: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-strip-{}-{tag}", std::process::id()));
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

const VIEW: ViewSpec = ViewSpec::Fit((200, 150));

fn pixels(a: &Applied) -> Vec<[u8; 4]> {
    match &a.frame.as_ref().expect("rendered").readback {
        Readback::Frame { pixels, .. } => pixels.clone(),
        r => panic!("{r:?}"),
    }
}

fn ids(strip: &Strip) -> Vec<u64> {
    strip.photos.iter().map(|p| p.id).collect()
}

fn edited(strip: &Strip) -> Vec<bool> {
    strip.photos.iter().map(|p| p.edited).collect()
}

#[test]
fn ids_are_stable_for_the_session_without_a_gpu() {
    let a = TempDir::new("ids-a", &["IMG_2.jpg", "IMG_10.jpg"]);
    let b = TempDir::new("ids-b", &["other.png"]);
    let engine = Engine::headless();
    let first = engine.open_folder(&a.0).unwrap();
    let names: Vec<&str> = first
        .strip
        .photos
        .iter()
        .map(|p| p.file_name.as_str())
        .collect();
    assert_eq!(names, ["IMG_2.jpg", "IMG_10.jpg"]);
    // Nothing renders without a GPU, so nothing is selected.
    assert!(matches!(first.first, Some(Err(EngineError::NoGpu(_)))));
    let other = engine.open_folder(&b.0).unwrap();
    let again = engine.open_folder(&a.0).unwrap();
    assert_eq!(ids(&again.strip), ids(&first.strip));
    assert!(!ids(&first.strip).contains(&other.strip.photos[0].id));
    // An empty folder: an empty strip, nothing selected.
    let empty = TempDir::new("ids-empty", &[]);
    let opened = engine.open_folder(&empty.0).unwrap();
    assert!(opened.strip.photos.is_empty() && opened.first.is_none());
    // A folder that cannot be read leaves the strip as it was.
    engine.open_folder(&a.0).unwrap();
    assert!(engine.open_folder(&a.0.join("gone")).is_err());
    assert_eq!(ids(&engine.strip()), ids(&first.strip));
}

#[test]
fn a_switch_opens_the_photo_in_develop() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let dir = TempDir::new("switch", &["a.jpg", "b.jpg", "c.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let [a, b, c] = ids(&opened.strip)[..] else {
        panic!()
    };
    opened.first.unwrap().unwrap();
    assert_eq!(fake.calls(), ["a.jpg"], "the first photo opens");
    assert_eq!(opened.strip.current, Some(a));
    engine.set_view(VIEW, Target::Srgb).unwrap();

    let switched = engine.select_photo(c).unwrap();
    assert_eq!(fake.calls(), ["c.jpg"]);
    assert_eq!(switched.state.photo, c);
    let strip = engine.strip();
    assert_eq!((strip.current, strip.direction), (Some(c), Direction::Next));
    // The frame is the photo's, as if opened from its input.
    let theirs = pixels(&switched);
    engine.open_source(source_for("c.jpg"), 1).unwrap();
    let want = pixels(&engine.set_view(VIEW, Target::Srgb).unwrap());
    assert_eq!(theirs, want);

    // Back by one, from 1:1: previous, at Fit in the same viewport. The
    // open photo again: nothing decoded.
    let one_to_one = ViewSpec::Region {
        out: (200, 150),
        zoom: 1.0,
        origin: (0.0, 0.0),
    };
    engine.set_view(one_to_one, Target::Srgb).unwrap();
    let back = engine.step(Direction::Previous).unwrap().unwrap();
    assert_eq!(back.state.photo, b);
    let Readback::Frame { size, .. } = back.frame.unwrap().readback else {
        panic!("not at Fit");
    };
    assert!(size.0 == 200 || size.1 == 150, "{size:?}");
    assert_eq!(engine.strip().direction, Direction::Previous);
    engine.select_photo(b).unwrap();
    assert_eq!(fake.calls(), ["b.jpg"]);
    assert_eq!(engine.strip().direction, Direction::Previous);
    // The strip does not wrap.
    engine.select_photo(a).unwrap();
    assert!(engine.step(Direction::Previous).unwrap().is_none());
    assert_eq!(engine.strip().current, Some(a));
    assert!(matches!(
        engine.select_photo(9999),
        Err(EngineError::NotInStrip(9999))
    ));
}

#[test]
fn the_edited_dot_follows_the_edit_across_folders() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let a = TempDir::new("dot-a", &["one.jpg", "two.jpg"]);
    let b = TempDir::new("dot-b", &["three.jpg"]);
    engine.open_folder(&a.0).unwrap().first.unwrap().unwrap();
    assert_eq!(edited(&engine.strip()), [false, false]);
    // Set by an edit, cleared by undoing back to the defaults.
    engine.apply(&[Change::Exposure(0.5)], Phase::End).unwrap();
    assert_eq!(edited(&engine.strip()), [true, false]);
    engine.undo().unwrap();
    assert_eq!(edited(&engine.strip()), [false, false]);
    engine.apply(&[Change::Contrast(20.0)], Phase::End).unwrap();
    let edit = engine.edit_state().unwrap().edit;

    // Kept across a folder switch and back, with its history.
    engine.open_folder(&b.0).unwrap().first.unwrap().unwrap();
    assert_eq!(edited(&engine.strip()), [false]);
    let back = engine.open_folder(&a.0).unwrap();
    assert_eq!(edited(&back.strip), [true, false]);
    let state = back.first.unwrap().unwrap().state;
    assert_eq!(state.edit, edit);
    assert!(state.edited);
    assert_eq!(state.undo.as_deref(), Some("Contrast"));
}

#[test]
fn a_switch_ends_the_gesture_first() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let dir = TempDir::new("gesture", &["a.jpg", "b.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let [a, b] = ids(&opened.strip)[..] else {
        panic!()
    };
    engine
        .apply(&[Change::Exposure(0.3)], Phase::Start)
        .unwrap();
    engine
        .apply(&[Change::Exposure(0.7)], Phase::Update)
        .unwrap();
    engine.select_photo(b).unwrap();
    let state = engine.select_photo(a).unwrap().state;
    assert_eq!(state.edit.exposure, 0.7);
    assert_eq!(state.undo.as_deref(), Some("Exposure"));

    // A folder with no photos, and a photo opened from its input, end it
    // too.
    let empty = TempDir::new("gesture-empty", &[]);
    engine
        .apply(&[Change::Contrast(15.0)], Phase::Start)
        .unwrap();
    engine.open_folder(&empty.0).unwrap();
    engine.open_folder(&dir.0).unwrap();
    let state = engine.edit_state().unwrap();
    assert_eq!((state.photo, state.edit.contrast), (a, 15.0));
    assert_eq!(state.undo.as_deref(), Some("Contrast"));
    engine.apply(&[Change::Whites(10.0)], Phase::Start).unwrap();
    engine.open_source(source_for("x.jpg"), 1).unwrap();
    let state = engine.select_photo(a).unwrap().state;
    assert_eq!(state.undo.as_deref(), Some("Whites"));
}

#[test]
fn a_photo_that_does_not_decode_is_still_selected() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(engine) = engine(&fake) else { return };
    let dir = TempDir::new("broken", &["a.jpg", "broken.jpg", "c.jpg"]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let [_, broken, c] = ids(&opened.strip)[..] else {
        panic!()
    };
    assert!(matches!(
        engine.step(Direction::Next),
        Err(EngineError::Decode(_))
    ));
    assert_eq!(engine.strip().current, Some(broken));
    assert!(engine.edit_state().is_none());
    assert!(matches!(
        engine.apply(&[], Phase::End),
        Err(EngineError::NoPhoto)
    ));
    // The arrows still move on from it.
    let next = engine.step(Direction::Next).unwrap().unwrap();
    assert_eq!(next.state.photo, c);
    // A folder with no photos closes the one open.
    let empty = TempDir::new("broken-empty", &[]);
    engine.open_folder(&empty.0).unwrap();
    assert!(engine.edit_state().is_none());
}
