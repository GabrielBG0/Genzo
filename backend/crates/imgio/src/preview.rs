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
//! enough to decode within the budget. Without one, the largest. Color
//! streams come before grayscale ones: an iPhone ProRAW DNG stores its
//! semantic mattes (sky, skin, hair) as grayscale JPEGs of the preview's
//! size. A grayscale stream is still the preview of a file with no color
//! one (a monochrome camera).
//!
//! A false match in compressed raw data is guarded against three ways: the
//! header must hold DQT, SOF and SOS in order with sane values; the stream
//! is cut at its EOI, and must be large enough for the size it claims; and
//! zune-jpeg decodes it in strict mode, so a truncated stream fails and the
//! next candidate is tried.

use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_jpeg::JpegDecoder;

use crate::{ColorEncoding, FileFormat, Preview, bands, exif, raster};

/// The preview size preferred: the smallest candidate with a long edge of
/// at least this many pixels. Cameras store one of 1616 to 2016 px beside
/// the full-size JPEG; that decodes in a few milliseconds.
pub(crate) const PREFERRED_EDGE: u32 = 1280;

/// Previews from this many pixels are decoded in bands on the rayon pool
/// of the caller (`bands.rs`), when their restart markers allow; smaller
/// ones take a few milliseconds whole.
const BANDS_FROM: u64 = 4_000_000;

/// Largest candidate considered, in pixels.
const MAX_PIXELS: u64 = 50_000_000;

/// An embedded JPEG stream: where it lies and the size its header claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub start: usize,
    pub end: usize,
    pub width: u32,
    pub height: u32,
    /// 1 for grayscale, 3 for color.
    pub components: u8,
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
/// frame size, its number of components and where the entropy data begin,
/// if the header is sane.
fn header(data: &[u8], start: usize) -> Option<(u32, u32, u8, usize)> {
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
                frame = Some((w, h, n as u8));
            }
            // Lossless, arithmetic or hierarchical frames: not a preview.
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None,
            0xDA => {
                let (w, h, n) = frame?;
                return dqt.then_some((w, h, n, at + 2 + len));
            }
            0xD9 => return None,
            _ => {}
        }
        at += 2 + len;
    }
    None
}

/// Every embedded JPEG stream in `data`, in file order, except one at
/// offset 0 (a JPEG file's own image, which is not its preview).
pub(crate) fn candidates(data: &[u8]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let finder = memchr::memmem::Finder::new(&[0xFF, 0xD8, 0xFF]);
    // A stream ends just past the first EOI after its entropy data begin
    // (stuffing keeps FF D9 out of entropy data). The first EOI after one
    // position is also the first after every earlier one, so it is found
    // once and reused: the scan stays linear however many headers there are.
    let eoi = memchr::memmem::Finder::new(&[0xFF, 0xD9]);
    let mut next_eoi: Option<usize> = None;
    for start in finder.find_iter(data) {
        if start == 0 {
            continue;
        }
        let Some((width, height, components, entropy)) = header(data, start) else {
            continue;
        };
        let end = match next_eoi {
            Some(e) if e >= entropy => e + 2,
            _ => match data.get(entropy..).and_then(|d| eoi.find(d)) {
                Some(off) => {
                    next_eoi = Some(entropy + off);
                    entropy + off + 2
                }
                // No EOI after here, so none after any later header either.
                None => break,
            },
        };
        let c = Candidate {
            start,
            end,
            width,
            height,
            components,
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

/// The candidates in the order they are tried: color ones before grayscale
/// ones, and among each, the smallest with a long edge of at least
/// `PREFERRED_EDGE` first, then larger ones, then the smaller ones from
/// largest down.
pub(crate) fn preference(mut c: Vec<Candidate>) -> Vec<Candidate> {
    c.sort_by_key(|c| {
        let big = c.long_edge() >= PREFERRED_EDGE;
        (
            c.components == 1,
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
    let mut decoder =
        JpegDecoder::new_with_options(ZCursor::new(stream), bands::options(ColorSpace::RGB));
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
    let cut = (c.pixels() >= BANDS_FROM)
        .then(|| bands::layout(stream))
        .flatten()
        .and_then(|l| bands::decode(stream, &l, rayon::current_num_threads() as u32));
    let rgba = match cut {
        Some(rgba) => rgba,
        None => bands::whole(stream)?,
    };
    if rgba.len() != c.pixels() as usize * 4 {
        return None;
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

/// For the corpus tests: every embedded stream of `data` that can be cut
/// into bands, decoded so at every thread count from 1 to 16, must be the
/// same bytes as decoded whole; how many decoded.
pub fn check_bands_in(data: &[u8]) -> usize {
    let mut cut = 0;
    for c in candidates(data) {
        let s = &data[c.start..c.end];
        let Some(l) = bands::layout(s) else {
            continue;
        };
        let whole = bands::whole(s);
        for threads in 1..=16 {
            let got = bands::decode(s, &l, threads);
            assert!(got == whole, "the stream at {} in {threads} bands", c.start);
        }
        cut += usize::from(whole.is_some());
    }
    cut
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
            components: 3,
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

    /// An iPhone ProRAW DNG's semantic mattes are grayscale JPEGs of the
    /// preview's size, and may come first in the file.
    #[test]
    fn prefers_color_to_grayscale() {
        let gray = |w, h| Candidate {
            components: 1,
            ..c(w, h)
        };
        let order = preference(vec![gray(2016, 1512), c(2015, 1512), gray(2017, 1512)]);
        let sizes: Vec<_> = order.iter().map(|c| c.width).collect();
        assert_eq!(sizes, [2015, 2016, 2017]);
        // A smaller color one still comes first.
        let order = preference(vec![gray(2016, 1512), c(320, 240)]);
        assert_eq!(order[0].components, 3);
        // Only grayscale (a monochrome camera): the usual rule among them.
        let order = preference(vec![gray(4000, 3000), gray(1620, 1080)]);
        assert_eq!(order[0].width, 1620);
    }

    fn fixture(name: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/images");
        std::fs::read(format!("{dir}/{name}")).unwrap()
    }

    /// The bytes as found: a grayscale stream first in the file, then a
    /// color one of the same size. The color one is the preview.
    #[test]
    fn a_grayscale_stream_before_the_preview_is_passed_over() {
        let mut file = b"II*\0".to_vec();
        file.extend(fixture("jpeg-gray.jpg")); // 37 x 29, 1 component
        file.extend(fixture("jpeg-420.jpg")); // 37 x 29, 3 components
        let found = candidates(&file);
        assert_eq!(
            found.iter().map(|c| c.components).collect::<Vec<_>>(),
            [1, 3]
        );
        let p = decode(&file, &FileHints::default()).unwrap();
        assert!(
            p.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[0] != px[1] || px[1] != px[2]),
            "the grayscale stream was decoded"
        );
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
