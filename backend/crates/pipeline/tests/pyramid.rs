//! Stage 4 and the pyramid on the GPU (DEV-01, Tiling): stages 1 to 4 tile
//! by tile, reduced into level 1, then levels 2 and up.
//!
//! - Tiled equals one tile, bit for bit, at every level, on sizes that
//!   need the odd-edge fold and the merged 1 px last tile.
//! - Each level matches the CPU reference (stage 4 and the reduction) on the
//!   GPU's own stage 3 output, within one f16 step.
//! - With the Pixel 4a's real profile, a synthetic sensor made by running
//!   the chain backwards from a known scene renders back to that scene.

use std::sync::{Mutex, MutexGuard};

use genzo_imgio::raw::{AsShot, CameraColor, ColorMatrix, Levels, Rect};
use genzo_pipeline::camera_color::{CameraTransform, camera_transform};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until, level_count};
use genzo_pipeline::gpu::synthetic::scene;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::{Cfa, Linearization, RawSource};
use genzo_pipeline::reference::stages::{reduce, stage4};

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
const WHITE: u32 = 16383;

/// A Bayer sensor (RGGB) whose white balanced camera values are
/// `camera(x, y)`, before the multipliers: the inverse of stages 1 and 2.
fn sensor(
    (w, h): (u32, u32),
    multipliers: [f32; 3],
    color: CameraColor,
    mut camera: impl FnMut(u32, u32) -> [f64; 3],
) -> RawSource {
    let colors = [0u8, 1, 1, 2];
    let levels = Levels {
        black: vec![BLACK],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![WHITE],
    };
    let range = f64::from(WHITE) - BLACK;
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = usize::from(colors[((y & 1) * 2 + (x & 1)) as usize]);
            let v = camera(x, y)[c] / f64::from(multipliers[c]);
            data.push((v * range + BLACK).round().clamp(0.0, 65535.0) as u16);
        }
    }
    RawSource {
        stored: (w, h),
        samples: 1,
        data,
        cfa: Some(Cfa::Bayer(colors)),
        active: Rect {
            top: 0,
            left: 0,
            bottom: h,
            right: w,
        },
        linearization: Linearization::new(&levels, 1, (w, h)).unwrap(),
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color,
        encoding: None,
    }
}

/// A made-up transform: white balance, a saturating matrix (so the
/// highlight desaturation shows), an exposure gain.
fn transform() -> CameraTransform {
    CameraTransform {
        white: genzo_color::camera::D65_XY,
        multipliers: [2.1, 1.0, 1.6],
        matrix: [[1.4, -0.3, -0.1], [-0.2, 1.3, -0.1], [0.0, -0.4, 1.4]],
        exposure: 1.5,
        fallback: None,
        highlight_desaturation: true,
    }
}

/// The synthetic scene as camera values, bright enough that the top of the
/// lines band reaches the clip and desaturates.
fn scene_sensor(size: (u32, u32)) -> RawSource {
    let t = transform();
    sensor(size, t.multipliers, CameraColor::default(), |x, y| {
        scene(x, y, size.0, size.1).map(|v| f64::from(v) * 1.9)
    })
}

/// The same scene as a linear DNG (3 samples per pixel) or a monochrome
/// plane (1, the scene's green), with noise reaching below black, so stage 4
/// and the pyramid see negative values.
fn linear_sensor(size: (u32, u32), samples: u32) -> RawSource {
    let t = transform();
    let (w, h) = size;
    let n = samples as usize;
    let levels = Levels {
        black: vec![BLACK; n],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![WHITE],
    };
    let range = f64::from(WHITE) - BLACK;
    let mut data = Vec::with_capacity((w * h) as usize * n);
    for y in 0..h {
        for x in 0..w {
            let rgb = scene(x, y, w, h);
            for p in 0..n {
                let c = if n == 1 { 1 } else { p };
                let noise = f64::from((x * 7 + y * 13 + p as u32 * 5) % 11) - 5.0;
                let gain = if n == 1 {
                    1.0
                } else {
                    f64::from(t.multipliers[c])
                };
                let v = f64::from(rgb[c]) * 1.9 / gain - 0.06;
                data.push((v * range + BLACK + noise).round().clamp(0.0, 65535.0) as u16);
            }
        }
    }
    RawSource {
        stored: size,
        samples,
        data,
        cfa: None,
        active: Rect {
            top: 0,
            left: 0,
            bottom: h,
            right: w,
        },
        linearization: Linearization::new(&levels, samples, size).unwrap(),
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color: CameraColor::default(),
        encoding: None,
    }
}

