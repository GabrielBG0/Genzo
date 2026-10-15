//! The spike pipeline against the synthetic scene's ground truth.
//!
//! These compare with the true image, not with a stored render, so they hold
//! on any adapter. Without an adapter they are skipped, unless
//! GENZO_REQUIRE_GPU=1 (set in CI, which runs them on lavapipe).
//!
//! The tests take turns on the GPU: WARP crashes (access violation) when
//! several devices are alive at once. The app only ever opens one.

mod rcd_reference;

use std::sync::{Arc, Mutex, MutexGuard};

use genzo_pipeline::gpu::spike::{DisplaySpace, SpikeParams, SpikePipeline, Stage};
use genzo_pipeline::gpu::synthetic::{scene, scene_mosaic};
use genzo_pipeline::gpu::thread::RenderThread;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

/// The lock comes first in the tuple on purpose: bindings drop in reverse
/// order, so `let Some((_lock, gpu))` drops the device before the lock.
fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => {
            eprintln!(
                "adapter: {} ({}), software: {}",
                gpu.info.name, gpu.info.backend, gpu.info.software
            );
            Some((lock, gpu))
        }
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// The scene's expected 8-bit sRGB pixel.
fn expected(x: u32, y: u32, w: u32, h: u32) -> [f64; 3] {
    scene(x, y, w, h).map(|c| genzo_color::srgb_encode(c as f64))
}

struct Stats {
    psnr: f64,
    mean_de: f64,
    p99_de: f64,
}

/// PSNR (8-bit) and ΔE2000 of `rgba` against the scene, over the pixels for
/// which `keep` is true.
fn compare(rgba: &[u8], w: u32, h: u32, keep: impl Fn(u32, u32) -> bool) -> Stats {
    let (mut se, mut n) = (0.0, 0.0);
    let mut des = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if !keep(x, y) {
                continue;
            }
            let i = (y * w + x) as usize * 4;
            let got = [0, 1, 2].map(|c| rgba[i + c] as f64 / 255.0);
            let want = expected(x, y, w, h).map(|c| (c * 255.0).round() / 255.0);
            for c in 0..3 {
                se += ((got[c] - want[c]) * 255.0).powi(2);
                n += 1.0;
            }
            des.push(genzo_color::delta_e_2000_srgb(got, want));
        }
    }
    des.sort_by(f64::total_cmp);
    let mse = se / n;
    Stats {
        psnr: if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64 * 255.0 / mse).log10()
        },
        mean_de: des.iter().sum::<f64>() / des.len() as f64,
        p99_de: des[(des.len() - 1) * 99 / 100],
    }
}

const W: u32 = 768;
const H: u32 = 512;

#[test]
fn reproduces_the_scene() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &scene_mosaic(W, H));
    let f = p.render(&gpu, SpikeParams::default(), (W, H)).unwrap();
    assert_eq!((f.width, f.height), (W, H));

    // Gradients and patches, away from the edges RCD hands to the box filter
    // and away from the patch borders.
    let smooth = compare(&f.rgba, W, H, |x, y| {
        let v = y as f32 / H as f32;
        let u = x as f32 * 6.0 / W as f32;
        x >= 12
            && y >= 12
            && x < W - 12
            && v < 0.8
            && (v < 0.38 || (u.fract() > 0.05 && u.fract() < 0.95))
            && !(0.38..0.42).contains(&v)
            && !(0.58..0.62).contains(&v)
    });
    eprintln!(
        "smooth: PSNR {:.1} dB, ΔE mean {:.3}, p99 {:.3}",
        smooth.psnr, smooth.mean_de, smooth.p99_de
    );
    assert!(smooth.psnr > 45.0, "{}", smooth.psnr);
    assert!(smooth.p99_de < 1.0, "{}", smooth.p99_de);

    // Gradients and patches with every edge included. The band of fine
    // anti-diagonal lines below them is left out: each anti-diagonal of a
    // Bayer sensor holds only green or only red and blue samples, a worst
    // case where darktable's RCD is ΔE 18 from the truth too. The lines are
    // checked against the reference instead (rcd_matches_the_darktable_reference).
    let edges = compare(&f.rgba, W, H, |_, y| (y as f32) < H as f32 * 0.8);
    eprintln!(
        "with edges: PSNR {:.1} dB, ΔE mean {:.3}, p99 {:.3}",
        edges.psnr, edges.mean_de, edges.p99_de
    );
    assert!(edges.psnr > 35.0, "{}", edges.psnr);
    assert!(edges.mean_de < 0.5, "{}", edges.mean_de);
}

