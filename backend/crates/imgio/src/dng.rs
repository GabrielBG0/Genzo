//! A DNG reader for linear (demosaiced) raw images: Apple ProRAW and other
//! LinearRaw DNGs. Every ProRAW goes through it, so the LinearizationTable
//! is applied one way, exactly: rawler 0.8 dithers it, panics on samples past
//! its end and wraps the brightest highlights to black (docs/m0-results.md).
//!
//! Tiles may be JPEG XL (compression 52546, DNG 1.7: iPhone 16 Pro on,
//! Samsung Expert RAW), lossless JPEG (compression 7: iPhone 12 to 15 Pro;
//! decoded by rawler's lossless JPEG decompressor, so only with the `raw`
//! feature) or uncompressed. CFA (Bayer) DNGs stay with rawler.
//!
//! The input is untrusted: every offset and size is checked, sizes are capped
//! before anything is allocated, and nothing here panics on bad data.

use rayon::prelude::*;

use jxl_oxide::{AllocTracker, JxlImage};

use crate::raw::{
    AsShot, CameraColor, ColorMatrix, DefaultCrop, Levels, Opcode, OpcodeLists, RawMetadata, Rect,
    parse_opcode_list,
};

/// TIFF compression code for JPEG XL tiles (DNG 1.7).
pub const COMPRESSION_JPEG_XL: u16 = 52546;
const COMPRESSION_NONE: u16 = 1;
/// TIFF compression code for lossless JPEG tiles.
pub const COMPRESSION_LOSSLESS_JPEG: u16 = 7;
const PHOTOMETRIC_LINEAR_RAW: u16 = 34892;
const PHOTOMETRIC_CFA: u16 = 32803;

/// Largest image accepted, in pixels (above any current sensor).
const MAX_PIXELS: u64 = 400_000_000;
/// Largest output accepted, in samples (2 bytes each): room for 133 MP of
/// RGB, against 146M samples for a 48 MP ProRAW. Checked before allocating,
/// since a file of a few hundred bytes can claim any size.
const MAX_SAMPLES: u64 = 400_000_000;
/// Largest tile side accepted.
const MAX_TILE_SIDE: u32 = 8192;
/// Most entries read from one IFD, and most values from one entry.
const MAX_IFD_ENTRIES: usize = 1024;
const MAX_VALUES: usize = 1 << 20;
/// Most SubIFDs searched for the raw image.
const MAX_SUB_IFDS: usize = 16;
/// Memory the JPEG XL decoder may hold at once, over all tiles in flight.
const JXL_MEMORY_LIMIT: usize = 2 << 30;

mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 254;
    pub const ORIENTATION: u16 = 274;
    pub const WIDTH: u16 = 256;
    pub const HEIGHT: u16 = 257;
    pub const BITS_PER_SAMPLE: u16 = 258;
    pub const COMPRESSION: u16 = 259;
    pub const PHOTOMETRIC: u16 = 262;
    pub const STRIP_OFFSETS: u16 = 273;
    pub const ROWS_PER_STRIP: u16 = 278;
    pub const STRIP_BYTE_COUNTS: u16 = 279;
    pub const SAMPLES_PER_PIXEL: u16 = 277;
    pub const PLANAR_CONFIGURATION: u16 = 284;
    pub const TILE_WIDTH: u16 = 322;
    pub const TILE_LENGTH: u16 = 323;
    pub const TILE_OFFSETS: u16 = 324;
    pub const TILE_BYTE_COUNTS: u16 = 325;
    pub const SUB_IFDS: u16 = 330;
    pub const DNG_VERSION: u16 = 50706;
    pub const LINEARIZATION_TABLE: u16 = 50712;
    pub const BLACK_LEVEL_REPEAT_DIM: u16 = 50713;
    pub const BLACK_LEVEL: u16 = 50714;
    pub const BLACK_LEVEL_DELTA_H: u16 = 50715;
    pub const BLACK_LEVEL_DELTA_V: u16 = 50716;
    pub const WHITE_LEVEL: u16 = 50717;
    pub const DEFAULT_CROP_ORIGIN: u16 = 50719;
    pub const DEFAULT_CROP_SIZE: u16 = 50720;
    pub const COLOR_MATRIX_1: u16 = 50721;
    pub const COLOR_MATRIX_2: u16 = 50722;
    pub const CAMERA_CALIBRATION_1: u16 = 50723;
    pub const CAMERA_CALIBRATION_2: u16 = 50724;
    pub const ANALOG_BALANCE: u16 = 50727;
    pub const AS_SHOT_NEUTRAL: u16 = 50728;
    pub const AS_SHOT_WHITE_XY: u16 = 50729;
    pub const BASELINE_EXPOSURE: u16 = 50730;
    pub const CALIBRATION_ILLUMINANT_1: u16 = 50778;
    pub const CALIBRATION_ILLUMINANT_2: u16 = 50779;
    pub const ACTIVE_AREA: u16 = 50829;
    pub const FORWARD_MATRIX_1: u16 = 50964;
    pub const FORWARD_MATRIX_2: u16 = 50965;
    pub const OPCODE_LIST_1: u16 = 51008;
    pub const OPCODE_LIST_2: u16 = 51009;
    pub const OPCODE_LIST_3: u16 = 51022;
    pub const PROFILE_GAIN_TABLE_MAP: u16 = 52525;
}

#[derive(Debug, thiserror::Error)]
pub enum DngError {
    #[error("not a TIFF or DNG file")]
    NotTiff,
    #[error("no full-size raw image (NewSubFileType 0) in IFD0 or its SubIFDs")]
    NoRawImage,
    #[error("not supported by the linear DNG reader: {0}")]
    Unsupported(String),
    #[error("{0} (needs the `{1}` feature)")]
    NotBuiltIn(&'static str, &'static str),
    #[error("image too large: {0}")]
    TooLarge(String),
    #[error("corrupt DNG: {0}")]
    Corrupt(String),
}

fn corrupt(message: impl Into<String>) -> DngError {
    DngError::Corrupt(message.into())
}

/// A decoded linear DNG and the metadata the pipeline needs from it.
#[derive(Debug, Clone)]
pub struct LinearDng {
    pub width: u32,
    pub height: u32,
    pub samples_per_pixel: u8,
    /// BitsPerSample of the stored samples, as the DNG declares it.
    pub bits_per_sample: u16,
    pub compression: u16,
    /// Interleaved samples, row by row, after the LinearizationTable and
    /// before BlackLevel and WhiteLevel are applied.
    pub pixels: Vec<u16>,
    /// Stored samples past the end of the LinearizationTable, which were
    /// given its last entry. The DNG spec has no rule for them.
    pub clamped: u64,
    pub linearization_entries: usize,
    /// The EXIF orientation, 1 to 8 (1 when absent or invalid). Not applied.
    pub orientation: u8,
    pub meta: RawMetadata,
}

/// Whether a TIFF file is a DNG (IFD0 holds a DNGVersion tag).
pub fn is_dng(data: &[u8]) -> bool {
    let Some(tiff) = Tiff::new(data) else {
        return false;
    };
    let Some(at) = tiff.u32(4) else {
        return false;
    };
    tiff.ifd(at as usize)
        .is_ok_and(|ifd0| find(&ifd0, tag::DNG_VERSION).is_some())
}

