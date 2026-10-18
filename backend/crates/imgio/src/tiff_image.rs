//! Plain TIFF images (not DNG, not camera RAW) through the `tiff` crate:
//! 8 and 16-bit gray, gray and alpha, RGB and RGBA, in any of the
//! crate's layouts (strips or tiles, chunky or planar, uncompressed, LZW,
//! Deflate, PackBits, JPEG). CMYK, Lab, palette (tiff 0.11 refuses it),
//! floating point and bilevel images are unsupported.

use std::io::Cursor;

use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tiff::{ColorType, TiffError};

use crate::tiff_ifd::{Tiff, find};
use crate::{ColorEncoding, DecodeError, DecodedImage, FileFormat, Pixels, raster};

const FORMAT: FileFormat = FileFormat::Tiff;

/// TIFF tags read for routing.
const NEW_SUBFILE_TYPE: u16 = 254;
const PHOTOMETRIC: u16 = 262;
const STRIP_BYTE_COUNTS: u16 = 279;
const TILE_BYTE_COUNTS: u16 = 325;

/// Bulk metadata a plain TIFF may carry beside its image: Photoshop's
/// layers (ImageSourceData) and image resources, XMP, an ICC profile.
const BULK_TAGS: [u16; 4] = [37724, 34377, 700, 34675];

/// Most IFDs followed in the chain.
const MAX_CHAIN: usize = 64;

/// Whether IFD0 holds a full image the way a plain TIFF does, so a TIFF
/// that no camera decoder took may be read as one.
///
/// IFD0 must not be reduced-resolution and must have a TIFF 6 photometric,
/// and the file must be accounted for: the image data of every IFD in the
/// chain (pages), the bulk metadata tags (layers, XMP, ICC), and up to
/// 64 KiB for everything else. TIFF-based camera RAW fails this: NEF, ARW,
/// 3FR and DCR mark IFD0 reduced, and Phase One IIQ keeps megabytes of raw
/// data that no IFD's strips point to. So a truncated camera RAW never comes
/// back as its thumbnail, while a layered, multi-page or XMP-heavy TIFF
/// still opens.
pub(crate) fn looks_like_plain_tiff(data: &[u8]) -> bool {
    (|| {
        let t = Tiff::new(data)?;
        let first = t.u32(4)? as usize;
        let ifd0 = t.ifd(first).ok()?;
        let reduced = t.first_int(&ifd0, NEW_SUBFILE_TYPE).unwrap_or(0) & 1 != 0;
        // The TIFF 6 interpretations (gray, RGB, palette, CMYK, YCbCr,
        // CIELab and its ICC and ITU variants, up to 10), so unsupported ones
        // reach the TIFF decoder and are named. CFA (32803) and LinearRaw
        // (34892) are raw data, never a plain image.
        let photometric = t.first_int(&ifd0, PHOTOMETRIC)?;
        let mut known = 0u64;
        let mut at = first;
        let mut seen = Vec::new();
        while at != 0 && seen.len() < MAX_CHAIN && !seen.contains(&at) {
            seen.push(at);
            let Ok(ifd) = t.ifd(at) else { break };
            if let Some(counts) =
                find(&ifd, STRIP_BYTE_COUNTS).or_else(|| find(&ifd, TILE_BYTE_COUNTS))
            {
                known += t
                    .ints(counts)
                    .ok()?
                    .iter()
                    .map(|&n| u64::from(n))
                    .sum::<u64>();
            }
            for tag in BULK_TAGS {
                known += find(&ifd, tag).map_or(0, Tiff::value_len);
            }
            at = t.next_ifd(at).unwrap_or(0) as usize;
        }
        let rest = (data.len() as u64).saturating_sub(known);
        Some(!reduced && photometric <= 10 && rest < 64 << 10)
    })()
    .unwrap_or(false)
}

fn tiff_error(e: TiffError) -> DecodeError {
    match e {
        TiffError::UnsupportedError(u) => DecodeError::Unsupported(FORMAT, u.to_string()),
        other => DecodeError::Corrupt {
            format: FORMAT,
            message: other.to_string(),
        },
    }
}

