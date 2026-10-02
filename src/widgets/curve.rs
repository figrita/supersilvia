// SPDX-License-Identifier: AGPL-3.0-or-later

//! The picture of a curve a hand performed: `automation`'s recording, drawn as it is made
//! and as it is played back.
//!
//! silvia draws the same band on the same node, in a canvas under its five knobs. What is
//! different here is where the points come from. The **saved** curve is one of the node's own
//! values — `ValueKind::Points`, read straight off the `Node`, so a patch just opened draws
//! its recording before anything has ticked. What the tick has that the document does not is
//! a recording in progress and where a playhead is, and that arrives as `Live::curve`.
//!
//! Read-only, so the region does not claim the pointer and a hand carries the node by its
//! curve exactly as by any other part of the body. Nothing here knows which node it is
//! drawing for: the bounds are the `ValueDef`'s own.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{Node, Point};
use crate::nodes::ValueKind;
use crate::ui::canvas;
use eframe::egui::{Sense, Shape, Stroke, WidgetType, pos2, vec2};

/// The curve band, flush to the body's bottom corners as a trace is, and the same 300 wide
/// body: a performance drawn in a column of 200 is a column.
pub const CURVE: RegionDef = RegionDef {
    size,
    show,
    width: Some(canvas::SCOPE_NODE_WIDTH),
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    canvas::CURVE_HEIGHT
}

/// The value this node declares as a curve, with the ends it is drawn against.
fn declared(node: &Node) -> Option<(&'static str, f32, f32)> {
    node.def.values.iter().find_map(|v| {
        let ValueKind::Points { min, max } = v.kind else {
            return None;
        };
        Some((v.key, min, max))
    })
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some((key, min, max)) = declared(r.node) else {
        return Vec::new();
    };
    // The tick's own points while it has any — a recording in progress is ahead of the
    // document by everything performed since it started — and the saved value otherwise.
    let saved = r.node.values.get(key).and_then(crate::graph::Value::points);
    let points: &[Point] = match r.live.curve {
        Some(curve) => &curve.points,
        None => saved.unwrap_or(&[]),
    };
    // Seconds across the band. The tick knows the duration it is recording into; a curve read
    // off a file has only what is in it, so it is drawn to its own last point.
    let span = r
        .live
        .curve
        .map(|c| c.span)
        .filter(|s| *s > 0.0)
        .unwrap_or_else(|| points.last().map_or(1.0, |p| p.time))
        .max(1e-3);
    let recording = r.live.curve.is_some_and(|c| c.recording);

    // Painted lines are not in the accessibility tree, so the band says what it holds.
    let name = format!("{} {} points", r.name("curve"), points.len());
    let w =
        r.ui.interact(r.rect, r.ui.id().with(("curve", r.id)), Sense::hover());
    crate::ui::accessible(&w, WidgetType::Other, &name);

    if r.rect.height() < 8.0 || r.zoom < 0.4 {
        return Vec::new();
    }
    let painter = r.ui.painter().clone();
    painter.hline(
        r.rect.x_range(),
        r.rect.max.y - 0.5,
        Stroke::new(1.0, r.theme.border_subtle()),
    );
    // Margin top and bottom, so a point at either end is a whole line rather than one the
    // band's own edge clipped.
    let band = r.rect.shrink2(vec2(0.0, 4.0 * r.zoom));
    let span_value = if (max - min).abs() < 1e-4 {
        1.0
    } else {
        max - min
    };
    let at = |p: &Point| {
        let x = band.min.x + band.width() * (p.time / span).clamp(0.0, 1.0);
        let y = band.max.y - band.height() * ((p.value - min) / span_value).clamp(0.0, 1.0);
        pos2(x, y)
    };

    // Where the playhead is, under the line: a recording fills from the left and a playback
    // runs across what is already there.
    if let Some(head) = r.live.curve.and_then(|c| c.head) {
        let x = band.min.x + band.width() * (head / span).clamp(0.0, 1.0);
        painter.vline(
            x,
            r.rect.y_range(),
            Stroke::new(1.0 * r.zoom, r.theme.accent()),
        );
    }

    // The port's own hue, as a trace is drawn in it: the line is a picture of what the node
    // publishes. Brighter while a hand is performing it — the accent is what this editor says
    // *this is happening now* with, on the playhead beside it too.
    let color = if recording {
        r.theme.accent()
    } else {
        r.theme.port(crate::graph::PortType::UniformNumber)
    };
    match points {
        // A performance of one value is a point, and a point is not a line: it draws as the
        // level it is, across the whole band, which is what the node is publishing.
        [only] => {
            let y = at(only).y;
            painter.hline(
                band.x_range(),
                y,
                Stroke::new(1.5 * r.zoom, color.gamma_multiply(0.7)),
            );
        }
        [] => {}
        many => {
            painter.add(Shape::line(
                many.iter().map(at).collect(),
                Stroke::new(1.5 * r.zoom, color),
            ));
        }
    }
    Vec::new()
}
