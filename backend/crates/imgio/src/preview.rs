//! Embedded previews (DEV-01, stage 0): the JPEG a camera or DNG writer
//! stores beside the raw data, shown while the full image decodes, and for
//! files Genzo cannot decode at all (Nikon High Efficiency, damaged files).
//!
//! The previews are found by their bytes, not by each format's container:
//! every baseline or progressive JPEG stream in the file is a candidate, its
//! header read without decoding. The same code then serves DNG, NEF, CR3,
//! ARW, RAF, ORF, RW2 and the rest, and still works on a file whose
//! container is damaged. Of the candidates, the smallest whose long edge is
//! at least `PREFERRED_EDGE` is decoded: big enough for a first frame, small
//! enough to decode within the budget. Without one, the largest.
//!
//! A false match in compressed raw data is guarded against three ways: the
//! header must hold DQT, SOF and SOS in order with sane values; the stream
//! is cut at its EOI, and must be large enough for the size it claims; and
//! zune-jpeg decodes it in strict mode, so a truncated stream fails and the
//! next candidate is tried.

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::{ColorEncoding, FileFormat, Preview, exif, raster};

/// The preview size preferred: the smallest candidate with a long edge of
/// at least this many pixels. Cameras store one of 1616 to 2016 px beside
/// the full-size JPEG; that decodes in a few milliseconds.
pub(crate) const PREFERRED_EDGE: u32 = 1280;

/// Largest candidate considered, in pixels.
const MAX_PIXELS: u64 = 50_000_000;

/// An embedded JPEG stream: where it lies and the size its header claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub start: usize,
    pub end: usize,
    pub width: u32,
    pub height: u32,
}

impl Candidate {
    fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }
    fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Reads a JPEG header at `start` (just past SOI) up to SOS; returns the
/// frame size and where the entropy data begin, if the header is sane.
fn header(data: &[u8], start: usize) -> Option<(u32, u32, usize)> {
    let mut at = start + 2;
    let (mut dqt, mut frame) = (false, None);
    for _ in 0..64 {
        // Markers may be preceded by fill bytes.
        while data.get(at) == Some(&0xFF) && data.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        let len = usize::from(u16::from_be_bytes([*data.get(at + 2)?, *data.get(at + 3)?]));
        if len < 2 || at + 2 + len > data.len() {
            return None;
        }
        let body = &data[at + 4..at + 2 + len];
        match marker {
            0xDB => dqt = true,
            // Baseline, extended and progressive Huffman DCT.
            0xC0..=0xC2 => {
                let (&precision, rest) = body.split_first()?;
                let h = u32::from(u16::from_be_bytes([*rest.first()?, *rest.get(1)?]));
                let w = u32::from(u16::from_be_bytes([*rest.get(2)?, *rest.get(3)?]));
                let n = usize::from(*rest.get(4)?);
                let components = rest.get(5..5 + 3 * n)?;
                let sane_sampling = components.as_chunks::<3>().0.iter().all(|c| {
                    let (hs, vs) = (c[1] >> 4, c[1] & 15);
                    (1..=4).contains(&hs) && (1..=4).contains(&vs)
                });
                if precision != 8 || !matches!(n, 1 | 3) || w == 0 || h == 0 || !sane_sampling {
                    return None;
                }
                frame = Some((w, h));
            }
            // Lossless, arithmetic or hierarchical frames: not a preview.
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None,
            0xDA => {
                let (w, h) = frame?;
                return dqt.then_some((w, h, at + 2 + len));
            }
            0xD9 => return None,
            _ => {}
        }
        at += 2 + len;
    }
    None
}

/// Where a stream's entropy data end: just past the first EOI. Stuffed
/// 0xFF bytes (FF 00), restart markers and later scans' headers (progressive)
/// are passed over.
fn stream_end(data: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        at += memchr::memchr(0xFF, data.get(at..)?)?;
        match *data.get(at + 1)? {
            0xD9 => return Some(at + 2),
            _ => at += 1,
        }
    }
}

/// Every embedded JPEG stream in `data`, in file order, except one at
/// offset 0 (a JPEG file's own image, which is not its preview).
pub(crate) fn candidates(data: &[u8]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let finder = memchr::memmem::Finder::new(&[0xFF, 0xD8, 0xFF]);
    for start in finder.find_iter(data) {
        if start == 0 {
            continue;
        }
        let Some((width, height, entropy)) = header(data, start) else {
            continue;
        };
        let Some(end) = stream_end(data, entropy) else {
            continue;
        };
        let c = Candidate {
            start,
            end,
            width,
            height,
        };
        // The stream must be able to hold the size it claims (at least a
        // bit per 8 x 8 block, as for JPEG files), and stay under the cap.
        let fits = raster::check_plausible(FileFormat::Jpeg, end - start, c.pixels(), 1024).is_ok();
        if fits && c.pixels() <= MAX_PIXELS {
            out.push(c);
        }
    }
    out
}

/// The candidates in the order they are tried: the smallest with a long
/// edge of at least `PREFERRED_EDGE` first, then larger ones, then the
/// smaller ones from largest down.
pub(crate) fn preference(mut c: Vec<Candidate>) -> Vec<Candidate> {
    c.sort_by_key(|c| {
        let big = c.long_edge() >= PREFERRED_EDGE;
        (
            !big,
            if big {
                c.pixels()
            } else {
                u64::MAX - c.pixels()
            },
        )
    });
    c.dedup_by_key(|c| (c.start, c.end));
    c
}

