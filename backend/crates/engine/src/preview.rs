//! The first pixels of a switch (DEV-01, States: Opening; LIB-09): the
//! photo's embedded preview, shown before its decode, and for a file Genzo
//! cannot render, instead of it.
//!
//! - The engine hands it over ready to draw: turned upright by the file's
//!   orientation, and in the frame's target (sRGB or Display P3), so the
//!   render that replaces it does not shift its color; and no larger than
//!   the Fit frame ([`reduce`]). An iPhone ProRAW file's only preview is
//!   full size, 48 MP: reduced first, it is converted, kept and handed over
//!   at about a seventeenth of that at 1920 x 1440.
//! - Every switch says what to show, a preview or none: without one, the
//!   viewport shows the neutral fill and the file name, not the photo
//!   before.
//! - Until contract 0.5.0 makes this the `PreviewShown` event, a sink the
//!   caller registers receives it ([`crate::Engine::with_preview_sink`]).
//! - It is prepared on a thread pool of its own. On rayon's global pool its
//!   work would wait behind a decode's (the switch's own, or a neighbor's
//!   in prefetch), which fills that pool for a second or more at 48 MP.

use std::sync::{Arc, OnceLock};

use genzo_color::output::{Target, fits, pixel};
use genzo_imgio::Preview;
use genzo_pipeline::raw::input_encoding;
use genzo_pipeline::reference::sampling::{orientation, shown};
use rayon::prelude::*;

/// A preview ready to draw: upright, in its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownPreview {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGBA, 8 bits per channel, encoded for `target`.
    pub rgba: Vec<u8>,
    pub target: Target,
}

impl ShownPreview {
    pub(crate) fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

/// What a switch shows at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub photo: u64,
    /// `None` when the file has none, or it cannot be read: the neutral
    /// fill and the file name.
    pub preview: Option<Arc<ShownPreview>>,
}

/// Receives each switch's [`Shown`], on the switching thread, before the
/// photo's decode. It must not call back into the strip's methods
/// (`select_photo`, `step`, `open_folder`), which wait for that switch.
pub type PreviewSink = Arc<dyn Fn(Shown) + Send + Sync>;

/// The engine's sink, for its `Debug`.
pub(crate) struct Sink(pub PreviewSink);

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sink")
    }
}

/// The pool a switch's preview is read and prepared on, one thread per
/// core: a large one is decoded in bands there (imgio's `bands.rs`). It
/// shares the cores with a decode, but never waits for its work to drain.
/// Only the switching thread uses it: the prefetch worker prepares its
/// neighbors' previews on the global pool ([`prepare_behind`]), so the
/// switch never waits behind one. Neither is a rayon thread; one on the
/// global pool could run a decode's work while it waits here.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("genzo-preview-{i}"))
            .build()
            .expect("the preview pool")
    })
}

/// Runs `f`, a switch's reading of its preview, on the preview pool.
pub(crate) fn on_pool<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    pool().install(f)
}

/// Runs `f` on the preview pool without waiting: the count of a shown
/// preview (DEV-03), once the switch has handed it over.
pub(crate) fn spawn(f: impl FnOnce() + Send + 'static) {
    pool().spawn(f)
}

/// `p` upright, in `target`, and reduced to fit in `fit` (the viewport),
/// for a switch to show at once. `None` when its color encoding is one
/// Genzo does not read (DEV-01, Stage 4), as for the image itself.
pub fn prepare(p: &Preview, target: Target, fit: (u32, u32)) -> Option<ShownPreview> {
    pool().install(|| prepare_behind(p, target, fit))
}

/// As [`prepare`], on the global pool, behind any decode there: for the
/// previews prefetch keeps.
pub(crate) fn prepare_behind(p: &Preview, target: Target, fit: (u32, u32)) -> Option<ShownPreview> {
    convert(&reduce(p, fit)?, target)
}

