// SPDX-License-Identifier: AGPL-3.0-or-later

//! The project: a folder, and the only thing supersilvia saves.
//!
//! ```text
//! friday/
//!   project.ssp          the workspaces in project order, the two counters, the glue,
//!                        the MIDI map and the Main Input
//!   workspaces/
//!     tunnel.ssw         one workspace: kind, name, its nodes and the cables between them
//!   assets/
//!     gumbasia.webm      media, referenced as assets/gumbasia.webm
//! ```
//!
//! **`assets/` is the project's media.** Every file that reaches a node is copied in and
//! referenced as `assets/<name>`, so the folder moves, zips and travels whole. [`Project::resolve`]
//! turns a reference back into a path; nothing outside this module knows where the project is.
//!
//! **The two extensions say which file is which.** A `.ssw` is always one workspace and
//! opens on its own; a `.ssp` is always a project, and is nothing without the folder around
//! it. Nothing has to look inside a file to know what it is.
//!
//! **A workspace file is portable and the project file is not.** A workspace file holds the
//! nodes it writes and every cable with both ends among them, so it means the same thing in
//! any project. Everything that is about *this* set of workspaces together — the order, the
//! id counters, a cable whose ends are on two of them, the other workspaces a shared node is
//! on — is glue, and glue lives in `project.ssp`.
//!
//! **On disk a node is written once, by the first workspace in project order among its set.**
//! Nothing stores that choice: it follows from the order, so reordering the workspaces moves
//! a shared node between files at the next save, and Save rewrites every file anyway.
//!
//! **A save lands whole or not at all, and the manifest's rename is the commit.** Every
//! workspace file is written beside its own as `<file>.<save>.tmp` and synced, then the
//! manifest is written to a temporary name, synced and renamed over `project.ssp`. Only then
//! are the workspace files renamed into place and the removed ones deleted, by [`finish`],
//! which [`Project::open`] runs too — so a save cut off anywhere opens as the last save
//! whole, or as this one whole once its manifest landed. Renaming each workspace file into
//! place before the manifest would open a save cut off between two of them as a mix of both,
//! with a node that moved files loaded twice.
//!
//! **A painting is a picture file in `assets/`**, `painting-<print>.png`, named by what is in
//! it: a `drawingcanvas`'s picture, which the workspace file names and does not hold. A save
//! writes each one that is not there yet before anything else, so the manifest's rename commits
//! a save whose pictures are already on the disk, and deletes the ones the save before it wrote
//! and this one no longer names, once it has committed — the one thing under `assets/` a save
//! deletes, because a painting file is the project's own and nobody else's. See
//! docs/decisions.md, *A painting is saved with the project*.
//!
//! **`.autosave/` is a project folder of its own**, written by the same save while the
//! document has unsaved edits and deleted by the next real one, so a crash, a kill or a lost
//! GPU costs at most the edits since the last autosave. It is never read except to offer
//! those edits back, and only when it is newer than the last save and differs from it: see
//! [`Project::autosave`] and [`Project::autosave_to_offer`].

use crate::graph::{Graph, NodeId, Workspace, WorkspaceId, WorkspaceKind};
use crate::maininput::MainInput;
use crate::mixer::Mixer;
use crate::nodes::{self, Assets};
use crate::workspace::{self, Ids, LoadError, LoadWarning, SavedConnection, WorkspaceFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The project file, at the root of the folder.
pub const MANIFEST: &str = "project.ssp";
/// Its extension, without the dot.
pub const EXTENSION: &str = "ssp";
/// Where the workspace files live, under the root.
pub const WORKSPACES: &str = "workspaces";
/// Where the project's media lives, under the root.
pub const ASSETS: &str = "assets";
/// Where the files derived from that media live, under the root: a transcode, a decoded
/// soundtrack. Made on demand and deletable at any time.
pub const CACHE: &str = "cache";
/// Where the unsaved edits are kept, under the root: see the module doc.
pub const AUTOSAVE: &str = ".autosave";

/// The `format` field of the project file. A file that does not carry this is not one.
const FORMAT: &str = "supersilvia-project";
/// Bumped only for a change old readers cannot cope with.
const VERSION: u32 = 1;

/// The longest a derived file name may be, before the extension. Long enough for any name
/// someone types and short enough to leave room under every filesystem's limit.
const NAME_LIMIT: usize = 64;

/// The project file itself.
///
/// Every field a later step adds arrives with `#[serde(default)]`, the way `connections` and
/// `shared` did: thumbnails, blurbs and the MIDI map are all glue, and none of them may move
/// the version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    /// Every workspace, in project order.
    workspaces: Vec<ManifestWorkspace>,
    /// Which workspaces have a tab. Session state, not an edit: it is the project's, and it
    /// never enters the undo history.
    open: Vec<WorkspaceId>,
    /// Which tab is showing.
    active: Active,
    /// The node id counter. The project owns it, so every node in every file of a project
    /// has an id no other node will be given.
    #[serde(default)]
    next_node_id: u32,
    #[serde(default)]
    next_workspace_id: u32,
    /// Cables whose two ends are written by different workspace files. No workspace file
    /// refers to a node it does not hold, so these have nowhere else to go.
    #[serde(default)]
    connections: Vec<SavedConnection>,
    /// The workspaces a shared node is on beyond the one that writes it.
    #[serde(default)]
    shared: Vec<SharedNode>,
    /// The MIDI map. A port's key rides as a string and is resolved against the node's own
    /// ports on load, the same trip a cable makes.
    #[serde(default)]
    midi: Vec<crate::midi::Saved>,
    /// Which save wrote this file, counted from one: its workspace files are the
    /// `<file>.<save>.tmp` [`finish`] renames into place.
    #[serde(default)]
    save: u64,
    /// The workspace files that save removed, which [`finish`] deletes.
    #[serde(default)]
    removed: Vec<String>,
    /// What the Main Input was pointed at and how it was tuned. A file from before it rode
    /// here opens with the Main Input at its default.
    #[serde(default)]
    main_input: MainInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestWorkspace {
    id: WorkspaceId,
    name: String,
    #[serde(default)]
    kind: WorkspaceKind,
    /// The file under `workspaces/`, name only. Kept here so renaming a workspace does not
    /// lose track of the file its nodes are in.
    file: String,
    /// Where this workspace was left: switching to its tab brings back the view you left.
    view: View,
}

/// Which tab is showing. The project tab is not a workspace, so it is its own case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Active {
    /// The pinned first tab, which lists what the project holds.
    #[default]
    Project,
    Workspace(WorkspaceId),
}

/// One workspace's pan and zoom, as the project file carries it.
///
/// Plain numbers rather than the canvas's `Transform`: the file layer describes the view, and
/// `ui/` is what turns it into a transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub pan: [f32; 2],
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan: [0.0, 0.0],
            zoom: 1.0,
        }
    }
}

/// Which workspaces are open, which tab is showing, and where each view was left.
///
/// **Session state, never an edit.** It is saved in `project.ssp` because it is about this
/// project rather than about the person, and it is not a `Command`, so opening and closing a
/// workspace never enters the undo history — exactly as switching tabs is not an edit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    /// Every workspace with a tab. Tab order is project order, so a set is enough.
    pub open: BTreeSet<WorkspaceId>,
    pub active: Active,
    /// Where each workspace's canvas was left. A workspace with no entry opens at the origin.
    pub views: BTreeMap<WorkspaceId, View>,
}

impl Session {
    /// The workspace whose canvas is showing, if a workspace is showing at all.
    pub fn active_workspace(&self) -> Option<WorkspaceId> {
        match self.active {
            Active::Project => None,
            Active::Workspace(id) => Some(id),
        }
    }

