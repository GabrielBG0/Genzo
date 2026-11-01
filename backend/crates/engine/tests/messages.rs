//! Contract 0.5.0 through `execute` and `query`, with an event sink, as the
//! shell and `cli run` drive it.
//!
//! - Every frame stored goes to the sink as `RenderFinished` before its
//!   counts go as `HistogramReady` (DEV-03): at Fit, at 1:1 (the whole
//!   photo's, later), for the switch's preview, and for the prefetch's
//!   sharper render.
//! - A view set before any folder is kept for its first photo.
//! - What the user can act on comes back as data: a refused pick as the
//!   edit with its reason, a photo that does not render as its selection
//!   with the reason.
//! - A command for a photo not open changes nothing and answers its edit;
//!   one for a photo never opened is an error. Each reply's revision
//!   orders it.
//!
//! A fake decoder turns each stub file into a synthetic photo, and can hold
//! one decode until the test releases it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use genzo_engine::api::edit::{Change, Phase};
use genzo_engine::api::{
    ClippingPreview, ColorSpace, Command, CommandResult, Event, PhotoEdit, PhotoKind, Query,
    QueryResult, View,
};
use genzo_engine::strip::Decode;
use genzo_engine::{Engine, EngineError, split_frame};
use genzo_imgio::{ColorEncoding, Preview};
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::RawSource;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

const UNSUPPORTED: &str = "Nikon High Efficiency RAW is not supported yet";

/// Decodes "dark" names to a black photo and "unsupported" ones to an
/// error; holds a decode while its name is held.
#[derive(Debug, Default)]
struct Fake {
    held: Mutex<HashSet<String>>,
    started: Mutex<HashSet<String>>,
    changed: Condvar,
}

impl Fake {
    fn hold(&self, name: &str) {
        self.held.lock().unwrap().insert(name.into());
    }

    fn release(&self, name: &str) {
        self.held.lock().unwrap().remove(name);
        self.changed.notify_all();
    }

    fn wait_started(&self, name: &str) {
        let mut s = self.started.lock().unwrap();
        while !s.contains(name) {
            s = self.changed.wait(s).unwrap();
        }
    }
}

fn source_for(name: &str) -> RawSource {
    let n = name.bytes().map(u32::from).sum::<u32>() % 7;
    let mut src = with_camera_profile(scene_raw(300 + 20 * n, 220 + 10 * n));
    if name.contains("dark") {
        src.data.fill(0);
    }
    src
}

impl Decode for Fake {
    fn decode(&self, path: &Path) -> Result<(RawSource, u8), EngineError> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        self.started.lock().unwrap().insert(name.clone());
        self.changed.notify_all();
        let mut held = self.held.lock().unwrap();
        while held.contains(&name) {
            held = self.changed.wait(held).unwrap();
        }
        if name.contains("unsupported") {
            return Err(EngineError::Unsupported(UNSUPPORTED.into()));
        }
        Ok((source_for(&name), 1))
    }

    fn preview(&self, _: &Path) -> Result<Option<Preview>, EngineError> {
        Ok(Some(Preview {
            width: 4,
            height: 2,
            rgba: [90, 120, 150, 255].repeat(8),
            orientation: 1,
            color: ColorEncoding::Srgb,
        }))
    }
}

/// Releases every held decode when dropped, before the engine is.
struct ReleaseAll(Arc<Fake>);

