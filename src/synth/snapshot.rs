// SPDX-License-Identifier: AGPL-3.0-or-later

//! What one tick left behind, as the editor reads it.
//!
//! The synth runs on a thread of its own, so nothing the editor draws may be a borrow of a
//! running node: every figure on the canvas, in the Status box and on the panels is a field
//! here, filled once at the end of a tick and handed over by a `mem::swap` under a mutex.
//! See [`super::thread`] for the swap and `docs/architecture.md` for the two channels.
//!
//! **Owned, and cheap on purpose.** The maps are the ones a tick already builds; a picture
//! is an `Arc<Frame>` and a published texture is a handle, so the cost per tick is refcounts
//! and a few hundred small entries rather than pixels. The three that are gathered only when
//! something is looking at them — the per-node report, and the Status box's CPU lines and
//! tick phases — are gated by the `report` and `status` flags [`super::Msg::Inputs`] carries.
//!
//! **Facts, overwritten; events, counted.** Everything here but `events` is a fact about the
//! tick that wrote it, and a snapshot the editor never takes loses nothing by being written
//! over. What happens once rides in [`super::Events`] instead.

use crate::audio::Scope;
use crate::compile;
use crate::graph::{NodeId, PortRef};
use crate::nodes::cpu::{Curve, Puck, TraceRing};
use crate::nodes::{Event, Frame, NodeNote};
use crate::render::Published;
use crate::render::{DropCauses, GpuTime};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The clock, as everything that reads a clock reads it. The `Clock` itself stays on the
/// synth thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockReport {
    pub ticks: u64,
    pub dt: f32,
    pub elapsed: f64,
}

impl ClockReport {
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Seconds since the previous tick.
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Seconds since the first tick. True elapsed time, never clamped.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }
}

impl Default for ClockReport {
    fn default() -> Self {
        Self {
            ticks: 0,
            dt: 0.0,
            elapsed: 0.0,
        }
    }
}

/// What the Main Input is doing, for the panel that is not on this thread.
#[derive(Debug, Default, Clone)]
pub struct MainInputReport {
    pub video_status: String,
    pub audio_status: String,
    pub error: Option<String>,
    pub devices: Vec<crate::audio::device::Listed>,
    pub cameras: Vec<(String, String)>,
    pub analysis: crate::audio::Analysis,
    pub sample_rate: f32,
    pub hears: bool,
    pub has_picture: bool,
    pub position: f64,
}

/// What the renderer reported, for the Status box, the cost strips and the job the editor
/// builds next.
///
/// One frame stale by construction, which is what it was when the editor asked the renderer
/// directly: a shader the renderer could not take is re-sent on the frame after, and a GPU
/// time is a figure about the frame before.
#[derive(Debug, Default, Clone)]
pub struct RenderReport {
    /// A run with no GPU at all: no device, no renderer, nothing drawn.
    pub has_gpu: bool,
    /// The plan the renderer has actually drawn from. The editor sends a shader once and
    /// re-sends only where `awaiting_shader` says it was not taken — which is a fact about a
    /// plan, so the editor must not read it against a snapshot from before the plan it just
    /// sent, or it would submit the same recompile on every frame until the synth caught up.
    pub plan_generation: u64,
    /// How many Outputs the last tick drew: the plan's, and those drawn on demand.
    pub drawn: usize,
    pub errors: HashMap<NodeId, String>,
    pub gpu_times: HashMap<NodeId, GpuTime>,
    /// Frames each Output did not draw on a tick it was drawn on, since the run began: every
    /// target was held, or a render's wait for its frame ran out.
    pub dropped_frames: HashMap<NodeId, u64>,
    /// Every drop in the project by cause, the mix's among the `held`.
    pub drops: DropCauses,
    /// Draws that found the GPU a whole queue of ticks behind and waited for the oldest,
    /// since the run began: the ticks the GPU slowed. See [`crate::render::TICKS_IN_FLIGHT`].
    pub gpu_waits: u64,
    pub linking: HashSet<NodeId>,
    /// Outputs whose shader the renderer has not taken, so the editor sends it again.
    pub awaiting_shader: HashSet<NodeId>,
    pub probe_awaiting: HashSet<NodeId>,
    /// Workspace passes the renderer has no program for, likewise.
    pub pass_awaiting: HashSet<crate::render::PassKey>,
}

