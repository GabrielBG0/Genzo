//! The decoder's camera color data as the color crate's DNG profile
//! (stages 2 and 4). The sizes are checked as the DNG SDK checks them.

use genzo_color::Mat3;
use genzo_color::camera::{AsShot, ColorSpec, D55_XY, DngProfile};
use genzo_color::encoding::InputEncoding;
use genzo_imgio::raw::{self, CameraColor};

fn mat3(v: &[f64]) -> Option<Mat3> {
    match *v {
        [a, b, c, d, e, f, g, h, i] => Some([[a, b, c], [d, e, f], [g, h, i]]),
        _ => None,
    }
}

/// The profile for a three-color camera. None without ColorMatrix1, for a
/// sensor of other than three colors, or for a matrix of the wrong size
/// (the SDK refuses those profiles). A CameraCalibration or AnalogBalance
/// of the wrong size is ignored, as the SDK ignores it.
pub fn dng_profile(c: &CameraColor) -> Option<DngProfile> {
    let cm1 = c.color_matrix_1.as_ref()?;
    let optional = |m: &Option<Vec<f64>>| match m {
        Some(v) => mat3(v).map(Some),
        None => Some(None),
    };
    let color_matrix_2 = optional(&c.color_matrix_2.as_ref().map(|m| m.matrix.clone()))?;
    Some(DngProfile {
        color_matrix_1: mat3(&cm1.matrix)?,
        illuminant_1: cm1.illuminant.unwrap_or(0),
        color_matrix_2,
        illuminant_2: c
            .color_matrix_2
            .as_ref()
            .and_then(|m| m.illuminant)
            .unwrap_or(0),
        forward_matrix_1: optional(&c.forward_matrix_1)?,
        forward_matrix_2: optional(&c.forward_matrix_2)?,
        camera_calibration_1: c.camera_calibration_1.as_deref().and_then(mat3),
        camera_calibration_2: c.camera_calibration_2.as_deref().and_then(mat3),
        analog_balance: match c.analog_balance.as_deref() {
            Some(&[r, g, b]) => [r, g, b],
            _ => [1.0; 3],
        },
        camera_calibration_signature: c.camera_calibration_signature.clone(),
        profile_calibration_signature: c.profile_calibration_signature.clone(),
    })
}

/// The recorded white balance.
pub fn as_shot(c: &CameraColor) -> Option<AsShot> {
    c.as_shot.map(|a| match a {
        raw::AsShot::Neutral(n) => AsShot::Neutral(n),
        raw::AsShot::WhiteXy(xy) => AsShot::WhiteXy(xy),
    })
}

/// Why a photo's color is not its camera's own (the status bar says so).
#[derive(Debug, Clone, PartialEq)]
pub enum ColorFallback {
    /// No usable color profile (a monochrome or four-color sensor, or a
    /// profile the SDK refuses): camera values pass through unchanged.
    NoProfile(String),
    /// The white point could not be solved from the as-shot data (a
    /// singular matrix): D55 is used, as the SDK does with no as-shot data.
    WhiteUnsolved,
}

