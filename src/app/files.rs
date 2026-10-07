// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk: dialogs, projects, assets, import and export.
//!
//! A dialog runs on its own thread and reports back through `pending_file`, because a modal
//! that blocks the frame also freezes the projector.

use super::{App, DROP_STAGGER, FIRST_DROP};
use crate::command::{Clip, Command};
use crate::graph::{Graph, NodeId};
use crate::platform::files::Pick;
use crate::project::{self, Active, Project};
use crate::ui::project::ProjectState;
use crate::ui::tabs::TabState;
use eframe::egui;

/// What a file dialog was opened for. The dialog runs on its own thread and reports back
/// here, because a modal that blocks the frame also freezes the projector.
pub(super) enum FileAsk {
    /// A project folder to open.
    OpenProject,
    /// A folder to make a project in, elsewhere than the projects folder: New project's
    /// Choose location…. It must be empty or not exist.
    NewProject,
    /// A folder to copy this project to.
    SaveAs,
    /// An `Asset` option on a node — a video's file. It carries what the option accepts,
    /// read off the node's definition where the request was made, so the dialog filters to
    /// exactly what the picker beside it offers.
    Option {
        node: NodeId,
        key: &'static str,
        accepts: crate::nodes::Accepts,
    },
    /// A folder to write one workspace, its picture and its media into. A folder that is
    /// already a project takes them into its own `workspaces/` and `assets/`.
    ExportWorkspace(crate::graph::WorkspaceId),
    /// A `.ssw` to bring into this project.
    ImportWorkspace,
    /// Any file, to copy into `assets/`.
    ImportAsset,
    /// A folder to copy one asset out to.
    ExportAsset(String),
    /// Another projects folder, from the Preferences window.
    ProjectsFolder,
    /// A folder for every project's recordings, from the Preferences window.
    RecordingsFolder,
    /// A clip or a sound for the Main Input panel. Not an option on a node, so it does not
    /// go through `SetOption` and does not enter the undo history — which source the rig is
    /// pointed at is the mixer's kind of state, not an edit.
    MainInput { audio: bool },
}

impl FileAsk {
    /// A project is a folder, so most of these pick one. The three that pick a file each
    /// filter to what they can actually use.
    fn pick(&self) -> Pick {
        match self {
            Self::OpenProject
            | Self::NewProject
            | Self::SaveAs
            | Self::ExportWorkspace(_)
            | Self::ExportAsset(_)
            | Self::ProjectsFolder
            | Self::RecordingsFolder => Pick::Folder,
            Self::Option { accepts, .. } => Pick::File(accepts.label, accepts.extensions),
            Self::MainInput { audio } => {
                let accepts = if *audio {
                    crate::nodes::Accepts::AUDIO
                } else {
                    crate::nodes::Accepts::VIDEO
                };
                Pick::File(accepts.label, accepts.extensions)
            }
            Self::ImportWorkspace => Pick::File("Workspace", &[crate::workspace::EXTENSION]),
            Self::ImportAsset => Pick::File("Any file", &[]),
        }
    }
}

/// A path's file name, for a status line that has no room for the rest.
fn name_of(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

/// Which node a dropped file becomes.
///
/// silvia's Image/GIF node is mostly a dashed panel saying *drop image here*, so a picture
/// reaches a patch in one gesture and without knowing the node's name. Here the gesture is
/// the window's rather than the node's, and this is the part that makes it land somewhere
/// that can show it.
///
/// **A picture is whatever `Accepts::IMAGE` lists**, the Image/GIF node's own file button's
/// list, GIFs included: an animated GIF is a picture that moves, and the Image/GIF node plays
/// it by its own delays. `Accepts::VIDEO` holds no picture, so the two lists never both claim
/// a file. Everything else makes a `video`.
pub fn node_for_drop(path: &std::path::Path) -> &'static crate::nodes::NodeDef {
    let picture = path
        .file_name()
        .is_some_and(|n| crate::nodes::Accepts::IMAGE.matches(&n.to_string_lossy()));
    if picture {
        &crate::nodes::imagegif::DEF
    } else {
        &crate::nodes::video::DEF
    }
}

