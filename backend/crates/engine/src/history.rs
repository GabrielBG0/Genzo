//! Each photo's edit and its history (APP-10, M1 part; DEV-01, *Data*): the
//! engine owns them, in memory for the session.
//!
//! - A delta between a gesture's *start* and *end* changes the edit the
//!   view shows, not the history. Its *end* records one step, labelled with
//!   the control it changed. A delta without a gesture is a start and an
//!   end in one.
//! - A change that leaves the edit as it was records nothing.
//! - Undo and redo do nothing during a gesture. A new step drops the steps
//!   that could have been redone. The depth is unlimited (owner,
//!   2026-10-24).
//! - Every value is clamped (`Edit::clamped`), and the crop is fitted to
//!   the image; a crop that cannot fit is refused and the edit keeps its
//!   last valid state (DEV-01, *Crop*).

use genzo_model::edit::{Change, Edit, Param, Phase, PhotoKind, WhiteBalance, WhiteBalanceMode};
use genzo_pipeline::geometry::{Crop, Geometry, fit};

/// One finished change: the whole edit after it, and what it changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub label: String,
    pub edit: Edit,
}

/// Why a delta was refused. The edit keeps its last valid state.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refused {
    #[error("the crop does not fit the image about its center")]
    CropDoesNotFit,
}

/// One photo's edit and history.
#[derive(Debug, Clone)]
pub struct PhotoEdits {
    kind: PhotoKind,
    defaults: Edit,
    /// The edit before any step: the defaults, or an edit loaded for the
    /// photo.
    base: Edit,
    /// The photo's active-area size and its file's orientation, for
    /// fitting the crop.
    size: (u32, u32),
    file_orientation: u8,
    steps: Vec<Step>,
    /// How many steps are applied.
    position: usize,
    /// The edit during a gesture, and its label.
    gesture: Option<Step>,
}

impl PhotoEdits {
    pub fn new(kind: PhotoKind, size: (u32, u32), file_orientation: u8) -> Self {
        Self::with_edit(kind, size, file_orientation, Edit::defaults(kind))
    }

    /// A photo whose edit starts as `edit` (one loaded for it, clamped)
    /// instead of its defaults, with no steps.
    pub fn with_edit(kind: PhotoKind, size: (u32, u32), file_orientation: u8, edit: Edit) -> Self {
        Self {
            kind,
            defaults: Edit::defaults(kind),
            base: edit.clamped(kind),
            size,
            file_orientation,
            steps: Vec::new(),
            position: 0,
            gesture: None,
        }
    }

    /// The edit as recorded: the last applied step's, or the base.
    pub fn committed(&self) -> &Edit {
        match self.position {
            0 => &self.base,
            n => &self.steps[n - 1].edit,
        }
    }

    /// The edit to show: the gesture's while one is in progress.
    pub fn current(&self) -> &Edit {
        self.gesture.as_ref().map_or(self.committed(), |g| &g.edit)
    }

    pub fn defaults(&self) -> &Edit {
        &self.defaults
    }

    /// Whether the edit differs from the photo's defaults: the edited dots.
    pub fn is_edited(&self) -> bool {
        self.current().is_edited(&self.defaults)
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// The label of the step Undo would undo, if any.
    pub fn undo_label(&self) -> Option<&str> {
        if self.in_gesture() || self.position == 0 {
            return None;
        }
        Some(&self.steps[self.position - 1].label)
    }

    /// The label of the step Redo would apply, if any.
    pub fn redo_label(&self) -> Option<&str> {
        if self.in_gesture() {
            return None;
        }
        self.steps.get(self.position).map(|s| s.label.as_str())
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.position]
    }

    /// Applies `changes` in `phase`. Returns whether a step was recorded.
    pub fn apply(&mut self, changes: &[Change], phase: Phase) -> Result<bool, Refused> {
        let base = self.current().clone();
        let mut edit = base.clone();
        for c in changes {
            edit = self.change(edit, c)?;
        }
        let label = match changes {
            [one] => one.label(),
            [first, ..] if changes.iter().all(|c| c.param() == first.param()) => first.label(),
            _ => "Edit".into(),
        };
        let label = self.gesture.as_ref().map_or(label, |g| g.label.clone());
        if phase != Phase::End {
            self.gesture = Some(Step { label, edit });
            return Ok(false);
        }
        self.gesture = None;
        if edit == *self.committed() {
            return Ok(false);
        }
        self.steps.truncate(self.position);
        self.steps.push(Step { label, edit });
        self.position += 1;
        Ok(true)
    }

