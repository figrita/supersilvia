// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pure graph data. No GPU, no egui, no code generation.
//!
//! Connection rules, in one place:
//!
//! - Types must match exactly, with one exception: a `UniformNumber` output feeds a
//!   `VaryingNumber` input for free, since one number broadcast over every pixel is a
//!   uniform. Everything else that would change a value's kind is a node.
//! - A **data** input takes at most one connection. Connecting to an occupied input replaces
//!   what was there, which is what dragging a new cable onto a full port does.
//! - **Action** connections are many-to-many: one output may trigger many inputs, and one
//!   input may be triggered by many outputs.
//! - Data edges may not form a cycle. Action edges may not connect a node to itself, but are
//!   otherwise unconstrained — they are CPU events, not values, and do not feed the compiler.
//! - A **dual** port's type is not its own. `PortDef::dual` marks a port declared
//!   `VaryingNumber` whose effective type the graph decides: an output is `UniformNumber`
//!   while every input of its node resolves to a uniform number, and the inputs of that node
//!   are `UniformNumber` while it is **pinned** — in `UniformNumber` mode with a flip that
//!   would leave a field feeding a `UniformNumber` input. The instance's `PortDef::ty` *is*
//!   that answer, recomputed here after every structural change, so the compiler and the
//!   canvas read the type they already read. A field onto a pinned input is an ordinary
//!   `TypeMismatch`: a field does not feed a `UniformNumber` input, and the port draws the
//!   type a cable may bring.

pub mod node;
mod order;
pub mod port;
pub mod workspace;

pub use node::{ControlRange, ControlValue, Node, PAINTING_FOLDER, Painting, Point, Value};
pub use port::{ConnectError, Connection, Kind, PortDef, PortRef, PortType, Rate};
pub use workspace::{LayoutMode, Workspace, WorkspaceId, WorkspaceKind};

use crate::nodes::NodeDef;
use emath::Pos2;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, OnceLock};

/// Node identity: one stable integer per node per project, never reused.
///
/// It is the *only* identity a node has. It appears in generated WGSL names, in the
/// accessibility tree, and in save files, and undo re-inserts a removed node under the same
/// one — which is what lets a command recorded before a deletion still address the node
/// after it is restored.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct NodeId(pub u32);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How a cable's value travels, fixed by its source port when the cable is made.
///
/// Stored on the index entry so a walk filters each edge without looking its port up. A
/// dual port's flip keeps it, since both of its types are data and a dual port is never
/// delayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// An event: ordered by the tick, invisible to the compiler and the cycle check.
    Action,
    /// This frame's value: a dependency for ordering and for termination.
    Immediate,
    /// Last frame's value: a dependency for ordering only, so a loop through one is feedback.
    Delayed,
}

impl Flow {
    fn of(port: &PortDef) -> Self {
        match (port.ty.is_data(), port.delayed) {
            (false, _) => Self::Action,
            (true, false) => Self::Immediate,
            (true, true) => Self::Delayed,
        }
    }

    fn is_data(self) -> bool {
        self != Self::Action
    }

    fn is_immediate(self) -> bool {
        self == Self::Immediate
    }
}

/// One cable as the index holds it: its two ends and how its value travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cable {
    pub from: PortRef,
    pub to: PortRef,
    flow: Flow,
}

impl Cable {
    fn is(&self, c: &Connection) -> bool {
        self.from == c.from && self.to == c.to
    }
}

/// Every cable, in the order it was made, and indexed by the node at each end.
///
/// The list is what the file writes and the compiler walks; the two maps answer *what feeds
/// this port* and *what does this port feed* without a scan, and are what every traversal
/// walks. **Kept, not derived**: `link` and every unlink write the list and the maps together.
/// One `Arc` holds all three, so a clone shares them and an edit copies them once.
#[derive(Debug, Clone, Default)]
struct Cables {
    list: Vec<Connection>,
    /// The cables into each node's inputs, keyed by the consumer, in the order they were made.
    into: HashMap<NodeId, Vec<Cable>>,
    /// The cables out of each node's outputs, keyed by the producer, in the order they were
    /// made.
    out_of: HashMap<NodeId, Vec<Cable>>,
}

impl Cables {
    fn push(&mut self, c: Connection, flow: Flow) {
        self.list.push(c);
        let cable = Cable {
            from: c.from,
            to: c.to,
            flow,
        };
        self.into.entry(c.to.node).or_default().push(cable);
        self.out_of.entry(c.from.node).or_default().push(cable);
    }

    /// Remove every cable `doomed` picks, from the list and both maps. Returns them in list
    /// order.
    fn remove_where(&mut self, doomed: impl Fn(&Connection) -> bool) -> Vec<Connection> {
        let (removed, kept): (Vec<_>, Vec<_>) = self.list.iter().partition(|c| doomed(c));
        self.list = kept;
        for c in &removed {
            Self::unindex(&mut self.into, c.to.node, c);
            Self::unindex(&mut self.out_of, c.from.node, c);
        }
        removed
    }

    fn unindex(map: &mut HashMap<NodeId, Vec<Cable>>, node: NodeId, c: &Connection) {
        if let Some(cables) = map.get_mut(&node) {
            cables.retain(|x| !x.is(c));
            if cables.is_empty() {
                map.remove(&node);
            }
        }
    }

    /// Work the flow of every cable out of `id` out again from `node`'s ports. A cable whose
    /// port the node does not have carries nothing a data walk follows.
    fn reflow(&mut self, id: NodeId, node: &Node) {
        let flow = |c: &Cable| node.output(c.from.key).map_or(Flow::Action, Flow::of);
        let consumers: Vec<NodeId> = self.leaving(id).iter().map(|c| c.to.node).collect();
        for c in self.out_of.get_mut(&id).into_iter().flatten() {
            c.flow = flow(c);
        }
        for consumer in consumers {
            for c in self.into.get_mut(&consumer).into_iter().flatten() {
                if c.from.node == id {
                    c.flow = flow(c);
                }
            }
        }
    }

