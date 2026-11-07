//! OUT-01 criterion 4, at the metadata level: every file of the local
//! corpus (`corpus/`, gitignored) has its EXIF read, written again as an
//! export's APP1, and wrapped in a small JPEG. Its capture time and
//! shooting values read back as from the source. The JPEGs are left in
//! `target/metadata-corpus/` for `scripts/exif-roundtrip.py`, which
//! compares ExifTool's reading of each with the source's. Skipped when the
//! corpus is absent.

use std::path::{Path, PathBuf};

use genzo_imgio::capture::capture;
use genzo_imgio::metadata::{APP1_MAX, read, write};

fn corpus() -> (PathBuf, Vec<PathBuf>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corpus");
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("reference") {
                    stack.push(p);
                }
            } else if p
                .extension()
                .is_some_and(|x| !["txt", "json", "md"].contains(&x.to_str().unwrap_or("")))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    (root, files)
}

/// An 8 × 8 gray JPEG with `app1` right after its SOI.
fn jpeg_with(app1: &[u8]) -> Vec<u8> {
    let body = genzo_imgio::jpeg::encode_rgba(&[128; 8 * 8 * 4], 8, 8, 90).unwrap();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(app1);
    out.extend_from_slice(&body[2..]);
    out
}

#[test]
fn corpus_metadata_survives_the_rewrite() {
    let (root, files) = corpus();
    if files.is_empty() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/metadata-corpus");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let mut none = Vec::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        let Some(meta) = read(&data) else {
            eprintln!("{name}: no metadata");
            none.push(name);
            continue;
        };
        let w = write(&meta, (8, 8), "Genzo test");
        assert!(w.app1.len() <= APP1_MAX, "{name}");
        let jpeg = jpeg_with(&w.app1);
        let source = capture(&data);
        let back = capture(&jpeg);
        assert_eq!(back.time, source.time, "{name}: capture time");
        assert_eq!(back.shooting, source.shooting, "{name}: shooting values");
        eprintln!(
            "{name}: IFD0 {}, Exif {}, GPS {} tags; {} bytes; dropped {:?}",
            meta.ifd0.len(),
            meta.exif.len(),
            meta.gps.len(),
            w.app1.len(),
            w.dropped
        );
        let flat = name.replace(['/', '\\', ' '], "_");
        std::fs::write(out.join(format!("{flat}.jpg")), &jpeg).unwrap();
        std::fs::write(out.join(format!("{flat}.source")), f.display().to_string()).unwrap();
    }
    eprintln!(
        "{} of {} without metadata: {none:?}",
        none.len(),
        files.len()
    );
}
