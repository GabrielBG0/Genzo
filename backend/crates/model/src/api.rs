//! Commands, queries and events that cross the contract.
//!
//! Commands change state and answer with a [`CommandResult`], queries read
//! it, events push changes back. Pixels never travel through these types;
//! see [`crate::frame`].
//!
//! Develop's messages (contract 0.5.0) name the photo they act on. One for
//! a photo no longer open changes nothing and answers with that photo's
//! edit as it is. Events come from several threads: a frame is drawn when
//! its generation is the newest seen, and an edit reply is taken when its
//! `revision` is the photo's newest.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::{Change, Edit, Param, Phase};
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
    /// The app was started to benchmark itself (M0 0.E on Linux and macOS,
    /// where no DevTools attach exists). The frontend runs a scripted drag and
    /// sends the result with [`Command::SaveAutobenchReport`].
    pub autobench: bool,
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

/// The color space a view's frames are encoded for: the display's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    Srgb,
    DisplayP3,
}

/// What the viewport shows (DEV-01, *Inputs*: `SetView`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum View {
    /// The whole photo, fitted into the viewport.
    Fit,
    /// The shown image at `zoom` (viewport pixels per image pixel; 1 is
    /// 1:1), its top left at (`x`, `y`) in shown image pixels, after
    /// geometry. With the viewport's size it is the visible rectangle. At
    /// 1:1 the engine snaps it to whole pixels.
    Region { zoom: f64, x: f64, y: f64 },
}

/// The Alt clipping preview during a tone slider's drag (DEV-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ClippingPreview {
    None,
    Highlights,
    Shadows,
}

/// Which way the strip's last move went: the prefetch's order (LIB-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Next,
    Previous,
}

/// Whether a strip photo renders (LIB-09, *Data*). Not known until it is
/// decoded, by a switch to it or by prefetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PhotoKind {
    Unknown,
    Renders,
    /// Recognized, not rendered: its embedded preview and the reason.
    Unsupported,
    Damaged,
    /// Gone since the folder was listed.
    Missing,
}

/// A capture time as EXIF states it, in the camera's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// From SubSecTimeOriginal, 0 when the file has none.
    pub nanos: u32,
}

/// A positive EXIF rational as the file stores it, unreduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Rational {
    pub num: u32,
    /// Never 0.
    pub den: u32,
}

/// The EXIF row's values (DEV-03). Each is absent when the file has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Shooting {
    pub iso: Option<u32>,
    /// In seconds.
    pub exposure_time: Option<Rational>,
    pub f_number: Option<Rational>,
    /// In millimeters, the lens's own.
    pub focal_length: Option<Rational>,
}

/// One photo of the strip (LIB-09, *Data*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StripPhoto {
    /// Stable for the session, by the file's canonical path.
    pub id: u64,
    pub file_name: String,
    pub capture_time: Option<CaptureTime>,
    pub shooting: Shooting,
    pub kind: PhotoKind,
    /// The status bar's reason for a photo that does not render.
    pub reason: Option<String>,
    /// Its edit differs from its defaults: the blue dot.
    pub edited: bool,
}

/// The open folder's photos, in strip order, and the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Strip {
    pub photos: Vec<StripPhoto>,
    pub current: Option<u64>,
    pub direction: Direction,
}

/// A photo's edit as the panels show it (APP-10, *The engine's side*).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PhotoEdit {
    pub photo: u64,
    pub edit: Edit,
    /// Differs from the photo's defaults: the edited dots.
    pub edited: bool,
    /// The label of the step Undo would undo, if any.
    pub undo: Option<String>,
    /// The label of the step Redo would apply, if any.
    pub redo: Option<String>,
    /// Counts the photo's changes this session. Replies can arrive out of
    /// order; the one with the highest revision is the photo's edit now.
    pub revision: u64,
}

/// One side's clipping (DEV-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Clipping {
    /// The pixels that clip on this side.
    pub pixels: u32,
    /// Whether each channel clips in at least one pixel.
    pub red: bool,
    pub green: bool,
    pub blue: bool,
}

/// The counts of one frame (DEV-03): numbers, not pixels, so JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Histogram {
    /// 256 bins each: bin `n` holds the pixels whose code is `n`.
    pub red: Vec<u32>,
    pub green: Vec<u32>,
    pub blue: Vec<u32>,
    pub highlights: Clipping,
    pub shadows: Clipping,
    /// The pixels counted.
    pub pixels: u32,
}

