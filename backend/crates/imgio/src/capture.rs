//! When a photo was taken (LIB-09: the filmstrip's order): EXIF
//! DateTimeOriginal with SubSecTimeOriginal, read from the file's metadata
//! only, never its pixels, so a folder of thousands lists quickly.

use crate::tiff_ifd::{Tiff, find};
use crate::{FileFormat, exif, sniff};

const EXIF_IFD: u16 = 34665;
const DATE_TIME_ORIGINAL: u16 = 0x9003;
const SUB_SEC_TIME_ORIGINAL: u16 = 0x9291;

/// A capture time as the camera wrote it: local time, no zone. Ordered
/// field by field, so sorting by it is sorting by time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// From SubSecTimeOriginal, 0 when the file has none.
    pub nanos: u32,
}

/// The file's capture time, or None when it has none, it is unreadable or
/// the format is unknown. Never panics, whatever the input.
pub fn capture_time(data: &[u8]) -> Option<CaptureTime> {
    match sniff(data)? {
        FileFormat::Jpeg => from_tiff(exif::tiff_payload(crate::jpeg_app1_exif(data)?), true),
        // DNG and the TIFF-based RAW formats; ORF and RW2 have their own
        // magic but the same structure.
        FileFormat::Tiff | FileFormat::Orf | FileFormat::Rw2 => from_tiff(data, true),
        // A CR3's CMT2 box is the EXIF IFD as a TIFF structure of its own.
        FileFormat::Cr3 => from_tiff(crate::cr3_box(data, b"CMT2")?, false),
        FileFormat::Png => from_tiff(crate::png_image::exif_chunk(data)?, true),
        FileFormat::Raf => {
            // The RAF header points to the embedded JPEG (offset at 84,
            // length at 88, big-endian), whose EXIF holds the camera's. Its
            // APP1 comes first, so a file's head is enough: the JPEG is cut
            // at what `data` holds.
            let at = u32::from_be_bytes(data.get(84..88)?.try_into().ok()?) as usize;
            let len = u32::from_be_bytes(data.get(88..92)?.try_into().ok()?) as usize;
            let end = at.checked_add(len)?.min(data.len());
            let jpeg = data.get(at..end)?;
            from_tiff(exif::tiff_payload(crate::jpeg_app1_exif(jpeg)?), true)
        }
        FileFormat::Heif => from_tiff(exif::tiff_payload(&crate::heic::exif(data)?), true),
    }
}

/// The capture time in a TIFF structure: in the IFD IFD0's ExifIFD tag
/// points to when `via_exif_ifd`, else in IFD0 itself.
fn from_tiff(tiff: &[u8], via_exif_ifd: bool) -> Option<CaptureTime> {
    let t = Tiff::new(tiff)?;
    let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
    let exif = if via_exif_ifd {
        t.ifd(t.first_int(&ifd0, EXIF_IFD)? as usize).ok()?
    } else {
        ifd0
    };
    let text = t.blob(find(&exif, DATE_TIME_ORIGINAL)?).ok()?;
    let sub = find(&exif, SUB_SEC_TIME_ORIGINAL).and_then(|e| t.blob(e).ok());
    parse(&text, sub.as_deref())
}

/// EXIF's `YYYY:MM:DD HH:MM:SS`, with its sub-seconds as written (the
/// digits of a decimal fraction). An all-zero or blank date is no date.
pub(crate) fn parse(text: &[u8], sub: Option<&[u8]>) -> Option<CaptureTime> {
    let text = text.get(..19)?;
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let s = std::str::from_utf8(text.get(r)?).ok()?;
        s.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let seps = [(4, b':'), (7, b':'), (10, b' '), (13, b':'), (16, b':')];
    if seps.iter().any(|&(i, c)| text[i] != c) {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 60 is a leap second.
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // The digits of a decimal fraction of a second, to nanoseconds.
    let nanos = sub.map_or(0, |s| {
        let digits = s.iter().take_while(|b| b.is_ascii_digit()).take(9);
        let (mut n, mut k) = (0u32, 0u32);
        for d in digits {
            n = n * 10 + u32::from(d - b'0');
            k += 1;
        }
        n * 10u32.pow(9 - k)
    });
    Some(CaptureTime {
        year: year as u16,
        month: month as u8,
        day: day as u8,
        hour: hour as u8,
        minute: minute as u8,
        second: second as u8,
        nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TIFF structure: IFD0 with an ExifIFD pointer, the EXIF IFD with
    /// DateTimeOriginal and, if given, SubSecTimeOriginal.
    fn tiff(date: &[u8; 20], sub: Option<&[u8]>, big_endian: bool) -> Vec<u8> {
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
        let mut out = if big_endian {
            b"MM\0*".to_vec()
        } else {
            b"II*\0".to_vec()
        };
        out.extend(u32b(8));
        // IFD0 at 8: one entry, ExifIFD (LONG) to 26.
        out.extend(u16b(1));
        out.extend(u16b(EXIF_IFD));
        out.extend(u16b(4));
        out.extend(u32b(1));
        out.extend(u32b(26));
        out.extend(u32b(0));
        assert_eq!(out.len(), 26);
        // The EXIF IFD at 26.
        let entries = 1 + u16::from(sub.is_some());
        let data_at = 26 + 2 + 12 * u32::from(entries) + 4;
        out.extend(u16b(entries));
        out.extend(u16b(DATE_TIME_ORIGINAL));
        out.extend(u16b(2));
        out.extend(u32b(20));
        out.extend(u32b(data_at));
        if let Some(s) = sub {
            out.extend(u16b(SUB_SEC_TIME_ORIGINAL));
            out.extend(u16b(2));
            out.extend(u32b(s.len() as u32));
            let mut v = [0u8; 4];
            v[..s.len().min(4)].copy_from_slice(&s[..s.len().min(4)]);
            out.extend(v);
        }
        out.extend(u32b(0));
        out.extend(date);
        out
    }

    #[test]
    fn reads_the_capture_time_in_both_byte_orders() {
        for be in [false, true] {
            let t = tiff(b"2024:07:19 18:03:42\0", Some(b"25\0"), be);
            let c = from_tiff(&t, true).unwrap();
            assert_eq!(
                (c.year, c.month, c.day, c.hour, c.minute, c.second, c.nanos),
                (2024, 7, 19, 18, 3, 42, 250_000_000)
            );
            let plain = from_tiff(&tiff(b"2024:07:19 18:03:42\0", None, be), true).unwrap();
            assert_eq!(plain.nanos, 0);
            assert!(plain < c, "sub-seconds order a burst");
        }
    }

    #[test]
    fn bad_dates_are_no_date() {
        for text in [
            &b"0000:00:00 00:00:00"[..],
            b"                   ",
            b"2024-07-19 18:03:42",
            b"2024:13:19 18:03:42",
            b"2024:07:19 24:03:42",
            b"2024:07:1",
        ] {
            assert_eq!(parse(text, None), None, "{}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn damaged_input_never_panics() {
        let t = tiff(b"2024:07:19 18:03:42\0", Some(b"123"), false);
        for len in 0..t.len() {
            let _ = from_tiff(&t[..len], true);
            let _ = capture_time(&t[..len]);
        }
        assert!(capture_time(&t).is_some(), "a TIFF file");
    }
}
