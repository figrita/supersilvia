// SPDX-License-Identifier: AGPL-3.0-or-later

//! The plan: which Outputs draw, with which shaders, fed from where.
//!
//! **A plan, not a frame.** What crosses is what the compiler decided, with each uniform as a
//! *provider*, and the synth turns it into a `FrameJob` on every tick of its own clock.
//! **A plan is built only when something it reads changed** — [`PlanKey`], or a rebuild or a
//! lost source waiting — so a scrub and an untouched frame build and send nothing. **A source
//! is sent once**: shaders and probes are hashed as they are built, and one the renderer
//! holds is not sent again. Which pass measures what ([`measured_on`]) and which Outputs draw
//! ([`Rule`], argued in [docs/rendering.md](../../../docs/rendering.md#which-outputs-draw))
//! are here because both change what is compiled or whether it runs.

use super::{BatchLink, OutputLink, SynthLink};
use crate::compile;
use crate::graph::{Graph, NodeId, PortRef, WorkspaceId};
use crate::maininput::MainInput;
use crate::mixer::Mixer;
use crate::project::AssetPaths;
use crate::render::Display;
use crate::render::{FrameJob, PassKey};
use crate::synth::{Host, Mix, Mode, Msg, Why, providers};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// What the plan reads that is not the link's own. `App` assembles one where it asks for a
/// plan.
pub(in crate::app) struct Session<'a> {
    pub graph: &'a Graph,
    /// The document's count of changes to what a plan reads off the graph. See
    /// `Document::shape`.
    pub shape: u64,
    /// The workspaces with a tab.
    pub open: &'a BTreeSet<WorkspaceId>,
    /// The workspace on screen: its Outputs draw, its pass draws thumbnails, and a reading
    /// on it is shown.
    pub active: Option<WorkspaceId>,
    pub mixer: &'a Mixer,
    /// The canvas in physical pixels, which a viewport-matched mix is sized to.
    pub mix_viewport: (u32, u32),
    /// Whether anything will read the per-node half of a tick.
    pub report: bool,
    /// Whether the Status box is open.
    pub status: bool,
    /// Whether the Main Input panel is unfolded, the only reason its picture is uploaded.
    pub preview: bool,
    /// The nodes whose own picture has a window of its own.
    pub windows: BTreeSet<NodeId>,
    /// The Outputs sent out over Syphon or NDI.
    pub sent: BTreeSet<NodeId>,
    /// The Output the offline render is capturing, while one runs.
    pub capture: Option<NodeId>,
}

impl Session<'_> {
    /// The mix as a plan states it: the decks and the size, and not the fade.
    fn mix(&self) -> Mix {
        Mix {
            a: self.mixer.a,
            b: self.mixer.b,
            resolution: self.mixer.resolution.pixels(self.mix_viewport),
        }
    }

    fn live_key(&self) -> LiveKey {
        LiveKey {
            shape: self.shape,
            open: self.open.clone(),
            decks: (self.mixer.a, self.mixer.b),
            sent: self.sent.clone(),
        }
    }
}

/// Everything a plan is a function of besides the shaders, compared whole.
#[derive(Debug, PartialEq)]
pub(super) struct PlanKey {
    shape: u64,
    open: BTreeSet<WorkspaceId>,
    active: Option<WorkspaceId>,
    mix: Mix,
    windows: BTreeSet<NodeId>,
    sent: BTreeSet<NodeId>,
    capture: Option<NodeId>,
}

impl PlanKey {
    fn of(from: &Session<'_>) -> Self {
        Self {
            shape: from.shape,
            open: from.open.clone(),
            active: from.active,
            mix: from.mix(),
            windows: from.windows.clone(),
            sent: from.sent.clone(),
            capture: from.capture,
        }
    }
}

/// What the live set is a function of: the graph's shape, the open tabs, the decks and the
/// Outputs sent out over Syphon or NDI.
#[derive(Debug, PartialEq)]
pub(super) struct LiveKey {
    shape: u64,
    open: BTreeSet<WorkspaceId>,
    decks: (Option<NodeId>, Option<NodeId>),
    sent: BTreeSet<NodeId>,
}

/// What the tick was last given beside the plan, so it is sent again only when it differs.
#[derive(Debug, PartialEq)]
pub(super) struct InputsKey {
    live: u64,
    report: bool,
    status: bool,
    preview: bool,
    assets: AssetPaths,
    main_input: MainInput,
}