/// IFD0 and the IFD of the full-size raw image (NewSubFileType 0), which is
/// IFD0 itself or one of its SubIFDs.
fn raw_ifd(tiff: &Tiff) -> Result<(Vec<Entry>, Vec<Entry>), DngError> {
    let ifd0_offset = tiff.u32(4).ok_or(DngError::NotTiff)?;
    let ifd0 = tiff.ifd(ifd0_offset as usize)?;
    let mut candidates = vec![ifd0.clone()];
    if let Some(e) = find(&ifd0, tag::SUB_IFDS) {
        for offset in tiff.ints(e)?.into_iter().take(MAX_SUB_IFDS) {
            candidates.push(tiff.ifd(offset as usize)?);
        }
    }
    let raw = candidates
        .into_iter()
        .find(|ifd| tiff.first_int(ifd, tag::NEW_SUBFILE_TYPE).unwrap_or(0) == 0)
        .ok_or(DngError::NoRawImage)?;
    Ok((ifd0, raw))
}

/// Reads the metadata of any DNG, linear or CFA, without decoding its
/// pixels. What the inventory of corpus files and the CFA path use.
pub fn read_metadata(data: &[u8]) -> Result<(RawMetadata, u8), DngError> {
    let tiff = Tiff::new(data).ok_or(DngError::NotTiff)?;
    let (ifd0, raw) = raw_ifd(&tiff)?;
    let spp = tiff.first_int(&raw, tag::SAMPLES_PER_PIXEL).unwrap_or(1);
    // Checked before `metadata` sizes the level arrays by it.
    if !(1..=4).contains(&spp) {
        return Err(DngError::Unsupported(format!("{spp} samples per pixel")));
    }
    let bits = tiff
        .first_int(&raw, tag::BITS_PER_SAMPLE)
        .and_then(|b| u16::try_from(b).ok())
        .filter(|b| (1..=16).contains(b))
        .ok_or_else(|| corrupt("BitsPerSample"))?;
    let width = tiff.required_int(&raw, tag::WIDTH)?;
    let height = tiff.required_int(&raw, tag::HEIGHT)?;
    Ok((
        metadata(&tiff, &ifd0, &raw, spp, bits, (width, height))?,
        orientation(&tiff, &ifd0),
    ))
}

fn orientation(tiff: &Tiff, ifd0: &[Entry]) -> u8 {
    match tiff.first_int(ifd0, tag::ORIENTATION) {
        Some(v @ 1..=8) => v as u8,
        _ => 1,
    }
}

/// Largest black level pattern side accepted (6 for X-Trans).
const MAX_BLACK_REPEAT: u32 = 16;

/// The stage 0 metadata. Tags are looked up in the raw IFD first, then in
/// IFD0, since writers differ on where they put them.
fn metadata(
    tiff: &Tiff,
    ifd0: &[Entry],
    raw: &[Entry],
    spp: u32,
    bits: u16,
    (width, height): (u32, u32),
) -> Result<RawMetadata, DngError> {
    let get = |t| find(raw, t).or_else(|| find(ifd0, t));
    let numbers = |t| get(t).map(|e| tiff.numbers(e)).transpose();
    let spp_len = spp as usize;

    let (rows, cols) = match get(tag::BLACK_LEVEL_REPEAT_DIM) {
        Some(e) => match tiff.ints(e)?[..] {
            [r, c]
                if (1..=MAX_BLACK_REPEAT).contains(&r) && (1..=MAX_BLACK_REPEAT).contains(&c) =>
            {
                (r, c)
            }
            ref other => {
                return Err(DngError::Unsupported(format!(
                    "a black level pattern of {other:?}"
                )));
            }
        },
        None => (1, 1),
    };
    let pattern = (rows * cols) as usize * spp_len;
    let black = match numbers(tag::BLACK_LEVEL)? {
        None => vec![0.0; pattern],
        Some(v) if v.len() == pattern => v,
        Some(v) if v.len() == 1 => vec![v[0]; pattern],
        Some(v) => {
            return Err(corrupt(format!(
                "{} black levels for a {rows}x{cols} pattern of {spp} samples",
                v.len()
            )));
        }
    };
    let white = match get(tag::WHITE_LEVEL).map(|e| tiff.ints(e)).transpose()? {
        None => vec![(1u32 << bits) - 1; spp_len],
        Some(v) if v.len() == spp_len => v,
        Some(v) if v.len() == 1 => vec![v[0]; spp_len],
        Some(v) => {
            return Err(corrupt(format!(
                "{} white levels for {spp} samples",
                v.len()
            )));
        }
    };

    let active_area = match get(tag::ACTIVE_AREA).map(|e| tiff.ints(e)).transpose()? {
        None => None,
        Some(v) => match v[..] {
            [top, left, bottom, right]
                if top < bottom && left < right && bottom <= height && right <= width =>
            {
                Some(Rect {
                    top,
                    left,
                    bottom,
                    right,
                })
            }
            _ => return Err(corrupt(format!("ActiveArea {v:?} in {width}x{height}"))),
        },
    };
    let default_crop = match (
        numbers(tag::DEFAULT_CROP_ORIGIN)?.as_deref(),
        numbers(tag::DEFAULT_CROP_SIZE)?.as_deref(),
    ) {
        (Some(&[x, y]), Some(&[w, h])) => Some(DefaultCrop {
            x,
            y,
            width: w,
            height: h,
        }),
        (None, Some(&[w, h])) => Some(DefaultCrop {
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        }),
        _ => None,
    };

    let matrix = |m, i| -> Result<Option<ColorMatrix>, DngError> {
        Ok(match get(m) {
            Some(e) => Some(ColorMatrix {
                illuminant: tiff.first_int(ifd0, i).and_then(|v| u16::try_from(v).ok()),
                matrix: tiff.numbers(e)?,
            }),
            None => None,
        })
    };
    let neutral = numbers(tag::AS_SHOT_NEUTRAL)?
        .and_then(|v| <[f64; 3]>::try_from(v.get(..3)?).ok())
        .filter(|n| n.iter().all(|c| c.is_finite() && *c > 0.0));
    let white_xy = numbers(tag::AS_SHOT_WHITE_XY)?
        .and_then(|v| <[f64; 2]>::try_from(&v[..]).ok())
        .filter(|xy| xy.iter().all(|c| c.is_finite() && *c > 0.0));
    let color = CameraColor {
        color_matrix_1: matrix(tag::COLOR_MATRIX_1, tag::CALIBRATION_ILLUMINANT_1)?,
        color_matrix_2: matrix(tag::COLOR_MATRIX_2, tag::CALIBRATION_ILLUMINANT_2)?,
        forward_matrix_1: numbers(tag::FORWARD_MATRIX_1)?,
        forward_matrix_2: numbers(tag::FORWARD_MATRIX_2)?,
        camera_calibration_1: numbers(tag::CAMERA_CALIBRATION_1)?,
        camera_calibration_2: numbers(tag::CAMERA_CALIBRATION_2)?,
        analog_balance: numbers(tag::ANALOG_BALANCE)?,
        as_shot: neutral
            .map(AsShot::Neutral)
            .or(white_xy.map(AsShot::WhiteXy)),
        baseline_exposure: numbers(tag::BASELINE_EXPOSURE)?
            .and_then(|v| v.first().copied())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0),
    };

    let list = |t| -> Result<Vec<Opcode>, DngError> {
        match get(t) {
            Some(e) => parse_opcode_list(&tiff.blob(e)?)
                .ok_or_else(|| corrupt(format!("opcode list in tag {t}"))),
            None => Ok(Vec::new()),
        }
    };
    Ok(RawMetadata {
        levels: Levels {
            black,
            black_repeat: (rows as u16, cols as u16),
            black_delta_h: numbers(tag::BLACK_LEVEL_DELTA_H)?.unwrap_or_default(),
            black_delta_v: numbers(tag::BLACK_LEVEL_DELTA_V)?.unwrap_or_default(),
            white,
        },
        active_area,
        default_crop,
        color,
        opcodes: OpcodeLists {
            list1: list(tag::OPCODE_LIST_1)?,
            list2: list(tag::OPCODE_LIST_2)?,
            list3: list(tag::OPCODE_LIST_3)?,
        },
        gain_table_map: get(tag::PROFILE_GAIN_TABLE_MAP)
            .map(|e| tiff.blob(e))
            .transpose()?,
    })
}