/// Every kind of input stage 4 takes: a Bayer mosaic after RCD, linear RGB,
/// a monochrome plane.
fn sources(size: (u32, u32)) -> [(&'static str, RawSource); 3] {
    [
        ("bayer", scene_sensor(size)),
        ("linear", linear_sensor(size, 3)),
        ("mono", linear_sensor(size, 1)),
    ]
}

fn pyramid_levels(gpu: &Gpu, config: TileConfig, src: &RawSource) -> Vec<Vec<[f32; 3]>> {
    let mut stages = RawStages::new(gpu, config);
    let p = stages.render_pyramid(gpu, src, &transform()).unwrap();
    (1..=p.levels)
        .map(|l| p.read_level(gpu, l).unwrap())
        .collect()
}

const ONE_TILE: TileConfig = TileConfig {
    tile: 4096,
    xtrans_tile: 4096,
    apron: 24,
};
const SMALL: TileConfig = TileConfig {
    tile: 64,
    xtrans_tile: 60,
    apron: 24,
};

/// Odd sizes fold an edge into the last texel; 1025 and 641 leave a 1 px
/// last tile at 64 px tiles, which joins the one before.
const SIZES: [(u32, u32); 2] = [(1030, 777), (1025, 641)];

#[test]
fn pyramid_tiles_equal_one_tile_bit_for_bit() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, src) in SIZES.into_iter().flat_map(sources) {
        let size = src.size();
        let whole = pyramid_levels(&gpu, ONE_TILE, &src);
        let tiled = pyramid_levels(&gpu, SMALL, &src);
        assert!(whole.len() >= 2, "{name} {size:?}: {} levels", whole.len());
        for (l, (a, b)) in whole.iter().zip(&tiled).enumerate() {
            let first = a
                .iter()
                .zip(b)
                .position(|(x, y)| x.map(f32::to_bits) != y.map(f32::to_bits));
            assert_eq!(first, None, "{name} {size:?} level {}", l + 1);
        }
    }
}

/// Within one f16 step of `want` (the GPU rounds to f16 however it does),
/// plus 1e-8: near zero, where linear input's noise sums positive and
/// negative values of about 0.03, the two sides' f32 sums differ by a few
/// 1e-9 before rounding (measured: one value in a million, 3e-9 over).
fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= step * 1.01 + 1e-8
}

#[test]
fn levels_match_the_cpu_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let t = transform();
    for (name, src) in SIZES.into_iter().flat_map(sources) {
        let size = src.size();
        let mut stages = RawStages::new(&gpu, SMALL);
        let rgb3 = stages
            .render(&gpu, &src, t.multipliers, Until::Stage3)
            .unwrap();
        let s4: Vec<f32> = rgb3
            .chunks(3)
            .flat_map(|c| stage4([c[0], c[1], c[2]], &t))
            .collect();
        let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
        assert_eq!(p.levels, level_count(p.size));
        // Level 1 from the GPU's stage 3; each level above from the GPU's
        // level below, so f16 rounding does not pile up.
        let (mut above, mut above_size) = (s4, size);
        let mut desaturated = 0;
        for level in 1..=p.levels {
            let (want, want_size) = reduce(&above, above_size, 3);
            assert_eq!(want_size, p.level_size(level));
            let got = p.read_level(&gpu, level).unwrap();
            let bad = got
                .iter()
                .flatten()
                .zip(&want)
                .filter(|(g, w)| !f16_close(**g, **w))
                .count();
            assert_eq!(bad, 0, "{name} {size:?} level {level}");
            if level == 1 {
                desaturated = rgb3
                    .chunks(3)
                    .filter(|c| c[0].max(c[1]).max(c[2]) > 0.95)
                    .count();
            }
            above = got.iter().flatten().copied().collect();
            above_size = want_size;
        }
        assert!(
            desaturated > 1000,
            "{name} {size:?}: {desaturated} pixels desaturate"
        );
        if name != "bayer" {
            assert!(rgb3.iter().any(|v| *v < 0.0), "{name}: no negative values");
        }
    }
}

/// The Pixel 4a's color tags, as the DNG SDK read them
/// (color/tests/fixtures/dng-sdk-color.jsonl).
fn pixel4a_color() -> CameraColor {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../color/tests/fixtures/dng-sdk-color.jsonl"
    ));
    let r: serde_json::Value = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|r| r["key"] == "raw/Google/Pixel 4a/PXL_20201121_100251397.dng")
        .unwrap();
    let i = &r["inputs"];
    let v = |k: &str| {
        i.get(k).map(|a| {
            a.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect::<Vec<_>>()
        })
    };
    let il = |k: &str| i[k].as_u64().map(|n| n as u16);
    let s = |k: &str| i[k].as_str().unwrap_or("").to_owned();
    let n = v("as_shot_neutral").unwrap();
    CameraColor {
        color_matrix_1: Some(ColorMatrix {
            illuminant: il("illuminant_1"),
            matrix: v("color_matrix_1").unwrap(),
        }),
        color_matrix_2: Some(ColorMatrix {
            illuminant: il("illuminant_2"),
            matrix: v("color_matrix_2").unwrap(),
        }),
        forward_matrix_1: v("forward_matrix_1"),
        forward_matrix_2: v("forward_matrix_2"),
        camera_calibration_1: v("camera_calibration_1"),
        camera_calibration_2: v("camera_calibration_2"),
        camera_calibration_signature: s("camera_calibration_signature"),
        profile_calibration_signature: s("profile_calibration_signature"),
        analog_balance: v("analog_balance"),
        as_shot: Some(AsShot::Neutral([n[0], n[1], n[2]])),
        baseline_exposure: 0.4,
        ..CameraColor::default()
    }
}

