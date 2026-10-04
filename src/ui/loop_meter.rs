// SPDX-License-Identifier: AGPL-3.0-or-later

//! The loop meter: where a node is in its own loop, drawn on its Time row in Loop mode where
//! the Speed knob stands in Free mode, at the knob's place and size.
//!
//! **Its look is its meaning.** The box is the loop, `period` of the node's own cycles long,
//! cut by a hairline at each whole cycle — four segments on a noise at Repeat 4, one on a
//! periodic node — and filled left to right by how far through the loop the node's Time is,
//! wrapping when the picture comes back. The text in its middle counts: the cycle it is in and
//! how many the loop has, `3/4`. A picture that never comes back has no loop, so the box spans
//! the one cycle it is in, says the whole cycles behind it, and leaves its right end open,
//! dashed: a solid end means it loops.
//!
//! It wears the s-number's chrome — its ground, its inset bevel, its font, and the quiet
//! uniform number tint a connected control meters in — because it sits where one does, but it
//! is a readout: no caps, no drag, no typing. It reads the Time as the node does
//! ([`crate::synth::Uniforms::time`]), never Offset, which on a node that draws is one value
//! per pixel. The progress arithmetic is [`crate::nodes::timing::Progress`]; see
//! [docs/ui.md](../../docs/ui.md#the-loop-meter).

use crate::nodes::timing::{Progress, Span};
use crate::ui::theme::{self, Theme};
use eframe::egui::{Align2, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, WidgetType, vec2};

/// How long a dash and the gap after it are on an open end, in points at zoom 1.
const DASH: f32 = 2.0;

/// The clear space either side of the caption that a hairline does not cross, in points at
/// zoom 1.
const GAP: f32 = 2.0;

/// What the meter says in its middle: the cycle within the loop, from one, and the loop's
/// length, `3/4`; on an open span, the whole cycles behind the node.
pub fn caption(p: &Progress) -> String {
    match p.span {
        Span::Loop { cycle, cycles, .. } => format!("{}/{cycles}", cycle + 1),
        // Plus zero, so a count rounded to minus zero is written `0`.
        Span::Open { count } => format!("{:.0}", count + 0.0),
    }
}

/// What a hover over the meter says.
pub fn tooltip(p: &Progress) -> String {
    match p.span {
        Span::Loop { cycle, cycles, .. } => format!("cycle {} of {cycles}", cycle + 1),
        Span::Open { count } => format!("{:.0} whole cycles; never comes back", count + 0.0),
    }
}

/// Draw the meter in `rect`, the place a number control takes on its row. `progress` is
/// `None` before the node's Time has a reading, which draws the empty box.
pub fn meter(
    ui: &mut Ui,
    rect: Rect,
    label: impl crate::ui::Name,
    progress: Option<Progress>,
    theme: &Theme,
    zoom: f32,
) {
    let painter = ui.painter_at(rect);
    let radius = theme::RADIUS_SM;
    painter.rect_filled(rect, CornerRadius::same(radius), theme.bg_interactive());
    let open = progress.is_some_and(|p| p.open());
    // The caption is laid out first, so the hairlines can part around it.
    let text = progress.as_ref().map(caption);
    let galley = text.as_ref().map(|text| {
        painter.layout_no_wrap(
            text.clone(),
            crate::ui::number::value_font(zoom),
            theme.readout(),
        )
    });
    let words = galley.as_ref().map_or(Rect::NOTHING, |g| {
        Align2::CENTER_CENTER
            .anchor_size(rect.center(), g.size())
            .expand2(vec2(GAP * zoom, 0.0))
    });
    if let Some(p) = &progress {
        let mut fill = rect;
        fill.set_width(rect.width() * p.fill.clamp(0.0, 1.0) as f32);
        let corners = CornerRadius {
            nw: radius,
            sw: radius,
            ne: 0,
            se: 0,
        };
        painter.rect_filled(fill, corners, theme.readout().gamma_multiply(0.15));
        // Hairlines between the cycles, inside the bevel, in the ground's own shadow, broken
        // where they would cross the caption.
        let inset = (1.5 * zoom).max(1.0);
        let stroke = Stroke::new(1.0, theme.bg_sunken());
        let (top, bottom) = (rect.min.y + inset, rect.max.y - inset);
        for at in p.dividers() {
            let x = (rect.min.x + rect.width() * at as f32).round() + 0.5;
            if (words.min.x..=words.max.x).contains(&x) {
                for (from, to) in [(top, words.min.y), (words.max.y, bottom)] {
                    if to > from {
                        painter.line_segment([Pos2::new(x, from), Pos2::new(x, to)], stroke);
                    }
                }
            } else {
                painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], stroke);
            }
        }
    }
    if open {
        // The bevel stops short of the right-hand corners, and the end is dashed in the
        // bevel's light tone.
        let width = (1.5 * zoom).max(1.0);
        let cut = rect.max.x - f32::from(radius) - width;
        crate::ui::number::bevel_border(
            &painter.with_clip_rect(rect.with_max_x(cut)),
            rect,
            zoom,
            theme,
        );
        let x = rect.max.x - width * 0.5;
        painter.extend(Shape::dashed_line(
            &[Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(width, theme.text_muted()),
            DASH * zoom,
            DASH * zoom,
        ));
    } else {
        crate::ui::number::bevel_border(&painter, rect, zoom, theme);
    }
    if let Some(galley) = galley {
        painter.galley(
            Align2::CENTER_CENTER
                .anchor_size(rect.center(), galley.size())
                .min,
            galley,
            theme.readout(),
        );
    }
    let response = ui.interact(rect, ui.id().with(("loop-meter", &label)), Sense::hover());
    crate::ui::accessible(
        &response,
        WidgetType::ProgressIndicator,
        format_args!("{label} {}", text.as_deref().unwrap_or("")),
    );
    if let Some(p) = progress {
        response.on_hover_ui(|ui| {
            ui.add(eframe::egui::Label::new(tooltip(&p)));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop counts from one over its length; an open span says the whole cycles behind it,
    /// never minus zero.
    #[test]
    fn the_caption_counts_the_cycle_in_the_loop() {
        assert_eq!(caption(&Progress::of(2.5, Some(4.0))), "3/4");
        assert_eq!(caption(&Progress::of(0.0, Some(4.0))), "1/4");
        assert_eq!(caption(&Progress::of(0.7, Some(1.0))), "1/1");
        assert_eq!(caption(&Progress::of(40.5, Some(64.0))), "41/64");
        assert_eq!(caption(&Progress::of(12.25, None)), "12");
        assert_eq!(caption(&Progress::of(-0.0, None)), "0");
        assert_eq!(caption(&Progress::of(-0.25, None)), "-1");
        assert_eq!(tooltip(&Progress::of(2.5, Some(4.0))), "cycle 3 of 4");
        assert_eq!(
            tooltip(&Progress::of(12.25, None)),
            "12 whole cycles; never comes back"
        );
    }
}
