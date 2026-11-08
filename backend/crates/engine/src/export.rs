//! M1's export (OUT-01): the photo open in Develop as a JPEG file.
//!
//! - **The command** ([`Engine::export_jpeg`]) checks the destination,
//!   ends a gesture in progress, and has the render thread take what the
//!   export renders, now: its edit, its input, a copy of its base layer.
//!   Later edits and switches do not change it.
//! - **The destination** is refused when it is a file this session listed
//!   (any folder opened, OUT-01 *Encoding and writing*), when it exists and
//!   `replace` was not given, or when an export in progress will write it.
//! - **A thread per export** reads the photo's metadata, then takes the
//!   rows of regions as the render thread finishes them and streams them
//!   into the JPEG writer (`genzo_export::jpeg`): a temporary file renamed
//!   at the end. It waits while a photo switch runs, so the encode never
//!   delays first pixels.
//! - **Progress** goes out as `ExportProgress`: queued, running with its
//!   rows, then exactly one of finished (with notes), failed or cancelled.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use genzo_model::api::{Event, ExportState};
use genzo_pipeline::gpu::develop::DevelopError;
use genzo_pipeline::gpu::thread::{ExportJob, ExportPiece};

use crate::develop::Target;
use crate::{Engine, EngineError, EventSink};

/// What the dialog sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub photo: u64,
    pub path: PathBuf,
    /// Fit the long edge to this many pixels; `None` for full size.
    pub long_edge: Option<u32>,
    /// 1 to 100.
    pub quality: u8,
    /// Replace a file already at `path`.
    pub replace: bool,
}

/// Refused: the destination is a photo Genzo opened (OUT-01).
pub const ORIGINAL: &str =
    "Genzo does not write over the photos it opens. Choose another name or folder.";

/// The smallest long edge an export takes: DEV-01's smallest crop.
pub const MIN_LONG_EDGE: u32 = 32;

/// How long a command waits for a photo opened from its prefetched level 1
/// to be decoded before its export can start.
const DECODE_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The exports in progress, by job.
#[derive(Debug, Default)]
pub(crate) struct Exports {
    next: AtomicU64,
    jobs: Mutex<HashMap<u64, Running>>,
    /// Signalled when a job ends.
    ended: std::sync::Condvar,
}

