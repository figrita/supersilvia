// SPDX-License-Identifier: AGPL-3.0-or-later

//! The running state, the one tick that advances it, and the thread it runs on.
//!
//! [`docs/cpu.md`](../../docs/cpu.md) draws the line this struct is cut along: what a saved
//! file holds — the graph, `Node::controls`, `Node::options` — against what a node
//! *computes*, which is a `Box<dyn CpuNode>` and everything it publishes. The editor keeps
//! the document and the synth keeps the running half, so a field is here exactly when a
//! tick writes it or a tick reads it as input.
//!
//! **The synth keeps its own time.** [`thread`] runs [`Synth::step`] once per display
//! interval from a deadline of its own, on a thread of its own, drawing on the one device the
//! editor paints through. Nothing on the frame thread paces it and nothing on the frame
//! thread blocks it: a minimized editor stops its own presents and the world goes on. `proposals/deterministic-loop.md` is the argument and the measurements.
//!
//! **What crosses, and in which direction.** In, a [`Msg`] on a channel drained at the top
//! of every tick — the graph as a shared `Arc`, the [`Plan`] the compiler built, a press, a
//! seek, the Main Input's choice, the rate, the MIDI map. Out, one [`Snapshot`] per tick,
//! swapped under a mutex held for the length of a `mem::swap`. Neither side waits for the
//! other. What happens once — a deck claim, a value written, a picture read back — rides out
//! in one ordered log per kind, counted, so a snapshot the editor never took delays it and
//! reorders nothing: see [`events`].
//!
//! **The MIDI reader's queue is the synth's**, drained at the top of every tick beside the
//! messages: a knob bound to a control writes that control on this thread's graph and
//! a note bound to an action input is the same press a button sends, so a controller plays
//! the world whether or not the editor is painting. What each tick applied rides out on the
//! snapshot and the editor puts it back through its own bus, which is how the document and
//! the undo history catch up — the shape a deck claim uses, for the same reason.
//!
//! **Every `CpuNode` is born here.** The trait is not `Send` — a cpal stream is not — and it
//! does not need to be: state is created on the first tick after a node appears and dropped
//! on the first tick after it is gone, so a node is created, ticked and dropped on this
//! thread and never crosses. The Main Input's capture and the offline render's writer are
//! the synth's for the same reason.
//!
//! **Another project is not an edit.** Node ids restart at 1 in every project, so anything
//! kept by id — a node's state, a world, a deck, a recording on its way to the document —
//! would land on whichever node of the new project holds that id. A [`Msg::Graph`] carries
//! a count of the projects opened, which Open and New raise and undo never does, and a new
//! one drops all of it here and in the renderer; the clock, the Main Input, MIDI and the fade
//! are the run's and stay. Undo keeps everything, because the graph it restores is the same
//! patch. See `docs/architecture.md#saving-and-opening`.
//!
//! **The renderer is here too, on the one device.** [`Synth::render`] draws the frame
//! the plan describes and publishes, for each Output and the mix, the newest frame the GPU
//! has finished; every window is a viewer that blits one without waiting. The editor still
//! compiles the shaders, because compilation reads the graph it owns; what crosses is the
//! plan in and the snapshot out.

pub mod events;
pub mod maininput;
pub mod meter;
pub mod offline;
pub mod plan;
pub mod snapshot;
pub mod thread;

pub use events::Events;
pub use meter::{Phases, Work};
pub use plan::{Mix, Mode, OutputPlan, PassPlan, Plan, ProbePlan, Sampling, Why};
pub use snapshot::{ClockReport, MainInputReport, PortThumb, RenderReport, Snapshot, Uniforms};
pub use thread::{Beat, Host, Msg};

use crate::clock::Clock;
use crate::compile::{self, UniformProvider, UniformType};
use crate::graph::{ControlValue, Graph, NodeId, PortRef};
use crate::maininput::MainInput;
use crate::midi::Target;
use crate::mixer::Channel;
use crate::nodes::{
    CpuNode, Event, Frame, MainInputFeed, NodeNote, Simulation, TickContext, alpha,
};
use crate::project::AssetPaths;
use crate::render::{
    FrameJob, Gpu, Live, OutputJob, OutputMode, PassJob, ProbeJob, Published, Renderer, SimJob,
    SourceJob, UniformValue,
};
use crate::transport::Transport;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Where each of a shader's uniforms gets its value from.
///
/// The providers themselves, not values: a plan carries these and the synth resolves them
/// where it draws, because a control's value and a published uniform number are both read
/// out of state it owns.
pub fn providers(shader: &compile::Shader) -> Vec<(Arc<str>, UniformProvider)> {
    shader
        .uniforms
        .iter()
        .map(|(name, provider)| (Arc::clone(name), provider.clone()))
        .collect()
}

