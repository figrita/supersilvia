// SPDX-License-Identifier: AGPL-3.0-or-later

//! The project tab: what the project holds, as a page rather than a modal.
//!
//! Two lists, which are the two things a project holds. **Workspaces** — one card per
//! workspace in project order, open or not, which is the whole reason this exists, since a
//! closed workspace has no tab and would otherwise be invisible. **Assets** — one card per
//! file in `assets/`, used or not, with which nodes on which workspaces reference it.
//!
//! Like [`crate::ui::show`] and [`crate::ui::tabs::show`] it draws and returns
//! [`ProjectAction`]s; it never mutates, never opens a dialog and never touches a file.
//! Reading a picture off disk and copying a file out are `App`'s, which is why a card is
//! handed a texture rather than a path.
//!
//! **A thing enters a project at the list and leaves from the thing.** Import… is at the top
//! of each section, because that is the list it adds to; Export is on the card, because it
//! acts on that one thing.
//!
//! A tab, not a modal and not a pane. A modal is chrome that stops the instrument and this is
//! a tool for a stage; a pane costs canvas width every frame for a list that is looked at
//! between songs.

use crate::graph::{NodeId, Workspace, WorkspaceId, WorkspaceKind};
use crate::ui::tabs;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align, Layout, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui, WidgetInfo,
    WidgetType, pos2,
};
use std::collections::{BTreeSet, HashMap};

/// The picture of a workspace. 16:9, at the scale an Output's own render is drawn on the
/// canvas, so a card is the size it will be whether or not there is a picture yet.
pub const THUMB: eframe::egui::Vec2 = eframe::egui::vec2(160.0, 90.0);

/// The icon an asset shows where there is no picture of it. By extension, because that is
/// all a file in `assets/` tells us without opening it — and a poster that has not been
/// decoded yet, or could not be, is exactly that case.
pub fn icon_for(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mts" | "ts" => "🎬",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" => "🖼",
        "wav" | "mp3" | "flac" | "ogg" | "opus" | "aac" | "m4a" => "🔊",
        _ => "📄",
    }
}
/// Padding inside a card.
const PAD: f32 = 10.0;
/// The widest the list gets, so a card is a card rather than a stripe across a wide window.
const LIST_WIDTH: f32 = 640.0;
/// How wide the blurb's editor is on a card.
const BLURB_WIDTH: f32 = 200.0;

/// What the project tab asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectAction {
    /// Give this workspace a tab and show it.
    Open(WorkspaceId),
    /// Take its tab away. Its nodes stay in the project.
    Close(WorkspaceId),
    /// Add a workspace of this kind, open it and show it.
    Add(WorkspaceKind),
    /// An edit, so `App` sends it to the bus.
    Rename { id: WorkspaceId, name: String },
    /// The same: a workspace's one line about itself.
    SetBlurb { id: WorkspaceId, blurb: String },
    /// The same, and it takes the nodes shown only here with it.
    Delete(WorkspaceId),
    /// Move a workspace to an index in project order, which is tab order.
    Move { id: WorkspaceId, to: usize },
    /// Copy it whole, beside it, as one undo step.
    Duplicate(WorkspaceId),
    /// Write this workspace out, with its assets, wherever the folder dialog says.
    Export(WorkspaceId),
    /// Take a `.ssw` into this project. The dialog is `App`'s to open.
    ImportWorkspace,
    /// Copy a file into `assets/`.
    ImportAsset,
    /// Copy one asset back out to a folder.
    ExportAsset(String),
    /// Open the folder the asset is in, in the desktop's file manager.
    RevealAsset(String),
    /// Delete it, which is refused while anything references it.
    RemoveAsset(String),
    /// Show the workspace this node is on and center it. Session state, not an edit.
    Navigate {
        workspace: WorkspaceId,
        node: NodeId,
    },
}