/// Every node shown on a workspace that is open, everything feeding a deck or an Output
/// sent out over Syphon or NDI, and every gear a node on an open workspace reads, with its own
/// upstream.
///
/// A node on no open workspace is **suspended**: its `tick` is not called and its Output is
/// not drawn. **On air outranks closed**: an Output on a mixer deck renders, and everything
/// upstream of it ticks, whatever its tabs are doing, so tidying the editor never freezes the
/// show. An Output another app takes over Syphon or NDI is on air there, and is held the same.
/// **A clock read outranks closed** the same way: a gear cabled into a node on an open tab
/// keeps counting, and what it counts with it, so closing the tab a gear sits on never stops
/// the time of what is shown.
pub(in crate::app) fn live_nodes(
    graph: &Graph,
    open: &BTreeSet<WorkspaceId>,
    mixer: &Mixer,
    sent: &BTreeSet<NodeId>,
) -> HashSet<NodeId> {
    let mut live: HashSet<NodeId> = graph
        .iter()
        .filter(|(_, n)| n.workspaces.iter().any(|w| open.contains(w)))
        .map(|(id, _)| id)
        .collect();
    let clocks: Vec<NodeId> = live
        .iter()
        .flat_map(|id| graph.cables_into(*id))
        .map(|c| c.from.node)
        .filter(|from| {
            !live.contains(from)
                && graph
                    .get(*from)
                    .is_some_and(|n| n.def.category == crate::nodes::Category::Gear)
        })
        .collect();
    for root in mixer.claimed().chain(sent.iter().copied()).chain(clocks) {
        live.extend(upstream_over_any_cable(graph, root));
    }
    live
}

/// Which workspace's pass measures each awake measured node: per workspace, in id order.
///
/// A node with a `measure` is measured once, in one pass — the first of its workspaces, in
/// project order, that has a tab, or, for a node awake only because a deck or a send reads
/// it, the first of its workspaces. A suspended node is measured nowhere: it does not tick,
/// so nothing would read the slot. See [docs/cpu.md](../../../docs/cpu.md#taps-the-other-direction).
fn measured_on(
    graph: &Graph,
    live: &HashSet<NodeId>,
    open: &BTreeSet<WorkspaceId>,
) -> BTreeMap<WorkspaceId, Vec<NodeId>> {
    let order: Vec<WorkspaceId> = graph.workspaces().iter().map(|w| w.id).collect();
    let mut on: BTreeMap<WorkspaceId, Vec<NodeId>> = BTreeMap::new();
    for (id, node) in graph.iter() {
        if !live.contains(&id) || node.def.measure_wgsl.is_none() {
            continue;
        }
        let first = |tab: bool| {
            order
                .iter()
                .copied()
                .find(|w| node.workspaces.contains(w) && (!tab || open.contains(w)))
        };
        if let Some(workspace) = first(true).or_else(|| first(false)) {
            on.entry(workspace).or_default().push(id);
        }
    }
    on
}

/// Every node `from` is computed from, over any cable, delayed ones included: the walk that
/// keeps a deck's upstream awake.
fn upstream_over_any_cable(graph: &Graph, from: NodeId) -> HashSet<NodeId> {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        if graph.get(id).is_none() || !seen.insert(id) {
            continue;
        }
        stack.extend(graph.cables_into(id).iter().map(|c| c.from.node));
    }
    seen
}

/// What one module reads of the rest of the graph, as the draw rule asks it: an Output's
/// picture, or one node's measurement.
struct Reads {
    /// The Outputs whose frame it samples, itself included where it feeds back.
    frames: Vec<NodeId>,
    /// The nodes whose published uniform numbers or colors it reads.
    uniforms: HashSet<NodeId>,
}

impl Reads {
    fn of(graph: &Graph, shader: &compile::Shader) -> Self {
        let mut frames = Vec::new();
        let mut uniforms = HashSet::new();
        for provider in shader.uniforms.values() {
            match provider {
                compile::UniformProvider::NodeTexture { node, .. }
                    if graph.get(*node).is_some_and(|n| n.def.is_output)
                        && !frames.contains(node) =>
                {
                    frames.push(*node);
                }
                compile::UniformProvider::NodeUniform { node, .. }
                | compile::UniformProvider::NodeCount { node, .. } => {
                    uniforms.insert(*node);
                }
                _ => {}
            }
        }
        Self { frames, uniforms }
    }
}

