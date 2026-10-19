//! The decoder's RAW output as stages 1 to 3 take it (DEV-01): the stored
//! samples, the active area, the CFA pattern and the constants of the
//! linearization, computed as the Adobe DNG SDK 1.4 computes them
//! (`dng_linearization_info.cpp`).

use genzo_color::encoding::{EncodingError, InputEncoding};
use genzo_imgio::opcodes::{FixBadPixelsConstant, GainMap, OpcodePlan, StageOpcode};
use genzo_imgio::raw::{CameraColor, Levels, Rect};
use genzo_imgio::{ColorEncoding, DecodedImage, Pixels};

/// What stages 1 to 3 cannot take.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RawError {
    #[error(transparent)]
    Encoding(#[from] EncodingError),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("corrupt levels: {0}")]
    Levels(String),
}

/// A color filter pattern from the stored image's first sample: the colors
/// 0 red, 1 green, 2 blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cfa {
    /// At (row, col) (0, 0), (0, 1), (1, 0), (1, 1).
    Bayer([u8; 4]),
    /// Fujifilm's 6 x 6, row by row.
    XTrans([[u8; 6]; 6]),
}

impl Cfa {
    /// The color at stored (row, col).
    pub fn color(&self, row: u32, col: u32) -> u8 {
        match self {
            Cfa::Bayer(b) => b[((row & 1) * 2 + (col & 1)) as usize],
            Cfa::XTrans(x) => x[(row % 6) as usize][(col % 6) as usize],
        }
    }
}

/// An image ready for stage 1: a RAW one, or a non-RAW one whose
/// `encoding` replaces the levels.
#[derive(Debug, Clone)]
pub struct RawSource {
    /// The stored image: every sample the file holds, masked borders included.
    pub stored: (u32, u32),
    /// Samples per pixel: 1 for a mosaic or a monochrome plane, 3 for linear RGB.
    pub samples: u32,
    /// Row by row, `samples` interleaved per pixel.
    pub data: Vec<u16>,
    /// The color filter pattern; None for linear input.
    pub cfa: Option<Cfa>,
    /// The ActiveArea, in stored coordinates. Stages 1 to 3 cover it.
    pub active: Rect,
    pub linearization: Linearization,
    /// OpcodeList1: on the stored values.
    pub fix_bad_pixels: Vec<FixBadPixelsConstant>,
    /// OpcodeList2: after linearization, in active-area coordinates.
    pub gain_maps: Vec<GainMap>,
    /// The camera's color data, for stages 2 and 4 ([`crate::camera_color`]).
    pub color: CameraColor,
    /// A non-RAW image's encoding (DEV-01, stage 4): stage 1 looks each
    /// code up in its curves in place of the levels, white balance is 1,
    /// and stage 4 takes its matrix ([`crate::camera_color::encoded_transform`]).
    pub encoding: Option<InputEncoding>,
}

/// Stage 1's levels, per plane, already multiplied by the scale: a sample
/// `v` at active-area (row, col) becomes
/// `v · scale − (pattern[row % rows][col % cols] + delta_h[col] + delta_v[row])`.
#[derive(Debug, Clone, PartialEq)]
pub struct Linearization {
    /// 1 / (white − the largest black level), per plane (`MaxBlackLevel`).
    pub scale: Vec<f32>,
    /// The black level pattern times the scale, `(row · cols + col) · planes + plane`.
    pub pattern: Vec<f32>,
    pub rows: u32,
    pub cols: u32,
    /// BlackLevelDeltaH and DeltaV times the scale, per plane: `[plane][col]`
    /// and `[plane][row]`. Empty when the file has none.
    pub delta_h: Vec<Vec<f32>>,
    pub delta_v: Vec<Vec<f32>>,
}

