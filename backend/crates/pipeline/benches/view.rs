//! Criterion benchmark for the view (DEV-01): the pyramid sampled into a
//! 2560 x 1920 view, then stage 9 into its RGBA8 frame, with the gamut
//! compression (a RAW photo) and without (an sRGB JPEG); and stages 6 and 7
//! (DEV-02's base curve with Genzo Standard). From a 48 MP
//! pyramid: level 2 (2016 x 1512) is too small, so sampling reads level 1
//! (4032 x 3024).
//!
//! Measured on the RTX 3060 (2026-10-19): sampling 2.8 ms, stage 9 1.3 ms
//! with the gamut compression, 0.33 ms without. Stage 6 0.39 ms with
//! Genzo Standard and 0.31 ms with Genzo Linear, so the base curve costs
//! about 0.07 ms (DEV-02 criterion 10 allows 1 ms); stage 7 0.39 ms.
//!
//! Skipped without a GPU adapter. `GENZO_ADAPTER=software` benchmarks the
//! software adapter.

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_color::output::Target;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};
use genzo_pipeline::look::{BaseCurve, Look, Profile};

fn view(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let src = scene_raw(8064, 6048);
    let t = camera_transform(&src.color, None);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut view = ViewStages::new(&gpu);
    let out = (2560, 1920);
    let wait = |gpu: &Gpu| {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    };
    view.sample(&gpu, p, 1, out);
    view.output(&gpu, Target::Srgb, true, true);
    wait(&gpu);
    let mut group = c.benchmark_group(format!("view 2560x1920 on {}", gpu.info.name));
    group.sample_size(20);
    group.bench_function("sample the pyramid", |b| {
        b.iter(|| {
            view.sample(&gpu, p, 1, out);
            wait(&gpu);
        })
    });
    for (name, compress) in [("stage 9, gamut compressed", true), ("stage 9", false)] {
        group.bench_function(name, |b| {
            b.iter(|| {
                view.output(&gpu, Target::Srgb, compress, true);
                wait(&gpu);
            })
        });
    }
    let base = BaseCurve::new(&src.color);
    let standard = Look {
        exposure: 0.3,
        contrast: 25.0,
        whites: -20.0,
        blacks: 15.0,
        vibrance: 30.0,
        saturation: 10.0,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    let linear = Look::with_profile(Profile::GenzoLinear);
    for (name, look) in [
        ("stage 6, Genzo Standard", standard),
        (
            "stage 6, Genzo Linear",
            Look {
                contrast: 25.0,
                ..linear
            },
        ),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                view.tone(&gpu, &look, Some(&base));
                wait(&gpu);
            })
        });
    }
    group.bench_function("stage 7", |b| {
        b.iter(|| {
            view.color(&gpu, &standard);
            wait(&gpu);
        })
    });
    group.finish();
}

criterion_group!(benches, view);
criterion_main!(benches);