/// Which Outputs draw on every tick and why, what each Output needs drawn beside it, and
/// which measured nodes are measured. An eligible Output left out of `draws` is idle.
struct Drawn {
    draws: HashMap<NodeId, Why>,
    /// Per Output, every other Output that draws on a tick it draws, in render order.
    needs: HashMap<NodeId, Vec<NodeId>>,
    /// The measured nodes whose reading something reads, whose pass measures on every tick.
    taps: HashSet<NodeId>,
    /// Per Output, the measured nodes a tick it draws reads: a deck the synth claims before
    /// the editor replans draws their passes beside it.
    wants: HashMap<NodeId, HashSet<NodeId>>,
    /// Every live node in a loop, the Outputs among them drawn only while the playhead moves.
    cyclic: HashSet<NodeId>,
}

/// The draw rule, over what one plan reads: worked out once per plan. The rule is
/// [docs/rendering.md](../../../docs/rendering.md#which-outputs-draw).
struct Rule<'a> {
    /// What each eligible Output's shader reads.
    reads: &'a HashMap<NodeId, Reads>,
    /// Every Output that can draw — awake, with a shader — in render order.
    eligible: &'a [NodeId],
    /// Every awake measured node, in id order.
    measured: Vec<NodeId>,
    /// What each measurement reads, as its pass runs it: the frames it samples and the
    /// readings its chain takes.
    measures: HashMap<NodeId, Reads>,
    /// Each measured node with the dual nodes a tick evaluates from it: each of those is only
    /// a formula of the reading, and a shader reading one reads the measurement.
    readers: HashMap<NodeId, HashSet<NodeId>>,
    /// The measured nodes whose reading matters whatever draws.
    matters: HashSet<NodeId>,
    /// Every live node in a cycle of data edges. See [`Graph::in_cycles`].
    cyclic: HashSet<NodeId>,
}

impl<'a> Rule<'a> {
    fn new(
        graph: &Graph,
        reads: &'a HashMap<NodeId, Reads>,
        eligible: &'a [NodeId],
        live: &HashSet<NodeId>,
        active: Option<WorkspaceId>,
        measured: Vec<NodeId>,
    ) -> Self {
        let cyclic = graph.in_cycles(live);
        let mut readers = HashMap::new();
        let mut matters = HashSet::new();
        let mut measures = HashMap::new();
        for node in &measured {
            let (reached, read_live) = read_through(graph, *node, live, active);
            let integrates = graph
                .get(*node)
                .and_then(|n| n.def.cpu.as_ref())
                .is_some_and(|c| c.integrates);
            if integrates || read_live || cyclic.contains(node) {
                matters.insert(*node);
            }
            readers.insert(*node, reached);
            if let Some(shader) = compile::wgsl::build_measure(graph, *node) {
                measures.insert(*node, Reads::of(graph, &shader));
            }
        }
        Self {
            reads,
            eligible,
            measured,
            measures,
            readers,
            matters,
            cyclic,
        }
    }

    fn frames(&self, id: NodeId) -> &[NodeId] {
        self.reads.get(&id).map_or(&[][..], |r| r.frames.as_slice())
    }

    /// Whether a module reading `reads` reads the measured node's reading.
    fn reads_reading(&self, reads: Option<&Reads>, measured: NodeId) -> bool {
        match (reads, self.readers.get(&measured)) {
            (Some(r), Some(readers)) => !r.uniforms.is_disjoint(readers),
            _ => false,
        }
    }

    /// `draws` closed over what drawing reads: an Output whose frame a drawing one samples,
    /// every measured node whose reading a drawing Output's shader or a measured node's chain
    /// reads — or, where `whole`, whose reading matters whatever draws — and every Output
    /// whose frame such a measurement samples, until none adds one. The measured nodes it
    /// took are the second half.
    fn close(
        &self,
        mut draws: HashMap<NodeId, Why>,
        whole: bool,
    ) -> (HashMap<NodeId, Why>, HashSet<NodeId>) {
        let mut taps: HashSet<NodeId> = HashSet::new();
        loop {
            let mut stack: Vec<NodeId> = draws.keys().copied().collect();
            stack.sort_unstable();
            while let Some(reader) = stack.pop() {
                for read in self.frames(reader) {
                    if self.eligible.contains(read) && !draws.contains_key(read) {
                        draws.insert(*read, Why::Frame(reader));
                        stack.push(*read);
                    }
                }
            }
            let mut added = false;
            for node in &self.measured {
                if taps.contains(node) {
                    continue;
                }
                let read = (whole && self.matters.contains(node))
                    || draws
                        .keys()
                        .any(|id| self.reads_reading(self.reads.get(id), *node))
                    || taps
                        .iter()
                        .any(|m| self.reads_reading(self.measures.get(m), *node));
                if !read {
                    continue;
                }
                taps.insert(*node);
                added = true;
                let frames = self
                    .measures
                    .get(node)
                    .map_or(&[][..], |r| r.frames.as_slice());
                for frame in frames {
                    if self.eligible.contains(frame) && !draws.contains_key(frame) {
                        draws.insert(*frame, Why::Tap(*node));
                    }
                }
            }
            if !added {
                return (draws, taps);
            }
        }
    }

