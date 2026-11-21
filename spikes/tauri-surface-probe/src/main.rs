//! Probe 1b and the K4 to K7 harness of docs/native-surface-spike.md.
//!
//! The window is the occlusion probe's: opaque, with a transparent webview
//! added through Tauri's `unstable` split. A harness thread then runs the
//! real render thread on a synthetic RAW and drags the exposure, either
//! presenting each frame to the window's surface (`HARNESS=native`, the
//! default) or reading it back as today (`HARNESS=readback`), and prints the
//! time per step. //!
//! Env: HARNESS, STEPS (default 300), PHOTO_MP (default 24),
//! GENZO_PRESENT=fifo|mailbox|immediate, GENZO_ADAPTER=software,
//! WEBVIEW2_USER_DATA_FOLDER (a fresh folder if 0x800700AA appears).
use std::time::{Duration, Instant};

use genzo_color::output::Target;
use genzo_model::edit::{Edit, Phase};
use genzo_pipeline::gpu::AdapterChoice;
use genzo_pipeline::gpu::develop::{Photo, as_shot};
use genzo_pipeline::gpu::surface::Pacing;
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::gpu::thread::{DevelopRequest, RenderThread};
use genzo_pipeline::histogram::Overlay;
use genzo_pipeline::reference::sampling::ViewSpec;
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{LogicalPosition, LogicalSize, WebviewUrl};

#[tauri::command]
fn clicked(n: u32) {
    println!("clicked {n}");
}

/// The page's hole, as it last reported it: (x, y, w, h) in device pixels.
static HOLE: std::sync::Mutex<Option<(i32, i32, u32, u32)>> = std::sync::Mutex::new(None);

