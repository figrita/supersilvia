// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tab bar: one tab per open workspace, and the pinned project tab at the far left.
//!
//! Like [`crate::ui::show`] and [`crate::ui::menu::show`], this draws and returns what the
//! interaction asked for; it never mutates. Opening, closing and switching are not commands —
//! they are session state, saved in the project file and never in the undo history — so what
//! `App` does with a [`TabAction`] is set a field rather than push an undo step.
//!
//! **Every tab is a real `Response` with `widget_info`**, named `tab project` and
//! `tab {workspace name}`, so `Ctrl+2` and a click on a tab are the same gesture to a test
//! and to the agent-driven layer.
//!
//! **The bar never runs off the window.** The tabs that fit are drawn in project order, the
//! one showing always among them, and the [`OVERFLOW`] list at the end names every workspace
//! in the project — the closed ones dimmed — so a tab that did not fit is one click away.

use crate::graph::{Workspace, WorkspaceId, WorkspaceKind};
use crate::project::Active;
use crate::ui::theme::{self, Theme};
use eframe::egui::{FontId, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::BTreeSet;

/// How tall a tab is, and therefore how tall the bar is.
pub const TAB_HEIGHT: f32 = 24.0;
/// Padding either side of a tab's label.
const TAB_PAD: f32 = 10.0;
/// Space between two tabs, so the strip reads as tabs rather than as one bar.
const TAB_GAP: f32 = 2.0;
/// The widest a tab gets, however long the name is. A long name is elided.
const TAB_MAX: f32 = 180.0;
/// How wide the inline rename editor is.
const RENAME_WIDTH: f32 = 140.0;
/// The list at the end of the bar of every workspace in the project.
pub const OVERFLOW: &str = "▾";
/// What the bar keeps free past the last tab, for the `+` and the [`OVERFLOW`] list.
const TRAILING: f32 = 64.0;
/// How many closed workspaces Reopen remembers, most recent last.
const CLOSED_KEPT: usize = 16;

/// What the tab bar asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabAction {
    /// Show this tab.
    Activate(Active),
    /// Add a workspace of this kind, open it and show it.
    Add(WorkspaceKind),
    /// Close a workspace: it keeps its nodes and loses its tab.
    Close(WorkspaceId),
    /// Give a workspace a tab, if it had none, and show it: a row of the overflow list, and
    /// Reopen closed workspace.
    Open(WorkspaceId),
    /// Copy a workspace whole, beside it, as one undo step.
    Duplicate(WorkspaceId),
    /// The one entry here that *is* an edit, so `App` sends it to the bus.
    Rename { id: WorkspaceId, name: String },
    /// A node drag is over this tab. A report rather than a request: the canvas is what
    /// knows when the pointer let go, so `App` joins the two and emits `ShowOn`. The
    /// project tab never reports one, because a node cannot be shown on it.
    DropTarget(WorkspaceId),
}

/// The rename in progress, which is the tab bar's own state.
///
/// Session state the way `CanvasState` is: it is not in any file, it is not undoable, and it
/// is why `ui/` may write it directly where it may not write the graph.
#[derive(Debug, Default)]
pub struct TabState {
    /// The workspace whose name is being edited inline, if one is.
    renaming: Option<Renaming>,
    /// Workspaces whose tabs were closed, the most recent last: what Reopen closed workspace
    /// gives back. Session state, so it goes with the project it was kept for.
    closed: Vec<WorkspaceId>,
}

/// A rename in progress: which workspace, the text so far, and whether the editor has been
/// given the keyboard yet. Focus is asked for once — asking every frame would take it back
/// from whatever the user clicked on to end the rename.
#[derive(Debug)]
struct Renaming {
    id: WorkspaceId,
    text: String,
    focused: bool,
}

impl TabState {
    /// Start renaming a workspace inline. The Workspace menu's Rename… asks for this rather
    /// than putting up a dialog, so there is one rename gesture and it is on the tab.
    pub fn rename(&mut self, id: WorkspaceId, from: &str) {
        self.renaming = Some(Renaming {
            id,
            text: from.to_string(),
            focused: false,
        });
    }

    /// Which workspace is being renamed, if any.
    pub fn renaming(&self) -> Option<WorkspaceId> {
        self.renaming.as_ref().map(|r| r.id)
    }

    /// A workspace's tab was closed, so Reopen closed workspace can give it back.
    pub fn closed(&mut self, id: WorkspaceId) {
        self.closed.retain(|w| *w != id);
        self.closed.push(id);
        if self.closed.len() > CLOSED_KEPT {
            self.closed.remove(0);
        }
    }