/// Decodes IFD0 of a plain TIFF to RGBA16.
pub(crate) fn decode(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    let mut limits = Limits::default();
    // The whole image is decoded at once, up to RGBA16 at the pixel limit.
    limits.decoding_buffer_size = (raster::MAX_PIXELS * 8) as usize;
    limits.intermediate_buffer_size = 1 << 30;
    // ICC profiles and color maps; real ones are far smaller.
    limits.ifd_value_size = 16 << 20;
    let mut d = Decoder::new(Cursor::new(data))
        .map_err(tiff_error)?
        .with_limits(limits);
    let (width, height) = d.dimensions().map_err(tiff_error)?;
    let pixels = raster::check_size(FORMAT, u64::from(width), u64::from(height))?;
    // LZW, the most expansive of the supported compressions, reaches about
    // 2700 to 1; 8192 leaves a margin. One byte per pixel is the least an
    // 8-bit gray image decodes to.
    raster::check_plausible(FORMAT, data.len(), pixels as u64, 8192)?;
    let color_type = d.colortype().map_err(tiff_error)?;
    let (channels, bits) = match color_type {
        ColorType::Gray(b) => (1, b),
        ColorType::Multiband {
            bit_depth,
            num_samples: 2,
        } => (2, bit_depth),
        ColorType::GrayA(b) => (2, b),
        ColorType::RGB(b) | ColorType::YCbCr(b) => (3, b),
        ColorType::RGBA(b) => (4, b),
        other => {
            return Err(DecodeError::Unsupported(
                FORMAT,
                format!("TIFF color type {other:?}"),
            ));
        }
    };
    // SampleFormat: 1 unsigned (the default), 2 signed, 3 floating point.
    // One value per sample; they agree for the colors read here.
    let sample_format = d
        .find_tag_unsigned_vec::<u16>(Tag::SampleFormat)
        .ok()
        .flatten()
        .and_then(|v| v.first().copied());
    match sample_format {
        None | Some(1) => {}
        Some(3) => {
            return Err(DecodeError::Unsupported(
                FORMAT,
                "floating-point TIFF samples".into(),
            ));
        }
        Some(f) => {
            return Err(DecodeError::Unsupported(
                FORMAT,
                format!("TIFF sample format {f}"),
            ));
        }
    }
    let planar = d
        .find_tag_unsigned::<u16>(Tag::PlanarConfiguration)
        .ok()
        .flatten()
        == Some(2);
    if !matches!(bits, 8 | 16) {
        return Err(DecodeError::Unsupported(
            FORMAT,
            format!("{bits}-bit TIFF samples"),
        ));
    }
    let associated_alpha = channels % 2 == 0
        && d.find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
            .ok()
            .flatten()
            .is_some_and(|v| v.first() == Some(&1));
    let icc = d
        .get_tag_u8_vec(Tag::IccProfile)
        .ok()
        .filter(|icc| !icc.is_empty());
    let orientation = match d.find_tag_unsigned::<u16>(Tag::Orientation) {
        Ok(Some(v @ 1..=8)) => v as u8,
        _ => 1,
    };

    let mut out = raster::rgba16(FORMAT, pixels)?;
    // read_image_to_buffer reads every plane of a planar image (read_image
    // reads only the first), one plane after another.
    let mut image = DecodingResult::U8(Vec::new());
    d.read_image_to_buffer(&mut image).map_err(tiff_error)?;
    if planar && channels > 1 {
        image = match image {
            DecodingResult::U8(v) => DecodingResult::U8(interleave(&v, channels, pixels)?),
            DecodingResult::U16(v) => DecodingResult::U16(interleave(&v, channels, pixels)?),
            other => other,
        };
    }
    match image {
        DecodingResult::U8(src) => {
            check_len(&src, pixels * channels).map(|()| raster::from8(&mut out, &src, channels))?
        }
        DecodingResult::U16(src) => {
            check_len(&src, pixels * channels).map(|()| raster::from16(&mut out, &src, channels))?
        }
        _ => {
            return Err(DecodeError::Unsupported(
                FORMAT,
                "floating-point or signed TIFF samples".into(),
            ));
        }
    }
    if let ColorType::YCbCr(_) = color_type {
        // The tiff crate leaves YCbCr as it is (JPEG-in-TIFF comes back
        // upsampled but not converted).
        let coefficients = d
            .get_tag_f32_vec(Tag::Unknown(YCBCR_COEFFICIENTS))
            .ok()
            .and_then(|v| <[f32; 3]>::try_from(v).ok())
            .unwrap_or([0.299, 0.587, 0.114]);
        let reference = d
            .get_tag_f32_vec(Tag::Unknown(REFERENCE_BLACK_WHITE))
            .ok()
            .and_then(|v| <[f32; 6]>::try_from(v).ok())
            .unwrap_or([0.0, 255.0, 128.0, 255.0, 128.0, 255.0]);
        ycbcr_to_rgb(&mut out, coefficients, reference);
    }
    if associated_alpha {
        unpremultiply(&mut out);
    }
    Ok(DecodedImage {
        format: FORMAT,
        width,
        height,
        source_bits: bits,
        has_alpha: channels % 2 == 0,
        pixels: Pixels::Rgba16 {
            data: out,
            color: icc.map_or(ColorEncoding::Srgb, ColorEncoding::Icc),
        },
        orientation,
        exif: None,
    })
}

