// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document: the graph, the undo ring, the edit serials and the clipboard.
//!
//! [`Document::apply`] is the one door into the graph. It checks a command, writes it, keeps
//! the undo step, and reports what it did — which Outputs it made stale, which nodes it took
//! away, what it planted — as an [`apply::Applied`], and does nothing about any of it: rebuilding a
//! shader, republishing the graph, the selection and the tabs belong to the parts that own
//! them, and `App::apply` is the one place that tells each. Undo is a snapshot rather than
//! an inverse, so no command can have a wrong undo: see [`history`].

mod apply;
mod history;

pub(super) use apply::Env;
pub(super) use history::{Origin, UNDO_DEPTH};

use crate::command::Clip;
use crate::graph::Graph;
use emath::Vec2;
use history::{Gesture, Step};
use std::sync::Arc;

/// What a copy left behind, and where on screen it was when it was taken.
///
/// **Session state.** A clipboard is not the document: copying changes no graph state, so it
/// is not a command, it is not undoable, and it is not in the file. It is not the system
/// clipboard either — what is on it is a piece of graph, which nothing outside this process
/// could read — so it lives and dies with the run.
#[derive(Debug, Default)]
pub(super) struct Clipboard {
    pub(super) clip: Clip,
    /// The clip's anchor as it sat in the window, in screen points from the canvas origin.
    ///
    /// Screen points rather than world units, because an unaimed paste lands where the copy
    /// visually *was*: the same spot in the window, whatever the zoom and the pan have done
    /// since, and whichever workspace is being looked at. A world offset would instead put
    /// the clip at whatever coordinate another workspace happens to have there.
    pub(super) anchor: Vec2,
}

#[derive(Debug, Default)]
pub(super) struct Document {
    /// The graph on screen. Shared with the synth and with every undo step, and copied on
    /// write: see [`Document::graph_mut`].
    graph: Arc<Graph>,
    /// States to go back to, oldest first, capped at `UNDO_DEPTH`. What
    /// [`Document::history`] lists, one command per step.
    undo: Vec<Step>,
    /// States undone away from, most recent last. Cleared by any new edit.
    redo: Vec<Step>,
    /// The gesture in progress, whose step is the one on top of `undo`: see
    /// [`history::Gesture`].
    gesture: Gesture,
    /// The serial of the graph on screen, and the one the last save wrote.
    ///
    /// A fresh serial per edit, restored by undo and redo from the step it came from, is
    /// what makes `dirty` a comparison of two integers rather than a second copy of the
    /// graph — and it answers *false* after undoing back to the state that was saved.
    edit: u64,
    next_edit: u64,
    saved_edit: u64,
    /// Counts every change to what the render plan reads off the graph: a command that
    /// [reshapes](crate::command::Command::reshapes) it, and every graph restored or replaced whole. Never
    /// restored by undo, so two different graphs never share a number.
    shape: u64,
    /// What a copy took, until the next one replaces it.
    clipboard: Clipboard,
}

impl Document {
    pub(super) fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The graph as the handle the synth and the undo ring share.
    pub(super) fn shared(&self) -> &Arc<Graph> {
        &self.graph
    }

    /// The graph to write, for an edit. Where the synth or an undo step still holds the
    /// graph on screen, the first write copies it — pointer bumps, see `Graph::clone` — and
    /// every write after it copies the node it lands on and nothing else.
    fn graph_mut(&mut self) -> &mut Graph {
        Arc::make_mut(&mut self.graph)
    }

    /// Put a whole new graph on screen with no history behind it: a project opened or made.
    pub(super) fn replace(&mut self, graph: Graph) {
        self.graph = Arc::new(graph);
        self.reset_history();
        self.shape += 1;
    }

    /// See the field. The plan compares it against the one it was last built on.
    pub(super) fn shape(&self) -> u64 {
        self.shape
    }

    /// Has anything changed since the last save?
    ///
    /// Two integers rather than a comparison against a copy of the saved graph: every edit
    /// takes a fresh serial and undo restores the one its step carried, so undoing back to
    /// what was saved answers *no* without anything having to diff a graph.
    pub(super) fn dirty(&self) -> bool {
        self.edit != self.saved_edit
    }

    /// The graph on screen is the one on disk.
    pub(super) fn mark_saved(&mut self) {
        self.saved_edit = self.edit;
    }

    /// The graph on screen is not the one on disk, though no edit made it: a recovered
    /// autosave. A serial of its own, so it reads as unsaved until the next save.
    pub(super) fn mark_unsaved(&mut self) {
        self.next_edit += 1;
        self.edit = self.next_edit;
    }

    /// The serial of the graph on screen: a different number whenever the graph is.
    pub(super) fn edit(&self) -> u64 {
        self.edit
    }

    pub(super) fn clipboard(&self) -> &Clipboard {
        &self.clipboard
    }

    pub(super) fn set_clipboard(&mut self, clipboard: Clipboard) {
        self.clipboard = clipboard;
    }
}
