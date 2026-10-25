//! One photo in Develop on the GPU (DEV-01, *Re-runs, quality levels and
//! generations*): the pyramid, the level-0 tiles, the base layer and the
//! view's caches of one photo, and [`Develop::render`], which takes the
//! whole edit and the view and re-runs from the earliest stage whose inputs
//! changed.
//!
//! - **Interactive** (a gesture's *start* and *update*): white balance is
//!   the drag ratio on the pyramid's values; the base layer keeps its last
//!   white and crop; at Fit the frame is the half-size preview.
//! - **Settled** (*end*, or a delta without a gesture): a new white
//!   rebuilds the pyramid in place, the tiles the view and the base layer
//!   read first ([`RawStages::rebuild_first`]); the base layer follows the
//!   white and the geometry; the frame is whole.
//! - **The rest of a rebuild** runs in [`Develop::continue_rebuild`],
//!   between requests, and stops between tiles when asked. Until it
//!   finishes the pyramid holds both whites, so a render that would read
//!   past the first part (another view or geometry) finishes it first.
//!
//! - **Opened from a prefetched level 1** ([`Develop::open_level1`]), a
//!   photo has no input until [`Develop::attach`] gives it one. Meanwhile a
//!   view that wants level 0 upsamples level 1 and is provisional, and a
//!   render that needs the input (a settled new white, the pyramid made
//!   again) fails with [`DevelopError::NeedsSource`].
//!
//! [`Develop::reran`] says what the last render ran, so tests can hold it
//! to DEV-01's re-run table. Only the render thread holds it.

use genzo_color::encoding::InputEncoding;
use genzo_color::output::Target;
use genzo_imgio::raw::CameraColor;
use genzo_model::edit::{Edit, PhotoKind, WhiteBalanceMode};
use genzo_model::frame::Rect;

use super::base::BaseLayer;
use super::level0::Level0Cache;
use super::raw_stages::{Level1, Level1Build, RawStages, Rebuild, TileConfig, level0_reach};
use super::readback::{ViewTile, visible_tiles};
use super::view::{ViewStages, compresses, dithers};
use super::{Gpu, GpuError};
use crate::camera_color::{
    CameraTransform, balanced_encoded_transform, camera_transform, drag_ratio, encoded_transform,
};
use crate::geometry::{Geometry, fit};
use crate::look::{BaseCurve, Look, Profile};
use crate::raw::RawSource;
use crate::reference::base::working_size;
use crate::reference::sampling::{Frame, ViewSpec, plan_view, plan_view_without_level0};

/// The gesture phase is the contract's (`model::edit::Phase`).
pub use genzo_model::edit::Phase;

#[derive(Debug, thiserror::Error)]
pub enum DevelopError {
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// The edit's crop fits nowhere about its center (DEV-01, *Crop*): the
    /// engine refuses such a delta before it gets here.
    #[error("the crop does not fit the image")]
    CropDoesNotFit,
    #[error("no photo is open")]
    NoPhoto,
    /// The render needs the photo's input, which is not attached yet: a
    /// settled new white, or the pyramid made again. The render thread
    /// holds it until the input comes.
    #[error("the photo is not decoded yet")]
    NeedsSource,
    /// An input attached to a photo of another size: the file changed
    /// since its level 1 was prefetched. The photo keeps waiting.
    #[error("the decoded photo is {got:?}, its prefetched level 1 {want:?}")]
    NotItsInput { got: (u32, u32), want: (u32, u32) },
}

/// A photo to develop: its decoded input and the file's orientation.
#[derive(Debug)]
pub struct Photo {
    pub src: RawSource,
    /// EXIF, 1 to 8.
    pub orientation: u8,
}

/// What Develop keeps of a photo besides its input's samples: enough to
/// show it from a prefetched level 1 before it is decoded (DEV-01,
/// *Prefetched neighbors*).
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoInfo {
    /// Level 0's size, [`RawSource::size`].
    pub size: (u32, u32),
    /// EXIF, 1 to 8.
    pub orientation: u8,
    color: CameraColor,
    encoding: Option<InputEncoding>,
}

impl PhotoInfo {
    pub fn of(photo: &Photo) -> Self {
        Self {
            size: photo.src.size(),
            orientation: photo.orientation,
            color: photo.src.color.clone(),
            encoding: photo.src.encoding.clone(),
        }
    }

