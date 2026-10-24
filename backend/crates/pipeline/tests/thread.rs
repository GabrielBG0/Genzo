//! Develop on the render thread (`gpu::thread`): latest-wins coalescing of
//! Develop renders, and renders for a photo that is no longer open.

use std::sync::Mutex;

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase, WhiteBalanceMode};
use genzo_pipeline::gpu::develop::{Photo, Readback};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::thread::{DevelopRequest, RenderThread};
use genzo_pipeline::gpu::{AdapterChoice, GpuError};
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
