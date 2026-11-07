//! OUT-01 through the engine: an export of the open photo writes a JPEG
//! with its metadata, reports its progress, never writes over a photo the
//! session listed, can be cancelled, and holds the edit it started with.
//! The real render thread and decoder; skipped without a GPU adapter.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::api::{Event, ExportState};
use genzo_engine::develop::{Target, ViewSpec};
use genzo_engine::export::{ExportRequest, ORIGINAL};
use genzo_engine::{Engine, EngineError};
use genzo_imgio::metadata::{Metadata, Tag, read, write};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-export-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self(dir)
    }
    fn photos(&self) -> PathBuf {
        self.0.join("photos")
    }
    fn out(&self, name: &str) -> PathBuf {
        self.0.join("out").join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Seen = Arc<Mutex<Vec<(u64, u32, ExportState)>>>;

fn engine() -> Option<(Engine, Seen)> {
    let seen: Seen = Arc::default();
    let s = Arc::clone(&seen);
    let engine = Engine::new().with_event_sink(Arc::new(move |e| {
        if let Event::ExportProgress {
            job,
            rows_done,
            state,
            ..
        } = e
        {
            s.lock().unwrap().push((job, rows_done, state));
        }
    }));
    if engine.info().adapter.name == "none" {
        assert_ne!(
            std::env::var("GENZO_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "no GPU"
        );
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some((engine, seen))
}

/// A `w` × `h` JPEG with camera EXIF and a GPS position.
fn camera_jpeg(w: u32, h: u32) -> Vec<u8> {
    let le = |v: u32| v.to_le_bytes().to_vec();
    let rational = |n: u32, d: u32| [le(n), le(d)].concat();
    let ascii = |tag, text: &[u8]| Tag {
        tag,
        ty: 2,
        count: text.len() as u32,
        value: text.to_vec(),
    };
    let meta = Metadata {
        ifd0: vec![ascii(271, b"Genzo Cam\0"), ascii(272, b"Test 1\0")],
        exif: vec![
            Tag {
                tag: 34855,
                ty: 3,
                count: 1,
                value: 200u16.to_le_bytes().to_vec(),
            },
            ascii(0x9003, b"2026:10:12 17:02:04\0"),
        ],
        gps: vec![
            ascii(1, b"S\0"),
            Tag {
                tag: 2,
                ty: 5,
                count: 3,
                value: [rational(22, 1), rational(54, 1), rational(1234, 100)].concat(),
            },
        ],
    };
    let app1 = write(&meta, (w, h), "camera").app1;
    let px: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 120, 255])
        .collect();
    let body = genzo_imgio::jpeg::encode_rgba(&px, w, h, 95).unwrap();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&body[2..]);
    out
}

fn request(photo: u64, path: &Path) -> ExportRequest {
    ExportRequest {
        photo,
        path: path.to_path_buf(),
        long_edge: None,
        quality: 90,
        replace: false,
    }
}

/// The events of `job`, and its last state.
fn states(seen: &Seen, job: u64) -> Vec<(u32, ExportState)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|e| e.0 == job)
        .map(|e| (e.1, e.2.clone()))
        .collect()
}

fn ends(events: &[(u32, ExportState)]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(
                e.1,
                ExportState::Finished { .. } | ExportState::Failed { .. } | ExportState::Cancelled
            )
        })
        .count()
}

