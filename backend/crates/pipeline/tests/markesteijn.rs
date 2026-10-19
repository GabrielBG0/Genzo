//! The X-Trans demosaic (DEV-01, stage 3: Markesteijn, 3 passes).
//!
//! The CPU port of darktable's C code (tests/markesteijn_reference) against
//! the truth on a synthetic sensor.

#[allow(dead_code)]
mod markesteijn_reference;

use genzo_pipeline::gpu::synthetic::scene;
use markesteijn_reference::{hexagons, markesteijn};

/// The Fujifilm X-T5's pattern from its sensor origin (cfa_corpus.rs).
pub const XT5: [[u8; 6]; 6] = {
    const R: u8 = 0;
    const G: u8 = 1;
    const B: u8 = 2;
    [
        [G, G, R, G, G, B],
        [G, G, B, G, G, R],
        [B, R, G, R, B, G],
        [G, G, B, G, G, R],
        [G, G, R, G, G, B],
        [R, B, G, B, R, G],
    ]
};

fn mosaic(w: usize, h: usize, rgb: impl Fn(usize, usize) -> [f32; 3]) -> Vec<f32> {
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            rgb(x, y)[XT5[y % 6][x % 6] as usize]
        })
        .collect()
}

#[test]
fn the_cpu_port_reproduces_a_known_scene() {
    let (w, h) = (300usize, 240usize);
    let truth = |x: usize, y: usize| scene(x as u32, y as u32, w as u32, h as u32);
    let cfa = mosaic(w, h, truth);
    {
        let out = markesteijn(&cfa, w, h, &XT5);
        // The smooth gradients (top 40 %), past the 17 px border.
        let mut worst = 0.0f32;
        let mut sum = 0.0f64;
        let mut n = 0;
        for y in 20..(h as f32 * 0.38) as usize {
            for x in 20..w - 20 {
                let t = truth(x, y);
                for c in 0..3 {
                    let d = (out[y * w + x][c] - t[c]).abs() / t[c].max(0.05);
                    worst = worst.max(d);
                    sum += f64::from(d);
                    n += 1;
                }
            }
        }
        println!(
            "gradients mean {:.2e}, worst {worst:.2e}",
            sum / f64::from(n)
        );
        assert!(worst < 0.03, "{worst}");
        // Flat patches: exact away from their edges.
        let (x, y) = (w / 12, (h as f32 * 0.5) as usize);
        let t = truth(x, y);
        for c in 0..3 {
            assert!(
                (out[y * w + x][c] - t[c]).abs() < 1e-5,
                "{c}: {:?} vs {t:?}",
                out[y * w + x]
            );
        }
    }
}

/// darktable's C code gives both pixels of a red/blue pair the first one's
/// green min/max; its OpenCL kernel, and Genzo's WGSL, give each pixel the
/// min/max over its own and its partner's hexagon of greens. The two agree
/// because a pair's pixels share their hexagon.
#[test]
fn a_pair_shares_its_hexagon_of_greens() {
    let (allhex, _) = hexagons(&XT5);
    let fc = |y: i64, x: i64| XT5[(y % 6) as usize][(x % 6) as usize];
    let greens = |y: i64, x: i64| {
        let hex = &allhex[(y % 3) as usize][(x % 3) as usize];
        let mut g: Vec<(i64, i64)> = hex[..6]
            .iter()
            .map(|&o| {
                // Offsets are h + v * 122 (the C tile stride).
                // h and v are within ±2, so v is o / 122 rounded.
                let v = (o + 61).div_euclid(122);
                let h = o - v * 122;
                (y + v, x + h)
            })
            .collect();
        g.sort();
        g
    };
    let mut pairs = 0;
    for y in 12..24 {
        for x in 12..24 {
            if fc(y, x) == 1 {
                continue;
            }
            for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                if fc(y + dy, x + dx) != 1 {
                    assert!(greens(y, x).iter().all(|&(gy, gx)| fc(gy, gx) == 1));
                    assert_eq!(greens(y, x), greens(y + dy, x + dx), "({x}, {y})");
                    pairs += 1;
                }
            }
        }
    }
    assert!(pairs > 50, "{pairs}");
}