/// The page's own state: a rename, a blurb being typed, a delete waiting to be confirmed,
/// a card being dragged.
///
/// Session state the way `CanvasState` is — not in any file, not undoable.
#[derive(Debug, Default)]
pub struct ProjectState {
    /// The card whose name is being edited, the text so far, and whether the editor has
    /// been given the keyboard yet. Focus is asked for once, not every frame.
    renaming: Option<(WorkspaceId, String, bool)>,
    /// The card whose blurb is being edited, and the text so far. Unlike the name, the
    /// blurb is edited in place with no button to start it: the line *is* the editor.
    blurb: Option<(WorkspaceId, String)>,
    /// The workspace whose Delete is asking. Deleting removes nodes, so it asks.
    confirming: Option<WorkspaceId>,
    /// The card under the pointer during a reorder drag.
    dragging: Option<WorkspaceId>,
}

/// One node that references an asset, and where to go to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetUser {
    pub node: NodeId,
    /// Which workspace clicking it goes to: the first open one the node is on.
    pub workspace: WorkspaceId,
    /// `video3.file on Tunnel`, which is what the tree and the button both say.
    pub label: String,
}

/// One file in `assets/`, as the page draws it.
pub struct AssetCard {
    pub name: String,
    /// What a node's option holds to name it: `assets/<name>`.
    pub reference: String,
    pub size: u64,
    /// Which nodes on which workspaces reference it. Empty is what makes Remove possible.
    pub users: Vec<AssetUser>,
    /// A picture, for the formats there is one for. A video's first frame is a decode, so
    /// a clip shows its icon instead.
    pub thumbnail: Option<TextureHandle>,
    /// The icon a card with no picture shows.
    pub icon: &'static str,
}

/// Everything the page needs to draw itself, read-only.
pub struct ProjectPage<'a> {
    /// The project's name, which is its folder's.
    pub name: &'a str,
    pub workspaces: &'a [Workspace],
    pub open: &'a BTreeSet<WorkspaceId>,
    /// How many nodes each workspace shows, for the line under its name.
    pub counts: &'a dyn Fn(WorkspaceId) -> usize,
    /// Each workspace's picture, where one has been read off disk. A workspace with no
    /// entry, or a `None`, gets the placeholder.
    pub thumbnails: &'a HashMap<WorkspaceId, Option<TextureHandle>>,
    /// Every file in `assets/`, in name order.
    pub assets: &'a [AssetCard],
}

/// Draw the page and return every action the interaction asked for.
pub fn show(
    ui: &mut Ui,
    state: &mut ProjectState,
    page: &ProjectPage<'_>,
    theme: &Theme,
) -> Vec<ProjectAction> {
    let mut actions = Vec::new();
    eframe::egui::ScrollArea::vertical().show(ui, |ui| {
        // Clamped to what there is: a right-aligned control inside a width the panel does
        // not have would be laid out off the edge of it.
        ui.set_max_width(LIST_WIDTH.min(ui.available_width()));
        ui.add_space(PAD);
        ui.heading(page.name);
        ui.add_space(PAD);

        ui.horizontal(|ui| {
            ui.label("Workspaces");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("New workspace", |ui| {
                    for kind in [WorkspaceKind::Video] {
                        if ui.button(tabs::label_of(kind)).clicked() {
                            actions.push(ProjectAction::Add(kind));
                            ui.close();
                        }
                    }
                });
                // A workspace enters the project at the list it joins, which is this one.
                if ui.button("Import…").clicked() {
                    actions.push(ProjectAction::ImportWorkspace);
                }
            });
        });
        ui.add_space(4.0);

        // Where each card ended up, so a dropped card knows which index it was dropped on.
        let mut rows: Vec<(WorkspaceId, Rect)> = Vec::new();
        for workspace in page.workspaces {
            let rect = card(ui, state, page, workspace, theme, &mut actions);
            rows.push((workspace.id, rect));
        }

        // Drag to reorder: the index of the card the pointer let go over.
        if let Some(id) = state.dragging
            && ui.input(|i| i.pointer.any_released())
        {
            if let Some(at) = ui.ctx().pointer_latest_pos()
                && let Some(to) = rows.iter().position(|(_, r)| at.y <= r.max.y)
                && rows.get(to).is_some_and(|(over, _)| *over != id)
            {
                actions.push(ProjectAction::Move { id, to });
            }
            state.dragging = None;
        }

        ui.add_space(PAD);
        assets(ui, page, theme, &mut actions);
    });
    actions
}

