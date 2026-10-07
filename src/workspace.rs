// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing `.ssw` workspace files.
//!
//! One file holds one workspace: its name, its kind, the nodes it writes and every
//! connection with both ends among them. It is self-contained and portable, which is what
//! makes it the unit that leaves a project by export and enters one by import.
//!
//! The file is a `WorkspaceFile`, not a `Graph`. Two reasons, and both matter more than the
//! duplication costs:
//!
//! - `Graph` carries derived caches (`OnceCell`, `RefCell`) that have no business in a file,
//!   and port lists copied from the node definitions at insertion.
//! - **Ports are not saved.** They are rebuilt from the registry on load, so a node whose
//!   definition gained a port since the file was written opens with that port present. Saving
//!   them would freeze every node's shape at the moment somebody hit Save.
//!
//! What is saved is what the user actually chose: which nodes, where, with what control and
//! option values, wired how.

use crate::graph::{
    ControlValue, Graph, LayoutMode, NodeId, PortRef, Workspace, WorkspaceId, WorkspaceKind,
};
use crate::nodes;
use emath::Pos2;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// The `format` field. A file that does not carry this is not ours.
const FORMAT: &str = "supersilvia-workspace";
/// Bumped only for a change old readers cannot cope with.
const VERSION: u32 = 1;

/// The file extension, without the dot. A `.ssw` is always one workspace; the project it
/// may be part of is a `.ssp`.
pub const EXTENSION: &str = "ssw";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub format: String,
    pub version: u32,
    /// The workspace's name, which is what its tab will say. A file names itself, so an
    /// exported one arrives as the workspace it was.
    pub name: String,
    pub kind: WorkspaceKind,
    /// A line about what this workspace is for. In the workspace's own file rather than in
    /// the manifest, because it describes the workspace and travels with an export.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blurb: String,
    /// How this workspace is navigated.
    pub layout: LayoutMode,
    pub nodes: Vec<SavedNode>,
    pub connections: Vec<SavedConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedNode {
    pub id: NodeId,
    pub slug: String,
    pub pos: Pos2,
    /// Ordered so a saved file diffs cleanly against the next one.
    pub controls: BTreeMap<String, ControlValue>,
    pub options: BTreeMap<String, String>,
    /// This node's own state: a control's range where it disagrees with its definition,
    /// keyed by the input port, and whatever the definition declares as a `ValueDef`. Empty
    /// for almost every node, so it is left out rather than written as `{}` — the two
    /// attributes are one pair and go together.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, crate::graph::Value>,
    /// Left out entirely when the node is expanded, which is most of them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// A width a hand dragged, for the kinds that let one be. Left out entirely for a node
    /// still the width its kind asks for, which is almost all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// A text box's height a hand dragged. Left out for a box still the height its kind
    /// declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPort {
    pub node: NodeId,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub from: SavedPort,
    pub to: SavedPort,
}

impl SavedConnection {
    /// One live connection as the file writes it.
    pub fn of(c: &crate::graph::Connection) -> Self {
        Self {
            from: SavedPort {
                node: c.from.node,
                key: c.from.key.to_string(),
            },
            to: SavedPort {
                node: c.to.node,
                key: c.to.key.to_string(),
            },
        }
    }
}

