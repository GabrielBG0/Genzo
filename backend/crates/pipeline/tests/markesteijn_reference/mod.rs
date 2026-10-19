//! Derived from darktable's src/iop/demosaicing/xtrans.c and vng.c:
//!   Copyright (C) 2010-2026 darktable developers.
//!   xtrans_interpolate adapted from dcraw 9.20; Frank Markesteijn's
//!   algorithm for Fuji X-Trans sensors.
//! darktable is free software under the GNU General Public License, version 3
//! or (at your option) any later version. Modified for Genzo (2026): ported to
//! Rust as a test-only reference.
//! Genzo is distributed under the GPL-3.0; see LICENSE.
//!
//! A literal CPU port of darktable's `xtrans_markesteijn_interpolate`
//! (3 passes, darktable commit 8fb2851, 2026-10-18), its 122 px tiles
//! included, followed by the linear border fill of `_vng_lininterpolate`.
//! Test-only: it checks the WGSL port.

const TS: i64 = 122;
const PAD_G1_G3: i64 = 3;
const PAD_G_INTERP: i64 = 3;
const PAD_G_RECALC: i64 = 6;

fn sqrf(x: f32) -> f32 {
    x * x
}

fn fc(row: i64, col: i64, xtrans: &[[u8; 6]; 6]) -> usize {
    xtrans[((row + 600) % 6) as usize][((col + 600) % 6) as usize] as usize
}

/// The green hexagon around each pixel of a 3 x 3 cell, as tile offsets
/// (h + v * TS), and the solitary green's position.
pub fn hexagons(xtrans: &[[u8; 6]; 6]) -> ([[[i64; 8]; 3]; 3], (i64, i64)) {
    const ORTH: [i64; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];
    const PATT: [[i64; 16]; 2] = [
        [0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0],
        [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1],
    ];
    let mut allhex = [[[0i64; 8]; 3]; 3];
    let (mut sgrow, mut sgcol) = (0, 0);
    for row in 0..3i64 {
        for col in 0..3i64 {
            let mut ng = 0;
            let mut d = 0usize;
            while d < 10 {
                let g = usize::from(fc(row, col, xtrans) == 1);
                if fc(row + ORTH[d], col + ORTH[d + 2], xtrans) == 1 {
                    ng = 0;
                } else {
                    ng += 1;
                }
                if ng == 4 {
                    sgrow = row;
                    sgcol = col;
                }
                if ng == g + 1 {
                    for c in 0..8 {
                        let v = ORTH[d] * PATT[g][c * 2] + ORTH[d + 1] * PATT[g][c * 2 + 1];
                        let h = ORTH[d + 2] * PATT[g][c * 2] + ORTH[d + 3] * PATT[g][c * 2 + 1];
                        let k = c ^ (g * 2 & d);
                        allhex[row as usize][col as usize][k] = h + v * TS;
                    }
                }
                d += 2;
            }
        }
    }
    (allhex, (sgrow, sgcol))
}

fn hexmap(row: i64, col: i64, allhex: &[[[i64; 8]; 3]; 3]) -> &[i64; 8] {
    &allhex[((row + 600) % 3) as usize][((col + 600) % 3) as usize]
}

