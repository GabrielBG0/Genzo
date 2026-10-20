//! Stage 7 (DEV-01, M1 part): Vibrance, Saturation and the B&W treatment,
//! in Oklab (chroma is the length of a and b, so scaling it keeps lightness
//! and hue). The CPU reference for the GPU's stage 7.

use crate::output::{OKLAB_M1, OKLAB_M2, oklch, xyz_to_oklab};
use crate::{Mat3, SRGB_TO_XYZ, WORKING_TO_XYZ, invert, mat_mul, mul};

/// AP1 luminance: the Y row of the working space.
pub fn luminance(rgb: [f64; 3]) -> f64 {
    let y = WORKING_TO_XYZ[1];
    y[0] * rgb[0] + y[1] * rgb[1] + y[2] * rgb[2]
}

/// `C_ref`: the Oklab chroma of the most saturated sRGB primary (blue).
pub fn reference_chroma() -> f64 {
    let column = |c: usize| [SRGB_TO_XYZ[0][c], SRGB_TO_XYZ[1][c], SRGB_TO_XYZ[2][c]];
    (0..3)
        .map(|c| oklch(xyz_to_oklab(column(c)))[1])
        .fold(0.0, f64::max)
}

pub(crate) fn smoothstep(lo: f64, hi: f64, x: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much a hue (Oklab, degrees) counts as skin: 1 from 25° to 65°, 0
/// below 15° and above 75°, a smoothstep between (owner, 2026-10-19).
pub fn skin(hue: f64) -> f64 {
    smoothstep(15.0, 25.0, hue) * (1.0 - smoothstep(65.0, 75.0, hue))
}

/// Vibrance's chroma gain for chroma `c` and hue `h`, with `v` the slider
/// over 100. Positive values raise low chroma most, and skin half as much;
/// negative values desaturate evenly, like Saturation.
pub fn vibrance_gain(c: f64, h: f64, v: f64, c_ref: f64) -> f64 {
    if v < 0.0 {
        return 1.0 + v;
    }
    let weight = (1.0 - c / c_ref).max(0.0) * (1.0 - 0.5 * skin(h));
    1.0 + v * weight
}

/// The matrices between the working space and Oklab's cone responses.
#[derive(Debug, Clone, Copy)]
pub struct OklabMatrices {
    /// Working space to cone responses (M1 · AP1 to XYZ).
    pub to_lms: Mat3,
    /// Cone responses back to the working space.
    pub from_lms: Mat3,
}

impl OklabMatrices {
    pub fn working() -> Self {
        let to_lms = mat_mul(&OKLAB_M1, &WORKING_TO_XYZ);
        Self {
            to_lms,
            from_lms: invert(&to_lms).expect("invertible"),
        }
    }
}

/// Stage 7 for one working-space pixel. `vibrance` and `saturation` are the
/// sliders over 100; `monochrome` is the B&W treatment, whose output is the
/// luminance in all three channels (Vibrance and Saturation do nothing).
/// Vibrance applies before Saturation.
pub fn look(rgb: [f64; 3], vibrance: f64, saturation: f64, monochrome: bool) -> [f64; 3] {
    if monochrome {
        return [luminance(rgb); 3];
    }
    if vibrance == 0.0 && saturation == 0.0 {
        return rgb;
    }
    let m = OklabMatrices::working();
    let lms = mul(&m.to_lms, rgb).map(f64::cbrt);
    let lab = mul(&OKLAB_M2, lms);
    let [_, c, h] = oklch(lab);
    let gain = vibrance_gain(c, h, vibrance, reference_chroma()) * (1.0 + saturation);
    let lab = [lab[0], lab[1] * gain, lab[2] * gain];
    let lms = mul(&invert(&OKLAB_M2).expect("invertible"), lab).map(|v| v * v * v);
    mul(&m.from_lms, lms)
}
