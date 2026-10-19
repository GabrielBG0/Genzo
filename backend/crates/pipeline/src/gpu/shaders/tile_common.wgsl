// Shared by the tiled passes of stages 1 to 3 (DEV-01, Tiling). Prepended
// to each pass's source.
//
// A tile's textures cover its padded rect: the output tile plus the apron,
// clamped to the active area. "Local" coordinates are within that rect,
// "global" ones within the active area, "stored" ones within the stored
// image (the active area starts at `origin.zw` there). Reads clamp to the
// local rect; everything that depends on where a pixel is in the image (the
// CFA color, RCD's border) uses global or stored coordinates, so a tile
// computes exactly what the whole image would, away from its apron's edge.

struct Tile {
    // The padded rect's width and height; the active area's width and height.
    size: vec4<u32>,
    // The padded rect's top left in the active area; the active area's left
    // and top in the stored image.
    origin: vec4<u32>,
    // The uploaded raw rect in the stored image: x, y, width, height.
    raw: vec4<u32>,
    // Samples per pixel, GainMap count, 1 if FixBadPixelsConstant applies,
    // its Bayer phase.
    info: vec4<u32>,
    // FixBadPixelsConstant's constant; the black pattern's rows and columns;
    // bit 0: a DeltaH table, bit 1: a DeltaV table.
    levels: vec4<u32>,
    // The Bayer colors (0 red, 1 green, 2 blue) at stored (row, col) (0, 0),
    // (0, 1), (1, 0), (1, 1).
    bayer: vec4<u32>,
    // White balance multipliers for red, green and blue; the clip level.
    wb: vec4<f32>,
    // 1 / (white - largest black), per plane.
    scale: vec4<f32>,
    // Offsets of the DeltaH and DeltaV tables in `black`, per plane: the
    // pattern starts at 0.
    deltas: vec4<u32>,
    // The mosaic: 0 none (linear input), 1 Bayer (`bayer`), 2 X-Trans.
    cfa: vec4<u32>,
    // The X-Trans pattern from the stored origin, row by row, four colors
    // per vec4.
    xtrans: array<vec4<u32>, 9>,
}

@group(0) @binding(0) var<uniform> tile: Tile;

const EPS: f32 = 1e-5;
const EPSSQ: f32 = 1e-10;

// Shaders avoid indexing vectors with runtime values: the FXC compiler used
// by D3D12 on WARP rejects it. `pick3`, `pick4` and `onehot` stand in for it.
fn pick3(v: vec3<f32>, i: u32) -> f32 {
    return select(select(v.x, v.y, i == 1u), v.z, i == 2u);
}

fn pick4u(v: vec4<u32>, i: u32) -> u32 {
    return select(select(select(v.x, v.y, i == 1u), v.z, i == 2u), v.w, i == 3u);
}

fn onehot(i: u32) -> vec3<f32> {
    return vec3<f32>(f32(i == 0u), f32(i == 1u), f32(i == 2u));
}

fn local_size() -> vec2<i32> {
    return vec2<i32>(tile.size.xy);
}

fn image_size() -> vec2<i32> {
    return vec2<i32>(tile.size.zw);
}

// Local to active-area coordinates.
fn global(p: vec2<i32>) -> vec2<i32> {
    return p + vec2<i32>(tile.origin.xy);
}

// Local to stored-image coordinates.
fn stored(p: vec2<i32>) -> vec2<i32> {
    return global(p) + vec2<i32>(tile.origin.zw);
}

fn in_tile(p: vec2<i32>) -> bool {
    return all(p >= vec2<i32>(0)) && all(p < local_size());
}

// Whether local `p` lies inside the active area.
fn in_image(p: vec2<i32>) -> bool {
    let g = global(p);
    return all(g >= vec2<i32>(0)) && all(g < image_size());
}

// The CFA color at local `p`, from the stored image's phase.
fn fc(p: vec2<i32>) -> u32 {
    let s = stored(p);
    if (tile.cfa.x == 2u) {
        let i = u32((s.y % 6) * 6 + s.x % 6);
        return pick4u(tile.xtrans[i / 4u], i % 4u);
    }
    return pick4u(tile.bayer, u32((s.y & 1) * 2 + (s.x & 1)));
}

// Loads one channel with the coordinates clamped to the tile.
fn ld(t: texture_2d<f32>, p: vec2<i32>) -> f32 {
    return textureLoad(t, clamp(p, vec2<i32>(0), local_size() - 1), 0).r;
}

fn ld2(t: texture_2d<f32>, p: vec2<i32>) -> vec2<f32> {
    return textureLoad(t, clamp(p, vec2<i32>(0), local_size() - 1), 0).rg;
}

fn sq(x: f32) -> f32 {
    return x * x;
}
