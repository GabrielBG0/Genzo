//! The engine: the backend's only public face.
//!
//! The shell and the cli talk to the backend through this crate alone. It
//! exposes commands, queries, events and a frame stream, and re-exports the
//! contract types so callers never depend on `genzo-model` directly.
//!
//! A command answers with its result; every event goes to the event sink
//! ([`Engine::with_event_sink`]), from whichever thread produced it.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use genzo_color::white_balance::Preset;
use genzo_jobs::Generations;
use genzo_model::api::{
    AdapterInfo, AdapterKind, Command, CommandResult, EngineInfo, Event, Handshake,
    HandshakeResult, Query, QueryResult, WhiteBalancePreset,
};
use genzo_model::contract::ContractVersion;
use genzo_model::frame::PixelFormat;
use genzo_model::frame::{Rect, TileHeader};
use genzo_pipeline::gpu::thread::RenderThread;
use genzo_pipeline::gpu::{AdapterChoice, GpuError};

pub mod contract;
pub mod develop;
pub mod export;
pub mod files;
pub mod folder;
mod frames;
pub mod histogram;
pub mod history;
mod messages;
mod prefetch;
pub mod preview;
pub mod strip;
pub mod thumbs;

/// The contract types, re-exported for the shell and the cli.
pub mod api {
    pub use genzo_model::CONTRACT_VERSION;
    pub use genzo_model::api::*;
    pub use genzo_model::contract::ContractVersion;
    /// The edit struct and its changes (DEV-01, *Data*; APP-10).
    pub use genzo_model::edit;
    pub use genzo_model::frame::{
        PixelFormat, Rect, TILE_HEADER_LEN, TILE_MAGIC, TILE_SIZE, TileDecodeError, TileHeader,
    };
}

/// Version of the engine build.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How many finished frames are kept for the frontend to fetch.
const KEPT_FRAMES: usize = 4;

/// JPEG quality of drag previews. The frame is replaced by a lossless one when
/// the slider is released, so this trades softness for bytes.
pub(crate) const PROXY_JPEG_QUALITY: u8 = 80;

/// Most bands a drag preview is split into. Each band is encoded on its own
/// thread and decoded in its own `createImageBitmap` call in the webview.
pub(crate) const PROXY_MAX_BANDS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid size {width}×{height}")]
    InvalidSize { width: u32, height: u32 },
    #[error("{0:?} cannot be requested; the engine chooses JPEG itself for previews")]
    InvalidFormat(PixelFormat),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Decode(String),
    /// A file Genzo recognizes but does not render, with the reason the
    /// status bar shows (DEV-01, States).
    #[error("{0}")]
    Unsupported(String),
    /// Truncated, or failing a decoder's bounds checks (DEV-01, States).
    #[error("This file is damaged ({0})")]
    Damaged(String),
    /// A file listed in the strip that is no longer there (LIB-09, States).
    #[error("This file is no longer there ({0})")]
    Missing(String),
    #[error("no GPU: {0}")]
    NoGpu(String),
    #[error("{0}")]
    Gpu(String),
    #[error("no photo is open")]
    NoPhoto,
    /// A command for a photo never opened this session.
    #[error("photo {0} was not opened this session")]
    UnknownPhoto(u64),
    /// A request no frontend sends: a view or a point that is not a number.
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("photo {0} is not in the strip")]
    NotInStrip(u64),
    /// A delta the engine refused; the edit keeps its last valid state.
    #[error("{0}")]
    Refused(String),
}

impl From<GpuError> for EngineError {
    fn from(e: GpuError) -> Self {
        Self::Gpu(e.to_string())
    }
}

impl EngineError {
    /// A decoder's error as the photo state it puts the file in: damaged
    /// when corrupt, else unsupported with its own message as the reason
    /// ("Nikon High Efficiency RAW is not supported yet").
    fn decode(e: genzo_imgio::DecodeError) -> Self {
        use genzo_imgio::DecodeError as D;
        match e {
            D::Corrupt { message, .. } => Self::Damaged(message),
            D::Unsupported(_, reason) => Self::Unsupported(reason),
            D::UnknownFormat => {
                Self::Unsupported("This file's format is not one Genzo reads".into())
            }
            e @ D::NotBuiltIn(..) => Self::Unsupported(e.to_string()),
        }
    }
}

