// SPDX-License-Identifier: AGPL-3.0-or-later

//! Project ▸ New project… and Save as…: a name, and the projects folder it goes in.
//!
//! A small modal rather than a folder dialog, because a new project or a copy of one is almost
//! always one more folder in the same place, and a folder dialog that has to be handed an
//! empty folder makes the person make one first, in a dialog that was not built for it. The
//! name field is filled — the next free *Untitled N* for a new project, the project's own name
//! or the next free one after it for a copy — and selected, so Enter takes it and typing
//! replaces it. **Choose location…** keeps the folder dialog for anywhere else. Why a name
//! cannot be had — none, a character this machine's folders cannot hold, a name already there
//! — is said in the window on a line of its own that is there whether or not it says anything,
//! so typing never moves the buttons.
//!
//! It draws and returns, as every surface in `ui/` does: `App` checks the name each frame and
//! makes the project, or the copy.

use super::theme::Theme;
use eframe::egui::{self, Button, Context, Key, Modal, RichText, TextEdit};
use std::path::{Path, PathBuf};

/// What the name is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Project ▸ New project…: an empty project in the folder.
    New,
    /// Project ▸ Save as…: this project copied into the folder, and carried on in there.
    SaveAs,
}

impl Purpose {
    fn heading(self) -> &'static str {
        match self {
            Self::New => "New project",
            Self::SaveAs => "Save project as",
        }
    }

    /// The button that takes the name.
    pub fn button(self) -> &'static str {
        match self {
            Self::New => "Create",
            Self::SaveAs => "Save",
        }
    }
}

/// What the window asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectNameAction {
    /// Make the project, or the copy, in this folder, which the name was checked to give.
    Chosen(PathBuf),
    /// Put the folder dialog up instead.
    ChooseLocation,
    Cancel,
}

/// The window's state across frames: what it is for, and the name being typed.
#[derive(Debug, Clone)]
pub struct ProjectNameState {
    pub purpose: Purpose,
    pub name: String,
    /// Whether the field has had the keyboard handed to it, which it is once, on opening.
    focused: bool,
}

impl ProjectNameState {
    /// The window, opening with this name in its field.
    pub fn new(purpose: Purpose, name: String) -> Self {
        Self {
            purpose,
            name,
            focused: false,
        }
    }
}

/// What the window draws beside the name.
pub struct ProjectNameView<'a> {
    /// The projects folder, or `None` where there is nowhere to keep one.
    pub dir: Option<&'a Path>,
    /// The folder the name gives, or why it gives none.
    pub verdict: &'a Result<PathBuf, String>,
}

/// The field's accessible name.
pub const NAME: &str = "Project name";

/// Draw the window. Returns what was asked of it this frame.
pub fn show(
    ctx: &Context,
    state: &mut ProjectNameState,
    view: &ProjectNameView<'_>,
    theme: &Theme,
) -> Option<ProjectNameAction> {
    let mut asked = None;
    let purpose = state.purpose;
    let modal = Modal::new(egui::Id::new("project-name")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.heading(purpose.heading());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = ui.label(NAME);
            let field = ui
                .add(
                    TextEdit::singleline(&mut state.name)
                        .desired_width(f32::INFINITY)
                        .hint_text("Untitled"),
                )
                .labelled_by(label.id);
            if !state.focused {
                state.focused = true;
                field.request_focus();
                select_all(ui, &field, &state.name);
            }
            if field.lost_focus()
                && ui.input(|i| i.key_pressed(Key::Enter))
                && let Ok(root) = view.verdict
            {
                asked = Some(ProjectNameAction::Chosen(root.clone()));
            }
        });
        ui.horizontal(|ui| {
            ui.label("In the folder");
            let text = match view.dir {
                Some(dir) => {
                    let whole = dir.display().to_string();
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    let glyph = ui.fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
                    let fits = (ui.available_width() / glyph).floor().max(3.0) as usize;
                    ui.add(
                        egui::Label::new(
                            RichText::new(super::prefs::middle_cut(&whole, fits)).monospace(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(whole);
                    return;
                }
                None => "nowhere to keep projects",
            };
            ui.label(RichText::new(text).color(theme.text_muted()));
        });
        // The reason, or nothing, on a line that is there either way.
        let reason = view.verdict.as_ref().err().map_or("", String::as_str);
        ui.add_sized(
            [WIDTH, ui.text_style_height(&egui::TextStyle::Body)],
            egui::Label::new(RichText::new(reason).color(ui.visuals().error_fg_color)).truncate(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let entry = ui.add_enabled(view.verdict.is_ok(), Button::new(purpose.button()));
            crate::ui::pointing(&entry);
            if entry.clicked()
                && let Ok(root) = view.verdict
            {
                asked = Some(ProjectNameAction::Chosen(root.clone()));
            }
            if ui.button("Choose location…").clicked() {
                asked = Some(ProjectNameAction::ChooseLocation);
            }
            if ui.button("Cancel").clicked() {
                asked = Some(ProjectNameAction::Cancel);
            }
        });
    });
    if modal.should_close() && asked.is_none() {
        asked = Some(ProjectNameAction::Cancel);
    }
    asked
}

/// The window's inner width.
const WIDTH: f32 = 440.0;

/// Select the whole of a field's text, so typing replaces it.
fn select_all(ui: &egui::Ui, field: &egui::Response, text: &str) {
    if let Some(mut edit) = egui::text_edit::TextEditState::load(ui.ctx(), field.id) {
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        );
        edit.cursor.set_char_range(Some(range));
        edit.store(ui.ctx(), field.id);
    }
}
