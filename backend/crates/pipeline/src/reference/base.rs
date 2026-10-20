//! The base layer of Highlights and Shadows (DEV-01, stage 6, step 2): a
//! guided filter of `t = log2(Y / 0.18)` guided by itself, computed once
//! per edit on the whole image at a fixed working size, and upsampled with
//! the same filter at the output: `B = Ā · t + b̄`, where `Ā` and `b̄` are
//! the filter's averaged coefficients, read bilinearly at the output
//! pixel's place in the working image. The definition `gpu::base` is
//! tested against.

use genzo_color::tone::MIDDLE_GRAY;

use super::sampling::Sampling;

/// The working size's long edge, in pixels.
pub const WORKING_EDGE: u32 = 1536;

/// The filter's radius as a fraction of the working image's long edge.
pub const RADIUS_FRACTION: f64 = 0.02;

/// The guided filter's ε, in stops²: edges of more than about a quarter
/// stop are kept (owner, 2026-10-19).
pub const EPSILON: f64 = 0.05;

/// The lowest `t` the filter sees, in stops: a pixel at or below
/// `0.18 × 2^−16` luminance (noise around black, negative values) counts
/// as that, so its noise adds no variance.
pub const T_FLOOR: f64 = -16.0;

/// The working size of an image shown at `shown` (full resolution): its
/// long edge 1536 px, or the image's own size when that is smaller.
pub fn working_size(shown: (u32, u32)) -> (u32, u32) {
    let long = shown.0.max(shown.1);
    if long <= WORKING_EDGE {
        return shown;
    }
    let scale = f64::from(WORKING_EDGE) / f64::from(long);
    let side = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
    (side(shown.0), side(shown.1))
}

/// The filter's radius at working size `size`: 2 % of the long edge,
/// rounded, at least 1 (31 px at 1536).
pub fn radius(size: (u32, u32)) -> u32 {
    ((f64::from(size.0.max(size.1)) * RADIUS_FRACTION).round() as u32).max(1)
}

/// `t` of luminance `y`, floored at [`T_FLOOR`].
pub fn floored_t(y: f64) -> f64 {
    (y.max(MIDDLE_GRAY * T_FLOOR.exp2()) / MIDDLE_GRAY).log2()
}

/// The mean of `values` (row by row, `size`) over a `2r + 1` square around
/// each pixel, the window cropped to the image and divided by the pixels
/// it holds. Separable: every row of a cropped window holds as many pixels.
pub fn box_mean(values: &[f64], (w, h): (u32, u32), r: u32) -> Vec<f64> {
    let (w, h, r) = (w as usize, h as usize, r as usize);
    let mut across = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
            let sum: f64 = values[y * w + lo..=y * w + hi].iter().sum();
            across[y * w + x] = sum / (hi - lo + 1) as f64;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            let sum: f64 = (lo..=hi).map(|k| across[k * w + x]).sum();
            out[y * w + x] = sum / (hi - lo + 1) as f64;
        }
    }
    out
}

/// The guided filter of `t` (row by row, `size`) guided by itself, with
/// radius `r` and `eps`: per pixel `(Ā, b̄)`, the box means of
/// `a = σ² / (σ² + ε)` and `b = μ (1 − a)`, where `μ` and `σ²` are `t`'s
/// box mean and variance. The base is then `Ā · t + b̄`.
pub fn coefficients(t: &[f64], size: (u32, u32), r: u32, eps: f64) -> Vec<[f64; 2]> {
    let mean = box_mean(t, size, r);
    let squares: Vec<f64> = t.iter().map(|v| v * v).collect();
    let mean_sq = box_mean(&squares, size, r);
    let (a, b): (Vec<f64>, Vec<f64>) = mean
        .iter()
        .zip(&mean_sq)
        .map(|(&m, &m2)| {
            let var = (m2 - m * m).max(0.0);
            let a = var / (var + eps);
            (a, m * (1.0 - a))
        })
        .unzip();
    box_mean(&a, size, r)
        .into_iter()
        .zip(box_mean(&b, size, r))
        .map(|(a, b)| [a, b])
        .collect()
}

