// SPDX-License-Identifier: AGPL-3.0-or-later

//! Moving between workspaces and making one from another: `Ctrl+Tab`, the duplicate.
//!
//! Switching is session state and never an edit, as everywhere else on the bar; the
//! duplicate is the one edit here, a single `Command::DuplicateWorkspace`, and showing the
//! copy afterwards is session state again.

use super::App;
use crate::command::Command;
use crate::graph::WorkspaceId;
use eframe::egui;

/// `<name> copy`, or `<name> copy 2` and up where that is taken: a tab's name is how a
/// person says which one they mean, so two never share one.
pub fn copy_name(name: &str, taken: &std::collections::HashSet<&str>) -> String {
    let first = format!("{name} copy");
    if !taken.contains(first.as_str()) {
        return first;
    }
    // One more candidate than there are names taken, so one of them is free.
    (2..=taken.len() + 2)
        .map(|n| format!("{name} copy {n}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or(first)
}

impl App {
    /// `Ctrl+Tab` and `Ctrl+Shift+Tab`: the next tab along the bar and the one before,
    /// wrapping at the ends, over the project tab and every open workspace in the order
    /// `Ctrl+1..9` counts them.
    ///
    /// `Ctrl` rather than the platform's command key on a Mac too, as every browser and editor
    /// binds it. Never while another of our windows has the keyboard; a text field does not
    /// take `Ctrl+Tab`, so the editor's own focus is the whole of the guard. The longer chord is
    /// consumed first, because `Ctrl+Tab` matches a press with `Shift` held as well.
    pub(super) fn switch_tabs_by_key(&mut self, ui: &egui::Ui) {
        if !ui.input(|i| i.viewport().focused.unwrap_or(true)) {
            return;
        }
        let (back, forth) = ui.input_mut(|i| {
            (
                i.consume_shortcut(&crate::ui::menu::keys::PREVIOUS_TAB),
                i.consume_shortcut(&crate::ui::menu::keys::NEXT_TAB),
            )
        });
        if back {
            self.step_tab(false);
        } else if forth {
            self.step_tab(true);
        }
    }

    /// Show the tab after the one showing, or before it, wrapping round.
    pub fn step_tab(&mut self, forward: bool) {
        let order = self.tab_bar().order();
        let at = order.iter().position(|t| *t == self.active()).unwrap_or(0);
        let n = order.len();
        let to = if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        };
        self.activate(order[to]);
    }

    /// Copy a workspace whole, beside it, as one undo step; then open the copy and show it,
    /// looking where the original was left.
    pub fn duplicate_workspace(&mut self, id: WorkspaceId) {
        let graph = self.doc.graph();
        let Some(at) = graph.workspaces().iter().position(|w| w.id == id) else {
            return;
        };
        let name = {
            let taken = graph.workspaces().iter().map(|w| w.name.as_str()).collect();
            copy_name(&graph.workspaces()[at].name, &taken)
        };
        if let Err(err) = self.apply(Command::DuplicateWorkspace { id, name }) {
            log::debug!("duplicate refused: {err}");
            return;
        }
        // The command puts the copy straight after the original.
        let Some(copy) = self.doc.graph().workspaces().get(at + 1).map(|w| w.id) else {
            return;
        };
        self.stash_view();
        if let Some(view) = self.project.session().views.get(&id).copied() {
            self.project.session_mut().views.insert(copy, view);
        }
        self.open_workspace(copy);
    }
}

#[cfg(test)]
mod tests {
    use super::copy_name;

    #[test]
    fn a_copy_counts_up_past_the_names_taken() {
        let taken = ["Tunnel", "Tunnel copy", "Tunnel copy 2"]
            .into_iter()
            .collect();
        assert_eq!(copy_name("Tunnel", &taken), "Tunnel copy 3");
        assert_eq!(copy_name("Feedback", &taken), "Feedback copy");
    }
}