    /// The whole rule, from the roots the session names.
    fn apply(&self, graph: &Graph, from: &Session<'_>) -> Drawn {
        let mut roots: HashMap<NodeId, Why> = HashMap::new();
        for id in self.eligible {
            let Some(node) = graph.get(*id) else { continue };
            let why = if from.active.is_some_and(|w| node.workspaces.contains(&w)) {
                Why::Tab
            } else if from.mixer.a == Some(*id) || from.mixer.b == Some(*id) {
                Why::Deck
            } else if from.windows.contains(id) {
                Why::Window
            } else if from.sent.contains(id) {
                Why::Sent
            } else if from.capture == Some(*id) {
                Why::Capture
            } else if self.cyclic.contains(id) {
                Why::Feedback
            } else {
                continue;
            };
            roots.insert(*id, why);
        }
        let (draws, taps) = self.close(roots, true);

        let mut needs: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut wants: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();
        for id in self.eligible {
            let (closed, read) = self.close(HashMap::from([(*id, Why::Tab)]), false);
            let list = self
                .eligible
                .iter()
                .copied()
                .filter(|o| o != id && closed.contains_key(o))
                .collect();
            needs.insert(*id, list);
            wants.insert(*id, read);
        }
        Drawn {
            draws,
            needs,
            taps,
            wants,
            cyclic: self.cyclic.clone(),
        }
    }
}

/// What reads a measured node's reading as a shader would: the node and every dual node a
/// tick evaluates from it. And whether the reading is live for any other reason: one of
/// those is on the workspace being looked at, whose row shows the number, or an awake CPU
/// node reads one.
fn read_through(
    graph: &Graph,
    measured: NodeId,
    live: &HashSet<NodeId>,
    active: Option<WorkspaceId>,
) -> (HashSet<NodeId>, bool) {
    let mut reached = HashSet::from([measured]);
    let mut stack = vec![measured];
    let mut read_live = false;
    while let Some(id) = stack.pop() {
        let Some(node) = graph.get(id) else { continue };
        read_live |= active.is_some_and(|w| node.workspaces.contains(&w));
        for port in &node.outputs {
            if !port.ty.is_uniform() {
                continue;
            }
            for to in graph.targets_of(PortRef::new(id, port.key)) {
                let Some(target) = graph.get(to.node) else {
                    continue;
                };
                if target.def.cpu.is_some() {
                    read_live |= live.contains(&to.node);
                    continue;
                }
                let evaluated = target.def.outputs.iter().any(|o| {
                    o.eval.is_some() && target.output(o.key).is_some_and(|p| p.ty.is_uniform())
                });
                if evaluated && reached.insert(to.node) {
                    stack.push(to.node);
                }
            }
        }
    }
    (reached, read_live)
}

/// Every Output node, in the order their shaders must run: producers first. `frame`
/// edges between Outputs are ordinary data edges, so the graph's topological order
/// already sequences them (§2b). A measurement samples frames too, but in its workspace's
/// pass, after every Output, so it adds nothing here.
fn outputs_in_render_order(graph: &Graph) -> Vec<NodeId> {
    graph
        .topological_order()
        .iter()
        .copied()
        .filter(|id| graph.get(*id).is_some_and(|n| n.def.is_output))
        .collect()
}

impl SynthLink {
    /// Every node the session leaves awake: worked out once per change to the graph's
    /// shape, the open tabs or the decks, and read by the plan, which pass measures what,
    /// and the tick's inputs alike.
    pub(in crate::app) fn live(&mut self, from: &Session<'_>) -> &HashSet<NodeId> {
        let key = from.live_key();
        if self.live_key.as_ref() != Some(&key) {
            let live = live_nodes(from.graph, from.open, from.mixer, &from.sent);
            if live != self.live {
                self.live = live;
                self.live_generation += 1;
            }
            self.live_key = Some(key);
        }
        &self.live
    }

    /// These Outputs' shaders must be rebuilt before the next plan.
    pub(in crate::app) fn mark_stale(&mut self, outputs: impl IntoIterator<Item = NodeId>) {
        self.needs_recompile.extend(outputs);
    }