    /// The cables into one node.
    fn entering(&self, node: NodeId) -> &[Cable] {
        self.into.get(&node).map_or(&[], Vec::as_slice)
    }

    /// The cables out of one node.
    fn leaving(&self, node: NodeId) -> &[Cable] {
        self.out_of.get(&node).map_or(&[], Vec::as_slice)
    }
}

/// Where one end of a cable drag reaches over immediate data edges: every node a cable from
/// it could not land on without closing a loop.
///
/// A drag pins one end and asks the cycle question once per candidate port, every time
/// about that end, so the canvas walks it once and keeps this for the drag.
/// [`Graph::can_connect_within`] answers from it while the graph holds the cables it was
/// walked over, and walks the graph itself otherwise.
#[derive(Debug, Clone)]
pub struct Reach {
    /// The port the drag started on.
    end: PortRef,
    /// True where `end` is an output, so the set is upstream of it; downstream otherwise.
    upstream: bool,
    nodes: HashSet<NodeId>,
    /// The cables it was walked over. Held rather than compared by address, so a freed list
    /// cannot come back at the same one.
    cables: Arc<Cables>,
}

impl Reach {
    /// Was this walked from `end`, over the cables `graph` holds?
    pub fn is_current(&self, graph: &Graph, end: PortRef) -> bool {
        self.end == end && Arc::ptr_eq(&self.cables, &graph.cables)
    }

    /// Can `start` reach `goal`? Answered only where the pinned end is the one this was
    /// walked from.
    fn answers(&self, start: NodeId, goal: NodeId) -> Option<bool> {
        let (pinned, other) = if self.upstream {
            (goal, start)
        } else {
            (start, goal)
        };
        (pinned == self.end.node).then(|| other == pinned || self.nodes.contains(&other))
    }
}

/// The document: every node, every cable, every workspace.
///
/// **Shared, not copied.** Each node is an `Arc<Node>`, and the cables with their index and
/// the workspace list are behind an `Arc` of their own, so a clone is pointer bumps and a
/// write to one node copies that node alone, through `Arc::make_mut`. The editor, every undo
/// step and the synth hold graphs as `Arc<Graph>`, which is why it is `Sync`: the two orders
/// are `OnceLock`. See docs/architecture.md, *The two channels*.
#[derive(Debug, Clone)]
pub struct Graph {
    /// Ordered by id, so iteration and the topological seed are deterministic without
    /// anyone having to sort.
    nodes: BTreeMap<NodeId, Arc<Node>>,
    cables: Arc<Cables>,
    /// Next id to hand out. Monotonic per project; never rewound, so an id is never reused
    /// even after the node holding it is deleted — and a graph an undo restores takes the
    /// higher of its own counter and the one it replaces.
    next_id: u32,
    /// The two orders, cleared together by any structural change. `OnceLock` so a query
    /// stays `&self`; a clone shares a built one, which is exact because each is a function
    /// of the nodes and cables the clone shares.
    topo: OnceLock<Arc<[NodeId]>>,
    tick_topo: OnceLock<Arc<[NodeId]>>,
    /// Every workspace, in project order. Never empty.
    workspaces: Arc<Vec<Workspace>>,
    /// Next workspace id to hand out. Monotonic per project, like `next_id`.
    next_workspace_id: u32,
    /// Inside a bulk change: see [`Graph::begin_bulk`].
    bulk: bool,
}

/// The name a graph's first workspace carries.
const FIRST_WORKSPACE: &str = "Workspace 1";

impl Default for Graph {
    /// A graph always has a workspace, so a node always has one to land on.
    fn default() -> Self {
        let mut graph = Self {
            nodes: BTreeMap::new(),
            cables: Arc::default(),
            next_id: 0,
            topo: OnceLock::new(),
            tick_topo: OnceLock::new(),
            workspaces: Arc::default(),
            next_workspace_id: 0,
            bulk: false,
        };
        graph.add_workspace(FIRST_WORKSPACE.to_string(), WorkspaceKind::Video);
        graph
    }
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------------------------------------------------------- nodes

    /// Add a node on a set of workspaces.
    ///
    /// The set is filtered to live workspaces and falls back to the default one, so a node
    /// is on a workspace by construction rather than by anyone remembering to put it there.
    pub fn add_node(
        &mut self,
        def: &'static NodeDef,
        pos: Pos2,
        inputs: Vec<PortDef>,
        outputs: Vec<PortDef>,
        workspaces: BTreeSet<WorkspaceId>,
    ) -> NodeId {
        self.next_id += 1;
        let id = NodeId(self.next_id);
        self.insert_node(
            id,
            Node {
                def,
                pos,
                workspaces,
                inputs,
                outputs,
                dragged_width: None,
                dragged_height: None,
                controls: HashMap::new(),
                values: BTreeMap::new(),
                options: BTreeMap::new(),
                collapsed: false,
            },
        );
        id
    }

    /// The node id counter, for the project file to carry. A project reopened with a
    /// rewound counter would hand a new node an id another file still names.
    pub fn next_node_id(&self) -> u32 {
        self.next_id
    }

    /// Carry the counter forward to at least `next`. Loading a project, and nothing else.
    ///
    /// It only ever moves forward, so a file naming a lower one cannot make an id reusable.
    pub fn set_next_node_id(&mut self, next: u32) {
        self.next_id = self.next_id.max(next);
    }

    /// Carry both id counters forward to at least `other`'s. What undo and redo do to the
    /// graph they restore, so an id handed out after that graph was taken is never handed
    /// out again.
    ///
    /// Writes nothing, and so copies nothing, where this graph's counters are the higher.
    pub fn carry_counters(this: &mut Arc<Self>, other: &Self) {
        if this.next_id >= other.next_id && this.next_workspace_id >= other.next_workspace_id {
            return;
        }
        let graph = Arc::make_mut(this);
        graph.next_id = graph.next_id.max(other.next_id);
        graph.next_workspace_id = graph.next_workspace_id.max(other.next_workspace_id);
    }