    /// Drop what the graph no longer has, and leave something open and something showing.
    ///
    /// Run on both sides of the file, so neither a hand-edited manifest nor a workspace
    /// deleted since the session was recorded can leave the app with no tab. A session that
    /// has never been recorded — nothing open at all — lands on the first workspace rather
    /// than on the project tab, which is what a project made a moment ago should show.
    pub fn reconcile(&mut self, graph: &Graph) {
        let unrecorded = self.open.is_empty();
        self.open.retain(|w| graph.has_workspace(*w));
        self.views.retain(|w, _| graph.has_workspace(*w));
        if self.open.is_empty() {
            self.open.insert(graph.default_workspace());
        }
        if unrecorded {
            self.active = Active::Workspace(graph.default_workspace());
        }
        if let Active::Workspace(id) = self.active
            && !self.open.contains(&id)
        {
            self.active = Active::Project;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedNode {
    node: NodeId,
    workspaces: Vec<WorkspaceId>,
}

/// A project folder, and which file each workspace was last written to.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    /// One file name per workspace. A workspace missing from this map has never been
    /// written and gets a name derived from its own at the next save; a name in it whose
    /// workspace is gone names the file that save deletes.
    files: BTreeMap<WorkspaceId, String>,
    /// The number of the save on disk, the manifest's `save`.
    saved: u64,
    /// The painting files that save names, `assets/painting-<print>.png`: what the next save
    /// deletes if it no longer names them.
    paintings: BTreeSet<String>,
    /// Which workspaces are open, which tab is showing, and each one's view.
    session: Session,
    /// Which Outputs are on the decks and how they are mixed.
    ///
    /// **Session state, written to no file.** Which Output is on air and where the fade is
    /// are a hand on the instrument tonight, not part of the patch: a project opens with
    /// nothing already on air and the canvas not already covered by a mix. Held here rather
    /// than on `App` only because `Project` is what the app hands around.
    mixer: Mixer,
    /// What the Main Input is pointed at and how it is tuned.
    ///
    /// **Saved with the project, and never an edit**, as silvia's is: written by every save,
    /// read back by Open through [`MainInput::restored`], carried by Save as. Choosing a
    /// source is playing rather than editing, so it is not a command and does not mark the
    /// project unsaved, as a tab or a MIDI binding does not.
    main_input: MainInput,
    /// Which MIDI message drives which control.
    ///
    /// **Project data, and the tier test says so outright.** A binding names a node in *this*
    /// project, so it is meaningless anywhere else and cannot be a preference; and it is
    /// about the whole patch rather than one workspace, so it cannot ride in a `.ssw`. Which
    /// controller is plugged in is the rig and is written nowhere — the binding names a
    /// channel and a number, not a box.
    midi: crate::midi::Bindings,
}

impl Project {
    /// A project at this root. Touches no disk: the folder is written by [`Project::save`].
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: BTreeMap::new(),
            saved: 0,
            paintings: BTreeSet::new(),
            session: Session::default(),
            mixer: Mixer::default(),
            main_input: MainInput::default(),
            midi: crate::midi::Bindings::default(),
        }
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// The same, to write. A claim, a fade and a method are playing, not editing, so none
    /// of them is a command and none touches the undo history.
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    pub fn main_input(&self) -> &MainInput {
        &self.main_input
    }

    /// The same, to write. Choosing a source is playing, not editing, so it is not a command
    /// and it never touches the undo history.
    pub fn main_input_mut(&mut self) -> &mut MainInput {
        &mut self.main_input
    }

    /// Which workspaces are open, which tab is showing, and each one's view.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The same, to write. Opening a tab, closing one and panning a canvas all land here;
    /// none of them is a command, so none of them touches the undo history.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Check the session and the decks against a graph: see [`Session::reconcile`] and
    /// [`Mixer::reconcile`]. The MIDI map is not: a binding outlives its node, so an undo
    /// brings it back, and only its live half is published or saved — see
    /// [`crate::midi::Bindings::live`].
    pub fn reconcile(&mut self, graph: &Graph) {
        self.session.reconcile(graph);
        self.mixer.reconcile(graph);
    }

    /// Which MIDI message drives which control.
    pub fn midi(&self) -> &crate::midi::Bindings {
        &self.midi
    }

    /// The same, to write. Learning and unlearning land here; neither is a command, so
    /// neither touches the undo history — binding a knob is setting the rig up, not editing
    /// the patch, even though what it *writes* is the patch.
    pub fn midi_mut(&mut self) -> &mut crate::midi::Bindings {
        &mut self.midi
    }

    /// A project under the system temp directory, unique to this process and call.
    ///
    /// What `App::headless` and the kittest harness get, so no test can reach the real data
    /// directory — the same job `preferences::Store::in_memory` does. Nothing is written
    /// until something saves.
    pub fn scratch() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self::new(
            std::env::temp_dir().join(format!("supersilvia-scratch-{}-{n}", std::process::id())),
        )
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder's name, which is the project's name: what the title bar and the status
    /// line show.
    pub fn name(&self) -> String {
        self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// Is there a project here? A folder holding a project file.
    pub fn is_project(root: &Path) -> bool {
        root.join(MANIFEST).is_file()
    }

    /// Read a project: the manifest, then every workspace file it lists, then the glue.
    ///
    /// Failures split the way the compiler's do. A missing or unreadable *project* file is a
    /// [`LoadError`] and nothing on screen changes; anything else — a listed file that is
    /// gone, a stray file adopted, a cable the graph refused — is a [`LoadWarning`] and the
    /// project opens without it.
    pub fn open(root: PathBuf) -> Result<(Self, Graph, Vec<LoadWarning>), LoadError> {
        let text = std::fs::read_to_string(root.join(MANIFEST)).map_err(LoadError::Io)?;
        let header = workspace::Header::of(&text)?;
        if header.format != FORMAT {
            return Err(LoadError::NotAProject(header.format));
        }
        if header.version > VERSION {
            return Err(LoadError::UnsupportedVersion(header.version));
        }
        let manifest: Manifest = serde_json::from_str(&text).map_err(LoadError::Json)?;

        let dir = root.join(WORKSPACES);
        // A save cut off after its commit is finished before anything is read. A folder that
        // cannot be written opens as it stands.
        let _ = finish(
            &dir,
            manifest.save,
            manifest.workspaces.iter().map(|w| w.file.as_str()),
            &manifest.removed,
            &mut || Ok(()),
        );
        let mut warnings = Vec::new();
        let mut files = BTreeMap::new();
        // Every workspace the project file names, whether or not its file could be read.
        let ids: HashSet<WorkspaceId> = manifest.workspaces.iter().map(|w| w.id).collect();

        // Every listed file is read before any node lands, so a workspace whose file is gone
        // is dropped before a node can name it.
        let mut listed = Vec::new();
        for w in manifest.workspaces {
            match workspace::read(&dir.join(&w.file)) {
                Ok(file) => listed.push((w, file)),
                Err(e) => warnings.push(LoadWarning::MissingWorkspace {
                    file: w.file,
                    reason: e.to_string(),
                }),
            }
        }

        let mut graph = Graph::new();
        graph.set_workspaces(
            listed
                .iter()
                // The layout mode is the workspace's own and rides in the workspace file, so
                // it comes back from there rather than from the manifest.
                .map(|(w, file)| Workspace {
                    id: w.id,
                    name: w.name.clone(),
                    kind: w.kind,
                    // The blurb is the workspace's own and rides in its file, beside the
                    // layout mode and for the same reason: both travel with an export.
                    blurb: file.blurb.clone(),
                    layout: file.layout,
                })
                .collect(),
            manifest.next_workspace_id,
        );
        graph.set_next_node_id(manifest.next_node_id);
        // Every file and then the glue is one change, settled once at the end.
        graph.begin_bulk();

        let session = Session {
            open: manifest.open.into_iter().collect(),
            active: manifest.active,
            views: listed.iter().map(|(w, _)| (w.id, w.view)).collect(),
        };

        let assets = root.join(ASSETS);
        let mut paintings = BTreeSet::new();
        for (w, mut file) in listed {
            files.insert(w.id, w.file);
            paintings.extend(painting_references(&file));
            warnings.extend(read_paintings(&mut file, &assets));
            warnings.extend(file.insert_into(&mut graph, w.id, Ids::Keep));
        }

        // The glue, after every file: both halves name nodes another file may hold.
        //
        // A workspace the project file *lists* but could not read is dropped above, and a
        // membership naming it goes quietly with it — the missing file is already reported.
        // A workspace it does not list at all is a project file that disagrees with itself,
        // and there is nowhere to put the node: that is an error, not a warning.
        for shared in manifest.shared {
            for id in shared.workspaces {
                if !ids.contains(&id) {
                    return Err(LoadError::UnknownWorkspace(id));
                }
                if graph.has_workspace(id)
                    && let Some(node) = graph.get_mut(shared.node)
                {
                    node.workspaces.insert(id);
                }
            }
        }
        let identity = HashMap::new();
        for c in &manifest.connections {
            warnings.extend(workspace::connect(&mut graph, c, &identity));
        }
        // After the glue, which is the last of the project's cables: a dual output fed from
        // another workspace is only decided once every one of them is in.
        warnings.extend(workspace::settle(&mut graph));
        // Two Outputs a file sends under one name: the later takes a ` copy`, in the graph
        // alone, so the project opens clean and the file changes only if it is saved.
        let outputs: Vec<NodeId> = graph
            .iter()
            .filter(|(_, node)| node.def.is_output)
            .map(|(id, _)| id)
            .collect();
        nodes::output::settle_names(&mut graph, &outputs);

        let mut project = Self {
            root,
            files,
            saved: manifest.save,
            paintings,
            session,
            // The mixer is not in the file and starts at its default; the Main Input is, and
            // comes back as silvia's does. See the fields on `Project`.
            mixer: Mixer::default(),
            main_input: manifest.main_input.restored(),
            // The map is, and its port keys are resolved against the graph the file just
            // built — a binding to a control that is gone is dropped in the resolving.
            midi: crate::midi::Bindings::restored(&manifest.midi, &graph),
        };
        project.adopt_strays(&mut graph, &mut warnings);
        // After the strays, so an adopted workspace is one of the ones the session names.
        project.reconcile(&graph);
        Ok((project, graph, warnings))
    }

    /// Take in every `workspaces/*.ssw` the project file does not list.
    ///
    /// Someone who drops a workspace into the folder from a file manager gets what they
    /// meant. Its ids come from another counter, so the file arrives through `insert_node`
    /// with fresh ones and every cable inside it re-pointed to match.
    fn adopt_strays(&mut self, graph: &mut Graph, warnings: &mut Vec<LoadWarning>) {
        let dir = self.root.join(WORKSPACES);
        let known: HashSet<&str> = self.files.values().map(String::as_str).collect();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut strays: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == workspace::EXTENSION))
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| !known.contains(n.to_string_lossy().as_ref()))
            })
            .collect();
        // Read in name order, so two strays adopted together get their ids in an order that
        // does not depend on the directory.
        strays.sort();

        for path in strays {
            let name = file_name(&path);
            match workspace::read(&path) {
                Ok(mut file) => {
                    self.paintings.extend(painting_references(&file));
                    warnings.extend(read_paintings(&mut file, &self.root.join(ASSETS)));
                    let workspace = graph.add_workspace(
                        if file.name.is_empty() {
                            stem(&path)
                        } else {
                            file.name.clone()
                        },
                        file.kind,
                    );
                    // The file's own, not the preference: a workspace laid out as a strip is
                    // laid out as a strip for whoever opens it.
                    graph.set_layout(workspace, file.layout);
                    graph.set_blurb(workspace, file.blurb.clone());
                    // Open, so a file someone dropped into the folder arrives with a tab
                    // rather than only a warning.
                    self.session.open.insert(workspace);
                    warnings.extend(file.insert_into(graph, workspace, Ids::Fresh));
                    warnings.push(LoadWarning::AdoptedWorkspace { file: name.clone() });
                    self.files.insert(workspace, name);
                }
                Err(e) => warnings.push(LoadWarning::UnreadableWorkspace {
                    file: name,
                    reason: e.to_string(),
                }),
            }
        }
    }

    /// Write the whole folder: the project file and every workspace file, committed by the
    /// manifest's rename — see the module doc.
    ///
    /// A workspace removed since the last save has its file deleted, and so does a save's
    /// own temporary file, and nothing else is ever deleted by a save — a stray somebody put
    /// in the folder is adopted, not binned.
    pub fn save(&mut self, graph: &Graph) -> Result<(), LoadError> {
        self.save_with(graph, &mut || Ok(()))
    }

    /// [`Project::save`], calling `after_each` once each file is in place and stopping at the
    /// first error it returns: a save cut off after any one file, for a test.
    pub fn save_with(
        &mut self,
        graph: &Graph,
        after_each: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> Result<(), LoadError> {
        // Nothing writes a session naming a workspace that is not there, or a deck naming
        // an Output that is not: a workspace deleted since the tab was recorded leaves the
        // set here rather than on the next open.
        self.reconcile(graph);
        let dir = self.root.join(WORKSPACES);
        std::fs::create_dir_all(&dir).map_err(LoadError::Io)?;
        // The pictures before the files that name them, so a manifest never commits a
        // painting the disk does not have.
        let paintings = write_paintings(graph, &self.root.join(ASSETS)).map_err(LoadError::Io)?;

        let order: Vec<WorkspaceId> = graph.workspaces().iter().map(|w| w.id).collect();
        let mut owned: BTreeMap<WorkspaceId, BTreeSet<NodeId>> = BTreeMap::new();
        let mut shared: Vec<SharedNode> = Vec::new();
        let mut owner: HashMap<NodeId, WorkspaceId> = HashMap::new();
        for (id, node) in graph.iter() {
            let Some(first) = order.iter().find(|w| node.workspaces.contains(w)) else {
                continue;
            };
            owner.insert(id, *first);
            owned.entry(*first).or_default().insert(id);
            let rest: Vec<WorkspaceId> = node
                .workspaces
                .iter()
                .copied()
                .filter(|w| w != first)
                .collect();
            if !rest.is_empty() {
                shared.push(SharedNode {
                    node: id,
                    workspaces: rest,
                });
            }
        }

        // A workspace keeps the file it was written to, so renaming it does not orphan one.
        // Every name the last save wrote is taken, a removed workspace's included: that file
        // is deleted after this save writes, so a new workspace given it would be deleted too.
        let mut files: BTreeMap<WorkspaceId, String> = BTreeMap::new();
        let mut taken: HashSet<String> = self.files.values().cloned().collect();
        for w in graph.workspaces() {
            let name = match self.files.get(&w.id) {
                Some(existing) if !files.values().any(|f| f == existing) => existing.clone(),
                _ => unique_file_name(&w.name, &taken),
            };
            taken.insert(name.clone());
            files.insert(w.id, name);
        }

        let save = self.saved + 1;
        let removed: Vec<String> = self
            .files
            .iter()
            .filter(|(id, _)| !files.contains_key(id))
            .map(|(_, file)| file.clone())
            .collect();
        let manifest = Manifest {
            format: FORMAT.to_string(),
            version: VERSION,
            save,
            removed,
            // A binding whose node is gone stays in the map for an undo, and not in the file.
            midi: self.midi.live_in(graph).saved(),
            main_input: self.main_input.clone(),
            workspaces: graph
                .workspaces()
                .iter()
                .map(|w| ManifestWorkspace {
                    id: w.id,
                    name: w.name.clone(),
                    kind: w.kind,
                    file: files[&w.id].clone(),
                    view: self.session.views.get(&w.id).copied().unwrap_or_default(),
                })
                .collect(),
            open: self.session.open.iter().copied().collect(),
            active: self.session.active,
            next_node_id: graph.next_node_id(),
            next_workspace_id: graph.next_workspace_id(),
            connections: graph
                .connections()
                .iter()
                .filter(|c| owner.get(&c.from.node) != owner.get(&c.to.node))
                .map(SavedConnection::of)
                .collect(),
            shared,
        };
        let json = serde_json::to_string_pretty(&manifest).map_err(LoadError::Json)? + "\n";

        for w in graph.workspaces() {
            let nodes = owned.remove(&w.id).unwrap_or_default();
            let text = workspace::to_text(&WorkspaceFile::of(graph, w, &nodes))?;
            write_synced(&dir.join(staged_name(&files[&w.id], save)), &text)
                .and_then(|()| after_each())
                .map_err(LoadError::Io)?;
        }
        let staged = self.root.join(format!("{MANIFEST}.tmp"));
        sync_dir(&dir)
            .and_then(|()| write_synced(&staged, &json))
            .and_then(|()| std::fs::rename(&staged, self.root.join(MANIFEST)))
            .and_then(|()| sync_dir(&self.root))
            .map_err(LoadError::Io)?;
        // Committed: the folder is this save now, whatever is left of it to finish.
        self.files = files;
        self.saved = save;
        let superseded: Vec<String> = self.paintings.difference(&paintings).cloned().collect();
        self.paintings = paintings;
        after_each().map_err(LoadError::Io)?;
        let listed = manifest.workspaces.iter().map(|w| w.file.as_str());
        finish(&dir, save, listed, &manifest.removed, after_each).map_err(LoadError::Io)?;
        // The pictures the last save named and this one does not, unless a node has taken one
        // up as a file of its own.
        let users = asset_users(graph);
        let assets = self.root.join(ASSETS);
        for reference in superseded {
            if users.contains_key(&reference) || !is_painting_name(&reference) {
                continue;
            }
            if let Some(path) = painting_path(&assets, &reference) {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    /// Where this workspace's picture lives: `workspaces/<name>.png`, beside its own file.
    ///
    /// `None` for a workspace that has never been written, which has no file name yet. The
    /// picture is written by the app once the renderer has read one back, not by
    /// [`Project::save`]: nothing on the frame thread waits on the GPU for a picture of
    /// itself.
    pub fn thumbnail_path(&self, id: WorkspaceId) -> Option<PathBuf> {
        let file = self.files.get(&id)?;
        Some(self.root.join(WORKSPACES).join(thumbnail_name(file)))
    }

    /// Where this project's autosave lives.
    pub fn autosave_dir(&self) -> PathBuf {
        self.root.join(AUTOSAVE)
    }

    /// Write `graph` with this project's session, MIDI map and Main Input into
    /// `.autosave/`, through the same save as [`Project::save`], and touch nothing else.
    ///
    /// `shadow` is the project that writes there, kept between autosaves so a workspace
    /// removed since the last one has its file removed as a save's would. With none, or one
    /// rooted elsewhere, it is what is there opened, strays and all, so this save removes
    /// every file it does not write and the last autosave stands until this one commits; a
    /// folder that does not open as a project is emptied instead.
    pub fn autosave(&self, graph: &Graph, shadow: &mut Option<Project>) -> Result<(), LoadError> {
        let dir = self.autosave_dir();
        let writer = match shadow {
            Some(writer) if writer.root == dir => writer,
            _ => {
                let there = if let Ok((there, _, _)) = Project::open(dir.clone()) {
                    there
                } else {
                    remove_dir(&dir).map_err(LoadError::Io)?;
                    Project::new(dir)
                };
                shadow.insert(there)
            }
        };
        writer.session = self.session.clone();
        writer.midi = self.midi.clone();
        writer.main_input = self.main_input.clone();
        writer.save(graph)
    }

    /// The autosave, where one is worth offering: newer than `project.ssp` and a different
    /// document from `graph`, which is what that file opened as. One that is not is deleted
    /// here; one that cannot be read is left where it is and not offered.
    pub fn autosave_to_offer(&self, graph: &Graph) -> Option<Recovered> {
        let dir = self.autosave_dir();
        let modified = |path: PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let when = modified(dir.join(MANIFEST))?;
        let newer = modified(self.root.join(MANIFEST)).is_none_or(|saved| when > saved);
        let offer = if newer {
            match Project::open(dir.clone()) {
                Ok((project, autosaved, _)) => {
                    (!autosaved.same_document(graph)).then_some(Recovered {
                        project,
                        graph: autosaved,
                        when,
                    })
                }
                Err(e) => {
                    log::warn!("could not read the autosave in {}: {e}", dir.display());
                    return None;
                }
            }
        } else {
            None
        };
        if offer.is_none()
            && let Err(e) = remove_dir(&dir)
        {
            log::warn!("could not remove {}: {e}", dir.display());
        }
        offer
    }

    /// Take a recovered autosave as this project's document: its session, MIDI map and Main
    /// Input, and the graph to put on screen. The folder, its files and its last save stay
    /// this project's, so the next Save writes the recovered document here.
    pub fn recover(&mut self, recovered: Recovered) -> Graph {
        self.session = recovered.project.session;
        self.midi = recovered.project.midi;
        self.main_input = recovered.project.main_input;
        recovered.graph
    }

    /// Copy the folder to a new root and hand back the project there.
    ///
    /// Save as: last week's set is forked for tonight without being touched. The caller
    /// saves into the copy, so whatever is on screen lands in it too. Everything goes —
    /// `renders/` and `snaps/` are the person's work — but `.autosave/`, which belongs to the
    /// folder it was written for, and `cache/`, which the copy makes again on demand.
    ///
    /// A project whose folder was never written — the launch's scratch one, saved for the
    /// first time — has nothing to copy, and the copy is the save alone.
    ///
    /// A root that is this project's folder or inside it is refused before anything is
    /// written.
    pub fn fork_to(&self, root: PathBuf) -> Result<Self, LoadError> {
        if inside(&root, &self.root) {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a project cannot be copied into its own folder",
            )));
        }
        if self.root.exists() {
            copy_dir(&self.root, &root, &[AUTOSAVE, CACHE]).map_err(LoadError::Io)?;
        } else {
            std::fs::create_dir_all(&root).map_err(LoadError::Io)?;
        }
        Ok(Self {
            root,
            files: self.files.clone(),
            saved: self.saved,
            paintings: self.paintings.clone(),
            session: self.session.clone(),
            // The map and the Main Input are the project's, so they fork with it; an asset
            // the Main Input names is copied with the folder and resolves against the copy.
            midi: self.midi.clone(),
            main_input: self.main_input.clone(),
            // A fork is a copy of the patch, and nothing on the decks is part of that.
            mixer: Mixer::default(),
        })
    }

    /// Write one workspace out on its own: the file, its picture, and its media.
    ///
    /// **Every node on the workspace goes, shared or not, as a plain node**, with every
    /// cable whose two ends are both here. That is the point of an export: the file works
    /// on its own, and being shown on three other workspaces is a fact about *this* rig
    /// rather than about the workspace. What stayed behind is the report's business, not a
    /// silence.
    ///
    /// **A folder that is already a project takes the files into its own `workspaces/` and
    /// `assets/`**, so *export into that project* is the same gesture as *export to a
    /// folder*; the other project adopts the stray next time it is opened.
    ///
    /// # Errors
    /// No such workspace, a kind that means nothing on its own, or the files could not be
    /// written.
    pub fn export_workspace(
        &self,
        graph: &Graph,
        id: WorkspaceId,
        dest: &Path,
    ) -> Result<ExportReport, ExportError> {
        let workspace = graph
            .workspace(id)
            .ok_or(ExportError::NoSuchWorkspace(id))?;
        if !workspace.kind.is_portable() {
            return Err(ExportError::NotPortable(workspace.kind));
        }

        // Into a project's own folders where the destination is one, and into the folder
        // itself where it is not.
        let dir = if Project::is_project(dest) {
            dest.join(WORKSPACES)
        } else {
            dest.to_path_buf()
        };
        // `assets/` beside the file either way: in a project that is the project's own, and
        // in a plain folder it is what makes the exported `assets/<name>` resolve.
        let assets = dest.join(ASSETS);
        std::fs::create_dir_all(&dir).map_err(|e| ExportError::File(LoadError::Io(e)))?;

        let nodes: BTreeSet<NodeId> = graph.on_workspace(id).map(|(node, _)| node).collect();
        let mut file = WorkspaceFile::of(graph, workspace, &nodes);

        // The media, resolved through this project and copied in beside the file, so the
        // exported `assets/<name>` means something wherever the folder ends up.
        let mut report = ExportReport::default();
        for node in &mut file.nodes {
            let Some(def) = nodes::find(&node.slug) else {
                continue;
            };
            for option in def.options {
                if !option.is_asset() {
                    continue;
                }
                let Some(value) = node.options.get(option.key).filter(|v| !v.is_empty()) else {
                    continue;
                };
                let source = self.resolve(value);
                let Ok(copied) = copy_asset(&source, &assets) else {
                    report.missing.push(value.clone());
                    continue;
                };
                let copied = reference(&copied);
                report.assets.push(copied.clone());
                node.options.insert(option.key.to_string(), copied);
            }
        }
        // The paintings, written beside it as the project's own save writes them, so the file's
        // `assets/painting-<print>.png` names a picture wherever the folder ends up.
        for node in &file.nodes {
            for value in node.values.values() {
                let Some(painting) = value.painting() else {
                    continue;
                };
                let reference = painting.file().to_string();
                let written = if painting.is_read() {
                    write_painting(painting, &assets)
                } else {
                    // Still a name: its file, under the same name, since the name is what the
                    // exported file says.
                    match (
                        painting_path(&self.root.join(ASSETS), &reference),
                        painting_path(&assets, &reference),
                    ) {
                        (Some(from), Some(to)) => std::fs::create_dir_all(&assets)
                            .and_then(|()| std::fs::copy(from, to))
                            .map(|_| ()),
                        _ => Err(std::io::ErrorKind::InvalidInput.into()),
                    }
                };
                match written {
                    Ok(()) => report.assets.push(reference),
                    Err(_) => report.missing.push(reference),
                }
            }
        }
        report.assets.sort();
        report.assets.dedup();

        // The map is the project's, so nothing of it rides in a workspace file. Every
        // binding onto a node being exported is one the other end will not have.
        report.midi = self
            .midi
            .iter()
            .filter(|(_, b)| b.target.port().is_some_and(|p| nodes.contains(&p.node)))
            .count();

        // A cable with one end elsewhere has nowhere to go in a file that names only the
        // nodes it holds, and the other workspaces a node is on are the project's glue.
        for c in graph.connections() {
            let (from, to) = (nodes.contains(&c.from.node), nodes.contains(&c.to.node));
            if from != to {
                report.dropped_cables.push(format!(
                    "{}.{} to {}.{}",
                    c.from.node, c.from.key, c.to.node, c.to.key
                ));
            }
        }
        for node in &nodes {
            let Some(n) = graph.get(*node) else { continue };
            let elsewhere: Vec<String> = n
                .workspaces
                .iter()
                .filter(|w| **w != id)
                .filter_map(|w| graph.workspace(*w))
                .map(|w| w.name.clone())
                .collect();
            if !elsewhere.is_empty() {
                report.shared.push((*node, elsewhere));
            }
        }

        let taken: HashSet<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let name = unique_file_name(&workspace.name, &taken);
        let path = dir.join(&name);
        workspace::write(&file, &path).map_err(ExportError::File)?;
        // The picture travels beside the file, so the card in the other project has one
        // before that project has ever rendered this workspace.
        if let Some(from) = self.thumbnail_path(id).filter(|p| p.is_file()) {
            let _ = std::fs::copy(from, dir.join(thumbnail_name(&name)));
        }
        report.file = path;
        Ok(report)
    }

    /// Take a `.ssw` into this project: fresh ids, its media copied in, and a report.
    ///
    /// Every node arrives through `insert_node` under an id from **this** project's counter
    /// and every cable inside the file is re-pointed to match, because a file's ids are
    /// stable once it is in a project and mean nothing outside one.
    ///
    /// Its media is looked for in the `assets/` beside the file — or in `../assets/` when
    /// the file is inside a `workspaces/` folder, which is where an export into a project
    /// put it — and copied in through [`Project::import_asset`], so the references land
    /// pointing at this project's own copies. One that cannot be found is reported and the
    /// option is left pointing at it: a `video` node with a missing file already says so.
    ///
    /// # Errors
    /// The file could not be read, or is not a workspace file.
    pub fn import_workspace(
        &self,
        file: &Path,
        graph: &mut Graph,
    ) -> Result<(WorkspaceId, ImportReport), LoadError> {
        let mut read = workspace::read(file)?;
        let beside = assets_beside(file);

        let mut report = ImportReport {
            name: if read.name.is_empty() {
                stem(file)
            } else {
                read.name.clone()
            },
            nodes: read.nodes.len(),
            ..ImportReport::default()
        };

        for node in &mut read.nodes {
            let Some(def) = nodes::find(&node.slug) else {
                continue;
            };
            for option in def.options {
                if !option.is_asset() {
                    continue;
                }
                let Some(value) = node.options.get(option.key).filter(|v| !v.is_empty()) else {
                    continue;
                };
                let source = match value.strip_prefix(ASSETS).and_then(|r| r.strip_prefix('/')) {
                    Some(name) => beside.join(name),
                    // A path from outside anybody's project means itself, here as anywhere.
                    None => PathBuf::from(value),
                };
                match self.import_asset(&source) {
                    Ok(reference) => {
                        report.assets.push(reference.clone());
                        node.options.insert(option.key.to_string(), reference);
                    }
                    Err(_) => report.missing.push(value.clone()),
                }
            }
        }
        // Its paintings, read out of the folder it came with. Held in memory from here, they
        // are written into this project's `assets/` by its next save like any painting.
        for reference in painting_references(&read) {
            if painting_path(&beside, &reference).is_some_and(|p| p.is_file()) {
                report.assets.push(reference);
            } else {
                report.missing.push(reference);
            }
        }
        let unread = read_paintings(&mut read, &beside);
        report.assets.sort();
        report.assets.dedup();

        let workspace = graph.add_workspace(report.name.clone(), read.kind);
        graph.set_layout(workspace, read.layout);
        graph.set_blurb(workspace, read.blurb.clone());
        report.warnings = read.insert_into(graph, workspace, Ids::Fresh);
        report.warnings.extend(unread);
        // An Output that arrives under a name another already goes out under takes a
        // ` copy`, as a pasted one does.
        let arrived: Vec<NodeId> = graph.on_workspace(workspace).map(|(id, _)| id).collect();
        nodes::output::settle_names(graph, &arrived);
        Ok((workspace, report))
    }

    /// Delete one asset and everything derived from it.
    ///
    /// **Refused while anything references it**, because a node pointing at a file the
    /// project no longer holds is a silent break: the usage list on the card is the answer
    /// to what to do about it. `cache/` goes with the file, since a transcode of something
    /// that is gone is bytes nobody will ever ask for.
    ///
    /// # Errors
    /// Something references it, the reference is not one of this project's assets, or the
    /// file could not be deleted.
    pub fn remove_asset(&self, graph: &Graph, reference: &str) -> Result<(), String> {
        let users = asset_users(graph);
        if let Some(using) = users.get(reference).filter(|u| !u.is_empty()) {
            return Err(format!(
                "{reference} is used by {}",
                using
                    .iter()
                    .map(|(node, key)| {
                        let slug = graph.get(*node).map_or("node", |n| n.def.slug);
                        format!("{slug}{node}.{key}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if reference
            .strip_prefix(ASSETS)
            .and_then(|r| r.strip_prefix('/'))
            .is_none()
        {
            return Err(format!("{reference} is not one of this project's assets"));
        }
        let path = self.resolve(reference);
        // Before the file goes: what was derived from it is named after it, and the name
        // needs the file to still be there.
        let derived = crate::video::clip::cache_entries(&self.cache_dir(), &path);
        std::fs::remove_file(&path).map_err(|e| format!("{reference}: {e}"))?;
        for entry in derived {
            let _ = std::fs::remove_file(entry);
        }
        Ok(())
    }

    /// Copy a file into `assets/` and hand back the reference that names it.
    ///
    /// Every way a file reaches a node goes through here: a drop on the window, the file
    /// button's dialog, an import on the project tab. The reference is
    /// `assets/<name>` with forward slashes, so the folder can be moved, zipped and sent and
    /// every reference inside it still resolves.
    ///
    /// A file already in this project's `assets/` is not copied again, and neither is one
    /// whose bytes are already there under another name: the same content is one asset. A
    /// name already taken gets a numeric suffix, so `clip.webm` from two folders becomes
    /// `clip.webm` and `clip-2.webm`.
    pub fn import_asset(&self, source: &Path) -> std::io::Result<String> {
        let dir = self.root.join(ASSETS);
        let name = file_name(source);
        if name.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} names no file", source.display()),
            ));
        }
        if source.parent().is_some_and(|p| same_folder(p, &dir)) {
            return Ok(reference(&name));
        }

        let size = std::fs::metadata(source)?.len();
        let print = fingerprint(source)?;
        if let Some(existing) = self.same_content(&dir, size, print) {
            return Ok(reference(&existing));
        }

        std::fs::create_dir_all(&dir)?;
        let name = unique_asset_name(&dir, &name);
        std::fs::copy(source, dir.join(&name))?;
        Ok(reference(&name))
    }

    /// Whether this project is the one in `root`, compared as folders rather than as
    /// spellings of a path.
    pub fn is_at(&self, root: &Path) -> bool {
        same_folder(&self.root, root)
    }

    /// The name of the asset holding these bytes already, if one does.
    ///
    /// Length is the cheap half of the comparison: a file of another size cannot be the same
    /// content, so only a candidate that could be is read.
    fn same_content(&self, dir: &Path, size: u64, print: u64) -> Option<String> {
        for asset in self.assets() {
            if asset.size != size {
                continue;
            }
            if fingerprint(&dir.join(&asset.name)).is_ok_and(|p| p == print) {
                return Some(asset.name);
            }
        }
        None
    }

    /// Where a reference names a file on this machine.
    ///
    /// A reference under `assets/` is joined onto the root natively, which is what makes a
    /// forward-slash reference mean the same thing wherever the folder is. Anything else —
    /// an absolute path somebody typed into the option by hand — is its own answer.
    pub fn resolve(&self, reference: &str) -> PathBuf {
        let Some(rest) = reference
            .strip_prefix(ASSETS)
            .and_then(|r| r.strip_prefix('/'))
        else {
            return PathBuf::from(reference);
        };
        let mut path = self.root.join(ASSETS);
        for segment in rest.split('/') {
            path.push(segment);
        }
        path
    }

    /// Where the files derived from this project's media go: `cache/` beside `assets/`.
    ///
    /// In the project rather than in `$XDG_CACHE_HOME` because a transcode is minutes of
    /// hardware encoding, and a folder that carries its own opens on another machine at
    /// once. It is still a cache: nothing lists it, Save never touches it, and deleting it
    /// costs a re-import and nothing else.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(CACHE)
    }

    /// Every file in `assets/`, used or not, in name order. An empty list where there is no
    /// folder yet, which is a project nobody has imported into.
    pub fn assets(&self) -> Vec<AssetInfo> {
        let Ok(entries) = std::fs::read_dir(self.root.join(ASSETS)) else {
            return Vec::new();
        };
        let mut out: Vec<AssetInfo> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                AssetInfo {
                    size: e.metadata().map_or(0, |m| m.len()),
                    reference: reference(&name),
                    name,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// An autosave worth offering back: what [`Project::autosave_to_offer`] found and
/// [`Project::recover`] takes.
#[derive(Debug)]
pub struct Recovered {
    project: Project,
    graph: Graph,
    /// When it was written.
    pub when: std::time::SystemTime,
}

/// What an export wrote, and what it left behind.
///
/// The glue is about *this* rig — a cable to a node on another workspace, the other
/// workspaces a shared node is on, a MIDI binding — so none of it travels, and the report is
/// what makes that a statement rather than a silence. It goes to the status line and to the
/// log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    /// The `.ssw` that was written.
    pub file: PathBuf,
    /// Cables with one end on a node that is not on this workspace, as `from.key to to.key`.
    pub dropped_cables: Vec<String>,
    /// Each node shown here *and* elsewhere, with the other workspaces' names.
    pub shared: Vec<(NodeId, Vec<String>)>,
    /// The media copied under `assets/` beside the file, as references.
    pub assets: Vec<String>,
    /// References this project could not resolve to a file, so nothing was copied for them.
    pub missing: Vec<String>,
    /// MIDI bindings onto a node being exported. The map is the project's and no `.ssw`
    /// holds any of it, so every one of them stays behind.
    pub midi: usize,
}

impl std::fmt::Display for ExportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exported {}",
            self.file.file_name().map_or_else(
                || self.file.display().to_string(),
                |n| n.to_string_lossy().into_owned()
            )
        )?;
        if !self.assets.is_empty() {
            write!(f, " with {} asset(s)", self.assets.len())?;
        }
        let mut left = Vec::new();
        if !self.dropped_cables.is_empty() {
            left.push(format!("{} cable(s) elsewhere", self.dropped_cables.len()));
        }
        if !self.shared.is_empty() {
            left.push(format!("{} shared node(s)", self.shared.len()));
        }
        if !self.missing.is_empty() {
            left.push(format!("{} missing asset(s)", self.missing.len()));
        }
        if self.midi > 0 {
            left.push(format!("{} MIDI binding(s)", self.midi));
        }
        if !left.is_empty() {
            write!(f, "; left behind: {}", left.join(", "))?;
        }
        Ok(())
    }
}

