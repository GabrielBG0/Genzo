//! A synthetic RGBA8 frame for the M0 latency spike. Cheap to compute, and it
//! visibly changes with `phase` so a dropped or stale frame is easy to spot.

use genzo_model::frame::Rect;

/// Renders the pixels of `rect` from a `width` × `height` test pattern.
pub fn render_tile(width: u32, height: u32, rect: Rect, phase: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
    let shift = (phase.clamp(0.0, 1.0) * 255.0) as u32;
    for y in rect.y..rect.y + rect.height {
        let g = (y * 255 / height.max(1)) as u8;
        for x in rect.x..rect.x + rect.width {
            let r = ((x * 255 / width.max(1) + shift) % 256) as u8;
            let checker = ((x / 64) + (y / 64)) % 2 == 0;
            let b = if checker { 200 } else { 60 };
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_size_and_phase() {
        let rect = Rect {
            x: 10,
            y: 20,
            width: 4,
            height: 3,
        };
        let a = render_tile(100, 100, rect, 0.0);
        let b = render_tile(100, 100, rect, 0.5);
        assert_eq!(a.len(), 4 * 3 * 4);
        assert_ne!(a, b);
        assert!(a.chunks(4).all(|px| px[3] == 255));
    }
}