#[derive(Debug)]
struct Running {
    /// The destination, as compared: its folder canonicalized.
    destination: PathBuf,
    cancelled: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Exports {
    /// Reserves `destination` for a new job, unless an export in progress
    /// will write it: the check and the reservation under one lock, so two
    /// commands for one path (a double click) cannot both pass.
    fn reserve(&self, destination: &Path, cancelled: &Arc<AtomicBool>) -> Option<u64> {
        let mut jobs = self.jobs.lock().expect("exports");
        if jobs.values().any(|r| r.destination == destination) {
            return None;
        }
        let job = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        jobs.insert(
            job,
            Running {
                destination: destination.to_path_buf(),
                cancelled: Arc::clone(cancelled),
                thread: None,
            },
        );
        Some(job)
    }

    /// Waits until every export has ended.
    pub(crate) fn wait_idle(&self) {
        let mut jobs = self.jobs.lock().expect("exports");
        while !jobs.is_empty() {
            jobs = self.ended.wait(jobs).expect("exports");
        }
    }

    /// Cancels every export and waits for their threads: when the engine
    /// goes.
    pub(crate) fn stop(&self) {
        let threads: Vec<_> = {
            let mut jobs = self.jobs.lock().expect("exports");
            jobs.values_mut()
                .filter_map(|r| {
                    r.cancelled.store(true, Ordering::SeqCst);
                    r.thread.take()
                })
                .collect()
        };
        for t in threads {
            let _ = t.join();
        }
    }
}

/// `path`'s folder canonicalized, with its file name: the same file
/// whatever spelling of the folder, before the file exists. A file that
/// exists is resolved whole, links included.
fn resolve(path: &Path) -> Result<PathBuf, EngineError> {
    if let Ok(whole) = std::fs::canonicalize(path) {
        return Ok(whole);
    }
    let name = path
        .file_name()
        .ok_or_else(|| EngineError::Invalid(format!("{} names no file", path.display())))?;
    let folder = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let folder = std::fs::canonicalize(folder)
        .map_err(|_| EngineError::Io(format!("The folder {} does not exist", folder.display())))?;
    Ok(folder.join(name))
}

impl Engine {
    /// Starts exporting `request.photo`, which must be open in Develop and
    /// render, and answers its job. A gesture in progress ends first.
    pub fn export_jpeg(&self, request: ExportRequest) -> Result<u64, EngineError> {
        if !(1..=100).contains(&request.quality) {
            return Err(EngineError::Invalid(format!("quality {}", request.quality)));
        }
        if let Some(edge) = request.long_edge
            && edge < MIN_LONG_EDGE
        {
            return Err(EngineError::Invalid(format!(
                "a long edge of {edge} px; {MIN_LONG_EDGE} at least"
            )));
        }
        let jpeg = request
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        if !jpeg {
            return Err(EngineError::Invalid(format!(
                "{} is not a .jpg file",
                request.path.display()
            )));
        }
        let destination = resolve(&request.path)?;
        let (edit, source) = {
            let mut s = self.session.lock().expect("session");
            if s.strip.is_listed(&destination) {
                return Err(EngineError::Refused(ORIGINAL.into()));
            }
            if s.open != Some(request.photo) {
                return Err(EngineError::NoPhoto);
            }
            s.end_gesture();
            let edit = s
                .edits
                .get(&request.photo)
                .map(|p| p.current().clone())
                .ok_or(EngineError::UnknownPhoto(request.photo))?;
            (edit, s.strip.path_of(request.photo))
        };
        if destination.exists() && !request.replace {
            let name = destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            return Err(EngineError::Refused(format!("{name} already exists")));
        }
        let gpu = self.gpu()?;
        // Reserved before the snapshot, so a second command for the same
        // path in the meantime is refused.
        let cancelled = Arc::new(AtomicBool::new(false));
        let Some(job) = self.exports.reserve(&destination, &cancelled) else {
            return Err(EngineError::Refused(
                "An export to this file is in progress".into(),
            ));
        };
        let started = std::time::Instant::now();
        let export = loop {
            match gpu.export(request.photo, job, &edit, Target::Srgb, request.long_edge) {
                Ok(Ok(export)) => break Ok(export),
                Ok(Err(DevelopError::NeedsSource))
                    if started.elapsed() < DECODE_WAIT
                        && self.session.lock().expect("session").open == Some(request.photo) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Ok(Err(DevelopError::NeedsSource)) => {
                    break Err(EngineError::Refused(
                        "The photo was closed before it finished opening".into(),
                    ));
                }
                Ok(Err(e)) => break Err(EngineError::Gpu(e.to_string())),
                Err(e) => break Err(e.into()),
            }
        };
        let export = match export {
            Ok(e) => e,
            Err(e) => {
                self.exports.jobs.lock().expect("exports").remove(&job);
                self.exports.ended.notify_all();
                return Err(e);
            }
        };
        let task = Task {
            job,
            photo: request.photo,
            path: request.path.display().to_string(),
            destination,
            source,
            quality: request.quality,
            export,
            cancelled,
            events: self.events.clone(),
            switching: Arc::clone(&self.switching),
            exports: Arc::clone(&self.exports),
        };
        task.progress(0, ExportState::Queued);
        let thread = std::thread::Builder::new()
            .name(format!("genzo-export-{job}"))
            .spawn(move || task.run())
            .map_err(|e| EngineError::Io(e.to_string()))?;
        if let Some(r) = self.exports.jobs.lock().expect("exports").get_mut(&job) {
            r.thread = Some(thread);
        }
        Ok(job)
    }

    /// Cancels export `job` after the region it is rendering; its file is
    /// not written. A job that has ended is no error.
    pub fn cancel_export(&self, job: u64) -> Result<(), EngineError> {
        let known = match self.exports.jobs.lock().expect("exports").get(&job) {
            Some(r) => {
                r.cancelled.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        };
        if known {
            self.gpu()?.cancel_export(job)?;
        }
        Ok(())
    }

    /// Cancels every export and waits until each has ended, its temporary
    /// file deleted: the app is quitting (OUT-01). The shell calls it, as
    /// the process ends without dropping the engine.
    pub fn stop_exports(&self) {
        self.exports.stop();
    }

    /// Waits until every export has ended: for the tests and the cli.
    #[doc(hidden)]
    pub fn wait_exports_idle(&self) {
        self.exports.wait_idle();
    }
}

/// One export's thread.
struct Task {
    job: u64,
    photo: u64,
    path: String,
    destination: PathBuf,
    /// The photo's file, for its metadata.
    source: Option<PathBuf>,
    quality: u8,
    export: ExportJob,
    cancelled: Arc<AtomicBool>,
    events: Option<EventSink>,
    switching: Arc<Mutex<()>>,
    exports: Arc<Exports>,
}

impl Task {
    fn progress(&self, rows_done: u32, state: ExportState) {
        if let Some(sink) = &self.events {
            sink.send(Event::ExportProgress {
                job: self.job,
                photo: self.photo,
                path: self.path.clone(),
                rows_done,
                rows_total: self.export.size.1,
                state,
            });
        }
    }