/// Where the offline render is, while one is running. See [`crate::app::render`].
#[derive(Debug, Default, Clone)]
pub struct RenderState {
    pub running: Option<RenderProgress>,
    /// How the last one ended, until the next one starts, and which render it was.
    pub outcome: Option<(u64, crate::app::render::Outcome)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderProgress {
    pub output: NodeId,
    pub written: u32,
    pub frames: u32,
    pub destination: std::path::PathBuf,
}

/// One varying output evaluated on the compiler's thumbnail grid, bottom row first.
#[derive(Debug)]
pub struct PortThumb {
    /// A number's cells are `f32` bits; a color's are `pack4x8unorm`.
    pub number: bool,
    pub words: Vec<u32>,
}

/// Everything one tick published.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// Which graph this tick ran over. A counter rather than a pointer: the synth owns its
    /// copy outright, so there is no `Arc` for the editor to hold against its own.
    pub generation: u64,
    /// Which project that graph is, as [`super::Msg::Graph`] counts them.
    pub project: u64,
    /// Which publish this is, counting from the synth's first. What the mailbox orders by:
    /// the editor keeps the newer of what it holds and what the slot holds, so a frame that
    /// repaints between two ticks reads the same snapshot twice rather than the one it
    /// already read back to the synth. `clock.ticks` cannot serve, because an offline render
    /// publishes without ticking the clock.
    pub seq: u64,
    pub clock: ClockReport,
    /// The transport: the playhead, and whether it plays.
    pub transport: crate::transport::Report,
    /// A node's own line on its body — a transcode's progress, a tap saying which Output
    /// measured it. Gathered only while something is drawing them.
    pub notes: HashMap<NodeId, NodeNote>,
    /// Every CPU node whose own `CpuNode::error` says something, and what: the flag on its
    /// header and a row of the problems list. Gathered with the notes.
    pub faults: HashMap<NodeId, String>,
    pub scopes: HashMap<NodeId, Scope>,
    /// The cells under a node's trace: `CpuNode::caption`, gathered while something draws them.
    pub captions: HashMap<NodeId, Vec<(&'static str, String)>>,
    pub playheads: HashMap<NodeId, f32>,
    pub traces: HashMap<NodeId, TraceRing>,
    /// What a node's transport is doing with a curve this tick — `automation`'s recording as
    /// it is performed. Gathered while something draws it, as the traces are.
    pub curves: HashMap<NodeId, Curve>,
    /// Where a pad's puck is and what pulls on it this tick — `xypad`'s. Gathered while
    /// something draws it, as the curves are.
    /// What each gear is doing, for the region that draws it. Gathered while something draws
    /// the canvas.
    pub gears: HashMap<NodeId, crate::nodes::gear::Reading>,
    pub pucks: HashMap<NodeId, Puck>,
    /// Every uniform number published, in `f64` and never wrapped, and where each node that
    /// moves with time is under its Time key.
    pub uniforms: HashMap<PortRef, f64>,
    pub uniform_colors: HashMap<PortRef, [f32; 4]>,
    pub frames: HashMap<PortRef, Arc<Frame>>,
    pub actions: HashMap<PortRef, Vec<Event>>,
    pub held: HashSet<PortRef>,
    pub readbacks: HashMap<NodeId, [u32; compile::TAP_WORDS]>,
    /// Each varying output's thumbnail, from the last frame the GPU finished.
    pub thumbs: HashMap<PortRef, Arc<PortThumb>>,
    /// Where the last whole tick's milliseconds went, on the CPU and the GPU, measured only
    /// while the Status box is open. See [`super::meter`].
    pub phases: Option<super::Phases>,
    /// Every CPU node's line for the Status box, gathered only while it is open.
    pub cpu_lines: Vec<(NodeId, String, String)>,
    /// Everything one CPU node reports, for `App::cpu_report`, gathered with the lines.
    pub cpu_reports: HashMap<NodeId, String>,
    /// Every one-shot event the editor has not acknowledged, one ordered log per kind, each
    /// beside a count of every one there has been. See [`super::events`].
    pub events: super::Events,
    /// The count at which the editor's learning began, or `u64::MAX` while nothing is
    /// learning: the binding is made from the first message read after the asking, never from
    /// one this tick had already driven a control with.
    pub midi_learn_from: u64,
    /// Every control a bound knob has moved that the editor has not answered for, newest
    /// value per control, in node order.
    ///
    /// **Not a window and not a count**: one entry per control, republished whole every tick
    /// until the editor's own graph shows it has landed. A long sweep of one knob can no more
    /// push another knob's last value off this than a map can lose a key, which is exactly
    /// what a window of writes would have done — and the synth would have gone on restoring
    /// the lost one on every graph, so the world and the file would have disagreed for good.
    /// The editor skips an entry its own graph already agrees with.
    pub midi_writes: Vec<(PortRef, f32)>,
    /// Where a bound fader has put the Main Mixer's balance, while the editor's own mixer
    /// has not caught up. Republished every tick until a fade the editor sends says it has,
    /// exactly as a control write is; `None` once it lands or when no fader has moved.
    pub midi_balance: Option<f32>,
    /// Where a bound note or CC has switched Blackout and Freeze, while the editor's mixer has
    /// not caught up: the fade's rule, for the mixer's two presses.
    pub midi_blackout: Option<bool>,
    pub midi_freeze: Option<bool>,
    /// Every bound control whose fader is out of soft takeover's pick-up, and where that
    /// fader is in the control's own units: the ghost mark the control wears.
    pub midi_ghosts: Vec<(crate::midi::Target, f32)>,
    /// The `seq` of the last [`super::Msg::Fade`] this tick drew with, so a hand's move of the
    /// fade bars a fader's write until the synth has seen it.
    pub fade_seq: u64,
    pub main_input: MainInputReport,
    pub render: RenderReport,
    pub offline: RenderState,
    /// Every live recording running, one per Output. See [`super::record`].
    pub recordings: Vec<super::RecordProgress>,
    /// What the last frame drew, for a viewer to blit.
    pub published: Arc<Published>,
}

impl Snapshot {
    /// Forget everything this says about the project's nodes, and every event it carries
    /// for them — a deck claim, a recording, a knob's write, a thumbnail, a Snap, a probe's
    /// counts. What is left is the run's: the clock, the Main Input, MIDI's log and the fade,
    /// the renderer's totals, and the mix. See [`super`] on another project.
    pub fn forget_project(&mut self) {
        self.notes.clear();
        self.faults.clear();
        self.scopes.clear();
        self.captions.clear();
        self.playheads.clear();
        self.traces.clear();
        self.curves.clear();
        self.pucks.clear();
        self.gears.clear();
        self.uniforms.clear();
        self.uniform_colors.clear();
        self.frames.clear();
        self.actions.clear();
        self.held.clear();
        self.readbacks.clear();
        self.thumbs.clear();
        self.cpu_lines.clear();
        self.cpu_reports.clear();
        self.events.forget_project();
        self.midi_writes.clear();
        self.midi_ghosts.clear();
        let render = &mut self.render;
        render.errors.clear();
        render.gpu_times.clear();
        render.dropped_frames.clear();
        render.linking.clear();
        render.awaiting_shader.clear();
        render.probe_awaiting.clear();
        render.pass_awaiting.clear();
        self.published = Arc::new(Published {
            tick: self.published.tick,
            mixer: self.published.mixer.clone(),
            holds: self.published.holds.clone(),
            ..Published::default()
        });
    }