/// Why a workspace could not be exported.
#[derive(Debug)]
pub enum ExportError {
    NoSuchWorkspace(WorkspaceId),
    /// Everything in a workspace of this kind is a reference into other workspaces, so it
    /// means nothing on its own and travels with the project instead. No kind is like that
    /// yet; a timeline will be.
    NotPortable(WorkspaceKind),
    File(LoadError),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchWorkspace(id) => write!(f, "no such workspace: {id}"),
            Self::NotPortable(kind) => write!(
                f,
                "a {kind:?} workspace is references into other workspaces, so it cannot be \
                 exported on its own; it travels with the project"
            ),
            Self::File(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// What an import brought in, and what it could not find.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// The name the imported workspace arrived under.
    pub name: String,
    /// How many nodes the file described.
    pub nodes: usize,
    /// The media copied into this project, as references into it.
    pub assets: Vec<String>,
    /// References the `assets/` beside the file did not hold. The option keeps the
    /// reference it had — a `video` node with a missing file already shows its error.
    pub missing: Vec<String>,
    /// What the file itself lost on the way in: an unknown node kind, a refused cable.
    pub warnings: Vec<LoadWarning>,
}

impl std::fmt::Display for ImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "imported {} ({} nodes)", self.name, self.nodes)?;
        if !self.assets.is_empty() {
            write!(f, " with {} asset(s)", self.assets.len())?;
        }
        if !self.missing.is_empty() {
            write!(f, "; {} asset(s) not found", self.missing.len())?;
        }
        if !self.warnings.is_empty() {
            write!(
                f,
                "; {} warning(s) — {}",
                self.warnings.len(),
                self.warnings[0]
            )?;
        }
        Ok(())
    }
}

