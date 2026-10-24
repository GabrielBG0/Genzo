//! `list_folder` over the local corpus (`corpus/`, gitignored), through the
//! real read path: every corpus file is a photo, and its capture time comes
//! from the file's first MiB. Skipped when the corpus is absent.

use std::path::{Path, PathBuf};

use genzo_engine::folder::{is_photo, list_folder, needs_whole_file};

fn folders(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() && !p.ends_with("reference") {
                stack.push(p.clone());
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn corpus_folders_list_from_the_head_of_each_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corpus");
    let dirs = folders(&root);
    if dirs.is_empty() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    let (mut listed, mut dated, mut whole) = (0, 0, Vec::new());
    for dir in dirs {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            if p.is_file()
                && !name.ends_with(".txt")
                && !name.ends_with(".json")
                && !name.ends_with(".md")
            {
                assert!(is_photo(&name), "{} is a photo", p.display());
                if needs_whole_file(&p) {
                    whole.push(name);
                }
            }
        }
        for photo in list_folder(&dir).unwrap() {
            listed += 1;
            dated += usize::from(photo.capture_time.is_some());
        }
    }
    eprintln!("{listed} listed, {dated} dated; past the first MiB: {whole:?}");
    assert!(listed > 0);
}