    /// Whether `level1`, built for this photo, is what `edit`'s white
    /// balance renders: opened, it needs no rebuild. A cache of prefetched
    /// neighbors keeps an entry only while this holds.
    pub fn level1_fits(&self, level1: &Level1, edit: &Edit) -> bool {
        same_white(&self.white(edit), &level1.transform)
    }

    /// The transform stages 2 and 4 use for `edit`'s white balance: what
    /// [`Develop::render`] holds the pyramid to, so a level 1 built with it
    /// needs no rebuild once opened.
    pub fn white(&self, edit: &Edit) -> CameraTransform {
        let wb = &edit.white_balance;
        if wb.mode == WhiteBalanceMode::AsShot {
            return as_shot_of(&self.color, self.encoding.as_ref()).0;
        }
        let (temp, tint) = (f64::from(wb.temp), f64::from(wb.tint));
        match &self.encoding {
            Some(e) => balanced_encoded_transform(e, temp, tint),
            None => camera_transform(
                &self.color,
                Some(genzo_color::camera::xy_from_temperature(temp, tint)),
            ),
        }
    }
}

/// What the last [`Develop::render`] ran, earliest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reran {
    /// The pyramid: built for a new photo, or the first part of a white
    /// balance rebuild.
    pub pyramid: bool,
    /// The rest of a pending rebuild, finished before this render read past
    /// its first part.
    pub finished_rebuild: bool,
    pub base_layer: bool,
    /// The sampling of the pyramid or level 0 into the view (stage 4
    /// sampled).
    pub sample: bool,
    pub tone: bool,
    pub color: bool,
    pub output: bool,
}

/// The earliest view stage a change re-runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum From {
    Sample,
    Tone,
    Color,
    Output,
}

/// What the view's caches and frame hold.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    geometry: Geometry,
    frame: Frame,
    view: ViewSpec,
    ratio: Option<[[f32; 3]; 3]>,
    look: Look,
    target: Target,
    half: bool,
    dither: bool,
    /// Drawn from level 1 where level 0 was wanted, the input not attached.
    provisional: bool,
}

/// One photo's GPU state.
#[derive(Debug)]
pub struct Develop {
    stages: RawStages,
    cache: Level0Cache,
    view: ViewStages,
    layer: BaseLayer,
    photo: Option<Loaded>,
    /// The level-0 cache's key: every pyramid or white gets a new one.
    key: u64,
    shown: Option<Shown>,
    /// What the base layer was built from: the white and the geometry.
    layer_for: Option<([f64; 2], [f32; 3], Geometry)>,
    rebuild: Option<Rebuild>,
    reran: Reran,
}

#[derive(Debug)]
struct Loaded {
    info: PhotoInfo,
    /// None while a photo opened from its level 1 waits for its input.
    src: Option<RawSource>,
    kind: PhotoKind,
    as_shot: CameraTransform,
    curve: Option<BaseCurve>,
}

impl Loaded {
    fn new(info: PhotoInfo, src: Option<RawSource>) -> Self {
        let (as_shot, kind) = as_shot_of(&info.color, info.encoding.as_ref());
        let curve = info.encoding.is_none().then(|| BaseCurve::new(&info.color));
        Self {
            info,
            src,
            kind,
            as_shot,
            curve,
        }
    }
}

impl Develop {
    pub fn new(gpu: &Gpu, config: TileConfig) -> Self {
        Self {
            stages: RawStages::new(gpu, config),
            cache: Level0Cache::new(gpu),
            view: ViewStages::new(gpu),
            layer: BaseLayer::new(gpu),
            photo: None,
            key: 0,
            shown: None,
            layer_for: None,
            rebuild: None,
            reran: Reran::default(),
        }
    }

    /// Opens `photo` in place of the one held: its pyramid with the as-shot
    /// white. Its defaults are `Edit::defaults(develop.kind())`.
    pub fn open(&mut self, gpu: &Gpu, photo: Photo) -> Result<(), DevelopError> {
        self.close();
        let info = PhotoInfo::of(&photo);
        let loaded = self.photo.insert(Loaded::new(info, Some(photo.src)));
        // Failed, the photo stays open and the next render tries again.
        let src = loaded.src.as_ref().expect("given");
        self.stages.render_pyramid(gpu, src, &loaded.as_shot)?;
        Ok(())
    }

