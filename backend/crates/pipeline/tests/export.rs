//! OUT-01 criteria 1 and 5 on the GPU: a full-size export's regions are
//! the 1:1 view's pixels, for Bayer, X-Trans, linear and non-RAW input,
//! with an edit that moves every stage, a crop, a quarter turn and a
//! straighten; region boundaries leave no trace; and the export keeps
//! rendering the photo as it was after Develop has opened another.

use std::sync::{Mutex, MutexGuard};

use genzo_color::output::Target;
use genzo_model::edit::{Edit, WhiteBalanceMode};
use genzo_model::frame::Rect;
use genzo_pipeline::gpu::develop::{Develop, Phase, Photo};
use genzo_pipeline::gpu::export::{ExportStages, REGION};
use genzo_pipeline::gpu::raw_stages::TileConfig;
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw, with_camera_profile};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::ViewSpec;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => Some((lock, gpu)),
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// Small stage 1–4 tiles, so regions and tiles cross each other.
const TILES: TileConfig = TileConfig {
    tile: 128,
    xtrans_tile: 128,
    apron: 24,
};

fn encoded(size: (u32, u32)) -> RawSource {
    let rgba: Vec<u16> = (0..size.0 * size.1)
        .flat_map(|i| {
            let (x, y) = (i % size.0, i / size.0);
            [
                x * 65535 / size.0,
                y * 65535 / size.1,
                (x * 7 + y * 3) % 65536,
                65535,
            ]
            .map(|v| v as u16)
        })
        .collect();
    let enc = genzo_color::encoding::InputEncoding::display_p3();
    RawSource::from_encoded(size, &rgba, &enc).unwrap()
}

fn linear(size: (u32, u32)) -> RawSource {
    let rgba: Vec<u16> = (0..size.0 * size.1)
        .flat_map(|i| {
            [
                (i * 13 % 65536) as u16,
                (i * 7 % 50000) as u16,
                20000,
                65535,
            ]
        })
        .collect();
    let enc = genzo_color::encoding::InputEncoding::nclx(9, 8).unwrap();
    RawSource::from_encoded(size, &rgba, &enc).unwrap()
}

fn sources() -> Vec<(&'static str, Photo)> {
    let size = (1030, 777);
    let photo = |src, orientation| Photo { src, orientation };
    vec![
        (
            "bayer",
            photo(with_camera_profile(scene_raw(size.0, size.1)), 6),
        ),
        ("x-trans", photo(scene_xtrans_raw(size.0, size.1), 1)),
        ("linear", photo(linear(size), 3)),
        ("display p3", photo(encoded(size), 8)),
    ]
}

/// Every stage moves: white balance, exposure, local tone, contrast,
/// color, a turned and straightened crop.
fn edited(defaults: &Edit) -> Edit {
    let mut e = defaults.clone();
    e.white_balance.mode = WhiteBalanceMode::Custom;
    e.white_balance.temp = 6100.0;
    e.white_balance.tint = 8.0;
    e.exposure = 0.3;
    e.highlights = -60.0;
    e.shadows = 50.0;
    e.contrast = 20.0;
    e.vibrance = 25.0;
    e.saturation = -10.0;
    e.quarter_turns = 1;
    e.crop.cx = 0.55;
    e.crop.cy = 0.45;
    e.crop.w = 0.8;
    e.crop.h = 0.75;
    e.crop.angle = 6.0;
    e
}

/// The shown image cut into `w` × `h` rects, the last of each row and
/// column cut short.
fn rects(size: (u32, u32), (w, h): (u32, u32)) -> Vec<Rect> {
    let mut out = Vec::new();
    for y in (0..size.1).step_by(h as usize) {
        for x in (0..size.0).step_by(w as usize) {
            out.push(Rect {
                x,
                y,
                width: w.min(size.0 - x),
                height: h.min(size.1 - y),
            });
        }
    }
    out
}

/// Runs the rest of a white balance rebuild, as the render thread does
/// before an export region.
fn settle(gpu: &Gpu, d: &mut Develop) {
    while d.rebuilding() {
        d.continue_rebuild(gpu, || false).unwrap();
    }
}

/// The 1:1 view of `rect`, as the viewport draws it.
fn one_to_one(gpu: &Gpu, d: &mut Develop, edit: &Edit, rect: Rect) -> Vec<u8> {
    let view = ViewSpec::Region {
        out: (rect.width, rect.height),
        zoom: 1.0,
        origin: (f64::from(rect.x), f64::from(rect.y)),
    };
    d.render(gpu, edit, view, Target::Srgb, Phase::End).unwrap();
    settle(gpu, d);
    d.view().read_frame(gpu).unwrap().into_flattened()
}

