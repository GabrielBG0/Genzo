//! The decoder's camera color data as the color crate's DNG profile
//! (stages 2 and 4). The sizes are checked as the DNG SDK checks them.

use genzo_color::Mat3;
use genzo_color::camera::{AsShot, DngProfile};
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