    /// Opens a photo from its prefetched `level1` in place of the one held,
    /// without its input (DEV-01, *Prefetched neighbors*): the pyramid is
    /// uploaded, with level 1's white. [`Develop::attach`] gives it its
    /// input later.
    pub fn open_level1(
        &mut self,
        gpu: &Gpu,
        info: PhotoInfo,
        level1: &Level1,
    ) -> Result<(), DevelopError> {
        // A pending rebuild goes before the upload makes it stale.
        self.close();
        self.photo = Some(Loaded::new(info, None));
        // Failed, the photo stays open and makes its pyramid once its
        // input comes.
        self.stages.upload_pyramid(gpu, level1)?;
        Ok(())
    }

    /// Gives the photo opened from its level 1 its decoded input. The next
    /// render draws a provisional frame again, from level 0.
    pub fn attach(&mut self, src: RawSource) -> Result<(), DevelopError> {
        let p = self.photo.as_mut().ok_or(DevelopError::NoPhoto)?;
        if src.size() != p.info.size {
            return Err(DevelopError::NotItsInput {
                got: src.size(),
                want: p.info.size,
            });
        }
        p.src = Some(src);
        Ok(())
    }

    /// Whether the open photo has its input.
    pub fn decoded(&self) -> bool {
        self.photo.as_ref().is_some_and(|p| p.src.is_some())
    }