    /// These nodes left the graph: nothing built for them survives.
    pub(in crate::app) fn forget(&mut self, removed: &[NodeId]) {
        for id in removed {
            self.outputs.remove(id);
            self.needs_recompile.remove(id);
        }
    }

    /// The graph was replaced whole, so every Output is rebuilt against it rather than
    /// working out which changed. Past the compiler it costs only what changed: a source the
    /// renderer holds is not sent, and one it held before is still linked there.
    pub(in crate::app) fn rebuild_all(&mut self, graph: &Graph) {
        for link in self.outputs.values_mut() {
            link.shader = None;
        }
        self.needs_recompile = graph
            .iter()
            .filter(|(_, n)| n.def.is_output)
            .map(|(id, _)| id)
            .collect();
    }

    /// True when this Output's shader will be rebuilt for the next plan.
    pub(in crate::app) fn needs_recompile(&self, node: NodeId) -> bool {
        self.needs_recompile.contains(&node)
    }

    /// Take the pending rebuilds, as a plan would.
    pub(in crate::app) fn take_recompiles(&mut self) -> Vec<NodeId> {
        self.needs_recompile.drain().collect()
    }

    /// The shader an Output is drawing with, where it has one.
    pub(in crate::app) fn shader(&self, output: NodeId) -> Option<&compile::Shader> {
        self.outputs.get(&output)?.shader.as_deref()
    }

    /// How many shaders are built, and how many uniforms they feed between them.
    pub(in crate::app) fn shader_counts(&self) -> (usize, usize) {
        let shaders = self.outputs.values().filter_map(|l| l.shader.as_deref());
        (
            shaders.clone().count(),
            shaders.map(|s| s.uniforms.len()).sum(),
        )
    }

    /// The plan last handed over.
    pub(in crate::app) fn plan(&self) -> &crate::synth::Plan {
        &self.plan
    }

    /// How many plans have been handed over.
    pub(in crate::app) fn plan_seq(&self) -> u64 {
        self.plan_seq
    }

    /// How many sources, shaders and probes both, have been handed to the renderer.
    pub(in crate::app) fn sources_sent(&self) -> u64 {
        self.sources_sent
    }

    /// Send the next fade whatever it says, saying a hand moved `target` — the fade,
    /// Blackout or Freeze: its MIDI barrier waits on a fade even where the move left the
    /// value unchanged, and the synth takes the hand's value over a write it made before.
    pub(in crate::app) fn touch(&mut self, target: crate::midi::Target) {
        match target {
            crate::midi::Target::Balance => self.hands.balance = true,
            crate::midi::Target::Blackout => self.hands.blackout = true,
            crate::midi::Target::Freeze => self.hands.freeze = true,
            crate::midi::Target::Port(_) => {}
        }
    }

    /// How many fades have been handed over, and so the `seq` of the last one.
    pub(in crate::app) fn fade_seq(&self) -> u64 {
        self.fade_seq
    }

    /// Hand the synth the fade, the crossfade method, Blackout and Freeze where any changed,
    /// or where a hand moved one at all. On their own rather than in the plan, because a fade
    /// moves them every frame and nothing a plan works out reads them.
    fn push_fade(&mut self, from: &Session<'_>) {
        let m = from.mixer;
        let fade = (m.balance, m.method, m.blackout, m.freeze);
        if self.hands == crate::mixer::Hands::default() && self.fade_sent == Some(fade) {
            return;
        }
        self.fade_seq += 1;
        self.synth.send(Msg::Fade {
            balance: fade.0,
            method: fade.1,
            blackout: fade.2,
            freeze: fade.3,
            hands: self.hands,
            seq: self.fade_seq,
        });
        self.fade_sent = Some(fade);
        self.hands = crate::mixer::Hands::default();
    }

    /// Hand the tick what it is given beside the plan — the live set, what the editor is
    /// reading, the assets, the Main Input and whether its panel shows the picture — where
    /// any of it changed. Each can change with
    /// no command behind it, and reaches the tick even while no plan is built.
    pub(in crate::app) fn push_inputs(
        &mut self,
        from: &Session<'_>,
        assets: AssetPaths,
        main_input: MainInput,
    ) {
        self.live(from);
        let key = InputsKey {
            live: self.live_generation,
            report: from.report,
            status: from.status,
            preview: from.preview,
            assets,
            main_input,
        };
        if self.inputs_sent.as_ref() == Some(&key) {
            return;
        }
        self.synth.send(Msg::Inputs {
            live: self.live.clone(),
            report: key.report,
            status: key.status,
            assets: key.assets.clone(),
            main_input: key.main_input.clone(),
            main_input_preview: key.preview,
        });
        self.inputs_sent = Some(key);
    }

