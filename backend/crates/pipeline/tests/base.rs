//! The base layer of Highlights and Shadows on the GPU (DEV-01, stage 6,
//! step 2):
//!
//! - its coefficients match `reference::base` on the same working image,
//!   downsampled from a large pyramid and upsampled from a small one, in a
//!   turned orientation too;
//! - stage 6 with Highlights and Shadows matches `reference::look` reading
//!   the GPU's coefficients, at Fit and at 1:1 with a pan;
//! - criterion 12: Highlights at −100 lowers the brightest stop by at least
//!   1 EV and leaves middle gray within 0.05 EV, and Shadows mirrors it;
//!   on a 3-stop step edge the halo stays within 0.15 EV.
//!
//! Criterion 7 with Highlights −100 and Shadows +100 is in `level0.rs`.

use std::sync::{Mutex, MutexGuard};

use genzo_color::look::luminance;
use genzo_imgio::raw::{Levels, Rect};
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::{Linearization, RawSource};
use genzo_pipeline::reference::base::{
    EPSILON, base, coefficients, coefficients_at, floored_t, radius, view_to_working,
};
use genzo_pipeline::reference::look::stage6;
use genzo_pipeline::reference::sampling::{Sampling, ViewSpec, plan, sample, shown};

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

const BLACK: f64 = 512.0;
const WHITE: f64 = 16383.0;

/// A linear DNG-like source whose camera records the working space: pixel
/// (x, y) is `rgb(x, y)` (0 to 1), brightened by `baseline_exposure` in
/// stage 4 so values past white can be made.
fn linear(
    (w, h): (u32, u32),
    baseline_exposure: f64,
    rgb: impl Fn(u32, u32) -> [f32; 3],
) -> RawSource {
    let levels = Levels {
        black: vec![BLACK; 3],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![WHITE as u32],
    };
    let data = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| rgb(x, y))
        .map(|v| (BLACK + f64::from(v.clamp(0.0, 1.0)) * (WHITE - BLACK)).round() as u16)
        .collect();
    let mut src = RawSource {
        stored: (w, h),
        samples: 3,
        data,
        cfa: None,
        active: Rect {
            top: 0,
            left: 0,
            bottom: h,
            right: w,
        },
        linearization: Linearization::new(&levels, 3, (w, h)).unwrap(),
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color: Default::default(),
        encoding: None,
    };
    src.color.baseline_exposure = baseline_exposure;
    src
}

/// A neutral source of luminance `0.18 · 2^t(x)` (t from −3 to +5 stops,
/// baseline exposure +3 EV).
fn stops(size: (u32, u32), t: impl Fn(u32, u32) -> f64) -> RawSource {
    linear(size, 3.0, |x, y| [(0.18 * t(x, y).exp2() / 8.0) as f32; 3])
}

/// The scene of the other tests, as working-space values.
fn scene_source(size: (u32, u32)) -> RawSource {
    linear(size, 0.0, |x, y| scene(x, y, size.0, size.1))
}

fn t_of(rgb: [f32; 3]) -> f64 {
    (luminance(rgb.map(f64::from)) / 0.18).log2()
}

/// The GPU's coefficients against the CPU's, from the CPU's own sampling
/// of the same pyramid level: (largest |ΔĀ|, largest |Δb̄|, largest ΔB).
fn compare_coefficients(gpu: &Gpu, src: &RawSource, o: u8) -> (f64, f64, f64) {
    let mut stages = RawStages::new(gpu, TileConfig::default());
    let p = stages
        .render_pyramid(gpu, src, &camera_transform(&src.color, None))
        .unwrap();
    let mut layer = BaseLayer::new(gpu);
    layer.build(gpu, p, src.size(), o);
    let size = layer.size();
    let s = plan(&p.level_sizes(), o, size);
    assert_eq!(layer.sampling(), Some(&s));
    let level = p.read_level(gpu, s.level).unwrap();
    let working = sample(&level, p.level_size(s.level), &s, size);
    let t: Vec<f64> = working
        .iter()
        .map(|px| floored_t(luminance(px.map(f64::from))))
        .collect();
    let want = coefficients(&t, size, radius(size), EPSILON);
    let got = layer.read(gpu).unwrap();
    let mut worst = (0f64, 0f64, 0f64);
    for i in 0..t.len() {
        let g = got[i].map(f64::from);
        worst.0 = worst.0.max((g[0] - want[i][0]).abs());
        worst.1 = worst.1.max((g[1] - want[i][1]).abs());
        worst.2 = worst.2.max((base(g, t[i]) - base(want[i], t[i])).abs());
    }
    worst
}

