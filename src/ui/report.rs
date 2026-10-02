// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help ▸ Report a problem…: a short form, what the report says about this computer, one big
//! **Copy report**, and the two places to paste it.
//!
//! One `egui::Window` of ordinary widgets, like About, and **of one size whatever is typed**:
//! each multi-line field is a fixed band that scrolls inside itself, a metadata row is cut
//! rather than wrapped, and the full report under its disclosure scrolls in a fixed frame of
//! its own, so only opening that disclosure changes the window's height.
//!
//! It draws and returns, as every surface in `ui/` does: `App` gathers the metadata on a
//! thread, composes the block, copies it and opens the links (`app::crashlog`).

use super::theme::Theme;
use eframe::egui::{
    self, Align2, Button, CollapsingHeader, Context, RichText, ScrollArea, TextEdit, Ui, Window,
    vec2,
};

/// The window's title, which is its id.
pub const TITLE: &str = "Report a problem";
/// The fields' accessible names: their labels.
pub const UP: &str = "What's up?";
pub const NAME: &str = "Your name or Discord handle";
/// The copy button, before and after it has copied what is in the form now.
pub const COPY: &str = "Copy report";
pub const COPIED: &str = "Copied. Now paste it in either place below";
/// The two links.
pub const DISCORD: &str = "Open the Discord bug channel";
pub const GITHUB: &str = "Open a GitHub issue";

/// How wide the window is.
const WIDTH: f32 = 520.0;
/// How tall the one multi-line field's band is.
const FIELD_HEIGHT: f32 = 126.0;
/// How tall the full report's frame is, once opened.
const FULL_HEIGHT: f32 = 220.0;

/// What the window asks for. It mutates nothing past its own fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAction {
    /// Copy everything, the fields and the metadata, as one block.
    Copy,
    OpenDiscord,
    OpenGitHub,
    /// The window's close button.
    Close,
}

/// The form across frames: what has been typed, and whether it has been copied since.
#[derive(Debug, Clone, Default)]
pub struct ReportState {
    pub up: String,
    pub name: String,
    /// The block on the clipboard is what the form holds now. Typing clears it.
    pub copied: bool,
    /// The full report's disclosure is open.
    pub show_full: bool,
    /// The first field has had the keyboard handed to it, which it is once, on opening.
    focused: bool,
}

/// What the window shows under the fields, once it has been gathered.
pub struct Gathered<'a> {
    pub os: &'a str,
    pub gpu: &'a str,
    /// The last run closed unexpectedly, so its log's end is included too.
    pub last_run: bool,
}

/// What the window draws besides the form.
pub struct ReportView<'a> {
    /// `None` while it is still being gathered.
    pub gathered: Option<Gathered<'a>>,
    /// The block Copy would copy, while the disclosure is open.
    pub full: Option<&'a str>,
}

/// Draw the window. Returns what was asked of it this frame.
pub fn show(
    ctx: &Context,
    state: &mut ReportState,
    view: &ReportView<'_>,
    theme: &Theme,
) -> Vec<ReportAction> {
    let mut actions = Vec::new();
    let mut open = true;
    Window::new(TITLE)
        .id(egui::Id::new(TITLE))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .fixed_size(vec2(WIDTH, 0.0))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let mut changed = false;
            changed |= multiline(ui, UP, &mut state.up, "up", !state.focused);
            state.focused = true;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = ui.label(NAME);
                changed |= ui
                    .add(
                        TextEdit::singleline(&mut state.name)
                            .desired_width(f32::INFINITY)
                            .hint_text("optional"),
                    )
                    .labelled_by(label.id)
                    .changed();
            });
            if changed {
                state.copied = false;
            }

            ui.add_space(8.0);
            ui.separator();
            metadata(ui, view, theme);
            let full = CollapsingHeader::new("The full report")
                .id_salt("report-full")
                .open(Some(state.show_full))
                .show(ui, |ui| {
                    ScrollArea::both()
                        .id_salt("report-full-text")
                        .max_height(FULL_HEIGHT)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(view.full.unwrap_or("Still gathering…")).monospace(),
                            );
                        });
                });
            if full.header_response.clicked() {
                state.show_full = !state.show_full;
            }

            ui.add_space(8.0);
            let label = if state.copied { COPIED } else { COPY };
            let copy = ui
                .add_enabled(
                    view.gathered.is_some(),
                    Button::new(RichText::new(label).size(16.0).strong())
                        .min_size(vec2(ui.available_width(), 36.0)),
                )
                .on_hover_text("Your words above, this computer, --check and the log's end")
                .on_disabled_hover_text("Still gathering what to say about this computer");
            if copy.clicked() {
                actions.push(ReportAction::Copy);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(DISCORD).clicked() {
                    actions.push(ReportAction::OpenDiscord);
                }
                if ui
                    .button(GITHUB)
                    .on_hover_text("Its title is the first line of What's up?")
                    .clicked()
                {
                    actions.push(ReportAction::OpenGitHub);
                }
            });
            ui.label(
                RichText::new("Copy first, then paste the report where either one opens.")
                    .color(theme.text_muted()),
            );
        });
    if !open {
        actions.push(ReportAction::Close);
    }
    actions
}

/// A label over a field of fixed height that scrolls inside itself. Returns whether its text
/// changed.
fn multiline(ui: &mut Ui, label: &str, text: &mut String, salt: &str, focus: bool) -> bool {
    let label = ui.label(label);
    let mut changed = false;
    ScrollArea::vertical()
        .id_salt(salt)
        .max_height(FIELD_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let field = ui
                .add(
                    TextEdit::multiline(text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(6),
                )
                .labelled_by(label.id);
            if focus {
                field.request_focus();
            }
            changed = field.changed();
        });
    changed
}

/// The rows under the fields: what the report says about this computer, each one line, cut
/// rather than wrapped.
fn metadata(ui: &mut Ui, view: &ReportView<'_>, theme: &Theme) {
    let row = |ui: &mut Ui, what: &str, value: &str| {
        ui.horizontal(|ui| {
            ui.add_sized(
                [56.0, ui.spacing().interact_size.y],
                egui::Label::new(RichText::new(what).color(theme.text_secondary())),
            );
            ui.add(egui::Label::new(value).truncate());
        });
    };
    row(ui, "Version", env!("CARGO_PKG_VERSION"));
    let (os, gpu, with) =
        view.gathered
            .as_ref()
            .map_or(("gathering…", "gathering…", "gathering…"), |g| {
                (
                    g.os,
                    g.gpu,
                    if g.last_run {
                        "--check, this run's log and the last run's, which closed unexpectedly"
                    } else {
                        "--check and the end of this run's log"
                    },
                )
            });
    row(ui, "OS", os);
    row(ui, "GPU", gpu);
    row(ui, "With", with);
}
