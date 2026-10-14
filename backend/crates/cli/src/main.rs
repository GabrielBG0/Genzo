//! `genzo-cli`: drives the engine without a UI.
//!
//! The cli must be able to do everything the UI does except draw. In M0 that
//! is the contract tooling, engine info and the test-pattern benchmark.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use genzo_engine::api::{Command, Event, PixelFormat};
use genzo_engine::{Engine, contract};

const USAGE: &str = "\
usage: genzo-cli <command>

commands:
  version                      print engine and contract versions
  info                         print engine info as JSON
  contract write [DIR]         regenerate contract/ (refuses without a version bump)
  contract check [DIR]         fail if contract/ is stale
  bench test-pattern [W H N]   render N test-pattern frames of W×H, print p50/p95
";

fn default_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contract")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["version"] => {
            let info = Engine::new().info();
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
        ["bench", "test-pattern", rest @ ..] => bench_test_pattern(rest),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// Engine-side cost of one frame: render plus tile encoding. The end-to-end
/// latency (IPC, upload, screen) is measured in the app, see 0.E.
fn bench_test_pattern(args: &[&str]) -> Result<(), String> {
    let num = |i: usize, default: u32| -> Result<u32, String> {
        args.get(i).map_or(Ok(default), |s| {
            s.parse().map_err(|_| format!("not a number: {s}"))
        })
    };
    let (width, height, n) = (num(0, 2560)?, num(1, 1440)?, num(2, 30)?.max(1));
    let engine = Engine::new();
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
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    println!(
        "test-pattern {width}x{height}, {n} frames: p50 {:.2} ms, p95 {:.2} ms",
        pct(0.5),
        pct(0.95)
    );
    Ok(())
}