/// What a dropped file becomes, by its name and the tab it is dropped on.
#[derive(Debug, Clone, Copy)]
enum Lands {
    /// A `.ssw`: a workspace of its own, imported, wherever it is dropped.
    Workspace,
    /// A node of this kind on the workspace showing, playing the file.
    Node(&'static crate::nodes::NodeDef),
    /// A file in `assets/` and nothing else: the project tab is showing.
    Asset,
    /// The window says a file is coming and not which, so what it makes is not known yet.
    Unknown,
}

/// What `path` makes, dropped with a workspace showing or with the project tab.
fn lands(path: Option<&std::path::Path>, on_canvas: bool) -> Lands {
    match path {
        None => Lands::Unknown,
        Some(p)
            if p.extension()
                .is_some_and(|e| e == crate::workspace::EXTENSION) =>
        {
            Lands::Workspace
        }
        Some(p) if on_canvas => Lands::Node(node_for_drop(p)),
        Some(_) => Lands::Asset,
    }
}

/// The one line a held drop says about itself: "new Video node on Tunnel", "tunnel.ssw:
/// import as workspace", "gumbasia.webm: add to assets".
///
/// Several files say each thing they will make, in one line, counted where there are more of
/// one: "2 new Video nodes, new Image/GIF node on Tunnel · tunnel.ssw: import as workspace".
fn drop_line(files: &[(Lands, String)], workspace: Option<&str>) -> String {
    let named = |what: Lands, one: &str, many: &str| -> Option<String> {
        let of: Vec<&str> = files
            .iter()
            .filter(|(l, _)| std::mem::discriminant(l) == std::mem::discriminant(&what))
            .map(|(_, n)| n.as_str())
            .collect();
        match of.as_slice() {
            [] => None,
            [name] if !name.is_empty() => Some(format!("{name}: {one}")),
            [_] => Some(format!("a file: {one}")),
            _ => Some(format!("{} files: {many}", of.len())),
        }
    };
    let mut parts = Vec::new();
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for (l, _) in files {
        if let Lands::Node(def) = l {
            let label = def.label;
            match kinds.iter_mut().find(|(k, _)| *k == label) {
                Some((_, n)) => *n += 1,
                None => kinds.push((label, 1)),
            }
        }
    }
    if !kinds.is_empty() {
        let nodes: Vec<String> = kinds
            .iter()
            .map(|(label, n)| {
                if *n == 1 {
                    format!("new {label} node")
                } else {
                    format!("{n} new {label} nodes")
                }
            })
            .collect();
        parts.push(format!(
            "{} on {}",
            nodes.join(", "),
            workspace.unwrap_or("this workspace")
        ));
    }
    parts.extend(named(
        Lands::Workspace,
        "import as workspace",
        "import as workspaces",
    ));
    parts.extend(named(Lands::Asset, "add to assets", "add to assets"));
    parts.extend(named(Lands::Unknown, "import", "import"));
    parts.join(" · ")
}

/// A clip's media, made this project's.
///
/// A clip copied in another project names its files by that project's `assets/…`, which
/// here would name a file this project does not have, or a different file under the same
/// name. So each one is resolved in the folder the clip came from and copied in through
/// [`Project::import_asset`], as an import copies a workspace's — the same file already here
/// is the asset it already is — and the option takes the reference that returns. A file that
/// is gone is left pointing at where it was looked for, and said.
///
/// `(None, None)` where there is nothing to do: a clip of this project's, or one naming no
/// file. Otherwise the clip to plant, and the line saying what came and what did not, with
/// whether a file did not.
pub(super) fn bring_media(
    clip: &Clip,
    project: &Project,
) -> (Option<Clip>, Option<(String, bool)>) {
    let Some(from) = clip.from.as_deref().filter(|root| !project.is_at(root)) else {
        return (None, None);
    };
    let source = Project::new(from.to_path_buf());
    let mut brought = clip.clone();
    let (mut copied, mut missing) = (Vec::new(), Vec::new());
    for node in &mut brought.nodes {
        for option in node.def.options {
            if !option.is_asset() {
                continue;
            }
            let Some(value) = node
                .options
                .get_mut(option.key)
                .filter(|v| v.starts_with(&format!("{}/", project::ASSETS)))
            else {
                continue;
            };
            let path = source.resolve(value);
            match project.import_asset(&path) {
                Ok(reference) => {
                    copied.push(reference.clone());
                    *value = reference;
                }
                Err(e) => {
                    log::warn!("a paste could not bring {}: {e}", path.display());
                    missing.push(value.clone());
                    *value = path.to_string_lossy().into_owned();
                }
            }
        }
    }
    if copied.is_empty() && missing.is_empty() {
        return (None, None);
    }
    copied.sort();
    copied.dedup();
    let came = match copied.as_slice() {
        [] => None,
        [one] => Some(format!("pasted with {one}")),
        many => Some(format!(
            "pasted with {} files into {}/",
            many.len(),
            project::ASSETS
        )),
    };
    let lost = (!missing.is_empty()).then(|| format!("could not find {}", missing.join(", ")));
    let failed = lost.is_some();
    let said: Vec<String> = came.into_iter().chain(lost).collect();
    (Some(brought), Some((said.join("; "), failed)))
}

/// What the status line says when there is nowhere to keep a project.
fn no_projects_dir() -> String {
    format!(
        "nowhere to keep projects: {}",
        crate::platform::dirs::DOCUMENTS_UNSET
    )
}

/// Where a dialog opens.
///
/// **The projects folder for Open and for a new project's folder**, since that is where
/// projects are kept — or the nearest folder above it that is there, before the first one is
/// made. The project's own folder for a node's file and an import, since that is what they
/// fill; its parent for Save as and the exports, since a copy of this project or a piece of it
/// goes beside it.
pub(super) fn start_dir(
    ask: &FileAsk,
    project: &std::path::Path,
    projects: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    match ask {
        FileAsk::OpenProject | FileAsk::NewProject | FileAsk::ProjectsFolder => projects
            .and_then(|dir| dir.ancestors().find(|p| p.is_dir()))
            .or(Some(project))
            .map(std::path::Path::to_path_buf),
        FileAsk::SaveAs | FileAsk::ExportWorkspace(_) | FileAsk::ExportAsset(_) => {
            project.parent().map(std::path::Path::to_path_buf)
        }
        FileAsk::Option { .. }
        | FileAsk::RecordingsFolder
        | FileAsk::MainInput { .. }
        | FileAsk::ImportWorkspace
        | FileAsk::ImportAsset => Some(project.to_path_buf()),
    }
}

impl App {
    /// Open a file dialog on its own thread.
    ///
    /// A dialog is a round trip to the desktop and then as long as the person takes. Doing
    /// that on the frame thread would stall every Output for as long as the
    /// dialog is up — so the dialog goes on a thread and the answer arrives through a
    /// channel, checked once a frame.
    pub(super) fn ask_for_file(&mut self, ask: FileAsk) {
        if self.media.file_busy() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let pick = ask.pick();
        let mut projects = self.projects_dir();
        // A projects folder that cannot be read is said, and the dialog opens wherever the
        // desktop opens it rather than inside a folder it cannot show.
        if matches!(ask, FileAsk::OpenProject | FileAsk::NewProject)
            && let Some(problem) = projects
                .as_deref()
                .and_then(|dir| project::projects_dir_problem(dir, false))
        {
            self.fail(problem);
            projects = None;
        }
        let start = match (&ask, &projects) {
            (FileAsk::OpenProject | FileAsk::NewProject, None) => None,
            // The folder chosen last, where it is there.
            (FileAsk::RecordingsFolder, _) => {
                let dir = self.recordings_dir();
                dir.is_dir()
                    .then_some(dir)
                    .or_else(|| start_dir(&ask, self.project.root(), None))
            }
            _ => start_dir(&ask, self.project.root(), projects.as_deref()),
        };
        match self.dialog_answer.clone() {
            Some(super::Canned(answer)) => {
                let _ = tx.send(answer);
            }
            None => {
                std::thread::spawn(move || {
                    let chosen = crate::platform::files::pick(pick, start.as_deref());
                    let _ = tx.send(chosen);
                });
            }
        }
        self.media.await_file(ask, rx);
    }

