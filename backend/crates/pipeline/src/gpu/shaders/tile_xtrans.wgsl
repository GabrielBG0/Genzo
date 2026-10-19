// Derived from darktable's data/kernels/demosaic_markesteijn.cl (copyright
// (c) 2016 Ulrich Pegelow) and src/iop/demosaicing/xtrans.c and vng.c
// (Copyright (C) 2010-2026 darktable developers; xtrans_interpolate adapted
// from dcraw 9.20, Frank Markesteijn's algorithm for Fuji X-Trans sensors).
// darktable is free software under the GNU General Public License, version 3
// or (at your option) any later version. Modified for Genzo (2026): ported to
// WGSL compute shaders, run per tile, with the passes after the multipass
// loop fused and darktable's C linear border fill.
// Genzo is distributed under the GPL-3.0; see LICENSE.

// Markesteijn, 3 passes, for one tile (DEV-01, stage 3). Entry points in
// the order the host runs them. `rgb` holds 8 directional RGB images, `n`
// pixels each, four floats per pixel; `scratch` holds, in turn, the green
// min/max (2 per pixel) and the solitary green runs' state (4 per pixel),
// then the 8 derivatives, then the 8 homogeneity sums (as u32); `homo`
// holds the 8 homogeneity maps, four per u32.

struct XImage {
    // The green hexagon around each pixel of a 3 x 3 cell (from the stored
    // origin), as (x, y) offsets: cell (row * 3 + col), entry k at
    // hex[(cell * 8 + k) / 2], .xy for even k, .zw for odd.
    hex: array<vec4<i32>, 36>,
    // The solitary green's (x, y) in the 3 x 3 cell.
    sgreen: vec4<i32>,
}

struct Run {
    // d, the direction (x, y), hcomp (solitary green); d (red and blue,
    // two by two).
    p: vec4<i32>,
    // The rgb image this run reads and writes, the base of the pass's set
    // (0 or 4).
    q: vec4<u32>,
}

@group(0) @binding(1) var<uniform> xi: XImage;
@group(0) @binding(2) var<uniform> run: Run;
@group(0) @binding(3) var cfa: texture_2d<f32>;
@group(0) @binding(4) var<storage, read_write> rgb: array<f32>;
@group(0) @binding(5) var<storage, read_write> scratch: array<u32>;
@group(0) @binding(6) var<storage, read_write> homo: array<u32>;
@group(0) @binding(7) var out: texture_storage_2d<rgba32float, write>;

const PAD_G1_G3: i32 = 3;
const PAD_G_INTERP: i32 = 3;
const PAD_G_RECALC: i32 = 6;
const PAD_RB_G: i32 = 5;
const PAD_RB_BR: i32 = 5;
const PAD_G22: i32 = 4;
const PAD_DRV: i32 = 14;
const PAD_HOMO: i32 = 15;
const PAD_TILE: i32 = 17;

fn npix() -> u32 {
    return tile.size.x * tile.size.y;
}

fn clamped(p: vec2<i32>) -> vec2<i32> {
    return clamp(p, vec2<i32>(0), local_size() - 1);
}

fn pidx(p: vec2<i32>) -> u32 {
    let q = clamped(p);
    return u32(q.y) * tile.size.x + u32(q.x);
}

// Channel c of direction image d at local p (reads clamp to the tile).
fn px(d: u32, p: vec2<i32>, c: u32) -> f32 {
    return rgb[((d * npix() + pidx(p)) * 4u) + c];
}

fn set_px(d: u32, p: vec2<i32>, c: u32, v: f32) {
    rgb[((d * npix() + pidx(p)) * 4u) + c] = v;
}

// The X-Trans color at local p, safe for neighbors past the tile.
fn color(p: vec2<i32>) -> u32 {
    let s = stored(p);
    let i = u32(((s.y % 6) + 6) % 6 * 6 + ((s.x % 6) + 6) % 6);
    return pick4u(tile.xtrans[i / 4u], i % 4u);
}

