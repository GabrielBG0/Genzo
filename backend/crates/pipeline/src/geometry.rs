//! The edit's geometry (DEV-01, *Geometry* and *Crop and rotation
//! coordinates*): quarter turns and a horizontal flip on top of the file's
//! orientation, then a crop rectangle, straightened about its center. The
//! sampler composes it into its one affine (`reference::sampling::Frame`).
//! It changes no pixel values.

use crate::reference::sampling::{orientation, shown};

/// The crop on the oriented image (normalized, 0 to 1 on each axis): its
/// center, its width and height as fractions of the oriented image's, and
/// the straighten angle in degrees, positive clockwise, applied in pixel
/// space about the center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub angle: f64,
}

impl Crop {
    /// The whole image, straight.
    pub const FULL: Crop = Crop {
        cx: 0.5,
        cy: 0.5,
        w: 1.0,
        h: 1.0,
        angle: 0.0,
    };

    /// The crop's size in pixels of an oriented image of size `oriented`.
    pub fn pixels(&self, oriented: (u32, u32)) -> (f64, f64) {
        (
            self.w * f64::from(oriented.0),
            self.h * f64::from(oriented.1),
        )
    }

    /// Its four corners in oriented pixels: top left, top right, bottom
    /// right, bottom left.
    pub fn corners(&self, oriented: (u32, u32)) -> [[f64; 2]; 4] {
        let (cw, ch) = self.pixels(oriented);
        let c = [
            self.cx * f64::from(oriented.0),
            self.cy * f64::from(oriented.1),
        ];
        let (sin, cos) = self.angle.to_radians().sin_cos();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(u, v)| {
            let (dx, dy) = (u * cw, v * ch);
            [c[0] + cos * dx - sin * dy, c[1] + sin * dx + cos * dy]
        })
    }
}

/// The straighten angle's range, in degrees either way.
pub const MAX_ANGLE: f64 = 45.0;

/// The smallest crop, in source pixels on each side.
pub const MIN_CROP: f64 = 32.0;

/// The edit's geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Clockwise, 0 to 3.
    pub quarter_turns: u8,
    pub flip_horizontal: bool,
    pub crop: Crop,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            quarter_turns: 0,
            flip_horizontal: false,
            crop: Crop::FULL,
        }
    }
}

/// The EXIF orientation (1 to 8) of an image whose file says `file`, turned
/// `turns` quarters clockwise and then, if `flip`, mirrored left to right.
pub fn compose(file: u8, turns: u8, flip: bool) -> u8 {
    // Where an oriented point (a, b) comes from in the file's shown image:
    // a flip mirrors a; each clockwise turn takes (a, b) from (b, 1 − a).
    let mut m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    if flip {
        m = then(m, [[-1.0, 0.0, 1.0], [0.0, 1.0, 0.0]]);
    }
    for _ in 0..turns % 4 {
        m = then(m, [[0.0, 1.0, 0.0], [-1.0, 0.0, 1.0]]);
    }
    let total = then(m, orientation(file));
    (1..=8)
        .find(|&o| orientation(o) == total)
        .expect("the eight orientations are closed under composition")
}

/// `outer ∘ inner` for maps of normalized coordinates, rows `[a, b, 1]`:
/// apply `inner` to the point, then `outer` to the result.
fn then(inner: [[f64; 3]; 2], outer: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    [0, 1].map(|i| {
        [
            outer[i][0] * inner[0][0] + outer[i][1] * inner[1][0],
            outer[i][0] * inner[0][1] + outer[i][1] * inner[1][1],
            outer[i][0] * inner[0][2] + outer[i][1] * inner[1][2] + outer[i][2],
        ]
    })
}

impl Geometry {
    /// The orientation the sampler applies, from the file's `file`.
    pub fn orientation(&self, file: u8) -> u8 {
        compose(file, self.quarter_turns, self.flip_horizontal)
    }

