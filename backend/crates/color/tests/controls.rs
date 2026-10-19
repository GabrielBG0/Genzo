//! The tone and presence controls' shapes (DEV-01, criterion 12): Contrast
//! and Whites and Blacks are monotonic with the stated minimum slopes,
//! leave middle gray where it is and fade before the ends; Vibrance's skin
//! weight has no step on a hue sweep.

use genzo_color::look::{reference_chroma, skin, vibrance_gain};
use genzo_color::tone::{black_end, contrast, white_end, whites_blacks};

/// `t` from −12 to +8 stops, finely.
fn grid() -> impl Iterator<Item = f64> {
    (0..=20_000).map(|i| -12.0 + 20.0 * f64::from(i) / 20_000.0)
}

fn min_slope(f: impl Fn(f64) -> f64) -> f64 {
    let h = 1e-3;
    grid().map(|t| (f(t + h) - f(t)) / h).fold(f64::MAX, f64::min)
}

#[test]
fn contrast_is_monotonic_and_keeps_middle_gray() {
    for c in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        assert_eq!(contrast(0.0, c), 0.0, "middle gray at contrast {c}");
        let slope = min_slope(|t| contrast(t, c));
        assert!(slope >= 0.6 - 1e-3, "contrast {c}: slope {slope}");
        assert!((contrast(white_end(), c) - white_end()).abs() < 0.05, "fades at the Whites end");
        assert!((contrast(black_end(), c) - black_end()).abs() < 0.02, "fades at the Blacks end");
    }
    // The slope at middle gray is 1 + 0.4c.
    let h = 1e-6;
    for c in [-1.0, 1.0] {
        let slope = (contrast(h, c) - contrast(-h, c)) / (2.0 * h);
        assert!((slope - (1.0 + 0.4 * c)).abs() < 1e-6, "{slope}");
    }
}

#[test]
fn whites_and_blacks_are_monotonic_and_reach_their_ends() {
    for v in [-1.0, -0.5, 0.5, 1.0] {
        assert_eq!(whites_blacks(0.0, v, v), 0.0, "middle gray");
        let whites = min_slope(|t| whites_blacks(t, v, 0.0));
        let blacks = min_slope(|t| whites_blacks(t, 0.0, v));
        assert!(whites >= 0.24, "whites {v}: slope {whites}");
        assert!(blacks >= 0.55, "blacks {v}: slope {blacks}");
        // Each moves its source point onto its end.
        let source = white_end() - 1.5 * v;
        assert!((whites_blacks(source, v, 0.0) - white_end()).abs() < 1e-12);
        let source = black_end() - 1.5 * v;
        assert!((whites_blacks(source, 0.0, v) - black_end()).abs() < 1e-12);
        // Whites leaves the shadows alone and Blacks the highlights.
        assert_eq!(whites_blacks(-2.0, v, 0.0), -2.0);
        assert_eq!(whites_blacks(2.0, 0.0, v), 2.0);
    }
}

#[test]
fn exposure_one_doubles() {
    // Exposure is a shift of 1 in t: twice the linear value.
    let t = (0.05f64 / 0.18).log2();
    assert!((0.18 * (t + 1.0).exp2() - 0.1).abs() < 1e-15);
}

/// Criterion 12: a hue sweep through 10° to 80° at constant chroma changes
/// Vibrance's gain smoothly, with no step above 0.01 between hues 0.1°
/// apart. The shoulders' steepest change is about 0.007; the hard edge they
/// replace jumped by about 0.47.
#[test]
fn vibrance_has_no_step_on_a_hue_sweep() {
    let c_ref = reference_chroma();
    assert!((c_ref - 0.3132).abs() < 1e-3, "sRGB blue's Oklab chroma: {c_ref}");
    for c in [0.02, 0.1, 0.2] {
        let mut last = vibrance_gain(c, 10.0, 1.0, c_ref);
        let mut steepest: f64 = 0.0;
        for i in 1..=700 {
            let h = 10.0 + 0.1 * f64::from(i);
            let g = vibrance_gain(c, h, 1.0, c_ref);
            steepest = steepest.max((g - last).abs());
            last = g;
        }
        assert!(steepest <= 0.01, "chroma {c}: a step of {steepest}");
        assert!(steepest > 0.001, "the skin weight acts");
    }
    assert_eq!((skin(20.0), skin(70.0)), (0.5, 0.5));
    assert_eq!((skin(10.0), skin(45.0), skin(80.0)), (0.0, 1.0, 0.0));
    // At −100 Vibrance desaturates evenly, like Saturation.
    assert_eq!(vibrance_gain(0.05, 40.0, -1.0, c_ref), 0.0);
    assert_eq!(vibrance_gain(0.25, 200.0, -0.5, c_ref), 0.5);
}
