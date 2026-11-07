//! `genzo-cli thumbs` and `bench thumbs` (LIB-09): a JPEG whose EXIF
//! segment holds a preview has a thumbnail, written as a JPEG file; a file
//! without one is listed as such. Headless: no GPU needed.

use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// imgio's 4:2:0 JPEG fixture, which is also its own preview here: inside
/// the EXIF segment of a copy of itself.
fn jpeg_with_preview() -> Vec<u8> {
    let jpeg = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../imgio/tests/fixtures/images/jpeg-420.jpg"),
    )
    .unwrap();
    let mut app1 = b"Exif\0\0II*\0\x08\0\0\0\0\0".to_vec();
    app1.extend_from_slice(&jpeg);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn cli(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_genzo-cli"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn thumbs_writes_each_thumbnail_and_bench_thumbs_times_them() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("genzo-cli-thumbs-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let photos = dir.0.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::write(photos.join("a.jpg"), jpeg_with_preview()).unwrap();
    std::fs::write(photos.join("b.png"), b"\x89PNG\r\n\x1a\n").unwrap();

    let out = dir.0.join("out");
    let stdout = cli(&["thumbs".as_ref(), photos.as_os_str(), out.as_os_str()]);
    assert!(stdout.contains("b.png: no thumbnail"), "{stdout}");
    let written = std::fs::read(out.join("a.jpg.jpg")).unwrap();
    assert!(written.starts_with(&[0xFF, 0xD8]), "a JPEG");
    assert!(!out.join("b.png.jpg").exists());

    let json = dir.0.join("metrics.json");
    let stdout = cli(&[
        "bench".as_ref(),
        "thumbs".as_ref(),
        photos.as_os_str(),
        "1".as_ref(),
        "--json".as_ref(),
        json.as_os_str(),
    ]);
    assert!(stdout.contains("2 photos, 1 with a thumbnail"), "{stdout}");
    let metrics: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    for name in ["thumbs/made, per file", "thumbs/from disk, per file"] {
        assert!(
            metrics[name]["p95_ms"].as_f64().is_some(),
            "{name}: {metrics}"
        );
    }
}