    /// Outputs the renderer says it has no program for, and probes likewise, believed only
    /// against a snapshot that has drawn from the plan that carried their last source.
    /// `OutputPlan::shader` is one-shot, so without this a source the renderer could not take
    /// would leave that Output black with no error.
    fn awaiting(&self) -> (HashSet<NodeId>, HashSet<NodeId>) {
        let render = &self.snapshot.render;
        let drawn = render.plan_generation;
        let believed = |id: &NodeId, sent_on: fn(&OutputLink) -> Option<u64>| {
            self.outputs
                .get(id)
                .and_then(sent_on)
                .is_none_or(|sent| drawn >= sent)
        };
        let shaders = render
            .awaiting_shader
            .iter()
            .copied()
            .filter(|id| believed(id, |l| l.sent_on))
            .collect();
        let probes = render
            .probe_awaiting
            .iter()
            .copied()
            .filter(|id| believed(id, |l| l.probe_sent_on))
            .collect();
        (shaders, probes)
    }

    /// Whether the next plan would differ from the last one sent: its key moved, or it has
    /// work to carry — an awake Output to rebuild, or a source the renderer asked for again.
    fn plan_stale(&mut self, from: &Session<'_>, key: &PlanKey) -> bool {
        if self.built.as_ref() != Some(key) {
            return true;
        }
        if !self.needs_recompile.is_empty() {
            self.live(from);
            if self.needs_recompile.iter().any(|id| self.live.contains(id)) {
                return true;
            }
        }
        let (shaders, probes) = self.awaiting();
        let has =
            |id: &NodeId, part: fn(&OutputLink) -> bool| self.outputs.get(id).is_some_and(part);
        shaders.iter().any(|id| has(id, |l| l.shader.is_some()))
            || probes.iter().any(|id| has(id, |l| l.probe.is_some()))
            || self.passes_awaiting().iter().any(|key| {
                self.passes
                    .get(&key.workspace)
                    .is_some_and(|l| key.batch < l.batches.len())
            })
    }

    /// Passes the renderer says it has no program for, on the terms of [`Self::awaiting`].
    fn passes_awaiting(&self) -> HashSet<PassKey> {
        let render = &self.snapshot.render;
        render
            .pass_awaiting
            .iter()
            .copied()
            .filter(|key| {
                self.passes
                    .get(&key.workspace)
                    .and_then(|l| l.batches.get(key.batch))
                    .and_then(|b| b.sent_on)
                    .is_none_or(|sent| render.plan_generation >= sent)
            })
            .collect()
    }

    /// Every pass: each open workspace's, and each closed one's that measures something a
    /// deck or a send keeps awake, in as many batches as its textures need. Rebuilt where the
    /// graph's shape, what it measures or whether it has thumbnails moved; a batch is sent
    /// where the renderer does not hold it.
    ///
    /// An open workspace's pass carries its thumbnails whether or not it is looked at, so a
    /// tab looked at again has them on its first tick; it draws them only while it is. A
    /// batch draws its measurements' square on every tick one of them is read, and on every
    /// tick an Output reading one draws.
    fn plan_passes(
        &mut self,
        from: &Session<'_>,
        measured: &BTreeMap<WorkspaceId, Vec<NodeId>>,
        drawn: &Drawn,
    ) -> Vec<crate::synth::PassPlan> {
        let awaiting = self.passes_awaiting();
        let seq = self.plan_seq;
        let workspaces: BTreeSet<WorkspaceId> =
            from.open.iter().chain(measured.keys()).copied().collect();
        self.passes.retain(|w, _| workspaces.contains(w));
        let mut passes = Vec::new();
        for workspace in &workspaces {
            let measures = measured.get(workspace).map_or(&[][..], Vec::as_slice);
            let thumbs = from.open.contains(workspace);
            let link = self.passes.entry(*workspace).or_default();
            let rebuilt = link.shape != Some(from.shape)
                || link.measured != measures
                || link.thumbs != thumbs;
            if rebuilt {
                link.shape = Some(from.shape);
                link.measured = measures.to_vec();
                link.thumbs = thumbs;
                let shaders = compile::wgsl::build_pass(from.graph, *workspace, measures, thumbs);
                // A batch keeps what the renderer holds under its key, so one that came out
                // the same is not sent again.
                let old = std::mem::take(&mut link.batches);
                link.batches = shaders
                    .into_iter()
                    .enumerate()
                    .map(|(i, shader)| BatchLink {
                        built: compile::source_hash(&shader.body),
                        shader: Arc::new(shader),
                        held: old.get(i).and_then(|b| b.held),
                        sent_on: old.get(i).and_then(|b| b.sent_on),
                    })
                    .collect();
            }
            for (batch, link) in link.batches.iter_mut().enumerate() {
                let key = PassKey {
                    workspace: *workspace,
                    batch,
                };
                let send = (rebuilt && link.held != Some(link.built)) || awaiting.contains(&key);
                if send {
                    link.sent_on = Some(seq);
                    link.held = Some(link.built);
                    self.sources_sent += 1;
                }
                let shader = &link.shader;
                passes.push(crate::synth::PassPlan {
                    key,
                    shader: send.then(|| Arc::clone(shader)),
                    source: link.built,
                    resolution: shader.pass_size(),
                    region: shader.tap_region(),
                    uniforms: providers(shader),
                    taps: shader.slots(),
                    tap_nodes: shader.taps.iter().map(|(node, _)| *node).collect(),
                    thumb_base: shader.thumb_base(),
                    thumbs: shader.thumbs.clone(),
                    shown: from.active == Some(*workspace) && !shader.thumbs.is_empty(),
                    measures: shader
                        .taps
                        .iter()
                        .any(|(node, _)| drawn.taps.contains(node)),
                    wanted_by: drawn
                        .wants
                        .iter()
                        .filter(|(_, read)| shader.taps.iter().any(|(node, _)| read.contains(node)))
                        .map(|(output, _)| *output)
                        .collect(),
                });
            }
        }
        passes
    }