#[test]
fn an_export_writes_the_photo_with_its_metadata() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, seen)) = engine() else {
        return;
    };
    let dir = TempDir::new("writes");
    std::fs::write(dir.photos().join("a.jpg"), camera_jpeg(1300, 700)).unwrap();
    engine
        .set_view(ViewSpec::Fit((400, 300)), Target::Srgb)
        .ok();
    let opened = engine.open_folder(&dir.photos()).unwrap();
    opened.first.unwrap().unwrap();
    let id = opened.strip.photos[0].id;
    engine.apply(&[Change::Exposure(0.3)], Phase::End).unwrap();

    let out = dir.out("a.jpg");
    let job = engine.export_jpeg(request(id, &out)).unwrap();
    engine.wait_exports_idle();
    let events = states(&seen, job);
    assert_eq!(events.first().map(|e| &e.1), Some(&ExportState::Queued));
    assert_eq!(ends(&events), 1, "{events:?}");
    assert_eq!(
        events.last().unwrap(),
        &(700, ExportState::Finished { notes: vec![] })
    );
    let rows: Vec<u32> = events.iter().map(|e| e.0).collect();
    assert!(rows.windows(2).all(|w| w[0] <= w[1]), "{rows:?}");

    let file = std::fs::read(&out).unwrap();
    assert!(!genzo_export::jpeg::temporary(&out).exists());
    let img = genzo_imgio::decode(&file, Default::default()).unwrap();
    assert_eq!((img.width, img.height), (1300, 700));
    assert_eq!(img.orientation, 1);
    let meta = read(&file).expect("the export carries EXIF");
    let tag = |ifd: &[Tag], n: u16| ifd.iter().find(|t| t.tag == n).map(|t| t.value.clone());
    assert_eq!(tag(&meta.ifd0, 271).unwrap(), b"Genzo Cam\0");
    assert_eq!(tag(&meta.exif, 34855).unwrap(), 200u16.to_le_bytes());
    assert_eq!(meta.gps.iter().find(|t| t.tag == 2).unwrap().count, 3);
    // `read` leaves out the pixel dimensions, which a rewrite sets anew:
    // the segment as written has the export's.
    let at = file.windows(6).position(|w| w == b"Exif\0\0").unwrap();
    let written = genzo_imgio::metadata::from_exif(&file[at..]).unwrap();
    assert_eq!(tag(&written.exif, 40962).unwrap(), 1300u32.to_le_bytes());
    assert_eq!(tag(&written.exif, 40963).unwrap(), 700u32.to_le_bytes());
    assert!(file.windows(12).any(|w| w == b"ICC_PROFILE\0"));

    // Resized: the long edge fitted, the other side rounded; never
    // enlarged.
    let small = dir.out("small.jpg");
    let mut r = request(id, &small);
    r.long_edge = Some(400);
    let job = engine.export_jpeg(r).unwrap();
    engine.wait_exports_idle();
    let events = states(&seen, job);
    assert_eq!(ends(&events), 1, "{events:?}");
    assert_eq!(
        events.last().unwrap(),
        &(215, ExportState::Finished { notes: vec![] })
    );
    let file = std::fs::read(&small).unwrap();
    let img = genzo_imgio::decode(&file, Default::default()).unwrap();
    assert_eq!((img.width, img.height), (400, 215));
    let at = file.windows(6).position(|w| w == b"Exif\0\0").unwrap();
    let written = genzo_imgio::metadata::from_exif(&file[at..]).unwrap();
    assert_eq!(tag(&written.exif, 40962).unwrap(), 400u32.to_le_bytes());
    // Every part of it is the full-size export's, downscaled: close to box
    // means of its decoded codes (the export averages in linear light, and
    // both are JPEGs, so close and not equal).
    let full = genzo_imgio::decode(&std::fs::read(&out).unwrap(), Default::default()).unwrap();
    let rgb16 = |img: &genzo_imgio::DecodedImage| match &img.pixels {
        genzo_imgio::Pixels::Rgba16 { data, .. } => data.clone(),
        other => panic!("{other:?}"),
    };
    let (big_px, small_px) = (rgb16(&full), rgb16(&img));
    let starts = |len: u32, n: u32| -> Vec<u32> { (0..=n).map(|i| i * len / n).collect() };
    let (cs, rs) = (starts(1300, 400), starts(700, 215));
    let mut worst = 0.0f64;
    for oy in 0..215u32 {
        for ox in 0..400u32 {
            for c in 0..3 {
                let mut sum = 0.0;
                let mut n = 0.0;
                for y in rs[oy as usize]..rs[oy as usize + 1] {
                    for x in cs[ox as usize]..cs[ox as usize + 1] {
                        sum += f64::from(big_px[((y * 1300 + x) * 4 + c) as usize]) / 257.0;
                        n += 1.0;
                    }
                }
                let got = f64::from(small_px[((oy * 400 + ox) * 4 + c) as usize]) / 257.0;
                worst = worst.max((got - sum / n).abs());
            }
        }
    }
    assert!(
        worst < 6.0,
        "{worst:.1} codes from the full-size export, downscaled"
    );
    let big = dir.out("big.jpg");
    let mut r = request(id, &big);
    r.long_edge = Some(5000);
    engine.export_jpeg(r).unwrap();
    engine.wait_exports_idle();
    let img = genzo_imgio::decode(&std::fs::read(&big).unwrap(), Default::default()).unwrap();
    assert_eq!((img.width, img.height), (1300, 700), "never enlarged");
}

