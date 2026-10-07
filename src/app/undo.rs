// SPDX-License-Identifier: AGPL-3.0-or-later

//! Undo and redo by name, from the app's side: the toast that says what an undo changed and
//! offers to go there, and the Undo History's walk to any step in the ring.
//!
//! Every step already carries the command that made it (`document::history`), so a name is
//! read off that command against the graphs on either side of it — `Command::name` — and
//! nothing new is kept. The Edit menu, `Ctrl`+Z and the native bar all reach an undo through
//! `App::handle_menu`, which is the one place that says what it did; a test calling
//! [`App::undo`] directly gets the undo and no toast. See docs/ui.md, "Undo by name".

use super::{App, Go};
use crate::command::Subject;

impl App {
    /// What an undo would take back, by name: the Edit menu's *Undo Delete 3 nodes*.
    pub fn undo_name(&self) -> Option<String> {
        self.doc.undo_name()
    }

    /// What a redo would put back, by name.
    pub fn redo_name(&self) -> Option<String> {
        self.doc.redo_name()
    }

    /// Every step in the ring by name: what can be undone, oldest first, and what can be
    /// redone, the next one first. The Undo History's rows.
    pub fn step_names(&self) -> (Vec<String>, Vec<String>) {
        self.doc.step_names()
    }

    /// Undo or redo until `kept` steps are left to undo: the Undo History's click. Zero is as
    /// far back as the ring goes, and the undo steps plus the redo steps as far forward.
    ///
    /// The graph is put on screen once, at the end, rather than once per step walked. Says
    /// what changed, as one undo does, and answers whether anything did.
    pub fn travel_to(&mut self, kept: usize) -> bool {
        // The document is closed while a render reads it, an undo as much as an edit.
        if self.rendering() {
            return false;
        }
        let mut undid = 0_usize;
        let mut redid = 0_usize;
        let mut last = None;
        while self.doc.undo_len() > kept {
            let name = self.doc.undo_name();
            let subject = self.doc.undo_command().map(crate::Command::subject);
            if !self.doc.undo() {
                break;
            }
            undid += 1;
            last = name.zip(subject);
        }
        while self.doc.undo_len() < kept {
            let name = self.doc.redo_name();
            let subject = self.doc.redo_command().map(crate::Command::subject);
            if !self.doc.redo() {
                break;
            }
            redid += 1;
            last = name.zip(subject);
        }
        let Some((name, subject)) = last else {
            return false;
        };
        self.after_time_travel();
        let steps = undid.max(redid);
        let verb = if undid > 0 { "Undid" } else { "Redid" };
        let said = if steps == 1 {
            format!("{verb} {name}")
        } else {
            format!("{verb} {steps} steps, through {name}")
        };
        let go = self.go_for(&subject);
        self.say_with_go(said, go);
        true
    }

    /// One undo, and a toast saying what it took back. What the Edit menu and `Ctrl`+Z do.
    pub(super) fn undo_and_say(&mut self) {
        let kept = self.doc.undo_len();
        if kept > 0 {
            self.travel_to(kept - 1);
        }
    }

    /// One redo, and a toast saying what it put back.
    pub(super) fn redo_and_say(&mut self) {
        if self.doc.can_redo() {
            self.travel_to(self.doc.undo_len() + 1);
        }
    }

    /// Where a toast about this change offers to go, or `None` where the change is already
    /// in view or is nowhere: the first node it names that is still in the graph, on the
    /// workspace a link to it would open, and otherwise the workspace it names.
    fn go_for(&self, subject: &Subject) -> Option<Go> {
        let graph = self.doc.graph();
        let shown = self.active_workspace();
        if let Some(node) = subject
            .nodes
            .iter()
            .copied()
            .find(|id| graph.get(*id).is_some())
        {
            let home = self.home_of(node)?;
            return (shown != Some(home) || !self.node_in_view(node))
                .then_some(Go::Node(home, node));
        }
        let workspace = subject.workspace.filter(|w| graph.has_workspace(*w))?;
        (shown != Some(workspace)).then_some(Go::Workspace(workspace))
    }

    /// Whether any of this node is inside the canvas as it was last drawn.
    fn node_in_view(&self, node: crate::graph::NodeId) -> bool {
        let graph = self.doc.graph();
        let Some(rect) = crate::ui::canvas::Layouts::one(graph, node)
            .find(node)
            .map(|l| l.rect)
        else {
            return false;
        };
        let origin = self.canvas.origin();
        let transform = self.canvas.transform();
        let on_screen = emath::Rect::from_two_pos(
            transform.to_screen(origin, rect.min),
            transform.to_screen(origin, rect.max),
        );
        let canvas = emath::Rect::from_min_size(
            origin,
            emath::vec2(self.canvas.width(), self.canvas.height()),
        );
        canvas.intersects(on_screen)
    }

    /// Edit ▸ Undo History…, while it is open. A click on a row walks the ring to it, after
    /// the window has drawn.
    pub(super) fn show_undo_history(&mut self, ctx: &eframe::egui::Context) {
        let Some(mut state) = self.undo_history.take() else {
            return;
        };
        let (undo, redo) = self.doc.step_names();
        let view = crate::ui::history::HistoryView {
            undo: &undo,
            redo: &redo,
        };
        let actions = crate::ui::history::show(
            ctx,
            &mut state,
            &view,
            &self.theme,
            &self.prefs.get().windows,
        );
        let mut open = true;
        for action in actions {
            match action {
                crate::ui::history::HistoryAction::GoTo(kept) => {
                    self.travel_to(kept);
                }
                crate::ui::history::HistoryAction::Close => open = false,
            }
        }
        if open {
            self.undo_history = Some(state);
        }
    }

    /// Open Edit ▸ Undo History…, scrolled to the step on screen.
    pub(super) fn open_undo_history(&mut self) {
        self.undo_history.get_or_insert_default();
    }

    /// Take the view where a toast's ▸ go points.
    pub(super) fn go(&mut self, to: Go) {
        match to {
            Go::Node(workspace, node) => self.navigate_to(workspace, node),
            Go::Workspace(workspace) => self.open_workspace(workspace),
        }
    }
}
