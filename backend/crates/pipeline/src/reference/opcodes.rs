//! CPU reference for the DNG opcodes the pipeline applies (DEV-01, stage 1),
//! written to the Adobe DNG SDK's behavior (`dng_gain_map.cpp`,
//! `dng_bad_pixels.cpp`, `dng_misc_opcodes.cpp`). The GPU stage 1 is tested
//! against these.
//!
//! Coordinates are those of the image the opcode list runs on: for
//! OpcodeList1 the raw image as stored, for OpcodeList2 and 3 the image after
//! linearization, which starts at the ActiveArea's top left. The caller
//! passes that image.

use genzo_imgio::opcodes::{AreaSpec, FixBadPixelsConstant, GainMap};

/// The rows and columns of `area` inside a `height` x `width` image, as
/// (top, left, bottom, right), aligned to the area's pitch from its own top
/// left (`dng_area_spec::Overlap`). None when they do not meet. An empty
/// area covers the whole image.
fn overlap(area: &AreaSpec, height: u32, width: u32) -> Option<(u32, u32, u32, u32)> {
    if area.top >= area.bottom || area.left >= area.right {
        return (height > 0 && width > 0).then_some((0, 0, height, width));
    }
    let (t, l) = (area.top, area.left);
    let (b, r) = (area.bottom.min(height), area.right.min(width));
    if t >= b || l >= r {
        return None;
    }
    // Already at the area's own top left, so the pitch phase holds.
    Some((t, l, b, r))
}

/// A map coordinate along one axis: the SDK's interpolation index for a
/// pixel's center, as (first point, second point, fraction).
fn map_index(pixel: u32, size: u32, origin: f64, spacing: f64, points: u32) -> (u32, u32, f32) {
    if points == 1 {
        return (0, 0, 0.0);
    }
    let f = ((f64::from(pixel) + 0.5) / f64::from(size) - origin) / spacing;
    if f <= 0.0 {
        return (0, 0, 0.0);
    }
    let i = f as u32;
    if i >= points - 1 {
        return (points - 1, points - 1, 0.0);
    }
    (i, i + 1, (f - f64::from(i)) as f32)
}

/// GainMap: multiplies each covered sample of `image` (interleaved, `planes`
/// per pixel, normalized so white is 1) by the map's gain at the pixel's
/// relative position, clipping at 1. Positions are of pixel centers, (row +
/// 0.5) / height, and the map is interpolated bilinearly and held constant
/// past its first and last points.
pub fn apply_gain_map(image: &mut [f32], width: u32, height: u32, planes: u32, gm: &GainMap) {
    let Some((t, l, b, r)) = overlap(&gm.area, height, width) else {
        return;
    };
    let area = &gm.area;
    let first = area.plane;
    let last = area.plane.saturating_add(area.planes).min(planes);
    for plane in first..last {
        let map_plane = plane.min(gm.map_planes - 1);
        for row in (t..b).step_by(area.row_pitch as usize) {
            let (v1, v2, fv) = map_index(row, height, gm.origin_v, gm.spacing_v, gm.points_v);
            for col in (l..r).step_by(area.col_pitch as usize) {
                let (h1, h2, fh) = map_index(col, width, gm.origin_h, gm.spacing_h, gm.points_h);
                let at = |v, h| gm.entry(v, h, map_plane);
                let top = at(v1, h1) * (1.0 - fh) + at(v1, h2) * fh;
                let bottom = at(v2, h1) * (1.0 - fh) + at(v2, h2) * fh;
                let gain = top * (1.0 - fv) + bottom * fv;
                let i = (row as usize * width as usize + col as usize) * planes as usize
                    + plane as usize;
                image[i] = (image[i] * gain).min(1.0);
            }
        }
    }
}