    /// What a uniform number output published, if anything did.
    pub fn uniform(&self, port: PortRef) -> Option<f64> {
        self.uniforms.get(&port).copied()
    }

    pub fn uniform_color(&self, port: PortRef) -> Option<[f32; 4]> {
        self.uniform_colors.get(&port).copied()
    }

    pub fn frame(&self, port: PortRef) -> Option<&Arc<Frame>> {
        self.frames.get(&port)
    }

    /// Every event an action output fired on this tick, in the order it fired them.
    pub fn edges(&self, port: PortRef) -> &[Event] {
        self.actions.get(&port).map_or(&[], Vec::as_slice)
    }

    pub fn trace(&self, node: NodeId) -> Option<&TraceRing> {
        self.traces.get(&node)
    }

    /// What one node's transport is doing with its curve, if it has one.
    pub fn curve(&self, node: NodeId) -> Option<&Curve> {
        self.curves.get(&node)
    }

    /// Where one pad's puck is, if the node has a pad.
    pub fn puck(&self, node: NodeId) -> Option<&Puck> {
        self.pucks.get(&node)
    }

    pub fn is_held(&self, port: PortRef) -> bool {
        self.held.contains(&port)
    }

    pub fn readback(&self, node: NodeId) -> Option<&[u32; compile::TAP_WORDS]> {
        self.readbacks.get(&node)
    }
}

/// A view of the two uniform maps, for the canvas.
///
/// `Copy` and two words wide, so threading it to every node costs no copy of either map. It
/// is the whole of what the canvas knows about live values: `ui/` reads it and nothing else,
/// and it is declared in `synth/` so reading it costs no dependency on the tree that draws
/// it.
#[derive(Clone, Copy)]
pub struct Uniforms<'a> {
    numbers: &'a HashMap<PortRef, f64>,
    colors: &'a HashMap<PortRef, [f32; 4]>,
}

