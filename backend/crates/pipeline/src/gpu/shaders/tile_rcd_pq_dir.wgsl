// Derived from darktable's data/kernels/demosaic_rcd.cl and
// src/iop/demosaicing/rcd.c:
//   Copyright (C) 2010-2026 darktable developers.
//   Ratio Corrected Demosaicing (RCD) by Luis Sanz Rodríguez, release 2.3,
//   https://github.com/LuisSR/RCD-Demosaicing, licensed under the GNU GPL v3;
//   tiling by Ingo Weyrich; optimisations by Hanno Schwalm.
// darktable is free software under the GNU General Public License, version 3
// or (at your option) any later version. Modified for Genzo (2026): ported to
// WGSL compute shaders with one-value-per-pixel buffers, run per tile.
// Genzo is distributed under the GPL-3.0; see LICENSE.

// RCD step 4.1: P/Q diagonal discrimination strength.

@group(0) @binding(1) var pq: texture_2d<f32>;
@group(0) @binding(2) var pq_dir_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_tile(p)) {
        return;
    }
    let ps = max(EPSSQ, ld2(pq, p + vec2(-1, -1)).x + ld2(pq, p).x + ld2(pq, p + vec2(1, 1)).x);
    let qs = max(EPSSQ, ld2(pq, p + vec2(1, -1)).y + ld2(pq, p).y + ld2(pq, p + vec2(-1, 1)).y);
    textureStore(pq_dir_out, p, vec4<f32>(ps / (ps + qs), 0.0, 0.0, 0.0));
}