/// One engine instance. Thread-safe; the shell keeps one for the app's
/// lifetime.
#[derive(Debug)]
pub struct Engine {
    adapter: AdapterInfo,
    /// The render thread, if a GPU adapter (hardware or software) opened.
    /// The prefetch worker holds it too.
    gpu: Option<Arc<RenderThread>>,
    generations: Arc<Generations>,
    /// Finished frames by generation: the concatenated tile messages. The
    /// prefetch worker stores its renders here too.
    frames: Arc<Mutex<FrameStore>>,
    /// Where the autobench report goes (`GENZO_AUTOBENCH`), if the app was
    /// started to benchmark itself.
    autobench: Option<PathBuf>,
    /// Develop's photos, edits and histories, and the strip.
    session: Arc<Mutex<develop::Session>>,
    /// Held for a whole photo switch, so two cannot interleave. Exports
    /// wait on it too, so their encoding never delays a switch.
    switching: Arc<Mutex<()>>,
    /// What opens the strip's files.
    decoder: Arc<dyn strip::Decode>,
    /// The neighbors' prefetch: its plan and its cache.
    prefetch: Arc<prefetch::Shared>,
    /// The prefetch worker, started on the first switch.
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Receives what each strip switch shows at once.
    preview_sink: Option<preview::Sink>,
    /// The histogram's counts, their sink and the latest (DEV-03).
    histograms: Arc<histogram::Histograms>,
    /// The strip's thumbnails (LIB-09).
    thumbs: Arc<thumbs::Thumbs>,
    /// The exports in progress (OUT-01).
    exports: Arc<export::Exports>,
    /// Where events go.
    events: Option<EventSink>,
}

/// Receives every event, on the thread that produced it: a command's own,
/// the histogram's waiter, the prefetch worker or the preview pool. It
/// must not call back into the engine.
pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// The engine's event sink, for its `Debug`.
#[derive(Clone)]
pub(crate) struct EventSink(Events);