    /// Collect a finished file dialog, if one finished.
    pub(super) fn poll_file_dialog(&mut self) {
        let Some((ask, path)) = self.media.poll_file() else {
            return;
        };
        match ask {
            FileAsk::SaveAs => self.save_project_as(path),
            FileAsk::OpenProject => self.open_project(path),
            FileAsk::NewProject => self.new_project(path),
            FileAsk::Option { node, key, .. } => {
                // Every way a file reaches a node copies it into the project.
                let Some(value) = self.import_asset(&path) else {
                    return;
                };
                if let Err(err) = self.apply(Command::SetOption { node, key, value }) {
                    log::debug!("command refused: {err}");
                }
            }
            FileAsk::ExportWorkspace(id) => self.export_workspace(id, &path),
            FileAsk::ImportWorkspace => self.import_workspace_file(&path),
            FileAsk::ImportAsset => {
                if let Some(reference) = self.import_asset(&path) {
                    self.media.set_status(format!("imported {reference}"));
                }
            }
            FileAsk::ExportAsset(reference) => self.export_asset(&reference, &path),
            FileAsk::ProjectsFolder => {
                self.media
                    .set_status(format!("projects folder: {}", path.display()));
                self.prefs.set_projects_dir(path);
            }
            FileAsk::RecordingsFolder => {
                self.media
                    .set_status(format!("recordings folder: {}", path.display()));
                // The project's own `recordings/` chosen by hand is no folder of its own.
                let own = self.project.root().join(super::record::RECORDINGS);
                let chosen = (path != own).then_some(path);
                self.prefs.set_recordings_dir(chosen);
            }
            FileAsk::MainInput { audio } => {
                // Every way a file reaches the rig copies it into the project, exactly as a
                // node's file button does: a show travels with its media.
                let Some(asset) = self.import_asset(&path) else {
                    return;
                };
                let input = self.project.main_input_mut();
                if audio {
                    input.audio = crate::maininput::AudioSource::File { asset };
                } else {
                    input.video = crate::maininput::VideoSource::File { asset };
                }
            }
        }
    }

    /// A file dropped on the window becomes a node that plays it, where it was pointed.
    ///
    /// **Which node is the file's own answer**, [`node_for_drop`]: a picture, still or an
    /// animated GIF, makes an `imagegif` and everything else makes a `video`. **Where is the
    /// pointer's**: on the workspace showing, with the node's corner under the pointer — or at
    /// the view's centre where the pointer is over a panel rather than the canvas. On Wayland
    /// the drop arrives through [`Self::feed_file_drags`], with the drag's position as the
    /// pointer's. A second file lands a step down and across from the first.
    ///
    /// **On the project tab a file is an asset and nothing else**: there is no canvas there
    /// to put a node on, and the Assets list is what it joins.
    ///
    /// **A `.ssw` is a workspace and is imported instead**, wherever it is dropped, which is
    /// the same gesture the project tab's Import… is: a thing enters a project at the list it
    /// joins, and dropping it on the window is that import by another route.
    pub(super) fn take_dropped_files(&mut self, ui: &egui::Ui) {
        let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        // Read once, before an imported workspace takes the tab.
        let workspace = self.active_workspace();
        let mut at = self.drop_point(ui.ctx());
        for path in dropped {
            match lands(Some(&path), workspace.is_some()) {
                Lands::Workspace => self.import_workspace_file(&path),
                Lands::Asset | Lands::Unknown => {
                    if let Some(reference) = self.import_asset(&path) {
                        self.media.set_status(format!("imported {reference}"));
                    }
                }
                Lands::Node(def) => {
                    let Some(workspace) = workspace else {
                        continue;
                    };
                    let here = at;
                    at += DROP_STAGGER;
                    let Ok(()) = self.apply(Command::AddNode {
                        slug: def.slug,
                        at: here,
                        workspace,
                    }) else {
                        continue;
                    };
                    let Some(node) = self.doc.graph().iter().map(|(id, _)| id).last() else {
                        continue;
                    };
                    // A drop is an import: the file is copied into the project and the option
                    // holds the reference. A copy that failed leaves the node with no file
                    // rather than a path into somebody else's folder.
                    let Some(value) = self.import_asset(&path) else {
                        continue;
                    };
                    let _ = self.apply(Command::SetOption {
                        node,
                        key: "file",
                        value,
                    });
                }
            }
        }
    }

