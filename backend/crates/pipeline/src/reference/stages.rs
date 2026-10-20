//! CPU reference for stages 1 and 2 (DEV-01): the whole image at once, in the
//! order the GPU runs them per tile.

use super::opcodes::{apply_gain_map, fix_bad_pixels_constant, gain_map_sample};
use crate::raw::RawSource;
use genzo_model::frame::Rect;

/// Stage 1 over the active area: FixBadPixelsConstant (OpcodeList1) on the
/// stored values, the SDK's linearization, then the GainMaps (OpcodeList2).
/// Row by row, `samples` planes interleaved, white at 1. Negative values
/// are kept (DEV-01); the SDK clips them to 0. A non-RAW image's codes go
/// through its encoding's curves instead.
pub fn stage1(src: &RawSource) -> Vec<f32> {
    let (sw, sh) = src.stored;
    let mut stored = src.data.clone();
    for op in &src.fix_bad_pixels {
        fix_bad_pixels_constant(&mut stored, sw, sh, op);
    }
    let (w, h) = src.size();
    let n = src.samples as usize;
    let lin = &src.linearization;
    let mut out = Vec::with_capacity(w as usize * h as usize * n);
    for row in 0..h {
        let y = (row + src.active.top) as usize;
        for col in 0..w {
            let x = (col + src.active.left) as usize;
            for p in 0..n {
                let code = stored[(y * sw as usize + x) * n + p];
                out.push(match &src.encoding {
                    Some(e) => e.curves[p].code(code),
                    None => f32::from(code) * lin.scale[p] - lin.black(row, col, p),
                });
            }
        }
    }
    for gm in &src.gain_maps {
        apply_gain_map(&mut out, w, h, src.samples, gm);
    }
    out
}

/// [`stage1`] for the active-area pixels in `rect` only, equal to the
/// whole image's values there, without the cost of the whole image: what the
/// white balance picker reads (DEV-02).
pub fn stage1_window(src: &RawSource, rect: Rect) -> Vec<f32> {
    let (sw, sh) = src.stored;
    let n = src.samples as usize;
    let (w, h) = src.size();
    // FixBadPixelsConstant runs on a copy of the stored window. Each op reads
    // neighbors up to 2 px away, so a margin of 2 px per op keeps the
    // window's own pixels exact; an even origin keeps the Bayer phase's
    // parity. The whole-image path treats the stored values as one plane,
    // and so does this.
    let margin = 2 * src.fix_bad_pixels.len() as u32;
    let (left, top) = (src.active.left + rect.x, src.active.top + rect.y);
    let x0 = left.saturating_sub(margin) & !1;
    let y0 = top.saturating_sub(margin) & !1;
    let x1 = (left + rect.width + margin).min(sw);
    let y1 = (top + rect.height + margin).min(sh);
    let fixing = !src.fix_bad_pixels.is_empty();
    let (ww, wh) = if fixing { (x1 - x0, y1 - y0) } else { (0, 0) };
    let mut window = Vec::new();
    if fixing {
        assert_eq!(n, 1, "FixBadPixelsConstant is for one-plane images");
        for y in y0..y1 {
            let start = (y * sw + x0) as usize;
            window.extend_from_slice(&src.data[start..start + ww as usize]);
        }
        for op in &src.fix_bad_pixels {
            fix_bad_pixels_constant(&mut window, ww, wh, op);
        }
    }
    let lin = &src.linearization;
    let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize * n);
    for row in rect.y..rect.y + rect.height {
        let y = row + src.active.top;
        for col in rect.x..rect.x + rect.width {
            let x = col + src.active.left;
            for p in 0..n {
                let code = if fixing {
                    window[((y - y0) * ww + (x - x0)) as usize]
                } else {
                    src.data[(y as usize * sw as usize + x as usize) * n + p]
                };
                let mut v = match &src.encoding {
                    Some(e) => e.curves[p].code(code),
                    None => f32::from(code) * lin.scale[p] - lin.black(row, col, p),
                };
                for gm in &src.gain_maps {
                    v = gain_map_sample(v, gm, (row, col, p as u32), (w, h, src.samples));
                }
                out.push(v);
            }
        }
    }
    out
}

/// Stage 2: each value times its channel's multiplier, then clipped where
/// the first channel saturates, `min(multipliers)` (DEV-01).
pub fn stage2(image: &mut [f32], src: &RawSource, multipliers: [f32; 3]) {
    let clip = multipliers[0].min(multipliers[1]).min(multipliers[2]);
    let (w, _) = src.size();
    for (i, v) in image.iter_mut().enumerate() {
        let gain = match src.cfa {
            Some(_) => {
                let (row, col) = ((i as u32) / w, (i as u32) % w);
                multipliers[usize::from(src.color_at(row, col).expect("a Bayer image"))]
            }
            None if src.samples == 3 => multipliers[i % 3],
            // A monochrome plane has no white balance.
            None => 1.0,
        };
        *v = (*v * gain).min(if src.samples == 1 && src.cfa.is_none() {
            1.0
        } else {
            clip
        });
    }
}

/// Where highlight desaturation starts: the largest white balanced camera
/// channel at this fraction of the clip level (DEV-01, stage 4).
pub const DESATURATE_FROM: f32 = 0.95;

/// The working space's luminance: the Y row of `WORKING_TO_XYZ`.
pub fn luminance_row() -> [f32; 3] {
    genzo_color::WORKING_TO_XYZ[1].map(|v| v as f32)
}

