//! Readback of the frame (DEV-01, Readback): only what is sent, into two
//! staging buffers made with the frame and kept for as long as its size.
//! At 1:1 that is the visible 256 px tiles of the shown image, at Fit the
//! whole frame. wgpu cannot copy into a mapped buffer, so a buffer is
//! mapped for the read and unmapped before the next copy into it.
//!
//! Each rectangle is one texture-to-buffer copy, its rows padded to the
//! 256 bytes a copy's row needs and its start at a multiple of 512 bytes:
//! D3D12 without unrestricted copy pitches (the windows-2022 runner's WARP)
//! needs that, and wgpu would otherwise copy through an intermediate buffer
//! of its own. The rectangles go into the first buffer and spill into the
//! second; a request larger than both takes another round.

use std::sync::mpsc;

use genzo_model::frame::{Rect, TILE_SIZE};

use super::raw_stages::padded_row;
use super::{Gpu, GpuError};

/// A tile a 1:1 view sends: where it is in the shown image (the tile
/// protocol's x, y, width and height, at mip level 0) and the same pixels
/// in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTile {
    pub image: Rect,
    pub frame: Rect,
}

/// The 256 px tiles of a `shown`-sized image that an `out`-sized 1:1 view
/// with whole-pixel `offset` (its top left in the shown image,
/// [`crate::reference::sampling::ViewSpec::anchored`]) shows, row by row,
/// each clipped to the view and to the image. A view past the image's
/// edge shows only the tiles inside it.
pub fn visible_tiles(offset: (i32, i32), out: (u32, u32), shown: (u32, u32)) -> Vec<ViewTile> {
    let span = |o: i32, len: u32, size: u32| {
        let start = i64::from(o).max(0);
        let end = (i64::from(o) + i64::from(len)).min(i64::from(size));
        (start < end).then_some((start, end))
    };
    let (Some((x0, x1)), Some((y0, y1))) = (
        span(offset.0, out.0, shown.0),
        span(offset.1, out.1, shown.1),
    ) else {
        return Vec::new();
    };
    let t = i64::from(TILE_SIZE);
    let mut tiles = Vec::new();
    for ty in y0 / t..=(y1 - 1) / t {
        let (top, bottom) = ((ty * t).max(y0), (ty * t + t).min(y1));
        for tx in x0 / t..=(x1 - 1) / t {
            let (left, right) = ((tx * t).max(x0), (tx * t + t).min(x1));
            let rect = |x: i64, y: i64| Rect {
                x: x as u32,
                y: y as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            };
            tiles.push(ViewTile {
                image: rect(left, top),
                frame: rect(left - i64::from(offset.0), top - i64::from(offset.1)),
            });
        }
    }
    tiles
}

/// The two staging buffers of a frame.
#[derive(Debug)]
pub(crate) struct Staging {
    buffers: [wgpu::Buffer; 2],
}

/// Where a copy may start in a buffer (D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT).
const PLACEMENT: u64 = 512;

/// Bytes of `rect`'s copy: its rows, padded.
fn copy_bytes(rect: &Rect) -> u64 {
    u64::from(padded_row(rect.width, 4)) * u64::from(rect.height)
}

/// Where the first of `rects` that fit go, in two buffers of `capacity`
/// bytes each: (buffer, offset) per rectangle placed, in order.
fn place(rects: &[Rect], capacity: u64) -> Vec<(usize, u64)> {
    let mut placed = Vec::new();
    let (mut buffer, mut offset) = (0, 0u64);
    for rect in rects {
        let bytes = copy_bytes(rect);
        assert!(bytes <= capacity, "{rect:?} is larger than a buffer");
        offset = offset.next_multiple_of(PLACEMENT);
        if offset + bytes > capacity {
            if buffer == 1 {
                break;
            }
            (buffer, offset) = (1, 0);
        }
        placed.push((buffer, offset));
        offset += bytes;
    }
    placed
}

