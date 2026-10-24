//! Opening a folder (LIB-09, M1 part): the photos directly inside it, in
//! capture-time order (owner, 2026-10-24).
//!
//! - Files with an extension Genzo reads, hidden files and sidecars
//!   skipped. Subfolders are not read.
//! - By EXIF capture time with its sub-seconds; ties, and files without a
//!   capture time, by file name with numbers compared as numbers. Undated
//!   files come after the dated ones.
//! - Capture times are read from each file's metadata in parallel, never
//!   its pixels.

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};

use genzo_imgio::capture::{CaptureTime, can_lie_past_the_head, capture_time};
use rayon::prelude::*;

use crate::EngineError;

/// The extensions of the formats Genzo reads (DEV-01, Stage 0), lower case.
pub const EXTENSIONS: &[&str] = &[
    // Camera RAW and DNG.
    "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "iiq", "kdc", "mef", "mos",
    "mrw", "nef", "nrw", "orf", "pef", "raf", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
    // Listed though not rendered: GoPro GPR is unsupported by name (DEV-01,
    // Stage 0), and shows its reason.
    "gpr", // The rest.
    "jpg", "jpeg", "heic", "heif", "hif", "tif", "tiff", "png",
];

/// How much of a file is read for its capture time first. The whole file is
/// read only when its metadata lies past it (a HEIC's EXIF item in `mdat`).
const HEAD: u64 = 1 << 20;

/// One photo of an opened folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderPhoto {
    pub path: PathBuf,
    pub file_name: String,
    pub capture_time: Option<CaptureTime>,
}

/// The photos directly inside `folder`, in the filmstrip's order.
pub fn list_folder(folder: &Path) -> Result<Vec<FolderPhoto>, EngineError> {
    let read = std::fs::read_dir(folder)
        .map_err(|e| EngineError::Io(format!("{}: {e}", folder.display())))?;
    let mut paths = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| EngineError::Io(e.to_string()))?;
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_file && is_photo(&name) {
            paths.push((entry.path(), name));
        }
    }
    let mut photos: Vec<FolderPhoto> = paths
        .into_par_iter()
        .map(|(path, file_name)| FolderPhoto {
            capture_time: read_capture_time(&path),
            path,
            file_name,
        })
        .collect();
    photos.sort_by(order);
    Ok(photos)
}

/// Whether a file name is a photo Genzo lists: not hidden, with one of
/// [`EXTENSIONS`].
pub fn is_photo(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Whether reading `path`'s capture time needed more than its head: for
/// the corpus test, which counts them.
#[doc(hidden)]
pub fn needs_whole_file(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    if file.by_ref().take(HEAD).read_to_end(&mut head).is_err() {
        return false;
    }
    capture_time(&head).is_none() && worth_the_rest(&head)
}

/// Whether to read past the head that held no capture time: when there is
/// a rest, and the format can keep its metadata there.
fn worth_the_rest(head: &[u8]) -> bool {
    head.len() as u64 == HEAD && can_lie_past_the_head(head)
}

fn read_capture_time(path: &Path) -> Option<CaptureTime> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.by_ref().take(HEAD).read_to_end(&mut head).ok()?;
    if let Some(t) = capture_time(&head) {
        return Some(t);
    }
    if !worth_the_rest(&head) {
        return None;
    }
    file.read_to_end(&mut head).ok()?;
    capture_time(&head)
}

/// The filmstrip's order: dated before undated, by time, then by name.
fn order(a: &FolderPhoto, b: &FolderPhoto) -> Ordering {
    match (a.capture_time, b.capture_time) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| natural(&a.file_name, &b.file_name))
}

/// File names compared with their runs of digits as numbers, the rest case
/// blind: IMG_2 before IMG_10. Equal names by that measure fall back to
/// the plain comparison, so the order is total.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let run = |s: &[u8]| s.iter().take_while(|c| c.is_ascii_digit()).count();
                let (m, n) = (run(x), run(y));
                // Leading zeros do not count; then the longer run is larger.
                let trim = |s: &[u8]| {
                    let z = s.iter().take_while(|&&c| c == b'0').count();
                    s[z.min(s.len().saturating_sub(1))..].to_vec()
                };
                let (dx, dy) = (trim(&x[..m]), trim(&y[..n]));
                let c = dx.len().cmp(&dy.len()).then_with(|| dx.cmp(&dy));
                if c != Ordering::Equal {
                    return c;
                }
                x = &x[m..];
                y = &y[n..];
            }
            (Some(p), Some(q)) => {
                let c = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
                if c != Ordering::Equal {
                    return c;
                }
                x = &x[1..];
                y = &y[1..];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_numbers_as_numbers() {
        let mut names = vec![
            "IMG_10.jpg",
            "img_2.jpg",
            "IMG_1.CR3",
            "IMG_1.JPG",
            "IMG_002.jpg",
            "a.png",
        ];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(
            names,
            [
                "a.png",
                "IMG_1.CR3",
                "IMG_1.JPG",
                "IMG_002.jpg",
                "img_2.jpg",
                "IMG_10.jpg"
            ]
        );
    }

    #[test]
    fn which_files_are_photos() {
        for yes in ["a.NEF", "b.jpeg", "c.Heic", "d.dng", "e.tif"] {
            assert!(is_photo(yes), "{yes}");
        }
        for no in [".hidden.jpg", "a.xmp", "b.txt", "noext", "c.jpg.part"] {
            assert!(!is_photo(no), "{no}");
        }
    }
}
