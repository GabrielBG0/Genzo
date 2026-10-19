// Stage 7 (DEV-01, M1 part) on the view, as color::look computes it:
// Vibrance then Saturation scale Oklab chroma (a and b), keeping lightness
// and hue; the B&W treatment writes AP1 luminance to all three channels.

struct Look {
    // AP1 to Oklab's cone responses, and back.
    to_lms: array<vec4<f32>, 3>,
    from_lms: array<vec4<f32>, 3>,
    // Cube-rooted cone responses to Oklab (M2), and back.
    lms_to_lab: array<vec4<f32>, 3>,
    lab_to_lms: array<vec4<f32>, 3>,
    y_row: vec4<f32>,
    // Vibrance and Saturation over 100, 1 for B&W, C_ref.
    controls: vec4<f32>,
    // Width, height.
    size: vec4<u32>,
}

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var input: texture_2d<f32>;
@group(0) @binding(2) var result: texture_storage_2d<rgba16float, write>;

fn m3(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}

fn cbrt(v: vec3<f32>) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(1.0 / 3.0));
}

// 1 from 25° to 65°, 0 below 15° and above 75° (color::look::skin).
fn skin(hue: f32) -> f32 {
    return smoothstep(15.0, 25.0, hue) * (1.0 - smoothstep(65.0, 75.0, hue));
}

fn vibrance_gain(c: f32, hue: f32) -> f32 {
    let v = look.controls.x;
    if (v < 0.0) {
        return 1.0 + v;
    }
    return 1.0 + v * max(1.0 - c / look.controls.w, 0.0) * (1.0 - 0.5 * skin(hue));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= look.size.x || id.y >= look.size.y) {
        return;
    }
    let texel = textureLoad(input, vec2<i32>(id.xy), 0);
    var rgb = texel.rgb;
    if (look.controls.z != 0.0) {
        rgb = vec3<f32>(dot(look.y_row.xyz, rgb));
    } else {
        let lab = m3(look.lms_to_lab, cbrt(m3(look.to_lms, rgb)));
        let c = length(lab.yz);
        var hue = degrees(atan2(lab.z, lab.y));
        if (hue < 0.0) {
            hue += 360.0;
        }
        let gain = vibrance_gain(c, hue) * (1.0 + look.controls.y);
        let lms = m3(look.lab_to_lms, vec3<f32>(lab.x, lab.yz * gain));
        rgb = m3(look.from_lms, lms * lms * lms);
    }
    textureStore(result, vec2<i32>(id.xy), vec4<f32>(rgb, texel.a));
}
