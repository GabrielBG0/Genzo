//! GPU memory for one 48 MP edit (DEV-01, Tiling): the cap of each item, as
//! the spec's table states it, in bytes (MB = 10^6). The memory test holds
//! what exists to these, as the driver allocates it: NVIDIA's Vulkan driver
//! pads textures by up to 7 % (measured 2026-10-18), so the caps leave room
//! for that. The rest are fixed caps of later steps.

const MB: u64 = 1_000_000;

/// The pyramid, levels 1 and up, RGBA16F: 98 MB for level 1, plus a third
/// (130 MB; 132.3 MB as NVIDIA's driver allocates it).
pub const PYRAMID: u64 = 133 * MB;
/// Stages 1 to 4 scratch for one tile (60 MB for Bayer; 64.2 MB on NVIDIA).
pub const TILE_SCRATCH: u64 = 65 * MB;
/// Upload staging for one tile (linear DNG, 3 × u16: 6.9 MB; 7.5 MB on
/// NVIDIA).
pub const UPLOAD: u64 = 8 * MB;
/// The level-0 tile cache at 1:1 (plan step 6).
pub const LEVEL0_CACHE: u64 = 256 * MB;
/// The viewport stage caches (steps 6 and 7).
pub const VIEWPORT_CACHES: u64 = 128 * MB;
/// The output frame and two persistent readback buffers, RGBA8.
pub const OUTPUT: u64 = 60 * MB;
/// Histogram and clipping masks.
pub const HISTOGRAM: u64 = 10 * MB;

/// The whole budget's items.
pub const ITEMS: [u64; 7] = [
    PYRAMID,
    TILE_SCRATCH,
    UPLOAD,
    LEVEL0_CACHE,
    VIEWPORT_CACHES,
    OUTPUT,
    HISTOGRAM,
];

/// The budget (DEV-01: under 1 GB).
pub const LIMIT: u64 = 1_000 * MB;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_items_add_up_to_the_spec_total() {
        let total: u64 = ITEMS.iter().sum();
        assert_eq!(total, 660 * MB);
        assert!(total < LIMIT);
    }
}
