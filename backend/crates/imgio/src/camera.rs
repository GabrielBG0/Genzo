//! Camera RAW and CFA DNG through rawler (DEV-01, stage 0): the sensor
//! mosaic, its color filter pattern, and the metadata stage 1 to 4 need.
//!
//! For a DNG, rawler supplies only the samples; the pattern, levels,
//! matrices, opcodes and orientation come from this crate's own tag reader
//! (`dng::read_metadata`, `dng::cfa_pattern`), which reads every DNG tag
//! rawler leaves out and shifts the pattern the right way. For
//! other formats the metadata is mapped from rawler's.
//!
//! rawler is not hardened against bad input the way this crate is: a panic
//! in it is turned into an error. Before a CFA DNG reaches it,
//! `dng::check_cfa_for_rawler` turns away what it decodes wrongly.

use rawler::RawlerError;
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;

use crate::raw::{
    AsShot, CameraColor, CfaPattern, ColorMatrix, DefaultCrop, Levels, OpcodeLists, RawMetadata,
    Rect,
};
use crate::{DecodeError, DecodedImage, FileFormat, Pixels, dng};

/// Largest image accepted, in pixels, as for linear DNGs.
const MAX_PIXELS: u64 = 400_000_000;
/// Largest repeating pattern accepted (X-Trans is 6 × 6).
const MAX_PATTERN: usize = 8;

fn unsupported(format: FileFormat, why: impl Into<String>) -> DecodeError {
    DecodeError::Unsupported(format, why.into())
}

fn corrupt(format: FileFormat, message: impl Into<String>) -> DecodeError {
    DecodeError::Corrupt {
        format,
        message: message.into(),
    }
}

/// rawler's errors. Its `DecoderFailed` covers both damaged files and
/// variants it does not read, told apart only by the message, so the
/// message is matched for the cases DEV-01 names (Nikon High Efficiency) and
/// for rawler's own "unsupported" wording.
fn rawler_error(format: FileFormat, e: RawlerError) -> DecodeError {
    match e {
        // No camera format matched: for a TIFF, a plain TIFF image, which
        // Genzo does not decode yet.
        RawlerError::Unsupported { what, .. }
            if format == FileFormat::Tiff && what.contains("No decoder found") =>
        {
            unsupported(format, "plain TIFF images are not supported yet")
        }
        RawlerError::Unsupported {
            what, make, model, ..
        } if make.trim().is_empty() && model.trim().is_empty() => unsupported(format, what),
        RawlerError::Unsupported {
            what, make, model, ..
        } => unsupported(format, format!("{make} {model}: {what}")),
        RawlerError::DecoderFailed(m) if m.contains("HighEfficency") => {
            unsupported(format, "Nikon High Efficiency RAW is not supported yet")
        }
        RawlerError::DecoderFailed(m)
            if [
                "not supported",
                "Unsupported",
                "unsupported",
                "find camera",
                "No decoder",
            ]
            .iter()
            .any(|w| m.contains(w)) =>
        {
            unsupported(format, m)
        }
        RawlerError::DecoderFailed(m) => corrupt(format, m),
    }
}

fn run_rawler(data: &[u8], format: FileFormat) -> Result<RawImage, DecodeError> {
    std::panic::catch_unwind(|| {
        let source = RawSource::new_from_slice(data);
        rawler::decode(&source, &RawDecodeParams::default())
    })
    .map_err(|_| corrupt(format, "the RAW decoder panicked"))?
    .map_err(|e| rawler_error(format, e))
}

/// The EXIF orientation code of rawler's orientation.
fn orientation(o: Orientation) -> u8 {
    match o {
        Orientation::HorizontalFlip => 2,
        Orientation::Rotate180 => 3,
        Orientation::VerticalFlip => 4,
        Orientation::Transpose => 5,
        Orientation::Rotate90 => 6,
        Orientation::Transverse => 7,
        Orientation::Rotate270 => 8,
        Orientation::Normal | Orientation::Unknown => 1,
    }
}

