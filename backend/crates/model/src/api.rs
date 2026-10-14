//! Commands, queries and events that cross the contract.
//!
//! Commands change state, queries read it, events push changes back.
//! Pixels never travel through these types; see [`crate::frame`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::frame::PixelFormat;

/// What kind of GPU adapter the render thread got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AdapterKind {
    /// A real GPU.
    Hardware,
    /// A software rasterizer: WARP on Windows, lavapipe or llvmpipe on Linux.
    Software,
    /// No adapter yet. The GPU pipeline is not wired in M0.
    None,
}

/// The adapter the engine renders with. Drives the status bar's
/// software-fallback notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub kind: AdapterKind,
    /// Driver-reported adapter name.
    pub name: String,
    /// Graphics API, for example "Vulkan", "Metal" or "Dx12".
    pub backend: String,
    /// Why the engine fell back to a software adapter, if it did.
    pub fallback_reason: Option<String>,
}

/// Sent by the frontend before anything else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Handshake {
    /// The contract version the frontend was built against.
    pub contract_version: String,
}

/// The engine's answer to a [`Handshake`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResult {
    pub compatible: bool,
    pub engine_contract_version: String,
    pub engine_version: String,
}

/// Static facts about the running engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub engine_version: String,
    pub contract_version: String,
    pub adapter: AdapterInfo,
}

/// Requests that change engine state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    /// M0 spike: render a synthetic test pattern and stream it as tiles.
    /// Used to measure the pixels-through-the-webview path (0.E).
    RenderTestPattern {
        width: u32,
        height: u32,
        /// Changes the pattern, standing in for a slider value.
        phase: f32,
        format: PixelFormat,
    },
}

/// Requests that only read engine state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Query {
    GetEngineInfo,
}

/// The answer to a [`Query`], one variant per query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum QueryResult {
    EngineInfo { info: EngineInfo },
}

/// Pushed from the engine to the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// The adapter changed, for example after a failed self-test.
    AdapterInfo { adapter: AdapterInfo },
    /// Every tile of a render generation is available.
    RenderFinished {
        generation: u64,
        tile_count: u32,
        /// Engine-side render time, for the latency harness.
        render_ms: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_json_shape() {
        let c = Command::RenderTestPattern {
            width: 2560,
            height: 1440,
            phase: 0.5,
            format: PixelFormat::Rgba8Srgb,
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "renderTestPattern",
                "width": 2560,
                "height": 1440,
                "phase": 0.5,
                "format": "rgba8Srgb"
            })
        );
    }

    #[test]
    fn query_json_shape() {
        let json = serde_json::to_value(Query::GetEngineInfo).unwrap();
        assert_eq!(json, serde_json::json!({ "type": "getEngineInfo" }));
    }
}