    /// The workspace Reopen closed workspace gives back: the most recently closed that is
    /// still in the project and still has no tab. One deleted since, or opened again some
    /// other way, is passed over.
    pub fn to_reopen(&self, bar: &TabBar<'_>) -> Option<WorkspaceId> {
        self.closed
            .iter()
            .rev()
            .copied()
            .find(|id| !bar.open.contains(id) && bar.workspaces.iter().any(|w| w.id == *id))
    }
}

/// Everything the bar needs to draw itself, read-only.
pub struct TabBar<'a> {
    /// Every workspace in the project, in project order. The bar draws the open ones.
    pub workspaces: &'a [Workspace],
    pub open: &'a BTreeSet<WorkspaceId>,
    pub active: Active,
    /// A node is being dragged on the canvas, as it reported last frame. A tab under the
    /// pointer lights up while it is, because dropping one there is the gesture people use.
    pub dragging: bool,
    /// Width the caller keeps free at the bar's right end, for what it draws there.
    pub reserve: f32,
}

impl TabBar<'_> {
    /// The tabs in order: the project tab, then one per open workspace in project order.
    ///
    /// `Ctrl+1..9` indexes exactly this, which is what makes the shortcut and the click the
    /// same gesture.
    pub fn order(&self) -> Vec<Active> {
        std::iter::once(Active::Project)
            .chain(
                self.workspaces
                    .iter()
                    .filter(|w| self.open.contains(&w.id))
                    .map(|w| Active::Workspace(w.id)),
            )
            .collect()
    }
}

/// Draw one tab and report what it was asked to do.
///
/// Hand-painted rather than an `egui::Button`, for the same reason every control on the
/// canvas is: the widget carries the name the tree sees, which is the tab's identity, not
/// whatever text happens to fit in it.
fn tab(
    ui: &mut Ui,
    label: &str,
    name: &str,
    active: bool,
    dragging: bool,
    theme: &Theme,
) -> eframe::egui::Response {
    let galley = tab_galley(ui, label, theme);
    let width = (galley.size().x + TAB_PAD * 2.0).min(TAB_MAX);
    let (rect, response) = ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::click());
    let owned = name.to_string();
    response.widget_info(|| {
        eframe::egui::WidgetInfo::selected(eframe::egui::WidgetType::Button, true, active, &owned)
    });

    // `contains_pointer`, not `hovered`: egui takes hover away from every other widget while
    // something is being dragged, and a node drag over a tab is exactly that case.
    let offered = dragging && response.contains_pointer();
    let hovered = response.hovered();
    let fill = if offered {
        theme.bg_active()
    } else if active {
        theme.bg_tertiary()
    } else if hovered {
        theme.bg_secondary()
    } else {
        theme.bg_sunken()
    };
    // One radius for the fill and the border both: a tab is round at the top and square
    // where it meets the canvas, and a border rounded on a corner the fill left square
    // leaves the fill outside the line.
    let corner = eframe::egui::CornerRadius {
        nw: theme::RADIUS_SM,
        ne: theme::RADIUS_SM,
        sw: 0,
        se: 0,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, corner, fill);
    // The active tab is joined to the canvas below it by a lit top edge, which is what says
    // "this one" without a second color. Held in by the corner radius at each end, so it
    // stops where the arc starts rather than squaring the two corners off again.
    if active {
        let inset = f32::from(theme::RADIUS_SM);
        painter.line_segment(
            [
                pos2(rect.min.x + inset, rect.min.y + 1.0),
                pos2(rect.max.x - inset, rect.min.y + 1.0),
            ],
            Stroke::new(2.0, theme.primary()),
        );
    }
    painter.rect_stroke(
        rect,
        corner,
        if offered {
            Stroke::new(2.0, theme.primary())
        } else {
            Stroke::new(1.0, theme.border_subtle())
        },
        StrokeKind::Inside,
    );
    painter.galley(
        pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        if active {
            theme.text_primary()
        } else {
            theme.text_secondary()
        },
    );
    response
}

/// A tab's label, laid out as the tab draws it.
fn tab_galley(ui: &Ui, label: &str, theme: &Theme) -> std::sync::Arc<eframe::egui::Galley> {
    ui.painter().layout(
        label.to_string(),
        FontId::monospace(theme::FONT_BASE),
        theme.text_primary(),
        TAB_MAX - TAB_PAD * 2.0,
    )
}

