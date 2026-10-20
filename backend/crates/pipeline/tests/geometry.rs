//! The edit's geometry on the GPU (DEV-01, Geometry): quarter turns, flip,
//! crop and straighten composed into the sampler's one affine.
//!
//! - Straightened crops, turned and flipped, match `reference::sampling` at
//!   Fit (pyramid levels, or level 0 for a tight crop) and at 1:1 (level-0
//!   tiles) within two f16 steps.
//! - A straight crop on whole pixels at 1:1 is the uncropped view's pixels,
//!   bit for bit.
//! - A 2560 x 1440 view at 1:1, straightened 45°, fits the level-0 cache.
//! - The base layer covers the cropped image only, and a view whose frame
//!   is ahead of the base's (mid-gesture) still reads it.

use std::sync::{Mutex, MutexGuard};

use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::geometry::{Crop, Geometry, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::{Level0Cache, TILE, slots_for};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{Look, Profile};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{Frame, ViewSpec, plan_view, sample};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => {
            eprintln!("adapter: {} ({})", gpu.info.name, gpu.info.backend);
            Some((lock, gpu))
        }
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// Two f16 steps, plus 1e-5 for the GPU's f32 Lanczos weights (as
/// tests/view.rs).
fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= 2.0 * step + 1e-5
}

/// Level 0 of `src`, through an uncropped 1:1 view of the whole image (an
/// exact copy, tests/level0.rs).
fn level0_image(
    gpu: &Gpu,
    stages: &mut RawStages,
    cache: &mut Level0Cache,
    src: &RawSource,
) -> Vec<[f32; 3]> {
    let mut view = ViewStages::new(gpu);
    let spec = ViewSpec::Region {
        out: src.size(),
        zoom: 1.0,
        origin: (0.0, 0.0),
    };
    let s = view.show(gpu, stages, cache, src, 1, 1, spec).unwrap();
    assert_eq!(s.level, 0);
    view.read_sampled(gpu).unwrap()
}

fn geometries() -> Vec<(&'static str, u8, Geometry)> {
    let crop = Crop {
        cx: 0.47,
        cy: 0.52,
        w: 0.6,
        h: 0.5,
        angle: 13.0,
    };
    vec![
        (
            "straightened",
            1,
            Geometry {
                crop,
                ..Geometry::default()
            },
        ),
        (
            "turned and flipped",
            1,
            Geometry {
                quarter_turns: 1,
                flip_horizontal: true,
                crop: Crop {
                    angle: -30.0,
                    ..crop
                },
            },
        ),
        (
            "file turned, tight",
            6,
            Geometry {
                quarter_turns: 2,
                flip_horizontal: false,
                crop: Crop {
                    w: 0.2,
                    h: 0.15,
                    angle: 41.0,
                    ..crop
                },
            },
        ),
    ]
}

#[test]
fn straightened_crops_match_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let size = src.size();
    let t = camera_transform(&src.color, None);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let level0 = level0_image(&gpu, &mut stages, &mut cache, &src);
    let mut view = ViewStages::new(&gpu);
    for (name, file, g) in geometries() {
        let crop = fit(g.crop, size, g.orientation(file)).expect("fits");
        let frame = Frame::of(&Geometry { crop, ..g }, file);
        let framed = frame.pixel_size(size);
        for spec in [
            ViewSpec::Fit((framed.0 / 2, framed.1 / 2)),
            ViewSpec::Fit((framed.0 / 5, framed.1 / 5)),
            ViewSpec::Fit((framed.0 * 3 / 2, framed.1 * 3 / 2)),
            ViewSpec::Region {
                out: (160, 120),
                zoom: 1.0,
                origin: (17.0, 9.0),
            },
        ] {
            let out = spec.out();
            let s = view
                .show(&gpu, &mut stages, &mut cache, &src, 1, frame, spec)
                .unwrap();
            let got = view.read_sampled(&gpu).unwrap();
            let want = if s.level == 0 {
                sample(&level0, size, &s, out)
            } else {
                let p = stages.pyramid().unwrap();
                sample(&p.read_level(&gpu, s.level).unwrap(), s.level_size, &s, out)
            };
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                for c in 0..3 {
                    assert!(
                        f16_close(g[c], w[c]),
                        "{name}, {spec:?} (level {}), pixel {i}: {g:?} vs {w:?}",
                        s.level
                    );
                }
            }
            eprintln!("{name}, {spec:?}: level {}", s.level);
        }
    }
}