#[test]
fn a_real_profile_renders_the_scene_back() {
    let Some((_lock, gpu)) = gpu() else { return };
    let color = pixel4a_color();
    let t = camera_transform(&color, None);
    assert_eq!(t.fallback, None);
    let inverse = genzo_color::invert(&t.matrix.map(|r| r.map(f64::from))).unwrap();
    // The scene in the working space, kept below the desaturation range.
    let to_ap1 = genzo_color::mat_mul(
        &genzo_color::invert(&genzo_color::WORKING_TO_XYZ).unwrap(),
        &genzo_color::SRGB_TO_XYZ,
    );
    let size = (1024u32, 768u32);
    // Half the scene's saturation, so every color is one the camera can
    // record (no negative camera values, which a sensor cannot hold).
    let ap1 = |x: u32, y: u32| {
        let s = scene(x, y, size.0, size.1).map(f64::from);
        let mean = (s[0] + s[1] + s[2]) / 3.0;
        genzo_color::mul(&to_ap1, s.map(|v| (v + mean) * 0.5 * 0.6))
    };
    let mut lowest = f64::INFINITY;
    let src = sensor(size, t.multipliers, color.clone(), |x, y| {
        let c = genzo_color::mul(&inverse, ap1(x, y));
        lowest = lowest.min(c[0].min(c[1]).min(c[2]));
        c
    });
    assert!(
        lowest > 0.0,
        "the scene leaves the camera's gamut: {lowest}"
    );
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
    let got = p.read_level(&gpu, 1).unwrap();
    let (lw, lh) = p.size;
    let gain = 2f64.powf(0.4);
    // The smooth gradients (the top 40 %), away from the borders, against
    // the scene's 2 x 2 means.
    let mut worst = 0.0f64;
    let mut at = (0, 0, 0, 0.0, 0.0);
    for j in 8..(lh as f64 * 0.38) as u32 {
        for i in 8..lw - 8 {
            let mut want = [0.0f64; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let s = ap1(2 * i + dx, 2 * j + dy);
                for c in 0..3 {
                    want[c] += s[c] * gain / 4.0;
                }
            }
            for c in 0..3 {
                let g = f64::from(got[(j * lw + i) as usize][c]);
                let d = (g - want[c]).abs() / want[c].abs().max(0.05);
                if d > worst {
                    worst = d;
                    at = (i, j, c, g, want[c]);
                }
            }
        }
    }
    println!("gradients: worst relative difference {worst:.2e} at {at:?}");
    // 14-bit quantization, RCD on smooth color and f16 storage: measured
    // 0.13 % on the RTX 3060.
    assert!(worst < 3e-3, "{worst}");
    // A neutral patch (0.18 gray, at x 0 to 1/6, y 60 to 80 %) comes out
    // neutral at 0.18 · 0.6 · 2^0.4 in every channel.
    let want = 0.18 * 0.6 * gain;
    assert_eq!(
        scene(size.0 / 12, (size.1 as f64 * 0.7) as u32, size.0, size.1),
        [0.18; 3]
    );
    let (i, j) = (lw / 12, (lh as f64 * 0.7) as u32);
    let g = got[(j * lw + i) as usize];
    for c in g {
        assert!((f64::from(c) - want).abs() / want < 2e-3, "{g:?} vs {want}");
    }
}

/// The stored golden of the pyramid's first level: the synthetic scene
/// through stages 1 to 4 (the camera records the working space, so the
/// matrix is the identity), viewed as sRGB. Written on lavapipe.
#[test]
fn golden_pyramid_level1() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = genzo_pipeline::gpu::synthetic::scene_raw(768, 512);
    let t = camera_transform(&src.color, None);
    let mut stages = RawStages::new(&gpu, SMALL);
    let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
    let (w, h) = p.size;
    let level1 = p.read_level(&gpu, 1).unwrap();
    let to_srgb = genzo_color::mat_mul(
        &genzo_color::invert(&genzo_color::SRGB_TO_XYZ).unwrap(),
        &genzo_color::WORKING_TO_XYZ,
    );
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "pyramid/synthetic-level1",
        genzo_golden::Space::Linear(genzo_golden::View {
            matrix: to_srgb,
            exposure_ev: 0.0,
        }),
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    )
    // As raw-stages/synthetic-bayer: RCD's direction choices can flip
    // between adapters that fuse multiply-adds and those that do not, which
    // moves a few pixels a lot; the 2 x 2 means soften it.
    .with_tolerance(genzo_golden::Tolerance {
        mean_de: 0.05,
        max_de: 6.0,
        min_psnr_db: 65.0,
        max_abs: None,
    });
    genzo_golden::check(&case, &genzo_golden::Image::new(w, h, level1));
}
