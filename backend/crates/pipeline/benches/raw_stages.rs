//! Criterion benchmark for stages 1 to 3 on the GPU, tile by tile, on a
//! 12 MP synthetic Bayer sensor (DEV-01, Tiling): the settled render's work
//! up to the demosaic. Each tile is read back for now; plan step 5b2 feeds
//! the tiles to stage 4 and the pyramid instead.
//!
//! Measured on the RTX 3060 (2026-10-18), stages 1 to 3: 107 ms per 12 MP
//! frame. Copying the mapped RGBA32F tiles out takes about 58 ms of that and
//! goes away in 5b2; the GPU work and the copy into the readback buffer about
//! 18 ms; uploading the raw tiles (straight from the stored rows, no
//! repacking) about 1 ms.
//!
//! Skipped without a GPU adapter. `GENZO_ADAPTER=software` benchmarks the
//! software adapter.

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};

fn stages(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let src = scene_raw(4032, 3024);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages
        .render(&gpu, &src, [2.0, 1.0, 1.5], Until::Stage3)
        .unwrap();
    let mut group = c.benchmark_group(format!("raw stages 12MP on {}", gpu.info.name));
    group.sample_size(10);
    for (name, until) in [
        ("stages 1-2", Until::Stage2),
        ("stages 1-3 (RCD)", Until::Stage3),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| stages.render(&gpu, &src, [2.0, 1.0, 1.5], until).unwrap())
        });
    }
    group.finish();
}

/// The settled render's raw part as it runs: stages 1 to 4 into the
/// pyramid, nothing read back.
fn pyramid(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut group = c.benchmark_group(format!("pyramid on {}", gpu.info.name));
    group.sample_size(10);
    for (name, size) in [("12MP", (4032, 3024)), ("48MP", (8064, 6048))] {
        let src = scene_raw(size.0, size.1);
        let mut t = camera_transform(&src.color, None);
        t.multipliers = [2.0, 1.0, 1.5];
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        stages.render_pyramid(&gpu, &src, &t).unwrap();
        group.bench_function(name, |b| {
            b.iter(|| {
                stages.render_pyramid(&gpu, &src, &t).unwrap();
            })
        });
    }
    group.finish();
}

criterion_group!(benches, stages, pyramid);
criterion_main!(benches);
