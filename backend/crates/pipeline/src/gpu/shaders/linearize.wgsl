// Stage 1 and 2: raw code values to normalized, white-balanced CFA samples.

@group(0) @binding(1) var raw: texture_2d<u32>;
@group(0) @binding(2) var cfa_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let code = f32(textureLoad(raw, p, 0).r);
    let v = max(code - params.levels.x, 0.0) * params.levels.y * pick3(params.wb.xyz, fc(p));
    textureStore(cfa_out, p, vec4<f32>(v, 0.0, 0.0, 0.0));
}