// Hexagon entry k around local p.
fn hexo(p: vec2<i32>, k: u32) -> vec2<i32> {
    let s = stored(p);
    let cell = u32(((s.y % 3) + 3) % 3 * 3 + ((s.x % 3) + 3) % 3);
    let e = cell * 8u + k;
    let v = xi.hex[e / 2u];
    return select(v.xy, v.zw, (e & 1u) == 1u);
}

// (stored - sgreen) % 3, C's truncated remainder, as darktable computes it.
fn mod3y(p: vec2<i32>) -> i32 {
    return (stored(p).y - xi.sgreen.y) % 3;
}

fn mod3x(p: vec2<i32>) -> i32 {
    return (stored(p).x - xi.sgreen.x) % 3;
}

// Whether local p is at least `border` from the active area's edges.
fn inside(p: vec2<i32>, border: i32) -> bool {
    let g = global(p);
    return in_tile(p) && all(g >= vec2<i32>(border)) && all(g < image_size() - border);
}

fn gmin(p: vec2<i32>) -> f32 {
    return bitcast<f32>(scratch[pidx(p) * 2u]);
}

fn gmax(p: vec2<i32>) -> f32 {
    return bitcast<f32>(scratch[pidx(p) * 2u + 1u]);
}

// The solitary green runs' state, after the min/max.
fn aux(p: vec2<i32>, c: u32) -> f32 {
    return bitcast<f32>(scratch[2u * npix() + pidx(p) * 4u + c]);
}

fn set_aux(p: vec2<i32>, c: u32, v: f32) {
    scratch[2u * npix() + pidx(p) * 4u + c] = bitcast<u32>(v);
}

// The mosaic into the first four direction images.
@compute @workgroup_size(16, 16)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_tile(p)) {
        return;
    }
    let f = color(p);
    let v = max(0.0, textureLoad(cfa, p, 0).r);
    for (var d = 0u; d < 4u; d++) {
        for (var c = 0u; c < 3u; c++) {
            set_px(d, p, c, select(0.0, v, c == f));
        }
    }
}

// The first set's four images into the second (the second pass on).
@compute @workgroup_size(16, 16)
fn copy_set(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_tile(p)) {
        return;
    }
    for (var d = 0u; d < 4u; d++) {
        for (var c = 0u; c < 4u; c++) {
            set_px(d + 4u, p, c, px(d, p, c));
        }
    }
}

// The green min/max of a red/blue pixel: over its hexagon and its pair
// partner's (they are the same hexagon; markesteijn_green_minmax).
@compute @workgroup_size(16, 16)
fn minmax(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_G1_G3) || color(p) == 1u) {
        return;
    }
    var lo = 3.40282347e38;
    var hi = 1.17549435e-38;
    for (var c = 0u; c < 6u; c++) {
        let v = px(0u, p + hexo(p, c), 1u);
        lo = min(lo, v);
        hi = max(hi, v);
    }
    var q = p;
    let nb = array<vec2<i32>, 4>(vec2(1, 0), vec2(0, 1), vec2(-1, 0), vec2(0, -1));
    for (var n = 0; n < 4; n++) {
        if (color(p + nb[n]) != 1u) {
            q = p + nb[n];
        }
    }
    for (var c = 0u; c < 6u; c++) {
        let v = px(0u, q + hexo(q, c), 1u);
        lo = min(lo, v);
        hi = max(hi, v);
    }
    scratch[pidx(p) * 2u] = bitcast<u32>(lo);
    scratch[pidx(p) * 2u + 1u] = bitcast<u32>(hi);
}