/// Everything the graph computes, and nothing it is.
#[allow(clippy::struct_excessive_bools)]
pub struct Synth {
    /// The graph the tick runs over: the editor's, as it stood after the last edit.
    ///
    /// **Shared, and written only as a copy.** The editor and its undo ring hold the same
    /// `Arc`, and `Graph` is `Sync`, so reading it from here is reading theirs. A knob or a
    /// tick writing a control goes through `Arc::make_mut`: the first write after a
    /// graph arrives copies it — pointer bumps, and the one node written — and the rest
    /// write that copy in place, because nothing else holds it.
    graph: Arc<Graph>,
    /// How many graphs have been handed over: the same number over a frame is a frame that
    /// crossed nothing.
    generation: u64,
    /// Which project the graph is, as [`Msg::Graph`] counts them. See
    /// [`Synth::replace_project`].
    project: u64,
    /// How many snapshots have gone out. `Snapshot::seq`, so the mailbox can tell newer
    /// from older.
    seq: u64,
    /// The one clock.
    clock: Clock,
    /// The playhead over it, which every speed integrates. See [`crate::transport`].
    transport: Transport,
    /// Where each node was on the transport when it last ticked, so its next tick's advance
    /// is its own: the whole gap for a node that slept on a closed tab.
    seen: HashMap<NodeId, crate::transport::Seen>,
    /// Every free-running node's own playheads: its Speed integrated against its advance, the
    /// one place a node's time is kept (`nodes::timing::Pace`). Dropped when it loops.
    paces: HashMap<NodeId, crate::nodes::timing::Pace>,
    /// The same, for the Main Input's clip, which ticks every tick.
    main_input_seen: Option<crate::transport::Seen>,
    /// One tick per this many milliseconds, from the editor's display or the preference.
    interval_ms: f32,
    /// Each CPU node's state, with the slug it was created for. Ids restart at 1 in every
    /// graph, so a state is only reused while the node under that id is the same kind.
    cpu: HashMap<NodeId, (&'static str, Box<dyn CpuNode>)>,
    /// What every uniform number output published this frame.
    uniforms: HashMap<PortRef, f32>,
    /// Every count published whole this frame, in `f64` and unbounded — a gear's Cycles, the
    /// Time node's Seconds, an unplugged Time's ambient reading — which a Time reads at
    /// `f64`'s precision on the CPU and split into a whole part and a fraction in a shader.
    /// See [`TickContext::publish_count`].
    counts: HashMap<PortRef, f64>,
    /// What every uniform color output published this frame, beside `uniforms` rather than
    /// inside it: the two kinds are read by different callers — one becomes a `float`
    /// uniform and a number on a row, the other a `vec4` and a swatch — and a map per kind
    /// keeps both lookups a plain `get` with nothing to unwrap.
    uniform_colors: HashMap<PortRef, [f32; 4]>,
    /// What every texture-publishing CPU node published this frame.
    frames: HashMap<PortRef, Arc<Frame>>,
    /// Every simulation a node steps on the GPU: its shape as the last tick left it, and the
    /// passes no frame job has taken yet. See [`crate::nodes::sim`].
    sims: HashMap<PortRef, Simulation>,
    /// Every action edge fired this frame, by the output port that fired it. Cleared at the
    /// top of each tick: an event lives for one frame.
    actions: HashMap<PortRef, Vec<Event>>,
    /// The number outputs put somewhere this frame rather than moved there: a gear's Reset.
    /// Cleared at the top of each tick, as an event is. [`TickContext::jump`].
    jumps: HashSet<PortRef>,
    /// Where a hand dragged a picture's scrubber, waiting for the next tick to act on it.
    /// One-shot, unlike `held`: a seek is a place asked for once, so it is cleared as soon as
    /// the ticks have read it. Not a command and not an edit — where a clip is playing from
    /// is the node's own state, not the document's.
    seeks: HashMap<NodeId, f32>,
    /// What a hand did on a node's own surface, waiting for the next tick: one-shot, as a
    /// seek is, and for the same reason not an edit — a pad's wells are its runtime state.
    touches: HashMap<NodeId, Vec<crate::nodes::cpu::Touch>>,
    /// Action ports whose button is held. Not a command and not an edit: a press is a hand
    /// on an instrument, like a pan or a View toggle, and it never reaches the undo
    /// history.
    held: HashSet<PortRef>,
    /// What `held` was on the previous tick, so a button on an Output — which has no
    /// `tick` of its own to remember it — is read as a press on the frame it goes down.
    prev_held: HashSet<PortRef>,
    /// The pointer's half of `held`: the buttons the canvas drew a finger on this frame.
    /// Replaced wholesale every frame, which is why it is not `held` itself.
    held_pointer: HashSet<PortRef>,
    /// Every other source's half: a note from a controller, a test, an agent. Held until
    /// let go, and never touched by the canvas.
    held_apart: HashSet<PortRef>,
    /// Each measuring node's slot after the last frame the GPU finished, out of the one
    /// Output picked for it. Written where the frame was drawn, read by the next tick.
    readbacks: HashMap<NodeId, [u32; compile::TAP_WORDS]>,
    /// Each varying output's thumbnail, from the last pass that drew it.
    thumbs: HashMap<PortRef, Arc<PortThumb>>,
    /// What the Main Input panel has open: the camera, the capture, the screen cast. The
    /// *choice* is in the project; this is what it costs. See [`maininput::Live`].
    main_input: maininput::Live,
    /// The choice itself, as the panel last left it.
    main_input_choice: MainInput,
    /// Thresholds the Main Input crossed since the last frame, collected once and fired by
    /// every `maininput` node, because they are all reading the one signal.
    main_input_crossings: Vec<crate::audio::Crossing>,
    /// Where an asset reference points, as a value the editor sends rather than a borrow of
    /// the project.
    assets: AssetPaths,
    /// What the editor last described: the shaders, the uniform providers, the mix, and
    /// which nodes are awake. Kept and redrawn every tick, because the synth ticks when the
    /// editor is not painting.
    plan: Plan,
    /// Which Output is on each deck, **as the mix being rendered has it**.
    ///
    /// A `Show on A` is an action input read inside the tick, so a sequencer lane or a
    /// Master Gear's Trigger can cut decks with nobody watching the editor — and the editor is
    /// what owns the project's mixer. If the claim only took effect where the editor applied
    /// it, a minimized editor would hold every cut until it came back and then land them all
    /// at once. So the claim lands here, on the mix this thread renders, and the editor
    /// reconciles the project's mixer from `Events::decks` and republishes a plan that
    /// agrees — one tick out and one frame back.
    ///
    /// Only *which Output is on which deck* is here. The mix resolution stays the plan's and
    /// the fade and the crossfade method the editor's last [`Msg::Fade`]: no action input
    /// touches them, so nothing inside a tick can move them and there is nothing for the two
    /// sides to disagree about.
    decks: (Option<NodeId>, Option<NodeId>),
    /// The decks as the **last plan** stated them, so a plan built before the editor saw a
    /// claim does not undo it: the plan's decks are adopted only where they changed.
    planned_decks: (Option<NodeId>, Option<NodeId>),
    /// The MIDI reader's queues, drained at the top of every tick. More than one because
    /// there are two ways in — the device's reader thread, and the editor's own seam, which
    /// is what a test posts a message through on a box with nothing plugged in.
    midi: Vec<std::sync::mpsc::Receiver<crate::midi::Wire>>,
    /// The map, as the editor last published it. A copy, so reading it costs the tick
    /// nothing and the editor is free to edit its own.
    midi_map: crate::midi::Bindings,
    /// A control is waiting to be learned, so nothing is driven. The editor owns learning —
    /// it needs the pointer — and says when it starts and stops.
    midi_learning: bool,
    /// Every control a bound knob has written that the editor has not answered for: the
    /// newest value per control, republished whole every tick and retired where the editor's
    /// own graph shows it has landed. See [`MidiWrite`].
    ///
    /// **One entry per control, not a list of writes.** A window of writes would be a window:
    /// a long sweep of one knob would push another knob's last value out of it and that
    /// control would never reach the document, while `published` here went on restoring it on
    /// every graph — the world and the file disagreeing for the rest of the session. A map
    /// keyed by the control cannot lose one, however long the editor is away.
    midi_owned: HashMap<PortRef, MidiWrite>,
    /// The fade, where a bound knob has moved it and the editor has not answered for it.
    /// The same shape as an entry in `midi_owned`, for the same reason: what the arriving
    /// plan says is measured against what the last one said, and unchanged there means the
    /// editor has not seen the write yet, so the knob's value stands.
    midi_balance: Option<RigWrite<f32>>,
    /// Blackout and Freeze, where a bound note or CC has switched them and the editor has not
    /// answered for it: the fade's rule, for the mixer's two presses.
    midi_blackout: Option<RigWrite<bool>>,
    midi_freeze: Option<RigWrite<bool>>,
    /// The fade and the crossfade method the mix is drawn with: the editor's last
    /// [`Msg::Fade`], or where a bound fader has since put the fade.
    balance: f32,
    method: crate::mixer::Method,
    /// Blackout and Freeze as the mix is drawn: the editor's last [`Msg::Fade`], or where a
    /// bound note has since switched them. See [`crate::mixer::Mixer::blackout`].
    blackout: bool,
    freeze: bool,
    /// **Soft takeover**, the preference: a CC whose value disagrees with its control's moves
    /// nothing until the fader passes the control's value. See [`picks_up`].
    soft_takeover: bool,
    /// Where each bound fader last was, as the 0–127 it sent, so the next message can tell
    /// whether it passed its control's value on the way.
    faders: HashMap<Target, u8>,
    /// Every bound control whose fader is out of pick-up, and where that fader is in the
    /// control's own units: the ghost mark the control wears. Empty with soft takeover off.
    ghosts: HashMap<Target, f32>,
    /// The last [`Msg::Fade`]'s `seq`, handed back on every snapshot.
    fade_seq: u64,
    /// Which triggers, on which devices, are holding each action input down, so two notes
    /// bound to one target are two hands on it: the button comes up when the last of them
    /// does. The device, so one that goes away lets go of what it held and nothing else.
    midi_notes: HashMap<PortRef, HashSet<(crate::midi::Device, crate::midi::Trigger)>>,
    /// Every one-shot event the editor has not acknowledged, one ordered log per kind — the
    /// MIDI messages read among them, for the monitor and for learning, since the writes
    /// above are how a value travels. See [`events`].
    events: Events,
    /// The count learning began at, or `u64::MAX` while nothing is learning. The editor binds
    /// only from a message read **after** it asked, so a message this thread had already
    /// driven a control with does not also become the binding.
    midi_learn_from: u64,
    /// The per-node half of the snapshot, filled by each tick while something is drawing it.
    notes: HashMap<NodeId, NodeNote>,
    /// Each CPU node's own error, where it has one.
    faults: HashMap<NodeId, String>,
    scopes: HashMap<NodeId, Scope>,
    /// What each node says under its trace, as `(key, value)` cells — `adsr`'s gate and stage.
    captions: HashMap<NodeId, Vec<(&'static str, String)>>,
    playheads: HashMap<NodeId, f32>,
    /// The offline render, while one is running. It drives the clock and steps frames, and
    /// it runs here because it owns the context and the nodes. See [`offline`].
    offline: Option<offline::Render>,
    /// How the last render ended, until the next one starts.
    outcome: Option<(u64, crate::app::render::Outcome)>,
    /// The renderer, on the synth's own [`Gpu`], which it holds. `None` where there is no GPU
    /// at all — every test, and egui_kittest.
    ///
    /// **Made current once, on this thread, and left current.** The context is the synth's
    /// for the life of the run; nothing enters it and nothing leaves it.
    renderer: Option<Renderer>,
    /// What the last frame published, for the viewers. An `Arc` because every paint
    /// callback in the frame holds one and none of them may copy it.
    published: Arc<Published>,
    /// The same, in the slot a window painted outside the editor's pass reads — a picture
    /// window. Written **here**, on every tick, because the frame thread does not run while
    /// the editor is minimized and that is exactly when a picture window must go on showing
    /// new frames.
    live: Arc<Live>,
    /// Where the pointer is over each surface. Written by whoever sees it — the pictures
    /// thread's own `wl_pointer`, or the editor's egui — and read once at the top of every
    /// tick, the way every device here is read.
    pointer: Arc<crate::pointer::Feed>,
    /// The buffer this tick writes its snapshot into, swapped with the editor's at the end
    /// of the tick. Held rather than allocated so the maps keep their capacity.
    out: Box<Snapshot>,
    /// Where each tick's milliseconds went, while the Status box is open. See [`meter`].
    laps: meter::Laps,
    /// The nodes an open workspace, or a claimed deck, leaves awake. Everything else is
    /// suspended and does not tick.
    awake: HashSet<NodeId>,
    /// Whether anything will read the per-node half of the snapshot. False with no canvas
    /// on screen and no picture in a window of its own.
    report: bool,
    /// Whether the Status box is open, which is the only thing that reads the CPU lines.
    status: bool,
    /// Whether the Main Input panel is unfolded, the only reason its picture is uploaded.
    main_input_preview: bool,
    /// How many Outputs the last job drew, for the Status box.
    drawn: usize,
}

use crate::audio::Scope;

/// A control a bound knob has written, and what the editor's last graph said it was.
///
/// `published` is the whole of the clobber guard: an arriving graph still carrying it was
/// built before the editor saw the write, and the knob's `value` stands. A graph carrying
/// anything else has answered for the write, and the entry goes — which also cancels it, so
/// a hand that moved the same control is not overwritten a frame later.
///
/// A **range narrowed between the write and the reconcile** converges inside that frame: the
/// editor fits the command to the new range, publishes a value that is therefore not
/// `published`, the entry is retired, and the next tick adopts the editor's — the document
/// and the tick agree on the fitted value rather than on the one the old range scaled.
/// [`MidiWrite`] for the mixer's controls — the fade, Blackout, Freeze — whose values are
/// plain values on the mix rather than controls on the graph.
#[derive(Clone, Copy)]
struct RigWrite<T> {
    published: T,
    value: T,
}

impl<T: Copy + PartialEq> RigWrite<T> {
    /// What the editor sent, against a write a knob or a note made: the knob's value stands
    /// while the editor's is still the one the knob moved away from, and the editor's is
    /// adopted, and the write retired, once it says anything else — or once a hand moved the
    /// control, which is later and wins even where it put it back where the knob found it.
    /// See [`Msg::Fade`].
    fn adopt(held: &mut Option<Self>, sent: T, hand: bool, current: &mut T) {
        if let Some(write) = held.filter(|w| !hand && w.published == sent) {
            *current = write.value;
        } else {
            *held = None;
            *current = sent;
        }
    }

    /// A knob or a note moved this control to `value`, from `current`.
    fn write(held: &mut Option<Self>, current: &mut T, value: T) {
        let published = *current;
        *current = value;
        held.get_or_insert(Self { published, value }).value = value;
    }
}

/// **Soft takeover**: whether a fader that sent `code`, and last sent `last`, may move a
/// control standing at `current`. `at` turns a 0–127 code into the control's own units, as
/// the binding scales it.
///
/// It may when the control is within one code of where the fader is now, or still within one
/// code of where the fader last was — it is following — or when the move from the last code
/// to this one passed over it. A first message, with no last code, takes over only near.
pub fn picks_up(at: impl Fn(u8) -> f32, last: Option<u8>, code: u8, current: f32) -> bool {
    let around = |c: u8| {
        let (lo, hi) = (at(c.saturating_sub(1)), at(c.saturating_add(1).min(127)));
        current >= lo.min(hi) - f32::EPSILON && current <= lo.max(hi) + f32::EPSILON
    };
    around(code)
        || last.is_some_and(|last| {
            let (from, to) = (at(last), at(code));
            around(last) || (current >= from.min(to) && current <= from.max(to))
        })
}

struct MidiWrite {
    published: ControlValue,
    value: f32,
}

impl Default for Synth {
    fn default() -> Self {
        Self {
            graph: Arc::default(),
            generation: 0,
            project: 0,
            seq: 0,
            clock: Clock::new(),
            transport: Transport::default(),
            seen: HashMap::new(),
            paces: HashMap::new(),
            main_input_seen: None,
            interval_ms: thread::DEFAULT_INTERVAL_MS,
            cpu: HashMap::new(),
            uniforms: HashMap::new(),
            counts: HashMap::new(),
            uniform_colors: HashMap::new(),
            frames: HashMap::new(),
            sims: HashMap::new(),
            actions: HashMap::new(),
            jumps: HashSet::new(),
            seeks: HashMap::new(),
            touches: HashMap::new(),
            held: HashSet::new(),
            prev_held: HashSet::new(),
            held_pointer: HashSet::new(),
            held_apart: HashSet::new(),
            readbacks: HashMap::new(),
            thumbs: HashMap::new(),
            main_input: maininput::Live::default(),
            main_input_choice: MainInput::default(),
            main_input_crossings: Vec::new(),
            assets: AssetPaths::default(),
            plan: Plan::default(),
            decks: (None, None),
            planned_decks: (None, None),
            midi: Vec::new(),
            midi_map: crate::midi::Bindings::default(),
            midi_learning: false,
            midi_owned: HashMap::new(),
            midi_balance: None,
            midi_blackout: None,
            midi_freeze: None,
            balance: crate::render::MixerJob::default().balance,
            method: crate::mixer::Method::default(),
            blackout: false,
            freeze: false,
            soft_takeover: false,
            faders: HashMap::new(),
            ghosts: HashMap::new(),
            fade_seq: 0,
            midi_notes: HashMap::new(),
            events: Events::default(),
            midi_learn_from: u64::MAX,
            notes: HashMap::new(),
            faults: HashMap::new(),
            scopes: HashMap::new(),
            captions: HashMap::new(),
            playheads: HashMap::new(),
            offline: None,
            outcome: None,
            renderer: None,
            published: Arc::default(),
            live: Arc::default(),
            pointer: Arc::default(),
            out: Box::default(),
            laps: meter::Laps::default(),
            awake: HashSet::new(),
            report: false,
            status: false,
            main_input_preview: false,
            drawn: 0,
        }
    }
}

impl Synth {
    // ------------------------------------------------------------ what crosses in

    /// Apply one message. The whole of the editor's side of the seam.
    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Graph { mut graph, project } => {
                if project != self.project {
                    self.replace_project(project);
                }
                // A control a knob wrote survives a graph built before the editor saw the
                // write. The same rule the decks keep, and the same comparison: what the
                // arriving graph says is measured against what the **last** one said, not
                // against what this thread is holding — unchanged there means the editor has
                // not answered for the write yet, so the knob's value stands. Where it has
                // changed the editor has seen it, by reconciling the write or by a hand
                // moving the same control, and the editor's value is the document's.
                //
                // Written only where the arriving graph disagrees with the knob, so a graph it
                // has nothing to say about stays the editor's own and is not copied.
                self.midi_owned.retain(|port, held| {
                    let Some(published) = graph
                        .get(port.node)
                        .and_then(|n| n.controls.get(port.key))
                        .copied()
                    else {
                        return false;
                    };
                    if published != held.published {
                        return false;
                    }
                    let value = ControlValue::Float(held.value);
                    if published != value
                        && let Some(node) = Arc::make_mut(&mut graph).get_mut(port.node)
                    {
                        node.controls.insert(port.key, value);
                    }
                    true
                });
                self.graph = graph;
                self.generation += 1;
            }
            Msg::Plan(plan) => {
                // A shader is one-shot and a plan outlives the frame that built it: a
                // source kept here would be uploaded again on every tick until the next
                // edit. Whatever the renderer did not take is asked for again by the
                // editor, which reads `awaiting_shader` off the snapshot.
                let planned = (plan.mixer.a, plan.mixer.b);
                // Adopted only where the editor actually changed them. A plan built before
                // the editor had seen this tick's claim still names the old deck, and taking
                // it wholesale would undo a cut a sequencer made a tick ago.
                if planned.0 != self.planned_decks.0 {
                    self.decks.0 = planned.0;
                }
                if planned.1 != self.planned_decks.1 {
                    self.decks.1 = planned.1;
                }
                self.planned_decks = planned;
                let mut plan = *plan;
                // A shader is one-shot, and a plan is replaced whole. The editor builds one
                // on every frame that changes what a plan reads and this thread drains all
                // of them at the top of a tick, so the plan carrying a fresh source can be
                // superseded before the renderer sees it — and a source the job has not taken yet is still in
                // `self.plan`, because `job` takes it out. Carry it over rather than lose
                // it: that was a new Output staying black until the next recompile.
                for out in &mut plan.outputs {
                    if out.shader.is_none() {
                        out.shader = self
                            .plan
                            .outputs
                            .iter_mut()
                            .find(|o| o.node == out.node)
                            .and_then(|o| o.shader.take());
                    }
                }
                for pass in &mut plan.passes {
                    if pass.shader.is_none() {
                        pass.shader = self
                            .plan
                            .passes
                            .iter_mut()
                            .find(|p| p.key == pass.key)
                            .and_then(|p| p.shader.take());
                    }
                }
                for probe in &mut plan.probes {
                    if probe.shader.is_none() {
                        probe.shader = self
                            .plan
                            .probes
                            .iter_mut()
                            .find(|p| p.output == probe.output)
                            .and_then(|p| p.shader.take());
                    }
                }
                // A port no pass draws any more has no thumbnail to hold, and a node no pass
                // measures has no reading: a tap cut loose loses its number instead of
                // freezing it. A pass that stopped drawing is still in the plan, so what it
                // measured holds its last.
                let thumbed: HashSet<PortRef> = plan
                    .passes
                    .iter()
                    .flat_map(|p| p.thumbs.iter().map(|(port, _)| *port))
                    .collect();
                self.thumbs.retain(|port, _| thumbed.contains(port));
                let measured: HashSet<NodeId> = plan
                    .passes
                    .iter()
                    .flat_map(|p| p.tap_nodes.iter().copied())
                    .collect();
                self.readbacks.retain(|id, _| measured.contains(id));
                self.plan = plan;
            }
            Msg::Inputs {
                live,
                report,
                status,
                assets,
                main_input,
                main_input_preview,
            } => {
                self.awake = live;
                self.report = report;
                self.status = status;
                self.assets = assets;
                self.main_input_choice = main_input;
                self.main_input_preview = main_input_preview;
            }
            Msg::Fade {
                balance,
                method,
                blackout,
                freeze,
                hands,
                seq,
            } => {
                // A fade a knob moved survives a fade sent before the editor saw the move, by
                // the rule the controls above keep: measured against what the **last** one
                // said. Changed there means the editor has answered — by reconciling the
                // write, or by a hand on the fade, which is later and wins. Blackout and
                // Freeze keep the same rule, each on its own.
                RigWrite::adopt(
                    &mut self.midi_balance,
                    balance,
                    hands.balance,
                    &mut self.balance,
                );
                RigWrite::adopt(
                    &mut self.midi_blackout,
                    blackout,
                    hands.blackout,
                    &mut self.blackout,
                );
                RigWrite::adopt(
                    &mut self.midi_freeze,
                    freeze,
                    hands.freeze,
                    &mut self.freeze,
                );
                self.method = method;
                self.fade_seq = seq;
            }
            Msg::MidiIn(rx) => self.midi.push(rx),
            Msg::MidiMap(map) => {
                self.midi_map = *map;
                // A control no longer bound has no fader to steer back to it.
                let bound: HashSet<Target> = self.midi_map.iter().map(|(_, b)| b.target).collect();
                self.ghosts.retain(|target, _| bound.contains(target));
                self.faders.retain(|target, _| bound.contains(target));
            }
            Msg::SoftTakeover(on) => {
                self.soft_takeover = on;
                if !on {
                    self.ghosts.clear();
                }
            }
            Msg::MidiReleaseAll => self.release_notes(|_| true),
            Msg::MidiLearn(on) => {
                self.midi_learning = on;
                self.midi_learn_from = if on {
                    self.events.midi.count()
                } else {
                    u64::MAX
                };
            }
            Msg::Press(port, down) => {
                if down {
                    self.held_apart.insert(port);
                } else {
                    self.held_apart.remove(&port);
                }
                self.gather_held();
            }
            Msg::PointerHeld(held) => {
                self.held_pointer = held;
                self.gather_held();
            }
            Msg::Seeks(seeks) => self.seeks.extend(seeks),
            Msg::Touches(touches) => {
                for (node, touch) in touches {
                    self.touches.entry(node).or_default().push(touch);
                }
            }
            Msg::ResetCpu => self.reset_cpu(),
            Msg::Transport(command) => self.transport.apply(command),
            Msg::Interval(ms) => self.interval_ms = ms.max(1.0),
            Msg::ReopenVideo => self.main_input.reopen_video(),
            Msg::ForgetDevices => self.main_input.forget_devices(),
            Msg::Thumbnails(nodes) => {
                if let Some(r) = &mut self.renderer {
                    for node in nodes {
                        r.request_thumbnail(node);
                    }
                }
            }
            Msg::StartRender(request) => {
                self.start_render(request.output, &request.settings, request.seq);
            }
            Msg::CancelRender => {
                if let Some(r) = &mut self.offline {
                    r.cancel();
                }
            }
        }
    }