    /// The same region after one more quarter turn clockwise: the crop
    /// moves with the image (DEV-01: a turn keeps the selected region).
    pub fn turned(&self) -> Self {
        let c = self.crop;
        Self {
            quarter_turns: (self.quarter_turns + 1) % 4,
            // A turn swaps the axes; the new turn applies before the flip,
            // so with a flip it turns the other way on screen.
            crop: if self.flip_horizontal {
                Crop {
                    cx: c.cy,
                    cy: 1.0 - c.cx,
                    w: c.h,
                    h: c.w,
                    angle: c.angle,
                }
            } else {
                Crop {
                    cx: 1.0 - c.cy,
                    cy: c.cx,
                    w: c.h,
                    h: c.w,
                    angle: c.angle,
                }
            },
            ..*self
        }
    }

    /// The same region after a horizontal flip: the crop mirrors, and so
    /// does its angle.
    pub fn flipped(&self) -> Self {
        let c = self.crop;
        Self {
            flip_horizontal: !self.flip_horizontal,
            crop: Crop {
                cx: 1.0 - c.cx,
                angle: -c.angle,
                ..c
            },
            ..*self
        }
    }
}

/// The crop as the engine accepts it on an image whose full-resolution
/// stored size is `level0`, oriented by `o` (DEV-01, Crop):
///
/// - the angle is clamped to ±45°;
/// - a rotated rectangle that leaves the oriented image shrinks about its
///   center, keeping its aspect, until it fits;
/// - None when the result would be under 32 pixels on a side, so the delta
///   is refused and the crop keeps its last valid state (owner,
///   2026-10-20). The center is never moved.
pub fn fit(crop: Crop, level0: (u32, u32), o: u8) -> Option<Crop> {
    let oriented = shown(level0, o);
    let (w, h) = (f64::from(oriented.0), f64::from(oriented.1));
    let crop = Crop {
        angle: crop.angle.clamp(-MAX_ANGLE, MAX_ANGLE),
        w: crop.w.min(1.0),
        h: crop.h.min(1.0),
        ..crop
    };
    if !(0.0..=1.0).contains(&crop.cx) || !(0.0..=1.0).contains(&crop.cy) {
        return None;
    }
    // Each corner is the center plus s times its offset; the largest s
    // (up to 1) that keeps every corner inside.
    let c = [crop.cx * w, crop.cy * h];
    let mut s: f64 = 1.0;
    for corner in crop.corners(oriented) {
        for (axis, limit) in [w, h].into_iter().enumerate() {
            let k = corner[axis] - c[axis];
            if k > 0.0 {
                s = s.min((limit - c[axis]) / k);
            } else if k < 0.0 {
                s = s.min(c[axis] / -k);
            }
        }
    }
    let fitted = Crop {
        w: crop.w * s,
        h: crop.h * s,
        ..crop
    };
    let (cw, ch) = fitted.pixels(oriented);
    // A hair under, from rounding in s, still counts as the minimum.
    (cw >= MIN_CROP - 1e-9 && ch >= MIN_CROP - 1e-9).then_some(fitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::sampling::orientation;

    /// Where a crop's corners land in the stored image, normalized, as a
    /// sorted set (a turn or flip changes which corner is which).
    fn stored_corners(g: &Geometry, file: u8, level0: (u32, u32)) -> Vec<[i64; 2]> {
        let o = g.orientation(file);
        let oriented = shown(level0, o);
        let r = orientation(o);
        let mut out: Vec<[i64; 2]> = g
            .crop
            .corners(oriented)
            .iter()
            .map(|p| {
                let (a, b) = (p[0] / f64::from(oriented.0), p[1] / f64::from(oriented.1));
                [0, 1].map(|i| ((r[i][0] * a + r[i][1] * b + r[i][2]) * 1e9).round() as i64)
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn turns_and_flips_compose_as_exif_says() {
        // One clockwise turn of an upright image is EXIF 6, three are 8,
        // two are 3; a flip is 2.
        assert_eq!(compose(1, 1, false), 6);
        assert_eq!(compose(1, 2, false), 3);
        assert_eq!(compose(1, 3, false), 8);
        assert_eq!(compose(1, 0, true), 2);
        // A vertical flip (4) is a horizontal flip and two turns.
        assert_eq!(compose(1, 2, true), 4);
        for o in 1..=8 {
            assert_eq!(compose(o, 0, false), o);
            assert_eq!(compose(o, 4, false), o, "four turns");
            assert_eq!(compose(compose(o, 0, true), 0, true), o, "two flips");
            // A turn after the file's 8 (counterclockwise) undoes it.
            assert_eq!(compose(8, 1, false), 1);
        }
    }

    #[test]
    fn a_turn_or_flip_keeps_the_selected_region() {
        let level0 = (6000, 4000);
        let crop = Crop {
            cx: 0.4,
            cy: 0.55,
            w: 0.3,
            h: 0.5,
            angle: 12.0,
        };
        for file in 1..=8 {
            for flip in [false, true] {
                let g = Geometry {
                    quarter_turns: 0,
                    flip_horizontal: flip,
                    crop,
                };
                let want = stored_corners(&g, file, level0);
                let mut t = g;
                for turn in 1..=4 {
                    t = t.turned();
                    assert_eq!(
                        stored_corners(&t, file, level0),
                        want,
                        "{file} {flip} {turn}"
                    );
                }
                assert_eq!(t, g, "four turns are the identity");
                let f = g.flipped();
                assert_eq!(stored_corners(&f, file, level0), want, "flip");
                assert_eq!(f.flipped(), g);
                // Straightening keeps the crop's pixel aspect through a turn.
                let a = g.crop.pixels(shown(level0, g.orientation(file)));
                let b = g
                    .turned()
                    .crop
                    .pixels(shown(level0, g.turned().orientation(file)));
                assert!((a.0 - b.1).abs() < 1e-9 && (a.1 - b.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn the_fit_shrinks_about_the_center() {
        let level0 = (4000, 3000);
        // Inside already: unchanged.
        let inside = Crop {
            cx: 0.5,
            cy: 0.5,
            w: 0.5,
            h: 0.5,
            angle: 10.0,
        };
        assert_eq!(fit(inside, level0, 1), Some(inside));
        // The whole image at 20°: it shrinks, keeps its center and aspect,
        // and every corner touches or stays inside.
        let tilted = Crop {
            angle: 20.0,
            ..Crop::FULL
        };
        let got = fit(tilted, level0, 1).unwrap();
        assert_eq!((got.cx, got.cy, got.angle), (0.5, 0.5, 20.0));
        assert!((got.w - got.h).abs() < 1e-12 && got.w < 1.0);
        let corners = got.corners(level0);
        assert!(corners.iter().all(|c| {
            (-1e-9..=4000.0 + 1e-9).contains(&c[0]) && (-1e-9..=3000.0 + 1e-9).contains(&c[1])
        }));
        assert!(
            corners
                .iter()
                .any(|c| c[0].abs() < 1e-6 || c[1].abs() < 1e-6),
            "it touches an edge"
        );
        // The angle is clamped to 45°, either way.
        assert_eq!(
            fit(
                Crop {
                    angle: 60.0,
                    ..inside
                },
                level0,
                1
            )
            .unwrap()
            .angle,
            45.0
        );
        // Near the edge no 32 px crop fits about that center: refused.
        let edge = Crop {
            cx: 10.0 / 4000.0,
            cy: 0.5,
            w: 0.2,
            h: 0.2,
            angle: 0.0,
        };
        assert_eq!(fit(edge, level0, 1), None);
        // 16 px from the edge, straight and square (600 px), the minimum
        // just fits: shrinking keeps the aspect, so a wider crop would not.
        let just = Crop {
            cx: 16.0 / 4000.0,
            w: 0.15,
            ..edge
        };
        let got = fit(just, level0, 1).unwrap();
        assert!((got.w * 4000.0 - 32.0).abs() < 1e-9, "{}", got.w * 4000.0);
        // Under the minimum by request: refused too.
        let tiny = Crop {
            w: 20.0 / 4000.0,
            ..inside
        };
        assert_eq!(fit(tiny, level0, 1), None);
        // Sizes are in the oriented image: a turned file swaps them.
        let wide = Crop {
            w: 1.0,
            h: 0.01,
            ..inside
        };
        assert!(fit(wide, level0, 1).is_none(), "30 px high");
        assert!(fit(wide, level0, 6).is_some(), "40 px high once turned");
    }
}
