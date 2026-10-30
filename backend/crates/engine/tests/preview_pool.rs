//! A switch's preview while a decode fills rayon's global pool (DEV-01,
//! States: Opening): it is read and prepared on a pool of its own, so it
//! never waits for the decode's work to drain.
//!
//! A fake decoder holds every thread of the global pool for the names with
//! "gate" in them, until the test opens the gate. A "big" one's preview is
//! a real 2048 x 2048 JPEG with restart markers, which imgio decodes in
//! bands on the pool of its caller. This file is its own test
//! binary because of that: any other test here using rayon would stall.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use genzo_engine::preview::Shown;
use genzo_engine::strip::Decode;
use genzo_engine::{Engine, EngineError};
use genzo_imgio::{ColorEncoding, Preview};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
struct State {
    open: bool,
    /// Global pool threads held now.
    held: usize,
    /// Gated decodes started.
    gated: Vec<String>,
}

#[derive(Debug, Default)]
struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        self.state.lock().unwrap().open = true;
        self.changed.notify_all();
    }

    fn wait_open(&self) {
        let mut s = self.state.lock().unwrap();
        while !s.open {
            s = self.changed.wait(s).unwrap();
        }
    }

    /// Waits until the decode of `name` holds the whole global pool.
    fn wait_holding(&self, name: &str) {
        let mut s = self.state.lock().unwrap();
        while !s.gated.iter().any(|n| n == name) {
            s = self.changed.wait(s).unwrap();
        }
    }
}

/// Opens the gate when dropped, before the engine is: the engine's drop
/// waits for the prefetch worker, which may wait at the gate.
struct OpenOnDrop(Arc<Gate>);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

#[derive(Debug, Default)]
struct Fake {
    gate: Arc<Gate>,
}

fn source_for(name: &str) -> RawSource {
    let n = name.bytes().map(u32::from).sum::<u32>() % 7;
    with_camera_profile(scene_raw(300 + 20 * n, 220 + 10 * n))
}

impl Decode for Fake {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains("gate") && !self.gate.state.lock().unwrap().open {
            // Every global pool thread but this one, if it is one, waits
            // at the gate; then this one does.
            let others =
                rayon::current_num_threads() - usize::from(rayon::current_thread_index().is_some());
            for _ in 0..others {
                let gate = self.gate.clone();
                rayon::spawn(move || {
                    gate.state.lock().unwrap().held += 1;
                    gate.changed.notify_all();
                    gate.wait_open();
                    gate.state.lock().unwrap().held -= 1;
                });
            }
            let mut s = self.gate.state.lock().unwrap();
            while s.held < others && !s.open {
                s = self.gate.changed.wait(s).unwrap();
            }
            s.gated.push(name.clone());
            self.gate.changed.notify_all();
            drop(s);
            self.gate.wait_open();
        }
        Ok((source_for(&name), 1))
    }

    /// A 4 × 2 Display P3 preview: converted for an sRGB frame. A big
    /// name's is a real JPEG.
    fn preview(&self, path: &Path) -> Result<Option<Preview>, EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains("big") {
            return Ok(genzo_imgio::decode_preview(big_preview()).unwrap());
        }
        let v = name.bytes().fold(0u8, u8::wrapping_add);
        Ok(Some(Preview {
            width: 4,
            height: 2,
            rgba: [v, 2, 3, 255].repeat(8),
            orientation: 1,
            color: ColorEncoding::DisplayP3,
        }))
    }
}