/// One file in `assets/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    pub name: String,
    /// Bytes on disk.
    pub size: u64,
    /// What a node's option holds to name it: `assets/<name>`.
    pub reference: String,
}

/// Which nodes reference which asset, by walking every node's `Asset` options.
///
/// The node declares which of its options name files, so the walk asks rather than infers.
/// Reading it off the shape — an option with no choices holds a path — is what this did
/// before `OptionKind`, and it made every free-running string a file whether it was one or
/// not.
pub fn asset_users(graph: &Graph) -> BTreeMap<String, Vec<(NodeId, &'static str)>> {
    let mut out: BTreeMap<String, Vec<(NodeId, &'static str)>> = BTreeMap::new();
    for (id, node) in graph.iter() {
        // A painting's picture file is used by the node that painted it, which is what keeps
        // its card from offering to delete it.
        for (key, value) in &node.values {
            if let Some(painting) = value.painting() {
                out.entry(painting.file().to_string())
                    .or_default()
                    .push((id, *key));
            }
        }
        for option in node.def.options {
            if !option.is_asset() {
                continue;
            }
            match node.options.get(option.key) {
                Some(value) if !value.is_empty() => {
                    out.entry(value.clone()).or_default().push((id, option.key));
                }
                _ => {}
            }
        }
    }
    out
}

impl Assets for Project {
    fn resolve(&self, reference: &str) -> PathBuf {
        Project::resolve(self, reference)
    }

    fn cache_dir(&self) -> PathBuf {
        Project::cache_dir(self)
    }
}

/// Where a reference points, as a value rather than as a borrow of the project.
///
/// The synth resolves asset references on a thread of its own and the project is the
/// editor's, so what crosses is this: the root, which is the whole of what
/// [`Project::resolve`] and [`Project::cache_dir`] read. Sent again whenever the project
/// being edited changes, which is the only time it can be wrong.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetPaths {
    root: PathBuf,
}

