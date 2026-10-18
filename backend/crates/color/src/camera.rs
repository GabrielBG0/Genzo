//! Camera color for RAW files (DEV-01, stages 2 and 4): from a DNG camera
//! profile and a white point to white balance multipliers and a camera to
//! working space matrix.
//!
//! This follows the Adobe DNG SDK 1.4 (`dng_color_spec.cpp`,
//! `dng_camera_profile.cpp`, `dng_temperature.cpp`, `dng_xy_coord.cpp`) step
//! by step, in f64, and is tested against that SDK's own results
//! (scripts/color-reference.py). The profile connection space is CIE XYZ
//! with the DNG D50 white, xy (0.3457, 0.3585), not the ICC one.
//!
//! Only three color channels are handled; four-color sensors are refused.
//! DNG 1.6's third illuminant (ColorMatrix3) is not read: SDK 1.4 predates
//! it and no file in the corpus has one.
//!
//! Ported from the DNG SDK 1.4 as distributed in gopro/gpr, under this
//! notice. Changes: Rust and three channels only; errors are values instead
//! of exceptions; the stage 2 and 4 split is Genzo's.
//!
//! Copyright (c) 1999 - 2014, Adobe Systems Incorporated
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are
//! met:
//!
//! * Redistributions of source code must retain the above copyright notice,
//!   this list of conditions and the following disclaimer.
//! * Redistributions in binary form must reproduce the above copyright
//!   notice, this list of conditions and the following disclaimer in the
//!   documentation and/or other materials provided with the distribution.
//! * Neither the name of Adobe Systems Incorporated, nor the names of its
//!   contributors may be used to endorse or promote products derived from
//!   this software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
//! IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
//! THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
//! PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::{Mat3, WORKING_TO_XYZ, invert, mat_mul, mul};

/// White points as CIE xy, the values the DNG SDK uses.
pub const D50_XY: [f64; 2] = [0.3457, 0.3585];
pub const D55_XY: [f64; 2] = [0.3324, 0.3474];
pub const D65_XY: [f64; 2] = [0.3127, 0.3290];
pub const STD_A_XY: [f64; 2] = [0.4476, 0.4074];

/// A DNG camera profile as stored in the file, for three color channels.
/// Non-DNG RAW files give the color matrices only.
#[derive(Debug, Clone, PartialEq)]
pub struct DngProfile {
    /// ColorMatrix1: XYZ to camera, row-major.
    pub color_matrix_1: Mat3,
    /// CalibrationIlluminant1 (an EXIF LightSource value), 0 when absent.
    pub illuminant_1: u16,
    pub color_matrix_2: Option<Mat3>,
    pub illuminant_2: u16,
    /// ForwardMatrix1 and 2: white balanced camera values to XYZ D50.
    pub forward_matrix_1: Option<Mat3>,
    pub forward_matrix_2: Option<Mat3>,
    /// CameraCalibration1 and 2; absent means identity.
    pub camera_calibration_1: Option<Mat3>,
    pub camera_calibration_2: Option<Mat3>,
    /// AnalogBalance; 1 when absent.
    pub analog_balance: [f64; 3],
    /// CameraCalibrationSignature and ProfileCalibrationSignature, empty when
    /// absent. The camera calibration is used only when they are equal.
    pub camera_calibration_signature: String,
    pub profile_calibration_signature: String,
}

impl DngProfile {
    /// A profile of one color matrix and nothing else.
    pub fn from_matrix(color_matrix_1: Mat3, illuminant_1: u16) -> Self {
        DngProfile {
            color_matrix_1,
            illuminant_1,
            color_matrix_2: None,
            illuminant_2: 0,
            forward_matrix_1: None,
            forward_matrix_2: None,
            camera_calibration_1: None,
            camera_calibration_2: None,
            analog_balance: [1.0; 3],
            camera_calibration_signature: String::new(),
            profile_calibration_signature: String::new(),
        }
    }
}

