//! One photo in Develop (`gpu::develop`, DEV-01, *Re-runs, quality levels
//! and generations*).
//!
//! - **The re-run table**, row by row: each kind of change re-runs from
//!   its earliest stage and no earlier ([`Reran`]).
//! - **Settled frames** after scripted sequences of changes, gestures
//!   included, equal the frame a fresh `Develop` renders for the final
//!   edit, bit for bit. That covers a white balance rebuild stopped between
//!   tiles and then a crop change, a new view or a new drag; a stage that
//!   turns back into the identity; and the level-0 tiles at 1:1 after a new
//!   white.
//! - **A white balance drag while the rest of a rebuild waits** reads only
//!   the first part's texels: its frame equals the same drag after the rest
//!   finished, bit for bit, at Fit and at 1:1.
//! - **A non-RAW photo** (its white balance a relative shift, Embedded, no
//!   dither while unedited) follows the same rules.
//! - **During a gesture at Fit** the frame is the half-size preview.

use std::sync::{Mutex, MutexGuard};

use genzo_color::encoding::InputEncoding;
use genzo_color::output::Target;
use genzo_model::edit::{Edit, Profile, Treatment, WhiteBalanceMode};
use genzo_pipeline::gpu::develop::{Develop, Phase, Photo, Reran};
use genzo_pipeline::gpu::raw_stages::TileConfig;
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::ViewSpec;

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

/// Small tiles, so a rebuild's first part leaves tiles for the rest.
const TILES: TileConfig = TileConfig {
    tile: 128,
    xtrans_tile: 128,
    apron: 24,
};

const FIT: ViewSpec = ViewSpec::Fit((420, 310));
const ONE_TO_ONE: ViewSpec = ViewSpec::Region {
    out: (300, 220),
    zoom: 1.0,
    origin: (180.0, 140.0),
};

fn photo() -> Photo {
    Photo {
        src: with_camera_profile(scene_raw(1030, 777)),
        orientation: 6,
    }
}

fn opened(gpu: &Gpu) -> (Develop, Edit) {
    let mut d = Develop::new(gpu, TILES);
    d.open(gpu, photo()).unwrap();
    let defaults = Edit::defaults(d.kind().unwrap());
    (d, defaults)
}

/// A warmer custom white.
fn warmer(mut e: Edit, temp: f32) -> Edit {
    e.white_balance.mode = WhiteBalanceMode::Custom;
    e.white_balance.temp = temp;
    e.white_balance.tint = 8.0;
    e
}

/// A tight, turned and straightened crop, so a rebuild's first part does
/// not cover the image.
fn cropped(mut e: Edit) -> Edit {
    e.quarter_turns = 1;
    e.crop.cx = 0.6;
    e.crop.cy = 0.4;
    e.crop.w = 0.35;
    e.crop.h = 0.3;
    e.crop.angle = 6.0;
    e
}

fn toned(mut e: Edit) -> Edit {
    e.exposure = 0.3;
    e.highlights = -60.0;
    e.shadows = 50.0;
    e.contrast = 20.0;
    e.vibrance = 25.0;
    e
}

fn frame(gpu: &Gpu, d: &Develop) -> Vec<[u8; 4]> {
    d.view().read_frame(gpu).unwrap()
}

/// The settled frame a fresh `Develop` renders for `edit` in `view`.
fn fresh(gpu: &Gpu, edit: &Edit, view: ViewSpec) -> Vec<[u8; 4]> {
    let (mut d, _) = opened(gpu);
    d.render(gpu, edit, view, Target::Srgb, Phase::End).unwrap();
    frame(gpu, &d)
}

fn same(gpu: &Gpu, d: &Develop, edit: &Edit, view: ViewSpec, what: &str) {
    let got = frame(gpu, d);
    let want = fresh(gpu, edit, view);
    let off = got.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(got.len(), want.len(), "{what}");
    assert_eq!(off, 0, "{what}: {off} of {} pixels differ", got.len());
}