/// Stages 2 and 4 for one white point, from one `camera_to_working` call so
/// the two cannot drift apart.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraTransform {
    /// The white point, CIE xy.
    pub white: [f64; 2],
    /// Stage 2: per channel gains; the smallest is 1, the clip level.
    pub multipliers: [f32; 3],
    /// Stage 4: white balanced camera RGB to the working space (AP1, D65).
    pub matrix: [[f32; 3]; 3],
    /// Stage 4: 2^BaselineExposure, the SDK's total: rounded to 1/100 stop,
    /// plus the profile's offset (`color::tone::total_baseline_exposure`).
    pub exposure: f32,
    pub fallback: Option<ColorFallback>,
    /// Stage 4: whether highlights desaturate toward the clip level. A
    /// camera's do; a non-RAW image's values are final, so theirs do not.
    pub highlight_desaturation: bool,
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The transform for `white`, or for the as-shot white when None (D55 when
/// the file has none, as the SDK renders it).
pub fn camera_transform(c: &CameraColor, white: Option<[f64; 2]>) -> CameraTransform {
    let exposure = 2f64.powf(genzo_color::tone::total_baseline_exposure(
        c.baseline_exposure,
        c.baseline_exposure_offset,
    )) as f32;
    let passthrough = |why: String| {
        // Without a matrix, the as-shot neutral still balances the channels.
        let multipliers = match c.as_shot {
            Some(raw::AsShot::Neutral(n)) if n.iter().all(|v| *v > 0.0) => {
                let max = n[0].max(n[1]).max(n[2]);
                n.map(|v| (max / v) as f32)
            }
            _ => [1.0; 3],
        };
        CameraTransform {
            white: D55_XY,
            multipliers,
            matrix: IDENTITY,
            exposure,
            fallback: Some(ColorFallback::NoProfile(why)),
            highlight_desaturation: true,
        }
    };
    let Some(profile) = dng_profile(c) else {
        return passthrough("no three-color ColorMatrix".into());
    };
    let spec = match ColorSpec::new(&profile) {
        Ok(spec) => spec,
        Err(e) => return passthrough(e.to_string()),
    };
    let (mut xy, mut fallback) = match white.or_else(|| spec.as_shot_xy(as_shot(c))) {
        Some(xy) => (xy, None),
        None => (D55_XY, Some(ColorFallback::WhiteUnsolved)),
    };
    let solved = spec.camera_to_working(xy).or_else(|| {
        xy = D55_XY;
        fallback = Some(ColorFallback::WhiteUnsolved);
        spec.camera_to_working(xy)
    });
    let Some(t) = solved else {
        return passthrough("a singular color matrix".into());
    };
    CameraTransform {
        white: xy,
        multipliers: t.multipliers.map(|v| v as f32),
        matrix: t.matrix.map(|row| row.map(|v| v as f32)),
        exposure,
        fallback,
        highlight_desaturation: true,
    }
}

/// Stage 4 for a non-RAW image: its encoding's matrix into the working
/// space, no white balance, no exposure, no highlight desaturation.
pub fn encoded_transform(e: &InputEncoding) -> CameraTransform {
    CameraTransform {
        white: genzo_color::camera::D65_XY,
        multipliers: [1.0; 3],
        matrix: e.to_working.map(|row| row.map(|v| v as f32)),
        exposure: 1.0,
        fallback: None,
        highlight_desaturation: false,
    }
}

/// [`encoded_transform`] with the non-RAW white balance `temp` and `tint`
/// (DEV-01, *White balance*): the CAT16 shift composed after the encoding's
/// matrix, so a settled render bakes it into stage 4.
pub fn balanced_encoded_transform(e: &InputEncoding, temp: f64, tint: f64) -> CameraTransform {
    let shift = genzo_color::white_balance::relative_adaptation(temp, tint);
    CameraTransform {
        matrix: genzo_color::mat_mul(&shift, &e.to_working).map(|row| row.map(|v| v as f32)),
        ..encoded_transform(e)
    }
}

/// The white balance drag (DEV-01, stage 2): what turns values stage 4
/// produced with `old` into those it would produce with `new`, as one
/// working space matrix, `M_new · diag(r) · M_old⁻¹` with `r` the new
/// multipliers over the old. Stage 2's clipping and stage 4's highlight
/// desaturation are not linear, so clipped highlights differ from a
/// settled render; everything else matches it. The exposure is the file's,
/// the same on both sides. None when `old`'s matrix is singular.
pub fn drag_ratio(old: &CameraTransform, new: &CameraTransform) -> Option<[[f32; 3]; 3]> {
    let f64s = |m: &[[f32; 3]; 3]| m.map(|row| row.map(f64::from));
    let r = [0, 1, 2].map(|c| f64::from(new.multipliers[c]) / f64::from(old.multipliers[c]));
    let ratio = [[r[0], 0.0, 0.0], [0.0, r[1], 0.0], [0.0, 0.0, r[2]]];
    let undo = genzo_color::invert(&f64s(&old.matrix))?;
    let m = genzo_color::mat_mul(&genzo_color::mat_mul(&f64s(&new.matrix), &ratio), &undo);
    Some(m.map(|row| row.map(|v| v as f32)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use genzo_imgio::raw::ColorMatrix;

    fn color(matrix: Vec<f64>, as_shot: Option<raw::AsShot>) -> CameraColor {
        CameraColor {
            color_matrix_1: Some(ColorMatrix {
                illuminant: Some(21),
                matrix,
            }),
            as_shot,
            baseline_exposure: 0.5,
            ..CameraColor::default()
        }
    }

    // Maps the PCS white to about (1, 1, 1): a plausible XYZ to camera matrix.
    const CM: [f64; 9] = [1.0371, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.212];

    #[test]
    fn neutral_maps_to_white_through_both_stages() {
        let c = color(CM.to_vec(), Some(raw::AsShot::Neutral([0.6, 1.0, 0.8])));
        let t = camera_transform(&c, None);
        assert_eq!(t.fallback, None);
        assert!((t.exposure - 2f32.sqrt()).abs() < 1e-6);
        let min = t.multipliers.iter().copied().fold(f32::INFINITY, f32::min);
        assert!((min - 1.0).abs() < 1e-6, "{:?}", t.multipliers);
        // The as-shot neutral, balanced, lands on AP1 (1, 1, 1).
        let n = [0.6f32, 1.0, 0.8];
        let balanced = [0, 1, 2].map(|i| n[i] * t.multipliers[i]);
        for row in t.matrix {
            let v: f32 = (0..3).map(|i| row[i] * balanced[i]).sum();
            assert!((v - 1.0).abs() < 1e-4, "{v}");
        }
        // An explicit white is used as given.
        let d65 = camera_transform(&c, Some(genzo_color::camera::D65_XY));
        assert_eq!(d65.white, genzo_color::camera::D65_XY);
        assert_ne!(d65.multipliers, t.multipliers);
    }

    #[test]
    fn falls_back_without_a_usable_profile() {
        // No matrix: pass through, balanced by the as-shot neutral.
        let mut c = color(CM.to_vec(), Some(raw::AsShot::Neutral([0.5, 1.0, 0.8])));
        c.color_matrix_1 = None;
        let t = camera_transform(&c, None);
        assert!(matches!(t.fallback, Some(ColorFallback::NoProfile(_))));
        assert_eq!(t.matrix, IDENTITY);
        assert_eq!(t.multipliers, [2.0, 1.0, 1.25]);
        // A singular matrix: no white solves, and no transform.
        let t = camera_transform(&color(vec![0.0; 9], None), None);
        assert!(
            matches!(t.fallback, Some(ColorFallback::NoProfile(_))),
            "{t:?}"
        );
        // A NaN in the profile is refused.
        let mut m = CM.to_vec();
        m[0] = f64::NAN;
        assert!(matches!(
            camera_transform(&color(m, None), None).fallback,
            Some(ColorFallback::NoProfile(_))
        ));
    }

    #[test]
    fn no_as_shot_white_is_d55() {
        let t = camera_transform(&color(CM.to_vec(), None), None);
        assert_eq!((t.white, t.fallback), (D55_XY, None));
    }

    fn stage4(t: &CameraTransform, camera: [f32; 3]) -> [f32; 3] {
        let balanced = [0, 1, 2].map(|i| camera[i] * t.multipliers[i]);
        t.matrix
            .map(|row| (0..3).map(|i| row[i] * balanced[i]).sum())
    }

    fn apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
        m.map(|row| (0..3).map(|i| row[i] * v[i]).sum())
    }

    #[test]
    fn the_drag_ratio_turns_one_white_into_another() {
        let c = color(CM.to_vec(), Some(raw::AsShot::Neutral([0.6, 1.0, 0.8])));
        let old = camera_transform(&c, None);
        let same = drag_ratio(&old, &old).unwrap();
        for (i, row) in same.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - f32::from(u8::from(i == j))).abs() < 1e-6, "{same:?}");
            }
        }
        for (t, tint) in [(2856.0, 0.0), (7500.0, 10.0), (4000.0, -30.0)] {
            let xy = genzo_color::camera::xy_from_temperature(t, tint);
            let new = camera_transform(&c, Some(xy));
            let m = drag_ratio(&old, &new).unwrap();
            for camera in [[0.1, 0.2, 0.05], [0.3, 0.3, 0.3], [0.02, 0.4, 0.2]] {
                let (got, want) = (apply(&m, stage4(&old, camera)), stage4(&new, camera));
                for k in 0..3 {
                    assert!(
                        (got[k] - want[k]).abs() < 1e-5,
                        "{t} {tint}: {got:?} {want:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_non_raw_drag_is_its_cat16_shift() {
        let e = InputEncoding::srgb();
        let old = encoded_transform(&e);
        let new = balanced_encoded_transform(&e, 40.0, -25.0);
        let m = drag_ratio(&old, &new).unwrap();
        let want = genzo_color::white_balance::relative_adaptation(40.0, -25.0);
        for (a, b) in m.iter().flatten().zip(want.iter().flatten()) {
            assert!((f64::from(*a) - b).abs() < 1e-6, "{m:?}");
        }
    }
}