    /// Where on the canvas showing a drop lands, in world units: under the pointer while it is
    /// over the canvas, and the view's centre otherwise.
    fn drop_point(&self, ctx: &egui::Context) -> egui::Pos2 {
        let origin = self.canvas_origin();
        let canvas = egui::Rect::from_min_size(
            origin,
            egui::vec2(self.canvas.width(), self.canvas.height()),
        );
        if canvas.width() <= 0.0 || canvas.height() <= 0.0 {
            return FIRST_DROP;
        }
        let pointer = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| canvas.contains(*p));
        self.canvas_transform()
            .to_world(origin, pointer.unwrap_or_else(|| canvas.center()))
    }

    /// The drags winit does not hear — Wayland's, from [`crate::platform::filedrop`] — put
    /// into egui's input as its own hovered and dropped files, so
    /// [`Self::take_dropped_files`] and [`Self::show_drop_hint`] read one input wherever the
    /// drag came from. **Its position goes in as the pointer's**: a drag holds the pointer,
    /// so egui has no other word of where it is, and the drop lands under it.
    ///
    /// Run before each frame, from `raw_input_hook`. The files held are set again every
    /// frame until the drag leaves or lands, as winit's are.
    pub(super) fn feed_file_drags(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        use crate::platform::filedrop::Drag;
        // The surface's logical pixels are egui's points before the person's zoom.
        let zoom = ctx.zoom_factor();
        let point = |(x, y): (f32, f32)| egui::pos2(x / zoom, y / zoom);
        for drag in self.filedrop.take() {
            match drag {
                Drag::Entered { files, at } => {
                    self.held = Some(files);
                    raw.events.push(egui::Event::PointerMoved(point(at)));
                }
                Drag::Moved(at) => raw.events.push(egui::Event::PointerMoved(point(at))),
                Drag::Left => {
                    self.held = None;
                    raw.events.push(egui::Event::PointerGone);
                }
                Drag::Dropped { files, at } => {
                    self.held = None;
                    raw.events.push(egui::Event::PointerMoved(point(at)));
                    raw.dropped_files.extend(
                        files
                            .into_iter()
                            .map(|path| std::sync::Arc::new(DroppedPath(path)) as _),
                    );
                }
            }
        }
        if let Some(files) = &self.held {
            // A source that would not say which files still holds something: one with no
            // path, which the hint reads as a file it does not know.
            raw.hovered_files = if files.is_empty() {
                vec![egui::HoveredFile::default()]
            } else {
                files
                    .iter()
                    .map(|path| egui::HoveredFile {
                        path: Some(path.clone()),
                        ..Default::default()
                    })
                    .collect()
            };
        }
    }

