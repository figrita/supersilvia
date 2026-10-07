// SPDX-License-Identifier: AGPL-3.0-or-later

//! The strip across the editor while a render runs.
//!
//! A render is modal — the frame thread is rendering, not performing, and the document is
//! closed — so the editor says it in a way nobody can miss: a band in the action color under
//! the tabs, over every workspace, with the progress and the one button that cancels. It is
//! drawn from a [`RenderView`] and returns whether that button was clicked; nothing here
//! mutates.

use crate::ui::RenderView;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, vec2,
};

/// The band's height in points.
const HEIGHT: f32 = 36.0;

/// Draw the band and return true when *Cancel render* was clicked.
pub fn show(ui: &mut Ui, view: &RenderView, destination: &str, theme: &Theme) -> bool {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let action = theme.port(crate::graph::PortType::Action);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, action);

    // The progress, as a darker band growing across the whole strip behind the words.
    let fraction = view.written as f32 / view.frames.max(1) as f32;
    let mut done = rect;
    done.max.x = rect.min.x + rect.width() * fraction.clamp(0.0, 1.0);
    painter.rect_filled(done, CornerRadius::ZERO, Color32::from_black_alpha(90));

    let text = format!(
        "RENDERING  Output {}   frame {} / {}   {destination}",
        view.node.0, view.written, view.frames
    );
    painter.text(
        rect.left_center() + vec2(16.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(theme::font_size(theme::FONT_BASE, 1.0)),
        Color32::BLACK,
    );

    // The one button. Right-aligned, in the strip's own colors inverted, so it reads as
    // the thing to press rather than as more of the band.
    let caption = "Cancel render";
    let button = Rect::from_min_size(
        rect.right_center() + vec2(-16.0 - 130.0, -12.0),
        vec2(130.0, 24.0),
    );
    let response = ui.interact(button, ui.id().with("cancel-render"), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        button,
        CornerRadius::same(theme::RADIUS_SM),
        if response.hovered() {
            theme.bg_hover()
        } else {
            theme.bg_primary()
        },
    );
    painter.rect_stroke(
        button,
        CornerRadius::same(theme::RADIUS_SM),
        Stroke::new(1.0, Color32::BLACK),
        StrokeKind::Inside,
    );
    painter.text(
        button.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(theme::font_size(theme::FONT_BASE, 1.0)),
        theme.text_primary(),
    );
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, caption);
    response.clicked()
}
