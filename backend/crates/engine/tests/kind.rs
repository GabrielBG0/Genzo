//! LIB-09 criterion 3, and part of DEV-01 criterion 17's engine side: files
//! Genzo cannot render are listed, say why, and leave nothing open in
//! Develop. A DNG with a required unknown opcode is imgio's own test
//! (`dng.rs`); the engine maps every unsupported file the same way. The
//! embedded preview they show comes with the next step.
//! Each strip photo's `kind` is learned from its decode: by a switch to it,
//! or by prefetch as a neighbor.
//!
//! Real files through the real decoder: imgio's fixtures (a JPEG, a CMYK
//! JPEG, a damaged PNG), and from the corpus, when present, Nikon High
//! Efficiency and GoPro GPR.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use genzo_engine::api::edit::Phase;
use genzo_engine::develop::{Target, ViewSpec};
use genzo_engine::strip::{Direction, PhotoKind, Strip};
use genzo_engine::{Engine, EngineError};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../imgio/tests/fixtures/images")
        .join(name);
    std::fs::read(path).unwrap()
}

/// A PNG that renders. The fixtures are 37 × 29, under the 32 px a crop
/// needs on its short side.
fn renders() -> Vec<u8> {
    let (w, h) = (96u32, 64u32);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    let mut writer = enc.write_header().unwrap();
    let data: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % w) as u8, (i / w) as u8, 128])
        .collect();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
    out
}

struct TempDir(PathBuf);

impl TempDir {
    /// A folder of files: (name, contents).
    fn new(tag: &str, files: &[(&str, Vec<u8>)]) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-kind-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, data) in files {
            std::fs::write(dir.join(name), data).unwrap();
        }
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn engine() -> Option<Engine> {
    let engine = Engine::new();
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some(engine)
}

fn kinds(strip: &Strip) -> Vec<PhotoKind> {
    strip.photos.iter().map(|p| p.kind.clone()).collect()
}

/// Nothing is open in Develop: no edit, and edits are refused, so the
/// frontend disables the controls.
fn nothing_open(engine: &Engine) {
    assert!(engine.edit_state().is_none());
    assert!(matches!(
        engine.apply(&[], Phase::End),
        Err(EngineError::NoPhoto)
    ));
}

#[test]
fn unsupported_and_damaged_files_are_listed_with_their_reason() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    engine
        .set_view(ViewSpec::Fit((200, 150)), Target::Srgb)
        .ok();
    let dir = TempDir::new(
        "listed",
        &[
            ("a_good.png", renders()),
            ("b_cmyk.jpg", fixture("jpeg-cmyk.jpg")),
            ("c_damaged.png", fixture("fuzz-png-plte-7-bytes.png")),
            ("d_gone.png", renders()),
            ("e_text.tif", b"not an image".to_vec()),
        ],
    );
    let opened = engine.open_folder(&dir.0).unwrap();
    let names: Vec<&str> = opened
        .strip
        .photos
        .iter()
        .map(|p| p.file_name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "a_good.png",
            "b_cmyk.jpg",
            "c_damaged.png",
            "d_gone.png",
            "e_text.tif"
        ]
    );
    let [_, cmyk, damaged, gone, _] =
        opened.strip.photos.iter().map(|p| p.id).collect::<Vec<_>>()[..]
    else {
        panic!()
    };
    // Listing decodes nothing: the first photo is known from its switch.
    assert_eq!(opened.strip.photos[0].kind, PhotoKind::Renders);
    opened.first.unwrap().unwrap();

    // The neighbors are known from their prefetch decodes, unselected;
    // the photo past them is not decoded yet.
    engine.wait_prefetch_idle();
    let strip = engine.strip();
    let first_kinds = kinds(&strip);
    let [good, unsupported, bad, unknown, _] = &first_kinds[..] else {
        panic!()
    };
    assert_eq!(*good, PhotoKind::Renders);
    let PhotoKind::Unsupported(why) = unsupported else {
        panic!("{unsupported:?}")
    };
    assert!(why.contains("CMYK JPEG"), "{why}");
    assert_eq!(unsupported.reason(), Some(why.as_str()));
    assert_eq!(*bad, PhotoKind::Damaged);
    assert_eq!(bad.reason(), Some("This file is damaged"));
    assert_eq!(*unknown, PhotoKind::Unknown);
    assert_eq!((good.reason(), unknown.reason()), (None, None));

    // A file gone since listing: missing, until a listing finds it again.
    // It goes before it is anyone's neighbor, so no prefetch read it.
    std::fs::remove_file(dir.0.join("d_gone.png")).unwrap();
    assert!(matches!(
        engine.select_photo(gone),
        Err(EngineError::Missing(_))
    ));
    assert_eq!(engine.strip().current, Some(gone));
    nothing_open(&engine);
    let missing = engine.strip().photos[3].kind.clone();
    assert_eq!(missing, PhotoKind::Missing);
    assert_eq!(missing.reason(), Some("This file is no longer there"));
    // Its neighbor past it, prefetched now: a file of no format Genzo
    // reads, whatever its extension.
    engine.wait_prefetch_idle();
    assert_eq!(
        engine.strip().photos[4].kind,
        PhotoKind::Unsupported("This file's format is not one Genzo reads".into())
    );

    // Selected, each says why, stays selected, and opens nothing. The
    // arrows move on from them.
    assert!(matches!(
        engine.step(Direction::Previous),
        Err(EngineError::Damaged(_))
    ));
    assert_eq!(engine.strip().current, Some(damaged));
    nothing_open(&engine);
    match engine.step(Direction::Previous) {
        Err(EngineError::Unsupported(m)) => assert_eq!(m, *why),
        other => panic!("{other:?}"),
    }
    assert_eq!(engine.strip().current, Some(cmyk));
    nothing_open(&engine);
    engine.wait_prefetch_idle();
    // The failed decodes changed nothing they knew.
    assert_eq!(kinds(&engine.strip())[..3], first_kinds[..3]);

    std::fs::write(dir.0.join("d_gone.png"), renders()).unwrap();
    let again = engine.open_folder(&dir.0).unwrap();
    engine.wait_prefetch_idle();
    // Kept across the listing, by id; the missing one listed again is
    // not missing, and not decoded again yet.
    assert_eq!(again.strip.photos[3].id, gone);
    let kinds_again = kinds(&engine.strip());
    assert_eq!(kinds_again[..3], first_kinds[..3]);
    assert_eq!(kinds_again[3], PhotoKind::Unknown);
}