/// Demosaics `cfa` (`width` x `height`, the X-Trans pattern `xtrans` from its
/// first sample) into RGB. Pixels within 17 of an edge come from the linear
/// fill, as in darktable.
pub fn markesteijn(
    cfa: &[f32],
    width: usize,
    height: usize,
    xtrans: &[[u8; 6]; 6],
) -> Vec<[f32; 3]> {
    let (w, h) = (width as i64, height as i64);
    let input = |row: i64, col: i64| cfa[(row * w + col) as usize];
    let passes = 3;
    let ndir: usize = 8;
    let (allhex, (sgrow, sgcol)) = hexagons(xtrans);
    let pad_tile: i64 = 17;
    let mut out = vec![[0.0f32; 4]; width * height];

    let tts = (TS * TS) as usize;
    let mut buf = vec![0.0f32; ndir * tts * 3];
    let mut yuv = vec![0.0f32; 3 * tts];
    let mut drv = vec![0.0f32; ndir * tts];
    let mut gmin = vec![0.0f32; tts];
    let mut gmax = vec![0.0f32; tts];
    let mut homo = vec![0u8; ndir * tts];
    let mut homosum = vec![0u8; ndir * tts];

    let mut top = -pad_tile;
    while top < h - pad_tile {
        let mut left = -pad_tile;
        while left < w - pad_tile {
            let mut mrow = (top + TS).min(h + pad_tile);
            let mut mcol = (left + TS).min(w + pad_tile);
            // rgb index of (dir, tile row, tile col, channel), as a flat
            // offset like the C code's pointer arithmetic.
            let at = |d: i64, r: i64, c: i64| ((d * TS + r) * TS + c) * 3;

            for row in top..mrow {
                for col in left..mcol {
                    let p = at(0, row - top, col - left) as usize;
                    if col >= 0 && row >= 0 && col < w && row < h {
                        let f = fc(row, col, xtrans);
                        for c in 0..3 {
                            buf[p + c] = if c == f {
                                input(row, col).max(0.0)
                            } else {
                                0.0
                            };
                        }
                    } else {
                        let c = fc(row, col, xtrans);
                        let translate =
                            |n: i64, size: i64| if n >= size { 2 * size - n - 2 } else { n.abs() };
                        for cc in 0..3 {
                            if cc != c {
                                buf[p + cc] = 0.0;
                            } else {
                                let (cy, cx) = (translate(row, h), translate(col, w));
                                if c == fc(cy, cx, xtrans) {
                                    buf[p + c] = input(cy, cx).max(0.0);
                                } else {
                                    let mut sum = 0.0f32;
                                    let mut count = 0u8;
                                    for y in row - 1..=row + 1 {
                                        for x in col - 1..=col + 1 {
                                            let (yy, xx) = (translate(y, h), translate(x, w));
                                            if fc(yy, xx, xtrans) == c {
                                                sum += input(yy, xx).max(0.0);
                                                count += 1;
                                            }
                                        }
                                    }
                                    buf[p + c] = sum / f32::from(count);
                                }
                            }
                        }
                    }
                }
            }
            // duplicate rgb[0] to rgb[1..3]
            for d in 1..4 {
                let (a, b) = buf.split_at_mut(d * tts * 3);
                b[..tts * 3].copy_from_slice(&a[..tts * 3]);
            }

            // Green minimum and maximum of red/blue pairs.
            let g_of = |buf: &[f32], i: i64| buf[(i + 1) as usize];
            // (Both pixels of a pair share their hexagon of greens, so the
            // first one's min/max is the pair's: what darktable's OpenCL
            // kernel computes per pixel.)
            let mut row = top + PAD_G1_G3;
            while row < mrow - PAD_G1_G3 {
                let mut min = f32::MAX;
                let mut max = 0.0f32;
                let mut col = left + PAD_G1_G3;
                while col < mcol - PAD_G1_G3 {
                    if fc(row, col, xtrans) == 1 {
                        min = f32::MAX;
                        max = 0.0;
                        col += 1;
                        continue;
                    }
                    if max == 0.0 {
                        let base = at(0, row - top, col - left);
                        let hex = hexmap(row, col, &allhex);
                        for c in 0..6 {
                            let val = g_of(&buf, base + hex[c] * 3);
                            if min > val {
                                min = val;
                            }
                            if max < val {
                                max = val;
                            }
                        }
                    }
                    let gi = ((row - top) * TS + col - left) as usize;
                    gmin[gi] = min;
                    gmax[gi] = max;
                    // C's % keeps the dividend's sign.
                    match (row - sgrow) % 3 {
                        1 => {
                            if row < mrow - 4 {
                                row += 1;
                                col -= 1;
                            }
                        }
                        2 => {
                            min = f32::MAX;
                            max = 0.0;
                            col += 2;
                            if col < mcol - 4 && row > top + 3 {
                                row -= 1;
                            }
                        }
                        _ => {}
                    }
                    col += 1;
                }
                row += 1;
            }

            // Interpolate green horizontally, vertically, and along both diagonals.
            for row in top + PAD_G_INTERP..mrow - PAD_G_INTERP {
                for col in left + PAD_G_INTERP..mcol - PAD_G_INTERP {
                    let f = fc(row, col, xtrans);
                    if f == 1 {
                        continue;
                    }
                    let p = at(0, row - top, col - left);
                    let hex = hexmap(row, col, &allhex);
                    let px = |o: i64, ch: usize| buf[(p + o * 3) as usize + ch];
                    let mut color = [0.0f32; 4];
                    color[0] = 0.6796875 * (px(hex[1], 1) + px(hex[0], 1))
                        - 0.1796875 * (px(2 * hex[1], 1) + px(2 * hex[0], 1));
                    color[1] = 0.87109375 * px(hex[3], 1)
                        + px(hex[2], 1) * 0.13
                        + 0.359375 * (px(0, f) - px(-hex[2], f));
                    for c in 0..2 {
                        color[2 + c] = 0.640625 * px(hex[4 + c], 1)
                            + 0.359375 * px(-2 * hex[4 + c], 1)
                            + 0.12890625
                                * (2.0 * px(0, f) - px(3 * hex[4 + c], f) - px(-3 * hex[4 + c], f));
                    }
                    let gi = ((row - top) * TS + col - left) as usize;
                    let flip = usize::from((row - sgrow) % 3 == 0);
                    for (c, v) in color.iter().enumerate() {
                        let d = (c ^ flip) as i64;
                        buf[at(d, row - top, col - left) as usize + 1] =
                            v.max(gmin[gi]).min(gmax[gi]);
                    }
                }
            }

            let mut base_dir: i64 = 0; // rgb += 4 on the second pass
            for pass in 0..passes {
                if pass == 1 {
                    let (a, b) = buf.split_at_mut(4 * tts * 3);
                    b[..4 * tts * 3].copy_from_slice(&a[..4 * tts * 3]);
                    base_dir = 4;
                }
                let rgbat = |d: i64, r: i64, c: i64| at(base_dir + d, r, c);

                // Recalculate green from interpolated values of closer pixels.
                if pass > 0 {
                    for row in top + PAD_G_RECALC..mrow - PAD_G_RECALC {
                        for col in left + PAD_G_RECALC..mcol - PAD_G_RECALC {
                            let f = fc(row, col, xtrans);
                            if f == 1 {
                                continue;
                            }
                            let hex = hexmap(row, col, &allhex);
                            let gi = ((row - top) * TS + col - left) as usize;
                            for d in 3..6usize {
                                let id = ((d - 2) ^ usize::from((row - sgrow) % 3 == 0)) as i64;
                                let r = rgbat(id, row - top, col - left);
                                let v = |o: i64, ch: usize| buf[(r + o * 3) as usize + ch];
                                let val = v(-2 * hex[d], 1) + 2.0 * v(hex[d], 1)
                                    - v(-2 * hex[d], f)
                                    - 2.0 * v(hex[d], f)
                                    + 3.0 * v(0, f);
                                buf[r as usize + 1] = (val / 3.0).max(gmin[gi]).min(gmax[gi]);
                            }
                        }
                    }
                }

                // Interpolate red and blue values for solitary green pixels.
                let pad_rb_g: i64 = 5;
                // C's integer division truncates toward zero, as Rust's does.
                let mut row = (top - sgrow + pad_rb_g + 2) / 3 * 3 + sgrow;
                while row < mrow - pad_rb_g {
                    let mut col = (left - sgcol + pad_rb_g + 2) / 3 * 3 + sgcol;
                    while col < mcol - pad_rb_g {
                        let mut rfx = rgbat(0, row - top, col - left);
                        let mut hh = fc(row, col + 1, xtrans);
                        let mut diff = [0.0f32; 6];
                        let mut color = [[0.0f32; 6]; 2];
                        let mut i: i64 = 1;
                        for d in 0..6usize {
                            for c in 0..2 {
                                let off = (i << c) * 3;
                                let v = |o: i64, ch: usize| buf[(rfx + o) as usize + ch];
                                let g = 2.0 * v(0, 1) - v(off, 1) - v(-off, 1);
                                color[usize::from(hh != 0)][d] = g + v(off, hh) + v(-off, hh);
                                if d > 1 {
                                    diff[d] +=
                                        sqrf(v(off, 1) - v(-off, 1) - v(off, hh) + v(-off, hh))
                                            + sqrf(g);
                                }
                                hh ^= 2;
                            }
                            if d < 2 || d & 1 == 1 {
                                let d_out = d - usize::from(d > 1 && diff[d - 1] < diff[d]);
                                buf[rfx as usize] = color[0][d_out] / 2.0;
                                buf[rfx as usize + 2] = color[1][d_out] / 2.0;
                                rfx += TS * TS * 3;
                            }
                            i ^= TS ^ 1;
                            hh ^= 2;
                        }
                        col += 3;
                    }
                    row += 3;
                }

                // Interpolate red for blue pixels and vice versa.
                let pad_rb_br: i64 = 5;
                for row in top + pad_rb_br..mrow - pad_rb_br {
                    for col in left + pad_rb_br..mcol - pad_rb_br {
                        let f = 2 - fc(row, col, xtrans);
                        if f == 1 {
                            continue;
                        }
                        let mut rfx = rgbat(0, row - top, col - left);
                        let c: i64 = if (row - sgrow) % 3 != 0 { TS } else { 1 };
                        let hstep: i64 = 3 * (c ^ TS ^ 1);
                        for d in 0..4i64 {
                            let v = |o: i64, ch: usize| buf[(rfx + o * 3) as usize + ch];
                            let i = if d > 1
                                || ((d ^ c) & 1) != 0
                                || ((v(0, 1) - v(c, 1)).abs() + (v(0, 1) - v(-c, 1)).abs())
                                    < 2.0
                                        * ((v(0, 1) - v(hstep, 1)).abs()
                                            + (v(0, 1) - v(-hstep, 1)).abs())
                            {
                                c
                            } else {
                                hstep
                            };
                            let val =
                                (v(i, f) + v(-i, f) + 2.0 * v(0, 1) - v(i, 1) - v(-i, 1)) / 2.0;
                            buf[rfx as usize + f] = val;
                            rfx += TS * TS * 3;
                        }
                    }
                }

                // Fill in red and blue for 2x2 blocks of green.
                let pad_g22: i64 = 4;
                for row in top + pad_g22..mrow - pad_g22 {
                    if (row - sgrow) % 3 == 0 {
                        continue;
                    }
                    for col in left + pad_g22..mcol - pad_g22 {
                        if (col - sgcol) % 3 == 0 {
                            continue;
                        }
                        let mut rfx = rgbat(0, row - top, col - left);
                        let hex = hexmap(row, col, &allhex);
                        let mut d = 0;
                        while d < ndir {
                            let v = |o: i64, ch: usize| buf[(rfx + o * 3) as usize + ch];
                            // Both channels read only neighbors, so they are
                            // computed first and written after.
                            let vals = if hex[d] + hex[d + 1] != 0 {
                                let g = 3.0 * v(0, 1) - 2.0 * v(hex[d], 1) - v(hex[d + 1], 1);
                                [0usize, 2]
                                    .map(|c| (g + 2.0 * v(hex[d], c) + v(hex[d + 1], c)) / 3.0)
                            } else {
                                let g = 2.0 * v(0, 1) - v(hex[d], 1) - v(hex[d + 1], 1);
                                [0usize, 2].map(|c| (g + v(hex[d], c) + v(hex[d + 1], c)) / 2.0)
                            };
                            buf[rfx as usize] = vals[0];
                            buf[rfx as usize + 2] = vals[1];
                            d += 2;
                            rfx += TS * TS * 3;
                        }
                    }
                }
            }

            mrow -= top;
            mcol -= left;

            // Convert to YPbPr and differentiate in all directions.
            const DIR: [i64; 4] = [1, TS, TS + 1, TS - 1];
            for d in 0..ndir {
                let pad_yuv: i64 = 13;
                for row in pad_yuv..mrow - pad_yuv {
                    for col in pad_yuv..mcol - pad_yuv {
                        let r = at(d as i64, row, col) as usize;
                        let rx = &buf[r..r + 3];
                        let y = 0.2627 * rx[0] + 0.6780 * rx[1] + 0.0593 * rx[2];
                        let i = (row * TS + col) as usize;
                        yuv[i] = y;
                        yuv[tts + i] = (rx[2] - y) * 0.56433;
                        yuv[2 * tts + i] = (rx[0] - y) * 0.67815;
                    }
                }
                let f = DIR[d & 3];
                let pad_drv: i64 = 14;
                for row in pad_drv..mrow - pad_drv {
                    for col in pad_drv..mcol - pad_drv {
                        let i = row * TS + col;
                        let y = |ch: usize, o: i64| yuv[ch * tts + (i + o) as usize];
                        drv[d * tts + i as usize] = sqrf(2.0 * y(0, 0) - y(0, f) - y(0, -f))
                            + sqrf(2.0 * y(1, 0) - y(1, f) - y(1, -f))
                            + sqrf(2.0 * y(2, 0) - y(2, f) - y(2, -f));
                    }
                }
            }

            // Homogeneity maps from the derivatives.
            homo.iter_mut().for_each(|v| *v = 0);
            let pad_homo: i64 = 15;
            for row in pad_homo..mrow - pad_homo {
                for col in pad_homo..mcol - pad_homo {
                    let i = (row * TS + col) as usize;
                    let mut tr = f32::MAX;
                    for d in 0..ndir {
                        if tr > drv[d * tts + i] {
                            tr = drv[d * tts + i];
                        }
                    }
                    tr *= 8.0;
                    for d in 0..ndir {
                        for v in -1..=1i64 {
                            for hh in -1..=1i64 {
                                let j = ((row + v) * TS + col + hh) as usize;
                                homo[d * tts + i] += u8::from(drv[d * tts + j] <= tr);
                            }
                        }
                    }
                }
            }

            // 5 x 5 sums of the homogeneity maps.
            for d in 0..ndir {
                for row in pad_tile..mrow - pad_tile {
                    let mut col = pad_tile - 5;
                    let mut v5sum = [0u8; 5];
                    homosum[d * tts + (row * TS + col) as usize] = 0;
                    col += 1;
                    while col < mcol - pad_tile {
                        let mut colsum = 0u8;
                        for v in -2..=2i64 {
                            colsum += homo[d * tts + ((row + v) * TS + col + 2) as usize];
                        }
                        let i = d * tts + (row * TS + col) as usize;
                        // uint8_t arithmetic, as in C: the difference can wrap.
                        homosum[i] = homosum[i - 1]
                            .wrapping_sub(v5sum[(col % 5) as usize])
                            .wrapping_add(colsum);
                        v5sum[(col % 5) as usize] = colsum;
                        col += 1;
                    }
                }
            }

            // Average the most homogeneous pixels.
            for row in pad_tile..mrow - pad_tile {
                for col in pad_tile..mcol - pad_tile {
                    let i = (row * TS + col) as usize;
                    let mut hm = [0u8; 8];
                    let mut maxval = 0u8;
                    for d in 0..ndir {
                        hm[d] = homosum[d * tts + i];
                        maxval = maxval.max(hm[d]);
                    }
                    maxval -= maxval >> 3;
                    for d in 0..ndir - 4 {
                        if hm[d] < hm[d + 4] {
                            hm[d] = 0;
                        } else if hm[d] > hm[d + 4] {
                            hm[d + 4] = 0;
                        }
                    }
                    let mut avg = [0.0f32; 4];
                    for d in 0..ndir {
                        if hm[d] >= maxval {
                            let r = at(d as i64, row, col) as usize;
                            for c in 0..3 {
                                avg[c] += buf[r + c];
                            }
                            avg[3] += 1.0;
                        }
                    }
                    let o = ((row + top) * w + col + left) as usize;
                    for c in 0..3 {
                        out[o][c] = (avg[c] / avg[3]).max(0.0);
                    }
                }
            }
            left += TS - pad_tile * 2;
        }
        top += TS - pad_tile * 2;
    }
    lininterpolate(&mut out, cfa, width, height, xtrans, pad_tile as usize);
    out.into_iter().map(|p| [p[0], p[1], p[2]]).collect()
}

