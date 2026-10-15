// RCD step 4.1: P/Q diagonal discrimination strength.

@group(0) @binding(1) var pq: texture_2d<f32>;
@group(0) @binding(2) var pq_dir_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let ps = max(EPSSQ, ld2(pq, p + vec2(-1, -1)).x + ld2(pq, p).x + ld2(pq, p + vec2(1, 1)).x);
    let qs = max(EPSSQ, ld2(pq, p + vec2(1, -1)).y + ld2(pq, p).y + ld2(pq, p + vec2(-1, 1)).y);
    textureStore(pq_dir_out, p, vec4<f32>(ps / (ps + qs), 0.0, 0.0, 0.0));
}
