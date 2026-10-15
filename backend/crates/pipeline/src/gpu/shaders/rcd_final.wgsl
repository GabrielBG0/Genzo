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

// RCD step 4.3: red and blue at green positions, then assembly of the full
// RGB image. RCD is unstable near the edges, so a border of BORDER pixels uses
// a 3×3 same-color average instead (darktable's demosaic_box3).

@group(0) @binding(1) var cfa: texture_2d<f32>;
@group(0) @binding(2) var green: texture_2d<f32>;
@group(0) @binding(3) var rb: texture_2d<f32>;
@group(0) @binding(4) var vh: texture_2d<f32>;
@group(0) @binding(5) var rgb_out: texture_storage_2d<rgba16float, write>;

const BORDER: i32 = 9;

fn box3(p: vec2<i32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    var cnt = vec3<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let q = p + vec2(dx, dy);
            if (in_image(q)) {
                let m = onehot(fc(q));
                sum += m * ld(cfa, q);
                cnt += m;
            }
        }
    }
    return sum / max(cnt, vec3<f32>(1.0));
}

// Red (`blue` false) or blue (`blue` true) of `rb` at `q`.
fn rb_at(q: vec2<i32>, blue: bool) -> f32 {
    let v = ld2(rb, q);
    return select(v.x, v.y, blue);
}

// Red or blue at a green position.
fn at_green(p: vec2<i32>, blue: bool, g: f32, vh_disc: f32) -> f32 {
    let n = rb_at(p + vec2(0, -1), blue);
    let s = rb_at(p + vec2(0, 1), blue);
    let w = rb_at(p + vec2(-1, 0), blue);
    let e = rb_at(p + vec2(1, 0), blue);
    let sn_abs = abs(n - s);
    let ew_abs = abs(w - e);
    let n_grad = EPS + abs(g - ld(green, p + vec2(0, -2))) + sn_abs + abs(n - rb_at(p + vec2(0, -3), blue));
    let s_grad = EPS + abs(g - ld(green, p + vec2(0, 2))) + sn_abs + abs(s - rb_at(p + vec2(0, 3), blue));
    let w_grad = EPS + abs(g - ld(green, p + vec2(-2, 0))) + ew_abs + abs(w - rb_at(p + vec2(-3, 0), blue));
    let e_grad = EPS + abs(g - ld(green, p + vec2(2, 0))) + ew_abs + abs(e - rb_at(p + vec2(3, 0), blue));

    let n_est = n - ld(green, p + vec2(0, -1));
    let s_est = s - ld(green, p + vec2(0, 1));
    let w_est = w - ld(green, p + vec2(-1, 0));
    let e_est = e - ld(green, p + vec2(1, 0));

    let v_est = (n_grad * s_est + s_grad * n_est) / (n_grad + s_grad);
    let h_est = (e_grad * w_est + w_grad * e_est) / (e_grad + w_grad);
    return g + mix(v_est, h_est, clamp(vh_disc, 0.0, 1.0));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let size = full_size();
    var rgb: vec3<f32>;
    if (any(p < vec2<i32>(BORDER)) || any(p >= size - BORDER)) {
        rgb = box3(p);
    } else if (fc(p) == 1u) {
        let g = ld(green, p);
        let vh_c = ld(vh, p);
        let vh_n = 0.25 * (ld(vh, p + vec2(-1, -1)) + ld(vh, p + vec2(1, -1)) + ld(vh, p + vec2(-1, 1)) + ld(vh, p + vec2(1, 1)));
        let vh_disc = select(vh_c, vh_n, abs(0.5 - vh_c) < abs(0.5 - vh_n));
        rgb = vec3<f32>(at_green(p, false, g, vh_disc), g, at_green(p, true, g, vh_disc));
    } else {
        let v = ld2(rb, p);
        rgb = vec3<f32>(v.x, ld(green, p), v.y);
    }
    textureStore(rgb_out, p, vec4<f32>(max(rgb, vec3<f32>(0.0)), 1.0));
}