/// Decodes one candidate strictly, to RGBA8, with the color encoding its
/// own markers declare (`None` when they declare none).
fn decode_candidate(
    data: &[u8],
    c: Candidate,
) -> Option<(Vec<u8>, Option<ColorEncoding>, Option<u8>)> {
    let stream = &data[c.start..c.end];
    let options = DecoderOptions::default()
        .set_strict_mode(true)
        .set_max_width(65_535)
        .set_max_height(65_535)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(stream), options);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    if (u32::from(info.width), u32::from(info.height)) != (c.width, c.height) {
        return None;
    }
    if !matches!(
        decoder.input_colorspace(),
        Some(ColorSpace::YCbCr | ColorSpace::RGB | ColorSpace::Luma)
    ) {
        return None;
    }
    let rgb = decoder.decode().ok()?;
    if rgb.len() != c.pixels() as usize * 3 {
        return None;
    }
    let mut rgba = Vec::with_capacity(c.pixels() as usize * 4);
    for p in rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    let exif = decoder.exif().map(|e| exif::tiff_payload(e).to_vec());
    let color = match decoder.icc_profile() {
        Some(icc) if !icc.is_empty() => Some(ColorEncoding::Icc(icc)),
        _ if exif.as_deref().is_some_and(exif::is_adobe_rgb) => Some(ColorEncoding::AdobeRgb),
        _ => None,
    };
    let orientation = exif.as_deref().and_then(exif::orientation_tag);
    Some((rgba, color, orientation))
}

/// The file-level facts a preview takes when its own stream says nothing:
/// the orientation and color the file declares.
#[derive(Debug, Clone, Default)]
pub(crate) struct FileHints {
    pub orientation: Option<u8>,
    pub color: Option<ColorEncoding>,
}

/// The embedded preview of a file, by the rule above; `None` when the file
/// holds no usable JPEG stream.
pub(crate) fn decode(data: &[u8], hints: &FileHints) -> Option<Preview> {
    for c in preference(candidates(data)) {
        if let Some((rgba, color, orientation)) = decode_candidate(data, c) {
            return Some(Preview {
                width: c.width,
                height: c.height,
                rgba,
                orientation: hints.orientation.or(orientation).unwrap_or(1),
                color: color
                    .or_else(|| hints.color.clone())
                    .unwrap_or(ColorEncoding::Srgb),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(width: u32, height: u32) -> Candidate {
        Candidate {
            start: width as usize,
            end: width as usize + 1,
            width,
            height,
        }
    }

    #[test]
    fn prefers_the_smallest_preview_big_enough() {
        let order = preference(vec![c(160, 120), c(8256, 5504), c(1620, 1080), c(640, 424)]);
        let sizes: Vec<_> = order.iter().map(|c| c.width).collect();
        assert_eq!(sizes, [1620, 8256, 640, 160]);
        // Nothing big enough: the largest first.
        let order = preference(vec![c(160, 120), c(640, 424)]);
        assert_eq!(order[0].width, 640);
    }

    fn fixture(name: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/images");
        std::fs::read(format!("{dir}/{name}")).unwrap()
    }

    /// A big preview cut short, then a small intact one: strict decoding
    /// refuses the first (never half an image filled with gray), and the
    /// next one in preference order is used.
    #[test]
    fn a_truncated_preview_falls_back_to_the_next() {
        let wide = fixture("jpeg-wide.jpg"); // 17000 x 8: preferred
        let small = fixture("jpeg-444.jpg"); // 37 x 29
        let mut file = b"RAWDATA".to_vec();
        file.extend_from_slice(&wide[..wide.len() / 2]);
        file.extend_from_slice(b"more raw data");
        file.extend_from_slice(&small);
        let found = preference(candidates(&file));
        assert_eq!(found.first().map(|c| c.width), Some(17000), "{found:?}");
        let p = decode(&file, &FileHints::default()).expect("the small preview");
        assert_eq!((p.width, p.height), (37, 29));
        // The whole wide one decodes when intact.
        let mut file = b"RAWDATA".to_vec();
        file.extend_from_slice(&wide);
        assert_eq!(
            decode(&file, &FileHints::default()).map(|p| p.width),
            Some(17000)
        );
    }

    #[test]
    fn hints_win_over_the_stream() {
        let mut file = b"RAWDATA".to_vec();
        file.extend_from_slice(&fixture("jpeg-exif6-xmp.jpg"));
        // The stream's own EXIF says 6.
        assert_eq!(decode(&file, &FileHints::default()).unwrap().orientation, 6);
        let hints = FileHints {
            orientation: Some(3),
            color: Some(ColorEncoding::AdobeRgb),
        };
        let p = decode(&file, &hints).unwrap();
        assert_eq!((p.orientation, p.color), (3, ColorEncoding::AdobeRgb));
    }

    #[test]
    fn finds_nothing_in_random_bytes_and_never_panics() {
        let mut data = vec![0u8; 4096];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i.wrapping_mul(2_654_435_761) >> 13) as u8;
        }
        data[100..103].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        assert!(decode(&data, &FileHints::default()).is_none());
        for len in 0..data.len() {
            let _ = candidates(&data[..len]);
        }
    }
}
