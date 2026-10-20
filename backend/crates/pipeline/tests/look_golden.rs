//! Stored goldens of stages 6 and 7 (DEV-02 criterion 6, DEV-01 criterion
//! 5), written on lavapipe: the stage 7 cache through color::output's
//! stage 9 on the CPU, as display floats. The 8-bit frame is not compared:
//! its dither flips a code wherever devices differ by a hair, which on a
//! dark gray turns neutral into ΔE 2 (Metal against lavapipe, measured);
//! stage 9 and the dither have their own golden (view/synthetic-fit-srgb).
//!
//! - the synthetic scene with Genzo Standard and tone controls;
//! - the synthetic scene with Genzo Linear and the presence controls;
//! - the synthetic scene with Highlights and Shadows (the base layer);
//! - the synthetic scene with a camera profile, during a white balance
//!   drag (the ratio on the sampled pyramid);
//! - the synthetic scene turned, flipped, cropped and straightened, the
//!   base layer built on the crop;
//! - the iPhone 12 Pro ProRAW (CC0) with Genzo Standard, through the file's
//!   own ProfileToneCurve, at default edits and with Highlights and
//!   Shadows. CI runs this file after fetching the corpus
//!   (GENZO_REQUIRE_CORPUS=1); elsewhere a missing file skips.

#[allow(dead_code)]
mod sdk_stage2;

use std::sync::{Mutex, MutexGuard};

use genzo_color::camera::{temperature_from_xy, xy_from_temperature};
use genzo_color::output::{Target, encode};
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::camera_color::{camera_transform, drag_ratio};
use genzo_pipeline::geometry::{Crop, Geometry, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::view::{ViewStages, compresses};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{Frame, ViewSpec};

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

/// Renders `src` at Fit into `out` with `look` and checks it against the
/// golden `name`.
fn check(gpu: &Gpu, src: &RawSource, look: &Look, out: (u32, u32), name: &str) {
    check_with(gpu, src, look, out, name, None);
}

/// [`check`], with the white balance drag `ratio` applied to the sampled
/// pyramid.
fn check_with(
    gpu: &Gpu,
    src: &RawSource,
    look: &Look,
    out: (u32, u32),
    name: &str,
    ratio: Option<[[f32; 3]; 3]>,
) {
    let mut stages = RawStages::new(gpu, TileConfig::default());
    let p = stages
        .render_pyramid(gpu, src, &camera_transform(&src.color, None))
        .unwrap();
    let mut view = ViewStages::new(gpu);
    // The photo's aspect within `out`, unrotated.
    let (w, h) = p.size;
    let scale = (f64::from(out.0) / f64::from(w)).min(f64::from(out.1) / f64::from(h));
    let fit = (
        (f64::from(w) * scale).round() as u32,
        (f64::from(h) * scale).round() as u32,
    );
    view.set_ratio(ratio);
    view.sample(gpu, p, 1, fit);
    let base = BaseCurve::new(&src.color);
    let mut layer = BaseLayer::new(gpu);
    if look.is_local() {
        layer.build(gpu, p, src.size(), 1);
    }
    view.tone(gpu, look, Some(&base), Some(&layer));
    view.color(gpu, look);
    let compress = compresses(p, Target::Srgb);
    let pixels = view
        .read_latest(gpu)
        .unwrap()
        .iter()
        .map(|px| encode(px.map(f64::from), Target::Srgb, compress).map(|v| v as f32))
        .collect();
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        name,
        genzo_golden::Space::DisplaySrgb,
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(&case, &genzo_golden::Image::new(fit.0, fit.1, pixels));
}

#[test]
fn golden_synthetic_standard() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1536, 1024);
    let look = Look {
        exposure: 0.4,
        contrast: 35.0,
        whites: -30.0,
        blacks: 20.0,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    check(&gpu, &src, &look, (300, 200), "look/synthetic-standard");
}

#[test]
fn golden_synthetic_linear_presence() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1536, 1024);
    let look = Look {
        exposure: 0.8,
        vibrance: 60.0,
        saturation: -20.0,
        ..Look::with_profile(Profile::GenzoLinear)
    };
    check(
        &gpu,
        &src,
        &look,
        (300, 200),
        "look/synthetic-linear-presence",
    );
}

