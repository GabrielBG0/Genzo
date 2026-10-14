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

#[cfg(test)]
mod tests {
    use super::*;

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
