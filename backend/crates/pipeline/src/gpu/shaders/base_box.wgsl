// The base layer's guided filter (DEV-01, stage 6, step 2), as
// reference::base::coefficients computes it, in four separable box passes
// over two planes of one u32 per pixel, the window cropped to the image:
//
// 0. across, from t (f32): the row window's mean and variance, as f16;
// 1. down: the square window's mean μ and variance σ², then
//    a = σ² / (σ² + ε) and b = μ (1 − a), as f16;
// 2. across, then 3. down: the means of a and b, as f16.
//
// The plane between passes 0 and 1 holds each row window's mean and
// variance, not its sums of t and t²: at t of −16, t² is 256, where f16
// steps by 0.25 and ε is 0.05. Sums are of differences from the center
// pixel, in f32. Pass 1 adds the rows' own variances to the variance of
// their means, since each row of a cropped window holds as many pixels.

struct Box {
    // Width, height, radius, pass (0 to 3).
    size: vec4<u32>,
    // ε, unused.
    eps: vec4<f32>,
}

@group(0) @binding(0) var<uniform> box: Box;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

fn index(x: u32, y: u32) -> u32 {
    return y * box.size.x + x;
}

// The pixels of the window along an axis of length n around i.
fn window(i: u32, n: u32) -> vec2<u32> {
    let r = box.size.z;
    return vec2<u32>(select(0u, i - r, i >= r), min(i + r, n - 1u));
}

fn moments_across(x: u32, y: u32) -> u32 {
    let w = window(x, box.size.x);
    let c = bitcast<f32>(src[index(x, y)]);
    var s = 0.0;
    var ss = 0.0;
    for (var k = w.x; k <= w.y; k++) {
        let d = bitcast<f32>(src[index(k, y)]) - c;
        s += d;
        ss += d * d;
    }
    let n = f32(w.y - w.x + 1u);
    let m = s / n;
    return pack2x16float(vec2<f32>(c + m, max(ss / n - m * m, 0.0)));
}

fn coefficients_down(x: u32, y: u32) -> u32 {
    let w = window(y, box.size.y);
    let c = unpack2x16float(src[index(x, y)]).x;
    var s = 0.0;
    var ss = 0.0;
    var rows = 0.0;
    for (var k = w.x; k <= w.y; k++) {
        let v = unpack2x16float(src[index(x, k)]);
        let d = v.x - c;
        s += d;
        ss += d * d;
        rows += v.y;
    }
    let n = f32(w.y - w.x + 1u);
    let m = s / n;
    let variance = max(rows / n + ss / n - m * m, 0.0);
    let a = variance / (variance + box.eps.x);
    return pack2x16float(vec2<f32>(a, (c + m) * (1.0 - a)));
}

fn mean_across(x: u32, y: u32) -> u32 {
    let w = window(x, box.size.x);
    var s = vec2<f32>(0.0);
    for (var k = w.x; k <= w.y; k++) {
        s += unpack2x16float(src[index(k, y)]);
    }
    return pack2x16float(s / f32(w.y - w.x + 1u));
}

fn mean_down(x: u32, y: u32) -> u32 {
    let w = window(y, box.size.y);
    var s = vec2<f32>(0.0);
    for (var k = w.x; k <= w.y; k++) {
        s += unpack2x16float(src[index(x, k)]);
    }
    return pack2x16float(s / f32(w.y - w.x + 1u));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= box.size.x || id.y >= box.size.y) {
        return;
    }
    var v: u32;
    switch box.size.w {
        case 0u: {
            v = moments_across(id.x, id.y);
        }
        case 1u: {
            v = coefficients_down(id.x, id.y);
        }
        case 2u: {
            v = mean_across(id.x, id.y);
        }
        default: {
            v = mean_down(id.x, id.y);
        }
    }
    dst[index(id.x, id.y)] = v;
}
