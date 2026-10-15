//! The engine: the backend's only public face.
//!
//! The shell and the cli talk to the backend through this crate alone. It
//! exposes commands, queries, events and a frame stream, and re-exports the
//! contract types so callers never depend on `genzo-model` directly.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use genzo_jobs::Generations;
use genzo_model::api::{
    AdapterInfo, AdapterKind, Command, EngineInfo, Event, Handshake, HandshakeResult, Query,
    QueryResult, SpikeSource,
};
use genzo_model::contract::ContractVersion;
use genzo_model::frame::PixelFormat;
use genzo_model::frame::{Rect, TILE_SIZE, TileHeader};
use genzo_pipeline::gpu::spike::{DisplaySpace, SpikeParams};
use genzo_pipeline::gpu::thread::RenderThread;
use genzo_pipeline::gpu::{AdapterChoice, GpuError};

pub mod contract;
pub mod files;

/// The contract types, re-exported for the shell and the cli.
pub mod api {
    pub use genzo_model::CONTRACT_VERSION;
    pub use genzo_model::api::*;
    pub use genzo_model::contract::ContractVersion;
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
const PROXY_JPEG_QUALITY: u8 = 80;

/// Most bands a drag preview is split into. Each band is encoded on its own
/// thread and decoded in its own `createImageBitmap` call in the webview.
const PROXY_MAX_BANDS: usize = 4;

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
    #[error("no GPU: {0}")]
    NoGpu(String),
    #[error("{0}")]
    Gpu(String),
}

impl From<GpuError> for EngineError {
    fn from(e: GpuError) -> Self {
        Self::Gpu(e.to_string())
    }
}

impl EngineError {
    fn decode(e: genzo_imgio::DecodeError) -> Self {
        Self::Decode(e.to_string())
    }
}

/// One engine instance. Thread-safe; the shell keeps one for the app's
/// lifetime.
#[derive(Debug)]
pub struct Engine {
    adapter: AdapterInfo,
    /// The render thread, if a GPU adapter (hardware or software) opened.
    gpu: Option<RenderThread>,
    generations: Generations,
    /// Finished frames by generation: the concatenated tile messages.
    frames: Mutex<FrameStore>,
}

#[derive(Debug, Default)]
struct FrameStore {
    by_generation: BTreeMap<u64, Vec<u8>>,
    order: VecDeque<u64>,
}

