//! What a RAW decoder hands the pipeline besides the samples: levels, color
//! data, opcode lists and orientation (DEV-01, stage 0). The same types serve
//! every RAW path, so the pipeline never sees which decoder or file format
//! produced them.

/// The color filter pattern of a CFA sensor: which color each site of the
/// repeating pattern records, with site (0, 0) at the sensor data's first
/// sample (not the active area's). 2 × 2 for Bayer, 6 × 6 for X-Trans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfaPattern {
    pub width: u8,
    pub height: u8,
    /// Row by row: 0 red, 1 green, 2 blue.
    pub colors: Vec<u8>,
}

impl CfaPattern {
    /// The color recorded at sensor row `y`, column `x`.
    pub fn color_at(&self, y: usize, x: usize) -> u8 {
        let (w, h) = (usize::from(self.width), usize::from(self.height));
        self.colors[(y % h) * w + x % w]
    }
}

/// Black and white levels, as the file states them. The pipeline applies
/// them in stage 1; the decoder never does.
#[derive(Debug, Clone, PartialEq)]
pub struct Levels {
    /// Black levels, row by row over a `black_repeat` pattern, each entry
    /// holding one value per sample. Its length is rows × columns × samples.
    /// The pattern, BlackLevelDeltaH and BlackLevelDeltaV start at the
    /// active area's top left (as in DNG), not at the sensor's first sample.
    pub black: Vec<f64>,
    /// Rows and columns of the black level pattern (DNG BlackLevelRepeatDim):
    /// 1 × 1 for one level per channel, 2 × 2 for Bayer, 6 × 6 for X-Trans.
    pub black_repeat: (u16, u16),
    /// Per-column black level offsets (DNG BlackLevelDeltaH), empty if none.
    pub black_delta_h: Vec<f64>,
    /// Per-row black level offsets (DNG BlackLevelDeltaV), empty if none.
    pub black_delta_v: Vec<f64>,
    /// White level, one per sample, or one for all.
    pub white: Vec<u32>,
}

/// A rectangle in sensor pixels, `top` and `left` included, `bottom` and
/// `right` excluded (DNG ActiveArea order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

/// The default crop (DNG DefaultCropOrigin and DefaultCropSize), in pixels
/// relative to the active area's top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefaultCrop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A matrix and the illuminant it was calibrated for (EXIF LightSource).
#[derive(Debug, Clone, PartialEq)]
pub struct ColorMatrix {
    pub illuminant: Option<u16>,
    /// Row-major, 3 columns, one row per sample: ColorMatrix maps XYZ to
    /// camera. A ForwardMatrix is 3 rows with one column per sample and maps
    /// white balanced camera values to XYZ D50.
    pub matrix: Vec<f64>,
}

/// The white balance the camera recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AsShot {
    /// DNG AsShotNeutral: the camera values of a neutral, first three only.
    Neutral([f64; 3]),
    /// DNG AsShotWhiteXY: the white point as a CIE xy chromaticity.
    WhiteXy([f64; 2]),
}

/// The camera's color data, as stored. Stage 4 turns it into a matrix.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CameraColor {
    pub color_matrix_1: Option<ColorMatrix>,
    pub color_matrix_2: Option<ColorMatrix>,
    /// Calibrated for the same illuminants as the color matrices.
    pub forward_matrix_1: Option<Vec<f64>>,
    pub forward_matrix_2: Option<Vec<f64>>,
    /// DNG CameraCalibration1 and 2: per-unit corrections, square, one row
    /// per sample. Absent means identity.
    pub camera_calibration_1: Option<Vec<f64>>,
    pub camera_calibration_2: Option<Vec<f64>>,
    /// DNG CameraCalibrationSignature and ProfileCalibrationSignature, empty
    /// when absent. The camera calibration applies only when they are equal,
    /// two absent ones included (as in the DNG SDK).
    pub camera_calibration_signature: String,
    pub profile_calibration_signature: String,
    /// DNG AnalogBalance, one gain per sample. Absent means 1.
    pub analog_balance: Option<Vec<f64>>,
    pub as_shot: Option<AsShot>,
    /// In stops; 0 when the file states none.
    pub baseline_exposure: f64,
}

/// An opcode's kind (DNG specification 1.7, chapter 7). The values are the
/// opcode IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpcodeKind {
    WarpRectilinear,
    WarpFisheye,
    FixVignetteRadial,
    FixBadPixelsConstant,
    FixBadPixelsList,
    TrimBounds,
    MapTable,
    MapPolynomial,
    GainMap,
    DeltaPerRow,
    DeltaPerColumn,
    ScalePerRow,
    ScalePerColumn,
    WarpRectilinear2,
    Unknown(u32),
}

