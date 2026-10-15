//! Criterion benchmarks for the spike pipeline, one per re-run entry point
//! ([`Stage`]), on a 12 MP synthetic raw into a 2560 px viewport. Each
//! measures a whole render as the render thread sees it: the stages that
//! re-run, the GPU wait and the readback. GPU-only time comes from
//! `genzo-cli bench gpu`, which reads timestamp queries.
//!
//! Skipped without a GPU adapter. `GENZO_ADAPTER=software` benchmarks the
//! software adapter.

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_pipeline::gpu::spike::{SpikeParams, SpikePipeline, Stage};
use genzo_pipeline::gpu::synthetic::scene_mosaic;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};

const VIEW: (u32, u32) = (2560, 2560);

fn stages(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &scene_mosaic(4032, 3024));
    p.render(&gpu, SpikeParams::default(), VIEW).unwrap();

    let mut group = c.benchmark_group(format!("spike 12MP to 2560px on {}", gpu.info.name));
    group.sample_size(20);
    let mut ev = 0.0f32;
    group.bench_function("output (exposure drag)", |b| {
        b.iter(|| {
            ev = if ev > 0.5 { 0.0 } else { ev + 0.01 };
            let f = p
                .render(
                    &gpu,
                    SpikeParams {
                        exposure_ev: ev,
                        interactive: true,
                        ..Default::default()
                    },
                    VIEW,
                )
                .unwrap();
            assert_eq!(f.from, Stage::Output);
        })
    });
    let mut r = 1.0f32;
    group.bench_function("viewport (white balance drag)", |b| {
        b.iter(|| {
            r = if r > 1.5 { 1.0 } else { r + 0.01 };
            let f = p
                .render(
                    &gpu,
                    SpikeParams {
                        wb: [r, 1.0, 1.0],
                        interactive: true,
                        ..Default::default()
                    },
                    VIEW,
                )
                .unwrap();
            assert_eq!(f.from, Stage::Viewport);
        })
    });
    group.bench_function("linearize (settled white balance)", |b| {
        b.iter(|| {
            r = if r > 1.5 { 1.0 } else { r + 0.01 };
            let f = p
                .render(
                    &gpu,
                    SpikeParams {
                        wb: [r, 1.0, 1.0],
                        ..Default::default()
                    },
                    VIEW,
                )
                .unwrap();
            assert_eq!(f.from, Stage::Linearize);
        })
    });
    group.finish();
}

criterion_group!(benches, stages);
criterion_main!(benches);