/// Reads the full-size linear raw image of a DNG.
pub fn decode_linear(data: &[u8]) -> Result<LinearDng, DngError> {
    let tiff = Tiff::new(data).ok_or(DngError::NotTiff)?;
    let (ifd0, raw) = raw_ifd(&tiff)?;

    let photometric = tiff.required_int(&raw, tag::PHOTOMETRIC)?;
    match u16::try_from(photometric) {
        Ok(PHOTOMETRIC_LINEAR_RAW) => {}
        Ok(PHOTOMETRIC_CFA) => {
            return Err(DngError::Unsupported(
                "a CFA (Bayer) image; the raw decoder reads those".into(),
            ));
        }
        _ => {
            return Err(DngError::Unsupported(format!(
                "photometric interpretation {photometric}"
            )));
        }
    }
    let compression = tiff.first_int(&raw, tag::COMPRESSION).unwrap_or(1);
    let compression = u16::try_from(compression).unwrap_or(0);
    match compression {
        COMPRESSION_JPEG_XL | COMPRESSION_NONE => {}
        COMPRESSION_LOSSLESS_JPEG if cfg!(feature = "raw") => {}
        COMPRESSION_LOSSLESS_JPEG => {
            return Err(DngError::NotBuiltIn(
                "lossless JPEG tiles (compression 7)",
                "raw",
            ));
        }
        _ => {
            return Err(DngError::Unsupported(format!("compression {compression}")));
        }
    }
    if tiff.first_int(&raw, tag::PLANAR_CONFIGURATION).unwrap_or(1) != 1 {
        return Err(DngError::Unsupported("planar sample layout".into()));
    }
    let width = tiff.required_int(&raw, tag::WIDTH)?;
    let height = tiff.required_int(&raw, tag::HEIGHT)?;
    let spp = tiff.first_int(&raw, tag::SAMPLES_PER_PIXEL).unwrap_or(1);
    // Strips (Samsung) are read as tiles the width of the image.
    let strips = find(&raw, tag::TILE_OFFSETS).is_none();
    let (offsets_tag, counts_tag, tile_w, tile_h) = if strips {
        if find(&raw, tag::STRIP_OFFSETS).is_none() {
            return Err(corrupt("no tile or strip offsets"));
        }
        let rows = tiff.first_int(&raw, tag::ROWS_PER_STRIP).unwrap_or(height);
        let rows = rows.min(height);
        (tag::STRIP_OFFSETS, tag::STRIP_BYTE_COUNTS, width, rows)
    } else {
        (
            tag::TILE_OFFSETS,
            tag::TILE_BYTE_COUNTS,
            tiff.required_int(&raw, tag::TILE_WIDTH)?,
            tiff.required_int(&raw, tag::TILE_LENGTH)?,
        )
    };
    if !(1..=4).contains(&spp) {
        return Err(DngError::Unsupported(format!("{spp} samples per pixel")));
    }
    if width == 0 || height == 0 || tile_w == 0 || tile_h == 0 {
        return Err(corrupt("zero image or tile size"));
    }
    if tile_w > MAX_TILE_SIDE || tile_h > MAX_TILE_SIDE {
        return Err(DngError::TooLarge(format!("tiles of {tile_w}x{tile_h}")));
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS || pixels * u64::from(spp) > MAX_SAMPLES {
        return Err(DngError::TooLarge(format!("{width}x{height}x{spp}")));
    }
    let bits = tiff
        .ints(find(&raw, tag::BITS_PER_SAMPLE).ok_or_else(|| corrupt("no BitsPerSample"))?)?
        .first()
        .copied()
        .unwrap_or(0);
    let bits = u16::try_from(bits).map_err(|_| corrupt("BitsPerSample"))?;
    if !(1..=16).contains(&bits) {
        return Err(DngError::Unsupported(format!("{bits} bits per sample")));
    }

    let tiles_across = width.div_ceil(tile_w);
    let tiles_down = height.div_ceil(tile_h);
    let offsets = tiff.ints(find(&raw, offsets_tag).ok_or_else(|| corrupt("no offsets"))?)?;
    let counts = tiff.ints(find(&raw, counts_tag).ok_or_else(|| corrupt("no byte counts"))?)?;
    let tile_count = (tiles_across as usize) * (tiles_down as usize);
    if offsets.len() != tile_count || counts.len() != tile_count {
        return Err(corrupt(format!(
            "{} tile offsets and {} byte counts for {tiles_across}x{tiles_down} tiles",
            offsets.len(),
            counts.len()
        )));
    }
    let tiles: Vec<&[u8]> = offsets
        .iter()
        .zip(&counts)
        .map(|(&o, &n)| {
            let (o, n) = (o as usize, n as usize);
            o.checked_add(n)
                .and_then(|end| data.get(o..end))
                .ok_or_else(|| corrupt("a tile lies outside the file"))
        })
        .collect::<Result<_, _>>()?;
    // Checked before the output is allocated: tiles that cannot hold their
    // pixels fail here, not after a large allocation.
    let row_bytes = tile_w as usize * spp as usize * 2;
    for (i, tile) in tiles.iter().enumerate() {
        if tile.is_empty() {
            return Err(corrupt("an empty tile"));
        }
        let y0 = (i / tiles_across as usize) * tile_h as usize;
        // The last strip holds only the rows left; tiles are always full.
        let rows = if strips {
            (tile_h as usize).min(height as usize - y0)
        } else {
            tile_h as usize
        };
        if compression == COMPRESSION_NONE && tile.len() < row_bytes * rows {
            return Err(corrupt("an uncompressed tile is too short"));
        }
    }

    let table: Vec<u16> = match find(&raw, tag::LINEARIZATION_TABLE) {
        Some(e) => tiff
            .ints(e)?
            .into_iter()
            .map(|v| u16::try_from(v).unwrap_or(u16::MAX))
            .collect(),
        None => Vec::new(),
    };

    let layout = Layout {
        width: width as usize,
        height: height as usize,
        spp: spp as usize,
        tile_w: tile_w as usize,
        tile_h: tile_h as usize,
        tiles_across: tiles_across as usize,
        bits,
        big_endian: tiff.big_endian,
        strips,
    };
    let meta = metadata(&tiff, &ifd0, &raw, spp, bits, (width, height))?;

    let len = layout.width * layout.height * layout.spp;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| DngError::TooLarge(format!("no memory for {len} samples")))?;
    pixels.resize(len, 0u16);
    let tracker = AllocTracker::with_limit(JXL_MEMORY_LIMIT);
    // One band per row of tiles, decoded in parallel; each band owns its rows
    // of the output.
    let band_len = layout.tile_h * layout.width * layout.spp;
    let clamped = pixels
        .par_chunks_mut(band_len)
        .enumerate()
        .map(|(band, out)| {
            let mut clamped = 0;
            for col in 0..layout.tiles_across {
                let bytes = tiles[band * layout.tiles_across + col];
                let tile = Placement {
                    x0: col * layout.tile_w,
                    y0: band * layout.tile_h,
                };
                clamped += match compression {
                    COMPRESSION_JPEG_XL => jxl_tile(bytes, &layout, tile, out, &table, &tracker)?,
                    #[cfg(feature = "raw")]
                    COMPRESSION_LOSSLESS_JPEG => {
                        lossless_jpeg_tile(bytes, &layout, tile, out, &table)?
                    }
                    _ => uncompressed_tile(bytes, &layout, tile, out, &table)?,
                };
            }
            Ok(clamped)
        })
        .collect::<Result<Vec<u64>, DngError>>()?
        .into_iter()
        .sum();

    Ok(LinearDng {
        width,
        height,
        samples_per_pixel: spp as u8,
        bits_per_sample: bits,
        compression,
        pixels,
        clamped,
        linearization_entries: table.len(),
        orientation: orientation(&tiff, &ifd0),
        meta,
    })
}

