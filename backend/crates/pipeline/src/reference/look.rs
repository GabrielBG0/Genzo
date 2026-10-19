//! Stages 6 and 7 for one working-space pixel (DEV-01): the definition the
//! GPU's view passes are tested against.

use genzo_color::look::{look, luminance};
use genzo_color::tone::{MIDDLE_GRAY, base_curve, contrast, whites_blacks, working_to_prophoto};
use genzo_color::{Mat3, invert};

use crate::look::{BaseCurve, Look, Profile};

/// Below this luminance a pixel has no meaningful `t`, so only exposure's
/// gain applies (noise around black, negative values).
pub const MIN_LUMINANCE: f64 = 1e-10;

/// The ProPhoto matrices of the base curve.
pub fn prophoto() -> (Mat3, Mat3) {
    let to = working_to_prophoto();
    (to, invert(&to).expect("invertible"))
}

/// Stage 6 on a working-space pixel: exposure, Whites and Blacks, Contrast
/// on luminance (RGB scaled by the luminance ratio), then the profile's base
/// curve. `base` is needed for Genzo Standard.
pub fn stage6(rgb: [f64; 3], l: &Look, base: Option<&BaseCurve>) -> [f64; 3] {
    let gain = 2f64.powf(f64::from(l.exposure));
    let y = luminance(rgb);
    let ratio = if y > MIN_LUMINANCE {
        let t = (y / MIDDLE_GRAY).log2() + f64::from(l.exposure);
        let t = whites_blacks(t, f64::from(l.whites) / 100.0, f64::from(l.blacks) / 100.0);
        let t = contrast(t, f64::from(l.contrast) / 100.0);
        MIDDLE_GRAY * t.exp2() / y
    } else {
        gain
    };
    let rgb = rgb.map(|c| c * ratio);
    match (l.profile, base) {
        (Profile::GenzoStandard, Some(b)) => {
            let (to, from) = prophoto();
            base_curve(rgb, &to, &from, &b.ramp(l.exposure), &b.table)
        }
        (Profile::GenzoStandard, None) => panic!("Genzo Standard needs its base curve"),
        _ => rgb,
    }
}

/// Stage 7 on a pixel stage 6 produced.
pub fn stage7(rgb: [f64; 3], l: &Look) -> [f64; 3] {
    look(
        rgb,
        f64::from(l.vibrance) / 100.0,
        f64::from(l.saturation) / 100.0,
        l.monochrome,
    )
}
