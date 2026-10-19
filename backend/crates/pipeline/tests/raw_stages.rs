//! Stages 1 to 3 on the GPU, tile by tile (DEV-01, Tiling), on a synthetic
//! sensor that puts every coordinate system to work: an active area at an
//! odd offset inside a masked border, black levels per site plus DeltaH and
//! DeltaV, bad pixels at the active area's edge (whose neighbors lie in the
//! border) and on tile boundaries, and GainMaps with a pitch that straddle
//! tiles.
//!
//! - A tiled render equals a render of the whole image as one tile, bit for
//!   bit; with an apron too narrow for RCD it does not (negative control).
//! - Stages 1 and 2 match the CPU reference to f32 rounding.
//! - The demosaic matches the CPU port of darktable's rcd.c.

#[allow(dead_code)]
mod rcd_reference;

use std::sync::{Mutex, MutexGuard};

use genzo_imgio::opcodes::{AreaSpec, FixBadPixelsConstant, GainMap};
use genzo_imgio::raw::{Levels, Rect};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
use genzo_pipeline::gpu::synthetic::scene;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::{Cfa, Linearization, RawSource};
use genzo_pipeline::reference::stages::{stage1, stage2};

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

const W: u32 = 300;
const H: u32 = 200;
/// The active area's top left in the stored image: odd, so the CFA phase
/// differs between the two.
const LEFT: u32 = 3;
const TOP: u32 = 5;
const WHITE: u32 = 16383;
const WB: [f32; 3] = [2.1, 1.0, 1.6];

/// A small deterministic noise, in codes.
fn noise(x: u32, y: u32) -> f64 {
    let h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 1000;
    (f64::from(h) / 1000.0 - 0.5) * 8.0
}

fn levels(planes: usize) -> Levels {
    let black: Vec<f64> = [510.0, 512.0, 514.0, 516.0]
        .iter()
        .flat_map(|&b| (0..planes).map(move |p| b + p as f64))
        .collect();
    Levels {
        black,
        black_repeat: (2, 2),
        black_delta_h: (0..W).map(|c| f64::from(c % 7) * 0.5).collect(),
        black_delta_v: (0..H).map(|r| f64::from(r % 5) * 0.25).collect(),
        white: vec![WHITE],
    }
}

fn gain_maps(planes: u32) -> Vec<GainMap> {
    vec![
        // Every other row from row 1, every other column: one Bayer site.
        GainMap {
            area: AreaSpec {
                top: 1,
                left: 0,
                bottom: H,
                right: W,
                plane: 0,
                planes,
                row_pitch: 2,
                col_pitch: 2,
            },
            points_v: 3,
            points_h: 4,
            spacing_v: 0.5,
            spacing_h: 1.0 / 3.0,
            origin_v: 0.0,
            origin_h: 0.0,
            map_planes: 1,
            gains: vec![1.0, 1.3, 1.6, 2.0, 1.1, 1.2, 1.4, 1.5, 1.9, 1.0, 1.2, 1.7],
        },
        // The whole image, with an origin past the left edge.
        GainMap {
            area: AreaSpec {
                top: 0,
                left: 0,
                bottom: 0,
                right: 0,
                plane: 0,
                planes,
                row_pitch: 1,
                col_pitch: 1,
            },
            points_v: 2,
            points_h: 2,
            spacing_v: 1.0,
            spacing_h: 0.5,
            origin_v: 0.0,
            origin_h: 0.25,
            map_planes: 1,
            gains: vec![0.9, 1.1, 1.05, 0.95],
        },
    ]
}

/// The scene's linear value as the sensor records it: the darkest patch
/// (0.03) sits at the black level, so the noise reaches below it.
fn value(scene: f32) -> f64 {
    f64::from(scene) * 0.9 - 0.027
}