impl Staging {
    /// Buffers for a `size` frame (RGBA8), each holding all of it.
    pub(crate) fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        let bytes = copy_bytes(&Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        });
        let buffer = |label| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            buffers: [buffer("frame staging 1"), buffer("frame staging 2")],
        }
    }

    /// The bytes of both buffers.
    pub(crate) fn bytes(&self) -> u64 {
        self.buffers.iter().map(wgpu::Buffer::size).sum()
    }

    /// `rects` of `frame` (an R32Uint texture of packed RGBA8), each as its
    /// pixels row by row, 4 bytes each.
    pub(crate) fn read(
        &self,
        gpu: &Gpu,
        frame: &wgpu::Texture,
        rects: &[Rect],
    ) -> Result<Vec<Vec<u8>>, GpuError> {
        let mut out = Vec::with_capacity(rects.len());
        let mut rest = rects;
        while !rest.is_empty() {
            for rect in rest {
                assert!(
                    rect.x + rect.width <= frame.width() && rect.y + rect.height <= frame.height(),
                    "{rect:?} is outside the frame"
                );
            }
            let placed = place(rest, self.buffers[0].size());
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            for (rect, &(b, at)) in rest.iter().zip(&placed) {
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: frame,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: rect.x,
                            y: rect.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &self.buffers[b],
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: at,
                            bytes_per_row: Some(padded_row(rect.width, 4)),
                            rows_per_image: Some(rect.height),
                        },
                    },
                    wgpu::Extent3d {
                        width: rect.width,
                        height: rect.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            gpu.queue.submit([encoder.finish()]);
            let used = placed.last().map_or(0, |&(b, _)| b + 1);
            let (tx, rx) = mpsc::channel();
            for b in 0..used {
                let tx = tx.clone();
                self.buffers[b].map_async(wgpu::MapMode::Read, .., move |r| {
                    let _ = tx.send(r);
                });
            }
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            for _ in 0..used {
                rx.recv()
                    .map_err(|e| GpuError::Readback(e.to_string()))?
                    .map_err(|e| GpuError::Readback(e.to_string()))?;
            }
            let result = (|| {
                let mapped = (0..used)
                    .map(|b| self.buffers[b].get_mapped_range(..))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| GpuError::Readback(e.to_string()))?;
                for (rect, &(b, at)) in rest.iter().zip(&placed) {
                    let row = padded_row(rect.width, 4) as usize;
                    let line = rect.width as usize * 4;
                    let mut pixels = Vec::with_capacity(line * rect.height as usize);
                    for y in 0..rect.height as usize {
                        let start = at as usize + y * row;
                        pixels.extend_from_slice(&mapped[b][start..start + line]);
                    }
                    out.push(pixels);
                }
                Ok(())
            })();
            for b in 0..used {
                self.buffers[b].unmap();
            }
            result?;
            rest = &rest[placed.len()..];
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn copies_start_on_the_placement_alignment_and_spill() {
        // Partial tiles: rows of 512 bytes, 7 rows, so 3584 bytes each.
        let rects = vec![rect(0, 0, 100, 7); 5];
        let capacity = 8192;
        let placed = place(&rects, capacity);
        assert_eq!(placed, [(0, 0), (0, 3584), (1, 0), (1, 3584)]);
        // 3584 is a multiple of 512; an odd size is not, and moves the next.
        let odd = [rect(0, 0, 10, 3), rect(0, 0, 10, 3)];
        assert_eq!(place(&odd, capacity), [(0, 0), (0, 1024)]);
        for (_, at) in place(&odd, capacity) {
            assert_eq!(at % PLACEMENT, 0);
        }
    }

    #[test]
    fn a_view_on_the_grid_shows_whole_tiles() {
        let tiles = visible_tiles((256, 512), (512, 256), (2000, 1000));
        let image: Vec<Rect> = tiles.iter().map(|t| t.image).collect();
        assert_eq!(image, [rect(256, 512, 256, 256), rect(512, 512, 256, 256)]);
        assert_eq!(tiles[1].frame, rect(256, 0, 256, 256));
    }

    #[test]
    fn a_panned_view_clips_its_edge_tiles() {
        // 300..900 across: 300..512, 512..768, 768..900; 10..266 down:
        // 10..256, 256..266.
        let tiles = visible_tiles((300, 10), (600, 256), (2000, 1000));
        assert_eq!(tiles.len(), 6);
        assert_eq!(tiles[0].image, rect(300, 10, 212, 246));
        assert_eq!(tiles[0].frame, rect(0, 0, 212, 246));
        assert_eq!(tiles[2].image, rect(768, 10, 132, 246));
        assert_eq!(tiles[5].image, rect(768, 256, 132, 10));
        assert_eq!(tiles[5].frame, rect(468, 246, 132, 10));
        // The tiles cover the view once.
        let area: u32 = tiles.iter().map(|t| t.frame.width * t.frame.height).sum();
        assert_eq!(area, 600 * 256);
    }

    #[test]
    fn a_view_past_the_image_shows_only_the_image() {
        let tiles = visible_tiles((-100, 900), (400, 400), (1000, 1000));
        let frame: Vec<Rect> = tiles.iter().map(|t| t.frame).collect();
        assert_eq!(frame, [rect(100, 0, 256, 100), rect(356, 0, 44, 100)]);
        assert_eq!(tiles[1].image, rect(256, 900, 44, 100));
        assert!(visible_tiles((1000, 0), (100, 100), (1000, 1000)).is_empty());
        assert!(visible_tiles((-100, 0), (100, 100), (1000, 1000)).is_empty());
    }
}