/// Stage 4 for one white balanced, demosaiced camera pixel: the camera
/// matrix into the working space; then highlight desaturation, a blend
/// toward the pixel's own luminance whose weight rises linearly from 0, where
/// the largest camera channel is at [`DESATURATE_FROM`] of the clip level,
/// to 1 at the clip level (when the transform has it); then
/// 2^BaselineExposure.
pub fn stage4(rgb: [f32; 3], t: &crate::camera_color::CameraTransform) -> [f32; 3] {
    let m = &t.matrix;
    let w = [0, 1, 2].map(|r| m[r][0] * rgb[0] + m[r][1] * rgb[1] + m[r][2] * rgb[2]);
    let clip = t.multipliers[0].min(t.multipliers[1]).min(t.multipliers[2]);
    let top = rgb[0].max(rgb[1]).max(rgb[2]);
    let k = if t.highlight_desaturation {
        ((top - DESATURATE_FROM * clip) / ((1.0 - DESATURATE_FROM) * clip)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let l = luminance_row();
    let y = l[0] * w[0] + l[1] * w[1] + l[2] * w[2];
    w.map(|c| (c + (y - c) * k) * t.exposure)
}

/// The size of the pyramid level below `size`: half, rounded down, at least
/// 1, as a texture's mip chain sizes it.
pub fn half(size: (u32, u32)) -> (u32, u32) {
    ((size.0 / 2).max(1), (size.1 / 2).max(1))
}

/// The source columns (or rows) texel `i` of a level of `n` texels averages,
/// from a level of `src` texels: a pair, and the last texel also takes an
/// odd leftover, so every source texel counts once.
pub fn span(i: u32, n: u32, src: u32) -> std::ops::Range<u32> {
    let start = (2 * i).min(src - 1);
    let end = if i + 1 == n { src } else { 2 * i + 2 };
    start..end
}

/// One pyramid level from the one above it (`width` × `height`, `channels`
/// interleaved): the mean of each texel's [`span`]s.
pub fn reduce(
    image: &[f32],
    (width, height): (u32, u32),
    channels: usize,
) -> (Vec<f32>, (u32, u32)) {
    let (w, h) = half((width, height));
    let mut out = Vec::with_capacity((w * h) as usize * channels);
    for j in 0..h {
        for i in 0..w {
            let (xs, ys) = (span(i, w, width), span(j, h, height));
            let n = (xs.len() * ys.len()) as f32;
            for c in 0..channels {
                let mut sum = 0.0f32;
                for y in ys.clone() {
                    for x in xs.clone() {
                        sum += image[(y * width + x) as usize * channels + c];
                    }
                }
                out.push(sum / n);
            }
        }
    }
    (out, (w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_color::CameraTransform;

    fn transform() -> CameraTransform {
        CameraTransform {
            white: genzo_color::camera::D65_XY,
            multipliers: [2.0, 1.0, 1.5],
            // A saturating matrix, so desaturation shows.
            matrix: [[1.4, -0.3, -0.1], [-0.2, 1.3, -0.1], [0.0, -0.4, 1.4]],
            exposure: 2.0,
            fallback: None,
            highlight_desaturation: true,
        }
    }

    #[test]
    fn desaturation_blends_only_the_top_five_percent() {
        let t = transform();
        let plain = |rgb: [f32; 3]| {
            let m = &t.matrix;
            [0, 1, 2].map(|r| (m[r][0] * rgb[0] + m[r][1] * rgb[1] + m[r][2] * rgb[2]) * t.exposure)
        };
        // Below 0.95 of the clip (1): the matrix only.
        let low = [0.3, 0.94, 0.5];
        assert_eq!(stage4(low, &t), plain(low));
        // At the clip: exactly neutral.
        let out = stage4([0.3, 1.0, 0.5], &t);
        assert!(
            (out[0] - out[1]).abs() < 1e-6 && (out[1] - out[2]).abs() < 1e-6,
            "{out:?}"
        );
        // In between, the spread shrinks as the top channel rises.
        let spread = |v: [f32; 3]| v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
        let mut last = f32::INFINITY;
        for g in [0.95f32, 0.96, 0.97, 0.98, 0.99, 1.0] {
            let s = spread(stage4([0.3, g, 0.5], &t));
            assert!(s < last, "{g}: {s} after {last}");
            last = s;
        }
    }

    #[test]
    fn reduce_folds_odd_edges_and_keeps_the_mean() {
        assert_eq!(half((5, 1)), (2, 1));
        assert_eq!((span(0, 2, 5), span(1, 2, 5)), (0..2, 2..5));
        assert_eq!(span(0, 1, 1), 0..1);
        // 5 x 3, one channel: values 0..15.
        let img: Vec<f32> = (0..15).map(|v| v as f32).collect();
        let (out, size) = reduce(&img, (5, 3), 1);
        assert_eq!(size, (2, 1));
        // Texel 0: columns 0-1, rows 0-2 (rows fold: one texel takes all 3).
        assert_eq!(out[0], (0.0 + 1.0 + 5.0 + 6.0 + 10.0 + 11.0) / 6.0);
        assert_eq!(
            out[1],
            (2.0 + 3.0 + 4.0 + 7.0 + 8.0 + 9.0 + 12.0 + 13.0 + 14.0) / 9.0
        );
        // Every source texel counts once, so the mean weighted by area holds.
        let mean = (out[0] * 6.0 + out[1] * 9.0) / 15.0;
        assert!((mean - 7.0).abs() < 1e-6);
    }
}
