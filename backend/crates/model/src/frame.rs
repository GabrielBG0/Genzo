//! The binary tile protocol.
//!
//! Pixels never travel as JSON. Each tile is one binary message: a fixed
//! little-endian header followed by the pixel payload.
//!
//! | Offset | Size | Field          |
//! |-------:|-----:|----------------|
//! |      0 |    4 | magic `GZT1`   |
//! |      4 |    2 | header length  |
//! |      6 |    1 | pixel format   |
//! |      7 |    1 | mip level      |
//! |      8 |    8 | generation     |
//! |     16 |    4 | x              |
//! |     20 |    4 | y              |
//! |     24 |    4 | width          |
//! |     28 |    4 | height         |
//! |     32 |    4 | image width    |
//! |     36 |    4 | image height   |
//! |     40 |    4 | payload length |
//!
//! For the raw formats the payload is `width * height * 4` bytes. For the JPEG
//! formats it is one baseline JPEG of exactly `width` × `height` pixels, so its
//! length varies; the header's payload length is the only way to know it.
//!
//! `x`, `y`, `width` and `height` are in pixels of the given mip level.
//! `image width` and `image height` are the full image at that mip level.
//! Readers skip `header length` bytes before the payload, so the header can
//! grow without breaking old readers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const TILE_MAGIC: [u8; 4] = *b"GZT1";
pub const TILE_HEADER_LEN: usize = 44;
/// Tile edge length at 1:1, in pixels.
pub const TILE_SIZE: u32 = 256;

/// Pixel format of a tile payload, with the color space the webview should
/// assume when it converts to the monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PixelFormat {
    /// 8-bit RGBA, sRGB encoded, straight alpha.
    Rgba8Srgb,
    /// 8-bit RGBA, Display P3 encoded, straight alpha.
    Rgba8DisplayP3,
    /// A baseline JPEG, sRGB encoded. Used for drag previews, where a small
    /// payload matters more than exact pixels.
    JpegSrgb,
    /// A baseline JPEG, Display P3 encoded (no ICC profile inside).
    JpegDisplayP3,
}

impl PixelFormat {
    pub const fn code(self) -> u8 {
        match self {
            PixelFormat::Rgba8Srgb => 1,
            PixelFormat::Rgba8DisplayP3 => 2,
            PixelFormat::JpegSrgb => 3,
            PixelFormat::JpegDisplayP3 => 4,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(PixelFormat::Rgba8Srgb),
            2 => Some(PixelFormat::Rgba8DisplayP3),
            3 => Some(PixelFormat::JpegSrgb),
            4 => Some(PixelFormat::JpegDisplayP3),
            _ => None,
        }
    }

    /// Bytes per pixel of an uncompressed format, `None` for the JPEG ones.
    pub const fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            PixelFormat::Rgba8Srgb | PixelFormat::Rgba8DisplayP3 => Some(4),
            PixelFormat::JpegSrgb | PixelFormat::JpegDisplayP3 => None,
        }
    }

    pub const fn is_jpeg(self) -> bool {
        self.bytes_per_pixel().is_none()
    }

    /// The same color space as a JPEG payload.
    pub const fn as_jpeg(self) -> Self {
        match self {
            PixelFormat::Rgba8Srgb | PixelFormat::JpegSrgb => PixelFormat::JpegSrgb,
            PixelFormat::Rgba8DisplayP3 | PixelFormat::JpegDisplayP3 => PixelFormat::JpegDisplayP3,
        }
    }
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The decoded header of one tile message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TileHeader {
    /// Render generation. The frontend drops tiles older than the newest
    /// generation it has seen.
    pub generation: u64,
    pub rect: Rect,
    pub mip_level: u8,
    pub format: PixelFormat,
    pub image_width: u32,
    pub image_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TileDecodeError {
    TooShort,
    BadMagic,
    UnknownFormat(u8),
    PayloadLength { expected: usize, actual: usize },
}

impl std::fmt::Display for TileDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "tile message shorter than its header"),
            Self::BadMagic => write!(f, "tile message has the wrong magic"),
            Self::UnknownFormat(c) => write!(f, "unknown pixel format code {c}"),
            Self::PayloadLength { expected, actual } => {
                write!(f, "tile payload is {actual} bytes, expected {expected}")
            }
        }
    }
}

impl std::error::Error for TileDecodeError {}

impl TileHeader {
    /// Payload size implied by the rectangle and format, or `None` for the
    /// JPEG formats, whose payload length varies.
    pub fn payload_len(&self) -> Option<usize> {
        self.format
            .bytes_per_pixel()
            .map(|bpp| self.rect.width as usize * self.rect.height as usize * bpp)
    }

