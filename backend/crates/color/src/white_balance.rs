//! What Temp and Tint mean (DEV-01, *White balance*; the presets of DEV-02).
//!
//! **RAW:** Temp and Tint name the scene's white point, through the DNG
//! SDK's Robertson temperature and its tint scale, `Duv = −tint / 3000`
//! ([`temperature_from_xy`], [`xy_from_temperature`]). Stage 4 turns that
//! white into multipliers and a matrix with [`ColorSpec::camera_to_working`].
//!
//! **Non-RAW:** the pixels are already balanced, so Temp and Tint are shifts
//! from the encoded white, D65 (owner, 2026-10-20). The shifted white is the
//! one Robertson's method gives for a reciprocal temperature of
//! `D65's mired − temp` and a tint of `D65's tint + tint`: the same code as
//! RAW, so it moves along the Planckian locus at D65's distance from it,
//! with no jump anywhere on the slider. The image is adapted from that white
//! to D65 with CAT16.
//!
//! [`ColorSpec::camera_to_working`]: crate::camera::ColorSpec::camera_to_working

use std::sync::LazyLock;

use crate::camera::{D65_XY, temperature_from_xy, xy_from_temperature, xy_to_xyz};
use crate::{Mat3, WORKING_TO_XYZ, invert, mat_mul, mul};

/// A white balance preset for RAW files (DEV-02). Non-RAW files have only
/// As Shot and Custom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
}

impl Preset {
    /// The presets in the dropdown's order, after As Shot.
    pub const ALL: [Preset; 6] = [
        Preset::Daylight,
        Preset::Cloudy,
        Preset::Shade,
        Preset::Tungsten,
        Preset::Fluorescent,
        Preset::Flash,
    ];

    /// The name the dropdown shows.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Daylight => "Daylight",
            Preset::Cloudy => "Cloudy",
            Preset::Shade => "Shade",
            Preset::Tungsten => "Tungsten",
            Preset::Fluorescent => "Fluorescent",
            Preset::Flash => "Flash",
        }
    }

    /// The preset's white point: the CIE 2° observer xy of its illuminant.
    pub fn xy(self) -> [f64; 2] {
        match self {
            Preset::Daylight => [0.33242, 0.34743],    // D55
            Preset::Cloudy => [0.31271, 0.32902],      // D65
            Preset::Shade => [0.29902, 0.31485],       // D75
            Preset::Tungsten => [0.44757, 0.40745],    // A
            Preset::Fluorescent => [0.37208, 0.37529], // F2, cool white
            Preset::Flash => xy_from_temperature(5500.0, 0.0),
        }
    }

    /// The Temp (kelvin) and Tint the preset sets.
    pub fn temp_tint(self) -> (f64, f64) {
        temperature_from_xy(self.xy())
    }

    /// The preset the dropdown names for a Custom white balance: the one
    /// whose Temp and Tint equal `temp` and `tint` as the sliders show them
    /// (whole kelvin, whole tint). None shows "Custom".
    pub fn matching(temp: f64, tint: f64) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| {
            let (t, n) = p.temp_tint();
            t.round() == temp.round() && n.round() == tint.round()
        })
    }
}

/// D65 as Robertson's method places it: its reciprocal temperature
/// (mired) and its tint. The non-RAW shifts are measured from these.
static D65_ROBERTSON: LazyLock<(f64, f64)> = LazyLock::new(|| {
    let (t, tint) = temperature_from_xy(D65_XY);
    (1.0e6 / t, tint)
});

/// The white a non-RAW photo's Temp and Tint (−100 to +100 each) name.
/// 0 and 0 give D65, −100 about 3940 K (cooler), +100 about 18,600 K.
pub fn relative_white(temp: f64, tint: f64) -> [f64; 2] {
    let (mired, d65_tint) = *D65_ROBERTSON;
    xy_from_temperature(1.0e6 / (mired - temp), d65_tint + tint)
}

/// The non-RAW Temp and Tint of a white point: the inverse of
/// [`relative_white`], which the picker uses.
pub fn relative_from_xy(xy: [f64; 2]) -> (f64, f64) {
    let (mired, d65_tint) = *D65_ROBERTSON;
    let (t, tint) = temperature_from_xy(xy);
    (mired - 1.0e6 / t, tint - d65_tint)
}

/// CAT16's matrix from XYZ to its cone space (Li et al. 2017).
const M16: Mat3 = [
    [0.401_288, 0.650_173, -0.051_461],
    [-0.250_268, 1.204_414, 0.045_854],
    [-0.002_079, 0.048_952, 0.953_127],
];

/// The full CAT16 adaptation, in XYZ, from white `from` to white `to`.
pub fn cat16(from: [f64; 2], to: [f64; 2]) -> Mat3 {
    let (a, b) = (mul(&M16, xy_to_xyz(from)), mul(&M16, xy_to_xyz(to)));
    let gains = [
        [b[0] / a[0], 0.0, 0.0],
        [0.0, b[1] / a[1], 0.0],
        [0.0, 0.0, b[2] / a[2]],
    ];
    let inverse = invert(&M16).expect("CAT16 is invertible");
    mat_mul(&mat_mul(&inverse, &gains), &M16)
}

/// A non-RAW photo's white balance as one matrix on working space values:
/// CAT16 from [`relative_white`] to the white 0 and 0 name, which is D65 to
/// about 1e-7 and makes 0 and 0 an exact identity.
pub fn relative_adaptation(temp: f64, tint: f64) -> Mat3 {
    let to_xyz = WORKING_TO_XYZ;
    let from_xyz = invert(&to_xyz).expect("the working space is invertible");
    let cat = cat16(relative_white(temp, tint), relative_white(0.0, 0.0));
    mat_mul(&mat_mul(&from_xyz, &cat), &to_xyz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_an_identity() {
        let m = relative_adaptation(0.0, 0.0);
        for (i, row) in m.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 1e-12, "{m:?}");
            }
        }
    }

    #[test]
    fn presets_match_their_own_values() {
        for p in Preset::ALL {
            let (t, n) = p.temp_tint();
            assert_eq!(Preset::matching(t, n), Some(p), "{}", p.name());
        }
        assert_eq!(Preset::matching(5432.0, 3.0), None);
    }
}
