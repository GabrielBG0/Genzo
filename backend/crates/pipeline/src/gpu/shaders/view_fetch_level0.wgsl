// A texel of level 0, for view_sample.wgsl: its 256 px cache tile's slot in
// the atlas, through the table (gpu::level0). A tile without a slot reads 0;
// the view never asks for one.

const CACHE_TILE: u32 = 256u;
const NO_SLOT: u32 = 0xffffffffu;

@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> slots: array<u32>;

fn fetch(p: vec2<i32>) -> vec3<f32> {
    let q = vec2<u32>(p);
    let t = q / CACHE_TILE;
    let slot = slots[t.y * view.grid.x + t.x];
    if (slot == NO_SLOT) {
        return vec3<f32>(0.0);
    }
    let origin = vec2<u32>(slot % view.grid.y, slot / view.grid.y) * CACHE_TILE;
    return textureLoad(atlas, vec2<i32>(origin + q % CACHE_TILE), 0).rgb;
}