/// One workspace's card. Returns the rectangle it took, for the reorder drop test.
fn card(
    ui: &mut Ui,
    state: &mut ProjectState,
    page: &ProjectPage<'_>,
    workspace: &Workspace,
    theme: &Theme,
    actions: &mut Vec<ProjectAction>,
) -> Rect {
    let id = workspace.id;
    let is_open = page.open.contains(&id);

    let frame = eframe::egui::Frame::new()
        .fill(theme.bg_sunken())
        .stroke(Stroke::new(
            1.0,
            if state.dragging == Some(id) {
                theme.primary()
            } else {
                theme.border_subtle()
            },
        ))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(PAD as i8);

    // The card itself is a widget: `UiBuilder::sense` registers its sense *below* everything
    // drawn inside it, so the buttons on it still take their own clicks and what is left
    // over — the picture, the name, the space around them — is the card's.
    let response = ui
        .scope_builder(
            eframe::egui::UiBuilder::new().sense(Sense::click_and_drag()),
            |ui| {
                frame.show(ui, |ui| {
                    // The card fills the list rather than shrinking to its contents, so the
                    // section's own buttons line up with its right edge and two cards of
                    // different lengths are the same shape.
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        picture(
                            ui,
                            theme,
                            page.thumbnails.get(&id).and_then(Option::as_ref),
                            THUMB,
                            "🖼",
                        );

                        ui.vertical(|ui| {
                            if state.renaming.as_ref().is_some_and(|(w, _, _)| *w == id) {
                                if let Some(action) = rename_editor(ui, state, id) {
                                    actions.push(action);
                                }
                            } else {
                                // Nothing drawn on the card is interactive on its own
                                // account, labels included — `theme::apply` turns text
                                // selection off everywhere so a click on the name is a click
                                // on the card.
                                ui.label(workspace.name.as_str());
                            }
                            ui.label(format!(
                                "{} · {} · {}",
                                tabs::label_of(workspace.kind),
                                if is_open { "open" } else { "closed" },
                                node_count((page.counts)(id)),
                            ));
                            blurb_editor(ui, state, workspace, actions);

                            ui.horizontal(|ui| {
                                if state.confirming == Some(id) {
                                    // Deleting a workspace removes the nodes shown only on it, so it
                                    // asks — inline, because a modal stops the instrument.
                                    ui.label("Delete it and its nodes?");
                                    if ui.button("Delete").clicked() {
                                        actions.push(ProjectAction::Delete(id));
                                        state.confirming = None;
                                    }
                                    if ui.button("Keep").clicked() {
                                        state.confirming = None;
                                    }
                                    return;
                                }
                                if is_open {
                                    if ui.button("Close").clicked() {
                                        actions.push(ProjectAction::Close(id));
                                    }
                                } else if ui.button("Open").clicked() {
                                    actions.push(ProjectAction::Open(id));
                                }
                                if ui.button("Rename").clicked() {
                                    state.renaming = Some((id, workspace.name.clone(), false));
                                }
                                if named(
                                    ui.button("Duplicate"),
                                    &format!("duplicate {}", workspace.name),
                                )
                                .clicked()
                                {
                                    actions.push(ProjectAction::Duplicate(id));
                                }
                                // Export leaves from the thing, so it is on the card as
                                // well as on the Workspace menu.
                                if named(ui.button("Export"), &format!("export {}", workspace.name))
                                    .clicked()
                                {
                                    actions.push(ProjectAction::Export(id));
                                }
                                if ui.button("Delete").clicked() {
                                    state.confirming = Some(id);
                                }
                            });
                        });
                    });
                });
            },
        )
        .response;

    let name = format!("workspace card {}", workspace.name);
    let selected = is_open;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, name.clone()));
    if response.clicked() {
        // Click opens and activates, whether or not it had a tab.
        actions.push(ProjectAction::Open(id));
    }
    if response.drag_started() {
        state.dragging = Some(id);
    }
    if state.dragging == Some(id)
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        // A line under the pointer, so a drag says where it would land.
        ui.painter().line_segment(
            [
                pos2(response.rect.min.x, at.y),
                pos2(response.rect.max.x, at.y),
            ],
            Stroke::new(2.0, theme.primary()),
        );
    }
    ui.add_space(6.0);
    response.rect
}

