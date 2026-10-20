//! The white balance drag (DEV-01, stage 2): while Temp or Tint is dragged,
//! the view applies `camera_color::drag_ratio` to the values it samples
//! from the pyramid built with the old white, instead of rebuilding it.
//!
//! - The sampled values with the ratio match `reference::sampling` with the
//!   ratio applied after, within two f16 steps, at Fit in three
//!   orientations. Level 0 is covered by the 1:1 comparison below.
//! - Against the settled render (the pyramid rebuilt with the new white):
//!   a non-RAW drag is exact up to f16; a RAW drag differs only where
//!   stage 2 clips or stage 4 desaturates. Those pixels are masked: either
//!   white's balanced camera values reach the desaturation threshold, read
//!   from a render without desaturation, the mask dilated by 2 px for the
//!   sampling filter.

#[allow(dead_code)]
mod sdk_stage2;

use std::sync::{Mutex, MutexGuard};

use genzo_color::camera::{temperature_from_xy, xy_from_temperature};
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::camera_color::{
    CameraTransform, balanced_encoded_transform, camera_transform, drag_ratio, encoded_transform,
};
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{blocks, scene_raw, with_camera_profile};
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{ViewSpec, sample};
use genzo_pipeline::reference::stages::DESATURATE_FROM;

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

/// Within two f16 steps, plus 1e-5 (as tests/view.rs).
fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= 2.0 * step + 1e-5
}

fn apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

#[test]
fn the_ratio_applies_to_what_is_sampled() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = profiled_scene();
    let old = camera_transform(&src.color, None);
    let new = camera_transform(&src.color, Some(xy_from_temperature(3200.0, 15.0)));
    let m = drag_ratio(&old, &new).unwrap();
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &old).unwrap();
    let mut view = ViewStages::new(&gpu);
    view.set_ratio(Some(m));
    let p = stages.pyramid().unwrap();
    for (o, out) in [(1u8, (500, 375)), (6, (251, 333)), (1, (700, 520))] {
        let s = view.sample(&gpu, p, o, out);
        let level = p.read_level(&gpu, s.level).unwrap();
        let want: Vec<_> = sample(&level, s.level_size, &s, out)
            .into_iter()
            .map(|v| apply(&m, v))
            .collect();
        let got = view.read_sampled(&gpu).unwrap();
        let bad = got
            .iter()
            .flatten()
            .zip(want.iter().flatten())
            .filter(|(g, w)| !f16_close(**g, **w))
            .count();
        assert_eq!(bad, 0, "orientation {o}, {out:?} from level {}", s.level);
    }
    // Stopping the ratio samples the pyramid as it is again.
    view.set_ratio(None);
    let s = view.sample(&gpu, p, 1, (500, 375));
    let level = p.read_level(&gpu, s.level).unwrap();
    let want = sample(&level, s.level_size, &s, (500, 375));
    let got = view.read_sampled(&gpu).unwrap();
    assert!(
        got.iter()
            .flatten()
            .zip(want.iter().flatten())
            .all(|(g, w)| f16_close(*g, *w))
    );
}

/// The view's sampled values for `spec` of `src` rendered with `t`,
/// with the drag `ratio` applied or not.
fn sampled(
    gpu: &Gpu,
    src: &RawSource,
    t: &CameraTransform,
    ratio: Option<[[f32; 3]; 3]>,
    spec: ViewSpec,
) -> Vec<[f32; 3]> {
    let mut stages = RawStages::new(gpu, TileConfig::default());
    stages.render_pyramid(gpu, src, t).unwrap();
    let mut cache = Level0Cache::new(gpu);
    let mut view = ViewStages::new(gpu);
    view.set_ratio(ratio);
    view.show(gpu, &mut stages, &mut cache, src, 1, 1u8, spec)
        .unwrap();
    view.read_sampled(gpu).unwrap()
}

/// ΔE2000 between two working space values, on XYZ relative to a white of
/// Y 1 (stage 4 output is scene referred, before any tone).
fn delta_e(a: [f32; 3], b: [f32; 3]) -> f64 {
    let m = genzo_color::WORKING_TO_XYZ;
    let lab = |v: [f32; 3]| {
        let xyz = genzo_color::mul(&m, v.map(f64::from));
        genzo_color::xyz_to_lab(xyz, genzo_color::D65_WHITE)
    };
    genzo_color::delta_e_2000(lab(a), lab(b))
}

