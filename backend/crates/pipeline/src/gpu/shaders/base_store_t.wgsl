// The base layer's working image (DEV-01, stage 6, step 2), for
// view_sample.wgsl: t = log2(Y / 0.18), floored, as f32 bits, row by row
// (reference::base::floored_t).

struct Working {
    // AP1's Y row.
    y_row: vec4<f32>,
    // The luminance of the floor (0.18 · 2^T_FLOOR), unused.
    floor: vec4<f32>,
}

@group(0) @binding(2) var<storage, read_write> plane: array<u32>;
@group(0) @binding(3) var<uniform> working: Working;

fn store(p: vec2<u32>, rgb: vec3<f32>) {
    let y = max(dot(working.y_row.xyz, rgb), working.floor.x);
    plane[p.y * view.sizes.z + p.x] = bitcast<u32>(log2(y / 0.18));
}