/// The coefficients match the reference within what their f16 storage
/// allows: Ā to 2e-3, b̄ and the base itself to 0.02 stops.
#[test]
fn the_coefficients_match_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, size, o) in [
        // Level 1 (1600 x 1200) downsampled to 1536 x 1152.
        ("large", (3200, 2400), 1),
        // Level 1 (320 x 240) upsampled to 640 x 480.
        ("small", (640, 480), 1),
        // A quarter turn: the working image is 480 x 640.
        ("turned", (640, 480), 6),
    ] {
        let worst = compare_coefficients(&gpu, &scene_source(size), o);
        eprintln!(
            "{name}: worst |dA| {:.1e}, |db| {:.1e}, |dB| {:.1e} stops",
            worst.0, worst.1, worst.2
        );
        assert!(
            worst.0 <= 2e-3 && worst.1 <= 0.02 && worst.2 <= 0.02,
            "{name}: {worst:?}"
        );
    }
}

/// Two f16 steps of the larger magnitude, plus 2e-5 for the GPU's f32
/// log2, exp2 and smoothstep (as tests/look.rs).
fn close(got: f32, want: f64) -> bool {
    let step = (want.abs() as f32).max(6.2e-5) / 1024.0;
    (got - want as f32).abs() <= 2.0 * step + 2e-5
}

/// Stage 6 as the reference computes it from the view's sampled values and
/// the GPU's coefficients, against the GPU's stage 6.
fn check_stage6(
    sampled: &[[f32; 3]],
    toned: &[[f32; 3]],
    out: (u32, u32),
    view: &Sampling,
    layer: (&BaseLayer, &[[f32; 2]]),
    look: &Look,
    curve: &BaseCurve,
) {
    let (layer, plane) = layer;
    let plane: Vec<[f64; 2]> = plane.iter().map(|ab| ab.map(f64::from)).collect();
    let m = view_to_working(view, layer.sampling().unwrap());
    for (i, (s, got)) in sampled.iter().zip(toned).enumerate() {
        let p = [
            (i as u32 % out.0) as f64 + 0.5,
            (i as u32 / out.0) as f64 + 0.5,
        ];
        let at = [0, 1].map(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2]);
        let ab = coefficients_at(&plane, layer.size(), at);
        let s = s.map(f64::from);
        let local = base(ab, (luminance(s) / 0.18).log2());
        let want = stage6(s, look, Some(curve), Some(local));
        for c in 0..3 {
            assert!(
                close(got[c], want[c]),
                "pixel {i} channel {c}: {} vs {} from {s:?}",
                got[c],
                want[c]
            );
        }
    }
}

fn local_looks() -> Vec<Look> {
    let standard = Look::with_profile(Profile::GenzoStandard);
    vec![
        Look {
            highlights: -100.0,
            shadows: 100.0,
            ..standard
        },
        Look {
            exposure: 0.7,
            highlights: -60.0,
            shadows: 45.0,
            whites: -10.0,
            blacks: 20.0,
            contrast: 30.0,
            ..standard
        },
        Look {
            exposure: -1.2,
            highlights: 80.0,
            shadows: -100.0,
            ..Look::with_profile(Profile::GenzoLinear)
        },
    ]
}

/// Stage 6 reads the base layer as the reference does: at Fit, and at 1:1
/// panned in a turned orientation, where the view's pixels map onto the
/// working image through both samplings.
#[test]
fn stage_6_matches_the_reference_with_the_base_layer() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_source((1200, 900));
    let t = camera_transform(&src.color, None);
    let curve = BaseCurve::new(&src.color);
    for o in [1, 6] {
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        stages.render_pyramid(&gpu, &src, &t).unwrap();
        let mut layer = BaseLayer::new(&gpu);
        layer.build(&gpu, stages.pyramid().unwrap(), src.size(), o);
        let plane = layer.read(&gpu).unwrap();
        let mut cache = Level0Cache::new(&gpu);
        let mut view = ViewStages::new(&gpu);
        let shown_size = shown(src.size(), o);
        let fit = (shown_size.0 / 3, shown_size.1 / 3);
        for spec in [
            ViewSpec::Fit(fit),
            ViewSpec::Region {
                out: (320, 240),
                zoom: 1.0,
                origin: (301.0, 257.0),
            },
        ] {
            let s = view
                .show(&gpu, &mut stages, &mut cache, &src, 1, o, spec)
                .unwrap();
            let sampled = view.read_latest(&gpu).unwrap();
            for look in local_looks() {
                view.tone(&gpu, &look, Some(&curve), Some(&layer));
                let toned = view.read_latest(&gpu).unwrap();
                check_stage6(
                    &sampled,
                    &toned,
                    spec.out(),
                    &s,
                    (&layer, &plane),
                    &look,
                    &curve,
                );
            }
        }
    }
}

