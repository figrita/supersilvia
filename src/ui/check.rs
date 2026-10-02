// SPDX-License-Identifier: AGPL-3.0-or-later

//! A show/hide tick, and the row several of them share.
//!
//! silvia's `input[type="checkbox"]`: a rounded square of `bg-secondary` with a
//! `border-normal` hairline, holding a checkmark in `primary` when it is on, with its caption
//! beside it — and the caption is part of the target, because silvia wraps both in a
//! `<label>`. The row is silvia's `.audio-visibility-toggles`: every tick on one line, each
//! centerd in an equal share of the width, which is what `justify-content: space-around`
//! does — unless a caption would not fit its equal share, when each share is its tick's own
//! width and what is left is spread evenly ([`shares`]).
//!
//! It reports a **click**, not a level: unlike `press`, a tick is an edit — the value is an
//! option, so it goes through `Command::SetOption` and rides the undo history like any other.

use crate::ui::node_widget::{Keep, advance, fit};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, WidgetInfo,
    WidgetType, pos2, vec2,
};

/// The box itself: silvia's `1.15em` against the 11px the toggle row sets, rounded to a
/// whole point because everything else on a node is.
pub const BOX: f32 = 13.0;

/// Clear space between the box and its caption — silvia's `gap: 0.3rem`.
const GAP: f32 = 4.0;

/// How wide each tick's share of a row `room` wide is, for these captions: equal shares,
/// silvia's `space-around`, wherever every tick and its whole caption fits in one; otherwise
/// each its own width with what is left spread evenly, so a long caption beside short ones is
/// read whole rather than elided.
pub fn shares(ui: &Ui, captions: &[&str], room: f32, zoom: f32) -> Vec<f32> {
    let n = captions.len().max(1) as f32;
    let equal = room / n;
    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
    let advance = advance(ui.ctx(), &font);
    let natural: Vec<f32> = captions
        .iter()
        .map(|c| (BOX + GAP) * zoom + c.chars().count() as f32 * advance)
        .collect();
    let total: f32 = natural.iter().sum();
    if natural.iter().all(|w| *w <= equal) || total > room {
        return vec![equal; captions.len()];
    }
    let spare = (room - total) / n;
    natural.iter().map(|w| w + spare).collect()
}

/// Draw one tick with its caption, centerd in `slot`, and hand back its response: `clicked` is
/// the edit, and the caller hangs any hover text on it.
///
/// `name` is the accessible name — `{slug}{id}.{key}` — the same shape every other control on
/// a node reports, so a test or an agent script finds `video1.scope` by it.
pub fn tick(
    ui: &mut Ui,
    slot: Rect,
    caption: &str,
    on: bool,
    name: impl crate::ui::Name,
    theme: &Theme,
    zoom: f32,
) -> Response {
    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
    let size = BOX * zoom;
    // The caption takes what is left of the slot after the box and the gap, elided from the
    // end where it does not fit — a zoomed-out `Uniforms` reads as `Unifo…` rather than
    // sliding under the tick beside it.
    let room = (slot.width() - size - GAP * zoom).max(0.0);
    // One glyph-width lookup, not one per measurement: `advance` takes the context's font
    // lock, and this runs for every tick on every node that has one, every frame.
    let advance = advance(ui.ctx(), &font);
    let text = fit(caption, advance, room, Keep::Start);
    let width = size + GAP * zoom + text.chars().count() as f32 * advance;
    // Centerd in its share of the row, box first, as the `<label>`'s flex line is.
    let left = slot.center().x - width * 0.5;
    let hit = Rect::from_min_size(
        Pos2::new(left, slot.center().y - slot.height() * 0.5),
        vec2(width, slot.height()),
    );
    let response = ui.interact(hit, ui.id().with(("check", &name)), Sense::click());
    let square = Rect::from_center_size(
        Pos2::new(left + size * 0.5, slot.center().y),
        vec2(size, size),
    );

    let painter = ui.painter();
    crate::ui::field(
        painter,
        square,
        theme.bg_secondary(),
        None,
        if response.hovered() {
            theme.primary_muted()
        } else {
            theme.border_normal()
        },
    );
    if on {
        mark(painter, square, theme.primary());
    }
    painter.text(
        Pos2::new(square.max.x + GAP * zoom, slot.center().y),
        Align2::LEFT_CENTER,
        &text,
        font,
        if on {
            theme.text_secondary()
        } else {
            theme.text_muted()
        },
    );

    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, name.to_string()));
    response
}

/// silvia's checkmark, which is a `clip-path` polygon rather than a glyph: the same two
/// strokes it describes, drawn as geometry so the mark is the mark at any zoom and does not
/// depend on a font carrying `✓`.
fn mark(painter: &Painter, square: Rect, color: Color32) {
    // The polygon's own proportions, as fractions of the box: down to the low point at 44%
    // across, then up to the top right.
    let p = |x: f32, y: f32| {
        pos2(
            square.min.x + square.width() * x,
            square.min.y + square.height() * y,
        )
    };
    let stroke = Stroke::new((square.width() * 0.16).max(1.0), color);
    // The short arm is a third of the box rather than silvia's fifth: at thirteen points a
    // fifth is barely longer than the stroke drawing it, and the mark reads as a slash.
    painter.line_segment([p(0.18, 0.46), p(0.40, 0.70)], stroke);
    painter.line_segment([p(0.40, 0.70), p(0.82, 0.24)], stroke);
}