impl Linearization {
    /// The SDK's constants for `planes` planes. Positions start at the active
    /// area's top left, as `levels` does.
    pub fn new(levels: &Levels, planes: u32, active: (u32, u32)) -> Result<Self, RawError> {
        let (rows, cols) = (
            u32::from(levels.black_repeat.0),
            u32::from(levels.black_repeat.1),
        );
        let n = planes as usize;
        if rows == 0 || cols == 0 || levels.black.len() != (rows * cols) as usize * n {
            return Err(RawError::Levels(format!(
                "{} black levels for a {rows}x{cols} pattern of {planes} planes",
                levels.black.len()
            )));
        }
        let (width, height) = active;
        for (what, delta, len) in [
            ("BlackLevelDeltaH", &levels.black_delta_h, width),
            ("BlackLevelDeltaV", &levels.black_delta_v, height),
        ] {
            if !delta.is_empty() && delta.len() != len as usize {
                return Err(RawError::Levels(format!(
                    "{} {what} entries for {len}",
                    delta.len()
                )));
            }
        }
        // The largest value of each delta per pattern phase (`MaxBlackLevel`):
        // 0 without deltas.
        let max_phase = |delta: &[f64], phases: u32| {
            let mut max = vec![0.0f64; phases as usize];
            for (j, &d) in delta.iter().enumerate() {
                let entry = &mut max[j % phases as usize];
                *entry = if j < phases as usize { d } else { entry.max(d) };
            }
            max
        };
        let max_h = max_phase(&levels.black_delta_h, cols);
        let max_v = max_phase(&levels.black_delta_v, rows);
        let black = |j: u32, k: u32, p: usize| levels.black[((j * cols + k) as usize) * n + p];
        let mut scale = Vec::with_capacity(n);
        for p in 0..n {
            let white = match levels.white.as_slice() {
                [w] => *w,
                w if w.len() == n => w[p],
                w => {
                    return Err(RawError::Levels(format!(
                        "{} white levels for {planes} planes",
                        w.len()
                    )));
                }
            };
            let mut max_black = f64::NEG_INFINITY;
            for j in 0..rows {
                for k in 0..cols {
                    let b = black(j, k, p) + max_h[k as usize] + max_v[j as usize];
                    max_black = max_black.max(b);
                }
            }
            let range = f64::from(white) - max_black;
            // Also refuses NaN.
            if range.is_nan() || range <= 0.0 {
                return Err(RawError::Levels(format!(
                    "white {white} not above black {max_black}"
                )));
            }
            scale.push(1.0 / range);
        }
        let pattern = (0..rows * cols)
            .flat_map(|i| (0..n).map(move |p| (i, p)))
            .map(|(i, p)| (levels.black[i as usize * n + p] * scale[p]) as f32)
            .collect();
        let per_plane = |delta: &[f64]| -> Vec<Vec<f32>> {
            if delta.is_empty() {
                return Vec::new();
            }
            scale
                .iter()
                .map(|s| delta.iter().map(|d| (d * s) as f32).collect())
                .collect()
        };
        Ok(Linearization {
            scale: scale.iter().map(|&s| s as f32).collect(),
            pattern,
            rows,
            cols,
            delta_h: per_plane(&levels.black_delta_h),
            delta_v: per_plane(&levels.black_delta_v),
        })
    }

    /// The scaled black level at active-area (row, col), plane `p`.
    pub fn black(&self, row: u32, col: u32, p: usize) -> f32 {
        let planes = self.scale.len();
        let i = ((row % self.rows) * self.cols + col % self.cols) as usize * planes + p;
        let mut b = self.pattern[i];
        if let Some(h) = self.delta_h.get(p) {
            b += h[col as usize];
        }
        if let Some(v) = self.delta_v.get(p) {
            b += v[row as usize];
        }
        b
    }
}

