//! When and how a photo was taken, from its EXIF IFD, read from the file's
//! metadata only, never its pixels, so a folder of thousands lists quickly:
//! DateTimeOriginal with SubSecTimeOriginal (LIB-09: the filmstrip's
//! order), and ISO, exposure time, f-number and focal length (DEV-03: the
//! histogram's EXIF row).

use crate::tiff_ifd::{Entry, Tiff, find};
use crate::{FileFormat, exif, sniff};

const EXIF_IFD: u16 = 34665;
const DATE_TIME_ORIGINAL: u16 = 0x9003;
const SUB_SEC_TIME_ORIGINAL: u16 = 0x9291;
const EXPOSURE_TIME: u16 = 33434;
const F_NUMBER: u16 = 33437;
const PHOTOGRAPHIC_SENSITIVITY: u16 = 34855;
const FOCAL_LENGTH: u16 = 37386;

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

/// A positive EXIF rational as the file stores it, unreduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: u32,
    /// Never 0.
    pub den: u32,
}

impl Rational {
    pub fn value(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

/// How a photo was shot. A value the file does not carry, or carries as 0
/// or with a zero denominator, is None.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Shooting {
    /// PhotographicSensitivity, its first value when it holds several.
    pub iso: Option<u32>,
    /// In seconds.
    pub exposure_time: Option<Rational>,
    pub f_number: Option<Rational>,
    /// In millimeters, the lens's own, not the 35 mm equivalent.
    pub focal_length: Option<Rational>,
}

/// What a file's EXIF says of its capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Capture {
    pub time: Option<CaptureTime>,
    pub shooting: Shooting,
}