#[test]
fn originals_existing_files_and_busy_paths_are_refused() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, _)) = engine() else { return };
    let dir = TempDir::new("refused");
    let first = dir.0.join("first");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::write(first.join("old.jpg"), camera_jpeg(64, 48)).unwrap();
    std::fs::write(dir.photos().join("a.jpg"), camera_jpeg(64, 48)).unwrap();
    engine
        .set_view(ViewSpec::Fit((200, 150)), Target::Srgb)
        .ok();
    engine.open_folder(&first).unwrap();
    let opened = engine.open_folder(&dir.photos()).unwrap();
    let id = opened.strip.photos[0].id;
    let refused = |path: &Path, replace: bool| {
        let mut r = request(id, path);
        r.replace = replace;
        match engine.export_jpeg(r) {
            Err(EngineError::Refused(why)) => why,
            other => panic!("{}: {other:?}", path.display()),
        }
    };
    // The photo itself, through another spelling of its folder.
    let spelled = dir.photos().join(".").join("a.jpg");
    assert_eq!(refused(&spelled, true), ORIGINAL);
    // A photo of a folder opened before.
    assert_eq!(refused(&first.join("old.jpg"), true), ORIGINAL);
    // An existing file without replace; with it, it is replaced.
    let out = dir.out("x.jpg");
    std::fs::write(&out, b"before").unwrap();
    assert_eq!(refused(&out, false), "x.jpg already exists");
    let mut r = request(id, &out);
    r.replace = true;
    engine.export_jpeg(r).unwrap();
    engine.wait_exports_idle();
    assert!(std::fs::read(&out).unwrap().starts_with(&[0xFF, 0xD8]));
    // Not a .jpg, no folder, a resize (2b).
    assert!(matches!(
        engine.export_jpeg(request(id, &dir.out("x.png"))),
        Err(EngineError::Invalid(_))
    ));
    assert!(matches!(
        engine.export_jpeg(request(id, &dir.0.join("none").join("x.jpg"))),
        Err(EngineError::Io(_))
    ));
    let mut r = request(id, &dir.out("small.jpg"));
    r.long_edge = Some(31);
    assert!(matches!(
        engine.export_jpeg(r),
        Err(EngineError::Invalid(_))
    ));
    // Nothing was written over the photos.
    assert_eq!(
        std::fs::read(dir.photos().join("a.jpg")).unwrap(),
        camera_jpeg(64, 48)
    );
}

#[test]
fn a_cancelled_export_writes_nothing_and_an_edit_meanwhile_changes_nothing() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((engine, seen)) = engine() else {
        return;
    };
    let dir = TempDir::new("cancel");
    std::fs::write(dir.photos().join("a.jpg"), camera_jpeg(3000, 2200)).unwrap();
    engine
        .set_view(ViewSpec::Fit((400, 300)), Target::Srgb)
        .ok();
    let opened = engine.open_folder(&dir.photos()).unwrap();
    let id = opened.strip.photos[0].id;
    engine.apply(&[Change::Exposure(0.4)], Phase::End).unwrap();

    // Exported during a drag: the gesture ends first, recording its step.
    engine
        .apply(&[Change::Shadows(30.0)], Phase::Start)
        .unwrap();
    engine
        .apply(&[Change::Shadows(35.0)], Phase::Update)
        .unwrap();
    let undo_before = engine.edit_state().unwrap().undo;
    engine
        .export_jpeg(request(id, &dir.out("drag.jpg")))
        .unwrap();
    engine.wait_exports_idle();
    let state = engine.edit_state().unwrap();
    assert_ne!(state.undo, undo_before, "the drag's step is recorded");
    assert_eq!(state.edit.shadows, 35.0);
    engine.undo().unwrap();

    // Two commands for one path at once, as a double click sends them:
    // one starts, the other is refused.
    let twice = dir.out("twice.jpg");
    let results: Vec<_> = std::thread::scope(|scope| {
        let a = scope.spawn(|| engine.export_jpeg(request(id, &twice)));
        let b = scope.spawn(|| engine.export_jpeg(request(id, &twice)));
        vec![a.join().unwrap(), b.join().unwrap()]
    });
    engine.wait_exports_idle();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(std::fs::read(&twice).unwrap().starts_with(&[0xFF, 0xD8]));

    // Alone.
    let alone = dir.out("alone.jpg");
    engine.export_jpeg(request(id, &alone)).unwrap();
    engine.wait_exports_idle();
    // With edits and renders while it runs: the same bytes.
    let busy = dir.out("busy.jpg");
    engine.export_jpeg(request(id, &busy)).unwrap();
    engine.apply(&[Change::Exposure(-1.0)], Phase::End).unwrap();
    engine.apply(&[Change::Contrast(40.0)], Phase::End).unwrap();
    engine.wait_exports_idle();
    assert_eq!(
        std::fs::read(&alone).unwrap(),
        std::fs::read(&busy).unwrap()
    );

    // Cancelled at once: no file, no temporary file, one cancelled end.
    let gone = dir.out("gone.jpg");
    let job = engine.export_jpeg(request(id, &gone)).unwrap();
    // A second export to the same path, while the first runs, is refused,
    // with replace too: the first one's file does not exist yet.
    let mut again = request(id, &gone);
    again.replace = true;
    match engine.export_jpeg(again) {
        Err(EngineError::Refused(why)) => assert_eq!(why, "An export to this file is in progress"),
        other => panic!("{other:?}"),
    }
    engine.cancel_export(job).unwrap();
    engine.wait_exports_idle();
    let events = states(&seen, job);
    assert_eq!(ends(&events), 1, "{events:?}");
    let last = &events.last().unwrap().1;
    if *last == ExportState::Cancelled {
        assert!(!gone.exists());
    } else {
        // It finished before the cancel came: a whole file.
        assert!(matches!(last, ExportState::Finished { .. }), "{last:?}");
    }
    assert!(!genzo_export::jpeg::temporary(&gone).exists());
    // Cancelling an export that ended is no error.
    engine.cancel_export(job).unwrap();
}
