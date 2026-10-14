//! The engine: the backend's only public face.
//!
//! The shell and the cli talk to the backend through this crate alone. It
//! exposes commands, queries, events and a frame stream, and re-exports the
//! contract types so callers never depend on `genzo-model` directly.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use genzo_jobs::Generations;
use genzo_model::api::{
    AdapterInfo, AdapterKind, Command, EngineInfo, Event, Handshake, HandshakeResult, Query,
    QueryResult,
};
use genzo_model::contract::ContractVersion;
use genzo_model::frame::{Rect, TILE_SIZE, TileHeader};

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

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid size {width}×{height}")]
    InvalidSize { width: u32, height: u32 },
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Decode(String),
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
    pub fn new() -> Self {
        Self {
            // The wgpu render thread and its adapter self-test are the 0.D spike.
            adapter: AdapterInfo {
                kind: AdapterKind::None,
                name: "CPU test pattern".into(),
                backend: "none".into(),
                fallback_reason: Some("GPU pipeline not built yet (M0 spike 0.D)".into()),
            },
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

    /// Runs a command and returns the events it produced.
    pub fn execute(&self, command: Command) -> Result<Vec<Event>, EngineError> {
        match command {
            Command::RenderTestPattern {
                width,
                height,
                phase,
                format,
            } => {
                if width == 0 || height == 0 || width > 16384 || height > 16384 {
                    return Err(EngineError::InvalidSize { width, height });
                }
                let started = Instant::now();
                let generation = self.generations.next();
                let tiles = genzo_pipeline::tiles::grid(width, height, TILE_SIZE);
                let mut frame = Vec::with_capacity(
                    width as usize * height as usize * format.bytes_per_pixel()
                        + tiles.len() * genzo_model::frame::TILE_HEADER_LEN,
                );
                for rect in &tiles {
                    // Latest wins: a newer request makes this one stale.
                    if self.generations.is_stale(generation) {
                        return Ok(Vec::new());
                    }
                    let pixels =
                        genzo_pipeline::test_pattern::render_tile(width, height, *rect, phase);
                    let header = TileHeader {
                        generation,
                        rect: *rect,
                        mip_level: 0,
                        format,
                        image_width: width,
                        image_height: height,
                    };
                    frame.extend_from_slice(&header.encode(&pixels));
                }
                self.frames
                    .lock()
                    .expect("frame store poisoned")
                    .put(generation, frame);
                Ok(vec![Event::RenderFinished {
                    generation,
                    tile_count: tiles.len() as u32,
                    render_ms: started.elapsed().as_secs_f64() * 1000.0,
                }])
            }
        }
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
        let e = Engine::new();
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
        let e = Engine::new();
        let generation = render(&e, 600, 300);
        let bytes = e.frame(generation).unwrap();
        let tiles = split_frame(&bytes).unwrap();
        assert_eq!(tiles.len(), 6);
        assert_eq!(covered_area(&tiles), 600 * 300);
        assert!(tiles.iter().all(|(h, _)| h.generation == generation));
    }

    #[test]
    fn keeps_only_recent_frames() {
        let e = Engine::new();
        let first = render(&e, 8, 8);
        for _ in 0..KEPT_FRAMES {
            render(&e, 8, 8);
        }
        assert!(e.frame(first).is_none());
    }

    #[test]
    fn rejects_empty_size() {
        let e = Engine::new();
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
}
