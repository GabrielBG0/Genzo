//! Stages 6 and 7 on the GPU (DEV-01; the base curve, DEV-02) against
//! `reference::look`, from the same sampled values: within two f16 steps.
//! Criterion 3 of DEV-02 asks one step for the base curve alone; the extra
//! step is the stage 6 cache's own f16 rounding before stage 7 reads it.

use std::sync::{Mutex, MutexGuard};

use genzo_imgio::raw::CameraColor;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::scene_raw;
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::reference::look::{stage6, stage7};

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

/// Two f16 steps of the larger magnitude, plus 2e-5 for the GPU's f32
/// log2, exp2 and cube roots.
fn close(got: f32, want: f64) -> bool {
    let step = (want.abs() as f32).max(6.2e-5) / 1024.0;
    (got - want as f32).abs() <= 2.0 * step + 2e-5
}

fn looks() -> Vec<(&'static str, Look)> {
    let standard = Look::with_profile(Profile::GenzoStandard);
    let linear = Look::with_profile(Profile::GenzoLinear);
    vec![
        ("standard, defaults", standard),
        (
            "standard, every tone control",
            Look {
                exposure: 1.3,
                contrast: 60.0,
                whites: -40.0,
                blacks: 70.0,
                ..standard
            },
        ),
        (
            "standard, the far ends",
            Look {
                exposure: -0.7,
                contrast: -100.0,
                whites: 100.0,
                blacks: -100.0,
                ..standard
            },
        ),
        (
            "linear, tone and saturation",
            Look {
                exposure: 0.5,
                contrast: 100.0,
                blacks: 100.0,
                saturation: -30.0,
                vibrance: 100.0,
                ..linear
            },
        ),
        (
            "standard, presence",
            Look {
                vibrance: -60.0,
                saturation: 80.0,
                ..standard
            },
        ),
        (
            "standard, B&W",
            Look {
                monochrome: true,
                vibrance: 50.0,
                exposure: 0.25,
                ..standard
            },
        ),
        (
            "embedded, presence",
            Look {
                vibrance: 40.0,
                saturation: 20.0,
                ..Look::with_profile(Profile::Embedded)
            },
        ),
    ]
}

/// A ProfileToneCurve like Apple's, darker in the midtones.
fn with_profile_curve(c: &CameraColor) -> CameraColor {
    CameraColor {
        profile_tone_curve: Some(vec![0.0, 0.0, 0.1, 0.04, 0.3, 0.22, 0.7, 0.72, 1.0, 1.0]),
        ..c.clone()
    }
}

/// The synthetic scene spans luminance 0.026 to 0.63; BaselineExposures of
/// −3, 0 and +2.5 EV take it from below the ramp's toe to past white.
const BASELINE_EXPOSURES: [f64; 3] = [-3.0, 0.0, 2.5];

fn scene_at(baseline_exposure: f64) -> genzo_pipeline::raw::RawSource {
    let mut src = scene_raw(640, 480);
    src.color.baseline_exposure = baseline_exposure;
    src
}

#[test]
fn stages_6_and_7_match_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    for be in BASELINE_EXPOSURES {
        let src = scene_at(be);
        let p = stages
            .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
            .unwrap();
        let out = (320, 240);
        view.sample(&gpu, p, 1, out);
        let sampled = view.read_latest(&gpu).unwrap();
        let curves = [
            ("ACR3", BaseCurve::new(&src.color)),
            (
                "a profile curve",
                BaseCurve::new(&with_profile_curve(&src.color)),
            ),
        ];
        for (curve_name, base) in &curves {
            for (name, look) in looks() {
                if *curve_name != "ACR3" && look.profile != Profile::GenzoStandard {
                    continue;
                }
                view.tone(&gpu, &look, Some(base));
                let toned = view.read_latest(&gpu).unwrap();
                view.color(&gpu, &look);
                let colored = view.read_latest(&gpu).unwrap();
                let mut worst = (0f64, 0usize);
                for i in 0..sampled.len() {
                    let s = sampled[i].map(f64::from);
                    let want6 = stage6(s, &look, Some(base));
                    for c in 0..3 {
                        assert!(
                            close(toned[i][c], want6[c]),
                            "{name} ({curve_name}), stage 6, pixel {i} channel {c}: {} vs {} from {s:?}",
                            toned[i][c],
                            want6[c]
                        );
                    }
                    // Stage 7 from what the stage 6 cache holds.
                    let want7 = stage7(toned[i].map(f64::from), &look);
                    for c in 0..3 {
                        let d = (f64::from(colored[i][c]) - want7[c]).abs();
                        if d > worst.0 {
                            worst = (d, i);
                        }
                        assert!(
                            close(colored[i][c], want7[c]),
                            "{name} ({curve_name}), stage 7, pixel {i} channel {c}: {} vs {}",
                            colored[i][c],
                            want7[c]
                        );
                    }
                }
                eprintln!(
                    "BE {be:+}, {name} ({curve_name}): stage 7 worst {:.2e}",
                    worst.0
                );
            }
        }
    }
}