struct Layout {
    width: usize,
    height: usize,
    spp: usize,
    tile_w: usize,
    tile_h: usize,
    tiles_across: usize,
    bits: u16,
    big_endian: bool,
    /// Strips, not tiles: the last one holds only the rows left, unpadded.
    strips: bool,
}

#[derive(Clone, Copy)]
struct Placement {
    x0: usize,
    y0: usize,
}

impl Layout {
    /// Rows a tile or strip stores: the tile height, or for a strip the
    /// rows that fall inside the image.
    fn stored_rows(&self, y0: usize) -> usize {
        if self.strips {
            self.tile_h.min(self.height - y0)
        } else {
            self.tile_h
        }
    }

    /// Columns and rows of a tile that fall inside the image.
    fn visible(&self, tile: Placement) -> (usize, usize) {
        (
            self.tile_w.min(self.width - tile.x0),
            self.tile_h.min(self.height - tile.y0),
        )
    }
}

/// Maps a stored sample through the LinearizationTable. Returns the value
/// and whether it was past the end of the table.
#[inline]
fn linearize(v: u16, table: &[u16]) -> (u16, bool) {
    match table.last() {
        None => (v, false),
        Some(&last) => match table.get(usize::from(v)) {
            Some(&t) => (t, false),
            None => (last, true),
        },
    }
}

/// Decodes one JPEG XL tile into its band. Returns the clamped sample count.
fn jxl_tile(
    bytes: &[u8],
    layout: &Layout,
    tile: Placement,
    band: &mut [u16],
    table: &[u16],
    tracker: &AllocTracker,
) -> Result<u64, DngError> {
    let image = JxlImage::builder()
        .alloc_tracker(tracker.clone())
        .read(bytes)
        .map_err(|e| corrupt(format!("JPEG XL tile: {e}")))?;
    let (cols, rows) = layout.visible(tile);
    let (jw, jh) = (image.width() as usize, image.height() as usize);
    // Checked before rendering: a small tile must not claim a huge image.
    // Edge tiles may be stored cropped to the image or padded to full size.
    if jw > layout.tile_w || jh > layout.tile_h || jw < cols || jh < rows {
        return Err(corrupt(format!(
            "a {jw}x{jh} JPEG XL tile in a {}x{} tile grid",
            layout.tile_w, layout.tile_h
        )));
    }
    // jxl-oxide hands out samples scaled to 0..1 by the declared bit depth;
    // scale them back to the integers the DNG stored.
    let scale = match image.image_header().metadata.bit_depth {
        jxl_oxide::image::BitDepth::IntegerSample { bits_per_sample } if bits_per_sample <= 16 => {
            ((1u32 << bits_per_sample) - 1) as f32
        }
        other => {
            return Err(DngError::Unsupported(format!(
                "JPEG XL samples of {other:?}"
            )));
        }
    };
    let render = image
        .render_frame(0)
        .map_err(|e| corrupt(format!("JPEG XL tile: {e}")))?;
    let mut stream = render.stream_no_alpha();
    if stream.channels() as usize != layout.spp {
        return Err(corrupt(format!(
            "a JPEG XL tile with {} channels in a {}-sample image",
            stream.channels(),
            layout.spp
        )));
    }
    let spp = layout.spp;
    let mut row = vec![0f32; jw * spp];
    let mut clamped = 0;
    for y in 0..rows {
        if stream.write_to_buffer(&mut row) != row.len() {
            return Err(corrupt("a JPEG XL tile ended early"));
        }
        let out = &mut band[(y * layout.width + tile.x0) * spp..][..cols * spp];
        for (o, &v) in out.iter_mut().zip(&row) {
            let stored = (v * scale).round().clamp(0.0, 65535.0) as u16;
            let (value, past_end) = linearize(stored, table);
            *o = value;
            clamped += u64::from(past_end);
        }
    }
    Ok(clamped)
}

/// Decodes one lossless JPEG tile into its band with rawler's decompressor.
/// Returns the clamped sample count.
#[cfg(feature = "raw")]
fn lossless_jpeg_tile(
    bytes: &[u8],
    layout: &Layout,
    tile: Placement,
    band: &mut [u16],
    table: &[u16],
) -> Result<u64, DngError> {
    let row_len = layout.tile_w * layout.spp;
    let stored_rows = layout.stored_rows(tile.y0);
    let tile_samples = row_len * stored_rows;
    // The stream is read as a flat run of samples, as TIFF lays out a tile.
    // It must cover the tile, with at most a few rows to spare.
    let decoded = crate::ljpeg::decode(bytes, tile_samples + 4 * row_len).map_err(|e| match e {
        crate::ljpeg::Error::Unsupported(m) => DngError::Unsupported(m),
        crate::ljpeg::Error::Corrupt(m) => corrupt(format!("lossless JPEG tile: {m}")),
    })?;
    if decoded.data.len() < tile_samples {
        return Err(corrupt(format!(
            "lossless JPEG tile: a {}x{} stream for a {row_len}x{stored_rows} tile",
            decoded.width, decoded.height
        )));
    }
    let decoded = decoded.data;

    let (cols, rows) = layout.visible(tile);
    let spp = layout.spp;
    let mut clamped = 0;
    for y in 0..rows {
        let src = &decoded[y * row_len..][..cols * spp];
        let out = &mut band[(y * layout.width + tile.x0) * spp..][..cols * spp];
        for (o, &stored) in out.iter_mut().zip(src) {
            let (value, past_end) = linearize(stored, table);
            *o = value;
            clamped += u64::from(past_end);
        }
    }
    Ok(clamped)
}

