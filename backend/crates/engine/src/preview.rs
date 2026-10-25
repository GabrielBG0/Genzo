//! The first pixels of a switch (DEV-01, States: Opening; LIB-09): the
//! photo's embedded preview, shown before its decode, and for a file Genzo
//! cannot render, instead of it.
//!
//! - The engine hands it over ready to draw: turned upright by the file's
//!   orientation, and in the frame's target (sRGB or Display P3), so the
//!   render that replaces it does not shift its color. Scaling is the
//!   viewport's fit transform.
//! - Every switch says what to show, a preview or none: without one, the
//!   viewport shows the neutral fill and the file name, not the photo
//!   before.
//! - Until contract 0.5.0 makes this the `PreviewShown` event, a sink the
//!   caller registers receives it ([`crate::Engine::with_preview_sink`]).

use std::sync::Arc;

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

/// `p` upright and in `target`. `None` when its color encoding is one
/// Genzo does not read (DEV-01, Stage 4), as for the image itself.
pub fn prepare(p: &Preview, target: Target) -> Option<ShownPreview> {
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
    rgba.par_chunks_mut(4 * sw as usize)
        .enumerate()
        .for_each(|(y, row)| {
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
                        let linear = [0, 1, 2].map(|c| lin[c][src[c] as usize]);
                        let working = genzo_color::mul(&e.to_working, linear);
                        let rgb = pixel(working, (x as u32, y as u32), target, compress, false);
                        out[..3].copy_from_slice(&rgb);
                    }
                    _ => out[..3].copy_from_slice(&src[..3]),
                }
                out[3] = src[3];
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
        let up = prepare(&grid(1, ColorEncoding::Srgb), Target::Srgb).unwrap();
        assert_eq!((up.width, up.height), (3, 2));
        assert_eq!(up.rgba, grid(1, ColorEncoding::Srgb).rgba);
        // 6: turned a quarter clockwise. The shown top left is the stored
        // bottom left, the shown top right the stored top left.
        let six = prepare(&grid(6, ColorEncoding::Srgb), Target::Srgb).unwrap();
        assert_eq!((six.width, six.height), (2, 3));
        assert_eq!(at(&six, 0, 0), [0, 10]);
        assert_eq!(at(&six, 1, 0), [0, 0]);
        assert_eq!(at(&six, 1, 2), [20, 0]);
        // 3: a half turn; 2: mirrored; 8: a quarter counterclockwise.
        let three = prepare(&grid(3, ColorEncoding::Srgb), Target::Srgb).unwrap();
        assert_eq!(at(&three, 0, 0), [20, 10]);
        let two = prepare(&grid(2, ColorEncoding::Srgb), Target::Srgb).unwrap();
        assert_eq!(at(&two, 0, 1), [20, 10]);
        let eight = prepare(&grid(8, ColorEncoding::Srgb), Target::Srgb).unwrap();
        assert_eq!(at(&eight, 0, 0), [20, 0]);
        // Every orientation keeps every pixel once.
        for o in 1..=8 {
            let s = prepare(&grid(o, ColorEncoding::Srgb), Target::Srgb).unwrap();
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
            let s = prepare(&p, target).unwrap();
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
            assert_eq!(prepare(&p, target).unwrap().rgba[3], 128);
        }
    }

    #[test]
    fn a_wrong_size_or_an_unreadable_encoding_shows_none() {
        let mut p = grid(1, ColorEncoding::Srgb);
        p.rgba.pop();
        assert!(prepare(&p, Target::Srgb).is_none());
        // No pixels at all: none, not a panic in the worker.
        for (width, height) in [(0, 0), (0, 5), (5, 0)] {
            let empty = Preview {
                width,
                height,
                rgba: Vec::new(),
                ..grid(6, ColorEncoding::Srgb)
            };
            assert!(
                prepare(&empty, Target::Srgb).is_none(),
                "{width} x {height}"
            );
        }
        let p = grid(1, ColorEncoding::Icc(b"not a profile".to_vec()));
        assert!(prepare(&p, Target::Srgb).is_none());
    }
}
