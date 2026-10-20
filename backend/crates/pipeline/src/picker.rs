//! The white balance picker (DEV-02, *The picker*): the Temp and Tint that
//! make the area around a point on the shown image neutral.
//!
//! It reads the decoded input on the CPU, through stage 1 only
//! (`reference::stages::stage1_window`), not the pyramid (owner,
//! 2026-10-20): stage 4 desaturates highlights toward their luminance,
//! which no inverse undoes, so a clipped area read back from the pyramid
//! looks like an unclipped gray. The decoded input shows clipping as it is.
//!
//! - **RAW:** the mean of each camera channel over the window is the camera
//!   neutral, and its white point comes from the same neutral-to-xy solve as
//!   As Shot (`ColorSpec::neutral_to_xy`), then Robertson's Temp and Tint.
//! - **Non-RAW:** the window's mean in the working space, its xy, and the
//!   relative Temp and Tint of that white
//!   (`color::white_balance::relative_from_xy`).
//!
//! Values past the sliders' ranges are clamped to them.

use genzo_color::camera::{ColorSpec, temperature_from_xy, xyz_to_xy};
use genzo_color::white_balance::relative_from_xy;
use genzo_color::{WORKING_TO_XYZ, mul};
use genzo_model::frame::Rect;

use crate::camera_color::{camera_transform, dng_profile};
use crate::raw::RawSource;
use crate::reference::sampling::Frame;
use crate::reference::stages::stage1_window;

/// The window's side in full-resolution pixels: the 5 × 5 texels of pyramid
/// level 1 DEV-02 names.
pub const WINDOW: u32 = 10;

/// A sample at or above this fraction of white is clipped (DEV-02: within
/// 5 % of the clip level).
pub const CLIPPED: f32 = 0.95;

/// Below this luminance, with white at 1, the area is too dark (0.5 %).
pub const TOO_DARK: f64 = 0.005;

/// Why a pick changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickError {
    /// A sample of the area is within 5 % of white.
    Clipped,
    /// The area's luminance is below 0.5 % of white.
    TooDark,
    /// The image records no color (a monochrome sensor), or its camera
    /// profile is unusable, so Temp and Tint mean nothing.
    NoColor,
}

impl PickError {
    /// What the status bar says (DEV-02).
    pub fn message(self) -> &'static str {
        match self {
            PickError::Clipped => "Can't set white balance here: the area is clipped",
            PickError::TooDark => "Can't set white balance here: the area is too dark",
            PickError::NoColor => "This camera records no color",
        }
    }
}

/// The RAW sliders' ranges (DEV-01, *Data*).
const RAW_TEMP: (f64, f64) = (2000.0, 50_000.0);
const RAW_TINT: f64 = 150.0;
/// The non-RAW sliders' range, for Temp and Tint alike.
const RELATIVE: f64 = 100.0;

/// The full-resolution window around `point` (the framed image's
/// normalized coordinates) of `src` shown in `frame`, inside the image.
pub fn window(src: &RawSource, frame: Frame, point: (f64, f64)) -> Rect {
    let (w, h) = src.size();
    let (x, y) = frame.stored_point((w, h), point);
    let side = |c: f64, size: u32| {
        let n = WINDOW.min(size);
        let first = (c.floor() as i64 - i64::from(n / 2)).clamp(0, i64::from(size - n));
        (first as u32, n)
    };
    let ((x0, width), (y0, height)) = (side(x, w), side(y, h));
    Rect {
        x: x0,
        y: y0,
        width,
        height,
    }
}

/// The Temp and Tint that make the area around `point` neutral: kelvin and
/// tint for RAW, relative shifts for non-RAW.
pub fn pick(src: &RawSource, frame: Frame, point: (f64, f64)) -> Result<(f64, f64), PickError> {
    let rect = window(src, frame, point);
    let values = stage1_window(src, rect);
    if values.iter().any(|v| *v >= CLIPPED) {
        return Err(PickError::Clipped);
    }
    match &src.encoding {
        Some(e) => {
            let mut sum = [0f64; 3];
            for px in values.as_chunks::<3>().0 {
                let rgb = [0, 1, 2].map(|c| f64::from(px[c]));
                let working = mul(&e.to_working, rgb);
                for c in 0..3 {
                    sum[c] += working[c];
                }
            }
            let n = (values.len() / 3) as f64;
            let xyz = mul(&WORKING_TO_XYZ, sum.map(|v| v / n));
            if xyz[1] < TOO_DARK {
                return Err(PickError::TooDark);
            }
            let (temp, tint) = relative_from_xy(xyz_to_xy(xyz));
            Ok((
                temp.clamp(-RELATIVE, RELATIVE),
                tint.clamp(-RELATIVE, RELATIVE),
            ))
        }
        None => {
            let neutral = camera_mean(src, rect, &values).ok_or(PickError::NoColor)?;
            let spec = dng_profile(&src.color)
                .and_then(|p| ColorSpec::new(&p).ok())
                .ok_or(PickError::NoColor)?;
            let xy = spec.neutral_to_xy(neutral).ok_or(PickError::NoColor)?;
            // Balanced at its own white, the area is a neutral gray whose
            // value is any channel times its multiplier.
            let t = camera_transform(&src.color, Some(xy));
            if t.fallback.is_some() {
                return Err(PickError::NoColor);
            }
            let gray = f64::from(t.multipliers[1]) * neutral[1];
            if gray < TOO_DARK {
                return Err(PickError::TooDark);
            }
            let (temp, tint) = temperature_from_xy(xy);
            Ok((
                temp.clamp(RAW_TEMP.0, RAW_TEMP.1),
                tint.clamp(-RAW_TINT, RAW_TINT),
            ))
        }
    }
}

/// The mean camera value per channel over `rect`: by CFA color for a
/// mosaic, by plane for a linear image. None for a monochrome image, or a
/// window that misses a color.
fn camera_mean(src: &RawSource, rect: Rect, values: &[f32]) -> Option<[f64; 3]> {
    let mut sum = [0f64; 3];
    let mut count = [0u32; 3];
    match (src.samples, &src.cfa) {
        (1, Some(_)) => {
            let mut i = 0;
            for row in rect.y..rect.y + rect.height {
                for col in rect.x..rect.x + rect.width {
                    let c = usize::from(src.color_at(row, col)?);
                    sum[c] += f64::from(values[i]);
                    count[c] += 1;
                    i += 1;
                }
            }
        }
        (3, None) => {
            for px in values.as_chunks::<3>().0 {
                for c in 0..3 {
                    sum[c] += f64::from(px[c]);
                    count[c] += 1;
                }
            }
        }
        _ => return None,
    }
    if count.contains(&0) {
        return None;
    }
    Some([0, 1, 2].map(|c| sum[c] / f64::from(count[c])))
}
