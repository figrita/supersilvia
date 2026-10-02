// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command bus.
//!
//! Every user-visible mutation goes through `Command`. The UI only emits them, and tests
//! build graphs with a few `apply` calls instead of dozens of simulated drags.
//!
//! A command that fails does not reach the history; `apply` returns the reason as a
//! `Command::NoSuchNode`-style variant rather than panicking.

use crate::graph::{
    ConnectError, ControlRange, ControlValue, Graph, LayoutMode, Node, NodeId, PortRef,
    WorkspaceId, WorkspaceKind,
};
use emath::Pos2;

/// A cable inside a [`Clip`], by index into its nodes.
///
/// An index rather than a `PortRef`, because a clip has no node ids: the nodes it holds are
/// out of the graph, and the ids they had belong to the graph they were lifted from — which
/// may have dropped them by the time the clip is planted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipEdge {
    pub from: (usize, &'static str),
    pub to: (usize, &'static str),
}

/// Nodes out of the graph, whole, with the cables between them.
///
/// What a copy holds and what a paste plants. Owned nodes rather than ids, so the command
/// carrying one is self-contained: it plants what it was built with however the graph has
/// moved on, where a command that read the clipboard at apply time would replay differently
/// every time the clipboard changed.
///
/// A cable is here only when **both** its ends are: a clip is a working piece of graph, not
/// a heap of nodes, and an edge from outside has no second source to plant.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    pub nodes: Vec<Node>,
    pub edges: Vec<ClipEdge>,
    /// The project folder the nodes' `assets/…` references name files in: the one a copy
    /// was taken in. A paste into another project copies each of those files into its own
    /// `assets/`, as an import does. `None` is the graph the clip is planted into, which is
    /// what a duplicate is.
    pub from: Option<std::path::PathBuf>,
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The clip's anchor: the top-left corner of the box its node positions make.
    ///
    /// What a paste point is measured from, so the nodes keep their layout relative to one
    /// another and the whole arrives where it was asked for.
    pub fn anchor(&self) -> Option<Pos2> {
        let mut nodes = self.nodes.iter();
        let first = nodes.next()?.pos;
        Some(nodes.fold(first, |at, node| at.min(node.pos)))
    }
}

/// What a new workspace is born holding.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Seed {
    /// Nothing. What an imported or restored workspace gets.
    #[default]
    Empty,
    /// A Main Input on the left, an Output on the right, and a cable between them: the
    /// shortest patch that shows a picture, so a new tab is something to play with rather
    /// than an empty plane.
    ///
    /// `width` is the canvas the Output is placed against, carried in the command for the
    /// reason `AutoArrange` carries its height: redo has to put the node back where the first
    /// run put it, whatever the window is doing by then.
    SourceToOutput { width: f32 },
}