/// The coefficients at working coordinates `at` (pixel centers at
/// `i + 0.5`), bilinearly from `plane` (row by row, `size`), edges clamped.
pub fn coefficients_at(plane: &[[f64; 2]], (w, h): (u32, u32), at: [f64; 2]) -> [f64; 2] {
    let (ux, uy) = (at[0] - 0.5, at[1] - 0.5);
    let (fx, fy) = (ux - ux.floor(), uy - uy.floor());
    let clamp = |v: f64, n: u32| (v as i64).clamp(0, i64::from(n) - 1) as usize;
    let (x0, y0) = (ux.floor(), uy.floor());
    let (xa, xb) = (clamp(x0, w), clamp(x0 + 1.0, w));
    let (ya, yb) = (clamp(y0, h), clamp(y0 + 1.0, h));
    let at = |x: usize, y: usize| plane[y * w as usize + x];
    let mix = |p: [f64; 2], q: [f64; 2], f: f64| [0, 1].map(|i| p[i] * (1.0 - f) + q[i] * f);
    mix(
        mix(at(xa, ya), at(xb, ya), fx),
        mix(at(xa, yb), at(xb, yb), fx),
        fy,
    )
}

/// The base `B` at 0 EV of a pixel whose `t` (at 0 EV) is `t` and whose
/// coefficients are `ab`. Exposure shifts it by the same EV.
pub fn base(ab: [f64; 2], t: f64) -> f64 {
    ab[0] * t.max(T_FLOOR) + ab[1]
}