impl OpcodeKind {
    pub fn from_id(id: u32) -> Self {
        use OpcodeKind::*;
        match id {
            1 => WarpRectilinear,
            2 => WarpFisheye,
            3 => FixVignetteRadial,
            4 => FixBadPixelsConstant,
            5 => FixBadPixelsList,
            6 => TrimBounds,
            7 => MapTable,
            8 => MapPolynomial,
            9 => GainMap,
            10 => DeltaPerRow,
            11 => DeltaPerColumn,
            12 => ScalePerRow,
            13 => ScalePerColumn,
            14 => WarpRectilinear2,
            other => Unknown(other),
        }
    }
}

/// One opcode of an opcode list. Its parameters stay raw bytes (big-endian,
/// whatever the file's byte order) until the stage that applies it parses
/// them.
#[derive(Debug, Clone, PartialEq)]
pub struct Opcode {
    pub kind: OpcodeKind,
    /// The DNG version the opcode was introduced in.
    pub dng_version: [u8; 4],
    /// The opcode may be skipped by a reader that does not implement it.
    pub optional: bool,
    /// The opcode may be skipped for preview-quality renders.
    pub preview_skip: bool,
    pub params: Vec<u8>,
}

/// DNG OpcodeList1, 2 and 3: on the stored values, after linearization, and
/// after the demosaic.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpcodeLists {
    pub list1: Vec<Opcode>,
    pub list2: Vec<Opcode>,
    pub list3: Vec<Opcode>,
}

/// Most opcodes accepted in one list. Real files carry a handful.
const MAX_OPCODES: u32 = 4096;

/// Parses a DNG opcode list. `None` when the bytes do not hold one.
pub fn parse_opcode_list(bytes: &[u8]) -> Option<Vec<Opcode>> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(
            bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    let count = u32_at(0)?;
    if count > MAX_OPCODES {
        return None;
    }
    let mut at = 4;
    let mut list = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = u32_at(at)?;
        let version: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let flags = u32_at(at + 8)?;
        let len = u32_at(at + 12)? as usize;
        let start = at + 16;
        let params = bytes.get(start..start.checked_add(len)?)?;
        list.push(Opcode {
            kind: OpcodeKind::from_id(id),
            dng_version: version,
            optional: flags & 1 != 0,
            preview_skip: flags & 2 != 0,
            params: params.to_vec(),
        });
        at = start + len;
    }
    Some(list)
}

/// Everything a RAW decoder returns besides the samples.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMetadata {
    pub levels: Levels,
    /// The sensor area holding image data; absent means all of it.
    pub active_area: Option<Rect>,
    pub default_crop: Option<DefaultCrop>,
    pub color: CameraColor,
    pub opcodes: OpcodeLists,
    /// What the pipeline applies of `opcodes`, typed, and whether lens
    /// corrections were skipped (`crate::opcodes::plan`). Filled by
    /// `imgio::decode`; empty from the metadata readers.
    pub opcode_plan: crate::opcodes::OpcodePlan,
    /// DNG ProfileGainTableMap (ProRAW's local tone map), raw bytes. Read and
    /// kept, never applied in M1 (decided).
    pub gain_table_map: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opcode(id: u32, flags: u32, params: &[u8]) -> Vec<u8> {
        let mut out = id.to_be_bytes().to_vec();
        out.extend([1, 3, 0, 0]);
        out.extend(flags.to_be_bytes());
        out.extend((params.len() as u32).to_be_bytes());
        out.extend(params);
        out
    }

    #[test]
    fn parses_opcode_lists() {
        let mut bytes = 2u32.to_be_bytes().to_vec();
        bytes.extend(opcode(9, 0, &[1, 2, 3]));
        bytes.extend(opcode(77, 3, &[]));
        let list = parse_opcode_list(&bytes).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].kind, OpcodeKind::GainMap);
        assert_eq!(list[0].dng_version, [1, 3, 0, 0]);
        assert!(!list[0].optional && !list[0].preview_skip);
        assert_eq!(list[0].params, [1, 2, 3]);
        assert_eq!(list[1].kind, OpcodeKind::Unknown(77));
        assert!(list[1].optional && list[1].preview_skip);

        assert_eq!(parse_opcode_list(&0u32.to_be_bytes()), Some(Vec::new()));
    }

    #[test]
    fn rejects_short_or_inconsistent_lists() {
        let mut bytes = 1u32.to_be_bytes().to_vec();
        bytes.extend(opcode(9, 0, &[1, 2, 3]));
        for len in 0..bytes.len() {
            assert_eq!(parse_opcode_list(&bytes[..len]), None, "length {len}");
        }
        assert_eq!(parse_opcode_list(&u32::MAX.to_be_bytes()), None);
        // A parameter size past the end, and one that overflows.
        let mut bytes = 1u32.to_be_bytes().to_vec();
        bytes.extend(opcode(9, 0, &[]));
        bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse_opcode_list(&bytes), None);
    }
}