impl Drop for ReleaseAll {
    fn drop(&mut self) {
        self.0.held.lock().unwrap().clear();
        self.0.changed.notify_all();
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str, names: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("genzo-messages-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"stub").unwrap();
        }
        Self(dir)
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An engine with the fake decoder, and the events it sent.
struct Session {
    engine: Engine,
    events: Arc<Mutex<Vec<Event>>>,
    _release: ReleaseAll,
}

impl Session {
    fn new(fake: &Arc<Fake>) -> Option<Self> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let engine = Engine::new()
            .with_decoder(fake.clone())
            .with_event_sink(Arc::new(move |e| sink.lock().unwrap().push(e)));
        if engine.info().adapter.name == "none" {
            if std::env::var("GENZO_REQUIRE_GPU").as_deref() == Ok("1") {
                panic!("no GPU adapter");
            }
            eprintln!("skipped: no GPU adapter");
            return None;
        }
        Some(Self {
            engine,
            events,
            _release: ReleaseAll(fake.clone()),
        })
    }

    fn run(&self, command: Command) -> CommandResult {
        self.engine.execute(command).unwrap()
    }

    /// The events sent since the last call, once every count asked for
    /// is sent.
    fn events(&self) -> Vec<Event> {
        self.engine.wait_histograms_idle();
        std::mem::take(&mut self.events.lock().unwrap())
    }

    fn view(&self, width: u32, height: u32, view: View) {
        let done = self.run(Command::SetView {
            width,
            height,
            view,
            space: ColorSpace::Srgb,
        });
        assert_eq!(done, CommandResult::Done);
    }

    fn delta(&self, photo: u64, change: Change, phase: Phase) -> (PhotoEdit, bool) {
        match self.run(Command::ApplyEditDelta {
            photo,
            changes: vec![change],
            phase,
        }) {
            CommandResult::Edit {
                edit,
                recorded,
                refused: None,
            } => (edit, recorded),
            other => panic!("{other:?}"),
        }
    }

    fn edit_of(&self, command: Command) -> PhotoEdit {
        match self.run(command) {
            CommandResult::Edit {
                edit,
                recorded: false,
                refused: None,
            } => edit,
            other => panic!("{other:?}"),
        }
    }
}

/// The opened folder's photo ids and its first photo's edit.
fn opened(result: CommandResult) -> (Vec<u64>, PhotoEdit) {
    match result {
        CommandResult::Folder {
            strip,
            edit: Some(edit),
            reason: None,
        } => (strip.photos.iter().map(|p| p.id).collect(), edit),
        other => panic!("{other:?}"),
    }
}

/// The generations of the frames sent, in order.
fn frames(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::RenderFinished { generation, .. } => Some(*generation),
            _ => None,
        })
        .collect()
}

/// Checks that each count follows the frame or the preview it counts, and
/// returns the generations counted.
fn counts_follow_their_frames(events: &[Event]) -> Vec<u64> {
    let mut seen = Vec::new();
    let mut counted = Vec::new();
    for e in events {
        match e {
            Event::RenderFinished { generation, .. } => seen.push(*generation),
            Event::PreviewShown {
                generation: Some(g),
                ..
            } => seen.push(*g),
            Event::HistogramReady {
                generation,
                histogram,
                ..
            } => {
                assert!(
                    seen.contains(generation),
                    "HistogramReady {generation} before its frame, in {:?}",
                    kinds(events)
                );
                assert_eq!(histogram.red.len(), 256);
                counted.push(*generation);
            }
            _ => {}
        }
    }
    counted
}

fn kinds(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            Event::RenderFinished { generation, .. } => format!("frame {generation}"),
            Event::PreviewShown { generation, .. } => format!("preview {generation:?}"),
            Event::HistogramReady { generation, .. } => format!("counts {generation}"),
            e => format!("{e:?}"),
        })
        .collect()
}

#[test]
fn a_view_set_before_any_folder_is_kept_for_its_first_photo() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(s) = Session::new(&fake) else { return };
    // Nothing is open: kept, and nothing rendered.
    s.view(200, 150, View::Fit);
    assert!(s.events().is_empty());

    let dir = TempDir::new("view", &["p0.jpg", "p1.jpg"]);
    let (ids, edit) = opened(s.run(Command::OpenFolder { path: dir.path() }));
    assert_eq!(edit.photo, ids[0]);
    let events = s.events();
    // Its preview first, then its frame, fitted into the view.
    let Some(Event::PreviewShown {
        photo,
        generation: Some(preview),
        tile_count: Some(1),
    }) = events.first()
    else {
        panic!("{:?}", kinds(&events))
    };
    assert_eq!(*photo, ids[0]);
    let frame = frames(&events)[0];
    assert!(frame > *preview);
    let bytes = s.engine.frame(frame).expect("kept");
    let tiles = split_frame(&bytes).unwrap();
    let (w, h) = (tiles[0].0.image_width, tiles[0].0.image_height);
    assert!(w <= 200 && h <= 150 && (w == 200 || h == 150), "{w} × {h}");
}