impl FrameStore {
    fn put(&mut self, generation: u64, bytes: Vec<u8>) {
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
    /// forces the software adapter.
    pub fn new() -> Self {
        Self::start(Some(AdapterChoice::from_env()))
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
        Self {
            adapter,
            gpu,
            generations: Generations::new(),
            frames: Mutex::new(FrameStore::default()),
        }
    }

    pub fn info(&self) -> EngineInfo {
        EngineInfo {
            engine_version: ENGINE_VERSION.into(),
            contract_version: genzo_model::CONTRACT_VERSION.into(),
            adapter: self.adapter.clone(),
        }
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
        match query {
            Query::GetEngineInfo => QueryResult::EngineInfo { info: self.info() },
        }
    }

    fn gpu(&self) -> Result<&RenderThread, EngineError> {
        self.gpu.as_ref().ok_or_else(|| {
            EngineError::NoGpu(self.adapter.fallback_reason.clone().unwrap_or_default())
        })
    }

    /// Runs a command and returns the events it produced.
    pub fn execute(&self, command: Command) -> Result<Vec<Event>, EngineError> {
        match command {
            Command::RenderTestPattern {
                width,
                height,
                phase,
                format,
            } => {
                check_size(width, height)?;
                check_format(format)?;
                let started = Instant::now();
                let generation = self.generations.next();
                let Some(tile_count) =
                    self.store_frame(generation, width, height, format, |rect| {
                        genzo_pipeline::test_pattern::render_tile(width, height, rect, phase)
                    })
                else {
                    return Ok(Vec::new());
                };
                Ok(vec![Event::RenderFinished {
                    generation,
                    tile_count,
                    render_ms: started.elapsed().as_secs_f64() * 1000.0,
                    gpu_ms: None,
                }])
            }
            Command::LoadSpikeImage {
                source: SpikeSource::Synthetic { width, height },
            } => {
                check_size(width, height)?;
                // The CFA pattern repeats every two pixels; keep it whole.
                let image = genzo_pipeline::gpu::synthetic::scene_mosaic(width & !1, height & !1);
                let (width, height) = (image.width, image.height);
                self.gpu()?.set_image(Arc::new(image))?;
                Ok(vec![Event::SpikeImageLoaded { width, height }])
            }
            Command::RenderSpike {
                width,
                height,
                exposure_ev,
                wb,
                format,
                interactive,
                proxy,
            } => {
                check_size(width, height)?;
                check_format(format)?;
                let started = Instant::now();
                let generation = self.generations.next();
                // The preview only makes sense during a drag.
                let proxy = proxy && interactive;
                let params = SpikeParams {
                    wb,
                    exposure_ev,
                    space: match format {
                        PixelFormat::Rgba8Srgb | PixelFormat::JpegSrgb => DisplaySpace::Srgb,
                        PixelFormat::Rgba8DisplayP3 | PixelFormat::JpegDisplayP3 => {
                            DisplaySpace::DisplayP3
                        }
                    },
                    interactive,
                    proxy,
                };
                // None: a newer render replaced this one on the render thread.
                let Some(frame) = self.gpu()?.render(params, (width, height))? else {
                    return Ok(Vec::new());
                };
                if proxy {
                    let Some(band_count) = self.store_jpeg_bands(generation, &frame, format) else {
                        return Ok(Vec::new());
                    };
                    return Ok(vec![Event::RenderFinished {
                        generation,
                        tile_count: band_count,
                        render_ms: started.elapsed().as_secs_f64() * 1000.0,
                        gpu_ms: frame.gpu_ms,
                    }]);
                }
                let row = frame.width as usize * 4;
                let Some(tile_count) =
                    self.store_frame(generation, frame.width, frame.height, format, |rect| {
                        let mut out =
                            Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
                        for y in rect.y..rect.y + rect.height {
                            let start = y as usize * row + rect.x as usize * 4;
                            out.extend_from_slice(
                                &frame.rgba[start..start + rect.width as usize * 4],
                            );
                        }
                        out
                    })
                else {
                    return Ok(Vec::new());
                };
                Ok(vec![Event::RenderFinished {
                    generation,
                    tile_count,
                    render_ms: started.elapsed().as_secs_f64() * 1000.0,
                    gpu_ms: frame.gpu_ms,
                }])
            }
        }
    }

    /// Splits a `width` × `height` frame into tiles, encodes them and keeps
    /// the result for [`Engine::frame`]. Returns the tile count, or `None`
    /// when a newer generation made this one stale (latest wins).
    fn store_frame(
        &self,
        generation: u64,
        width: u32,
        height: u32,
        format: PixelFormat,
        mut tile_pixels: impl FnMut(Rect) -> Vec<u8>,
    ) -> Option<u32> {
        let tiles = genzo_pipeline::tiles::grid(width, height, TILE_SIZE);
        let mut frame = Vec::with_capacity(
            width as usize * height as usize * format.bytes_per_pixel().unwrap_or(0)
                + tiles.len() * genzo_model::frame::TILE_HEADER_LEN,
        );
        for rect in &tiles {
            if self.generations.is_stale(generation) {
                return None;
            }
            let header = TileHeader {
                generation,
                rect: *rect,
                mip_level: 0,
                format,
                image_width: width,
                image_height: height,
            };
            frame.extend_from_slice(&header.encode(&tile_pixels(*rect)));
        }
        self.frames
            .lock()
            .expect("frame store poisoned")
            .put(generation, frame);
        Some(tiles.len() as u32)
    }

    /// Splits a frame into horizontal bands, encodes each as a JPEG on the
    /// rayon pool and keeps the messages for [`Engine::frame`]. Returns the
    /// band count, or `None` when a newer generation made this one stale.
    fn store_jpeg_bands(
        &self,
        generation: u64,
        frame: &genzo_pipeline::gpu::spike::Frame,
        format: PixelFormat,
    ) -> Option<u32> {
        use rayon::prelude::*;

        let (w, h) = (frame.width as usize, frame.height as usize);
        // Band heights are multiples of the 16-row JPEG block, so no band
        // pads or blurs across its edge.
        let bands = rayon::current_num_threads().clamp(1, PROXY_MAX_BANDS);
        let band_h = (h.div_ceil(bands)).div_ceil(16) * 16;
        let rows: Vec<(usize, usize)> = (0..h)
            .step_by(band_h)
            .map(|y| (y, band_h.min(h - y)))
            .collect();
        let encoded: Vec<Vec<u8>> = rows
            .par_iter()
            .map(|&(y, rows)| {
                genzo_imgio::jpeg::encode_rgba(
                    &frame.rgba[y * w * 4..(y + rows) * w * 4],
                    w as u32,
                    rows as u32,
                    PROXY_JPEG_QUALITY,
                )
                .ok()
            })
            .collect::<Option<_>>()?;
        if self.generations.is_stale(generation) {
            return None;
        }
        let mut message = Vec::with_capacity(
            encoded.iter().map(Vec::len).sum::<usize>()
                + encoded.len() * genzo_model::frame::TILE_HEADER_LEN,
        );
        for (&(y, rows), jpeg) in rows.iter().zip(&encoded) {
            let header = TileHeader {
                generation,
                rect: Rect {
                    x: 0,
                    y: y as u32,
                    width: w as u32,
                    height: rows as u32,
                },
                mip_level: 0,
                format: format.as_jpeg(),
                image_width: w as u32,
                image_height: h as u32,
            };
            message.extend_from_slice(&header.encode(jpeg));
        }
        let count = rows.len() as u32;
        self.frames
            .lock()
            .expect("frame store poisoned")
            .put(generation, message);
        Some(count)
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

/// Callers ask for a color space; JPEG is a transport choice the engine makes.
fn check_format(format: PixelFormat) -> Result<(), EngineError> {
    if format.is_jpeg() {
        return Err(EngineError::InvalidFormat(format));
    }
    Ok(())
}

fn check_size(width: u32, height: u32) -> Result<(), EngineError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PixelFormat;

    fn render(engine: &Engine, w: u32, h: u32) -> u64 {
        let events = engine
            .execute(Command::RenderTestPattern {
                width: w,
                height: h,
                phase: 0.25,
                format: PixelFormat::Rgba8Srgb,
            })
            .unwrap();
        match events.as_slice() {
            [Event::RenderFinished { generation, .. }] => *generation,
            other => panic!("unexpected events {other:?}"),
        }
    }

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
    fn test_pattern_frame_covers_image() {
        let e = Engine::headless();
        let generation = render(&e, 600, 300);
        let bytes = e.frame(generation).unwrap();
        let tiles = split_frame(&bytes).unwrap();
        assert_eq!(tiles.len(), 6);
        assert_eq!(covered_area(&tiles), 600 * 300);
        assert!(tiles.iter().all(|(h, _)| h.generation == generation));
    }

    #[test]
    fn keeps_only_recent_frames() {
        let e = Engine::headless();
        let first = render(&e, 8, 8);
        for _ in 0..KEPT_FRAMES {
            render(&e, 8, 8);
        }
        assert!(e.frame(first).is_none());
    }

    #[test]
    fn rejects_empty_size() {
        let e = Engine::headless();
        assert!(
            e.execute(Command::RenderTestPattern {
                width: 0,
                height: 10,
                phase: 0.0,
                format: PixelFormat::Rgba8Srgb,
            })
            .is_err()
        );
    }

    fn spike(width: u32, height: u32, interactive: bool) -> Command {
        spike_with(width, height, interactive, false)
    }

    fn spike_with(width: u32, height: u32, interactive: bool, proxy: bool) -> Command {
        Command::RenderSpike {
            width,
            height,
            exposure_ev: 0.0,
            wb: [1.0; 3],
            format: PixelFormat::Rgba8DisplayP3,
            interactive,
            proxy,
        }
    }

    #[test]
    fn jpeg_cannot_be_requested() {
        let e = Engine::headless();
        assert!(matches!(
            e.execute(Command::RenderTestPattern {
                width: 8,
                height: 8,
                phase: 0.0,
                format: PixelFormat::JpegSrgb,
            }),
            Err(EngineError::InvalidFormat(PixelFormat::JpegSrgb))
        ));
    }

    #[test]
    fn headless_engine_refuses_gpu_work() {
        let e = Engine::headless();
        assert_eq!(e.info().adapter.kind, AdapterKind::None);
        assert!(matches!(
            e.execute(spike(64, 64, false)),
            Err(EngineError::NoGpu(_))
        ));
    }

    #[test]
    fn spike_renders_through_the_gpu_as_tiles() {
        let e = Engine::new();
        if e.info().adapter.kind == AdapterKind::None {
            assert_ne!(
                std::env::var("GENZO_REQUIRE_GPU").as_deref(),
                Ok("1"),
                "no GPU adapter"
            );
            return;
        }
        let loaded = e
            .execute(Command::LoadSpikeImage {
                source: SpikeSource::Synthetic {
                    width: 1201,
                    height: 800,
                },
            })
            .unwrap();
        // Odd sizes are trimmed to keep the CFA pattern whole.
        assert_eq!(
            loaded,
            vec![Event::SpikeImageLoaded {
                width: 1200,
                height: 800
            }]
        );
        let events = e.execute(spike(600, 600, false)).unwrap();
        let [
            Event::RenderFinished {
                generation,
                tile_count,
                ..
            },
        ] = events.as_slice()
        else {
            panic!("unexpected events {events:?}");
        };
        let bytes = e.frame(*generation).unwrap();
        let tiles = split_frame(&bytes).unwrap();
        assert_eq!(tiles.len() as u32, *tile_count);
        // 1200 × 800 fit into 600 × 600 is 600 × 400.
        assert_eq!(covered_area(&tiles), 600 * 400);
        assert!(
            tiles
                .iter()
                .all(|(h, _)| h.format == PixelFormat::Rgba8DisplayP3 && h.image_width == 600)
        );
    }

    #[test]
    fn drag_preview_is_a_half_size_jpeg() {
        let e = Engine::new();
        if e.info().adapter.kind == AdapterKind::None {
            assert_ne!(
                std::env::var("GENZO_REQUIRE_GPU").as_deref(),
                Ok("1"),
                "no GPU adapter"
            );
            return;
        }
        e.execute(Command::LoadSpikeImage {
            source: SpikeSource::Synthetic {
                width: 1200,
                height: 800,
            },
        })
        .unwrap();
        let frame_of = |command| {
            let events = e.execute(command).unwrap();
            let [Event::RenderFinished { generation, .. }] = events.as_slice() else {
                panic!("unexpected events {events:?}");
            };
            e.frame(*generation).unwrap()
        };

        // 1200 × 800 fit into 600 × 600 is 600 × 400, so the preview is 300 × 200.
        let bytes = frame_of(spike_with(600, 600, true, true));
        let bands = split_frame(&bytes).unwrap();
        assert!(!bands.is_empty());
        assert_eq!(covered_area(&bands), 300 * 200);
        for (header, jpeg) in &bands {
            assert_eq!(header.format, PixelFormat::JpegDisplayP3);
            assert_eq!((header.image_width, header.image_height), (300, 200));
            assert_eq!((header.rect.x, header.rect.width), (0, 300));
            assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        }
        let raw = frame_of(spike_with(600, 600, true, false)).len();
        assert!(
            bytes.len() * 10 < raw,
            "preview is {} of {raw} bytes",
            bytes.len()
        );

        // A preview is only for drags: a settled render ignores the flag.
        let bytes = frame_of(spike_with(600, 600, false, true));
        let tiles = split_frame(&bytes).unwrap();
        assert_eq!(covered_area(&tiles), 600 * 400);
        assert!(
            tiles
                .iter()
                .all(|(h, _)| h.format == PixelFormat::Rgba8DisplayP3)
        );
    }
}