/// How wide a tab is drawn, as [`tab`] draws it.
fn tab_width(ui: &Ui, label: &str, theme: &Theme) -> f32 {
    (tab_galley(ui, label, theme).size().x + TAB_PAD * 2.0).min(TAB_MAX)
}

/// Which of the open tabs fit in `room`, in project order, `active` always among them.
///
/// Every tab that fits from the front, and where the one showing is not among those, the
/// tail is given up until it fits after them. A tab that is not drawn is in the overflow
/// list, so nothing is out of reach; the one showing must be on the bar, because the lit tab
/// is what says which canvas this is.
pub fn fitting(
    widths: &[(WorkspaceId, f32)],
    room: f32,
    active: Option<WorkspaceId>,
) -> Vec<WorkspaceId> {
    let cost = |id: WorkspaceId| {
        widths
            .iter()
            .find(|(w, _)| *w == id)
            .map_or(0.0, |(_, width)| width + TAB_GAP)
    };
    let mut used = 0.0;
    let mut shown = Vec::new();
    for (id, width) in widths {
        if used + width > room {
            break;
        }
        used += width + TAB_GAP;
        shown.push(*id);
    }
    let Some(active) = active.filter(|a| !shown.contains(a) && cost(*a) > 0.0) else {
        return shown;
    };
    while used + cost(active) - TAB_GAP > room
        && let Some(last) = shown.pop()
    {
        used -= cost(last);
    }
    shown.push(active);
    shown
}

/// Reopen closed workspace, on every tab's menu. Greyed while there is none to give back.
fn reopen_entry(ui: &mut Ui, state: &TabState, bar: &TabBar<'_>, actions: &mut Vec<TabAction>) {
    let reopen = state.to_reopen(bar);
    let entry = ui.add_enabled(
        reopen.is_some(),
        eframe::egui::Button::new("Reopen closed workspace"),
    );
    let entry = match reopen.and_then(|id| bar.workspaces.iter().find(|w| w.id == id)) {
        Some(w) => entry.on_hover_text(w.name.as_str()),
        None => entry,
    };
    if entry.clicked()
        && let Some(id) = reopen
    {
        actions.push(TabAction::Open(id));
        ui.close();
    }
}

/// The inline rename editor, wherever it is drawn.
///
/// Enter and clicking away both commit — egui's single-line editor drops focus on Enter, so
/// one test covers both — and Escape drops it. A rename that vanished because the pointer
/// moved would be the worst of the three.
fn rename_editor(ui: &mut Ui, state: &mut TabState, width: f32) -> Option<TabAction> {
    let renaming = state.renaming.as_mut()?;
    let editor = ui.add(
        eframe::egui::TextEdit::singleline(&mut renaming.text)
            .desired_width(width)
            .id(ui.id().with(("rename tab", renaming.id))),
    );
    if !renaming.focused {
        editor.request_focus();
        renaming.focused = true;
    }
    if ui.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
        state.renaming = None;
        return None;
    }
    if !editor.lost_focus() {
        return None;
    }
    let done = state.renaming.take()?;
    let name = done.text.trim();
    (!name.is_empty()).then(|| TabAction::Rename {
        id: done.id,
        name: name.to_string(),
    })
}

