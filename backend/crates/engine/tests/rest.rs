//! A gesture at rest settles (DEV-01, *Re-runs, quality levels and generations*;
//! `Engine::with_rest_settle`): a slider held still gets the settled frame
//! without its release, the gesture goes on, and a release needs none.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::api::{Event, PixelFormat};
use genzo_engine::develop::{Readback, Target, ViewSpec};
use genzo_engine::{Engine, split_frame};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

const REST: Duration = Duration::from_millis(40);

type Rendered = Arc<Mutex<Vec<u64>>>;

/// An engine that settles after [`REST`], and the generations it rendered.
fn engine(rest: bool) -> Option<(Engine, Rendered)> {
    let rendered = Rendered::default();
    let sink = rendered.clone();
    let mut engine = Engine::new().with_event_sink(Arc::new(move |e| {
        if let Event::RenderFinished { generation, .. } = e {
            sink.lock().unwrap().push(generation);
        }
    }));
    if rest {
        engine = engine.with_rest_settle_after(REST);
    }
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some((engine, rendered))
}

/// Waits for a frame after `generation`, at most 10 s: its generation.
fn frame_after(rendered: &Rendered, generation: u64) -> Option<u64> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let newer = rendered
            .lock()
            .unwrap()
            .iter()
            .copied()
            .find(|g| *g > generation);
        if newer.is_some() || Instant::now() > until {
            return newer;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_gesture_at_rest_settles_and_goes_on() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, rendered)) = engine(true) else {
        return;
    };
    engine
        .open_source(with_camera_profile(scene_raw(1030, 777)), 1)
        .unwrap();
    let settled = engine
        .set_view(ViewSpec::Fit((600, 450)), Target::Srgb)
        .unwrap();
    let Readback::Frame { size: full, .. } = settled.frame.unwrap().readback else {
        panic!("a frame at Fit")
    };

    // The drag's frame is the half-size preview.
    let a = engine
        .apply(&[Change::Exposure(0.5)], Phase::Start)
        .unwrap();
    let drag = a.frame.unwrap();
    assert!(matches!(drag.readback, Readback::Frame { half: true, .. }));

    // At rest, the settled frame follows on its own: whole and lossless.
    let at_rest = frame_after(&rendered, drag.generation).expect("a frame at rest");
    let bytes = engine.frame(at_rest).expect("kept");
    let tiles = split_frame(&bytes).unwrap();
    assert_eq!(tiles.len(), 1);
    let (header, _) = tiles[0];
    assert_eq!(header.format, PixelFormat::Rgba8Srgb);
    assert_eq!((header.rect.width, header.rect.height), full);

    // The gesture goes on: nothing is recorded, and the next move is a
    // drag's frame again.
    assert_eq!(engine.edit_state().unwrap().undo, None);
    let a = engine
        .apply(&[Change::Exposure(0.7)], Phase::Update)
        .unwrap();
    assert!(!a.recorded);
    let drag = a.frame.unwrap();
    assert!(matches!(drag.readback, Readback::Frame { half: true, .. }));

    // Its release settles and records one step; no frame follows it.
    let a = engine.apply(&[Change::Exposure(0.7)], Phase::End).unwrap();
    assert!(a.recorded);
    let released = a.frame.unwrap().generation;
    std::thread::sleep(REST * 5);
    assert_eq!(rendered.lock().unwrap().iter().max(), Some(&released));
    assert!(engine.edit_state().unwrap().undo.is_some());
}

#[test]
fn a_moving_gesture_does_not_settle() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, rendered)) = engine(true) else {
        return;
    };
    engine
        .open_source(with_camera_profile(scene_raw(1030, 777)), 1)
        .unwrap();
    engine
        .set_view(ViewSpec::Fit((600, 450)), Target::Srgb)
        .unwrap();
    engine
        .apply(&[Change::Exposure(0.1)], Phase::Start)
        .unwrap();
    // Moves closer together than the rest: every frame is the drag's.
    let mut last = 0;
    for i in 2..12 {
        let a = engine
            .apply(&[Change::Exposure(0.1 * i as f32)], Phase::Update)
            .unwrap();
        last = a.frame.unwrap().generation;
    }
    let seen = rendered.lock().unwrap().clone();
    assert!(
        seen.iter().all(|g| *g <= last),
        "no frame but the drag's: {seen:?}, last {last}"
    );
    engine.apply(&[], Phase::End).unwrap();
}

#[test]
fn without_it_a_gesture_settles_only_on_its_release() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, rendered)) = engine(false) else {
        return;
    };
    engine
        .open_source(with_camera_profile(scene_raw(1030, 777)), 1)
        .unwrap();
    engine
        .set_view(ViewSpec::Fit((600, 450)), Target::Srgb)
        .unwrap();
    let a = engine
        .apply(&[Change::Exposure(0.5)], Phase::Start)
        .unwrap();
    let drag = a.frame.unwrap().generation;
    std::thread::sleep(REST * 5);
    assert_eq!(rendered.lock().unwrap().iter().max(), Some(&drag));
}
