// The drag preview: the 8-bit display frame at half size, a 2×2 box filter
// clamped at the edges for odd sizes. Averages the encoded values, which is
// close enough for a preview that is replaced by the full frame on release.

@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (any(p >= vec2<i32>(textureDimensions(dst)))) {
        return;
    }
    let last = vec2<i32>(textureDimensions(src)) - 1;
    let q = 2 * p;
    let sum = textureLoad(src, min(q, last), 0)
        + textureLoad(src, min(q + vec2(1, 0), last), 0)
        + textureLoad(src, min(q + vec2(0, 1), last), 0)
        + textureLoad(src, min(q + vec2(1, 1), last), 0);
    textureStore(dst, p, sum * 0.25);
}