/// A user-visible mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Add a node of the named kind, on one workspace.
    ///
    /// The workspace is named rather than implied: a node has to be somewhere, and the
    /// command carries where so that redo puts it back on the same tab.
    AddNode {
        slug: &'static str,
        at: Pos2,
        workspace: WorkspaceId,
    },
    /// Remove nodes, and everything connected to them. One command rather than one per
    /// node, so deleting a selection is one undo step.
    RemoveNodes(Vec<NodeId>),
    /// Copy nodes, with their control values and ranges, options and collapsed state.
    ///
    /// A cable is copied when **both** its ends are in the set: the copy is a working piece
    /// of graph, not a heap of disconnected nodes. A cable from outside into the set is not,
    /// because there is no second source to duplicate.
    Duplicate {
        nodes: Vec<NodeId>,
        /// Where the copies land relative to the originals.
        offset: emath::Vec2,
    },
    /// Plant a clip: the nodes it holds, with fresh ids, and the cables between them.
    ///
    /// The nodes travel **in** the command rather than being read off the clipboard when it
    /// is applied, for the reason every other command carries its own arguments: a command
    /// that read session state would replay against whatever the clipboard held by then.
    /// The clipboard itself is never an edit and never enters the history.
    Paste {
        clip: Clip,
        /// Where the clip's anchor lands, in world units. The rest of it keeps its shape
        /// around that corner.
        at: Pos2,
        /// Which workspace the nodes land on. A pasted node belongs to the workspace being
        /// looked at, whatever the one it was copied from was.
        workspace: WorkspaceId,
    },
    Connect {
        from: PortRef,
        to: PortRef,
    },
    /// Put a node between two ports whose types a cable cannot join, and wire both sides.
    ///
    /// **One command, so a conversion is one undo step**: a node appears and two cables land,
    /// and a hand that changes its mind wants all three gone at once. What the pairing is —
    /// which input the cable lands on and which output carries on — is the conversion menu's
    /// question, so the command carries both keys rather than taking the node's first of each.
    Bridge {
        from: PortRef,
        to: PortRef,
        slug: &'static str,
        /// The inputs `from` connects to. A list, because a casting may fan one cable out:
        /// a number becomes a gray by landing on an `rgba`'s red, green and blue at once.
        inputs: &'static [&'static str],
        /// The bridge's output, which connects to `to`.
        output: &'static str,
        /// Where the node lands: the midpoint of the two nodes it goes between.
        at: Pos2,
        workspace: WorkspaceId,
    },
    /// Add a node and land a cable on it from a port already on the canvas: what a cable let
    /// go over empty canvas ends in, once the browser has said which kind.
    ///
    /// **One command, so the node and its cable are one undo step**, as a bridge's are.
    /// Which way the cable runs is `end`'s: from an output it lands on the new node's input
    /// `key`, and into an input it leaves the new node's output `key`.
    AddConnected {
        slug: &'static str,
        at: Pos2,
        workspace: WorkspaceId,
        /// The port the cable was dragged out of.
        end: PortRef,
        /// The new node's port the cable lands on.
        key: &'static str,
    },
    /// Put a node already on the canvas into a cable: the cable goes, and two land in its
    /// place, `from` into the node's `input` and the node's `output` into `to`.
    ///
    /// The shape of [`Command::Bridge`] for a node that exists: what dropping a node on a
    /// cable does, and one undo step for the same reason.
    Splice {
        node: NodeId,
        from: PortRef,
        to: PortRef,
        input: &'static str,
        output: &'static str,
    },
    /// Remove everything feeding an input. A data input takes one connection, an action
    /// input may take several, and clicking the port clears all of them.
    Disconnect {
        to: PortRef,
    },
    /// Remove one specific edge, which is what clicking a single cable does.
    DisconnectEdge {
        from: PortRef,
        to: PortRef,
    },
    /// Remove every connection on one port, whichever side of it that port is. What
    /// right-clicking a port dot does: `Disconnect` only reaches an input, and a cable
    /// leaving an output has nowhere else to be cleared from in one gesture.
    DisconnectPort(PortRef),
    /// Remove every connection touching these nodes, in and out.
    ///
    /// One command over a list, so disconnecting a selection is one undo step. Structural:
    /// the Outputs downstream of any of them rebuild.
    DisconnectAll(Vec<NodeId>),
    /// Every control on these nodes back to its definition's default, and their own ranges
    /// cleared.
    ///
    /// The number control's `R` for a whole node: the value *and* the range, since a range
    /// is the node's own. Options are untouched — they are structural, and a reset that
    /// rebuilt a shader would be a different command.
    ResetControls(Vec<NodeId>),
    /// Move nodes to absolute positions.
    ///
    /// A list rather than one node, because dragging a selection has to be one undo step and
    /// one snapshot; `history::joins` coalesces consecutive moves of the same set.
    MoveNodes {
        moves: Vec<(NodeId, Pos2)>,
    },
    SetControl {
        node: NodeId,
        key: &'static str,
        value: ControlValue,
    },
    /// Set several of one node's controls together.
    ///
    /// A list rather than one key, for the same reason `MoveNodes` takes one: a handle that
    /// carries two axes writes both of them every frame it is dragged, and two commands a
    /// frame would be two undo steps a frame. `history::joins` coalesces consecutive writes
    /// of the same keys, so the whole drag is one step.
    SetControls {
        node: NodeId,
        values: Vec<(&'static str, ControlValue)>,
    },
    /// Give one control its own ends and quantum, or hand it back to its definition's.
    ///
    /// A range is document data, like a position: it describes the graph rather than the
    /// session, so it goes through the bus, rides in the file, and is undoable. The stored
    /// value is re-fitted to the new range in the same step — a control cannot sit outside
    /// the track it is drawn on.
    SetRange {
        node: NodeId,
        key: &'static str,
        range: ControlRange,
    },
    /// Hand a control's range back to its definition's.
    ClearRange {
        node: NodeId,
        key: &'static str,
    },
    SetOption {
        node: NodeId,
        key: &'static str,
        value: String,
    },
    /// Set some of one node's options and number controls together: what a button on the
    /// node's body writes — `lyapunov`'s Random Seq, `slimemold`'s presets.
    ///
    /// One command, so one press is one undo step whatever it writes. Each option is checked
    /// as `SetOption` checks it and each control fitted as `SetControls` fits it, all before
    /// anything is written; a `Code` option among them rebuilds the Outputs the node reaches,
    /// and anything else is a uniform or a tick's to read, which rebuilds nothing. Never
    /// coalesced: two presses are two edits.
    SetSettings {
        node: NodeId,
        options: Vec<(&'static str, String)>,
        controls: Vec<(&'static str, ControlValue)>,
    },
    /// Write one of a node's own declared values — a note's text, and later a pad's position
    /// or a grid's pattern.
    ///
    /// Never a recompile: a value reaches no generator and no uniform. It is document data,
    /// so it is undoable and rides in the file, and it coalesces per key the way a control
    /// does — typing a sentence into a note is one undo step, not one per keystroke.
    SetValue {
        node: NodeId,
        key: &'static str,
        value: crate::graph::Value,
    },
    /// Set one node's body width, or hand it back to the width its kind asks for.
    ///
    /// Layout, and document data like a position: it goes through the bus, is undoable and
    /// rides in the file. `history::joins` coalesces consecutive widths of the same node, so
    /// a drag on a note's grip is one undo step rather than one per frame. A width below
    /// what the kind asks for is not refused — `canvas::node_width` is the one place that
    /// decides how wide a node is drawn, and it holds every width to that floor.
    SetNodeWidth {
        node: NodeId,
        width: Option<f32>,
    },
    /// Draw a node as a header only, or restore it. Presentation, so it never recompiles;
    /// document data, so it is undoable and rides in the file.
    SetCollapsed {
        nodes: Vec<NodeId>,
        collapsed: bool,
    },
    /// How one workspace is navigated and laid out. Layout is document data, like a
    /// position, so it goes through the bus and is undoable — and it belongs to the
    /// workspace, so the command names which one.
    SetLayout {
        workspace: WorkspaceId,
        mode: LayoutMode,
    },
    /// Append a workspace in project order.
    ///
    /// The id it lands under is the graph's to hand out; a caller that needs it reads the
    /// last workspace back, the way `AddNode`'s caller reads the highest node id.
    AddWorkspace {
        name: String,
        kind: WorkspaceKind,
        /// The mode it opens in. A workspace's mode is its own document data from then on;
        /// this is only what it is born with, and `App` fills it in from the preference the
        /// same way it fills in which workspace a menu action meant.
        layout: LayoutMode,
        /// What it is born holding. Part of *this* command rather than three more, so that
        /// one gesture is one undo step: a new tab you did not want goes away in one press,
        /// not four.
        seed: Seed,
    },
    /// Remove a workspace, every node left on none, and every cable touching those nodes.
    ///
    /// Refused for the last workspace: a node has to be on one, so there has to be one.
    RemoveWorkspace(WorkspaceId),
    RenameWorkspace {
        id: WorkspaceId,
        name: String,
    },
    /// A workspace's one-line blurb, which is what its card in the project tab says about
    /// it. An edit, so it goes through the bus and is undoable exactly as a rename is.
    SetBlurb {
        id: WorkspaceId,
        blurb: String,
    },
    /// Move a workspace to an index in project order.
    MoveWorkspace {
        id: WorkspaceId,
        to: usize,
    },
    /// Copy a workspace whole, beside the original in project order, under `name`.
    ///
    /// **One command, so a duplicate is one undo step.** Every node on it is copied as a plain
    /// node on the copy alone, shared or not, with the cables between them; a cable arriving
    /// from a node elsewhere arrives at the copy too, and one leaving for a node elsewhere does
    /// not. The name is chosen where the duplicate is asked for, for the reason `AddWorkspace`
    /// carries its own.
    DuplicateWorkspace {
        id: WorkspaceId,
        name: String,
    },
    /// Put nodes on a workspace, keeping the ones they are already on.
    ShowOn {
        nodes: Vec<NodeId>,
        workspace: WorkspaceId,
    },
    /// Take nodes off a workspace. Refused if it would leave any of them on none.
    HideFrom {
        nodes: Vec<NodeId>,
        workspace: WorkspaceId,
    },
    /// Put nodes on one workspace and no other.
    MoveTo {
        nodes: Vec<NodeId>,
        workspace: WorkspaceId,
    },
    /// Take a `.ssw` into this project as a new workspace, with fresh ids and its media
    /// copied in.
    ///
    /// **A command, so importing is one undo step** — nodes appear, and that is an edit like
    /// any other. It carries the file rather than the nodes because the remap is the
    /// operation: which ids they land under is this project's counter's answer, not the
    /// file's. Undo restores the snapshot from before it, as every command's does; the
    /// assets it copied into the folder stay, because Save deletes nothing under `assets/`
    /// and neither does undo.
    ImportWorkspace {
        file: std::path::PathBuf,
    },
    /// Rank one workspace's nodes into columns and stack them, for a strip of this height.
    ///
    /// The height travels in the command because redo replays it: an arrange that re-read
    /// the viewport would put nodes somewhere else if the window had been resized since.
    /// Arranging is an edit of one canvas, so it names the workspace and moves nothing on
    /// any other.
    AutoArrange {
        workspace: WorkspaceId,
        height: f32,
    },
}

impl Command {
    /// Whether a tick could tell this command happened.
    ///
    /// The tick walks the graph's order and reads slugs, controls, options and values. It
    /// reads no position, no collapsed flag, no workspace name and no layout mode, so a
    /// command that touches only those is not handed across to the synth: a node drag emits
    /// one `MoveNodes` a frame and there is nothing in it for a tick to see. Everything else
    /// answers true, including anything added later, because the copy being one edit behind
    /// is a bug and one copy too many is a cost.
    pub fn affects_tick(&self) -> bool {
        !matches!(
            self,
            Self::MoveNodes { .. }
                | Self::SetNodeWidth { .. }
                | Self::AutoArrange { .. }
                | Self::SetCollapsed { .. }
                | Self::SetLayout { .. }
                | Self::RenameWorkspace { .. }
                | Self::SetBlurb { .. }
                | Self::MoveWorkspace { .. }
        )
    }

    /// Whether this command can change what the render plan is built from.
    ///
    /// The plan reads which nodes and cables there are, which workspaces show them and in
    /// what order, and an Output's options — never a control's value, a range, a note, a
    /// position or a name. So a scrub, a reset and a drag answer false and rebuild no plan,
    /// and everything else answers true, including anything added later, for the reason
    /// [`Self::affects_tick`] does.
    pub fn reshapes(&self) -> bool {
        !matches!(
            self,
            Self::SetControl { .. }
                | Self::SetControls { .. }
                | Self::SetRange { .. }
                | Self::ClearRange { .. }
                | Self::ResetControls(..)
                | Self::SetValue { .. }
                | Self::MoveNodes { .. }
                | Self::SetNodeWidth { .. }
                | Self::AutoArrange { .. }
                | Self::SetCollapsed { .. }
                | Self::SetLayout { .. }
                | Self::RenameWorkspace { .. }
                | Self::SetBlurb { .. }
        )
    }
}

/// What a command was about: the nodes it names and the workspace it names, either of which
/// may be gone by the time anyone asks. Where an undo's toast offers to go.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Subject {
    pub nodes: Vec<NodeId>,
    pub workspace: Option<WorkspaceId>,
}

impl Command {
    /// What this command is called where a person reads it: the Edit menu's *Undo Delete 3
    /// nodes*, the toast after an undo, a row of the Undo History.
    ///
    /// A node is named by its kind's label and a row by the label it wears, looked up in
    /// `graphs` in order — the graphs on either side of the command, since a deleted node is
    /// only in the one before and an added one only in the one after. Where none of them
    /// holds it, the name says *node* rather than guessing.
    pub fn name(&self, graphs: &[&Graph]) -> String {
        let node = |id: NodeId| graphs.iter().find_map(|g| g.get(id));
        let nodes = |ids: &[NodeId]| match ids {
            [one] => node(*one).map_or_else(|| "node".to_owned(), |n| n.def.label.to_owned()),
            many => format!("{} nodes", many.len()),
        };
        let workspace = |id: WorkspaceId| {
            graphs
                .iter()
                .find_map(|g| g.workspace(id))
                .map_or_else(|| "a workspace".to_owned(), |w| w.name.clone())
        };
        // A control, an option or a value by the label its row wears, after its node's.
        let row = |id: NodeId, key: &'static str| {
            let Some(n) = node(id) else {
                return key.to_owned();
            };
            let def = n.def;
            let label = def
                .inputs
                .iter()
                .chain(def.hidden)
                .find(|i| i.key == key)
                .map(|i| i.label)
                .or_else(|| def.options.iter().find(|o| o.key == key).map(|o| o.label))
                .or_else(|| def.values.iter().find(|v| v.key == key).map(|v| v.label))
                .filter(|label| !label.is_empty() && *label != def.label);
            label.map_or_else(|| def.label.to_owned(), |l| format!("{} {l}", def.label))
        };
        match self {
            Self::AddNode { slug, .. } => format!("Add {}", crate::nodes::label_of(slug)),
            Self::RemoveNodes(ids) => format!("Delete {}", nodes(ids)),
            Self::Duplicate { nodes: ids, .. } => format!("Duplicate {}", nodes(ids)),
            Self::Paste { clip, .. } => match clip.nodes.as_slice() {
                [one] => format!("Paste {}", one.def.label),
                many => format!("Paste {} nodes", many.len()),
            },
            Self::Connect { .. } => "Connect".to_owned(),
            Self::Bridge { slug, .. } => {
                format!("Connect through {}", crate::nodes::label_of(slug))
            }
            Self::AddConnected { slug, .. } => {
                format!("Add {}, connected", crate::nodes::label_of(slug))
            }
            Self::Splice { node: id, .. } => format!("Insert {}", nodes(&[*id])),
            Self::Disconnect { .. } | Self::DisconnectEdge { .. } | Self::DisconnectPort(_) => {
                "Disconnect".to_owned()
            }
            Self::DisconnectAll(ids) => format!("Disconnect {}", nodes(ids)),
            Self::ResetControls(ids) => format!("Reset {}", nodes(ids)),
            Self::MoveNodes { moves } => {
                let ids: Vec<NodeId> = moves.iter().map(|(id, _)| *id).collect();
                format!("Move {}", nodes(&ids))
            }
            Self::SetControl { node, key, .. } | Self::SetOption { node, key, .. } => {
                format!("Change {}", row(*node, key))
            }
            Self::SetControls { node: id, .. } | Self::SetSettings { node: id, .. } => {
                format!("Change {}", nodes(&[*id]))
            }
            Self::SetRange { node, key, .. } => format!("Change {} range", row(*node, key)),
            Self::ClearRange { node, key } => format!("Reset {} range", row(*node, key)),
            Self::SetValue { node, key, .. } => format!("Edit {}", row(*node, key)),
            Self::SetNodeWidth { node: id, .. } => format!("Resize {}", nodes(&[*id])),
            Self::SetCollapsed {
                nodes: ids,
                collapsed,
            } => format!(
                "{} {}",
                if *collapsed { "Collapse" } else { "Expand" },
                nodes(ids)
            ),
            Self::SetLayout { mode, .. } => match mode {
                LayoutMode::Canvas => "Lay out as a canvas".to_owned(),
                LayoutMode::Linear => "Lay out as a strip".to_owned(),
            },
            Self::AddWorkspace { name, .. } => format!("New workspace {name}"),
            Self::RemoveWorkspace(id) => format!("Delete {}", workspace(*id)),
            Self::RenameWorkspace { name, .. } => format!("Rename to {name}"),
            Self::SetBlurb { id, .. } => format!("Edit {} blurb", workspace(*id)),
            Self::MoveWorkspace { id, .. } => format!("Reorder {}", workspace(*id)),
            Self::DuplicateWorkspace { id, .. } => {
                format!("Duplicate workspace {}", workspace(*id))
            }
            Self::ShowOn {
                nodes: ids,
                workspace: w,
            } => format!("Show {} on {}", nodes(ids), workspace(*w)),
            Self::HideFrom {
                nodes: ids,
                workspace: w,
            } => format!("Take {} off {}", nodes(ids), workspace(*w)),
            Self::MoveTo {
                nodes: ids,
                workspace: w,
            } => format!("Move {} to {}", nodes(ids), workspace(*w)),
            Self::ImportWorkspace { file } => format!(
                "Import {}",
                file.file_stem()
                    .map_or_else(|| "workspace".into(), |s| s.to_string_lossy())
            ),
            Self::AutoArrange { .. } => "Auto-arrange".to_owned(),
        }
    }

    /// The nodes and the workspace this command names. A paste's and a duplicate's new nodes
    /// are not among them: their ids are the graph's to hand out, and the command never knew
    /// them.
    pub fn subject(&self) -> Subject {
        let (nodes, workspace) = match self {
            Self::AddNode { workspace, .. }
            | Self::Paste { workspace, .. }
            | Self::SetLayout { workspace, .. }
            | Self::AutoArrange { workspace, .. } => (Vec::new(), Some(*workspace)),
            Self::Bridge {
                from,
                to,
                workspace,
                ..
            } => (vec![from.node, to.node], Some(*workspace)),
            Self::AddConnected { end, workspace, .. } => (vec![end.node], Some(*workspace)),
            Self::Splice { node, from, to, .. } => (vec![*node, from.node, to.node], None),
            Self::RemoveNodes(ids)
            | Self::DisconnectAll(ids)
            | Self::ResetControls(ids)
            | Self::Duplicate { nodes: ids, .. }
            | Self::SetCollapsed { nodes: ids, .. } => (ids.clone(), None),
            Self::Connect { from, to } | Self::DisconnectEdge { from, to } => {
                (vec![from.node, to.node], None)
            }
            Self::Disconnect { to: port } | Self::DisconnectPort(port) => (vec![port.node], None),
            Self::MoveNodes { moves } => (moves.iter().map(|(id, _)| *id).collect(), None),
            Self::SetControl { node, .. }
            | Self::SetControls { node, .. }
            | Self::SetRange { node, .. }
            | Self::ClearRange { node, .. }
            | Self::SetOption { node, .. }
            | Self::SetSettings { node, .. }
            | Self::SetValue { node, .. }
            | Self::SetNodeWidth { node, .. } => (vec![*node], None),
            Self::ShowOn { nodes, workspace }
            | Self::HideFrom { nodes, workspace }
            | Self::MoveTo { nodes, workspace } => (nodes.clone(), Some(*workspace)),
            Self::RemoveWorkspace(id)
            | Self::RenameWorkspace { id, .. }
            | Self::SetBlurb { id, .. }
            | Self::MoveWorkspace { id, .. }
            | Self::DuplicateWorkspace { id, .. } => (Vec::new(), Some(*id)),
            Self::AddWorkspace { .. } | Self::ImportWorkspace { .. } => (Vec::new(), None),
        };
        Subject { nodes, workspace }
    }
}