#[test]
fn a_straight_crop_at_one_to_one_is_a_copy() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let size = src.size();
    let t = camera_transform(&src.color, None);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    for (file, turns, flip) in [(1, 0, false), (1, 1, true), (6, 3, false)] {
        let g = Geometry {
            quarter_turns: turns,
            flip_horizontal: flip,
            crop: Crop::FULL,
        };
        let o = g.orientation(file);
        let oriented = genzo_pipeline::reference::sampling::shown(size, o);
        let (ow, oh) = (f64::from(oriented.0), f64::from(oriented.1));
        // 400 x 300 source pixels whose top left is at (250, 130).
        let crop = Crop {
            cx: 450.0 / ow,
            cy: 280.0 / oh,
            w: 400.0 / ow,
            h: 300.0 / oh,
            angle: 0.0,
        };
        let frame = Frame::of(&Geometry { crop, ..g }, file);
        let pan = |origin| ViewSpec::Region {
            out: (300, 200),
            zoom: 1.0,
            origin,
        };
        view.show(
            &gpu,
            &mut stages,
            &mut cache,
            &src,
            1,
            frame,
            pan((40.0, 60.0)),
        )
        .unwrap();
        let cropped = view.read_sampled(&gpu).unwrap();
        view.show(
            &gpu,
            &mut stages,
            &mut cache,
            &src,
            1,
            o,
            pan((290.0, 190.0)),
        )
        .unwrap();
        let uncropped = view.read_sampled(&gpu).unwrap();
        let same = cropped
            .iter()
            .zip(&uncropped)
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "file {file}, turns {turns}, flip {flip}");
    }
}

/// The level-0 tiles a straightened view needs: its bounding box in the
/// image grows by up to √2 at 45°, and must still fit the cache, which
/// holds 448 tiles on an 8192 px device, so 1:1 does not fall back.
#[test]
fn a_view_straightened_45_degrees_fits_the_tile_cache() {
    let level0 = (8064, 6048);
    let levels: Vec<(u32, u32)> = (1..=5).map(|l| (8064 >> l, 6048 >> l)).collect();
    let crop = fit(
        Crop {
            angle: 45.0,
            ..Crop::FULL
        },
        level0,
        1,
    )
    .unwrap();
    let frame = Frame {
        orientation: 1,
        crop: Some(crop),
    };
    let out = (2560, 1440);
    let framed = frame.pixel_size(level0);
    let spec = ViewSpec::Region {
        out,
        zoom: 1.0,
        origin: (
            f64::from(framed.0 - out.0) / 2.0,
            f64::from(framed.1 - out.1) / 2.0,
        ),
    };
    let s = plan_view(level0, &levels, frame, spec);
    assert_eq!(s.level, 0);
    let tiles = Level0Cache::tiles_for(s.source_rect(out), level0).len() as u32;
    let straight =
        Level0Cache::tiles_for(plan_view(level0, &levels, 1, spec).source_rect(out), level0).len()
            as u32;
    let (across, down) = slots_for(8192);
    eprintln!(
        "45°: {tiles} tiles of {TILE} px, straight {straight}; the cache holds {}",
        across * down
    );
    assert!(tiles <= across * down, "{tiles}");
}

