// A texel of a pyramid level, for view_sample.wgsl.

@group(0) @binding(1) var level: texture_2d<f32>;

fn fetch(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(level, p, 0).rgb;
}