    fn run(self) {
        let mut notes = Vec::new();
        let exif = self.metadata(&mut notes);
        let settings = genzo_export::jpeg::Settings {
            quality: self.quality,
            exif,
            icc: genzo_color::output::srgb_icc().to_vec(),
        };
        let mut failure: Option<String> = None;
        let mut running = false;
        let mut reported = 0u32;
        let mut reducer = self.export.resized().then(|| {
            genzo_export::resize::Reducer::new(
                self.export.source,
                self.export.size,
                self.export.stage9,
            )
        });
        let result = genzo_export::jpeg::write_file(
            &self.destination,
            self.export.size,
            &settings,
            || loop {
                if self.cancelled.load(Ordering::SeqCst) {
                    return None;
                }
                let piece = match self.export.strips.recv() {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => {
                        failure = Some(e.to_string());
                        return None;
                    }
                    Err(_) => return None,
                };
                // Not during a photo switch: its first pixels come first.
                drop(self.switching.lock());
                if !running {
                    running = true;
                    self.progress(0, ExportState::Running);
                }
                match (piece, reducer.as_mut()) {
                    (ExportPiece::Rows(strip), None) => {
                        return Some(genzo_export::jpeg::Strip {
                            y: strip.y,
                            height: strip.height,
                            rgba: strip.rgba,
                        });
                    }
                    (ExportPiece::Region { rect, rgb }, Some(r)) => {
                        r.add(rect, &rgb);
                        // The output rows the row of regions finished.
                        if rect.x + rect.width == self.export.source.0 {
                            let (y, rgba, height) = r.finish(rect.y + rect.height);
                            if height > 0 {
                                return Some(genzo_export::jpeg::Strip { y, height, rgba });
                            }
                        }
                    }
                    _ => {
                        failure = Some("the export's pieces were not of its kind".into());
                        return None;
                    }
                }
            },
            |rows| {
                // Not during a photo switch, between block rows too: its
                // first pixels come first.
                drop(self.switching.lock());
                // An event a percent, not a block row.
                let total = self.export.size.1.max(1);
                if u64::from(rows) * 100 / u64::from(total)
                    != u64::from(reported) * 100 / u64::from(total)
                    || rows == total
                {
                    reported = rows;
                    self.progress(rows, ExportState::Running);
                }
            },
        );
        let state = match result {
            Ok(()) => ExportState::Finished { notes },
            Err(_) if self.cancelled.load(Ordering::SeqCst) => ExportState::Cancelled,
            Err(genzo_export::jpeg::JpegError::Stopped) => match failure {
                Some(reason) => ExportState::Failed { reason },
                None => ExportState::Cancelled,
            },
            Err(e) => ExportState::Failed {
                reason: e.to_string(),
            },
        };
        let rows = match state {
            ExportState::Finished { .. } => self.export.size.1,
            _ => 0,
        };
        self.progress(rows, state);
        self.exports.jobs.lock().expect("exports").remove(&self.job);
        self.exports.ended.notify_all();
    }

    /// The photo's EXIF rewritten for the export's size, and notes on what
    /// did not go out.
    fn metadata(&self, notes: &mut Vec<String>) -> Option<Vec<u8>> {
        let read = self
            .source
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|data| genzo_imgio::metadata::read(&data));
        let Some(meta) = read else {
            if self.source.is_some() {
                notes.push("The photo's metadata could not be read; the export has none".into());
            }
            return None;
        };
        let software = format!("Genzo {}", crate::ENGINE_VERSION);
        let written = genzo_imgio::metadata::write(&meta, self.export.size, &software);
        if written.dropped.contains(&37500) {
            notes.push("The camera's maker notes were too large to keep".into());
        } else if !written.dropped.is_empty() {
            notes.push("Some metadata was too large to keep".into());
        }
        Some(written.app1)
    }
}
