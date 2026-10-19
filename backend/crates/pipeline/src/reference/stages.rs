//! CPU reference for stages 1 and 2 (DEV-01): the whole image at once, in the
//! order the GPU runs them per tile.

use super::opcodes::{apply_gain_map, fix_bad_pixels_constant};
use crate::raw::RawSource;

/// Stage 1 over the active area: FixBadPixelsConstant (OpcodeList1) on the
/// stored values, the SDK's linearization, then the GainMaps (OpcodeList2).
/// Row by row, `samples` planes interleaved, white at 1. Negative values
/// are kept (DEV-01); the SDK clips them to 0.
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
                let v = f32::from(stored[(y * sw as usize + x) * n + p]);
                out.push(v * lin.scale[p] - lin.black(row, col, p));
            }
        }
    }
    for gm in &src.gain_maps {
        apply_gain_map(&mut out, w, h, src.samples, gm);
    }
    out
}

/// Stage 2: each value times its channel's multiplier, then clipped where
/// the first channel saturates, `min(multipliers)` (DEV-01).
pub fn stage2(image: &mut [f32], src: &RawSource, multipliers: [f32; 3]) {
    let clip = multipliers[0].min(multipliers[1]).min(multipliers[2]);
    let (w, _) = src.size();
    for (i, v) in image.iter_mut().enumerate() {
        let gain = match src.bayer {
            Some(_) => {
                let (row, col) = ((i as u32) / w, (i as u32) % w);
                multipliers[usize::from(src.color_at(row, col).expect("a Bayer image"))]
            }
            None if src.samples == 3 => multipliers[i % 3],
            // A monochrome plane has no white balance.
            None => 1.0,
        };
        *v = (*v * gain).min(if src.samples == 1 && src.bayer.is_none() {
            1.0
        } else {
            clip
        });
    }
}
