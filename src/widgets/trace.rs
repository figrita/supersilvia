// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture of the shape a node is editing — an envelope, a waveform, a travel — where four
//! digits in a column cannot show one.
//!
//! Read-only: the samples are `CpuNode::trace`'s own ring, what the node published over the
//! last few seconds, each dated and drawn where its time puts it. So the region does not claim
//! the pointer, and a hand carries the node by its trace exactly as by any other part of the
//! body.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::ui::{canvas, theme};
use eframe::egui::{Align2, FontId, Rect, Sense, WidgetType, pos2, vec2};

/// A trace band, flush to the body's bottom corners as an Output's render and the audio scope
/// are. No heading: it is the whole of what these nodes have below their rows, and it widens
/// them to the scope's own 300 rather than asking for a sixth width.
pub const TRACE: RegionDef = RegionDef {
    size,
    show,
    width: Some(canvas::SCOPE_NODE_WIDTH),
    ..RegionDef::EMPTY
};

/// The cells under a trace, silvia's own pair beneath its envelope graph: a tiny label and
/// the reading under it, in even columns across the body.
///
/// What the cells say is `CpuNode::caption`'s, so the region knows nothing about which node
/// it is drawing for. It reserves its height whether or not the node has ticked, because
/// layout runs long before anything is published and a band that appeared on the first tick
/// would jog the node.
pub const CAPTION: RegionDef = RegionDef {
    size: caption_size,
    show: caption,
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    canvas::TRACE_HEIGHT
}

fn caption_size(_node: &Node) -> f32 {
    canvas::CAPTION_HEIGHT
}

fn caption(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(cells) = r.live.caption.filter(|c| !c.is_empty()) else {
        return Vec::new();
    };
    let painter = r.ui.painter().clone();
    let label_font = FontId::proportional(theme::font_size(theme::FONT_TINY, r.zoom));
    let value_font = FontId::proportional(theme::font_size(theme::FONT_BASE, r.zoom));
    let width = r.rect.width() / cells.len() as f32;
    for (index, (key, value)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(r.rect.min.x + width * index as f32, r.rect.min.y),
            vec2(width, r.rect.height()),
        );
        // The key is the label: one word, the way silvia's "Gate" and "Phase" are.
        let mut heading = String::from(*key);
        heading[..1].make_ascii_uppercase();
        painter.text(
            pos2(cell.center().x, cell.min.y),
            Align2::CENTER_TOP,
            &heading,
            label_font.clone(),
            r.theme.text_muted(),
        );
        painter.text(
            pos2(cell.center().x, cell.max.y),
            Align2::CENTER_BOTTOM,
            value,
            value_font.clone(),
            r.theme.readout(),
        );
        // Painted text is not in the accessibility tree, so the cell is a widget saying what
        // it reads — which is what a test, and anything driving the app, asks for.
        let name = format!("{} {value}", r.name(key));
        let w = r.ui.interact(
            cell,
            r.ui.id().with(("caption", r.id, *key)),
            Sense::hover(),
        );
        crate::ui::accessible(&w, WidgetType::Label, &name);
    }
    Vec::new()
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    // A node that has not ticked yet has an empty band rather than a drawn zero.
    let Some(ring) = r.live.trace else {
        return Vec::new();
    };
    // The port's own hue: what the line is a picture of is what the node publishes.
    let color = r
        .node
        .def
        .outputs
        .first()
        .map_or(r.theme.primary(), |o| r.theme.port(o.ty));
    crate::ui::scope::trace(r.ui, r.rect, ring, color, r.theme, r.zoom);
    Vec::new()
}