/// LibRaw's Markesteijn (dcraw's 3-pass xtrans_interpolate) on the X-T5,
/// as interior crops (scripts/markesteijn-reference.py). The CPU port must
/// track it closely, and clearly closer than a plain linear demosaic. They
/// differ by design: LibRaw's homogeneity test works in CIELab, darktable's
/// in YPbPr.
#[test]
fn the_cpu_port_tracks_libraw_on_the_xt5() {
    use genzo_imgio::{DecodeOptions, Pixels, decode};
    let key = "raw/Fujifilm/X-T5/DSCF0021.RAF";
    let Some(path) = genzo_golden::corpus(key) else {
        return;
    };
    let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let Pixels::Cfa { pattern, data, raw } = &img.pixels else {
        panic!("a mosaic");
    };
    let width = img.width as usize;
    let mut xt = [[0u8; 6]; 6];
    for (y, row) in xt.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            *c = pattern.color_at(y, x);
        }
    }
    assert_eq!(xt, XT5);
    let l = &raw.levels;
    assert!(l.black.iter().all(|&b| b == l.black[0]));
    let (black, white) = (l.black[0] as f32, l.white[0] as f32);

    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/libraw-xt5-markesteijn.bin"
    ))
    .unwrap();
    assert_eq!(&bytes[..4], b"GZX1");
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let mut at = 8;
    let (mut ours, mut lin, mut theirs) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..u32_at(4) {
        let (x0, y0, cw, ch) = (u32_at(at), u32_at(at + 4), u32_at(at + 8), u32_at(at + 12));
        at += 16;
        // The crop with a 48 px margin (a multiple of 6, so the pattern
        // stays the sensor's), normalized as stage 1 does.
        const M: usize = 48;
        let (w, h) = (cw + 2 * M, ch + 2 * M);
        let cfa: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (x0 - M + i % w, y0 - M + i / w);
                (f32::from(data[y * width + x]) - black) / (white - black)
            })
            .collect();
        let m = markesteijn(&cfa, w, h, &XT5);
        let li = markesteijn_reference::linear(&cfa, w, h, &XT5);
        for y in 0..ch {
            for x in 0..cw {
                let i = (y + M) * w + x + M;
                let j = at + ((y * cw + x) * 3) * 2;
                ours.push(m[i]);
                lin.push(li[i]);
                theirs.push([0, 1, 2].map(|c| {
                    f32::from(u16::from_le_bytes([bytes[j + 2 * c], bytes[j + 2 * c + 1]]))
                        / 65535.0
                }));
            }
        }
        at += cw * ch * 6;
    }
    // Relative per-pixel differences where LibRaw is above 2 % of white
    // (and below its clip).
    let stats = |a: &[[f32; 3]]| {
        let mut d: Vec<f32> = a
            .iter()
            .zip(&theirs)
            .flat_map(|(p, q)| {
                (0..3)
                    .filter(move |&c| q[c] > 0.02 && q[c] < 0.98)
                    .map(move |c| (p[c] - q[c]).abs() / q[c])
            })
            .collect();
        d.sort_by(f32::total_cmp);
        let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
        (q(0.5), q(0.9), q(0.99), d.len())
    };
    let (m50, m90, m99, n) = stats(&ours);
    let (l50, l90, l99, _) = stats(&lin);
    println!("Markesteijn vs LibRaw: median {m50:.4}, 90 % {m90:.4}, 99 % {m99:.4} ({n} values)");
    println!("linear      vs LibRaw: median {l50:.4}, 90 % {l90:.4}, 99 % {l99:.4}");
    // Measured (2026-10-18): median 0.0003, 90 % 0.0073; linear 0.0048 and
    // 0.0448.
    assert!(m50 < 0.002, "median {m50}");
    assert!(m90 < 0.25 * l90, "{m90} against linear {l90}");
}

// The GPU port (gpu::raw_stages, tile_xtrans.wgsl) on a synthetic X-Trans
// sensor whose active area starts 4 and 5 px in (1 and 2 mod 3), so the
// stored-origin pattern, hexagons and solitary green phase all matter.

use std::sync::{Mutex, MutexGuard};

use genzo_imgio::raw::{Levels, Rect};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
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
const H: u32 = 222;
const LEFT: u32 = 4;
const TOP: u32 = 5;
const WB: [f32; 3] = [1.9, 1.0, 1.5];