    /// The interval a tick is paced to, in milliseconds.
    pub fn interval_ms(&self) -> f32 {
        self.interval_ms
    }

    /// What this thread's own copy of the graph says a control is: the value the tick just
    /// drew from, which is not the document's while a knob has moved and the editor has not
    /// run. `None` for a port that is not a control here.
    pub fn control(&self, port: PortRef) -> Option<ControlValue> {
        self.graph.get(port.node)?.controls.get(port.key).copied()
    }

    /// Which Output is on each deck **as the mix being rendered has it**, which is ahead of
    /// the project's mixer by however long the editor has not run. [`Synth::claim_decks`] is
    /// what puts a claim here.
    pub fn decks(&self) -> (Option<NodeId>, Option<NodeId>) {
        self.decks
    }

    // ------------------------------------------------------------ the one step

    /// One tick of the world: advance the clock, run every CPU node, draw the frame, and
    /// leave a snapshot.
    ///
    /// The one body both paths run. [`thread::run`] calls it from the synth thread's own
    /// loop and `App::tick` calls it inline where there is no GPU, so a test and a
    /// performance execute the same code through the same channel and the same swap.
    pub fn step(&mut self, beat: Beat) {
        // Before anything reads the graph or the buttons: a message applied after the tick
        // that read them would reach the audience a frame late, which is the whole thing
        // this is here to fix.
        self.take_midi();
        self.laps.lap(Work::Midi);
        // A render drives the clock itself, and the wall is not consulted while it does.
        if self.offline.is_some() {
            self.render_frame();
            return;
        }
        // The clock moves, and the transport reads it. A live step bounds a stateful node's
        // `dt` at `MAX_DT`; a stepped frame is not a stall, and is not bounded.
        let limit = match beat {
            Beat::Wall(now) => {
                self.clock.tick(now);
                self.transport.follow(self.clock.elapsed());
                crate::transport::MAX_DT
            }
            Beat::Delta(dt) => {
                self.clock.advance(dt);
                self.transport.follow(self.clock.elapsed());
                crate::transport::MAX_DT
            }
            Beat::At(t) => {
                self.clock.set_elapsed(t);
                self.transport.drive(t, t);
                f32::INFINITY
            }
        };
        let decks = self.tick(limit);
        self.claim_decks(&decks);
        for claim in decks {
            self.events.decks.push(claim);
        }
        self.laps.lap(Work::Nodes);
        self.render();
        self.publish();
    }

