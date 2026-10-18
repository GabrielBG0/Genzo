//! DEV-01, stage 0: lists what every DNG in the corpus carries beyond its
//! pixels (opcodes by list and optional flag, levels, crop, color tags), so
//! the spec's opcode inventory and the stage 4 inputs come from real files.
//!
//!   cargo run --release -p genzo-imgio --features dng --example dng_inventory -- ../corpus

use std::path::{Path, PathBuf};

use genzo_imgio::dng;

/// Every DNG under `dir`. A directory or entry that cannot be read is an
/// error, so the inventory never looks complete when it is not.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dng"))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether a square matrix of `n` rows is the identity.
fn identity(m: &[f64]) -> bool {
    let n = (m.len() as f64).sqrt() as usize;
    n * n == m.len()
        && m.iter()
            .enumerate()
            .all(|(i, &v)| v == if i % (n + 1) == 0 { 1.0 } else { 0.0 })
}

fn report(data: &[u8]) -> String {
    let (meta, orientation) = match dng::read_metadata(data) {
        Ok(m) => m,
        Err(e) => return format!("    error: {e}"),
    };
    let mut lines = Vec::new();
    let l = &meta.levels;
    let delta = |d: &[f64]| {
        if d.iter().all(|&v| v == 0.0) {
            format!("{} zero", d.len())
        } else {
            let max = d.iter().fold(0.0f64, |a, &b| a.max(b.abs()));
            format!("{} values, max |{max}|", d.len())
        }
    };
    lines.push(format!(
        "levels: black {:?} over {}x{}, delta H {}, delta V {}, white {:?}",
        l.black,
        l.black_repeat.0,
        l.black_repeat.1,
        delta(&l.black_delta_h),
        delta(&l.black_delta_v),
        l.white
    ));
    lines.push(format!(
        "geometry: orientation {orientation}, active area {:?}, default crop {:?}",
        meta.active_area, meta.default_crop
    ));
    let c = &meta.color;
    let present = |m: &Option<Vec<f64>>| match m {
        None => "-",
        Some(m) if identity(m) => "identity",
        Some(_) => "set",
    };
    lines.push(format!(
        "color: CM1 {:?} CM2 {:?} | FM1 {} FM2 {} | CC1 {} CC2 {} | analog balance {:?} | as shot {:?} | baseline exposure {}",
        c.color_matrix_1.as_ref().map(|m| m.illuminant),
        c.color_matrix_2.as_ref().map(|m| m.illuminant),
        present(&c.forward_matrix_1),
        present(&c.forward_matrix_2),
        present(&c.camera_calibration_1),
        present(&c.camera_calibration_2),
        c.analog_balance,
        c.as_shot,
        c.baseline_exposure
    ));
    lines.push(format!(
        "gain table map: {}",
        meta.gain_table_map
            .as_ref()
            .map_or("-".into(), |g| format!("{} bytes", g.len()))
    ));
    for (name, list) in [
        ("OpcodeList1", &meta.opcodes.list1),
        ("OpcodeList2", &meta.opcodes.list2),
        ("OpcodeList3", &meta.opcodes.list3),
    ] {
        for op in list {
            lines.push(format!(
                "{name}: {:?}{}{} ({} parameter bytes, DNG {}.{})",
                op.kind,
                if op.optional { ", optional" } else { "" },
                if op.preview_skip {
                    ", preview skip"
                } else {
                    ""
                },
                op.params.len(),
                op.dng_version[0],
                op.dng_version[1]
            ));
        }
    }
    lines
        .iter()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "corpus".into());
    let mut files = Vec::new();
    if let Err(e) = walk(Path::new(&root), &mut files) {
        eprintln!("{root}: {e}");
        std::process::exit(1);
    }
    files.sort();
    for file in files {
        let name = file.strip_prefix(&root).unwrap_or(&file).display();
        match std::fs::read(&file) {
            Ok(data) => println!("{name}\n{}\n", report(&data)),
            Err(e) => println!("{name}\n    error: {e}\n"),
        }
    }
}
