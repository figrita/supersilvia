// SPDX-License-Identifier: AGPL-3.0-or-later

//! The problems badge and the list it opens.
//!
//! A failure said only in the Status box is said to nobody, since the box is off by default.
//! The badge is the one place every standing problem is counted whatever is open: a shader
//! that failed, a node whose device would not open or stopped answering, every warning the
//! last Open dropped, what the last export left behind, and the failures said this session.
//! It is a fixed-width slot in the menu bar, empty while there is nothing, so a count arriving
//! moves nothing beside it. A click opens the list, errors first, each row whole on its hover
//! and a `▸ go` where the problem has a node to go to. [docs/ui.md](../../docs/ui.md#problems).
//!
//! Like the rest of `ui/` it draws and returns: the list is handed in and what a hand asked
//! for comes back as a [`ProblemAction`].

use crate::graph::NodeId;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    self, Align2, Area, Context, FontId, Id, Order, Rect, RichText, Sense, Ui, WidgetType, pos2,
    vec2,
};

/// How bad one problem is. Errors sort first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something that does not work: a shader, a device, a save.
    Error,
    /// Something that works with a part left out: a warning of the last Open, what an export
    /// left behind.
    Warning,
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub text: String,
    /// The node the problem is about, where it has one still in the graph: the row's `▸ go`.
    pub go: Option<NodeId>,
}

/// What the badge counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
    pub errors: usize,
    pub warnings: usize,
}

impl Count {
    pub fn of(problems: &[Problem]) -> Self {
        let errors = problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        Self {
            errors,
            warnings: problems.len() - errors,
        }
    }

    pub fn total(self) -> usize {
        self.errors + self.warnings
    }
}

/// What a hand asked of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemAction {
    /// Show this node, on its workspace, centred.
    Go(NodeId),
    /// Forget what was said — the failures, the last Open's warnings, the last export's
    /// report. What is still wrong stays, since it is still wrong.
    Clear,
}

/// The badge's accessible name, followed by its count: `problems 3`.
pub const BADGE: &str = "problems";

/// Where the badge last stood, kept in egui's memory so the list can hang under it.
fn badge_id() -> Id {
    Id::new("problems-badge")
}

/// How many digits the count is laid out for. A count past it reads `99+`.
const COUNT_CHARS: usize = 3;
/// Horizontal padding either side of the badge's contents.
const PAD: f32 = 4.0;
/// How wide the list is, whatever it holds.
const LIST_WIDTH: f32 = 460.0;
/// How tall the list's rows may run before they scroll.
const LIST_HEIGHT: f32 = 320.0;

/// The count as the badge prints it: up to two digits, then `99+`.
pub fn count_text(n: usize) -> String {
    if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

/// The badge's width: a square for the glyph and the count's three characters.
fn badge_width(ui: &Ui, height: f32) -> f32 {
    let font = FontId::proportional(theme::FONT_BASE);
    let advance = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    height + advance * COUNT_CHARS as f32 + PAD * 2.0
}

/// The badge, into a right-to-left layout: `⚠ 3` at a fixed width, or the same width of
/// nothing while there is nothing to count. True on the frame it is clicked.
pub fn badge(ui: &mut Ui, count: Count, theme: &Theme) -> bool {
    let height = ui.spacing().interact_size.y;
    let width = badge_width(ui, height);
    if count.total() == 0 {
        ui.add_space(width);
        return false;
    }
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    ui.ctx().data_mut(|d| d.insert_temp(badge_id(), rect));
    crate::ui::accessible(
        &response,
        WidgetType::Button,
        format_args!("{BADGE} {}", count.total()),
    );
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_SM),
            theme.bg_secondary(),
        );
    }
    // The accent while anything is an error: the palette's one color for failing. Warnings
    // alone are the ordinary ink, and the glyph and the count say the rest.
    let ink = if count.errors > 0 {
        theme.accent()
    } else {
        theme.text_secondary()
    };
    let glyph = Rect::from_min_size(pos2(rect.min.x + PAD, rect.min.y), vec2(height, height));
    painter.text(
        glyph.center(),
        Align2::CENTER_CENTER,
        "⚠",
        FontId::proportional(theme::FONT_BASE),
        ink,
    );
    painter.text(
        pos2(glyph.max.x, rect.center().y),
        Align2::LEFT_CENTER,
        count_text(count.total()),
        FontId::proportional(theme::FONT_BASE),
        ink,
    );
    let said = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    let parts: Vec<String> = [
        said(count.errors, "error", "errors"),
        said(count.warnings, "warning", "warnings"),
    ]
    .into_iter()
    .flatten()
    .collect();
    response
        .on_hover_text(format!("{}: click for the list", parts.join(", ")))
        .clicked()
}