impl Project {
    /// This project's asset paths, for the synth.
    pub fn asset_paths(&self) -> AssetPaths {
        AssetPaths {
            root: self.root.clone(),
        }
    }
}

impl Assets for AssetPaths {
    fn resolve(&self, reference: &str) -> PathBuf {
        let Some(rest) = reference
            .strip_prefix(ASSETS)
            .and_then(|r| r.strip_prefix('/'))
        else {
            return PathBuf::from(reference);
        };
        let mut path = self.root.join(ASSETS);
        for segment in rest.split('/') {
            path.push(segment);
        }
        path
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join(CACHE)
    }
}

/// Every painting a workspace file names, by the reference it names it with.
fn painting_references(file: &WorkspaceFile) -> Vec<String> {
    file.nodes
        .iter()
        .flat_map(|n| n.values.values())
        .filter_map(crate::graph::Value::painting)
        .map(|p| p.file().to_string())
        .collect()
}

/// Where a painting's reference names a file in a folder of assets: `assets/<name>` with a
/// plain file name, and nothing else — a painting is never a path somebody typed.
fn painting_path(assets: &Path, reference: &str) -> Option<PathBuf> {
    let name = reference
        .strip_prefix(ASSETS)
        .and_then(|r| r.strip_prefix('/'))?;
    let plain = !name.is_empty()
        && !name.contains(['/', '\\'])
        && name != ".."
        && name != "."
        && Path::new(name).file_name().is_some_and(|n| n == name);
    plain.then(|| assets.join(name))
}

