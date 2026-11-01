//! Develop's frames in the binary tile protocol (DEV-01, *Readback*), as
//! `Engine::frame` hands them to the frontend.
//!
//! - At Fit, settled: lossless 256 px tiles that put the read-back frame
//!   together again, bit for bit, in the target's format.
//! - At Fit, during a gesture: the half-size preview as JPEG bands, which
//!   cover it.
//! - At 1:1: each visible tile placed by its rect in the shown image, the
//!   header naming the shown image's size; lossless settled, JPEG during a
//!   gesture.

use std::sync::Mutex;

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::api::{PixelFormat, Rect, TileHeader};
use genzo_engine::develop::{Applied, Readback, Target, ViewSpec};
use genzo_engine::{Engine, covered_area, split_frame};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn engine() -> Option<Engine> {
    let engine = Engine::new();
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some(engine)
}

/// The frame's tile messages, decoded, after checking each names its
/// generation, and that `Applied::tiles` counts them.
fn tiles(engine: &Engine, a: &Applied) -> Vec<(TileHeader, Vec<u8>)> {
    let generation = a.frame.as_ref().expect("rendered").generation;
    let bytes = engine.frame(generation).expect("kept");
    let tiles = split_frame(&bytes).unwrap();
    assert_eq!(Some(tiles.len() as u32), a.tiles);
    assert!(tiles.iter().all(|(h, _)| h.generation == generation));
    tiles.into_iter().map(|(h, p)| (h, p.to_vec())).collect()
}

/// The image the tiles put together, RGBA8.
fn assemble(tiles: &[(TileHeader, Vec<u8>)]) -> Vec<u8> {
    let (w, h) = (tiles[0].0.image_width, tiles[0].0.image_height);
    let mut out = vec![0; w as usize * h as usize * 4];
    for (t, px) in tiles {
        let Rect { x, y, width, .. } = t.rect;
        for (r, row) in px.chunks_exact(width as usize * 4).enumerate() {
            let at = ((y as usize + r) * w as usize + x as usize) * 4;
            out[at..at + row.len()].copy_from_slice(row);
        }
    }
    out
}

#[test]
fn a_fit_frame_goes_as_lossless_tiles_or_as_jpeg_bands() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    engine
        .open_source(with_camera_profile(scene_raw(1030, 777)), 1)
        .unwrap();
    for (target, format) in [
        (Target::Srgb, PixelFormat::Rgba8Srgb),
        (Target::DisplayP3, PixelFormat::Rgba8DisplayP3),
    ] {
        let a = engine.set_view(ViewSpec::Fit((600, 450)), target).unwrap();
        let Readback::Frame { size, pixels, .. } = &a.frame.as_ref().unwrap().readback else {
            panic!("a frame at Fit")
        };
        let t = tiles(&engine, &a);
        assert!(t.len() > 1, "split in 256 px tiles");
        assert!(t.iter().all(|(h, _)| h.format == format
            && (h.image_width, h.image_height) == *size
            && h.rect.width <= 256
            && h.rect.height <= 256));
        let refs: Vec<_> = t.iter().map(|(h, p)| (*h, p.as_slice())).collect();
        assert_eq!(covered_area(&refs), u64::from(size.0 * size.1));
        assert_eq!(assemble(&t), pixels.as_flattened(), "bit for bit");
    }
    // The drag's half-size preview: JPEG bands of its own size.
    let a = engine
        .apply(&[Change::Exposure(0.5)], Phase::Start)
        .unwrap();
    let Readback::Frame { size, half, .. } = &a.frame.as_ref().unwrap().readback else {
        panic!("a frame at Fit")
    };
    assert!(*half);
    let t = tiles(&engine, &a);
    let refs: Vec<_> = t.iter().map(|(h, p)| (*h, p.as_slice())).collect();
    assert_eq!(covered_area(&refs), u64::from(size.0 * size.1));
    for (h, jpeg) in &t {
        assert_eq!(h.format, PixelFormat::JpegDisplayP3);
        assert_eq!((h.image_width, h.image_height), *size);
        assert_eq!((h.rect.x, h.rect.width), (0, size.0), "a whole band");
        let img = genzo_imgio::decode(jpeg, Default::default()).unwrap();
        assert_eq!((img.width, img.height), (h.rect.width, h.rect.height));
    }
}

#[test]
fn a_1_1_tile_is_placed_in_the_shown_image() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let size = (1300, 900);
    engine
        .open_source(with_camera_profile(scene_raw(size.0, size.1)), 1)
        .unwrap();
    let view = ViewSpec::Region {
        out: (600, 450),
        zoom: 1.0,
        origin: (300.0, 200.0),
    };
    let a = engine.set_view(view, Target::Srgb).unwrap();
    let Readback::Tiles { shown, tiles: read } = &a.frame.as_ref().unwrap().readback else {
        panic!("tiles at 1:1")
    };
    assert_eq!(*shown, size);
    let t = tiles(&engine, &a);
    assert_eq!(t.len(), read.len());
    for ((h, px), (v, want)) in t.iter().zip(read) {
        assert_eq!(h.rect, v.image, "placed in the shown image");
        assert_eq!((h.image_width, h.image_height), size);
        assert_eq!(h.format, PixelFormat::Rgba8Srgb);
        assert_eq!(px, want, "lossless");
    }
    // During a gesture each tile is a JPEG of its own rect.
    let d = engine
        .apply(&[Change::Exposure(0.5)], Phase::Start)
        .unwrap();
    let Readback::Tiles { tiles: read, .. } = &d.frame.as_ref().unwrap().readback else {
        panic!("tiles at 1:1")
    };
    let t = tiles(&engine, &d);
    for ((h, jpeg), (v, _)) in t.iter().zip(read) {
        assert_eq!(h.rect, v.image);
        assert_eq!(h.format, PixelFormat::JpegSrgb);
        let img = genzo_imgio::decode(jpeg, Default::default()).unwrap();
        assert_eq!((img.width, img.height), (v.image.width, v.image.height));
    }
}
