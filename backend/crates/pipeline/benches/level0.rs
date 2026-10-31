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
//! 47 ms, pan 6.6 ms.
//!
//! The whole-photo histogram at 1:1 (DEV-03 criterion 9: under 20 ms at
//! 48 MP), through `Develop` as the render thread runs it: the pass itself
//! (sampled at the Fit size from the pyramid, stages 6 and 7, counted),
//! and the settled 1:1 render after it, which samples the view again from
//! the level-0 tiles still held. Measured on the RTX 3060 (2026-10-31):
//! the pass 2.35 ms (the Fit size 1920 x 1440 reads level 2), the render
//! after it 2.57 ms.
//!
//! Skipped without a GPU adapter; `GENZO_ADAPTER=software` benchmarks the
//! software adapter.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_color::output::Target;
use genzo_model::edit::Edit;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::develop::{Develop, Phase, Photo};
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

fn whole_histogram(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut group = c.benchmark_group(format!("1:1 2560x1440 histogram on {}", gpu.info.name));
    group.sample_size(10);
    let mut d = Develop::new(&gpu, TileConfig::default());
    d.open(
        &gpu,
        Photo {
            src: scene_raw(8064, 6048),
            orientation: 1,
        },
    )
    .unwrap();
    let e = Edit {
        exposure: 0.4,
        contrast: 20.0,
        highlights: -40.0,
        shadows: 30.0,
        vibrance: 20.0,
        ..Edit::defaults(d.kind().unwrap())
    };
    let view = ViewSpec::Region {
        out: (2560, 1440),
        zoom: 1.0,
        origin: (1024.0, 2048.0),
    };
    let wait = |gpu: &Gpu| {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    };
    let render = |d: &mut Develop| {
        d.render(&gpu, &e, view, Target::Srgb, Phase::End).unwrap();
        wait(&gpu);
    };
    render(&mut d);
    group.bench_function("Bayer 48MP, the whole-photo pass", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                render(&mut d);
                let start = Instant::now();
                d.count_whole(&gpu).unwrap().expect("counted");
                total += start.elapsed();
            }
            total
        })
    });
    group.bench_function("Bayer 48MP, the 1:1 render after it", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                render(&mut d);
                d.count_whole(&gpu).unwrap().expect("counted");
                let start = Instant::now();
                render(&mut d);
                total += start.elapsed();
            }
            total
        })
    });
    group.finish();
}

criterion_group!(benches, one_to_one, whole_histogram);
criterion_main!(benches);