    /// Whether the last frame was drawn from level 1 where level 0 was
    /// wanted, because the input was not attached: it renders again once
    /// it is.
    pub fn provisional(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.provisional)
    }

    /// Starts a prefetched neighbor's level 1 with `transform`, beside the
    /// open photo's pyramid, on the same tile stages
    /// ([`RawStages::start_level1`]).
    pub fn start_level1(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        transform: &CameraTransform,
    ) -> Level1Build {
        self.stages.start_level1(gpu, src, transform)
    }

    /// Runs `build`'s tiles, asking `stop` before each one
    /// ([`RawStages::continue_level1`]). The open photo's state is left as
    /// it was.
    pub fn continue_level1(
        &mut self,
        gpu: &Gpu,
        src: &RawSource,
        build: &mut Level1Build,
        stop: impl FnMut() -> bool,
    ) -> Result<bool, GpuError> {
        self.stages.continue_level1(gpu, src, build, stop)
    }

    fn close(&mut self) {
        self.photo = None;
        self.shown = None;
        self.layer_for = None;
        self.rebuild = None;
        self.layer.clear();
        self.key += 1;
    }

    /// The open photo's kind, for its defaults and ranges.
    pub fn kind(&self) -> Option<PhotoKind> {
        self.photo.as_ref().map(|p| p.kind)
    }

    /// The view's caches and frame, for reading the frame back.
    pub fn view(&self) -> &ViewStages {
        &self.view
    }

    /// What the last render ran.
    pub fn reran(&self) -> Reran {
        self.reran
    }

    /// Whether the rest of a white balance rebuild is waiting.
    pub fn rebuilding(&self) -> bool {
        self.rebuild.is_some()
    }

    /// Runs the rest of the pending rebuild, asking `stop` before each tile.
    /// Returns true when none is left.
    pub fn continue_rebuild(
        &mut self,
        gpu: &Gpu,
        stop: impl FnMut() -> bool,
    ) -> Result<bool, DevelopError> {
        let (Some(rebuild), Some(p)) = (self.rebuild.as_mut(), self.photo.as_ref()) else {
            return Ok(true);
        };
        let src = p.src.as_ref().expect("a rebuild has its input");
        let done = self.stages.rebuild_rest(gpu, src, rebuild, stop)?;
        if done {
            self.rebuild = None;
        }
        Ok(done)
    }

    /// The transform stages 2 and 4 use for `edit`'s white balance.
    fn transform(p: &Loaded, edit: &Edit) -> CameraTransform {
        if edit.white_balance.mode == WhiteBalanceMode::AsShot {
            return p.as_shot.clone();
        }
        p.info.white(edit)
    }

    /// Renders `edit` (clamped, `Edit::clamped`) in `view` for `target`,
    /// re-running from the earliest stage whose inputs changed since the
    /// last render. The frame is then in [`Develop::view`]: at Fit during a
    /// gesture, the half-size preview in its top left.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        edit: &Edit,
        view: ViewSpec,
        target: Target,
        phase: Phase,
    ) -> Result<Reran, DevelopError> {
        let p = self.photo.as_ref().ok_or(DevelopError::NoPhoto)?;
        let size = p.info.size;
        let file = p.info.orientation;
        let mut geometry = Geometry::from(edit);
        geometry.crop = fit(geometry.crop, size, geometry.orientation(file))
            .ok_or(DevelopError::CropDoesNotFit)?;
        let frame = Frame::of(&geometry, file);
        let wanted = Self::transform(p, edit);
        let look = Look::from(edit);
        let edited = edit.is_edited(&Edit::defaults(p.kind));
        let interactive = phase.interactive();
        let mut reran = Reran::default();
        if self.stages.pyramid().is_none() {
            // A failed render or rebuild dropped it (DEV-01, *States*: out of
            // GPU memory, retried once); the photo's input is still here,
            // or is waited for.
            let src = p.src.as_ref().ok_or(DevelopError::NeedsSource)?;
            self.shown = None;
            self.layer_for = None;
            self.rebuild = None;
            self.stages.render_pyramid(gpu, src, &wanted)?;
            self.key += 1;
            reran.pyramid = true;
        }
        let old = self.shown.clone();
        let moved = old
            .as_ref()
            .is_none_or(|s| s.geometry != geometry || s.view != view);

        // A pending rebuild's first part covers what the last view and base
        // layer read: anything that reads elsewhere waits for the rest.
        if let Some(rebuild) = self.rebuild.as_mut()
            && moved
        {
            let src = p.src.as_ref().expect("a rebuild has its input");
            self.stages.rebuild_rest(gpu, src, rebuild, || false)?;
            self.rebuild = None;
            reran.finished_rebuild = true;
        }

        let held = &self.stages.pyramid().expect("built above").transform;
        let new_white = !same_white(held, &wanted);
        let mut ratio = None;
        if new_white && interactive {
            ratio = drag_ratio(held, &wanted);
        } else if new_white {
            let src = p.src.as_ref().ok_or(DevelopError::NeedsSource)?;
            // The tiles the view and the new base layer read go first.
            let level0 = size;
            let levels = self.stages.pyramid().expect("built above").level_sizes();
            // The view's plan, and the pyramid's in case level 0 does not
            // fit the tile cache (`ViewStages::show` falls back to it).
            let s = plan_view(level0, &levels, frame, view);
            let fallback = plan_view_without_level0(level0, &levels, frame, view);
            let working = working_size(frame.pixel_size(level0));
            let layer = plan_view_without_level0(level0, &levels, frame, ViewSpec::Fit(working));
            let reach = [
                level0_reach(&fallback, view.out(), level0),
                level0_reach(&layer, working, level0),
            ]
            .into_iter()
            .fold(level0_reach(&s, view.out(), level0), union);
            // A newer white replaces a pending rebuild.
            self.rebuild = None;
            // On an error the pyramid goes, and the next render makes it again.
            let rebuild = self.stages.rebuild_first(gpu, src, &wanted, reach)?;
            self.rebuild = (rebuild.left() > 0).then_some(rebuild);
            self.key += 1;
            reran.pyramid = true;
        }

        let pyramid = self.stages.pyramid().expect("built above");
        let white_key = (pyramid.transform.white, pyramid.transform.multipliers);
        let layer_stale = self
            .layer_for
            .as_ref()
            .is_none_or(|(w, m, g)| (*w, *m) != white_key || *g != geometry);
        if layer_stale && (!interactive || self.layer_for.is_none()) {
            self.layer.build(gpu, pyramid, size, frame);
            self.layer_for = Some((white_key.0, white_key.1, geometry));
            reran.base_layer = true;
        }

        let half = interactive && matches!(view, ViewSpec::Fit(_));
        let dither = dithers(pyramid, edited);
        let provisional =
            p.src.is_none() && plan_view(size, &pyramid.level_sizes(), frame, view).level == 0;
        let shown = Shown {
            geometry,
            frame,
            view,
            ratio,
            look,
            target,
            half,
            dither,
            provisional,
        };
        let mut from = match &old {
            _ if reran.pyramid || reran.base_layer || reran.finished_rebuild => From::Sample,
            None => From::Sample,
            Some(s) if moved || s.ratio != ratio || s.provisional != provisional => From::Sample,
            Some(s) if tone_inputs(&s.look) != tone_inputs(&look) => From::Tone,
            Some(s) if color_inputs(&s.look) != color_inputs(&look) => From::Color,
            Some(s) if s.target != target || s.half != half || s.dither != dither => From::Output,
            Some(_) => {
                self.reran = reran;
                return Ok(reran);
            }
        };
        // A stage that turns into the identity is skipped, so the one before
        // it re-runs to become the latest cache (and stage 6's cache stops
        // counting as current only on a new sampling).
        if from == From::Color && look.color_is_identity() {
            from = From::Tone;
        }
        if from == From::Tone && look.tone_is_identity() {
            from = From::Sample;
        }

        if from <= From::Sample {
            self.view.set_ratio(ratio);
            self.view.show_with(
                gpu,
                &mut self.stages,
                &mut self.cache,
                p.src.as_ref(),
                size,
                self.key,
                frame,
                view,
            )?;
            reran.sample = true;
        }
        if from <= From::Tone && !look.tone_is_identity() {
            let curve = (look.profile == Profile::GenzoStandard)
                .then_some(p.curve.as_ref())
                .flatten();
            let layer = look.is_local().then_some(&self.layer);
            self.view.tone(gpu, &look, curve, layer);
            reran.tone = true;
        }
        if from <= From::Color && !look.color_is_identity() {
            self.view.color(gpu, &look);
            reran.color = true;
        }
        let pyramid = self.stages.pyramid().expect("built above");
        let compress = compresses(pyramid, target);
        if half {
            self.view.output_half(gpu, target, compress, dither);
        } else {
            self.view.output(gpu, target, compress, dither);
        }
        reran.output = true;
        self.shown = Some(shown);
        self.reran = reran;
        Ok(reran)
    }
}

