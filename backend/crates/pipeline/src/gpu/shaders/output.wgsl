// Stages 4 and 5: exposure, then the output transform to an 8-bit display
// encoding (sRGB or Display P3, both with the sRGB transfer curve).

@group(0) @binding(1) var working: texture_2d<f32>;
@group(0) @binding(2) var display_out: texture_storage_2d<rgba8unorm, write>;

fn encode(x: f32) -> f32 {
    if (x <= 0.0031308) {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= params.dims.zw)) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let w = textureLoad(working, p, 0).rgb * params.wb.w;
    let d = clamp(
        vec3<f32>(dot(params.out0.xyz, w), dot(params.out1.xyz, w), dot(params.out2.xyz, w)),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    textureStore(display_out, p, vec4<f32>(encode(d.r), encode(d.g), encode(d.b), 1.0));
}