/// The white balance the camera recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AsShot {
    /// AsShotNeutral: the camera values of a neutral.
    Neutral([f64; 3]),
    /// AsShotWhiteXY.
    WhiteXy([f64; 2]),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("a camera profile value is not finite")]
    NotFinite,
    #[error("ForwardMatrix{0} does not map camera white to D50")]
    ForwardMatrix(u8),
}

/// The profile prepared for a white point, as `dng_color_spec` holds it:
/// matrices normalized and rounded, AnalogBalance and CameraCalibration
/// folded into the color matrices, and the two calibrations ordered by
/// temperature.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSpec {
    temperature_1: f64,
    temperature_2: f64,
    color_matrix_1: Mat3,
    color_matrix_2: Mat3,
    forward_matrix_1: Option<Mat3>,
    forward_matrix_2: Option<Mat3>,
    camera_calibration_1: Mat3,
    camera_calibration_2: Mat3,
    analog_balance: Mat3,
}

/// The color for one white point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WhiteSpec {
    /// The camera values of the white, the largest at 1, all in 0.001 to 1.
    pub camera_white: [f64; 3],
    /// Camera values to XYZ D50. The camera white maps to the D50 white
    /// with Y = 1.
    pub camera_to_pcs: Mat3,
}

/// Stages 2 and 4 for one white point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraToWorking {
    /// Stage 2: per channel gains, 1 / camera white. The smallest is 1.
    pub multipliers: [f64; 3],
    /// Stage 4: white balanced camera values to the working space (AP1,
    /// D65). It maps (1, 1, 1) to (1, 1, 1).
    pub matrix: Mat3,
}

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The temperature of an EXIF LightSource, as the DNG SDK assigns it; 0 for
/// unknown, other and anything unlisted.
pub fn illuminant_temperature(light: u16) -> f64 {
    match light {
        17 | 3 => 2850.0,                  // Standard light A, tungsten
        24 => 3200.0,                      // ISO studio tungsten
        23 => 5000.0,                      // D50
        20 | 1 | 9 | 4 | 18 => 5500.0,     // D55, daylight, fine weather, flash, B
        21 | 19 | 10 => 6500.0,            // D65, C, cloudy
        22 | 11 => 7500.0,                 // D75, shade
        12 => (5700.0 + 7100.0) * 0.5,     // daylight fluorescent
        13 => (4600.0 + 5500.0) * 0.5,     // day white fluorescent
        14 | 2 => (3800.0 + 4500.0) * 0.5, // cool white fluorescent, fluorescent
        15 => (3250.0 + 3800.0) * 0.5,     // white fluorescent
        16 => (2600.0 + 3250.0) * 0.5,     // warm white fluorescent
        _ => 0.0,
    }
}

