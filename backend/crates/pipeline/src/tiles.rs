//! Splitting an image into tiles.

use genzo_model::frame::Rect;

/// Every tile of a `width` × `height` image, row by row. Edge tiles are
/// smaller than `tile` when the size does not divide evenly.
pub fn grid(width: u32, height: u32, tile: u32) -> Vec<Rect> {
    assert!(tile > 0, "tile size must be positive");
    let mut out = Vec::new();
    let mut y = 0;
    while y < height {
        let h = tile.min(height - y);
        let mut x = 0;
        while x < width {
            let w = tile.min(width - x);
            out.push(Rect {
                x,
                y,
                width: w,
                height: h,
            });
            x += tile;
        }
        y += tile;
    }
    out
}

/// Orders tiles so that those intersecting `visible` come first, then the
/// rest by distance from the center of `visible` (the "ring" around it).
pub fn visible_first(tiles: &mut [Rect], visible: Rect) {
    let cx = visible.x as i64 + visible.width as i64 / 2;
    let cy = visible.y as i64 + visible.height as i64 / 2;
    tiles.sort_by_key(|t| {
        let hit = t.x < visible.x + visible.width
            && visible.x < t.x + t.width
            && t.y < visible.y + visible.height
            && visible.y < t.y + t.height;
        let dx = t.x as i64 + t.width as i64 / 2 - cx;
        let dy = t.y as i64 + t.height as i64 / 2 - cy;
        (!hit, dx * dx + dy * dy)
    });
}

/// One tile of stages 1 to 3 (DEV-01, Tiling), in active-area coordinates
/// unless noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileJob {
    /// The pixels the tile delivers.
    pub out: Rect,
    /// `out` with the apron on each side, clamped to the image: what the
    /// tile computes.
    pub padded: Rect,
    /// In stored-image coordinates: `padded` plus `margin` more on each side,
    /// clamped to the stored image. What is uploaded.
    pub raw: Rect,
}

/// The starts of a row of `tile` px tiles over `len` px. A last tile 1 px
/// wide joins the one before, so every 2 × 2 block of the pyramid's first
/// level, the odd last pixel folded in included, lies in one tile.
fn starts(len: u32, tile: u32) -> Vec<u32> {
    let mut s: Vec<u32> = (0..len).step_by(tile as usize).collect();
    if s.len() > 1 && len - s[s.len() - 1] == 1 {
        s.pop();
    }
    s
}

/// The tiles of a `size` active area that starts at `offset` in a `stored`
/// image, with `tile` px output tiles (even, so tiles start on 2 × 2
/// blocks), an `apron` on each side and a raw `margin` past the apron
/// (FixBadPixelsConstant reads 2 px around). A last row or column of tiles
/// 1 px wide joins the one before.
pub fn jobs(
    size: (u32, u32),
    offset: (u32, u32),
    stored: (u32, u32),
    tile: u32,
    apron: u32,
    margin: u32,
) -> Vec<TileJob> {
    let grow = |r: Rect, by: u32, bounds: Rect| {
        let x = r.x.saturating_sub(by).max(bounds.x);
        let y = r.y.saturating_sub(by).max(bounds.y);
        let right = (r.x + r.width + by).min(bounds.x + bounds.width);
        let bottom = (r.y + r.height + by).min(bounds.y + bounds.height);
        Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    };
    let image = Rect {
        x: 0,
        y: 0,
        width: size.0,
        height: size.1,
    };
    let all = Rect {
        x: 0,
        y: 0,
        width: stored.0,
        height: stored.1,
    };
    assert!(
        tile >= 2 && tile.is_multiple_of(2),
        "tile size {tile} must be even"
    );
    let (xs, ys) = (starts(size.0, tile), starts(size.1, tile));
    let end = |s: &[u32], i: usize, len: u32| s.get(i + 1).copied().unwrap_or(len);
    let mut outs = Vec::with_capacity(xs.len() * ys.len());
    for (j, &y) in ys.iter().enumerate() {
        for (i, &x) in xs.iter().enumerate() {
            outs.push(Rect {
                x,
                y,
                width: end(&xs, i, size.0) - x,
                height: end(&ys, j, size.1) - y,
            });
        }
    }
    outs.into_iter()
        .map(|out| {
            let padded = grow(out, apron, image);
            let in_stored = Rect {
                x: padded.x + offset.0,
                y: padded.y + offset.1,
                ..padded
            };
            TileJob {
                out,
                padded,
                raw: grow(in_stored, margin, all),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_pixel_last_tile_joins_the_one_before() {
        let j = jobs((129, 64), (0, 0), (129, 64), 64, 8, 0);
        assert_eq!(j.len(), 2);
        assert_eq!((j[1].out.x, j[1].out.width), (64, 65));
        // Two pixels left over stay a tile of their own.
        assert_eq!(jobs((130, 64), (0, 0), (130, 64), 64, 8, 0).len(), 3);
    }

    #[test]
    fn jobs_pad_and_clamp() {
        // A 300 x 200 active area at (3, 5) in a 310 x 210 stored image.
        let j = jobs((300, 200), (3, 5), (310, 210), 128, 24, 2);
        assert_eq!(j.len(), 3 * 2);
        let r = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        // The first tile: clamped at the image's top left, apron on the right
        // and bottom; its raw rect reaches 2 px into the masked border.
        assert_eq!(j[0].out, r(0, 0, 128, 128));
        assert_eq!(j[0].padded, r(0, 0, 152, 152));
        assert_eq!(j[0].raw, r(1, 3, 156, 156));
        // An inner one, and the last, smaller one.
        assert_eq!(j[1].padded, r(104, 0, 176, 152));
        assert_eq!(j[5].out, r(256, 128, 44, 72));
        assert_eq!(j[5].padded, r(232, 104, 68, 96));
        // Clamped to the stored image: 3 + 300 + 2 > 310 does not happen
        // here, 5 + 200 + 2 = 207 fits.
        assert_eq!(j[5].raw, r(233, 107, 72, 100));
    }

    #[test]
    fn covers_image_exactly() {
        let tiles = grid(600, 300, 256);
        assert_eq!(tiles.len(), 3 * 2);
        let area: u64 = tiles.iter().map(|t| t.width as u64 * t.height as u64).sum();
        assert_eq!(area, 600 * 300);
        assert_eq!(
            tiles.last().unwrap(),
            &Rect {
                x: 512,
                y: 256,
                width: 88,
                height: 44
            }
        );
    }

    #[test]
    fn empty_image_has_no_tiles() {
        assert!(grid(0, 100, 256).is_empty());
    }

    #[test]
    fn visible_tiles_come_first() {
        let mut tiles = grid(1024, 1024, 256);
        let visible = Rect {
            x: 600,
            y: 600,
            width: 100,
            height: 100,
        };
        visible_first(&mut tiles, visible);
        assert_eq!(
            tiles[0],
            Rect {
                x: 512,
                y: 512,
                width: 256,
                height: 256
            }
        );
        assert_eq!(tiles.len(), 16);
    }
}
