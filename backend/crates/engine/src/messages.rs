//! Contract 0.5.0's Develop and strip messages: the commands and queries
//! mapped onto the engine's methods, and its values into the contract's
//! types.
//!
//! - What the user can act on comes back as data: a refused change or pick
//!   as the edit with its reason, a photo that does not render as its
//!   selection with the reason. A failure of the engine (no GPU, a device
//!   error) is the command's error.
//! - `SetView` with no photo open keeps the view for the next: done.

use std::path::Path;

use genzo_model::api::{
    self, ClippingPreview, ColorSpace, Command, CommandResult, Event, PhotoEdit, Query,
    QueryResult, View,
};
use genzo_model::edit::{Change, Phase};

use crate::develop::{Applied, EditState, Target, ViewSpec};
use crate::histogram::{Channels, Counted, Histogram, Preview};
use crate::preview::Shown;
use crate::strip::{Direction, PhotoKind, Strip};
use crate::{Engine, EngineError, check_size};

impl Engine {
    /// Runs one of contract 0.5.0's commands.
    pub(crate) fn develop_command(&self, command: Command) -> Result<CommandResult, EngineError> {
        match command {
            Command::OpenFolder { path } => {
                let opened = self.open_folder(Path::new(&path))?;
                let (edit, reason) = match opened.first {
                    Some(first) => selected(first)?,
                    None => (None, None),
                };
                Ok(CommandResult::Folder {
                    strip: strip(opened.strip),
                    edit,
                    reason,
                })
            }
            Command::SelectPhoto { photo } => {
                let (edit, reason) = selected(self.select_photo(photo))?;
                Ok(CommandResult::Selected {
                    photo,
                    edit,
                    reason,
                })
            }
            Command::SetView {
                width,
                height,
                view,
                space,
            } => {
                check_size(width, height)?;
                let view = match view {
                    View::Fit => ViewSpec::Fit((width, height)),
                    View::Region { zoom, x, y } => {
                        if !(zoom.is_finite() && zoom > 0.0 && x.is_finite() && y.is_finite()) {
                            return Err(EngineError::Invalid(format!(
                                "a view at zoom {zoom} from ({x}, {y})"
                            )));
                        }
                        ViewSpec::Region {
                            out: (width, height),
                            zoom,
                            origin: (x, y),
                        }
                    }
                };
                let target = match space {
                    ColorSpace::Srgb => Target::Srgb,
                    ColorSpace::DisplayP3 => Target::DisplayP3,
                };
                match self.set_view(view, target) {
                    Ok(_) | Err(EngineError::NoPhoto) => Ok(CommandResult::Done),
                    Err(e) => Err(e),
                }
            }
            Command::ApplyEditDelta {
                photo,
                changes,
                phase,
            } => self.edited(photo, self.apply_to(photo, &changes, phase)),
            Command::ResetParam { photo, param } => {
                let change = param.map_or(Change::ResetAll, Change::Reset);
                self.edited(photo, self.apply_to(photo, &[change], Phase::End))
            }
            Command::PickWhiteBalance { photo, x, y } => {
                if !(x.is_finite() && y.is_finite()) {
                    return Err(EngineError::Invalid(format!("a pick at ({x}, {y})")));
                }
                let point = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
                self.edited(photo, self.pick_white_balance(photo, point))
            }
            Command::SetClippingOverlay {
                shadows,
                highlights,
                shadow_color,
                highlight_color,
            } => {
                self.set_clipping_overlay(
                    shadows.then_some(shadow_color),
                    highlights.then_some(highlight_color),
                )?;
                Ok(CommandResult::Done)
            }
            Command::SetClippingPreview { preview } => {
                self.set_clipping_preview(match preview {
                    ClippingPreview::None => Preview::None,
                    ClippingPreview::Highlights => Preview::Highlights,
                    ClippingPreview::Shadows => Preview::Shadows,
                })?;
                Ok(CommandResult::Done)
            }
            Command::Undo { photo } => self.edited(photo, self.undo_of(photo)),
            Command::Redo { photo } => self.edited(photo, self.redo_of(photo)),
            Command::RenderTestPattern { .. }
            | Command::LoadSpikeImage { .. }
            | Command::RenderSpike { .. }
            | Command::SaveAutobenchReport { .. } => {
                unreachable!("the spike's commands run in Engine::execute")
            }
        }
    }

    /// Answers one of contract 0.5.0's queries.
    pub(crate) fn develop_query(&self, query: Query) -> QueryResult {
        match query {
            Query::ListFolderPhotos => QueryResult::FolderPhotos {
                strip: strip(self.strip()),
            },
            Query::GetEdit { photo } => QueryResult::Edit {
                edit: self.edit_of(photo).map(photo_edit),
            },
            Query::GetHistogram { photo } => QueryResult::Histogram {
                counted: self.histogram(photo).map(counted),
            },
            Query::GetEngineInfo => QueryResult::EngineInfo { info: self.info() },
        }
    }

