// The sampled cache, for view_sample.wgsl.

@group(0) @binding(2) var sampled: texture_storage_2d<rgba16float, write>;

fn store(p: vec2<u32>, rgb: vec3<f32>) {
    textureStore(sampled, vec2<i32>(p), vec4<f32>(rgb, 1.0));
}