#[test]
fn reruns_only_the_stages_that_changed() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &scene_mosaic(W, H));
    let base = SpikeParams::default();
    assert_eq!(p.render(&gpu, base, (W, H)).unwrap().from, Stage::Linearize);

    let brighter = SpikeParams {
        exposure_ev: 1.0,
        ..base
    };
    let f = p.render(&gpu, brighter, (W, H)).unwrap();
    assert_eq!(f.from, Stage::Output);
    // A mid-gray patch, one stop brighter.
    let (x, y) = (W / 12, H * 7 / 10);
    let i = (y * W + x) as usize * 4;
    let want = genzo_color::srgb_encode(0.36) * 255.0;
    assert!(
        (f.rgba[i] as f64 - want).abs() <= 2.0,
        "{} vs {want}",
        f.rgba[i]
    );

    let p3 = SpikeParams {
        space: DisplaySpace::DisplayP3,
        ..brighter
    };
    assert_eq!(p.render(&gpu, p3, (W, H)).unwrap().from, Stage::Output);
    assert_eq!(
        p.render(&gpu, p3, (W / 2, H / 2)).unwrap().from,
        Stage::Viewport
    );
    let warmer = SpikeParams {
        wb: [1.2, 1.0, 0.9],
        ..p3
    };
    assert_eq!(
        p.render(&gpu, warmer, (W / 2, H / 2)).unwrap().from,
        Stage::Linearize
    );

    // Dragging white balance skips the demosaic; releasing re-runs it.
    let drag = SpikeParams {
        wb: [1.3, 1.0, 0.8],
        interactive: true,
        ..warmer
    };
    let dragged = p.render(&gpu, drag, (W / 2, H / 2)).unwrap();
    assert_eq!(dragged.from, Stage::Viewport);
    let settled = p
        .render(
            &gpu,
            SpikeParams {
                interactive: false,
                ..drag
            },
            (W / 2, H / 2),
        )
        .unwrap();
    assert_eq!(settled.from, Stage::Linearize);
    // On smooth content the two agree closely; patch edges may differ a bit.
    let (w, h) = (W / 2, H / 2);
    let mut de = 0.0;
    let mut n = 0.0;
    for y in 8..h * 2 / 5 {
        for x in 8..w - 8 {
            let i = (y * w + x) as usize * 4;
            let px = |f: &[u8]| [0, 1, 2].map(|c| f[i + c] as f64 / 255.0);
            de += genzo_color::delta_e_2000_srgb(px(&dragged.rgba), px(&settled.rgba));
            n += 1.0;
        }
    }
    assert!(de / n < 0.5, "interactive vs settled ΔE {}", de / n);
}

#[test]
fn display_p3_output_differs_only_in_saturated_colors() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &scene_mosaic(W, H));
    let srgb = p.render(&gpu, SpikeParams::default(), (W, H)).unwrap();
    let p3 = p
        .render(
            &gpu,
            SpikeParams {
                space: DisplaySpace::DisplayP3,
                ..Default::default()
            },
            (W, H),
        )
        .unwrap();
    let px = |f: &[u8], x: u32, y: u32| {
        let i = (y * W + x) as usize * 4;
        [f[i], f[i + 1], f[i + 2]]
    };
    // A neutral patch is the same in both; the red patch is less saturated in P3.
    let gray = (W / 12, H * 7 / 10);
    let red = (W / 12, H / 2);
    let d = |a: [u8; 3], b: [u8; 3]| {
        (0..3)
            .map(|c| (a[c] as i32 - b[c] as i32).abs())
            .max()
            .unwrap()
    };
    assert!(d(px(&srgb.rgba, gray.0, gray.1), px(&p3.rgba, gray.0, gray.1)) <= 1);
    let (r_s, r_p) = (px(&srgb.rgba, red.0, red.1), px(&p3.rgba, red.0, red.1));
    assert!(
        r_p[0] < r_s[0] && r_p[1] > r_s[1],
        "sRGB {r_s:?} P3 {r_p:?}"
    );
}

#[test]
fn render_thread_coalesces_to_the_latest() {
    let Some((_lock, gpu)) = gpu() else { return };
    drop(gpu);
    let thread = Arc::new(RenderThread::spawn(AdapterChoice::from_env()).unwrap());
    thread.set_image(Arc::new(scene_mosaic(W, H))).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let t = Arc::clone(&thread);
            std::thread::spawn(move || {
                t.render(
                    SpikeParams {
                        exposure_ev: i as f32 * 0.1,
                        ..Default::default()
                    },
                    (W, H),
                )
                .unwrap()
            })
        })
        .collect();
    let frames: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // Every request is answered, and at least one of them with a frame.
    assert!(frames.iter().any(Option::is_some));
}

