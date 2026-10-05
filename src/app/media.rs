// SPDX-License-Identifier: AGPL-3.0-or-later

//! The media around the document: the asset list, the pictures on the cards, the file
//! dialog in flight, the status line, and the offline render's bookkeeping.
//!
//! [`Media`] reads the disk and a worker's channel and never the graph's state beyond what
//! it is handed: which workspaces there are, which project the files are in, which snapshot
//! a picture or an outcome arrived on. Every one of its reads happens off the frame — a
//! dialog and a poster each on a thread of their own, a thumbnail read back by the renderer
//! a frame or two after a save asked — so nothing here costs the show a frame.

use super::files::FileAsk;
use super::render::Outcome;
use crate::graph::{Graph, NodeId, WorkspaceId};
use crate::project::{AssetInfo, Project};
use crate::synth::Snapshot;
use eframe::egui;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Read a PNG off disk and hand it to egui as a texture. `None` where there is no file, or
/// it is not a PNG: a card with no picture draws its placeholder.
fn load_picture(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let image = crate::video::png::read(path).ok()?;
    let size = [image.width as usize, image.height as usize];
    Some(ctx.load_texture(
        path.display().to_string(),
        egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba),
        egui::TextureOptions::LINEAR,
    ))
}

#[derive(Default)]
pub(super) struct Media {
    /// Every file in `assets/`, as the canvas's file picker offers them.
    ///
    /// Cached rather than read where it is wanted: the canvas asks for it every frame and
    /// this is a directory listing. What keeps it true is that the folder has exactly four
    /// ways to change — an import, a removal, a workspace arriving with media, and the
    /// project being replaced — and each one refreshes it.
    assets: Vec<AssetInfo>,
    /// The poster being decoded, and the asset it is for.
    ///
    /// One at a time. A poster is a GStreamer pipeline decoding a real file, and a folder of
    /// twenty clips is not a reason to open twenty of them at once while the show is running.
    poster: Option<(String, Receiver<Result<(), String>>)>,
    /// Assets no poster could be made for — a font, a text file, a clip whose codec this
    /// machine cannot decode. Remembered so it is not attempted again every frame.
    posterless: HashSet<String>,
    /// Each workspace's picture as a texture, loaded from disk once and forgotten when a
    /// save rewrites it. `None` where there is no file: the card draws its placeholder.
    thumbnails: HashMap<WorkspaceId, Option<egui::TextureHandle>>,
    /// The same for an asset that has a picture of its own, keyed by its reference.
    asset_thumbnails: HashMap<String, Option<egui::TextureHandle>>,
    /// Workspaces whose picture a save asked for, and the Output that will supply it.
    ///
    /// A save marks them and moves on; the renderer reads each one back a frame or two
    /// later, the way a tap is collected. **Nothing on the frame thread waits on the GPU**,
    /// so a save never costs a frame.
    thumbnails_pending: BTreeMap<WorkspaceId, NodeId>,
    /// Frames left before a picture that never landed is given up on. A backstop: an Output
    /// that stops rendering between the request and the answer would otherwise leave the
    /// status line saying *pending* for the rest of the session.
    thumbnails_deadline: u32,
    /// An open file dialog, waiting on its own thread. `Some` also means "a dialog is
    /// already up", which is what stops a second one being opened behind it.
    pending_file: Option<(FileAsk, Receiver<Option<PathBuf>>)>,
    /// The result of the last save or load, for the status line.
    file_status: String,
    /// The file the status line is about, where it offers to show it: a Snap's picture, a
    /// render's film or folder. Cleared by every other line.
    status_shows: Option<PathBuf>,
    /// Whether the status line says something failed, which the Status box draws in the
    /// accent. Cleared by every other line.
    status_failed: bool,
    /// Failures said since `App` last heard them, oldest first: each one toasts and joins the
    /// problems list. See [`App::fail`](super::App::fail).
    unheard: Vec<String>,
    /// A render has been asked for and its outcome has not come back. The document is
    /// closed on the frame the button is pressed rather than a tick later. The render
    /// itself is the synth's — see [`crate::synth::offline`].
    render_asked: bool,
    /// How many renders have been asked for. An outcome is this render's only if it carries
    /// this number.
    render_seq: u64,
    /// Where the render in progress is writing, and how the last one ended.
    render_destination: Option<PathBuf>,
    render_outcome: Option<Outcome>,
    /// The live recordings asked for and how each Output's last one ended. The recording
    /// itself is the synth's — see [`crate::synth::record`].
    pub(super) records: super::record::Records,
}

