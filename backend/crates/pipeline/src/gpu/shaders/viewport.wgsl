// Stage 3: resample the mip pyramid to the viewport, apply the white balance
// ratio while dragging, then the camera matrix. The result is the
// working-space image, cached at viewport size.

@group(0) @binding(1) var mips: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var working_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let view = params.dims.zw;
    if (any(id.xy >= view)) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(view);
    let cam = textureSampleLevel(mips, samp, uv, params.levels.z).rgb * params.drag.xyz;
    let w = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam));
    textureStore(working_out, vec2<i32>(id.xy), vec4<f32>(w, 1.0));
}