/// xy to XYZ with Y = 1, the xy kept inside the range of real colors
/// (`XYtoXYZ`).
pub fn xy_to_xyz([x, y]: [f64; 2]) -> [f64; 3] {
    let (mut x, mut y) = (x.clamp(0.000_001, 0.999_999), y.clamp(0.000_001, 0.999_999));
    if x + y > 0.999_999 {
        let scale = 0.999_999 / (x + y);
        x *= scale;
        y *= scale;
    }
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// XYZ to xy; D50 for a total that is not positive (`XYZtoXY`).
pub fn xyz_to_xy([x, y, z]: [f64; 3]) -> [f64; 2] {
    let total = x + y + z;
    if total > 0.0 {
        [x / total, y / total]
    } else {
        D50_XY
    }
}

/// The linearized Bradford adaptation from `from` to `to`, each scale
/// limited to 0.1 to 10 (`MapWhiteMatrix`).
pub fn map_white_matrix(from: [f64; 2], to: [f64; 2]) -> Mat3 {
    const MB: Mat3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let w1 = mul(&MB, xy_to_xyz(from)).map(|c| c.max(0.0));
    let w2 = mul(&MB, xy_to_xyz(to)).map(|c| c.max(0.0));
    let mut a = [[0.0; 3]; 3];
    for i in 0..3 {
        let ratio = if w1[i] > 0.0 { w2[i] / w1[i] } else { 10.0 };
        a[i][i] = ratio.clamp(0.1, 10.0);
    }
    let inv = invert(&MB).expect("the Bradford matrix is invertible");
    mat_mul(&mat_mul(&inv, &a), &MB)
}

/// The SDK's rounding (`Round_int32`): half away from zero.
fn round_half_away(x: f64) -> f64 {
    (if x > 0.0 { x + 0.5 } else { x - 0.5 }).trunc()
}

/// `dng_vector::Round` and `dng_matrix::Round`: to a multiple of 1 / factor.
fn round_to(v: f64, factor: f64) -> f64 {
    round_half_away(v * factor) * (1.0 / factor)
}

/// The SDK's rounding of profile matrices and CameraCalibration to four
/// decimals.
fn round_4(m: &Mat3) -> Mat3 {
    m.map(|row| row.map(|v| round_to(v, 10_000.0)))
}

/// `SetCameraNeutral`: scaled so the largest value is 1, rounded to six
/// decimals. None when no value is positive.
fn normalize_neutral(n: [f64; 3]) -> Option<[f64; 3]> {
    let max = max_entry(n);
    (max > 0.0).then(|| n.map(|c| round_to(c * (1.0 / max), 1_000_000.0)))
}

/// `SetAnalogBalance`: scaled so the smallest gain is 1, rounded to six
/// decimals; all 1 when a gain is not positive.
fn normalize_analog_balance(b: [f64; 3]) -> [f64; 3] {
    let min = b[0].min(b[1]).min(b[2]);
    if min > 0.0 {
        b.map(|c| round_to(c * (1.0 / min), 1_000_000.0))
    } else {
        [1.0; 3]
    }
}

fn scale(m: &Mat3, s: f64) -> Mat3 {
    m.map(|row| row.map(|v| v * s))
}

fn blend(a: &Mat3, b: &Mat3, g: f64) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = g * a[i][j] + (1.0 - g) * b[i][j];
        }
    }
    out
}

fn diag(v: [f64; 3]) -> Mat3 {
    [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]]
}

fn max_entry(v: [f64; 3]) -> f64 {
    v[0].max(v[1]).max(v[2])
}

/// `NormalizeColorMatrix`: scaled so the PCS white has a largest camera
/// value of 1 when it is not already within 1 %, then rounded.
fn normalize_color_matrix(m: &Mat3) -> Mat3 {
    let max = max_entry(mul(m, xy_to_xyz(D50_XY)));
    let m = if max > 0.0 && !(0.99..=1.01).contains(&max) {
        scale(m, 1.0 / max)
    } else {
        *m
    };
    round_4(&m)
}

/// `ValidForwardMatrix`: camera (1, 1, 1) lands within 0.01 of the PCS
/// white.
fn valid_forward_matrix(m: &Mat3) -> bool {
    let xyz = mul(m, [1.0; 3]);
    let pcs = xy_to_xyz(D50_XY);
    (0..3).all(|i| (xyz[i] - pcs[i]).abs() <= 0.01)
}

/// `NormalizeForwardMatrix`: scaled per row so camera (1, 1, 1) maps to the
/// PCS white exactly.
fn normalize_forward_matrix(m: &Mat3) -> Mat3 {
    let xyz = mul(m, [1.0; 3]);
    let pcs = xy_to_xyz(D50_XY);
    let mut out = *m;
    for i in 0..3 {
        out[i] = m[i].map(|v| pcs[i] * (1.0 / xyz[i]) * v);
    }
    out
}