impl Media {
    /// What the status line says.
    pub(super) fn status(&self) -> &str {
        &self.file_status
    }

    pub(super) fn set_status(&mut self, status: impl Into<String>) {
        self.file_status = status.into();
        self.status_shows = None;
        self.status_failed = false;
    }

    /// A line saying something failed: the status line says it, and it waits for `App` to
    /// toast it and list it. Every failure that reaches the status line comes through here.
    pub(super) fn fail(&mut self, status: impl Into<String>) {
        let status = status.into();
        self.set_status(status.clone());
        self.status_failed = true;
        self.unheard.push(status);
    }

    /// Whether the status line says something failed.
    pub(super) fn status_failed(&self) -> bool {
        self.status_failed
    }

    /// The failures said since the last call, oldest first.
    pub(super) fn take_unheard(&mut self) -> Vec<String> {
        std::mem::take(&mut self.unheard)
    }

    /// A line about a file that was just written, which the status line offers to show.
    pub(super) fn set_status_showing(&mut self, status: impl Into<String>, file: PathBuf) {
        self.file_status = status.into();
        self.status_shows = Some(file);
        self.status_failed = false;
    }

    /// The file the status line offers to show, if it offers one.
    pub(super) fn status_shows(&self) -> Option<&std::path::Path> {
        self.status_shows.as_deref()
    }

    /// Every file in `assets/`.
    pub(super) fn assets(&self) -> &[AssetInfo] {
        &self.assets
    }

    /// Read `assets/` again: one of the four ways it changes has just happened.
    pub(super) fn refresh_assets(&mut self, project: &Project) {
        self.assets = project.assets();
    }

    /// Each asset's picture, where one has been made.
    pub(super) fn asset_thumbnails(&self) -> &HashMap<String, Option<egui::TextureHandle>> {
        &self.asset_thumbnails
    }

    /// Each workspace's picture, where one has been read.
    pub(super) fn thumbnails(&self) -> &HashMap<WorkspaceId, Option<egui::TextureHandle>> {
        &self.thumbnails
    }

    /// The project was replaced: a project carries its own pictures and its own media, so
    /// nothing from the last one may survive.
    pub(super) fn forget_project(&mut self, project: &Project) {
        self.thumbnails.clear();
        self.asset_thumbnails.clear();
        self.posterless.clear();
        self.poster = None;
        self.thumbnails_pending.clear();
        self.records.forget_project();
        self.refresh_assets(project);
    }

    /// An asset left the project, and its picture with it.
    pub(super) fn forget_asset(&mut self, reference: &str, project: &Project) {
        self.asset_thumbnails.remove(reference);
        self.posterless.remove(reference);
        self.refresh_assets(project);
    }