/// Pixels near highlights stage 2 clips or stage 4 desaturates under `old`
/// or `new`: their balanced camera values, from the old render without
/// highlight desaturation, at the desaturation threshold, dilated by 2 px.
fn highlight_mask(
    old_values: &[[f32; 3]],
    old: &CameraTransform,
    new: &CameraTransform,
    size: (u32, u32),
) -> Vec<bool> {
    let f64s = |m: &[[f32; 3]; 3]| m.map(|row| row.map(f64::from));
    let undo = genzo_color::invert(&f64s(&old.matrix)).unwrap();
    let r = [0, 1, 2].map(|c| new.multipliers[c] / old.multipliers[c]);
    let hot: Vec<bool> = old_values
        .iter()
        .map(|v| {
            let b = genzo_color::mul(&undo, v.map(|c| f64::from(c / old.exposure)));
            let top_old = b[0].max(b[1]).max(b[2]) as f32;
            let top_new = (0..3).map(|c| b[c] as f32 * r[c]).fold(f32::MIN, f32::max);
            top_old >= DESATURATE_FROM || top_new >= DESATURATE_FROM
        })
        .collect();
    let (w, h) = (size.0 as i64, size.1 as i64);
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            (-2..=2).any(|dy| {
                (-2..=2).any(|dx| {
                    let (xx, yy) = (x + dx, y + dy);
                    xx >= 0 && yy >= 0 && xx < w && yy < h && hot[(yy * w + xx) as usize]
                })
            })
        })
        .collect()
}

/// ΔE2000 of the unmasked pixels: the mean, the 99th percentile and the
/// largest.
#[derive(Debug, Clone, Copy, Default)]
struct Stats {
    mean: f64,
    p99: f64,
    max: f64,
}

impl Stats {
    fn worst(self, o: Stats) -> Stats {
        Stats {
            mean: self.mean.max(o.mean),
            p99: self.p99.max(o.p99),
            max: self.max.max(o.max),
        }
    }
}

/// The drag against the settled render for `src` from its as-shot white to
/// each of `whites` (Temp and Tint shifts): the worst at Fit, and at 1:1 in
/// the middle of the image.
fn drag_against_settled(
    gpu: &Gpu,
    name: &str,
    src: &RawSource,
    whites: &[(f64, f64)],
) -> [Stats; 2] {
    let old = camera_transform(&src.color, None);
    let (t0, tint0) = temperature_from_xy(old.white);
    let (w, h) = src.size();
    let specs = [
        ViewSpec::Fit((960, 720)),
        ViewSpec::Region {
            out: (512, 384),
            zoom: 1.0,
            origin: (f64::from(w / 2 - 256), f64::from(h / 2 - 192)),
        },
    ];
    // Without desaturation a clipped channel inverts to the clip level.
    let plain = CameraTransform {
        highlight_desaturation: false,
        ..old.clone()
    };
    let mut worst = [Stats::default(); 2];
    for &(dt, dtint) in whites {
        let new = camera_transform(
            &src.color,
            Some(xy_from_temperature(t0 + dt, tint0 + dtint)),
        );
        let m = drag_ratio(&old, &new).unwrap();
        for (k, spec) in specs.into_iter().enumerate() {
            let before = sampled(gpu, src, &plain, None, spec);
            let dragged = sampled(gpu, src, &old, Some(m), spec);
            let settled = sampled(gpu, src, &new, None, spec);
            let mask = highlight_mask(&before, &old, &new, spec.out());
            let mut e: Vec<f64> = dragged
                .iter()
                .zip(&settled)
                .zip(&mask)
                .filter(|(_, m)| !**m)
                .map(|((a, b), _)| delta_e(*a, *b))
                .collect();
            e.sort_by(f64::total_cmp);
            let n = e.len().max(1);
            let s = Stats {
                mean: e.iter().sum::<f64>() / n as f64,
                p99: e.get(n * 99 / 100).copied().unwrap_or(0.0),
                max: e.last().copied().unwrap_or(0.0),
            };
            let masked = mask.iter().filter(|m| **m).count() as f64 / mask.len() as f64;
            println!(
                "{name} as shot {t0:.0} K {tint0:+.1}, {dt:+.0} K {dtint:+.0}, {}: \
                 masked {:.2} %, ΔE mean {:.3} p99 {:.3} max {:.2}",
                ["Fit", "1:1"][k],
                100.0 * masked,
                s.mean,
                s.p99,
                s.max
            );
            worst[k] = worst[k].worst(s);
        }
    }
    worst
}

