//! The camera color math against the Adobe DNG SDK 1.4, on the corpus DNGs'
//! own tag values (tests/fixtures/dng-sdk-color.jsonl, written by
//! scripts/color-reference.py with the SDK probe in scripts/dng-sdk-probe).
//! The fixture holds the inputs, so this needs no corpus.
//!
//! The probe prints 12 decimals; a faithful f64 port agrees to about that,
//! so the tolerances are tight. Negative controls check that the comparison
//! tells a wrong calibration gate, a wrong matrix order and a missing 1e-4
//! rounding apart.

use genzo_color::camera::{
    AsShot, ColorSpec, DngProfile, temperature_from_xy, xy_from_temperature,
};
use genzo_color::{Mat3, mat_mul, mul};
use serde::Deserialize;

#[derive(Deserialize)]
struct Record {
    key: String,
    variant: Option<String>,
    inputs: Inputs,
    sdk: Sdk,
}

#[derive(Deserialize, Clone)]
struct Inputs {
    color_matrix_1: Vec<f64>,
    color_matrix_2: Option<Vec<f64>>,
    illuminant_1: Option<u16>,
    illuminant_2: Option<u16>,
    forward_matrix_1: Option<Vec<f64>>,
    forward_matrix_2: Option<Vec<f64>>,
    camera_calibration_1: Option<Vec<f64>>,
    camera_calibration_2: Option<Vec<f64>>,
    analog_balance: Option<Vec<f64>>,
    camera_calibration_signature: Option<String>,
    profile_calibration_signature: Option<String>,
    as_shot_neutral: Option<Vec<f64>>,
    as_shot_white_xy: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct Sdk {
    as_shot_source: String,
    specs: Vec<Spec>,
}

#[derive(Deserialize)]
struct Spec {
    white: String,
    xy: [f64; 2],
    temperature: f64,
    tint: f64,
    camera_white: [f64; 3],
    camera_to_pcs: Vec<f64>,
}

fn records() -> Vec<Record> {
    let text = include_str!("fixtures/dng-sdk-color.jsonl");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn mat(v: &[f64]) -> Mat3 {
    assert_eq!(v.len(), 9, "three color channels");
    [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]
}

fn profile(i: &Inputs) -> DngProfile {
    let m = |v: &Option<Vec<f64>>| v.as_deref().map(mat);
    DngProfile {
        color_matrix_1: mat(&i.color_matrix_1),
        illuminant_1: i.illuminant_1.unwrap_or(0),
        color_matrix_2: m(&i.color_matrix_2),
        illuminant_2: i.illuminant_2.unwrap_or(0),
        forward_matrix_1: m(&i.forward_matrix_1),
        forward_matrix_2: m(&i.forward_matrix_2),
        camera_calibration_1: m(&i.camera_calibration_1),
        camera_calibration_2: m(&i.camera_calibration_2),
        analog_balance: i
            .analog_balance
            .as_deref()
            .map_or([1.0; 3], |v| [v[0], v[1], v[2]]),
        camera_calibration_signature: i.camera_calibration_signature.clone().unwrap_or_default(),
        profile_calibration_signature: i.profile_calibration_signature.clone().unwrap_or_default(),
    }
}

fn as_shot(i: &Inputs) -> Option<AsShot> {
    match (&i.as_shot_neutral, &i.as_shot_white_xy) {
        (Some(n), _) => Some(AsShot::Neutral([n[0], n[1], n[2]])),
        (None, Some(xy)) => Some(AsShot::WhiteXy([xy[0], xy[1]])),
        (None, None) => None,
    }
}

/// The largest difference from the SDK over every white of a record:
/// (xy, temperature, tint, camera white, CameraToPCS).
fn worst(spec: &ColorSpec, r: &Record) -> [f64; 5] {
    let mut worst = [0.0f64; 5];
    let mut at = |k: usize, d: f64| worst[k] = worst[k].max(d.abs());
    let xy = spec
        .as_shot_xy(as_shot(&r.inputs))
        .expect("an as-shot white");
    let sdk = &r.sdk.specs[0];
    assert_eq!(sdk.white, "as_shot");
    at(0, xy[0] - sdk.xy[0]);
    at(0, xy[1] - sdk.xy[1]);
    for s in &r.sdk.specs {
        let (t, tint) = temperature_from_xy(s.xy);
        at(1, t - s.temperature);
        at(2, tint - s.tint);
        let Some(w) = spec.at_white(s.xy) else {
            at(3, f64::INFINITY);
            continue;
        };
        for c in 0..3 {
            at(3, w.camera_white[c] - s.camera_white[c]);
        }
        let m = mat(&s.camera_to_pcs);
        for (a, b) in w.camera_to_pcs.iter().flatten().zip(m.iter().flatten()) {
            at(4, a - b);
        }
    }
    worst
}

/// xy, temperature, tint, camera white, CameraToPCS. The probe prints xy,
/// camera white and matrices to 12 decimals, temperature and tint to 9.
const LIMITS: [f64; 5] = [1e-11, 1e-7, 5e-9, 1e-11, 1e-10];

fn within(w: &[f64; 5]) -> bool {
    w.iter().zip(LIMITS).all(|(d, l)| *d <= l)
}

#[test]
fn matches_the_sdk() {
    let records = records();
    assert!(records.len() >= 19, "{} records", records.len());
    for r in &records {
        let spec = ColorSpec::new(&profile(&r.inputs)).unwrap();
        let w = worst(&spec, r);
        println!("{} {:?}: {w:?}", r.key, r.variant);
        assert!(within(&w), "{} {:?}: {w:?}", r.key, r.variant);
        let source = match as_shot(&r.inputs) {
            Some(AsShot::Neutral(_)) => "neutral",
            Some(AsShot::WhiteXy(_)) => "white_xy",
            None => "default_d55",
        };
        assert_eq!(r.sdk.as_shot_source, source, "{}", r.key);
    }
}

#[test]
fn covers_the_sdk_branches() {
    let records = records();
    let has = |f: &dyn Fn(&Inputs) -> bool| records.iter().any(|r| f(&r.inputs));
    // ForwardMatrix and not; CameraCalibration used and gated off; AnalogBalance;
    // matrices in reverse temperature order; one usable calibration.
    assert!(has(&|i| i.forward_matrix_1.is_some()));
    assert!(has(&|i| i.forward_matrix_1.is_none()));
    let gate = |i: &Inputs| i.camera_calibration_signature == i.profile_calibration_signature;
    assert!(has(&|i| i.camera_calibration_1.is_some() && gate(i)));
    assert!(has(&|i| i.camera_calibration_1.is_some() && !gate(i)));
    assert!(has(&|i| i.analog_balance.is_some()));
    assert!(has(
        &|i| i.illuminant_1 == Some(21) && i.illuminant_2 == Some(17)
    ));
    assert!(has(&|i| i.illuminant_2 == Some(0)));
    assert!(has(&|i| i.illuminant_2 == Some(23)));
}

/// The record for `key`, the unpatched one.
fn record<'a>(records: &'a [Record], key: &str) -> &'a Record {
    records
        .iter()
        .find(|r| r.key == key && r.variant.is_none())
        .unwrap()
}

