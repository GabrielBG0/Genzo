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
            "{mode:?}: gradients mean {:.2e}, worst {worst:.2e}",
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
