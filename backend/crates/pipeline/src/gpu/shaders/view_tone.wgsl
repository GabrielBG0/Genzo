// Stage 6 (DEV-01, M1 part) on the view, as pipeline::reference::look
// computes it: exposure, Highlights and Shadows (from the base layer, read
// as reference::base does), Whites and Blacks, and Contrast on luminance
// in t = log2(Y / 0.18), RGB scaled by the luminance ratio; then, for Genzo
// Standard, the base curve (DEV-02): into linear ProPhoto, the exposure
// ramp per channel, the DNG SDK's RGBTone through the curve table, and
// back. The table is read from a buffer and interpolated here: a filtered
// texture's 8-bit fraction would not hold one f16 step.

struct Tone {
    // AP1's Y row.
    y_row: vec4<f32>,
    to_prophoto: array<vec4<f32>, 3>,
    from_prophoto: array<vec4<f32>, 3>,
    // Exposure (EV), Whites, Blacks and Contrast over 100.
    controls: vec4<f32>,
    // The Whites end, the Blacks end (in t), their range, Contrast's gain.
    ends: vec4<f32>,
    // The ramp: black, toe radius, slope, quadratic scale.
    ramp: vec4<f32>,
    // Highlights and Shadows over 100, the floor of t, 1 to read the base
    // layer.
    local: vec4<f32>,
    // Anchored view pixel centers (`offset` added) to the base layer's
    // working coordinates.
    to_base: array<vec4<f32>, 2>,
    // The base layer's width and height.
    base_size: vec4<u32>,
    // Width, height, 1 to apply the base curve, unused.
    size: vec4<u32>,
    // The view's whole-pixel offset (view_sample.wgsl).
    offset: vec4<i32>,
}

@group(0) @binding(0) var<uniform> tone: Tone;
@group(0) @binding(1) var input: texture_2d<f32>;
@group(0) @binding(2) var result: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<storage, read> curve: array<f32>;
// (Ā, b̄) per working pixel, as f16 pairs (gpu::base).
@group(0) @binding(4) var<storage, read> coefficients: array<u32>;

const MIDDLE_GRAY: f32 = 0.18;
const MIN_LUMINANCE: f32 = 1e-10;
const TABLE_SIZE: f32 = 4096.0;

fn m3(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}

// r(u): u² up to 1, then 2u − 1.
fn ramp_up(u: f32) -> f32 {
    if (u <= 1.0) {
        return u * u;
    }
    return 2.0 * u - 1.0;
}

// One return and no test on the slider values: an early-return form lost
// the device on WARP 10.0.20348 (Windows Server 2022) when both sliders
// were 0. A slider at 0 shifts by 0 here too, since its source point is
// then its end, which is not 0.
fn whites_blacks(t: f32) -> f32 {
    let range = tone.ends.z;
    // Whites above middle gray, Blacks below.
    let amount = select(tone.controls.z, tone.controls.y, t > 0.0);
    let end = select(tone.ends.y, tone.ends.x, t > 0.0);
    let source_point = end - range * amount;
    return t + range * amount * ramp_up(t / source_point);
}

fn contrast(t: f32) -> f32 {
    let sigma = select(1.6, 1.0, t > 0.0);
    return t + tone.ends.w * tone.controls.w * t * exp(-t * t / (2.0 * sigma * sigma));
}

fn coefficient(x: i32, y: i32) -> vec2<f32> {
    return unpack2x16float(coefficients[u32(y) * tone.base_size.x + u32(x)]);
}

// The base B at 0 EV of the pixel at view position p whose t at 0 EV is
// t0: the coefficients bilinearly at its working coordinates, edges
// clamped (reference::base::coefficients_at and base).
fn local_base(p: vec2<f32>, t0: f32) -> f32 {
    let at = vec3<f32>(p, 1.0);
    let u = vec2<f32>(dot(tone.to_base[0].xyz, at), dot(tone.to_base[1].xyz, at)) - 0.5;
    let i = floor(u);
    let f = u - i;
    let edge = vec2<i32>(tone.base_size.xy) - 1;
    let a = clamp(vec2<i32>(i), vec2<i32>(0), edge);
    let b = clamp(vec2<i32>(i) + 1, vec2<i32>(0), edge);
    let top = mix(coefficient(a.x, a.y), coefficient(b.x, a.y), f.x);
    let bottom = mix(coefficient(a.x, b.y), coefficient(b.x, b.y), f.x);
    let ab = mix(top, bottom, f.y);
    return ab.x * max(t0, tone.local.z) + ab.y;
}