/// The Bayer sensor: RGGB from the stored origin, the active area inset.
fn bayer() -> RawSource {
    let stored = (W + LEFT + 7, H + TOP + 7);
    let colors = [0u8, 1, 1, 2];
    let lin = Linearization::new(&levels(1), 1, (W, H)).unwrap();
    let mut data = vec![0u16; (stored.0 * stored.1) as usize];
    for y in 0..stored.1 {
        for x in 0..stored.0 {
            let c = colors[((y & 1) * 2 + (x & 1)) as usize];
            // The masked border reads 600; the active area the scene.
            let code = if (LEFT..LEFT + W).contains(&x) && (TOP..TOP + H).contains(&y) {
                let (ax, ay) = (x - LEFT, y - TOP);
                let v = value(scene(ax, ay, W, H)[usize::from(c)]);
                let black = f64::from(lin.black(ay, ax, 0)) / f64::from(lin.scale[0]);
                v / f64::from(lin.scale[0]) + black + noise(x, y)
            } else {
                600.0
            };
            data[(y * stored.0 + x) as usize] = code.round().clamp(1.0, 65535.0) as u16;
        }
    }
    // Bad pixels (0): at the active area's top-left corner and right edge,
    // where some neighbors are in the masked border; on a tile boundary of
    // 64 px tiles; and next to another bad pixel.
    for (ax, ay) in [
        (0, 0),
        (1, 1),
        (W - 1, 7),
        (64, 64),
        (63, 64),
        (150, 101),
        (152, 101),
    ] {
        data[((ay + TOP) * stored.0 + ax + LEFT) as usize] = 0;
    }
    RawSource {
        stored,
        samples: 1,
        data,
        cfa: Some(Cfa::Bayer(colors)),
        active: Rect {
            top: TOP,
            left: LEFT,
            bottom: TOP + H,
            right: LEFT + W,
        },
        linearization: lin,
        fix_bad_pixels: vec![FixBadPixelsConstant {
            constant: 0,
            bayer_phase: 1,
        }],
        gain_maps: gain_maps(1),
        color: Default::default(),
        encoding: None,
    }
}

/// A linear RGB image of the same scene, black levels per sample; with one
/// sample, a monochrome plane (the scene's green).
fn linear_with(samples: u32) -> RawSource {
    let stored = (W + LEFT + 7, H + TOP + 7);
    let n = samples as usize;
    let lin = Linearization::new(&levels(n), samples, (W, H)).unwrap();
    let mut data = vec![600u16; (stored.0 * stored.1 * samples) as usize];
    for ay in 0..H {
        for ax in 0..W {
            let (x, y) = (ax + LEFT, ay + TOP);
            let rgb = scene(ax, ay, W, H);
            for p in 0..n {
                let black = f64::from(lin.black(ay, ax, p)) / f64::from(lin.scale[p]);
                let code = value(rgb[if n == 1 { 1 } else { p }]) / f64::from(lin.scale[p])
                    + black
                    + noise(x * 3 + p as u32, y);
                data[((y * stored.0 + x) * samples) as usize + p] =
                    code.round().clamp(0.0, 65535.0) as u16;
            }
        }
    }
    RawSource {
        stored,
        samples,
        data,
        cfa: None,
        active: Rect {
            top: TOP,
            left: LEFT,
            bottom: TOP + H,
            right: LEFT + W,
        },
        linearization: lin,
        fix_bad_pixels: Vec::new(),
        gain_maps: gain_maps(samples),
        color: Default::default(),
        encoding: None,
    }
}

fn linear() -> RawSource {
    linear_with(3)
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

fn render(gpu: &Gpu, config: TileConfig, src: &RawSource, until: Until) -> Vec<f32> {
    RawStages::new(gpu, config)
        .render(gpu, src, WB, until)
        .unwrap()
}

/// The first value that differs, as (index, a, b).
fn first_difference(a: &[f32], b: &[f32]) -> Option<(usize, f32, f32)> {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .enumerate()
        .find(|(_, (x, y))| x.to_bits() != y.to_bits())
        .map(|(i, (x, y))| (i, *x, *y))
}

#[test]
fn tiles_equal_one_tile_bit_for_bit() {
    let Some((_lock, gpu)) = gpu() else { return };
    // Also without opcodes: empty GainMap and bad-pixel inputs.
    let mut plain = bayer();
    plain.gain_maps.clear();
    plain.fix_bad_pixels.clear();
    for (name, src) in [
        ("bayer", bayer()),
        ("linear", linear()),
        ("mono", linear_with(1)),
        ("plain", plain),
    ] {
        for until in [Until::Stage2, Until::Stage3] {
            let whole = render(&gpu, ONE_TILE, &src, until);
            let tiled = render(&gpu, SMALL, &src, until);
            assert_eq!(first_difference(&whole, &tiled), None, "{name} {until:?}");
        }
    }
}

#[test]
fn a_narrow_apron_shows() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = bayer();
    let whole = render(&gpu, ONE_TILE, &src, Until::Stage3);
    // 6 px: wide enough for the bad-pixel fix, not for RCD's 10.
    let narrow = TileConfig {
        tile: 64,
        xtrans_tile: 60,
        apron: 6,
    };
    let tiled = render(&gpu, narrow, &src, Until::Stage3);
    assert!(first_difference(&whole, &tiled).is_some());
}