    /// Rebuild whatever is stale and describe the frame for the synth.
    ///
    /// An Output shown only on closed workspaces stays in the plan as `Mode::Suspended`, with
    /// no shader sent, so the renderer keeps its targets: reopening must not reallocate them,
    /// and a reallocation is a black frame.
    fn build_plan(&mut self, from: &Session<'_>) -> crate::synth::Plan {
        let graph = from.graph;
        let order = outputs_in_render_order(graph);
        // The one place an Output's link is let go: the renderer drops one the plan no longer
        // carries, programs and all, so one that comes back is news.
        self.outputs.retain(|id, _| order.contains(id));
        let mut outputs = Vec::with_capacity(order.len());
        let live = self.live(from).clone();
        let (awaiting, probe_awaiting) = self.awaiting();
        let mut probes = Vec::new();
        // Every Output that can draw — awake, with a shader — in render order, and what each
        // one's shader reads.
        let mut eligible = Vec::new();
        let mut reads: HashMap<NodeId, Reads> = HashMap::new();

        for id in &order {
            let awake = live.contains(id);
            let seq = self.plan_seq;
            let link = self.outputs.entry(*id).or_default();
            // Left in `needs_recompile` while it sleeps, so an edit made to a closed
            // workspace is built on the frame its tab comes back.
            let recompiled = awake && self.needs_recompile.remove(id);
            if recompiled {
                link.shader = compile::wgsl::build(graph, *id).map(Arc::new);
                link.built = link.shader.as_ref().map(|s| compile::source_hash(&s.body));
            }

            let resolution = graph
                .get(*id)
                .map_or(crate::nodes::output::DEFAULT_RESOLUTION, |n| {
                    crate::nodes::output::resolution_of(n)
                });

            let source = link.built;
            let has_shader = link.shader.is_some();
            // A rebuild that came out as what the renderer already holds is not sent; one the
            // renderer lost is, whatever it holds.
            let changed = recompiled && source.is_some() && link.held != source;
            let send_shader = awake && has_shader && (changed || awaiting.contains(id));
            if send_shader {
                link.sent_on = Some(seq);
                if source.is_some() {
                    link.held = source;
                }
                self.sources_sent += 1;
            }
            if awake && !has_shader {
                // Dark: the renderer puts its program aside, so the next source is news.
                link.held = None;
            }

            // The probe follows the shader's source, for an awake Output with something to
            // draw, whether or not View ▸ Costs is on: the header warning reads it too.
            if awake && let Some(source) = source.filter(|_| has_shader) {
                let rebuilt = link.probed != Some(source) || link.probe.is_none();
                if rebuilt {
                    link.probed = Some(source);
                    link.probe = compile::wgsl::build_probe(graph, *id).map(Arc::new);
                }
                if let Some(probe) = &link.probe {
                    let probe_source = compile::source_hash(&probe.body);
                    let fresh = rebuilt && link.probe_held != Some(probe_source);
                    let send_probe = fresh || probe_awaiting.contains(id);
                    if send_probe {
                        link.probe_sent_on = Some(seq);
                        link.probe_held = Some(probe_source);
                        self.sources_sent += 1;
                    }
                    probes.push(crate::synth::ProbePlan {
                        output: *id,
                        shader: send_probe.then(|| Arc::clone(probe)),
                        source: probe_source,
                        uniforms: providers(probe),
                        taps: probe.taps.len(),
                    });
                }
            } else {
                // Out of the plan, so the renderer frees the probe: nothing it held is kept.
                link.probe = None;
                link.evaluations.clear();
                link.calls.clear();
                link.probed = None;
                link.probe_held = None;
            }
            let shader = link.shader.as_ref();
            if awake && let Some(shader) = shader {
                eligible.push(*id);
                reads.insert(*id, Reads::of(graph, shader));
            }
            outputs.push(crate::synth::OutputPlan {
                node: *id,
                resolution,
                shader: send_shader.then(|| shader.map(Arc::clone)).flatten(),
                source: source.unwrap_or_default(),
                // Idle until the draw rule below says otherwise, once every Output's reads
                // are known.
                mode: if !awake {
                    Mode::Suspended
                } else if shader.is_none() {
                    Mode::Dark
                } else {
                    Mode::Idle
                },
                reads: Vec::new(),
                needs: Vec::new(),
                feeds_back: false,
                uniforms: shader.map(|s| providers(s)).unwrap_or_default(),
                taps: shader.map_or(0, |s| s.slots()),
            });
        }

        let measured = measured_on(graph, &live, from.open);
        let mut every: Vec<NodeId> = measured.values().flatten().copied().collect();
        every.sort_unstable();
        let drawn =
            Rule::new(graph, &reads, &eligible, &live, from.active, every).apply(graph, from);
        let passes = self.plan_passes(from, &measured, &drawn);
        for out in &mut outputs {
            if let Some(why) = drawn.draws.get(&out.node) {
                out.mode = Mode::Draw { why: *why };
            }
            if let Some(needs) = drawn.needs.get(&out.node) {
                out.needs.clone_from(needs);
            }
            out.feeds_back = drawn.cyclic.contains(&out.node);
            if let Some(r) = reads.get_mut(&out.node) {
                out.reads = std::mem::take(&mut r.frames);
            }
        }
        crate::synth::Plan {
            generation: self.plan_seq,
            outputs,
            probes,
            passes,
            // The preview shows the mix: what the audience sees, whatever is selected.
            display: Display::Mixer,
            mixer: from.mix(),
            // The sampling a texture output declared travels with the plan: `render/` may
            // not read the registry, so the app is what copies it across.
            sampling: graph
                .iter()
                .flat_map(|(id, node)| {
                    node.def.outputs.iter().map(move |o| {
                        (
                            PortRef::new(id, o.key),
                            crate::synth::Sampling {
                                wrap: o.wrap,
                                filter: o.filter,
                            },
                        )
                    })
                })
                .collect(),
        }
    }

