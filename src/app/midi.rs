// SPDX-License-Identifier: AGPL-3.0-or-later

//! The editor's half of MIDI: learning, the monitor, and the document catching up.
//!
//! **A message is not acted on here.** The reader's queue is the synth's and the map is the
//! synth's copy, so a CC bound to a control moves that control inside the tick it arrived on
//! — see [`crate::synth::Synth::take_midi`] — whether or not this editor is painting. What
//! crosses back is two lists on the snapshot: every message the tick read, for the window's
//! monitor and for learning, and every control write it made, which `App` puts through the
//! command bus as the [`Command::SetControl`] it would have been. So the document, the file
//! and the undo history say what the knob did, one frame later.
//!
//! **Learning stays here**, because it needs the control under the pointer: the synth drives
//! nothing while a control is waiting, so a knob being taught does not also move whatever it
//! moved a moment ago, and the first message after the click is the one that binds.
//!
//! **A knob writes the undo history, and a gesture ends when the hand stops.** A control's
//! value is document data: it is saved, and a knob moved during a set changes what the file
//! says. But a drag ends on the pointer coming up and a hardware knob has no release, so the
//! gesture would run until some *other* command interrupted it and then swallow that too.
//! Instead the knobs hold it open until **silence**: a burst of CC that stops for [`SETTLE`]
//! lets its step close, which is one undo per stretch of knob movement — measured on the
//! editor's own clock, so a step closes once the editor runs again after the silence. Every
//! knob's write joins the one open gesture, a hand's drag included: see `document::history`.
//!
//! **A binding outlives its node.** A node deleted takes its bound control out of the graph
//! but not out of the map, so undoing the delete brings the knob back with it. The synth, the
//! window and the file see only the bindings whose control the graph has — see
//! [`MidiDesk::publish`] — and ids are never reused, so a binding left behind can never land
//! on another node.
//!
//! [`MidiDesk`] holds all of it. It is handed the link it sends through and the project whose
//! map and mixer it writes, and hands back the control writes for `App` to apply, since an
//! edit is the document's.

use crate::command::Command;
use crate::graph::{ControlValue, Graph, PortRef};
use crate::midi::{Kind, Target, Trigger};
use crate::project::Project;
use crate::synth::Msg;
use std::collections::{HashMap, VecDeque};

use super::link::SynthLink;

/// How long a bound control has to stop moving before it stops holding its undo step open.
///
/// A fifth of a second: longer than the gap between two messages from a knob being turned,
/// which is milliseconds, and shorter than the pause between two deliberate moves of it.
pub const SETTLE: f64 = 0.2;

/// How many messages the monitor keeps.
pub const MONITOR: usize = 200;

#[derive(Default)]
// Each of these is one independent fact about the rig: a monitor watching while a hand
// learns is both, and neither says anything about the other.
#[allow(clippy::struct_excessive_bools)]
pub(super) struct MidiDesk {
    /// MIDI in, open: the device list and the wiring. `None` on a box with no sequencer, and
    /// headless. The messages themselves are the synth's — see [`crate::synth::Msg::MidiIn`].
    midi: Option<crate::midi::Midi>,
    /// The editor's own way into the synth's MIDI queue, for a test and for an agent: a
    /// message posted here arrives exactly as a device's does, on the next tick. Opened by
    /// the first `apply_midi`, which is the only thing that uses it.
    tx: Option<std::sync::mpsc::Sender<crate::midi::Wire>>,
    /// The live half of the map as the synth last received it, so it is sent when it changes
    /// and not per frame. See [`MidiDesk::publish`].
    published: crate::midi::Bindings,
    /// **Inline only.** The editor's own seconds, for the silence a knob lets go of its undo
    /// step on. Under eframe the frame reads egui's clock instead; nothing drives both.
    clock: f64,
    /// Controls this editor moved itself, and the graph generation each move went out on: a
    /// MIDI write from before that is not applied over the hand. See [`MidiDesk::take`].
    barrier: HashMap<PortRef, u64>,
    /// The control waiting for the next message, while a hand is learning one.
    learning: Option<Target>,
    /// The `seq` of the fade a hand's own move of a mixer control went out on — the fade,
    /// Blackout, Freeze — barring a write the synth made before it saw the move. `barrier`,
    /// for the controls that are not ports.
    rig_barrier: HashMap<Target, u64>,
    /// When a bound control was last moved, so it can let go of its undo step on silence
    /// rather than on a release it will never get.
    moving: Option<f64>,
    /// The window's monitor is watching, so arriving messages are kept.
    monitor: bool,
    /// The newest messages, for the monitor. Empty unless it is watching.
    log: VecDeque<crate::midi::Message>,
    /// The MIDI window is open.
    window: bool,
    /// What each bound trigger last carried, for its row in the window. Only bound ones: an
    /// unbound knob has no row to put a number on, and keeping every CC a desk sends would
    /// be a map that grows all evening.
    values: HashMap<Trigger, u8>,
    /// The soft takeover preference as the synth last heard it.
    soft_takeover: Option<bool>,
}

