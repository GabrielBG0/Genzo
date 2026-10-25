//! `genzo-cli`: drives the engine without a UI.
//!
//! The cli must be able to do everything the UI does except draw. In M0 that
//! is the contract tooling, engine info, decoding, and the benchmarks for the
//! test pattern and the GPU spike pipeline.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use genzo_engine::api::{Command, Event, PixelFormat, SpikeSource};
use genzo_engine::develop::{Applied, Target, ViewSpec};
use genzo_engine::{Engine, contract, files};

const USAGE: &str = "\
usage: genzo-cli <command>

commands:
  version                      print engine and contract versions
  info                         print engine info as JSON
  contract write [DIR]         regenerate contract/ (refuses without a version bump)
  contract check [DIR]         fail if contract/ is stale
  inspect FILE                 decode a file, print what it holds and how long it took
  bench test-pattern [W H N]   render N test-pattern frames of W×H, print p50/p95
  bench decode FILE [T N]      decode FILE N times with T threads, print p50/p95
  bench gpu [W H N]            render a W×H synthetic raw into a 2560 px viewport,
                               N steps per slider, print GPU and engine p50/p95
  bench open FOLDER [W H N]    open FOLDER at Fit in a W×H view, N times: first
                               pixels, the fit render of photos not prefetched and
                               the switch to a prefetched neighbor, p50/p95

Every bench command also takes `--json FILE`: it writes the p50/p95 of each
metric there, for scripts/check-bench.py to compare against a stored baseline.
";