    /// Writes the header followed by `payload` into one message.
    ///
    /// Panics if `payload` does not match [`TileHeader::payload_len`].
    pub fn encode(&self, payload: &[u8]) -> Vec<u8> {
        if let Some(expected) = self.payload_len() {
            assert_eq!(payload.len(), expected, "tile payload size");
        }
        let mut out = Vec::with_capacity(TILE_HEADER_LEN + payload.len());
        out.extend_from_slice(&TILE_MAGIC);
        out.extend_from_slice(&(TILE_HEADER_LEN as u16).to_le_bytes());
        out.push(self.format.code());
        out.push(self.mip_level);
        out.extend_from_slice(&self.generation.to_le_bytes());
        for v in [
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.image_width,
            self.image_height,
            payload.len() as u32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        debug_assert_eq!(out.len(), TILE_HEADER_LEN);
        out.extend_from_slice(payload);
        out
    }

    /// Splits a message into its header and payload.
    pub fn decode(msg: &[u8]) -> Result<(Self, &[u8]), TileDecodeError> {
        if msg.len() < TILE_HEADER_LEN {
            return Err(TileDecodeError::TooShort);
        }
        if msg[0..4] != TILE_MAGIC {
            return Err(TileDecodeError::BadMagic);
        }
        let u32_at = |o: usize| u32::from_le_bytes(msg[o..o + 4].try_into().unwrap());
        let header_len = u16::from_le_bytes([msg[4], msg[5]]) as usize;
        if header_len < TILE_HEADER_LEN || msg.len() < header_len {
            return Err(TileDecodeError::TooShort);
        }
        let format =
            PixelFormat::from_code(msg[6]).ok_or(TileDecodeError::UnknownFormat(msg[6]))?;
        let header = TileHeader {
            generation: u64::from_le_bytes(msg[8..16].try_into().unwrap()),
            rect: Rect {
                x: u32_at(16),
                y: u32_at(20),
                width: u32_at(24),
                height: u32_at(28),
            },
            mip_level: msg[7],
            format,
            image_width: u32_at(32),
            image_height: u32_at(36),
        };
        let payload = &msg[header_len..];
        let declared = u32_at(40) as usize;
        let expected = header.payload_len().unwrap_or(declared);
        if declared != expected || payload.len() != expected {
            return Err(TileDecodeError::PayloadLength {
                expected,
                actual: payload.len(),
            });
        }
        Ok((header, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> TileHeader {
        TileHeader {
            generation: 0x0102_0304_0506_0708,
            rect: Rect {
                x: 256,
                y: 512,
                width: 3,
                height: 2,
            },
            mip_level: 1,
            format: PixelFormat::Rgba8DisplayP3,
            image_width: 1280,
            image_height: 720,
        }
    }

    #[test]
    fn round_trip() {
        let h = header();
        let payload: Vec<u8> = (0..h.payload_len().unwrap() as u8).collect();
        let msg = h.encode(&payload);
        assert_eq!(msg.len(), TILE_HEADER_LEN + 24);
        let (back, body) = TileHeader::decode(&msg).unwrap();
        assert_eq!(back, h);
        assert_eq!(body, &payload[..]);
    }

    #[test]
    fn layout_is_stable() {
        let msg = header().encode(&[0; 24]);
        assert_eq!(&msg[0..4], b"GZT1");
        assert_eq!(&msg[4..6], &44u16.to_le_bytes());
        assert_eq!(msg[6], 2);
        assert_eq!(msg[7], 1);
        assert_eq!(&msg[8..16], &0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(&msg[40..44], &24u32.to_le_bytes());
    }

    #[test]
    fn jpeg_payload_has_its_own_length() {
        let h = TileHeader {
            format: PixelFormat::JpegSrgb,
            ..header()
        };
        assert_eq!(h.payload_len(), None);
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9, 7];
        let msg = h.encode(&jpeg);
        let (back, body) = TileHeader::decode(&msg).unwrap();
        assert_eq!(back, h);
        assert_eq!(body, &jpeg[..]);
        assert!(matches!(
            TileHeader::decode(&msg[..msg.len() - 1]),
            Err(TileDecodeError::PayloadLength { .. })
        ));
    }

    #[test]
    fn rejects_bad_input() {
        let msg = header().encode(&[0; 24]);
        assert_eq!(
            TileHeader::decode(&msg[..10]),
            Err(TileDecodeError::TooShort)
        );
        let mut bad = msg.clone();
        bad[0] = b'X';
        assert_eq!(TileHeader::decode(&bad), Err(TileDecodeError::BadMagic));
        let mut bad = msg.clone();
        bad[6] = 99;
        assert_eq!(
            TileHeader::decode(&bad),
            Err(TileDecodeError::UnknownFormat(99))
        );
        assert!(matches!(
            TileHeader::decode(&msg[..msg.len() - 1]),
            Err(TileDecodeError::PayloadLength { .. })
        ));
    }
}
