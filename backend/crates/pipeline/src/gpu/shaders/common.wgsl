// Shared by every spike pass. Prepended to each pass's source.

struct Params {
    // Full image width and height, viewport width and height.
    dims: vec4<u32>,
    // The CFA color (0 red, 1 green, 2 blue) at (0,0), (1,0), (0,1), (1,1).
    cfa: vec4<u32>,
    // Black level, 1 / (white - black), the mip level to sample, unused.
    levels: vec4<f32>,
    // White balance gains for red, green and blue; exposure gain in w.
    wb: vec4<f32>,
    // Camera RGB to the working space, one row per vec4.
    cam0: vec4<f32>,
    cam1: vec4<f32>,
    cam2: vec4<f32>,
    // Working space to the display space, one row per vec4.
    out0: vec4<f32>,
    out1: vec4<f32>,
    out2: vec4<f32>,
    // While dragging: the white balance ratio applied after the demosaic.
    drag: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;

const EPS: f32 = 1e-5;
const EPSSQ: f32 = 1e-10;

// Shaders avoid indexing vectors with runtime values: the FXC compiler used
// by D3D12 on WARP rejects it. `pick3` and `onehot` stand in for it.
fn fc(p: vec2<i32>) -> u32 {
    let odd_x = (p.x & 1) == 1;
    let top = select(params.cfa.x, params.cfa.y, odd_x);
    let bottom = select(params.cfa.z, params.cfa.w, odd_x);
    return select(top, bottom, (p.y & 1) == 1);
}

fn pick3(v: vec3<f32>, i: u32) -> f32 {
    return select(select(v.x, v.y, i == 1u), v.z, i == 2u);
}

fn onehot(i: u32) -> vec3<f32> {
    return vec3<f32>(f32(i == 0u), f32(i == 1u), f32(i == 2u));
}

fn full_size() -> vec2<i32> {
    return vec2<i32>(params.dims.xy);
}

// Loads one channel with the coordinates clamped to the image.
fn ld(t: texture_2d<f32>, p: vec2<i32>) -> f32 {
    return textureLoad(t, clamp(p, vec2<i32>(0), full_size() - 1), 0).r;
}

fn ld2(t: texture_2d<f32>, p: vec2<i32>) -> vec2<f32> {
    return textureLoad(t, clamp(p, vec2<i32>(0), full_size() - 1), 0).rg;
}

fn sq(x: f32) -> f32 {
    return x * x;
}

fn in_image(p: vec2<i32>) -> bool {
    return all(p >= vec2<i32>(0)) && all(p < full_size());
}
