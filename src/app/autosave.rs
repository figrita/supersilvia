// SPDX-License-Identifier: AGPL-3.0-or-later

//! Autosave: while the document has unsaved edits, the project is written into its own
//! `.autosave/` ([`crate::project::AUTOSAVE`]) at most once every [`INTERVAL_S`], and a real
//! save, or a Discard, deletes it.
//!
//! **Nothing of it runs on the frame or the synth thread but a pointer copy.** The frame
//! records the newest unsaved document — the graph's `Arc` and a copy of the project's few
//! fields — when its edit serial moves, and a thread named `autosave` does the writing:
//! serializing, the files, their `fsync`s and the renames of [`Project::save`]. Two writes
//! never overlap, because one lock is held across each; the frame never takes that lock
//! except where a save or an open already waits on the disk.
//!
//! **A lost device writes at once**: [`device_lost`] is the same write, called from whichever
//! thread wgpu reports the loss on, with the newest document the frame recorded rather than
//! the one the last timed write took.

use crate::graph::Graph;
use crate::project::{Project, remove_dir};
use crate::workspace::LoadError;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The longest an unsaved edit waits to be written, in seconds. Half a minute of patching is
/// the most a crash can cost, and a write — a few kilobytes of JSON, synced — twice a minute
/// is nothing to a disk.
pub const INTERVAL_S: f64 = 30.0;

/// The editor's side of the autosave.
#[derive(Default)]
pub(super) struct Autosave {
    shared: Arc<Shared>,
    /// The write in flight.
    job: Option<std::thread::JoinHandle<()>>,
    /// The edit serial [`Shared::latest`] was recorded at, or `None` for a clean document.
    recorded: Option<u64>,
    /// `latest` has moved since the last write started.
    changed: bool,
    /// When the last write started, on the frame's clock.
    last: Option<f64>,
    /// The project on screen has a folder on disk. A project that was never saved has
    /// nowhere to autosave into, and is not autosaved: that is a test's scratch project.
    enabled: bool,
}

/// What the frame hands the writer, and the writer itself.
#[derive(Default)]
pub struct Shared {
    /// The newest unsaved document and the project it is in, or `None` while the document
    /// is clean.
    latest: Mutex<Option<(Arc<Graph>, Project)>>,
    /// Held across every write, so two never overlap.
    writer: Mutex<Writer>,
}

#[derive(Default)]
struct Writer {
    /// The project `.autosave/` is written through: see [`Project::autosave`].
    shadow: Option<Project>,
    /// The folder the last write went into, which a clean document deletes.
    dir: Option<PathBuf>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Write the newest document recorded, or delete the last write where the document is
    /// clean again. Waits for a write in flight.
    pub fn write_now(&self) -> Result<(), LoadError> {
        let mut writer = lock(&self.writer);
        let latest = lock(&self.latest).clone();
        if let Some((graph, project)) = latest {
            writer.dir = Some(project.autosave_dir());
            return project.autosave(&graph, &mut writer.shadow);
        }
        writer.shadow = None;
        match writer.dir.take() {
            Some(dir) => remove_dir(&dir).map_err(LoadError::Io),
            None => Ok(()),
        }
    }
}

/// What the person is told when the device is lost, once the newest unsaved document has
/// been written: the same line on stderr and in the log. `why` is wgpu's reason.
pub fn device_lost(shared: &Shared, why: &str) -> String {
    match shared.write_now() {
        Ok(()) => {
            format!("The GPU stopped responding. Your work was saved; restart supersilvia. ({why})")
        }
        Err(e) => format!(
            "The GPU stopped responding, and your unsaved work could not be saved: {e}. \
             Restart supersilvia. ({why})"
        ),
    }
}

impl Autosave {
    /// The half a lost device writes through.
    pub(super) fn shared(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }

    /// Once a frame: record the document where it moved, and start a write where one is due.
    pub(super) fn update(
        &mut self,
        now: f64,
        dirty: bool,
        edit: u64,
        graph: &Arc<Graph>,
        project: &Project,
    ) {
        if !self.enabled {
            return;
        }
        if self
            .job
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            self.wait();
        }
        let want = dirty.then_some(edit);
        if want != self.recorded {
            *lock(&self.shared.latest) = dirty.then(|| (Arc::clone(graph), project.clone()));
            self.recorded = want;
            self.changed = true;
        }
        let due = self.last.is_none_or(|last| now - last >= INTERVAL_S);
        if !self.changed || self.job.is_some() || !due {
            return;
        }
        self.changed = false;
        self.last = Some(now);
        let shared = Arc::clone(&self.shared);
        match std::thread::Builder::new()
            .name("autosave".into())
            .spawn(move || {
                if let Err(e) = shared.write_now() {
                    log::warn!("autosave failed: {e}");
                }
            }) {
            Ok(job) => self.job = Some(job),
            Err(e) => log::warn!("could not start the autosave: {e}"),
        }
    }

    /// Wait for the write in flight, if there is one.
    pub(super) fn wait(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.join();
        }
    }

    /// Start over on this project, leaving its folder as it is: a project opened, made or
    /// recovered.
    pub(super) fn reset(&mut self, project: &Project) {
        self.wait();
        *lock(&self.shared.latest) = None;
        *lock(&self.shared.writer) = Writer::default();
        self.recorded = None;
        self.changed = false;
        self.last = None;
        self.enabled = Project::is_project(project.root());
    }

    /// Delete the autosave and write no more of this document: its edits were discarded,
    /// and it stays open with them for the frames before the app quits or opens another
    /// project, so one more write would bring them back as a recovery. Opening or making a
    /// project starts the autosave again ([`Autosave::reset`]).
    pub(super) fn forget(&mut self, project: &Project) {
        self.clear(project);
        self.enabled = false;
    }

    /// Delete the autosave: the document was saved, or its edits discarded. The folder the
    /// last write went into goes too, which is a different one after a Save as.
    pub(super) fn clear(&mut self, project: &Project) {
        self.wait();
        let last = lock(&self.shared.writer).dir.take();
        for dir in last.into_iter().chain([project.autosave_dir()]) {
            if let Err(e) = remove_dir(&dir) {
                log::warn!("could not remove {}: {e}", dir.display());
            }
        }
        self.reset(project);
    }
}