/// Copies one uncompressed 16-bit tile into its band.
fn uncompressed_tile(
    bytes: &[u8],
    layout: &Layout,
    tile: Placement,
    band: &mut [u16],
    table: &[u16],
) -> Result<u64, DngError> {
    if layout.bits != 16 {
        return Err(DngError::Unsupported(format!(
            "uncompressed {}-bit samples",
            layout.bits
        )));
    }
    let row_bytes = layout.tile_w * layout.spp * 2;
    if bytes.len() < row_bytes * layout.stored_rows(tile.y0) {
        return Err(corrupt("an uncompressed tile is too short"));
    }
    let (cols, rows) = layout.visible(tile);
    let spp = layout.spp;
    let mut clamped = 0;
    for y in 0..rows {
        let src = &bytes[y * row_bytes..][..cols * spp * 2];
        let out = &mut band[(y * layout.width + tile.x0) * spp..][..cols * spp];
        let (pairs, _) = src.as_chunks::<2>();
        for (o, &b) in out.iter_mut().zip(pairs) {
            let stored = if layout.big_endian {
                u16::from_be_bytes(b)
            } else {
                u16::from_le_bytes(b)
            };
            let (value, past_end) = linearize(stored, table);
            *o = value;
            clamped += u64::from(past_end);
        }
    }
    Ok(clamped)
}

/// One IFD entry: its type, count and the 4 bytes that hold the value or
/// its offset.
#[derive(Debug, Clone, Copy)]
struct Entry {
    tag: u16,
    ty: u16,
    count: u32,
    value: [u8; 4],
}

fn find(ifd: &[Entry], tag: u16) -> Option<Entry> {
    ifd.iter().find(|e| e.tag == tag).copied()
}

/// A bounds-checked view of a TIFF file.
struct Tiff<'a> {
    data: &'a [u8],
    big_endian: bool,
}