#[test]
fn negative_controls_fail() {
    let records = records();
    let fails = |r: &Record, p: DngProfile| !within(&worst(&ColorSpec::new(&p).unwrap(), r));

    // The calibration gate flipped, both ways: the Samsung raw (no
    // signatures, so used) and Adobe's Samsung DNG (only a profile
    // signature, so not used).
    let used = record(&records, "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng");
    let mut p = profile(&used.inputs);
    p.camera_calibration_signature = "x".into();
    assert!(fails(used, p), "ignoring a used calibration");
    let gated = record(&records, "reference/20230817_120455/20230817_120455.dng");
    let mut p = profile(&gated.inputs);
    p.camera_calibration_signature = p.profile_calibration_signature.clone();
    assert!(fails(gated, p), "using a gated calibration");

    // The Samsung raw's D65 and A matrices given in the other order.
    let mut p = profile(&used.inputs);
    std::mem::swap(&mut p.illuminant_1, &mut p.illuminant_2);
    assert!(fails(used, p), "the matrices' order");

    // Skipping the SDK's 1e-4 rounding moves an entry by up to 5e-5, five
    // orders above the tolerances. A change of two rounding steps in one
    // entry of Halide's (unrounded) matrix shows through the rounding; the
    // unit tests check that a change inside one step is rounded away.
    let halide = record(&records, "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng");
    let mut p = profile(&halide.inputs);
    p.color_matrix_1[0][0] += 2e-4;
    assert!(fails(halide, p), "two rounding steps in one entry");
}

