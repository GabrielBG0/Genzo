//! DEV-01 criterion 10, the white balance math, and the non-RAW Temp and
//! Tint against colour-science (tests/fixtures/white-balance-colour.jsonl,
//! written by scripts/white-balance-reference.py). The AsShotNeutral round
//! trip is in tests/dng_sdk.rs, beside the SDK's profiles.

use genzo_color::camera::{temperature_from_xy, xy_from_temperature};
use genzo_color::white_balance::{
    Preset, cat16, relative_adaptation, relative_from_xy, relative_white,
};
use serde::Deserialize;

/// RAW Temp positions linear in mireds over 2000 to 50,000 K (DEV-02), and
/// Tint over −150 to +150.
fn raw_grid() -> impl Iterator<Item = (f64, f64)> {
    (0..=96).flat_map(|i| {
        let p = f64::from(i) / 96.0;
        let t = 1.0 / (1.0 / 2000.0 - p * (1.0 / 2000.0 - 1.0 / 50_000.0));
        (-6..=6).map(move |j| (t, f64::from(j) * 25.0))
    })
}

#[test]
fn raw_temp_and_tint_round_trip_within_1_k_and_a_tenth() {
    let (mut worst_t, mut worst_tint) = (0.0f64, 0.0f64);
    for (t, tint) in raw_grid() {
        let (t2, tint2) = temperature_from_xy(xy_from_temperature(t, tint));
        worst_t = worst_t.max((t2 - t).abs());
        worst_tint = worst_tint.max((tint2 - tint).abs());
    }
    println!("worst round trip: {worst_t:.4} K, tint {worst_tint:.5}");
    assert!(worst_t <= 1.0, "{worst_t} K");
    assert!(worst_tint <= 0.1, "tint {worst_tint}");
}

#[test]
fn d65_and_a_land_on_their_temperatures() {
    let (t, tint) = temperature_from_xy(Preset::Cloudy.xy());
    println!("D65: {t:.3} K, tint {tint:.4}");
    assert!((t - 6504.0).abs() <= 2.0, "D65 at {t} K");
    let (t, tint) = temperature_from_xy(Preset::Tungsten.xy());
    println!("A: {t:.3} K, tint {tint:.4}");
    assert!((t - 2856.0).abs() <= 2.0, "A at {t} K");
    assert!(tint.abs() <= 0.5, "A at tint {tint}");
}

#[test]
fn presets_have_the_values_dev02_names() {
    let shown = |p: Preset| {
        let (t, n) = p.temp_tint();
        (t.round(), n.round())
    };
    for p in Preset::ALL {
        println!("{}: {:?}", p.name(), shown(p));
    }
    // Robertson places D65 at 6503.01 K, inside criterion 10's 6504 ± 2.
    assert_eq!(shown(Preset::Cloudy), (6503.0, 10.0));
    assert_eq!(shown(Preset::Tungsten), (2856.0, 0.0));
    assert_eq!(shown(Preset::Flash), (5500.0, 0.0));
}

#[test]
fn relative_temp_spans_what_dev01_names() {
    let (cool, _) = temperature_from_xy(relative_white(-100.0, 0.0));
    let (warm, _) = temperature_from_xy(relative_white(100.0, 0.0));
    println!("−100: {cool:.1} K, +100: {warm:.1} K");
    assert!((cool - 3940.0).abs() < 5.0, "{cool}");
    assert!((warm - 18_600.0).abs() < 50.0, "{warm}");
    let w = relative_white(0.0, 0.0);
    let d65 = genzo_color::camera::D65_XY;
    assert!(
        (w[0] - d65[0]).abs() < 1e-6 && (w[1] - d65[1]).abs() < 1e-6,
        "{w:?}"
    );
}

/// The reference the non-RAW shifts start from, as DEV-01 quotes it:
/// D65_XY's Robertson mired and tint.
#[test]
fn relative_shifts_start_from_d65_as_dev01_quotes_it() {
    let (t, tint) = temperature_from_xy(genzo_color::camera::D65_XY);
    let mired = 1.0e6 / t;
    println!("D65_XY: {t:.3} K, {mired:.4} mired, tint {tint:.4}");
    assert_eq!((mired * 100.0).round() / 100.0, 153.76);
    assert_eq!((tint * 10.0).round() / 10.0, 9.8);
}

#[test]
fn relative_temp_and_tint_round_trip() {
    for i in -20..=20 {
        for j in -20..=20 {
            let (temp, tint) = (f64::from(i) * 5.0, f64::from(j) * 5.0);
            let (t2, n2) = relative_from_xy(relative_white(temp, tint));
            assert!((t2 - temp).abs() < 0.01, "{temp} {tint}: {t2}");
            assert!((n2 - tint).abs() < 0.01, "{temp} {tint}: {n2}");
        }
    }
}

/// The shifted white moves smoothly: no step between neighboring slider
/// values bigger than three times the average step, in uv.
#[test]
fn relative_white_has_no_jump() {
    let uv = |[x, y]: [f64; 2]| {
        let d = -2.0 * x + 12.0 * y + 3.0;
        [4.0 * x / d, 6.0 * y / d]
    };
    let steps: Vec<f64> = (-1000..1000)
        .map(|i| {
            let a = uv(relative_white(f64::from(i) / 10.0, 0.0));
            let b = uv(relative_white(f64::from(i + 1) / 10.0, 0.0));
            (a[0] - b[0]).hypot(a[1] - b[1])
        })
        .collect();
    let mean = steps.iter().sum::<f64>() / steps.len() as f64;
    let max = steps.iter().copied().fold(0.0, f64::max);
    assert!(max < 3.0 * mean, "max step {max}, mean {mean}");
}

/// A white adapted by its own shift comes out as the white 0 and 0 name.
#[test]
fn relative_adaptation_takes_the_shifted_white_to_white() {
    let to_xyz = genzo_color::WORKING_TO_XYZ;
    let from_xyz = genzo_color::invert(&to_xyz).unwrap();
    for (temp, tint) in [(-100.0, 0.0), (60.0, -40.0), (100.0, 100.0), (-30.0, 80.0)] {
        let xyz = genzo_color::camera::xy_to_xyz(relative_white(temp, tint));
        let rgb = genzo_color::mul(&from_xyz, xyz);
        let out = genzo_color::mul(&relative_adaptation(temp, tint), rgb);
        let white = genzo_color::mul(
            &from_xyz,
            genzo_color::camera::xy_to_xyz(relative_white(0.0, 0.0)),
        );
        for c in 0..3 {
            assert!((out[c] - white[c]).abs() < 1e-12, "{temp} {tint}: {out:?}");
        }
    }
}

#[derive(Deserialize)]
struct Reference {
    from: [f64; 2],
    to: [f64; 2],
    cat16: [[f64; 3]; 3],
}

#[test]
fn cat16_matches_colour_science() {
    let text = include_str!("fixtures/white-balance-colour.jsonl");
    let mut n = 0;
    for line in text.lines() {
        let r: Reference = serde_json::from_str(line).unwrap();
        let m = cat16(r.from, r.to);
        for (a, b) in m.iter().flatten().zip(r.cat16.iter().flatten()) {
            assert!((a - b).abs() < 1e-9, "{:?} → {:?}: {m:?}", r.from, r.to);
        }
        n += 1;
    }
    assert!(n >= 6, "{n} records");
}