/// Skipping stages 6 and 7 shows the sampled cache; running stage 7 alone
/// reads it too.
#[test]
fn stages_run_from_the_latest_cache() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(200, 150);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages
        .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
        .unwrap();
    let mut view = ViewStages::new(&gpu);
    view.sample(&gpu, p, 1, (100, 75));
    let sampled = view.read_latest(&gpu).unwrap();
    let mono = Look {
        monochrome: true,
        ..Look::with_profile(Profile::GenzoLinear)
    };
    view.color(&gpu, &mono);
    let colored = view.read_latest(&gpu).unwrap();
    for (s, c) in sampled.iter().zip(&colored) {
        let want = stage7(s.map(f64::from), &mono);
        assert!(close(c[0], want[0]) && c[0] == c[1] && c[1] == c[2]);
    }
    // Stage 7 again, still with no stage 6: from the sampled cache.
    view.color(&gpu, &mono);
    assert_eq!(view.read_latest(&gpu).unwrap(), colored);
    // A new sampling starts over from the sampled cache.
    view.sample(&gpu, p, 1, (100, 75));
    assert_eq!(view.read_latest(&gpu).unwrap(), sampled);
    // Stage 6 of an earlier sampling is not reused: after tone, a new
    // sampling, then stage 7 twice reads the new sampling.
    let bright = Look {
        exposure: 2.0,
        ..Look::with_profile(Profile::GenzoLinear)
    };
    view.tone(&gpu, &bright, None);
    view.sample(&gpu, p, 1, (100, 75));
    view.color(&gpu, &mono);
    view.color(&gpu, &mono);
    assert_eq!(view.read_latest(&gpu).unwrap(), colored);
}

/// The comparison can fail: the test scene spans the tone range, and the
/// reference for other settings is far from what the GPU made.
#[test]
fn the_comparison_tells_settings_apart() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for be in BASELINE_EXPOSURES {
        let src = scene_at(be);
        let p = stages
            .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
            .unwrap();
        view.sample(&gpu, p, 1, (320, 240));
        for s in view.read_latest(&gpu).unwrap() {
            let y = genzo_color::look::luminance(s.map(f64::from));
            (lo, hi) = (lo.min(y), hi.max(y));
        }
    }
    eprintln!("sampled luminance {lo:.2e} to {hi:.3}");
    assert!(
        lo < 0.004 && hi > 1.0,
        "the scenes span from below the black to past white"
    );
    let src = scene_at(0.0);
    let p = stages
        .render_pyramid(&gpu, &src, &camera_transform(&src.color, None))
        .unwrap();
    view.sample(&gpu, p, 1, (320, 240));
    let sampled = view.read_latest(&gpu).unwrap();
    let base = BaseCurve::new(&src.color);
    let look = Look {
        contrast: 30.0,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    view.tone(&gpu, &look, Some(&base));
    let toned = view.read_latest(&gpu).unwrap();
    let other = Look {
        contrast: 25.0,
        ..look
    };
    let differ = sampled
        .iter()
        .zip(&toned)
        .filter(|(s, t)| {
            let want = stage6(s.map(f64::from), &other, Some(&base));
            (0..3).any(|c| !close(t[c], want[c]))
        })
        .count();
    assert!(
        differ > sampled.len() / 4,
        "{differ} of {} differ",
        sampled.len()
    );
}