/// The drag frame against the settled render outside the masked highlights
/// (DEV-01, *Re-runs, quality levels and generations*; owner, 2026-10-20).
/// At 1:1 the settled render re-demosaics, and RCD and Markesteijn choose
/// their directions from white balanced values, so fine detail differs
/// until the drag ends.
fn check(name: &str, [fit, one]: [Stats; 2]) {
    assert!(fit.mean <= 0.1 && fit.p99 <= 0.5, "{name} at Fit: {fit:?}");
    assert!(one.mean <= 0.6 && one.p99 <= 5.0, "{name} at 1:1: {one:?}");
}

/// Shifts from the as-shot white: cooler, warmer, toward magenta, and both.
const WHITES: [(f64, f64); 4] = [(-1500.0, 0.0), (2500.0, 0.0), (0.0, 40.0), (-800.0, -30.0)];

/// The synthetic scene with a camera profile; with its own (none), every
/// drag is an identity.
fn profiled_scene() -> RawSource {
    with_camera_profile(scene_raw(1030, 777))
}

#[test]
fn a_raw_drag_matches_the_settled_render_outside_highlights() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = profiled_scene();
    check(
        "synthetic",
        drag_against_settled(&gpu, "synthetic", &src, &WHITES),
    );
}

/// A linear DNG, a Bayer DNG, a Bayer raw, X-Trans and the Leica's Bayer DNG.
/// CI runs this after fetching the corpus; elsewhere a missing file skips.
#[test]
fn a_raw_drag_matches_the_settled_render_on_the_corpus() {
    let Some((_lock, gpu)) = gpu() else { return };
    for key in [
        "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
        "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
        "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
        "raw/Fujifilm/X-T5/DSCF0021.RAF",
        "raw/Leica/Q2/L1000750.DNG",
    ] {
        let Some(path) = sdk_stage2::corpus_file(key) else {
            continue;
        };
        let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
        let src = RawSource::from_decoded(img).unwrap();
        check(key, drag_against_settled(&gpu, key, &src, &WHITES));
    }
}

/// A non-RAW drag is the settled render's own matrix, so the two differ
/// only by f16 rounding. The drag rounds twice (the pyramid, then the view
/// after the matrix), the settled render once, and the matrix mixes the
/// channels: within 3 f16 steps of the pixel's largest channel. One step is
/// about ΔE2000 0.1 at mid gray, so ΔE would measure the storage, not the
/// drag.
#[test]
fn a_non_raw_drag_matches_the_settled_render() {
    let Some((_lock, gpu)) = gpu() else { return };
    let colors: Vec<[u16; 3]> = (0..64 * 48)
        .map(|i| {
            let (x, y) = ((i % 64) as u16, (i / 64) as u16);
            [x * 1000, y * 1300, 65535 - x * 500 - y * 300]
        })
        .collect();
    let (size, rgba) = blocks(&colors, 64);
    let e = genzo_color::encoding::InputEncoding::srgb();
    let src = RawSource::from_encoded(size, &rgba, &e).unwrap();
    let old = encoded_transform(&e);
    for (temp, tint) in [(-100.0, 0.0), (60.0, -40.0), (100.0, 100.0)] {
        let new = balanced_encoded_transform(&e, temp, tint);
        let m = drag_ratio(&old, &new).unwrap();
        let spec = ViewSpec::Fit((100, 75));
        let dragged = sampled(&gpu, &src, &old, Some(m), spec);
        let settled = sampled(&gpu, &src, &new, None, spec);
        // The largest difference in f16 steps of the pixel's largest channel.
        let worst = dragged
            .iter()
            .zip(&settled)
            .map(|(a, b)| {
                let top = a.iter().chain(b).fold(6.2e-5f32, |m, v| m.max(v.abs()));
                (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max) / (top / 1024.0)
            })
            .fold(0.0, f32::max);
        println!("non-RAW {temp:+} {tint:+}: {worst:.2} f16 steps");
        assert!(worst <= 3.0, "{temp} {tint}: {worst} steps");
    }
}