/// A linear source of neutral luminance `0.18 · 2^t(x)` (baseline exposure
/// +3 EV, as tests/base.rs).
fn stops(size: (u32, u32), t: impl Fn(u32) -> f64) -> RawSource {
    use genzo_imgio::raw::{Levels, Rect};
    use genzo_pipeline::raw::Linearization;
    let levels = Levels {
        black: vec![512.0; 3],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![16383],
    };
    let data = (0..size.1)
        .flat_map(|_| (0..size.0).map(&t))
        .flat_map(|t| {
            let v = (0.18 * t.exp2() / 8.0).clamp(0.0, 1.0);
            [(512.0 + v * 15871.0).round() as u16; 3]
        })
        .collect();
    let mut src = RawSource {
        stored: size,
        samples: 3,
        data,
        cfa: None,
        active: Rect {
            top: 0,
            left: 0,
            bottom: size.1,
            right: size.0,
        },
        linearization: Linearization::new(&levels, 3, size).unwrap(),
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color: Default::default(),
        encoding: None,
    };
    src.color.baseline_exposure = 3.0;
    src
}

/// The base layer comes from the cropped image only: a bright band outside
/// the crop changes none of its coefficients, though without the crop it
/// does. And a view whose frame is ahead of the base's, as during a crop
/// gesture, still reads the base, edges clamped.
#[test]
fn the_base_layer_covers_the_crop() {
    let Some((_lock, gpu)) = gpu() else { return };
    let size = (1200, 800);
    let plain = stops(size, |x| if x < 300 { -1.0 } else { 0.5 });
    // The band starts 120 px past the crop's right edge (x 600).
    let banded = stops(size, |x| match x {
        0..300 => -1.0,
        300..720 => 0.5,
        _ => 4.0,
    });
    let crop = Crop {
        cx: 0.25,
        cy: 0.5,
        w: 0.5,
        h: 1.0,
        angle: 0.0,
    };
    let frame = Frame {
        orientation: 1,
        crop: Some(crop),
    };
    let coefficients = |src: &RawSource, frame: Frame| {
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        let p = stages
            .render_pyramid(&gpu, src, &camera_transform(&src.color, None))
            .unwrap();
        let mut layer = BaseLayer::new(&gpu);
        layer.build(&gpu, p, src.size(), frame);
        (layer.size(), layer.read(&gpu).unwrap())
    };
    let (a_size, a) = coefficients(&plain, frame);
    let (_, b) = coefficients(&banded, frame);
    assert_eq!(a_size, (600, 800), "the crop's own size");
    assert_eq!(a, b, "the band outside the crop changes nothing");
    let (_, a_whole) = coefficients(&plain, Frame::from(1));
    let (_, b_whole) = coefficients(&banded, Frame::from(1));
    assert_ne!(a_whole, b_whole, "without the crop it does");

    // Mid-gesture: the view already shows a straightened crop, the base is
    // still the old crop's.
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages
        .render_pyramid(&gpu, &banded, &camera_transform(&banded.color, None))
        .unwrap();
    let mut layer = BaseLayer::new(&gpu);
    layer.build(&gpu, p, size, frame);
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    let moved = Frame {
        orientation: 1,
        crop: Some(Crop {
            cx: 0.6,
            w: 0.7,
            angle: 20.0,
            ..crop
        }),
    };
    let moved = Frame {
        crop: Some(fit(moved.crop.unwrap(), size, 1).unwrap()),
        ..moved
    };
    view.show(
        &gpu,
        &mut stages,
        &mut cache,
        &banded,
        1,
        moved,
        ViewSpec::Fit((420, 280)),
    )
    .unwrap();
    let look = Look {
        highlights: -100.0,
        shadows: 100.0,
        ..Look::with_profile(Profile::GenzoLinear)
    };
    view.tone(&gpu, &look, None, Some(&layer));
    let toned = view.read_latest(&gpu).unwrap();
    assert!(toned.iter().flatten().all(|v| v.is_finite()));
}
