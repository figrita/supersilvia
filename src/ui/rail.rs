// SPDX-License-Identifier: AGPL-3.0-or-later

//! The minimap under a linear workspace.
//!
//! The canvas at a smaller transform, and nothing else: the same layout the canvas made this
//! frame and the same `cable::Curve`. A node is its own rectangle at its own position and a
//! cable is the cable, so a feedback loop bows downward here because it bows downward there.
//! Nothing about the graph is re-described at this size, and nothing is laid out again.

use crate::graph::{NodeId, PortType};
use crate::ui::CanvasFrame;
use crate::ui::canvas::{self, Transform};
use eframe::egui::{Align2, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2};

/// Icon size on the map: as large as the node's box allows, between these.
///
/// The floor is low deliberately. A node with one input and one output is the shortest thing
/// in the library, and a threshold set for a comfortable glyph silently blanks a whole class
/// of node — which reads as a bug in the map rather than as a scale limit.
const ICON_MIN: f32 = 6.0;
const ICON_MAX: f32 = 14.0;

/// How far the rail outside the view sinks back: the alpha of the rail's own ground, laid
/// over everything the canvas is not showing.
const SHADE: f32 = 0.6;

/// Put a world point in the middle of the canvas. The caller clamps to the strip's ends
/// afterwards, so asking for the very edge of the graph is not a special case here.
fn center_on(transform: &mut Transform, canvas_rect: Rect, world: Pos2) {
    transform.pan = (canvas_rect.size() * 0.5 - world.to_vec2()) * transform.zoom;
}