impl Capture {
    /// Whether nothing was found.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The file's capture time, or None when it has none, it is unreadable or
/// the format is unknown. Never panics, whatever the input.
pub fn capture_time(data: &[u8]) -> Option<CaptureTime> {
    capture(data).time
}

/// The file's capture time and shooting values, each empty when the file
/// has none, it is unreadable or the format is unknown. Never panics,
/// whatever the input.
pub fn capture(data: &[u8]) -> Capture {
    read(data).unwrap_or_default()
}

fn read(data: &[u8]) -> Option<Capture> {
    match sniff(data)? {
        FileFormat::Jpeg => from_tiff(exif::tiff_payload(crate::jpeg_app1_exif(data)?), true),
        // DNG and the TIFF-based RAW formats; ORF and RW2 have their own
        // magic but the same structure.
        FileFormat::Tiff | FileFormat::Orf | FileFormat::Rw2 => from_tiff(data, true),
        // A CR3's CMT2 box is the EXIF IFD as a TIFF structure of its own.
        FileFormat::Cr3 => from_tiff(crate::cr3_box(data, b"CMT2")?, false),
        FileFormat::Png => from_tiff(
            exif::tiff_payload(crate::png_image::exif_chunk(data)?),
            true,
        ),
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

/// Whether a file of `head`'s format can keep its capture time past its
/// head, so a reader that found none there should read the rest: TIFF
/// structures point anywhere, a HEIF's EXIF item and a PNG's eXIf chunk
/// can follow the pixels. A JPEG's APP1 comes first, a CR3's CMT2 box is
/// searched in the first MiB only, and a RAF's JPEG starts in its header.
pub fn can_lie_past_the_head(head: &[u8]) -> bool {
    matches!(
        sniff(head),
        Some(
            FileFormat::Tiff
                | FileFormat::Orf
                | FileFormat::Rw2
                | FileFormat::Heif
                | FileFormat::Png
        )
    )
}

/// The capture in a TIFF structure: in the IFD IFD0's ExifIFD tag points
/// to when `via_exif_ifd`, else in IFD0 itself.
fn from_tiff(tiff: &[u8], via_exif_ifd: bool) -> Option<Capture> {
    let t = Tiff::new(tiff)?;
    let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
    let exif = if via_exif_ifd {
        t.ifd(t.first_int(&ifd0, EXIF_IFD)? as usize).ok()?
    } else {
        ifd0
    };
    let time = find(&exif, DATE_TIME_ORIGINAL)
        .and_then(|e| t.blob(e).ok())
        .and_then(|text| {
            let sub = find(&exif, SUB_SEC_TIME_ORIGINAL).and_then(|e| t.blob(e).ok());
            parse(&text, sub.as_deref())
        });
    let shooting = Shooting {
        iso: t
            .first_int(&exif, PHOTOGRAPHIC_SENSITIVITY)
            .filter(|&iso| iso > 0),
        exposure_time: rational(&t, &exif, EXPOSURE_TIME),
        f_number: rational(&t, &exif, F_NUMBER),
        focal_length: rational(&t, &exif, FOCAL_LENGTH),
    };
    Some(Capture { time, shooting })
}

/// The first value of a RATIONAL entry, or of an integer one written
/// where EXIF asks for a rational. None when absent, 0 or over 0.
fn rational(t: &Tiff, ifd: &[Entry], tag: u16) -> Option<Rational> {
    let e = find(ifd, tag)?;
    let (num, den) = match e.ty {
        5 => {
            let (base, _) = t.location(e).ok()?;
            let b = t.raw::<8>(e, base, 0)?;
            let num = t.uint::<4>([b[0], b[1], b[2], b[3]]) as u32;
            let den = t.uint::<4>([b[4], b[5], b[6], b[7]]) as u32;
            (num, den)
        }
        1 | 3 | 4 => (*t.ints(e).ok()?.first()?, 1),
        _ => return None,
    };
    (num > 0 && den > 0).then_some(Rational { num, den })
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
            let c = from_tiff(&t, true).unwrap().time.unwrap();
            assert_eq!(
                (c.year, c.month, c.day, c.hour, c.minute, c.second, c.nanos),
                (2024, 7, 19, 18, 3, 42, 250_000_000)
            );
            let plain = from_tiff(&tiff(b"2024:07:19 18:03:42\0", None, be), true)
                .unwrap()
                .time
                .unwrap();
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
    fn a_png_exif_chunk_reads_with_or_without_its_prefix() {
        let chunk = |kind: &[u8; 4], body: &[u8]| {
            let mut c = (body.len() as u32).to_be_bytes().to_vec();
            c.extend(kind);
            c.extend(body);
            c.extend([0; 4]); // CRC, unchecked here
            c
        };
        let exif = tiff(b"2024:07:19 18:03:42\0", None, false);
        for prefix in [&b""[..], b"Exif\0\0"] {
            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            png.extend(chunk(b"IHDR", &[0; 13]));
            png.extend(chunk(b"IDAT", &[0; 8]));
            // After the pixels, where some writers put it.
            png.extend(chunk(b"eXIf", &[prefix, &exif].concat()));
            png.extend(chunk(b"IEND", &[]));
            let c = capture_time(&png).expect("dated");
            assert_eq!((c.year, c.second), (2024, 42), "prefix {prefix:?}");
            assert!(can_lie_past_the_head(&png));
        }
    }

    #[test]
    fn only_formats_with_metadata_anywhere_read_past_the_head() {
        let t = tiff(b"2024:07:19 18:03:42\0", None, false);
        assert!(can_lie_past_the_head(&t));
        assert!(!can_lie_past_the_head(b"\xFF\xD8\xFF\xE0 a JPEG"));
        assert!(!can_lie_past_the_head(b"\0\0\0\x18ftypcrx "));
        assert!(!can_lie_past_the_head(b"FUJIFILMCCD-RAW 0201"));
        assert!(!can_lie_past_the_head(b"not a photo"));
    }

    /// A TIFF structure: IFD0 with an ExifIFD pointer, then the EXIF IFD
    /// with `entries` (tag, type, count, value bytes in file order), their
    /// values past 4 bytes after it.
    fn exif_tiff(entries: &[(u16, u16, u32, Vec<u8>)], big_endian: bool) -> Vec<u8> {
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
        out.extend(u16b(1));
        out.extend(u16b(EXIF_IFD));
        out.extend(u16b(4));
        out.extend(u32b(1));
        out.extend(u32b(26));
        out.extend(u32b(0));
        let mut data_at = 26 + 2 + 12 * entries.len() as u32 + 4;
        let mut data: Vec<u8> = Vec::new();
        out.extend(u16b(entries.len() as u16));
        for (tag, ty, count, value) in entries {
            out.extend(u16b(*tag));
            out.extend(u16b(*ty));
            out.extend(u32b(*count));
            if value.len() <= 4 {
                let mut v = value.clone();
                v.resize(4, 0);
                out.extend(v);
            } else {
                out.extend(u32b(data_at));
                data_at += value.len() as u32;
                data.extend(value);
            }
        }
        out.extend(u32b(0));
        out.extend(data);
        out
    }

    fn rational_bytes(num: u32, den: u32, big_endian: bool) -> Vec<u8> {
        let b = |x: u32| {
            if big_endian {
                x.to_be_bytes()
            } else {
                x.to_le_bytes()
            }
        };
        [b(num), b(den)].concat()
    }

    #[test]
    fn reads_how_a_photo_was_shot_in_both_byte_orders() {
        for be in [false, true] {
            let short = |x: u16| if be { x.to_be_bytes() } else { x.to_le_bytes() }.to_vec();
            let t = exif_tiff(
                &[
                    (EXPOSURE_TIME, 5, 1, rational_bytes(1, 250, be)),
                    (F_NUMBER, 5, 1, rational_bytes(28, 10, be)),
                    // Two values: the first is the ISO.
                    (
                        PHOTOGRAPHIC_SENSITIVITY,
                        3,
                        2,
                        [short(400), short(800)].concat(),
                    ),
                    (FOCAL_LENGTH, 5, 1, rational_bytes(686, 100, be)),
                ],
                be,
            );
            let c = capture(&t);
            assert_eq!(c.time, None, "no date");
            let s = c.shooting;
            assert_eq!(s.iso, Some(400));
            assert_eq!(s.exposure_time, Some(Rational { num: 1, den: 250 }));
            assert_eq!(s.f_number.map(Rational::value), Some(2.8));
            assert_eq!(s.focal_length, Some(Rational { num: 686, den: 100 }));
        }
    }

    #[test]
    fn zero_values_and_odd_types_are_left_out() {
        let t = exif_tiff(
            &[
                (EXPOSURE_TIME, 5, 1, rational_bytes(0, 1, false)),
                (F_NUMBER, 5, 1, rational_bytes(28, 0, false)),
                (PHOTOGRAPHIC_SENSITIVITY, 3, 1, 0u16.to_le_bytes().to_vec()),
                // A SHORT where EXIF asks for a rational still reads.
                (FOCAL_LENGTH, 3, 1, 50u16.to_le_bytes().to_vec()),
            ],
            false,
        );
        let s = capture(&t).shooting;
        assert_eq!((s.iso, s.exposure_time, s.f_number), (None, None, None));
        assert_eq!(s.focal_length, Some(Rational { num: 50, den: 1 }));
        // An ASCII value where a rational belongs is no value.
        let text = exif_tiff(&[(F_NUMBER, 2, 4, b"2.8\0".to_vec())], false);
        assert_eq!(capture(&text).shooting.f_number, None);
        assert!(capture(b"not a photo").is_empty());
    }

    #[test]
    fn the_date_and_the_values_come_together() {
        let t = exif_tiff(
            &[
                (EXPOSURE_TIME, 5, 1, rational_bytes(1, 8000, false)),
                (DATE_TIME_ORIGINAL, 2, 20, b"2024:07:19 18:03:42\0".to_vec()),
            ],
            false,
        );
        let c = capture(&t);
        assert_eq!(c.time.map(|t| t.second), Some(42));
        assert_eq!(
            c.shooting.exposure_time,
            Some(Rational { num: 1, den: 8000 })
        );
        assert_eq!(capture_time(&t), c.time);
    }

    #[test]
    fn damaged_input_never_panics() {
        let t = tiff(b"2024:07:19 18:03:42\0", Some(b"123"), false);
        for len in 0..t.len() {
            let _ = from_tiff(&t[..len], true);
            let _ = capture_time(&t[..len]);
        }
        let shot = exif_tiff(
            &[
                (EXPOSURE_TIME, 5, 1, rational_bytes(1, 250, false)),
                (FOCAL_LENGTH, 5, 3, rational_bytes(24, 1, false).repeat(3)),
            ],
            false,
        );
        for len in 0..shot.len() {
            let _ = capture(&shot[..len]);
        }
        assert!(capture_time(&t).is_some(), "a TIFF file");
    }
}