#[test]
fn golden_synthetic_local_tone() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1536, 1024);
    let look = Look {
        highlights: -70.0,
        shadows: 60.0,
        exposure: 0.3,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    check(&gpu, &src, &look, (300, 200), "look/synthetic-local-tone");
}

/// A white balance drag golden: the synthetic scene with a camera profile,
/// built at its as-shot white and dragged 1500 K cooler and 20 toward
/// magenta, with Genzo Standard.
#[test]
fn golden_synthetic_wb_drag() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = with_camera_profile(scene_raw(1536, 1024));
    let old = camera_transform(&src.color, None);
    let (t, tint) = temperature_from_xy(old.white);
    let new = camera_transform(
        &src.color,
        Some(xy_from_temperature(t - 1500.0, tint + 20.0)),
    );
    let look = Look::with_profile(Profile::GenzoStandard);
    let ratio = drag_ratio(&old, &new);
    check_with(
        &gpu,
        &src,
        &look,
        (300, 200),
        "look/synthetic-wb-drag",
        ratio,
    );
}

/// A geometry golden: one clockwise turn, a flip, and a crop straightened
/// by 8 degrees, shown at Fit through the level-0 tiles where the crop
/// needs them, with Highlights and Shadows on a base built on the crop.
#[test]
fn golden_synthetic_cropped() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1536, 1024);
    let geometry = Geometry {
        quarter_turns: 1,
        flip_horizontal: true,
        crop: Crop {
            cx: 0.45,
            cy: 0.55,
            w: 0.7,
            h: 0.6,
            angle: 8.0,
        },
    };
    let crop = fit(geometry.crop, src.size(), geometry.orientation(1)).unwrap();
    let frame = Frame::of(&Geometry { crop, ..geometry }, 1);
    let look = Look {
        highlights: -60.0,
        shadows: 50.0,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages
        .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
        .unwrap();
    let compress = compresses(p, Target::Srgb);
    let mut layer = BaseLayer::new(&gpu);
    layer.build(&gpu, p, src.size(), frame);
    let framed = frame.pixel_size(src.size());
    let scale = 300.0 / f64::from(framed.0.max(framed.1));
    let out = (
        (f64::from(framed.0) * scale).round() as u32,
        (f64::from(framed.1) * scale).round() as u32,
    );
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    view.show(
        &gpu,
        &mut stages,
        &mut cache,
        &src,
        1,
        frame,
        ViewSpec::Fit(out),
    )
    .unwrap();
    let base = BaseCurve::new(&src.color);
    view.tone(&gpu, &look, Some(&base), Some(&layer));
    view.color(&gpu, &look);
    let pixels = view
        .read_latest(&gpu)
        .unwrap()
        .iter()
        .map(|px| encode(px.map(f64::from), Target::Srgb, compress).map(|v| v as f32))
        .collect();
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "look/synthetic-cropped",
        genzo_golden::Space::DisplaySrgb,
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(&case, &genzo_golden::Image::new(out.0, out.1, pixels));
}

#[test]
fn golden_iphone_12_pro_standard() {
    let Some(path) = sdk_stage2::corpus_file("raw/Apple/iPhone 12 Pro/IMG_1361.DNG") else {
        return;
    };
    let Some((_lock, gpu)) = gpu() else { return };
    let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let src = RawSource::from_decoded(img).unwrap();
    assert!(
        BaseCurve::new(&src.color).curve_error.is_none()
            && src
                .color
                .profile_tone_curve
                .as_ref()
                .is_some_and(|c| c.len() == 514),
        "the file's own 257-point curve is used"
    );
    let look = Look::with_profile(Profile::GenzoStandard);
    check(&gpu, &src, &look, (400, 300), "look/iphone-12-pro-standard");
    let local = Look {
        highlights: -80.0,
        shadows: 70.0,
        ..look
    };
    check(
        &gpu,
        &src,
        &local,
        (400, 300),
        "look/iphone-12-pro-local-tone",
    );
}