// Green horizontally, vertically and along both diagonals
// (markesteijn_interpolate_green).
@compute @workgroup_size(16, 16)
fn green(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    let f = color(p);
    if (!inside(p, PAD_G_INTERP) || f == 1u) {
        return;
    }
    var col: array<f32, 4>;
    col[0] = 0.6796875 * (px(0u, p + hexo(p, 1u), 1u) + px(0u, p + hexo(p, 0u), 1u))
        - 0.1796875 * (px(0u, p + 2 * hexo(p, 1u), 1u) + px(0u, p + 2 * hexo(p, 0u), 1u));
    col[1] = 0.87109375 * px(0u, p + hexo(p, 3u), 1u) + 0.13 * px(0u, p + hexo(p, 2u), 1u)
        + 0.359375 * (px(0u, p, f) - px(0u, p - hexo(p, 2u), f));
    for (var c = 0u; c < 2u; c++) {
        let h = hexo(p, 4u + c);
        col[2u + c] = 0.640625 * px(0u, p + h, 1u) + 0.359375 * px(0u, p - 2 * h, 1u)
            + 0.12890625 * (2.0 * px(0u, p, f) - px(0u, p + 3 * h, f) - px(0u, p - 3 * h, f));
    }
    let flip = select(0u, 1u, mod3y(p) == 0);
    for (var c = 0u; c < 4u; c++) {
        set_px(c ^ flip, p, 1u, clamp(col[c], gmin(p), gmax(p)));
    }
}

// Green again from interpolated values of closer pixels, in the second
// and third passes (markesteijn_recalculate_green).
@compute @workgroup_size(16, 16)
fn recalc(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    let f = color(p);
    if (!inside(p, PAD_G_RECALC) || f == 1u) {
        return;
    }
    let flip = select(0u, 1u, mod3y(p) == 0);
    for (var d = 3u; d < 6u; d++) {
        let img = run.q.y + ((d - 2u) ^ flip);
        let h = hexo(p, d);
        let v = px(img, p - 2 * h, 1u) + 2.0 * px(img, p + h, 1u) - px(img, p - 2 * h, f)
            - 2.0 * px(img, p + h, f) + 3.0 * px(img, p, f);
        set_px(img, p, 1u, clamp(v / 3.0, gmin(p), gmax(p)));
    }
}

// Red and blue for solitary green pixels, one of six runs
// (markesteijn_solitary_green).
@compute @workgroup_size(16, 16)
fn solitary(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_RB_G) || mod3x(p) != 0 || mod3y(p) != 0) {
        return;
    }
    let d = run.p.x;
    let dir = run.p.yz;
    let img = run.q.x;
    var h = color(p + vec2(1, 0)) ^ u32(run.p.w);
    var c4: array<f32, 4>;
    var o4: array<f32, 4>;
    if (d > 0) {
        for (var c = 0u; c < 4u; c++) {
            c4[c] = aux(p, c);
            o4[c] = c4[c];
        }
    }
    var diff = 0.0;
    for (var c = 0; c < 2; c++) {
        let off = dir << vec2<u32>(u32(c));
        let g = 2.0 * px(img, p, 1u) - px(img, p + off, 1u) - px(img, p - off, 1u);
        c4[h] = g + px(img, p + off, h) + px(img, p - off, h);
        if (d > 1) {
            diff += sq(px(img, p + off, 1u) - px(img, p - off, 1u) - px(img, p + off, h) + px(img, p - off, h))
                + sq(g);
        }
        h ^= 2u;
    }
    c4[3] = diff;
    if (d > 1 && (d & 1) == 1) {
        for (var c = 0u; c < 2u; c++) {
            c4[c * 2u] = select(c4[c * 2u], o4[c * 2u], o4[3] < diff);
        }
    }
    if (d < 2 || (d & 1) == 1) {
        for (var c = 0u; c < 2u; c++) {
            set_px(img, p, c * 2u, c4[c * 2u] / 2.0);
        }
    }
    for (var c = 0u; c < 4u; c++) {
        set_aux(p, c, c4[c]);
    }
}