/// Whether a reference has the name a save gives a painting, `assets/painting-<print>.png`:
/// the only files under `assets/` a save ever deletes, so a hand-edited workspace file naming
/// somebody's clip as a painting cannot have a save bin the clip.
fn is_painting_name(reference: &str) -> bool {
    reference
        .strip_prefix(ASSETS)
        .and_then(|r| r.strip_prefix("/painting-"))
        .and_then(|r| r.strip_suffix(".png"))
        .is_some_and(|print| print.len() == 16 && print.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Read every painting a workspace file names out of `assets`, in place: from here each one is
/// its pixels. One that cannot be read stays the name it was, opens as a blank canvas, and says
/// so.
fn read_paintings(file: &mut WorkspaceFile, assets: &Path) -> Vec<LoadWarning> {
    let mut warnings = Vec::new();
    for node in &mut file.nodes {
        for value in node.values.values_mut() {
            let Some(painting) = value.painting().filter(|p| !p.is_read()) else {
                continue;
            };
            let reference = painting.file().to_string();
            let read = painting_path(assets, &reference)
                .ok_or_else(|| "not a painting in assets/".to_string())
                .and_then(|path| crate::video::png::read(&path));
            match read {
                Ok(image) if image.width > 0 && image.height > 0 => {
                    *value = crate::graph::Value::Painting(crate::graph::Painting::new(
                        image.width,
                        image.height,
                        image.rgba,
                        0,
                    ));
                }
                Ok(_) => warnings.push(LoadWarning::UnreadPainting {
                    id: node.id,
                    file: reference,
                    reason: "a picture of no size".to_string(),
                }),
                Err(reason) => warnings.push(LoadWarning::UnreadPainting {
                    id: node.id,
                    file: reference,
                    reason,
                }),
            }
        }
    }
    warnings
}

/// Write every painting in the graph that `assets` does not hold yet, and hand back the
/// reference of every painting it names. One still unread is named and not written: its file
/// is wherever it was read from.
fn write_paintings(graph: &Graph, assets: &Path) -> std::io::Result<BTreeSet<String>> {
    let mut named = BTreeSet::new();
    for (_, node) in graph.iter() {
        for value in node.values.values() {
            let Some(painting) = value.painting() else {
                continue;
            };
            if painting.is_read() {
                write_painting(painting, assets)?;
            }
            named.insert(painting.file().to_string());
        }
    }
    if !named.is_empty() && assets.is_dir() {
        sync_dir(assets)?;
    }
    Ok(named)
}

/// One painting into `assets`, unless it is there already — its name is its content, so a
/// file of that name is that picture. Written beside itself, synced, then renamed into place,
/// so no reader ever finds half of one.
fn write_painting(painting: &crate::graph::Painting, assets: &Path) -> std::io::Result<()> {
    let Some((width, height, rgba)) = painting.pixels() else {
        return Ok(());
    };
    let Some(path) = painting_path(assets, painting.file()) else {
        return Ok(());
    };
    if path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(assets)?;
    let staged = path.with_extension("png.tmp");
    let image = crate::video::png::Image {
        width,
        height,
        rgba: rgba.to_vec(),
    };
    let fail = |reason: String| std::io::Error::other(format!("{}: {reason}", path.display()));
    crate::video::png::write(&staged, &image).map_err(fail)?;
    std::fs::File::open(&staged)?.sync_all()?;
    std::fs::rename(&staged, &path)
}

/// The reference naming a file in `assets/`. Forward slashes, always: the reference is
/// written into a file that has to mean the same thing on another machine.
fn reference(name: &str) -> String {
    format!("{ASSETS}/{name}")
}

/// FNV-1a over a file's bytes, read a block at a time so a long clip is not held in memory.
fn fingerprint(path: &Path) -> std::io::Result<u64> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hash);
        }
        for b in &buffer[..n] {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Are these the same folder? Compared after resolving both, so a relative root and an
/// absolute drop of a file already inside it are the same answer.
fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Is `path` the folder `dir` or somewhere under it? Compared after resolving both: `path` by
/// the longest part of it that is there, since it need not exist yet, and the rest of it by
/// its names, a `..` stepping up.
fn inside(path: &Path, dir: &Path) -> bool {
    use std::path::Component;
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let parts: Vec<Component> = path.components().collect();
    for there in (1..=parts.len()).rev() {
        let Ok(mut resolved) = parts[..there].iter().collect::<PathBuf>().canonicalize() else {
            continue;
        };
        for part in &parts[there..] {
            match part {
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::CurDir => {}
                name => resolved.push(name),
            }
        }
        return resolved.starts_with(&dir);
    }
    path.starts_with(&dir)
}

/// A file name that nothing in `assets/` already has. `clip.webm` becomes `clip-2.webm`.
fn unique_asset_name(dir: &Path, name: &str) -> String {
    let mut candidate = name.to_string();
    let mut n = 1;
    while dir.join(&candidate).exists() {
        n += 1;
        candidate = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => format!("{stem}-{n}.{ext}"),
            _ => format!("{name}-{n}"),
        };
    }
    candidate
}

