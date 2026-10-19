//! The base curve's properties (DEV-02, criteria 3 and 4) beyond the SDK
//! comparison in tone_sdk.rs: neutral stays neutral through the whole step
//! and stage 9, and RGBTone keeps each pixel's channel fraction (its hue in
//! ProPhoto).

use genzo_color::output::{Target, encode, oklch, xyz_to_oklab};
use genzo_color::tone::{
    CurveInputs, CurveTable, ExposureRamp, ToneCurve, base_curve, rgb_tone, working_to_prophoto,
};
use genzo_color::{SRGB_TO_XYZ, invert, mul};

fn standard() -> (ExposureRamp, CurveTable) {
    let inputs = CurveInputs {
        shadow_scale: None,
        no_default_black: false,
        scene_referred: true,
    };
    (
        ExposureRamp::new(1.0, inputs.black(0.0), inputs.black(0.0)),
        CurveTable::new(&ToneCurve::Acr3),
    )
}

#[test]
fn neutral_stays_neutral() {
    let to = working_to_prophoto();
    let from = invert(&to).unwrap();
    let (ramp, table) = standard();
    // Working-space white is D65 (1, 1, 1); the matrix takes it to ProPhoto
    // white within rounding.
    let p = mul(&to, [1.0, 1.0, 1.0]);
    assert!(
        p.iter().all(|c| (c - 1.0).abs() < 1e-12),
        "D65 white in ProPhoto: {p:?}"
    );
    let mut worst: f64 = 0.0;
    for i in 0..=200 {
        let y = 2f64.powf(-12.0 + 13.0 * f64::from(i) / 200.0);
        let out = base_curve([y; 3], &to, &from, &ramp, &table);
        let display = encode(out, Target::Srgb, true);
        let linear = display.map(genzo_color::srgb_decode);
        let lch = oklch(xyz_to_oklab(mul(&SRGB_TO_XYZ, linear)));
        worst = worst.max(lch[1]);
    }
    assert!(worst <= 0.001, "OkLCh chroma {worst}");
}

#[test]
fn rgb_tone_keeps_the_fraction() {
    let (_, table) = standard();
    let mut state = 0x1234_5678u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut worst: f64 = 0.0;
    let mut checked = 0;
    for _ in 0..100_000 {
        let v = [next(), next(), next()];
        let (lo, hi) = (
            v.iter().copied().fold(1.0, f32::min),
            v.iter().copied().fold(0.0, f32::max),
        );
        let o = rgb_tone(v, &table);
        let (olo, ohi) = (
            o.iter().copied().fold(1.0, f32::min),
            o.iter().copied().fold(0.0, f32::max),
        );
        // The fraction is computed in f32, as the SDK does: on spans below
        // 0.01 its rounding alone exceeds 1e-5, so those are left out.
        if hi - lo < 0.01 || ohi - olo < 0.01 {
            continue;
        }
        for c in 0..3 {
            let before = f64::from(v[c] - lo) / f64::from(hi - lo);
            let after = f64::from(o[c] - olo) / f64::from(ohi - olo);
            worst = worst.max((before - after).abs());
        }
        checked += 1;
    }
    assert!(checked > 90_000, "{checked} triples checked");
    assert!(worst <= 1e-5, "fraction moved by {worst:e}");
}

#[test]
fn middle_gray_through_the_whole_step() {
    let to = working_to_prophoto();
    let from = invert(&to).unwrap();
    let (ramp, table) = standard();
    let out = base_curve([0.18; 3], &to, &from, &ramp, &table);
    // DEV-02: the ramp's default black lowers 0.18 a little before the
    // curve, which alone maps 0.18 to 0.388.
    assert!(
        out.iter().all(|c| (c - 0.379).abs() < 0.001),
        "0.18 -> {out:?}"
    );
    let code = (genzo_color::srgb_encode(out[1]) * 255.0).round();
    assert_eq!(code, 166.0);
}
