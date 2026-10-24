//! Develop in the engine (DEV-01, APP-10): the open photo, each photo's
//! edit and history, and the renders the render thread runs for them.
//!
//! The engine owns the edit and its history. Each delta changes the edit
//! here first, then the whole edit goes to the render thread with a new
//! generation, so a newer request that replaces an older one loses nothing.
//! A gesture's *end* records its step whether or not its render ran.
//!
//! Until contract 0.5.0 these are Rust methods for the cli and the tests;
//! the shell's commands come with it.

use std::collections::HashMap;
use std::sync::mpsc;

use genzo_model::edit::{Change, Edit, Phase};
use genzo_pipeline::gpu::develop::{Photo, as_shot};
use genzo_pipeline::gpu::thread::{DevelopFrame, DevelopRequest};
use genzo_pipeline::raw::RawSource;

use crate::history::{PhotoEdits, Refused};
use crate::{Engine, EngineError};

pub use genzo_color::output::Target;
pub use genzo_pipeline::gpu::develop::{Readback, Reran};
pub use genzo_pipeline::reference::sampling::ViewSpec;

/// The session's photos and the one open in Develop.
#[derive(Debug)]
pub(crate) struct Session {
    pub(crate) edits: HashMap<u64, PhotoEdits>,
    pub(crate) open: Option<u64>,
    next_id: u64,
    pub(crate) view: ViewSpec,
    target: Target,
    pub(crate) strip: crate::strip::StripSession,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            edits: HashMap::new(),
            open: None,
            next_id: 1,
            view: ViewSpec::Fit((1280, 960)),
            target: Target::Srgb,
            strip: Default::default(),
        }
    }
}

impl Session {
    /// A new photo id, never given before this session.
    pub(crate) fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// The open photo's edit as the panels show it.
#[derive(Debug, Clone, PartialEq)]
pub struct EditState {
    pub photo: u64,
    pub edit: Edit,
    /// Differs from the photo's defaults: the edited dots.
    pub edited: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
}

/// What a delta, an undo or a redo did.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// Whether a history step was recorded.
    pub recorded: bool,
    pub state: EditState,
    /// The render, or None when a newer request replaced it.
    pub frame: Option<DevelopFrame>,
}

impl From<Refused> for EngineError {
    fn from(e: Refused) -> Self {
        Self::Refused(e.to_string())
    }
}

impl Engine {
    /// Decodes a file's bytes, off the render thread, and opens it in
    /// Develop. Returns the photo's id.
    pub fn open_photo(&self, data: Vec<u8>) -> Result<u64, EngineError> {
        let (tx, rx) = mpsc::channel();
        rayon::spawn(move || {
            let _ = tx.send(decode(&data));
        });
        let (src, orientation) = rx
            .recv()
            .map_err(|_| EngineError::Decode("the decoder stopped".into()))??;
        self.open_source(src, orientation)
    }

    /// Opens a decoded photo in Develop, with the file's `orientation`.
    /// Returns its id; its edit starts at its defaults.
    pub fn open_source(&self, src: RawSource, orientation: u8) -> Result<u64, EngineError> {
        let gpu = self.gpu()?;
        let (_, kind) = as_shot(&src);
        let size = src.size();
        let id = {
            let mut s = self.session.lock().expect("session");
            let id = s.take_id();
            s.edits.insert(id, PhotoEdits::new(kind, size, orientation));
            id
        };
        gpu.open(id, Photo { src, orientation })?;
        self.session.lock().expect("session").open = Some(id);
        Ok(id)
    }

    /// The open photo's edit, history labels and edited flag.
    pub fn edit_state(&self) -> Option<EditState> {
        let s = self.session.lock().expect("session");
        let id = s.open?;
        Some(state(id, &s.edits[&id]))
    }

    /// Sets the view and the target, and renders the open photo settled.
    pub fn set_view(&self, view: ViewSpec, target: Target) -> Result<Applied, EngineError> {
        {
            let mut s = self.session.lock().expect("session");
            s.view = view;
            s.target = target;
        }
        self.render(false, Phase::End)
    }

    /// Applies `changes` to the open photo in `phase` and renders it.
    pub fn apply(&self, changes: &[Change], phase: Phase) -> Result<Applied, EngineError> {
        let recorded = {
            let mut s = self.session.lock().expect("session");
            let id = s.open.ok_or(EngineError::NoPhoto)?;
            s.edits.get_mut(&id).expect("open").apply(changes, phase)?
        };
        self.render(recorded, phase)
    }

    /// Undoes the open photo's last step and renders it settled. Nothing
    /// changes without a step to undo, or during a gesture.
    pub fn undo(&self) -> Result<Applied, EngineError> {
        self.history(PhotoEdits::undo)
    }

    /// Applies the open photo's step undone last again.
    pub fn redo(&self) -> Result<Applied, EngineError> {
        self.history(PhotoEdits::redo)
    }

    fn history(&self, f: fn(&mut PhotoEdits) -> bool) -> Result<Applied, EngineError> {
        let changed = {
            let mut s = self.session.lock().expect("session");
            let id = s.open.ok_or(EngineError::NoPhoto)?;
            f(s.edits.get_mut(&id).expect("open"))
        };
        if !changed {
            let state = self.edit_state().ok_or(EngineError::NoPhoto)?;
            return Ok(Applied {
                recorded: false,
                state,
                frame: None,
            });
        }
        self.render(false, Phase::End)
    }

    /// Renders the open photo's current edit with a new generation.
    pub(crate) fn render(&self, recorded: bool, phase: Phase) -> Result<Applied, EngineError> {
        let gpu = self.gpu()?;
        let (state, request) = {
            let s = self.session.lock().expect("session");
            let id = s.open.ok_or(EngineError::NoPhoto)?;
            let p = &s.edits[&id];
            let request = DevelopRequest {
                generation: self.generations.next(),
                photo: id,
                edit: p.current().clone(),
                view: s.view,
                target: s.target,
                phase,
            };
            (state(id, p), request)
        };
        let frame = gpu.develop(request)?;
        Ok(Applied {
            recorded,
            state,
            frame,
        })
    }
}

/// A file's bytes decoded into the render thread's input, with the file's
/// orientation.
pub(crate) fn decode(data: &[u8]) -> Result<(RawSource, u8), EngineError> {
    let img = genzo_imgio::decode(data, genzo_imgio::DecodeOptions::default())
        .map_err(EngineError::decode)?;
    let orientation = img.orientation;
    RawSource::from_decoded(img)
        .map(|src| (src, orientation))
        .map_err(|e| EngineError::Decode(e.to_string()))
}

fn state(id: u64, p: &PhotoEdits) -> EditState {
    EditState {
        photo: id,
        edit: p.current().clone(),
        edited: p.is_edited(),
        undo: p.undo_label().map(String::from),
        redo: p.redo_label().map(String::from),
    }
}