/// Why a command could not be applied.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandError {
    /// The command named a node that is not in the graph.
    NoSuchNode(NodeId),
    /// The command named a node kind that is not in the registry.
    NoSuchNodeKind(&'static str),
    /// The graph refused the connection.
    Refused(ConnectError),
    /// Nothing was connected to that input.
    NotConnected(PortRef),
    /// That cable is already there, so making it again changes nothing.
    AlreadyConnected { from: PortRef, to: PortRef },
    /// An offline render is running, and the document is closed until it is done or
    /// canceled.
    Rendering,
    /// The node has no control or option under that key.
    NoSuchKey(NodeId, &'static str),
    /// The definition does not offer that value for this option.
    NoSuchChoice(NodeId, &'static str),
    /// The command named a workspace that is not in the graph.
    NoSuchWorkspace(WorkspaceId),
    /// The only workspace cannot be removed: every node has to be on one.
    LastWorkspace,
    /// The command would leave this node on no workspace.
    NoWorkspaceLeft(NodeId),
    /// A paste with nothing on the clipboard. Not a failure worth saying out loud: it is
    /// what stops an empty paste becoming an undo step.
    EmptyClip,
    /// The command reached the filesystem and it said no. The reason is already on the
    /// status line; this is what stops a failed import becoming an undo step.
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchNode(id) => write!(f, "no such node: {id:?}"),
            Self::NoSuchNodeKind(slug) => write!(f, "no such node kind: {slug:?}"),
            Self::Refused(e) => write!(f, "{e}"),
            Self::NotConnected(p) => write!(f, "nothing connected to {:?}.{}", p.node, p.key),
            Self::AlreadyConnected { from, to } => write!(
                f,
                "{:?}.{} already feeds {:?}.{}",
                from.node, from.key, to.node, to.key
            ),
            Self::Rendering => write!(f, "a render is running"),
            Self::NoSuchKey(id, key) => write!(f, "node {id:?} has no key {key:?}"),
            Self::NoSuchChoice(id, key) => {
                write!(f, "node {id:?} does not offer that value for {key:?}")
            }
            Self::NoSuchWorkspace(id) => write!(f, "no such workspace: {id}"),
            Self::LastWorkspace => write!(f, "the last workspace cannot be removed"),
            Self::NoWorkspaceLeft(id) => {
                write!(f, "node {id} would be left on no workspace")
            }
            Self::EmptyClip => write!(f, "nothing to paste"),
            Self::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for CommandError {}
