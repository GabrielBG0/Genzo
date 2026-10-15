//! JPEG encoding of display frames.
//!
//! Used for drag previews, where the frame is replaced by a lossless one as
//! soon as the slider is released. Not for export; that comes with M1.

use jpeg_encoder::{ColorType, Encoder, EncodingError};

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
