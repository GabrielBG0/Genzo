// Sampling the pyramid into the view (DEV-01, Sampling), as
// reference::sampling computes it: each output pixel's center goes through
// one affine into the chosen level, then a Lanczos-2 filter sized to the
// remaining scale factor, separable along the level's axes, edges clamped.
// `fetch` reads a texel: a pyramid level (view_fetch_pyramid.wgsl) or level
// 0 through the tile cache (view_fetch_level0.wgsl).

struct View {
    // Level coordinates of an output pixel's center: dot(row, (x, y, 1)).
    across: vec4<f32>,
    down: vec4<f32>,
    // The filter's scale per level axis (level texels per output pixel, at
    // least 1).
    footprint: vec4<f32>,
    // The level's width and height, the output's width and height.
    sizes: vec4<u32>,
    // Level 0: cache tiles across the image, atlas slots across.
    grid: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(2) var sampled: texture_storage_2d<rgba16float, write>;

const PI: f32 = 3.14159265358979;

// Lanczos-2; an integer distance other than 0 weighs exactly 0.
fn lanczos2(t0: f32) -> f32 {
    let t = abs(t0);
    if (t == 0.0) {
        return 1.0;
    }
    if (t >= 2.0 || fract(t) == 0.0) {
        return 0.0;
    }
    let p = PI * t;
    return 2.0 * sin(p) * sin(p * 0.5) / (p * p);
}

// The first tap and the tap count along an axis, for center `c`.
fn span(c: f32, f: f32) -> vec2<i32> {
    let reach = 2.0 * f;
    let first = i32(floor(c - 0.5 - reach)) + 1;
    let last = i32(ceil(c - 0.5 + reach)) - 1;
    return vec2<i32>(first, last - first + 1);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= view.sizes.z || id.y >= view.sizes.w) {
        return;
    }
    let p = vec3<f32>(f32(id.x) + 0.5, f32(id.y) + 0.5, 1.0);
    let c = vec2<f32>(dot(view.across.xyz, p), dot(view.down.xyz, p));
    let f = view.footprint.xy;
    let sx = span(c.x, f.x);
    let sy = span(c.y, f.y);
    var wx = 0.0;
    for (var i = 0; i < sx.y; i++) {
        wx += lanczos2((f32(sx.x + i) + 0.5 - c.x) / f.x);
    }
    var wy = 0.0;
    for (var j = 0; j < sy.y; j++) {
        wy += lanczos2((f32(sy.x + j) + 0.5 - c.y) / f.y);
    }
    let edge = vec2<i32>(view.sizes.xy) - 1;
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < sy.y; j++) {
        let ky = lanczos2((f32(sy.x + j) + 0.5 - c.y) / f.y) / wy;
        let row = clamp(sy.x + j, 0, edge.y);
        for (var i = 0; i < sx.y; i++) {
            let kx = lanczos2((f32(sx.x + i) + 0.5 - c.x) / f.x) / wx;
            let col = clamp(sx.x + i, 0, edge.x);
            sum += ky * kx * fetch(vec2<i32>(col, row));
        }
    }
    textureStore(sampled, vec2<i32>(id.xy), vec4<f32>(sum, 1.0));
}
