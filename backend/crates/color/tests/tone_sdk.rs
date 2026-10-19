//! The base curve (color::tone, DEV-02) against the Adobe DNG SDK 1.4:
//! criteria 1 to 4. The fixtures come from scripts/tone-reference.py with the
//! tone probe in scripts/dng-sdk-probe; they hold every input, so this needs
//! no corpus.
//!
//! The port mirrors the SDK's f32 and f64 operations in order, so the
//! curves' tables and RGBTone are expected to match bit for bit; the
//! tolerances are the spec's, and the exact checks run beside them.

use genzo_color::tone::{
    CurveError, CurveInputs, CurveTable, ExposureRamp, Spline, TABLE_SIZE, ToneCurve, acr3,
    choose_curve, profile_curve, rgb_tone, total_baseline_exposure,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    acr3: Acr3,
    ramps: Vec<Ramp>,
    splines: Vec<SplineCase>,
}

#[derive(Deserialize)]
struct Acr3 {
    evaluate: Vec<f64>,
    table: Vec<f32>,
}

#[derive(Deserialize)]
struct Ramp {
    white: f64,
    black: f64,
    evaluate: Vec<f64>,
    toe: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct SplineCase {
    name: String,
    valid: bool,
    points: Vec<f32>,
    evaluate: Option<Vec<f64>>,
    table: Option<Vec<f32>>,
}

#[derive(Deserialize)]
struct Params {
    file: String,
    total_baseline_exposure: f64,
    stage3_gain: f64,
    shadow_scale: f64,
    colorimetric_reference: u32,
    default_black_render: u32,
    profile_curve_points: usize,
    curve: String,
    white: f64,
    black: f64,
    stored: Stored,
}

#[derive(Deserialize)]
struct Stored {
    baseline_exposure: Option<f64>,
    shadow_scale: Option<f64>,
    baseline_exposure_offset: Option<f64>,
}

fn fixture() -> Fixture {
    let text = include_str!("fixtures/dng-sdk-tone.json");
    serde_json::from_str(text).expect("the tone fixture parses")
}

fn grid(n: usize) -> impl Iterator<Item = (usize, f64)> {
    (0..n).map(move |i| (i, i as f64 / (n - 1) as f64))
}

/// Criterion 1: the ACR3 table and curve are Adobe's.
#[test]
fn acr3_matches_the_sdk() {
    let f = fixture();
    assert_eq!(f.acr3.evaluate.len(), 10_001);
    let mut worst: f64 = 0.0;
    for (i, x) in grid(f.acr3.evaluate.len()) {
        let d = (acr3(x) - f.acr3.evaluate[i]).abs();
        worst = worst.max(d);
        // The probe prints f32 results with 9 digits, which round-trip.
        assert_eq!(acr3(x) as f32, f.acr3.evaluate[i] as f32, "acr3({x})");
    }
    assert!(worst <= 1e-6, "worst {worst:e}");

    let table = CurveTable::new(&ToneCurve::Acr3);
    assert_eq!(table.values().len(), TABLE_SIZE + 2);
    assert_eq!(
        &table.values()[..=TABLE_SIZE],
        &f.acr3.table[..],
        "the 4097 table"
    );
}

/// Criterion 2: the exposure ramp is Adobe's, toe included.
#[test]
fn ramp_matches_the_sdk() {
    let f = fixture();
    assert_eq!(f.ramps.len(), 5);
    for r in &f.ramps {
        let ramp = ExposureRamp::new(r.white, r.black, r.black);
        for (i, x) in grid(r.evaluate.len()) {
            let d = (ramp.evaluate(x) - r.evaluate[i]).abs();
            assert!(d <= 1e-9, "black {}: ramp({x}) off by {d:e}", r.black);
        }
        for &[x, y] in &r.toe {
            let d = (ramp.evaluate(x) - y).abs();
            assert!(d <= 1e-9, "black {}: toe ramp({x}) off by {d:e}", r.black);
        }
    }
}

/// Criterion 1: the ProfileToneCurve spline and its table are Adobe's, and
/// invalid curves fall back.
#[test]
fn splines_match_the_sdk() {
    let f = fixture();
    let mut valid_files = 0;
    for case in &f.splines {
        match profile_curve(&case.points) {
            Err(e) => {
                assert!(!case.valid, "{}: refused as {e:?}", case.name);
                assert_eq!(e, CurveError::Invalid, "{}", case.name);
            }
            Ok(spline) => {
                assert!(case.valid, "{}: the SDK calls it invalid", case.name);
                let expected = case.evaluate.as_ref().expect("valid cases carry values");
                for (i, x) in grid(expected.len()) {
                    let d = (spline.evaluate(x) - expected[i]).abs();
                    assert!(d <= 1e-9, "{}: spline({x}) off by {d:e}", case.name);
                }
                let table = CurveTable::new(&ToneCurve::Profile(spline));
                let sdk = case.table.as_ref().expect("valid cases carry a table");
                assert_eq!(
                    &table.values()[..=TABLE_SIZE],
                    &sdk[..],
                    "{} table",
                    case.name
                );
                valid_files += usize::from(case.name.contains('/'));
            }
        }
    }
    assert_eq!(valid_files, 2, "both ProRAW curves are in the fixture");
    let names: Vec<&str> = f.splines.iter().map(|s| s.name.as_str()).collect();
    for n in [
        "two",
        "three",
        "uneven",
        "non_monotonic",
        "x_not_increasing",
        "out_of_range",
    ] {
        assert!(names.contains(&n), "{n} missing");
    }
}

/// Criterion 1: what the decoder hands over for an unreadable tag, and the
/// remaining invalid shapes.
#[test]
fn invalid_curves_fall_back() {
    assert_eq!(profile_curve(&[]), Err(CurveError::Unreadable));
    assert_eq!(profile_curve(&[0.0, 0.0, 1.0]), Err(CurveError::Unreadable));
    assert_eq!(profile_curve(&[0.5, 0.5]), Err(CurveError::Invalid));
    assert_eq!(
        profile_curve(&[0.0, 0.0, 1.0, f32::NAN]),
        Err(CurveError::Invalid)
    );
    let (curve, error) = choose_curve(Some(&[]), true);
    assert_eq!(
        (curve, error),
        (ToneCurve::Acr3, Some(CurveError::Unreadable))
    );
    let (curve, error) = choose_curve(None, false);
    assert_eq!((curve, error), (ToneCurve::Identity, None));
    let (curve, _) = choose_curve(Some(&[0.0, 0.0, 1.0, 1.0]), false);
    assert!(
        matches!(curve, ToneCurve::Profile(_)),
        "a valid curve wins over output-referred"
    );
}

/// The probe's generator (tone_probe.cpp `triple`), for the RGBTone inputs.
struct Generator(u32);

impl Generator {
    fn next_unit(&mut self) -> f32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        (s >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    fn triple(&mut self, i: u32) -> [f32; 3] {
        let mut v = [self.next_unit(), self.next_unit(), self.next_unit()];
        match i % 16 {
            0 => v[1] = v[0],
            2 => v[2] = v[1],
            4 => v = [v[0]; 3],
            6 => v[(i % 3) as usize] = 0.0,
            8 => v[((i / 16) % 3) as usize] = 1.0,
            10 => v = [0.0; 3],
            12 => v = [1.0; 3],
            14 => v[2] = v[0],
            _ => {}
        }
        v
    }
}

/// Criterion 3: RGBTone is Adobe's, ties, zeros and ones included.
#[test]
fn rgb_tone_matches_the_sdk() {
    let bytes = include_bytes!("fixtures/dng-sdk-rgbtone.f32");
    let sdk: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(sdk.len(), 300_000);
    let table = CurveTable::new(&ToneCurve::Acr3);
    let mut generator = Generator(0x9E37_79B9);
    let mut exact = 0;
    for i in 0..100_000u32 {
        let out = rgb_tone(generator.triple(i), &table);
        let expected = &sdk[3 * i as usize..3 * i as usize + 3];
        for c in 0..3 {
            let d = (out[c] - expected[c]).abs();
            assert!(
                d <= 1e-6,
                "triple {i} channel {c}: {} vs {}",
                out[c],
                expected[c]
            );
        }
        exact += usize::from(out == [expected[0], expected[1], expected[2]]);
    }
    assert_eq!(exact, 100_000, "bit-exact triples");
}

fn params() -> Vec<Params> {
    include_str!("fixtures/dng-sdk-tone-params.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).expect("a params record parses"))
        .collect()
}

/// DEV-02 step 2 and the SDK's per-file parameters: the rounded total
/// BaselineExposure, the ShadowScale rule, the black in Genzo's units and
/// the curve choice, from the tags as stored.
#[test]
fn per_file_parameters_match_the_sdk() {
    let records = params();
    assert_eq!(records.len(), 8);
    for p in &records {
        assert_eq!(
            p.stage3_gain, 1.0,
            "{}: no corpus file merges CFA channels",
            p.file
        );
        let total = total_baseline_exposure(
            p.stored.baseline_exposure.unwrap_or(0.0),
            p.stored.baseline_exposure_offset.unwrap_or(0.0),
        );
        assert_eq!(
            total, p.total_baseline_exposure,
            "{}: total BaselineExposure",
            p.file
        );
        let inputs = CurveInputs {
            shadow_scale: p.stored.shadow_scale,
            no_default_black: p.default_black_render == 1,
            scene_referred: p.colorimetric_reference == 0,
        };
        assert_eq!(inputs.shadow_scale(), p.shadow_scale, "{}", p.file);
        // The SDK's white is 2^−max(0, E); Genzo's is 1 after the gain, and
        // its black is the SDK's in the same units.
        assert!(
            (p.white - 2f64.powf(-total.max(0.0))).abs() < 1e-12,
            "{}",
            p.file
        );
        let black = inputs.black(total);
        let expected = p.black * 2f64.powf(total);
        assert!(
            (black - expected).abs() < 1e-12,
            "{}: black {black} vs {expected}",
            p.file
        );
        let points = (p.profile_curve_points > 0).then(|| vec![0.0, 0.0, 1.0, 1.0]);
        let (curve, _) = choose_curve(points.as_deref(), inputs.scene_referred);
        let name = match curve {
            ToneCurve::Acr3 => "acr3",
            ToneCurve::Profile(_) => "profile",
            ToneCurve::Identity => "identity",
        };
        assert_eq!(name, p.curve, "{}", p.file);
    }
    // The rounding is real: the Halide DNG stores 0.0275 EV, the SDK uses 0.03.
    assert!(
        records
            .iter()
            .any(|p| p.stored.baseline_exposure != Some(p.total_baseline_exposure))
    );
}

/// Criterion 4: the ACR3 curve's properties.
#[test]
fn acr3_properties() {
    let table = CurveTable::new(&ToneCurve::Acr3);
    let v = table.values();
    assert!(v.windows(2).all(|w| w[1] >= w[0]), "monotonic");
    assert_eq!((v[0], v[TABLE_SIZE]), (0.0, 1.0));
    let gray = table.interpolate(0.18);
    assert!((gray - 0.388).abs() <= 0.001, "0.18 maps to {gray}");
    let _ = Spline::solve(&[[0.0, 0.0], [1.0, 1.0]]);
}
