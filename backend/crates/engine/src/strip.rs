//! The opened folder's session (LIB-09, M1 part): the strip's photos, the
//! current one, and switching between them in Develop.
//!
//! - A photo's id is stable for the session: the same file, by its
//!   canonical path, keeps its id across folder switches, and so its edit
//!   and history.
//! - Opening a folder replaces the strip and selects its first photo.
//! - A switch ends a gesture in progress first, which records its step,
//!   and resets the view to Fit at the viewport's size.
//! - The last move's direction is kept for the prefetch order. On opening
//!   a folder it is next.
//!
//! Until contract 0.5.0 these are Rust methods for the cli and the tests.

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use genzo_imgio::capture::CaptureTime;
use genzo_model::edit::Phase;
use genzo_pipeline::gpu::develop::{Photo, as_shot};
use genzo_pipeline::raw::RawSource;

use crate::develop::{Applied, ViewSpec};
use crate::folder::{FolderPhoto, list_folder};
use crate::history::PhotoEdits;
use crate::prefetch::{Worker, neighbors};
use crate::{Engine, EngineError};
use genzo_model::edit::Edit;

/// What opens a photo's file: its decoded input and its orientation.
/// Injected so tests can see which files are decoded, and when.
pub trait Decode: Send + Sync + Debug {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError>;
}

/// Reads the file and decodes it with `genzo-imgio`.
#[derive(Debug, Default)]
pub struct FileDecoder;

impl Decode for FileDecoder {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let data = std::fs::read(path).map_err(|e| {
            let what = format!("{}: {e}", path.display());
            match e.kind() {
                std::io::ErrorKind::NotFound => EngineError::Missing(what),
                _ => EngineError::Io(what),
            }
        })?;
        crate::develop::decode(&data)
    }
}

/// Which way the last move went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Next,
    Previous,
}

impl Direction {
    /// The move from `from` to `to` in the strip, after `last`: next when
    /// nothing was selected, `last` when it is no move.
    fn of(from: Option<usize>, to: usize, last: Self) -> Self {
        match from {
            Some(f) if to < f => Self::Previous,
            Some(f) if to == f => last,
            _ => Self::Next,
        }
    }
}

/// Whether a strip photo renders (LIB-09, Data; DEV-01, States). Listing
/// reads no pixels, so it is known once the photo is decoded: by a switch
/// to it, or by prefetch as a neighbor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PhotoKind {
    /// Not decoded yet this session.
    #[default]
    Unknown,
    Renders,
    /// Recognized but not rendered, with the reason the status bar shows.
    Unsupported(String),
    /// Truncated, or failing a decoder's bounds checks.
    Damaged,
    /// Gone since the folder was listed.
    Missing,
}

impl PhotoKind {
    /// The status bar's reason for a photo that shows only its embedded
    /// preview; `None` for one that renders or is not known yet.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unknown | Self::Renders => None,
            Self::Unsupported(why) => Some(why),
            Self::Damaged => Some("This file is damaged"),
            Self::Missing => Some("This file is no longer there"),
        }
    }

    /// What a decode says about its file. `None` for an error that is not
    /// the file's (a read error, a stopped decoder): the kind stays.
    fn of<T>(decoded: &Result<T, EngineError>) -> Option<Self> {
        match decoded {
            Ok(_) => Some(Self::Renders),
            Err(EngineError::Unsupported(why)) => Some(Self::Unsupported(why.clone())),
            Err(EngineError::Damaged(_)) => Some(Self::Damaged),
            Err(EngineError::Missing(_)) => Some(Self::Missing),
            Err(_) => None,
        }
    }
}

/// One photo of the strip, as `ListFolderPhotos` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripPhoto {
    pub id: u64,
    pub file_name: String,
    pub capture_time: Option<CaptureTime>,
    /// Whether it renders, once it was decoded.
    pub kind: PhotoKind,
    /// Its edit differs from its defaults: the blue dot.
    pub edited: bool,
}

/// The strip and its current photo.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Strip {
    pub photos: Vec<StripPhoto>,
    pub current: Option<u64>,
    pub direction: Direction,
}

/// An opened folder and the render of its first photo: `None` when it has
/// none, an error when that photo does not open (it is still selected).
#[derive(Debug)]
pub struct Opened {
    pub strip: Strip,
    pub first: Option<Result<Applied, EngineError>>,
}

/// The session's side of the strip.
#[derive(Debug, Default)]
pub(crate) struct StripSession {
    /// Every file seen this session, by canonical path.
    ids: std::collections::HashMap<PathBuf, u64>,
    photos: Vec<(u64, FolderPhoto)>,
    current: Option<usize>,
    direction: Direction,
    /// What each photo's last decode said, by id: kept across folder
    /// switches like the edits.
    kinds: std::collections::HashMap<u64, PhotoKind>,
}