    /// The top of a tick, before the editor's messages are drained: the last one's laps are
    /// folded, and this one's begin while the Status box is open.
    pub(crate) fn begin_tick(&mut self) {
        self.laps.begin(self.status);
    }

    /// The CPU time since the last lap of this tick was `work`'s.
    pub(crate) fn lap(&mut self, work: Work) {
        self.laps.lap(work);
    }

    /// The thread slept `took` to its deadline since the last lap.
    pub(crate) fn slept(&mut self, took: std::time::Duration) {
        self.laps.slept(took);
    }

    /// The one place per-frame CPU work happens.
    ///
    /// Every node with a CPU half ticks once, in topological order, so a producer's uniform
    /// number is published before its consumer reads it. State is created on the first tick
    /// after the node appears and dropped on the first tick after it is gone — a microphone
    /// or a camera lives exactly as long as its node.
    ///
    /// What it returns is the decks an Output was put on this tick, by a hand on `Show on A`
    /// or by an event arriving down that port: an Output has no `tick` of its own to notice
    /// either, and the reading of a button is an edge against the previous tick's, which is
    /// the tick's to close.
    fn tick(&mut self, limit: f32) -> Vec<(NodeId, Channel)> {
        self.tick_only(limit, None)
    }

    /// The nodes whose picture is not yet the one they asked for: see
    /// [`crate::nodes::CpuNode::waiting`].
    pub(super) fn waiting(&self) -> Vec<NodeId> {
        self.cpu
            .iter()
            .filter(|(_, (_, state))| state.waiting())
            .map(|(id, _)| *id)
            .collect()
    }

    /// A tick of `only` these nodes where it names some, every node where it is `None`: what
    /// a render holding a frame for a clip ticks again, with the transport where it was, so
    /// nothing else takes a second step.
    fn tick_only(&mut self, limit: f32, only: Option<&HashSet<NodeId>>) -> Vec<(NodeId, Channel)> {
        let elapsed = self.clock.elapsed();
        // What a node ticking for the first time, or with no CPU half, sees of the transport:
        // this tick's own motion.
        let now = self.transport.time_since(None);
        // A handle rather than a borrow: the loop below wants `&mut self` for everything it
        // publishes, and an `Arc` bump is the price of holding the graph still across it.
        // Dropped before the writes at the end, so they find the graph unshared here.
        let held = Arc::clone(&self.graph);
        let graph = held.as_ref();

        // Drop state for nodes that left, or whose id now belongs to a different kind. A
        // stale uniform number would otherwise keep feeding a uniform for a node that
        // is not there.
        self.cpu
            .retain(|id, (slug, _)| graph.get(*id).is_some_and(|n| n.def.slug == *slug));
        self.uniforms.retain(|p, _| graph.get(p.node).is_some());
        self.counts.retain(|p, _| graph.get(p.node).is_some());
        self.uniform_colors
            .retain(|p, _| graph.get(p.node).is_some());
        self.frames.retain(|p, _| graph.get(p.node).is_some());
        self.sims.retain(|p, _| graph.get(p.node).is_some());
        self.seen.retain(|id, _| graph.get(*id).is_some());
        self.paces
            .retain(|id, _| graph.get(*id).is_some_and(crate::nodes::timing::runs_free));
        // Edges do not survive a frame. A consumer that wants to remember one remembers it
        // itself, which is what keeps a stale event from firing twice.
        self.actions.clear();
        self.jumps.clear();
        self.held_pointer.retain(|p| graph.get(p.node).is_some());
        self.held_apart.retain(|p| graph.get(p.node).is_some());
        self.gather_held();
        let (assets, choice) = (self.assets.clone(), self.main_input_choice.clone());
        let measuring = self.laps.on();
        // Every figure here is on the thread's own CPU clock, as the `CPU nodes` row they sit
        // under is, so a node blocked off the CPU never outweighs the total.
        let started = measuring.then(meter::thread_cpu);
        let clip = self.transport.time_since(self.main_input_seen);
        self.main_input_seen = Some(self.transport.seen());
        self.tick_main_input(&assets, &choice, &clip);
        if let Some(started) = started {
            self.laps
                .main_input(meter::thread_cpu().saturating_sub(started));
        }

        // Action edges order this one too: an event is delivered inside the tick that fired
        // it, so a receiver has to run after its source.
        // Built once, read by every `maininput` node: that is what makes eight of them agree
        // about one signal. Borrowed from `main_input`, which nothing in the loop writes.
        let feed = MainInputFeed {
            frame: self.main_input.frame(),
            analysis: *self.main_input.analysis(),
            sample_rate: self.main_input.sample_rate(),
            gain: choice.gain,
            crossings: &self.main_input_crossings,
            config: choice.bands,
            thresholds: choice.levels,
        };
        // The pointer, read once for the whole walk for the same reason: two nodes framed
        // the same way must agree about where the hand is.
        let pointer = self.pointer.read();

        // A node on no open workspace is suspended: it does not tick. What it published last
        // is still in `uniforms` and `frames`, because the node is still in the graph — so a
        // running consumer reading a suspended producer holds its last value rather than
        // seeing a gap.
        let order: Vec<NodeId> = graph.tick_order().to_vec();
        // What the ticks asked to write onto their own knobs — a roll, a scrub. Gathered across
        // the walk and applied after it, because writing one mid-walk would change the graph
        // the rest of the walk is reading.
        let mut control_writes = Vec::new();
        // And what they asked to write into their own values — a recording, when it stops.
        let mut value_writes = Vec::new();
        for id in order {
            if !self.awake.contains(&id) || only.is_some_and(|only| !only.contains(&id)) {
                continue;
            }
            let Some(node) = graph.get(id) else {
                continue;
            };
            let def = node.def;
            let started = measuring.then(meter::thread_cpu);
            // This node's own reading of the transport: the advance since it last ticked, which
            // is the whole gap for a node waking on a reopened tab.
            let own = (def.cpu.is_some() || def.timing.is_some()).then(|| {
                let time = self.transport.time_since(self.seen.get(&id).copied());
                self.seen.insert(id, self.transport.seen());
                time
            });
            // Where a node that moves with time is: running free, its Speed integrated here into
            // a playhead of its own; looping, the playhead at its pace where nothing is in
            // its Time. A node that draws reads it as a count published under each Time's key, a
            // CPU node through `TickContext::cycle`. See `nodes::timing`.
            let mut free = None;
            if let (Some(timing), Some(time)) = (def.timing, own) {
                let running = crate::nodes::timing::runs_free(node);
                let mut pace = running.then(|| self.paces.entry(id).or_default());
                for axis in timing.axes() {
                    let key = PortRef::new(id, axis.time);
                    let at = match pace.as_deref_mut() {
                        Some(pace) => {
                            let speed = crate::nodes::cpu::number(
                                graph,
                                &self.uniforms,
                                PortRef::new(id, axis.speed),
                            );
                            let at = pace.step(axis.index, f64::from(speed), &time).to;
                            if axis.index == 0 {
                                free = Some(at);
                            }
                            Some(at * timing.pace_of(*axis))
                        }
                        None => graph
                            .source_of(key)
                            .is_none()
                            .then_some(time.playhead * timing.pace_of(*axis)),
                    };
                    if let Some(at) = at.filter(|_| def.cpu.is_none()) {
                        self.counts.insert(key, at);
                    }
                }
                if !running {
                    self.paces.remove(&id);
                }
            }
            // A dual output is the one CPU work a node with no `cpu` half has: its formula
            // in Rust, evaluated for whichever of its outputs the graph has resolved to a
            // uniform number. Producers come first in this order, so a chain of them settles
            // in one frame. A flipped one is withdrawn — it is a field, and a field has no
            // reading — and the row then draws nothing.
            if def.outputs.iter().any(|o| o.eval.is_some()) {
                let mut ctx = TickContext::new(
                    graph,
                    &mut self.uniforms,
                    &mut self.counts,
                    &mut self.uniform_colors,
                    &mut self.frames,
                    &mut self.sims,
                    &mut self.actions,
                    &mut self.jumps,
                    &self.held,
                    &self.readbacks,
                    &assets,
                    &self.seeks,
                    &self.touches,
                    feed,
                    pointer,
                    &mut control_writes,
                    &mut value_writes,
                    now.step(limit),
                    elapsed,
                    now,
                );
                for out in def.outputs {
                    let Some(eval) = out.eval else { continue };
                    let publishes = ctx
                        .node(id)
                        .and_then(|n| n.output(out.key))
                        .is_some_and(|p| p.ty.is_uniform());
                    if publishes {
                        let value = eval(id, &ctx);
                        ctx.publish(id, out.key, value);
                    } else {
                        ctx.withdraw(id, out.key);
                    }
                }
            }
            let Some(cpu) = &def.cpu else {
                if let Some(started) = started {
                    let took = meter::thread_cpu().saturating_sub(started);
                    self.laps.node(id, def.slug, took);
                }
                continue;
            };
            let state = self
                .cpu
                .entry(id)
                .or_insert_with(|| (def.slug, (cpu.create)()));
            let time = own.unwrap_or(now);
            let mut ctx = TickContext::new(
                graph,
                &mut self.uniforms,
                &mut self.counts,
                &mut self.uniform_colors,
                &mut self.frames,
                &mut self.sims,
                &mut self.actions,
                &mut self.jumps,
                &self.held,
                &self.readbacks,
                &assets,
                &self.seeks,
                &self.touches,
                feed,
                pointer,
                &mut control_writes,
                &mut value_writes,
                time.step(limit),
                elapsed,
                time,
            )
            .running_free(id, free);
            state.1.tick(id, &mut ctx);
            if let Some(started) = started {
                let took = meter::thread_cpu().saturating_sub(started);
                self.laps.node(id, def.slug, took);
            }
        }
        // After every node, so a sequencer's edge fired this tick lands this tick. None of
        // the three reads a control, so they are the same before the writes below as after.
        let decks = self.deck_claims(graph);
        let snaps = self.claims_on(graph, "snap");
        let fires = self.fires(graph);
        drop(held);
        // Every knob a tick asked to move, moved — on this thread's graph, and on the
        // document a frame later through the same seam a MIDI knob's write crosses.
        for (target, value) in control_writes {
            self.write_control(target, value);
        }
        // And every value one wrote onto itself, the same way: on this thread's graph now,
        // and on the document through the bus a frame later.
        for (node, key, value) in value_writes {
            self.write_value(node, key, value);
        }
        // A seek is a place asked for once: every tick has now seen it, and a scrubber the
        // hand let go of must not keep dragging the clip back to where it let go.
        self.seeks.clear();
        self.touches.clear();
        if self.report {
            self.gather_report();
        }
        // The frame's reading of the buttons, closed where it was read: the next tick sees
        // edges and not levels.
        self.prev_held.clone_from(&self.held);
        if let Some(r) = &mut self.renderer {
            for node in snaps {
                r.request_snap(node);
            }
        }
        if !fires.is_empty() {
            self.events.fired.push(fires.into_iter().collect());
        }
        decks
    }