/// A photo's histogram and the generation it counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Counted {
    pub photo: u64,
    pub generation: u64,
    pub histogram: Histogram,
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
        /// Send a half-size JPEG preview instead of the full frame. Only
        /// honored together with `interactive`; the frontend shows it scaled
        /// up. It exists because moving raw pixels into the webview is the
        /// bottleneck of a drag (0.E).
        proxy: bool,
    },
    /// M0 spike 0.E: store the autobench report (JSON written by the
    /// frontend) at the path the engine was started with. Fails when the
    /// engine is not in autobench mode.
    SaveAutobenchReport { report: String },
    /// Lists a folder, makes it the strip and selects its first photo
    /// (LIB-09). Answers [`CommandResult::Folder`].
    OpenFolder { path: String },
    /// Opens a strip photo in Develop (LIB-09; DEV-01). Its embedded
    /// preview comes first, as `PreviewShown`, then its frame. Answers
    /// [`CommandResult::Selected`].
    SelectPhoto { photo: u64 },
    /// The viewport's size in device pixels, what it shows and its color
    /// space. Renders the open photo settled; with none open it is kept
    /// for the next. Answers [`CommandResult::Done`].
    SetView {
        width: u32,
        height: u32,
        view: View,
        space: ColorSpace,
    },
    /// Changes to the photo's edit in a gesture's `phase`. A delta without
    /// a gesture is an *end*; each *end* records one step (DEV-01,
    /// *Inputs*; APP-10). Answers [`CommandResult::Edit`].
    ApplyEditDelta {
        photo: u64,
        changes: Vec<Change>,
        phase: Phase,
    },
    /// One field back to the photo's default, or with no `param` the whole
    /// edit, as one step. Answers [`CommandResult::Edit`].
    ResetParam { photo: u64, param: Option<Param> },
    /// The white balance picker (DEV-02): makes the area around (`x`, `y`)
    /// on the shown image, normalized, neutral, as one step. Answers
    /// [`CommandResult::Edit`], with the reason when it refuses.
    PickWhiteBalance { photo: u64, x: f64, y: f64 },
    /// J's clipping overlays, each with its token color as RGBA8 (DEV-03).
    /// Kept across photo switches. Answers [`CommandResult::Done`].
    SetClippingOverlay {
        shadows: bool,
        highlights: bool,
        shadow_color: [u8; 4],
        highlight_color: [u8; 4],
    },
    /// The Alt preview of the gesture in progress (DEV-03); it ends with
    /// the gesture. Answers [`CommandResult::Done`].
    SetClippingPreview { preview: ClippingPreview },
    /// Undoes the photo's last step (APP-10). Answers
    /// [`CommandResult::Edit`].
    Undo { photo: u64 },
    /// Applies the photo's step undone last again. Answers
    /// [`CommandResult::Edit`].
    Redo { photo: u64 },
}

/// The answer to a [`Command`]. A failure the user cannot act on (no GPU,
/// a device error) is the command's error instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandResult {
    /// Done; what it produced comes as events.
    Done,
    /// The photo's edit after the command, and whether it recorded a step.
    /// `refused` is the status bar's reason when the engine refused the
    /// change; the edit keeps its last valid state.
    Edit {
        edit: PhotoEdit,
        recorded: bool,
        refused: Option<String>,
    },
    /// The selected photo. When it does not render, `edit` is `None` and
    /// `reason` says why; it shows its embedded preview, if it has one.
    Selected {
        photo: u64,
        edit: Option<PhotoEdit>,
        reason: Option<String>,
    },
    /// The new strip, and its first photo as [`CommandResult::Selected`]
    /// gives it; neither for an empty folder.
    Folder {
        strip: Strip,
        edit: Option<PhotoEdit>,
        reason: Option<String>,
    },
}

/// Requests that only read engine state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Query {
    GetEngineInfo,
    /// The strip (LIB-09).
    ListFolderPhotos,
    /// A photo's edit, as the commands that change it answer it.
    GetEdit {
        photo: u64,
    },
    /// The latest histogram of the shown photo (DEV-03).
    GetHistogram {
        photo: u64,
    },
}

