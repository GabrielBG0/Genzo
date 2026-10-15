//! Derived from darktable's data/kernels/demosaic_rcd.cl and
//! src/iop/demosaicing/rcd.c:
//!   Copyright (C) 2010-2026 darktable developers.
//!   Ratio Corrected Demosaicing (RCD) by Luis Sanz Rodríguez, release 2.3,
//!   https://github.com/LuisSR/RCD-Demosaicing, licensed under the GNU GPL v3;
//!   tiling by Ingo Weyrich; optimisations by Hanno Schwalm.
//! darktable is free software under the GNU General Public License, version 3
//! or (at your option) any later version. Modified for Genzo (2026): ported to
//! Rust as a test-only reference, the whole image as one tile.
//! Genzo is distributed under the GPL-3.0; see LICENSE.
//!
//! A literal CPU port of darktable's `rcd.c` (GPL-3.0-or-later), with the
//! whole image as one tile. Test-only: it checks the WGSL port against the
//! reference, including darktable's half-width P/Q buffers.

const EPS: f32 = 1e-5;
const EPSSQ: f32 = 1e-10;

fn sqrf(x: f32) -> f32 {
    x * x
}

fn interpolatef(a: f32, b: f32, c: f32) -> f32 {
    // darktable: a * (b - c) + c
    a * (b - c) + c
}

/// How the P/Q diagonal discrimination (steps 4.0 and 4.1) is computed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PqMode {
    /// darktable's half-width buffers: the high-pass is evaluated at odd
    /// columns only, so one of the three terms per pixel is taken one column
    /// to the side.
    HalfWidth,
    /// One value per pixel, at the pixel itself. What the WGSL port does.
    PerPixel,
}