    /// Every action port that fired on this tick, output and input alike.
    ///
    /// An output fires when it emitted a down. An input fires when something wired to it
    /// did, or when a finger went down on its button — the same two readings
    /// [`Self::claims_on`] takes of `Show on A`, generalized to every action port there is,
    /// because the throb an editor draws is the same fact wherever it happens.
    ///
    /// Gathered here rather than re-derived by the editor: the edge is against the previous
    /// tick, and the previous tick is this thread's.
    fn fires(&self, graph: &Graph) -> HashSet<PortRef> {
        let mut out = HashSet::new();
        for (port, events) in &self.actions {
            if events.iter().any(|e| e.is_down()) {
                out.insert(*port);
                out.extend(graph.targets_of(*port));
            }
        }
        out.extend(self.held.difference(&self.prev_held).copied());
        out
    }

    /// The Outputs whose action input `key` fired on this tick.
    ///
    /// The reading an Output has no `CpuNode` to take for itself: a button that went down
    /// since the last tick, or an edge arriving from something that did — a sequencer lane
    /// and a finger being the same thing to it.
    fn claims_on(&self, graph: &Graph, key: &'static str) -> Vec<NodeId> {
        graph
            .iter()
            .filter(|(_, n)| n.def.is_output)
            .map(|(id, _)| id)
            .filter(|id| self.fired_on(graph, PortRef::new(*id, key)))
            .collect()
    }

    /// Whether this action input saw a down on this tick, from a finger or from a cable.
    fn fired_on(&self, graph: &Graph, port: PortRef) -> bool {
        let pressed = self.held.contains(&port) && !self.prev_held.contains(&port);
        pressed
            || graph.sources_of(port).any(|src| {
                self.actions
                    .get(&src)
                    .is_some_and(|es| es.iter().any(|e| e.is_down()))
            })
    }

    /// Put this tick's claims on the mix being rendered, so a cut reaches the audience on
    /// the tick it happened rather than on the editor's next frame.
    fn claim_decks(&mut self, claims: &[(NodeId, Channel)]) {
        for (node, channel) in claims {
            match channel {
                Channel::A => self.decks.0 = Some(*node),
                Channel::B => self.decks.1 = Some(*node),
            }
        }
        // A deck whose Output left the graph is empty, on the tick it went. The editor does
        // the same to the project's mixer from its own side.
        if self.decks.0.is_some_and(|id| self.graph.get(id).is_none()) {
            self.decks.0 = None;
        }
        if self.decks.1.is_some_and(|id| self.graph.get(id).is_none()) {
            self.decks.1 = None;
        }
    }

    /// The Outputs a press or an event put on a deck this tick.
    ///
    /// The two action inputs of an Output, read here because an Output has no `CpuNode` to
    /// read them: a button that went down since the last tick, or an edge arriving from
    /// something that did — a sequencer lane and a finger being the same thing to it.
    fn deck_claims(&self, graph: &Graph) -> Vec<(NodeId, Channel)> {
        let mut claims = Vec::new();
        for (id, _) in graph.iter().filter(|(_, n)| n.def.is_output) {
            for (key, channel) in [("show_a", Channel::A), ("show_b", Channel::B)] {
                if self.fired_on(graph, PortRef::new(id, key)) {
                    claims.push((id, channel));
                }
            }
        }
        claims
    }

    /// Empty the MIDI queues and act on what was in them, at the top of the tick.
    ///
    /// **The message is applied on the tick it arrives**, whatever the editor is doing: a CC
    /// bound to a number control writes that control on this thread's graph, and a
    /// note bound to an action input holds and releases the same button a hand would. Every
    /// message also rides out on the snapshot, in the order it arrived, because the window's
    /// monitor and learning are the editor's and both want the whole stream.
    ///
    /// Nothing is driven while the editor is learning — a knob being taught a control must
    /// not also move whatever it moved a moment ago — or while a render is running, where the
    /// document is closed and the editor would refuse the command anyway. **A note-off is the
    /// exception to both**: a note held when a render starts would otherwise be a gate that
    /// never closes, and letting go of a key is never the thing either rule is guarding.
    fn take_midi(&mut self) {
        let mut wires = Vec::new();
        for queue in &self.midi {
            wires.extend(queue.try_iter());
        }
        if wires.is_empty() {
            return;
        }
        let quiet = self.midi_learning || self.offline.is_some();
        for wire in wires {
            let (device, message) = match wire {
                crate::midi::Wire::Message(device, message) => (device, message),
                // A device that went away lets go of every note it held: the note-offs it
                // owes will never come. Never quiet, for the reason a note-off is not.
                crate::midi::Wire::Gone(device) => {
                    self.release_notes(|(from, _)| *from == device);
                    continue;
                }
            };
            self.events.midi.push(message);
            let Some(binding) = self.midi_map.get(message.trigger()) else {
                continue;
            };
            let held = (device, message.trigger());
            match (message.kind, binding.target) {
                (crate::midi::Kind::Control { value, .. }, Target::Port(target)) if !quiet => {
                    self.midi_control(target, value);
                }
                (crate::midi::Kind::Control { value, .. }, Target::Balance) if !quiet => {
                    self.midi_balance(value);
                }
                // A press of the mixer's: a CC holds it on at 64 and up, which is a toggle
                // button's 127 and 0 and a momentary one's hold; a note-on flips it.
                (crate::midi::Kind::Control { value, .. }, target) if !quiet => {
                    self.midi_switch(target, |_| value >= 64);
                }
                (crate::midi::Kind::Note { on: true, .. }, target)
                    if !quiet && target.is_switch() =>
                {
                    self.midi_switch(target, |on| !on);
                }
                (crate::midi::Kind::Note { on, .. }, Target::Port(target)) if on && !quiet => {
                    self.midi_note(target, held, true);
                }
                (crate::midi::Kind::Note { on: false, .. }, Target::Port(target)) => {
                    self.midi_note(target, held, false);
                }
                // A note on the fade is the plausible mistake a CC on a button is, and it
                // does the same thing: nothing, with its row still in the window.
                (crate::midi::Kind::Control { .. } | crate::midi::Kind::Note { .. }, _) => {}
            }
        }
    }

    /// Hold or release an action input from a note, counting **which** notes are holding it.
    ///
    /// Two notes bound to one target are two hands on one button: the first note-off must not
    /// let go while the other key is still down. Keyed by the trigger and the device rather
    /// than counted, so a controller that repeats a note-on does not leave the button held by
    /// a phantom, and a device that goes away takes only its own notes with it.
    fn midi_note(
        &mut self,
        target: PortRef,
        held: (crate::midi::Device, crate::midi::Trigger),
        down: bool,
    ) {
        let holding = self.midi_notes.entry(target).or_default();
        if down {
            holding.insert(held);
        } else {
            holding.remove(&held);
        }
        if holding.is_empty() {
            self.midi_notes.remove(&target);
            self.held_apart.remove(&target);
        } else {
            self.held_apart.insert(target);
        }
        self.gather_held();
    }

    /// Let go of every note that `which` names — every note, for the MIDI window's Release
    /// all; a device's, when it goes away — and of every action input left with none.
    fn release_notes(
        &mut self,
        which: impl Fn(&(crate::midi::Device, crate::midi::Trigger)) -> bool,
    ) {
        let mut freed = Vec::new();
        self.midi_notes.retain(|target, holding| {
            holding.retain(|held| !which(held));
            if holding.is_empty() {
                freed.push(*target);
            }
            !holding.is_empty()
        });
        for target in freed {
            self.held_apart.remove(&target);
        }
        self.gather_held();
    }

    /// Blackout or Freeze, switched by a message: `to` says what it becomes from what it is.
    /// Written on the mix this thread draws, so the show goes black on the tick the pad was
    /// hit, and carried to the editor's mixer as the fade is.
    fn midi_switch(&mut self, target: Target, to: impl Fn(bool) -> bool) {
        let (held, current) = match target {
            Target::Blackout => (&mut self.midi_blackout, &mut self.blackout),
            Target::Freeze => (&mut self.midi_freeze, &mut self.freeze),
            Target::Port(_) | Target::Balance => return,
        };
        let value = to(*current);
        RigWrite::write(held, current, value);
    }

    /// Whether a fader's message may move its control, under the preference, and the ghost
    /// mark it leaves when it may not. Every fader's last code is kept whatever the
    /// preference, so turning it on knows where each one is.
    fn takes_over(
        &mut self,
        target: Target,
        code: u8,
        current: f32,
        at: impl Fn(u8) -> f32,
    ) -> bool {
        let last = self.faders.insert(target, code);
        if !self.soft_takeover || picks_up(&at, last, code, current) {
            self.ghosts.remove(&target);
            return true;
        }
        self.ghosts.insert(target, at(code));
        false
    }

