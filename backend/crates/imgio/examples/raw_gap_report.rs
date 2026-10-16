//! 0.C gap report: decode every file under a directory with rawler and print
//! one Markdown table row per file (what decoded, what is missing). Files
//! rawler cannot read go through the linear DNG reader (src/dng.rs), which
//! reads ProRAW with JPEG XL tiles.
//!
//! cargo run -p genzo-imgio --features raw --example raw_gap_report -- ../corpus

use std::path::{Path, PathBuf};
use std::time::Instant;

use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e != "json" && e != "md") {
            out.push(path);
        }
    }
}

fn report(path: &Path) -> String {
    let started = Instant::now();
    let result = std::panic::catch_unwind(|| {
        let source = RawSource::new(path)?;
        rawler::decode(&source, &RawDecodeParams::default())
    });
    let ms = started.elapsed().as_millis();
    match result {
        Err(_) => format!("PANIC after {ms} ms{}", linear_dng(path)),
        Ok(Err(e)) => format!("ERROR after {ms} ms: {e}{}", linear_dng(path)),
        Ok(Ok(img)) => {
            let kind = match img.data {
                rawler::RawImageData::Integer(_) => "u16",
                rawler::RawImageData::Float(_) => "f32",
            };
            let mut missing = Vec::new();
            if img.color_matrix.is_empty() {
                missing.push("no color matrix");
            }
            if img.wb_coeffs.iter().take(3).any(|c| !c.is_finite()) {
                missing.push("no WB");
            }
            if img.whitelevel.0.is_empty() {
                missing.push("no white level");
            }
            if img.blacklevel.levels.is_empty() {
                missing.push("no black level");
            }
            format!(
                "ok {ms} ms | {}x{} cpp {} {} bps {} | {:?} | wb {:?} | white {:?} | black {:?} | matrices {:?} | active {} crop {} | dng_tags {:?} | missing: {}",
                img.width,
                img.height,
                img.cpp,
                kind,
                img.bps,
                img.photometric,
                img.wb_coeffs,
                img.whitelevel.0,
                img.blacklevel
                    .levels
                    .iter()
                    .map(|r| r.as_f32())
                    .collect::<Vec<_>>(),
                img.color_matrix.keys().collect::<Vec<_>>(),
                img.active_area.is_some(),
                img.crop_area.is_some(),
                {
                    let mut t: Vec<_> = img.dng_tags.keys().collect();
                    t.sort();
                    t
                },
                if missing.is_empty() {
                    "none".to_string()
                } else {
                    missing.join(", ")
                },
            )
        }
    }
}

/// The linear DNG reader's result for a file rawler failed on.
fn linear_dng(path: &Path) -> String {
    let Ok(data) = std::fs::read(path) else {
        return String::new();
    };
    let started = Instant::now();
    let result = genzo_imgio::dng::decode_linear(&data);
    let ms = started.elapsed().as_millis();
    match result {
        Err(e) => format!("\n    linear DNG reader: {e}"),
        Ok(img) => format!(
            "\n    linear DNG reader: ok {ms} ms | {}x{} cpp {} bps {} compression {} | table {} entries, {} clamped | white {:?} | black {:?} | matrices {}{} | neutral {:?} | baseline exposure {:?} | gain table map {} | opcode lists {:?}",
            img.width,
            img.height,
            img.samples_per_pixel,
            img.bits_per_sample,
            img.compression,
            img.linearization_entries,
            img.clamped,
            img.white_level,
            img.black_level,
            u8::from(img.color_matrix_1.is_some()),
            if img.color_matrix_2.is_some() {
                "+2"
            } else {
                ""
            },
            img.as_shot_neutral,
            img.baseline_exposure,
            img.has_gain_table_map,
            img.opcode_lists,
        ),
    }
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "corpus".into());
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    files.sort();
    for file in files {
        let name = file.strip_prefix(&root).unwrap_or(&file).display();
        println!("{name}\n    {}\n", report(&file));
    }
}