impl<'a> Tiff<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        let big_endian = match data.get(..4)? {
            b"II*\0" => false,
            b"MM\0*" => true,
            _ => return None,
        };
        Some(Self { data, big_endian })
    }

    fn bytes<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.data.get(at..at.checked_add(N)?)?.try_into().ok()
    }

    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.bytes::<2>(at)?;
        Some(if self.big_endian {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.bytes::<4>(at)?;
        Some(if self.big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    fn ifd(&self, at: usize) -> Result<Vec<Entry>, DngError> {
        let bad = || corrupt(format!("IFD at {at} lies outside the file"));
        let n = usize::from(self.u16(at).ok_or_else(bad)?);
        if n > MAX_IFD_ENTRIES {
            return Err(corrupt(format!("IFD at {at} has {n} entries")));
        }
        (0..n)
            .map(|i| {
                let e = at + 2 + i * 12;
                Ok(Entry {
                    tag: self.u16(e).ok_or_else(bad)?,
                    ty: self.u16(e + 2).ok_or_else(bad)?,
                    count: self.u32(e + 4).ok_or_else(bad)?,
                    value: self.bytes::<4>(e + 8).ok_or_else(bad)?,
                })
            })
            .collect()
    }

    /// An entry's values as raw bytes, for binary tags (opcode lists, the
    /// gain table map). Bounded by the file's size, not by `MAX_VALUES`.
    fn blob(&self, e: Entry) -> Result<Vec<u8>, DngError> {
        if !matches!(e.ty, 1 | 7) {
            return Err(corrupt(format!("tag {} is not binary", e.tag)));
        }
        let count = e.count as usize;
        if count <= 4 {
            return Ok(e.value[..count].to_vec());
        }
        let offset = if self.big_endian {
            u32::from_be_bytes(e.value)
        } else {
            u32::from_le_bytes(e.value)
        } as usize;
        offset
            .checked_add(count)
            .and_then(|end| self.data.get(offset..end))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| corrupt(format!("tag {} lies outside the file", e.tag)))
    }

    /// Where an entry's values start, and their total size in bytes.
    fn location(&self, e: Entry) -> Result<(usize, usize), DngError> {
        let size = match e.ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            other => return Err(corrupt(format!("tag {} has type {other}", e.tag))),
        };
        let count = e.count as usize;
        if count > MAX_VALUES {
            return Err(corrupt(format!("tag {} has {count} values", e.tag)));
        }
        let total = size * count;
        if total <= 4 {
            return Ok((usize::MAX, total));
        }
        let offset = if self.big_endian {
            u32::from_be_bytes(e.value)
        } else {
            u32::from_le_bytes(e.value)
        } as usize;
        match offset.checked_add(total) {
            Some(end) if end <= self.data.len() => Ok((offset, total)),
            _ => Err(corrupt(format!("tag {} lies outside the file", e.tag))),
        }
    }

    /// Reads value `i` of an entry as raw bytes of `N`, inline or not.
    fn raw<const N: usize>(&self, e: Entry, base: usize, i: usize) -> Option<[u8; N]> {
        if base == usize::MAX {
            e.value.get(i * N..i * N + N)?.try_into().ok()
        } else {
            self.bytes::<N>(base + i * N)
        }
    }

    fn uint<const N: usize>(&self, b: [u8; N]) -> u64 {
        let mut v = 0u64;
        for i in 0..N {
            let byte = if self.big_endian { b[i] } else { b[N - 1 - i] };
            v = (v << 8) | u64::from(byte);
        }
        v
    }

    /// All values of an unsigned integer entry (BYTE, SHORT or LONG).
    fn ints(&self, e: Entry) -> Result<Vec<u32>, DngError> {
        let (base, _) = self.location(e)?;
        let bad = || corrupt(format!("tag {}", e.tag));
        (0..e.count as usize)
            .map(|i| {
                Ok(match e.ty {
                    1 | 7 => self.uint(self.raw::<1>(e, base, i).ok_or_else(bad)?) as u32,
                    3 => self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as u32,
                    4 => self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32,
                    _ => return Err(corrupt(format!("tag {} is not an integer", e.tag))),
                })
            })
            .collect()
    }

    /// All values of a numeric entry, rationals included, as f64.
    fn numbers(&self, e: Entry) -> Result<Vec<f64>, DngError> {
        let (base, _) = self.location(e)?;
        let bad = || corrupt(format!("tag {}", e.tag));
        (0..e.count as usize)
            .map(|i| {
                Ok(match e.ty {
                    1 | 7 => self.uint(self.raw::<1>(e, base, i).ok_or_else(bad)?) as f64,
                    3 => self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as f64,
                    4 => self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as f64,
                    8 => f64::from(
                        self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as u16 as i16
                    ),
                    9 => f64::from(
                        self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32 as i32
                    ),
                    11 => f64::from(f32::from_bits(
                        self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32,
                    )),
                    12 => f64::from_bits(self.uint(self.raw::<8>(e, base, i).ok_or_else(bad)?)),
                    5 | 10 => {
                        let b = self.raw::<8>(e, base, i).ok_or_else(bad)?;
                        let num = self.uint::<4>([b[0], b[1], b[2], b[3]]) as u32;
                        let den = self.uint::<4>([b[4], b[5], b[6], b[7]]) as u32;
                        let (num, den) = if e.ty == 10 {
                            (f64::from(num as i32), f64::from(den as i32))
                        } else {
                            (f64::from(num), f64::from(den))
                        };
                        if den == 0.0 { 0.0 } else { num / den }
                    }
                    _ => return Err(corrupt(format!("tag {} is not a number", e.tag))),
                })
            })
            .collect()
    }

    fn first_int(&self, ifd: &[Entry], tag: u16) -> Option<u32> {
        self.ints(find(ifd, tag)?).ok()?.first().copied()
    }

    fn required_int(&self, ifd: &[Entry], tag: u16) -> Result<u32, DngError> {
        self.first_int(ifd, tag)
            .ok_or_else(|| corrupt(format!("missing or unreadable tag {tag}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 40;
    const H: u32 = 27;
    const T: u32 = 16;

    /// The synthetic image of tests/fixtures/dng/make-tiles.py.
    fn sample(x: u32, y: u32, c: u32) -> u16 {
        ((37 * x + 91 * y + 13 * c) % 1100) as u16
    }

    fn fixture(name: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dng");
        std::fs::read(format!("{dir}/{name}")).unwrap()
    }

    /// Tiles `<prefix>_<i>.<ext>` of tests/fixtures/dng.
    fn fixture_tiles(prefix: &str, ext: &str) -> Vec<Vec<u8>> {
        (0..6)
            .map(|i| fixture(&format!("{prefix}_{i}.{ext}")))
            .collect()
    }

    fn jxl_tiles(bits: u8) -> Vec<Vec<u8>> {
        fixture_tiles(&format!("t{bits}"), "jxl")
    }

    /// Full-size uncompressed 16-bit tiles of the same image.
    fn raw_tiles(be: bool) -> Vec<Vec<u8>> {
        (0..6)
            .map(|i| {
                let (x0, y0) = ((i % 3) * T, (i / 3) * T);
                let mut out = Vec::new();
                for y in 0..T {
                    for x in 0..T {
                        for c in 0..3 {
                            let v = sample(x0 + x, y0 + y, c);
                            out.extend(if be { v.to_be_bytes() } else { v.to_le_bytes() });
                        }
                    }
                }
                out
            })
            .collect()
    }

    enum Val {
        Short(Vec<u16>),
        Long(Vec<u32>),
        Rational(Vec<(u32, u32)>),
        SRational(Vec<(i32, i32)>),
        Undefined(Vec<u8>),
        Byte(Vec<u8>),
    }

    /// A minimal TIFF writer for building test DNGs.
    struct Writer {
        be: bool,
        buf: Vec<u8>,
    }

    impl Writer {
        fn new(be: bool) -> Self {
            let mut buf = if be {
                b"MM\0*".to_vec()
            } else {
                b"II*\0".to_vec()
            };
            buf.extend([0; 4]);
            Self { be, buf }
        }
        fn u16(&self, v: u16) -> [u8; 2] {
            if self.be {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }
        fn u32(&self, v: u32) -> [u8; 4] {
            if self.be {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }
        fn put(&mut self, bytes: &[u8]) -> u32 {
            if self.buf.len() % 2 == 1 {
                self.buf.push(0);
            }
            let at = self.buf.len() as u32;
            self.buf.extend_from_slice(bytes);
            at
        }
        fn ifd(&mut self, mut entries: Vec<(u16, Val)>) -> u32 {
            entries.sort_by_key(|(t, _)| *t);
            let mut encoded = Vec::new();
            for (tag, val) in entries {
                let (ty, count, bytes): (u16, usize, Vec<u8>) = match val {
                    Val::Short(v) => (3, v.len(), v.iter().flat_map(|&x| self.u16(x)).collect()),
                    Val::Long(v) => (4, v.len(), v.iter().flat_map(|&x| self.u32(x)).collect()),
                    Val::Rational(v) => (
                        5,
                        v.len(),
                        v.iter()
                            .flat_map(|&(n, d)| [self.u32(n), self.u32(d)].concat())
                            .collect(),
                    ),
                    Val::SRational(v) => (
                        10,
                        v.len(),
                        v.iter()
                            .flat_map(|&(n, d)| [self.u32(n as u32), self.u32(d as u32)].concat())
                            .collect(),
                    ),
                    Val::Undefined(v) => (7, v.len(), v),
                    Val::Byte(v) => (1, v.len(), v),
                };
                let value = if bytes.len() <= 4 {
                    let mut v = bytes.clone();
                    v.resize(4, 0);
                    v
                } else {
                    self.put(&bytes).to_le_bytes().to_vec()
                };
                let value = if bytes.len() <= 4 {
                    value
                } else {
                    self.u32(u32::from_le_bytes(value.try_into().unwrap()))
                        .to_vec()
                };
                encoded.push((tag, ty, count as u32, value));
            }
            let mut ifd = self.u16(encoded.len() as u16).to_vec();
            for (tag, ty, count, value) in encoded {
                ifd.extend(self.u16(tag));
                ifd.extend(self.u16(ty));
                ifd.extend(self.u32(count));
                ifd.extend(value);
            }
            ifd.extend([0; 4]);
            self.put(&ifd)
        }
    }

    struct Spec {
        be: bool,
        compression: u16,
        bits: u16,
        photometric: u16,
        spp: u16,
        size: (u32, u32),
        tile: (u32, u32),
        tiles: Vec<Vec<u8>>,
        table: bool,
        /// The raw image in a SubIFD behind a preview IFD0, or in IFD0.
        sub_ifd: bool,
        /// Strips of this many rows instead of tiles (`tile` is ignored).
        strips: Option<u32>,
    }

    fn spec(tiles: Vec<Vec<u8>>, compression: u16) -> Spec {
        Spec {
            be: false,
            compression,
            bits: 16,
            photometric: PHOTOMETRIC_LINEAR_RAW,
            spp: 3,
            size: (W, H),
            tile: (T, T),
            tiles,
            table: true,
            sub_ifd: true,
            strips: None,
        }
    }

    /// An OpcodeList2 holding one optional GainMap with 5 parameter bytes.
    fn gain_map_list() -> Vec<u8> {
        let mut list = 1u32.to_be_bytes().to_vec();
        for word in [9u32, 0x0103_0000, 1, 5] {
            list.extend(word.to_be_bytes());
        }
        list.extend([7; 5]);
        list
    }

    fn table() -> Vec<u16> {
        (0..1024u16).map(|i| i * 60).collect()
    }

    fn build(s: &Spec) -> Vec<u8> {
        let mut w = Writer::new(s.be);
        let offsets: Vec<u32> = s.tiles.iter().map(|t| w.put(t)).collect();
        let mut raw = vec![
            (tag::NEW_SUBFILE_TYPE, Val::Long(vec![0])),
            (tag::WIDTH, Val::Long(vec![s.size.0])),
            (tag::HEIGHT, Val::Long(vec![s.size.1])),
            (
                tag::BITS_PER_SAMPLE,
                Val::Short(vec![s.bits; usize::from(s.spp)]),
            ),
            (tag::COMPRESSION, Val::Short(vec![s.compression])),
            (tag::PHOTOMETRIC, Val::Short(vec![s.photometric])),
            (tag::SAMPLES_PER_PIXEL, Val::Short(vec![s.spp])),
        ];
        let counts = Val::Long(s.tiles.iter().map(|t| t.len() as u32).collect());
        raw.extend(match s.strips {
            Some(rows) => vec![
                (tag::ROWS_PER_STRIP, Val::Long(vec![rows])),
                (tag::STRIP_OFFSETS, Val::Long(offsets)),
                (tag::STRIP_BYTE_COUNTS, counts),
            ],
            None => vec![
                (tag::TILE_WIDTH, Val::Long(vec![s.tile.0])),
                (tag::TILE_LENGTH, Val::Long(vec![s.tile.1])),
                (tag::TILE_OFFSETS, Val::Long(offsets)),
                (tag::TILE_BYTE_COUNTS, counts),
            ],
        });
        raw.extend(vec![
            (
                tag::BLACK_LEVEL,
                Val::Rational(
                    [(1, 2), (0, 1), (3, 1)]
                        .into_iter()
                        .cycle()
                        .take(usize::from(s.spp))
                        .collect(),
                ),
            ),
            (tag::WHITE_LEVEL, Val::Long(vec![61380])),
            (tag::ACTIVE_AREA, Val::Long(vec![1, 2, s.size.1, s.size.0])),
            (
                tag::DEFAULT_CROP_ORIGIN,
                Val::Rational(vec![(1, 1), (1, 2)]),
            ),
            (tag::DEFAULT_CROP_SIZE, Val::Long(vec![30, 20])),
            (tag::OPCODE_LIST_2, Val::Undefined(gain_map_list())),
        ]);
        if s.table {
            raw.push((tag::LINEARIZATION_TABLE, Val::Short(table())));
        }
        let meta = vec![
            (
                tag::COLOR_MATRIX_1,
                Val::SRational(vec![
                    (-1, 2),
                    (1, 1),
                    (0, 1),
                    (1, 4),
                    (2, 1),
                    (0, 1),
                    (0, 1),
                    (0, 1),
                    (1, 1),
                ]),
            ),
            (tag::CALIBRATION_ILLUMINANT_1, Val::Short(vec![21])),
            (
                tag::FORWARD_MATRIX_1,
                Val::SRational(vec![
                    (1, 1),
                    (0, 1),
                    (0, 1),
                    (0, 1),
                    (1, 1),
                    (0, 1),
                    (0, 1),
                    (0, 1),
                    (1, 1),
                ]),
            ),
            (
                tag::AS_SHOT_NEUTRAL,
                // A fourth value, as some cameras write: only three are kept.
                Val::Rational(vec![(1, 2), (1, 1), (3, 4), (0, 0)]),
            ),
            (tag::DNG_VERSION, Val::Byte(vec![1, 6, 0, 0])),
            (tag::ORIENTATION, Val::Short(vec![6])),
            (tag::BASELINE_EXPOSURE, Val::SRational(vec![(-51, 100)])),
            (
                tag::PROFILE_GAIN_TABLE_MAP,
                Val::Undefined(vec![1, 2, 3, 4, 5, 6]),
            ),
        ];
        let ifd0 = if s.sub_ifd {
            let sub = w.ifd(raw);
            let mut ifd0 = meta;
            ifd0.extend([
                (tag::NEW_SUBFILE_TYPE, Val::Long(vec![1])),
                (tag::SUB_IFDS, Val::Long(vec![sub])),
                (tag::PHOTOMETRIC, Val::Short(vec![2])),
            ]);
            w.ifd(ifd0)
        } else {
            raw.extend(meta);
            w.ifd(raw)
        };
        let at = w.u32(ifd0);
        w.buf[4..8].copy_from_slice(&at);
        w.buf
    }

    /// Checks every pixel, the clamp count and the metadata.
    fn check(img: &LinearDng, with_table: bool) {
        assert_eq!((img.width, img.height, img.samples_per_pixel), (W, H, 3));
        let table = table();
        let mut clamped = 0;
        for y in 0..H {
            for x in 0..W {
                for c in 0..3 {
                    let stored = sample(x, y, c);
                    let want = if with_table {
                        clamped += u64::from(stored >= 1024);
                        table[usize::from(stored.min(1023))]
                    } else {
                        stored
                    };
                    let got = img.pixels[((y * W + x) * 3 + c) as usize];
                    assert_eq!(got, want, "pixel ({x}, {y}) sample {c}");
                }
            }
        }
        assert!(
            !with_table || clamped > 0,
            "the fixture must reach past the table"
        );
        assert_eq!(img.clamped, clamped);
        assert_eq!(img.orientation, 6);
        let meta = &img.meta;
        assert_eq!(meta.levels.black, vec![0.5, 0.0, 3.0]);
        assert_eq!(meta.levels.black_repeat, (1, 1));
        assert!(meta.levels.black_delta_h.is_empty() && meta.levels.black_delta_v.is_empty());
        assert_eq!(meta.levels.white, vec![61380; 3]);
        assert_eq!(
            meta.active_area,
            Some(Rect {
                top: 1,
                left: 2,
                bottom: H,
                right: W
            })
        );
        assert_eq!(
            meta.default_crop,
            Some(DefaultCrop {
                x: 1.0,
                y: 0.5,
                width: 30.0,
                height: 20.0
            })
        );
        let color = &meta.color;
        let m = color.color_matrix_1.as_ref().unwrap();
        assert_eq!(m.illuminant, Some(21));
        assert_eq!(&m.matrix[..4], &[-0.5, 1.0, 0.0, 0.25]);
        assert_eq!(color.color_matrix_2, None);
        assert_eq!(color.forward_matrix_1.as_deref().map(<[f64]>::len), Some(9));
        assert_eq!(color.forward_matrix_2, None);
        assert_eq!(color.as_shot, Some(AsShot::Neutral([0.5, 1.0, 0.75])));
        assert_eq!(color.baseline_exposure, -0.51);
        assert_eq!(
            meta.gain_table_map.as_deref(),
            Some(&[1, 2, 3, 4, 5, 6][..])
        );
        assert_eq!(meta.opcodes.list1, Vec::new());
        assert_eq!(meta.opcodes.list3, Vec::new());
        let gain = &meta.opcodes.list2;
        assert_eq!(gain.len(), 1);
        assert_eq!(gain[0].kind, crate::raw::OpcodeKind::GainMap);
        assert!(gain[0].optional);
        assert_eq!(gain[0].params, [7; 5]);
    }

    #[test]
    fn jpeg_xl_tiles_at_16_and_12_bits_in_both_byte_orders() {
        for bits in [16, 12] {
            for be in [false, true] {
                let mut s = spec(jxl_tiles(bits), COMPRESSION_JPEG_XL);
                s.be = be;
                let img = decode_linear(&build(&s)).unwrap();
                assert_eq!(img.compression, COMPRESSION_JPEG_XL);
                check(&img, true);
            }
        }
    }

    /// 12-bit lossless JPEG tiles as the iPhone 12 and 13 Pro write them.
    #[cfg(feature = "raw")]
    #[test]
    fn lossless_jpeg_tiles() {
        for be in [false, true] {
            let mut s = spec(fixture_tiles("lj12", "ljpg"), COMPRESSION_LOSSLESS_JPEG);
            s.be = be;
            s.bits = 12;
            let img = decode_linear(&build(&s)).unwrap();
            assert_eq!(img.compression, COMPRESSION_LOSSLESS_JPEG);
            check(&img, true);
        }
        // A JPEG XL stream where lossless JPEG is declared, and a tile cut
        // short: errors, not panics.
        let s = spec(jxl_tiles(16), COMPRESSION_LOSSLESS_JPEG);
        assert!(matches!(error(&s), DngError::Corrupt(m) if m.contains("lossless JPEG")));
        let mut s = spec(fixture_tiles("lj12", "ljpg"), COMPRESSION_LOSSLESS_JPEG);
        s.tiles[2].truncate(40);
        assert!(matches!(error(&s), DngError::Corrupt(_)));
    }

    /// A fuzzer input with a damaged lossless JPEG Huffman table: rawler's
    /// decompressor panics on it (huffman.rs, index out of bounds), and the
    /// panic must come back as an error, not unwind into the caller.
    #[cfg(feature = "raw")]
    #[test]
    fn a_panic_in_the_lossless_jpeg_decoder_is_an_error() {
        let file = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/dng/fuzz-ljpeg-huffman-panic.dng"
        ))
        .unwrap();
        // Bands decode in parallel, so the error reported may be another
        // tile's; what matters is that the panic does not escape.
        let result = decode_linear(&file);
        println!("{result:?}");
        assert!(matches!(result, Err(DngError::Corrupt(_))));
    }

    #[test]
    fn uncompressed_tiles_without_a_table() {
        for be in [false, true] {
            let mut s = spec(raw_tiles(be), COMPRESSION_NONE);
            s.be = be;
            s.table = false;
            check(&decode_linear(&build(&s)).unwrap(), false);
        }
    }

    /// Uncompressed 16-bit strips of `rows` rows; the last one holds only
    /// the rows left, as TIFF writes it.
    fn raw_strips(rows: u32) -> Vec<Vec<u8>> {
        (0..H.div_ceil(rows))
            .map(|i| {
                let mut out = Vec::new();
                for y in i * rows..((i + 1) * rows).min(H) {
                    for x in 0..W {
                        for c in 0..3 {
                            out.extend(sample(x, y, c).to_le_bytes());
                        }
                    }
                }
                out
            })
            .collect()
    }

    #[test]
    fn strips_with_a_short_last_strip() {
        let mut s = spec(raw_strips(16), COMPRESSION_NONE);
        s.strips = Some(16);
        check(&decode_linear(&build(&s)).unwrap(), true);
        // A last strip shorter than its rows.
        s.tiles[1].truncate(W as usize * 6 * 10);
        assert!(matches!(error(&s), DngError::Corrupt(m) if m.contains("too short")));
    }

    /// One 12-bit lossless JPEG strip with a restart every 4 rows, as
    /// Samsung writes them (tests/fixtures/dng/make-restarts.py).
    #[cfg(feature = "raw")]
    fn restart_strip() -> Spec {
        let mut s = spec(
            vec![fixture("lj12_restarts.ljpg")],
            COMPRESSION_LOSSLESS_JPEG,
        );
        s.bits = 12;
        s.strips = Some(H);
        s
    }

    #[cfg(feature = "raw")]
    #[test]
    fn lossless_jpeg_strip_with_restarts() {
        check(&decode_linear(&build(&restart_strip())).unwrap(), true);
    }

    #[test]
    fn raw_image_in_ifd0() {
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.sub_ifd = false;
        check(&decode_linear(&build(&s)).unwrap(), true);
    }

    fn error(s: &Spec) -> DngError {
        decode_linear(&build(s)).unwrap_err()
    }

    #[test]
    fn says_what_it_does_not_read() {
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.photometric = PHOTOMETRIC_CFA;
        assert!(matches!(error(&s), DngError::Unsupported(m) if m.contains("CFA")));

        if !cfg!(feature = "raw") {
            let s = spec(jxl_tiles(16), COMPRESSION_LOSSLESS_JPEG);
            assert!(matches!(error(&s), DngError::NotBuiltIn(_, "raw")));
        }
        let s = spec(jxl_tiles(16), 34713);
        assert!(matches!(error(&s), DngError::Unsupported(m) if m.contains("34713")));

        let mut s = spec(raw_tiles(false), COMPRESSION_NONE);
        s.bits = 12;
        assert!(matches!(error(&s), DngError::Unsupported(_)));

        assert!(matches!(decode_linear(b"GIF89a"), Err(DngError::NotTiff)));
    }

    #[test]
    fn read_metadata_rejects_a_huge_sample_count() {
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.spp = 60_000;
        assert!(
            matches!(read_metadata(&build(&s)), Err(DngError::Unsupported(m)) if m.contains("60000"))
        );
    }

    #[test]
    fn rejects_inconsistent_files() {
        // One tile short of the 3x2 grid.
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.tiles.pop();
        assert!(matches!(error(&s), DngError::Corrupt(_)));

        // JPEG XL tiles larger than the tile grid claims.
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.tile = (8, 8);
        s.size = (24, 16);
        assert!(matches!(error(&s), DngError::Corrupt(m) if m.contains("tile")));

        // A 3-channel tile in a 1-sample image.
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.spp = 1;
        assert!(matches!(error(&s), DngError::Corrupt(m) if m.contains("channels")));

        // Larger than any sensor.
        let mut s = spec(Vec::new(), COMPRESSION_JPEG_XL);
        s.size = (60_000, 60_000);
        assert!(matches!(error(&s), DngError::TooLarge(_)));

        // Within the pixel cap, but 900M samples: refused before allocating.
        let mut s = spec(Vec::new(), COMPRESSION_JPEG_XL);
        s.size = (20_000, 15_000);
        assert!(matches!(error(&s), DngError::TooLarge(_)));

        // An empty tile.
        let mut s = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        s.tiles[4].clear();
        assert!(matches!(error(&s), DngError::Corrupt(m) if m.contains("empty")));
    }

    /// Writes the seed inputs of the `dng_linear` and `decode` fuzz targets:
    /// cargo test -p genzo-imgio --features raw write_fuzz_seeds -- --ignored
    #[test]
    #[ignore = "writes files into fuzz/seeds"]
    fn write_fuzz_seeds() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/dng_linear");
        std::fs::create_dir_all(dir).unwrap();
        let jxl = spec(jxl_tiles(16), COMPRESSION_JPEG_XL);
        std::fs::write(format!("{dir}/jpeg-xl-tiles.dng"), build(&jxl)).unwrap();
        let mut plain = spec(raw_tiles(true), COMPRESSION_NONE);
        plain.be = true;
        std::fs::write(format!("{dir}/uncompressed-big-endian.dng"), build(&plain)).unwrap();
        let mut ljpeg = spec(fixture_tiles("lj12", "ljpg"), COMPRESSION_LOSSLESS_JPEG);
        ljpeg.bits = 12;
        std::fs::write(format!("{dir}/lossless-jpeg-tiles.dng"), build(&ljpeg)).unwrap();
        let mut strips = spec(raw_strips(16), COMPRESSION_NONE);
        strips.strips = Some(16);
        std::fs::write(format!("{dir}/uncompressed-strips.dng"), build(&strips)).unwrap();
        #[cfg(feature = "raw")]
        std::fs::write(
            format!("{dir}/lossless-jpeg-strip-restarts.dng"),
            build(&restart_strip()),
        )
        .unwrap();
    }

    #[test]
    fn truncated_and_damaged_files_never_panic() {
        let file = build(&spec(jxl_tiles(16), COMPRESSION_JPEG_XL));
        for len in 0..file.len() {
            let _ = decode_linear(&file[..len]);
        }
        // Flip every third byte in turn; errors are fine, panics are not.
        for i in (0..file.len()).step_by(3) {
            let mut damaged = file.clone();
            damaged[i] ^= 0xFF;
            let _ = decode_linear(&damaged);
        }
    }
}