/// Output `t` per pixel of an `out`-sized Fit view of `src` (Genzo Linear,
/// so stage 6's luminance is read before any base curve), for each look,
/// and the sampled `t`.
fn toned_t(
    gpu: &Gpu,
    src: &RawSource,
    out: (u32, u32),
    looks: &[Look],
) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut stages = RawStages::new(gpu, TileConfig::default());
    let p = stages
        .render_pyramid(gpu, src, &camera_transform(&src.color, None))
        .unwrap();
    let mut layer = BaseLayer::new(gpu);
    layer.build(gpu, p, src.size(), 1);
    let mut view = ViewStages::new(gpu);
    view.sample(gpu, p, 1, out);
    let sampled = view
        .read_latest(gpu)
        .unwrap()
        .into_iter()
        .map(t_of)
        .collect();
    let toned = looks
        .iter()
        .map(|look| {
            view.tone(gpu, look, None, Some(&layer));
            view.read_latest(gpu)
                .unwrap()
                .into_iter()
                .map(t_of)
                .collect()
        })
        .collect();
    (sampled, toned)
}

/// Criterion 12: on flat bands far wider than the filter, Highlights at
/// −100 lowers the brightest band by at least 1 EV and Shadows at +100
/// lifts the darkest by at least 1 EV, both leaving middle gray within
/// 0.05 EV.
#[test]
fn criterion_12_highlights_and_shadows() {
    let Some((_lock, gpu)) = gpu() else { return };
    // Bands at −3, 0 and +4 stops, 400 px wide: the radius is 24 px.
    let size = (1200, 400);
    let src = stops(size, |x, _| match x / 400 {
        0 => -3.0,
        1 => 0.0,
        _ => 4.0,
    });
    let linear = Look::with_profile(Profile::GenzoLinear);
    let looks = [
        linear,
        Look {
            highlights: -100.0,
            ..linear
        },
        Look {
            shadows: 100.0,
            ..linear
        },
    ];
    let (_, toned) = toned_t(&gpu, &src, size, &looks);
    // Each band's center.
    let at = |t: &[f64], band: u32| t[(200 * size.0 + 200 + 400 * band) as usize];
    let change = |look: usize, band: u32| at(&toned[look], band) - at(&toned[0], band);
    let (dark, gray, bright) = (at(&toned[0], 0), at(&toned[0], 1), at(&toned[0], 2));
    eprintln!("bands at t {dark:.3}, {gray:.3}, {bright:.3}");
    assert!(gray.abs() < 0.01 && dark < -2.9 && bright > 3.9);
    let (hi_bright, hi_gray) = (change(1, 2), change(1, 1));
    let (lo_dark, lo_gray) = (change(2, 0), change(2, 1));
    eprintln!(
        "Highlights −100: brightest {hi_bright:+.3} EV, gray {hi_gray:+.4}; \
         Shadows +100: darkest {lo_dark:+.3} EV, gray {lo_gray:+.4}"
    );
    assert!(hi_bright <= -1.0 && hi_gray.abs() <= 0.05);
    assert!(lo_dark >= 1.0 && lo_gray.abs() <= 0.05);
    // Each leaves the other end alone.
    assert!(change(1, 0).abs() <= 0.05 && change(2, 2).abs() <= 0.05);
}

/// Criterion 12's halo: on a 3-stop step edge, Highlights at −100 changes
/// the dark side within twice the radius of the edge by at most 0.15 EV
/// more than it changes the dark side far from it. The view is level 1's
/// size, an exact copy of it, so the step has no ringing (the working
/// image, sampled up from level 1, keeps its own); the change is measured
/// against Highlights at 0. The dark side sweeps through Highlights' ramp.
#[test]
fn criterion_12_halo() {
    let Some((_lock, gpu)) = gpu() else { return };
    let size = (1200, 300);
    let out = (600, 150);
    let edge = 300u32;
    let linear = Look::with_profile(Profile::GenzoLinear);
    let looks = [
        linear,
        Look {
            highlights: -100.0,
            ..linear
        },
    ];
    // The radius in view pixels: 24 working pixels at 1200 px, 12 here.
    let r = radius(size) * out.0 / size.0;
    let mut worst: f64 = 0.0;
    for dark in [0.0, 0.5, 1.0, 1.5] {
        let src = stops(size, |x, _| if x < 2 * edge { dark } else { dark + 3.0 });
        let (sampled, toned) = toned_t(&gpu, &src, out, &looks);
        let row = (out.1 / 2 * out.0) as usize;
        let change = |x: u32| toned[1][row + x as usize] - toned[0][row + x as usize];
        let far = change(edge / 4);
        let mut halo: f64 = 0.0;
        for x in edge - 2 * r..edge {
            let level = sampled[row + x as usize];
            assert!((level - dark).abs() < 0.01, "x {x}: {level}, a clean step");
            halo = halo.max((change(x) - far).abs());
        }
        eprintln!("dark side at {dark:+.1}: far change {far:+.3} EV, halo {halo:.4} EV");
        worst = worst.max(halo);
    }
    eprintln!("halo: worst {worst:.4} EV (radius {r} px)");
    assert!(worst <= 0.15, "{worst}");
}
