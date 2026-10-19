// Stage 4 at full resolution into the level-0 tile cache (DEV-01, At 1:1):
// each pixel of the tile's output goes through stage4() into its 256 px
// cache tile's slot in the atlas, found through the table; pixels whose
// cache tile has no slot are skipped.

const CACHE_TILE: u32 = 256u;
const NO_SLOT: u32 = 0xffffffffu;

@group(0) @binding(3) var atlas: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<storage, read> slots: array<u32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= s4.texels.z || id.y >= s4.texels.w) {
        return;
    }
    let g = s4.texels.xy + id.xy;
    let t = g / CACHE_TILE;
    let slot = slots[t.y * s4.level.x + t.x];
    if (slot == NO_SLOT) {
        return;
    }
    let origin = vec2<u32>(slot % s4.level.y, slot / s4.level.y) * CACHE_TILE;
    let c = textureLoad(rgb, vec2<i32>(g) - vec2<i32>(tile.origin.xy), 0).rgb;
    textureStore(atlas, vec2<i32>(origin + g % CACHE_TILE), vec4<f32>(stage4(c), 1.0));
}
