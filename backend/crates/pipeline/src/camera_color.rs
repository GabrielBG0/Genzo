//! The decoder's camera color data as the color crate's DNG profile
//! (stages 2 and 4). The sizes are checked as the DNG SDK checks them.

use genzo_color::Mat3;
use genzo_color::camera::{AsShot, ColorSpec, D55_XY, DngProfile};
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
    /// Stage 4: 2^BaselineExposure.
    pub exposure: f32,
    pub fallback: Option<ColorFallback>,
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The transform for `white`, or for the as-shot white when None (D55 when
/// the file has none, as the SDK renders it).
pub fn camera_transform(c: &CameraColor, white: Option<[f64; 2]>) -> CameraTransform {
    let exposure = 2f64.powf(c.baseline_exposure) as f32;
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
    }
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
}