/// A card's picture, letterboxed inside the space reserved for it — or the placeholder
/// rectangle and an icon where there is none, so the layout is the same either way.
pub fn picture(
    ui: &mut Ui,
    theme: &Theme,
    texture: Option<&TextureHandle>,
    size: eframe::egui::Vec2,
    icon: &str,
) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::RADIUS_SM, theme.screen_off());
    match texture {
        Some(texture) => {
            let picture = texture.size_vec2();
            let scale = f32::min(size.x / picture.x, size.y / picture.y);
            let fitted = Rect::from_center_size(rect.center(), picture * scale);
            eframe::egui::Image::new(texture)
                .corner_radius(theme::RADIUS_SM)
                .paint_at(ui, fitted);
        }
        None => {
            ui.painter().text(
                rect.center(),
                eframe::egui::Align2::CENTER_CENTER,
                icon,
                eframe::egui::FontId::proportional(size.y * 0.4),
                theme.text_muted(),
            );
        }
    }
    ui.painter().rect_stroke(
        rect,
        theme::RADIUS_SM,
        Stroke::new(1.0, theme.border_subtle()),
        StrokeKind::Inside,
    );
}

/// The blurb, edited in place: one line that is the editor rather than a label with a
/// button beside it. Committed on Enter or on losing focus, as a `SetBlurb`.
fn blurb_editor(
    ui: &mut Ui,
    state: &mut ProjectState,
    workspace: &Workspace,
    actions: &mut Vec<ProjectAction>,
) {
    let id = workspace.id;
    let mut text = match &state.blurb {
        Some((w, text)) if *w == id => text.clone(),
        _ => workspace.blurb.clone(),
    };
    let editor = ui.add(
        eframe::egui::TextEdit::singleline(&mut text)
            .id(ui.id().with(("blurb", id)))
            // A width of its own rather than whatever is left: the cards are a list, and a
            // field that is a different length on each of them reads as a mistake.
            .desired_width(BLURB_WIDTH)
            .hint_text("what this workspace is for"),
    );
    if editor.changed() {
        state.blurb = Some((id, text.clone()));
    }
    let committed = editor.lost_focus() || ui.input(|i| i.key_pressed(eframe::egui::Key::Enter));
    if committed && state.blurb.as_ref().is_some_and(|(w, _)| *w == id) {
        let (_, blurb) = state.blurb.take().expect("checked above");
        if blurb != workspace.blurb {
            actions.push(ProjectAction::SetBlurb { id, blurb });
        }
    }
}

/// The inline rename on a card. Enter and clicking away commit, Escape drops it — the same
/// three answers the tab bar's editor gives.
fn rename_editor(ui: &mut Ui, state: &mut ProjectState, id: WorkspaceId) -> Option<ProjectAction> {
    let (_, text, focused) = state.renaming.as_mut()?;
    let editor =
        ui.add(eframe::egui::TextEdit::singleline(text).id(ui.id().with(("rename card", id))));
    if !*focused {
        editor.request_focus();
        *focused = true;
    }
    if ui.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
        state.renaming = None;
        return None;
    }
    if !editor.lost_focus() {
        return None;
    }
    let (id, name, _) = state.renaming.take()?;
    let name = name.trim();
    (!name.is_empty()).then(|| ProjectAction::Rename {
        id,
        name: name.to_string(),
    })
}