impl MidiDesk {
    pub(super) fn new(midi: Option<crate::midi::Midi>) -> Self {
        Self {
            midi,
            ..Self::default()
        }
    }

    /// Hand the synth the map's live bindings — the ones whose control `graph` has — where
    /// they differ from what it already has.
    ///
    /// **The one place the map crosses.** Every path that changes a binding ends here —
    /// learning one, unbinding one, clearing them all, a node deleted out from under one or
    /// brought back by an undo, a project opened — for the same reason
    /// `SynthLink::publish_graph` is the one place the graph crosses: the tick reads the map,
    /// so a tick must never read one the editor is in the middle of writing, and a map that
    /// is one edit behind is a knob driving the wrong control.
    pub(super) fn publish(
        &mut self,
        bindings: &crate::midi::Bindings,
        graph: &Graph,
        link: &SynthLink,
    ) {
        if bindings.live(graph).eq(self.published.iter()) {
            return;
        }
        self.published = bindings.live_in(graph);
        link.send(Msg::MidiMap(Box::new(self.published.clone())));
    }

    /// Read what the tick left of MIDI and do the editor's half of it: the monitor, the
    /// binding a learn was waiting for, and the fade. Once a frame, after the snapshot is
    /// taken. What comes back is every control write a bound knob made that may still need
    /// applying — see [`Self::lands`] — for `App` to put through the bus.
    ///
    /// **What has not been seen**, not what the buffer holds: the messages are one of the
    /// synth's event logs, so a snapshot read twice yields nothing twice and a synth that
    /// ticked ten times between two frames is read in order and once. See
    /// [`crate::synth::events`].
    ///
    /// **A hand is later and wins.** A write may still be in a snapshot published before the
    /// editor dragged the same control, and applying it would take the drag back a frame
    /// after it happened. So a control this editor moved itself carries a barrier — the graph
    /// generation the move went out on — and a write is not applied until the tick has run
    /// over a graph at least that new. By then the synth has retired its own entry, so
    /// anything still here was written after the hand.
    pub(super) fn take(
        &mut self,
        link: &mut SynthLink,
        project: &mut Project,
    ) -> Vec<(PortRef, f32)> {
        let learn_from = link.snapshot().midi_learn_from;
        for (n, message) in link.snapshot().events.midi.unseen("MIDI messages") {
            self.observe(*message, n >= learn_from, link, project);
        }

        let snapshot = link.snapshot_mut();
        let generation = snapshot.generation;
        self.barrier.retain(|_, at| generation < *at);
        let writes = std::mem::take(&mut snapshot.midi_writes)
            .into_iter()
            .filter(|(target, _)| !self.barrier.contains_key(target))
            .collect();
        // The mixer's controls, by the same rule and with the same barrier, except that
        // landing one is not a command: the mixer is written directly, as the panel's own
        // drag and presses write it.
        let fade_seq = snapshot.fade_seq;
        let (balance, blackout, freeze) = (
            snapshot.midi_balance,
            snapshot.midi_blackout,
            snapshot.midi_freeze,
        );
        let mixer = project.mixer_mut();
        if let Some(balance) = balance
            && self.passed(Target::Balance, fade_seq)
            && mixer.balance != balance
        {
            mixer.set_balance(balance);
        }
        if let Some(on) = blackout
            && self.passed(Target::Blackout, fade_seq)
        {
            mixer.blackout = on;
        }
        if let Some(on) = freeze
            && self.passed(Target::Freeze, fade_seq)
        {
            mixer.freeze = on;
        }
        writes
    }