fn default_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contract")
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let json_out = take_json_flag(&mut args);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut metrics = Metrics::default();
    let result = match args.as_slice() {
        ["version"] => {
            let info = Engine::headless().info();
            println!(
                "engine {} contract {}",
                info.engine_version, info.contract_version
            );
            Ok(())
        }
        ["info"] => {
            let info = Engine::new().info();
            println!(
                "{}",
                serde_json::to_string_pretty(&info).expect("serializable")
            );
            Ok(())
        }
        ["contract", "write", rest @ ..] => {
            let dir = rest
                .first()
                .map_or_else(default_contract_dir, PathBuf::from);
            contract::write(&dir)
                .map(|changed| {
                    println!(
                        "{}",
                        if changed {
                            "contract written"
                        } else {
                            "contract unchanged"
                        }
                    );
                })
                .map_err(|e| e.to_string())
        }
        ["contract", "check", rest @ ..] => {
            let dir = rest
                .first()
                .map_or_else(default_contract_dir, PathBuf::from);
            contract::check(&dir)
                .map(|()| println!("contract up to date"))
                .map_err(|e| e.to_string())
        }
        ["inspect", file] => files::inspect(file.as_ref())
            .map(|r| {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&r).expect("serializable")
                )
            })
            .map_err(|e| e.to_string()),
        ["bench", "test-pattern", rest @ ..] => bench_test_pattern(rest, &mut metrics),
        ["bench", "decode", file, rest @ ..] => bench_decode(file, rest, &mut metrics),
        ["bench", "gpu", rest @ ..] => bench_gpu(rest, &mut metrics),
        ["bench", "open", folder, rest @ ..] => bench_open(folder, rest, &mut metrics),
        _ => Err(USAGE.to_owned()),
    };
    let result = result.and_then(|()| match &json_out {
        Some(path) => metrics.write(path),
        None => Ok(()),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn parse_num(args: &[&str], i: usize, default: u32) -> Result<u32, String> {
    args.get(i).map_or(Ok(default), |s| {
        s.parse().map_err(|_| format!("not a number: {s}"))
    })
}

/// Removes `--json FILE` from the arguments and returns FILE.
fn take_json_flag(args: &mut Vec<String>) -> Option<PathBuf> {
    let i = args.iter().position(|a| a == "--json")?;
    let path = args.get(i + 1)?.clone();
    args.drain(i..=i + 1);
    Some(path.into())
}

/// The p50 and p95 of each benchmarked metric, in milliseconds.
#[derive(Default)]
struct Metrics(Vec<(String, f64, f64)>);

impl Metrics {
    fn add(&mut self, name: impl Into<String>, p50: f64, p95: f64) {
        self.0.push((name.into(), p50, p95));
    }

    fn write(&self, path: &PathBuf) -> Result<(), String> {
        let rows: serde_json::Map<String, serde_json::Value> = self
            .0
            .iter()
            .map(|(name, p50, p95)| {
                (
                    name.clone(),
                    serde_json::json!({ "p50_ms": p50, "p95_ms": p95 }),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
        std::fs::write(
            path,
            text + "
",
        )
        .map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn percentiles(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    (pct(0.5), pct(0.95))
}

/// Full-image decode time for one file, with a given number of decode threads.
fn bench_decode(file: &str, args: &[&str], metrics: &mut Metrics) -> Result<(), String> {
    let threads = parse_num(args, 0, 4)?.clamp(1, 64) as u8;
    let n = parse_num(args, 1, 10)?.max(1);
    let times = files::time_decode(file.as_ref(), threads, n).map_err(|e| e.to_string())?;
    let (p50, p95) = percentiles(times);
    metrics.add(format!("decode/{threads} threads"), p50, p95);
    println!("decode {file}, {threads} threads, {n} runs: p50 {p50:.2} ms, p95 {p95:.2} ms");
    Ok(())
}

/// Engine-side cost of one frame: render plus tile encoding. The end-to-end
/// latency (IPC, upload, screen) is measured in the app, see 0.E.
fn bench_test_pattern(args: &[&str], metrics: &mut Metrics) -> Result<(), String> {
    let num = |i: usize, default: u32| -> Result<u32, String> {
        args.get(i).map_or(Ok(default), |s| {
            s.parse().map_err(|_| format!("not a number: {s}"))
        })
    };
    let (width, height, n) = (num(0, 2560)?, num(1, 1440)?, num(2, 30)?.max(1));
    let engine = Engine::headless();
    let mut times = Vec::with_capacity(n as usize);
    for i in 0..n {
        let started = Instant::now();
        let events = engine
            .execute(Command::RenderTestPattern {
                width,
                height,
                phase: i as f32 / n as f32,
                format: PixelFormat::Rgba8Srgb,
            })
            .map_err(|e| e.to_string())?;
        let Some(Event::RenderFinished { generation, .. }) = events.first() else {
            return Err("render produced no frame".into());
        };
        let bytes = engine.frame(*generation).ok_or("frame missing")?;
        std::hint::black_box(bytes);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let (p50, p95) = percentiles(times);
    metrics.add(format!("test-pattern/{width}x{height}"), p50, p95);
    println!("test-pattern {width}x{height}, {n} frames: p50 {p50:.2} ms, p95 {p95:.2} ms");
    Ok(())
}

/// The 0.D measurement: what one slider step costs on the GPU at 2560 px.
/// An exposure drag re-runs only the output stage; a white balance drag
/// re-runs everything from the raw data (linearize, demosaic, mips).
fn bench_gpu(args: &[&str], metrics: &mut Metrics) -> Result<(), String> {
    let (width, height, n) = (
        parse_num(args, 0, 8064)?,
        parse_num(args, 1, 6048)?,
        parse_num(args, 2, 50)?.max(2),
    );
    let engine = Engine::new();
    let info = engine.info().adapter;
    println!("adapter: {} ({}, {:?})", info.name, info.backend, info.kind);
    if let Some(reason) = &info.fallback_reason {
        println!("fallback: {reason}");
    }
    let started = Instant::now();
    engine
        .execute(Command::LoadSpikeImage {
            source: SpikeSource::Synthetic { width, height },
        })
        .map_err(|e| e.to_string())?;
    println!(
        "load {width}x{height}: {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    let render = |exposure_ev: f32,
                  wb: [f32; 3],
                  interactive: bool,
                  proxy: bool|
     -> Result<(f64, Option<f64>), String> {
        let events = engine
            .execute(Command::RenderSpike {
                width: 2560,
                height: 2560,
                exposure_ev,
                wb,
                format: PixelFormat::Rgba8Srgb,
                interactive,
                proxy,
            })
            .map_err(|e| e.to_string())?;
        match events.first() {
            Some(Event::RenderFinished {
                render_ms, gpu_ms, ..
            }) => Ok((*render_ms, *gpu_ms)),
            _ => Err("render produced no frame".into()),
        }
    };
    // The first render builds everything and warms the shader caches.
    render(0.0, [1.0; 3], false, false)?;
    // A drag sends interactive renders; the settled render after release
    // re-runs the demosaic.
    // "preview" is the half-size JPEG frame sent while dragging; its engine time
    // includes the encode, which the plain drag cases do not have.
    let cases = [
        ("exposure drag", false, true, false),
        ("exposure drag preview", false, true, true),
        ("white balance drag", true, true, false),
        ("white balance drag preview", true, true, true),
        ("white balance settled", true, false, false),
    ];
    for (name, vary_wb, interactive, proxy) in cases {
        let (mut engine_ms, mut gpu_ms) = (Vec::new(), Vec::new());
        for i in 0..n {
            let t = i as f32 / n as f32;
            let (e, g) = if vary_wb {
                render(0.0, [1.0 + 0.2 * t, 1.0, 1.0 - 0.2 * t], interactive, proxy)?
            } else {
                render(t * 2.0 - 1.0, [1.0; 3], interactive, proxy)?
            };
            engine_ms.push(e);
            gpu_ms.extend(g);
        }
        let (e50, e95) = percentiles(engine_ms);
        metrics.add(format!("gpu/{name}/engine"), e50, e95);
        let gpu = if gpu_ms.is_empty() {
            "GPU n/a (no timestamp queries)".to_owned()
        } else {
            let (g50, g95) = percentiles(gpu_ms);
            metrics.add(format!("gpu/{name}/gpu"), g50, g95);
            format!("GPU p50 {g50:.2} ms, p95 {g95:.2} ms")
        };
        println!(
            "{name}, {n} steps: {gpu}; engine (render, readback, tiles) p50 {e50:.2} ms, p95 {e95:.2} ms"
        );
    }
    Ok(())
}

/// LIB-09 criterion 5 and the photo-open budgets: opening a folder in
/// Develop at Fit.
///
/// - **First pixels:** each file read and its embedded preview decoded.
/// - **Fit render:** a switch to a photo not prefetched: decoded, opened
///   and rendered settled at Fit.
/// - **Neighbor switch:** a switch to a prefetched neighbor, rendered from
///   its level 1. When that frame is provisional (the view wants level 0),
///   also the time until the frame from level 0 is ready.
///
/// Each of the N passes starts a new engine, so nothing is cached. It moves
/// one photo right to a prefetched neighbor, waiting for the prefetch
/// first, then jumps four right to one that is not, and repeats.
fn bench_open(folder: &str, args: &[&str], metrics: &mut Metrics) -> Result<(), String> {
    let view = (parse_num(args, 0, 2560)?, parse_num(args, 1, 1440)?);
    let passes = parse_num(args, 2, 3)?.max(1);
    let folder = std::path::Path::new(folder);
    let mut times = OpenTimes::default();
    for pass in 1..=passes {
        println!("pass {pass} of {passes}");
        open_pass(folder, view, &mut times)?;
    }
    for (name, times) in [
        ("open/first pixels", times.previews),
        ("open/fit render", times.fits),
        ("open/neighbor switch", times.switches),
        ("open/neighbor switch, from level 0", times.sharp),
    ] {
        if times.is_empty() {
            println!("{name}: no samples");
            continue;
        }
        let count = times.len();
        let (p50, p95) = percentiles(times);
        metrics.add(name, p50, p95);
        println!("{name}, {count} samples: p50 {p50:.1} ms, p95 {p95:.1} ms");
    }
    Ok(())
}

#[derive(Default)]
struct OpenTimes {
    previews: Vec<f64>,
    fits: Vec<f64>,
    switches: Vec<f64>,
    sharp: Vec<f64>,
}

/// One pass of [`bench_open`], with a new engine.
fn open_pass(
    folder: &std::path::Path,
    view: (u32, u32),
    times: &mut OpenTimes,
) -> Result<(), String> {
    let engine = Engine::new();
    let info = engine.info().adapter;
    println!("adapter: {} ({}, {:?})", info.name, info.backend, info.kind);
    // Kept for the first photo; nothing is open yet to render.
    let _ = engine.set_view(ViewSpec::Fit(view), Target::Srgb);
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let opened = engine.open_folder(folder).map_err(|e| e.to_string())?;
    let first = opened
        .first
        .ok_or("no photos in the folder")?
        .map_err(|e| e.to_string())?;
    std::hint::black_box(first);
    let photos = opened.strip.photos;
    println!(
        "open {} photos, the first at Fit {}x{}: {:.0} ms (with the device's warm-up)",
        photos.len(),
        view.0,
        view.1,
        ms(started)
    );
    if photos.len() < 2 {
        return Err("a neighbor switch needs two photos or more".into());
    }
    // Timed without the prefetch's decodes beside them.
    engine.wait_prefetch_idle();
    for p in &photos {
        let path = folder.join(&p.file_name);
        if let Some(t) = files::time_preview(&path).map_err(|e| e.to_string())? {
            times.previews.push(t);
        }
    }

    let switch = |i: usize| -> Result<(Applied, f64), String> {
        let started = Instant::now();
        let a = engine
            .select_photo(photos[i].id)
            .map_err(|e| e.to_string())?;
        Ok((a, ms(started)))
    };
    let mut at = 0;
    while at + 1 < photos.len() {
        engine.wait_prefetch_idle();
        let next = at + 1;
        if !engine.prefetched().contains(&photos[next].id) {
            return Err(format!("{} was not prefetched", photos[next].file_name));
        }
        let started = Instant::now();
        let (a, t) = switch(next)?;
        times.switches.push(t);
        let mut line = format!("switch to {}: {t:.1} ms", photos[next].file_name);
        if a.frame.as_ref().is_some_and(|f| f.provisional) {
            // The worker decodes its input, attaches it and renders again.
            while engine.take_sharpened().is_none() {
                if started.elapsed().as_secs() > 30 {
                    return Err("no frame from level 0 after 30 s".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let t = ms(started);
            times.sharp.push(t);
            line += &format!(", provisional; from level 0 at {t:.0} ms");
        }
        println!("{line}");
        at = next;
        // Four right is no neighbor of this photo, so not prefetched.
        let cold = at + 4;
        if cold >= photos.len() {
            break;
        }
        engine.wait_prefetch_idle();
        let (_, t) = switch(cold)?;
        times.fits.push(t);
        println!("fit render of {}: {t:.0} ms", photos[cold].file_name);
        at = cold;
    }
    Ok(())
}