    /// Build the plan, keep it for the Status box, and hand it over — where anything it is
    /// built from changed since the last one. See the module's own doc.
    pub(in crate::app) fn publish_plan(&mut self, from: &Session<'_>) {
        self.push_fade(from);
        let key = PlanKey::of(from);
        if !self.plan_stale(from, &key) {
            return;
        }
        self.plan_seq += 1;
        let plan = self.build_plan(from);
        self.plan = plan.clone();
        self.synth.send(Msg::Plan(Box::new(plan)));
        self.built = Some(key);
    }

    /// **Test and inline only.** The frame the plan describes, resolved by the synth on
    /// this thread.
    pub(in crate::app) fn build_frame_job(&mut self, from: &Session<'_>) -> FrameJob {
        self.publish_plan(from);
        let Host::Inline { synth, rx, .. } = &mut self.synth else {
            return FrameJob::default();
        };
        while let Ok(msg) = rx.try_recv() {
            synth.handle(msg);
        }
        synth.job()
    }

    /// **Test accessor.** One shader's uniforms as the synth resolves them.
    pub(in crate::app) fn resolve_uniforms(
        &mut self,
        from: &Session<'_>,
        shader: &compile::Shader,
    ) -> Vec<(std::sync::Arc<str>, crate::render::UniformValue)> {
        self.publish_plan(from);
        self.synth.pump();
        self.synth
            .inline_synth()
            .map(|s| s.resolve_shader(shader))
            .unwrap_or_default()
    }
}