    /// One CC on one control: scale it into that control's own ends and write it.
    ///
    /// **The control's own range**, `nodes::control_range`, which is the instance's where it
    /// has one and the definition's otherwise — a speed narrowed to 0.9–1.1 gets the whole
    /// fader across that, which is the reason an instance may narrow a range at all.
    ///
    /// A binding onto something that is not a number does nothing. An action input bound to a
    /// CC is a plausible mistake and there is no sensible reading of a continuous value as a
    /// press; the row stays in the window, so what it names is visible rather than silently
    /// dropped.
    fn midi_control(&mut self, target: PortRef, value: u8) {
        let Some(node) = self.graph.get(target.node) else {
            return;
        };
        let Some(range) = crate::nodes::control_range(node.def, node, target.key) else {
            return;
        };
        // 0..127 across the range, linearly. A curve is a field on the binding that nothing
        // sets yet: see the proposal.
        let at = move |code: u8| range.min + f32::from(code) / 127.0 * (range.max - range.min);
        let current = match node.controls.get(target.key) {
            Some(ControlValue::Float(v)) => *v,
            _ => at(value),
        };
        if self.takes_over(Target::Port(target), value, current, at) {
            self.write_control(target, at(value));
        }
    }

    /// One CC on the fade: −1 at 0, +1 at 127, written onto the fade this thread renders
    /// with, so the mix moves on the tick the fader did. It rides out on the snapshot for
    /// the editor to write into the mixer — not through the bus, because [moving the fade
    /// is playing, not editing](crate::mixer::Mixer) and never enters the undo history.
    fn midi_balance(&mut self, value: u8) {
        let at = |code: u8| -1.0 + 2.0 * f32::from(code) / 127.0;
        if self.takes_over(Target::Balance, value, self.balance, at) {
            RigWrite::write(&mut self.midi_balance, &mut self.balance, at(value));
        }
    }

    /// Write one number control from this thread, and arrange for the document to catch up.
    ///
    /// The seam a bound MIDI knob and a tick's `write_control` both cross: the value lands on
    /// the graph this thread holds, so the world moves on the tick it was asked for, and
    /// rides out on the snapshot for [`crate::app::App`] to put through the command bus as
    /// the `SetControl` a hand would have sent. Fitted to the control's own range, because that
    /// is what the editor will fit it to anyway.
    ///
    /// A write onto something that is not a number control does nothing, which is what a
    /// binding pointing at an action input means.
    pub(crate) fn write_control(&mut self, target: PortRef, value: f32) {
        let Some(node) = self.graph.get(target.node) else {
            return;
        };
        let Some(range) = crate::nodes::control_range(node.def, node, target.key) else {
            return;
        };
        let Some(published) = node.controls.get(target.key).copied() else {
            return;
        };
        let value = value.clamp(range.min, range.max);
        // The first write after a graph arrives copies it, since the editor holds the same
        // one: pointer bumps and this node. Every write after that is in place, because the
        // tick has let go of its own handle and nothing else holds the copy.
        if let Some(node) = Arc::make_mut(&mut self.graph).get_mut(target.node) {
            node.controls.insert(target.key, ControlValue::Float(value));
        }
        // `published` is what the editor last said, taken on the **first** write only: every
        // write after it is against a value this thread put there, and a graph carrying that
        // value is a graph the editor has answered for.
        self.midi_owned
            .entry(target)
            .or_insert(MidiWrite { published, value })
            .value = value;
    }

    /// Write one of a node's own values from this thread, and arrange for the document to
    /// catch up.
    ///
    /// The seam [`Self::write_control`] crosses, for the other half of what a tick can say
    /// about the document. It needs no reconciliation against a hand: a recording is written
    /// when a transport stops rather than every frame, so there is no stream of writes for a
    /// later edit to be buried under.
    pub(crate) fn write_value(
        &mut self,
        node: NodeId,
        key: &'static str,
        value: crate::graph::Value,
    ) {
        let declared = self
            .graph
            .get(node)
            .is_some_and(|n| n.def.value(key).is_some());
        if !declared {
            return;
        }
        if let Some(n) = Arc::make_mut(&mut self.graph).get_mut(node) {
            n.values.insert(key, value.clone());
        }
        self.events.values.push((node, key, value));
    }

    /// Take what every CPU node says about itself, so `ui/` never reaches into one.
    fn gather_report(&mut self) {
        self.notes.clear();
        self.faults.clear();
        self.scopes.clear();
        self.captions.clear();
        self.playheads.clear();
        for (id, (_, state)) in &self.cpu {
            if let Some(error) = state.error() {
                self.faults.insert(*id, error);
            }
            if let Some(text) = state.status() {
                self.notes.insert(
                    *id,
                    NodeNote {
                        text,
                        progress: state.progress(),
                    },
                );
            }
            if let Some(scope) = state.scope() {
                self.scopes.insert(*id, scope);
            }
            let caption = state.caption();
            if !caption.is_empty() {
                self.captions.insert(*id, caption);
            }
            if let Some(playhead) = state.playhead() {
                self.playheads.insert(*id, playhead);
            }
        }
    }

    /// Every CPU node back to the state it was created in, and every published uniform and
    /// event forgotten with it, so the next tick is a first tick. Frames stay: a fresh node
    /// republishes its picture on that tick, and a texture that is still bound is not
    /// unbound for a frame in between.
    pub fn reset_cpu(&mut self) {
        for (_, state) in self.cpu.values_mut() {
            state.reset();
        }
        self.uniforms.clear();
        self.counts.clear();
        self.uniform_colors.clear();
        self.actions.clear();
        self.jumps.clear();
        // A reset node is born again on its next tick, and what was queued for the world it
        // had is not that world's.
        for sim in self.sims.values_mut() {
            sim.passes.clear();
        }
        self.notes.clear();
        self.faults.clear();
        self.scopes.clear();
        self.captions.clear();
        self.playheads.clear();
        // Every node is born again, and a node born takes the tick it is born on as its first:
        // a free-running one where the playhead puts it.
        self.seen.clear();
        self.paces.clear();
        // A reading from live play would feed an `autogain` on frame zero.
        self.readbacks.clear();
        self.thumbs.clear();
    }

    /// Another project's graph arrived: every node's state, everything published, the plan,
    /// the decks and the renderer's Outputs and worlds go. Nothing ticks until the next
    /// [`Msg::Inputs`], since the last one named the old project's assets.
    fn replace_project(&mut self, project: u64) {
        self.project = project;
        self.cpu.clear();
        self.reset_cpu();
        self.frames.clear();
        self.sims.clear();
        self.seeks.clear();
        self.touches.clear();
        self.held.clear();
        self.held_pointer.clear();
        self.held_apart.clear();
        self.prev_held.clear();
        self.events.forget_project();
        self.midi_owned.clear();
        self.midi_notes.clear();
        self.ghosts.clear();
        self.faders.clear();
        self.awake.clear();
        self.plan = Plan::default();
        self.decks = (None, None);
        self.planned_decks = (None, None);
        if let Some(renderer) = &mut self.renderer {
            renderer.forget_project();
        }
    }

    /// The union of the two sources, which is what `tick` reads.
    ///
    /// **An action input takes many sources and the hand is one more of them.** The pointer's
    /// half is whatever the canvas drew this frame and is replaced wholesale; everything else
    /// — a note from a controller, a test, the agent-driven layer — is held apart and lives
    /// until it is let go. One set for both would mean whichever wrote last won, and the
    /// canvas writes last every frame.
    fn gather_held(&mut self) {
        self.held = self.held_pointer.union(&self.held_apart).copied().collect();
    }

    // ------------------------------------------------------------ the frame it draws

