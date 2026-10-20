//! DEV-02 criterion 5: Genzo's render to the end of stage 6 with Genzo
//! Standard against the Adobe DNG SDK 1.4's `dng_render` (the probe's 64 px
//! block means in linear ProPhoto, tests/fixtures/sdk-render, written by
//! scripts/render-reference.py). Each corpus file is rendered at 1:1 over
//! its default crop, converted to linear ProPhoto and averaged the same way.
//!
//! The demosaics differ (RCD against the SDK's), and Genzo desaturates
//! highlights near the clip level, so the comparison is on block means. A
//! local corpus test: a missing file skips.

#[allow(dead_code)]
mod sdk_stage2;

use genzo_color::mul;
use genzo_color::tone::working_to_prophoto;
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::ViewSpec;
use serde::Deserialize;

#[derive(Deserialize)]
struct Render {
    key: String,
    stage3: [u32; 2],
    /// Left, top, right, bottom in the stage 3 image (the active area).
    crop: [u32; 4],
    exposure: f32,
    block: u32,
    means: Vec<Vec<[f64; 3]>>,
}

/// The view chunk the default crop is rendered in.
const CHUNK: u32 = 2048;

/// Genzo's block means over the default crop, in linear ProPhoto.
fn genzo_means(gpu: &Gpu, src: &RawSource, r: &Render) -> Vec<Vec<[f64; 3]>> {
    let mut stages = RawStages::new(gpu, TileConfig::default());
    stages
        .render_pyramid(gpu, src, &camera_transform(&src.color, None))
        .unwrap();
    let mut cache = Level0Cache::new(gpu);
    let mut view = ViewStages::new(gpu);
    let base = BaseCurve::new(&src.color);
    assert_eq!(base.curve_error, None, "{}", r.key);
    let look = Look {
        exposure: r.exposure,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    let to_prophoto = working_to_prophoto();
    let [left, top, right, bottom] = r.crop;
    let (bw, bh) = (r.means[0].len() as u32, r.means.len() as u32);
    assert_eq!(
        (bw, bh),
        ((right - left) / r.block, (bottom - top) / r.block)
    );
    let mut sums = vec![[0f64; 3]; (bw * bh) as usize];
    let (w, h) = (bw * r.block, bh * r.block);
    for cy in (0..h).step_by(CHUNK as usize) {
        for cx in (0..w).step_by(CHUNK as usize) {
            let out = (CHUNK.min(w - cx), CHUNK.min(h - cy));
            let spec = ViewSpec::Region {
                out,
                zoom: 1.0,
                origin: (f64::from(left + cx), f64::from(top + cy)),
            };
            let s = view
                .show(gpu, &mut stages, &mut cache, src, 1, 1, spec)
                .unwrap();
            assert_eq!(s.level, 0, "1:1 reads level 0");
            view.tone(gpu, &look, Some(&base), None);
            let pixels = view.read_latest(gpu).unwrap();
            for y in 0..out.1 {
                for x in 0..out.0 {
                    let p = mul(
                        &to_prophoto,
                        pixels[(y * out.0 + x) as usize].map(f64::from),
                    );
                    let b = ((cy + y) / r.block * bw + (cx + x) / r.block) as usize;
                    for c in 0..3 {
                        sums[b][c] += p[c];
                    }
                }
            }
        }
    }
    let n = f64::from(r.block * r.block);
    (0..bh)
        .map(|by| {
            (0..bw)
                .map(|bx| sums[(by * bw + bx) as usize].map(|v| v / n))
                .collect()
        })
        .collect()
}

#[test]
fn criterion_5_whole_render_against_the_sdk() {
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdk-render");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 8);
    let mut compared = 0;
    for path in files {
        let r: Render = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let Some(file) = sdk_stage2::corpus_file(&r.key) else {
            continue;
        };
        let img = decode(&std::fs::read(file).unwrap(), DecodeOptions::default()).unwrap();
        let src = RawSource::from_decoded(img).unwrap();
        assert_eq!(
            src.size(),
            (r.stage3[0], r.stage3[1]),
            "{}: stage 3 is the active area",
            r.key
        );
        let genzo = genzo_means(&gpu, &src, &r);
        let mut diffs: Vec<f64> = genzo
            .iter()
            .flatten()
            .zip(r.means.iter().flatten())
            .flat_map(|(g, s)| (0..3).map(move |c| (g[c] - s[c]).abs()))
            .collect();
        diffs.sort_by(f64::total_cmp);
        let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let p99 = diffs[diffs.len() * 99 / 100];
        let name = path.file_name().unwrap().to_string_lossy();
        let (worst_at, g, sdk) = genzo
            .iter()
            .flatten()
            .zip(r.means.iter().flatten())
            .enumerate()
            .max_by(|a, b| {
                let d = |(_, (g, s)): &(usize, (&[f64; 3], &[f64; 3]))| {
                    (0..3).map(|c| (g[c] - s[c]).abs()).fold(0.0, f64::max)
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, (g, s))| (i, *g, *s))
            .unwrap();
        let bw = r.means[0].len();
        println!(
            "{name}: worst block ({}, {}): Genzo {g:.3?}, SDK {sdk:.3?}",
            worst_at % bw,
            worst_at / bw
        );
        println!(
            "{name}: mean difference {:.3} %, 99th percentile {:.3} %, worst {:.3} %",
            mean * 100.0,
            p99 * 100.0,
            diffs.last().unwrap() * 100.0
        );
        assert!(mean <= 0.005, "{name}: mean {mean}");
        assert!(p99 <= 0.02, "{name}: 99th percentile {p99}");
        compared += 1;
    }
    println!("{compared} renders compared");
}