    /// Read at most one picture off disk per frame, and turn it into a texture.
    ///
    /// One, because reading a PNG is a GStreamer pipeline: doing every card's on the frame
    /// a project tab first appears would be the one thing the UI is not allowed to do, which
    /// is cost the render a frame. A card with nothing yet draws its placeholder, and has
    /// its picture the frame after.
    pub(super) fn load_one_picture(
        &mut self,
        ctx: &egui::Context,
        graph: &Graph,
        project: &Project,
    ) {
        self.collect_poster(ctx, project);
        let wanted = graph
            .workspaces()
            .iter()
            .map(|w| w.id)
            .find(|id| !self.thumbnails.contains_key(id));
        if let Some(id) = wanted {
            let texture = project
                .thumbnail_path(id)
                .and_then(|path| load_picture(ctx, &path));
            self.thumbnails.insert(id, texture);
            return;
        }
        let Some(asset) = self
            .assets
            .iter()
            .find(|a| !self.asset_thumbnails.contains_key(&a.reference))
            .cloned()
        else {
            return;
        };
        let path = project.resolve(&asset.reference);
        // A PNG is its own picture and needs no decode.
        if asset
            .name
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("png"))
        {
            let texture = load_picture(ctx, &path);
            self.asset_thumbnails.insert(asset.reference, texture);
            return;
        }
        // Everything else gets a poster: one frame out of it, written into `cache/` where
        // the transcode and the soundtrack already live, so it is made once per file rather
        // than once per session.
        let Some(poster) = crate::video::clip::poster_path(&project.cache_dir(), &path) else {
            self.asset_thumbnails.insert(asset.reference, None);
            return;
        };
        if poster.is_file() {
            let texture = load_picture(ctx, &poster);
            self.asset_thumbnails.insert(asset.reference, texture);
            return;
        }
        if self.posterless.contains(&asset.reference) {
            self.asset_thumbnails.insert(asset.reference, None);
            return;
        }
        // Nothing is written into `asset_thumbnails` yet: the entry is what stops the search
        // coming back here, and the answer is not known until the worker says so.
        if self.poster.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::video::clip::write_poster(&path, &poster));
            });
            self.poster = Some((asset.reference, rx));
        }
    }

    /// Take a finished poster, if one finished. Never waits.
    fn collect_poster(&mut self, ctx: &egui::Context, project: &Project) {
        let Some((reference, rx)) = self.poster.take() else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                self.poster = Some((reference, rx));
                return;
            }
            // The worker died without answering, which is a failure like any other.
            Err(TryRecvError::Disconnected) => Err("no answer".to_string()),
        };
        match result {
            Ok(()) => {
                let path = crate::video::clip::poster_path(
                    &project.cache_dir(),
                    &project.resolve(&reference),
                );
                let texture = path.as_deref().and_then(|p| load_picture(ctx, p));
                self.asset_thumbnails.insert(reference, texture);
            }
            Err(e) => {
                log::debug!("no poster for {reference}: {e}");
                self.posterless.insert(reference);
            }
        }
    }

    /// Whether a file dialog is already up.
    pub(super) fn file_busy(&self) -> bool {
        self.pending_file.is_some()
    }

    /// Keep the answer channel of a dialog just opened.
    pub(super) fn await_file(&mut self, ask: FileAsk, rx: Receiver<Option<PathBuf>>) {
        self.pending_file = Some((ask, rx));
    }

    /// A finished file dialog's question and the path it answered with, if one finished
    /// with a pick. A cancel, and a thread that died without answering, are both nothing.
    pub(super) fn poll_file(&mut self) -> Option<(FileAsk, PathBuf)> {
        let (ask, rx) = self.pending_file.take()?;
        match rx.try_recv() {
            Ok(chosen) => chosen.map(|path| (ask, path)),
            Err(TryRecvError::Empty) => {
                self.pending_file = Some((ask, rx));
                None
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Wait for a picture of each workspace from the Output standing for it: a save asked.
    pub(super) fn await_thumbnails(&mut self, pending: BTreeMap<WorkspaceId, NodeId>) {
        self.thumbnails_pending = pending;
        // Two seconds at sixty frames. A picture takes one or two; anything that has not
        // arrived by then is not coming.
        self.thumbnails_deadline = 120;
    }

    /// What a save says while its pictures are still on their way, and after.
    pub(super) fn saved_status(&self, project: &Project) -> String {
        if self.thumbnails_pending.is_empty() {
            Self::saved_line(project)
        } else {
            format!("{}, thumbnails pending", Self::saved_line(project))
        }
    }

    fn saved_line(project: &Project) -> String {
        format!("saved {}", project.name())
    }

    /// Write the pictures whose readback finished, and forget the textures they replace.
    ///
    /// Collected once a frame beside the taps, and for the same reason: the readback
    /// happens where the completion serial is already read, so nothing here waits.
    pub(super) fn collect_thumbnails(&mut self, snapshot: &Snapshot, project: &Project) {
        if self.thumbnails_pending.is_empty() {
            return;
        }
        if !snapshot.render.has_gpu {
            self.thumbnails_pending.clear();
            return;
        }
        let size = crate::render::readback::THUMBNAIL;
        for (_, (node, rgba)) in snapshot.events.thumbnails.unseen("thumbnails") {
            let wanted: Vec<WorkspaceId> = self
                .thumbnails_pending
                .iter()
                .filter(|(_, n)| *n == node)
                .map(|(w, _)| *w)
                .collect();
            for workspace in wanted {
                self.thumbnails_pending.remove(&workspace);
                let Some(path) = project.thumbnail_path(workspace) else {
                    continue;
                };
                let image = crate::video::png::Image {
                    width: size.0,
                    height: size.1,
                    rgba: rgba.clone(),
                };
                match crate::video::png::write(&path, &image) {
                    // The card reloads it, because the file it had is not the file there is.
                    Ok(()) => {
                        self.thumbnails.remove(&workspace);
                    }
                    Err(e) => log::warn!("could not write {}: {e}", path.display()),
                }
            }
        }
        self.thumbnails_deadline = self.thumbnails_deadline.saturating_sub(1);
        if self.thumbnails_deadline == 0 && !self.thumbnails_pending.is_empty() {
            log::warn!(
                "{} thumbnail(s) never arrived",
                self.thumbnails_pending.len()
            );
            self.thumbnails_pending.clear();
        }
        // The save is finished when the last picture is written, and says so.
        if self.thumbnails_pending.is_empty() && self.file_status.ends_with(", thumbnails pending")
        {
            self.set_status(Self::saved_line(project));
        }
    }

    /// A render has been asked for, writing to `destination`: the document closes now, and
    /// the number that comes back is the one its outcome must carry.
    pub(super) fn begin_render(&mut self, destination: PathBuf) -> u64 {
        self.render_destination = Some(destination);
        self.render_outcome = None;
        // This render's own number, so an outcome already in flight from the *last* one is
        // not mistaken for this one's and does not end it on its first frame.
        self.render_seq += 1;
        // Closed here rather than when the snapshot first says so: the very next command
        // must already be refused, and the snapshot is a tick away.
        self.render_asked = true;
        self.render_seq
    }

    /// Whether the document is closed. True from the moment a render is asked for, so the
    /// command after it is already refused, and false again when the outcome lands.
    pub(super) fn rendering(&self, snapshot: &Snapshot) -> bool {
        self.render_asked || snapshot.offline.running.is_some()
    }

    /// Where the render in progress is writing.
    pub(super) fn render_destination(&self) -> Option<&std::path::Path> {
        self.render_destination.as_deref()
    }

    /// How the last render ended, until the next one starts.
    pub(super) fn render_outcome(&self) -> Option<&Outcome> {
        self.render_outcome.as_ref()
    }

    /// Notice a render that ended: the outcome is in the snapshot exactly once, so it is
    /// taken here and turned into the line the file status shows. True on the frame it is
    /// taken.
    pub(super) fn observe_render(&mut self, snapshot: &Snapshot) -> bool {
        let Some((seq, outcome)) = snapshot.offline.outcome.clone() else {
            return false;
        };
        // Only this render's. A snapshot taken before the synth saw `StartRender` still
        // carries the previous outcome, and believing it would close the document again on
        // the frame after it opened.
        if seq != self.render_seq || self.render_outcome.as_ref() == Some(&outcome) {
            return false;
        }
        self.render_asked = false;
        match &outcome {
            Outcome::Done {
                frames,
                destination,
            } => self.set_status_showing(
                format!("rendered {frames} frames to {}", destination.display()),
                destination.clone(),
            ),
            Outcome::Canceled => self.set_status("render canceled"),
            Outcome::Failed(e) => self.fail(format!("render failed: {e}")),
        }
        self.render_outcome = Some(outcome);
        true
    }
}