/// TIFF tags of the YCbCr conversion (TIFF 6, section 21).
const YCBCR_COEFFICIENTS: u16 = 529;
const REFERENCE_BLACK_WHITE: u16 = 532;

/// YCbCr (8-bit, widened in `out`) to RGB, by TIFF 6 section 21: the
/// ReferenceBlackWhite ranges scale Y to 0..255 and Cb, Cr to -127..127,
/// then the luma coefficients give R, G and B. The defaults are BT.601 with
/// JPEG's full range, what JPEG-in-TIFF uses.
fn ycbcr_to_rgb(out: &mut [u16], [kr, kg, kb]: [f32; 3], rbw: [f32; 6]) {
    let scale =
        |v: u16, lo: f32, hi: f32, code: f32| (f32::from(v) / 257.0 - lo) * code / (hi - lo);
    for px in out.as_chunks_mut::<4>().0 {
        let y = scale(px[0], rbw[0], rbw[1], 255.0);
        let cb = scale(px[1], rbw[2], rbw[3], 127.0);
        let cr = scale(px[2], rbw[4], rbw[5], 127.0);
        let r = y + 2.0 * (1.0 - kr) * cr;
        let b = y + 2.0 * (1.0 - kb) * cb;
        let g = (y - kr * r - kb * b) / kg;
        for (o, v) in px[..3].iter_mut().zip([r, g, b]) {
            *o = raster::widen8(v.round().clamp(0.0, 255.0) as u8);
        }
    }
}

/// Planes, one after another, to interleaved samples.
fn interleave<T: Copy + Default>(
    planes: &[T],
    channels: usize,
    pixels: usize,
) -> Result<Vec<T>, DecodeError> {
    check_len(planes, pixels * channels)?;
    let mut out = vec![T::default(); planes.len()];
    for (c, plane) in planes.chunks_exact(pixels).enumerate() {
        for (i, &v) in plane.iter().enumerate() {
            out[i * channels + c] = v;
        }
    }
    Ok(out)
}

fn check_len<T>(src: &[T], want: usize) -> Result<(), DecodeError> {
    if src.len() == want {
        Ok(())
    } else {
        Err(DecodeError::Corrupt {
            format: FORMAT,
            message: format!("{} samples for {want}", src.len()),
        })
    }
}

/// Associated (premultiplied) alpha to straight alpha, rounded.
fn unpremultiply(out: &mut [u16]) {
    for px in out.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        for c in &mut px[..3] {
            *c = (u32::from(*c) * 65535 + a / 2)
                .checked_div(a)
                .map_or(0, |v| v.min(65535) as u16);
        }
    }
}
