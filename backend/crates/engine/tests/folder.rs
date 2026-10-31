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

/// A TIFF with an undated EXIF IFD holding ISO 400, 1/250 s, f/2.8 and
/// 6.86 mm, `gap` bytes after IFD0: past the head read first when large.
fn shot_tiff(gap: usize) -> Vec<u8> {
    let exif_at = 26 + gap;
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(34665u16.to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend((exif_at as u32).to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.resize(exif_at, 0);
    let data_at = (exif_at + 2 + 4 * 12 + 4) as u32;
    out.extend(4u16.to_le_bytes());
    // In tag order: ExposureTime, FNumber (RATIONAL), ISO (SHORT),
    // FocalLength (RATIONAL); the rationals' values follow in that order.
    let rational = |out: &mut Vec<u8>, tag: u16, slot: u32| {
        out.extend(tag.to_le_bytes());
        out.extend(5u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend((data_at + 8 * slot).to_le_bytes());
    };
    rational(&mut out, 33434, 0);
    rational(&mut out, 33437, 1);
    out.extend(34855u16.to_le_bytes());
    out.extend(3u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(400u32.to_le_bytes());
    rational(&mut out, 37386, 2);
    out.extend(0u32.to_le_bytes());
    for (num, den) in [(1u32, 250u32), (28, 10), (686, 100)] {
        out.extend(num.to_le_bytes());
        out.extend(den.to_le_bytes());
    }
    out
}

#[test]
fn lists_how_each_photo_was_shot() {
    let dir = TempDir(std::env::temp_dir().join(format!("genzo-shot-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    std::fs::write(dir.0.join("a_near.tif"), shot_tiff(0)).unwrap();
    // Its EXIF IFD 2 MiB in: read once the head held nothing.
    std::fs::write(dir.0.join("b_far.tif"), shot_tiff(2 << 20)).unwrap();
    std::fs::write(
        dir.0.join("c_none.tif"),
        dated_tiff("2024:05:01 10:00:00", ""),
    )
    .unwrap();

    let photos = list_folder(&dir.0).unwrap();
    let names: Vec<&str> = photos.iter().map(|p| p.file_name.as_str()).collect();
    assert_eq!(names, ["c_none.tif", "a_near.tif", "b_far.tif"]);
    for p in &photos[1..] {
        let s = p.shooting;
        assert_eq!(s.iso, Some(400), "{}", p.file_name);
        assert_eq!(s.exposure_time.map(|r| (r.num, r.den)), Some((1, 250)));
        assert_eq!(s.f_number.map(|r| r.value()), Some(2.8));
        assert_eq!(s.focal_length.map(|r| r.value()), Some(6.86));
    }
    assert_eq!(photos[0].shooting, Default::default(), "dated, not shot");
}

#[test]
fn an_unreadable_folder_is_an_error() {
    let missing = std::env::temp_dir().join("genzo-no-such-folder-here");
    assert!(list_folder(&missing).is_err());
}