#[tauri::command]
fn hole_rect(x: i32, y: i32, w: u32, h: u32) {
    *HOLE.lock().unwrap() = Some((x, y, w, h));
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![clicked, hole_rect])
        .setup(|app| {
            // WEBVIEW=child (default: transparent child webview in an opaque
            // window), child_opaque, or window (the ordinary builder, as the
            // shell uses today). For comparing resize behavior.
            let mode = std::env::var("WEBVIEW").unwrap_or_else(|_| "child".into());
            let window = if mode == "window" {
                let w = tauri::WebviewWindowBuilder::new(app, "main", WebviewUrl::App(std::env::var("PAGE").unwrap_or_else(|_| "index.html".into()).into()))
                    .inner_size(1200.0, 800.0)
                    .build()?;
                w.as_ref().window().clone()
            } else {
                let window = WindowBuilder::new(app, "main")
                    .inner_size(1200.0, 800.0)
                    .build()?;
                let size = window
                    .inner_size()?
                    .to_logical::<f64>(window.scale_factor()?);
                window.add_child(
                    WebviewBuilder::new("web", WebviewUrl::App(std::env::var("PAGE").unwrap_or_else(|_| "index.html".into()).into()))
                        .transparent(mode != "child_opaque")
                        .auto_resize(),
                    LogicalPosition::new(0.0, 0.0),
                    LogicalSize::new(size.width, size.height),
                )?;
                window
            };
            let target = window.clone();
            std::thread::spawn(move || harness(target));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run");
}

/// The hole the page leaves, in device pixels: (origin, size).
fn hole(window: &tauri::Window) -> ((i32, i32), (u32, u32)) {
    let s = window.scale_factor().unwrap();
    let inner = window.inner_size().unwrap();
    let origin = ((200.0 * s) as i32, (60.0 * s) as i32);
    let w = inner.width as i32 - (260.0 * s) as i32;
    let h = inner.height as i32 - (120.0 * s) as i32;
    (origin, (w.max(1) as u32, h.max(1) as u32))
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn harness(window: tauri::Window) {
    let mode = std::env::var("HARNESS").unwrap_or_else(|_| "native".into());
    if mode == "color" {
        return color_test(window);
    }
    let steps: usize = std::env::var("STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let mp: f64 = std::env::var("PHOTO_MP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24.0);
    let w = ((mp * 1e6 * 1.5).sqrt() as u32) & !1;
    let h = ((mp * 1e6 / 1.5).sqrt() as u32) & !1;

    for m in window.available_monitors().unwrap() {
        println!(
            "monitor {:?}: {:?} at {:?}, scale {}",
            m.name(),
            m.size(),
            m.position(),
            m.scale_factor()
        );
    }
    let t = RenderThread::spawn(AdapterChoice::from_env()).expect("render thread");
    println!("adapter: {:?}", t.info());
    let native = mode == "native";
    let (origin, mut view) = hole(&window);
    if let Ok(v) = std::env::var("VIEW") {
        // WxH: a view larger than the hole, to time a 2560 px viewport on a
        // small screen (the frame is clipped to the window when shown).
        let (a, b) = v.split_once('x').expect("VIEW=WxH");
        view = (a.parse().unwrap(), b.parse().unwrap());
    }
    if native {
        let inner = window.inner_size().unwrap();
        println!(
            "surface: {}",
            t.attach_surface(
                surface_on_main(&window, &t),
                (inner.width, inner.height),
                Pacing::from_env(),
                color_space_from_env(),
            )
            .expect("attach surface")
        );
        t.layout_surface((inner.width, inner.height), origin).unwrap();
    }
    let photo = Photo {
        src: with_camera_profile(scene_raw(w, h)),
        orientation: 1,
    };
    let (_, kind) = as_shot(&photo.src);
    t.open(1, photo).expect("open");
    let mut edit = Edit::defaults(kind);
    println!("{mode}: {w}x{h} photo, view {view:?} at {origin:?}, {steps} steps");

    let request = |generation: u64, edit: &Edit, phase: Phase| DevelopRequest {
        generation,
        photo: 1,
        edit: edit.clone(),
        view: ViewSpec::Fit(view),
        target: Target::Srgb,
        phase,
        overlay: Overlay::default(),
    };
    // The first frame builds the pyramid and the view; it is not a drag step.
    t.develop(request(1, &edit, Phase::Start)).expect("first").expect("frame");
    if std::env::var("ANIM").is_ok() {
        // Follow the page's reported hole and the window's own size, as the
        // shell will from the page's reports and the window's Resized and
        // ScaleFactorChanged events: a new hole is a render at the new view
        // size; either is a layout.
        println!("following the window; kill the process to end");
        let mut last_hole = None;
        let mut last_inner = (0, 0);
        let mut generation = 100u64;
        loop {
            std::thread::sleep(Duration::from_millis(1));
            let hole_now = *HOLE.lock().unwrap();
            let inner = window.inner_size().unwrap();
            let inner = (inner.width, inner.height);
            let Some((x, y, w, h)) = hole_now else { continue };
            if hole_now == last_hole && inner == last_inner {
                continue;
            }
            let hole_changed = hole_now != last_hole;
            last_hole = hole_now;
            last_inner = inner;
            t.layout_surface(inner, (x, y)).unwrap();
            if hole_changed {
                generation += 1;
                let _ = t.develop(DevelopRequest {
                    generation,
                    photo: 1,
                    edit: edit.clone(),
                    view: ViewSpec::Fit((w, h)),
                    target: Target::Srgb,
                    phase: Phase::End,
                    overlay: Overlay::default(),
                });
            }
        }
    }
    if std::env::var("RESIZE").is_ok() {
        // Follow the window as the real app will: a new hole means a new
        // layout and a render at the new view size.
        println!("following the window; kill the process to end");
        let mut last = (origin, view);
        let mut generation = 100u64;
        loop {
            std::thread::sleep(Duration::from_millis(2));
            let now = hole(&window);
            if now == last {
                continue;
            }
            last = now;
            let inner = window.inner_size().unwrap();
            t.layout_surface((inner.width, inner.height), now.0).unwrap();
            generation += 1;
            let r = DevelopRequest {
                generation,
                photo: 1,
                edit: edit.clone(),
                view: ViewSpec::Fit(now.1),
                target: Target::Srgb,
                phase: Phase::End,
                overlay: Overlay::default(),
            };
            let _ = t.develop(r);
        }
    }
    let (mut total, mut acquire, mut submit, mut gpu) = (vec![], vec![], vec![], vec![]);
    for i in 0..steps {
        edit.exposure = ((i as f32 * 0.07).sin() * 1.5).clamp(-5.0, 5.0);
        let t0 = Instant::now();
        let f = t
            .develop(request(2 + i as u64, &edit, Phase::Update))
            .expect("step")
            .expect("frame");
        total.push(t0.elapsed().as_secs_f64() * 1e3);
        if let genzo_pipeline::gpu::develop::Readback::Frame { size, half, .. } = &f.readback {
            // A Fit frame is the photo's aspect inside the view. Natively it
            // is never the half-size preview; read back it is, in a drag.
            assert!(!native || (!half && size.0 <= view.0 && size.1 <= view.1 && (size.0 == view.0 || size.1 == view.1)),
                "native drag frame is {size:?} half={half}, view {view:?}");
            if i == 0 {
                println!("  drag frame {size:?}, half-size preview: {half}");
            }
        }
        if let Some(p) = f.presented {
            acquire.push(p.acquire_ms);
            submit.push(p.submit_ms);
        }
        if let Some(g) = f.gpu_ms {
            gpu.push(g);
        }
        if i == 0 && native {
            // Let the first drag frame be seen before the rest runs by.
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    let report = |name: &str, v: &mut Vec<f64>| {
        if !v.is_empty() {
            println!(
                "  {name:<10} p50 {:6.2} ms  p95 {:6.2} ms  max {:6.2} ms",
                percentile(v, 0.5),
                percentile(v, 0.95),
                percentile(v, 1.0)
            );
        }
    };
    report("step", &mut total);
    report("acquire", &mut acquire);
    report("submit", &mut submit);
    report("gpu", &mut gpu);
    // The settled frame, as on release.
    let t0 = Instant::now();
    t.develop(request(10_000, &edit, Phase::End)).expect("settle").expect("frame");
    println!("  settled frame {:.2} ms", t0.elapsed().as_secs_f64() * 1e3);

    if std::env::var("HOLD").is_ok() {
        std::thread::sleep(Duration::from_secs(
            std::env::var("HOLD").unwrap().parse().unwrap_or(5),
        ));
    }
    // K7: the surface goes before the window does.
    if native {
        t.detach_surface().expect("detach");
        println!("surface detached");
    }
    drop(t);
    println!("render thread joined; closing the window");
    window.close().ok();
    std::thread::sleep(Duration::from_millis(500));
    std::process::exit(0);
}


/// `COLORSPACE=auto|srgb|p3`: how the swapchain is tagged.
fn color_space_from_env() -> wgpu::SurfaceColorSpace {
    match std::env::var("COLORSPACE").as_deref() {
        Ok("srgb") => wgpu::SurfaceColorSpace::Srgb,
        Ok("p3") => wgpu::SurfaceColorSpace::DisplayP3,
        _ => wgpu::SurfaceColorSpace::Auto,
    }
}

/// The sample points, as fractions of the frame, over the scene's patches.
const SAMPLES: [(f64, f64); 8] = [
    (0.08, 0.60), (0.25, 0.60), (0.42, 0.60), (0.58, 0.60),
    (0.75, 0.60), (0.92, 0.60), (0.25, 0.80), (0.58, 0.80),
];

/// The mechanism test of docs/native-surface-spike.md (color): one settled
/// frame, encoded for `FRAME_TARGET=srgb|p3`, either read back (the bytes,
/// without NATIVE=1) or presented to a swapchain tagged `COLORSPACE`
/// (with NATIVE=1) and held for HOLD seconds, so a capture of the screen can
/// be compared with the bytes.
fn color_test(window: tauri::Window) {
    let native = std::env::var("NATIVE").is_ok();
    let target = match std::env::var("FRAME_TARGET").as_deref() {
        Ok("p3") => Target::DisplayP3,
        _ => Target::Srgb,
    };
    let t = RenderThread::spawn(AdapterChoice::from_env()).expect("render thread");
    println!("adapter: {:?}", t.info());
    // Time for a script to move the window to the monitor under test.
    let delay: u64 = std::env::var("START_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    std::thread::sleep(Duration::from_secs(delay));
    println!("window scale {}", window.scale_factor().unwrap());
    println!(
        "window inner position {:?} inner size {:?} outer position {:?}",
        window.inner_position().unwrap(),
        window.inner_size().unwrap(),
        window.outer_position().unwrap()
    );
    let (origin, view) = hole(&window);
    if native {
        let inner = window.inner_size().unwrap();
        println!(
            "surface: {}",
            t.attach_surface(
                surface_on_main(&window, &t),
                (inner.width, inner.height),
                Pacing::from_env(),
                color_space_from_env(),
            )
            .expect("attach surface")
        );
        t.layout_surface((inner.width, inner.height), origin).unwrap();
    }
    let photo = Photo {
        src: with_camera_profile(scene_raw(3000, 2000)),
        orientation: 1,
    };
    let (_, kind) = as_shot(&photo.src);
    t.open(1, photo).expect("open");
    let edit = Edit::defaults(kind);
    // PRESENTS=n: how many times the same frame is rendered and presented
    // (a DX12 swapchain showed nothing after a single present).
    let presents: u64 = std::env::var("PRESENTS").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut last = None;
    for g in 1..=presents {
        last = t
            .develop(DevelopRequest {
                generation: g,
                photo: 1,
                edit: edit.clone(),
                view: ViewSpec::Fit(view),
                target,
                phase: Phase::End,
                overlay: Overlay::default(),
            })
            .expect("render");
        std::thread::sleep(Duration::from_millis(200));
    }
    let f = last.expect("frame");
    println!("target {target:?}, presented: {}", f.presented.is_some());
    println!("frame origin on screen (client px): {origin:?}");
    if let genzo_pipeline::gpu::develop::Readback::Frame { size, pixels, .. } = &f.readback {
        println!("frame {size:?}");
        if !pixels.is_empty() {
            for (i, (fx, fy)) in SAMPLES.iter().enumerate() {
                let (x, y) = ((size.0 as f64 * fx) as u32, (size.1 as f64 * fy) as u32);
                let p = pixels[(y * size.0 + x) as usize];
                println!("sample {i} at ({x},{y}): {} {} {}", p[0], p[1], p[2]);
            }
        } else {
            println!("sample points: {:?}", SAMPLES.iter().map(|(fx, fy)| ((size.0 as f64 * fx) as u32, (size.1 as f64 * fy) as u32)).collect::<Vec<_>>());
        }
    }
    let hold: u64 = std::env::var("HOLD").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
    std::thread::sleep(Duration::from_secs(hold));
    if native {
        t.detach_surface().ok();
    }
    drop(t);
    window.close().ok();
    std::thread::sleep(Duration::from_millis(500));
    std::process::exit(0);
}

/// A surface for the window, made on the main thread (Metal insists on it).
fn surface_on_main(window: &tauri::Window, t: &RenderThread) -> wgpu::Surface<'static> {
    let maker = t.surface_maker();
    let target = window.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || {
            let _ = tx.send(maker.create(target.into()));
        })
        .expect("main thread");
    rx.recv().expect("surface").expect("create surface")
}
