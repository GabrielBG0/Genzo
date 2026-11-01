//! `genzo-cli`: drives the engine without a UI.
//!
//! The cli must be able to do everything the UI does except draw: `run`
//! takes the contract's commands and queries as JSON lines, and prints
//! their results and the events they send. Besides it: the contract tooling,
//! engine info, decoding, and the benchmarks.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use genzo_engine::api::{Command, Event, PixelFormat, Query};
use genzo_engine::develop::{Applied, Target, ViewSpec};
use genzo_engine::preview::Shown;
use genzo_engine::{Engine, contract, files};

const USAGE: &str = "\
usage: genzo-cli <command>

commands:
  version                      print engine and contract versions
  info                         print engine info as JSON
  contract write [DIR]         regenerate contract/ (refuses without a version bump)
  contract check [DIR]         fail if contract/ is stale
  inspect FILE                 decode a file, print what it holds and how long it took
  run FILE                     run the commands and queries in FILE (- for stdin),
                               one JSON object a line, with a command or a query
                               key; print each result, error and event as a JSON
                               line keyed result, error or event. At the end it
                               waits for the events still to come
  bench test-pattern [W H N]   render N test-pattern frames of W×H, print p50/p95
  bench decode FILE [T N]      decode FILE N times with T threads, print p50/p95
  bench gpu [W H N]            open a W×H synthetic raw in Develop at Fit in a
                               2560 px viewport, N slider steps per case (drags
                               and settled deltas), print GPU and engine p50/p95
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
        ["run", file] => run(file),
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

/// One line of `run`'s input.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Line {
    Command(Command),
    Query(Query),
}

