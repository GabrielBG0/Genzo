// Stages 1 and 2 for one tile (DEV-01), as reference::stages computes them:
// FixBadPixelsConstant on the stored values, the levels, the GainMaps of
// OpcodeList2, then white balance and the highlight clip. A Bayer tile goes
// to `cfa_out`, clamped at 0 for the demosaic; a linear tile to `rgb_out`,
// negatives kept, a monochrome plane copied to all three channels.

struct Map {
    // The rows and columns it covers in the active area, already
    // intersected with it: top, left, bottom, right.
    area: vec4<u32>,
    // First plane, end plane (exclusive), row pitch, column pitch.
    pitch: vec4<u32>,
    // Points down and across, map planes, the offset of its gains.
    points: vec4<u32>,
    // Spacing down and across, origin down and across.
    geo: vec4<f32>,
}

@group(0) @binding(1) var raw: texture_2d<u32>;
@group(0) @binding(2) var<storage, read> black: array<f32>;
@group(0) @binding(3) var<storage, read> maps: array<Map>;
@group(0) @binding(4) var<storage, read> gains: array<f32>;
@group(0) @binding(5) var cfa_out: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var rgb_out: texture_storage_2d<rgba32float, write>;

// The stored sample at stored `s`, plane `plane`; `s` must lie in the raw rect.
fn raw_at(s: vec2<i32>, plane: u32) -> u32 {
    let r = vec2<i32>(tile.raw.xy);
    let x = (s.x - r.x) * i32(tile.info.x) + i32(plane);
    return textureLoad(raw, vec2<i32>(x, s.y - r.y), 0).r;
}

fn in_raw(s: vec2<i32>) -> bool {
    let r = vec2<i32>(tile.raw.xy);
    return all(s >= r) && all(s < r + vec2<i32>(tile.raw.zw));
}

// FixBadPixelsConstant: the rounded mean of the same-color neighbors that
// are not bad. The raw rect reaches 2 px past the tile, or to the stored
// image's edge, so a neighbor outside it is outside the image.
fn fixed(s: vec2<i32>, v: u32) -> u32 {
    let bad = tile.levels.x;
    if (tile.info.z == 0u || v != bad) {
        return v;
    }
    let phase = i32(tile.info.w + (tile.info.w >> 1u));
    let green = ((s.y + s.x + phase) & 1) == 0;
    var count = 0u;
    var total = 0u;
    for (var k = 0; k < 4; k++) {
        var d: vec2<i32>;
        if (green) {
            d = vec2<i32>(select(-1, 1, (k & 1) == 1), select(-1, 1, k >= 2));
        } else {
            d = select(vec2<i32>(0, select(-2, 2, k == 3)), vec2<i32>(select(-2, 2, k == 1), 0), k < 2);
        }
        let q = s + d;
        if (in_raw(q)) {
            let n = raw_at(q, 0u);
            if (n != bad) {
                count += 1u;
                total += n;
            }
        }
    }
    if (count == 4u) {
        return (total + 2u) >> 2u;
    }
    if (count > 0u) {
        return (total + (count >> 1u)) / count;
    }
    return v;
}

// The SDK's interpolation index along one axis for a pixel's center: first
// point, second point, fraction.
fn map_index(pixel: u32, size: u32, origin: f32, spacing: f32, points: u32) -> vec3<f32> {
    if (points == 1u) {
        return vec3<f32>(0.0);
    }
    let f = ((f32(pixel) + 0.5) / f32(size) - origin) / spacing;
    if (f <= 0.0) {
        return vec3<f32>(0.0);
    }
    let i = u32(f);
    if (i >= points - 1u) {
        return vec3<f32>(f32(points - 1u), f32(points - 1u), 0.0);
    }
    return vec3<f32>(f32(i), f32(i + 1u), f - f32(i));
}

fn gain_at(m: Map, v: u32, h: u32, plane: u32) -> f32 {
    return gains[m.points.w + (v * m.points.y + h) * m.points.z + plane];
}

// Every GainMap that covers global `g`, plane `plane`, in file order.
fn apply_maps(x0: f32, g: vec2<u32>, plane: u32) -> f32 {
    var x = x0;
    for (var k = 0u; k < tile.info.y; k++) {
        let m = maps[k];
        if (plane < m.pitch.x || plane >= m.pitch.y) {
            continue;
        }
        if (g.y < m.area.x || g.y >= m.area.z || g.x < m.area.y || g.x >= m.area.w) {
            continue;
        }
        if ((g.y - m.area.x) % m.pitch.z != 0u || (g.x - m.area.y) % m.pitch.w != 0u) {
            continue;
        }
        let mp = min(plane, m.points.z - 1u);
        let iv = map_index(g.y, tile.size.w, m.geo.z, m.geo.x, m.points.x);
        let ih = map_index(g.x, tile.size.z, m.geo.w, m.geo.y, m.points.y);
        let v1 = u32(iv.x);
        let v2 = u32(iv.y);
        let h1 = u32(ih.x);
        let h2 = u32(ih.y);
        let top = gain_at(m, v1, h1, mp) * (1.0 - ih.z) + gain_at(m, v1, h2, mp) * ih.z;
        let bottom = gain_at(m, v2, h1, mp) * (1.0 - ih.z) + gain_at(m, v2, h2, mp) * ih.z;
        x = min(x * (top * (1.0 - iv.z) + bottom * iv.z), 1.0);
    }
    return x;
}

// The scaled black level at global `g`, plane `plane`.
fn black_at(g: vec2<u32>, plane: u32) -> f32 {
    let planes = tile.info.x;
    let site = (g.y % tile.levels.y) * tile.levels.z + g.x % tile.levels.z;
    var b = black[site * planes + plane];
    if ((tile.levels.w & 1u) != 0u) {
        b += black[tile.deltas.x + plane * tile.size.z + g.x];
    }
    if ((tile.levels.w & 2u) != 0u) {
        b += black[tile.deltas.y + plane * tile.size.w + g.y];
    }
    return b;
}

fn scale_of(plane: u32) -> f32 {
    return select(select(tile.scale.x, tile.scale.y, plane == 1u), tile.scale.z, plane == 2u);
}

// Stage 1 for one plane.
fn linear(p: vec2<i32>, plane: u32) -> f32 {
    let s = stored(p);
    var v = raw_at(s, plane);
    if (plane == 0u && tile.info.x == 1u) {
        v = fixed(s, v);
    }
    let g = vec2<u32>(global(p));
    let x = f32(v) * scale_of(plane) - black_at(g, plane);
    return apply_maps(x, g, plane);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_tile(p)) {
        return;
    }
    let clip = tile.wb.w;
    if (tile.info.x == 3u) {
        let rgb = vec3<f32>(linear(p, 0u), linear(p, 1u), linear(p, 2u));
        textureStore(rgb_out, p, vec4<f32>(min(rgb * tile.wb.xyz, vec3<f32>(clip)), 1.0));
    } else if (tile.cfa.x == 0u) {
        // A monochrome plane: no white balance.
        let v = min(linear(p, 0u), 1.0);
        textureStore(rgb_out, p, vec4<f32>(v, v, v, 1.0));
    } else {
        let v = min(linear(p, 0u) * pick3(tile.wb.xyz, fc(p)), clip);
        textureStore(cfa_out, p, vec4<f32>(max(v, 0.0), 0.0, 0.0, 0.0));
    }
}
