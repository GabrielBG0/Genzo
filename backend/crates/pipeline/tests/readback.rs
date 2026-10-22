//! Readback (DEV-01, Readback): only what is sent, through the frame's two
//! staging buffers.
//!
//! - The visible tiles of a 1:1 view, read alone, are the frame's pixels
//!   bit for bit, also when they spill over both buffers and need more
//!   than one round.
//! - A pixel at 1:1 does not depend on the pan: two views that overlap
//!   give the same codes there, bit for bit, with every stage on (the base
//!   layer read by Highlights and Shadows, stage 7, the dither), in a
//!   turned and flipped, straightened crop too. So a tile the frontend
//!   holds stays valid, and a pan reads back only the new ones.

use std::sync::{Mutex, MutexGuard};

use genzo_color::output::Target;
use genzo_model::frame::Rect;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::geometry::{Crop, Geometry, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::readback::visible_tiles;
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::reference::sampling::{Frame, ViewSpec};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => {
            eprintln!("adapter: {} ({})", gpu.info.name, gpu.info.backend);
            Some((lock, gpu))
        }
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// Every stage on: local tone, contrast, stage 7, Genzo Standard.
fn look() -> Look {
    Look {
        exposure: 0.4,
        highlights: -70.0,
        shadows: 60.0,
        contrast: 25.0,
        whites: -15.0,
        vibrance: 30.0,
        saturation: -10.0,
        ..Look::with_profile(Profile::GenzoStandard)
    }
}

/// The frames of the test: the whole image, and a turned, flipped and
/// straightened crop.
fn frames(size: (u32, u32)) -> Vec<(&'static str, Frame)> {
    let g = Geometry {
        quarter_turns: 1,
        flip_horizontal: true,
        crop: Crop {
            cx: 0.5,
            cy: 0.5,
            w: 0.8,
            h: 0.8,
            angle: 7.5,
        },
    };
    let crop = fit(g.crop, size, g.orientation(6)).expect("fits");
    vec![
        ("whole, file turned", Frame::from(6)),
        (
            "turned, flipped, straightened",
            Frame::of(&Geometry { crop, ..g }, 6),
        ),
    ]
}

#[test]
fn visible_tiles_are_the_frames_pixels() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let t = camera_transform(&src.color, None);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    let spec = ViewSpec::Region {
        out: (600, 300),
        zoom: 1.0,
        origin: (301.0, 250.0),
    };
    view.show(&gpu, &mut stages, &mut cache, &src, 1, 1, spec)
        .unwrap();
    view.output(&gpu, Target::Srgb, true, true);
    let frame = view.read_frame(&gpu).unwrap();
    assert_eq!(view.offset(), (301, 250));
    let tiles = visible_tiles(view.offset(), spec.out(), src.size());
    assert_eq!(tiles.len(), 3 * 3, "partial tiles on every side");
    // Three times over: more than the two buffers hold, so three rounds.
    let rects: Vec<Rect> = (0..3).flat_map(|_| tiles.iter().map(|t| t.frame)).collect();
    let got = view.read_rects(&gpu, &rects).unwrap();
    assert_eq!(got.len(), rects.len());
    let w = spec.out().0;
    for (r, bytes) in rects.iter().zip(&got) {
        assert_eq!(bytes.len(), (r.width * r.height * 4) as usize);
        for (i, px) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (r.x + i as u32 % r.width, r.y + i as u32 / r.width);
            assert_eq!(*px, frame[(y * w + x) as usize], "{r:?} at ({x}, {y})");
        }
    }
}

#[test]
fn a_pixel_at_one_to_one_does_not_depend_on_the_pan() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let size = src.size();
    let t = camera_transform(&src.color, None);
    let curve = BaseCurve::new(&src.color);
    let look = look();
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    for (name, frame) in frames(size) {
        let mut layer = BaseLayer::new(&gpu);
        layer.build(&gpu, stages.pyramid().unwrap(), size, frame);
        let shown = frame.pixel_size(size);
        let out = (560, 560);
        // The views overlap, and both hold the tile at (256, 256) whole.
        let origins = [(190, 10), (40, 240)];
        let mut renders = Vec::new();
        for origin in origins {
            let spec = ViewSpec::Region {
                out,
                zoom: 1.0,
                origin: (f64::from(origin.0), f64::from(origin.1)),
            };
            let s = view
                .show(&gpu, &mut stages, &mut cache, &src, 1, frame, spec)
                .unwrap();
            assert_eq!(s.level, 0, "{name}");
            view.tone(&gpu, &look, Some(&curve), Some(&layer));
            view.color(&gpu, &look);
            view.output(&gpu, Target::Srgb, true, true);
            assert_eq!(view.offset(), origin);
            renders.push(view.read_frame(&gpu).unwrap());
        }
        let (a, b) = (origins[0], origins[1]);
        let mut shared = 0;
        let overlap = |i: usize, len: u32| {
            let [p, q] = [a, b].map(|o| [o.0, o.1][i]);
            p.max(q)..p.min(q) + len as i32
        };
        for y in overlap(1, out.1) {
            for x in overlap(0, out.0) {
                let at = |o: (i32, i32)| ((y - o.1) as u32 * out.0 + (x - o.0) as u32) as usize;
                assert_eq!(renders[0][at(a)], renders[1][at(b)], "{name}, ({x}, {y})");
                shared += 1;
            }
        }
        assert!(shared > 10_000, "{name}: {shared} pixels shared");
        // The whole tiles both views show are read back the same.
        let tiles = |o: (i32, i32)| visible_tiles(o, out, shown);
        let first = tiles(a);
        let mut whole = 0;
        for tb in tiles(b) {
            let Some(ta) = first.iter().find(|t| t.image == tb.image) else {
                continue;
            };
            let pixels = |render: &Vec<[u8; 4]>, r: Rect| {
                (r.y..r.y + r.height)
                    .flat_map(|y| (r.x..r.x + r.width).map(move |x| (x, y)))
                    .map(|(x, y)| render[(y * out.0 + x) as usize])
                    .collect::<Vec<_>>()
            };
            assert_eq!(pixels(&renders[0], ta.frame), pixels(&renders[1], tb.frame));
            whole += u32::from(tb.image.width == 256 && tb.image.height == 256);
        }
        assert!(whole >= 1, "{name}");
        eprintln!("{name}: {shared} pixels and {whole} whole tiles shared");
    }
}