/// The Assets section: every file in `assets/`, used or not.
fn assets(ui: &mut Ui, page: &ProjectPage<'_>, theme: &Theme, actions: &mut Vec<ProjectAction>) {
    ui.horizontal(|ui| {
        ui.label("Assets");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if named(ui.button("Import…"), "import asset").clicked() {
                actions.push(ProjectAction::ImportAsset);
            }
        });
    });
    ui.add_space(4.0);
    if page.assets.is_empty() {
        ui.label("Nothing yet. A file dropped on the window lands here.");
        return;
    }
    for asset in page.assets {
        asset_card(ui, asset, theme, actions);
    }
}

/// One asset's card: its picture or its icon, its name and size, and who is using it.
fn asset_card(ui: &mut Ui, asset: &AssetCard, theme: &Theme, actions: &mut Vec<ProjectAction>) {
    let frame = eframe::egui::Frame::new()
        .fill(theme.bg_sunken())
        .stroke(Stroke::new(1.0, theme.border_subtle()))
        .corner_radius(theme::RADIUS_MD)
        .inner_margin(PAD as i8);

    let response = ui
        .scope_builder(eframe::egui::UiBuilder::new().sense(Sense::hover()), |ui| {
            frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    picture(ui, theme, asset.thumbnail.as_ref(), THUMB * 0.5, asset.icon);
                    ui.vertical(|ui| {
                        ui.label(asset.name.as_str());
                        ui.label(size_of(asset.size));
                        if asset.users.is_empty() {
                            ui.label("used by nothing");
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("used by");
                                for user in &asset.users {
                                    // Each user goes there when clicked, the way a
                                    // cross-workspace tag does.
                                    if ui.link(user.label.as_str()).clicked() {
                                        actions.push(ProjectAction::Navigate {
                                            workspace: user.workspace,
                                            node: user.node,
                                        });
                                    }
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            if named(ui.button("Export"), &format!("export {}", asset.name))
                                .clicked()
                            {
                                actions.push(ProjectAction::ExportAsset(asset.reference.clone()));
                            }
                            if named(ui.button("Reveal"), &format!("reveal {}", asset.name))
                                .clicked()
                            {
                                actions.push(ProjectAction::RevealAsset(asset.reference.clone()));
                            }
                            if named(ui.button("Remove"), &format!("remove {}", asset.name))
                                .clicked()
                            {
                                actions.push(ProjectAction::RemoveAsset(asset.reference.clone()));
                            }
                        });
                    });
                });
            });
        })
        .response;

    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::Other,
        format_args!("asset card {}", asset.name),
    );
    ui.add_space(6.0);
}

/// Give a widget the name the accessibility tree sees, where the text on it is not unique.
///
/// Four cards each carrying a Remove would otherwise be four widgets called Remove, and a
/// test or an agent could reach none of them by name. **The widget carries the name; the
/// geometry carries the click** — the same split cables and tags are drawn under.
fn named(response: Response, name: &str) -> Response {
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, name);
    response
}

/// "3 nodes", and "1 node".
fn node_count(n: usize) -> String {
    if n == 1 {
        "1 node".to_string()
    } else {
        format!("{n} nodes")
    }
}

/// A file's size in whichever unit reads as a number rather than a wall of digits.
pub fn size_of(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let n = bytes as f64;
    for (limit, unit) in [(1e9, "GB"), (1e6, "MB"), (1e3, "kB")] {
        if n >= limit {
            return format!("{:.1} {unit}", n / limit);
        }
    }
    format!("{bytes} B")
}
