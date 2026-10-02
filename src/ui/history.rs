// SPDX-License-Identifier: AGPL-3.0-or-later

//! Edit ▸ Undo History…: the undo ring's steps by name, and a click that goes to one.
//!
//! An `egui::Window` of ordinary rows, like About, of one fixed size whatever the ring holds:
//! row 0 is as far back as undo goes, then every step that can be undone, oldest first, the
//! one the graph on screen stands at marked as chosen, then every step that can be redone,
//! dimmed. Rows are one height and truncate rather than wrap, and only those in view are laid
//! out, so a full ring of 256 costs what a short one does. It draws and returns; `App` walks
//! the ring. docs/ui.md, "Undo by name", has the rest.

use crate::ui::theme::Theme;
use eframe::egui::{self, Atom, Button, Context, RichText, ScrollArea, Window, vec2};

/// What the window draws: the names `App::step_names` reads off the ring.
pub struct HistoryView<'a> {
    /// What can be undone, oldest first. The last is where the graph on screen stands.
    pub undo: &'a [String],
    /// What can be redone, the next one first.
    pub redo: &'a [String],
}

/// What the window asks for. It mutates nothing itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    /// Undo or redo until this many steps are left to undo: the row clicked.
    GoTo(usize),
    /// The window's close button.
    Close,
}

/// The window's state across frames.
#[derive(Debug, Default)]
pub struct HistoryState {
    /// The list has been scrolled to the step on screen, which it is once, when it opens.
    scrolled: bool,
}

/// What row 0 says: the graph before the oldest step the ring keeps.
pub const START: &str = "Start";

/// The Undo History window.
pub fn show(
    ctx: &Context,
    state: &mut HistoryState,
    view: &HistoryView<'_>,
    theme: &Theme,
    saved: &crate::preferences::Placements,
) -> Vec<HistoryAction> {
    let mut actions = Vec::new();
    let mut open = true;
    let window = Window::new(super::placed::UNDO_HISTORY)
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .fixed_size(vec2(320.0, 360.0))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    let here = view.undo.len();
    let rows = 1 + view.undo.len() + view.redo.len();
    super::placed::place(ctx, window, super::placed::UNDO_HISTORY, saved).show(ctx, |ui| {
        let row_height = ui.spacing().interact_size.y;
        let pitch = row_height + ui.spacing().item_spacing.y;
        let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
        if !state.scrolled {
            // The step on screen in the middle of the list, the first time it is seen.
            scroll = scroll.vertical_scroll_offset((here as f32 * pitch - 160.0).max(0.0));
            state.scrolled = true;
        }
        scroll.show_rows(ui, row_height, rows, |ui, range| {
            let width = ui.available_width();
            for row in range {
                let name = match row {
                    0 => START,
                    r if r <= here => view.undo[r - 1].as_str(),
                    r => view.redo[r - here - 1].as_str(),
                };
                let text = RichText::new(format!("{row}  {name}"));
                // What a redo would put back is not what is on screen, so it reads dimmer.
                let text = if row > here {
                    text.color(theme.text_muted())
                } else {
                    text
                };
                let response = ui
                    .add(
                        Button::selectable(row == here, (text, Atom::grow()))
                            .truncate()
                            .min_size(vec2(width, row_height)),
                    )
                    .on_hover_text(match row {
                        0 => "As far back as undo goes",
                        r if r == here => "Where the graph on screen stands",
                        r if r < here => "Undo back to here",
                        _ => "Redo forward to here",
                    });
                if response.clicked() && row != here {
                    actions.push(HistoryAction::GoTo(row));
                }
            }
        });
    });
    if !open {
        actions.push(HistoryAction::Close);
    }
    actions
}
