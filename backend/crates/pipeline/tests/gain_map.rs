//! The GainMap CPU reference against Adobe DNG Converter, on the Google
//! Pixel 4a DNG (CC0, raw.pixls.us): four 30 x 40 lens-shading maps in
//! OpcodeList2, gains 1.0 to 3.4, one per Bayer site.
//!
//! Adobe's linear rendering of the file has the maps applied; it is stored
//! here as per-channel means of 64 x 64 blocks (scripts/gainmap-reference.py,
//! tests/fixtures/pixel4a-adobe-blocks.f32). Genzo's mosaic with the maps
//! applied must be proportional to it in every block: the ratio is flat for
//! the right maps, and visibly not for no maps or for the red and blue maps
//! swapped (both checked, so the test can fail). Adobe's demosaic leaves
//! about 6 % of block-to-block spread in red and blue, under 1 % in green.

use genzo_golden::corpus;
use genzo_imgio::opcodes::{GainMap, StageOpcode};
use genzo_imgio::{DecodeOptions, Pixels, decode};
use genzo_pipeline::reference::opcodes::apply_gain_map;

const BLOCK: usize = 64;
const KEY: &str = "raw/Google/Pixel 4a/PXL_20201121_100251397.dng";

struct Mosaic {
    width: usize,
    height: usize,
    /// Normalized samples: 0 at black, 1 at white.
    samples: Vec<f32>,
    /// The color (0 R, 1 G, 2 B) of each site.
    colors: Vec<u8>,
    maps: Vec<GainMap>,
}

fn load() -> Option<Mosaic> {
    let bytes = std::fs::read(corpus(KEY)?).unwrap();
    let img = decode(&bytes, DecodeOptions::default()).unwrap();
    let Pixels::Cfa { pattern, data, raw } = &img.pixels else {
        panic!("expected a CFA mosaic");
    };
    let (w, h) = (img.width as usize, img.height as usize);
    // The Pixel's active area is the whole sensor, so the image OpcodeList2
    // runs on is the decoded one.
    let area = raw.active_area.expect("an active area");
    assert_eq!(
        (area.top, area.left, area.bottom, area.right),
        (0, 0, h as u32, w as u32)
    );
    let levels = &raw.levels;
    let (br, bc) = (
        usize::from(levels.black_repeat.0),
        usize::from(levels.black_repeat.1),
    );
    let white = f64::from(levels.white[0]);
    let mut samples = Vec::with_capacity(w * h);
    let mut colors = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let black = levels.black[(y % br) * bc + x % bc];
            samples.push(((f64::from(data[y * w + x]) - black) / (white - black)) as f32);
            colors.push(pattern.color_at(y, x));
        }
    }
    let maps: Vec<GainMap> = raw
        .opcode_plan
        .list2
        .iter()
        .map(|op| match op {
            StageOpcode::GainMap(g) => g.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(maps.len(), 4, "one map per Bayer site");
    Some(Mosaic {
        width: w,
        height: h,
        samples,
        colors,
        maps,
    })
}

fn adobe_blocks() -> Vec<[f32; 3]> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pixel4a-adobe-blocks.f32"
    ))
    .unwrap();
    bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|b| [0, 1, 2].map(|c| f32::from_le_bytes(b[c * 4..c * 4 + 4].try_into().unwrap())))
        .collect()
}

/// For each color, the median of Adobe / Genzo over the interior blocks,
/// and the spread of that ratio (99th over 1st percentile).
fn ratios(m: &Mosaic, image: &[f32]) -> [(f64, f64); 3] {
    let adobe = adobe_blocks();
    let (bw, bh) = (m.width / BLOCK, m.height / BLOCK);
    assert_eq!(adobe.len(), bw * bh, "block grid");
    let mut per_color: [Vec<f64>; 3] = Default::default();
    // The outer ring of blocks is left out: Adobe's demosaic has edges.
    for by in 1..bh - 1 {
        for bx in 1..bw - 1 {
            let (mut sum, mut n) = ([0.0f64; 3], [0u32; 3]);
            for y in by * BLOCK..(by + 1) * BLOCK {
                for x in bx * BLOCK..(bx + 1) * BLOCK {
                    let c = usize::from(m.colors[y * m.width + x]);
                    sum[c] += f64::from(image[y * m.width + x]);
                    n[c] += 1;
                }
            }
            for c in 0..3 {
                let a = f64::from(adobe[by * bw + bx][c]);
                // Dark blocks are left out: their ratio is mostly noise.
                if a > 0.02 {
                    per_color[c].push(a / (sum[c] / f64::from(n[c])));
                }
            }
        }
    }
    per_color.map(|mut r| {
        r.sort_by(f64::total_cmp);
        let at = |q: f64| r[((r.len() - 1) as f64 * q) as usize];
        (at(0.5), at(0.99) / at(0.01))
    })
}

fn with_maps(m: &Mosaic, maps: &[GainMap]) -> Vec<f32> {
    let mut image = m.samples.clone();
    for gm in maps {
        apply_gain_map(&mut image, m.width as u32, m.height as u32, 1, gm);
    }
    image
}

#[test]
fn gain_maps_match_adobe() {
    let Some(m) = load() else { return };

    let applied = ratios(&m, &with_maps(&m, &m.maps));
    println!("applied:  {applied:.4?}");
    for (c, (median, spread)) in applied.iter().enumerate() {
        let limit = if c == 1 { 1.02 } else { 1.10 };
        assert!((median - 1.0).abs() < 0.01, "color {c}: median {median}");
        assert!(*spread < limit, "color {c}: spread {spread}, limit {limit}");
    }

    // Negative controls: the comparison tells a wrong result apart.
    let none = ratios(&m, &m.samples);
    println!("no maps:  {none:.4?}");
    assert!(none.iter().all(|(_, spread)| *spread > 2.0), "{none:?}");

    // The red and blue sites' maps swapped (Bayer sites (0, 0) and (1, 1)).
    let site = |g: &GainMap| (g.area.top & 1, g.area.left & 1);
    let swapped: Vec<GainMap> = m
        .maps
        .iter()
        .map(|g| {
            let mut g = g.clone();
            if matches!(site(&g), (0, 0) | (1, 1)) {
                let other = 1 - g.area.top % 2;
                g.area.top = other;
                g.area.left = other;
            }
            g
        })
        .collect();
    let wrong = ratios(&m, &with_maps(&m, &swapped));
    println!("swapped:  {wrong:.4?}");
    assert!(wrong[0].1 > 1.15 && wrong[2].1 > 1.15, "{wrong:?}");
}
