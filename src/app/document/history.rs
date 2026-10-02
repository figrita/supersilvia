// SPDX-License-Identifier: AGPL-3.0-or-later

//! Undo, redo, and the gesture that decides where one edit ends and the next begins.
//!
//! Undo is a snapshot rather than an inverse, so no command can have a wrong undo: it does
//! not have one at all. A snapshot is the graph itself, shared: see `Step`.
//!
//! **One gesture is open at a time**, and its step is the one on top of the ring. It stays
//! open until the pointer has been released and the bound knobs have fallen silent, and
//! every continuous write while it is open — the hand's, a knob's, a value the tick writes —
//! lands in its step, so undo takes that whole stretch back at once. See docs/architecture.md,
//! "A gesture ends on a release, or on silence".

use super::Document;
use crate::command::Command;
use crate::graph::{Graph, NodeId};
use std::sync::Arc;

/// How many edits are undoable at most. The second of the two caps on the ring, and the one
/// an ordinary session meets: a step holds only what its edit wrote.
pub(in crate::app) const UNDO_DEPTH: usize = 256;

/// How many bytes of snapshots the ring may hold. The first of the two caps.
///
/// Depth alone bounds the ring in steps and not in bytes. A step shares every node its edit
/// did not write, so a scrub of one knob costs that node and the graph's map of pointers —
/// but an edit that writes every node, an auto-arrange of a large workspace, holds a whole
/// graph of its own, and 256 of those is gigabytes. Budgeting the bytes gives that case a
/// ceiling that does not depend on how big a graph someone opens.
/// `Graph::footprint_beside` is the estimate it counts.
const UNDO_BYTES: usize = 64 * 1024 * 1024;

/// One undoable edit: the graph as it stood on the *other* side of `command`.
///
/// Undo is a snapshot, not an inverse, so restoring one is exact for every command that
/// exists and every command that ever will — including ones whose inverse would be awkward to
/// write, like duplicating a selection or pasting a subgraph. A command cannot have a wrong
/// undo, because it does not have an undo at all.
///
/// The snapshot is the very `Arc<Graph>` the editor held before the edit, shared with the
/// graph after it in every node the edit did not write, so taking one copies nothing.
#[derive(Debug, Clone)]
pub(super) struct Step {
    graph: Arc<Graph>,
    /// The command that produced the other side, or the last one a gesture wrote into it.
    /// Every step carries one: opening a project replaces the history rather than adding to
    /// it, so nothing else makes one.
    command: Command,
    /// What the snapshot holds that the graph beside it does not, measured once when the
    /// step is made rather than on every trim.
    bytes: usize,
    /// The edit serial of the graph in this step, so traveling back to it restores what
    /// `Document::dirty` compares against.
    serial: u64,
}

impl Step {
    /// A step holding `graph`, counted against `beside`: the graph on the other side of
    /// `command`, which is its neighbour in the ring.
    pub(super) fn new(graph: Arc<Graph>, beside: &Graph, command: Command, serial: u64) -> Self {
        Self {
            bytes: graph.footprint_beside(beside),
            graph,
            command,
            serial,
        }
    }
}

/// Who wrote a command, which decides what holds open the gesture it writes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Origin {
    /// The pointer or the keyboard. Holds the gesture open until a release.
    Hand,
    /// A bound MIDI control, applied as the document catches up with the tick. Holds the
    /// gesture open until the knobs fall silent.
    Midi,
    /// A value the tick wrote onto its own node. Joins a gesture that is open and holds none
    /// open.
    Tick,
}

/// The gesture in progress, whose step is the one on top of the ring. Open while either
/// half is.
///
/// Not `undo.last()` alone: undo pops the top and opening a file empties the ring, either of
/// which would let the next write join a step that is not this gesture's, and that step's
/// snapshot is then the wrong one to go back to.
#[derive(Debug, Clone, Default)]
pub(super) struct Gesture {
    /// The hand's last continuous command, until a release or a command of the hand's that
    /// does not [join](joins) it.
    hand: Option<Command>,
    /// A bound knob has written since the knobs last fell silent.
    knobs: bool,
}

