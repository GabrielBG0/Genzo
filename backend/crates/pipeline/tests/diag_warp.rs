//! TEMPORARY: why stages 6 and 7 lose the device on CI's WARP (Windows
//! Server 2022) but not on Windows 11. Logs every uncaptured error and the
//! device-lost reason, step by step, then fails with the log so CI shows it.
//! Removed before the PR is ready.

use std::sync::{Arc, Mutex};

use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};
use genzo_pipeline::look::{BaseCurve, Look, Profile};

#[test]
fn warp_diagnostics() {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        return;
    };
    let log = Arc::new(Mutex::new(format!(
        "adapter: {} ({})\n",
        gpu.info.name, gpu.info.backend
    )));
    let l = log.clone();
    gpu.device.on_uncaptured_error(Arc::new(move |e| {
        l.lock().unwrap().push_str(&format!("  uncaptured: {e}\n"))
    }));
    let l = log.clone();
    gpu.device.set_device_lost_callback(move |r, m| {
        l.lock()
            .unwrap()
            .push_str(&format!("  device lost: {r:?}: {m}\n"))
    });
    let step = |gpu: &Gpu, view: &ViewStages, name: &str| {
        let poll = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let read = view.read_latest(gpu).map(|v| v.len());
        log.lock()
            .unwrap()
            .push_str(&format!("{name}: poll {poll:?}, read {read:?}\n"));
    };
    let src = scene_raw(200, 150);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages
        .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
        .unwrap();
    let mut view = ViewStages::new(&gpu);
    view.sample(&gpu, p, 1, (100, 75));
    step(&gpu, &view, "sample");
    let linear = Look::with_profile(Profile::GenzoLinear);
    view.color(&gpu, &linear);
    step(&gpu, &view, "stage 7, color");
    let mono = Look {
        monochrome: true,
        ..linear
    };
    view.color(&gpu, &mono);
    step(&gpu, &view, "stage 7, B&W");
    view.tone(&gpu, &linear, None);
    step(&gpu, &view, "stage 6, linear");
    let contrast = Look {
        contrast: 30.0,
        ..linear
    };
    view.tone(&gpu, &contrast, None);
    step(&gpu, &view, "stage 6, linear, contrast");
    let base = BaseCurve::new(&src.color);
    let standard = Look::with_profile(Profile::GenzoStandard);
    view.tone(&gpu, &standard, Some(&base));
    step(&gpu, &view, "stage 6, standard");
    view.tone(
        &gpu,
        &Look {
            contrast: 30.0,
            ..standard
        },
        Some(&base),
    );
    step(&gpu, &view, "stage 6, standard, contrast");
    let vivid = Look {
        vibrance: 50.0,
        saturation: 20.0,
        ..standard
    };
    view.color(&gpu, &vivid);
    step(&gpu, &view, "stage 7, vibrance and saturation");
    panic!(
        "diagnostics (always fails, to show the log):\n{}",
        log.lock().unwrap()
    );
}