#[test]
fn each_frame_goes_before_its_counts_at_fit_and_at_1_1() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(s) = Session::new(&fake) else { return };
    s.view(200, 150, View::Fit);
    let dir = TempDir::new("order", &["p0.jpg", "p1.jpg"]);
    let (ids, _) = opened(s.run(Command::OpenFolder { path: dir.path() }));
    let p = ids[0];
    let events = s.events();
    let counted = counts_follow_their_frames(&events);
    assert_eq!(counted.len(), 2, "the preview's and the frame's");

    // A drag, the Alt preview during it, and its end: each frame counted
    // after it.
    s.delta(p, Change::Exposure(0.5), Phase::Start);
    let preview = ClippingPreview::Highlights;
    assert_eq!(
        s.run(Command::SetClippingPreview { preview }),
        CommandResult::Done
    );
    s.delta(p, Change::Exposure(0.7), Phase::End);
    let events = s.events();
    assert_eq!(frames(&events).len(), 3);
    assert_eq!(counts_follow_their_frames(&events), frames(&events));
    for e in &events {
        if let Event::RenderFinished { photo, .. } = e {
            assert_eq!(*photo, Some(p));
        }
    }

    // At 1:1 the whole photo's counts come after the visible tiles' frame,
    // under its generation; a pan counts the same pixels and sends none.
    s.view(
        200,
        150,
        View::Region {
            zoom: 1.0,
            x: 10.0,
            y: 10.0,
        },
    );
    let events = s.events();
    assert_eq!(counts_follow_their_frames(&events), frames(&events));
    assert_eq!(frames(&events).len(), 1);
    s.view(
        200,
        150,
        View::Region {
            zoom: 1.0,
            x: 30.0,
            y: 20.0,
        },
    );
    let events = s.events();
    assert_eq!(frames(&events).len(), 1);
    assert!(counts_follow_their_frames(&events).is_empty());

    // J's overlays render again, as one frame; outside a gesture the Alt
    // preview renders nothing.
    let overlay = Command::SetClippingOverlay {
        shadows: true,
        highlights: true,
        shadow_color: [0, 0, 255, 255],
        highlight_color: [255, 0, 0, 255],
    };
    assert_eq!(s.run(overlay), CommandResult::Done);
    assert_eq!(frames(&s.events()).len(), 1);
    let preview = ClippingPreview::Shadows;
    assert_eq!(
        s.run(Command::SetClippingPreview { preview }),
        CommandResult::Done
    );
    assert!(s.events().is_empty());
}

#[test]
fn the_prefetchs_sharper_render_is_sent_as_a_frame_then_counted() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(s) = Session::new(&fake) else { return };
    let dir = TempDir::new("sharpen", &["p0.jpg", "p1.jpg", "p2.jpg"]);
    // A view that wants level 0, so a switch from level 1 is provisional.
    s.view(600, 450, View::Fit);
    let (ids, _) = opened(s.run(Command::OpenFolder { path: dir.path() }));
    s.engine.wait_prefetch_idle();
    s.events();

    fake.hold("p1.jpg");
    let selected = s.run(Command::SelectPhoto { photo: ids[1] });
    assert!(matches!(
        selected,
        CommandResult::Selected {
            edit: Some(_),
            reason: None,
            ..
        }
    ));
    fake.wait_started("p1.jpg");
    fake.release("p1.jpg");
    s.engine.wait_prefetch_idle();
    let events = s.events();
    let sent = frames(&events);
    assert_eq!(sent.len(), 2, "the switch's frame, then the sharper one");
    assert_eq!(counts_follow_their_frames(&events).last(), sent.last());
    let sharp = s.engine.take_sharpened().expect("rendered again");
    assert_eq!(Some(&sharp.frame.unwrap().generation), sent.last());
    assert!(events.iter().all(|e| match e {
        Event::RenderFinished { photo, .. } => *photo == Some(ids[1]),
        _ => true,
    }));
}

