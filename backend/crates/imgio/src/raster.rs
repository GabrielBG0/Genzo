//! What the non-RAW decoders share: the size limit checked before any
//! allocation, and widening decoded samples to the RGBA16 every non-RAW
//! format hands the pipeline.

use crate::{DecodeError, FileFormat};

/// Largest image accepted, in pixels, as for RAW files. At 8 bytes a pixel
/// (RGBA16) the output tops out at 3.2 GB.
pub(crate) const MAX_PIXELS: u64 = 400_000_000;

/// Checks a size read from a header, before anything is allocated.
pub(crate) fn check_size(
    format: FileFormat,
    width: u64,
    height: u64,
) -> Result<usize, DecodeError> {
    let pixels = width.saturating_mul(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(DecodeError::Corrupt {
            format,
            message: format!("an image of {width}x{height}"),
        });
    }
    Ok(pixels as usize)
}

/// Checks that a file of `file_len` bytes can hold the image it claims,
/// before anything is allocated: a decoded size of `decoded_bytes` needs at
/// least `decoded_bytes / max_ratio` bytes of file. Each format's
/// `max_ratio` is about twice its compression's ceiling, so real files sit
/// far inside it, while a 2 kB header claiming 387 MP (a fuzz find) does not.
pub(crate) fn check_plausible(
    format: FileFormat,
    file_len: usize,
    decoded_bytes: u64,
    max_ratio: u64,
) -> Result<(), DecodeError> {
    if (file_len as u64).saturating_mul(max_ratio) < decoded_bytes {
        return Err(DecodeError::Corrupt {
            format,
            message: format!("{file_len} bytes cannot hold {decoded_bytes} bytes of image"),
        });
    }
    Ok(())
}

/// A zeroed RGBA16 buffer for `pixels`, or an error when memory is short.
pub(crate) fn rgba16(format: FileFormat, pixels: usize) -> Result<Vec<u16>, DecodeError> {
    let mut out = Vec::new();
    out.try_reserve_exact(pixels * 4)
        .map_err(|_| DecodeError::Corrupt {
            format,
            message: format!("no memory for {pixels} pixels"),
        })?;
    out.resize(pixels * 4, 0);
    Ok(out)
}

/// An 8-bit sample widened to 16 bits (0 to 0, 255 to 65535).
#[inline]
pub(crate) fn widen8(v: u8) -> u16 {
    u16::from(v) * 257
}

/// Fills RGBA16 from interleaved 8-bit samples with `channels` per pixel:
/// 1 gray, 2 gray and alpha, 3 RGB, 4 RGBA.
pub(crate) fn from8(out: &mut [u16], src: &[u8], channels: usize) {
    for (o, s) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.chunks_exact(channels))
    {
        let px = match *s {
            [g] => [g, g, g, 255],
            [g, a] => [g, g, g, a],
            [r, g, b] => [r, g, b, 255],
            [r, g, b, a] => [r, g, b, a],
            _ => unreachable!("1 to 4 channels"),
        };
        for (o, v) in o.iter_mut().zip(px) {
            *o = widen8(v);
        }
    }
}

/// As `from8`, for 16-bit samples.
pub(crate) fn from16(out: &mut [u16], src: &[u16], channels: usize) {
    for (o, s) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.chunks_exact(channels))
    {
        let px = match *s {
            [g] => [g, g, g, u16::MAX],
            [g, a] => [g, g, g, a],
            [r, g, b] => [r, g, b, u16::MAX],
            [r, g, b, a] => [r, g, b, a],
            _ => unreachable!("1 to 4 channels"),
        };
        *o = px;
    }
}
