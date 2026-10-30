//! A large embedded preview decoded in horizontal bands, one per thread
//! (DEV-01, first pixels). A full-size preview (iPhone ProRAW stores only
//! an 8064 x 6048 one) takes about 190 ms to decode on one thread.
//!
//! A baseline JPEG with restart markers can be cut where an MCU row starts
//! on a restart boundary: the DC predictions start again there, so the
//! entropy data from that point decode on their own, after the stream's
//! own header with the band's height in its frame. Chroma upsampling still
//! looks one row past a band's edge, so each band is decoded with a step of
//! MCU rows more above and below, which are thrown away. The result is the
//! same bytes as one decode of the whole stream (tested, and fuzzed in
//! `preview_bands`).
//!
//! Only one interleaved scan of three components is cut: a progressive,
//! gray or multi-scan stream, or one whose restart markers are not where
//! the header says, is decoded whole.

use rayon::prelude::*;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// Where a stream's parts lie, for cutting it into bands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Where the frame height lies in the SOF.
    height_at: usize,
    /// Where the entropy data begin, just past the SOS.
    entropy: usize,
    pub width: u32,
    pub height: u32,
    /// Pixel rows per MCU row.
    mcu_height: u32,
    mcu_rows: u32,
    per_row: u64,
    interval: u64,
    /// MCU rows between the rows where a band may start.
    step: u32,
    /// Each restart interval's entropy data, without its marker.
    intervals: Vec<(usize, usize)>,
}