/// A photo's as-shot transform and its kind, with the as-shot Temp and Tint
/// for a RAW file (DEV-01, *White balance*): what its defaults need
/// (`Edit::defaults`). The engine calls it off the render thread, after
/// decoding.
pub fn as_shot(src: &RawSource) -> (CameraTransform, PhotoKind) {
    as_shot_of(&src.color, src.encoding.as_ref())
}

fn as_shot_of(
    color: &CameraColor,
    encoding: Option<&InputEncoding>,
) -> (CameraTransform, PhotoKind) {
    match encoding {
        Some(e) => (encoded_transform(e), PhotoKind::NonRaw),
        None => {
            let t = camera_transform(color, None);
            let (temp, tint) = genzo_color::camera::temperature_from_xy(t.white);
            let kind = PhotoKind::Raw {
                as_shot_temp: temp as f32,
                as_shot_tint: tint as f32,
            };
            (t, kind)
        }
    }
}

/// What a render sends (DEV-01, *Readback*).
#[derive(Debug, Clone, PartialEq)]
pub enum Readback {
    /// The whole frame, RGBA8 row by row: a Fit view, or the half-size
    /// preview during a gesture (`half`).
    Frame {
        size: (u32, u32),
        half: bool,
        pixels: Vec<[u8; 4]>,
    },
    /// At 1:1, the visible 256 px tiles of the shown image, each RGBA8 row
    /// by row.
    Tiles(Vec<(ViewTile, Vec<u8>)>),
}

impl Develop {
    /// Reads back what the last render sends: at 1:1 only the visible
    /// tiles, otherwise the frame.
    pub fn read_back(&self, gpu: &Gpu) -> Result<Readback, GpuError> {
        let (p, s) = (
            self.photo.as_ref().expect("open a photo first"),
            self.shown.as_ref().expect("render first"),
        );
        if let ViewSpec::Region { zoom, .. } = s.view
            && zoom == 1.0
        {
            let shown = s.frame.pixel_size(p.info.size);
            let tiles = visible_tiles(self.view.offset(), s.view.out(), shown);
            let rects: Vec<Rect> = tiles.iter().map(|t| t.frame).collect();
            let bytes = self.view.read_rects(gpu, &rects)?;
            return Ok(Readback::Tiles(tiles.into_iter().zip(bytes).collect()));
        }
        Ok(Readback::Frame {
            size: self.view.frame_size(),
            half: s.half,
            pixels: self.view.read_frame(gpu)?,
        })
    }
}

/// Whether two transforms give the same stages 2 and 4.
fn same_white(a: &CameraTransform, b: &CameraTransform) -> bool {
    a.multipliers == b.multipliers && a.matrix == b.matrix
}

fn tone_inputs(l: &Look) -> [f32; 7] {
    let p = match l.profile {
        Profile::GenzoStandard => 0.0,
        Profile::GenzoLinear => 1.0,
        Profile::Embedded => 2.0,
    };
    [
        p,
        l.exposure,
        l.contrast,
        l.highlights,
        l.shadows,
        l.whites,
        l.blacks,
    ]
}

fn color_inputs(l: &Look) -> (bool, f32, f32) {
    (l.monochrome, l.vibrance, l.saturation)
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Some(Rect {
                x,
                y,
                width: (a.x + a.width).max(b.x + b.width) - x,
                height: (a.y + a.height).max(b.y + b.height) - y,
            })
        }
        (a, b) => a.or(b),
    }
}