impl ColorSpec {
    /// Prepares `profile` (`dng_camera_profile` loading followed by the
    /// `dng_color_spec` constructor).
    pub fn new(profile: &DngProfile) -> Result<Self, ProfileError> {
        let p = profile;
        let finite = |m: &Mat3| m.iter().flatten().all(|v| v.is_finite());
        let all = [Some(p.color_matrix_1), p.color_matrix_2, p.forward_matrix_1]
            .into_iter()
            .chain([
                p.forward_matrix_2,
                p.camera_calibration_1,
                p.camera_calibration_2,
            ])
            .flatten();
        if !all.into_iter().all(|m| finite(&m)) || !p.analog_balance.iter().all(|v| v.is_finite()) {
            return Err(ProfileError::NotFinite);
        }

        // The profile as the SDK loads it: color matrices normalized and
        // rounded, forward matrices rounded. The second calibration exists
        // only with a second color matrix.
        let cm1 = normalize_color_matrix(&p.color_matrix_1);
        let cm2 = p.color_matrix_2.map(|m| normalize_color_matrix(&m));
        let fm1 = p.forward_matrix_1.map(|m| round_4(&m));
        let fm2 = cm2.and(p.forward_matrix_2).map(|m| round_4(&m));
        for (i, fm) in [(1, fm1), (2, fm2)] {
            if fm.is_some_and(|m| !valid_forward_matrix(&m)) {
                return Err(ProfileError::ForwardMatrix(i));
            }
        }
        let t1 = illuminant_temperature(p.illuminant_1);
        let t2 = if cm2.is_some() {
            illuminant_temperature(p.illuminant_2)
        } else {
            0.0
        };

        let calibrated = p.camera_calibration_signature == p.profile_calibration_signature;
        let cc = |m: Option<Mat3>| m.filter(|_| calibrated).map_or(IDENTITY, |m| round_4(&m));
        let (cc1, cc2) = (cc(p.camera_calibration_1), cc(p.camera_calibration_2));
        let ab = diag(normalize_analog_balance(p.analog_balance));
        let fold = |cc: &Mat3, cm: &Mat3| mat_mul(&mat_mul(&ab, cc), cm);

        let fm1 = fm1.map(|m| normalize_forward_matrix(&m));
        let cm1 = fold(&cc1, &cm1);
        let spec = match cm2 {
            Some(cm2) if t1 > 0.0 && t2 > 0.0 && t1 != t2 => {
                let fm2 = fm2.map(|m| normalize_forward_matrix(&m));
                let cm2 = fold(&cc2, &cm2);
                let one = (t1, cm1, fm1, cc1);
                let two = (t2, cm2, fm2, cc2);
                let (a, b) = if t1 > t2 { (two, one) } else { (one, two) };
                ColorSpec {
                    temperature_1: a.0,
                    temperature_2: b.0,
                    color_matrix_1: a.1,
                    color_matrix_2: b.1,
                    forward_matrix_1: a.2,
                    forward_matrix_2: b.2,
                    camera_calibration_1: a.3,
                    camera_calibration_2: b.3,
                    analog_balance: ab,
                }
            }
            _ => ColorSpec {
                temperature_1: 5000.0,
                temperature_2: 5000.0,
                color_matrix_1: cm1,
                color_matrix_2: cm1,
                forward_matrix_1: fm1,
                forward_matrix_2: fm1,
                camera_calibration_1: cc1,
                camera_calibration_2: cc1,
                analog_balance: ab,
            },
        };
        Ok(spec)
    }

    /// `FindXYZtoCamera`: the color matrix for `white`, with the forward
    /// matrix and camera calibration, interpolated in inverse temperature.
    fn xyz_to_camera(&self, white: [f64; 2]) -> (Mat3, Option<Mat3>, Mat3) {
        let (t, _) = temperature_from_xy(white);
        let g = if t <= self.temperature_1 {
            1.0
        } else if t >= self.temperature_2 {
            0.0
        } else {
            (1.0 / t - 1.0 / self.temperature_2)
                / (1.0 / self.temperature_1 - 1.0 / self.temperature_2)
        };
        let pick = |a: &Mat3, b: &Mat3| {
            if g >= 1.0 {
                *a
            } else if g <= 0.0 {
                *b
            } else {
                blend(a, b, g)
            }
        };
        let cm = pick(&self.color_matrix_1, &self.color_matrix_2);
        let fm = match (self.forward_matrix_1, self.forward_matrix_2) {
            (Some(a), Some(b)) => Some(pick(&a, &b)),
            (a, b) => a.or(b),
        };
        let cc = pick(&self.camera_calibration_1, &self.camera_calibration_2);
        (cm, fm, cc)
    }