/// A file that could not be read at all. What is on screen is left alone.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// The `format` field is missing or names something else.
    NotAWorkspace(String),
    /// The same, for a project file.
    NotAProject(String),
    /// The project file names a workspace it does not list. Nothing can be placed on it, so
    /// the project does not open at all.
    UnknownWorkspace(WorkspaceId),
    /// Written by a newer supersilvia than this one.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "not valid JSON: {e}"),
            Self::NotAWorkspace(s) => write!(f, "not a supersilvia workspace (format: {s:?})"),
            Self::NotAProject(s) => write!(f, "not a supersilvia project (format: {s:?})"),
            Self::UnknownWorkspace(w) => {
                write!(f, "the project names workspace {w}, which it does not list")
            }
            Self::UnsupportedVersion(v) => {
                write!(f, "file version {v}, this build reads {VERSION}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// Something dropped while loading a file that was otherwise fine.
///
/// The same shape as `compile::Diagnostic`, and for the same reason: the file opens and the
/// show goes on, but nothing here is silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadWarning {
    /// No node definition with this slug. The node and its cables are dropped.
    UnknownNodeKind { id: NodeId, slug: String },
    /// Another file already has a node under this id. The first keeps it, and this one
    /// arrives under `fresh` with every cable of its own file.
    DuplicateId {
        id: NodeId,
        slug: String,
        fresh: NodeId,
    },
    /// The node exists but no longer has this input.
    UnknownControl { id: NodeId, key: String },
    /// The node exists but no longer has this option.
    UnknownOption { id: NodeId, key: String },
    /// The value cannot be stored under that key: a color where the definition declares a
    /// number, or an option value this build does not offer. The default is kept.
    WrongValue { id: NodeId, key: String },
    /// The port is gone, or the graph refused the edge.
    DroppedConnection { reason: String },
    /// The project file lists a workspace whose file could not be read. It is dropped.
    MissingWorkspace { file: String, reason: String },
    /// A file in `workspaces/` the project file does not list, taken in with fresh ids.
    AdoptedWorkspace { file: String },
    /// A file in `workspaces/` the project file does not list and that could not be read.
    UnreadableWorkspace { file: String, reason: String },
    /// A painting whose picture file could not be read. The node opens on a blank canvas
    /// and keeps the name, so a later save does not forget which file it was.
    UnreadPainting {
        id: NodeId,
        file: String,
        reason: String,
    },
}

impl std::fmt::Display for LoadWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNodeKind { id, slug } => {
                write!(f, "node {id}: no node kind {slug:?}, dropped")
            }
            Self::DuplicateId { id, slug, fresh } => {
                write!(
                    f,
                    "node {id}: {slug:?} shares its id with another, became {fresh}"
                )
            }
            Self::UnknownControl { id, key } => write!(f, "node {id}: no control {key:?}"),
            Self::UnknownOption { id, key } => write!(f, "node {id}: no option {key:?}"),
            Self::WrongValue { id, key } => {
                write!(
                    f,
                    "node {id}: {key:?} has a value of the wrong kind, kept the default"
                )
            }
            Self::DroppedConnection { reason } => write!(f, "dropped a connection: {reason}"),
            Self::MissingWorkspace { file, reason } => {
                write!(f, "workspace file {file:?} is missing: {reason}")
            }
            Self::AdoptedWorkspace { file } => {
                write!(f, "adopted {file:?}, which the project did not list")
            }
            Self::UnreadableWorkspace { file, reason } => {
                write!(f, "could not read {file:?}: {reason}")
            }
            Self::UnreadPainting { id, file, reason } => {
                write!(
                    f,
                    "node {id}: could not read its painting {file:?}, opened blank: {reason}"
                )
            }
        }
    }
}