impl StripSession {
    /// Notes what decoding photo `id` said about its file.
    pub(crate) fn learn<T>(&mut self, id: u64, decoded: &Result<T, EngineError>) {
        if let Some(kind) = PhotoKind::of(decoded) {
            self.kinds.insert(id, kind);
        }
    }
}

impl Engine {
    /// Lists `folder`, replaces the strip with it and selects its first
    /// photo. A folder that cannot be read leaves the strip as it was.
    pub fn open_folder(&self, folder: &Path) -> Result<Opened, EngineError> {
        let listed = list_folder(folder)?;
        let _switch = self.switching.lock().expect("switching");
        let first = {
            let mut s = self.session.lock().expect("session");
            let photos: Vec<(u64, FolderPhoto)> = listed
                .into_iter()
                .map(|p| {
                    let key = std::fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone());
                    let id = match s.strip.ids.get(&key) {
                        Some(&id) => id,
                        None => {
                            let id = s.take_id();
                            s.strip.ids.insert(key, id);
                            id
                        }
                    };
                    (id, p)
                })
                .collect();
            // Listed again, so there again.
            for (id, _) in &photos {
                if s.strip.kinds.get(id) == Some(&PhotoKind::Missing) {
                    s.strip.kinds.remove(id);
                }
            }
            s.strip.photos = photos;
            s.strip.current = None;
            s.strip.direction = Direction::Next;
            if s.strip.photos.is_empty() {
                s.end_gesture();
                s.open = None;
            }
            s.strip.photos.first().map(|(id, _)| *id)
        };
        let first = match first {
            Some(id) => Some(self.switch_to(id)),
            // Nothing to prefetch around.
            None => {
                self.replan(None);
                None
            }
        };
        Ok(Opened {
            strip: self.strip(),
            first,
        })
    }

    /// The strip, with each photo's edited flag.
    pub fn strip(&self) -> Strip {
        let s = self.session.lock().expect("session");
        Strip {
            photos: s
                .strip
                .photos
                .iter()
                .map(|(id, p)| StripPhoto {
                    id: *id,
                    file_name: p.file_name.clone(),
                    capture_time: p.capture_time,
                    kind: s.strip.kinds.get(id).cloned().unwrap_or_default(),
                    edited: s.edits.get(id).is_some_and(PhotoEdits::is_edited),
                })
                .collect(),
            current: s.strip.current.map(|i| s.strip.photos[i].0),
            direction: s.strip.direction,
        }
    }

    /// Selects photo `id` of the strip and opens it in Develop, rendered
    /// settled at Fit. A photo that does not decode is still selected,
    /// with nothing open, and its error returned.
    pub fn select_photo(&self, id: u64) -> Result<Applied, EngineError> {
        let _switch = self.switching.lock().expect("switching");
        self.switch_to(id)
    }

    /// Selects the photo after or before the current one. `None` at that
    /// end of the strip, or with nothing selected: the strip does not wrap.
    pub fn step(&self, direction: Direction) -> Result<Option<Applied>, EngineError> {
        let _switch = self.switching.lock().expect("switching");
        let id = {
            let s = self.session.lock().expect("session");
            let Some(i) = s.strip.current else {
                return Ok(None);
            };
            let j = match direction {
                Direction::Next => i + 1,
                Direction::Previous => match i.checked_sub(1) {
                    Some(j) => j,
                    None => return Ok(None),
                },
            };
            match s.strip.photos.get(j) {
                Some((id, _)) => *id,
                None => return Ok(None),
            }
        };
        self.switch_to(id).map(Some)
    }

    /// The switch itself; the caller holds `switching`.
    fn switch_to(&self, id: u64) -> Result<Applied, EngineError> {
        let gpu = self.gpu()?;
        let (path, edit) = {
            let mut s = self.session.lock().expect("session");
            let index = s
                .strip
                .photos
                .iter()
                .position(|(p, _)| *p == id)
                .ok_or(EngineError::NotInStrip(id))?;
            // The photo already open: no switch, nothing decoded again.
            if s.strip.current == Some(index) && s.open == Some(id) {
                drop(s);
                return self.render(false, Phase::End);
            }
            s.end_gesture();
            s.strip.direction = Direction::of(s.strip.current, index, s.strip.direction);
            s.strip.current = Some(index);
            s.open = None;
            let out = match s.view {
                ViewSpec::Fit(out) | ViewSpec::Region { out, .. } => out,
            };
            s.view = ViewSpec::Fit(out);
            let edit = s.edits.get(&id).map(|p| p.current().clone());
            (s.strip.photos[index].1.path.clone(), edit)
        };
        // A neighbor prefetched for its edit: shown from its level 1 at
        // once, its input decoded after, before any neighbor.
        if let Some(c) = edit.and_then(|e| self.prefetch.cached(id, &e)) {
            gpu.open_level1(id, c.info.clone(), c.level1.clone())?;
            self.session.lock().expect("session").open = Some(id);
            self.replan(Some((id, path)));
            return self.render(false, Phase::End);
        }
        // One being built: opened from the input its build holds.
        if !gpu.open_prefetching(id)? {
            let decoded = self.decode_off_thread(path);
            self.session
                .lock()
                .expect("session")
                .strip
                .learn(id, &decoded);
            let (src, orientation) = match decoded {
                Ok(d) => d,
                Err(e) => {
                    self.prefetch.failed(id);
                    self.replan(None);
                    return Err(e);
                }
            };
            {
                let (_, kind) = as_shot(&src);
                let size = src.size();
                let mut s = self.session.lock().expect("session");
                s.edits
                    .entry(id)
                    .or_insert_with(|| PhotoEdits::new(kind, size, orientation));
            }
            gpu.open(id, Photo { src, orientation })?;
        }
        self.session.lock().expect("session").open = Some(id);
        self.replan(None);
        self.render(false, Phase::End)
    }

    /// Plans the current photo's neighbors after a switch: `attach` first,
    /// then those not cached for their edit, in the direction of travel.
    /// Cancels the builds of the photos no longer neighbors.
    fn replan(&self, attach: Option<(u64, PathBuf)>) {
        let Some(gpu) = self.gpu.clone() else {
            return;
        };
        let around: Vec<(u64, PathBuf, Option<Edit>)> = {
            let s = self.session.lock().expect("session");
            match s.strip.current {
                None => Vec::new(),
                Some(i) => neighbors(s.strip.photos.len(), i, s.strip.direction)
                    .into_iter()
                    .map(|j| {
                        let (id, p) = &s.strip.photos[j];
                        let edit = s.edits.get(id).map(|e| e.current().clone());
                        (*id, p.path.clone(), edit)
                    })
                    .collect(),
            }
        };
        let around = around
            .into_iter()
            .map(|(id, path, edit)| {
                let cached = edit.is_some_and(|e| self.prefetch.cached(id, &e).is_some());
                (id, path, cached)
            })
            .collect();
        let wanted = self.prefetch.replan(attach, around);
        let _ = gpu.keep_prefetching(&wanted);
        let mut worker = self.worker.lock().expect("worker");
        if worker.is_none() {
            let w = Worker {
                shared: self.prefetch.clone(),
                gpu,
                session: self.session.clone(),
                generations: self.generations.clone(),
                decoder: self.decoder.clone(),
            };
            *worker = std::thread::Builder::new()
                .name("genzo-prefetch".into())
                .spawn(move || w.run())
                .ok();
        }
    }

    /// Waits until the prefetch has nothing left to do: for the tests and
    /// `cli bench open`.
    #[doc(hidden)]
    pub fn wait_prefetch_idle(&self) {
        self.prefetch.wait_idle();
    }

    /// The strip's photos whose level 1 is cached: for the tests.
    #[doc(hidden)]
    pub fn prefetched(&self) -> Vec<u64> {
        let s = self.session.lock().expect("session");
        s.strip
            .photos
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| self.prefetch.is_cached(*id))
            .collect()
    }

    /// The render from level 0 of a photo whose last frame was
    /// provisional, once its input came, if there is one not taken yet.
    pub fn take_sharpened(&self) -> Option<Applied> {
        self.prefetch.take_sharpened()
    }

    /// Runs the decoder on the rayon pool and waits for it.
    fn decode_off_thread(&self, path: PathBuf) -> Result<(RawSource, u8), EngineError> {
        let decoder = self.decoder.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        rayon::spawn(move || {
            let _ = tx.send(decoder.decode(&path));
        });
        rx.recv()
            .map_err(|_| EngineError::Decode("the decoder stopped".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_direction_is_the_last_move() {
        use Direction::{Next, Previous};
        assert_eq!(Direction::of(None, 3, Previous), Next);
        assert_eq!(Direction::of(Some(2), 3, Previous), Next);
        assert_eq!(Direction::of(Some(2), 1, Next), Previous);
        // A click on the current photo is no move.
        assert_eq!(Direction::of(Some(2), 2, Previous), Previous);
        assert_eq!(Direction::of(Some(2), 2, Next), Next);
    }
}