    /// The white point of a camera neutral (`NeutralToXY`): iterated from
    /// D50, since the matrix depends on the white. None when a matrix is
    /// singular.
    pub fn neutral_to_xy(&self, neutral: [f64; 3]) -> Option<[f64; 2]> {
        const MAX_PASSES: u32 = 30;
        let mut last = D50_XY;
        for pass in 0..MAX_PASSES {
            let (cm, _, _) = self.xyz_to_camera(last);
            let mut next = xyz_to_xy(mul(&invert(&cm)?, neutral));
            if (next[0] - last[0]).abs() + (next[1] - last[1]).abs() < 0.000_000_1 {
                return Some(next);
            }
            // Not converged: most likely a two value oscillation, so take
            // the mean of the last two estimates.
            if pass == MAX_PASSES - 1 {
                next = [(last[0] + next[0]) * 0.5, (last[1] + next[1]) * 0.5];
            }
            last = next;
        }
        Some(last)
    }

    /// The white point to render with: the as-shot neutral, else the as-shot
    /// xy, else D55 (as the SDK's renderer does). The neutral is first scaled
    /// to a largest value of 1 and both are rounded to six decimals, as the
    /// SDK stores them. None when a matrix is singular.
    pub fn as_shot_xy(&self, as_shot: Option<AsShot>) -> Option<[f64; 2]> {
        match as_shot {
            Some(AsShot::Neutral(n)) => match normalize_neutral(n) {
                Some(n) => self.neutral_to_xy(n),
                None => Some(D55_XY),
            },
            // SetCameraWhiteXY divides, unlike Round.
            Some(AsShot::WhiteXy([x, y])) if x > 0.0 && y > 0.0 => Some([
                round_half_away(x * 1_000_000.0) / 1_000_000.0,
                round_half_away(y * 1_000_000.0) / 1_000_000.0,
            ]),
            _ => Some(D55_XY),
        }
    }

    /// `SetWhiteXY`: the camera white and the camera to PCS matrix for
    /// `white`. None when a matrix is singular or the white has no positive
    /// camera value.
    pub fn at_white(&self, white: [f64; 2]) -> Option<WhiteSpec> {
        let (cm, fm, cc) = self.xyz_to_camera(white);
        let raw_white = mul(&cm, xy_to_xyz(white));
        let max = max_entry(raw_white);
        if !(max > 0.0 && max.is_finite()) {
            return None;
        }
        let white_scale = 1.0 / max;
        let camera_white = raw_white.map(|c| (white_scale * c).clamp(0.001, 1.0));

        // PCS to camera, scaled so the PCS white just reaches the first
        // camera channel's saturation.
        let pcs_to_camera = mat_mul(&cm, &map_white_matrix(D50_XY, white));
        let s = max_entry(mul(&pcs_to_camera, xy_to_xyz(D50_XY)));
        let pcs_to_camera = scale(&pcs_to_camera, 1.0 / s);

        let camera_to_pcs = match fm {
            Some(fm) => {
                let individual_to_reference = invert(&mat_mul(&self.analog_balance, &cc))?;
                let reference_white = mul(&individual_to_reference, camera_white);
                let unwhite = invert(&diag(reference_white))?;
                mat_mul(&mat_mul(&fm, &unwhite), &individual_to_reference)
            }
            None => invert(&pcs_to_camera)?,
        };
        Some(WhiteSpec {
            camera_white,
            camera_to_pcs,
        })
    }

