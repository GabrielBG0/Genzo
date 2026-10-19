//! The level-0 tile cache (DEV-01, At 1:1): full-resolution stage 4 output
//! in 256 px tiles, made on demand for what the view needs, kept in an LRU
//! with a fixed byte cap. Tiles live in one RGBA16F atlas of slots; a table
//! maps each cache tile of the image to its slot, and the sampler reads the
//! atlas through it. A Fit view larger than the pyramid's level 1 reads
//! level 0 too (decided 2026-10-19).

use std::collections::HashMap;

use genzo_model::frame::Rect;

use super::raw_stages::{Level0Target, RawStages};
use super::{Gpu, GpuError};
use crate::budget;
use crate::camera_color::CameraTransform;
use crate::raw::RawSource;

/// A cache tile's side, in level-0 pixels.
pub const TILE: u32 = 256;
/// The table's entry for a cache tile without a slot.
pub const NO_SLOT: u32 = u32::MAX;

#[derive(Debug, thiserror::Error)]
pub enum Level0Error {
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// More tiles at once than the atlas holds: the view needs `needed`, the
    /// atlas has `slots`. The caller falls back to the pyramid.
    #[error("{needed} level-0 tiles needed at once, {slots} slots")]
    Full { needed: usize, slots: usize },
}

/// The cache. Only the render thread holds it.
#[derive(Debug)]
pub struct Level0Cache {
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    slots_across: u32,
    slot_count: u32,
    /// The current image's table, `tiles.0 × tiles.1` entries.
    table: wgpu::Buffer,
    tiles: (u32, u32),
    /// The level-0 (active-area) size of the image the tiles are of.
    size: (u32, u32),
    /// The inputs the tiles were rendered with (the caller's key: photo and
    /// every input of stages 1 to 4). Another key empties the cache.
    key: Option<u64>,
    owner: Vec<Option<(u32, u32)>>,
    slot_of: HashMap<(u32, u32), u32>,
    used: Vec<u64>,
    clock: u64,
}

/// The slots an atlas gets: `TILE` px each, at most `budget::LEVEL0_CACHE`
/// bytes (RGBA16F), within the device's largest 2D texture. With a 7 %
/// driver padding (NVIDIA, measured 2026-10-18) that still fits the cap, so
/// the cap is divided by 1.07 first.
pub fn slots_for(max_texture: u32) -> (u32, u32) {
    let per_side = (max_texture / TILE).max(1);
    let slot_bytes = u64::from(TILE * TILE) * 8;
    let total = (budget::LEVEL0_CACHE as f64 / 1.07) as u64 / slot_bytes;
    let across = per_side.min(32).min(total as u32);
    let down = (total as u32 / across).min(per_side);
    (across, down)
}

impl Level0Cache {
    /// A cache sized to DEV-01's cap on this device.
    pub fn new(gpu: &Gpu) -> Self {
        let slots = slots_for(gpu.device.limits().max_texture_dimension_2d);
        Self::with_slots(gpu, slots)
    }