/// A neighbor prefetched, then gone: its level 1 still opens it at once,
/// and the decode of its input after the switch finds it missing.
#[test]
fn a_cached_neighbor_gone_is_found_missing_after_the_switch() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(engine) = engine() else { return };
    let dir = TempDir::new("attach", &[("a.png", renders()), ("b.png", renders())]);
    let opened = engine.open_folder(&dir.0).unwrap();
    let b = opened.strip.photos[1].id;
    engine.wait_prefetch_idle();
    assert_eq!(engine.prefetched(), [b]);
    assert_eq!(engine.strip().photos[1].kind, PhotoKind::Renders);
    std::fs::remove_file(dir.0.join("b.png")).unwrap();
    engine.select_photo(b).unwrap();
    engine.wait_prefetch_idle();
    assert_eq!(engine.strip().photos[1].kind, PhotoKind::Missing);
}

/// The corpus's unsupported RAW formats, when present (gitignored, so
/// local only): Nikon High Efficiency and GoPro GPR.
#[test]
fn corpus_formats_genzo_does_not_decode_say_why() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corpus/raw");
    let cases = [
        (
            "Nikon/Z 8/Nikon_Z8_high_efficiency_low.NEF",
            "Nikon High Efficiency RAW is not supported yet",
        ),
        (
            "GoPro/HERO6 Black/GOPR8921.GPR",
            "GoPro GPR (VC-5 compression) is not supported",
        ),
    ];
    let present: Vec<_> = cases
        .iter()
        .filter(|(p, _)| corpus.join(p).is_file())
        .collect();
    if present.is_empty() {
        eprintln!("skipped: no corpus at {}", corpus.display());
        return;
    }
    let Some(engine) = engine() else { return };
    for (path, want) in present {
        let from = corpus.join(path);
        let name = from.file_name().unwrap().to_string_lossy().into_owned();
        let dir = TempDir::new("corpus", &[(name.as_str(), std::fs::read(&from).unwrap())]);
        let opened = engine.open_folder(&dir.0).unwrap();
        match opened.first {
            Some(Err(EngineError::Unsupported(m))) => assert_eq!(m, *want, "{path}"),
            other => panic!("{path}: {other:?}"),
        }
        assert_eq!(
            opened.strip.photos[0].kind,
            PhotoKind::Unsupported(want.to_string()),
            "{path}"
        );
        nothing_open(&engine);
    }
}