    /// Put a node back under an id it already had. Undo of a removal, and loading a file:
    /// `add_node` is how a *new* node arrives.
    ///
    /// `next_id` still only moves forward, so restoring an old node cannot collide with one
    /// added after it.
    pub fn insert_node(&mut self, id: NodeId, mut node: Node) {
        self.next_id = self.next_id.max(id.0);
        node.workspaces.retain(|w| self.has_workspace(*w));
        if node.workspaces.is_empty() {
            node.workspaces.insert(self.default_workspace());
        }
        self.invalidate();
        // Over a node already here, the cables out of it travel as the new ports say.
        if !self.cables.leaving(id).is_empty() {
            Arc::make_mut(&mut self.cables).reflow(id, &node);
        }
        self.nodes.insert(id, Arc::new(node));
        self.recompute_effective_types(&[id]);
    }

    /// Remove a node and every connection touching it.
    pub fn remove_node(&mut self, id: NodeId) -> Option<Arc<Node>> {
        let node = self.nodes.remove(&id)?;
        // Before the edges go: afterwards there is nothing left to walk, and a consumer that
        // has just lost its source is exactly what needs recomputing.
        let consumers: Vec<NodeId> = self.cables.leaving(id).iter().map(|c| c.to.node).collect();
        self.unlink_where(|c| c.from.node == id || c.to.node == id);
        self.invalidate();
        self.recompute_effective_types(&consumers);
        Some(node)
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id).map(AsRef::as_ref)
    }

    /// One node to write. Where another graph still shares it — an undo step, the synth's
    /// copy — this copies that node and nothing else.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        // Control values and position do not affect topology, so the topo cache stands.
        self.nodes.get_mut(&id).map(Arc::make_mut)
    }

    /// Is this node the very allocation `other` holds under the same id? What says an edit
    /// left it alone, and what the undo ring's byte budget counts by.
    pub fn shares_node(&self, other: &Self, id: NodeId) -> bool {
        match (self.nodes.get(&id), other.nodes.get(&id)) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Is the cable list the very allocation `other` holds? True across any edit that made
    /// or broke no cable.
    pub fn shares_connections(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cables, &other.cables)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().map(|(id, node)| (*id, node.as_ref()))
    }

    // ---------------------------------------------------------------- workspaces

    /// Every workspace, in project order.
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    /// One workspace to write. The list is shared with the graph before the edit until the
    /// first write, which copies the list and not the nodes.
    fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut Workspace> {
        if !self.has_workspace(id) {
            return None;
        }
        Arc::make_mut(&mut self.workspaces)
            .iter_mut()
            .find(|w| w.id == id)
    }

    pub fn has_workspace(&self, id: WorkspaceId) -> bool {
        self.workspaces.iter().any(|w| w.id == id)
    }

    /// The first workspace in project order. A graph always has one.
    pub fn default_workspace(&self) -> WorkspaceId {
        self.workspaces
            .first()
            .expect("a graph always has at least one workspace")
            .id
    }

    /// The workspace id counter, for the file to carry: a reopened project carries on where
    /// it left off rather than reissuing an id a node still names.
    pub fn next_workspace_id(&self) -> u32 {
        self.next_workspace_id
    }

    /// Append a workspace and return its id.
    pub fn add_workspace(&mut self, name: String, kind: WorkspaceKind) -> WorkspaceId {
        self.next_workspace_id += 1;
        let id = WorkspaceId(self.next_workspace_id);
        Arc::make_mut(&mut self.workspaces).push(Workspace {
            id,
            name,
            kind,
            blurb: String::new(),
            layout: LayoutMode::default(),
        });
        id
    }

    /// A workspace's one-line blurb. Document data, saved in that workspace's own file.
    pub fn set_blurb(&mut self, id: WorkspaceId, blurb: String) -> bool {
        let Some(workspace) = self.workspace_mut(id) else {
            return false;
        };
        workspace.blurb = blurb;
        true
    }

    pub fn rename_workspace(&mut self, id: WorkspaceId, name: String) -> bool {
        let Some(workspace) = self.workspace_mut(id) else {
            return false;
        };
        workspace.name = name;
        true
    }

    /// Move a workspace to an index in project order. The index is clamped to the end.
    pub fn move_workspace(&mut self, id: WorkspaceId, to: usize) -> bool {
        let Some(from) = self.workspaces.iter().position(|w| w.id == id) else {
            return false;
        };
        let to = to.min(self.workspaces.len() - 1);
        let workspaces = Arc::make_mut(&mut self.workspaces);
        let workspace = workspaces.remove(from);
        workspaces.insert(to, workspace);
        true
    }

    /// Nodes that would be left on no workspace if this one went.
    pub fn nodes_only_on(&self, id: WorkspaceId) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.workspaces.len() == 1 && n.workspaces.contains(&id))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Remove a workspace, every node left on none, and every cable touching those nodes.
    ///
    /// Returns the nodes that went with it. `None` means nothing was removed: the id names
    /// no workspace, or it is the last one, which cannot go because a node has to be
    /// somewhere.
    pub fn remove_workspace(&mut self, id: WorkspaceId) -> Option<Vec<NodeId>> {
        if self.workspaces.len() < 2 || !self.has_workspace(id) {
            return None;
        }
        Arc::make_mut(&mut self.workspaces).retain(|w| w.id != id);
        let orphans = self.nodes_only_on(id);
        // Only the nodes that were on it: every other node stays shared.
        for node in self.nodes.values_mut() {
            if node.workspaces.contains(&id) {
                Arc::make_mut(node).workspaces.remove(&id);
            }
        }
        for node in &orphans {
            self.remove_node(*node);
        }
        Some(orphans)
    }

    /// Replace the workspace list wholesale. Loading a file, and nothing else.
    ///
    /// An empty list is ignored: a graph with no workspace has nowhere to put a node.
    pub fn set_workspaces(&mut self, workspaces: Vec<Workspace>, next_id: u32) {
        let Some(highest) = workspaces.iter().map(|w| w.id.0).max() else {
            return;
        };
        self.next_workspace_id = next_id.max(highest);
        self.workspaces = Arc::new(workspaces);
    }

    /// Every node shown on one workspace, in id order.
    ///
    /// The canvas draws exactly this, and the auto-arrange places exactly this: a workspace
    /// is a view, so everything that is about one canvas asks the set rather than the graph.
    pub fn on_workspace(&self, id: WorkspaceId) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .filter(move |(_, n)| n.workspaces.contains(&id))
            .map(|(id, node)| (*id, node.as_ref()))
    }

    /// Where a link that shows this node goes: `active` where the node is on it, else the
    /// first workspace in project order that the node is on and `open` has a tab for, else
    /// the first it is on at all. `None` for a node that is not there.
    ///
    /// The one answer for every link to a node — a tag, an asset's user, the Status box, the
    /// MIDI window, a deck, an Output's `!` — so none of them opens a tab where one that is
    /// already open would do.
    pub fn home_of(
        &self,
        node: NodeId,
        active: Option<WorkspaceId>,
        open: &BTreeSet<WorkspaceId>,
    ) -> Option<WorkspaceId> {
        let on = &self.get(node)?.workspaces;
        let ordered = || {
            self.workspaces
                .iter()
                .map(|w| w.id)
                .filter(|w| on.contains(w))
        };
        active
            .filter(|w| on.contains(w))
            .or_else(|| ordered().find(|w| open.contains(w)))
            .or_else(|| ordered().next())
    }

    // ---------------------------------------------------------------- layout

    /// How one workspace is navigated. `Canvas` for an id that names no workspace.
    pub fn layout_of(&self, id: WorkspaceId) -> LayoutMode {
        self.workspace(id)
            .map_or(LayoutMode::default(), |w| w.layout)
    }

    /// Set one workspace's layout mode. False if the id names no workspace.
    pub fn set_layout(&mut self, id: WorkspaceId, layout: LayoutMode) -> bool {
        let Some(workspace) = self.workspace_mut(id) else {
            return false;
        };
        workspace.layout = layout;
        true
    }

    // ---------------------------------------------------------------- connections

    pub fn connections(&self) -> &[Connection] {
        &self.cables.list
    }

    /// Whether two graphs are the same document: the same workspaces, nodes and cables in the
    /// same order, and the same two counters. What an autosave is held against the save it
    /// followed.
    pub fn same_document(&self, other: &Graph) -> bool {
        self.next_id == other.next_id
            && self.next_workspace_id == other.next_workspace_id
            && self.workspaces == other.workspaces
            && self.nodes == other.nodes
            && self.cables.list == other.cables.list
    }

    /// Check a prospective connection without making it.
    pub fn can_connect(&self, from: PortRef, to: PortRef) -> Result<(), ConnectError> {
        self.check(from, to, None)
    }

    /// [`Graph::can_connect`], with the cycle question answered from a drag's [`Reach`]
    /// where it can be: what the canvas asks of every candidate port while a cable is in
    /// flight.
    pub fn can_connect_within(
        &self,
        reach: &Reach,
        from: PortRef,
        to: PortRef,
    ) -> Result<(), ConnectError> {
        self.check(from, to, Some(reach))
    }

    fn check(&self, from: PortRef, to: PortRef, reach: Option<&Reach>) -> Result<(), ConnectError> {
        let from_node = self
            .nodes
            .get(&from.node)
            .ok_or(ConnectError::NoSuchNode(from.node))?;
        let to_node = self
            .nodes
            .get(&to.node)
            .ok_or(ConnectError::NoSuchNode(to.node))?;

        let from_port = from_node
            .output(from.key)
            .ok_or(ConnectError::NoSuchPort(from))?;
        let to_port = to_node.input(to.key).ok_or(ConnectError::NoSuchPort(to))?;

        if from.node == to.node {
            return Err(ConnectError::SelfConnection(from.node));
        }

        if crate::nodes::timing::is_inactive(to_node, to.key) {
            return Err(ConnectError::Inactive(to));
        }

        if from_port.ty.is_data() != to_port.ty.is_data() {
            return Err(ConnectError::ActionMismatch {
                from: from_port.ty,
                to: to_port.ty,
            });
        }
        if !from_port.ty.feeds(to_port.ty) {
            return Err(ConnectError::TypeMismatch {
                from: from_port.ty,
                to: to_port.ty,
            });
        }

        // Only an immediate data loop can deadlock the compiler. Action edges are CPU
        // events, and a loop through a delayed port is feedback: the consumer reads a
        // finished frame, so the compiler samples a texture instead of recursing.
        if Flow::of(from_port).is_immediate() && self.can_reach_within(reach, to.node, from.node) {
            return Err(ConnectError::WouldCycle {
                from: from.node,
                to: to.node,
            });
        }

        Ok(())
    }

    /// Connect an output to an input. Replaces any existing connection into a data input.
    ///
    /// `Ok(false)` where that very cable is already there, which changes nothing: not the
    /// list, not its order and not a type.
    pub fn connect(&mut self, from: PortRef, to: PortRef) -> Result<bool, ConnectError> {
        self.can_connect(from, to)?;
        Ok(self.link(from, to))
    }

    /// The same connection with the pinning refusal left to
    /// [`Graph::settle_effective_types`]: what a **bulk** change uses, since whether one of
    /// its cables demotes a dual node would otherwise depend on the order a file lists them.
    ///
    /// It walks past a field onto a **dual** input, and onto any `UniformNumber` input from a
    /// dual output inside a bulk change, whose type is not settled yet. Every other rule
    /// still applies.
    pub fn connect_loading(&mut self, from: PortRef, to: PortRef) -> Result<(), ConnectError> {
        match self.can_connect(from, to) {
            Ok(()) => {
                self.link(from, to);
                Ok(())
            }
            Err(ConnectError::TypeMismatch {
                from: PortType::VaryingNumber,
                to: PortType::UniformNumber,
            }) if self.is_dual_input(to) || (self.bulk && self.is_dual_output(from)) => {
                self.link(from, to);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Is this an input whose `UniformNumber` is the graph's answer rather than its own
    /// declaration?
    fn is_dual_input(&self, to: PortRef) -> bool {
        self.nodes
            .get(&to.node)
            .and_then(|n| n.input(to.key))
            .is_some_and(|p| p.dual)
    }

    /// Is this an output whose type is the graph's answer rather than its own declaration?
    fn is_dual_output(&self, from: PortRef) -> bool {
        self.nodes
            .get(&from.node)
            .and_then(|n| n.output(from.key))
            .is_some_and(|p| p.dual)
    }

    /// Begin a **bulk** change: a file load, an import, a paste — many nodes and cables that
    /// arrive as one change rather than as a sequence of gestures.
    ///
    /// Until [`Graph::settle_effective_types`] ends it, an insert or a cable leaves the dual
    /// ports' effective types alone rather than working them out again across the graph,
    /// which made a load quadratic. `settle` works them out once, from the whole graph.
    pub fn begin_bulk(&mut self) {
        self.bulk = true;
    }

    /// Is a bulk change open? A caller inside one leaves the settling to whoever opened it.
    pub fn in_bulk(&self) -> bool {
        self.bulk
    }

    /// Seat a checked edge. `can_connect` has already passed. False where the edge is
    /// already there.
    fn link(&mut self, from: PortRef, to: PortRef) -> bool {
        // Action edges are many-to-many, but the same edge twice is still one edge; and a
        // data input fed by this very source already has the cable it is being given.
        if self.sources_of(to).any(|f| f == from) {
            return false;
        }
        let is_data = self.nodes[&to.node]
            .input(to.key)
            .expect("checked by can_connect")
            .ty
            .is_data();
        if is_data {
            self.unlink_where(|c| c.to == to);
        }
        let flow = Flow::of(
            self.nodes[&from.node]
                .output(from.key)
                .expect("checked by can_connect"),
        );
        Arc::make_mut(&mut self.cables).push(Connection { from, to }, flow);
        self.invalidate();
        self.recompute_effective_types(&[to.node]);
        true
    }

    /// Remove every cable `doomed` picks, from the list and its index together. Returns the
    /// ones removed, and copies nothing where there are none — a graph an undo step shares
    /// stays shared.
    ///
    /// The derived views are the caller's to drop, since a caller removing nothing drops
    /// nothing.
    fn unlink_where(&mut self, doomed: impl Fn(&Connection) -> bool) -> Vec<Connection> {
        if !self.cables.list.iter().any(&doomed) {
            return Vec::new();
        }
        Arc::make_mut(&mut self.cables).remove_where(doomed)
    }

    /// Remove whatever feeds this input. Returns the connections that were removed.
    pub fn disconnect(&mut self, to: PortRef) -> Vec<Connection> {
        let removed = self.unlink_where(|c| c.to == to);
        if !removed.is_empty() {
            self.invalidate();
            self.recompute_effective_types(&[to.node]);
        }
        removed
    }

    /// Remove every cable into one of `id`'s inputs that its time mode puts away
    /// (`nodes::is_inactive`): what a change of mode drops in the same step, so a gear's
    /// Cycles left in a Speed never races off as a rate. Returns the ones removed.
    pub fn drop_inactive(&mut self, id: NodeId) -> Vec<Connection> {
        let Some(node) = self.nodes.get(&id) else {
            return Vec::new();
        };
        let inactive: Vec<&'static str> = node
            .inputs
            .iter()
            .map(|p| p.key)
            .filter(|key| crate::nodes::timing::is_inactive(node, key))
            .collect();
        let removed = self.unlink_where(|c| c.to.node == id && inactive.contains(&c.to.key));
        if !removed.is_empty() {
            self.invalidate();
            self.recompute_effective_types(&[id]);
        }
        removed
    }

    /// Remove every connection on one port, whichever side of it that port is bound as.
    /// Returns the ones removed. What a right-click on a port dot does: an input takes this
    /// over `disconnect` because the gesture reads the same on an output, which `disconnect`
    /// cannot touch at all.
    pub fn disconnect_port(&mut self, port: PortRef) -> Vec<Connection> {
        let removed = self.unlink_where(|c| c.from == port || c.to == port);
        if !removed.is_empty() {
            self.invalidate();
            let consumers: Vec<NodeId> = removed.iter().map(|c| c.to.node).collect();
            self.recompute_effective_types(&consumers);
        }
        removed
    }

    /// Remove every connection touching a node, in and out. Returns the ones removed.
    pub fn disconnect_node(&mut self, id: NodeId) -> Vec<Connection> {
        let removed = self.unlink_where(|c| c.from.node == id || c.to.node == id);
        if !removed.is_empty() {
            self.invalidate();
            let consumers: Vec<NodeId> = removed.iter().map(|c| c.to.node).collect();
            self.recompute_effective_types(&consumers);
        }
        removed
    }

    /// Remove one specific edge, which is what clicking a single cable does.
    pub fn disconnect_edge(&mut self, from: PortRef, to: PortRef) -> bool {
        let removed = !self
            .unlink_where(|c| c.from == from && c.to == to)
            .is_empty();
        if removed {
            self.invalidate();
            self.recompute_effective_types(&[to.node]);
        }
        removed
    }

    /// What feeds this data input, if anything.
    ///
    /// Out of the index: the cables into this one node, which is a handful, rather than every
    /// cable in the graph. The same for the two below.
    pub fn source_of(&self, to: PortRef) -> Option<PortRef> {
        self.cables
            .entering(to.node)
            .iter()
            .find(|c| c.to.key == to.key)
            .map(|c| c.from)
    }

    /// Everything feeding this input. A data input has at most one; an action input may
    /// have many, because action connections are many-to-many.
    pub fn sources_of(&self, to: PortRef) -> impl Iterator<Item = PortRef> + '_ {
        self.cables
            .entering(to.node)
            .iter()
            .filter(move |c| c.to.key == to.key)
            .map(|c| c.from)
    }

    /// Every cable into one node, whatever input it lands on, out of the index.
    pub fn cables_into(&self, node: NodeId) -> &[Cable] {
        self.cables.entering(node)
    }

    /// Every input this output feeds. For action outputs there may be many.
    pub fn targets_of(&self, from: PortRef) -> impl Iterator<Item = PortRef> + '_ {
        self.cables
            .leaving(from.node)
            .iter()
            .filter(move |c| c.from.key == from.key)
            .map(|c| c.to)
    }

    // ---------------------------------------------------------------- dual ports

    /// What rate a node's dual outputs run at on this instance.
    ///
    /// `Uniform` while every one of its inputs resolves to a uniform: a connected input
    /// whose source is effectively uniform, or an unconnected one holding a number or
    /// color control, which compiles to a uniform anyway. Anything else — a fragment
    /// cabled in, an input bound to a prelude global, an action button — makes the node a
    /// fragment, because that is what it has to read.
    fn effective_rate(&self, id: NodeId) -> Rate {
        let Some(node) = self.nodes.get(&id) else {
            return Rate::Varying;
        };
        let resolves = |p: &PortDef| match self.source_of(PortRef::new(id, p.key)) {
            Some(src) => self
                .nodes
                .get(&src.node)
                .and_then(|n| n.output(src.key))
                .is_some_and(|o| o.ty.is_uniform()),
            None => matches!(
                node.controls.get(p.key),
                Some(ControlValue::Float(_) | ControlValue::Color(_))
            ),
        };
        if node.inputs.iter().all(resolves) {
            Rate::Uniform
        } else {
            Rate::Varying
        }
    }

    /// Write one node's dual outputs' effective type into its instance ports.
    fn retype(&mut self, id: NodeId) {
        if !self.nodes.get(&id).is_some_and(|n| Self::is_dual(n)) {
            return;
        }
        let rate = self.effective_rate(id);
        if let Some(node) = self.nodes.get_mut(&id) {
            Self::set_rate(node, true, rate);
        }
    }

    /// Put every dual port on one side of a node at `rate`: its outputs, or its inputs.
    ///
    /// Only a node whose answer changes is written, so every other one stays shared with
    /// the graph before the edit.
    fn set_rate(node: &mut Arc<Node>, outputs: bool, rate: Rate) {
        let side = if outputs { &node.outputs } else { &node.inputs };
        let stands = side
            .iter()
            .filter(|p| p.dual)
            .all(|p| p.ty == p.ty.with_rate(rate));
        if stands {
            return;
        }
        let node = Arc::make_mut(node);
        let ports = if outputs {
            &mut node.outputs
        } else {
            &mut node.inputs
        };
        for p in ports.iter_mut().filter(|p| p.dual) {
            p.ty = p.ty.with_rate(rate);
        }
    }

    /// Write every dual node's dual inputs' effective type into its instance ports.
    ///
    /// A dual node is **pinned** when it publishes a uniform number and flipping it to a
    /// field would leave one of its flipped outputs feeding a `UniformNumber` input — so a
    /// field cannot land on any of its inputs, and those inputs read `UniformNumber`, which
    /// is the type a cable may bring them.
    ///
    /// Computed the way it propagates, which is **upstream**: one pass over the connections
    /// finds the dual nodes whose number a `UniformNumber` input reads directly, and a walk
    /// back through the dual producers feeding those carries the answer to every node that
    /// would flip with them. Asking each dual node the question separately is the same answer
    /// at the cost of a downstream walk per node, and each of those rescans the edge list.
    ///
    /// Nothing here reads a pin, only the static `PortDef::dual` and the output modes settled
    /// before it, so the answer does not depend on what the last sweep wrote.
    ///
    /// Pinning never orphans a cable: a pinned node is in `UniformNumber` mode, so every
    /// input of it that is connected at all is fed by a uniform number, and a uniform number
    /// feeds a `UniformNumber` input.
    fn repin(&mut self) {
        // Seeded with the nodes a reader pins outright, beside the reverse edges a pin
        // travels back along: a consumer that would flip, to the producers flipping it.
        let mut pinned: HashSet<NodeId> = HashSet::new();
        let mut producers: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for c in &self.cables.list {
            // Only a dual output publishing a uniform number can flip, and only a flip
            // demotes.
            let flips = self
                .nodes
                .get(&c.from.node)
                .and_then(|n| n.output(c.from.key))
                .is_some_and(|p| p.dual && p.ty.is_uniform());
            if !flips {
                continue;
            }
            let Some(consumer) = self.nodes.get(&c.to.node) else {
                continue;
            };
            let Some(input) = consumer.input(c.to.key) else {
                continue;
            };
            if input.ty.is_uniform() && !input.dual {
                pinned.insert(c.from.node);
            } else if Self::is_dual(consumer) {
                producers.entry(c.to.node).or_default().push(c.from.node);
            }
        }
        let mut queue: VecDeque<NodeId> = pinned.iter().copied().collect();
        while let Some(n) = queue.pop_front() {
            for producer in producers.get(&n).into_iter().flatten() {
                if pinned.insert(*producer) {
                    queue.push_back(*producer);
                }
            }
        }
        for (id, node) in &mut self.nodes {
            if !Self::is_dual(node) {
                continue;
            }
            let rate = if pinned.contains(id) {
                Rate::Uniform
            } else {
                Rate::Varying
            };
            Self::set_rate(node, false, rate);
        }
    }

    /// Recompute every dual output at or below `seeds`, then every dual node's input pins.
    ///
    /// A dual output is only ever fed downstream, and it is never delayed, so the closure
    /// over immediate edges is exactly the set a change can reach. Those edges never close a
    /// loop, so their order puts every producer before its consumers — a loop through a
    /// delayed port included — and a chain of duals settles in one pass. The pins are a
    /// sweep of their own, because they run the other way; see [`Graph::repin`].
    ///
    /// **Nothing derived is invalidated.** A flip between `VaryingNumber` and
    /// `UniformNumber` leaves a cable's `Flow` alone — both are data and a dual port is never
    /// delayed — so the index and the orders still describe the graph.
    ///
    /// Inside a bulk change this does nothing: the change ends in
    /// [`Graph::settle_effective_types`], which works every type out at once.
    pub fn recompute_effective_types(&mut self, seeds: &[NodeId]) {
        if self.bulk || !self.has_duals() {
            return;
        }
        if !seeds.is_empty() {
            let mut affected = self.walk(seeds.iter().copied(), true);
            affected.extend(seeds.iter().copied());
            for id in self
                .immediate_order()
                .into_iter()
                .filter(|id| affected.contains(id))
            {
                self.retype(id);
            }
        }
        self.repin();
    }

    /// Recompute every dual port in the graph, then drop the cables that leaves illegal.
    ///
    /// What a **bulk** change ends with — a file load, an import, a paste, the project file's
    /// glue — and what closes the one [`Graph::begin_bulk`] opened. The types are a function
    /// of the whole graph rather than of the order its cables arrived in, so they are
    /// resolved once at the end, and a consumer left reading a field as a uniform number
    /// loses its cable here rather than staying in the graph as an edge the editor could not
    /// have made. Returns the connections dropped, for the caller to report.
    pub fn settle_effective_types(&mut self) -> Vec<Connection> {
        self.bulk = false;
        let mut dropped = Vec::new();
        if !self.has_duals() {
            return dropped;
        }
        // One pass is almost always the whole of it: every cable dropped carries a field
        // into an input that reads a uniform number, and a node in `UniformNumber` mode has
        // no field feeding it, so a drop rarely changes an answer. A bulk change may have
        // seated a dual output's cable before its type was known, though, and dropping one
        // of those can hand its consumer back its knob — so the sweep runs again until
        // nothing drops. Each pass drops a cable, so it ends.
        loop {
            for id in self.immediate_order() {
                self.retype(id);
            }
            self.repin();
            let illegal: Vec<Connection> = self
                .cables
                .list
                .iter()
                .filter(|c| {
                    let source = self
                        .nodes
                        .get(&c.from.node)
                        .and_then(|n| n.output(c.from.key))
                        .map(|p| p.ty);
                    let target = self
                        .nodes
                        .get(&c.to.node)
                        .and_then(|n| n.input(c.to.key))
                        .map(|p| p.ty);
                    matches!((source, target), (Some(from), Some(to)) if !from.feeds(to))
                })
                .copied()
                .collect();
            if illegal.is_empty() {
                return dropped;
            }
            let removed = self.unlink_where(|c| illegal.contains(c));
            self.invalidate();
            dropped.extend(removed);
        }
    }

    /// Does any node here have a dual output? A graph with no uniform-number maths pays
    /// nothing.
    fn has_duals(&self) -> bool {
        self.nodes.values().any(|n| Self::is_dual(n))
    }

    /// A node whose port types are not its own: the one that carries a dual output, and
    /// therefore the dual inputs feeding it.
    fn is_dual(node: &Node) -> bool {
        node.outputs.iter().any(|p| p.dual)
    }

    // ---------------------------------------------------------------- traversal

    /// Every node reachable from `seeds` over immediate data edges, the seeds excluded
    /// unless one reaches another. `downstream` picks the direction: producers to
    /// consumers, or the reverse.
    ///
    /// Immediate, so the answer is also where the compiler would descend: the walk it makes
    /// from an Output's input stops at a delayed port.
    fn walk(&self, seeds: impl IntoIterator<Item = NodeId>, downstream: bool) -> HashSet<NodeId> {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<NodeId> = seeds.into_iter().collect();
        while let Some(n) = queue.pop_front() {
            let (cables, far): (&[Cable], fn(&Cable) -> NodeId) = if downstream {
                (self.cables.leaving(n), |c| c.to.node)
            } else {
                (self.cables.entering(n), |c| c.from.node)
            };
            for c in cables.iter().filter(|c| c.flow.is_immediate()) {
                if seen.insert(far(c)) {
                    queue.push_back(far(c));
                }
            }
        }
        seen
    }

    /// Every node upstream of `node` over immediate data edges: everything its value is
    /// computed from this frame.
    pub fn upstream_of(&self, node: NodeId) -> HashSet<NodeId> {
        self.walk([node], false)
    }

    /// Every node downstream of `node` over immediate data edges: everything computed from
    /// its value this frame.
    pub fn downstream_of(&self, node: NodeId) -> HashSet<NodeId> {
        self.walk([node], true)
    }

    /// Can `start` reach `goal` by following immediate data edges downstream? A node reaches
    /// itself. The cycle check asks it of a prospective cable. One walk per call: a caller
    /// asking about one node many times keeps the set, as a cable drag keeps a [`Reach`].
    pub fn can_reach(&self, start: NodeId, goal: NodeId) -> bool {
        start == goal || self.walk([goal], false).contains(&start)
    }

    /// [`Graph::can_reach`], answered from a drag's [`Reach`] where it covers the question.
    pub fn can_reach_within(&self, reach: Option<&Reach>, start: NodeId, goal: NodeId) -> bool {
        reach
            .filter(|r| Arc::ptr_eq(&r.cables, &self.cables))
            .and_then(|r| r.answers(start, goal))
            .unwrap_or_else(|| self.can_reach(start, goal))
    }

    /// Walk where one end of a cable drag reaches: upstream of an output, downstream of an
    /// input. Once per drag; see [`Reach`].
    pub fn reach(&self, end: PortRef) -> Reach {
        let upstream = self
            .nodes
            .get(&end.node)
            .is_some_and(|n| n.output(end.key).is_some());
        Reach {
            end,
            upstream,
            nodes: self.walk([end.node], !upstream),
            cables: Arc::clone(&self.cables),
        }
    }

    /// Which Output nodes are downstream of this node, following data edges.
    ///
    /// This is what decides whose shader has to be rebuilt when a connection changes. An
    /// Output that is itself downstream of another Output is included: §2b's `frame` port
    /// makes that a real edge.
    pub fn downstream_outputs(&self, start: NodeId) -> Vec<NodeId> {
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        let mut outputs = Vec::new();

        if self.nodes.get(&start).is_some_and(|n| n.def.is_output) {
            outputs.push(start);
        }
        while let Some(n) = queue.pop_front() {
            for c in self.cables.leaving(n).iter().filter(|c| c.flow.is_data()) {
                let next = c.to.node;
                if seen.insert(next) {
                    if self.nodes.get(&next).is_some_and(|n| n.def.is_output) {
                        outputs.push(next);
                    }
                    queue.push_back(next);
                }
            }
        }
        outputs
    }

    /// An estimate of the heap this graph occupies, in bytes.
    ///
    /// A `Graph`'s real size is not measurable without an allocator hook: this sums a
    /// per-node constant with the entries that actually vary between graphs. Calibrated so
    /// that a 200-node graph reads near 150 KB, which is what one measures.
    pub fn footprint(&self) -> usize {
        let nodes: usize = self.nodes.values().map(|n| footprint::node(n)).sum();
        self.nodes.len() * footprint::ENTRY
            + nodes
            + self.cables.list.len() * footprint::CONNECTION
            + self.workspaces.len() * footprint::WORKSPACE
    }

    /// What this graph holds that `other` does not, in the bytes [`Self::footprint`] counts.
    ///
    /// The undo ring is capped by this. Two graphs an edit apart share every node it did not
    /// write, and share the cable list and the workspace list unless it wrote those, so a
    /// step costs its own map of node pointers and whatever it holds alone — a scrub of one
    /// knob is one node, not the whole graph.
    pub fn footprint_beside(&self, other: &Self) -> usize {
        let nodes: usize = self
            .nodes
            .iter()
            .filter(|(id, n)| other.nodes.get(id).is_none_or(|o| !Arc::ptr_eq(n, o)))
            .map(|(id, n)| footprint::node_beside(n, other.nodes.get(id).map(|o| &**o)))
            .sum();
        let connections = if Arc::ptr_eq(&self.cables, &other.cables) {
            0
        } else {
            self.cables.list.len() * footprint::CONNECTION
        };
        let workspaces = if Arc::ptr_eq(&self.workspaces, &other.workspaces) {
            0
        } else {
            self.workspaces.len() * footprint::WORKSPACE
        };
        self.nodes.len() * footprint::ENTRY + nodes + connections + workspaces
    }

    /// Drop both orders. Any change to the node set or the connections calls this; control
    /// values and positions do not, because nothing derived depends on them.
    pub(crate) fn invalidate(&mut self) {
        self.topo.take();
        self.tick_topo.take();
    }
}