/// `p` reduced so that, upright, it fits in `fit` as a Fit frame does:
/// each pixel the mean of the codes of the stored pixels it covers. One
/// that fits already is kept as it is, never enlarged. `None` when its
/// pixels do not match its size, or `fit` is empty.
pub fn reduce(p: &Preview, fit: (u32, u32)) -> Option<Preview> {
    let (w, h) = (p.width, p.height);
    if w == 0 || h == 0 || fit.0 == 0 || fit.1 == 0 || p.rgba.len() != 4 * w as usize * h as usize {
        return None;
    }
    let (sw, sh) = shown((w, h), p.orientation);
    if sw <= fit.0 && sh <= fit.1 {
        return Some(p.clone());
    }
    // The Fit frame's rounding (`Frame::fit_in`), on the upright size.
    let side = |v: f64| (v.round() as u32).max(1);
    let (fw, fh) = (f64::from(fit.0), f64::from(fit.1));
    let (rw, rh) = if f64::from(sw) * fh >= f64::from(sh) * fw {
        (fit.0, side(f64::from(sh) * fw / f64::from(sw)))
    } else {
        (side(f64::from(sw) * fh / f64::from(sh)), fit.1)
    };
    // Back to the stored orientation, which turns a quarter for 5 to 8.
    let (ow, oh) = if (sw, sh) == (w, h) {
        (rw, rh)
    } else {
        (rh, rw)
    };
    // Pixel i of n covers the stored pixels from i * len / n up to
    // (i + 1) * len / n: at least one, since n is at most len.
    let span = |i: u32, n: u32, len: u32| {
        let at = |i: u32| (u64::from(i) * u64::from(len) / u64::from(n)) as usize;
        at(i)..at(i + 1)
    };
    let mut rgba = vec![0u8; 4 * ow as usize * oh as usize];
    rgba.par_chunks_mut(4 * ow as usize)
        .enumerate()
        .for_each(|(y, row)| {
            let ys = span(y as u32, oh, h);
            for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let xs = span(x as u32, ow, w);
                let mut sum = [0u64; 4];
                for sy in ys.clone() {
                    let line = sy * w as usize;
                    let px = &p.rgba[4 * (line + xs.start)..4 * (line + xs.end)];
                    for px in px.as_chunks::<4>().0 {
                        for c in 0..4 {
                            sum[c] += u64::from(px[c]);
                        }
                    }
                }
                let n = (ys.len() * xs.len()) as u64;
                *out = sum.map(|s| ((2 * s + n) / (2 * n)) as u8);
            }
        });
    Some(Preview {
        width: ow,
        height: oh,
        rgba,
        orientation: p.orientation,
        color: p.color.clone(),
    })
}

/// Rows per band of [`convert`]'s work, each with its own cache.
const BAND: usize = 16;
/// Entries of a band's cache of converted colors.
const MEMO: usize = 4096;