/// Writes the scene render to GENZO_DUMP (raw RGBA8) for a visual check.
#[test]
#[ignore = "debug aid"]
fn dump_scene() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &scene_mosaic(W, H));
    let f = p.render(&gpu, SpikeParams::default(), (W, H)).unwrap();
    std::fs::write(std::env::var("GENZO_DUMP").unwrap(), &f.rgba).unwrap();
}

/// The WGSL RCD against the literal port of darktable's rcd.c.
#[test]
fn rcd_matches_the_darktable_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let img = scene_mosaic(W, H);
    let mut p = SpikePipeline::new(&gpu);
    p.set_image(&gpu, &img);
    let f = p.render(&gpu, SpikeParams::default(), (W, H)).unwrap();

    let range = img.white - img.black;
    let cfa: Vec<f32> = img
        .data
        .iter()
        .map(|&c| (c as f32 - img.black).max(0.0) / range)
        .collect();
    let (w, h) = (W as usize, H as usize);
    let half = rcd_reference::rcd(&cfa, w, h, img.cfa, rcd_reference::PqMode::HalfWidth);
    let reference = rcd_reference::rcd(&cfa, w, h, img.cfa, rcd_reference::PqMode::PerPixel);
    let to_srgb = genzo_color::mat_mul(
        &genzo_color::invert(&genzo_color::SRGB_TO_XYZ).unwrap(),
        &genzo_color::WORKING_TO_XYZ,
    );

    // Per region: GPU against the reference, and the reference against the truth.
    let region = |y: u32| match y as f32 / H as f32 {
        v if v < 0.4 => 0,
        v if v < 0.8 => 1,
        _ => 2,
    };
    let mut stats = [(0.0f64, 0.0f64, 0.0f64, 0.0f64, (0u32, 0u32)); 3];
    let mut big = 0;
    let mut half_de = 0.0;
    for y in 12..H - 12 {
        for x in 12..W - 12 {
            let lin = reference[(y * W + x) as usize].map(f64::from);
            let encode = |lin: [f64; 3]| {
                genzo_color::mul(&to_srgb, lin)
                    .map(|c| (genzo_color::srgb_encode(c.clamp(0.0, 1.0)) * 255.0).round() / 255.0)
            };
            let want = encode(lin);
            let i = (y * W + x) as usize * 4;
            let got = [0, 1, 2].map(|c| f.rgba[i + c] as f64 / 255.0);
            let de = genzo_color::delta_e_2000_srgb(got, want);
            half_de += genzo_color::delta_e_2000_srgb(
                encode(half[(y * W + x) as usize].map(f64::from)),
                want,
            );
            let truth = genzo_color::delta_e_2000_srgb(want, expected(x, y, W, H));
            let s = &mut stats[region(y)];
            s.0 += de;
            s.1 += truth;
            s.2 += 1.0;
            if de > s.3 {
                s.3 = de;
                s.4 = (x, y);
            }
            if de > 1.0 {
                big += 1;
            }
        }
    }
    for (name, s) in ["gradients", "patches", "lines"].iter().zip(stats) {
        eprintln!(
            "{name}: GPU vs rcd.c ΔE mean {:.4} max {:.2} at {:?}; rcd.c (8-bit) vs truth ΔE mean {:.3}",
            s.0 / s.2,
            s.3,
            s.4,
            s.1 / s.2
        );
    }
    let (sum, n) = stats.iter().fold((0.0, 0.0), |a, s| (a.0 + s.0, a.1 + s.2));
    eprintln!("pixels with ΔE > 1 against rcd.c: {big}");
    eprintln!(
        "per-pixel vs darktable's half-width P/Q: ΔE mean {:.4}",
        half_de / n
    );
    assert!(sum / n < 0.1, "mean ΔE {}", sum / n);
    // Tolerance: one 8-bit step after f16 storage, which in dark saturated
    // colors reaches ΔE 1 to 2. Measured: max 0.97 (RTX 3060), 1.05 (WARP).
    let worst = stats.iter().map(|s| s.3).fold(0.0, f64::max);
    assert!(worst < 2.0, "max ΔE {worst}");
    assert!(
        big as f64 <= n * 1e-4,
        "{big} pixels more than ΔE 1 from the reference"
    );
    assert!(half_de / n < 0.02, "{}", half_de / n);
}
