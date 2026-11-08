//! DEV-12's uncropped view through the engine: while the crop tool is
//! open the photo renders whole and level in its orientation, a change to
//! the crop keeps it, nothing is recorded, and a switch ends it (criterion
//! 1).
//!
//! References are rendered by the same engine: a second device in one
//! process crashes WARP.

use std::sync::Mutex;

use genzo_engine::Engine;
use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
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

const CROP: Change = Change::Crop {
    cx: 0.45,
    cy: 0.5,
    w: 0.5,
    h: 0.6,
    angle: 7.0,
};

#[test]
fn the_photo_shows_whole_and_level_until_the_tool_closes() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let photo = engine.open_source(source(), 1).unwrap();
    engine.set_view(VIEW, Target::Srgb).unwrap();
    // The reference: the photo turned, with no crop.
    let turned = engine
        .apply(&[Change::Turn { clockwise: true }], Phase::End)
        .unwrap();
    let whole = pixels(&turned);
    assert_eq!(turned.state.shown, (777, 1030), "turned on its side");

    let cropped = engine.apply(&[CROP], Phase::End).unwrap();
    assert_ne!(pixels(&cropped), whole, "the crop shows");
    assert!(!cropped.state.uncropped);
    let shown = cropped.state.shown;
    assert!(shown.0 < 777 && shown.1 < 1030, "{shown:?}");

    // The tool opens: the whole oriented photo, bit for bit; nothing recorded.
    let on = engine.show_uncropped(photo, true).unwrap();
    assert!(on.state.uncropped);
    assert!(!on.recorded);
    assert_eq!(pixels(&on), whole);
    assert_eq!(on.state.shown, on.state.oriented);
    assert_eq!(
        on.state.edit.crop, cropped.state.edit.crop,
        "the crop is kept"
    );
    assert_eq!(on.state.revision, cropped.state.revision);

    // A crop change while it is open is recorded, and keeps the view.
    let moved = engine
        .apply(
            &[Change::Crop {
                cx: 0.55,
                cy: 0.5,
                w: 0.5,
                h: 0.6,
                angle: -3.0,
            }],
            Phase::End,
        )
        .unwrap();
    assert!(moved.recorded);
    assert!(moved.state.uncropped);
    assert_eq!(pixels(&moved), whole);

    // Closed: the crop shows again.
    let off = engine.show_uncropped(photo, false).unwrap();
    assert!(!off.state.uncropped);
    assert_ne!(pixels(&off), whole);
    assert_ne!(off.state.shown, off.state.oriented);

    // A switch ends it, and a photo not open is not changed.
    engine.show_uncropped(photo, true).unwrap();
    let next = engine.open_source(source(), 1).unwrap();
    assert!(!engine.edit_state().unwrap().uncropped);
    assert!(!engine.edit_of(photo).unwrap().uncropped);
    let other = engine.show_uncropped(photo, true);
    assert!(other.is_err() || !other.unwrap().state.uncropped);
    assert_ne!(next, photo);
}