/// A plain linear demosaic of the whole image (`_vng_lininterpolate` with no
/// border left to Markesteijn): the tests' negative control.
pub fn linear(cfa: &[f32], width: usize, height: usize, xtrans: &[[u8; 6]; 6]) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0f32; 4]; width * height];
    lininterpolate(&mut out, cfa, width, height, xtrans, width.max(height));
    out.into_iter().map(|p| [p[0], p[1], p[2]]).collect()
}

/// `_vng_lininterpolate`'s border fill for X-Trans, `border` px deep: the
/// outer 1 px frame as an unweighted 3 x 3 same-color mean, then the rest
/// with weights 2 across and 1 diagonally.
fn lininterpolate(
    out: &mut [[f32; 4]],
    cfa: &[f32],
    width: usize,
    height: usize,
    xtrans: &[[u8; 6]; 6],
    border: usize,
) {
    let (w, h) = (width as i64, height as i64);
    let input = |y: i64, x: i64| cfa[(y * w + x) as usize];
    // The 1 px frame.
    for row in 0..h {
        let mut col = 0;
        while col < w {
            if col == 1 && row >= 1 && row < h - 1 {
                col = w - 1;
            }
            let mut sum = [0.0f32; 4];
            let mut count = [0u8; 4];
            for y in row - 1..row + 2 {
                for x in col - 1..col + 2 {
                    if y >= 0 && x >= 0 && y < h && x < w {
                        let f = fc(y, x, xtrans);
                        sum[f] += input(y, x).max(0.0);
                        count[f] += 1;
                    }
                }
            }
            let f = fc(row, col, xtrans);
            let o = (row * w + col) as usize;
            for c in 0..3 {
                out[o][c] = if c != f && count[c] != 0 {
                    sum[c] / f32::from(count[c])
                } else {
                    input(row, col).max(0.0)
                };
            }
            col += 1;
        }
    }
    // The weighted lookup, per pattern site.
    #[derive(Clone)]
    struct Site {
        neighbors: Vec<(i64, i64, f32, usize)>,
        totals: Vec<(usize, f32)>,
        own: usize,
    }
    let mut sites = Vec::with_capacity(36);
    for row in 0..6i64 {
        for col in 0..6i64 {
            let f = fc(row, col, xtrans);
            let mut neighbors = Vec::new();
            let mut sum = [0i32; 4];
            for y in -1..=1i64 {
                for x in -1..=1i64 {
                    let weight = 1 << (i32::from(y == 0) + i32::from(x == 0));
                    let color = fc(row + y, col + x, xtrans);
                    if color == f {
                        continue;
                    }
                    neighbors.push((y, x, weight as f32, color));
                    sum[color] += weight;
                }
            }
            let totals = (0..3)
                .filter(|&c| c != f)
                .map(|c| (c, sum[c] as f32))
                .collect();
            sites.push(Site {
                neighbors,
                totals,
                own: f,
            });
        }
    }
    let border = border as i64;
    for row in 1..h - 1 {
        let mut col = 1;
        while col < w - 1 {
            if col == border && row >= border && row < h - border {
                col = w - border;
            }
            if col == w {
                break;
            }
            let site = &sites[((row % 6) * 6 + col % 6) as usize];
            let mut sum = [0.0f32; 4];
            for &(y, x, weight, color) in &site.neighbors {
                sum[color] += input(row + y, col + x).max(0.0) * weight;
            }
            let o = (row * w + col) as usize;
            for &(c, total) in &site.totals {
                out[o][c] = sum[c] / total;
            }
            out[o][site.own] = input(row, col);
            for c in 0..3 {
                out[o][c] = out[o][c].max(0.0);
            }
            col += 1;
        }
    }
}