/// The one control a command writes, if it writes exactly one.
///
/// `ClearRange` is here beside `SetControl` because resetting a control is both of them: the
/// pair is one gesture and belongs in one undo step, so one `R` is one undo.
pub(super) fn control_target(cmd: &Command) -> Option<(NodeId, &'static str)> {
    match cmd {
        Command::SetControl { node, key, .. }
        | Command::ClearRange { node, key }
        // A typed option row sends one `SetOption` per keystroke — see `ui/text.rs` — and
        // without this each character would open its own undo step. A picked-from-a-list
        // option coalesces the same way, harmlessly: two clicks on the same select in one
        // gesture are one choice either way.
        | Command::SetOption { node, key, .. }
        // And a note's text, for exactly the same reason: typing a sentence into one is a
        // sentence, not forty undo steps.
        | Command::SetValue { node, key, .. } => Some((*node, *key)),
        _ => None,
    }
}

/// Whether a command can be continued at all. Anything else ends the gesture, so the next
/// drag of the same node opens a fresh step instead of joining an old one.
pub(super) fn coalescable(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::MoveNodes { .. } | Command::SetNodeWidth { .. } | Command::SetControls { .. }
    ) || control_target(cmd).is_some()
}

/// Does the hand's `cmd` continue the hand's last command, `last`, with no knob turning?
///
/// A node drag emits one `MoveNodes` a frame and a control scrub one `SetControl` a frame.
/// Each is one gesture, so each is one undo step — and, just as importantly, one snapshot
/// rather than sixty. A gesture that continues also **joins** the command, which is what
/// lets a handle carrying two axes write two controls a frame without opening a step per
/// axis.
pub(super) fn joins(last: &Command, cmd: &Command) -> bool {
    match (last, cmd) {
        (Command::MoveNodes { moves: a }, Command::MoveNodes { moves: b }) => {
            // The same nodes, in the same order — one drag. A different set is a new
            // gesture, so releasing and grabbing something else is its own undo step.
            a.len() == b.len() && a.iter().zip(b).all(|((x, _), (y, _))| x == y)
        }
        // A grip dragged across the canvas writes a width a frame, and the whole drag is one
        // gesture — the same rule `MoveNodes` follows, for the same reason. And one press on a
        // region that writes a node's controls and its options together — a pad's preset, its
        // edges either side of its numbers — is one undo step: only across the two kinds, so
        // two options or two handles are still told apart as before.
        (Command::SetNodeWidth { node: a, .. }, Command::SetNodeWidth { node: b, .. })
        | (Command::SetControls { node: a, .. }, Command::SetOption { node: b, .. })
        | (Command::SetOption { node: a, .. }, Command::SetControls { node: b, .. }) => a == b,
        (
            Command::SetControls {
                node: a,
                values: va,
            },
            Command::SetControls {
                node: b,
                values: vb,
            },
        ) => {
            // The same handle, writing the same axes. A different set of keys is a different
            // thing under the pointer, so it opens its own step.
            a == b && va.len() == vb.len() && va.iter().zip(vb).all(|((x, _), (y, _))| x == y)
        }
        // One control written over and over — and the range cleared beside it, so `R` is one
        // undo rather than two.
        (last, cmd) => match (control_target(last), control_target(cmd)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
    }
}

impl Document {
    /// Does this command write into the open gesture's step, rather than opening its own?
    ///
    /// While a knob is turning every continuous write does, whatever it lands on: two knobs
    /// turned together, or a drag while one turns, are one move of the hands on the desk,
    /// and a step per knob per frame empties the ring in seconds. A value the tick writes
    /// joins whatever is open. The hand alone continues only what [`joins`] its last command.
    pub(super) fn continues(&self, cmd: &Command, origin: Origin) -> bool {
        let Gesture { hand, knobs } = &self.gesture;
        coalescable(cmd)
            && match origin {
                Origin::Hand => *knobs || hand.as_ref().is_some_and(|last| joins(last, cmd)),
                Origin::Midi | Origin::Tick => *knobs || hand.is_some(),
            }
    }

    /// What `cmd`, just applied, leaves of the gesture: a command that cannot continue ends
    /// it, and one that can holds it open by its origin.
    pub(super) fn gesture_after(&mut self, cmd: Command, origin: Origin) {
        if !coalescable(&cmd) {
            self.gesture = Gesture::default();
            return;
        }
        match origin {
            Origin::Hand => self.gesture.hand = Some(cmd),
            Origin::Midi => self.gesture.knobs = true,
            Origin::Tick => {}
        }
    }

    /// The pointer let go: the hand no longer holds the gesture open. A knob still turning
    /// does, so a release while one turns does not split its step.
    ///
    /// Without this a gesture would end only when some *other* command interrupted it, and
    /// two scrubs of one control with a release between them would collapse into a single
    /// step that undid both.
    pub(in crate::app) fn end_gesture(&mut self) {
        self.gesture.hand = None;
    }

    /// The knobs have been still for `midi::SETTLE`: they no longer hold the gesture open. A
    /// pointer still down does, so a knob falling silent does not split a scrub.
    pub(in crate::app) fn end_midi_gesture(&mut self) {
        self.gesture.knobs = false;
    }

    /// The command each step in the ring carries, oldest first: the one that opened it, or
    /// the last one its gesture wrote.
    ///
    /// A label per undo step rather than a log that replays: a gesture that wrote a value and
    /// then cleared its range shows only the clearing, and one that wrote two knobs shows the
    /// last write.
    pub(in crate::app) fn history(&self) -> Vec<Command> {
        self.undo.iter().map(|s| s.command.clone()).collect()
    }

    /// What each step is called, named against the graphs on either side of its command: the
    /// steps an undo walks back through, oldest first, and the ones a redo walks forward
    /// through, the next one first. See [`Command::name`].
    ///
    /// The graph before an undo step's command is the one the step holds, and the graph after
    /// it the next step's, or the one on screen; a redo step holds the graph after its
    /// command, and the one before it is the step to be redone ahead of it, or the one on
    /// screen.
    pub(in crate::app) fn step_names(&self) -> (Vec<String>, Vec<String>) {
        let undo = self
            .undo
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let after = self.undo.get(i + 1).map_or(&*self.graph, |s| &*s.graph);
                step.command.name(&[&step.graph, after])
            })
            .collect();
        let redo = self
            .redo
            .iter()
            .enumerate()
            .rev()
            .map(|(i, step)| {
                let before = self.redo.get(i + 1).map_or(&*self.graph, |s| &*s.graph);
                step.command.name(&[before, &step.graph])
            })
            .collect();
        (undo, redo)
    }

    /// What an undo would take back, by name.
    pub(in crate::app) fn undo_name(&self) -> Option<String> {
        let step = self.undo.last()?;
        Some(step.command.name(&[&step.graph, &self.graph]))
    }

    /// What a redo would put back, by name.
    pub(in crate::app) fn redo_name(&self) -> Option<String> {
        let step = self.redo.last()?;
        Some(step.command.name(&[&self.graph, &step.graph]))
    }

    /// The command an undo would take back.
    pub(in crate::app) fn undo_command(&self) -> Option<&Command> {
        self.undo.last().map(|s| &s.command)
    }

    /// The command a redo would put back.
    pub(in crate::app) fn redo_command(&self) -> Option<&Command> {
        self.redo.last().map(|s| &s.command)
    }

    pub(in crate::app) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(in crate::app) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps the undo ring is holding, which is what the two caps bound.
    pub(in crate::app) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub(in crate::app) fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Step back one edit. The graph on screen is replaced whole, so the caller rebuilds
    /// whatever it derived from it.
    pub(in crate::app) fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.gesture = Gesture::default();
        let restored = self.restore(step.graph);
        self.redo
            .push(Step::new(restored, &self.graph, step.command, self.edit));
        self.edit = step.serial;
        true
    }

    /// Step forward one edit, on the same terms as [`Self::undo`].
    pub(in crate::app) fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        self.gesture = Gesture::default();
        let restored = self.restore(step.graph);
        self.undo
            .push(Step::new(restored, &self.graph, step.command, self.edit));
        self.edit = step.serial;
        true
    }

    /// Drop the open gesture's step, where the hand is scrubbing this control, and restore
    /// the graph and serial it holds — undoing the drag without offering it as a redo. False
    /// where the hand is not. See `App::cancel_control_drag`.
    ///
    /// The step goes whole: whatever a knob or the tick wrote into it goes with the scrub,
    /// exactly as an undo of it would take it.
    pub(in crate::app) fn cancel_scrub(&mut self, node: NodeId, key: &'static str) -> bool {
        let mine = matches!(
            self.gesture.hand,
            Some(Command::SetControl { node: n, key: k, .. }) if n == node && k == key
        );
        if !mine {
            return false;
        }
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.gesture = Gesture::default();
        self.restore(step.graph);
        self.edit = step.serial;
        true
    }

    /// Drop the open gesture's step, where the hand is dragging exactly these nodes, and
    /// restore the graph and serial it holds: a node drag put back with `Escape`, on the
    /// terms [`Self::cancel_scrub`] gives a scrub. False where the hand is not, which is also
    /// a drag that has not moved anything yet.
    pub(in crate::app) fn cancel_move(&mut self, nodes: &[NodeId]) -> bool {
        let mine = matches!(
            &self.gesture.hand,
            Some(Command::MoveNodes { moves })
                if moves.len() == nodes.len() && moves.iter().zip(nodes).all(|((a, _), b)| a == b)
        );
        if !mine {
            return false;
        }
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.gesture = Gesture::default();
        self.restore(step.graph);
        self.edit = step.serial;
        true
    }

    /// Put a step's graph on screen, and hand back the one it replaces.
    ///
    /// **The id counters are not rewound.** The restored graph takes the higher of its own
    /// and the replaced graph's, so Add, Undo, Add hands out two ids rather than one id twice
    /// — a reissued `NodeId` would be named by a command in the log for another node, and a
    /// reissued `WorkspaceId` names another tab's file and picture.
    fn restore(&mut self, graph: Arc<Graph>) -> Arc<Graph> {
        let replaced = std::mem::replace(&mut self.graph, graph);
        Graph::carry_counters(&mut self.graph, &replaced);
        self.shape += 1;
        replaced
    }

    /// Record one undoable step and enforce the two caps on the ring.
    pub(super) fn push_undo(&mut self, step: Step) {
        self.undo.push(step);
        self.redo.clear();
        self.trim();
    }

    /// The gesture in progress wrote another frame: the command its step carries becomes
    /// this one, rather than a sixtieth step. The step's snapshot stays the graph before the
    /// gesture.
    pub(super) fn continue_step(&mut self, command: Command) {
        if let Some(step) = self.undo.last_mut() {
            step.command = command;
        }
    }

    /// Enforce the two caps on the ring.
    ///
    /// Oldest first, by bytes and then by depth. The newest step is never dropped, so one
    /// edit to a graph larger than the whole budget is still undoable once.
    fn trim(&mut self) {
        let mut excess = self.undo.len().saturating_sub(UNDO_DEPTH);
        let mut bytes: usize = self.undo[excess..].iter().map(|s| s.bytes).sum();
        while bytes > UNDO_BYTES && excess + 1 < self.undo.len() {
            bytes -= self.undo[excess].bytes;
            excess += 1;
        }
        if excess > 0 {
            self.undo.drain(..excess);
        }
    }

    /// Forget every edit. Opening or making a project, and nothing else.
    ///
    /// **Opening a project is not an undo step.** Undoing your way out of one project into
    /// the previous one is not a thing anyone wants at a show, and a snapshot from before
    /// the switch describes nodes the project on screen never had.
    pub(super) fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.gesture = Gesture::default();
        self.edit = 0;
        self.next_edit = 0;
        self.saved_edit = 0;
    }
}
