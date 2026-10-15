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

/// Where the raw image of the 0.D GPU spike comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SpikeSource {
    /// A generated Bayer scene of this size, standing in for a raw file.
    Synthetic { width: u32, height: u32 },
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
    /// M0 spike 0.D: load the raw image the GPU pipeline renders.
    LoadSpikeImage { source: SpikeSource },
    /// M0 spike 0.D: render the loaded raw image through the GPU pipeline
    /// into a viewport that fits `width` × `height`, and stream it as tiles.
    RenderSpike {
        width: u32,
        height: u32,
        exposure_ev: f32,
        /// White balance gains for red, green and blue.
        wb: [f32; 3],
        format: PixelFormat,
        /// A slider is being dragged: white balance is applied after the
        /// cached demosaic. Send `false` once the drag ends for the full
        /// quality render.
        interactive: bool,
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
        /// GPU time of the stages that ran, when the adapter can measure it.
        gpu_ms: Option<f64>,
    },
    /// The spike's raw image is loaded; renders now show it.
    SpikeImageLoaded { width: u32, height: u32 },
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
    fn render_spike_json_shape() {
        let c = Command::RenderSpike {
            width: 2560,
            height: 1440,
            exposure_ev: 0.5,
            wb: [2.0, 1.0, 1.5],
            format: PixelFormat::Rgba8DisplayP3,
            interactive: true,
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({
                "type": "renderSpike",
                "width": 2560,
                "height": 1440,
                "exposureEv": 0.5,
                "wb": [2.0, 1.0, 1.5],
                "format": "rgba8DisplayP3",
                "interactive": true
            })
        );
        let load = Command::LoadSpikeImage {
            source: SpikeSource::Synthetic {
                width: 8064,
                height: 6048,
            },
        };
        assert_eq!(
            serde_json::to_value(&load).unwrap(),
            serde_json::json!({
                "type": "loadSpikeImage",
                "source": { "type": "synthetic", "width": 8064, "height": 6048 }
            })
        );
    }

    #[test]
    fn query_json_shape() {
        let json = serde_json::to_value(Query::GetEngineInfo).unwrap();
        assert_eq!(json, serde_json::json!({ "type": "getEngineInfo" }));
    }
}
