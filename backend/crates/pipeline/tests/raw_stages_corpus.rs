//! Stages 1 to 3 on the GPU, on corpus files (a missing file is skipped):
//!
//! - stages 1 and 2 with unit multipliers against the DNG SDK's stage 2
//!   image of six DNGs (tests/fixtures/sdk-stage2), the bound the CPU
//!   reference meets in stage1_sdk.rs;
//! - the Pixel 4a through the demosaic against Adobe DNG Converter's linear
//!   rendering, per 64 px block (tests/fixtures/pixel4a-adobe-blocks.f32):
//!   the ratio is as flat as gain_map.rs requires.

#[allow(dead_code)]
mod sdk_stage2;

use std::sync::{Mutex, MutexGuard};

use genzo_golden::corpus;
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => Some((lock, gpu)),
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn load(key: &str) -> Option<RawSource> {
    let bytes = std::fs::read(corpus(key)?).unwrap();
    Some(RawSource::from_decoded(decode(&bytes, DecodeOptions::default()).unwrap()).unwrap())
}

/// As in stage1_sdk.rs: half a code times (1 + the largest gain), plus 0.1.
fn code_limit(src: &RawSource) -> f64 {
    let gain = src
        .gain_maps
        .iter()
        .flat_map(|g| g.gains.iter().copied())
        .fold(1.0f32, f32::max);
    0.5 * (1.0 + f64::from(gain)) + 0.1
}

#[test]
fn gpu_stage1_matches_the_sdk() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    for r in sdk_stage2::references() {
        let Some(src) = load(&r.key) else { continue };
        let got = stages.render(&gpu, &src, [1.0; 3], Until::Stage2).unwrap();
        // A monochrome or linear plane comes back as RGB; the SDK has the
        // file's planes.
        let got: Vec<f32> = if src.bayer.is_none() && src.samples == 1 {
            got.chunks(3).map(|c| c[0]).collect()
        } else {
            got
        };
        let (codes, mean) = sdk_stage2::distance(&r, &got);
        let limit = code_limit(&src);
        println!(
            "{}: {codes:.3} codes (limit {limit:.2}), block means {mean:.2e}",
            r.name
        );
        assert!(
            codes <= limit && mean <= 2e-6,
            "{}: {codes}, {mean}",
            r.name
        );
    }
}

const BLOCK: usize = 64;

#[test]
fn pixel4a_demosaic_matches_adobe() {
    let Some((_lock, gpu)) = gpu() else { return };
    let Some(src) = load("raw/Google/Pixel 4a/PXL_20201121_100251397.dng") else {
        return;
    };
    let rgb = RawStages::new(&gpu, TileConfig::default())
        .render(&gpu, &src, [1.0; 3], Until::Stage3)
        .unwrap();
    let adobe: Vec<[f32; 3]> = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pixel4a-adobe-blocks.f32"
    ))
    .unwrap()
    .as_chunks::<12>()
    .0
    .iter()
    .map(|b| [0, 1, 2].map(|c| f32::from_le_bytes(b[c * 4..c * 4 + 4].try_into().unwrap())))
    .collect();
    let (w, h) = src.size();
    let (bw, bh) = (w as usize / BLOCK, h as usize / BLOCK);
    assert_eq!(adobe.len(), bw * bh);
    let mut ratios: [Vec<f64>; 3] = Default::default();
    // The outer ring of blocks is left out, and dark blocks (noise).
    for by in 1..bh - 1 {
        for bx in 1..bw - 1 {
            let mut sum = [0.0f64; 3];
            for y in by * BLOCK..(by + 1) * BLOCK {
                for x in bx * BLOCK..(bx + 1) * BLOCK {
                    let i = (y * w as usize + x) * 3;
                    for c in 0..3 {
                        sum[c] += f64::from(rgb[i + c]);
                    }
                }
            }
            for c in 0..3 {
                let a = f64::from(adobe[by * bw + bx][c]);
                if a > 0.02 {
                    ratios[c].push(a / (sum[c] / (BLOCK * BLOCK) as f64));
                }
            }
        }
    }
    for (c, mut r) in ratios.into_iter().enumerate() {
        r.sort_by(f64::total_cmp);
        let at = |q: f64| r[((r.len() - 1) as f64 * q) as usize];
        let (median, spread) = (at(0.5), at(0.99) / at(0.01));
        println!("channel {c}: median {median:.4}, spread {spread:.4}");
        // The spread is gain_map.rs's limit: the lens shading is gone in
        // every block. The medians differ by more than there, because two
        // demosaics do not keep a block's mean alike: in the sparse red and
        // blue channels RCD's block means run 0.7 to 1.3 % above the mean of
        // the red or blue sites (measured, 2026-10-18), Adobe's about 0.6 %
        // below; green agrees to 0.1 %.
        let limit = if c == 1 { 1.02 } else { 1.10 };
        let off = if c == 1 { 0.005 } else { 0.025 };
        assert!((median - 1.0).abs() < off, "channel {c}: median {median}");
        assert!(spread < limit, "channel {c}: spread {spread}");
    }
}
