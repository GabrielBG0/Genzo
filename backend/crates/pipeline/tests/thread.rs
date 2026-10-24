//! Develop on the render thread (`gpu::thread`): latest-wins coalescing of
//! Develop renders, renders for a photo that is no longer open, and renders
//! held until the input of a photo opened from its level 1 comes.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase, WhiteBalanceMode};
use genzo_pipeline::gpu::develop::{Photo, PhotoInfo, Readback, as_shot};
use genzo_pipeline::gpu::raw_stages::{Level1, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::thread::{DevelopFrame, DevelopRequest, RenderThread};
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
    // An input of another size is refused, and the thread goes on.
    assert!(t.attach(1, photo((500, 400)).src).is_err());
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

fn frame_of(f: &DevelopFrame) -> &[[u8; 4]] {
    match &f.readback {
        Readback::Frame { pixels, .. } => pixels,
        r => panic!("{r:?}"),
    }
}

#[test]
fn a_neighbor_prefetches_beside_the_open_photo() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let a = photo((640, 480));
    let (_, kind) = as_shot(&a.src);
    let defaults = Edit::defaults(kind);
    let (fit, other) = (ViewSpec::Fit((240, 180)), ViewSpec::Fit((200, 150)));
    t.open(1, a).unwrap();
    t.develop(request(1, 1, &defaults, other)).unwrap().unwrap();
    let before = t.develop(request(2, 1, &defaults, fit)).unwrap().unwrap();

    let b = photo((500, 400));
    let (white, _) = as_shot(&b.src);
    let want = level1(&b);
    let got = t
        .prefetch(2, b, white)
        .unwrap()
        .recv_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got.id, 2);
    assert_eq!(got.info, PhotoInfo::of(&photo((500, 400))));
    assert!(got.level1.texels == want.texels, "the level 1, bit for bit");

    // The open photo's pyramid is as it was: sampled again, the same frame.
    t.develop(request(3, 1, &defaults, other)).unwrap().unwrap();
    let after = t.develop(request(4, 1, &defaults, fit)).unwrap().unwrap();
    assert!(after.reran.sample, "{after:?}");
    assert!(frame_of(&before) == frame_of(&after), "the open photo");

    // A switch to it renders as the photo opened from its input.
    t.open_level1(2, got.info, Arc::new(got.level1)).unwrap();
    let switched = t.develop(request(5, 2, &defaults, fit)).unwrap().unwrap();
    assert!(!switched.reran.pyramid && !switched.provisional);
    t.open(3, photo((500, 400))).unwrap();
    let opened = t.develop(request(6, 3, &defaults, fit)).unwrap().unwrap();
    assert!(
        frame_of(&switched) == frame_of(&opened),
        "the switch's frame"
    );
}

#[test]
fn prefetches_no_longer_wanted_are_cancelled() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    t.open(1, photo((320, 240))).unwrap();
    let queue = |id, size| {
        let p = photo(size);
        let (white, _) = as_shot(&p.src);
        t.prefetch(id, p, white).unwrap()
    };
    let two = queue(2, (2400, 1600));
    let three = queue(3, (640, 480));
    let four = queue(4, (500, 400));
    t.keep_prefetching(&[3]).unwrap();
    // The first may have finished before the cancel came.
    let two = two.recv_timeout(WAIT).unwrap().unwrap();
    eprintln!(
        "the first: {}",
        if two.is_some() { "built" } else { "cancelled" }
    );
    assert_eq!(
        four.recv_timeout(WAIT).unwrap().unwrap().map(|p| p.id),
        None
    );
    let three = three.recv_timeout(WAIT).unwrap().unwrap().unwrap();
    assert!(three.level1.texels == level1(&photo((640, 480))).texels);
}

#[test]
fn queued_prefetches_build_one_after_another() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let a = photo((4000, 3000));
    let (_, kind) = as_shot(&a.src);
    let mut warm = Edit::defaults(kind);
    warm.white_balance.mode = WhiteBalanceMode::Custom;
    warm.white_balance.temp = 4300.0;
    t.open(1, a).unwrap();
    let (b, c) = (photo((1200, 900)), photo((1100, 800)));
    let (wb, wc) = (as_shot(&b.src).0, as_shot(&c.src).0);
    // Queued while the thread rebuilds the open photo's pyramid, so it
    // takes them in one go; then nothing more is asked. Each build follows
    // the last without waiting for a request.
    let render = t
        .submit(request(1, 1, &warm, ViewSpec::Fit((160, 120))))
        .unwrap();
    let two = t.prefetch(2, b, wb).unwrap();
    let three = t.prefetch(3, c, wc).unwrap();
    t.keep_prefetching(&[2, 3]).unwrap();
    assert!(render.recv_timeout(WAIT).unwrap().unwrap().is_some());
    for (rx, id) in [(two, 2), (three, 3)] {
        let got = rx.recv_timeout(WAIT).unwrap().unwrap().map(|p| p.id);
        assert_eq!(got, Some(id));
    }
}

#[test]
fn a_switch_to_a_neighbor_being_built_opens_its_input() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let size = (1600, 1200);
    let p = photo(size);
    let (white, kind) = as_shot(&p.src);
    let defaults = Edit::defaults(kind);
    let fit = ViewSpec::Fit((240, 180));
    t.open(1, photo((320, 240))).unwrap();
    let pending = t.prefetch(2, p, white).unwrap();
    assert!(!t.open_prefetching(9).unwrap(), "nothing queued for 9");
    assert!(t.open_prefetching(2).unwrap());
    assert_eq!(
        pending.recv_timeout(WAIT).unwrap().unwrap().map(|p| p.id),
        None
    );
    let f = t.develop(request(1, 2, &defaults, fit)).unwrap().unwrap();
    // Opened from its input: a settled white needs nothing more.
    let mut warm = defaults.clone();
    warm.white_balance.mode = WhiteBalanceMode::Custom;
    warm.white_balance.temp = 4300.0;
    assert!(t.develop(request(2, 2, &warm, fit)).unwrap().is_some());
    t.open(3, photo(size)).unwrap();
    let g = t.develop(request(3, 3, &defaults, fit)).unwrap().unwrap();
    assert!(frame_of(&f) == frame_of(&g), "as the photo opened again");
}

#[test]
fn a_render_does_not_wait_for_a_prefetch() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = thread() else { return };
    let a = photo((320, 240));
    let (_, kind) = as_shot(&a.src);
    let defaults = Edit::defaults(kind);
    t.open(1, a).unwrap();
    let p = photo((4000, 3000));
    let (white, _) = as_shot(&p.src);
    let pending = t.prefetch(2, p, white).unwrap();
    // The build stops between tiles for the render, and resumes after it.
    let f = t.develop(request(1, 1, &defaults, ViewSpec::Fit((160, 120))));
    assert!(f.unwrap().is_some());
    assert!(
        matches!(pending.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "the render waited for the whole build"
    );
    assert!(pending.recv_timeout(WAIT).unwrap().unwrap().is_some());
}