/// Checks rawler's image is a mosaic Genzo reads, and takes its samples and
/// pattern.
fn mosaic(
    img: RawImage,
    format: FileFormat,
) -> Result<(CfaPattern, Vec<u16>, RawImage), DecodeError> {
    let RawPhotometricInterpretation::Cfa(config) = &img.photometric else {
        return Err(unsupported(
            format,
            format!("a {:?} image from the RAW decoder", img.photometric),
        ));
    };
    if img.cpp != 1 {
        return Err(unsupported(
            format,
            format!("a CFA image with {} samples per pixel", img.cpp),
        ));
    }
    let pixels = img.width as u64 * img.height as u64;
    if img.width == 0 || img.height == 0 || pixels > MAX_PIXELS {
        return Err(corrupt(
            format,
            format!("an image of {}x{}", img.width, img.height),
        ));
    }
    let cfa = &config.cfa;
    if !(1..=MAX_PATTERN).contains(&cfa.width) || !(1..=MAX_PATTERN).contains(&cfa.height) {
        return Err(unsupported(
            format,
            format!("a {}x{} CFA pattern", cfa.width, cfa.height),
        ));
    }
    let mut colors = Vec::with_capacity(cfa.width * cfa.height);
    for y in 0..cfa.height {
        for x in 0..cfa.width {
            match cfa.color_at(y, x) {
                c @ 0..=2 => colors.push(c as u8),
                _ => {
                    return Err(unsupported(
                        format,
                        format!("the color filter pattern {}", cfa.name),
                    ));
                }
            }
        }
    }
    let pattern = CfaPattern {
        width: cfa.width as u8,
        height: cfa.height as u8,
        colors,
    };
    let mut img = img;
    let data = match std::mem::replace(&mut img.data, RawImageData::Integer(Vec::new())) {
        RawImageData::Integer(d) => d,
        RawImageData::Float(_) => {
            return Err(unsupported(format, "floating-point CFA samples"));
        }
    };
    if data.len() as u64 != pixels {
        return Err(corrupt(
            format,
            format!("{} samples for {}x{}", data.len(), img.width, img.height),
        ));
    }
    Ok((pattern, data, img))
}

/// The bits a white level needs: what `source_bits` reports for formats
/// whose samples rawler widens to 16 bits.
fn bits_for(white: &[u32]) -> u8 {
    let max = white.iter().copied().max().unwrap_or(u32::from(u16::MAX));
    (32 - max.leading_zeros()).clamp(1, 16) as u8
}

/// A camera RAW (not DNG) through rawler, with its metadata mapped.
pub(crate) fn decode(data: &[u8], format: FileFormat) -> Result<DecodedImage, DecodeError> {
    let img = run_rawler(data, format)?;
    let (pattern, samples, img) = mosaic(img, format)?;
    let raw = metadata(&img, format)?;
    Ok(DecodedImage {
        format,
        width: img.width as u32,
        height: img.height as u32,
        source_bits: bits_for(&raw.levels.white),
        has_alpha: false,
        pixels: Pixels::Cfa {
            pattern,
            data: samples,
            raw: Box::new(raw),
        },
        orientation: orientation(img.orientation),
        exif: None,
    })
}