/// The affine from a view's pixel centers, `(x + 0.5, y + 0.5, 1)`, to
/// working coordinates, given the samplings of the view and of the working
/// image. Both map into levels of the same image, so they meet in the
/// stored image's normalized coordinates.
pub fn view_to_working(view: &Sampling, working: &Sampling) -> [[f64; 3]; 2] {
    let dims = |s: &Sampling| [f64::from(s.level_size.0), f64::from(s.level_size.1)];
    let (dv, dw) = (dims(view), dims(working));
    // n = view.affine · p / dv; working level = dw · n = M w + c, where
    // M and c are the working affine's 2 × 2 part and offset.
    let to_level: [[f64; 3]; 2] = [0, 1].map(|i| view.affine[i].map(|v| v * dw[i] / dv[i]));
    let a = &working.affine;
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let inv = [
        [a[1][1] / det, -a[0][1] / det],
        [-a[1][0] / det, a[0][0] / det],
    ];
    [0, 1].map(|i| {
        let row = |k: usize| inv[i][0] * to_level[0][k] + inv[i][1] * to_level[1][k];
        [
            row(0),
            row(1),
            row(2) - inv[i][0] * a[0][2] - inv[i][1] * a[1][2],
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::sampling::{ViewSpec, plan, plan_view};

    #[test]
    fn the_working_size_keeps_the_aspect() {
        assert_eq!(working_size((8064, 6048)), (1536, 1152));
        assert_eq!(working_size((3000, 4000)), (1152, 1536));
        assert_eq!(working_size((1200, 800)), (1200, 800));
        assert_eq!(radius((1536, 1152)), 31);
        assert_eq!(radius((40, 10)), 1);
    }

    #[test]
    fn the_box_mean_crops_its_window() {
        // 1 2 3 across one row, radius 1: (1+2)/2, 2, (2+3)/2.
        assert_eq!(box_mean(&[1.0, 2.0, 3.0], (3, 1), 1), vec![1.5, 2.0, 2.5]);
        let flat = vec![0.7; 35];
        assert!(
            box_mean(&flat, (7, 5), 3)
                .iter()
                .all(|&v| (v - 0.7).abs() < 1e-15)
        );
    }

    #[test]
    fn flat_areas_smooth_and_edges_stay() {
        // A 3-stop step: far from it the base is the level; across it the
        // variance (about 2 stops²) dwarfs ε, so the edge is kept.
        let size = (60, 20);
        let t: Vec<f64> = (0..1200)
            .map(|i| if i % 60 < 30 { 0.0 } else { 3.0 })
            .collect();
        let c = coefficients(&t, size, 4, EPSILON);
        let b = |x: usize| base(c[10 * 60 + x], t[10 * 60 + x]);
        assert!(b(5).abs() < 1e-12 && (b(55) - 3.0).abs() < 1e-12);
        assert!(b(29) < 0.1 && b(30) > 2.9, "{} {}", b(29), b(30));
        // Small detail is smoothed: a 0.1-stop ripple mostly goes.
        let ripple: Vec<f64> = (0..1200).map(|i| 0.1 * f64::from((i % 2) as u8)).collect();
        let c = coefficients(&ripple, size, 4, EPSILON);
        assert!(c.iter().all(|ab| ab[0] < 0.06), "a stays small");
    }

    /// Exposure is a constant shift in `t`: the base of `t + c` is the base
    /// of `t` plus `c`, so exposure needs no new base (DEV-01).
    #[test]
    fn exposure_shifts_the_base_exactly() {
        let size = (40, 30);
        let t: Vec<f64> = (0..1200)
            .map(|i| ((i % 40) as f64 * 0.37).sin() * 2.0 + if i % 40 > 25 { 2.5 } else { -1.0 })
            .collect();
        let at = coefficients(&t, size, 3, EPSILON);
        for c in [-2.0, 0.7, 3.0] {
            let shifted: Vec<f64> = t.iter().map(|v| v + c).collect();
            let moved = coefficients(&shifted, size, 3, EPSILON);
            for i in 0..t.len() {
                let d = base(moved[i], shifted[i]) - base(at[i], t[i]);
                assert!((d - c).abs() < 1e-9, "pixel {i}: {d} for {c}");
            }
        }
    }

    #[test]
    fn bilinear_reads_centers_exactly() {
        let plane: Vec<[f64; 2]> = (0..6).map(|i| [f64::from(i), -f64::from(i)]).collect();
        assert_eq!(coefficients_at(&plane, (3, 2), [1.5, 0.5]), [1.0, -1.0]);
        assert_eq!(coefficients_at(&plane, (3, 2), [2.0, 1.0]), [3.0, -3.0]);
        // Clamped past the edges.
        assert_eq!(coefficients_at(&plane, (3, 2), [-4.0, 9.0]), [3.0, -3.0]);
    }

    #[test]
    fn views_map_onto_the_working_image() {
        let levels = [(2000, 1500), (1000, 750), (500, 375), (250, 187)];
        let level0 = (4000, 3000);
        for o in 1..=8 {
            let shown = crate::reference::sampling::shown(level0, o);
            let ws = working_size(shown);
            let working = plan(&levels, o, ws);
            // Fit at the working size is the identity.
            let fit = plan_view(level0, &levels, o, ViewSpec::Fit(ws));
            let m = view_to_working(&fit, &working);
            let p = [10.5, 20.5];
            let got = [0, 1].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2]);
            assert!((got[0] - p[0]).abs() < 1e-9 && (got[1] - p[1]).abs() < 1e-9);
            // 1:1 at an origin: shown pixel (origin + p) at the working scale.
            let one = plan_view(
                level0,
                &levels,
                o,
                ViewSpec::Region {
                    out: (300, 200),
                    zoom: 1.0,
                    origin: (1000.0, 600.0),
                },
            );
            let m = view_to_working(&one, &working);
            let got = [0, 1].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2]);
            let k = f64::from(ws.0) / f64::from(shown.0);
            let want = [(1000.0 + p[0]) * k, (600.0 + p[1]) * k];
            assert!(
                (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9,
                "orientation {o}: {got:?} vs {want:?}"
            );
        }
    }
}