impl EventSink {
    pub(crate) fn send(&self, event: Event) {
        (self.0)(event)
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

#[derive(Debug, Default)]
pub(crate) struct FrameStore {
    by_generation: BTreeMap<u64, Vec<u8>>,
    order: VecDeque<u64>,
}

impl FrameStore {
    pub(crate) fn put(&mut self, generation: u64, bytes: Vec<u8>) {
        self.by_generation.insert(generation, bytes);
        self.order.push_back(generation);
        while self.order.len() > KEPT_FRAMES {
            if let Some(old) = self.order.pop_front() {
                self.by_generation.remove(&old);
            }
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Starts the engine and its render thread. `GENZO_ADAPTER=software`
    /// forces the software adapter. `GENZO_AUTOBENCH=FILE` starts the app's
    /// self-benchmark, which saves its report to FILE.
    pub fn new() -> Self {
        let mut engine = Self::start(Some(AdapterChoice::from_env()));
        engine.autobench = std::env::var_os("GENZO_AUTOBENCH")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        engine
    }

    /// An engine without a render thread, for work that never renders on the
    /// GPU (contract tooling, decoding, the test pattern).
    pub fn headless() -> Self {
        Self::start(None)
    }

    fn start(choice: Option<AdapterChoice>) -> Self {
        let spawned = match choice {
            Some(choice) => RenderThread::spawn(choice),
            None => Err(GpuError::Device("headless engine, no GPU started".into())),
        };
        let (gpu, adapter) = match spawned {
            Ok(thread) => {
                let info = thread.info();
                let adapter = AdapterInfo {
                    kind: if info.software {
                        AdapterKind::Software
                    } else {
                        AdapterKind::Hardware
                    },
                    name: info.name.clone(),
                    backend: info.backend.clone(),
                    fallback_reason: info.fallback_reason.clone(),
                };
                (Some(thread), adapter)
            }
            Err(e) => (
                None,
                AdapterInfo {
                    kind: AdapterKind::None,
                    name: "none".into(),
                    backend: "none".into(),
                    fallback_reason: Some(e.to_string()),
                },
            ),
        };
        let prefetch: Arc<prefetch::Shared> = Arc::default();
        Self {
            adapter,
            gpu: gpu.map(Arc::new),
            generations: Arc::new(Generations::new()),
            frames: Arc::default(),
            autobench: None,
            session: Arc::default(),
            switching: Arc::default(),
            exports: Arc::default(),
            decoder: Arc::new(strip::FileDecoder),
            thumbs: Arc::new(Self::thumbs_in(None, &prefetch)),
            prefetch,
            worker: Mutex::new(None),
            preview_sink: None,
            histograms: Arc::default(),
            events: None,
        }
    }

    fn thumbs_in(dir: Option<PathBuf>, prefetch: &Arc<prefetch::Shared>) -> thumbs::Thumbs {
        let prefetch = Arc::clone(prefetch);
        thumbs::Thumbs::new(dir, Box::new(move || prefetch.busy()))
    }

    /// This engine, keeping the strip's thumbnails on disk under `dir`
    /// (LIB-09), so a folder opened again reads no photo for them. Without
    /// one they are kept in memory only. Call it before
    /// [`Self::with_event_sink`] or [`Self::with_thumb_sink`].
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.thumbs = Arc::new(Self::thumbs_in(Some(dir), &self.prefetch));
        self
    }

    /// This engine, handing each made thumbnail to `sink`: the photo, and
    /// whether it has one. [`Self::with_event_sink`] sets it to send
    /// `ThumbnailReady`.
    pub fn with_thumb_sink(self, sink: thumbs::ThumbSink) -> Self {
        self.thumbs.set_sink(sink);
        self
    }

    /// This engine, opening the strip's files with `decoder`.
    pub fn with_decoder(mut self, decoder: Arc<dyn strip::Decode>) -> Self {
        self.decoder = decoder;
        self
    }

    /// This engine, handing each strip switch's first pixels to `sink`
    /// before the photo decodes. [`Self::with_event_sink`] sets it to send
    /// `PreviewShown`.
    pub fn with_preview_sink(mut self, sink: preview::PreviewSink) -> Self {
        self.preview_sink = Some(preview::Sink(sink));
        self
    }

    /// This engine, handing each histogram count to `sink`.
    /// [`Self::with_event_sink`] sets it to send `HistogramReady`.
    pub fn with_histogram_sink(mut self, sink: histogram::HistogramSink) -> Self {
        self.histograms = Arc::new(histogram::Histograms::with_sink(sink));
        self
    }

    /// This engine, sending every event to `sink`: the spike's, each
    /// Develop frame's `RenderFinished` (the prefetch's sharper render
    /// too), `PreviewShown`, `HistogramReady` and `ThumbnailReady`. A frame's
    /// `RenderFinished` goes before its `HistogramReady` (DEV-03). It
    /// replaces the preview, histogram and thumbnail sinks.
    pub fn with_event_sink(self, sink: Events) -> Self {
        let (preview, counts, thumbs) = (sink.clone(), sink.clone(), sink.clone());
        let mut engine = self
            .with_thumb_sink(Arc::new(move |photo, found| {
                thumbs(Event::ThumbnailReady { photo, found })
            }))
            .with_preview_sink(Arc::new(move |s: preview::Shown| {
                preview(messages::preview_shown(&s))
            }))
            .with_histogram_sink(Arc::new(move |c: histogram::Counted| {
                counts(messages::histogram_ready(c))
            }));
        engine.events = Some(EventSink(sink));
        engine
    }

    /// Sends `event` to the sink, if there is one.
    fn emit(&self, event: Event) {
        if let Some(sink) = &self.events {
            sink.send(event);
        }
    }

    pub fn info(&self) -> EngineInfo {
        EngineInfo {
            engine_version: ENGINE_VERSION.into(),
            contract_version: genzo_model::CONTRACT_VERSION.into(),
            adapter: self.adapter.clone(),
            autobench: self.autobench.is_some(),
            white_balance_presets: Preset::ALL
                .into_iter()
                .map(|p| {
                    let (temp, tint) = p.temp_tint();
                    WhiteBalancePreset {
                        name: p.name().into(),
                        temp: temp as f32,
                        tint: tint as f32,
                    }
                })
                .collect(),
        }
    }

    /// Whether the GPU has timestamp queries, so renders carry their GPU
    /// time (`RenderFinished.gpu_ms`).
    pub fn gpu_timestamps(&self) -> bool {
        self.gpu.as_ref().is_some_and(|g| g.info().timestamps)
    }

    /// Checks the frontend's contract version against the engine's.
    pub fn handshake(&self, hello: &Handshake) -> HandshakeResult {
        let ours = ContractVersion::current();
        let compatible = ContractVersion::parse(&hello.contract_version)
            .is_some_and(|theirs| theirs.is_compatible_with(ours));
        HandshakeResult {
            compatible,
            engine_contract_version: ours.to_string(),
            engine_version: ENGINE_VERSION.into(),
        }
    }

    pub fn query(&self, query: Query) -> QueryResult {
        self.develop_query(query)
    }

    fn gpu(&self) -> Result<&RenderThread, EngineError> {
        self.gpu.as_deref().ok_or_else(|| {
            EngineError::NoGpu(self.adapter.fallback_reason.clone().unwrap_or_default())
        })
    }

    /// Runs a command and returns its result. Its events go to the sink
    /// before it returns, but for those that come later: the whole photo's
    /// histogram, the prefetch's sharper render.
    pub fn execute(&self, command: Command) -> Result<CommandResult, EngineError> {
        let events = match command {
            Command::SaveAutobenchReport { report } => {
                let path = self.autobench.as_ref().ok_or_else(|| {
                    EngineError::Io("not in autobench mode (GENZO_AUTOBENCH is unset)".into())
                })?;
                write_atomically(path, report.as_bytes())
                    .map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
                Vec::new()
            }
            command => return self.develop_command(command),
        };
        for event in events {
            self.emit(event);
        }
        Ok(CommandResult::Done)
    }

    /// The tile messages of a finished generation, concatenated. Each message
    /// carries its own length, see `api::TileHeader`.
    pub fn frame(&self, generation: u64) -> Option<Vec<u8>> {
        self.frames
            .lock()
            .expect("frame store poisoned")
            .by_generation
            .get(&generation)
            .cloned()
    }
}

impl Drop for Engine {
    /// Stops the prefetch worker before the render thread goes: it holds
    /// the thread too, and a second device in one process crashes WARP.
    fn drop(&mut self) {
        self.exports.stop();
        self.thumbs.stop();
        let worker = self.worker.get_mut().ok().and_then(Option::take);
        if let Some(handle) = worker {
            self.prefetch.stop();
            if let Some(gpu) = &self.gpu {
                // Frees a build the worker waits on.
                let _ = gpu.keep_prefetching(&[]);
            }
            let _ = handle.join();
        }
    }
}

pub(crate) fn check_size(width: u32, height: u32) -> Result<(), EngineError> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(EngineError::InvalidSize { width, height });
    }
    Ok(())
}

/// Splits a concatenated frame into its tiles.
pub fn split_frame(mut bytes: &[u8]) -> Result<Vec<(TileHeader, &[u8])>, api::TileDecodeError> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        if bytes.len() < api::TILE_HEADER_LEN {
            return Err(api::TileDecodeError::TooShort);
        }
        let header_len = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        let payload_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
        let end = header_len + payload_len;
        if bytes.len() < end {
            return Err(api::TileDecodeError::TooShort);
        }
        out.push(TileHeader::decode(&bytes[..end])?);
        bytes = &bytes[end..];
    }
    Ok(out)
}