// Red for blue pixels and vice versa, one direction image per run
// (markesteijn_red_and_blue).
@compute @workgroup_size(16, 16)
fn red_blue(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    let f = 2u - color(p);
    if (!inside(p, PAD_RB_BR) || f == 1u) {
        return;
    }
    let d = run.p.x;
    let img = run.q.x;
    let horiz = mod3y(p) == 0;
    let c = select(vec2(0, 1), vec2(1, 0), horiz);
    let h = select(vec2(3, 0), vec2(0, 3), horiz);
    let g = px(img, p, 1u);
    let pick_c = d > 1 || (d == 0 && horiz) || (d == 1 && !horiz)
        || abs(g - px(img, p + c, 1u)) + abs(g - px(img, p - c, 1u))
            < 2.0 * (abs(g - px(img, p + h, 1u)) + abs(g - px(img, p - h, 1u)));
    let off = select(h, c, pick_c);
    let v = (px(img, p + off, f) + px(img, p - off, f) + 2.0 * g - px(img, p + off, 1u) - px(img, p - off, 1u)) / 2.0;
    set_px(img, p, f, v);
}

// Red and blue for 2 x 2 blocks of green, one direction image per run
// (markesteijn_interpolate_twoxtwo).
@compute @workgroup_size(16, 16)
fn two_by_two(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_G22) || mod3y(p) == 0 || mod3x(p) == 0) {
        return;
    }
    let d = u32(run.p.x);
    let img = run.q.x;
    let a = hexo(p, d);
    let b = hexo(p, d + 1u);
    // The C code tests the tile offsets h + v * TS for zero; with |h|, |v| < 3
    // that is a + b == (0, 0).
    let far = any(a + b != vec2(0));
    let m0 = select(2.0, 3.0, far);
    let m1 = select(-1.0, -2.0, far);
    let m3 = select(1.0, 2.0, far);
    let m4 = select(0.5, 1.0 / 3.0, far);
    let g = m0 * px(img, p, 1u) + m1 * px(img, p + a, 1u) - px(img, p + b, 1u);
    let r = (g + m3 * px(img, p + a, 0u) + px(img, p + b, 0u)) * m4;
    let bl = (g + m3 * px(img, p + a, 2u) + px(img, p + b, 2u)) * m4;
    set_px(img, p, 0u, r);
    set_px(img, p, 2u, bl);
}

fn yuv(d: u32, p: vec2<i32>) -> vec3<f32> {
    let r = px(d, p, 0u);
    let g = px(d, p, 1u);
    let b = px(d, p, 2u);
    let y = 0.2627 * r + 0.6780 * g + 0.0593 * b;
    return vec3(y, (b - y) * 0.56433, (r - y) * 0.67815);
}

fn drv(d: u32, p: vec2<i32>) -> f32 {
    return bitcast<f32>(scratch[d * npix() + pidx(p)]);
}

// Derivatives of all 8 images in their direction, in YPbPr
// (markesteijn_convert_yuv and markesteijn_differentiate).
@compute @workgroup_size(16, 16)
fn derivatives(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_DRV)) {
        return;
    }
    let dirs = array<vec2<i32>, 4>(vec2(1, 0), vec2(0, 1), vec2(1, 1), vec2(-1, 1));
    for (var d = 0u; d < 8u; d++) {
        let o = dirs[d & 3u];
        let v = 2.0 * yuv(d, p) - yuv(d, p + o) - yuv(d, p - o);
        scratch[d * npix() + pidx(p)] = bitcast<u32>(dot(v, v));
    }
}

// Homogeneity maps (markesteijn_homo_threshold and _homo_set).
@compute @workgroup_size(16, 16)
fn homogeneity(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_HOMO)) {
        return;
    }
    var tr = 3.40282347e38;
    for (var d = 0u; d < 8u; d++) {
        tr = min(tr, drv(d, p));
    }
    tr *= 8.0;
    var packed = array<u32, 2>(0u, 0u);
    for (var d = 0u; d < 8u; d++) {
        var n = 0u;
        for (var v = -1; v <= 1; v++) {
            for (var h = -1; h <= 1; h++) {
                n += select(0u, 1u, drv(d, p + vec2(h, v)) <= tr);
            }
        }
        packed[d / 4u] |= n << ((d % 4u) * 8u);
    }
    homo[pidx(p) * 2u] = packed[0];
    homo[pidx(p) * 2u + 1u] = packed[1];
}

