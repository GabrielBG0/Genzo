// Stage 4 for one pixel (DEV-01), shared by the pyramid's reduction
// (tile_stage4.wgsl) and the level-0 tiles (tile_stage4_full.wgsl): the
// camera matrix into the working space, highlight desaturation, then
// 2^BaselineExposure, as reference::stages::stage4 computes it.

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
    // The pyramid: the first level 1 texel this tile writes, and how many
    // across and down. Level 0: the active-area pixels it writes, x, y,
    // width, height.
    texels: vec4<u32>,
    // The pyramid: the level 1 size. Level 0: cache tiles across the image,
    // atlas slots across.
    level: vec4<u32>,
}

@group(0) @binding(1) var<uniform> s4: Stage4;
@group(0) @binding(2) var rgb: texture_2d<f32>;

fn stage4(c: vec3<f32>) -> vec3<f32> {
    let w = vec3<f32>(dot(s4.m0.xyz, c), dot(s4.m1.xyz, c), dot(s4.m2.xyz, c));
    let clip = s4.params.y;
    let knee = s4.params.z;
    let top = max(c.x, max(c.y, c.z));
    let k = clamp((top - knee * clip) / ((1.0 - knee) * clip), 0.0, 1.0) * s4.params.w;
    let y = dot(s4.luma.xyz, w);
    return (w + (vec3<f32>(y) - w) * k) * s4.params.x;
}