// One caller inside this crate; a hasher parameter here would spread across every function
// the view is handed to.
#[allow(clippy::implicit_hasher)]
impl<'a> Uniforms<'a> {
    pub fn new(numbers: &'a HashMap<PortRef, f64>, colors: &'a HashMap<PortRef, [f32; 4]>) -> Self {
        Self { numbers, colors }
    }

    /// Everything one snapshot published, which is what the canvas is handed.
    pub fn of(snapshot: &'a Snapshot) -> Self {
        Self::new(&snapshot.uniforms, &snapshot.uniform_colors)
    }

    /// What a port published this tick, exactly: what its row prints, so a gear's Cycles
    /// climb for as long as the show runs. `None` where it has published nothing, which is
    /// what a port drawn without a number means.
    pub fn get(self, port: PortRef) -> Option<f64> {
        self.numbers.get(&port).copied()
    }

    /// What the Time at `at` reads, with `source` the output cabled into it: what that output
    /// published, as `UniformProvider::NodeCount` and `TickContext::cycle` resolve it; and with
    /// nothing cabled in, the reading published under the Time's own key. `None` where nothing
    /// has been published.
    pub fn time(self, at: PortRef, source: Option<PortRef>) -> Option<f64> {
        self.get(source.unwrap_or(at))
    }

    /// What a uniform color port published, on the same terms as [`Self::get`].
    pub fn color(self, port: PortRef) -> Option<[f32; 4]> {
        self.colors.get(&port).copied()
    }

    /// What arrives at an input whose source is `source`.
    ///
    /// The resolution `TickContext::input` performs, without its fall back to the input's
    /// own control: the caller draws that where this is `None`.
    pub fn arriving(self, source: Option<PortRef>) -> Option<f64> {
        self.get(source?)
    }

    /// What color arrives at an input whose source is `source`. [`Self::arriving`] for the
    /// other kind, and what makes a swatch on a connected color input a meter of the color
    /// being fed to it rather than of the one it last held.
    pub fn arriving_color(self, source: Option<PortRef>) -> Option<[f32; 4]> {
        self.color(source?)
    }
}
