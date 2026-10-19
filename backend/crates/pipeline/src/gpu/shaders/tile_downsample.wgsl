// One pyramid level from the one above it: the mean of each texel's 2 x 2
// block, an odd last row or column folded into the last texel
// (reference::stages::reduce).

struct Sizes {
    // The source level's width and height, then this level's.
    sizes: vec4<u32>,
}

@group(0) @binding(0) var<uniform> sz: Sizes;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;

fn span(i: u32, n: u32, len: u32) -> vec2<u32> {
    let start = min(2u * i, len - 1u);
    let end = select(2u * i + 2u, len, i + 1u == n);
    return vec2<u32>(start, end);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= sz.sizes.z || id.y >= sz.sizes.w) {
        return;
    }
    let xs = span(id.x, sz.sizes.z, sz.sizes.x);
    let ys = span(id.y, sz.sizes.w, sz.sizes.y);
    var sum = vec3<f32>(0.0);
    for (var y = ys.x; y < ys.y; y++) {
        for (var x = xs.x; x < xs.y; x++) {
            sum += textureLoad(src, vec2<i32>(i32(x), i32(y)), 0).rgb;
        }
    }
    let n = f32((xs.y - xs.x) * (ys.y - ys.x));
    textureStore(dst, vec2<i32>(id.xy), vec4<f32>(sum / n, 1.0));
}