#[test]
fn full_size_regions_are_the_one_to_one_view() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, photo) in sources() {
        let mut d = Develop::new(&gpu, TILES);
        d.open(&gpu, photo).unwrap();
        let edit = edited(&Edit::defaults(d.kind().unwrap()));
        let inputs = d.export_inputs(&gpu, &edit, Target::Srgb).unwrap();
        settle(&gpu, &mut d);
        let size = inputs.size();
        let mut export = ExportStages::new(&gpu);
        // Regions that cross the stage 1–4 tiles and the level-0 cache's.
        for rect in rects(size, (300, 220)) {
            let got = d.render_export(&gpu, &mut export, &inputs, rect).unwrap();
            let want = one_to_one(&gpu, &mut d, &edit, rect);
            let off = got
                .chunks(4)
                .zip(want.chunks(4))
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(got.len(), want.len(), "{name} {rect:?}");
            assert_eq!(off, 0, "{name} {rect:?}: {off} pixels differ from 1:1");
        }
        // The whole image as one region equals it cut into regions.
        assert!(size.0 <= REGION && size.1 <= REGION, "{size:?}");
        let whole_rect = Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        };
        let whole = d
            .render_export(&gpu, &mut export, &inputs, whole_rect)
            .unwrap();
        for rect in rects(size, (256, 200)) {
            let part = d.render_export(&gpu, &mut export, &inputs, rect).unwrap();
            for row in 0..rect.height {
                let at = |x: u32, y: u32, w: u32| ((y * w + x) * 4) as usize;
                let a = at(rect.x, rect.y + row, size.0);
                let b = at(0, row, rect.width);
                let len = (rect.width * 4) as usize;
                assert_eq!(
                    &whole[a..a + len],
                    &part[b..b + len],
                    "{name} {rect:?} row {row}"
                );
            }
        }
        eprintln!("{name}: {size:?} matches 1:1");
    }
}

#[test]
fn an_export_keeps_its_photo_after_develop_opens_another() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut all = sources();
    let (_, other) = all.pop().unwrap();
    let (_, photo) = all.swap_remove(1);
    let mut d = Develop::new(&gpu, TILES);
    d.open(&gpu, photo).unwrap();
    let edit = edited(&Edit::defaults(d.kind().unwrap()));
    let inputs = d.export_inputs(&gpu, &edit, Target::Srgb).unwrap();
    settle(&gpu, &mut d);
    let mut export = ExportStages::new(&gpu);
    let rect = Rect {
        x: 100,
        y: 80,
        width: 400,
        height: 300,
    };
    let before = d.render_export(&gpu, &mut export, &inputs, rect).unwrap();
    // A switch, and renders of the other photo between regions.
    d.open(&gpu, other).unwrap();
    let other_edit = Edit::defaults(d.kind().unwrap());
    d.render(
        &gpu,
        &other_edit,
        ViewSpec::Fit((300, 200)),
        Target::Srgb,
        Phase::End,
    )
    .unwrap();
    let after = d.render_export(&gpu, &mut export, &inputs, rect).unwrap();
    assert_eq!(
        before, after,
        "the X-Trans export, after a Display P3 photo opened"
    );
    // A fresh export stage set gives the same pixels too.
    let mut fresh = ExportStages::new(&gpu);
    assert_eq!(
        before,
        d.render_export(&gpu, &mut fresh, &inputs, rect).unwrap()
    );
    eprintln!("export stages hold {} MB", export.bytes() >> 20);
}

/// `budget::EXPORT`: an export's GPU memory at a whole region, with the
/// base layer's copy (its 1536 px working size for a large photo).
#[test]
fn an_export_holds_its_budget() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut d = Develop::new(&gpu, TILES);
    d.open(
        &gpu,
        Photo {
            src: with_camera_profile(scene_raw(3100, 2100)),
            orientation: 1,
        },
    )
    .unwrap();
    let mut edit = Edit::defaults(d.kind().unwrap());
    edit.highlights = -40.0;
    edit.crop.angle = 45.0;
    let inputs = d.export_inputs(&gpu, &edit, Target::Srgb).unwrap();
    settle(&gpu, &mut d);
    let mut export = ExportStages::new(&gpu);
    let rect = Rect {
        x: 0,
        y: 0,
        width: REGION,
        height: REGION.min(inputs.size().1),
    };
    d.render_export(&gpu, &mut export, &inputs, rect).unwrap();
    let held = export.bytes() + inputs.bytes();
    eprintln!(
        "export: stages {} B, base layer copy {} B, {} MB in all",
        export.bytes(),
        inputs.bytes(),
        held / 1_000_000
    );
    assert!(held <= genzo_pipeline::budget::EXPORT, "{held}");
}