    /// Undoes the last step. False when there is none, or during a gesture.
    pub fn undo(&mut self) -> bool {
        if self.undo_label().is_none() {
            return false;
        }
        self.position -= 1;
        true
    }

    /// Applies the step undone last again. False when there is none, or
    /// during a gesture.
    pub fn redo(&mut self) -> bool {
        if self.redo_label().is_none() {
            return false;
        }
        self.position += 1;
        true
    }

    fn change(&self, mut e: Edit, c: &Change) -> Result<Edit, Refused> {
        let d = &self.defaults;
        match c {
            Change::Profile(p) => e.profile = *p,
            Change::Treatment(t) => e.treatment = *t,
            Change::WhiteBalance { mode, temp, tint } => {
                e.white_balance = WhiteBalance {
                    mode: *mode,
                    temp: *temp,
                    tint: *tint,
                    unknown: e.white_balance.unknown,
                };
            }
            Change::Exposure(v) => e.exposure = *v,
            Change::Contrast(v) => e.contrast = *v,
            Change::Highlights(v) => e.highlights = *v,
            Change::Shadows(v) => e.shadows = *v,
            Change::Whites(v) => e.whites = *v,
            Change::Blacks(v) => e.blacks = *v,
            Change::Vibrance(v) => e.vibrance = *v,
            Change::Saturation(v) => e.saturation = *v,
            Change::Crop {
                cx,
                cy,
                w,
                h,
                angle,
            } => {
                let crop = Crop {
                    cx: f64::from(*cx),
                    cy: f64::from(*cy),
                    w: f64::from(*w),
                    h: f64::from(*h),
                    angle: f64::from(*angle),
                };
                let g = Geometry::from(&e);
                let fitted = fit(crop, self.size, g.orientation(self.file_orientation))
                    .ok_or(Refused::CropDoesNotFit)?;
                set_crop(&mut e, fitted);
            }
            Change::Turn { clockwise } => {
                let mut g = Geometry::from(&e);
                // Counterclockwise is three clockwise turns.
                for _ in 0..if *clockwise { 1 } else { 3 } {
                    g = g.turned();
                }
                e.quarter_turns = g.quarter_turns;
                set_crop(&mut e, g.crop);
            }
            Change::Flip => {
                let g = Geometry::from(&e).flipped();
                e.flip_horizontal = g.flip_horizontal;
                set_crop(&mut e, g.crop);
            }
            Change::Reset(p) => match p {
                Param::Profile => e.profile = d.profile,
                Param::Treatment => e.treatment = d.treatment,
                Param::WhiteBalance => {
                    e.white_balance.mode = WhiteBalanceMode::AsShot;
                }
                Param::Exposure => e.exposure = d.exposure,
                Param::Contrast => e.contrast = d.contrast,
                Param::Highlights => e.highlights = d.highlights,
                Param::Shadows => e.shadows = d.shadows,
                Param::Whites => e.whites = d.whites,
                Param::Blacks => e.blacks = d.blacks,
                Param::Vibrance => e.vibrance = d.vibrance,
                Param::Saturation => e.saturation = d.saturation,
                Param::Crop => e.crop = keep_unknown(d.crop.clone(), e.crop),
                Param::Orientation => {
                    e.quarter_turns = d.quarter_turns;
                    e.flip_horizontal = d.flip_horizontal;
                    e.crop = keep_unknown(d.crop.clone(), e.crop);
                }
            },
            Change::ResetAll => {
                // Fields this build does not know stay as they were, at
                // every level.
                let mut reset = d.clone();
                reset.unknown = e.unknown;
                reset.white_balance.unknown = e.white_balance.unknown;
                reset.crop = keep_unknown(reset.crop, e.crop);
                e = reset;
            }
        }
        Ok(e.clamped(self.kind))
    }
}

/// `fresh` with the unknown fields of `old`.
fn keep_unknown(
    mut fresh: genzo_model::edit::Crop,
    old: genzo_model::edit::Crop,
) -> genzo_model::edit::Crop {
    fresh.unknown = old.unknown;
    fresh
}

fn set_crop(e: &mut Edit, c: Crop) {
    e.crop.cx = c.cx as f32;
    e.crop.cy = c.cy as f32;
    e.crop.w = c.w as f32;
    e.crop.h = c.h as f32;
    e.crop.angle = c.angle as f32;
}
