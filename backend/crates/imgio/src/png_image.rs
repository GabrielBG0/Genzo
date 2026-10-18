//! PNG through the `png` crate. Palette, tRNS and 1, 2 and 4-bit gray are
//! expanded; 16-bit images keep their 16 bits; Adam7 is deinterlaced. The
//! orientation comes from the eXIf chunk.
//!
//! Color, by the PNG 3 precedence: cICP, then iCCP, then sRGB. A file with
//! only gAMA or cHRM, or no color chunk at all, is taken as sRGB (decided
//! for M1, DEV-01).

use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Limits, Transformations};

use crate::{ColorEncoding, DecodeError, DecodedImage, FileFormat, Pixels, exif, raster};

const FORMAT: FileFormat = FileFormat::Png;

fn png_error(e: png::DecodingError) -> DecodeError {
    match e {
        png::DecodingError::LimitsExceeded => DecodeError::Corrupt {
            format: FORMAT,
            message: "the image is too large".into(),
        },
        other => DecodeError::Corrupt {
            format: FORMAT,
            message: other.to_string(),
        },
    }
}

/// Decodes a PNG (its default image; an APNG's first frame) to RGBA16.
pub(crate) fn decode(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    // Room for the decoder's own buffers at the pixel limit; the size is
    // checked against the header below before the image is allocated.
    let limits = Limits {
        bytes: (raster::MAX_PIXELS * 8) as usize,
    };
    let mut decoder = Decoder::new_with_limits(Cursor::new(data), limits);
    decoder.set_transformations(Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(png_error)?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    let pixels = raster::check_size(FORMAT, u64::from(width), u64::from(height))?;
    let source_bits = info.bit_depth as u8;
    let color = if let Some(c) = info.coding_independent_code_points {
        match (c.color_primaries, c.transfer_function) {
            (1, 13) => ColorEncoding::Srgb,
            (12, 13) => ColorEncoding::DisplayP3,
            (p, t) => ColorEncoding::Nclx {
                primaries: u16::from(p),
                transfer: u16::from(t),
                matrix: u16::from(c.matrix_coefficients),
                full_range: c.is_video_full_range_image,
            },
        }
    } else if let Some(icc) = info.icc_profile.as_ref().filter(|i| !i.is_empty()) {
        ColorEncoding::Icc(icc.to_vec())
    } else {
        ColorEncoding::Srgb
    };
    // png 0.17 writes eXIf but does not read it.
    let exif = exif_chunk(data)
        .map(|e| exif::tiff_payload(e).to_vec())
        .filter(|e| !e.is_empty());

    let (color_type, depth) = reader.output_color_type();
    let channels = match color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => unreachable!("EXPAND turns palettes into RGB(A)"),
    };
    let bytes_per_sample = if depth == BitDepth::Sixteen { 2 } else { 1 };
    let size = reader.output_buffer_size();
    if size != pixels * channels * bytes_per_sample {
        return Err(DecodeError::Corrupt {
            format: FORMAT,
            message: format!("{size} output bytes for {pixels} pixels"),
        });
    }
    let mut buf = Vec::new();
    buf.try_reserve_exact(size)
        .map_err(|_| DecodeError::Corrupt {
            format: FORMAT,
            message: format!("no memory for {pixels} pixels"),
        })?;
    buf.resize(size, 0);
    reader.next_frame(&mut buf).map_err(png_error)?;

    let mut out = raster::rgba16(FORMAT, pixels)?;
    if bytes_per_sample == 2 {
        // PNG stores 16-bit samples big-endian.
        let samples: Vec<u16> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_be_bytes(b))
            .collect();
        raster::from16(&mut out, &samples, channels);
    } else {
        raster::from8(&mut out, &buf, channels);
    }
    Ok(DecodedImage {
        format: FORMAT,
        width,
        height,
        source_bits,
        has_alpha: channels % 2 == 0,
        pixels: Pixels::Rgba16 { data: out, color },
        orientation: exif.as_deref().map_or(1, exif::orientation),
        exif,
    })
}

/// The body of the eXIf chunk, if the file has one. The chunk layout was
/// already validated by the decoder; this walk only checks bounds.
fn exif_chunk(data: &[u8]) -> Option<&[u8]> {
    let mut at = 8; // after the signature
    while at + 8 <= data.len() {
        let len = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as usize;
        let kind = &data[at + 4..at + 8];
        let body = data.get(at + 8..(at + 8).checked_add(len)?)?;
        match kind {
            b"eXIf" => return Some(body),
            b"IEND" => return None,
            _ => at = at + 12 + len,
        }
    }
    None
}
