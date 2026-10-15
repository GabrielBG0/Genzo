// RCD step 1: vertical and horizontal directional discrimination.
//
// Ported from darktable's demosaic_rcd.cl (GPL-3.0-or-later), after the RCD
// algorithm by Luis Sanz Rodríguez. Every RCD pass works on full-resolution
// textures, one value per pixel, instead of darktable's half-width buffers.

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
