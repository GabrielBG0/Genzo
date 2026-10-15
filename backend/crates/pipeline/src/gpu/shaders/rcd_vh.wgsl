// Derived from darktable's data/kernels/demosaic_rcd.cl and
// src/iop/demosaicing/rcd.c:
//   Copyright (C) 2010-2026 darktable developers.
//   Ratio Corrected Demosaicing (RCD) by Luis Sanz Rodríguez, release 2.3,
//   https://github.com/LuisSR/RCD-Demosaicing, licensed under the GNU GPL v3;
//   tiling by Ingo Weyrich; optimisations by Hanno Schwalm.
// darktable is free software under the GNU General Public License, version 3
// or (at your option) any later version. Modified for Genzo (2026): ported to
// WGSL compute shaders with full-resolution, one-value-per-pixel buffers.
// Genzo is distributed under the GPL-3.0; see LICENSE.

// RCD step 1: vertical and horizontal directional discrimination.

@group(0) @binding(1) var cfa: texture_2d<f32>;
@group(0) @binding(2) var vh_out: texture_storage_2d<r32float, write>;

// The squared high-pass filter on color differences along direction `u`.
fn hpf(p: vec2<i32>, u: vec2<i32>) -> f32 {
    return sq(ld(cfa, p - 3 * u) - ld(cfa, p - u) - ld(cfa, p + u) + ld(cfa, p + 3 * u)
        - 3.0 * (ld(cfa, p - 2 * u) + ld(cfa, p + 2 * u)) + 6.0 * ld(cfa, p));
}

fn vdiff(p: vec2<i32>) -> f32 {
    return hpf(p, vec2<i32>(0, 1));
}

fn hdiff(p: vec2<i32>) -> f32 {
    return hpf(p, vec2<i32>(1, 0));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let v = max(EPSSQ, vdiff(p - vec2<i32>(0, 1)) + vdiff(p) + vdiff(p + vec2<i32>(0, 1)));
    let h = max(EPSSQ, hdiff(p - vec2<i32>(1, 0)) + hdiff(p) + hdiff(p + vec2<i32>(1, 0)));
    textureStore(vh_out, p, vec4<f32>(v / (v + h), 0.0, 0.0, 0.0));
}
