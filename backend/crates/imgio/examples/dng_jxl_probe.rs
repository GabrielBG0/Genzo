//! 0.C: time the linear DNG reader (src/dng.rs) on one file and report its
//! peak memory. Release build, as the app would run it:
//!
//! cargo run --release -p genzo-imgio --example dng_jxl_probe -- FILE [--dump OUT]
//!
//! `--dump` writes the decoded samples as little-endian u16, row by row, to
//! compare byte for byte with another decoder.
//!
//! Peak memory is the process's high-water mark (VmHWM), on Linux only. It
//! includes the file itself, read into memory first.

use std::time::Instant;

fn peak_mib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: dng_jxl_probe FILE [--dump OUT]");
    let dump = match args.get(1).map(String::as_str) {
        Some("--dump") => Some(args.get(2).expect("--dump needs a file")),
        _ => None,
    };
    let data = std::fs::read(path).expect("read file");
    println!(
        "file {} MiB, peak after reading {:?} MiB",
        data.len() >> 20,
        peak_mib()
    );
    let started = Instant::now();
    let img = match genzo_imgio::dng::decode_linear(&data) {
        Ok(img) => img,
        Err(e) => {
            println!("not decoded: {e}");
            return;
        }
    };
    println!("decoded in {:?}", started.elapsed());
    println!(
        "{}x{}, {} samples, {} bits, compression {}; table {} entries, {} samples past its end",
        img.width,
        img.height,
        img.samples_per_pixel,
        img.bits_per_sample,
        img.compression,
        img.linearization_entries,
        img.clamped
    );
    let spp = usize::from(img.samples_per_pixel);
    let n = (img.pixels.len() / spp) as f64;
    let means: Vec<String> = (0..spp)
        .map(|c| {
            let sum: u64 = img
                .pixels
                .iter()
                .skip(c)
                .step_by(spp)
                .map(|&v| u64::from(v))
                .sum();
            format!("{:.0}", sum as f64 / n)
        })
        .collect();
    println!("mean linear value per channel: {}", means.join(" "));
    println!(
        "pixels {} MiB, peak memory {:?} MiB",
        (img.pixels.len() * 2) >> 20,
        peak_mib()
    );
    if let Some(out) = dump {
        let bytes: Vec<u8> = img.pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out, bytes).expect("write dump");
        println!("samples written to {out}");
    }
}
