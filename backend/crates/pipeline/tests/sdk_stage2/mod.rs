//! The DNG SDK's stage 2 images of corpus DNGs, as crops and block means
//! (tests/fixtures/sdk-stage2, written by scripts/stage1-reference.py), and
//! the comparison of a stage 1 result against them.

use std::path::Path;

pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Planes interleaved, 65535 = 1.
    pub samples: Vec<u16>,
}

pub struct Reference {
    pub name: String,
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub planes: u32,
    pub block: u32,
    pub crops: Vec<Crop>,
    /// Per block, row by row, planes interleaved.
    pub blocks: Vec<f32>,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

pub fn references() -> Vec<Reference> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdk-stage2");
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).unwrap()).unwrap();
    index
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let name = e["name"].as_str().unwrap().to_owned();
            let b = std::fs::read(dir.join(format!("{name}.bin"))).unwrap();
            assert_eq!(&b[..4], b"GZC1");
            let (width, height, planes, block, n) = (
                u32_at(&b, 4),
                u32_at(&b, 8),
                u32_at(&b, 12),
                u32_at(&b, 16),
                u32_at(&b, 20),
            );
            let mut at = 24;
            let mut crops = Vec::new();
            for _ in 0..n {
                let (x, y, w, h) = (
                    u32_at(&b, at),
                    u32_at(&b, at + 4),
                    u32_at(&b, at + 8),
                    u32_at(&b, at + 12),
                );
                at += 16;
                let len = (w * h * planes) as usize;
                let samples = b[at..at + 2 * len]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                at += 2 * len;
                crops.push(Crop {
                    x,
                    y,
                    w,
                    h,
                    samples,
                });
            }
            let blocks = b[at..]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect::<Vec<_>>();
            assert_eq!(
                blocks.len() as u32,
                (width / block) * (height / block) * planes,
                "{name}: block means"
            );
            Reference {
                name,
                key: e["key"].as_str().unwrap().to_owned(),
                width,
                height,
                planes,
                block,
                crops,
                blocks,
            }
        })
        .collect()
}

/// How far a stage 1 result (`width` × `height`, planes interleaved, white
/// at 1) is from the SDK's: the largest difference in 16-bit codes over the
/// crops, and the largest difference of a block mean. The SDK clips at 0 and
/// 1 and stores 16 bits, so the result is clipped and compared as such.
pub fn distance(r: &Reference, image: &[f32]) -> (f64, f64) {
    let n = r.planes as usize;
    let at = |x: u32, y: u32, p: usize| {
        f64::from(image[(y as usize * r.width as usize + x as usize) * n + p]).clamp(0.0, 1.0)
    };
    let mut codes = 0.0f64;
    for c in &r.crops {
        for y in 0..c.h {
            for x in 0..c.w {
                for p in 0..n {
                    let want = f64::from(c.samples[((y * c.w + x) as usize) * n + p]);
                    let got = at(c.x + x, c.y + y, p) * 65535.0;
                    codes = codes.max((got - want).abs());
                }
            }
        }
    }
    let (bw, bh) = (r.width / r.block, r.height / r.block);
    let mut mean = 0.0f64;
    for by in 0..bh {
        for bx in 0..bw {
            for p in 0..n {
                let mut sum = 0.0;
                for y in by * r.block..(by + 1) * r.block {
                    for x in bx * r.block..(bx + 1) * r.block {
                        // The SDK's 16-bit rounding, so both means see it.
                        sum += (at(x, y, p) * 65535.0).round() / 65535.0;
                    }
                }
                let got = sum / f64::from(r.block * r.block);
                let want = f64::from(r.blocks[((by * bw + bx) as usize) * n + p]);
                mean = mean.max((got - want).abs());
            }
        }
    }
    (codes, mean)
}
