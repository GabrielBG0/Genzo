//! Develop's frames as the binary tile protocol carries them (DEV-01,
//! *Readback*; `api::TileHeader`), kept by generation for the frontend to
//! fetch.
//!
//! - **At Fit** the frame goes whole, in 256 px tiles, lossless. During a
//!   gesture it is the half-size preview, as JPEG bands (quality 80): the
//!   frontend scales it into the viewport, as the frame's own size says.
//! - **At 1:1** each visible tile goes on its own, placed by its rect in the
//!   shown image, which the header names as the image: a tile the frontend
//!   holds stays valid across whole-pixel pans. During a gesture each is a
//!   JPEG; settled, lossless.
//! - The JPEGs are encoded on rayon, one band or tile per task.

use rayon::prelude::*;

use genzo_color::output::Target;
use genzo_model::frame::{PixelFormat, Rect, TILE_HEADER_LEN, TILE_SIZE, TileHeader};
use genzo_pipeline::gpu::develop::Readback;

use crate::{PROXY_JPEG_QUALITY, PROXY_MAX_BANDS};

/// The lossless format of a frame for `target`.
pub(crate) fn format(target: Target) -> PixelFormat {
    match target {
        Target::Srgb => PixelFormat::Rgba8Srgb,
        Target::DisplayP3 => PixelFormat::Rgba8DisplayP3,
    }
}

/// `readback` of render `generation` as tile messages, concatenated, and
/// their count. `interactive` sends the 1:1 tiles as JPEG. `None` when a
/// JPEG could not be encoded.
pub(crate) fn encode(
    generation: u64,
    readback: &Readback,
    target: Target,
    interactive: bool,
) -> Option<(Vec<u8>, u32)> {
    let format = format(target);
    let header = |rect: Rect, format, image: (u32, u32)| TileHeader {
        generation,
        rect,
        mip_level: 0,
        format,
        image_width: image.0,
        image_height: image.1,
    };
    let mut tiles: Vec<Vec<u8>> = match readback {
        Readback::Frame {
            size, half, pixels, ..
        } => {
            let rgba: &[u8] = pixels.as_flattened();
            if *half {
                bands(rgba, *size)?
                    .into_iter()
                    .map(|(rect, jpeg)| header(rect, format.as_jpeg(), *size).encode(&jpeg))
                    .collect()
            } else {
                return Some(whole(generation, rgba, *size, target));
            }
        }
        Readback::Tiles { shown, tiles } if interactive => tiles
            .par_iter()
            .map(|(t, rgba)| {
                let jpeg = genzo_imgio::jpeg::encode_rgba(
                    rgba,
                    t.image.width,
                    t.image.height,
                    PROXY_JPEG_QUALITY,
                )
                .ok()?;
                Some(header(t.image, format.as_jpeg(), *shown).encode(&jpeg))
            })
            .collect::<Option<_>>()?,
        Readback::Tiles { shown, tiles } => tiles
            .iter()
            .map(|(t, rgba)| header(t.image, format, *shown).encode(rgba))
            .collect(),
    };
    let count = tiles.len() as u32;
    let mut message = Vec::with_capacity(
        tiles.iter().map(Vec::len).sum::<usize>() + tiles.len() * TILE_HEADER_LEN,
    );
    for t in &mut tiles {
        message.append(t);
    }
    Some((message, count))
}

/// A `size` RGBA8 frame for `target` as lossless 256 px tile messages,
/// concatenated, and their count.
pub(crate) fn whole(
    generation: u64,
    rgba: &[u8],
    size: (u32, u32),
    target: Target,
) -> (Vec<u8>, u32) {
    let row = size.0 as usize * 4;
    let rects = genzo_pipeline::tiles::grid(size.0, size.1, TILE_SIZE);
    let mut message = Vec::with_capacity(rgba.len() + rects.len() * TILE_HEADER_LEN);
    for rect in &rects {
        let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
        for y in rect.y..rect.y + rect.height {
            let start = y as usize * row + rect.x as usize * 4;
            out.extend_from_slice(&rgba[start..start + rect.width as usize * 4]);
        }
        let header = TileHeader {
            generation,
            rect: *rect,
            mip_level: 0,
            format: format(target),
            image_width: size.0,
            image_height: size.1,
        };
        message.extend_from_slice(&header.encode(&out));
    }
    (message, rects.len() as u32)
}

/// A `size` frame as horizontal JPEG bands, each a multiple of the 16-row
/// JPEG block high, so no band pads or blurs across its edge.
pub(crate) fn bands(rgba: &[u8], size: (u32, u32)) -> Option<Vec<(Rect, Vec<u8>)>> {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let n = rayon::current_num_threads().clamp(1, PROXY_MAX_BANDS);
    let band_h = h.div_ceil(n).div_ceil(16) * 16;
    let rows: Vec<(usize, usize)> = (0..h)
        .step_by(band_h.max(1))
        .map(|y| (y, band_h.min(h - y)))
        .collect();
    rows.par_iter()
        .map(|&(y, rows)| {
            let jpeg = genzo_imgio::jpeg::encode_rgba(
                &rgba[y * w * 4..(y + rows) * w * 4],
                w as u32,
                rows as u32,
                PROXY_JPEG_QUALITY,
            )
            .ok()?;
            let rect = Rect {
                x: 0,
                y: y as u32,
                width: w as u32,
                height: rows as u32,
            };
            Some((rect, jpeg))
        })
        .collect()
}