/// A change, its phase and what it must re-run.
type Row = (&'static str, fn(&mut Edit), Phase, Reran);

const NOTHING: Reran = Reran {
    pyramid: false,
    finished_rebuild: false,
    base_layer: false,
    sample: false,
    tone: false,
    color: false,
    output: false,
};

#[test]
fn each_change_re_runs_from_its_stage() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let mut edit = toned(defaults.clone());
    let go = |d: &mut Develop, e: &Edit, view, target, phase| {
        d.render(&gpu, e, view, target, phase).unwrap()
    };
    let first = go(&mut d, &edit, FIT, Target::Srgb, Phase::End);
    assert!(first.base_layer && first.sample && first.tone && first.color && first.output);
    assert!(!first.pyramid, "open built the pyramid");

    // Nothing changed: nothing runs.
    assert_eq!(go(&mut d, &edit, FIT, Target::Srgb, Phase::End), NOTHING);

    // The rows of DEV-01's table.
    let tone = Reran {
        tone: true,
        color: true,
        output: true,
        ..NOTHING
    };
    let color = Reran {
        color: true,
        output: true,
        ..NOTHING
    };
    let sample = Reran {
        sample: true,
        ..tone
    };
    let rows: Vec<Row> = vec![
        ("exposure", |e| e.exposure = 0.8, Phase::End, tone),
        ("whites", |e| e.whites = -30.0, Phase::End, tone),
        ("shadows", |e| e.shadows = 10.0, Phase::End, tone),
        (
            "profile",
            |e| e.profile = Profile::GenzoLinear,
            Phase::End,
            tone,
        ),
        ("vibrance", |e| e.vibrance = 40.0, Phase::End, color),
        ("saturation", |e| e.saturation = -20.0, Phase::End, color),
        (
            "B&W",
            |e| e.treatment = Treatment::Monochrome,
            Phase::End,
            color,
        ),
        ("crop, dragged", |e| e.crop.w = 0.8, Phase::Update, sample),
        (
            "crop, released",
            |_| {},
            Phase::End,
            Reran {
                base_layer: true,
                ..sample
            },
        ),
        (
            "quarter turn",
            |e| e.quarter_turns = 1,
            Phase::End,
            Reran {
                base_layer: true,
                ..sample
            },
        ),
        (
            "white balance, dragged",
            |e| *e = warmer(e.clone(), 4200.0),
            Phase::Update,
            sample,
        ),
        (
            "white balance, released",
            |_| {},
            Phase::End,
            Reran {
                pyramid: true,
                base_layer: true,
                ..sample
            },
        ),
    ];
    for (name, change, phase, want) in rows {
        change(&mut edit);
        let got = go(&mut d, &edit, FIT, Target::Srgb, phase);
        assert_eq!(got, want, "{name}");
    }
    // The view re-runs from the sampling and keeps the base layer.
    let got = go(&mut d, &edit, ONE_TO_ONE, Target::Srgb, Phase::End);
    assert_eq!(
        Reran {
            finished_rebuild: false,
            ..got
        },
        sample,
        "view"
    );
    // The target re-runs stage 9 only.
    let got = go(&mut d, &edit, ONE_TO_ONE, Target::DisplayP3, Phase::End);
    assert_eq!(
        got,
        Reran {
            output: true,
            ..NOTHING
        },
        "target"
    );
}

#[test]
fn settled_frames_equal_a_fresh_render() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let go = |d: &mut Develop, e: &Edit, view, phase| {
        d.render(&gpu, e, view, Target::Srgb, phase).unwrap();
    };

    // Tone, crop and white balance, each with a gesture.
    let a = toned(defaults.clone());
    go(&mut d, &a, FIT, Phase::End);
    let b = cropped(a.clone());
    go(&mut d, &b, FIT, Phase::Start);
    go(&mut d, &b, FIT, Phase::End);
    let c = warmer(b.clone(), 3900.0);
    go(&mut d, &warmer(b.clone(), 4500.0), FIT, Phase::Start);
    go(&mut d, &c, FIT, Phase::Update);
    go(&mut d, &c, FIT, Phase::End);
    assert!(
        d.rebuilding(),
        "a tight crop leaves the rest of the rebuild"
    );
    same(&gpu, &d, &c, FIT, "after a white balance in a crop");

    // The rebuild stopped between tiles, then the crop changes: the rest is
    // finished before the new crop is read.
    let mut asked = 0;
    assert!(
        !d.continue_rebuild(&gpu, || {
            asked += 1;
            asked > 3
        })
        .unwrap()
    );
    let e = Edit {
        crop: defaults.crop.clone(),
        quarter_turns: 0,
        ..c.clone()
    };
    go(&mut d, &e, FIT, Phase::End);
    assert!(d.reran().finished_rebuild);
    same(&gpu, &d, &e, FIT, "a stopped rebuild, then the whole image");

    // Again in a crop, stopped, then a new white balance drag and release.
    let f = warmer(cropped(e.clone()), 6100.0);
    go(&mut d, &f, FIT, Phase::End);
    assert!(!d.continue_rebuild(&gpu, || true).unwrap());
    let g = warmer(f.clone(), 7300.0);
    go(&mut d, &g, FIT, Phase::Start);
    go(&mut d, &g, FIT, Phase::End);
    same(&gpu, &d, &g, FIT, "a stopped rebuild, then a new white");

    // Stage 7 alone turns back into the identity, then stage 6 too.
    let only_color = Edit {
        vibrance: 0.0,
        ..g.clone()
    };
    go(&mut d, &only_color, FIT, Phase::End);
    same(&gpu, &d, &only_color, FIT, "stage 7 back to the identity");
    let h = Edit {
        exposure: 0.0,
        contrast: 0.0,
        highlights: 0.0,
        shadows: 0.0,
        vibrance: 0.0,
        profile: Profile::GenzoLinear,
        ..g.clone()
    };
    go(&mut d, &h, FIT, Phase::End);
    same(&gpu, &d, &h, FIT, "tone and color back to the identity");

    // 1:1 after a new white: the level-0 tiles follow it.
    go(&mut d, &h, ONE_TO_ONE, Phase::End);
    let i = warmer(h.clone(), 5000.0);
    go(&mut d, &i, ONE_TO_ONE, Phase::End);
    same(&gpu, &d, &i, ONE_TO_ONE, "1:1 after a new white");

    // Back to the defaults.
    go(&mut d, &defaults, FIT, Phase::End);
    same(&gpu, &d, &defaults, FIT, "the defaults again");
}