    /// The job this tick's plan describes, with the uniforms resolved and the textures
    /// every CPU node published attached.
    ///
    /// Resolved **here** rather than by the editor: a uniform number's value and a control's
    /// are both read out of state this thread owns, and resolving them on the frame thread
    /// would make every one of them a tick stale.
    pub fn job(&mut self) -> FrameJob {
        let graph = Arc::clone(&self.graph);
        let hold = self.offline.as_ref().is_some_and(offline::Render::holding);
        let clear = self
            .offline
            .as_ref()
            .is_some_and(offline::Render::wants_clear);
        // A supersampled render draws the Output it is rendering larger and nothing else
        // larger at all: the whole patch above it is evaluated at that size, which is what
        // makes the frame that comes back down clean rather than merely smoothed.
        let supersampled = self
            .offline
            .as_ref()
            .map(|r| (r.output(), r.scale()))
            .filter(|(_, scale)| *scale > 1);
        let drawing = self.drawing();
        // What a render draws: the Output and every Output it reads, whose programs it waits
        // for before it steps a frame (`program_ready`).
        let rendered: HashSet<NodeId> = self
            .offline
            .as_ref()
            .and_then(|r| self.plan.outputs.iter().find(|o| o.node == r.output()))
            .map(|o| {
                std::iter::once(o.node)
                    .chain(o.needs.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        // In the plan's order, which is the graph's: a producer before its consumers.
        let mut outputs = Vec::with_capacity(self.plan.outputs.len());
        for o in &mut self.plan.outputs {
            let mode = match o.mode {
                Mode::Suspended => OutputMode::Suspended,
                // A render's hold draws nothing, but a program on its way to what the render
                // draws is handed over, so the frame it steps next is drawn with it.
                Mode::Idle | Mode::Draw { .. }
                    if hold && o.shader.is_some() && rendered.contains(&o.node) =>
                {
                    OutputMode::Idle
                }
                _ if hold => OutputMode::Suspended,
                Mode::Dark => OutputMode::Dark,
                Mode::Idle | Mode::Draw { .. } if !drawing.contains(&o.node) => OutputMode::Idle,
                Mode::Idle | Mode::Draw { .. } => OutputMode::Draw,
            };
            outputs.push(OutputJob {
                node: o.node,
                resolution: match supersampled {
                    Some((node, scale)) if node == o.node => {
                        (o.resolution.0 * scale, o.resolution.1 * scale)
                    }
                    _ => o.resolution,
                },
                // One-shot: taken out of the plan on the tick it is sent. Not to a suspended
                // Output, a render's hold included, whose source the renderer ignores: it
                // waits in the plan for the first job that is not.
                shader: if mode == OutputMode::Suspended {
                    None
                } else {
                    o.shader.take()
                },
                source: o.source,
                mode,
                uniforms: Vec::new(),
                taps: o.taps,
                clear,
            });
        }
        let probes = self
            .plan
            .probes
            .iter_mut()
            .map(|p| ProbeJob {
                output: p.output,
                shader: p.shader.take(),
                source: p.source,
                uniforms: Vec::new(),
                taps: p.taps,
            })
            .collect::<Vec<_>>();
        self.drawn = outputs.iter().filter(|o| o.mode.draws()).count();
        // Resolved after the walk above, because resolving borrows `self`.
        for (job, plan) in outputs.iter_mut().zip(&self.plan.outputs) {
            if job.mode.draws() {
                job.uniforms = self.resolve(&graph, &plan.uniforms);
            }
        }
        let mut probes = probes;
        for (job, plan) in probes.iter_mut().zip(&self.plan.probes) {
            if drawing.contains(&plan.output) {
                job.uniforms = self.resolve(&graph, &plan.uniforms);
            }
        }
        // Nothing draws while a render holds, a pass included: the render's frames are the
        // only work the GPU is given.
        let mut passes: Vec<PassJob> = self
            .plan
            .passes
            .iter_mut()
            .map(|p| PassJob {
                key: p.key,
                shader: p.shader.take(),
                source: p.source,
                resolution: p.resolution,
                // Nobody is looking at its thumbnails: only its measurements' square.
                region: (!p.shown).then_some(p.region),
                uniforms: Vec::new(),
                taps: p.taps,
                draws: (p.shown || p.measures || p.wanted_by.iter().any(|o| drawing.contains(o)))
                    && !hold,
            })
            .collect();
        for (job, plan) in passes.iter_mut().zip(&self.plan.passes) {
            if job.draws {
                job.uniforms = self.resolve(&graph, &plan.uniforms);
            }
        }
        let sources = self
            .frames
            .iter()
            // The Main Input's own picture rides in as one more source, under a port no node
            // owns, so the panel's preview is the ordinary blit and not a special case in
            // `render/`. Only while the panel is unfolded, because this is an upload.
            .chain(
                self.main_input
                    .frame()
                    .filter(|_| self.main_input_preview)
                    .map(|f| (&crate::maininput::PREVIEW, f)),
            )
            .map(|(port, f)| match self.plan.sampling.get(port) {
                Some(s) => SourceJob {
                    port: *port,
                    frame: Arc::clone(f),
                    wrap: s.wrap,
                    filter: s.filter,
                },
                None => SourceJob::new(*port, Arc::clone(f)),
            })
            .collect();
        // A simulation's passes are this tick's work and are run once, so the job takes
        // them; the shape stays, so a node that did not tick keeps its world and its picture.
        let sims = self
            .sims
            .iter_mut()
            .map(|(port, sim)| {
                let sampling = self.plan.sampling.get(port);
                SimJob {
                    port: *port,
                    sim: Simulation {
                        size: sim.size,
                        agents: sim.agents,
                        params: sim.params.clone(),
                        passes: std::mem::take(&mut sim.passes),
                        steps: sim.steps,
                    },
                    wrap: sampling.map_or(crate::nodes::TextureWrap::Mirror, |s| s.wrap),
                    filter: sampling.map_or(crate::nodes::TextureFilter::Linear, |s| s.filter),
                }
            })
            .collect();
        FrameJob {
            // The playhead's fraction of a second, which is all a shader reads of it, at the
            // same precision at every second of the show.
            time: crate::nodes::phasor::split(self.transport.playhead())[1],
            outputs,
            probes,
            passes,
            display: self.plan.display,
            mixer: crate::render::MixerJob {
                // The decks and the fade are the synth's; the size is the plan's.
                a: self.decks.0,
                b: self.decks.1,
                balance: self.balance,
                method: self.method,
                resolution: self.plan.mixer.resolution,
                blackout: self.blackout,
                freeze: self.freeze,
            },
            sources,
            sims,
        }
    }

    /// The Outputs this tick draws: the plan's own, and every one something has asked a
    /// picture of since the plan was built — a deck a press or an edge claimed inside a
    /// tick, the render, a picture the renderer wants — with what the plan says it
    /// needs, so the picture reads this tick's frames and readings.
    #[doc(hidden)]
    pub fn drawing(&self) -> HashSet<NodeId> {
        // **Pause freezes feedback**: on a tick the playhead stood still, an Output in a loop
        // draws only where a picture is asked of it — its first, a Snap, a save's thumbnail.
        // Everything else it would draw for, the tab, a deck or a window, sees its last frame.
        let still = self.transport.stood_still();
        let frozen: HashSet<NodeId> = self
            .plan
            .outputs
            .iter()
            .filter(|o| still && o.feeds_back)
            .map(|o| o.node)
            .collect();
        let mut drawing: HashSet<NodeId> = self
            .plan
            .outputs
            .iter()
            .filter(|o| o.mode.why().is_some())
            .map(|o| o.node)
            .filter(|node| !frozen.contains(node))
            .collect();
        let decks = [self.decks.0, self.decks.1];
        let render = self.offline.as_ref().map(offline::Render::output);
        for o in &self.plan.outputs {
            let pictured = render == Some(o.node)
                || self
                    .renderer
                    .as_ref()
                    .is_some_and(|r| r.wants_picture(o.node));
            if pictured {
                drawing.insert(o.node);
                drawing.extend(o.needs.iter().copied());
            } else if decks.contains(&Some(o.node)) && !frozen.contains(&o.node) {
                drawing.insert(o.node);
                drawing.extend(o.needs.iter().filter(|n| !frozen.contains(n)).copied());
            }
        }
        drawing
    }

    /// Turn each uniform provider into a concrete value, read fresh from the graph and from
    /// what this tick published.
    ///
    /// A color — a color control, or a color a CPU node published — is straight on the CPU
    /// and premultiplied here, the one place it enters a shader ([`alpha::premultiply`]).
    ///
    /// The name is an `Arc<str>` clone, not a new `String`: this runs for every uniform of
    /// every Output on every tick.
    fn resolve(
        &self,
        graph: &Graph,
        uniforms: &[(Arc<str>, UniformProvider)],
    ) -> Vec<(Arc<str>, UniformValue)> {
        uniforms
            .iter()
            .filter_map(|(name, provider)| {
                let value = match provider {
                    UniformProvider::Control { node, key, .. } => {
                        match graph.get(*node)?.controls.get(key)? {
                            ControlValue::Float(v) => UniformValue::Float(*v),
                            ControlValue::Color(v) => UniformValue::Vec4(alpha::premultiply(*v)),
                        }
                    }
                    UniformProvider::NodeTexture { node, port } => {
                        UniformValue::NodeTexture(PortRef::new(*node, port))
                    }
                    UniformProvider::NodeUniform { node, port, ty } => {
                        let at = PortRef::new(*node, port);
                        match ty {
                            UniformType::Vec4 => UniformValue::Vec4(alpha::premultiply(
                                self.uniform_colors.get(&at).copied().unwrap_or([0.0; 4]),
                            )),
                            _ => {
                                UniformValue::Float(self.uniforms.get(&at).copied().unwrap_or(0.0))
                            }
                        }
                    }
                    UniformProvider::NodeCount { node, port } => {
                        let at = PortRef::new(*node, port);
                        let count = self.counts.get(&at).copied().unwrap_or_else(|| {
                            f64::from(self.uniforms.get(&at).copied().unwrap_or(0.0))
                        });
                        UniformValue::Vec2(crate::nodes::phasor::split(count))
                    }
                    UniformProvider::Option { node, key } => {
                        let n = graph.get(*node)?;
                        let value = n.options.get(key).map_or("", |v| v.as_str());
                        UniformValue::Int(n.def.option(key)?.index_of(value))
                    }
                };
                Some((Arc::clone(name), value))
            })
            .collect()
    }

    /// **Test accessor.** One shader's uniforms, resolved against the graph this synth
    /// holds and what its last tick published.
    #[doc(hidden)]
    pub fn resolve_shader(&self, shader: &compile::Shader) -> Vec<(Arc<str>, UniformValue)> {
        let graph = Arc::clone(&self.graph);
        self.resolve(&graph, &providers(shader))
    }

    /// Draw one frame on the synth's context and publish what it drew, then take what the
    /// GPU read back.
    fn render(&mut self) {
        let job = self.job();
        self.laps.lap(Work::Job);
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        renderer.set_measuring(self.laps.on());
        renderer.draw(&job);
        drop(job);
        self.laps.lap(Work::Draw);
        self.laps.gpu(&renderer.take_gpu_spans());
        self.published = Arc::new(renderer.publish());
        self.live.set(Arc::clone(&self.published));
        self.laps.lap(Work::Publish);
        self.collect_readbacks();
        self.laps.lap(Work::Readbacks);
    }

    /// Route what each workspace pass read back: each tap slot to the node that owns it, and
    /// each thumbnail's words to its port.
    ///
    /// A measured node is measured in one pass, its workspace's, so its reading comes from
    /// there and nowhere else. A pass that did not report replaces nothing, so a measurement
    /// keeps its last value across a dropped frame rather than blinking to zero, and a
    /// thumbnail is taken only from the pass on screen, which draws them.
    fn collect_readbacks(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        for (key, words) in renderer.take_passes() {
            let Some(pass) = self.plan.passes.iter().find(|p| p.key == key) else {
                continue;
            };
            for (i, node) in pass.tap_nodes.iter().enumerate() {
                let Some(slot) = words.get(i * compile::TAP_WORDS..(i + 1) * compile::TAP_WORDS)
                else {
                    continue;
                };
                let slot: [u32; compile::TAP_WORDS] = slot.try_into().expect("slice is TAP_WORDS");
                self.readbacks.insert(*node, slot);
            }
            if !pass.shown {
                continue;
            }
            for (i, (port, ty)) in pass.thumbs.iter().enumerate() {
                let at = pass.thumb_base + i * compile::THUMB_CELLS;
                let Some(cells) = words.get(at..at + compile::THUMB_CELLS) else {
                    continue;
                };
                self.thumbs.insert(
                    *port,
                    Arc::new(PortThumb {
                        number: *ty == crate::graph::PortType::VaryingNumber,
                        words: cells.to_vec(),
                    }),
                );
            }
        }
    }

    // ------------------------------------------------------------ what crosses out

    /// Fill the buffer this tick's snapshot goes in and hand it back.
    ///
    /// The cost is refcounts and small maps: a picture is an `Arc<Frame>`, a published
    /// texture is a handle, and the two gathered halves — the per-node report and the
    /// Status box's CPU lines — are only built while something is drawing them.
    fn publish(&mut self) {
        let clock = self.report_clock();
        self.seq += 1;
        let out = &mut *self.out;
        // A buffer filled before the project changed, which the editor may never have taken.
        if out.project != self.project {
            out.forget_project();
            out.project = self.project;
        }
        out.generation = self.generation;
        out.seq = self.seq;
        out.clock = clock;
        out.transport = self.transport.report();
        out.notes.clone_from(&self.notes);
        out.faults.clone_from(&self.faults);
        out.scopes.clone_from(&self.scopes);
        out.captions.clone_from(&self.captions);
        out.playheads.clone_from(&self.playheads);
        out.traces.clear();
        out.curves.clear();
        out.pucks.clear();
        out.gears.clear();
        if self.report {
            out.gears.extend(
                self.cpu
                    .iter()
                    .filter_map(|(id, (_, s))| Some((*id, s.gear()?))),
            );
            out.traces.extend(
                self.cpu
                    .iter()
                    .filter_map(|(id, (_, s))| Some((*id, s.trace()?.clone()))),
            );
            out.curves.extend(
                self.cpu
                    .iter()
                    .filter_map(|(id, (_, s))| Some((*id, s.curve()?))),
            );
            out.pucks.extend(
                self.cpu
                    .iter()
                    .filter_map(|(id, (_, s))| Some((*id, s.puck()?))),
            );
        }
        out.uniforms.clone_from(&self.uniforms);
        out.counts.clone_from(&self.counts);
        out.uniform_colors.clone_from(&self.uniform_colors);
        out.frames.clone_from(&self.frames);
        out.actions.clone_from(&self.actions);
        out.held.clone_from(&self.held);
        out.readbacks.clone_from(&self.readbacks);
        out.thumbs.clone_from(&self.thumbs);
        out.cpu_lines.clear();
        out.cpu_reports.clear();
        if self.status {
            let mut ids: Vec<NodeId> = self.cpu.keys().copied().collect();
            ids.sort();
            for id in ids {
                let Some((slug, state)) = self.cpu.get(&id) else {
                    continue;
                };
                out.cpu_reports.insert(
                    id,
                    format!(
                        "error {:?} | status {:?} | debug {:?}",
                        state.error(),
                        state.status(),
                        state.debug()
                    ),
                );
                if let Some(line) = state
                    .error()
                    .or_else(|| state.status())
                    .or_else(|| state.debug())
                {
                    out.cpu_lines.push((id, format!("{slug}{id}"), line));
                }
            }
        }
        out.midi_learn_from = self.midi_learn_from;
        // The writes are the whole of what has not landed yet, in node order so two controls
        // reconcile the same way on every run.
        out.midi_writes.clear();
        out.midi_writes
            .extend(self.midi_owned.iter().map(|(p, w)| (*p, w.value)));
        out.midi_writes.sort_by_key(|(p, _)| (p.node, p.key));
        out.midi_balance = self.midi_balance.map(|w| w.value);
        out.midi_blackout = self.midi_blackout.map(|w| w.value);
        out.midi_freeze = self.midi_freeze.map(|w| w.value);
        out.midi_ghosts.clear();
        out.midi_ghosts
            .extend(self.ghosts.iter().map(|(t, v)| (*t, *v)));
        out.fade_seq = self.fade_seq;
        out.main_input = MainInputReport {
            video_status: self.main_input.video_status(&self.main_input_choice.video),
            audio_status: self.main_input.audio_status(&self.main_input_choice.audio),
            error: self.main_input.error().map(str::to_string),
            devices: self.main_input.devices().to_vec(),
            cameras: self.main_input.cameras().to_vec(),
            analysis: *self.main_input.analysis(),
            sample_rate: self.main_input.sample_rate(),
            hears: self.main_input.hears(),
            has_picture: self.main_input.frame().is_some(),
            position: self.main_input.position(),
        };
        out.published = Arc::clone(&self.published);
        // After the borrow of `out` is done with: both read the renderer, which is beside
        // it on `self`.
        self.out.render = self.render_report();
        self.out.offline = self.offline_report();
        // Each taken off the renderer once, so each is an event.
        if let Some(r) = &mut self.renderer {
            let probes = r.take_probes();
            if !probes.is_empty() {
                self.events.probes.push(probes);
            }
            for thumbnail in r.take_thumbnails() {
                self.events.thumbnails.push(thumbnail);
            }
            for snap in r.take_snaps() {
                self.events.snaps.push(snap);
            }
        }
        self.events.copy_into(&mut self.out.events);
        self.laps.poll_process();
        self.out.phases = self.laps.report();
    }

    fn render_report(&mut self) -> RenderReport {
        let generation = self.plan.generation;
        let Some(renderer) = &mut self.renderer else {
            return RenderReport {
                plan_generation: generation,
                ..RenderReport::default()
            };
        };
        let outputs: Vec<NodeId> = self.plan.outputs.iter().map(|o| o.node).collect();
        RenderReport {
            has_gpu: true,
            plan_generation: self.plan.generation,
            drawn: self.drawn,
            errors: renderer.errors.clone(),
            gpu_times: outputs
                .iter()
                .filter_map(|id| Some((*id, renderer.gpu_time(*id)?)))
                .collect(),
            dropped_frames: outputs
                .iter()
                .map(|id| (*id, renderer.dropped_frames(*id)))
                .collect(),
            drops: renderer.all_drops(),
            gpu_waits: renderer.waits(),
            linking: outputs
                .iter()
                .copied()
                .filter(|id| renderer.is_linking(*id))
                .collect(),
            awaiting_shader: outputs
                .iter()
                .copied()
                .filter(|id| renderer.awaiting_shader(*id))
                .collect(),
            probe_awaiting: outputs
                .iter()
                .copied()
                .filter(|id| renderer.probe_awaiting(*id))
                .collect(),
            pass_awaiting: self
                .plan
                .passes
                .iter()
                .map(|p| p.key)
                .filter(|key| renderer.pass_awaiting(*key))
                .collect(),
        }
    }

    /// Take the snapshot this tick wrote, leaving an empty buffer for the next one.
    pub(crate) fn take_snapshot(&mut self) -> Box<Snapshot> {
        std::mem::take(&mut self.out)
    }

    /// Put the editor's old buffer back, to be written over, and let go of every event it
    /// says the editor has seen. The maps keep their capacity, so a tick allocates nothing it
    /// did not already allocate once.
    pub(crate) fn put_buffer(&mut self, mut buffer: Box<Snapshot>) {
        self.events.forget_seen(&buffer.events);
        // The spent buffer's pictures are a tick or more old, and every target they name is
        // held while they are: the newest is what it would be written over with anyway.
        buffer.published = Arc::clone(&self.published);
        self.out = buffer;
    }

    // ------------------------------------------------------------ the GPU half

    /// The slot every viewer outside the editor's pass reads, handed over before the first
    /// tick. See [`thread::Host::live`].
    pub fn publish_into(&mut self, live: Arc<Live>) {
        self.live = live;
    }

    /// The slot the pointer's two sources write into, handed over before the first tick.
    /// See [`thread::Host::pointer`].
    pub fn point_from(&mut self, pointer: Arc<crate::pointer::Feed>) {
        self.pointer = pointer;
    }

    /// Take the GPU this synth will draw on, and make the renderer on it.
    ///
    /// Called **on the thread that will own it**: the editor's device, asked of it with
    /// [`Gpu::for_synth`]. A test hands in a device of its own, and the offline render is
    /// driven through it with a real GPU and no eframe.
    pub fn attach_gpu(&mut self, gpu: Gpu) -> Result<(), String> {
        self.renderer = Some(Renderer::new(gpu)?);
        Ok(())
    }

    /// Let go of the renderer and everything it holds, which wgpu frees once the GPU is done
    /// with it.
    pub fn destroy_gpu(&mut self) {
        self.renderer = None;
    }

    // ------------------------------------------------------------ test accessors

    /// Everything one CPU node reports, for diagnostics. The same line the Status box
    /// gathers, asked for one node at a time.
    pub fn cpu_report(&self, node: NodeId) -> String {
        match self.cpu.get(&node) {
            Some((_, state)) => format!(
                "error {:?} | status {:?} | debug {:?}",
                state.error(),
                state.status(),
                state.debug()
            ),
            None => "no cpu state".to_string(),
        }
    }

    /// The clock as the snapshot reports it, read straight off it where the synth is on this
    /// thread.
    pub fn report_clock(&self) -> ClockReport {
        ClockReport {
            ticks: self.clock.ticks(),
            dt: self.clock.dt(),
            elapsed: self.clock.elapsed(),
        }
    }

    /// **Test accessor.** The simulation a node last published on one of its ports: its shape,
    /// its numbers and the steps it has taken, with the passes the last frame job took.
    #[doc(hidden)]
    pub fn simulation(&self, port: PortRef) -> Option<&Simulation> {
        self.sims.get(&port)
    }

    /// **Test accessor.** What the viewers were handed on the last tick.
    #[doc(hidden)]
    pub fn published(&self) -> &Published {
        &self.published
    }

    /// **Test accessor.** What a simulation holds on the GPU right now, read back and waited
    /// for. `None` without a renderer, or before the simulation's first frame job.
    #[doc(hidden)]
    pub fn read_simulation(&mut self, port: PortRef) -> Option<crate::render::SimReadback> {
        self.renderer.as_ref()?.read_simulation(port)
    }

    /// **Test accessor.** Leave this Output's link in flight unfinished until let go.
    #[doc(hidden)]
    pub fn hold_link(&mut self, node: NodeId, hold: bool) {
        if let Some(r) = &mut self.renderer {
            r.hold_link(node, hold);
        }
    }

    /// **Test accessor.** An Output's latest frame as RGBA8, rows bottom first, read back
    /// and waited for. `None` without a renderer, or for an Output it has never been handed.
    #[doc(hidden)]
    pub fn read_output(&self, node: NodeId) -> Option<(u32, u32, Vec<u8>)> {
        let (width, height, halves) = self.renderer.as_ref()?.read_output(node)?;
        Some((
            width,
            height,
            crate::render::readback::half_to_rgba8(&halves),
        ))
    }

    /// The transport as it stands.
    pub fn transport_report(&self) -> crate::transport::Report {
        self.transport.report()
    }

    /// The clock a test drives, and the one `clock::Stepper` swaps its own into. Reachable
    /// only while the synth is inline, which is where a test drives a clock.
    pub fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::picks_up;

    /// A fader across 0 to 127 in the control's own units, one code a unit.
    fn at(code: u8) -> f32 {
        f32::from(code)
    }

    #[test]
    fn a_first_message_takes_over_only_near() {
        assert!(picks_up(at, None, 64, 64.0), "on it");
        assert!(picks_up(at, None, 63, 64.0), "a code away");
        assert!(
            !picks_up(at, None, 10, 64.0),
            "far away, with no last code to cross from"
        );
    }

    #[test]
    fn a_fader_takes_over_once_it_passes_the_control() {
        assert!(!picks_up(at, Some(10), 40, 64.0), "short of it");
        assert!(picks_up(at, Some(40), 90, 64.0), "over it on the way up");
        assert!(picks_up(at, Some(90), 30, 64.0), "and on the way down");
    }

    #[test]
    fn a_fader_the_control_still_follows_goes_on_driving_it() {
        assert!(
            picks_up(at, Some(64), 100, 64.0),
            "the control is where it was left"
        );
    }
}