    /// Stages 2 and 4 for `white`: the white balance multipliers, and the
    /// matrix from white balanced camera values to the working space, the
    /// PCS adapted from D50 to D65 with the same Bradford transform.
    pub fn camera_to_working(&self, white: [f64; 2]) -> Option<CameraToWorking> {
        let w = self.at_white(white)?;
        let xyz_to_working = invert(&WORKING_TO_XYZ).expect("the working space is invertible");
        let pcs_to_working = mat_mul(&xyz_to_working, &map_white_matrix(D50_XY, D65_XY));
        Some(CameraToWorking {
            multipliers: w.camera_white.map(|c| 1.0 / c),
            matrix: mat_mul(
                &mat_mul(&pcs_to_working, &w.camera_to_pcs),
                &diag(w.camera_white),
            ),
        })
    }
}

/// Scale between distances in uv and the tint parameter.
const TINT_SCALE: f64 = -3000.0;

/// Isotemperature lines from Wyszecki & Stiles, "Color Science", second
/// edition, page 228: (reciprocal megakelvin, u, v, slope).
const TEMP_TABLE: [[f64; 4]; 31] = [
    [0.0, 0.18006, 0.26352, -0.24341],
    [10.0, 0.18066, 0.26589, -0.25479],
    [20.0, 0.18133, 0.26846, -0.26876],
    [30.0, 0.18208, 0.27119, -0.28539],
    [40.0, 0.18293, 0.27407, -0.30470],
    [50.0, 0.18388, 0.27709, -0.32675],
    [60.0, 0.18494, 0.28021, -0.35156],
    [70.0, 0.18611, 0.28342, -0.37915],
    [80.0, 0.18740, 0.28668, -0.40955],
    [90.0, 0.18880, 0.28997, -0.44278],
    [100.0, 0.19032, 0.29326, -0.47888],
    [125.0, 0.19462, 0.30141, -0.58204],
    [150.0, 0.19962, 0.30921, -0.70471],
    [175.0, 0.20525, 0.31647, -0.84901],
    [200.0, 0.21142, 0.32312, -1.0182],
    [225.0, 0.21807, 0.32909, -1.2168],
    [250.0, 0.22511, 0.33439, -1.4512],
    [275.0, 0.23247, 0.33904, -1.7298],
    [300.0, 0.24010, 0.34308, -2.0637],
    [325.0, 0.24702, 0.34655, -2.4681],
    [350.0, 0.25591, 0.34951, -2.9641],
    [375.0, 0.26400, 0.35200, -3.5814],
    [400.0, 0.27218, 0.35407, -4.3633],
    [425.0, 0.28039, 0.35577, -5.3762],
    [450.0, 0.28863, 0.35714, -6.7262],
    [475.0, 0.29685, 0.35823, -8.5955],
    [500.0, 0.30505, 0.35907, -11.324],
    [525.0, 0.31320, 0.35968, -15.628],
    [550.0, 0.32129, 0.36011, -23.325],
    [575.0, 0.32931, 0.36038, -40.770],
    [600.0, 0.33724, 0.36051, -116.45],
];

