// SPDX-License-Identifier: AGPL-3.0-or-later

//! One line of what a node is doing, across the foot of its body.
//!
//! silvia keeps a status line in the body of several nodes — the divider counting down to
//! its next pass, the clock naming the beat — and it is the difference between a node you
//! can see working and four rows that never move. Here it is a region like any other,
//! drawing whatever the node's own [`CpuNode::status`](crate::nodes::CpuNode::status) says,
//! so nothing above it knows which node it is drawing.
//!
//! Read-only prose: the region does not claim the pointer, and a hand carries the node by
//! its status line exactly as by any other part of the body. A line beginning with
//! [`FLASH`] is drawn in the accent instead of the body text — the node saying *this just
//! happened*, which a count alone cannot say.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::cpu::FLASH;
use crate::ui::canvas;
use eframe::egui::{Align2, FontId, Sense, WidgetType};

/// A status line, flush to the foot of the body: one line of monospace and the air around
/// it, which is the Output's own status row rather than a control's height.
pub const STATUS: RegionDef = RegionDef {
    size,
    show,
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    canvas::READOUT_ROW_PITCH
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    // A node that has not ticked yet, or whose tick has nothing to say, has an empty band
    // rather than a drawn placeholder.
    let Some(text) = r.live.status else {
        return Vec::new();
    };
    let flashing = text.starts_with(FLASH);
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        r.zoom,
    ));
    r.ui.painter().text(
        r.rect.left_center() + eframe::egui::vec2(STATUS_INSET * r.zoom, 0.0),
        Align2::LEFT_CENTER,
        text,
        font,
        if flashing {
            r.theme.accent()
        } else {
            r.theme.text_secondary()
        },
    );
    // Named, not only painted: what a node says about itself is a fact a test and the
    // agent-driven layer have to be able to read.
    let name = r.name("status");
    let label = format!("{name} {text}");
    let w =
        r.ui.interact(r.rect, r.ui.id().with(("status", name)), Sense::hover());
    crate::ui::accessible(&w, WidgetType::Label, &label);
    Vec::new()
}

/// How far the line sits in from the body's edge, matching an option row's own label and the
/// Output's status cells.
const STATUS_INSET: f32 = 8.0;