/// Bring a loose `.ssw` into a project of its own, named after the file, under `into`.
///
/// The one case where "there is no file that is not in a project" has to be said by the
/// program: someone hands you a workspace, and it becomes something you can open.
pub fn import(file: &Path, into: &Path) -> Result<(Project, Graph, ImportReport), LoadError> {
    let base = sanitize(&stem(file));
    let mut root = into.join(&base);
    let mut n = 1;
    while root.exists() {
        n += 1;
        root = into.join(format!("{base}-{n}"));
    }
    let mut project = Project::new(root);
    let mut graph = Graph::new();
    // Through the same import the project tab and a dropped file use, so a loose file
    // arrives with its media copied in and its ids remapped exactly as any other does.
    let empty = graph.default_workspace();
    let (id, report) = project.import_workspace(file, &mut graph)?;
    // The workspace a new graph starts with is not what anybody asked for, and it holds
    // nothing: the import put every node on its own.
    graph.remove_workspace(empty);
    project.session.open.insert(id);
    project.session.active = Active::Workspace(id);
    project.save(&graph)?;
    Ok((project, graph, report))
}

/// The project a path is inside, if it is inside one. A file in `workspaces/` opens the
/// project that holds it.
pub fn enclosing(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(here) = dir {
        if Project::is_project(here) {
            return Some(here.to_path_buf());
        }
        dir = here.parent();
    }
    None
}