fn xtrans_sensor() -> RawSource {
    let stored = (W + LEFT + 6, H + TOP + 7);
    let levels = Levels {
        black: vec![1020.0],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![16383],
    };
    let lin = Linearization::new(&levels, 1, (W, H)).unwrap();
    let range = 16383.0 - 1020.0;
    let mut data = vec![1100u16; (stored.0 * stored.1) as usize];
    for ay in 0..H {
        for ax in 0..W {
            let (x, y) = (ax + LEFT, ay + TOP);
            let c = usize::from(XT5[(y % 6) as usize][(x % 6) as usize]);
            let v = f64::from(scene(ax, ay, W, H)[c]) * 0.9 / f64::from(WB[c]);
            let noise = f64::from((x * 7 + y * 13) % 9) - 4.0;
            data[(y * stored.0 + x) as usize] = (v * range + 1020.0 + noise).round() as u16;
        }
    }
    RawSource {
        stored,
        samples: 1,
        data,
        cfa: Some(Cfa::XTrans(XT5)),
        active: Rect {
            top: TOP,
            left: LEFT,
            bottom: TOP + H,
            right: LEFT + W,
        },
        linearization: lin,
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color: Default::default(),
    }
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

fn render(gpu: &Gpu, config: TileConfig, src: &RawSource) -> Vec<f32> {
    RawStages::new(gpu, config)
        .render(gpu, src, WB, Until::Stage3)
        .unwrap()
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

#[test]
fn gpu_tiles_equal_one_tile_bit_for_bit() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = xtrans_sensor();
    let whole = render(&gpu, ONE_TILE, &src);
    // Determinism first: a race would show here.
    assert_eq!(
        first_difference(&whole, &render(&gpu, ONE_TILE, &src)),
        None,
        "two runs"
    );
    let tiled = render(&gpu, SMALL, &src);
    assert_eq!(first_difference(&whole, &tiled), None, "tiled");
    // 12 px is less than the 3-pass support (17).
    let narrow = TileConfig { apron: 12, ..SMALL };
    assert!(first_difference(&whole, &render(&gpu, narrow, &src)).is_some());
}

#[test]
fn gpu_matches_the_cpu_port() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = xtrans_sensor();
    let got = render(&gpu, SMALL, &src);
    let mut cfa = stage1(&src);
    stage2(&mut cfa, &src, WB);
    cfa.iter_mut().for_each(|v| *v = v.max(0.0));
    // The pattern from the active area's origin, which the port takes.
    let mut active = [[0u8; 6]; 6];
    for (r, row) in active.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = src.color_at(r as u32, c as u32).unwrap();
        }
    }
    let want = markesteijn(&cfa, W as usize, H as usize, &active);
    let mut d = Vec::new();
    for (i, w) in want.iter().enumerate() {
        for c in 0..3 {
            let g = got[i * 3 + c];
            d.push((g - w[c]).abs() / w[c].abs().max(0.05));
        }
    }
    d.sort_by(f32::total_cmp);
    let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
    let flipped = d.iter().filter(|&&v| v > 1e-2).count();
    println!(
        "GPU vs CPU port: median {:.1e}, 99.9 % {:.1e}, max {:.1e}; {flipped} of {} above 1 %",
        q(0.5),
        q(0.999),
        q(1.0),
        d.len()
    );
    assert!(q(0.5) < 1e-6, "{}", q(0.5));
    // Markesteijn picks directions by comparing derivatives and counts, so
    // a last-bit difference (the GPU's fused multiply-adds, a dot product's
    // order, a multiply by 1/3 for a divide) moves a few values far, on high
    // contrast. Those must be values where the algorithm itself is that
    // sensitive: where the CPU port moves as far when its input is nudged
    // by a few parts in 10^7 to 10^6. Measured (2026-10-19): of the GPU's
    // values above 1 %, 29 of 29 on WARP and 82 of 94 on the RTX 3060 are
    // such values (208 of them); 80 % must be.
    let mut sensitive = vec![false; want.len() * 3];
    for nudge in [
        1e-7f32, -1e-7, 3e-7, -3e-7, 5e-7, -5e-7, 1e-6, -1e-6, 2e-6, -2e-6, 3e-6, -3e-6,
    ] {
        let nudged: Vec<f32> = cfa.iter().map(|v| v * (1.0 + nudge)).collect();
        let again = markesteijn(&nudged, W as usize, H as usize, &active);
        for (i, (p, q)) in want.iter().zip(&again).enumerate() {
            for c in 0..3 {
                if (p[c] - q[c]).abs() / p[c].abs().max(0.05) > 1e-2 {
                    sensitive[i * 3 + c] = true;
                }
            }
        }
    }
    let rel = |k: usize| (got[k] - want[k / 3][k % 3]).abs() / want[k / 3][k % 3].abs().max(0.05);
    let gpu_flips: Vec<usize> = (0..want.len() * 3).filter(|&k| rel(k) > 1e-2).collect();
    let explained = gpu_flips.iter().filter(|&&k| sensitive[k]).count();
    let total = sensitive.iter().filter(|&&s| s).count();
    println!(
        "{explained} of {} GPU flips at values the CPU port flips under a nudge ({total} such values)",
        gpu_flips.len()
    );
    assert!(
        explained * 10 >= gpu_flips.len() * 8,
        "{explained} of {}",
        gpu_flips.len()
    );
}

