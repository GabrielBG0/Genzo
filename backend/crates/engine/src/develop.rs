//! Develop in the engine (DEV-01, APP-10): the open photo, each photo's
//! edit and history, and the renders the render thread runs for them.
//!
//! The engine owns the edit and its history. Each delta changes the edit
//! here first, then the whole edit goes to the render thread with a new
//! generation, so a newer request that replaces an older one loses nothing.
//! A gesture's *end* records its step whether or not its render ran.
//!
//! The clipping overlays (DEV-03) are the session's, not a photo's: J's
//! stays on across photo switches (owner, 2026-10-30), and the Alt preview
//! lasts only as long as the gesture it was set in. Each change renders
//! again, through stage 9 only.
//!
//! Until contract 0.5.0 these are Rust methods for the cli and the tests;
//! the shell's commands come with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};

use genzo_jobs::Generations;

use genzo_model::edit::{Change, Edit, Phase};
use genzo_pipeline::gpu::develop::{Photo, as_shot};
use genzo_pipeline::gpu::thread::{DevelopFrame, DevelopRequest, RenderThread};
use genzo_pipeline::raw::{RawError, RawSource};

use crate::histogram::{Counted, Histograms, Key, Overlay, Preview};
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
    pub(crate) target: Target,
    pub(crate) strip: crate::strip::StripSession,
    /// The generation of the last Develop render issued.
    pub(crate) issued: u64,
    /// The generation of the last Develop frame, if it came back
    /// provisional: drawn from level 1 before the photo's input came.
    pub(crate) provisional: Option<u64>,
    /// The J overlay and the Alt preview, sent with each render.
    pub(crate) overlay: Overlay,
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
            issued: 0,
            provisional: None,
            overlay: Overlay::default(),
        }
    }
}

impl Session {
    /// Ends the open photo's gesture, if one is in progress, recording its
    /// step as its *end* would: before that photo goes. The Alt preview
    /// ends with it.
    pub(crate) fn end_gesture(&mut self) {
        self.overlay.preview = Preview::None;
        if let Some(open) = self.open
            && let Some(p) = self.edits.get_mut(&open)
            && p.in_gesture()
        {
            // No change to apply, so nothing to refuse.
            let _ = p.apply(&[], Phase::End);
        }
    }

    /// Whether the open photo's gesture is in progress.
    fn in_gesture(&self) -> bool {
        self.open
            .and_then(|id| self.edits.get(&id))
            .is_some_and(PhotoEdits::in_gesture)
    }

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
        // A strip switch cannot interleave with this opening.
        let _switch = self.switching.lock().expect("switching");
        let (_, kind) = as_shot(&src);
        let size = src.size();
        let id = {
            let mut s = self.session.lock().expect("session");
            s.end_gesture();
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

    /// Turns the J overlays on or off, each with its color as RGBA8, and
    /// renders the open photo again. They stay as set across photo
    /// switches. `None` with no photo open: the state is kept for the next.
    pub fn set_clipping_overlay(
        &self,
        shadows: Option<[u8; 4]>,
        highlights: Option<[u8; 4]>,
    ) -> Result<Option<Applied>, EngineError> {
        let phase = {
            let mut s = self.session.lock().expect("session");
            s.overlay.shadows = shadows;
            s.overlay.highlights = highlights;
            if s.open.is_none() {
                return Ok(None);
            }
            if s.in_gesture() {
                Phase::Update
            } else {
                Phase::End
            }
        };
        self.render(false, phase).map(Some)
    }

    /// Sets the Alt clipping preview of the gesture in progress and renders
    /// its frame again. Outside a gesture there is none: `None`, nothing
    /// rendered, as when the preview is already `preview`.
    pub fn set_clipping_preview(&self, preview: Preview) -> Result<Option<Applied>, EngineError> {
        {
            let mut s = self.session.lock().expect("session");
            if !s.in_gesture() || s.overlay.preview == preview {
                return Ok(None);
            }
            s.overlay.preview = preview;
        }
        self.render(false, Phase::Update).map(Some)
    }

    /// The latest histogram of `photo`, if it is the one shown
    /// (`GetHistogram`).
    pub fn histogram(&self, photo: u64) -> Option<Counted> {
        self.histograms.latest(photo)
    }

    /// Waits until every histogram count asked for is sent or dropped: for
    /// the tests.
    #[doc(hidden)]
    pub fn wait_histograms_idle(&self) {
        self.histograms.wait_idle();
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
        render_when(
            self.gpu()?,
            &self.session,
            &self.generations,
            &self.histograms,
            recorded,
            phase,
            |_| true,
        )
        .map(|a| a.expect("always rendered"))
    }
}

/// Renders the open photo's current edit with a new generation, and notes
/// whether the frame came back provisional. The engine's renders and the
/// prefetch worker's both come here, and so does each frame's histogram:
/// its own at Fit, else the whole photo's asked for after a settled
/// frame. Nothing is issued, and the answer is
/// `None`, unless `when` holds of the session as the generation is taken:
/// the check and the request are one step.
pub(crate) fn render_when(
    gpu: &RenderThread,
    session: &Arc<Mutex<Session>>,
    generations: &Generations,
    histograms: &Arc<Histograms>,
    recorded: bool,
    phase: Phase,
    when: impl FnOnce(&Session) -> bool,
) -> Result<Option<Applied>, EngineError> {
    let (state, request) = {
        let mut s = session.lock().expect("session");
        let id = s.open.ok_or(EngineError::NoPhoto)?;
        if !when(&s) {
            return Ok(None);
        }
        let generation = generations.next();
        s.issued = generation;
        // A settled render shows no preview, whatever was set.
        if phase == Phase::End || !s.in_gesture() {
            s.overlay.preview = Preview::None;
        }
        let p = &s.edits[&id];
        let request = DevelopRequest {
            generation,
            photo: id,
            edit: p.current().clone(),
            view: s.view,
            target: s.target,
            phase,
            overlay: s.overlay,
        };
        (state(id, p), request)
    };
    let generation = request.generation;
    let key = Key {
        photo: request.photo,
        edit: request.edit.clone(),
        target: request.target,
        view: request.view,
    };
    let frame = gpu.develop(request)?;
    if let Some(f) = &frame {
        let last = {
            let mut s = session.lock().expect("session");
            // A frame older than the last one issued says nothing about it.
            let last = s.issued == generation;
            if last {
                s.provisional = f.provisional.then_some(generation);
            }
            last
        };
        match &f.histogram {
            Some(h) => histograms.deliver(Counted {
                photo: key.photo,
                generation,
                histogram: h.clone(),
            }),
            None if last && !phase.interactive() => histograms.whole(gpu, session, generation, key),
            None => {}
        }
    }
    Ok(Some(Applied {
        recorded,
        state,
        frame,
    }))
}

/// A file's bytes decoded into the render thread's input, with the file's
/// orientation.
pub(crate) fn decode(data: &[u8]) -> Result<(RawSource, u8), EngineError> {
    let img = genzo_imgio::decode(data, genzo_imgio::DecodeOptions::default())
        .map_err(EngineError::decode)?;
    let orientation = img.orientation;
    RawSource::from_decoded(img)
        .map(|src| (src, orientation))
        .map_err(|e| match e {
            // Corrupt levels are a damaged file; the rest (missing color
            // data, an encoding M1 refuses) unsupported (DEV-01, States).
            RawError::Levels(m) => EngineError::Damaged(m),
            RawError::Unsupported(m) => EngineError::Unsupported(m),
            e @ RawError::Encoding(_) => EngineError::Unsupported(e.to_string()),
        })
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