#[test]
fn stage_split_reproduces_the_chain() {
    for r in &records() {
        let spec = ColorSpec::new(&profile(&r.inputs)).unwrap();
        let xy = spec.as_shot_xy(as_shot(&r.inputs)).unwrap();
        let w = spec.at_white(xy).unwrap();
        let s = spec.camera_to_working(xy).unwrap();
        let min = s.multipliers.iter().copied().fold(f64::INFINITY, f64::min);
        assert!((min - 1.0).abs() < 1e-12, "{}: {:?}", r.key, s.multipliers);
        // Stage 4 after stage 2 is the whole chain on raw camera values.
        let gains = [
            [s.multipliers[0], 0.0, 0.0],
            [0.0, s.multipliers[1], 0.0],
            [0.0, 0.0, s.multipliers[2]],
        ];
        let chain = mat_mul(&s.matrix, &gains);
        let camera = [0.2, 0.5, 0.3];
        let direct = mul(
            &mat_mul(
                &mat_mul(
                    &genzo_color::invert(&genzo_color::WORKING_TO_XYZ).unwrap(),
                    &genzo_color::camera::map_white_matrix(
                        genzo_color::camera::D50_XY,
                        genzo_color::camera::D65_XY,
                    ),
                ),
                &w.camera_to_pcs,
            ),
            camera,
        );
        let split = mul(&chain, camera);
        for c in 0..3 {
            assert!((split[c] - direct[c]).abs() < 1e-12, "{}", r.key);
        }
        // The as-shot white comes out neutral, at 1.
        let out = mul(&chain, w.camera_white);
        for c in out {
            assert!((c - 1.0).abs() < 1e-9, "{}: {out:?}", r.key);
        }
    }
}

#[derive(Deserialize)]
struct Temperature {
    temperature: f64,
    tint: f64,
    xy: [f64; 2],
    back: [f64; 2],
}

#[test]
fn temperature_and_tint_match_the_sdk() {
    let text = include_str!("fixtures/dng-sdk-temperature.jsonl");
    let mut n = 0;
    for line in text.lines() {
        let t: Temperature = serde_json::from_str(line).unwrap();
        let xy = xy_from_temperature(t.temperature, t.tint);
        assert!(
            (xy[0] - t.xy[0]).abs() < 1e-11 && (xy[1] - t.xy[1]).abs() < 1e-11,
            "{} K {}: {xy:?} vs {:?}",
            t.temperature,
            t.tint,
            t.xy
        );
        let (back_t, back_tint) = temperature_from_xy(t.xy);
        assert!(
            (back_t - t.back[0]).abs() < 1e-6,
            "{back_t} vs {}",
            t.back[0]
        );
        assert!(
            (back_tint - t.back[1]).abs() < 1e-7,
            "{back_tint} vs {}",
            t.back[1]
        );
        n += 1;
    }
    assert_eq!(n, 12);
}

/// DEV-01 criterion 10: AsShotNeutral to Temp and Tint and back gives the
/// same camera neutral within 1e-4, on every SDK profile with a neutral
/// whose white Temp can name (2000 to 50,000 K). Only the Leica variant
/// with an unknown second illuminant lands outside: past the end of
/// Robertson's table, where no temperature round-trips.
#[test]
fn as_shot_neutral_round_trips_through_temp_and_tint() {
    let mut n = 0;
    let mut worst = 0.0f64;
    let mut outside = Vec::new();
    for r in &records() {
        let Some(AsShot::Neutral(neutral)) = as_shot(&r.inputs) else {
            continue;
        };
        let spec = ColorSpec::new(&profile(&r.inputs)).unwrap();
        let xy = spec.as_shot_xy(Some(AsShot::Neutral(neutral))).unwrap();
        let (t, tint) = temperature_from_xy(xy);
        if !(2000.0..=50_000.0).contains(&t) {
            outside.push(r.variant.clone().unwrap_or_default());
            continue;
        }
        let back = spec.at_white(xy_from_temperature(t, tint)).unwrap();
        let max = neutral.iter().copied().fold(0.0, f64::max);
        for (b, n) in back.camera_white.iter().zip(neutral) {
            worst = worst.max((b - n / max).abs());
        }
        n += 1;
    }
    println!("{n} neutrals, worst {worst:.2e}");
    assert_eq!(outside, ["illuminant_2_unknown"]);
    assert!(n >= 10, "{n} neutrals");
    assert!(worst <= 1e-4, "{worst}");
}
