// RCD steps 2 and 4.0: the low-pass filter, and the squared high-pass filter
// along the P (\) and Q (/) diagonals.

@group(0) @binding(1) var cfa: texture_2d<f32>;
@group(0) @binding(2) var lpf_out: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var pq_out: texture_storage_2d<rg32float, write>;

fn diag_hpf(p: vec2<i32>, u: vec2<i32>) -> f32 {
    return sq((ld(cfa, p - 3 * u) - ld(cfa, p - u) - ld(cfa, p + u) + ld(cfa, p + 3 * u))
        - 3.0 * (ld(cfa, p - 2 * u) + ld(cfa, p + 2 * u)) + 6.0 * ld(cfa, p));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let lpf = ld(cfa, p)
        + 0.5 * (ld(cfa, p + vec2(0, -1)) + ld(cfa, p + vec2(0, 1)) + ld(cfa, p + vec2(-1, 0)) + ld(cfa, p + vec2(1, 0)))
        + 0.25 * (ld(cfa, p + vec2(-1, -1)) + ld(cfa, p + vec2(1, -1)) + ld(cfa, p + vec2(-1, 1)) + ld(cfa, p + vec2(1, 1)));
    textureStore(lpf_out, p, vec4<f32>(lpf, 0.0, 0.0, 0.0));
    let pd = diag_hpf(p, vec2<i32>(1, 1));
    let qd = diag_hpf(p, vec2<i32>(-1, 1));
    textureStore(pq_out, p, vec4<f32>(pd, qd, 0.0, 0.0));
}