/// One of LibRaw's crops: sensor (x, y), size and RGB, white at 1.
type Crop = ((usize, usize), (usize, usize), Vec<[f32; 3]>);

fn libraw_crops() -> Vec<Crop> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/libraw-xt5-markesteijn.bin"
    ))
    .unwrap();
    assert_eq!(&bytes[..4], b"GZX1");
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let mut at = 8;
    let mut crops = Vec::new();
    for _ in 0..u32_at(4) {
        let (x, y, w, h) = (u32_at(at), u32_at(at + 4), u32_at(at + 8), u32_at(at + 12));
        at += 16;
        let rgb = (0..w * h)
            .map(|i| {
                [0, 1, 2].map(|c| {
                    let j = at + (i * 3 + c) * 2;
                    f32::from(u16::from_le_bytes([bytes[j], bytes[j + 1]])) / 65535.0
                })
            })
            .collect();
        at += w * h * 6;
        crops.push(((x, y), (w, h), rgb));
    }
    crops
}

/// The GPU's own render of the X-T5 (stages 1 to 3, no white balance)
/// against LibRaw, with the CPU port's bounds (the_cpu_port_tracks_libraw).
#[test]
fn gpu_tracks_libraw_on_the_xt5() {
    use genzo_imgio::{DecodeOptions, decode};
    let Some((_lock, gpu)) = gpu() else { return };
    let Some(path) = genzo_golden::corpus("raw/Fujifilm/X-T5/DSCF0021.RAF") else {
        return;
    };
    let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let src = RawSource::from_decoded(img).unwrap();
    assert_eq!(src.cfa, Some(Cfa::XTrans(XT5)));
    let rgb = RawStages::new(&gpu, TileConfig::default())
        .render(&gpu, &src, [1.0; 3], Until::Stage3)
        .unwrap();
    let width = src.size().0 as usize;
    let (left, top) = (src.active.left as usize, src.active.top as usize);
    let mut d = Vec::new();
    for ((x0, y0), (w, h), theirs) in libraw_crops() {
        for y in 0..h {
            for x in 0..w {
                let i = (y0 + y - top) * width + (x0 + x - left);
                let q = theirs[y * w + x];
                for c in 0..3 {
                    if q[c] > 0.02 && q[c] < 0.98 {
                        d.push((rgb[i * 3 + c] - q[c]).abs() / q[c]);
                    }
                }
            }
        }
    }
    d.sort_by(f32::total_cmp);
    let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
    println!(
        "GPU vs LibRaw: median {:.4}, 90 % {:.4}, 99 % {:.4} ({} values)",
        q(0.5),
        q(0.9),
        q(0.99),
        d.len()
    );
    assert!(q(0.5) < 0.002 && q(0.9) < 0.0112, "{} {}", q(0.5), q(0.9));
}

/// The stored golden of the X-Trans path, stages 1 to 3 (camera RGB after
/// Markesteijn), tiled, on the synthetic sensor. Written on lavapipe.
#[test]
fn golden_xtrans_stages() {
    let Some((_lock, gpu)) = gpu() else { return };
    let rgb = render(&gpu, SMALL, &xtrans_sensor());
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "raw-stages/synthetic-xtrans",
        genzo_golden::Space::Linear(genzo_golden::View::IDENTITY),
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    )
    // As for RCD: direction choices that flip between adapters with and
    // without fused multiply-adds move a few pixels a lot (see
    // gpu_matches_the_cpu_port), so only the maximum is loosened. Measured,
    // WARP against the RTX 3060: PSNR 70.0 dB, ΔE mean 0.004, p99 0.032,
    // 0.05 % of pixels above ΔE 1, and a max of 15.2 where a flip lands in
    // a dark saturated pixel (0.048 in linear terms).
    .with_tolerance(genzo_golden::Tolerance {
        mean_de: 0.05,
        max_de: 20.0,
        min_psnr_db: 65.0,
        max_abs: None,
    });
    let pixels = rgb.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    genzo_golden::check(&case, &genzo_golden::Image::new(W, H, pixels));
}
