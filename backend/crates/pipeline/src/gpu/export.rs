//! An export's renders on the GPU (OUT-01, *How it renders*).
//!
//! - **What an export holds** ([`ExportInputs`], taken by
//!   [`super::develop::Develop::export_inputs`] when Export is pressed):
//!   the photo's decoded input, shared; the camera transform of its settled
//!   white; the frame; the look and base curve; a copy of the base layer;
//!   and whether stage 9 compresses and dithers. Nothing of the open
//!   photo's pyramid, level-0 cache or view, so a switch or a drag does not
//!   touch it.
//! - **Full size** ([`ExportStages::render`]): one region of the shown
//!   image at 1:1 at a time, stages 1 to 9, through a level-0 cache and
//!   view stages of the export's own. Stages 1 to 4 run through the render
//!   thread's [`RawStages`], whose per-call uploads make each region its
//!   own. A region's pixels are the 1:1 view's at the same place: the same
//!   tiles, sampled at the same anchored positions, with the same dither.
//! - **Memory** ([`ExportStages::bytes`], `budget::EXPORT`): an atlas of
//!   [`SLOTS`] level-0 tiles, enough for a region under any straighten, and
//!   view caches of one region.

use std::sync::Arc;

use genzo_color::output::Target;
use genzo_model::frame::Rect;

use super::base::BaseLayer;
use super::develop::DevelopError;
use super::level0::{Level0Cache, Level0Error};
use super::raw_stages::RawStages;
use super::view::ViewStages;
use super::{Gpu, GpuError};
use crate::camera_color::CameraTransform;
use crate::histogram::Overlay;
use crate::look::{BaseCurve, Look, Profile};
use crate::raw::RawSource;
use crate::reference::sampling::{Frame, ViewSpec};

/// An export region's side, in output pixels.
pub const REGION: u32 = 1024;

/// The export's level-0 atlas, in slots of 256 px. A 1024 px region under
/// a 45° straighten reads a source square of about 1448 px plus the
/// bilinear neighbors: 7 × 7 tiles at most, wherever it lies, so 8 × 8.
pub const SLOTS: (u32, u32) = (8, 8);

/// The key of the export's own level-0 cache: it holds one photo's tiles
/// for one transform, for its whole life.
const KEY: u64 = 1;

/// What an export renders, taken when Export is pressed.
#[derive(Debug)]
pub struct ExportInputs {
    pub(crate) src: Arc<RawSource>,
    pub(crate) transform: CameraTransform,
    pub(crate) frame: Frame,
    pub(crate) look: Look,
    pub(crate) curve: Option<BaseCurve>,
    pub(crate) layer: Option<BaseLayer>,
    pub(crate) compress: bool,
    pub(crate) dither: bool,
    pub(crate) target: Target,
}

impl ExportInputs {
    /// The shown image's size, at full resolution: the full-size export's.
    pub fn size(&self) -> (u32, u32) {
        self.frame.pixel_size(self.src.size())
    }

    /// The GPU memory it holds: the base layer's copy.
    pub fn bytes(&self) -> u64 {
        self.layer.as_ref().map_or(0, BaseLayer::bytes)
    }

    /// The input, shared with the photo it came from.
    pub fn source(&self) -> &Arc<RawSource> {
        &self.src
    }
}

/// An export's own level-0 cache and view stages.
#[derive(Debug)]
pub struct ExportStages {
    cache: Level0Cache,
    view: ViewStages,
}

impl ExportStages {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            cache: Level0Cache::with_slots(gpu, SLOTS),
            view: ViewStages::new(gpu),
        }
    }

    /// The GPU memory held: the atlas, and the view's caches and frame at
    /// the last region's size.
    pub fn bytes(&self) -> u64 {
        self.cache.bytes() + self.view.cache_bytes() + self.view.output_bytes()
    }

    /// Renders `rect` of the shown image at full resolution, stages 1 to 9,
    /// and reads it back: RGBA8, row by row, `rect`'s width by its height.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        stages: &mut RawStages,
        inputs: &ExportInputs,
        rect: Rect,
    ) -> Result<Vec<u8>, DevelopError> {
        let view = ViewSpec::Region {
            out: (rect.width, rect.height),
            zoom: 1.0,
            origin: (f64::from(rect.x), f64::from(rect.y)),
        };
        let look = &inputs.look;
        self.view.set_ratio(None);
        self.view
            .show_level0(
                gpu,
                stages,
                &mut self.cache,
                &inputs.src,
                &inputs.transform,
                KEY,
                inputs.frame,
                view,
            )
            .map_err(|e| match e {
                Level0Error::Gpu(e) => DevelopError::from(e),
                full @ Level0Error::Full { .. } => {
                    DevelopError::from(GpuError::Request(full.to_string()))
                }
            })?;
        if !look.tone_is_identity() {
            let curve = (look.profile == Profile::GenzoStandard)
                .then_some(inputs.curve.as_ref())
                .flatten();
            let layer = look.is_local().then_some(inputs.layer.as_ref()).flatten();
            self.view.tone(gpu, look, curve, layer);
        }
        if !look.color_is_identity() {
            self.view.color(gpu, look);
        }
        self.view.set_counting(false);
        self.view.set_overlay(Overlay::default());
        self.view
            .output(gpu, inputs.target, inputs.compress, inputs.dither);
        let pixels = self.view.read_frame(gpu)?;
        Ok(pixels.into_flattened())
    }
}