/// Draw the minimap and let it move the view. True where it moved it, which outranks a glide
/// the view was on. Scrolling is view state, so nothing here reaches the command bus.
pub fn show(
    ui: &mut Ui,
    rail: Rect,
    canvas_rect: Rect,
    frame: &CanvasFrame<'_>,
    // Every node on the workspace as the canvas laid it out this frame, so a note is its
    // drawn height here too.
    layouts: &canvas::Layouts,
    transform: &mut Transform,
    // What the map frames. Along the strip it is `CanvasState::bounds`, the rectangle the
    // view is clamped to, so the map is a map of the strip and not only of what is on it and
    // a drag that holds the strip still holds the map's scale still with it. Across it, a
    // whole viewport's height whatever the nodes occupy, so a node's place on the map does
    // not move because another node collapsed.
    bounds: Option<Rect>,
) -> bool {
    let (graph, theme) = (frame.graph, frame.theme);
    // The minimap draws the same cables the canvas does, so it sags with them. At this scale
    // the difference is a pixel or two, and a schematic that disagrees with the thing it is a
    // schematic of is worse than one that is hard to read.
    let droop = frame.prefs.cable_droop;
    let painter = ui.painter_at(rail);
    painter.rect_filled(rail, 0.0, theme.bg_sunken());
    painter.hline(
        rail.x_range(),
        rail.min.y,
        Stroke::new(1.0, theme.border_subtle()),
    );

    let Some(bounds) = bounds else {
        return false;
    };
    // One scale for both axes, so the map is the graph's shape and not a squashed version of
    // it, and centerd in whatever room is left over — which on a short strip is a real
    // gutter at each end, because the height it has to fit is a whole viewport rather than
    // the nodes' own extent.
    let zoom = (rail.width() / bounds.width())
        .min(rail.height() / bounds.height())
        .min(1.0);
    let shown = bounds.size() * zoom;
    let mini = Transform {
        zoom,
        pan: (rail.size() - shown) * 0.5 - bounds.min.to_vec2() * zoom,
    };
    let origin = rail.min;

    // --- interaction, before painting, so a hovered node can change what is painted --------
    let mut hovered: Option<NodeId> = None;
    let mut go_to: Option<Pos2> = None;
    for laid in layouts.iter() {
        let (id, body) = (laid.id, mini.to_screen_rect(origin, laid.rect));
        let Some(node) = graph.get(id) else { continue };
        let response = ui.interact(
            // Never smaller than a pointer can hit, however far out the map is scaled.
            body.expand2(vec2(
                (6.0 - body.width() * 0.5).max(0.0),
                (5.0 - body.height() * 0.5).max(0.0),
            )),
            ui.id().with(("rail", id)),
            Sense::click(),
        );
        crate::ui::accessible(
            &response,
            eframe::egui::WidgetType::Button,
            format_args!("rail {}{}", node.def.slug, id),
        );
        if response.hovered() {
            hovered = Some(id);
        }
        if response.clicked() {
            go_to = Some(laid.rect.center());
        }
    }
    // Click or drag anywhere on the map and the view goes there, centerd on the pointer.
    // Not a relative pan: on a map this small a point is a long way, and "show me that" is
    // the only thing anyone means by clicking one.
    let bar = ui.interact(rail, ui.id().with("rail-bar"), Sense::click_and_drag());
    crate::ui::cursor(
        &bar,
        if bar.dragged() {
            eframe::egui::CursorIcon::Grabbing
        } else {
            eframe::egui::CursorIcon::PointingHand
        },
    );
    let mut sent = false;
    if (bar.clicked() || bar.dragged())
        && let Some(p) = bar.interact_pointer_pos()
    {
        center_on(transform, canvas_rect, mini.to_world(origin, p));
        sent = true;
    }
    if let Some(node) = go_to {
        center_on(transform, canvas_rect, node);
        sent = true;
    }

    // --- cables, behind the nodes, through the canvas's own routing -----------------------
    let slots = layouts.ports();
    for c in graph.connections() {
        let (Some(a), Some(b)) = (layouts.slot(c.from), layouts.slot(c.to)) else {
            continue;
        };
        let (from, to) = (
            mini.to_screen(origin, slots[a].center),
            mini.to_screen(origin, slots[b].center),
        );
        let ty = slots[a].ty;
        let lit = hovered.is_some_and(|h| h == c.from.node || h == c.to.node);
        let color = match hovered {
            Some(_) if !lit => theme.wire(ty).gamma_multiply(0.3),
            Some(_) => theme.wire(ty).gamma_multiply(1.5),
            None => theme.wire(ty),
        };
        let clear = layouts.clearance(c.from.node, c.to.node, &mini, origin);
        crate::ui::cable::Curve::new(from, to, ty, clear, droop)
            .paint(&painter, Stroke::new(1.0, color));
    }

    // --- nodes ------------------------------------------------------------------------------
    let mut bodies: Vec<Shape> = Vec::with_capacity(layouts.len());
    let mut kinds: Vec<PortType> = Vec::new();
    for laid in layouts.iter() {
        let (id, body) = (laid.id, mini.to_screen_rect(origin, laid.rect));
        let Some(node) = graph.get(id) else { continue };
        let alpha = if hovered.is_some_and(|h| h == id) {
            0.85
        } else {
            0.45
        };
        // Striped by everything the node produces, in declaration order. A node commonly
        // publishes more than one kind — a tap is a picture *and* the numbers measured from
        // it — so picking the first output would say something untrue about half the
        // library. The stripes are the port colors, which already derive from the four
        // anchors, so the map needs no palette of its own.
        kinds.clear();
        for p in &node.outputs {
            if !kinds.contains(&p.ty) {
                kinds.push(p.ty);
            }
        }
        if kinds.is_empty() {
            bodies.push(Shape::rect_filled(
                body,
                1.0,
                theme.primary_muted().gamma_multiply(alpha),
            ));
            continue;
        }
        // Stacked, because that is how the node stacks: outputs are rows down the body, so
        // a band per kind sits where those rows sit.
        let band = body.height() / kinds.len() as f32;
        for (n, ty) in kinds.iter().enumerate() {
            let at = body.min.y + band * n as f32;
            bodies.push(Shape::rect_filled(
                Rect::from_min_size(
                    eframe::egui::pos2(body.min.x, at),
                    eframe::egui::vec2(body.width(), band),
                ),
                1.0,
                theme.port(*ty).gamma_multiply(alpha),
            ));
        }
    }
    painter.extend(bodies);
    for laid in layouts.iter() {
        let body = mini.to_screen_rect(origin, laid.rect);
        let Some(node) = graph.get(laid.id) else {
            continue;
        };
        let size = (body.width().min(body.height()) * 0.9).min(ICON_MAX);
        if size < ICON_MIN {
            continue;
        }
        let icon = node.def.icon;
        painter.text(
            body.center(),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(size),
            theme.text_primary(),
        );
    }

    // --- the window over it -------------------------------------------------------------
    let view = Rect::from_min_size(
        mini.to_screen(origin, Pos2::new(-transform.pan.x, -transform.pan.y)),
        canvas_rect.size() * zoom,
    );
    let view = view.intersect(rail);
    // Everything off the end of the view sinks back into the rail's own ground, so the strip
    // the window is on reads at a glance rather than having to be found by its outline.
    let shade = theme.bg_sunken().gamma_multiply(SHADE);
    for outside in [
        Rect::from_min_max(rail.min, Pos2::new(view.min.x, rail.max.y)),
        Rect::from_min_max(Pos2::new(view.max.x, rail.min.y), rail.max),
        Rect::from_min_max(
            Pos2::new(view.min.x, rail.min.y),
            Pos2::new(view.max.x, view.min.y),
        ),
        Rect::from_min_max(
            Pos2::new(view.min.x, view.max.y),
            Pos2::new(view.max.x, rail.max.y),
        ),
    ] {
        if outside.is_positive() {
            painter.rect_filled(outside, 0.0, shade);
        }
    }
    painter.rect_stroke(
        view,
        1.0,
        Stroke::new(1.0, theme.primary()),
        StrokeKind::Inside,
    );
    painter.rect_filled(view, 1.0, theme.primary().gamma_multiply(0.08));
    sent
}