/// Where projects live when the `projects_dir` preference says nothing: `supersilvia` in the
/// person's documents folder — `XDG_DOCUMENTS_DIR`, falling back to `$HOME/Documents`, on
/// Linux, and `~/Documents` on macOS. See [`crate::platform::dirs`] and
/// [`crate::preferences::Preferences::projects_dir`].
///
/// `None` means there is nowhere to put one, which is a machine with no `$HOME`.
pub fn default_projects_dir() -> Option<PathBuf> {
    Some(crate::platform::dirs::documents()?.join("supersilvia"))
}

/// Why the projects folder cannot be used, or `None` where it can: it is read, after being made
/// where `make` is set and it is not there. Without `make`, a folder that is not there yet is
/// no problem, since it is made when a project first needs it.
///
/// Nothing falls back to another folder: what failed is said, with the folder's whole path
/// and the system's reason, and on a Mac what to allow where it was not allowed.
pub fn projects_dir_problem(dir: &Path, make: bool) -> Option<String> {
    if make && let Err(e) = std::fs::create_dir_all(dir) {
        return Some(folder_problem("make", dir, &e));
    }
    match std::fs::read_dir(dir) {
        Ok(_) => None,
        Err(e) if !make && e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(folder_problem("read", dir, &e)),
    }
}

/// `could not read the projects folder /x: Permission denied`, and what to allow where the
/// machine has something to say about it.
fn folder_problem(verb: &str, dir: &Path, e: &std::io::Error) -> String {
    let hint = match crate::platform::dirs::DENIED_HINT {
        Some(hint) if e.kind() == std::io::ErrorKind::PermissionDenied => format!(" — {hint}"),
        _ => String::new(),
    };
    format!(
        "could not {verb} the projects folder {}: {e}{hint}",
        dir.display()
    )
}

/// The name New project offers: `Untitled`, or the first `Untitled N` from 2 that is not in
/// `dir` already.
pub fn next_untitled(dir: &Path) -> String {
    let mut name = "Untitled".to_owned();
    let mut n = 1_u64;
    while dir.join(&name).exists() {
        n += 1;
        name = format!("Untitled {n}");
    }
    name
}

/// The name Save as… offers a project called `name`: `name` where it is not in `dir`, and
/// otherwise the next free count after it — `Friday 2` for `Friday`, and `Friday 3` rather than
/// `Friday 2 2` for `Friday 2`.
pub fn next_copy(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_owned();
    }
    let (base, from) = match name.rsplit_once(' ') {
        Some((base, n)) if !base.is_empty() => {
            n.parse::<u32>().map_or((name, 1), |n| (base, u64::from(n)))
        }
        _ => (name, 1),
    };
    let mut n = from;
    loop {
        n += 1;
        let candidate = format!("{base} {n}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
}

/// The folder a new project called `name` is made in, inside `dir`, or why it cannot be: no
/// name, a character a folder's name cannot hold on this machine, or a name already taken
/// there. Spaces at either end are not part of the name.
///
/// # Errors
/// The reason, as the New project window says it.
pub fn new_project_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the project a name.".to_owned());
    }
    if let Some(c) = name
        .chars()
        .find(|c| crate::platform::dirs::NOT_IN_NAMES.contains(c))
    {
        return Err(format!("A name cannot hold {}", c.escape_default()));
    }
    if name == "." || name == ".." {
        return Err(format!("A name cannot be {name}"));
    }
    let root = dir.join(name);
    if root.exists() {
        return Err(format!("There is already a folder called {name} here."));
    }
    Ok(root)
}

/// A workspace name as a file name: safe, short, and never empty.
///
/// Anything that is not a letter, a digit, a space or one of `-_.` becomes `-`, since a
/// name is typed by a person and a path separator in one would write somewhere else.
fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().count() >= NAME_LIMIT {
            break;
        }
    }
    let out = out.trim().trim_matches('.').trim().to_string();
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

/// A file name derived from a workspace name, with a numeric suffix where the plain one is
/// already spoken for. Two workspaces called Tunnel get `Tunnel.ssw` and `Tunnel-2.ssw`.
fn unique_file_name(name: &str, taken: &HashSet<String>) -> String {
    let base = sanitize(name);
    let mut candidate = format!("{base}.{}", workspace::EXTENSION);
    let mut n = 1;
    while taken.contains(&candidate) {
        n += 1;
        candidate = format!("{base}-{n}.{}", workspace::EXTENSION);
    }
    candidate
}

/// Where a workspace file's own media sits: the `assets/` beside it, or `../assets/` where
/// the file is inside a `workspaces/` folder, which is where an export into a project put
/// it. Both of them mean *the project this file came out of*.
fn assets_beside(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    if dir.file_name().is_some_and(|n| n == WORKSPACES)
        && let Some(root) = dir.parent()
    {
        return root.join(ASSETS);
    }
    dir.join(ASSETS)
}

/// Copy one file into a folder of assets, and hand back the name it landed under.
///
/// The same content already there under any name is that asset, and a name already taken by
/// something else gets a numeric suffix — the same two rules [`Project::import_asset`]
/// follows, because an export is an import in the other direction.
fn copy_asset(source: &Path, dir: &Path) -> std::io::Result<String> {
    let name = file_name(source);
    if name.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} names no file", source.display()),
        ));
    }
    let size = std::fs::metadata(source)?.len();
    let print = fingerprint(source)?;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            if entry.metadata().is_ok_and(|m| m.len() == size)
                && fingerprint(&entry.path()).is_ok_and(|p| p == print)
            {
                return Ok(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    std::fs::create_dir_all(dir)?;
    let name = unique_asset_name(dir, &name);
    std::fs::copy(source, dir.join(&name))?;
    Ok(name)
}

/// Put a committed save's workspace files in place.
///
/// Each `<file>.<save>.tmp` of the listed files is renamed over its file, then each file the
/// save removed is deleted with its picture, then every other temporary workspace file —
/// what a save that never committed left — is deleted.
fn finish<'a>(
    dir: &Path,
    save: u64,
    listed: impl Iterator<Item = &'a str>,
    removed: &[String],
    after_each: &mut dyn FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let listed: HashSet<&str> = listed.collect();
    for file in &listed {
        let staged = dir.join(staged_name(file, save));
        if staged.exists() {
            std::fs::rename(staged, dir.join(file))?;
            after_each()?;
        }
    }
    for file in removed.iter().filter(|f| !listed.contains(f.as_str())) {
        let _ = std::fs::remove_file(dir.join(file));
        let _ = std::fs::remove_file(dir.join(thumbnail_name(file)));
    }
    for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        if is_staged(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// Where a save numbered `save` writes a workspace file before it is committed.
fn staged_name(file: &str, save: u64) -> String {
    format!("{file}.{save}.tmp")
}

/// Is this a name [`staged_name`] makes?
fn is_staged(name: &str) -> bool {
    name.strip_suffix(".tmp")
        .and_then(|rest| rest.rsplit_once('.'))
        .is_some_and(|(file, save)| {
            file.ends_with(&format!(".{}", workspace::EXTENSION)) && save.parse::<u64>().is_ok()
        })
}

/// Write a file whole and wait until it is on the disk.
fn write_synced(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Wait until a folder's entries — a rename, a new file — are on the disk.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Delete a folder and everything in it; one that is not there is already gone.
pub fn remove_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// A workspace file's name, as the picture beside it: `Tunnel.ssw` becomes `Tunnel.png`.
fn thumbnail_name(file: &str) -> String {
    let stem = file
        .rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_string();
    format!("{stem}.png")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Copy a folder and everything under it but the entries of its own named in `skip`. What
/// Save as does before it saves, skipping the autosave and the cache.
fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if skip.iter().any(|s| entry.file_name() == *s) {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, &[])?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