    /// While files are held over the window: an outline round where they would land, and one
    /// line saying what the drop will make. Gone the frame they leave or land.
    ///
    /// `central` is the area between the panels, which is the project tab's when it is
    /// showing; a workspace's is its canvas.
    pub(super) fn show_drop_hint(&self, ctx: &egui::Context, central: egui::Rect) {
        let held: Vec<Option<std::path::PathBuf>> =
            ctx.input(|i| i.raw.hovered_files.iter().map(|f| f.path.clone()).collect());
        if held.is_empty() {
            return;
        }
        let workspace = self.active_workspace();
        let files: Vec<(Lands, String)> = held
            .iter()
            .map(|p| {
                (
                    lands(p.as_deref(), workspace.is_some()),
                    p.as_deref().map(name_of).unwrap_or_default(),
                )
            })
            .collect();
        let name = workspace
            .and_then(|id| self.doc.graph().workspace(id))
            .map(|w| w.name.as_str());
        let line = drop_line(&files, name);
        let rect = if workspace.is_some() && self.canvas.width() > 0.0 {
            egui::Rect::from_min_size(
                self.canvas_origin(),
                egui::vec2(self.canvas.width(), self.canvas.height()),
            )
        } else {
            central
        };
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop outline"),
        ))
        .rect_stroke(
            rect.shrink(2.0),
            crate::ui::theme::RADIUS_MD,
            egui::Stroke::new(2.0, self.theme.primary()),
            egui::StrokeKind::Inside,
        );
        // Fixed height and one line however many files: a long one is truncated, never wrapped.
        egui::Area::new(egui::Id::new("drop hint"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(rect.center_top() + egui::vec2(0.0, 12.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(self.theme.bg_secondary())
                    .stroke(egui::Stroke::new(1.0, self.theme.primary()))
                    .show(ui, |ui| {
                        ui.set_max_width((rect.width() - 48.0).max(120.0));
                        ui.add(egui::Label::new(line).truncate());
                    });
            });
    }

    /// Copy a file into the project's `assets/` and hand back the reference naming it.
    ///
    /// `None` where the copy failed, with the reason on the status line: an option left
    /// unset is a node that plays nothing, which is visible, and a reference to a file the
    /// project does not hold would not be.
    pub fn import_asset(&mut self, path: &std::path::Path) -> Option<String> {
        match self.project.import_asset(path) {
            Ok(reference) => {
                self.media.refresh_assets(&self.project);
                Some(reference)
            }
            Err(e) => {
                log::warn!("could not import {}: {e}", path.display());
                self.fail(format!("could not import {}: {e}", name_of(path)));
                None
            }
        }
    }

    /// The project on screen. `Save` writes here and there is nowhere else it could.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The projects folder: the one the preference chooses, or `supersilvia` in the documents
    /// folder. Where New project makes a project, Untitled is made and Open project starts.
    pub fn projects_dir(&self) -> Option<std::path::PathBuf> {
        self.prefs.get().projects_dir()
    }

    /// The view the active tab was left at, as the session records it.
    ///
    /// Called whenever the project is replaced: a project carries its own tabs and its own
    /// views, so nothing from the last one may survive into it.
    fn restore_view(&mut self) {
        self.tabs = TabState::default();
        self.project_tab = ProjectState::default();
        // A project carries its own pictures and its own media, so nothing from the last one
        // may survive.
        self.media.forget_project(&self.project);
        self.canvas
            .restore_view(match self.project.session().active {
                Active::Workspace(id) => self
                    .project
                    .session()
                    .views
                    .get(&id)
                    .copied()
                    .unwrap_or_default(),
                Active::Project => crate::project::View::default(),
            });
    }

    /// Put this project and its graph where the last one was: what Open, New and a loose
    /// workspace do. The synth drops the old project's running state, which an undo keeps,
    /// every picture window on a node closes while the mix's stays, and the show plays from
    /// zero: nothing of the transport is saved.
    pub(super) fn replace_project(&mut self, project: Project, graph: Graph) {
        self.doc.replace(graph);
        self.project = project;
        self.home = super::Home::Folder;
        self.recovery = None;
        self.autosave.reset(&self.project);
        self.link.replace_project();
        self.transport(crate::transport::Command::Play);
        self.transport(crate::transport::Command::Seek(0.0));
        self.after_time_travel();
        self.restore_view();
        self.next_drop = FIRST_DROP;
        self.show.problems.forget_project();
        for ask in self.wall.forget_nodes() {
            self.pictures.send(ask);
        }
    }

    /// True, with a line on the status bar, while a render is reading the document.
    fn refuse_while_rendering(&mut self) -> bool {
        let rendering = self.rendering();
        if rendering {
            self.fail("a render is running: cancel it first");
        }
        rendering
    }

    /// Write the whole folder: the project file and every workspace file.
    ///
    /// The pictures are asked for here and land later — see [`App::mark_thumbnails`].
    ///
    /// # Errors
    /// The folder could not be written, as the status line says it. Only the confirm reads
    /// the error, because only the confirm has something waiting on the save; for a Save of
    /// its own the status line is the whole answer.
    pub fn save_project(&mut self) -> Result<(), String> {
        // The launch's scratch project has no folder of its own: a Save is Save as…, and the
        // temp folder is written nothing.
        if self.home == super::Home::Scratch {
            let why = "this project has no folder yet: choose one".to_string();
            self.media.set_status(why.clone());
            self.ask_for_file(FileAsk::SaveAs);
            return Err(why);
        }
        // The live transform is the active tab's view, and the file is what keeps it.
        self.stash_view();
        match self.project.save(self.doc.graph()) {
            Ok(()) => {
                self.doc.mark_saved();
                self.autosave.clear(&self.project);
                self.prefs.push_recent(self.project.root().to_path_buf());
                // After the save, so every workspace has the file name its picture goes
                // beside.
                self.mark_thumbnails();
                let status = self.media.saved_status(&self.project);
                self.media.set_status(status);
                Ok(())
            }
            Err(e) => {
                let status = format!("save failed: {e}");
                self.fail(status.clone());
                Err(status)
            }
        }
    }

    /// Ask the renderer for a picture of each open workspace.
    ///
    /// **The Output that stands for a workspace** is the selected preview Output when it is
    /// on that workspace — that is the one being looked at — and otherwise the first Output
    /// on it in id order. Only an Output that is awake and has a shader is asked: a
    /// suspended or unplugged one would never render, so the request would never land and
    /// the save would never finish saying so.
    fn mark_thumbnails(&mut self) {
        let mut pending = std::collections::BTreeMap::new();
        if self.has_gpu {
            let live = self.live_nodes();
            let graph = self.doc.graph();
            let selected = self
                .canvas
                .selected_where(|id| graph.get(id).is_some_and(|n| n.def.is_output));
            for workspace in self.project.session().open.iter().copied() {
                let on_here = |id: NodeId| {
                    graph
                        .get(id)
                        .is_some_and(|n| n.def.is_output && n.workspaces.contains(&workspace))
                };
                let node = selected.filter(|id| on_here(*id)).or_else(|| {
                    graph
                        .on_workspace(workspace)
                        .find(|(_, n)| n.def.is_output)
                        .map(|(id, _)| id)
                });
                let Some(node) =
                    node.filter(|id| live.contains(id) && self.link.shader(*id).is_some())
                else {
                    continue;
                };
                pending.insert(workspace, node);
            }
        }
        if !pending.is_empty() {
            let asked: Vec<NodeId> = pending.values().copied().collect();
            self.link.send(crate::synth::Msg::Thumbnails(asked));
        }
        self.media.await_thumbnails(pending);
    }

    /// Write every Snap that has landed, and say where it went.
    ///
    /// **Into `snaps/` inside the project folder.** The project folder is the thing that
    /// travels, so a picture taken out of a patch belongs beside the patch that made it —
    /// but not in `workspaces/` or `assets/`, which are the project's own machinery and are
    /// read back on open. A snap is nobody's input: it is a person's picture, so it gets a
    /// folder of its own that nothing loads.
    pub(super) fn collect_snaps(&mut self) {
        let snaps: Vec<_> = self
            .link
            .snapshot()
            .events
            .snaps
            .unseen("snaps")
            .map(|(_, snap)| snap.clone())
            .collect();
        for (node, width, height, rgba) in snaps {
            let slug = self.doc.graph().get(node).map_or("output", |n| n.def.slug);
            let stamp = stamp(std::time::SystemTime::now());
            let dir = self.project.root().join(SNAPS);
            let path = free_name(&dir, &format!("{slug}{node}-{stamp}"));
            let image = crate::video::png::Image {
                width,
                height,
                rgba,
            };
            match crate::video::png::write(&path, &image) {
                Ok(()) => {
                    let said = format!("snapped {width}x{height} to {SNAPS}/{}", name_of(&path));
                    self.say_written(said.clone(), path.clone());
                    self.media.set_status_showing(said, path);
                }
                Err(e) => {
                    log::warn!("could not write {}: {e}", path.display());
                    self.fail(format!("snap failed: {e}"));
                }
            }
        }
    }

    /// Replace everything with the project in this folder.
    ///
    /// It clears the history rather than pushing a step onto it: see `reset_history`.
    pub fn open_project(&mut self, root: std::path::PathBuf) {
        if self.refuse_while_rendering() {
            return;
        }
        match Project::open(root.clone()) {
            Ok((project, graph, warnings)) => {
                self.replace_project(project, graph);
                // An autosave newer than the save is offered back, with the question over
                // the project as it was saved.
                self.recovery = self.project.autosave_to_offer(self.doc.graph());
                self.media.set_status(if warnings.is_empty() {
                    format!("opened {}", self.project.name())
                } else {
                    format!(
                        "opened {} with {} warning(s) — {}",
                        self.project.name(),
                        warnings.len(),
                        warnings[0]
                    )
                });
                for w in &warnings {
                    log::warn!("opening {}: {w}", root.display());
                }
                // Every one of them, where the line above has room for the first.
                self.show.problems.opened(warnings);
                self.prefs.push_recent(root);
            }
            Err(e) => self.fail(format!("open failed: {e}")),
        }
    }

    /// Make a project in an empty folder and switch to it: one empty video workspace.
    ///
    /// A folder with anything in it is refused. Writing a project over whatever someone
    /// picked by accident is not a thing to do on a folder pick.
    pub fn new_project(&mut self, root: std::path::PathBuf) {
        if self.refuse_while_rendering() {
            return;
        }
        if root.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            self.fail(format!("{} is not empty", name_of(&root)));
            return;
        }
        let mut project = Project::new(root.clone());
        let graph = Graph::new();
        if let Err(e) = project.save(&graph) {
            self.fail(format!("new project failed: {e}"));
            return;
        }
        self.replace_project(project, graph);
        self.media
            .set_status(format!("new project {}", name_of(&root)));
        self.prefs.push_recent(root);
    }

    /// Copy the folder somewhere else and carry on working there.
    pub fn save_project_as(&mut self, root: std::path::PathBuf) {
        if root.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            self.fail(format!("{} is not empty", name_of(&root)));
            return;
        }
        match self.project.fork_to(root) {
            Ok(project) => {
                self.project = project;
                self.home = super::Home::Folder;
                self.media.refresh_assets(&self.project);
                let _ = self.save_project();
            }
            Err(e) => self.fail(format!("save as failed: {e}")),
        }
    }

    /// A loose `.ssw` outside any project becomes a project of its own, in the projects
    /// directory, holding that one workspace.
    ///
    /// It goes through the same [`Project::import_workspace`](crate::project::Project::import_workspace)
    /// the project tab and a dropped file use, so a file handed to somebody arrives the same
    /// way however it is opened.
    pub fn open_loose_workspace(&mut self, file: &std::path::Path) {
        if self.refuse_while_rendering() {
            return;
        }
        let Some(dir) = self.projects_dir() else {
            self.fail(no_projects_dir());
            return;
        };
        match project::import(file, &dir) {
            Ok((project, graph, report)) => {
                self.replace_project(project, graph);
                self.media
                    .set_status(format!("{report} into {}", self.project.name()));
                for w in &report.warnings {
                    log::warn!("importing {}: {w}", file.display());
                }
                self.show.problems.opened(report.warnings);
                self.prefs.push_recent(self.project.root().to_path_buf());
            }
            Err(e) => self.fail(format!("import failed: {e}")),
        }
    }

    /// Take a `.ssw` into the project on screen, open it and show it.
    ///
    /// **The import is a `Command`**, so it is one undo step and the command log is honest
    /// about where those nodes came from. Opening the tab afterwards is not: showing a
    /// workspace is session state, as it is everywhere else.
    pub fn import_workspace_file(&mut self, file: &std::path::Path) {
        self.imported = None;
        if let Err(err) = self.apply(Command::ImportWorkspace {
            file: file.to_path_buf(),
        }) {
            self.fail(format!("import failed: {err}"));
            return;
        }
        // A workspace arrives with its media, which is the fourth way `assets/` changes.
        self.media.refresh_assets(&self.project);
        if let Some(id) = self.imported.take() {
            self.open_workspace(id);
        }
    }

    /// Write one workspace out to a folder, and say what stayed behind.
    pub fn export_workspace(&mut self, id: crate::graph::WorkspaceId, dest: &std::path::Path) {
        match self.project.export_workspace(self.doc.graph(), id, dest) {
            Ok(report) => {
                for cable in &report.dropped_cables {
                    log::info!("export left behind the cable {cable}");
                }
                for (node, workspaces) in &report.shared {
                    log::info!("node {node} is also on {}", workspaces.join(", "));
                }
                for missing in &report.missing {
                    log::warn!("export could not find {missing}");
                }
                self.media.set_status(report.to_string());
                self.show.problems.exported(report);
            }
            Err(e) => self.fail(format!("export failed: {e}")),
        }
    }

    /// Copy one asset back out of the project, into a folder somebody picked.
    fn export_asset(&mut self, reference: &str, dest: &std::path::Path) {
        let source = self.project.resolve(reference);
        let target = dest.join(name_of(&source));
        match std::fs::copy(&source, &target) {
            Ok(_) => self.media.set_status(format!(
                "exported {} to {}",
                name_of(&source),
                name_of(dest)
            )),
            Err(e) => self.fail(format!("export failed: {e}")),
        }
    }

    /// Show the projects folder in the desktop's file manager, making it first if this is the
    /// first anyone has asked for it.
    pub(super) fn show_projects_folder(&mut self) {
        let Some(dir) = self.projects_dir() else {
            self.fail(no_projects_dir());
            return;
        };
        match project::projects_dir_problem(&dir, true) {
            None => crate::platform::files::reveal(dir),
            Some(problem) => self.fail(problem),
        }
    }

    /// Show this project's folder in the desktop's file manager: Project ▸ Show project folder.
    pub(super) fn show_project_folder(&self) {
        crate::platform::files::reveal(self.project.root().to_path_buf());
    }

    /// Show the file the status line is about — a Snap, a render — selected in its folder.
    pub(super) fn show_status_file(&self) {
        if let Some(file) = self.media.status_shows() {
            crate::platform::files::reveal_file(file.to_path_buf());
        }
    }

    /// Show an asset's folder in the desktop's file manager.
    ///
    /// Spawned and never waited on: a file manager takes as long as it takes, and the frame
    /// thread is not allowed to find out how long that is.
    pub(super) fn reveal_asset(&mut self, reference: &str) {
        let path = self.project.resolve(reference);
        let dir = path.parent().unwrap_or(&path).to_path_buf();
        self.media.set_status(format!("opened {}", dir.display()));
        crate::platform::files::reveal(dir);
    }

    /// Delete an asset, or say what is still using it.
    pub(super) fn remove_asset(&mut self, reference: &str) {
        match self.project.remove_asset(self.doc.graph(), reference) {
            Ok(()) => {
                self.media.forget_asset(reference, &self.project);
                self.media.set_status(format!("removed {reference}"));
            }
            Err(e) => self.fail(e),
        }
    }

    /// What the command line named: a project folder, a file inside one, or a loose file.
    pub fn open_argument(&mut self, path: &std::path::Path) {
        if let Some(root) = project::enclosing(path) {
            self.open_project(root);
        } else if path.is_dir() {
            self.fail(format!("{} is not a project", name_of(path)));
        } else {
            self.open_loose_workspace(path);
        }
    }

    /// The project to land in on launch: the most recent one still there, or a new
    /// `Untitled` in the projects folder.
    ///
    /// A folder dialog before anyone has drawn anything is the wrong first thing to see, so
    /// a machine with nothing recent gets a project made for it rather than a question — and
    /// the status line says where, in full, since nobody chose the place.
    pub fn open_last_or_untitled(&mut self) {
        if let Some(root) = self
            .prefs
            .get()
            .recent
            .iter()
            .find(|p| Project::is_project(p))
            .cloned()
        {
            self.open_project(root);
            return;
        }
        let Some(dir) = self.projects_dir() else {
            self.fail(no_projects_dir());
            self.home = super::Home::Scratch;
            return;
        };
        // A folder that cannot be made or read is said, and the editor comes up on the empty
        // document it started with rather than somewhere nobody chose.
        if let Some(problem) = project::projects_dir_problem(&dir, true) {
            log::warn!("{problem}");
            self.fail(problem);
            self.home = super::Home::Scratch;
            return;
        }
        let root = dir.join("Untitled");
        if Project::is_project(&root) {
            self.open_project(root);
        } else {
            self.new_project(root.clone());
            if self.project.root() == root {
                self.media
                    .set_status(format!("new project at {}", root.display()));
            }
        }
    }
}