/// Temperature (kelvin) and tint of a white point, by Robertson's method
/// (`dng_temperature::Set_xy_coord`).
pub fn temperature_from_xy([x, y]: [f64; 2]) -> (f64, f64) {
    let u = 2.0 * x / (1.5 - x + 6.0 * y);
    let v = 3.0 * y / (1.5 - x + 6.0 * y);
    let (mut last_dt, mut last_du, mut last_dv) = (0.0, 0.0, 0.0);
    for index in 1..=30 {
        let [_, tu, tv, slope] = TEMP_TABLE[index];
        let len = (1.0 + slope * slope).sqrt();
        let (du, dv) = (1.0 / len, slope / len);
        let (uu, vv) = (u - tu, v - tv);
        // Distance above or below the line.
        let mut dt = -uu * dv + vv * du;
        if dt <= 0.0 || index == 30 {
            if dt > 0.0 {
                dt = 0.0;
            }
            dt = -dt;
            let f = if index == 1 { 0.0 } else { dt / (last_dt + dt) };
            let [r0, u0, v0, _] = TEMP_TABLE[index - 1];
            let [r1, u1, v1, _] = TEMP_TABLE[index];
            let temperature = 1.0e6 / (r0 * f + r1 * (1.0 - f));
            let uu = u - (u0 * f + u1 * (1.0 - f));
            let vv = v - (v0 * f + v1 * (1.0 - f));
            let (du, dv) = (du * (1.0 - f) + last_du * f, dv * (1.0 - f) + last_dv * f);
            let len = (du * du + dv * dv).sqrt();
            let tint = (uu * du / len + vv * dv / len) * TINT_SCALE;
            return (temperature, tint);
        }
        last_dt = dt;
        last_du = du;
        last_dv = dv;
    }
    unreachable!("the last line always matches")
}