    /// The answer to a command that changes photo `photo`'s edit: a
    /// refusal is its edit as it is, with the reason.
    fn edited(
        &self,
        photo: u64,
        applied: Result<Applied, EngineError>,
    ) -> Result<CommandResult, EngineError> {
        match applied {
            Ok(a) => Ok(CommandResult::Edit {
                edit: photo_edit(a.state),
                recorded: a.recorded,
                refused: None,
            }),
            Err(EngineError::Refused(why)) => Ok(CommandResult::Edit {
                edit: self
                    .edit_of(photo)
                    .map(photo_edit)
                    .ok_or(EngineError::UnknownPhoto(photo))?,
                recorded: false,
                refused: Some(why),
            }),
            Err(e) => Err(e),
        }
    }
}

/// A switch's outcome as the reply gives it: the photo's edit, or why it
/// does not render.
fn selected(
    switched: Result<Applied, EngineError>,
) -> Result<(Option<PhotoEdit>, Option<String>), EngineError> {
    let kind = match switched {
        Ok(a) => return Ok((Some(photo_edit(a.state)), None)),
        Err(EngineError::Unsupported(why)) => PhotoKind::Unsupported(why),
        Err(EngineError::Damaged(_)) => PhotoKind::Damaged,
        Err(EngineError::Missing(_)) => PhotoKind::Missing,
        Err(e) => return Err(e),
    };
    Ok((None, kind.reason().map(String::from)))
}

pub(crate) fn photo_edit(s: EditState) -> PhotoEdit {
    PhotoEdit {
        photo: s.photo,
        edit: s.edit,
        edited: s.edited,
        undo: s.undo,
        redo: s.redo,
        revision: s.revision,
    }
}

pub(crate) fn strip(s: Strip) -> api::Strip {
    api::Strip {
        photos: s
            .photos
            .into_iter()
            .map(|p| api::StripPhoto {
                id: p.id,
                file_name: p.file_name,
                capture_time: p.capture_time.map(|t| api::CaptureTime {
                    year: t.year,
                    month: t.month,
                    day: t.day,
                    hour: t.hour,
                    minute: t.minute,
                    second: t.second,
                    nanos: t.nanos,
                }),
                shooting: {
                    let r = |r: Option<genzo_imgio::capture::Rational>| {
                        r.map(|r| api::Rational {
                            num: r.num,
                            den: r.den,
                        })
                    };
                    api::Shooting {
                        iso: p.shooting.iso,
                        exposure_time: r(p.shooting.exposure_time),
                        f_number: r(p.shooting.f_number),
                        focal_length: r(p.shooting.focal_length),
                    }
                },
                reason: p.kind.reason().map(String::from),
                kind: match p.kind {
                    PhotoKind::Unknown => api::PhotoKind::Unknown,
                    PhotoKind::Renders => api::PhotoKind::Renders,
                    PhotoKind::Unsupported(_) => api::PhotoKind::Unsupported,
                    PhotoKind::Damaged => api::PhotoKind::Damaged,
                    PhotoKind::Missing => api::PhotoKind::Missing,
                },
                edited: p.edited,
            })
            .collect(),
        current: s.current,
        direction: match s.direction {
            Direction::Next => api::Direction::Next,
            Direction::Previous => api::Direction::Previous,
        },
    }
}

pub(crate) fn histogram(h: &Histogram) -> api::Histogram {
    let clipping = |c: &crate::histogram::Clipping| {
        let has = |channel: u8| c.channels & (1 << channel) as Channels != 0;
        api::Clipping {
            pixels: c.pixels,
            red: has(0),
            green: has(1),
            blue: has(2),
        }
    };
    let [red, green, blue] = h.bins.map(|b| b.to_vec());
    api::Histogram {
        red,
        green,
        blue,
        highlights: clipping(&h.highlights),
        shadows: clipping(&h.shadows),
        pixels: h.pixels,
    }
}

pub(crate) fn counted(c: Counted) -> api::Counted {
    api::Counted {
        photo: c.photo,
        generation: c.generation,
        histogram: histogram(&c.histogram),
    }
}

/// The event for a count handed to the histogram sink.
pub(crate) fn histogram_ready(c: Counted) -> Event {
    Event::HistogramReady {
        photo: c.photo,
        generation: c.generation,
        histogram: histogram(&c.histogram),
    }
}

/// The event for a switch's first pixels. Its pixels stay in the frame
/// store.
pub(crate) fn preview_shown(s: &Shown) -> Event {
    Event::PreviewShown {
        photo: s.photo,
        generation: s.frame.map(|(g, _)| g),
        tile_count: s.frame.map(|(_, n)| n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::histogram::Clipping;

    #[test]
    fn the_counts_keep_their_channels() {
        let mut h = Histogram::default();
        h.bins[0][3] = 7;
        h.bins[2][250] = 9;
        // Bit 0 is red, 1 green, 2 blue: stage 9's order.
        h.highlights = Clipping {
            pixels: 5,
            channels: 0b001,
        };
        h.shadows = Clipping {
            pixels: 6,
            channels: 0b110,
        };
        h.pixels = 16;
        let c = histogram(&h);
        assert_eq!((c.red[3], c.green[3], c.blue[250]), (7, 0, 9));
        assert_eq!(c.red.len(), 256);
        let side = |c: api::Clipping| (c.pixels, c.red, c.green, c.blue);
        assert_eq!(side(c.highlights), (5, true, false, false));
        assert_eq!(side(c.shadows), (6, false, true, true));
        assert_eq!(c.pixels, 16);
    }
}