/// Draw the bar and return every action the interaction asked for.
pub fn show(ui: &mut Ui, state: &mut TabState, bar: &TabBar<'_>, theme: &Theme) -> Vec<TabAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = TAB_GAP;

        // Which tabs fit, worked out before any is drawn, so the one showing is never the one
        // pushed off the end.
        let project_label = "◫ Project";
        let room = ui.available_width()
            - bar.reserve
            - TRAILING
            - tab_width(ui, project_label, theme)
            - TAB_GAP;
        let widths: Vec<(WorkspaceId, f32)> = bar
            .workspaces
            .iter()
            .filter(|w| bar.open.contains(&w.id))
            .map(|w| {
                let width = if state.renaming() == Some(w.id) {
                    RENAME_WIDTH
                } else {
                    tab_width(ui, &w.name, theme)
                };
                (w.id, width)
            })
            .collect();
        let active = match bar.active {
            Active::Workspace(id) => Some(id),
            Active::Project => None,
        };
        let shown = fitting(&widths, room, active);

        // Pinned, leftmost, and the one tab that is not a workspace and cannot be closed.
        // A node cannot be shown on the project tab, so it never lights up and never offers
        // itself as a drop.
        let project = tab(
            ui,
            project_label,
            "tab project",
            bar.active == Active::Project,
            false,
            theme,
        );
        if project.clicked() {
            actions.push(TabAction::Activate(Active::Project));
        }
        // With every tab closed it is the only tab there is, so the way back is here too.
        project.context_menu(|ui| reopen_entry(ui, state, bar, &mut actions));

        for id in shown {
            let Some(workspace) = bar.workspaces.iter().find(|w| w.id == id) else {
                continue;
            };
            if state.renaming() == Some(id) {
                if let Some(action) = rename_editor(ui, state, RENAME_WIDTH) {
                    actions.push(action);
                }
                continue;
            }

            let response = tab(
                ui,
                &workspace.name,
                &format!("tab {}", workspace.name),
                bar.active == Active::Workspace(id),
                bar.dragging,
                theme,
            );
            if bar.dragging && response.contains_pointer() {
                actions.push(TabAction::DropTarget(id));
            }
            if response.clicked() {
                actions.push(TabAction::Activate(Active::Workspace(id)));
            }
            if response.double_clicked() {
                state.rename(id, &workspace.name);
            }
            response.context_menu(|ui| {
                if ui.button("Rename").clicked() {
                    state.rename(id, &workspace.name);
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    actions.push(TabAction::Duplicate(id));
                    ui.close();
                }
                if ui.button("Close").clicked() {
                    actions.push(TabAction::Close(id));
                    ui.close();
                }
                ui.separator();
                reopen_entry(ui, state, bar, &mut actions);
            });
        }

        // One entry per kind that has a UI, which is one today. A menu rather than a plain
        // button, because the second kind is a proposal away and the gesture should not move.
        ui.menu_button("+", |ui| {
            for kind in [WorkspaceKind::Video] {
                if ui.button(label_of(kind)).clicked() {
                    actions.push(TabAction::Add(kind));
                    ui.close();
                }
            }
        });

        // Every workspace in project order, open or not, the closed ones dimmed: the tabs
        // that did not fit, and the ones that have no tab, in one list.
        let list = ui.menu_button(OVERFLOW, |ui| {
            for workspace in bar.workspaces {
                let open = bar.open.contains(&workspace.id);
                let text = eframe::egui::RichText::new(workspace.name.as_str()).color(if open {
                    theme.text_primary()
                } else {
                    theme.text_muted()
                });
                let row = ui.selectable_label(bar.active == Active::Workspace(workspace.id), text);
                crate::ui::accessible(
                    &row,
                    eframe::egui::WidgetType::Button,
                    format_args!("show {}", workspace.name),
                );
                let row = if open {
                    row
                } else {
                    row.on_hover_text("closed: opens a tab")
                };
                if row.clicked() {
                    actions.push(TabAction::Open(workspace.id));
                    ui.close();
                }
            }
        });
        crate::ui::accessible(
            &list.response,
            eframe::egui::WidgetType::Button,
            "all workspaces",
        );
    });
    actions
}

/// What a kind is called in the menus. One arm per `WorkspaceKind`, so a new kind is a
/// compile error here rather than a tab with no name.
pub fn label_of(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Video => "Video",
    }
}

#[cfg(test)]
mod tests {
    use super::{TAB_GAP, fitting};
    use crate::graph::WorkspaceId;

    /// Five tabs of 100 with room for three: the first three, and where the one showing is
    /// further along, it takes the last place.
    #[test]
    fn the_tabs_that_fit_keep_the_one_showing() {
        let widths: Vec<(WorkspaceId, f32)> = (1..=5).map(|i| (WorkspaceId(i), 100.0)).collect();
        let room = 3.0 * 100.0 + 2.0 * TAB_GAP;
        let ids = |v: Vec<WorkspaceId>| v.into_iter().map(|w| w.0).collect::<Vec<_>>();
        assert_eq!(ids(fitting(&widths, room, None)), [1, 2, 3]);
        assert_eq!(ids(fitting(&widths, room, Some(WorkspaceId(2)))), [1, 2, 3]);
        assert_eq!(ids(fitting(&widths, room, Some(WorkspaceId(5)))), [1, 2, 5]);
        assert_eq!(
            ids(fitting(&widths, 50.0, Some(WorkspaceId(4)))),
            [4],
            "the one showing, even with room for none"
        );
        assert_eq!(
            ids(fitting(&widths, 1000.0, Some(WorkspaceId(4)))),
            [1, 2, 3, 4, 5]
        );
    }
}
