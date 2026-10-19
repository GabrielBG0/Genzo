// Stage 4 for one tile, reduced straight into the pyramid's first level
// (DEV-01): each white balanced, demosaiced camera pixel goes through the
// camera matrix into the working space, highlight desaturation and
// 2^BaselineExposure; each level 1 texel is the mean of its 2 x 2 block, an
// odd last row or column folded into the last texel, as reference::stages
// computes it. Level 0 is never stored.

struct Stage4 {
    // The camera to working space matrix, one row per vec4.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // The working space's luminance row.
    luma: vec4<f32>,
    // 2^BaselineExposure, the clip level, where desaturation starts (a
    // fraction of the clip level), 1 to desaturate highlights or 0 not to.
    params: vec4<f32>,
    // The first level 1 texel this tile writes, and how many across and down.
    texels: vec4<u32>,
    // The level 1 size.
    level: vec4<u32>,
}

@group(0) @binding(1) var<uniform> s4: Stage4;
@group(0) @binding(2) var rgb: texture_2d<f32>;
@group(0) @binding(3) var level1: texture_storage_2d<rgba16float, write>;

fn stage4(c: vec3<f32>) -> vec3<f32> {
    let w = vec3<f32>(dot(s4.m0.xyz, c), dot(s4.m1.xyz, c), dot(s4.m2.xyz, c));
    let clip = s4.params.y;
    let knee = s4.params.z;
    let top = max(c.x, max(c.y, c.z));
    let k = clamp((top - knee * clip) / ((1.0 - knee) * clip), 0.0, 1.0) * s4.params.w;
    let y = dot(s4.luma.xyz, w);
    return (w + (vec3<f32>(y) - w) * k) * s4.params.x;
}

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
    textureStore(level1, vec2<i32>(t), vec4<f32>(sum / n, 1.0));
}
