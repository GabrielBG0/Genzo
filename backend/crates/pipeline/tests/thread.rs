//! Develop on the render thread (`gpu::thread`): latest-wins coalescing of
//! Develop renders, renders for a photo that is no longer open, and renders
//! held until the input of a photo opened from its level 1 comes.

use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase, WhiteBalanceMode};
use genzo_pipeline::gpu::develop::{Photo, PhotoInfo, Readback, as_shot};
use genzo_pipeline::gpu::raw_stages::{Level1, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::thread::{DevelopRequest, RenderThread};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::reference::sampling::ViewSpec;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn thread() -> Option<RenderThread> {
    match RenderThread::spawn(AdapterChoice::from_env()) {
        Ok(t) => Some(t),
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn photo(size: (u32, u32)) -> Photo {
    Photo {
        src: with_camera_profile(scene_raw(size.0, size.1)),
        orientation: 1,
    }
}

fn request(generation: u64, photo: u64, edit: &Edit, view: ViewSpec) -> DevelopRequest {
    DevelopRequest {
        generation,
        photo,
        edit: edit.clone(),
        view,
        target: Target::Srgb,
        phase: Phase::End,
    }
}

#[test]
fn the_newest_develop_render_wins() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let p = photo((2400, 1600));
    let (_, kind) = genzo_pipeline::gpu::develop::as_shot(&p.src);
    t.open(1, p).unwrap();
    let defaults = Edit::defaults(kind);
    // Renders queued back to back. Which of them share a batch depends on
    // when the thread wakes, so the test holds what is true in every
    // interleaving: the newest always renders, every one replaced has a
    // newer one that rendered, and none renders out of order.
    let view = ViewSpec::Region {
        out: (1600, 1200),
        zoom: 1.0,
        origin: (0.0, 0.0),
    };
    for round in 0..3u64 {
        let pending: Vec<_> = (1..=6)
            .map(|g| {
                let mut e = defaults.clone();
                e.white_balance.mode = WhiteBalanceMode::Custom;
                e.white_balance.temp = 3000.0 + 400.0 * g as f32 + 50.0 * round as f32;
                t.submit(request(round * 10 + g, 1, &e, view)).unwrap()
            })
            .collect();
        let answers: Vec<Option<u64>> = pending
            .into_iter()
            .map(|rx| rx.recv().unwrap().unwrap().map(|f| f.generation))
            .collect();
        assert_eq!(
            *answers.last().unwrap(),
            Some(round * 10 + 6),
            "{answers:?}"
        );
        for (i, a) in answers.iter().enumerate() {
            if a.is_none() {
                assert!(answers[i + 1..].iter().any(Option::is_some), "{answers:?}");
            }
        }
        let ran: Vec<u64> = answers.iter().flatten().copied().collect();
        assert!(ran.windows(2).all(|w| w[0] < w[1]), "{answers:?}");
        eprintln!("round {round}: {answers:?}");
    }
}

#[test]
fn a_render_for_a_photo_no_longer_open_is_dropped() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let a = photo((640, 480));
    let (_, kind) = genzo_pipeline::gpu::develop::as_shot(&a.src);
    let defaults = Edit::defaults(kind);
    let fit = ViewSpec::Fit((320, 240));
    t.open(1, a).unwrap();
    assert!(t.develop(request(1, 1, &defaults, fit)).unwrap().is_some());
    t.open(2, photo((500, 400))).unwrap();
    assert_eq!(t.develop(request(2, 1, &defaults, fit)).unwrap(), None);
    let f = t.develop(request(3, 2, &defaults, fit)).unwrap().unwrap();
    match f.readback {
        Readback::Frame { size, half, .. } => {
            assert_eq!((size, half), ((320, 240), false));
        }
        r => panic!("{r:?}"),
    }
}

/// Long enough for any answer that should come.
const WAIT: Duration = Duration::from_secs(30);

fn level1(p: &Photo) -> Level1 {
    let gpu = Gpu::open(AdapterChoice::from_env()).unwrap();
    let (t, _) = as_shot(&p.src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut b = stages.start_level1(&gpu, &p.src, &t);
    assert!(
        stages
            .continue_level1(&gpu, &p.src, &mut b, || false)
            .unwrap()
    );
    b.finish()
}

#[test]
fn a_render_that_needs_the_input_waits_for_it() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let size = (640, 480);
    let p = photo(size);
    let (_, kind) = as_shot(&p.src);
    let defaults = Edit::defaults(kind);
    let fit = ViewSpec::Fit((240, 180));
    t.open_level1(1, PhotoInfo::of(&p), Arc::new(level1(&p)))
        .unwrap();
    let f = t.develop(request(1, 1, &defaults, fit)).unwrap().unwrap();
    assert!(!f.provisional && !f.reran.pyramid, "{f:?}");

    // Settled new whites wait for the input; the newer replaces the older.
    let warmer = |temp| {
        let mut e = defaults.clone();
        e.white_balance.mode = WhiteBalanceMode::Custom;
        e.white_balance.temp = temp;
        e
    };
    let older = t.submit(request(2, 1, &warmer(4000.0), fit)).unwrap();
    assert_eq!(
        older.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    );
    let newer = t.submit(request(3, 1, &warmer(4500.0), fit)).unwrap();
    assert_eq!(older.recv_timeout(WAIT).unwrap().unwrap(), None);
    assert_eq!(
        newer.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    );
    // Another photo's input is not this one's.
    t.attach(2, photo((500, 400)).src).unwrap();
    assert_eq!(
        newer.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    );
    t.attach(1, photo(size).src).unwrap();
    let f = newer.recv_timeout(WAIT).unwrap().unwrap().unwrap();
    assert_eq!(f.generation, 3);
    assert!(f.reran.pyramid, "{f:?}");

    // A held render is dropped when another photo opens.
    let q = photo((500, 400));
    t.open_level1(2, PhotoInfo::of(&q), Arc::new(level1(&q)))
        .unwrap();
    let held = t.submit(request(4, 2, &warmer(5000.0), fit)).unwrap();
    assert_eq!(
        held.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    );
    t.open(3, photo((320, 240))).unwrap();
    assert_eq!(held.recv_timeout(WAIT).unwrap().unwrap(), None);
}
