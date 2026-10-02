// SPDX-License-Identifier: AGPL-3.0-or-later

//! The editor's end of the synth: what crosses to it, and what comes back.
//!
//! [`SynthLink`] owns the [`Host`], the snapshot the last tick left, the plan, and what it
//! keeps for each Output. Everything the editor sends goes through it, and everything it knows
//! about running state is the snapshot it holds. It is handed the graph and the session rather
//! than reaching for them: the document is `Document`'s, the tabs and mixer the project's.
//!
//! **Another project is counted, not diffed.** Open and New raise a count that crosses with
//! every graph, and undo never raises it, because only another project's ids name other
//! nodes; the synth drops what it held for the old one's, and this side forgets what it kept
//! per Output and what a snapshot of the old project says. See `synth` on another project.
//!
//! - [`plan`] — the plan, the shaders and workspace passes it carries, which pass measures
//!   what, and when each is sent.
//! - [`measure`] — what the probes counted, and the dropped frames.

pub(super) mod measure;
mod plan;

pub(super) use plan::Session;

use crate::compile;
use crate::graph::{Graph, NodeId, PortRef, WorkspaceId};
use crate::render::Published;
use crate::synth::{Host, Msg, Snapshot};
use measure::Rate;
use plan::{InputsKey, LiveKey, PlanKey};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(super) struct SynthLink {
    /// A thread of its own in the app, inline on the caller's thread headless. The editor
    /// sends a [`crate::synth::Msg`] and reads `snapshot`; neither waits on the other.
    synth: Host,
    /// What the synth's last tick published, taken at the top of every frame.
    snapshot: Box<Snapshot>,
    /// The finished textures each window blits, held until the next frame's snapshot
    /// replaces them, after this frame's swap has submitted every blit made from them.
    published: Arc<Published>,
    /// Buttons pressed and not yet confirmed by a snapshot, which may be several frames away:
    /// without this a pressed button would read *released* until then, and flicker.
    pressing: HashSet<PortRef>,
    /// How many graphs have been handed across.
    published_generation: u64,
    /// How many projects this run has opened: raised by Open and New and by nothing else,
    /// and handed across with every graph. See [`SynthLink::replace_project`].
    project: u64,
    /// The plan last handed over, for the Status box's list of Outputs and the preview.
    plan: crate::synth::Plan,
    /// How many plans have been handed over, and so the generation of the last one. See
    /// [`crate::synth::RenderReport::plan_generation`].
    plan_seq: u64,
    /// Pruned to the Outputs the plan carries.
    outputs: HashMap<NodeId, OutputLink>,
    /// Outputs whose shader must be rebuilt before the next plan.
    needs_recompile: HashSet<NodeId>,
    /// Pruned to the open workspaces.
    passes: HashMap<WorkspaceId, PassLink>,
    /// What the last plan was built from.
    built: Option<PlanKey>,
    /// The fade and the crossfade method last handed over, how many have been, and whether
    /// the next must be sent whatever it says. See `SynthLink::push_fade`.
    fade_sent: Option<(f32, crate::mixer::Method, bool, bool)>,
    fade_seq: u64,
    hands: crate::mixer::Hands,
    /// Every node the session leaves awake, what it was worked out from, and a count of the
    /// times it came out different.
    live: HashSet<NodeId>,
    live_key: Option<LiveKey>,
    live_generation: u64,
    /// What the tick was last given beside the plan.
    inputs_sent: Option<InputsKey>,
    /// How many sources, shaders and probes both, have gone to the renderer.
    sources_sent: u64,
    /// Ticks that waited for the GPU, as a rate, for the Status box.
    waits: Rate,
}

/// Everything the link keeps for one Output: its shader and probe, what the renderer holds of
/// each, and what came back.
///
/// A source is kept as a hash of its fragment body, and one the renderer holds is not sent
/// again. The plan a source went out on is kept because the renderer's "send it again" is
/// believed only from a snapshot that has drawn from that plan or a later one; against an
/// older one, the same source would go out every frame until the synth ticks.
#[derive(Debug, Default)]
struct OutputLink {
    /// The shader as last compiled, so a frame that does not recompile still knows which
    /// uniforms to feed. A graph replaced whole drops it and keeps `built`, so a rebuild can
    /// tell it rebuilt the same thing.
    shader: Option<std::sync::Arc<compile::Shader>>,
    built: Option<u64>,
    /// The shader the renderer holds, and the plan it last went out on.
    held: Option<u64>,
    sent_on: Option<u64>,
    /// The cost probe: the shader with every node counting, and the source it was built from.
    probe: Option<std::sync::Arc<compile::Shader>>,
    probed: Option<u64>,
    /// The probe the renderer holds, and the plan it last went out on.
    probe_held: Option<u64>,
    probe_sent_on: Option<u64>,
    /// What the probe last counted: each node's evaluations per frame, and each (node,
    /// input) pair's calls. Kept while a probe drops a frame.
    evaluations: HashMap<NodeId, f64>,
    calls: HashMap<(NodeId, &'static str), f64>,
    /// Dropped frames as a rate, for the cost strip.
    drops: Rate,
}

/// Everything the link keeps for one workspace's pass: its module, what it was built from,
/// and what the renderer holds of it, on the terms an [`OutputLink`] keeps its shader.
///
/// **Rebuilt on every change to the graph's shape**, not on a list of what an edit made
/// stale: a pass holds every node on its workspace and every chain above them, which may
/// run through any other, and building it is string work. What reaches the renderer is only
/// a source it does not hold.
#[derive(Debug, Default)]
struct PassLink {
    /// The graph's shape, the measured nodes and whether it has thumbnails, as it was built.
    shape: Option<u64>,
    measured: Vec<NodeId>,
    thumbs: bool,
    /// Its batches, in order: one, unless the workspace binds more textures than one pass
    /// may. See `compile::PASS_TEXTURES`.
    batches: Vec<BatchLink>,
}

/// One batch of a workspace's pass: its module and its source, and the source the renderer
/// holds under its key and the plan that last carried one.
#[derive(Debug)]
struct BatchLink {
    shader: Arc<compile::Shader>,
    built: u64,
    held: Option<u64>,
    sent_on: Option<u64>,
}

impl SynthLink {
    pub(super) fn new(synth: Host) -> Self {
        Self {
            synth,
            snapshot: Box::default(),
            published: Arc::default(),
            pressing: HashSet::new(),
            published_generation: 0,
            project: 0,
            plan: crate::synth::Plan::default(),
            plan_seq: 0,
            outputs: HashMap::new(),
            needs_recompile: HashSet::new(),
            passes: HashMap::new(),
            built: None,
            fade_sent: None,
            fade_seq: 0,
            hands: crate::mixer::Hands::default(),
            live: HashSet::new(),
            live_key: None,
            live_generation: 0,
            inputs_sent: None,
            sources_sent: 0,
            waits: Rate::default(),
        }
    }

