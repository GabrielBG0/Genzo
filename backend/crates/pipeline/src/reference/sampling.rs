//! CPU reference for sampling the pyramid into the view (DEV-01, Sampling):
//! the smallest level at least as large as the output, a Lanczos-2 filter
//! sized to the remaining scale factor, and one coordinate transform from
//! output pixels to level texels. The transform holds the file's
//! orientation now; the edit's quarter turns, flip and crop compose into it
//! later.

/// Where an output pixel's center lands in a level, and how wide a filter
/// that needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Sampling {
    /// The pyramid level, 1 (half size) and up.
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