/// The folder inside a project that Snaps are written into.
pub const SNAPS: &str = "snaps";

/// `<stem>.png` in `dir`, with a counter added where that name is taken.
///
/// Two Snaps inside one second is a hand on the button twice, or a sequencer firing fast;
/// neither should overwrite the other. The counter stops at [`SNAPS_PER_SECOND`] and the last
/// name is reused: a second holding that many Snaps is a stuck button, and walking the folder
/// forever on the frame thread would be the worse failure.
fn free_name(dir: &std::path::Path, stem: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{stem}.png"));
    if !first.exists() {
        return first;
    }
    (2..=SNAPS_PER_SECOND)
        .map(|n| dir.join(format!("{stem}-{n}.png")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem}-{SNAPS_PER_SECOND}.png")))
}

/// How many Snaps one second's stamp may hold before the name stops counting.
const SNAPS_PER_SECOND: u32 = 999;

/// The moment as a file name carries it: `YYYYMMDD-HHMMSS`.
///
/// **UTC**, because there is no timezone database in this binary and there is not going to
/// be one for a file name. What the stamp is for is telling two Snaps apart and sorting
/// them, and UTC does both exactly as local time would.
pub(super) fn stamp(at: std::time::SystemTime) -> String {
    let secs = at
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    let (hh, mm, ss) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Howard Hinnant's `civil_from_days`: a day number since 1970-01-01 as a calendar date, with
/// the era arithmetic that makes the leap years fall out rather than being special-cased.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod start_tests {
    use super::{FileAsk, start_dir};
    use std::path::{Path, PathBuf};

    /// **Open project… and a new project's folder start in the projects folder**, or the
    /// nearest folder above it that is there; Save as beside the project; a node's file inside
    /// it.
    #[test]
    fn open_starts_in_the_projects_folder() {
        let base = std::env::temp_dir().join(format!("ssw-start-{}", std::process::id()));
        let projects = base.join("Documents").join("supersilvia");
        std::fs::create_dir_all(base.join("Documents")).unwrap();
        let project = Path::new("/elsewhere/friday");

        let start = |ask| start_dir(&ask, project, Some(&projects));
        assert_eq!(
            start(FileAsk::OpenProject),
            Some(base.join("Documents")),
            "before the projects folder exists"
        );
        std::fs::create_dir_all(&projects).unwrap();
        assert_eq!(start(FileAsk::OpenProject), Some(projects.clone()));
        assert_eq!(start(FileAsk::NewProject), Some(projects.clone()));
        assert_eq!(start(FileAsk::SaveAs), Some(PathBuf::from("/elsewhere")));
        assert_eq!(start(FileAsk::ImportAsset), Some(project.to_path_buf()));
        assert_eq!(
            start_dir(&FileAsk::OpenProject, project, None),
            Some(project.to_path_buf()),
            "with no projects folder at all, the project's own"
        );
        std::fs::remove_dir_all(&base).ok();
    }
}

#[cfg(test)]
mod drop_tests {
    use super::{Lands, drop_line, lands};
    use std::path::Path;

    fn held(names: &[&str], on_canvas: bool) -> Vec<(Lands, String)> {
        names
            .iter()
            .map(|n| (lands(Some(Path::new(n)), on_canvas), (*n).to_string()))
            .collect()
    }

    /// The line says what the drop will make, by the file and the tab it is over.
    #[test]
    fn a_held_drop_says_what_it_will_make() {
        let on = |names: &[&str]| drop_line(&held(names, true), Some("Tunnel"));
        assert_eq!(on(&["gumbasia.webm"]), "new Video node on Tunnel");
        assert_eq!(on(&["logo.png"]), "new Image/GIF node on Tunnel");
        assert_eq!(on(&["tunnel.ssw"]), "tunnel.ssw: import as workspace");
        assert_eq!(
            on(&["a.webm", "b.mov", "c.gif", "d.ssw"]),
            "2 new Video nodes, new Image/GIF node on Tunnel · d.ssw: import as workspace"
        );
        let project = |names: &[&str]| drop_line(&held(names, false), None);
        assert_eq!(project(&["gumbasia.webm"]), "gumbasia.webm: add to assets");
        assert_eq!(project(&["a.webm", "b.png"]), "2 files: add to assets");
        assert_eq!(
            project(&["tunnel.ssw"]),
            "tunnel.ssw: import as workspace",
            "a workspace is a workspace on either tab"
        );
        assert_eq!(
            drop_line(&[(Lands::Unknown, String::new())], None),
            "a file: import"
        );
    }
}

#[cfg(test)]
mod snap_tests {
    use super::stamp;
    use std::time::{Duration, UNIX_EPOCH};

    /// The stamp is the calendar, not a count of seconds: the epoch itself, a leap day, and
    /// the turn of a century that is not a leap year.
    #[test]
    fn a_stamp_is_the_calendar() {
        let at = |secs| stamp(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "19700101-000000");
        assert_eq!(at(86_399), "19700101-235959");
        // 2000-02-29T12:34:56Z, a leap day in a century that is one.
        assert_eq!(at(951_827_696), "20000229-123456");
        // 2100-03-01T00:00:00Z, the day after a February that has no 29th.
        assert_eq!(at(4_107_542_400), "21000301-000000");
    }
}

/// A file dropped where winit did not hear it, as egui holds one.
#[derive(Debug)]
struct DroppedPath(std::path::PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|err| err.to_string())
    }
}
