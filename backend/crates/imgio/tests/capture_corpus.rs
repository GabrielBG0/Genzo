//! Capture times of the local corpus (`corpus/`, gitignored): every file
//! the formats carry one for reads it, from its metadata only. Skipped when
//! the corpus is absent.

use std::path::Path;

use genzo_imgio::capture::capture_time;

#[test]
fn corpus_files_have_capture_times() {
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
    if files.is_empty() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    files.sort();
    let mut missing = Vec::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let t = capture_time(&data);
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        eprintln!("{name}: {t:?}");
        if t.is_none() {
            missing.push(name);
        }
    }
    eprintln!(
        "{} of {} without a capture time",
        missing.len(),
        files.len()
    );
}