    /// Whether a write of the synth's onto a mixer control may land: the synth has seen the
    /// fade carrying the hand's last move of it, which retires the barrier.
    fn passed(&mut self, target: Target, fade_seq: u64) -> bool {
        match self.rig_barrier.get(&target) {
            Some(at) if fade_seq < *at => false,
            _ => {
                self.rig_barrier.remove(&target);
                true
            }
        }
    }

    /// Whether a knob's write is still news to the document: an entry the editor's own
    /// graph already agrees with is a write that has landed, which is what keeps a map
    /// republished every tick from being a command every frame. One that is news marks the
    /// knob as moving at `now`.
    pub(super) fn lands(&mut self, graph: &Graph, target: PortRef, value: f32, now: f64) -> bool {
        let landed = graph
            .get(target.node)
            .and_then(|n| n.controls.get(target.key))
            .is_some_and(|v| *v == ControlValue::Float(value));
        if !landed {
            self.moving = Some(now);
        }
        !landed
    }

    /// Bar a control this editor moved itself from being written back over by a MIDI write
    /// the synth made before it saw the move. `generation` is the graph the move went out on.
    pub(super) fn bar(&mut self, cmd: &Command, generation: u64) {
        match cmd {
            Command::SetControl { node, key, .. }
            | Command::SetRange { node, key, .. }
            | Command::ClearRange { node, key } => {
                self.barrier.insert(PortRef::new(*node, key), generation);
            }
            Command::SetControls { node, values }
            | Command::SetSettings {
                node,
                controls: values,
                ..
            } => {
                for (key, _) in values {
                    self.barrier.insert(PortRef::new(*node, key), generation);
                }
            }
            _ => {}
        }
    }

    /// A hand moved one of the mixer's controls: bar a knob's or a note's write until the
    /// synth has seen the fade the move goes out on.
    pub(super) fn bar_rig(&mut self, target: Target, fade: u64) {
        self.rig_barrier.insert(target, fade);
    }

    /// Hand the synth the soft takeover preference, where it changed.
    pub(super) fn soft_takeover(&mut self, on: bool, link: &SynthLink) {
        if self.soft_takeover != Some(on) {
            self.soft_takeover = Some(on);
            link.send(Msg::SoftTakeover(on));
        }
    }

