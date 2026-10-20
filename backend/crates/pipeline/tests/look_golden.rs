//! Stored goldens of stages 6 and 7 (DEV-02 criterion 6, DEV-01 criterion
//! 5), written on lavapipe: the stage 7 cache through color::output's
//! stage 9 on the CPU, as display floats. The 8-bit frame is not compared:
//! its dither flips a code wherever devices differ by a hair, which on a
//! dark gray turns neutral into ΔE 2 (Metal against lavapipe, measured);
//! stage 9 and the dither have their own golden (view/synthetic-fit-srgb).
//!
//! - the synthetic scene with Genzo Standard and tone controls;
//! - the synthetic scene with Genzo Linear and the presence controls;
//! - the iPhone 12 Pro ProRAW (CC0) with Genzo Standard, through the file's
//!   own ProfileToneCurve. CI runs this file after fetching the corpus
//!   (GENZO_REQUIRE_CORPUS=1); elsewhere a missing file skips.

#[allow(dead_code)]
mod sdk_stage2;

use std::sync::{Mutex, MutexGuard};

use genzo_color::output::{Target, encode};
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::{ViewStages, compresses};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::RawSource;

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
    view.sample(gpu, p, 1, fit);
    let base = BaseCurve::new(&src.color);
    view.tone(gpu, look, Some(&base));
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
}
