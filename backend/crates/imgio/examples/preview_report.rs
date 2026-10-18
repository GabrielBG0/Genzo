//! DEV-01, stage 0: the embedded preview `imgio::decode_preview` picks for
//! each file, how long it takes (search and decode), and the embedded JPEGs
//! it chose from. The budget for first pixels is 100 ms (plan, M1 budgets).
//!
//!   cargo run --release -p genzo-imgio --example preview_report -- FILE...

use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                println!("{path}\n    error: {e}");
                continue;
            }
        };
        // Best of 5, the file already in memory.
        let mut best = f64::MAX;
        let mut result = None;
        for _ in 0..5 {
            let started = Instant::now();
            let r = genzo_imgio::decode_preview(&data);
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
            result = Some(r);
        }
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
        match result.unwrap() {
            Ok(Some(p)) => println!(
                "{name}: {}x{} orientation {} {:?} in {best:.1} ms",
                p.width,
                p.height,
                p.orientation,
                match &p.color {
                    genzo_imgio::ColorEncoding::Icc(icc) => format!("ICC {} bytes", icc.len()),
                    other => format!("{other:?}"),
                }
            ),
            Ok(None) => println!("{name}: no preview ({best:.1} ms)"),
            Err(e) => println!("{name}: {e}"),
        }
    }
}