fn homo_at(d: u32, p: vec2<i32>) -> u32 {
    return (homo[pidx(p) * 2u + d / 4u] >> ((d % 4u) * 8u)) & 0xFFu;
}

// 5 x 5 sums of the homogeneity maps, into `scratch` (the derivatives are
// done with) (markesteijn_homo_sum).
@compute @workgroup_size(16, 16)
fn homo_sum(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p, PAD_TILE)) {
        return;
    }
    for (var d = 0u; d < 8u; d++) {
        var s = 0u;
        for (var v = -2; v <= 2; v++) {
            for (var h = -2; h <= 2; h++) {
                s += homo_at(d, p + vec2(h, v));
            }
        }
        scratch[d * npix() + pidx(p)] = s;
    }
}

fn cfa_at(p: vec2<i32>) -> f32 {
    return textureLoad(cfa, clamped(p), 0).r;
}

// darktable's C linear fill of the border: the outer 1 px frame as an
// unweighted 3 x 3 same-color mean, the rest with weights 2 across and 1
// diagonally (_vng_lininterpolate).
fn linear(p: vec2<i32>) -> vec3<f32> {
    let g = global(p);
    let size = image_size();
    let f = color(p);
    var sum = vec3<f32>(0.0);
    var cnt = vec3<f32>(0.0);
    let frame = g.x == 0 || g.y == 0 || g.x == size.x - 1 || g.y == size.y - 1;
    for (var v = -1; v <= 1; v++) {
        for (var h = -1; h <= 1; h++) {
            let q = p + vec2(h, v);
            let gq = g + vec2(h, v);
            if (frame && (any(gq < vec2(0)) || any(gq >= size))) {
                continue;
            }
            let c = color(q);
            if (!frame && c == f) {
                continue;
            }
            let w = select(f32(1 << u32(select(0, 1, v == 0) + select(0, 1, h == 0))), 1.0, frame);
            sum += onehot(c) * max(0.0, cfa_at(q)) * w;
            cnt += onehot(c) * w;
        }
    }
    let own = cfa_at(p);
    var out3 = select(sum / max(cnt, vec3(1e-30)), vec3(0.0), cnt == vec3(0.0));
    if (frame) {
        // A color with no neighbor keeps the pixel's own (clamped) value.
        out3 = select(out3, vec3(max(0.0, own)), cnt == vec3(0.0));
    }
    // The pixel's own color is its own value.
    out3 = mix(out3, vec3(select(own, max(0.0, own), frame)), onehot(f));
    return max(out3, vec3(0.0));
}

// The most homogeneous directions averaged (markesteijn_homo_max,
// _homo_max_corr, _homo_quench, _accu and _final), or the linear fill near
// the active area's edge.
@compute @workgroup_size(16, 16)
fn final_rgb(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_tile(p)) {
        return;
    }
    if (!inside(p, PAD_TILE)) {
        textureStore(out, p, vec4(linear(p), 1.0));
        return;
    }
    var hm: array<u32, 8>;
    var maxval = 0u;
    for (var d = 0u; d < 8u; d++) {
        hm[d] = scratch[d * npix() + pidx(p)];
        maxval = max(maxval, hm[d]);
    }
    maxval -= maxval >> 3u;
    for (var d = 0u; d < 4u; d++) {
        let a = hm[d];
        let b = hm[d + 4u];
        if (a < b) {
            hm[d] = 0u;
        } else if (a > b) {
            hm[d + 4u] = 0u;
        }
    }
    var sum = vec3<f32>(0.0);
    var n = 0.0;
    for (var d = 0u; d < 8u; d++) {
        if (hm[d] >= maxval) {
            sum += vec3(px(d, p, 0u), px(d, p, 1u), px(d, p, 2u));
            n += 1.0;
        }
    }
    textureStore(out, p, vec4(max(sum / n, vec3(0.0)), 1.0));
}