/// The answer to a [`Query`], one variant per query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum QueryResult {
    EngineInfo {
        info: EngineInfo,
    },
    FolderPhotos {
        strip: Strip,
    },
    /// `None` for a photo not opened this session.
    Edit {
        edit: Option<PhotoEdit>,
    },
    /// `None` unless the photo is the one shown and has been counted.
    Histogram {
        counted: Option<Counted>,
    },
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
    /// Every tile of a render generation is available. Develop's frames
    /// name their photo. Draw a frame only when its generation is the
    /// newest seen.
    RenderFinished {
        photo: Option<u64>,
        generation: u64,
        tile_count: u32,
        /// Engine-side render time, for the latency harness.
        render_ms: f64,
        /// GPU time of the stages that ran, when the adapter can measure it.
        gpu_ms: Option<f64>,
    },
    /// The spike's raw image is loaded; renders now show it.
    SpikeImageLoaded { width: u32, height: u32 },
    /// A selected photo's embedded preview, before it decodes (DEV-01,
    /// *Opening*): its tiles, lossless, under `generation`. Both are
    /// `None` when it has none: the neutral fill and the file name.
    PreviewShown {
        photo: u64,
        generation: Option<u64>,
        tile_count: Option<u32>,
    },
    /// The shown photo's counts changed (DEV-03). At 1:1 it follows the
    /// visible tiles' `RenderFinished`.
    HistogramReady {
        photo: u64,
        generation: u64,
        histogram: Histogram,
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
    fn render_spike_json_shape() {
        let c = Command::RenderSpike {
            width: 2560,
            height: 1440,
            exposure_ev: 0.5,
            wb: [2.0, 1.0, 1.5],
            format: PixelFormat::Rgba8DisplayP3,
            interactive: true,
            proxy: true,
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
                "interactive": true,
                "proxy": true
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
        let json = serde_json::to_value(Query::GetHistogram { photo: 3 }).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "getHistogram", "photo": 3 })
        );
    }

    /// The edit and its changes keep the names they are stored under.
    #[test]
    fn develop_commands_json_shape() {
        use crate::edit::{Change, Param, Phase};
        let delta = Command::ApplyEditDelta {
            photo: 7,
            changes: vec![
                Change::Exposure(0.5),
                Change::Turn { clockwise: true },
                Change::Flip,
                Change::Reset(Param::WhiteBalance),
            ],
            phase: Phase::Start,
        };
        let json = serde_json::json!({
            "type": "applyEditDelta",
            "photo": 7,
            "changes": [
                { "Exposure": 0.5 },
                { "Turn": { "clockwise": true } },
                "Flip",
                { "Reset": "WhiteBalance" }
            ],
            "phase": "Start"
        });
        assert_eq!(serde_json::to_value(&delta).unwrap(), json);
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), delta);
        let view = Command::SetView {
            width: 1280,
            height: 960,
            view: View::Region {
                zoom: 1.0,
                x: 10.0,
                y: 20.0,
            },
            space: ColorSpace::DisplayP3,
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "type": "setView",
                "width": 1280,
                "height": 960,
                "view": { "type": "region", "zoom": 1.0, "x": 10.0, "y": 20.0 },
                "space": "displayP3"
            })
        );
        let overlay = Command::SetClippingOverlay {
            shadows: true,
            highlights: false,
            shadow_color: [0, 0, 255, 255],
            highlight_color: [255, 0, 0, 255],
        };
        assert_eq!(
            serde_json::to_value(&overlay).unwrap(),
            serde_json::json!({
                "type": "setClippingOverlay",
                "shadows": true,
                "highlights": false,
                "shadowColor": [0, 0, 255, 255],
                "highlightColor": [255, 0, 0, 255]
            })
        );
        let reset_all = serde_json::json!({ "type": "resetParam", "photo": 7, "param": null });
        assert_eq!(
            serde_json::from_value::<Command>(reset_all).unwrap(),
            Command::ResetParam {
                photo: 7,
                param: None
            }
        );
    }

    #[test]
    fn replies_and_events_json_shape() {
        use crate::edit::{Edit, PhotoKind as EditKind};
        let reply = CommandResult::Edit {
            edit: PhotoEdit {
                photo: 7,
                edit: Edit::defaults(EditKind::NonRaw),
                edited: false,
                undo: Some("Exposure".into()),
                redo: None,
                revision: 4,
            },
            recorded: true,
            refused: None,
        };
        let json = serde_json::to_value(&reply).unwrap();
        assert_eq!(json["type"], "edit");
        assert_eq!(json["edit"]["revision"], 4);
        assert_eq!(json["edit"]["undo"], "Exposure");
        // The edit as stored: snake_case fields, its unknown ones left out.
        assert_eq!(json["edit"]["edit"]["schema_version"], 1);
        assert_eq!(json["edit"]["edit"]["white_balance"]["mode"], "AsShot");
        assert_eq!(json["edit"]["edit"]["profile"], "Embedded");
        assert_eq!(json["refused"], serde_json::Value::Null);

        let side = Clipping {
            pixels: 2,
            red: true,
            green: false,
            blue: false,
        };
        let ready = Event::HistogramReady {
            photo: 7,
            generation: 9,
            histogram: Histogram {
                red: vec![0; 256],
                green: vec![0; 256],
                blue: vec![0; 256],
                highlights: side,
                shadows: side,
                pixels: 12,
            },
        };
        let json = serde_json::to_value(&ready).unwrap();
        assert_eq!(json["type"], "histogramReady");
        assert_eq!(json["histogram"]["red"].as_array().unwrap().len(), 256);
        assert_eq!(
            json["histogram"]["highlights"],
            serde_json::json!({ "pixels": 2, "red": true, "green": false, "blue": false })
        );
        assert_eq!(
            serde_json::to_value(Event::PreviewShown {
                photo: 7,
                generation: None,
                tile_count: None
            })
            .unwrap(),
            serde_json::json!({
                "type": "previewShown", "photo": 7, "generation": null, "tileCount": null
            })
        );
    }
}
