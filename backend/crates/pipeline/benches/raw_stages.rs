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
use genzo_color::encoding::InputEncoding;
use genzo_model::frame::Rect;
use genzo_pipeline::camera_color::{camera_transform, encoded_transform};
use genzo_pipeline::geometry::{Crop, Geometry, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until, level0_reach};
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw};
use genzo_pipeline::gpu::{AdapterChoice, Gpu};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{Frame, ViewSpec, plan_view_without_level0};

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
    for (name, size, xtrans) in [
        ("12MP", (4032, 3024), false),
        ("48MP", (8064, 6048), false),
        // The X-T5's 40 MP, through Markesteijn.
        ("X-Trans 40MP", (7872, 5196), true),
    ] {
        let src = if xtrans {
            scene_xtrans_raw(size.0, size.1)
        } else {
            scene_raw(size.0, size.1)
        };
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
    // A non-RAW image: a 24 MP sRGB JPEG's codes, through the curves.
    let (w, h) = (6000u32, 4000u32);
    let rgba: Vec<u16> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [x * 65535 / w, y * 65535 / h, (x ^ y) & 0xffff, 65535].map(|v| v as u16)
        })
        .collect();
    let enc = InputEncoding::srgb();
    let src = RawSource::from_encoded((w, h), &rgba, &enc).unwrap();
    let t = encoded_transform(&enc);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    group.bench_function("sRGB 24MP", |b| {
        b.iter(|| {
            stages.render_pyramid(&gpu, &src, &t).unwrap();
        })
    });
    group.finish();
}

/// The settled white balance at Fit (DEV-01, Settled): the pyramid rebuilt
/// in place, the first part (the tiles a 2560 x 1440 Fit view of a crop a
/// third of each side and its base layer read) against the whole rebuild,
/// at 48 MP. Without a crop the first part is every tile.
fn rebuild(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let src = scene_raw(8064, 6048);
    let size = src.size();
    let old = camera_transform(&src.color, None);
    let mut new = old.clone();
    new.multipliers = [2.0, 1.0, 1.5];
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &old).unwrap();
    let crop = Crop {
        cx: 0.5,
        cy: 0.5,
        w: 1.0 / 3.0,
        h: 1.0 / 3.0,
        angle: 0.0,
    };
    let g = Geometry {
        crop: fit(crop, size, 1).expect("fits"),
        ..Geometry::default()
    };
    let frame = Frame::of(&g, 1);
    let out = (2560, 1440);
    let p = stages.pyramid().unwrap();
    let s = plan_view_without_level0(size, &p.level_sizes(), frame, ViewSpec::Fit(out));
    let mut layer = BaseLayer::new(&gpu);
    layer.build(&gpu, p, size, frame);
    let [a, b] = [
        level0_reach(&s, out, size),
        level0_reach(layer.sampling().unwrap(), layer.size(), size),
    ]
    .map(|r| r.expect("a pyramid level"));
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let reach = Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    };
    let mut group = c.benchmark_group(format!("white balance rebuild 48MP on {}", gpu.info.name));
    group.sample_size(10);
    group.bench_function("first part, crop of a third", |b| {
        b.iter(|| stages.rebuild_first(&gpu, &src, &new, Some(reach)).unwrap())
    });
    group.bench_function("whole", |b| {
        b.iter(|| {
            let mut r = stages.rebuild_first(&gpu, &src, &new, None).unwrap();
            stages.rebuild_rest(&gpu, &src, &mut r, || false).unwrap()
        })
    });
    group.finish();
}

/// A prefetched neighbor at 48 MP (DEV-01, Prefetched neighbors): its
/// level 1 rendered tile by tile and read back beside the held pyramid,
/// and the switch's GPU part, that level 1 uploaded and levels 2 and up
/// computed. The switch is held to 150 ms with its Fit render (plan,
/// performance budgets). Measured on the RTX 3060 (2026-10-24): 124 ms
/// for the level 1, 22.5 ms for the switch.
fn prefetch(c: &mut Criterion) {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let src = scene_raw(8064, 6048);
    let mut t = camera_transform(&src.color, None);
    t.multipliers = [2.0, 1.0, 1.5];
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut build = || {
        let mut b = stages.start_level1(&gpu, &src, &t);
        assert!(
            stages
                .continue_level1(&gpu, &src, &mut b, || false)
                .unwrap()
        );
        b.finish()
    };
    let level1 = build();
    let mut group = c.benchmark_group(format!("prefetch 48MP on {}", gpu.info.name));
    group.sample_size(10);
    group.bench_function("neighbor's level 1, read back", |b| b.iter(&mut build));
    group.bench_function("switch: level 1 uploaded, levels 2 and up", |b| {
        b.iter(|| {
            stages.upload_pyramid(&gpu, &level1).unwrap();
        })
    });
    group.finish();
}
criterion_group!(benches, stages, pyramid, rebuild, prefetch);
criterion_main!(benches);