/// A CFA DNG: samples and pattern from rawler, everything else from this
/// crate's DNG tag reader.
pub(crate) fn decode_dng(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    let format = FileFormat::Tiff;
    let dng_error = |e: dng::DngError| match e {
        dng::DngError::Unsupported(m) => unsupported(format, m),
        dng::DngError::NotBuiltIn(_, feature) => DecodeError::NotBuiltIn(format, feature),
        other => corrupt(format, other.to_string()),
    };
    let (width, height) = dng::check_cfa_for_rawler(data).map_err(dng_error)?;
    let (meta, orientation) = dng::read_metadata(data).map_err(dng_error)?;
    let pattern = dng::cfa_pattern(data).map_err(dng_error)?;
    let img = run_rawler(data, format)?;
    let (_, samples, img) = mosaic(img, format)?;
    if (img.width as u64, img.height as u64) != (u64::from(width), u64::from(height)) {
        return Err(corrupt(
            format,
            format!(
                "the RAW decoder read {}x{} from a {width}x{height} raw image",
                img.width, img.height
            ),
        ));
    }
    Ok(DecodedImage {
        format,
        width,
        height,
        source_bits: bits_for(&meta.levels.white),
        has_alpha: false,
        pixels: Pixels::Cfa {
            pattern,
            data: samples,
            raw: Box::new(meta),
        },
        orientation,
        exif: None,
    })
}

/// rawler's rectangle as ours, if it is non-empty and lies inside the
/// image. rawler builds it from file data without checks.
fn rect(r: &rawler::imgop::Rect, width: usize, height: usize) -> Option<Rect> {
    let bottom = r.p.y.checked_add(r.d.h).filter(|&b| b <= height)?;
    let right = r.p.x.checked_add(r.d.w).filter(|&b| b <= width)?;
    (r.d.w > 0 && r.d.h > 0).then_some(Rect {
        top: r.p.y as u32,
        left: r.p.x as u32,
        bottom: bottom as u32,
        right: right as u32,
    })
}

/// The stage 0 metadata of a camera RAW, from rawler's fields.
fn metadata(img: &RawImage, format: FileFormat) -> Result<RawMetadata, DecodeError> {
    // Black levels per pattern site, row by row: rawler builds them over
    // the CFA's repeat (checked against LibRaw on the OM-3, whose two
    // greens differ).
    let b = &img.blacklevel;
    let (rows, cols) = (b.height.max(1), b.width.max(1));
    let black: Vec<f64> = b.levels.iter().map(|r| f64::from(r.as_f32())).collect();
    let black = match black.len() {
        n if n == rows * cols => black,
        1 => vec![black[0]; rows * cols],
        n => {
            return Err(corrupt(
                format,
                format!("{n} black levels for a {rows}x{cols} pattern"),
            ));
        }
    };
    if rows > MAX_PATTERN || cols > MAX_PATTERN {
        return Err(unsupported(
            format,
            format!("a {rows}x{cols} black level pattern"),
        ));
    }
    let white = match img.whitelevel.0.first() {
        Some(&w) if w > 0 => vec![w],
        _ => return Err(corrupt(format, "no white level")),
    };

    let bad = |what: &str, r: &rawler::imgop::Rect| {
        corrupt(
            format,
            format!("{what} {r:?} in a {}x{} image", img.width, img.height),
        )
    };
    let active = match &img.active_area {
        Some(r) => Some(rect(r, img.width, img.height).ok_or_else(|| bad("active area", r))?),
        None => None,
    };
    let origin = active.map_or((0, 0), |a| (a.left, a.top));
    let default_crop = match &img.crop_area {
        Some(c) => {
            let c = rect(c, img.width, img.height).ok_or_else(|| bad("crop", c))?;
            Some(DefaultCrop {
                x: f64::from(c.left) - f64::from(origin.0),
                y: f64::from(c.top) - f64::from(origin.1),
                width: f64::from(c.right - c.left),
                height: f64::from(c.bottom - c.top),
            })
        }
        None => None,
    };

    // The two matrices for the lowest and highest color temperature, as
    // ColorMatrix1 and 2 (rawler keys them by EXIF illuminant).
    let mut matrices: Vec<(u16, Vec<f64>)> = img
        .color_matrix
        .iter()
        .filter(|(_, m)| m.len() >= 9)
        .map(|(i, m)| (*i as u16, m[..9].iter().map(|&v| f64::from(v)).collect()))
        .collect();
    matrices.sort_by(|a, b| cct(a.0).total_cmp(&cct(b.0)));
    let first = matrices.first().cloned();
    let last = if matrices.len() > 1 {
        matrices.last().cloned()
    } else {
        None
    };
    let to_matrix = |(illuminant, matrix): (u16, Vec<f64>)| ColorMatrix {
        illuminant: Some(illuminant),
        matrix,
    };

    // White balance coefficients are the inverse of the camera neutral:
    // neutral = (G/R, 1, G/B). Only the first three are valid.
    let wb = &img.wb_coeffs;
    let as_shot = if wb[..3].iter().all(|c| c.is_finite() && *c > 0.0) {
        let g = f64::from(wb[1]);
        Some(AsShot::Neutral([
            g / f64::from(wb[0]),
            1.0,
            g / f64::from(wb[2]),
        ]))
    } else {
        None
    };

    Ok(RawMetadata {
        levels: Levels {
            black,
            black_repeat: (rows as u16, cols as u16),
            black_delta_h: Vec::new(),
            black_delta_v: Vec::new(),
            white,
        },
        active_area: active,
        default_crop,
        color: CameraColor {
            color_matrix_1: first.map(to_matrix),
            color_matrix_2: last.map(to_matrix),
            as_shot,
            ..CameraColor::default()
        },
        opcodes: OpcodeLists::default(),
        gain_table_map: None,
    })
}

