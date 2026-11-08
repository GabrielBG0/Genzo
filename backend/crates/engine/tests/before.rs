//! WF-02's M1 toggle through the engine: the before is the photo's
//! defaults, records nothing, and any change to the edit or a switch turns
//! it off (criteria 1 to 6).
//!
//! References are rendered by opening the photo again in the same engine:
//! a second device in one process crashes WARP.

use std::sync::Mutex;

use genzo_engine::Engine;
use genzo_engine::api::edit::{Change, Param, Phase, WhiteBalanceMode};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
use genzo_engine::export::ExportRequest;
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

const VIEW: ViewSpec = ViewSpec::Fit((420, 310));

fn engine() -> Option<Engine> {
    let engine = Engine::new();
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some(engine)
}

fn source() -> RawSource {
    with_camera_profile(scene_raw(1030, 777))
}

fn pixels(a: &Applied) -> Vec<[u8; 4]> {
    match &a.frame.as_ref().expect("rendered").readback {
        Readback::Frame { pixels, .. } => pixels.clone(),
        r => panic!("{r:?}"),
    }
}

fn edits() -> [Change; 3] {
    [
        Change::WhiteBalance {
            mode: WhiteBalanceMode::Custom,
            temp: 3900.0,
            tint: 8.0,
        },
        Change::Exposure(0.7),
        Change::Shadows(40.0),
    ]
}

/// The photo opened again and edited with `edits()`; its id.
fn edited(engine: &Engine) -> u64 {
    let photo = engine.open_source(source(), 6).unwrap();
    engine.set_view(VIEW, Target::Srgb).unwrap();
    for c in edits() {
        engine.apply(&[c], Phase::End).unwrap();
    }
    photo
}

#[test]
fn the_before_is_the_defaults_and_records_nothing() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    // The reference: the photo with no edit, and its histogram.
    let fresh = engine.open_source(source(), 6).unwrap();
    let defaults = pixels(&engine.set_view(VIEW, Target::Srgb).unwrap());
    engine.wait_histograms_idle();
    let default_counts = engine.histogram(fresh).expect("counted").histogram;

    let photo = edited(&engine);
    let after = pixels(&engine.set_view(VIEW, Target::Srgb).unwrap());
    assert_ne!(after, defaults, "the edit shows");
    let state = engine.edit_state().unwrap();

    // Criterion 1: the before's frame is the defaults', bit for bit.
    let before = engine.show_before(photo, true).unwrap();
    assert!(before.state.before);
    assert!(!before.recorded);
    assert_eq!(pixels(&before), defaults);
    // Criterion 5: the histogram counts the before.
    engine.wait_histograms_idle();
    assert_eq!(engine.histogram(photo).unwrap().histogram, default_counts);
    // The view keeps it: a new view renders the before again.
    let resized = engine
        .set_view(ViewSpec::Fit((300, 220)), Target::Srgb)
        .unwrap();
    assert!(resized.state.before);
    engine.set_view(VIEW, Target::Srgb).unwrap();

    // Criterion 2: nothing recorded, and the after comes back.
    let back = engine.show_before(photo, false).unwrap();
    assert_eq!(pixels(&back), after);
    let now = engine.edit_state().unwrap();
    assert_eq!(
        (now.edit, now.edited, now.undo, now.redo, now.revision),
        (
            state.edit,
            state.edited,
            state.undo,
            state.redo,
            state.revision
        )
    );
    let undone = engine.undo().unwrap();
    assert_eq!(undone.state.edit.shadows, 0.0, "the last real step");

    // A photo not open: nothing changes.
    let other = engine.show_before(photo + 100, true);
    assert!(other.is_err() || !other.unwrap().state.before);
}

#[test]
fn a_change_or_a_switch_turns_the_before_off() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let photo = edited(&engine);
    // Criterion 3: each change answers the after, its frame the edit's.
    type Act = fn(&Engine, u64) -> Applied;
    let acts: [(&str, Act); 5] = [
        ("a delta", |e, p| {
            e.apply_to(p, &[Change::Contrast(20.0)], Phase::End)
                .unwrap()
        }),
        ("a gesture's start", |e, p| {
            e.apply_to(p, &[Change::Contrast(25.0)], Phase::Start)
                .unwrap()
        }),
        ("a reset", |e, p| {
            e.apply_to(p, &[Change::Reset(Param::Contrast)], Phase::End)
                .unwrap()
        }),
        ("undo", |e, p| e.undo_of(p).unwrap()),
        ("redo", |e, p| e.redo_of(p).unwrap()),
    ];
    for (name, act) in acts {
        let on = engine.show_before(photo, true).unwrap();
        assert!(on.state.before, "{name}");
        let a = act(&engine, photo);
        assert!(!a.state.before, "{name} turns the before off");
        assert!(a.frame.is_some(), "{name} renders");
        if name == "a gesture's start" {
            engine.apply_to(photo, &[], Phase::End).unwrap();
        }
        // Its frame is the edit's: the same as rendering the edit again
        // (a gesture's frames are the drag preview's, so not compared).
        let again = engine.set_view(VIEW, Target::Srgb).unwrap();
        assert!(!again.state.before);
        if name != "a gesture's start" {
            assert_eq!(pixels(&a), pixels(&again), "{name}");
        }
    }

    // A pick: the first point the picker takes.
    let picked = (1..10)
        .map(|i| f64::from(i) / 10.0)
        .find_map(|t| {
            engine.show_before(photo, true).unwrap();
            engine.pick_white_balance(photo, (t, t)).ok()
        })
        .expect("a point the picker takes");
    assert!(!picked.state.before, "a pick turns the before off");

    // Criterion 4: a switch turns it off.
    engine.show_before(photo, true).unwrap();
    let next = engine.open_source(source(), 6).unwrap();
    assert!(!engine.edit_state().unwrap().before);
    // And the photo left keeps no before either.
    assert!(!engine.edit_of(photo).unwrap().before);
    assert_ne!(next, photo);
}

#[test]
fn an_export_has_the_edit_whatever_shows() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let dir = std::env::temp_dir().join(format!("genzo-before-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let photo = edited(&engine);
    let export = |name: &str| {
        let path = dir.join(name);
        engine
            .export_jpeg(ExportRequest {
                photo,
                path: path.clone(),
                long_edge: Some(400),
                quality: 90,
                replace: false,
            })
            .unwrap();
        engine.wait_exports_idle();
        std::fs::read(path).unwrap()
    };
    // Criterion 6.
    let off = export("off.jpg");
    engine.show_before(photo, true).unwrap();
    let on = export("on.jpg");
    assert!(
        engine.edit_state().unwrap().before,
        "an export changes nothing shown"
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(off, on);
}
