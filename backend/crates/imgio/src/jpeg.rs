//! JPEG: decoding photos (zune-jpeg) and encoding display frames.
//!
//! Encoding is used for drag previews, where the frame is replaced by a
//! lossless one as soon as the slider is released. Not for export; that
//! comes with M1.

use jpeg_encoder::{ColorType, Encoder, EncodingError};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;
use zune_jpeg::errors::DecodeErrors;

use crate::{ColorEncoding, DecodeError, DecodedImage, FileFormat, Pixels, exif, raster};

/// JPEG's own limit on each side.
const MAX_SIDE: usize = 65_535;

fn jpeg_error(e: DecodeErrors) -> DecodeError {
    let format = FileFormat::Jpeg;
    match e {
        DecodeErrors::Unsupported(scheme) => {
            DecodeError::Unsupported(format, format!("{scheme:?}"))
        }
        other => DecodeError::Corrupt {
            format,
            message: format!("{other:?}"),
        },
    }
}

/// Decodes a JPEG to RGBA16. CMYK and YCCK JPEGs are unsupported (DEV-01);
/// grayscale comes out as equal R, G and B. The orientation is reported,
/// not applied.
pub(crate) fn decode(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    let format = FileFormat::Jpeg;
    let options = DecoderOptions::default()
        .set_max_width(MAX_SIDE)
        .set_max_height(MAX_SIDE)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder.decode_headers().map_err(jpeg_error)?;
    let info = decoder.info().ok_or_else(|| DecodeError::Corrupt {
        format,
        message: "no frame header".into(),
    })?;
    match decoder.input_colorspace() {
        Some(ColorSpace::CMYK | ColorSpace::YCCK) => {
            return Err(DecodeError::Unsupported(format, "CMYK JPEG".into()));
        }
        Some(ColorSpace::RGB | ColorSpace::YCbCr | ColorSpace::Luma) => {}
        other => {
            return Err(DecodeError::Unsupported(
                format,
                format!("JPEG color space {other:?}"),
            ));
        }
    }
    let pixels = raster::check_size(format, u64::from(info.width), u64::from(info.height))?;
    // Every 8 x 8 luma block costs at least one bit, so a valid file holds at
    // most 512 pixels per byte; 1024 leaves a margin.
    raster::check_plausible(format, data.len(), pixels as u64, 1024)?;
    let size = decoder
        .output_buffer_size()
        .ok_or_else(|| DecodeError::Corrupt {
            format,
            message: "no output size".into(),
        })?;
    if size != pixels * 3 {
        return Err(DecodeError::Corrupt {
            format,
            message: format!("{size} output bytes for {pixels} pixels"),
        });
    }
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(size)
        .map_err(|_| DecodeError::Corrupt {
            format,
            message: format!("no memory for {pixels} pixels"),
        })?;
    rgb.resize(size, 0);
    decoder.decode_into(&mut rgb).map_err(jpeg_error)?;
    let mut out = raster::rgba16(format, pixels)?;
    raster::from8(&mut out, &rgb, 3);
    drop(rgb);

    let exif = decoder
        .exif()
        .map(|e| exif::tiff_payload(e).to_vec())
        .filter(|e| !e.is_empty());
    let color = match decoder.icc_profile() {
        Some(icc) if !icc.is_empty() => ColorEncoding::Icc(icc),
        _ if exif.as_deref().is_some_and(exif::is_adobe_rgb) => ColorEncoding::AdobeRgb,
        // JPEG without a profile is taken as sRGB.
        _ => ColorEncoding::Srgb,
    };
    Ok(DecodedImage {
        format,
        width: u32::from(info.width),
        height: u32::from(info.height),
        source_bits: 8,
        has_alpha: false,
        pixels: Pixels::Rgba16 { data: out, color },
        orientation: exif.as_deref().map_or(1, exif::orientation),
        exif,
    })
}

/// Encodes tightly packed RGBA8 pixels (alpha is ignored) as a baseline JPEG.
///
/// `quality` is 1 to 100. Returns an error for a zero or oversized dimension
/// or when `rgba` is not `width * height * 4` bytes.
pub fn encode_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, EncodingError> {
    let (w, h) = (
        u16::try_from(width).map_err(|_| EncodingError::BadImageData {
            length: rgba.len(),
            required: width as usize * height as usize * 4,
        })?,
        u16::try_from(height).map_err(|_| EncodingError::BadImageData {
            length: rgba.len(),
            required: width as usize * height as usize * 4,
        })?,
    );
    let mut out = Vec::with_capacity(rgba.len() / 8);
    Encoder::new(&mut out, quality.clamp(1, 100)).encode(rgba, w, h, ColorType::Rgba)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_valid_jpeg() {
        let rgba: Vec<u8> = (0..32 * 16)
            .flat_map(|i| [(i % 256) as u8, (i / 4 % 256) as u8, 128, 255])
            .collect();
        let jpeg = encode_rgba(&rgba, 32, 16, 80).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "starts with SOI");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "ends with EOI");
        assert!(jpeg.len() < rgba.len());
    }

    #[test]
    fn rejects_wrong_length_and_empty_size() {
        assert!(encode_rgba(&[0; 15], 2, 2, 80).is_err());
        assert!(encode_rgba(&[], 0, 0, 80).is_err());
        assert!(encode_rgba(&[0; 4], 70000, 1, 80).is_err());
    }
}