impl WorkspaceFile {
    /// The nodes named, and every connection with both ends among them.
    ///
    /// A node is written by exactly one workspace of the set it is on; which one is the
    /// project's business, so the caller names the nodes rather than this deciding.
    pub fn of(graph: &Graph, workspace: &Workspace, nodes: &BTreeSet<NodeId>) -> Self {
        let mut saved: Vec<SavedNode> = graph
            .iter()
            .filter(|(id, _)| nodes.contains(id))
            .map(|(id, node)| SavedNode {
                id,
                slug: node.def.slug.to_string(),
                pos: node.pos,
                collapsed: node.collapsed,
                width: node.dragged_width,
                height: node.dragged_height,
                controls: node
                    .controls
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), *v))
                    .collect(),
                values: node
                    .values
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), v.clone()))
                    .collect(),
                options: node
                    .options
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), v.clone()))
                    .collect(),
            })
            .collect();
        saved.sort_by_key(|n| n.id);

        let connections = graph
            .connections()
            .iter()
            .filter(|c| nodes.contains(&c.from.node) && nodes.contains(&c.to.node))
            .map(SavedConnection::of)
            .collect();

        Self {
            format: FORMAT.to_string(),
            version: VERSION,
            name: workspace.name.clone(),
            kind: workspace.kind,
            blurb: workspace.blurb.clone(),
            layout: workspace.layout,
            nodes: saved,
            connections,
        }
    }

    /// Put this file's nodes and cables into a graph, on `workspace`.
    ///
    /// **`Ids::Keep`** is what a project does: ids are unique project-wide, so a file
    /// round-trips exactly and the WGSL function names in a saved graph still describe the
    /// reopened one. **`Ids::Fresh`** is what adopting a stray file and importing one do:
    /// the ids come from another counter, so every node takes a new one and every cable
    /// inside the file is re-pointed to match.
    ///
    /// A bulk change: the effective types are settled once, after the last cable, rather
    /// than after every node and cable. Where the caller already opened one — a project
    /// reading every file and then its glue — the settling is the caller's.
    pub fn insert_into(
        self,
        graph: &mut Graph,
        workspace: WorkspaceId,
        ids: Ids,
    ) -> Vec<LoadWarning> {
        let mut warnings = Vec::new();
        let settles = !graph.in_bulk();
        graph.begin_bulk();
        // The id a saved node ended up under, which is its own unless they were remapped.
        let mut placed: HashMap<NodeId, NodeId> = HashMap::new();

        for saved in self.nodes {
            let Some(def) = nodes::find(&saved.slug) else {
                warnings.push(LoadWarning::UnknownNodeKind {
                    id: saved.id,
                    slug: saved.slug,
                });
                continue;
            };
            let fresh = NodeId(graph.next_node_id() + 1);
            let id = match ids {
                // An id another file already holds is a project that disagrees with itself.
                // The node that arrived first keeps it; this one takes a fresh id, which the
                // cables inside this file follow, and says so.
                Ids::Keep if graph.get(saved.id).is_some() => {
                    warnings.push(LoadWarning::DuplicateId {
                        id: saved.id,
                        slug: saved.slug.clone(),
                        fresh,
                    });
                    fresh
                }
                Ids::Keep => saved.id,
                Ids::Fresh => fresh,
            };
            placed.insert(saved.id, id);

            // Ports come from the definition as it is now, never from the file.
            let (inputs, outputs) = def.port_defs();
            graph.insert_node(
                id,
                crate::graph::Node {
                    def,
                    pos: saved.pos,
                    workspaces: BTreeSet::from([workspace]),
                    inputs,
                    outputs,
                    dragged_width: saved.width,
                    dragged_height: saved.height,
                    controls: HashMap::default(),
                    values: BTreeMap::default(),
                    options: BTreeMap::default(),
                    collapsed: saved.collapsed,
                },
            );

            // Start from the definition's defaults, then lay the file's values over them, so
            // a control added since the file was written gets its default rather than nothing.
            nodes::apply_defaults(graph, id);

            let node = graph.get_mut(id).expect("just inserted");
            // Options first, since a control's range can follow them: an Offset reaches one
            // period either way, and the period is the node's Repeat.
            for (key, value) in saved.options {
                match def.options.iter().find(|o| o.key == key) {
                    Some(option) if nodes::option_is_valid(def, option.key, &value) => {
                        node.options.insert(option.key, value);
                    }
                    Some(_) => warnings.push(LoadWarning::WrongValue { id, key }),
                    None => warnings.push(LoadWarning::UnknownOption { id, key }),
                }
            }
            // Ranges before controls, so a control is fitted to the range this node has
            // rather than to the one it is about to stop having.
            for (key, value) in saved.values {
                let Some(range) = value.range() else {
                    // Anything that is not a range is a declared value: a note's prose, and
                    // later a pad's position or a grid's pattern. It is checked against the
                    // definition's own list rather than against the ports.
                    match def.value(&key) {
                        Some(v) => {
                            node.values.insert(v.key, value);
                        }
                        None => warnings.push(LoadWarning::UnknownControl { id, key }),
                    }
                    continue;
                };
                match nodes::coerce_range(def, &key, range) {
                    Some((key, range)) => {
                        node.values.insert(key, crate::graph::Value::Range(range));
                    }
                    // `NodeDef::input` searches the hidden controls too. A hidden control
                    // is a control: it is stored on the node and saved with the file, so a
                    // loader that only looked at the ports would drop every one of them —
                    // an audio node's band tuning, silently, on every reload.
                    None if def.input(&key).is_some() => {
                        warnings.push(LoadWarning::WrongValue { id, key });
                    }
                    None => warnings.push(LoadWarning::UnknownControl { id, key }),
                }
            }
            // An Offset last, since its range follows the node's other controls too: a
            // Euclidean Rhythm's period is where its lanes' lengths meet.
            let (offsets, controls): (Vec<_>, Vec<_>) =
                saved.controls.into_iter().partition(|(key, _)| {
                    key == nodes::timing::OFFSET || key == nodes::timing::OFFSET_Y
                });
            for (key, value) in controls.into_iter().chain(offsets) {
                // The key has to become the definition's `&'static str`: that is what makes
                // a bogus key from a file unrepresentable rather than merely unlikely. The
                // *value* goes through the same coercion the command bus uses, so a file
                // cannot seat a color under a float uniform or a zoom outside its range.
                if def.input(&key).is_none() {
                    warnings.push(LoadWarning::UnknownControl { id, key });
                    continue;
                }
                // The instance's range is already seated above, so a value is fitted to the
                // range this node actually has rather than to its definition's.
                let range = nodes::control_range(def, node, &key);
                match nodes::coerce_control(def, &key, value, range) {
                    Some((key, value)) => {
                        node.controls.insert(key, value);
                    }
                    None => warnings.push(LoadWarning::WrongValue { id: saved.id, key }),
                }
            }
        }

        for c in self.connections {
            warnings.extend(connect(graph, &c, &placed));
        }
        // Every cable of this file is in, so the dual outputs can be resolved from the whole
        // of what it says rather than from the order it listed them in.
        if settles {
            warnings.extend(settle(graph));
        }

        warnings
    }
}

/// Which ids a file's nodes take as they enter a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ids {
    /// The ones in the file. A project's ids are unique across every file in it.
    Keep,
    /// New ones from this graph's counter. A file from elsewhere brings ids from another.
    Fresh,
}