/// FixBadPixelsConstant on a Bayer image's stored values: every sample equal
/// to the constant becomes the rounded mean of its same-color neighbors that
/// are not bad, read from the original image. A green site, where the sum
/// of row, column, phase and half the phase is even, uses its four diagonal
/// neighbors; a red or blue site the four two pixels away on its row and
/// column. A sample with no good neighbor keeps its value.
///
/// The SDK pads the image at its edges; here a neighbor outside the image is
/// simply not counted, which only differs for bad pixels on the outer two
/// rows or columns.
pub fn fix_bad_pixels_constant(
    image: &mut [u16],
    width: u32,
    height: u32,
    op: &FixBadPixelsConstant,
) {
    let Ok(bad) = u16::try_from(op.constant) else {
        return; // No 16-bit sample can equal it.
    };
    let src = image.to_vec();
    let (w, h) = (width as i64, height as i64);
    let phase = i64::from(op.bayer_phase + (op.bayer_phase >> 1));
    let diagonal = [(-1, -1), (-1, 1), (1, -1), (1, 1)];
    let axes = [(-2, 0), (2, 0), (0, -2), (0, 2)];
    for row in 0..h {
        for col in 0..w {
            let i = (row * w + col) as usize;
            if src[i] != bad {
                continue;
            }
            let neighbors = if (row + col + phase) & 1 == 0 {
                &diagonal
            } else {
                &axes
            };
            let (mut count, mut total) = (0u32, 0u32);
            for &(dr, dc) in neighbors {
                let (y, x) = (row + dr, col + dc);
                if (0..h).contains(&y) && (0..w).contains(&x) {
                    let v = src[(y * w + x) as usize];
                    if v != bad {
                        count += 1;
                        total += u32::from(v);
                    }
                }
            }
            // The SDK's rounding: (sum + 2) >> 2 for four, else to nearest.
            if count == 4 {
                image[i] = ((total + 2) >> 2) as u16;
            } else if let Some(mean) = (total + (count >> 1)).checked_div(count) {
                image[i] = mean as u16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(top: u32, left: u32, bottom: u32, right: u32, pitch: u32) -> AreaSpec {
        AreaSpec {
            top,
            left,
            bottom,
            right,
            plane: 0,
            planes: 1,
            row_pitch: pitch,
            col_pitch: pitch,
        }
    }

    fn map(area: AreaSpec, points_v: u32, points_h: u32, gains: Vec<f32>) -> GainMap {
        GainMap {
            area,
            points_v,
            points_h,
            spacing_v: if points_v > 1 {
                1.0 / f64::from(points_v - 1)
            } else {
                1.0
            },
            spacing_h: if points_h > 1 {
                1.0 / f64::from(points_h - 1)
            } else {
                1.0
            },
            origin_v: 0.0,
            origin_h: 0.0,
            map_planes: 1,
            gains,
        }
    }

    #[test]
    fn interpolates_at_pixel_centers() {
        // Gains 1 and 3 across 4 columns: centers at 1/8, 3/8, 5/8, 7/8.
        let mut img = vec![0.25f32; 4];
        apply_gain_map(
            &mut img,
            4,
            1,
            1,
            &map(area(0, 0, 1, 4, 1), 1, 2, vec![1.0, 3.0]),
        );
        let want = [1.25f32, 1.75, 2.25, 2.75].map(|g| 0.25 * g);
        for (got, want) in img.iter().zip(want) {
            assert!((got - want).abs() < 1e-6, "{img:?}");
        }
    }

    #[test]
    fn clips_at_white_and_holds_past_the_map() {
        let mut img = vec![0.6f32; 4];
        apply_gain_map(
            &mut img,
            4,
            1,
            1,
            &map(area(0, 0, 1, 4, 1), 1, 1, vec![2.0]),
        );
        assert_eq!(img, [1.0; 4]);
        // Origin past the left edge: the first columns take the first gain.
        let mut gm = map(area(0, 0, 1, 4, 1), 1, 2, vec![2.0, 4.0]);
        gm.origin_h = 0.5;
        gm.spacing_h = 0.25;
        let mut img = vec![0.1f32; 4];
        apply_gain_map(&mut img, 4, 1, 1, &gm);
        assert!(
            (img[0] - 0.2).abs() < 1e-6 && (img[1] - 0.2).abs() < 1e-6,
            "{img:?}"
        );
    }

    #[test]
    fn follows_the_area_pitch_and_clamps_it_to_the_image() {
        // Rows from 1, every other row and column; the area runs past the
        // image (as Samsung's do), which only clamps it.
        let mut img = vec![0.5f32; 4 * 4];
        apply_gain_map(
            &mut img,
            4,
            4,
            1,
            &map(area(1, 0, 3000, 5000, 2), 1, 1, vec![1.5]),
        );
        for row in 0..4 {
            for col in 0..4 {
                let want = if row % 2 == 1 && col % 2 == 0 {
                    0.75
                } else {
                    0.5
                };
                assert_eq!(img[row * 4 + col], want, "({row}, {col})");
            }
        }
    }

    #[test]
    fn touches_only_its_planes() {
        let mut gm = map(area(0, 0, 0, 0, 1), 1, 1, vec![2.0]);
        gm.area.plane = 1;
        gm.area.planes = 1;
        let mut img = vec![0.25f32; 2 * 3];
        apply_gain_map(&mut img, 2, 1, 3, &gm);
        assert_eq!(img, [0.25, 0.5, 0.25, 0.25, 0.5, 0.25]);
    }

    #[test]
    fn fixes_bad_pixels_from_same_color_neighbors() {
        // 6 x 6, phase 0: green sites where row + col is even. Bad (0) at
        // one green and one red or blue site.
        let (w, h) = (6u32, 6u32);
        let mut img: Vec<u16> = (0..36).map(|i| 100 + i as u16).collect();
        let op = FixBadPixelsConstant {
            constant: 0,
            bayer_phase: 0,
        };
        // (2, 2): row + col + 0 even, a "green" site for this phase.
        img[2 * 6 + 2] = 0;
        // (2, 3): odd, a red/blue site.
        img[2 * 6 + 3] = 0;
        let src = img.clone();
        fix_bad_pixels_constant(&mut img, w, h, &op);
        let at = |r: usize, c: usize| u32::from(src[r * 6 + c]);
        let diag = at(1, 1) + at(1, 3) + at(3, 1) + at(3, 3);
        assert_eq!(u32::from(img[2 * 6 + 2]), (diag + 2) >> 2);
        // Its axis neighbors: (0,3), (4,3), (2,1), (2,5); none is bad.
        let axes = at(0, 3) + at(4, 3) + at(2, 1) + at(2, 5);
        assert_eq!(u32::from(img[2 * 6 + 3]), (axes + 2) >> 2);
        // Phase 1 swaps which sites are green.
        let mut img2 = src.clone();
        fix_bad_pixels_constant(
            &mut img2,
            w,
            h,
            &FixBadPixelsConstant {
                constant: 0,
                bayer_phase: 1,
            },
        );
        assert_eq!(u32::from(img2[2 * 6 + 2]), (axes_at(&src, 2, 2) + 2) >> 2);
        // At a corner, only the neighbors inside count.
        let mut img3 = src.clone();
        img3[0] = 0;
        fix_bad_pixels_constant(&mut img3, w, h, &op);
        assert_eq!(img3[0], src[7]); // only (1, 1) exists
    }

    fn axes_at(src: &[u16], r: usize, c: usize) -> u32 {
        [(r - 2, c), (r + 2, c), (r, c - 2), (r, c + 2)]
            .iter()
            .map(|&(y, x)| u32::from(src[y * 6 + x]))
            .sum()
    }

    #[test]
    fn leaves_images_without_bad_pixels_alone() {
        let mut img: Vec<u16> = (1..=36).collect();
        let before = img.clone();
        fix_bad_pixels_constant(
            &mut img,
            6,
            6,
            &FixBadPixelsConstant {
                constant: 0,
                bayer_phase: 2,
            },
        );
        assert_eq!(img, before);
    }
}
