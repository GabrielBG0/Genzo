//! Criterion benchmark for the view (DEV-01): the pyramid sampled into a
//! 2560 x 1920 view, then stage 9 into its RGBA8 frame, with the gamut
//! compression (a RAW photo) and without (an sRGB JPEG); stages 6 and 7
//! (DEV-02's base curve with Genzo Standard); and the base layer. From a 48 MP
//! pyramid: level 2 (2016 x 1512) is too small, so sampling reads level 1
//! (4032 x 3024).
//!
//! Measured on the RTX 3060 (2026-10-19): sampling 2.8 ms, stage 9 1.3 ms
//! with the gamut compression, 0.33 ms without. Stage 6 0.39 ms with
//! Genzo Standard and 0.31 ms with Genzo Linear, so the base curve costs
//! about 0.07 ms (DEV-02 criterion 10 allows 1 ms); stage 7 0.39 ms.
//! The base layer of Highlights and Shadows (1536 x 1152 from the 48 MP
//! pyramid, paid on a white balance, crop or turn) 2.85 ms; stage 6 reading
//! it 0.45 ms, about 0.06 ms over Genzo Standard alone (2026-10-20).
//! Sampling a crop straightened 20 degrees 1.8 ms: the crop is smaller than
//! the image, so its filter is narrower (2026-10-20).
//! Sampling with the view resized first, as each crop delta at Fit does
//! now that the frame keeps the crop's aspect: 2.95 ms against 2.81 ms
//! (2026-10-26).
//! Readback through the frame's staging buffers: the whole frame (what Fit
//! sends), and one column of 256 px tiles (what a 256 px pan at 1:1 sends):
//! 11.5 ms for the 19.7 MB frame (M0 measured about 9 ms for a 2560 x 1440
//! one, 14.7 MB), 0.94 ms for the column (2026-10-22).
//! Stage 9 and its readback, as a drag at Fit sends them: 3.2 ms for the
//! half-size preview (1280 x 960, 4.9 MB), 12.0 ms for the full frame
//! (2026-10-22).
//! Stage 9 counting the histogram (DEV-03), its clear and copy included:
//! 1.47 ms with the gamut compression against 1.35 ms, 0.50 ms without it
//! against 0.37 ms, so about 0.12 ms at 2560 x 1920 (DEV-03 criterion 9
//! allows 0.2 ms at 2560 x 1440); the J overlay adds nothing measurable,
//! 0.50 ms (2026-10-31).
//!
//! Skipped without a GPU adapter. `GENZO_ADAPTER=software` benchmarks the
//! software adapter.

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_color::output::Target;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::geometry::{Crop, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::readback::visible_tiles;
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu};
use genzo_pipeline::histogram::{Overlay, Preview};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::reference::sampling::{Frame, ViewSpec, plan_view_without_level0};

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
    // A white balance drag: the same sampling with the ratio matrix after.
    view.set_ratio(Some([
        [1.21, -0.08, -0.02],
        [0.03, 0.98, -0.01],
        [0.01, -0.06, 0.82],
    ]));
    group.bench_function("sample the pyramid, white balance drag", |b| {
        b.iter(|| {
            view.sample(&gpu, p, 1, out);
            wait(&gpu);
        })
    });
    view.set_ratio(None);
    // A crop dragged at Fit: the frame's size follows the crop's aspect,
    // so each delta resizes the view's textures before sampling.
    let mut narrower = false;
    group.bench_function("sample the pyramid, resized each time", |b| {
        b.iter(|| {
            narrower = !narrower;
            view.sample(&gpu, p, 1, (out.0 - u32::from(narrower), out.1));
            wait(&gpu);
        })
    });
    view.sample(&gpu, p, 1, out);
    // A crop straightened by 20 degrees, at the same view size: the
    // pyramid's level 1 sampled along a rotated affine.
    let crop = fit(
        Crop {
            angle: 20.0,
            ..Crop::FULL
        },
        src.size(),
        1,
    )
    .unwrap();
    let straightened = Frame {
        orientation: 1,
        crop: Some(crop),
    };
    let s = plan_view_without_level0(
        src.size(),
        &p.level_sizes(),
        straightened,
        ViewSpec::Fit(out),
    );
    group.bench_function("sample a crop straightened 20 degrees", |b| {
        b.iter(|| {
            view.sample_with(&gpu, p, None, &s, out);
            wait(&gpu);
        })
    });
    view.sample(&gpu, p, 1, out);
    for (name, compress) in [("stage 9, gamut compressed", true), ("stage 9", false)] {
        group.bench_function(name, |b| {
            b.iter(|| {
                view.output(&gpu, Target::Srgb, compress, true);
                wait(&gpu);
            })
        });
    }
    // DEV-03: the histogram counted, then with J's overlay drawn too.
    view.set_counting(true);
    for (name, compress) in [
        ("stage 9, gamut compressed, counting", true),
        ("stage 9, counting", false),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| {
                view.output(&gpu, Target::Srgb, compress, true);
                wait(&gpu);
            })
        });
    }
    view.set_overlay(Overlay {
        highlights: Some([255, 107, 107, 255]),
        shadows: Some([91, 157, 255, 255]),
        preview: Preview::None,
    });
    group.bench_function("stage 9, counting, J overlay", |b| {
        b.iter(|| {
            view.output(&gpu, Target::Srgb, false, true);
            wait(&gpu);
        })
    });
    view.set_overlay(Overlay::default());
    view.set_counting(false);
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
                view.tone(&gpu, &look, Some(&base), None);
                wait(&gpu);
            })
        });
    }
    // The base layer: built on a white balance, crop or turn, not per
    // slider frame; then stage 6 reading it, which every Highlights and
    // Shadows frame pays (criterion 14's 16 ms).
    let mut layer = BaseLayer::new(&gpu);
    group.bench_function("base layer, 1536 x 1152", |b| {
        b.iter(|| {
            layer.build(&gpu, p, src.size(), 1);
            wait(&gpu);
        })
    });
    let local = Look {
        highlights: -60.0,
        shadows: 40.0,
        ..standard
    };
    group.bench_function("stage 6, Highlights and Shadows", |b| {
        b.iter(|| {
            view.tone(&gpu, &local, Some(&base), Some(&layer));
            wait(&gpu);
        })
    });
    group.bench_function("stage 7", |b| {
        b.iter(|| {
            view.color(&gpu, &standard);
            wait(&gpu);
        })
    });
    view.output(&gpu, Target::Srgb, true, true);
    wait(&gpu);
    group.bench_function("readback, the whole frame", |b| {
        b.iter(|| view.read_frame(&gpu).unwrap())
    });
    let column: Vec<_> = visible_tiles((0, 0), out, out)
        .into_iter()
        .filter(|t| t.frame.x == out.0 - 256)
        .map(|t| t.frame)
        .collect();
    assert_eq!(column.len(), 8);
    group.bench_function("readback, one column of tiles", |b| {
        b.iter(|| view.read_rects(&gpu, &column).unwrap())
    });
    // The drag preview at Fit: stage 9 at half size and its readback.
    group.bench_function("half-size preview, stage 9 and readback", |b| {
        b.iter(|| {
            view.output_half(&gpu, Target::Srgb, true, true);
            view.read_frame(&gpu).unwrap()
        })
    });
    group.bench_function("full frame, stage 9 and readback", |b| {
        b.iter(|| {
            view.output(&gpu, Target::Srgb, true, true);
            view.read_frame(&gpu).unwrap()
        })
    });
    group.finish();
}

criterion_group!(benches, view);
criterion_main!(benches);