    /// A hand on the transport, sent as it is.
    pub(super) fn transport(&mut self, command: crate::transport::Command) {
        self.send(Msg::Transport(command));
    }

    /// The transport as the last tick left it: the playhead and whether it plays.
    pub(super) fn transport_report(&self) -> crate::transport::Report {
        self.synth.inline_synth().map_or(
            self.snapshot.transport,
            crate::synth::Synth::transport_report,
        )
    }

    /// The host itself, for what is asked of the thread rather than sent to it: its rate,
    /// its pointer feed, stopping it, and the synth where it is inline.
    pub(super) fn host(&self) -> &Host {
        &self.synth
    }

    pub(super) fn host_mut(&mut self) -> &mut Host {
        &mut self.synth
    }

    pub(super) fn send(&self, msg: Msg) {
        self.synth.send(msg);
    }

    /// What the synth's last tick left.
    pub(super) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The snapshot, for what the editor takes out of it: the MIDI writes it answers for, and
    /// the maps the canvas borrows while it draws.
    pub(super) fn snapshot_mut(&mut self) -> &mut Box<Snapshot> {
        &mut self.snapshot
    }

    /// What the windows blit this frame.
    pub(super) fn published(&self) -> &Arc<Published> {
        &self.published
    }

    /// Hold on to what the snapshot just taken published, for every viewer this frame.
    pub(super) fn latch_published(&mut self) {
        self.published = Arc::clone(&self.snapshot.published);
    }

    /// Hold or release the button on an action input, and remember the press until a
    /// snapshot agrees.
    pub(super) fn press(&mut self, port: PortRef, down: bool) {
        self.synth.send(Msg::Press(port, down));
        if down {
            self.pressing.insert(port);
        } else {
            self.pressing.remove(&port);
        }
    }

    /// See [`Snapshot::is_held`], plus a press this frame that no tick has answered yet.
    pub(super) fn is_held(&self, port: PortRef) -> bool {
        self.snapshot.is_held(port) || self.pressing.contains(&port)
    }

    /// How many graphs have been handed to the synth.
    pub(super) fn graph_generation(&self) -> u64 {
        self.published_generation
    }

    /// Hand the synth the graph to tick over: **the one place it crosses**, an `Arc` bump
    /// once per command a tick could observe. The editor copies before it writes.
    pub(super) fn publish_graph(&mut self, graph: &Arc<Graph>) {
        self.published_generation += 1;
        self.synth.send(Msg::Graph {
            graph: Arc::clone(graph),
            project: self.project,
        });
    }

    /// Another project replaces this one, and the next graph handed across is its: the count
    /// is raised, and what this side keeps per Output, the presses and the snapshot's word
    /// on the old project's nodes are forgotten, so every source is sent again.
    pub(super) fn replace_project(&mut self) {
        self.project += 1;
        self.outputs.clear();
        self.needs_recompile.clear();
        self.passes.clear();
        self.built = None;
        self.inputs_sent = None;
        self.pressing.clear();
        self.snapshot.forget_project();
        self.published = Arc::clone(&self.snapshot.published);
    }

    /// Take the newest snapshot, and forget the presses it now agrees with. A snapshot of a
    /// project that has been replaced is read for nothing about a node.
    pub(super) fn take(&mut self) {
        let spent = std::mem::take(&mut self.snapshot);
        self.snapshot = self.synth.take(spent);
        if self.snapshot.project != self.project {
            self.snapshot.forget_project();
        }
        self.pressing
            .retain(|port| !self.snapshot.held.contains(port));
    }

    /// The clock, as it stands: read straight off it where the synth is on this thread, since
    /// it has moved since the last tick, and as of the last tick where it is not, since
    /// reading it there would wait on the synth thread.
    pub(super) fn clock(&self) -> crate::synth::ClockReport {
        self.synth
            .inline_synth()
            .map_or(self.snapshot.clock, crate::synth::Synth::report_clock)
    }

    /// Everything one CPU node reports, asked of the synth where it is on this thread and
    /// read off the snapshot where it is not.
    pub(super) fn cpu_report(&self, node: NodeId) -> String {
        match self.synth.inline_synth() {
            Some(synth) => synth.cpu_report(node),
            None => self
                .snapshot
                .cpu_reports
                .get(&node)
                .cloned()
                .unwrap_or_else(|| "no cpu state".to_string()),
        }
    }
}