#[test]
fn stages_1_and_2_match_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, src) in [
        ("bayer", bayer()),
        ("linear", linear()),
        ("mono", linear_with(1)),
    ] {
        let got = render(&gpu, SMALL, &src, Until::Stage2);
        // A monochrome plane comes back copied to R, G and B.
        let got: Vec<f32> = if name == "mono" {
            assert!(got.chunks(3).all(|c| c[0] == c[1] && c[1] == c[2]));
            got.chunks(3).map(|c| c[0]).collect()
        } else {
            got
        };
        let mut want = stage1(&src);
        stage2(&mut want, &src, WB);
        if src.cfa.is_some() {
            // The demosaic's input is clamped at 0.
            want.iter_mut().for_each(|v| *v = v.max(0.0));
        }
        let mut worst = (0.0f32, 0);
        let mut negative = 0;
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            // Relative, and absolute below 0.05, where f32's spacing is fixed.
            let d = (g - w).abs() / w.abs().max(0.05);
            if d > worst.0 {
                worst = (d, i);
            }
            negative += usize::from(*w < 0.0);
        }
        println!(
            "{name}: worst relative difference {:.2e} at {}",
            worst.0, worst.1
        );
        // f32 on both sides, with the GPU's map index in f32 instead of f64.
        assert!(worst.0 < 2e-6, "{name}: {worst:?}");
        if name == "linear" {
            assert!(negative > 0, "the noise reaches below black");
        }
    }
}

#[test]
fn the_bad_pixels_are_fixed() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = bayer();
    let got = render(&gpu, SMALL, &src, Until::Stage2);
    // Without the fix, the bad sites read 0 minus the black level: negative,
    // clamped to 0. Fixed, they hold their neighbors' mean.
    for (x, y) in [(0u32, 0u32), (W - 1, 7), (64, 64), (63, 64)] {
        assert!(got[(y * W + x) as usize] > 0.01, "({x}, {y})");
    }
}

#[test]
fn rcd_matches_the_darktable_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = bayer();
    let got = render(&gpu, SMALL, &src, Until::Stage3);
    let mut cfa = stage1(&src);
    stage2(&mut cfa, &src, WB);
    cfa.iter_mut().for_each(|v| *v = v.max(0.0));
    // The pattern at the active area's origin, which is what rcd.c sees.
    let pattern = [0, 1, W, W + 1].map(|i| src.color_at(i / W, i % W).unwrap());
    let want = rcd_reference::rcd(
        &cfa,
        W as usize,
        H as usize,
        pattern,
        rcd_reference::PqMode::PerPixel,
    );
    // RCD picks between two direction estimates with a comparison, so a
    // last-bit difference between the GPU's and the CPU's arithmetic (fused
    // multiply-adds, for one) can flip a choice and move a value a lot. The
    // check is therefore on the distribution, as the spike's RCD test does.
    let mut d = Vec::new();
    // The reference is valid 9 px in; the GPU uses box3 there.
    for y in 12..H - 12 {
        for x in 12..W - 12 {
            let i = (y * W + x) as usize;
            for c in 0..3 {
                let (g, w) = (got[i * 3 + c], want[i][c]);
                d.push((g - w).abs() / w.abs().max(1e-2));
            }
        }
    }
    d.sort_by(f32::total_cmp);
    let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
    let mean = d.iter().map(|&v| f64::from(v)).sum::<f64>() / d.len() as f64;
    let flipped = d.iter().filter(|&&v| v > 1e-2).count();
    println!(
        "RCD: relative difference mean {mean:.1e}, median {:.1e}, 99.9 % {:.1e}, max {:.1e}; {flipped} of {} above 1 %",
        q(0.5),
        q(0.999),
        q(1.0),
        d.len()
    );
    assert!(mean < 1e-4, "{mean}");
    assert!(q(0.5) < 1e-6, "{}", q(0.5));
    assert!(flipped as f64 <= d.len() as f64 * 1e-3, "{flipped}");
}

/// The stored golden of stages 1 to 3 (camera RGB after RCD), tiled, on the
/// synthetic sensor. Written on lavapipe, compared on every other adapter.
#[test]
fn golden_raw_stages() {
    let Some((_lock, gpu)) = gpu() else { return };
    let rgb = render(&gpu, SMALL, &bayer(), Until::Stage3);
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "raw-stages/synthetic-bayer",
        genzo_golden::Space::Linear(genzo_golden::View::IDENTITY),
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    )
    // RCD's direction choices can flip between adapters that fuse
    // multiply-adds and those that do not (see rcd_matches_the_darktable_
    // reference), which moves a few pixels a lot. Measured WARP against the
    // RTX 3060: PSNR 77.6 dB, ΔE p99 0.015, 0.003 % of pixels above ΔE 1, max
    // 2.3. So the mean and PSNR stay tight and only the maximum is loosened.
    .with_tolerance(genzo_golden::Tolerance {
        mean_de: 0.05,
        max_de: 6.0,
        min_psnr_db: 65.0,
        max_abs: None,
    });
    let pixels = rgb.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    genzo_golden::check(&case, &genzo_golden::Image::new(W, H, pixels));
}