/// Demosaics `cfa` (w × h, row-major) with the given 2×2 color pattern.
/// Returns linear RGB per pixel. Only rows and columns 9..len-9 are valid.
pub fn rcd(cfa: &[f32], w: usize, h: usize, pattern: [u8; 4], mode: PqMode) -> Vec<[f32; 3]> {
    assert!(w.is_multiple_of(2) && h.is_multiple_of(2));
    let fc = |row: usize, col: usize| pattern[(row & 1) * 2 + (col & 1)] as usize;
    let (w1, w2, w3, w4) = (w, 2 * w, 3 * w, 4 * w);
    let mut rgb = vec![vec![0f32; w * h]; 3];
    for row in 0..h {
        for col in 0..w {
            rgb[fc(row, col)][row * w + col] = cfa[row * w + col];
        }
    }

    // Step 1: vertical and horizontal discrimination.
    let mut vh = vec![0f32; w * h];
    let vd = |i: usize| {
        sqrf(
            (cfa[i - w3] - cfa[i - w1] - cfa[i + w1] + cfa[i + w3])
                - 3.0 * (cfa[i - w2] + cfa[i + w2])
                + 6.0 * cfa[i],
        )
    };
    let hd = |i: usize| {
        sqrf(
            (cfa[i - 3] - cfa[i - 1] - cfa[i + 1] + cfa[i + 3]) - 3.0 * (cfa[i - 2] + cfa[i + 2])
                + 6.0 * cfa[i],
        )
    };
    for row in 4..h - 4 {
        for col in 4..w - 4 {
            let i = row * w + col;
            let v = (vd(i - w1) + vd(i) + vd(i + w1)).max(EPSSQ);
            let hh = (hd(i - 1) + hd(i) + hd(i + 1)).max(EPSSQ);
            vh[i] = v / (v + hh);
        }
    }

    // Step 2: low-pass filter, half-width.
    let mut lpf = vec![0f32; w * h / 2];
    for row in 2..h - 2 {
        let mut col = 2 + (fc(row, 0) & 1);
        while col < w - 2 {
            let i = row * w + col;
            lpf[i / 2] = cfa[i]
                + 0.5 * (cfa[i - w1] + cfa[i + w1] + cfa[i - 1] + cfa[i + 1])
                + 0.25 * (cfa[i - w1 - 1] + cfa[i - w1 + 1] + cfa[i + w1 - 1] + cfa[i + w1 + 1]);
            col += 2;
        }
    }

    // Step 3: green at red and blue.
    for row in 4..h - 4 {
        let mut col = 4 + (fc(row, 0) & 1);
        while col < w - 4 {
            let i = row * w + col;
            let li = i / 2;
            let c = cfa[i];
            let n_grad = EPS
                + (cfa[i - w1] - cfa[i + w1]).abs()
                + (c - cfa[i - w2]).abs()
                + (cfa[i - w1] - cfa[i - w3]).abs()
                + (cfa[i - w2] - cfa[i - w4]).abs();
            let s_grad = EPS
                + (cfa[i + w1] - cfa[i - w1]).abs()
                + (c - cfa[i + w2]).abs()
                + (cfa[i + w1] - cfa[i + w3]).abs()
                + (cfa[i + w2] - cfa[i + w4]).abs();
            let w_grad = EPS
                + (cfa[i - 1] - cfa[i + 1]).abs()
                + (c - cfa[i - 2]).abs()
                + (cfa[i - 1] - cfa[i - 3]).abs()
                + (cfa[i - 2] - cfa[i - 4]).abs();
            let e_grad = EPS
                + (cfa[i + 1] - cfa[i - 1]).abs()
                + (c - cfa[i + 2]).abs()
                + (cfa[i + 1] - cfa[i + 3]).abs()
                + (cfa[i + 2] - cfa[i + 4]).abs();
            let l = lpf[li];
            let n_est = cfa[i - w1] * (l + l) / (EPS + l + lpf[li - w1]);
            let s_est = cfa[i + w1] * (l + l) / (EPS + l + lpf[li + w1]);
            let w_est = cfa[i - 1] * (l + l) / (EPS + l + lpf[li - 1]);
            let e_est = cfa[i + 1] * (l + l) / (EPS + l + lpf[li + 1]);
            let v_est = (s_grad * n_est + n_grad * s_est) / (n_grad + s_grad);
            let h_est = (w_grad * e_est + e_grad * w_est) / (e_grad + w_grad);
            let cv = vh[i];
            let nv = 0.25 * (vh[i - w1 - 1] + vh[i - w1 + 1] + vh[i + w1 - 1] + vh[i + w1 + 1]);
            let disc = if (0.5 - cv).abs() < (0.5 - nv).abs() {
                nv
            } else {
                cv
            };
            rgb[1][i] = interpolatef(disc.clamp(0.0, 1.0), h_est, v_est);
            col += 2;
        }
    }

    // Steps 4.0 and 4.1: P/Q diagonal high-pass and discrimination. `pq_at`
    // gives the discrimination at a red or blue pixel index.
    let p_hpf_at = |i: usize| {
        sqrf(
            (cfa[i - w3 - 3] - cfa[i - w1 - 1] - cfa[i + w1 + 1] + cfa[i + w3 + 3])
                - 3.0 * (cfa[i - w2 - 2] + cfa[i + w2 + 2])
                + 6.0 * cfa[i],
        )
    };
    let q_hpf_at = |i: usize| {
        sqrf(
            (cfa[i - w3 + 3] - cfa[i - w1 + 1] - cfa[i + w1 - 1] + cfa[i + w3 - 3])
                - 3.0 * (cfa[i - w2 + 2] + cfa[i + w2 - 2])
                + 6.0 * cfa[i],
        )
    };
    let mut pq = vec![0f32; w * h];
    match mode {
        PqMode::HalfWidth => {
            let mut p_hpf = vec![0f32; w * h / 2];
            let mut q_hpf = vec![0f32; w * h / 2];
            for row in 3..h - 3 {
                let mut col = 3;
                while col < w - 3 {
                    let i = row * w + col;
                    p_hpf[i / 2] = p_hpf_at(i);
                    q_hpf[i / 2] = q_hpf_at(i);
                    col += 2;
                }
            }
            for row in 4..h - 4 {
                let mut col = 4 + (fc(row, 0) & 1);
                while col < w - 4 {
                    let i = row * w + col;
                    let (i2, i3, i4) = (i / 2, (i - w1 - 1) / 2, (i + w1 - 1) / 2);
                    let ps = (p_hpf[i3] + p_hpf[i2] + p_hpf[i4 + 1]).max(EPSSQ);
                    let qs = (q_hpf[i3 + 1] + q_hpf[i2] + q_hpf[i4]).max(EPSSQ);
                    // Stored per pixel; darktable stores it at i / 2, and its
                    // neighbor reads at (i ± w ± 1) / 2 land on the same values.
                    pq[i] = ps / (ps + qs);
                    col += 2;
                }
            }
        }
        PqMode::PerPixel => {
            for row in 6..h - 6 {
                let mut col = 6 + (fc(row, 0) & 1);
                while col < w - 6 {
                    let i = row * w + col;
                    let ps = (p_hpf_at(i - w1 - 1) + p_hpf_at(i) + p_hpf_at(i + w1 + 1)).max(EPSSQ);
                    let qs = (q_hpf_at(i - w1 + 1) + q_hpf_at(i) + q_hpf_at(i + w1 - 1)).max(EPSSQ);
                    pq[i] = ps / (ps + qs);
                    col += 2;
                }
            }
        }
    }

    // Step 4.2: red at blue and blue at red.
    for row in 4..h - 4 {
        let mut col = 4 + (fc(row, 0) & 1);
        while col < w - 4 {
            let i = row * w + col;
            let c = 2 - fc(row, col);
            let cv = pq[i];
            let nv = 0.25 * (pq[i - w1 - 1] + pq[i - w1 + 1] + pq[i + w1 - 1] + pq[i + w1 + 1]);
            let disc = if (0.5 - cv).abs() < (0.5 - nv).abs() {
                nv
            } else {
                cv
            };
            let (rc, g) = (&rgb[c], &rgb[1]);
            let nw_grad = EPS
                + (rc[i - w1 - 1] - rc[i + w1 + 1]).abs()
                + (rc[i - w1 - 1] - rc[i - w3 - 3]).abs()
                + (g[i] - g[i - w2 - 2]).abs();
            let ne_grad = EPS
                + (rc[i - w1 + 1] - rc[i + w1 - 1]).abs()
                + (rc[i - w1 + 1] - rc[i - w3 + 3]).abs()
                + (g[i] - g[i - w2 + 2]).abs();
            let sw_grad = EPS
                + (rc[i - w1 + 1] - rc[i + w1 - 1]).abs()
                + (rc[i + w1 - 1] - rc[i + w3 - 3]).abs()
                + (g[i] - g[i + w2 - 2]).abs();
            let se_grad = EPS
                + (rc[i - w1 - 1] - rc[i + w1 + 1]).abs()
                + (rc[i + w1 + 1] - rc[i + w3 + 3]).abs()
                + (g[i] - g[i + w2 + 2]).abs();
            let nw_est = rc[i - w1 - 1] - g[i - w1 - 1];
            let ne_est = rc[i - w1 + 1] - g[i - w1 + 1];
            let sw_est = rc[i + w1 - 1] - g[i + w1 - 1];
            let se_est = rc[i + w1 + 1] - g[i + w1 + 1];
            let p_est = (nw_grad * se_est + se_grad * nw_est) / (nw_grad + se_grad);
            let q_est = (ne_grad * sw_est + sw_grad * ne_est) / (ne_grad + sw_grad);
            let v = g[i] + interpolatef(disc.clamp(0.0, 1.0), q_est, p_est);
            rgb[c][i] = v;
            col += 2;
        }
    }

    // Step 4.3: red and blue at green.
    for row in 4..h - 4 {
        let mut col = 4 + (fc(row, 1) & 1);
        while col < w - 4 {
            let i = row * w + col;
            let cv = vh[i];
            let nv = 0.25 * (vh[i - w1 - 1] + vh[i - w1 + 1] + vh[i + w1 - 1] + vh[i + w1 + 1]);
            let disc = if (0.5 - cv).abs() < (0.5 - nv).abs() {
                nv
            } else {
                cv
            };
            let g = rgb[1][i];
            let n1 = EPS + (g - rgb[1][i - w2]).abs();
            let s1 = EPS + (g - rgb[1][i + w2]).abs();
            let w1g = EPS + (g - rgb[1][i - 2]).abs();
            let e1 = EPS + (g - rgb[1][i + 2]).abs();
            let (gmw, gpw, gm1, gp1) =
                (rgb[1][i - w1], rgb[1][i + w1], rgb[1][i - 1], rgb[1][i + 1]);
            for c in [0, 2] {
                let rc = &rgb[c];
                let sn = (rc[i - w1] - rc[i + w1]).abs();
                let ew = (rc[i - 1] - rc[i + 1]).abs();
                let n_grad = n1 + sn + (rc[i - w1] - rc[i - w3]).abs();
                let s_grad = s1 + sn + (rc[i + w1] - rc[i + w3]).abs();
                let w_grad = w1g + ew + (rc[i - 1] - rc[i - 3]).abs();
                let e_grad = e1 + ew + (rc[i + 1] - rc[i + 3]).abs();
                let n_est = rc[i - w1] - gmw;
                let s_est = rc[i + w1] - gpw;
                let w_est = rc[i - 1] - gm1;
                let e_est = rc[i + 1] - gp1;
                let v_est = (n_grad * s_est + s_grad * n_est) / (n_grad + s_grad);
                let h_est = (e_grad * w_est + w_grad * e_est) / (e_grad + w_grad);
                let v = g + interpolatef(disc.clamp(0.0, 1.0), h_est, v_est);
                rgb[c][i] = v;
            }
            col += 2;
        }
    }

    (0..w * h)
        .map(|i| [rgb[0][i].max(0.0), rgb[1][i].max(0.0), rgb[2][i].max(0.0)])
        .collect()
}
