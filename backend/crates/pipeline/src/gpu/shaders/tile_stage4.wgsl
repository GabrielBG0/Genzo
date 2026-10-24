// Stage 4 for one tile, reduced straight into the pyramid's first level
// (DEV-01): each white balanced, demosaiced camera pixel goes through the
// camera matrix into the working space, highlight desaturation and
// 2^BaselineExposure; each level 1 texel is the mean of its 2 x 2 block, an
// odd last row or column folded into the last texel, as reference::stages
// computes it. Level 0 is never stored whole (tile_stage4_full.wgsl keeps
// tiles of it). A prefetched neighbor's tiles each go to a texture of their
// own, read back into CPU memory, and are stored from its top left.

@group(0) @binding(3) var level1: texture_storage_2d<rgba16float, write>;

// The source pixels texel `i` of `n` averages over `src`.
fn span(i: u32, n: u32, src: u32) -> vec2<u32> {
    let start = min(2u * i, src - 1u);
    let end = select(2u * i + 2u, src, i + 1u == n);
    return vec2<u32>(start, end);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= s4.texels.z || id.y >= s4.texels.w) {
        return;
    }
    let t = s4.texels.xy + id.xy;
    let xs = span(t.x, s4.level.x, tile.size.z);
    let ys = span(t.y, s4.level.y, tile.size.w);
    var sum = vec3<f32>(0.0);
    for (var y = ys.x; y < ys.y; y++) {
        for (var x = xs.x; x < xs.y; x++) {
            let p = vec2<i32>(i32(x), i32(y)) - vec2<i32>(tile.origin.xy);
            sum += stage4(textureLoad(rgb, p, 0).rgb);
        }
    }
    let n = f32((xs.y - xs.x) * (ys.y - ys.x));
    textureStore(level1, vec2<i32>(t) - vec2<i32>(s4.level.zw), vec4<f32>(sum / n, 1.0));
}