impl RawSource {
    /// The decoder's output for stages 1 to 3.
    pub fn from_decoded(img: DecodedImage) -> Result<Self, RawError> {
        let stored = (img.width, img.height);
        let (samples, data, pattern, raw) = match img.pixels {
            Pixels::Cfa { pattern, data, raw } => (1, data, Some(pattern), raw),
            Pixels::Linear {
                samples_per_pixel,
                data,
                raw,
            } => (u32::from(samples_per_pixel), data, None, raw),
            Pixels::Rgba16 { data, color } => {
                return Self::from_encoded(stored, &data, &input_encoding(&color)?);
            }
        };
        if samples != 1 && samples != 3 {
            return Err(RawError::Unsupported(format!(
                "{samples} samples per pixel"
            )));
        }
        let cfa = match pattern {
            Some(p) if p.width == 2 && p.height == 2 => Some(Cfa::Bayer([
                p.colors[0],
                p.colors[1],
                p.colors[2],
                p.colors[3],
            ])),
            Some(p) if p.width == 6 && p.height == 6 => {
                let mut x = [[0u8; 6]; 6];
                for (r, row) in x.iter_mut().enumerate() {
                    row.copy_from_slice(&p.colors[r * 6..r * 6 + 6]);
                }
                Some(Cfa::XTrans(x))
            }
            Some(p) => {
                return Err(RawError::Unsupported(format!(
                    "a {}x{} color filter pattern",
                    p.width, p.height
                )));
            }
            None => None,
        };
        let active = raw.active_area.unwrap_or(Rect {
            top: 0,
            left: 0,
            bottom: stored.1,
            right: stored.0,
        });
        let size = (active.right - active.left, active.bottom - active.top);
        let linearization = Linearization::new(&raw.levels, samples, size)?;
        let color = raw.color.clone();
        let OpcodePlan { list1, list2, .. } = raw.opcode_plan;
        if !matches!(cfa, Some(Cfa::Bayer(_))) && !list1.is_empty() {
            return Err(RawError::Unsupported(
                "FixBadPixelsConstant on an image without a Bayer pattern".into(),
            ));
        }
        // imgio's plan puts each kind only in the list it runs from.
        let fix_bad_pixels = list1
            .into_iter()
            .filter_map(|op| match op {
                StageOpcode::FixBadPixelsConstant(f) => Some(f),
                StageOpcode::GainMap(_) => None,
            })
            .collect();
        let fix_bad_pixels: Vec<FixBadPixelsConstant> = fix_bad_pixels;
        // Each would read the previous one's result; the GPU applies one.
        if fix_bad_pixels.len() > 1 {
            return Err(RawError::Unsupported(format!(
                "{} FixBadPixelsConstant opcodes",
                fix_bad_pixels.len()
            )));
        }
        let gain_maps = list2
            .into_iter()
            .filter_map(|op| match op {
                StageOpcode::GainMap(g) => Some(g),
                StageOpcode::FixBadPixelsConstant(_) => None,
            })
            .collect();
        Ok(RawSource {
            stored,
            samples,
            data,
            cfa,
            active,
            linearization,
            fix_bad_pixels,
            gain_maps,
            color,
            encoding: None,
        })
    }

    /// A non-RAW image: RGBA16 codes row by row, `encoding` their meaning.
    /// Alpha is dropped (DEV-01), so a tile uploads three samples per
    /// pixel, as a linear DNG's does.
    pub fn from_encoded(
        (width, height): (u32, u32),
        rgba: &[u16],
        encoding: &InputEncoding,
    ) -> Result<Self, RawError> {
        let pixels = width as usize * height as usize;
        if width == 0 || height == 0 || rgba.len() != pixels * 4 {
            return Err(RawError::Unsupported(format!(
                "{} RGBA samples for {width}x{height}",
                rgba.len()
            )));
        }
        let data = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        Ok(RawSource {
            stored: (width, height),
            samples: 3,
            data,
            cfa: None,
            active: Rect {
                top: 0,
                left: 0,
                bottom: height,
                right: width,
            },
            // Unused: the curves replace the levels.
            linearization: Linearization {
                scale: vec![1.0; 3],
                pattern: vec![0.0; 3],
                rows: 1,
                cols: 1,
                delta_h: Vec::new(),
                delta_v: Vec::new(),
            },
            fix_bad_pixels: Vec::new(),
            gain_maps: Vec::new(),
            color: CameraColor::default(),
            encoding: Some(encoding.clone()),
        })
    }

    /// The active area's width and height.
    pub fn size(&self) -> (u32, u32) {
        (
            self.active.right - self.active.left,
            self.active.bottom - self.active.top,
        )
    }

    /// The color at active-area (row, col) of a mosaic.
    pub fn color_at(&self, row: u32, col: u32) -> Option<u8> {
        Some(
            self.cfa?
                .color(row + self.active.top, col + self.active.left),
        )
    }
}

/// What a decoder's color encoding means for stage 1 and 4.
pub fn input_encoding(color: &ColorEncoding) -> Result<InputEncoding, EncodingError> {
    match color {
        ColorEncoding::Srgb => Ok(InputEncoding::srgb()),
        ColorEncoding::DisplayP3 => Ok(InputEncoding::display_p3()),
        ColorEncoding::AdobeRgb => Ok(InputEncoding::adobe_rgb()),
        ColorEncoding::Icc(bytes) => InputEncoding::icc(bytes),
        // The matrix and range are libheif's to undo; it already has.
        ColorEncoding::Nclx {
            primaries,
            transfer,
            ..
        } => InputEncoding::nclx(*primaries, *transfer),
    }
}