/// The area of an image covered by its tiles. Used by tests and the cli.
pub fn covered_area(tiles: &[(TileHeader, &[u8])]) -> u64 {
    tiles
        .iter()
        .map(|(h, _)| {
            let Rect { width, height, .. } = h.rect;
            width as u64 * height as u64
        })
        .sum()
}

/// Writes `bytes` to a new sibling of `path`, then renames it into place, so
/// a reader waiting for `path` to appear never sees half a file. The sibling
/// is created fresh (never an existing file) and removed if anything fails.
fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.{}.partial", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_checks_version() {
        let e = Engine::headless();
        let ok = e.handshake(&Handshake {
            contract_version: genzo_model::CONTRACT_VERSION.into(),
        });
        assert!(ok.compatible);
        let bad = e.handshake(&Handshake {
            contract_version: "999.0.0".into(),
        });
        assert!(!bad.compatible);
        let garbage = e.handshake(&Handshake {
            contract_version: "nope".into(),
        });
        assert!(!garbage.compatible);
    }

    #[test]
    fn info_lists_the_white_balance_presets_in_order() {
        let presets = Engine::headless().info().white_balance_presets;
        let names: Vec<_> = presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Daylight",
                "Cloudy",
                "Shade",
                "Tungsten",
                "Fluorescent",
                "Flash"
            ]
        );
        for (p, preset) in presets.iter().zip(Preset::ALL) {
            let (temp, tint) = preset.temp_tint();
            assert_eq!((p.temp, p.tint), (temp as f32, tint as f32), "{}", p.name);
        }
    }

    #[test]
    fn autobench_report_needs_autobench_mode() {
        let mut e = Engine::headless();
        assert!(!e.info().autobench);
        let save = |e: &Engine| {
            e.execute(Command::SaveAutobenchReport {
                report: "{\"ok\":true}".into(),
            })
        };
        assert!(save(&e).is_err());

        let path =
            std::env::temp_dir().join(format!("genzo-autobench-{}.json", std::process::id()));
        e.autobench = Some(path.clone());
        assert!(e.info().autobench);
        assert_eq!(save(&e).unwrap(), CommandResult::Done);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"ok\":true}");
        std::fs::remove_file(&path).unwrap();
        // No temporary sibling is left behind, after success or failure.
        let leftovers = || {
            std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .filter_map(Result::ok)
                .any(|f| {
                    let name = f.file_name().to_string_lossy().into_owned();
                    name.starts_with(&format!(".genzo-autobench-{}", std::process::id()))
                        && name.ends_with(".partial")
                })
        };
        assert!(!leftovers());
        // The write succeeds and the rename fails: the destination is a directory.
        let dir = std::env::temp_dir().join(format!("genzo-autobench-{}-dir", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        e.autobench = Some(dir.clone());
        assert!(save(&e).is_err());
        assert!(!leftovers());
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn rejects_empty_size() {
        let e = Engine::headless();
        assert!(
            e.execute(Command::SetView {
                width: 0,
                height: 10,
                view: api::View::Fit,
                space: api::ColorSpace::Srgb,
            })
            .is_err()
        );
    }

    #[test]
    fn a_synthetic_photo_opens_only_for_the_autobench() {
        let mut e = Engine::headless();
        let open = |e: &Engine| {
            e.execute(Command::OpenSynthetic {
                width: 64,
                height: 48,
            })
        };
        assert!(matches!(open(&e), Err(EngineError::Invalid(_))));
        // In autobench mode it goes to the GPU, which a headless engine has not.
        e.autobench = Some(std::env::temp_dir().join("genzo-unused-report.json"));
        assert!(matches!(open(&e), Err(EngineError::NoGpu(_))));
    }

    #[test]
    fn a_synthetic_photo_is_a_folder_of_one_in_develop() {
        let mut e = Engine::new();
        if e.info().adapter.kind == AdapterKind::None {
            assert_ne!(
                std::env::var("GENZO_REQUIRE_GPU").as_deref(),
                Ok("1"),
                "no GPU adapter"
            );
            return;
        }
        e.autobench = Some(std::env::temp_dir().join("genzo-unused-report.json"));
        let r = e
            .execute(Command::OpenSynthetic {
                width: 641,
                height: 480,
            })
            .unwrap();
        let CommandResult::Folder {
            strip,
            edit,
            reason,
        } = r
        else {
            panic!("{r:?}")
        };
        let edit = edit.expect("it renders");
        assert_eq!(reason, None);
        assert_eq!(strip.current, Some(edit.photo));
        assert_eq!(strip.photos.len(), 1);
        // Even sizes keep the CFA pattern whole.
        assert_eq!((edit.shown.width, edit.shown.height), (640, 480));
        assert_eq!(strip.photos[0].file_name, "Synthetic 640 × 480");
    }
}