#[test]
fn a_gesture_at_fit_shows_the_half_size_preview() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let e = toned(defaults);
    d.render(&gpu, &e, FIT, Target::Srgb, Phase::End).unwrap();
    let full = d.view().frame_size();
    let moved = Edit {
        exposure: 1.0,
        ..e.clone()
    };
    d.render(&gpu, &moved, FIT, Target::Srgb, Phase::Update)
        .unwrap();
    let half = d.view().frame_size();
    assert_eq!(half, (full.0.div_ceil(2), full.1.div_ceil(2)));
    // Released: the whole frame, as a fresh render.
    let r = d
        .render(&gpu, &moved, FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.output && !r.sample, "{r:?}");
    assert_eq!(d.view().frame_size(), full);
    same(&gpu, &d, &moved, FIT, "released");
    // At 1:1 a gesture keeps the whole frame.
    d.render(&gpu, &moved, ONE_TO_ONE, Target::Srgb, Phase::Update)
        .unwrap();
    assert_eq!(d.view().frame_size(), ONE_TO_ONE.out());
}

#[test]
fn a_drag_does_not_wait_for_the_rest() {
    let Some((_lock, gpu)) = gpu() else { return };
    for view in [FIT, ONE_TO_ONE] {
        let mut frames = Vec::new();
        for finish in [false, true] {
            let (mut d, defaults) = opened(&gpu);
            let released = warmer(cropped(toned(defaults)), 4100.0);
            d.render(&gpu, &released, view, Target::Srgb, Phase::End)
                .unwrap();
            assert!(d.rebuilding(), "{view:?}: the crop leaves a rest");
            let mut asked = 0;
            d.continue_rebuild(&gpu, || {
                asked += 1;
                asked > 2
            })
            .unwrap();
            if finish {
                assert!(d.continue_rebuild(&gpu, || false).unwrap());
            }
            let dragged = warmer(released.clone(), 6600.0);
            let r = d
                .render(&gpu, &dragged, view, Target::Srgb, Phase::Update)
                .unwrap();
            assert!(!r.finished_rebuild && !r.pyramid, "{view:?}: {r:?}");
            assert_eq!(d.rebuilding(), !finish);
            frames.push(frame(&gpu, &d));
        }
        let off = frames[0]
            .iter()
            .zip(&frames[1])
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(off, 0, "{view:?}: {off} pixels differ");
    }
}

#[test]
fn a_non_raw_photo_follows_the_same_rules() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (w, h) = (900u32, 640u32);
    let rgba: Vec<u16> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [x * 65535 / w, y * 65535 / h, ((x ^ y) * 37) & 0xffff, 65535].map(|v| v as u16)
        })
        .collect();
    let enc = InputEncoding::srgb();
    let photo = || Photo {
        src: RawSource::from_encoded((w, h), &rgba, &enc).unwrap(),
        orientation: 1,
    };
    let fresh = |edit: &Edit| {
        let mut d = Develop::new(&gpu, TILES);
        d.open(&gpu, photo()).unwrap();
        d.render(&gpu, edit, FIT, Target::Srgb, Phase::End).unwrap();
        frame(&gpu, &d)
    };
    let mut d = Develop::new(&gpu, TILES);
    d.open(&gpu, photo()).unwrap();
    let defaults = Edit::defaults(d.kind().unwrap());
    assert_eq!(defaults.profile, Profile::Embedded);
    d.render(&gpu, &defaults, FIT, Target::Srgb, Phase::End)
        .unwrap();
    let mut shifted = cropped(defaults.clone());
    shifted.white_balance.mode = WhiteBalanceMode::Custom;
    shifted.white_balance.temp = 35.0;
    shifted.white_balance.tint = -20.0;
    shifted.saturation = 15.0;
    d.render(&gpu, &shifted, FIT, Target::Srgb, Phase::Start)
        .unwrap();
    let r = d
        .render(&gpu, &shifted, FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.pyramid && r.base_layer, "{r:?}");
    assert_eq!(frame(&gpu, &d), fresh(&shifted), "a relative white");
    // Back to the defaults: the white is the encoded one again, and the
    // unedited photo shows its own codes, undithered.
    let r = d
        .render(&gpu, &defaults, FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.pyramid && r.output, "{r:?}");
    assert_eq!(frame(&gpu, &d), fresh(&defaults), "the defaults again");
    // Setting a control to the value it already has re-runs nothing.
    let same = Edit {
        exposure: 0.0,
        ..defaults.clone()
    };
    let r = d
        .render(&gpu, &same, FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(!r.output, "{r:?}");
}
