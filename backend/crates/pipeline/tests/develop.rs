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
//! - **Opened from a prefetched level 1** (DEV-01, *Prefetched
//!   neighbors*): without its input, a Fit view that level 1 covers, a
//!   tone change and a white balance drag equal a fresh render's frames,
//!   bit for bit, and no pyramid is rendered. A settled new white waits for
//!   the input; a view that wants level 0 is provisional until it comes,
//!   then equals a fresh render.
//! - **The histogram and the overlays** (DEV-03): at Fit each frame's
//!   counts, the drag preview's included, are a CPU count of its bytes; at
//!   1:1 there are none with the frame, and the whole-photo pass equals the
//!   Fit histogram bin for bin where Fit reads level 1 (criterion 4). After
//!   the pass, a 1:1 drag and its release render the frames they would
//!   have without it, and no cache was made again. Turning an overlay on or
//!   off re-runs stage 9 alone.

use std::sync::{Mutex, MutexGuard};

use genzo_color::encoding::InputEncoding;
use genzo_color::output::Target;
use genzo_model::edit::{Edit, Profile, Treatment, WhiteBalanceMode};
use genzo_pipeline::gpu::develop::{
    Develop, DevelopError, Phase, Photo, PhotoInfo, Reran, as_shot,
};
use genzo_pipeline::gpu::raw_stages::{Level1, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::histogram::{self, Overlay, Preview};
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

/// A Fit view larger than level 1 of the turned 1030 x 777 photo: its
/// frame is 420 x 557, level 1 shows 389 x 515.
const FIT: ViewSpec = ViewSpec::Fit((420, 560));
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

/// A Fit view that level 1 of the turned 1030 x 777 photo covers.
const SMALL_FIT: ViewSpec = ViewSpec::Fit((300, 220));

/// `photo()`'s level 1 with its as-shot white, as prefetch builds it.
fn level1(gpu: &Gpu) -> Level1 {
    let p = photo();
    let (t, _) = as_shot(&p.src);
    let mut stages = RawStages::new(gpu, TILES);
    let mut b = stages.start_level1(gpu, &p.src, &t);
    assert!(
        stages
            .continue_level1(gpu, &p.src, &mut b, || false)
            .unwrap()
    );
    b.finish()
}

/// A `Develop` that opened `photo()` from its level 1, without its input.
fn from_level1(gpu: &Gpu) -> (Develop, Edit) {
    let mut d = Develop::new(gpu, TILES);
    d.open_level1(gpu, PhotoInfo::of(&photo()), &level1(gpu))
        .unwrap();
    let defaults = Edit::defaults(d.kind().unwrap());
    (d, defaults)
}

#[test]
fn a_photo_opened_from_its_level1_renders_as_a_fresh_one() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = from_level1(&gpu);
    assert!(!d.decoded());
    let r = d
        .render(&gpu, &defaults, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(!r.pyramid && r.sample, "{r:?}");
    assert!(!d.provisional());
    same(&gpu, &d, &defaults, SMALL_FIT, "the defaults from level 1");

    let a = toned(defaults.clone());
    d.render(&gpu, &a, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    same(&gpu, &d, &a, SMALL_FIT, "a tone change from level 1");

    // A white balance drag is a ratio on the pyramid: no input needed.
    let b = warmer(a.clone(), 4200.0);
    let r = d
        .render(&gpu, &b, SMALL_FIT, Target::Srgb, Phase::Update)
        .unwrap();
    assert!(!r.pyramid, "{r:?}");
    let (mut e, _) = opened(&gpu);
    e.render(&gpu, &a, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    e.render(&gpu, &b, SMALL_FIT, Target::Srgb, Phase::Update)
        .unwrap();
    assert_eq!(frame(&gpu, &d), frame(&gpu, &e), "a drag from level 1");

    // Its release rebuilds the pyramid, which needs the input.
    let err = d
        .render(&gpu, &b, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap_err();
    assert!(matches!(err, DevelopError::NeedsSource), "{err:?}");
    let other = with_camera_profile(scene_raw(800, 600));
    let err = d.attach(other).unwrap_err();
    assert!(matches!(err, DevelopError::NotItsInput { .. }), "{err:?}");
    assert!(!d.decoded());
    d.attach(photo().src).unwrap();
    assert!(d.decoded());
    let r = d
        .render(&gpu, &b, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.pyramid, "{r:?}");
    same(&gpu, &d, &b, SMALL_FIT, "the release once the input came");
}

#[test]
fn a_view_that_wants_level0_is_provisional_until_the_input_comes() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = from_level1(&gpu);
    let a = toned(defaults.clone());
    for view in [FIT, ONE_TO_ONE] {
        d.render(&gpu, &a, view, Target::Srgb, Phase::End).unwrap();
        assert!(d.provisional(), "{view:?}");
        assert_ne!(
            frame(&gpu, &d),
            fresh(&gpu, &a, view),
            "{view:?}: level 1 upsampled, not level 0"
        );
    }
    // The same request again once the input came: drawn from level 0.
    d.attach(photo().src).unwrap();
    let r = d
        .render(&gpu, &a, ONE_TO_ONE, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.sample && !r.pyramid, "{r:?}");
    assert!(!d.provisional());
    same(&gpu, &d, &a, ONE_TO_ONE, "1:1 once the input came");
    d.render(&gpu, &a, FIT, Target::Srgb, Phase::End).unwrap();
    same(&gpu, &d, &a, FIT, "Fit once the input came");
}

#[test]
fn a_level1_opened_over_a_photo_drops_its_state() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    // The held photo leaves a rebuild waiting, a base layer and a frame.
    let a = warmer(cropped(toned(defaults.clone())), 3900.0);
    d.render(&gpu, &a, FIT, Target::Srgb, Phase::End).unwrap();
    assert!(d.rebuilding());
    d.open_level1(&gpu, PhotoInfo::of(&photo()), &level1(&gpu))
        .unwrap();
    assert!(!d.rebuilding() && !d.decoded());
    let r = d
        .render(&gpu, &defaults, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert!(r.base_layer && r.sample && !r.pyramid, "{r:?}");
    same(
        &gpu,
        &d,
        &defaults,
        SMALL_FIT,
        "a level 1 over a held photo",
    );
}

/// Bright enough to clip highlights in places.
fn clipping(mut e: Edit) -> Edit {
    e.exposure = 1.5;
    e.blacks = -40.0;
    e
}

const J: Overlay = Overlay {
    highlights: Some([255, 107, 107, 255]),
    shadows: Some([91, 157, 255, 255]),
    preview: Preview::None,
};

#[test]
fn at_fit_the_histogram_is_the_frames() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let e = clipping(toned(defaults));
    d.render(&gpu, &e, FIT, Target::Srgb, Phase::End).unwrap();
    let h = d.histogram(&gpu).unwrap().expect("counted at Fit");
    assert_eq!(h, histogram::count(&frame(&gpu, &d)));
    assert!(h.highlights.pixels > 0 && h.shadows.pixels > 0, "{h:?}");
    // The drag preview's frame, half size.
    let moved = Edit {
        exposure: 1.8,
        ..e.clone()
    };
    d.render(&gpu, &moved, FIT, Target::Srgb, Phase::Update)
        .unwrap();
    let h = d.histogram(&gpu).unwrap().expect("counted while dragging");
    let (w, ht) = d.view().frame_size();
    assert_eq!(h.pixels, w * ht);
    assert_eq!(h, histogram::count(&frame(&gpu, &d)));
    // At 1:1 the frame is not the whole photo.
    d.render(&gpu, &moved, ONE_TO_ONE, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(d.histogram(&gpu).unwrap(), None);
}

#[test]
fn an_overlay_re_runs_stage_9_alone() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let e = clipping(toned(defaults));
    for view in [FIT, ONE_TO_ONE] {
        d.set_overlay(Overlay::default());
        d.render(&gpu, &e, view, Target::Srgb, Phase::End).unwrap();
        let plain = frame(&gpu, &d);
        let counted = d.histogram(&gpu).unwrap();
        let only_output = Reran {
            output: true,
            ..NOTHING
        };
        d.set_overlay(J);
        let r = d.render(&gpu, &e, view, Target::Srgb, Phase::End).unwrap();
        assert_eq!(r, only_output, "{view:?}");
        let drawn = frame(&gpu, &d);
        let want: Vec<_> = plain.iter().map(|&px| J.apply(px)).collect();
        assert_eq!(drawn, want, "{view:?}: J drawn over the frame");
        assert_ne!(drawn, plain, "{view:?}: something clips");
        // The counts are the photo's, not the overlay's.
        assert_eq!(d.histogram(&gpu).unwrap(), counted, "{view:?}");
        d.set_overlay(Overlay::default());
        let r = d.render(&gpu, &e, view, Target::Srgb, Phase::End).unwrap();
        assert_eq!(r, only_output, "{view:?}");
        assert_eq!(frame(&gpu, &d), plain, "{view:?}: J off again");
        // Nothing changed, nothing re-runs.
        let r = d.render(&gpu, &e, view, Target::Srgb, Phase::End).unwrap();
        assert_eq!(r, NOTHING, "{view:?}");
    }
}

#[test]
fn at_1_1_the_whole_photo_is_counted_at_the_fit_size() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    // Turned and straightened, with a crop loose enough that SMALL_FIT
    // still reads level 1; a 1:1 view of its size has its Fit size.
    let mut e = clipping(toned(defaults));
    e.quarter_turns = 1;
    e.crop.w = 0.8;
    e.crop.h = 0.85;
    e.crop.angle = 6.0;
    d.render(&gpu, &e, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    let at_fit = d.histogram(&gpu).unwrap().expect("Fit");
    let one_to_one = ViewSpec::Region {
        out: SMALL_FIT.out(),
        zoom: 1.0,
        origin: (0.0, 0.0),
    };
    d.render(&gpu, &e, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    let bytes = (d.view().cache_bytes(), d.view().output_bytes());
    // The 1:1 view's frames, without the pass.
    let settled = frame(&gpu, &d);
    let moved = Edit {
        exposure: 1.7,
        ..e.clone()
    };
    d.render(&gpu, &moved, one_to_one, Target::Srgb, Phase::Update)
        .unwrap();
    let dragged = frame(&gpu, &d);
    d.render(&gpu, &e, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(frame(&gpu, &d), settled);

    let whole = d.count_whole(&gpu).unwrap().expect("counted");
    assert_eq!(whole, at_fit, "the same counts as at Fit, bin for bin");
    assert_eq!(
        (d.view().cache_bytes(), d.view().output_bytes()),
        bytes,
        "in the view's own caches"
    );
    // The caches no longer hold the view: the drag samples it again.
    let r = d
        .render(&gpu, &moved, one_to_one, Target::Srgb, Phase::Update)
        .unwrap();
    assert!(r.sample, "{r:?}");
    assert_eq!(frame(&gpu, &d), dragged, "the drag after the pass");
    d.render(&gpu, &e, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(frame(&gpu, &d), settled, "the release after the pass");
    assert_eq!(
        (d.view().cache_bytes(), d.view().output_bytes()),
        bytes,
        "nothing made again"
    );

    // Not after a drag's render, nor at Fit; but after its release with
    // the drag's edit, which renders nothing again.
    d.render(&gpu, &moved, one_to_one, Target::Srgb, Phase::Update)
        .unwrap();
    assert_eq!(d.count_whole(&gpu).unwrap(), None);
    let r = d
        .render(&gpu, &moved, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(r, NOTHING);
    assert!(d.count_whole(&gpu).unwrap().is_some());
    d.render(&gpu, &e, SMALL_FIT, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(d.count_whole(&gpu).unwrap(), None);
}

#[test]
fn where_fit_reads_level0_the_pass_reads_level1() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (mut d, defaults) = opened(&gpu);
    let e = clipping(toned(defaults));
    // FIT is larger than level 1: its frame is 420 x 557, level 1's 389 x 515.
    d.render(&gpu, &e, FIT, Target::Srgb, Phase::End).unwrap();
    let at_fit = d.histogram(&gpu).unwrap().expect("Fit");
    let one_to_one = ViewSpec::Region {
        out: FIT.out(),
        zoom: 1.0,
        origin: (100.0, 200.0),
    };
    d.render(&gpu, &e, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    let settled = frame(&gpu, &d);
    let whole = d.count_whole(&gpu).unwrap().expect("counted");
    assert_eq!(whole.pixels, 389 * 515, "level 1's own size");
    // Close to the Fit histogram, not equal: level 0 is sharper than
    // level 1 at the synthetic scene's hard edges. Measured on the RTX
    // 3060: the channels' means 2.2 codes apart at most, the share of
    // highlight-clipped pixels 0.12 points apart (2026-10-31).
    let mean = |h: &histogram::Histogram, c: usize| {
        let sum: f64 = (0..256).map(|i| i as f64 * f64::from(h.bins[c][i])).sum();
        sum / f64::from(h.pixels)
    };
    for c in 0..3 {
        let (a, b) = (mean(&whole, c), mean(&at_fit, c));
        assert!((a - b).abs() < 4.0, "channel {c}: {a} against {b}");
    }
    let share = |h: &histogram::Histogram| f64::from(h.highlights.pixels) / f64::from(h.pixels);
    let (a, b) = (share(&whole), share(&at_fit));
    assert!((a - b).abs() < 0.01, "highlights {a} against {b}");
    d.render(&gpu, &e, one_to_one, Target::Srgb, Phase::End)
        .unwrap();
    assert_eq!(frame(&gpu, &d), settled);
}
