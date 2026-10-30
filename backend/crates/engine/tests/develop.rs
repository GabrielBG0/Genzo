//! Develop through the engine: the edit and history owned here, the renders
//! on the render thread (DEV-01, APP-10).
//!
//! - APP-10's criterion 8: after undoing a white balance step, the settled
//!   frame equals a fresh render of the restored edit, bit for bit.
//! - A request that arrives while the rest of a rebuild runs stops it, and
//!   the frames still equal fresh renders.
//!
//! References are rendered by opening the photo again in the same engine:
//! a second device in one process crashes WARP.

use std::sync::Mutex;

use genzo_engine::Engine;
use genzo_engine::api::edit::{Change, Phase, WhiteBalanceMode};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn engine() -> Option<Engine> {
    let engine = Engine::new();
    if engine.edit_state().is_none() && engine.info().adapter.name == "none" {
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

/// The frame of `changes` applied one by one to a fresh opening of the
/// photo.
fn fresh(engine: &Engine, changes: &[Change]) -> Vec<[u8; 4]> {
    engine.open_source(source(), 6).unwrap();
    let mut last = engine.set_view(VIEW, Target::Srgb).unwrap();
    for c in changes {
        last = engine.apply(std::slice::from_ref(c), Phase::End).unwrap();
    }
    pixels(&last)
}

const VIEW: ViewSpec = ViewSpec::Fit((420, 310));

fn white(temp: f32) -> Change {
    Change::WhiteBalance {
        mode: WhiteBalanceMode::Custom,
        temp,
        tint: 5.0,
    }
}

fn tight_crop() -> Change {
    Change::Crop {
        cx: 0.6,
        cy: 0.4,
        w: 0.3,
        h: 0.3,
        angle: 5.0,
    }
}

#[test]
fn undoing_a_white_balance_renders_the_restored_edit() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    engine.open_source(source(), 6).unwrap();
    engine.set_view(VIEW, Target::Srgb).unwrap();
    let steps = [tight_crop(), white(3700.0), Change::Exposure(0.6)];
    for c in &steps {
        let a = engine.apply(std::slice::from_ref(c), Phase::End).unwrap();
        assert!(a.recorded, "{c:?}");
    }
    // Undo the exposure, then the white balance.
    let after_one = engine.undo().unwrap();
    assert_eq!(after_one.state.undo.as_deref(), Some("White balance"));
    let after_two = engine.undo().unwrap();
    assert_eq!(after_two.state.redo.as_deref(), Some("White balance"));
    let (one, two) = (pixels(&after_one), pixels(&after_two));
    assert_eq!(one, fresh(&engine, &steps[..2]), "the white balance kept");
    assert_eq!(two, fresh(&engine, &steps[..1]), "the white balance undone");
}

#[test]
fn a_request_during_the_rest_still_renders_as_fresh() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    engine.open_source(source(), 6).unwrap();
    engine.set_view(VIEW, Target::Srgb).unwrap();
    engine.apply(&[tight_crop()], Phase::End).unwrap();
    // The release leaves the rest of the rebuild to the render thread;
    // the requests after it come while it runs.
    engine.apply(&[white(4300.0)], Phase::End).unwrap();
    engine
        .apply(&[Change::Highlights(-50.0)], Phase::Start)
        .unwrap();
    let toned = engine
        .apply(&[Change::Highlights(-50.0)], Phase::End)
        .unwrap();
    let want = fresh(
        &engine,
        &[tight_crop(), white(4300.0), Change::Highlights(-50.0)],
    );
    assert_eq!(pixels(&toned), want, "a tone change during the rest");
}

fn size(a: &Applied) -> (u32, u32) {
    match &a.frame.as_ref().expect("rendered").readback {
        Readback::Frame { size, .. } => *size,
        r => panic!("{r:?}"),
    }
}

/// A Fit view is the framed image scaled to fit the viewport, its aspect
/// kept: the frame is that size, not the viewport's (DEV-01, SetView).
#[test]
fn a_fit_frame_keeps_the_photos_aspect() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let wide = ViewSpec::Fit((320, 180));
    // 1030 × 777 in a 16:9 viewport: the height binds, 238.6 px wide.
    engine.open_source(source(), 1).unwrap();
    assert_eq!(
        size(&engine.set_view(wide, Target::Srgb).unwrap()),
        (239, 180)
    );
    // The file's orientation 6 shows it 777 × 1030.
    engine.open_source(source(), 6).unwrap();
    assert_eq!(
        size(&engine.set_view(wide, Target::Srgb).unwrap()),
        (136, 180)
    );
    // A tall viewport: the width binds.
    let tall = engine.set_view(ViewSpec::Fit((100, 400)), Target::Srgb);
    assert_eq!(size(&tall.unwrap()), (100, 133));
    // A crop has its own aspect: half the width, 388.5 × 1030.
    engine.set_view(wide, Target::Srgb).unwrap();
    let half = Change::Crop {
        cx: 0.5,
        cy: 0.5,
        w: 0.5,
        h: 1.0,
        angle: 0.0,
    };
    let cropped = engine.apply(&[half], Phase::End).unwrap();
    assert_eq!(size(&cropped), (68, 180));
}

/// A photo under 32 px on a side (the smallest crop) renders whole, at a
/// view larger or smaller than itself, and takes edits.
#[test]
fn a_photo_under_the_smallest_crop_renders() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    for (w, h) in [(30, 200), (20, 12), (2, 2)] {
        engine
            .open_source(with_camera_profile(scene_raw(w, h)), 1)
            .unwrap_or_else(|e| panic!("{w} x {h}: {e}"));
        for view in [ViewSpec::Fit((120, 90)), ViewSpec::Fit((10, 10))] {
            let a = engine
                .set_view(view, Target::Srgb)
                .unwrap_or_else(|e| panic!("{w} x {h}, {view:?}: {e}"));
            assert!(!pixels(&a).is_empty(), "{w} x {h}, {view:?}");
        }
        let a = engine
            .apply(&[Change::Exposure(1.0)], Phase::End)
            .unwrap_or_else(|e| panic!("{w} x {h}: {e}"));
        assert!(!pixels(&a).is_empty(), "{w} x {h}");
    }
}
