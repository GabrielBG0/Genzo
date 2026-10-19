//! Criterion benchmark for 1:1 (DEV-01, criterion 14: newly exposed area
//! under 100 ms): a 2560 x 1440 view at 1:1, its level-0 tiles rendered
//! (stages 1 to 4 at full resolution), sampled and through stage 9.
//!
//! - "cold": every visible tile is new (a photo just opened at 1:1).
//! - "pan 256 px": the view moves one cache tile right, so one column of
//!   tiles is new and the rest are held.
//!
//! On a 48 MP Bayer and a 40 MP X-Trans synthetic sensor. Measured on the
//! RTX 3060 (2026-10-19): Bayer cold 5.9 ms, pan 2.5 ms; X-Trans cold
//! 47 ms, pan 6.6 ms. Skipped without a
//! GPU adapter; `GENZO_ADAPTER=software` benchmarks the software adapter.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_color::output::Target;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw};
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};
use genzo_pipeline::reference::sampling::ViewSpec;

fn one_to_one(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut group = c.benchmark_group(format!("1:1 2560x1440 on {}", gpu.info.name));
    group.sample_size(10);
    for (name, src) in [
        ("Bayer 48MP", scene_raw(8064, 6048)),
        ("X-Trans 40MP", scene_xtrans_raw(7872, 5196)),
    ] {
        let mut t = camera_transform(&src.color, None);
        t.multipliers = [2.0, 1.0, 1.5];
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        stages.render_pyramid(&gpu, &src, &t).unwrap();
        let mut cache = Level0Cache::new(&gpu);
        let mut view = ViewStages::new(&gpu);
        let mut key = 0u64;
        let show = |stages: &mut RawStages,
                    cache: &mut Level0Cache,
                    view: &mut ViewStages,
                    key: u64,
                    x: f64| {
            let spec = ViewSpec::Region {
                out: (2560, 1440),
                zoom: 1.0,
                origin: (x, 2048.0),
            };
            view.show(&gpu, stages, cache, &src, key, 1, spec).unwrap();
            view.output(&gpu, Target::Srgb, true, true);
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        };
        group.bench_function(format!("{name}, cold"), |b| {
            b.iter(|| {
                key += 1;
                show(&mut stages, &mut cache, &mut view, key, 1024.0);
            })
        });
        group.bench_function(format!("{name}, pan 256 px"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    key += 1;
                    show(&mut stages, &mut cache, &mut view, key, 1024.0);
                    let start = Instant::now();
                    show(&mut stages, &mut cache, &mut view, key, 1280.0);
                    total += start.elapsed();
                }
                total
            })
        });
    }
    group.finish();
}

criterion_group!(benches, one_to_one);
criterion_main!(benches);