/// A TIFF holding a 2048 x 2048 4:2:0 JPEG with a restart every MCU row:
/// a preview imgio decodes in bands.
fn big_preview() -> &'static [u8] {
    static FILE: OnceLock<Vec<u8>> = OnceLock::new();
    FILE.get_or_init(|| {
        let side = 2048_u16;
        let rgb: Vec<u8> = (0..u32::from(side) * u32::from(side))
            .flat_map(|i| [(i % 2048 / 8) as u8, (i / 2048 / 8) as u8, 128])
            .collect();
        let mut file = b"II*\0".to_vec();
        let mut e = jpeg_encoder::Encoder::new(&mut file, 80);
        e.set_restart_interval(side / 16);
        e.encode(&rgb, side, side, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        file
    })
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str, names: &[&str]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("genzo-preview-pool-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"stub").unwrap();
        }
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the preview sink received.
#[derive(Default)]
struct Seen {
    shown: Mutex<Vec<Shown>>,
    changed: Condvar,
}

impl Seen {
    /// Whether a preview was shown for `photo` within 10 s.
    fn shown_within(&self, photo: u64) -> bool {
        let s = self.shown.lock().unwrap();
        let (s, _) = self
            .changed
            .wait_timeout_while(s, Duration::from_secs(10), |s| {
                !s.iter().any(|x| x.photo == photo && x.preview.is_some())
            })
            .unwrap();
        s.iter().any(|x| x.photo == photo && x.preview.is_some())
    }
}

fn engine(fake: &Arc<Fake>) -> Option<(Arc<Engine>, Arc<Seen>)> {
    let seen = Arc::new(Seen::default());
    let s = seen.clone();
    let engine = Engine::new()
        .with_decoder(fake.clone())
        .with_preview_sink(Arc::new(move |shown| {
            s.shown.lock().unwrap().push(shown);
            s.changed.notify_all();
        }));
    if engine.info().adapter.name == "none" {
        if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
            panic!("no GPU adapter");
        }
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    Some((Arc::new(engine), seen))
}

#[test]
fn a_cold_switch_shows_its_preview_while_its_decode_fills_the_pool() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some((engine, seen)) = engine(&fake) else {
        return;
    };
    let _open = OpenOnDrop(fake.gate.clone());
    let names = ["p0.jpg", "p1.jpg", "p2.jpg", "p3.jpg", "p4_gate.jpg"];
    let dir = TempDir::new("own", &names);
    let p: Vec<u64> = engine
        .open_folder(&dir.0)
        .unwrap()
        .strip
        .photos
        .iter()
        .map(|p| p.id)
        .collect();
    engine.wait_prefetch_idle();

    // p4 is no neighbor of p0: the switch decodes it, and that decode
    // holds the global pool.
    let (e, far) = (engine.clone(), p[4]);
    let switch = std::thread::spawn(move || e.select_photo(far).map(|a| a.state.photo));
    let shown = seen.shown_within(p[4]);
    fake.gate.wait_holding("p4_gate.jpg");
    assert!(!switch.is_finished(), "its decode still holds the pool");
    fake.gate.open();
    assert!(shown, "the preview waited for the decode");
    assert_eq!(switch.join().unwrap().unwrap(), p[4]);
}

#[test]
fn a_cold_switch_shows_its_preview_while_a_neighbor_decode_fills_the_pool() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some((engine, seen)) = engine(&fake) else {
        return;
    };
    let _open = OpenOnDrop(fake.gate.clone());
    let names = ["p0.jpg", "p1_gate.jpg", "p2.jpg", "p3.jpg", "p4_big.jpg"];
    let dir = TempDir::new("neighbor", &names);
    big_preview();
    let p: Vec<u64> = engine
        .open_folder(&dir.0)
        .unwrap()
        .strip
        .photos
        .iter()
        .map(|p| p.id)
        .collect();
    // Prefetch decodes p1, the next neighbor, which holds the global pool.
    fake.gate.wait_holding("p1_gate.jpg");

    // p4 is no neighbor of p0: its preview, decoded in bands, while p1
    // still decodes.
    let (e, far) = (engine.clone(), p[4]);
    let switch = std::thread::spawn(move || e.select_photo(far).map(|a| a.state.photo));
    let shown = seen.shown_within(p[4]);
    fake.gate.open();
    assert!(shown, "the preview waited for the neighbor's decode");
    assert_eq!(switch.join().unwrap().unwrap(), p[4]);
}
