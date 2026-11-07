//! LIB-09's thumbnails through the contract: `RequestThumbnails`, then a
//! `ThumbnailReady` per photo asked for, and the thumbnail fetched by
//! photo id as one JPEG tile message. Kept on disk when the engine has a
//! cache directory. Headless: thumbnails never touch the GPU.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use genzo_engine::Engine;
use genzo_engine::api::{Command, CommandResult, Event, PixelFormat};

/// A JPEG file whose EXIF segment holds a `w` × `h` preview of one color.
fn jpeg_with_preview(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let solid = |w: u32, h: u32, c: [u8; 3]| -> Vec<u8> {
        let px: Vec<u8> = (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
        genzo_imgio::jpeg::encode_rgba(&px, w, h, 90).unwrap()
    };
    let mut app1 = b"Exif\0\0II*\0\x08\0\0\0\0\0".to_vec();
    app1.extend_from_slice(&solid(w, h, rgb));
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&solid(64, 48, [128; 3])[2..]);
    out
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("genzo-thumbs-it-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Each `ThumbnailReady`: the photo, and whether it has one.
type Got = Arc<Mutex<Vec<(u64, bool)>>>;

fn start(cache: PathBuf) -> (Engine, Got) {
    let got: Got = Arc::default();
    let g = Arc::clone(&got);
    let engine = Engine::headless()
        .with_cache_dir(cache)
        .with_event_sink(Arc::new(move |e| {
            if let Event::ThumbnailReady { photo, found } = e {
                g.lock().unwrap().push((photo, found));
            }
        }));
    (engine, got)
}

#[test]
fn request_thumbnails_sends_each_and_serves_it_by_photo_id() {
    let dir = TempDir::new("contract");
    let photos = dir.0.join("photos");
    std::fs::write(
        photos.join("a.jpg"),
        jpeg_with_preview(600, 400, [40, 80, 120]),
    )
    .unwrap();
    std::fs::write(photos.join("b.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(photos.join("c.jpg"), jpeg_with_preview(300, 450, [9; 3])).unwrap();

    let (engine, got) = start(dir.0.join("cache"));
    // Headless, the first photo does not open; the strip is there.
    let strip = engine.open_folder(&photos).unwrap().strip;
    let ids: Vec<u64> = strip.photos.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(engine.thumbnail(ids[0]), None, "not made yet");

    // An id not in the strip is passed over.
    let result = engine
        .execute(Command::RequestThumbnails {
            photos: vec![ids[2], 999, ids[0], ids[1]],
        })
        .unwrap();
    assert_eq!(result, CommandResult::Done);
    engine.wait_thumbnails_idle();
    let mut events = got.lock().unwrap().clone();
    events.sort_unstable();
    let mut want = vec![(ids[0], true), (ids[1], false), (ids[2], true)];
    want.sort_unstable();
    assert_eq!(events, want);

    let a = engine.thumbnail(ids[0]).expect("a's thumbnail");
    let tiles = genzo_engine::split_frame(&a).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].0.format, PixelFormat::JpegSrgb);
    assert_eq!((tiles[0].0.rect.width, tiles[0].0.rect.height), (320, 213));
    let c = engine.thumbnail(ids[2]).expect("c's thumbnail");
    let (h, _) = &genzo_engine::split_frame(&c).unwrap()[0];
    assert_eq!((h.rect.width, h.rect.height), (213, 320));
    assert_eq!(engine.thumbnail(ids[1]), None, "a PNG has no preview");
    assert_eq!(engine.thumbnail(999), None);

    // Asked again: answered at once from what was made.
    got.lock().unwrap().clear();
    engine.request_thumbnails(&[ids[0]]);
    assert_eq!(*got.lock().unwrap(), [(ids[0], true)]);
    drop(engine);

    // A later session reads them from disk: the same bytes.
    let (engine, got) = start(dir.0.join("cache"));
    let strip = engine.open_folder(&photos).unwrap().strip;
    let id = strip.photos[0].id;
    engine.request_thumbnails(&[id]);
    engine.wait_thumbnails_idle();
    assert_eq!(*got.lock().unwrap(), [(id, true)]);
    assert_eq!(engine.thumbnail(id).unwrap(), a);
}