/// `p` upright and in `target`, at its own size.
fn convert(p: &Preview, target: Target) -> Option<ShownPreview> {
    let (w, h) = (p.width, p.height);
    if w == 0 || h == 0 || p.rgba.len() != 4 * w as usize * h as usize {
        return None;
    }
    let (sw, sh) = shown((w, h), p.orientation);
    let map = orientation(p.orientation);
    // Already the target's codes: copied, as an unedited non-RAW photo
    // shows its own codes (DEV-01, Stage 9).
    let same = matches!(
        (&p.color, target),
        (genzo_imgio::ColorEncoding::Srgb, Target::Srgb)
            | (genzo_imgio::ColorEncoding::DisplayP3, Target::DisplayP3)
    );
    let enc = if same {
        None
    } else {
        Some(input_encoding(&p.color).ok()?)
    };
    let compress = enc.as_ref().is_some_and(|e| !fits(&e.to_working, target));
    // Each 8-bit code's linear light, per channel.
    let lin: Option<Vec<[f64; 256]>> = enc.as_ref().map(|e| {
        e.curves
            .iter()
            .map(|c| std::array::from_fn(|v| c.eval(v as f64 / 255.0)))
            .collect()
    });
    let mut rgba = vec![0u8; 4 * sw as usize * sh as usize];
    rgba.par_chunks_mut(4 * sw as usize * BAND)
        .enumerate()
        .for_each(|(band, rows)| {
            // Without a dither, a pixel's codes depend on its source codes
            // alone, so each band keeps the colors it converted: gamut
            // compression searches for the gamut's edge, which costs far
            // more than a lookup. Keyed by the codes with bit 24 set, so 0
            // is an empty slot.
            let mut keys = vec![0u32; MEMO];
            let mut values = vec![[0u8; 3]; MEMO];
            for (r, row) in rows.chunks_mut(4 * sw as usize).enumerate() {
                let y = band * BAND + r;
                let b = (y as f64 + 0.5) / f64::from(sh);
                for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let a = (x as f64 + 0.5) / f64::from(sw);
                    let at = |r: [f64; 3], n: u32| {
                        ((r[0] * a + r[1] * b + r[2]) * f64::from(n)).floor() as u32
                    };
                    let (sx, sy) = (at(map[0], w).min(w - 1), at(map[1], h).min(h - 1));
                    let i = 4 * (sy as usize * w as usize + sx as usize);
                    let src = &p.rgba[i..i + 4];
                    match (&enc, &lin) {
                        (Some(e), Some(lin)) => {
                            let key = 1 << 24
                                | u32::from(src[0]) << 16
                                | u32::from(src[1]) << 8
                                | u32::from(src[2]);
                            let slot = (key.wrapping_mul(0x9E37_79B1) >> 20) as usize % MEMO;
                            if keys[slot] != key {
                                let linear = [0, 1, 2].map(|c| lin[c][src[c] as usize]);
                                let working = genzo_color::mul(&e.to_working, linear);
                                keys[slot] = key;
                                values[slot] = pixel(working, (0, 0), target, compress, false);
                            }
                            out[..3].copy_from_slice(&values[slot]);
                        }
                        _ => out[..3].copy_from_slice(&src[..3]),
                    }
                    out[3] = src[3];
                }
            }
        });
    Some(ShownPreview {
        width: sw,
        height: sh,
        rgba,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use genzo_imgio::ColorEncoding;

    /// A viewport larger than any preview here.
    const BIG: (u32, u32) = (4096, 4096);

    /// A 3 × 2 preview whose pixel (x, y) has red x and green y.
    fn grid(orientation: u8, color: ColorEncoding) -> Preview {
        let rgba = (0..2u8)
            .flat_map(|y| (0..3u8).flat_map(move |x| [10 * x, 10 * y, 7, 255]))
            .collect();
        Preview {
            width: 3,
            height: 2,
            rgba,
            orientation,
            color,
        }
    }

    fn at(s: &ShownPreview, x: u32, y: u32) -> [u8; 2] {
        let i = 4 * (y * s.width + x) as usize;
        [s.rgba[i], s.rgba[i + 1]]
    }

    #[test]
    fn it_is_turned_upright() {
        let up = prepare(&grid(1, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
        assert_eq!((up.width, up.height), (3, 2));
        assert_eq!(up.rgba, grid(1, ColorEncoding::Srgb).rgba);
        // 6: turned a quarter clockwise. The shown top left is the stored
        // bottom left, the shown top right the stored top left.
        let six = prepare(&grid(6, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
        assert_eq!((six.width, six.height), (2, 3));
        assert_eq!(at(&six, 0, 0), [0, 10]);
        assert_eq!(at(&six, 1, 0), [0, 0]);
        assert_eq!(at(&six, 1, 2), [20, 0]);
        // 3: a half turn; 2: mirrored; 8: a quarter counterclockwise.
        let three = prepare(&grid(3, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
        assert_eq!(at(&three, 0, 0), [20, 10]);
        let two = prepare(&grid(2, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
        assert_eq!(at(&two, 0, 1), [20, 10]);
        let eight = prepare(&grid(8, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
        assert_eq!(at(&eight, 0, 0), [20, 0]);
        // Every orientation keeps every pixel once.
        for o in 1..=8 {
            let s = prepare(&grid(o, ColorEncoding::Srgb), Target::Srgb, BIG).unwrap();
            let mut got: Vec<[u8; 2]> = (0..s.height)
                .flat_map(|y| (0..s.width).map(move |x| (x, y)))
                .map(|(x, y)| at(&s, x, y))
                .collect();
            got.sort();
            let mut want: Vec<[u8; 2]> = (0..3)
                .flat_map(|x| (0..2).map(move |y| [10 * x, 10 * y]))
                .collect();
            want.sort();
            assert_eq!(got, want, "{o}");
        }
    }

    /// Converted as stage 9 converts a non-RAW photo: an Adobe RGB
    /// preview's codes become the sRGB ones of the same color, Display P3
    /// ones on a P3 target stay, sRGB ones on a P3 target change.
    #[test]
    fn it_is_in_the_frame_target() {
        let one = |color: ColorEncoding, rgb: [u8; 3], target: Target| {
            let p = Preview {
                width: 1,
                height: 1,
                rgba: vec![rgb[0], rgb[1], rgb[2], 255],
                orientation: 1,
                color,
            };
            let s = prepare(&p, target, BIG).unwrap();
            [s.rgba[0], s.rgba[1], s.rgba[2]]
        };
        // Against the formulas, colors inside sRGB: within one code (the
        // 8-bit table against 16-bit codes). Neutrals stay neutral.
        let formula = |rgb: [u8; 3]| {
            let adobe = genzo_color::encoding::InputEncoding::adobe_rgb();
            let w = adobe.decode(rgb.map(|v| u16::from(v) * 257));
            pixel(w, (0, 0), Target::Srgb, false, false)
        };
        for rgb in [
            [0, 0, 0],
            [60, 60, 60],
            [128, 128, 128],
            [255; 3],
            [150, 60, 40],
        ] {
            let got = one(ColorEncoding::AdobeRgb, rgb, Target::Srgb);
            let want = formula(rgb);
            let diff = (0..3).map(|c| (i16::from(got[c]) - i16::from(want[c])).abs());
            assert!(diff.max() <= Some(1), "{rgb:?}: {got:?} against {want:?}");
            if rgb[0] == rgb[1] {
                assert!(got[0] == got[1] && got[1] == got[2], "{rgb:?}: {got:?}");
            }
        }
        // A gray's code moves: Adobe RGB's curve is a 2.2 power.
        assert_ne!(one(ColorEncoding::AdobeRgb, [60; 3], Target::Srgb), [60; 3]);
        // Adobe RGB's green is outside sRGB: gamut compressed as stage 9
        // does, keeping its hue, not clipped to sRGB's green.
        let green = one(ColorEncoding::AdobeRgb, [0, 255, 0], Target::Srgb);
        let adobe = genzo_color::encoding::InputEncoding::adobe_rgb();
        let working = adobe.decode([0, 65535, 0]);
        assert_eq!(green, pixel(working, (0, 0), Target::Srgb, true, false));
        assert_ne!(green, pixel(working, (0, 0), Target::Srgb, false, false));
        assert_eq!(
            one(ColorEncoding::DisplayP3, [200, 30, 90], Target::DisplayP3),
            [200, 30, 90]
        );
        let p3 = one(ColorEncoding::Srgb, [200, 30, 90], Target::DisplayP3);
        assert_ne!(p3, [200, 30, 90]);
        assert_eq!(
            one(ColorEncoding::Srgb, [200, 30, 90], Target::Srgb),
            [200, 30, 90]
        );
    }

    /// Alpha is kept as it is, converted or copied.
    #[test]
    fn alpha_is_kept() {
        for (color, target) in [
            (ColorEncoding::Srgb, Target::Srgb),
            (ColorEncoding::AdobeRgb, Target::Srgb),
        ] {
            let p = Preview {
                width: 1,
                height: 1,
                rgba: vec![90, 120, 30, 128],
                orientation: 6,
                color,
            };
            assert_eq!(prepare(&p, target, BIG).unwrap().rgba[3], 128);
        }
    }

    #[test]
    fn a_wrong_size_or_an_unreadable_encoding_shows_none() {
        let mut p = grid(1, ColorEncoding::Srgb);
        p.rgba.pop();
        assert!(prepare(&p, Target::Srgb, BIG).is_none());
        // No pixels at all: none, not a panic in the worker.
        for (width, height) in [(0, 0), (0, 5), (5, 0)] {
            let empty = Preview {
                width,
                height,
                rgba: Vec::new(),
                ..grid(6, ColorEncoding::Srgb)
            };
            assert!(
                prepare(&empty, Target::Srgb, BIG).is_none(),
                "{width} x {height}"
            );
        }
        let p = grid(1, ColorEncoding::Icc(b"not a profile".to_vec()));
        assert!(prepare(&p, Target::Srgb, BIG).is_none());
    }

    /// The reduced pixels: the Fit frame's size, never larger than the
    /// preview, each the rounded mean of the codes it covers.
    #[test]
    fn it_is_reduced_to_the_fit_frame() {
        // 4 x 2 into 2 x 2: 2 x 1, each pixel the mean of a 2 x 2 block.
        let p = Preview {
            width: 4,
            height: 2,
            rgba: [
                [0, 10, 0, 255],
                [2, 10, 0, 255],
                [100, 0, 0, 255],
                [101, 0, 0, 0],
                [4, 10, 0, 255],
                [6, 11, 0, 255],
                [100, 0, 0, 255],
                [102, 0, 0, 0],
            ]
            .concat(),
            orientation: 1,
            color: ColorEncoding::Srgb,
        };
        let r = reduce(&p, (2, 2)).unwrap();
        assert_eq!((r.width, r.height), (2, 1));
        // 11 / 4 = 2.75 rounds to 3, 41 / 4 = 10.25 to 10, 403 / 4 to 101,
        // 510 / 4 = 127.5 up to 128.
        assert_eq!(r.rgba, [3, 10, 0, 255, 101, 0, 0, 128]);
        // The same through prepare, as the switch shows it.
        let s = prepare(&p, Target::Srgb, (2, 2)).unwrap();
        assert_eq!((s.width, s.height, &s.rgba), (2, 1, &r.rgba));
        // Fits already, or a larger viewport: as it is, never enlarged.
        for fit in [(4, 2), (4000, 3000)] {
            let same = reduce(&p, fit).unwrap();
            assert_eq!((same.width, same.height, &same.rgba), (4, 2, &p.rgba));
        }
        // The fit frame's rounding: 7 x 3 into 4 x 4 is 4 x 2 (1.71).
        let wide = Preview {
            width: 7,
            height: 3,
            rgba: vec![9; 4 * 21],
            ..p.clone()
        };
        let r = reduce(&wide, (4, 4)).unwrap();
        assert_eq!((r.width, r.height), (4, 2));
        assert!(r.rgba.iter().all(|&v| v == 9));
        // Turned a quarter (6): it fits upright, as 2 x 4 into 1 x 2 shown,
        // so it is stored 2 x 1, and the frame's upright size is 1 x 2.
        let six = Preview {
            orientation: 6,
            ..p.clone()
        };
        let r = reduce(&six, (1, 2)).unwrap();
        assert_eq!((r.width, r.height), (2, 1));
        assert_eq!(r.orientation, 6);
        let s = prepare(&six, Target::Srgb, (1, 2)).unwrap();
        assert_eq!((s.width, s.height), (1, 2));
        // An empty viewport or a wrong size: none.
        assert!(reduce(&p, (0, 2)).is_none());
        let mut short = p.clone();
        short.rgba.pop();
        assert!(reduce(&short, (2, 2)).is_none());
    }

    /// The colors each band keeps are exactly the ones converted on their
    /// own, however many collide in its cache.
    #[test]
    fn kept_colors_are_converted_exactly() {
        let mut state = 0x2545_f491_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        // Far more colors than a band's cache holds, some repeated.
        let (w, h) = (97u32, 61u32);
        let mut rgba = Vec::new();
        for i in 0..w * h {
            let v = if i % 3 == 0 { 0x0080_4020 } else { next() };
            rgba.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]);
        }
        // Adobe RGB into sRGB: converted and gamut compressed.
        let p = Preview {
            width: w,
            height: h,
            rgba,
            orientation: 1,
            color: ColorEncoding::AdobeRgb,
        };
        let s = prepare(&p, Target::Srgb, BIG).unwrap();
        for (i, px) in p.rgba.as_chunks::<4>().0.iter().enumerate() {
            let one = Preview {
                width: 1,
                height: 1,
                rgba: px.to_vec(),
                ..p.clone()
            };
            let want = prepare(&one, Target::Srgb, BIG).unwrap().rgba;
            assert_eq!(&s.rgba[4 * i..4 * i + 4], &want[..], "pixel {i}");
        }
    }
}
