//! The histogram and the clipping warnings (DEV-03), as stage 9 counts and
//! draws them, and their CPU reference: what the frame's RGBA8 codes say.
//!
//! - A pixel's highlights clip when any of R, G and B is 255; its shadows
//!   when any is 0 (owner, 2026-10-30). One pixel can clip in both.
//! - The J overlay draws a clipped pixel in the highlight color, or the
//!   shadow color; red wins for a pixel that clips in both when both
//!   overlays are on.
//! - The Alt preview replaces the whole frame: black with each
//!   highlight-clipped pixel in its clipped channels, or white with each
//!   shadow-clipped pixel in its. The J overlay is not drawn under it.
//! - Counting reads the photo's codes, never the overlay's or the
//!   preview's.

/// Bit 0 for R, 1 for G, 2 for B.
pub type Channels = u8;

/// One side's clipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Clipping {
    /// The pixels that clip on this side.
    pub pixels: u32,
    /// The channels that clip in at least one pixel.
    pub channels: Channels,
}

/// The counts of one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    /// R, G and B: bin `n` holds the pixels whose code is `n`.
    pub bins: [[u32; 256]; 3],
    pub highlights: Clipping,
    pub shadows: Clipping,
    /// The pixels counted.
    pub pixels: u32,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: [[0; 256]; 3],
            highlights: Clipping::default(),
            shadows: Clipping::default(),
            pixels: 0,
        }
    }
}

/// The 32-bit words of the GPU's histogram buffer: 3 × 256 bins, then
/// the highlight pixels and channels, the shadow pixels and channels, and
/// the pixels counted.
pub const WORDS: usize = 3 * 256 + 5;

impl Histogram {
    /// From the GPU buffer's words ([`WORDS`]).
    pub fn from_words(w: &[u32]) -> Self {
        assert_eq!(w.len(), WORDS);
        let mut bins = [[0; 256]; 3];
        for (c, b) in bins.iter_mut().enumerate() {
            b.copy_from_slice(&w[256 * c..256 * (c + 1)]);
        }
        Self {
            bins,
            highlights: Clipping {
                pixels: w[768],
                channels: w[769] as Channels,
            },
            shadows: Clipping {
                pixels: w[770],
                channels: w[771] as Channels,
            },
            pixels: w[772],
        }
    }
}

/// The channels of `px` at 255, and those at 0.
pub fn clipped(px: [u8; 4]) -> (Channels, Channels) {
    let mut high = 0;
    let mut low = 0;
    for (c, &v) in px[..3].iter().enumerate() {
        high |= u8::from(v == 255) << c;
        low |= u8::from(v == 0) << c;
    }
    (high, low)
}

/// The counts of RGBA8 pixels: the CPU reference of stage 9's.
pub fn count(pixels: &[[u8; 4]]) -> Histogram {
    let mut h = Histogram::default();
    for &px in pixels {
        for (bins, &v) in h.bins.iter_mut().zip(&px[..3]) {
            bins[usize::from(v)] += 1;
        }
        let (high, low) = clipped(px);
        if high != 0 {
            h.highlights.pixels += 1;
            h.highlights.channels |= high;
        }
        if low != 0 {
            h.shadows.pixels += 1;
            h.shadows.channels |= low;
        }
        h.pixels += 1;
    }
    h
}

/// The Alt clipping preview (DEV-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preview {
    #[default]
    None,
    Highlights,
    Shadows,
}

/// What stage 9 draws over the photo. Off by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overlay {
    /// The J overlay's highlight color, RGBA8, when on.
    pub highlights: Option<[u8; 4]>,
    /// Its shadow color, when on.
    pub shadows: Option<[u8; 4]>,
    pub preview: Preview,
}

impl Overlay {
    /// What stage 9 writes for the photo's pixel `px`: the CPU reference.
    pub fn apply(&self, px: [u8; 4]) -> [u8; 4] {
        let (high, low) = clipped(px);
        let full = |mask: Channels, on: u8, off: u8| {
            let mut out = [off, off, off, 255];
            for (c, v) in out[..3].iter_mut().enumerate() {
                if mask & (1 << c) != 0 {
                    *v = on;
                }
            }
            out
        };
        match self.preview {
            // Black, and the clipped channels at full value.
            Preview::Highlights => return full(high, 255, 0),
            // White, and the clipped channels at 0.
            Preview::Shadows => return full(low, 0, 255),
            Preview::None => {}
        }
        if let (Some(color), true) = (self.highlights, high != 0) {
            return [color[0], color[1], color[2], 255];
        }
        if let (Some(color), true) = (self.shadows, low != 0) {
            return [color[0], color[1], color[2], 255];
        }
        px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_clips() {
        // DEV-03 criterion 2's patches.
        let patches = [
            [255, 128, 128, 255],
            [255, 255, 255, 255],
            [0, 128, 128, 255],
            [0, 0, 255, 255],
            [128, 128, 128, 255],
        ];
        let h = count(&patches);
        assert_eq!(h.pixels, 5);
        assert_eq!(
            h.highlights,
            Clipping {
                pixels: 3,
                channels: 0b111
            }
        );
        assert_eq!(
            h.shadows,
            Clipping {
                pixels: 2,
                channels: 0b011
            }
        );
        assert_eq!(clipped(patches[3]), (0b100, 0b011));
        assert_eq!(clipped(patches[4]), (0, 0));
        assert_eq!((h.bins[0][255], h.bins[0][0], h.bins[0][128]), (2, 2, 1));
        assert_eq!(h.bins[2][255], 2);
        let words: Vec<u32> = h
            .bins
            .iter()
            .flatten()
            .copied()
            .chain([3, 0b111, 2, 0b011, 5])
            .collect();
        assert_eq!(Histogram::from_words(&words), h);
    }

    #[test]
    fn the_overlay_and_the_preview() {
        let (red, blue) = ([255, 107, 107, 255], [91, 157, 255, 255]);
        let both = Overlay {
            highlights: Some(red),
            shadows: Some(blue),
            preview: Preview::None,
        };
        let gray = [128, 128, 128, 255];
        assert_eq!(both.apply(gray), gray);
        assert_eq!(both.apply([255, 128, 128, 255]), red);
        assert_eq!(both.apply([0, 128, 128, 255]), blue);
        // Both sides: red wins with both on, blue with shadows alone.
        let saturated = [0, 0, 255, 255];
        assert_eq!(both.apply(saturated), red);
        let shadows = Overlay {
            highlights: None,
            ..both
        };
        assert_eq!(shadows.apply(saturated), blue);
        assert_eq!(Overlay::default().apply(saturated), saturated);
        // The preview replaces the frame, J or not.
        let high = Overlay {
            preview: Preview::Highlights,
            ..both
        };
        assert_eq!(high.apply(gray), [0, 0, 0, 255]);
        assert_eq!(high.apply([255, 255, 10, 255]), [255, 255, 0, 255]);
        assert_eq!(high.apply(saturated), [0, 0, 255, 255]);
        let low = Overlay {
            preview: Preview::Shadows,
            ..both
        };
        assert_eq!(low.apply(gray), [255, 255, 255, 255]);
        assert_eq!(low.apply(saturated), [0, 0, 255, 255]);
        assert_eq!(low.apply([0, 0, 0, 255]), [0, 0, 0, 255]);
    }
}