    /// A cache of `slots` (across, down) slots.
    pub fn with_slots(gpu: &Gpu, (across, down): (u32, u32)) -> Self {
        let atlas = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("level-0 atlas"),
            size: wgpu::Extent3d {
                width: across * TILE,
                height: down * TILE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let count = across * down;
        Self {
            atlas,
            atlas_view,
            slots_across: across,
            slot_count: count,
            table: table(gpu, 1),
            tiles: (0, 0),
            size: (0, 0),
            key: None,
            owner: vec![None; count as usize],
            slot_of: HashMap::new(),
            used: vec![0; count as usize],
            clock: 0,
        }
    }

    /// The atlas's bytes (RGBA16F).
    pub fn bytes(&self) -> u64 {
        u64::from(self.atlas.width()) * u64::from(self.atlas.height()) * 8
    }

    pub fn slots(&self) -> u32 {
        self.slot_count
    }

    /// How many tiles are held.
    pub fn held(&self) -> usize {
        self.slot_of.len()
    }

    /// The cache tiles covering `rect` (level-0 pixels), clamped to an
    /// image of `size`.
    pub fn tiles_for(rect: Rect, size: (u32, u32)) -> Vec<(u32, u32)> {
        let right = (rect.x + rect.width).min(size.0);
        let bottom = (rect.y + rect.height).min(size.1);
        if rect.x >= right || rect.y >= bottom {
            return Vec::new();
        }
        let mut out = vec![];
        for ty in rect.y / TILE..bottom.div_ceil(TILE) {
            for tx in rect.x / TILE..right.div_ceil(TILE) {
                out.push((tx, ty));
            }
        }
        out
    }

    /// The ring of one tile around `tiles`, inside an image of `size`: what
    /// is rendered after the visible tiles, so a small pan finds its tiles.
    pub fn ring(tiles: &[(u32, u32)], size: (u32, u32)) -> Vec<(u32, u32)> {
        let grid = (size.0.div_ceil(TILE), size.1.div_ceil(TILE));
        let mut out = vec![];
        for &(tx, ty) in tiles {
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (x, y) = (i64::from(tx) + dx, i64::from(ty) + dy);
                    if x < 0 || y < 0 || x >= i64::from(grid.0) || y >= i64::from(grid.1) {
                        continue;
                    }
                    let t = (x as u32, y as u32);
                    if !tiles.contains(&t) && !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
        }
        out
    }

    fn reset(&mut self, gpu: &Gpu, size: (u32, u32), key: u64) {
        self.tiles = (size.0.div_ceil(TILE), size.1.div_ceil(TILE));
        self.size = size;
        self.key = Some(key);
        self.table = table(gpu, (self.tiles.0 * self.tiles.1) as usize);
        self.owner.iter_mut().for_each(|o| *o = None);
        self.slot_of.clear();
    }

    /// Makes `want` (cache tiles of `src`) present, rendering the missing
    /// ones through `stages` with `transform`. `key` names the photo and
    /// every input of stages 1 to 4; a new one empties the cache. The tiles
    /// in `want` are never evicted to make room for each other.
    pub fn ensure(
        &mut self,
        gpu: &Gpu,
        stages: &mut RawStages,
        src: &RawSource,
        transform: &CameraTransform,
        key: u64,
        want: &[(u32, u32)],
    ) -> Result<(), Level0Error> {
        let size = src.size();
        if self.key != Some(key) || self.size != size {
            self.reset(gpu, size, key);
        }
        self.clock += 1;
        let missing: Vec<(u32, u32)> = want
            .iter()
            .copied()
            .filter(|t| !self.slot_of.contains_key(t))
            .collect();
        if want.len() > self.slot_count as usize {
            return Err(Level0Error::Full {
                needed: want.len(),
                slots: self.slot_count as usize,
            });
        }
        for t in want {
            if let Some(&s) = self.slot_of.get(t) {
                self.used[s as usize] = self.clock;
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        for &t in &missing {
            // A free slot, else the least recently used one not wanted now.
            let slot = (0..self.slot_count)
                .filter(|&s| self.used[s as usize] < self.clock || self.owner[s as usize].is_none())
                .min_by_key(|&s| (self.owner[s as usize].is_some(), self.used[s as usize]))
                .expect("fewer wanted tiles than slots");
            if let Some(old) = self.owner[slot as usize].take() {
                self.slot_of.remove(&old);
            }
            self.owner[slot as usize] = Some(t);
            self.slot_of.insert(t, slot);
            self.used[slot as usize] = self.clock;
        }
        let mut entries = vec![NO_SLOT; (self.tiles.0 * self.tiles.1) as usize];
        for (&(tx, ty), &slot) in &self.slot_of {
            entries[(ty * self.tiles.0 + tx) as usize] = slot;
        }
        gpu.queue
            .write_buffer(&self.table, 0, bytemuck::cast_slice(&entries));
        // One region over the missing tiles: rendered as the whole image
        // would be, so cached tiles inside it are rewritten with the same
        // values.
        let (x0, y0) = missing
            .iter()
            .fold((u32::MAX, u32::MAX), |(a, b), t| (a.min(t.0), b.min(t.1)));
        let (x1, y1) = missing
            .iter()
            .fold((0, 0), |(a, b), t| (a.max(t.0 + 1), b.max(t.1 + 1)));
        let region = Rect {
            x: x0 * TILE,
            y: y0 * TILE,
            width: (x1 * TILE).min(size.0) - x0 * TILE,
            height: (y1 * TILE).min(size.1) - y0 * TILE,
        };
        let target = Level0Target {
            atlas: &self.atlas_view,
            slots: &self.table,
            tiles_across: self.tiles.0,
            slots_across: self.slots_across,
        };
        if let Err(e) = stages.render_level0(gpu, src, transform, region, &target) {
            // The slots just given to `missing` hold no valid pixels: forget
            // everything, so the next call starts over and renders again.
            self.key = None;
            return Err(e.into());
        }
        Ok(())
    }

    pub(crate) fn atlas_view(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    pub(crate) fn table(&self) -> &wgpu::Buffer {
        &self.table
    }

    /// Cache tiles across the image, slots across the atlas.
    pub(crate) fn grid(&self) -> (u32, u32) {
        (self.tiles.0, self.slots_across)
    }

    /// The level-0 size the tiles are of.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Reads a held tile back, RGB row by row (`TILE` × `TILE`, the image's
    /// edge tiles included at full size): for tests.
    pub fn read_tile(&self, gpu: &Gpu, t: (u32, u32)) -> Result<Option<Vec<[f32; 3]>>, GpuError> {
        let Some(&slot) = self.slot_of.get(&t) else {
            return Ok(None);
        };
        let origin = (
            (slot % self.slots_across) * TILE,
            (slot / self.slots_across) * TILE,
        );
        super::view::read_rgba16f(gpu, &self.atlas, origin, (TILE, TILE)).map(Some)
    }
}

fn table(gpu: &Gpu, entries: usize) -> wgpu::Buffer {
    let b = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("level-0 table"),
        size: (entries.max(1) * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue
        .write_buffer(&b, 0, bytemuck::cast_slice(&vec![NO_SLOT; entries.max(1)]));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_fits_the_cap_with_padding() {
        let (a, d) = slots_for(8192);
        assert_eq!((a, d), (32, 14));
        let bytes = u64::from(a * d) * u64::from(TILE * TILE) * 8;
        assert!(
            bytes as f64 * 1.07 <= budget::LEVEL0_CACHE as f64,
            "{bytes}"
        );
        // A device with smaller textures gets fewer slots, never a larger
        // texture.
        let (a, d) = slots_for(4096);
        assert!(a * TILE <= 4096 && d * TILE <= 4096);
    }

    #[test]
    fn tiles_and_their_ring() {
        let size = (1000, 600);
        let t = Level0Cache::tiles_for(
            Rect {
                x: 250,
                y: 0,
                width: 20,
                height: 300,
            },
            size,
        );
        assert_eq!(t, [(0, 0), (1, 0), (0, 1), (1, 1)]);
        let ring = Level0Cache::ring(&t, size);
        assert_eq!(ring.len(), 5, "{ring:?}");
        assert!(ring.iter().all(|r| !t.contains(r) && r.0 <= 3 && r.1 <= 2));
    }
}