/// Make one saved connection, reporting whatever stopped it.
///
/// `placed` maps a file's node ids to the ids they landed under, which is the identity for
/// everything but an import.
pub(crate) fn connect(
    graph: &mut Graph,
    c: &SavedConnection,
    placed: &HashMap<NodeId, NodeId>,
) -> Option<LoadWarning> {
    let from = placed.get(&c.from.node).copied().unwrap_or(c.from.node);
    let to = placed.get(&c.to.node).copied().unwrap_or(c.to.node);
    let Some(from) = resolve(graph, from, &c.from.key, false) else {
        return Some(LoadWarning::DroppedConnection {
            reason: format!("no output {}.{}", c.from.node, c.from.key),
        });
    };
    let Some(to) = resolve(graph, to, &c.to.key, true) else {
        return Some(LoadWarning::DroppedConnection {
            reason: format!("no input {}.{}", c.to.node, c.to.key),
        });
    };
    // Through the ordinary rules, so a hand-edited file cannot produce a graph the editor
    // could not have produced itself — with the one rule a file cannot be checked against
    // cable by cable left to `settle`, since whether an edge demotes a dual output depends on
    // which of the file's other edges are already in.
    graph
        .connect_loading(from, to)
        .err()
        .map(|e| LoadWarning::DroppedConnection {
            reason: e.to_string(),
        })
}

/// Resolve every dual output from the whole graph and report the cables that leaves illegal.
///
/// What ends a bulk change: a file's own cables, and the project file's cross-workspace glue
/// after them. A consumer left reading a field as a uniform number loses its cable, because
/// the alternative is an edge in the graph that no gesture could have made.
pub(crate) fn settle(graph: &mut Graph) -> Vec<LoadWarning> {
    let dropped = graph.settle_effective_types();
    dropped
        .into_iter()
        .map(|c| {
            let name = |p: crate::graph::PortRef| {
                format!(
                    "{}{}.{}",
                    graph.get(p.node).map_or("node", |n| n.def.slug),
                    p.node,
                    p.key
                )
            };
            LoadWarning::DroppedConnection {
                reason: format!(
                    "{} is a field here, which {} reads as a number",
                    name(c.from),
                    name(c.to)
                ),
            }
        })
        .collect()
}

/// Turn a saved port into a `PortRef`, whose key must be a definition's `&'static str`.
fn resolve(graph: &Graph, node: NodeId, key: &str, input: bool) -> Option<PortRef> {
    let found = graph.get(node)?;
    let ports = if input { &found.inputs } else { &found.outputs };
    let port = ports.iter().find(|p| p.key == key)?;
    Some(PortRef::new(node, port.key))
}

/// Write one workspace file.
pub fn write(file: &WorkspaceFile, path: &Path) -> Result<(), LoadError> {
    std::fs::write(path, to_text(file)?).map_err(LoadError::Io)
}

/// One workspace file's text. Pretty-printed: it is a document, and a document should diff.
pub fn to_text(file: &WorkspaceFile) -> Result<String, LoadError> {
    Ok(serde_json::to_string_pretty(file).map_err(LoadError::Json)? + "\n")
}

/// Read one workspace file, without putting it anywhere.
pub fn read(path: &Path) -> Result<WorkspaceFile, LoadError> {
    parse(&std::fs::read_to_string(path).map_err(LoadError::Io)?)
}

pub fn parse(text: &str) -> Result<WorkspaceFile, LoadError> {
    let header = Header::of(text)?;
    if header.format != FORMAT {
        return Err(LoadError::NotAWorkspace(header.format));
    }
    if header.version > VERSION {
        return Err(LoadError::UnsupportedVersion(header.version));
    }
    serde_json::from_str(text).map_err(LoadError::Json)
}

/// What every file of ours starts with, read before the rest of it.
///
/// Two passes over the text, and the second one is what a file of ours costs anyway. It is
/// what makes "this is not a workspace" the error for somebody else's JSON, rather than
/// whichever field of ours it happened to be missing.
#[derive(Deserialize)]
pub(crate) struct Header {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub version: u32,
}

impl Header {
    pub(crate) fn of(text: &str) -> Result<Self, LoadError> {
        serde_json::from_str(text).map_err(LoadError::Json)
    }
}

/// One file as a graph of its own: a new graph with one workspace, holding everything the
/// file describes. What a loose `.ssw` opened on its own becomes.
pub fn from_str(text: &str) -> Result<(Graph, Vec<LoadWarning>), LoadError> {
    let file = parse(text)?;
    let mut graph = Graph::new();
    let workspace = graph.default_workspace();
    graph.set_layout(workspace, file.layout);
    graph.rename_workspace(workspace, file.name.clone());
    graph.set_blurb(workspace, file.blurb.clone());
    let warnings = file.insert_into(&mut graph, workspace, Ids::Keep);
    Ok((graph, warnings))
}