#[test]
fn a_refusal_and_a_photo_that_does_not_render_answer_with_their_reason() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(s) = Session::new(&fake) else { return };
    s.view(200, 150, View::Fit);
    let dir = TempDir::new("reason", &["a_dark.jpg", "b_unsupported.jpg"]);
    let (ids, _) = opened(s.run(Command::OpenFolder { path: dir.path() }));
    s.events();

    // A black photo is too dark everywhere: its edit as it was, no step,
    // no frame.
    let picked = s.run(Command::PickWhiteBalance {
        photo: ids[0],
        x: 0.5,
        y: 0.5,
    });
    let CommandResult::Edit {
        edit,
        recorded: false,
        refused: Some(why),
    } = picked
    else {
        panic!("{picked:?}")
    };
    assert_eq!(why, "Can't set white balance here: the area is too dark");
    assert!(!edit.edited);
    assert!(frames(&s.events()).is_empty());

    // Selected, it shows its preview and says why; nothing is open.
    let selected = s.run(Command::SelectPhoto { photo: ids[1] });
    assert_eq!(
        selected,
        CommandResult::Selected {
            photo: ids[1],
            edit: None,
            reason: Some(UNSUPPORTED.into()),
        }
    );
    let events = s.events();
    assert!(matches!(
        events[..],
        [Event::PreviewShown { photo, generation: Some(_), .. }, ..] if photo == ids[1]
    ));
    assert!(frames(&events).is_empty());
    let QueryResult::FolderPhotos { strip } = s.engine.query(Query::ListFolderPhotos) else {
        panic!()
    };
    let listed = &strip.photos[1];
    assert_eq!(strip.current, Some(ids[1]));
    assert_eq!(listed.kind, PhotoKind::Unsupported);
    assert_eq!(listed.reason.as_deref(), Some(UNSUPPORTED));
}

#[test]
fn a_command_for_a_photo_not_open_changes_nothing_and_replies_are_ordered() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Arc::new(Fake::default());
    let Some(s) = Session::new(&fake) else { return };
    s.view(200, 150, View::Fit);
    let dir = TempDir::new("stale", &["p0.jpg", "p1.jpg"]);
    let (ids, first) = opened(s.run(Command::OpenFolder { path: dir.path() }));
    let (p0, p1) = (ids[0], ids[1]);

    // Each change raises the revision, a gesture's updates too.
    let (start, recorded) = s.delta(p0, Change::Exposure(0.3), Phase::Start);
    assert!(!recorded);
    let (end, recorded) = s.delta(p0, Change::Exposure(0.6), Phase::End);
    assert!(recorded);
    assert!(first.revision < start.revision && start.revision < end.revision);
    assert_eq!(end.undo.as_deref(), Some("Exposure"));
    let reset = s.run(Command::ResetParam {
        photo: p0,
        param: None,
    });
    let CommandResult::Edit {
        edit: reset,
        recorded: true,
        refused: None,
    } = reset
    else {
        panic!("{reset:?}")
    };
    assert!(!reset.edited && reset.revision > end.revision);
    assert_eq!(reset.undo.as_deref(), Some("Reset all"));
    let undone = s.edit_of(Command::Undo { photo: p0 });
    assert_eq!(undone.edit, end.edit);
    assert!(undone.revision > reset.revision);
    s.events();

    // After a switch, p0's undo and delta change nothing and answer its
    // edit as it is, at the same revision.
    assert!(matches!(
        s.run(Command::SelectPhoto { photo: p1 }),
        CommandResult::Selected { edit: Some(_), .. }
    ));
    // p1's frame may come again from level 0, later: not p0's.
    s.engine.wait_prefetch_idle();
    s.events();
    assert_eq!(s.edit_of(Command::Undo { photo: p0 }), undone);
    let stale = s.delta(p0, Change::Exposure(2.0), Phase::End);
    assert_eq!(stale, (undone.clone(), false));
    assert!(s.events().iter().all(|e| !matches!(
        e,
        Event::RenderFinished { photo, .. } if *photo == Some(p0)
    )));
    let QueryResult::Edit { edit: Some(asked) } = s.engine.query(Query::GetEdit { photo: p0 })
    else {
        panic!()
    };
    assert_eq!(asked, undone);

    // Only the shown photo has a histogram.
    let counted = |photo| match s.engine.query(Query::GetHistogram { photo }) {
        QueryResult::Histogram { counted } => counted,
        other => panic!("{other:?}"),
    };
    assert!(counted(p0).is_none());
    assert_eq!(counted(p1).expect("counted").photo, p1);

    // A photo never opened: no edit, and its commands are errors.
    let never = p1 + 100;
    assert_eq!(
        s.engine.query(Query::GetEdit { photo: never }),
        QueryResult::Edit { edit: None }
    );
    assert!(matches!(
        s.engine.execute(Command::Undo { photo: never }),
        Err(EngineError::UnknownPhoto(p)) if p == never
    ));
}