/// Prints one JSON line of `run`'s output, whole, whichever thread sends it.
fn print_line(key: &str, value: impl serde::Serialize) {
    let line = serde_json::json!({ key: value });
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `run FILE`: the contract's messages, as the shell passes them.
fn run(file: &str) -> Result<(), String> {
    let input: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = std::fs::File::open(file).map_err(|e| format!("{file}: {e}"))?;
        Box::new(std::io::BufReader::new(f))
    };
    let engine = Engine::new().with_event_sink(Arc::new(|e: Event| print_line("event", e)));
    for (n, line) in input.lines().enumerate() {
        let line = line.map_err(|e| format!("{file}: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Line>(&line) {
            Ok(Line::Command(c)) => match engine.execute(c) {
                Ok(r) => print_line("result", r),
                Err(e) => print_line("error", e.to_string()),
            },
            Ok(Line::Query(q)) => print_line("result", engine.query(q)),
            Err(e) => return Err(format!("{file}:{}: {e}", n + 1)),
        }
    }
    engine.wait_histograms_idle();
    engine.wait_prefetch_idle();
    engine.wait_histograms_idle();
    Ok(())
}

/// `engine`, its events sent to the receiver.
fn recorded(engine: Engine) -> (Engine, Receiver<Event>) {
    let (tx, rx) = channel();
    let engine = engine.with_event_sink(Arc::new(move |e| {
        let _ = tx.send(e);
    }));
    (engine, rx)
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
    let (engine, events) = recorded(Engine::headless());
    let mut times = Vec::with_capacity(n as usize);
    for i in 0..n {
        let started = Instant::now();
        engine
            .execute(Command::RenderTestPattern {
                width,
                height,
                phase: i as f32 / n as f32,
                format: PixelFormat::Rgba8Srgb,
            })
            .map_err(|e| e.to_string())?;
        let Ok(Event::RenderFinished { generation, .. }) = events.try_recv() else {
            return Err("render produced no frame".into());
        };
        let bytes = engine.frame(generation).ok_or("frame missing")?;
        std::hint::black_box(bytes);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let (p50, p95) = percentiles(times);
    metrics.add(format!("test-pattern/{width}x{height}"), p50, p95);
    println!("test-pattern {width}x{height}, {n} frames: p50 {p50:.2} ms, p95 {p95:.2} ms");
    Ok(())
}

/// What one slider step costs at 2560 px, sent as the UI sends it: a
/// synthetic RAW open in Develop at Fit in a 2560 px viewport, each case
/// N steps of `ApplyEditDelta`.
///
/// - **Drags:** a gesture's start, N updates and its end; each update is
///   timed. At Fit an update renders the half-size preview and sends it as
///   JPEG tiles. An exposure drag re-runs from stage 7's input; a white
///   balance drag applies the white as a ratio after the cached demosaic.
/// - **Settled:** N single deltas, each its own step: an exposure re-runs
///   the stages after sampling, a white balance rebuilds the pyramid from
///   the RAW data (the tiles the view reads first).
///
/// "engine" is the request until its tiles are kept (render, readback,
/// encoding), "gpu" the render's GPU time from timestamp queries.
fn bench_gpu(args: &[&str], metrics: &mut Metrics) -> Result<(), String> {
    use genzo_engine::api::edit::{Change, Phase, WhiteBalanceMode};
    use genzo_engine::api::{ColorSpace, View};

    let (width, height, n) = (
        parse_num(args, 0, 8064)?,
        parse_num(args, 1, 6048)?,
        parse_num(args, 2, 50)?.max(2),
    );
    let (engine, events) = recorded(Engine::new());
    let info = engine.info().adapter;
    println!("adapter: {} ({}, {:?})", info.name, info.backend, info.kind);
    if let Some(reason) = &info.fallback_reason {
        println!("fallback: {reason}");
    }
    let timestamps = engine.gpu_timestamps();
    println!("timestamps: {}", if timestamps { "yes" } else { "no" });
    let started = Instant::now();
    let photo = engine
        .open_synthetic(width, height)
        .map_err(|e| e.to_string())?;
    engine
        .execute(Command::SetView {
            width: 2560,
            height: 2560,
            view: View::Fit,
            space: ColorSpace::Srgb,
        })
        .map_err(|e| e.to_string())?;
    println!(
        "open {width}x{height} and render it at Fit: {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    // One delta, and the times of the frame it rendered.
    let send = |changes: Vec<Change>, phase: Phase| -> Result<(f64, Option<f64>), String> {
        events.try_iter().for_each(drop);
        engine
            .execute(Command::ApplyEditDelta {
                photo,
                changes,
                phase,
            })
            .map_err(|e| e.to_string())?;
        events
            .try_iter()
            .find_map(|e| match e {
                Event::RenderFinished {
                    render_ms, gpu_ms, ..
                } => Some((render_ms, gpu_ms)),
                _ => None,
            })
            .ok_or_else(|| "a delta rendered no frame".to_owned())
    };
    let exposure = |t: f32| vec![Change::Exposure(t * 2.0 - 1.0)];
    let white = |t: f32| {
        vec![Change::WhiteBalance {
            mode: WhiteBalanceMode::Custom,
            temp: 4000.0 + 3000.0 * t,
            tint: 10.0 * t,
        }]
    };
    // The first deltas warm the shader caches of every path.
    for (change, phase) in [
        (exposure(0.5), Phase::Start),
        (exposure(0.6), Phase::Update),
        (exposure(0.7), Phase::End),
        (white(0.5), Phase::Start),
        (white(0.6), Phase::Update),
        (white(0.7), Phase::End),
    ] {
        send(change, phase)?;
    }

    type Delta = fn(f32) -> Vec<Change>;
    let cases: [(&str, Delta, bool); 4] = [
        ("exposure drag", exposure, true),
        ("white balance drag", white, true),
        ("exposure settled", exposure, false),
        ("white balance settled", white, false),
    ];
    for (name, delta, drag) in cases {
        let (mut engine_ms, mut gpu_ms) = (Vec::new(), Vec::new());
        if drag {
            send(delta(0.0), Phase::Start)?;
        }
        for i in 0..n {
            let t = (i + 1) as f32 / n as f32;
            let (e, g) = send(delta(t), if drag { Phase::Update } else { Phase::End })?;
            engine_ms.push(e);
            gpu_ms.extend(g);
        }
        if drag {
            send(delta(1.0), Phase::End)?;
        }
        if timestamps && gpu_ms.len() != engine_ms.len() {
            return Err(format!("{name}: a render without its GPU time"));
        }
        let (e50, e95) = percentiles(engine_ms);
        metrics.add(format!("develop/{name}/engine"), e50, e95);
        let gpu = if gpu_ms.is_empty() {
            "GPU n/a (no timestamp queries)".to_owned()
        } else {
            let (g50, g95) = percentiles(gpu_ms);
            metrics.add(format!("develop/{name}/gpu"), g50, g95);
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
/// - **First pixels:** from a switch to its embedded preview reaching the
///   preview sink, ready to draw: read from the file for a photo not
///   prefetched, kept by the prefetch for a neighbor. Photos without one
///   give no sample.
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
        ("open/first pixels, neighbor", times.neighbor_previews),
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
    neighbor_previews: Vec<f64>,
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
    // When the last switch's preview arrived, and for which photo.
    let shown: Arc<Mutex<Option<(u64, Instant)>>> = Arc::default();
    let s = shown.clone();
    let engine = Engine::new().with_preview_sink(Arc::new(move |x: Shown| {
        if x.preview.is_some() {
            *s.lock().unwrap() = Some((x.photo, Instant::now()));
        }
    }));
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
    // A switch: its result, its time, and the time to its preview.
    let switch = |i: usize| -> Result<(Applied, f64, Option<f64>), String> {
        shown.lock().unwrap().take();
        let started = Instant::now();
        let a = engine
            .select_photo(photos[i].id)
            .map_err(|e| e.to_string())?;
        let t = ms(started);
        let first = match *shown.lock().unwrap() {
            Some((id, at)) if id == photos[i].id => Some((at - started).as_secs_f64() * 1000.0),
            _ => None,
        };
        Ok((a, t, first))
    };
    let mut at = 0;
    while at + 1 < photos.len() {
        engine.wait_prefetch_idle();
        let next = at + 1;
        if !engine.prefetched().contains(&photos[next].id) {
            return Err(format!("{} was not prefetched", photos[next].file_name));
        }
        let started = Instant::now();
        let (a, t, first) = switch(next)?;
        times.switches.push(t);
        times.neighbor_previews.extend(first);
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
        let (_, t, first) = switch(cold)?;
        times.fits.push(t);
        times.previews.extend(first);
        println!("fit render of {}: {t:.0} ms", photos[cold].file_name);
        at = cold;
    }
    Ok(())
}