    /// Post a message as though a device had sent it, or a device going. See
    /// `App::apply_midi`.
    pub(super) fn post(&mut self, wire: crate::midi::Wire, link: &SynthLink) {
        // Opened where it is first used rather than beside every `App`, so the one the synth
        // is holding is the one this posts into however the app was built.
        if self.tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tx = Some(tx);
            link.send(Msg::MidiIn(rx));
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(wire);
        }
    }

    /// One message, as the tick read it: learn it, or note what it carried.
    ///
    /// Learning consumes the message. The synth drives nothing while a control is waiting, so
    /// this is the whole of what happens to the message that binds — and `learnable` is what
    /// makes that true across the tick it takes the asking to arrive: a message read *before*
    /// the synth heard about the learn may already have driven its old target, and binding
    /// from it would be the one thing this gesture must not do. The binding crosses to the
    /// synth when `App::take_midi` publishes, before the next tick.
    fn observe(
        &mut self,
        message: crate::midi::Message,
        learnable: bool,
        link: &SynthLink,
        project: &mut Project,
    ) {
        self.keep(message);
        if let Some(target) = learnable.then(|| self.learning.take()).flatten() {
            project.midi_mut().bind(message.trigger(), target);
            link.send(Msg::MidiLearn(false));
            return;
        }
        // What a bound trigger last carried, for its row in the window. Bound ones only: an
        // unbound knob has no row to put a number on, and keeping every CC a desk sends
        // would be a map that grows all evening.
        if let Kind::Control { value, .. } = message.kind
            && project.midi().get(message.trigger()).is_some()
        {
            self.values.insert(message.trigger(), value);
        }
    }

    /// Whether the knob has been still for [`SETTLE`], which lets go of its undo step. True
    /// once per burst.
    pub(super) fn settled(&mut self, now: f64) -> bool {
        if let Some(last) = self.moving
            && now - last >= SETTLE
        {
            self.moving = None;
            return true;
        }
        false
    }

    /// Keep the newest messages for the window's monitor, and only while it is watching.
    ///
    /// A ring of [`MONITOR`] and no more: a knob sends a hundred messages a second and a log
    /// that kept them all would be the largest thing in the process by the end of a set.
    fn keep(&mut self, message: crate::midi::Message) {
        if !self.monitor {
            return;
        }
        if self.log.len() == MONITOR {
            self.log.pop_front();
        }
        self.log.push_back(message);
    }

    /// Start learning: the next message binds to this control.
    pub(super) fn learn(&mut self, target: Target, link: &SynthLink) {
        self.learning = Some(target);
        link.send(Msg::MidiLearn(true));
    }

    /// Stop learning, having bound nothing.
    pub(super) fn cancel_learn(&mut self, link: &SynthLink) {
        self.learning = None;
        link.send(Msg::MidiLearn(false));
    }

    /// The control currently waiting for a message, if one is.
    pub(super) fn learning(&self) -> Option<Target> {
        self.learning
    }

    /// **Inline only.** Move the editor's own clock by one step's worth, and read it.
    pub(super) fn advance(&mut self, beat: crate::synth::Beat) -> f64 {
        let dt = match beat {
            crate::synth::Beat::Wall(now) => {
                self.clock = now;
                0.0
            }
            crate::synth::Beat::Delta(dt) => f64::from(dt),
            crate::synth::Beat::At(t) => {
                self.clock = t;
                0.0
            }
        };
        self.clock += dt;
        self.clock
    }

    pub(super) fn open_window(&mut self) {
        self.window = true;
    }

    pub(super) fn window_open(&self) -> bool {
        self.window
    }

    /// What the MIDI window draws: the devices, the live map, learning, the monitor.
    pub(super) fn view<'a>(&'a self, graph: &'a Graph) -> crate::ui::midi::MidiView<'a> {
        crate::ui::midi::MidiView {
            sources: self
                .midi
                .as_ref()
                .map(crate::midi::Midi::sources)
                .unwrap_or_default(),
            absent: self.midi.is_none(),
            bindings: &self.published,
            learning: self.learning,
            monitor: self.monitor,
            log: &self.log,
            values: &self.values,
            graph,
        }
    }

    /// Answer one thing the window asked: the rig's half here, and the map's in `project`,
    /// which crosses on the next `publish`. A node to show and the mixer to unfold are
    /// `App`'s, and are handed back.
    pub(super) fn answer(
        &mut self,
        action: crate::ui::midi::MidiAction,
        link: &SynthLink,
        project: &mut Project,
    ) -> Option<crate::ui::midi::MidiAction> {
        use crate::ui::midi::MidiAction as A;
        match action {
            A::Rescan => {
                if let Some(midi) = &mut self.midi {
                    midi.connect_all();
                }
            }
            A::Unbind(trigger) => {
                self.values.remove(&trigger);
                project.midi_mut().unbind(trigger);
            }
            A::ClearAll => {
                self.values.clear();
                project.midi_mut().clear();
            }
            A::SetMonitor(on) => {
                self.monitor = on;
                if !on {
                    self.log.clear();
                }
            }
            A::ClearLog => self.log.clear(),
            A::CancelLearn => self.cancel_learn(link),
            A::ReleaseAll => link.send(Msg::MidiReleaseAll),
            A::Close => self.window = false,
            A::GoTo(_) | A::ShowMixer => return Some(action),
        }
        None
    }
}