/// Approximate correlated color temperature of an EXIF illuminant, only to
/// order the matrices.
fn cct(illuminant: u16) -> f64 {
    match illuminant {
        17 | 3 | 24 => 2856.0, // A, tungsten, ISO studio tungsten
        18 => 4874.0,          // B
        19 => 6774.0,          // C
        20 => 5503.0,          // D55
        21 | 1 | 9 => 6504.0,  // D65, daylight, fine weather
        22 => 7504.0,          // D75
        23 => 5003.0,          // D50
        10 => 6500.0,
        11 => 7500.0,
        12 => 6430.0,
        13 => 5000.0,
        14 => 4150.0,
        15 => 3450.0,
        2 => 4000.0,
        4 => 5500.0,
        _ => 5000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dng");
        std::fs::read(format!("{dir}/{name}")).unwrap()
    }

    /// Inputs the `decode` fuzz target found. One made rawler's TIFF parser
    /// allocate 13.7 GB for a tag's claimed count (now checked against the
    /// file, vendor/rawler/src/formats/tiff/entry.rs); the other overflows in
    /// rawler's rectangle arithmetic (a caught panic). Both are errors.
    #[test]
    fn fuzz_finds_are_errors() {
        for name in ["fuzz-rawler-tag-count-oom.bin", "fuzz-rawler-rect-overflow.bin"] {
            let result = crate::decode(&fixture(name), crate::DecodeOptions::default());
            assert!(result.is_err(), "{name}: {result:?}");
        }
    }

    #[test]
    fn orientation_round_trips_every_exif_value() {
        for v in 1..=8u16 {
            assert_eq!(u16::from(orientation(Orientation::from_u16(v))), v);
        }
        assert_eq!(orientation(Orientation::from_u16(0)), 1);
        assert_eq!(orientation(Orientation::from_u16(9)), 1);
    }

    #[test]
    fn rectangles_outside_the_image_are_refused() {
        use rawler::imgop::{Dim2, Point, Rect as R};
        let r = |x, y, w, h| R::new(Point::new(x, y), Dim2::new(w, h));
        assert_eq!(
            rect(&r(2, 1, 8, 4), 10, 5),
            Some(Rect {
                top: 1,
                left: 2,
                bottom: 5,
                right: 10
            })
        );
        assert_eq!(rect(&r(3, 0, 8, 4), 10, 5), None);
        assert_eq!(rect(&r(0, 0, 0, 4), 10, 5), None);
        assert_eq!(rect(&r(usize::MAX, 0, 2, 1), 10, 5), None);
    }
}
