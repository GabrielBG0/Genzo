//! Stage 1's CPU reference against the Adobe DNG SDK 1.4's stage 2 image of
//! six corpus DNGs (tests/fixtures/sdk-stage2): FixBadPixelsConstant, the
//! black and white levels and the GainMaps, over the ActiveArea. Needs the
//! corpus; a missing file is skipped.

mod sdk_stage2;

use genzo_golden::corpus;
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::stages::stage1;

/// The SDK stores stage 2 in 16 bits: linearized values are rounded to half
/// a code, a GainMap scales that error by its gain and rounds again, and its
/// fixed-point levels add a little. So the bound is half a code times
/// (1 + the largest gain), plus 0.1.
fn code_limit(src: &RawSource) -> f64 {
    let gain = src
        .gain_maps
        .iter()
        .flat_map(|g| g.gains.iter().copied())
        .fold(1.0f32, f32::max);
    0.5 * (1.0 + f64::from(gain)) + 0.1
}

const MEAN: f64 = 2e-6;

#[test]
fn cpu_stage1_matches_the_sdk() {
    for r in sdk_stage2::references() {
        let Some(path) = corpus(&r.key) else { continue };
        let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
        let src = RawSource::from_decoded(img).unwrap();
        assert_eq!(src.size(), (r.width, r.height), "{}", r.name);
        assert_eq!(src.samples, r.planes, "{}", r.name);
        let (codes, mean) = sdk_stage2::distance(&r, &stage1(&src));
        let limit = code_limit(&src);
        println!(
            "{}: {codes:.3} codes (limit {limit:.2}), block means {mean:.2e}",
            r.name
        );
        assert!(
            codes <= limit && mean <= MEAN,
            "{}: {codes} codes, {mean}",
            r.name
        );
    }
}

/// The comparison tells wrong semantics apart: each of these must fail.
#[test]
fn negative_controls_fail() {
    let refs = sdk_stage2::references();
    let load = |name: &str| {
        let r = refs.iter().find(|r| r.name == name).unwrap();
        let path = corpus(&r.key)?;
        let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
        Some((r, RawSource::from_decoded(img).unwrap()))
    };
    let fails = |r: &sdk_stage2::Reference, src: &RawSource| {
        let (codes, mean) = sdk_stage2::distance(r, &stage1(src));
        println!("{}: {codes:.2} codes, {mean:.2e}", r.name);
        codes > code_limit(src) || mean > MEAN
    };
    if let Some((r, src)) = load("pixel4a") {
        // Without the GainMaps.
        let mut s = src.clone();
        s.gain_maps.clear();
        assert!(fails(r, &s), "no gain maps");
        // The black level rows of the pattern swapped (1023 and 1022).
        let mut s = src.clone();
        let l = &mut s.linearization;
        let cols = l.cols as usize;
        let (a, b) = l.pattern.split_at_mut(cols);
        a.swap_with_slice(&mut b[..cols]);
        assert!(fails(r, &s), "black rows swapped");
    }
    if let Some((r, src)) = load("leica-q2") {
        // The active area two columns off (its right edge is 32 px inside
        // the sensor): same CFA phase, other samples.
        let mut s = src.clone();
        assert!(s.active.right + 2 <= s.stored.0);
        s.active.left += 2;
        s.active.right += 2;
        assert!(fails(r, &s), "active area off by two columns");
    }
}