// Highlights and Shadows (color::tone::highlights_shadows): B moves by up
// to 2 EV, weighted by smoothstep ramps to +2.5 and -4 EV.
fn highlights_shadows(t: f32, b: f32) -> f32 {
    let up = smoothstep(0.0, 2.5, b);
    let down = smoothstep(0.0, 4.0, -b);
    return t + 2.0 * (tone.local.x * up + tone.local.y * down);
}

// dng_function_exposure_ramp.
fn exposure_ramp(x: f32) -> f32 {
    let black = tone.ramp.x;
    let radius = tone.ramp.y;
    if (x <= black - radius) {
        return 0.0;
    }
    if (x >= black + radius) {
        return min((x - black) * tone.ramp.z, 1.0);
    }
    let y = x - (black - radius);
    return tone.ramp.w * y * y;
}

// dng_1d_table::Interpolate.
fn table(x: f32) -> f32 {
    let y = clamp(x, 0.0, 1.0) * TABLE_SIZE;
    let index = min(u32(y), 4096u);
    let fract = y - f32(index);
    return curve[index] * (1.0 - fract) + curve[index + 1u] * fract;
}

// RefBaselineRGBTone's step: hi and lo through the curve, mid at its
// fraction between them.
fn tone3(hi: f32, mid: f32, lo: f32) -> vec3<f32> {
    let hh = table(hi);
    let ll = table(lo);
    return vec3<f32>(hh, ll + ((hh - ll) * (mid - lo) / (hi - lo)), ll);
}

fn rgb_tone(c: vec3<f32>) -> vec3<f32> {
    let r = c.x;
    let g = c.y;
    let b = c.z;
    if (r >= g) {
        if (g > b) {
            return tone3(r, g, b);
        } else if (b > r) {
            let t = tone3(b, r, g);
            return vec3<f32>(t.y, t.z, t.x);
        } else if (b > g) {
            let t = tone3(r, b, g);
            return vec3<f32>(t.x, t.z, t.y);
        }
        let rr = table(r);
        let gg = table(g);
        return vec3<f32>(rr, gg, gg);
    }
    if (r >= b) {
        let t = tone3(g, r, b);
        return vec3<f32>(t.y, t.x, t.z);
    } else if (b > g) {
        let t = tone3(b, g, r);
        return vec3<f32>(t.z, t.y, t.x);
    }
    let t = tone3(g, b, r);
    return vec3<f32>(t.z, t.x, t.y);
}

fn base_curve(rgb: vec3<f32>) -> vec3<f32> {
    let p = clamp(m3(tone.to_prophoto, rgb), vec3<f32>(0.0), vec3<f32>(1.0));
    let ramped = vec3<f32>(exposure_ramp(p.x), exposure_ramp(p.y), exposure_ramp(p.z));
    return m3(tone.from_prophoto, rgb_tone(ramped));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= tone.size.x || id.y >= tone.size.y) {
        return;
    }
    let texel = textureLoad(input, vec2<i32>(id.xy), 0);
    var rgb = texel.rgb;
    let y = dot(tone.y_row.xyz, rgb);
    var ratio = exp2(tone.controls.x);
    if (y > MIN_LUMINANCE) {
        let t0 = log2(y / MIDDLE_GRAY);
        var t = t0 + tone.controls.x;
        if (tone.local.w != 0.0) {
            // Exposure shifts the base by the same amount.
            let p = vec2<f32>(vec2<i32>(id.xy) + tone.offset.xy) + 0.5;
            t = highlights_shadows(t, local_base(p, t0) + tone.controls.x);
        }
        t = contrast(whites_blacks(t));
        ratio = MIDDLE_GRAY * exp2(t) / y;
    }
    rgb = rgb * ratio;
    if (tone.size.z != 0u) {
        rgb = base_curve(rgb);
    }
    textureStore(result, vec2<i32>(id.xy), vec4<f32>(rgb, texel.a));
}
