//! The few EXIF facts decoding needs, read from the TIFF structure inside an
//! EXIF block (JPEG APP1, PNG eXIf) with the crate's bounded TIFF reader.

use crate::tiff_ifd::{Tiff, find};

const ORIENTATION: u16 = 274;
const EXIF_IFD: u16 = 34665;
const COLOR_SPACE: u16 = 0xA001;
const INTEROP_IFD: u16 = 0xA005;
const INTEROP_INDEX: u16 = 0x0001;

/// The TIFF structure of an EXIF block: JPEG's APP1 starts with
/// `Exif\0\0`, PNG's eXIf does not.
pub(crate) fn tiff_payload(exif: &[u8]) -> &[u8] {
    exif.strip_prefix(b"Exif\0\0").unwrap_or(exif)
}

/// The orientation in IFD0, 1 to 8; 1 when absent, invalid or unreadable.
pub(crate) fn orientation(tiff: &[u8]) -> u8 {
    (|| {
        let t = Tiff::new(tiff)?;
        let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
        match t.first_int(&ifd0, ORIENTATION)? {
            v @ 1..=8 => Some(v as u8),
            _ => None,
        }
    })()
    .unwrap_or(1)
}

/// Whether the EXIF block declares Adobe RGB the way DCF cameras do when
/// they embed no ICC profile: ColorSpace 0xFFFF (uncalibrated) and the
/// Interoperability index "R03".
pub(crate) fn is_adobe_rgb(tiff: &[u8]) -> bool {
    (|| {
        let t = Tiff::new(tiff)?;
        let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
        let exif = t.ifd(t.first_int(&ifd0, EXIF_IFD)? as usize).ok()?;
        if t.first_int(&exif, COLOR_SPACE)? != 0xFFFF {
            return Some(false);
        }
        let interop = t.ifd(t.first_int(&exif, INTEROP_IFD)? as usize).ok()?;
        let index = t.blob(find(&interop, INTEROP_INDEX)?).ok()?;
        Some(index.starts_with(b"R03"))
    })()
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TIFF structure with one IFD0 entry: Orientation = `v`.
    fn exif(v: u16, big_endian: bool) -> Vec<u8> {
        let mut out = if big_endian {
            b"MM\0*\0\0\0\x08".to_vec()
        } else {
            b"II*\0\x08\0\0\0".to_vec()
        };
        let u16b = |x: u16| {
            if big_endian {
                x.to_be_bytes()
            } else {
                x.to_le_bytes()
            }
        };
        let u32b = |x: u32| {
            if big_endian {
                x.to_be_bytes()
            } else {
                x.to_le_bytes()
            }
        };
        out.extend(u16b(1));
        out.extend(u16b(ORIENTATION));
        out.extend(u16b(3));
        out.extend(u32b(1));
        out.extend(u16b(v));
        out.extend([0, 0]);
        out.extend(u32b(0));
        out
    }

    #[test]
    fn reads_every_orientation_in_both_byte_orders() {
        for be in [false, true] {
            for v in 1..=8 {
                assert_eq!(orientation(&exif(v, be)), v as u8);
            }
            assert_eq!(orientation(&exif(0, be)), 1);
            assert_eq!(orientation(&exif(9, be)), 1);
        }
        let mut with_prefix = b"Exif\0\0".to_vec();
        with_prefix.extend(exif(6, true));
        assert_eq!(orientation(tiff_payload(&with_prefix)), 6);
    }

    #[test]
    fn damaged_exif_never_panics() {
        let e = exif(6, false);
        for len in 0..e.len() {
            let _ = orientation(&e[..len]);
            let _ = is_adobe_rgb(&e[..len]);
        }
        assert!(!is_adobe_rgb(&e));
    }
}