/// Whether the pointer is on the badge, so a click there toggles the list rather than also
/// counting as a click away from it.
pub fn on_badge(ctx: &Context) -> bool {
    let rect = ctx.data(|d| d.get_temp::<Rect>(badge_id()));
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    matches!((rect, pointer), (Some(r), Some(p)) if r.contains(p))
}

/// What one frame of the list did.
pub struct Listed {
    pub actions: Vec<ProblemAction>,
    /// A click landed outside the list and off the badge.
    pub dismissed: bool,
}

/// The list, hung under the badge by its right edge: errors first, a row each, cut to one
/// line and whole on its hover, and **Clear** at the foot.
pub fn list(ctx: &Context, problems: &[Problem], theme: &Theme) -> Listed {
    let under = ctx.data(|d| d.get_temp::<Rect>(badge_id())).map_or_else(
        || pos2(ctx.content_rect().max.x - 8.0, 32.0),
        |r| r.right_bottom(),
    );
    let mut actions = Vec::new();
    let shown = Area::new(Id::new("problems-list"))
        .order(Order::Foreground)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(under + vec2(0.0, 4.0))
        .movable(false)
        .show(ctx, |ui| {
            crate::ui::popup::frame(ui.style(), theme, Theme::border_normal).show(ui, |ui| {
                ui.set_width(LIST_WIDTH);
                let count = Count::of(problems);
                ui.label(
                    RichText::new(if count.total() == 0 {
                        "Nothing is wrong.".to_string()
                    } else {
                        format!("Problems: {}", count.total())
                    })
                    .color(theme.text_secondary()),
                );
                egui::ScrollArea::vertical()
                    .max_height(LIST_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, problem) in problems.iter().enumerate() {
                            if let Some(asked) = row(ui, i, problem, theme) {
                                actions.push(asked);
                            }
                        }
                    });
                ui.separator();
                let clear = ui
                    .button("Clear")
                    .on_hover_text("Forget what was said: the failures, the last Open's warnings and the last export's report. What is still wrong stays.");
                crate::ui::accessible(&clear, WidgetType::Button, "Clear problems");
                if clear.clicked() {
                    actions.push(ProblemAction::Clear);
                }
            });
        });
    let away = crate::ui::popup::clicked_away(ctx, &shown.response) && !on_badge(ctx);
    Listed {
        actions,
        dismissed: away,
    }
}

/// One problem: what it is, the line cut to the row, and `▸ go` at the right where there is
/// a node to go to.
fn row(ui: &mut Ui, i: usize, problem: &Problem, theme: &Theme) -> Option<ProblemAction> {
    let small = FontId::proportional(theme::FONT_TINY);
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let (word, ink) = match problem.severity {
        Severity::Error => ("error", theme.accent()),
        Severity::Warning => ("warn", theme.text_muted()),
    };
    let word_width = 6.0 * ui.ctx().fonts_mut(|f| f.glyph_width(&small, '0'));
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        word,
        small.clone(),
        ink,
    );
    let go_width = if problem.go.is_some() { 48.0 } else { 0.0 };
    let text_rect = Rect::from_min_max(
        pos2(rect.min.x + word_width, rect.min.y),
        pos2(rect.max.x - go_width, rect.max.y),
    );
    let line = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&problem.text).color(theme.text_primary()))
                        .truncate(),
                )
            },
        )
        .inner;
    crate::ui::accessible(
        &line,
        WidgetType::Label,
        format_args!("problem {} {}", i + 1, problem.text),
    );
    line.on_hover_text(&problem.text);
    let node = problem.go?;
    let go_rect = Rect::from_min_max(pos2(rect.max.x - go_width, rect.min.y), rect.max);
    let go = ui.put(
        go_rect,
        egui::Button::new(RichText::new("▸ go").color(theme.primary())).frame(false),
    );
    crate::ui::accessible(
        &go,
        WidgetType::Button,
        format_args!("problem {} go", i + 1),
    );
    go.on_hover_text("Show this node")
        .clicked()
        .then_some(ProblemAction::Go(node))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count is two digits and then `99+`, so the badge's width holds whatever it says.
    #[test]
    fn the_count_stays_inside_its_three_characters() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(7), "7");
        assert_eq!(count_text(99), "99");
        assert_eq!(count_text(100), "99+");
        assert_eq!(count_text(12_000), "99+");
        assert!(count_text(usize::MAX).chars().count() <= COUNT_CHARS);
    }

    #[test]
    fn a_count_splits_errors_from_warnings() {
        let p = |severity| Problem {
            severity,
            text: String::new(),
            go: None,
        };
        let count = Count::of(&[
            p(Severity::Warning),
            p(Severity::Error),
            p(Severity::Warning),
        ]);
        assert_eq!(
            count,
            Count {
                errors: 1,
                warnings: 2
            }
        );
        assert_eq!(count.total(), 3);
    }
}
