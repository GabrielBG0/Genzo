// Stage 9, the output transform (DEV-01), as color::output computes it:
// linear AP1 to the target's linear RGB, hue-preserving gamut compression
// in Oklab chroma, the clamp, the sRGB transfer function, and 8 bits with a
// triangular dither from an integer hash (none for an unedited non-RAW
// photo). The dither is drawn at the pixel's anchored position (`offset`
// added), so at 1:1 it does not depend on the pan. Each pixel goes out as
// one u32 texel, R in the low byte. The half-size drag preview takes each
// pixel from the mean of a 2 x 2 block of the cache, in linear light.

struct Output {
    // AP1 to linear target RGB, one row per vec4.
    to_target: array<vec4<f32>, 3>,
    // Linear target RGB to Oklab's cone responses (M1 · RGB to XYZ).
    to_lms: array<vec4<f32>, 3>,
    // Cone responses (cube-rooted) to Oklab: M2.
    lms_to_lab: array<vec4<f32>, 3>,
    // Back: Oklab to cone responses (cubed after), and those to the target.
    lab_to_lms: array<vec4<f32>, 3>,
    lms_to_rgb: array<vec4<f32>, 3>,
    // Threshold, limit, power, 1 to compress or 0 not to.
    compression: vec4<f32>,
    // Width, height, bisection steps, 1 to dither or 0 not to.
    size: vec4<u32>,
    // The view's whole-pixel offset (view_sample.wgsl), and 2 for the
    // half-size preview or 1.
    offset: vec4<i32>,
}

@group(0) @binding(0) var<uniform> out: Output;
@group(0) @binding(1) var sampled: texture_2d<f32>;
@group(0) @binding(2) var frame: texture_storage_2d<r32uint, write>;

fn m3(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}

fn cbrt(v: vec3<f32>) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(1.0 / 3.0));
}

fn to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    return m3(out.lms_to_lab, cbrt(m3(out.to_lms, rgb)));
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let lms = m3(out.lab_to_lms, lab);
    return m3(out.lms_to_rgb, lms * lms * lms);
}

fn inside(rgb: vec3<f32>) -> bool {
    return all(rgb >= vec3<f32>(0.0)) && all(rgb <= vec3<f32>(1.0));
}

// The ACES reference gamut compression's curve.
fn compress_distance(d: f32, threshold: f32, limit: f32, power: f32) -> f32 {
    if (d < threshold) {
        return d;
    }
    let scale = (limit - threshold) / pow(pow((1.0 - threshold) / (limit - threshold), -power) - 1.0, 1.0 / power);
    let n = (d - threshold) / scale;
    return threshold + scale * n / pow(1.0 + pow(n, power), 1.0 / power);
}

fn compress(rgb: vec3<f32>) -> vec3<f32> {
    let lab = to_oklab(rgb);
    let chroma = length(lab.yz);
    if (lab.x <= 0.0 || lab.x >= 1.0 || chroma <= 0.0) {
        return rgb;
    }
    let dir = lab.yz / chroma;
    // The target's largest chroma at this lightness and hue, by bisection
    // (color::output::max_chroma).
    var lo = 0.0;
    var hi = 0.5;
    for (var i = 0u; i < out.size.z; i++) {
        let mid = 0.5 * (lo + hi);
        if (inside(from_oklab(vec3<f32>(lab.x, mid * dir)))) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if (lo <= 0.0) {
        return rgb;
    }
    let c = out.compression;
    let r = chroma / lo;
    if (r < c.x) {
        return rgb;
    }
    let k = min(compress_distance(r, c.x, c.y, c.z), 1.0);
    return from_oklab(vec3<f32>(lab.x, k * lo * dir));
}

// The cache value pixel `p` of the frame starts from: the cache's own, or
// at half size the mean of its 2 x 2 block, clamped at the edge.
fn source(p: vec2<i32>) -> vec3<f32> {
    var rgb = textureLoad(sampled, p, 0).rgb;
    if (out.offset.z == 2) {
        let last = vec2<i32>(textureDimensions(sampled)) - 1;
        let q = 2 * p;
        let sum = textureLoad(sampled, min(q, last), 0).rgb
            + textureLoad(sampled, min(q + vec2(1, 0), last), 0).rgb
            + textureLoad(sampled, min(q + vec2(0, 1), last), 0).rgb
            + textureLoad(sampled, min(q + vec2(1, 1), last), 0).rgb;
        rgb = sum * 0.25;
    }
    return rgb;
}

fn srgb_encode(v: f32) -> f32 {
    if (v <= 0.0031308) {
        return 12.92 * v;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

// PCG's output permutation as a hash (color::output::pcg).
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn dither(x: u32, y: u32) -> f32 {
    let a = pcg(x ^ pcg(y));
    let b = pcg(a);
    return f32(a >> 8u) / 16777216.0 + f32(b >> 8u) / 16777216.0 - 1.0;
}

fn quantize(v: f32, noise: f32) -> u32 {
    return u32(clamp(floor(v * 255.0 + 0.5 + noise), 0.0, 255.0));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= out.size.x || id.y >= out.size.y) {
        return;
    }
    var rgb = m3(out.to_target, source(vec2<i32>(id.xy)));
    if (out.compression.w != 0.0) {
        rgb = compress(rgb);
    }
    let at = bitcast<vec2<u32>>(vec2<i32>(id.xy) + out.offset.xy);
    let noise = select(0.0, dither(at.x, at.y), out.size.w != 0u);
    let r = quantize(srgb_encode(clamp(rgb.x, 0.0, 1.0)), noise);
    let g = quantize(srgb_encode(clamp(rgb.y, 0.0, 1.0)), noise);
    let b = quantize(srgb_encode(clamp(rgb.z, 0.0, 1.0)), noise);
    let rgba = r | (g << 8u) | (b << 16u) | (255u << 24u);
    textureStore(frame, vec2<i32>(id.xy), vec4<u32>(rgba, 0u, 0u, 0u));
}