/// The constants [`Graph::footprint`] sums.
mod footprint {
    use super::Node;

    /// One entry in the graph's map: an id and a pointer, in a B-tree node with room for
    /// more.
    pub const ENTRY: usize = 24;
    /// A `Node` with its port lists, its maps and their table allocations.
    const NODE: usize = 320;
    /// One `PortDef` in a node's copy of its definition's port lists.
    const PORT: usize = 32;
    /// One entry in `controls`, in a table sized for more.
    const CONTROL: usize = 48;
    /// One entry in `options`: a key and a heap-allocated value.
    const OPTION: usize = 96;
    /// One entry in `values`: a range is `Copy` and small, a note's prose is not, so this is
    /// the floor rather than the figure. A graph whose notes are essays pays more than the
    /// estimate says, and the byte budget is a budget, not an accountant.
    const VALUE: usize = 48;
    /// One workspace id in a node's set, as a `BTreeSet` node holds it.
    const MEMBERSHIP: usize = 32;
    /// One cable, with its two index entries and its share of the two orders.
    pub const CONNECTION: usize = 96;
    pub const WORKSPACE: usize = 96;

    pub fn node(n: &Node) -> usize {
        node_beside(n, None)
    }

    /// One node, counting a painting's pixels unless `other` — the same node on the far side
    /// of an edit — holds that very picture. A painting is the one value that can be
    /// megabytes, and a step that turned a brush knob shares its picture with the graph
    /// beside it, so counting it there would spend the budget on bytes nobody copied.
    pub fn node_beside(n: &Node, other: Option<&Node>) -> usize {
        let pictures: usize = n
            .values
            .iter()
            .filter_map(|(key, v)| Some((key, v.painting()?)))
            .filter(|(key, p)| {
                other
                    .and_then(|o| o.values.get(*key))
                    .and_then(super::Value::painting)
                    .is_none_or(|q| !q.shares(p))
            })
            .map(|(_, p)| p.bytes())
            .sum();
        NODE + (n.inputs.len() + n.outputs.len()) * PORT
            + n.controls.len() * CONTROL
            + n.options.len() * OPTION
            + n.values.len() * VALUE
            + n.workspaces.len() * MEMBERSHIP
            + pictures
    }
}