/// The layout of a baseline JPEG `s` (from its SOI to its EOI), if it can
/// be cut into bands.
pub(crate) fn layout(s: &[u8]) -> Option<Layout> {
    if s.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    let (mut frame, mut interval) = (None, 0u64);
    let entropy = loop {
        // Markers may be preceded by fill bytes.
        while s.get(at) == Some(&0xFF) && s.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *s.get(at)? != 0xFF {
            return None;
        }
        let marker = *s.get(at + 1)?;
        let len = usize::from(u16::from_be_bytes([*s.get(at + 2)?, *s.get(at + 3)?]));
        let body = s.get(at + 4..(at + 2 + len).max(at + 4))?;
        if len < 2 {
            return None;
        }
        match marker {
            // Baseline and extended Huffman DCT, three components.
            0xC0 | 0xC1 => {
                let [8, h0, h1, w0, w1, 3, c @ ..] = body else {
                    return None;
                };
                let (h, w) = (
                    u16::from_be_bytes([*h0, *h1]),
                    u16::from_be_bytes([*w0, *w1]),
                );
                let sampling: Vec<_> = c
                    .get(..9)?
                    .chunks(3)
                    .map(|c| (c[1] >> 4, c[1] & 15))
                    .collect();
                if w == 0
                    || h == 0
                    || sampling
                        .iter()
                        .any(|&(x, y)| !(1..=4).contains(&x) || !(1..=4).contains(&y))
                {
                    return None;
                }
                let mh = sampling.iter().map(|s| s.0).max()?;
                let mv = sampling.iter().map(|s| s.1).max()?;
                frame = Some((
                    at + 5,
                    u32::from(w),
                    u32::from(h),
                    8 * u32::from(mh),
                    8 * u32::from(mv),
                ));
            }
            // Any other frame: progressive, lossless, arithmetic.
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None,
            0xDD => interval = u64::from(u16::from_be_bytes([*body.first()?, *body.get(1)?])),
            // One scan of all three components, in full.
            0xDA => {
                let [3, rest @ ..] = body else {
                    return None;
                };
                if rest.get(6..9)? != [0, 63, 0] {
                    return None;
                }
                break at + 2 + len;
            }
            0xD0..=0xD9 => return None,
            _ => {}
        }
        at += 2 + len;
    };
    let (height_at, width, height, mcu_width, mcu_height) = frame?;
    if interval == 0 {
        return None;
    }
    let per_row = u64::from(width.div_ceil(mcu_width));
    let mcu_rows = height.div_ceil(mcu_height);
    let count = (per_row * u64::from(mcu_rows)).div_ceil(interval);
    // Every marker in the entropy data is a restart, numbered in turn,
    // until the EOI; one per interval but the last.
    let mut intervals = Vec::with_capacity(usize::try_from(count).ok()?.min(s.len() / 2));
    let (mut start, mut i) = (entropy, entropy);
    loop {
        let ff = i + memchr::memchr(0xFF, s.get(i..)?)?;
        let mut m = ff + 1;
        while s.get(m) == Some(&0xFF) {
            m += 1;
        }
        match *s.get(m)? {
            // A stuffed byte.
            0x00 => {}
            n @ 0xD0..=0xD7 => {
                if usize::from(n - 0xD0) != intervals.len() % 8
                    || intervals.len() as u64 + 1 >= count
                {
                    return None;
                }
                intervals.push((start, ff));
                start = m + 1;
            }
            0xD9 => {
                intervals.push((start, ff));
                break;
            }
            _ => return None,
        }
        i = m + 1;
    }
    if intervals.len() as u64 != count {
        return None;
    }
    // A band may start on row r when r * per_row is a whole number of
    // intervals.
    let step = u32::try_from(interval / gcd(interval, per_row)).ok()?;
    Some(Layout {
        height_at,
        entropy,
        width,
        height,
        mcu_height,
        mcu_rows,
        per_row,
        interval,
        step,
        intervals,
    })
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The options both a band and a whole stream are decoded with.
pub(crate) fn options(out: ColorSpace) -> DecoderOptions {
    DecoderOptions::default()
        .set_strict_mode(true)
        .set_max_width(65_535)
        .set_max_height(65_535)
        .jpeg_set_out_colorspace(out)
}

/// `s`, laid out as `l`, decoded to RGBA in up to `bands` bands at once;
/// `None` when any band fails. Bands cannot be cut finer than `l.step`.
pub(crate) fn decode(s: &[u8], l: &Layout, bands: u32) -> Option<Vec<u8>> {
    let per_band = l.mcu_rows.div_ceil(bands.max(1)).div_ceil(l.step) * l.step;
    let row = l.width as usize * 4;
    let mut out = vec![0; row * l.height as usize];
    let chunk = (per_band * l.mcu_height) as usize * row;
    out.par_chunks_mut(chunk)
        .enumerate()
        .try_for_each(|(k, out)| {
            let first = u32::try_from(k).ok()? * per_band;
            let from = first.saturating_sub(l.step);
            let to = (first + per_band + l.step).min(l.mcu_rows);
            let rgba = band(s, l, from, to)?;
            let skip = ((first - from) * l.mcu_height) as usize * row;
            out.copy_from_slice(rgba.get(skip..skip + out.len())?);
            Some(())
        })?;
    Some(out)
}

/// MCU rows `from..to` of `s`, decoded on their own to RGBA.
fn band(s: &[u8], l: &Layout, from: u32, to: u32) -> Option<Vec<u8>> {
    let height = (to * l.mcu_height).min(l.height) - from * l.mcu_height;
    let first = (u64::from(from) * l.per_row / l.interval) as usize;
    let last = (u64::from(to) * l.per_row).div_ceil(l.interval) as usize;
    let (start, end) = (
        l.intervals.get(first)?.0,
        l.intervals.get(last.checked_sub(1)?)?.1,
    );
    let mut b = Vec::with_capacity(l.entropy + end - start + 2);
    b.extend_from_slice(&s[..l.entropy]);
    b[l.height_at..l.height_at + 2].copy_from_slice(&u16::try_from(height).ok()?.to_be_bytes());
    b.extend_from_slice(&s[start..end]);
    b.extend_from_slice(&[0xFF, 0xD9]);
    let mut d = JpegDecoder::new_with_options(ZCursor::new(&b[..]), options(ColorSpace::RGBA));
    let rgba = d.decode().ok()?;
    (rgba.len() == l.width as usize * height as usize * 4).then_some(rgba)
}

/// Decodes `s` whole, to RGBA, as a preview is when it cannot be cut.
pub(crate) fn whole(s: &[u8]) -> Option<Vec<u8>> {
    let mut d = JpegDecoder::new_with_options(ZCursor::new(s), options(ColorSpace::RGB));
    let rgb = d.decode().ok()?;
    let (w, h) = d.dimensions()?;
    if rgb.len() != w * h * 3 {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    Some(rgba)
}

/// For the `preview_bands` fuzz target: `s` cut into bands as thin as its
/// restart markers allow, and into two, must decode to the same bytes as
/// `s` whole, or fail.
pub fn check(s: &[u8]) {
    let Some(l) = layout(s) else {
        return;
    };
    // The whole decode would refuse more; keep the fuzzer's memory small.
    if u64::from(l.width) * u64::from(l.height) > 1 << 22 {
        return;
    }
    for bands in [u32::MAX, 2] {
        if let Some(cut) = decode(s, &l, bands) {
            assert!(
                whole(s).as_deref() == Some(&cut[..]),
                "{bands} bands differ from one"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

    /// A w x h image with detail everywhere (chroma too), encoded with
    /// `sampling` and a restart every `interval` MCUs.
    fn jpeg(w: u16, h: u16, sampling: SamplingFactor, interval: u16) -> Vec<u8> {
        let mut rgb = Vec::new();
        let mut x = 0x2545_F491_u32;
        for i in 0..u32::from(w) * u32::from(h) {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let (c, r) = (i % u32::from(w), i / u32::from(w));
            rgb.extend_from_slice(&[
                (c * 7 + r) as u8,
                (x >> 24) as u8 | 64,
                (r * 5) as u8 ^ (x as u8 & 31),
            ]);
        }
        let mut out = Vec::new();
        let mut e = Encoder::new(&mut out, 90);
        e.set_sampling_factor(sampling);
        e.set_restart_interval(interval);
        e.encode(&rgb, w, h, ColorType::Rgb).unwrap();
        out
    }

    #[test]
    fn bands_are_the_same_bytes_as_one_decode() {
        use SamplingFactor::{F_1_1, F_1_2, F_2_1, F_2_2};
        // Odd sizes, every sampling, an interval of a row, of two rows,
        // and of a row and a half (a band may start every third row: the
        // 26 MCUs of 203 px in 8, the 4 of 64 px in 16).
        for (w, h, sampling, mcu) in [
            (203_u16, 157_u16, F_2_2, (16_u16, 16_u32)),
            (203, 157, F_2_1, (16, 8)),
            (203, 157, F_1_1, (8, 8)),
            (203, 157, F_1_2, (8, 16)),
            (64, 300, F_2_2, (16, 16)),
        ] {
            let per_row = w.div_ceil(mcu.0);
            let mut intervals = vec![(per_row, 1), (2 * per_row, 2)];
            if per_row % 2 == 0 {
                intervals.push((per_row * 3 / 2, 3));
            }
            for (interval, step) in intervals {
                let s = jpeg(w, h, sampling, interval);
                let l =
                    layout(&s).unwrap_or_else(|| panic!("{w}x{h} {sampling:?} every {interval}"));
                assert_eq!(
                    (l.width, l.height, l.mcu_height),
                    (u32::from(w), u32::from(h), mcu.1)
                );
                let at = format!("{w}x{h} {sampling:?} every {interval}");
                assert_eq!(l.step, step, "{at}");
                let whole = whole(&s).unwrap();
                for bands in [1, 2, 3, 7, u32::MAX] {
                    let cut = decode(&s, &l, bands).unwrap();
                    assert!(cut == whole, "{at}, {bands} bands");
                }
            }
        }
    }

    #[test]
    fn only_a_stream_whose_markers_agree_is_cut() {
        let s = jpeg(203, 157, SamplingFactor::F_2_2, 13);
        assert!(layout(&s).is_some());
        // No restart markers.
        assert_eq!(layout(&jpeg(203, 157, SamplingFactor::F_2_2, 0)), None);
        // A restart marker numbered out of turn, one missing, the stream
        // cut short, another marker in the data: all decoded whole.
        let rst: Vec<usize> = (0..s.len() - 1)
            .filter(|&i| s[i] == 0xFF && (0xD0..=0xD7).contains(&s[i + 1]))
            .collect();
        let mut wrong = s.clone();
        wrong[rst[3] + 1] = 0xD0;
        assert_eq!(layout(&wrong), None);
        let mut missing = s.clone();
        missing.drain(rst[3]..rst[3] + 2);
        assert_eq!(layout(&missing), None);
        assert_eq!(layout(&s[..s.len() - 2]), None);
        let mut other = s.clone();
        other[rst[3] + 1] = 0xC4;
        assert_eq!(layout(&other), None);
        // Nor one scan per component, as jpeg-encoder writes 4:1:1.
        let s = jpeg(203, 157, SamplingFactor::F_4_1, 7);
        assert!(whole(&s).is_some());
        assert_eq!(layout(&s), None);
        // Gray streams are not cut.
        let mut out = Vec::new();
        let mut e = Encoder::new(&mut out, 90);
        e.set_restart_interval(4);
        e.encode(&[128; 64 * 64], 64, 64, ColorType::Luma).unwrap();
        assert_eq!(layout(&out), None);
    }

    #[test]
    fn checking_random_bytes_never_panics() {
        let s = jpeg(64, 48, SamplingFactor::F_2_2, 2);
        check(&s);
        let mut x = 0x9E37_79B9_u32;
        for i in 0..2000 {
            let mut t = s.clone();
            for _ in 0..1 + i % 4 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = x as usize % t.len();
                t[at] = (x >> 8) as u8;
            }
            check(&t);
            check(&t[..x as usize % t.len()]);
        }
    }
}