/// The white point of a temperature (kelvin) and tint
/// (`dng_temperature::Get_xy_coord`).
pub fn xy_from_temperature(temperature: f64, tint: f64) -> [f64; 2] {
    let r = 1.0e6 / temperature;
    let offset = tint * (1.0 / TINT_SCALE);
    for index in 0..=29 {
        if r < TEMP_TABLE[index + 1][0] || index == 29 {
            let [r0, u0, v0, s0] = TEMP_TABLE[index];
            let [r1, u1, v1, s1] = TEMP_TABLE[index + 1];
            let f = (r1 - r) / (r1 - r0);
            let mut u = u0 * f + u1 * (1.0 - f);
            let mut v = v0 * f + v1 * (1.0 - f);
            let (len1, len2) = ((1.0 + s0 * s0).sqrt(), (1.0 + s1 * s1).sqrt());
            let (uu1, vv1) = (1.0 / len1, s0 / len1);
            let (uu2, vv2) = (1.0 / len2, s1 / len2);
            let (uu3, vv3) = (uu1 * f + uu2 * (1.0 - f), vv1 * f + vv2 * (1.0 - f));
            let len3 = (uu3 * uu3 + vv3 * vv3).sqrt();
            u += uu3 / len3 * offset;
            v += vv3 / len3 * offset;
            return [1.5 * u / (u - 4.0 * v + 2.0), v / (u - 4.0 * v + 2.0)];
        }
    }
    unreachable!("index 29 always matches")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maps the PCS white to about (1, 1, 1), so normalization leaves it
    /// alone; four decimals.
    const CM: Mat3 = [[1.0371, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.212]];

    #[test]
    fn rounds_matrices_as_the_sdk_loads_them() {
        let base = ColorSpec::new(&DngProfile::from_matrix(CM, 21)).unwrap();
        let mut cm = CM;
        cm[1][0] += 3e-5; // inside one 1e-4 step
        assert_eq!(
            ColorSpec::new(&DngProfile::from_matrix(cm, 21)).unwrap(),
            base
        );
        cm[1][0] += 4e-5; // past it
        assert_ne!(
            ColorSpec::new(&DngProfile::from_matrix(cm, 21)).unwrap(),
            base
        );
        // Half away from zero, on both sides.
        assert_eq!(round_to(0.000_05, 10_000.0), 0.0001);
        assert_eq!(round_to(-0.000_05, 10_000.0), -0.0001);
    }

    #[test]
    fn normalizes_color_matrices_outside_one_percent() {
        let spec = ColorSpec::new(&DngProfile::from_matrix(scale(&CM, 0.5), 21)).unwrap();
        assert_eq!(
            spec.color_matrix_1,
            round_4(&scale(
                &CM,
                0.5 / (0.5 * max_entry(mul(&CM, xy_to_xyz(D50_XY))))
            ))
        );
        let spec = ColorSpec::new(&DngProfile::from_matrix(scale(&CM, 1.005), 21)).unwrap();
        assert_eq!(spec.color_matrix_1, round_4(&scale(&CM, 1.005)));
    }

    #[test]
    fn one_matrix_counts_as_5000_kelvin() {
        for p in [
            DngProfile::from_matrix(CM, 21),
            DngProfile {
                color_matrix_2: Some(CM),
                illuminant_2: 255, // other: no temperature
                ..DngProfile::from_matrix(CM, 17)
            },
            DngProfile {
                color_matrix_2: Some(CM),
                illuminant_2: 21,
                ..DngProfile::from_matrix(CM, 21) // equal temperatures
            },
        ] {
            let spec = ColorSpec::new(&p).unwrap();
            assert_eq!((spec.temperature_1, spec.temperature_2), (5000.0, 5000.0));
        }
    }

    #[test]
    fn as_shot_white_falls_back_to_d55() {
        let spec = ColorSpec::new(&DngProfile::from_matrix(CM, 21)).unwrap();
        assert_eq!(spec.as_shot_xy(None), Some(D55_XY));
        assert_eq!(
            spec.as_shot_xy(Some(AsShot::Neutral([0.0, 0.0, -1.0]))),
            Some(D55_XY)
        );
        assert_eq!(
            spec.as_shot_xy(Some(AsShot::WhiteXy([0.0, 0.3]))),
            Some(D55_XY)
        );
        assert_eq!(
            spec.as_shot_xy(Some(AsShot::WhiteXy([0.312_700_4, 0.329]))),
            Some([0.3127, 0.329])
        );
        // A neutral is scaled before use: its size does not matter.
        let a = spec
            .as_shot_xy(Some(AsShot::Neutral([0.5, 1.0, 0.7])))
            .unwrap();
        let b = spec
            .as_shot_xy(Some(AsShot::Neutral([1.0, 2.0, 1.4])))
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn refuses_bad_profiles_without_panicking() {
        let mut p = DngProfile::from_matrix(CM, 21);
        p.color_matrix_1[0][0] = f64::NAN;
        assert_eq!(ColorSpec::new(&p), Err(ProfileError::NotFinite));
        let mut p = DngProfile::from_matrix(CM, 21);
        p.forward_matrix_1 = Some(IDENTITY); // maps 1 to (1, 1, 1), not D50
        assert_eq!(ColorSpec::new(&p), Err(ProfileError::ForwardMatrix(1)));
        // A singular matrix: no white, no solution.
        let spec = ColorSpec::new(&DngProfile::from_matrix([[1.0, 0.0, 0.0]; 3], 21)).unwrap();
        assert_eq!(spec.neutral_to_xy([1.0, 1.0, 1.0]), None);
        let spec = ColorSpec::new(&DngProfile::from_matrix([[0.0; 3]; 3], 21)).unwrap();
        assert_eq!(spec.at_white(D65_XY), None);
        assert_eq!(spec.camera_to_working(D65_XY), None);
    }

    #[test]
    fn temperature_and_tint_round_trip() {
        for t in [2000.0, 2850.0, 4000.0, 5000.0, 6500.0, 10_000.0, 25_000.0] {
            for tint in [-50.0, 0.0, 30.0] {
                let (t2, tint2) = temperature_from_xy(xy_from_temperature(t, tint));
                // Robertson's lines are not quite parallel, so the round
                // trip is close, not exact.
                assert!((t2 - t).abs() / t < 1e-3, "{t} {tint}: {t2}");
                assert!((tint2 - tint).abs() < 0.05, "{t} {tint}: {tint2}");
            }
        }
    }

    #[test]
    fn illuminants_take_the_sdk_temperatures() {
        assert_eq!(illuminant_temperature(17), 2850.0);
        assert_eq!(illuminant_temperature(21), 6500.0);
        assert_eq!(illuminant_temperature(23), 5000.0);
        assert_eq!(illuminant_temperature(14), 4150.0);
        assert_eq!(illuminant_temperature(0), 0.0);
        assert_eq!(illuminant_temperature(255), 0.0);
    }
}
