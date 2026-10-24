//! LIB-09's criteria 1 and 2 on a temporary folder: which files are listed,
//! and their order.

use std::path::PathBuf;

use genzo_engine::folder::list_folder;

/// A minimal TIFF: IFD0 with an ExifIFD pointer, the EXIF IFD with
/// DateTimeOriginal and SubSecTimeOriginal, little-endian.
fn dated_tiff(date: &str, sub: &str) -> Vec<u8> {
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(34665u16.to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(26u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    let data_at = 26 + 2 + 2 * 12 + 4;
    out.extend(2u16.to_le_bytes());
    out.extend(0x9003u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(20u32.to_le_bytes());
    out.extend((data_at as u32).to_le_bytes());
    out.extend(0x9291u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(4u32.to_le_bytes());
    let mut s = [0u8; 4];
    s[..sub.len()].copy_from_slice(sub.as_bytes());
    out.extend(s);
    out.extend(0u32.to_le_bytes());
    out.extend(date.as_bytes());
    out.push(0);
    out
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lists_the_photos_directly_inside_in_capture_time_order() {
    let dir = TempDir(std::env::temp_dir().join(format!("genzo-folder-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(dir.0.join("sub")).unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(dir.0.join(name), bytes).unwrap();
    // Dated: a burst within one second, ordered by its sub-seconds as a
    // decimal fraction (".12" before ".9"), and an earlier shot with a later
    // name.
    write("burst_b.tif", &dated_tiff("2024:05:01 10:00:00", "9"));
    write("burst_a.tif", &dated_tiff("2024:05:01 10:00:00", "12"));
    write("zzz_early.TIF", &dated_tiff("2023:12:31 23:59:59", ""));
    // Undated, by natural name after the dated ones.
    write("IMG_10.jpg", b"\xFF\xD8\xFF\xE0 not much of a JPEG");
    write("IMG_2.png", b"\x89PNG\r\n\x1a\n");
    write("IMG_2.CR3", b"\0\0\0\x18ftypcrx ");
    // Not listed.
    write(".hidden.tif", &dated_tiff("2020:01:01 00:00:00", ""));
    write("burst_a.xmp", b"<x:xmpmeta/>");
    write("notes.txt", b"hello");
    std::fs::write(
        dir.0.join("sub").join("inner.tif"),
        dated_tiff("2020:01:01 00:00:00", ""),
    )
    .unwrap();

    let photos = list_folder(&dir.0).unwrap();
    let names: Vec<&str> = photos.iter().map(|p| p.file_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "zzz_early.TIF",
            "burst_a.tif",
            "burst_b.tif",
            "IMG_2.CR3",
            "IMG_2.png",
            "IMG_10.jpg"
        ]
    );
    let t = photos[1].capture_time.unwrap();
    assert_eq!((t.year, t.second, t.nanos), (2024, 0, 120_000_000));
    assert!(photos[3..].iter().all(|p| p.capture_time.is_none()));
}

#[test]
fn an_unreadable_folder_is_an_error() {
    let missing = std::env::temp_dir().join("genzo-no-such-folder-here");
    assert!(list_folder(&missing).is_err());
}
