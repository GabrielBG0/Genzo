//! TEMPORARY: why stages 6 and 7 lose the device on CI's WARP (Windows
//! Server 2022) but not on Windows 11. Logs every uncaptured error and the
//! device-lost reason instead of panicking, step by step. Removed before
//! the PR is ready.

use std::sync::Arc;

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
    eprintln!("diag adapter: {} ({})", gpu.info.name, gpu.info.backend);
    gpu.device
        .on_uncaptured_error(Arc::new(|e| eprintln!("diag uncaptured: {e}")));
    gpu.device
        .set_device_lost_callback(|r, m| eprintln!("diag device lost: {r:?}: {m}"));
    let poll = |gpu: &Gpu, step: &str| {
        let r = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        eprintln!("diag {step}: poll {r:?}");
    };
    let src = scene_raw(200, 150);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages
        .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
        .unwrap();
    poll(&gpu, "pyramid");
    let mut view = ViewStages::new(&gpu);
    poll(&gpu, "view new");
    view.sample(&gpu, p, 1, (100, 75));
    poll(&gpu, "sample");
    eprintln!(
        "diag read sampled: {:?}",
        view.read_latest(&gpu).map(|v| v.len())
    );
    let linear = Look::with_profile(Profile::GenzoLinear);
    view.color(&gpu, &linear);
    poll(&gpu, "color (stage 7) on sampled");
    eprintln!(
        "diag read colored: {:?}",
        view.read_latest(&gpu).map(|v| v.len())
    );
    view.tone(&gpu, &linear, None);
    poll(&gpu, "tone (stage 6), linear");
    eprintln!(
        "diag read toned: {:?}",
        view.read_latest(&gpu).map(|v| v.len())
    );
    let base = BaseCurve::new(&src.color);
    view.tone(
        &gpu,
        &Look::with_profile(Profile::GenzoStandard),
        Some(&base),
    );
    poll(&gpu, "tone (stage 6), standard");
    eprintln!(
        "diag read toned standard: {:?}",
        view.read_latest(&gpu).map(|v| v.len())
    );
}
