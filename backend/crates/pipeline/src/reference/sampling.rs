//! CPU reference for sampling the pyramid into the view (DEV-01, Sampling):
//! the smallest level at least as large as the output, a Lanczos-2 filter
//! sized to the remaining scale factor, and one coordinate transform from
//! output pixels to level texels. The transform holds the file's
//! orientation composed with the edit's quarter turns and flip, and the
//! crop with its straighten angle ([`Frame`]).

use crate::geometry::{Crop, Geometry};

/// Where an output pixel's center lands in a level, and how wide a filter
/// that needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Sampling {
    /// The level: 0 is full resolution (the level-0 tiles), 1 (half size)
    /// and up the pyramid's.
    pub level: u32,
    pub level_size: (u32, u32),
    /// Level coordinates of output pixel (x, y)'s center:
    /// `affine[r] · (x + 0.5, y + 0.5, 1)`, `r` 0 for across, 1 for down.
    pub affine: [[f64; 3]; 2],
    /// Per level axis, the filter's scale: level texels per output pixel,
    /// at least 1 (upsampling keeps the plain Lanczos-2).
    pub footprint: [f64; 2],
}

/// The file's orientation (EXIF, 1 to 8) as a map from the shown image's
/// normalized coordinates (a, b) to the stored image's (x, y), both 0 to 1:
/// rows `[a, b, 1]` coefficients for x and for y.
pub fn orientation(o: u8) -> [[f64; 3]; 2] {
    match o {
        2 => [[-1.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        3 => [[-1.0, 0.0, 1.0], [0.0, -1.0, 1.0]],
        4 => [[1.0, 0.0, 0.0], [0.0, -1.0, 1.0]],
        5 => [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        6 => [[0.0, 1.0, 0.0], [-1.0, 0.0, 1.0]],
        7 => [[0.0, -1.0, 1.0], [-1.0, 0.0, 1.0]],
        8 => [[0.0, -1.0, 1.0], [1.0, 0.0, 0.0]],
        // 1, and values outside 1 to 8, which EXIF does not define.
        _ => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    }
}

/// Whether orientation `o` turns the image a quarter, swapping its sides.
pub fn swaps(o: u8) -> bool {
    (5..=8).contains(&o)
}

/// The shown size of a stored `size` in orientation `o`.
pub fn shown(size: (u32, u32), o: u8) -> (u32, u32) {
    if swaps(o) { (size.1, size.0) } else { size }
}

/// The sampling of `levels` (level 1 first, stored orientation) into an
/// `out`-sized view of the whole image shown in orientation `o`.
pub fn plan(levels: &[(u32, u32)], o: u8, out: (u32, u32)) -> Sampling {
    assert!(!levels.is_empty() && out.0 > 0 && out.1 > 0);
    // The smallest level at least as large as the output on both sides,
    // else level 1.
    let index = (0..levels.len())
        .rev()
        .find(|&i| {
            let s = shown(levels[i], o);
            s.0 >= out.0 && s.1 >= out.1
        })
        .unwrap_or(0);
    let (lw, lh) = levels[index];
    let r = orientation(o);
    let (ow, oh) = (f64::from(out.0), f64::from(out.1));
    let dims = [f64::from(lw), f64::from(lh)];
    // Level = dims · (R · (px / ow, py / oh) + t), folded into one affine
    // so that equal sizes give a scale of exactly 1.
    let affine = [0, 1].map(|i| {
        [
            dims[i] * r[i][0] / ow,
            dims[i] * r[i][1] / oh,
            dims[i] * r[i][2],
        ]
    });
    let footprint = [0, 1].map(|i| affine[i][0].abs().max(affine[i][1].abs()).max(1.0));
    Sampling {
        level: index as u32 + 1,
        level_size: (lw, lh),
        affine,
        footprint,
    }
}

/// The image a view shows: the stored image oriented by `orientation` (the
/// file's composed with the edit's turns and flip, `geometry::compose`),
/// then cropped and straightened. Without a crop it is the whole oriented
/// image, sampled exactly as before crops existed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub orientation: u8,
    pub crop: Option<Crop>,
}

impl From<u8> for Frame {
    /// The whole image in orientation `o`.
    fn from(o: u8) -> Self {
        Self {
            orientation: o,
            crop: None,
        }
    }
}

impl Frame {
    /// The frame of `geometry` on a file whose orientation is `file`; no
    /// crop when it is the whole image, straight.
    pub fn of(geometry: &Geometry, file: u8) -> Self {
        Self {
            orientation: geometry.orientation(file),
            crop: (geometry.crop != Crop::FULL).then_some(geometry.crop),
        }
    }

    /// The framed image's size in source pixels (the crop's, unrounded), of
    /// a stored image whose full resolution is `level0`.
    pub fn size(&self, level0: (u32, u32)) -> (f64, f64) {
        let oriented = shown(level0, self.orientation);
        match self.crop {
            None => (f64::from(oriented.0), f64::from(oriented.1)),
            Some(c) => c.pixels(oriented),
        }
    }

    /// The framed image's size in whole pixels, at least 1 on each side:
    /// what a Fit view of it is sized from.
    pub fn pixel_size(&self, level0: (u32, u32)) -> (u32, u32) {
        let (w, h) = self.size(level0);
        ((w.round() as u32).max(1), (h.round() as u32).max(1))
    }

    /// The map from framed source pixels (`(u, v, 1)`, 0 to the frame's
    /// size) to oriented source pixels: the crop's center plus the offset,
    /// rotated. Straight, it is a shift by the crop's top left.
    fn oriented_map(&self, level0: (u32, u32), crop: &Crop) -> [[f64; 3]; 2] {
        let oriented = shown(level0, self.orientation);
        let (w, h) = (f64::from(oriented.0), f64::from(oriented.1));
        let (cw, ch) = crop.pixels(oriented);
        let (sin, cos) = crop.angle.to_radians().sin_cos();
        [
            [cos, -sin, crop.cx * w - cos * cw / 2.0 + sin * ch / 2.0],
            [sin, cos, crop.cy * h - sin * cw / 2.0 - cos * ch / 2.0],
        ]
    }
}

/// What a view shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewSpec {
    /// The whole image in an `out`-sized view.
    Fit((u32, u32)),
    /// An `out`-sized view of the shown image at `zoom` (view pixels per
    /// image pixel; 1 is 1:1) whose top left is at `origin` (shown image
    /// pixels: the framed image's, when cropped).
    Region {
        out: (u32, u32),
        zoom: f64,
        origin: (f64, f64),
    },
}

impl ViewSpec {
    pub fn out(&self) -> (u32, u32) {
        match *self {
            ViewSpec::Fit(out) | ViewSpec::Region { out, .. } => out,
        }
    }
}

/// The sampling for `view` of an image whose level 0 (full resolution, the
/// active area) is `level0` and whose pyramid levels are `levels` (level 1
/// first), shown in orientation `o`: the smallest level at least as large
/// as the view needs, level 0 when no pyramid level is (decided
/// 2026-10-19), and level 0 past 1:1.
pub fn plan_view(
    level0: (u32, u32),
    levels: &[(u32, u32)],
    frame: impl Into<Frame>,
    view: ViewSpec,
) -> Sampling {
    plan_levels(level0, levels, frame.into(), view, true)
}

/// As [`plan_view`], from the pyramid's levels only: what a view falls back
/// to when the level-0 tiles it needs do not fit the cache. Level 1 is
/// upsampled where level 0 would have been read.
pub fn plan_view_without_level0(
    level0: (u32, u32),
    levels: &[(u32, u32)],
    frame: impl Into<Frame>,
    view: ViewSpec,
) -> Sampling {
    plan_levels(level0, levels, frame.into(), view, false)
}

fn plan_levels(
    level0: (u32, u32),
    levels: &[(u32, u32)],
    frame: Frame,
    view: ViewSpec,
    with_level0: bool,
) -> Sampling {
    let o = frame.orientation;
    if let Some(crop) = frame.crop {
        return plan_cropped(level0, levels, &frame, &crop, view, with_level0);
    }
    match view {
        ViewSpec::Fit(out) => {
            let fits = |l: (u32, u32)| {
                let s = shown(l, o);
                s.0 >= out.0 && s.1 >= out.1
            };
            if !levels.is_empty() && (fits(levels[0]) || !with_level0) {
                return plan(levels, o, out);
            }
            let mut s = plan(&[level0], o, out);
            s.level = 0;
            s
        }
        ViewSpec::Region { out, zoom, origin } => {
            assert!(out.0 > 0 && out.1 > 0 && zoom > 0.0);
            // Level texels per image pixel on each shown axis must reach the
            // zoom.
            let full = shown(level0, o);
            let all: Vec<(u32, u32)> = std::iter::once(level0)
                .chain(levels.iter().copied())
                .collect();
            let index = (0..all.len())
                .rev()
                .find(|&i| {
                    let s = shown(all[i], o);
                    f64::from(s.0) >= f64::from(full.0) * zoom
                        && f64::from(s.1) >= f64::from(full.1) * zoom
                })
                .unwrap_or(0);
            let index = if with_level0 { index } else { index.max(1) };
            let (lw, lh) = all[index];
            let r = orientation(o);
            let dims = [f64::from(lw), f64::from(lh)];
            let shown_dims = [f64::from(full.0), f64::from(full.1)];
            let origin = [origin.0, origin.1];
            // Level_i = Σ_j R_ij · (dims_i / shown_j) · (origin_j + p_j / zoom)
            //   + t_i · dims_i; level 0 at 1:1 has every scale exactly 1.
            let affine = [0, 1].map(|i| {
                let scale = [0, 1].map(|j| dims[i] / shown_dims[j]);
                [
                    r[i][0] * scale[0] / zoom,
                    r[i][1] * scale[1] / zoom,
                    r[i][0] * scale[0] * origin[0]
                        + r[i][1] * scale[1] * origin[1]
                        + r[i][2] * dims[i],
                ]
            });
            let footprint = [0, 1].map(|i| affine[i][0].abs().max(affine[i][1].abs()).max(1.0));
            Sampling {
                level: index as u32,
                level_size: (lw, lh),
                affine,
                footprint,
            }
        }
    }
}

/// [`plan_levels`] for a cropped frame: one affine from view pixels through
/// framed source pixels, the straightened crop and the orientation, to the
/// level's texels. The level is the smallest whose texels per view pixel
/// reach 1 along both of the view's axes (Fit), or whose texels per source
/// pixel reach the zoom (a region), as without a crop; the filter's scale
/// per level axis is the length of that axis's row of the affine.
fn plan_cropped(
    level0: (u32, u32),
    levels: &[(u32, u32)],
    frame: &Frame,
    crop: &Crop,
    view: ViewSpec,
    with_level0: bool,
) -> Sampling {
    let px = frame.oriented_map(level0, crop);
    let oriented = shown(level0, frame.orientation);
    let dims = [f64::from(oriented.0), f64::from(oriented.1)];
    let r = orientation(frame.orientation);
    let (fw, fh) = frame.size(level0);
    // View pixels to framed source pixels: per axis a scale and an offset.
    let (scale, offset) = match view {
        ViewSpec::Fit(out) => {
            assert!(out.0 > 0 && out.1 > 0);
            ([fw / f64::from(out.0), fh / f64::from(out.1)], [0.0, 0.0])
        }
        ViewSpec::Region { out, zoom, origin } => {
            assert!(out.0 > 0 && out.1 > 0 && zoom > 0.0);
            ([1.0 / zoom; 2], [origin.0, origin.1])
        }
    };
    // Level_i = sum_j R_ij * (d_i / dims_j) * oriented_j + t_i * d_i: as
    // without a crop, one division per axis pair, so level 0 straight at
    // 1:1 has every scale exactly 1.
    let affine_for = |level: (u32, u32)| -> [[f64; 3]; 2] {
        let d = [f64::from(level.0), f64::from(level.1)];
        [0, 1].map(|i| {
            let k = [0, 1].map(|j| r[i][j] * (d[i] / dims[j]));
            let row = [
                k[0] * px[0][0] + k[1] * px[1][0],
                k[0] * px[0][1] + k[1] * px[1][1],
                k[0] * px[0][2] + k[1] * px[1][2] + r[i][2] * d[i],
            ];
            [
                row[0] * scale[0],
                row[1] * scale[1],
                row[0] * offset[0] + row[1] * offset[1] + row[2],
            ]
        })
    };
    let all: Vec<(u32, u32)> = std::iter::once(level0)
        .chain(levels.iter().copied())
        .collect();
    // Texels per view pixel along each view axis, the affine's columns,
    // reach 1: at a zoom, the level's texels per source pixel reach it.
    let enough = |dims: (u32, u32)| {
        let a = affine_for(dims);
        (0..2).all(|j| a[0][j].hypot(a[1][j]) >= 1.0 - 1e-9)
    };
    let first = if with_level0 { 0 } else { 1 };
    let index = (first..all.len())
        .rev()
        .find(|&i| enough(all[i]))
        .unwrap_or(first);
    let affine = affine_for(all[index]);
    let footprint = [0, 1].map(|i| affine[i][0].hypot(affine[i][1]).max(1.0));
    Sampling {
        level: index as u32,
        level_size: all[index],
        affine,
        footprint,
    }
}

impl Sampling {
    /// The level texels an `out`-sized view reads, filter reach included,
    /// clamped to the level.
    pub fn source_rect(&self, out: (u32, u32)) -> genzo_model::frame::Rect {
        let a = &self.affine;
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (x, y) in [
            (0.0, 0.0),
            (f64::from(out.0), 0.0),
            (0.0, f64::from(out.1)),
            (f64::from(out.0), f64::from(out.1)),
        ] {
            for i in 0..2 {
                let v = a[i][0] * x + a[i][1] * y + a[i][2];
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
            }
        }
        let size = [self.level_size.0, self.level_size.1];
        let [x0, y0] = [0, 1].map(|i| {
            let reach = 2.0 * self.footprint[i] + 1.0;
            (lo[i] - reach).floor().max(0.0).min(f64::from(size[i])) as u32
        });
        let [x1, y1] = [0, 1].map(|i| {
            let reach = 2.0 * self.footprint[i] + 1.0;
            (hi[i] + reach).ceil().max(0.0).min(f64::from(size[i])) as u32
        });
        genzo_model::frame::Rect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

/// Lanczos-2: `sinc(t) · sinc(t / 2)` inside |t| < 2. Integer `t` other
/// than 0 weighs exactly 0, so sampling a level at its own texel centers is
/// an exact copy.
pub fn lanczos2(t: f64) -> f64 {
    let t = t.abs();
    if t == 0.0 {
        return 1.0;
    }
    if t >= 2.0 || t.fract() == 0.0 {
        return 0.0;
    }
    let p = std::f64::consts::PI * t;
    2.0 * p.sin() * (p / 2.0).sin() / (p * p)
}

/// The taps along one axis for a center `c` in level texels: first index
/// and the normalized weights, indices clamped to the level later.
pub fn taps(c: f64, footprint: f64) -> (i64, Vec<f64>) {
    let reach = 2.0 * footprint;
    let first = (c - 0.5 - reach).floor() as i64 + 1;
    let last = (c - 0.5 + reach).ceil() as i64 - 1;
    let mut w: Vec<f64> = (first..=last)
        .map(|j| lanczos2((j as f64 + 0.5 - c) / footprint))
        .collect();
    let sum: f64 = w.iter().sum();
    for v in &mut w {
        *v /= sum;
    }
    (first, w)
}

/// The view: `level` (RGB, row by row, `size`) sampled per `s` into `out`.
pub fn sample(
    level: &[[f32; 3]],
    size: (u32, u32),
    s: &Sampling,
    out: (u32, u32),
) -> Vec<[f32; 3]> {
    let (w, h) = (i64::from(size.0), i64::from(size.1));
    let mut image = Vec::with_capacity(out.0 as usize * out.1 as usize);
    for py in 0..out.1 {
        for px in 0..out.0 {
            let (x, y) = (f64::from(px) + 0.5, f64::from(py) + 0.5);
            let a = &s.affine;
            let cx = a[0][0] * x + a[0][1] * y + a[0][2];
            let cy = a[1][0] * x + a[1][1] * y + a[1][2];
            let (fx, wx) = taps(cx, s.footprint[0]);
            let (fy, wy) = taps(cy, s.footprint[1]);
            let mut sum = [0f64; 3];
            for (j, wy) in wy.iter().enumerate() {
                let row = (fy + j as i64).clamp(0, h - 1);
                for (i, wx) in wx.iter().enumerate() {
                    let col = (fx + i as i64).clamp(0, w - 1);
                    let t = level[(row * w + col) as usize];
                    for c in 0..3 {
                        sum[c] += wy * wx * f64::from(t[c]);
                    }
                }
            }
            image.push(sum.map(|v| v as f32));
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_map_corners_as_exif_says() {
        // Where the shown image's top-left and top-right corners come from.
        let at = |o, a: f64, b: f64| {
            let r = orientation(o);
            [0, 1].map(|i| r[i][0] * a + r[i][1] * b + r[i][2])
        };
        // 6 turns the stored image 90 degrees clockwise: the shown top
        // left is the stored bottom left.
        assert_eq!(at(6, 0.0, 0.0), [0.0, 1.0]);
        assert_eq!(at(6, 1.0, 0.0), [0.0, 0.0]);
        // 8 turns it counterclockwise: the shown top left is the stored
        // top right.
        assert_eq!(at(8, 0.0, 0.0), [1.0, 0.0]);
        assert_eq!(at(3, 0.0, 0.0), [1.0, 1.0]);
        assert_eq!(at(2, 0.0, 0.0), [1.0, 0.0]);
        assert_eq!(at(4, 0.0, 0.0), [0.0, 1.0]);
        assert_eq!(at(5, 1.0, 0.0), [0.0, 1.0]);
        assert_eq!(at(7, 0.0, 0.0), [1.0, 1.0]);
        for o in 1..=8 {
            // Each is a bijection of the unit square's corners.
            let mut corners: Vec<_> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                .iter()
                .map(|&(a, b)| at(o, a, b).map(|v| v as i32))
                .collect();
            corners.sort();
            assert_eq!(corners, [[0, 0], [0, 1], [1, 0], [1, 1]], "{o}");
        }
    }

    /// A crop of the whole image, straight, plans as no crop does: the same
    /// level, and the same affine and filter to rounding, in every
    /// orientation, at Fit and at a zoom.
    #[test]
    fn a_full_crop_plans_as_no_crop() {
        let level0 = (4000, 3000);
        let levels = [(2000, 1500), (1000, 750), (500, 375), (250, 187)];
        for o in 1..=8 {
            let full = Frame {
                orientation: o,
                crop: Some(Crop::FULL),
            };
            let s = shown(level0, o);
            for view in [
                ViewSpec::Fit((s.0 / 3, s.1 / 3)),
                ViewSpec::Fit((s.0 / 9, s.1 / 9)),
                ViewSpec::Fit(s),
                ViewSpec::Region {
                    out: (640, 480),
                    zoom: 0.37,
                    origin: (123.0, 45.5),
                },
                ViewSpec::Region {
                    out: (640, 480),
                    zoom: 1.0,
                    origin: (700.0, 300.0),
                },
            ] {
                for with in [true, false] {
                    let (a, b) = if with {
                        (
                            plan_view(level0, &levels, o, view),
                            plan_view(level0, &levels, full, view),
                        )
                    } else {
                        (
                            plan_view_without_level0(level0, &levels, o, view),
                            plan_view_without_level0(level0, &levels, full, view),
                        )
                    };
                    assert_eq!(
                        (a.level, a.level_size),
                        (b.level, b.level_size),
                        "{o} {view:?}"
                    );
                    for i in 0..2 {
                        assert!((a.footprint[i] - b.footprint[i]).abs() < 1e-9);
                        for k in 0..3 {
                            assert!(
                                (a.affine[i][k] - b.affine[i][k]).abs() < 1e-9,
                                "{o} {view:?}: {:?} vs {:?}",
                                a.affine,
                                b.affine
                            );
                        }
                    }
                }
            }
        }
    }

    /// A straight crop on whole pixels at 1:1 is an exact copy of that
    /// region of level 0, in every orientation and with turns and flips.
    #[test]
    fn a_straight_crop_at_one_to_one_is_a_copy() {
        let level0 = (40, 30);
        let image: Vec<[f32; 3]> = (0..1200)
            .map(|i| [i as f32, (i * 7 % 13) as f32, 0.5])
            .collect();
        let levels = [(20, 15), (10, 7)];
        for file in 1..=8 {
            for (turns, flip) in [(0, false), (1, false), (3, true)] {
                let g = Geometry {
                    quarter_turns: turns,
                    flip_horizontal: flip,
                    crop: Crop::FULL,
                };
                let o = g.orientation(file);
                let oriented = shown(level0, o);
                let (ow, oh) = (f64::from(oriented.0), f64::from(oriented.1));
                // 16 x 12 source pixels from (4, 6) of the oriented image.
                let crop = Crop {
                    cx: (4.0 + 8.0) / ow,
                    cy: (6.0 + 6.0) / oh,
                    w: 16.0 / ow,
                    h: 12.0 / oh,
                    angle: 0.0,
                };
                let frame = Frame::of(&Geometry { crop, ..g }, file);
                assert_eq!(frame.pixel_size(level0), (16, 12));
                let view = ViewSpec::Region {
                    out: (10, 8),
                    zoom: 1.0,
                    origin: (3.0, 2.0),
                };
                let s = plan_view(level0, &levels, frame, view);
                assert_eq!(s.level, 0);
                let got = sample(&image, level0, &s, (10, 8));
                // The same pixels through the uncropped 1:1 view, shifted.
                let whole = plan_view(
                    level0,
                    &levels,
                    o,
                    ViewSpec::Region {
                        out: (10, 8),
                        zoom: 1.0,
                        origin: (7.0, 8.0),
                    },
                );
                let want = sample(&image, level0, &whole, (10, 8));
                assert_eq!(got, want, "file {file}, turns {turns}, flip {flip}");
            }
        }
    }

    /// Straightened, the view's corners land on the rotated crop's corners,
    /// and a tight crop at Fit reads level 0 where the pyramid is too small.
    #[test]
    fn a_straightened_crop_maps_onto_its_corners() {
        let level0 = (4000, 3000);
        let levels = [(2000, 1500), (1000, 750), (500, 375), (250, 187)];
        let crop = Crop {
            cx: 0.45,
            cy: 0.6,
            w: 0.3,
            h: 0.25,
            angle: 17.0,
        };
        let frame = Frame {
            orientation: 1,
            crop: Some(crop),
        };
        let size = frame.size(level0);
        assert!((size.0 - 1200.0).abs() < 1e-9 && (size.1 - 750.0).abs() < 1e-9);
        let out = (600, 375);
        let s = plan_view(level0, &levels, frame, ViewSpec::Fit(out));
        // Two source pixels per view pixel: level 1 has one texel each.
        assert_eq!(s.level, 1);
        let corners = crop.corners(level0);
        let a = &s.affine;
        for (k, (x, y)) in [(0.0, 0.0), (600.0, 0.0), (600.0, 375.0), (0.0, 375.0)]
            .into_iter()
            .enumerate()
        {
            let at = [0, 1].map(|i| a[i][0] * x + a[i][1] * y + a[i][2]);
            for i in 0..2 {
                assert!((at[i] * 2.0 - corners[k][i]).abs() < 1e-9, "corner {k}");
            }
        }
        // The same crop in a view 4 times larger needs level 0.
        let big = plan_view(level0, &levels, frame, ViewSpec::Fit((2400, 1500)));
        assert_eq!(big.level, 0);
        let fallback =
            plan_view_without_level0(level0, &levels, frame, ViewSpec::Fit((2400, 1500)));
        assert_eq!(fallback.level, 1);
        // The filter's scale is the length of each level axis's row.
        assert!((s.footprint[0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lanczos_weights_are_normalized_and_vanish_at_integers() {
        assert_eq!(lanczos2(1.0), 0.0);
        assert_eq!(lanczos2(-1.0), 0.0);
        assert_eq!(lanczos2(2.0), 0.0);
        for c in [10.5, 10.25, 3.9] {
            for f in [1.0, 1.3, 1.99] {
                let (_, w) = taps(c, f);
                assert!(w.len() <= 8, "{} taps", w.len());
                assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            }
        }
        let (first, w) = taps(10.5, 1.0);
        assert_eq!(first, 9);
        assert_eq!(w, [0.0, 1.0, 0.0]);
    }

    /// The level whose size the view has is copied exactly, in every
    /// orientation (rotated or mirrored).
    #[test]
    fn a_view_the_size_of_a_level_is_a_copy() {
        let levels = [(40, 30), (20, 15), (10, 7)];
        let size = levels[1];
        let level: Vec<[f32; 3]> = (0..size.0 * size.1)
            .map(|i| [i as f32, (i * 7 % 13) as f32, 0.5])
            .collect();
        for o in 1..=8 {
            let out = shown(size, o);
            let s = plan(&levels, o, out);
            assert_eq!(s.level, 2, "{o}");
            let view = sample(&level, size, &s, out);
            for py in 0..out.1 {
                for px in 0..out.0 {
                    let r = orientation(o);
                    let (a, b) = (
                        (f64::from(px) + 0.5) / f64::from(out.0),
                        (f64::from(py) + 0.5) / f64::from(out.1),
                    );
                    let x = ((r[0][0] * a + r[0][1] * b + r[0][2]) * f64::from(size.0)) as u32;
                    let y = ((r[1][0] * a + r[1][1] * b + r[1][2]) * f64::from(size.1)) as u32;
                    assert_eq!(
                        view[(py * out.0 + px) as usize],
                        level[(y * size.0 + x) as usize],
                        "orientation {o} at {px}, {py}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_level_is_the_smallest_large_enough() {
        let levels = [(2016, 1512), (1008, 756), (504, 378), (252, 189)];
        assert_eq!(plan(&levels, 1, (1000, 750)).level, 2);
        assert_eq!(plan(&levels, 1, (1008, 756)).level, 2);
        assert_eq!(plan(&levels, 1, (1009, 700)).level, 1);
        assert_eq!(plan(&levels, 1, (200, 150)).level, 4);
        // Turned: the shown width is the stored height.
        assert_eq!(plan(&levels, 6, (756, 1008)).level, 2);
        // Larger than level 1: level 1, upsampled.
        let s = plan(&levels, 1, (2560, 1920));
        assert_eq!((s.level, s.footprint), (1, [1.0, 1.0]));
    }

    /// At 1:1 from level 0, a view is an exact copy of its region, in every
    /// orientation, at any integer origin.
    #[test]
    fn one_to_one_is_a_copy_of_level_0() {
        let size = (40, 30);
        let level: Vec<[f32; 3]> = (0..size.0 * size.1).map(|i| [i as f32, 0.0, 1.0]).collect();
        for o in 1..=8 {
            let full = shown(size, o);
            let (out, origin) = ((13, 9), (7.0, 5.0));
            let s = plan_view(
                size,
                &[(20, 15)],
                o,
                ViewSpec::Region {
                    out,
                    zoom: 1.0,
                    origin,
                },
            );
            assert_eq!((s.level, s.footprint), (0, [1.0, 1.0]), "{o}");
            let view = sample(&level, size, &s, out);
            for py in 0..out.1 {
                for px in 0..out.0 {
                    // The shown pixel, then the stored one by the corner map.
                    let (u, v) = (7 + px, 5 + py);
                    let r = orientation(o);
                    let (a, b) = (
                        (f64::from(u) + 0.5) / f64::from(full.0),
                        (f64::from(v) + 0.5) / f64::from(full.1),
                    );
                    let x = ((r[0][0] * a + r[0][1] * b + r[0][2]) * f64::from(size.0)) as u32;
                    let y = ((r[1][0] * a + r[1][1] * b + r[1][2]) * f64::from(size.1)) as u32;
                    assert_eq!(
                        view[(py * out.0 + px) as usize],
                        level[(y * size.0 + x) as usize],
                        "orientation {o} at {px}, {py}"
                    );
                }
            }
            let rect = s.source_rect(out);
            assert!(
                rect.width >= 13.min(full.0) && rect.width <= size.0,
                "{rect:?}"
            );
        }
    }

    #[test]
    fn fit_reads_level_0_when_level_1_is_too_small() {
        let level0 = (4032, 3024);
        let levels = [(2016, 1512), (1008, 756)];
        assert_eq!(
            plan_view(level0, &levels, 1, ViewSpec::Fit((2000, 1500))).level,
            1
        );
        let s = plan_view(level0, &levels, 1, ViewSpec::Fit((2560, 1920)));
        assert_eq!(s.level, 0);
        assert!(s.footprint[0] > 1.5 && s.footprint[0] < 1.6);
        assert_eq!(
            plan_view(level0, &levels, 1, ViewSpec::Fit((5000, 4000))).footprint,
            [1.0, 1.0]
        );
        // A region at half zoom reads level 1; at 1:1 and past, level 0.
        let at = |zoom| {
            plan_view(
                level0,
                &levels,
                1,
                ViewSpec::Region {
                    out: (800, 600),
                    zoom,
                    origin: (0.0, 0.0),
                },
            )
            .level
        };
        assert_eq!(
            (at(0.5), at(0.6), at(1.0), at(2.0), at(0.25)),
            (1, 0, 0, 0, 2)
        );
    }

    #[test]
    fn a_flat_level_stays_flat_at_any_scale() {
        let size = (37, 29);
        let level = vec![[0.25f32, 0.5, 0.75]; (size.0 * size.1) as usize];
        for out in [(37, 29), (25, 20), (19, 15), (60, 47)] {
            let s = plan(&[size], 1, out);
            for v in sample(&level, size, &s, out) {
                for (c, want) in v.iter().zip([0.25, 0.5, 0.75]) {
                    assert!((c - want).abs() < 1e-6, "{out:?}: {v:?}");
                }
            }
        }
    }
}
