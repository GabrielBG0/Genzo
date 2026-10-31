// Stage 9, the output transform (DEV-01), as color::output computes it:
// linear AP1 to the target's linear RGB, hue-preserving gamut compression
// in Oklab chroma, the clamp, the sRGB transfer function, and 8 bits with a
// triangular dither from an integer hash (none for an unedited non-RAW
// photo). The dither is drawn at the pixel's anchored position (`offset`
// added), so at 1:1 it does not depend on the pan. Each pixel goes out as
// one u32 texel, R in the low byte. The half-size drag preview takes each
// pixel from the mean of a 2 x 2 block of the cache, in linear light.
//
// The histogram (DEV-03, pipeline::histogram) is counted from the codes
// before anything is drawn over them: per workgroup in shared atomics,
// merged into the storage buffer. Then the Alt preview replaces the pixel,
// or the J overlay draws a clipped one in its color.

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
    // The J overlay's highlight and shadow colors (RGBA8, R low), 1 when
    // each is on (bit 0 highlights, bit 1 shadows), and the preview: 0
    // none, 1 highlights, 2 shadows.
    overlay: vec4<u32>,
    // 1 to count, 1 to write the frame, and the view's width and height:
    // the cache's textures can be larger (ViewStages keeps them when the
    // view shrinks), so the half-size preview clamps at the view's edge.
    mode: vec4<u32>,
}

@group(0) @binding(0) var<uniform> out: Output;
@group(0) @binding(1) var sampled: texture_2d<f32>;
@group(0) @binding(2) var frame: texture_storage_2d<r32uint, write>;
// pipeline::histogram::WORDS: R, G and B bins, highlight pixels and
// channels, shadow pixels and channels, pixels counted.
@group(0) @binding(3) var<storage, read_write> histogram: array<atomic<u32>, 773>;

const WORDS: u32 = 773u;
const HIGH_PIXELS: u32 = 768u;
const HIGH_CHANNELS: u32 = 769u;
const LOW_PIXELS: u32 = 770u;
const LOW_CHANNELS: u32 = 771u;
const PIXELS: u32 = 772u;

var<workgroup> local: array<atomic<u32>, 773>;

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
        let last = vec2<i32>(out.mode.zw) - 1;
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

// R, G and B set to `on` where `mask` has their bit and `off` elsewhere,
// opaque.
fn by_mask(mask: u32, on: u32, off: u32) -> u32 {
    var rgba = 255u << 24u;
    for (var c = 0u; c < 3u; c++) {
        rgba |= select(off, on, (mask & (1u << c)) != 0u) << (8u * c);
    }
    return rgba;
}

// What is written for the photo's pixel (pipeline::histogram::Overlay::apply).
fn drawn(rgba: u32, high: u32, low: u32) -> u32 {
    let preview = out.overlay.w;
    if (preview == 1u) {
        return by_mask(high, 255u, 0u);
    }
    if (preview == 2u) {
        return by_mask(low, 0u, 255u);
    }
    if ((out.overlay.z & 1u) != 0u && high != 0u) {
        return out.overlay.x | (255u << 24u);
    }
    if ((out.overlay.z & 2u) != 0u && low != 0u) {
        return out.overlay.y | (255u << 24u);
    }
    return rgba;
}

@compute @workgroup_size(16, 16)
fn main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    // No early return: the barriers need every invocation.
    let counting = out.mode.x != 0u;
    if (counting) {
        for (var i = li; i < WORDS; i += 256u) {
            atomicStore(&local[i], 0u);
        }
    }
    workgroupBarrier();
    if (id.x < out.size.x && id.y < out.size.y) {
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
        let high = u32(r == 255u) | (u32(g == 255u) << 1u) | (u32(b == 255u) << 2u);
        let low = u32(r == 0u) | (u32(g == 0u) << 1u) | (u32(b == 0u) << 2u);
        if (counting) {
            atomicAdd(&local[r], 1u);
            atomicAdd(&local[256u + g], 1u);
            atomicAdd(&local[512u + b], 1u);
            if (high != 0u) {
                atomicAdd(&local[HIGH_PIXELS], 1u);
                atomicOr(&local[HIGH_CHANNELS], high);
            }
            if (low != 0u) {
                atomicAdd(&local[LOW_PIXELS], 1u);
                atomicOr(&local[LOW_CHANNELS], low);
            }
            atomicAdd(&local[PIXELS], 1u);
        }
        if (out.mode.y != 0u) {
            textureStore(frame, vec2<i32>(id.xy), vec4<u32>(drawn(rgba, high, low), 0u, 0u, 0u));
        }
    }
    workgroupBarrier();
    if (counting) {
        for (var i = li; i < WORDS; i += 256u) {
            let v = atomicLoad(&local[i]);
            if (v != 0u) {
                if (i == HIGH_CHANNELS || i == LOW_CHANNELS) {
                    atomicOr(&histogram[i], v);
                } else {
                    atomicAdd(&histogram[i], v);
                }
            }
        }
    }
}
