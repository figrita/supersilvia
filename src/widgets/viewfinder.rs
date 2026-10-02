// SPDX-License-Identifier: AGPL-3.0-or-later

//! The camcorder's viewfinder: a black square you aim a feedback loop in.
//!
//! silvia's `camcordercrt.js` draws a 200 square under its knobs — the tunnel the feedback
//! camera is looking down, a crosshair where the camera is pointed, a tilt meter and four
//! corner brackets — and a hand drags it to drift, shift-drags to tilt, scrolls to zoom,
//! shift-scrolls to rotate and double-clicks to put it all back.
//!
//! Here it is a region that **claims the pointer**, so every press inside it is the
//! viewfinder's and none of it starts a node drag. What it writes are the node's own six
//! aiming controls, as [`RegionEvent::Controls`] through the command bus: `SetControls`
//! coalesces consecutive writes of the same keys, so a drag is one undo step and the reset is
//! one of its own.
//!
//! The six keys are the node's, named once in `nodes::camcordercrt` and read from there, so
//! a rename cannot leave the region writing a control that is not there. A picture of six
//! numbers has to know which six.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node};
use crate::nodes::camcordercrt::{DRIFT_X, DRIFT_Y, ROTATION, TILT_X, TILT_Y, ZOOM};
use crate::ui::canvas;
use eframe::egui::{Color32, CornerRadius, Pos2, Rect, Stroke, pos2, vec2};

/// A square viewfinder, flush to the body's bottom corners as a picture is.
pub const VIEWFINDER: RegionDef = RegionDef {
    size,
    show,
    claims_pointer: true,
    ..RegionDef::EMPTY
};

/// How much of the square the tunnel's outermost rectangle takes, silvia's 20 px margin in a
/// 200 canvas.
const MARGIN: f32 = 0.1;
/// How many rectangles deep the tunnel is drawn, and how much of one survives into the next.
const DEPTH: usize = 50;
const FALLOFF: f32 = 0.88;
/// A drift of one unit, in squares: silvia's `driftPx = w * 5`.
const DRIFT_SPAN: f32 = 5.0;
/// How far the tilt meter reads, which is the tilt controls' own end.
const TILT_MAX: f32 = 2.0;

/// silvia's canvas is a square of the body's width, and the square is what makes the tunnel
/// legible: a band any shorter turns fifty nested rectangles into a smear.
fn size(node: &Node) -> f32 {
    canvas::node_width(node)
}

/// One control's value on this node, or its definition's default where nothing has been
/// written yet.
fn value(node: &Node, key: &str) -> f32 {
    if let Some(ControlValue::Float(v)) = node.controls.get(key) {
        return *v;
    }
    default(node, key)
}

fn default(node: &Node, key: &str) -> f32 {
    match node.def.input(key).map(|p| &p.control) {
        Some(crate::nodes::Control::Number { default, .. }) => *default,
        _ => 0.0,
    }
}

/// A control written to, fitted to the range it actually has on this node.
fn write(node: &Node, key: &'static str, v: f32) -> (&'static str, ControlValue) {
    let fitted = match crate::nodes::control_range(node.def, node, key) {
        Some(range) => v.clamp(range.min, range.max),
        None => v,
    };
    (key, ControlValue::Float(fitted))
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    paint(r);
    gestures(r)
}

// ------------------------------------------------------------------------------ the picture

/// The tunnel, the crosshair, the tilt meter and the brackets, over the ground a picture on a
/// node has.
fn paint(r: &mut RegionUi<'_>) {
    let radius = r.corner as u8;
    r.ui.painter().rect_filled(
        r.rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: radius,
            se: radius,
        },
        r.theme.screen_off(),
    );
    let side = r.rect.width().min(r.rect.height());
    if side < 24.0 {
        return;
    }
    let square = Rect::from_center_size(r.rect.center(), vec2(side, side));
    // A drift of a tenth carries a rectangle half a square per turn of the tunnel, and by the
    // fiftieth it is nowhere near the node. What is painted past the band is the canvas, so
    // the band is the edge of the picture exactly as a browser canvas is silvia's.
    let outside = r.ui.clip_rect();
    r.ui.set_clip_rect(outside.intersect(r.rect));
    let node = r.node;
    let (zoom, rot) = (value(node, ZOOM), value(node, ROTATION));
    let (tilt_x, tilt_y) = (value(node, TILT_X), value(node, TILT_Y));
    let (drift_x, drift_y) = (value(node, DRIFT_X), value(node, DRIFT_Y));

    tunnel(r, square, zoom, rot, tilt_x, tilt_y, drift_x, drift_y);
    crosshair(r, square, drift_x, drift_y);
    tilt_meter(r, square, tilt_x, tilt_y);
    brackets(r, square);
    r.ui.set_clip_rect(outside);
}

/// The nested rectangles the camera is looking down, each one the last put through the very
/// transform the shader puts a sample through.
#[allow(clippy::too_many_arguments)]
fn tunnel(
    r: &mut RegionUi<'_>,
    square: Rect,
    zoom: f32,
    rot: f32,
    tilt_x: f32,
    tilt_y: f32,
    drift_x: f32,
    drift_y: f32,
) {
    let center = square.center();
    let half = square.size() * (0.5 - MARGIN);
    let drift_px = square.width() * DRIFT_SPAN;
    let (cr, sr) = (rot.cos(), rot.sin());
    let mut corners = [
        vec2(-half.x, -half.y),
        vec2(half.x, -half.y),
        vec2(half.x, half.y),
        vec2(-half.x, half.y),
    ];
    for i in 0..DEPTH {
        let fade = FALLOFF.powi(i as i32);
        if fade < 0.015 {
            break;
        }
        let color = if i == 0 {
            r.theme.text_primary()
        } else {
            r.theme.primary()
        };
        let width = if i == 0 { 2.0 } else { 1.0 };
        let points: Vec<Pos2> = corners.iter().map(|c| center + *c).collect();
        r.ui.painter().add(eframe::egui::Shape::closed_line(
            points,
            Stroke::new(width * r.zoom, color.gamma_multiply(fade)),
        ));
        for c in &mut corners {
            let (mut x, mut y) = (c.x * zoom, c.y * zoom);
            let (rx, ry) = (x * cr - y * sr, x * sr + y * cr);
            x = rx;
            y = ry;
            // The shader's own perspective divide, over the same normalized coordinates.
            let pw =
                (1.0 + (-x / square.width()) * tilt_x + (y / square.height()) * tilt_y).max(0.001);
            x /= pw;
            y /= pw;
            *c = vec2(x - drift_x * drift_px, y + drift_y * drift_px);
        }
    }
}

/// Where the camera is pointed: silvia's red ring and cross, at the drift offset.
fn crosshair(r: &mut RegionUi<'_>, square: Rect, drift_x: f32, drift_y: f32) {
    let drift_px = square.width() * DRIFT_SPAN;
    let at = square.center() + vec2(drift_x * drift_px, -drift_y * drift_px);
    let arm = square.width() * 0.045;
    let stroke = Stroke::new(1.5 * r.zoom, r.theme.accent());
    r.ui.painter()
        .line_segment([at - vec2(arm, 0.0), at + vec2(arm, 0.0)], stroke);
    r.ui.painter()
        .line_segment([at - vec2(0.0, arm), at + vec2(0.0, arm)], stroke);
    r.ui.painter().circle_stroke(at, arm * 0.55, stroke);
}

/// The tilt, as a dot on a small crosshair in the top-right corner.
fn tilt_meter(r: &mut RegionUi<'_>, square: Rect, tilt_x: f32, tilt_y: f32) {
    let ring = square.width() * 0.06;
    let at = pos2(square.max.x - ring * 2.0, square.min.y + ring * 2.0);
    let faint = r.theme.text_muted();
    let stroke = Stroke::new(1.0 * r.zoom, faint);
    r.ui.painter().circle_stroke(at, ring, stroke);
    r.ui.painter()
        .line_segment([at - vec2(ring, 0.0), at + vec2(ring, 0.0)], stroke);
    r.ui.painter()
        .line_segment([at - vec2(0.0, ring), at + vec2(0.0, ring)], stroke);
    let dot = at
        + vec2(
            (tilt_x / TILT_MAX).clamp(-1.0, 1.0) * ring,
            -(tilt_y / TILT_MAX).clamp(-1.0, 1.0) * ring,
        );
    r.ui.painter()
        .circle_filled(dot, ring * 0.25, r.theme.text_secondary());
}

/// The four corner brackets that say this is a viewfinder and not a picture.
fn brackets(r: &mut RegionUi<'_>, square: Rect) {
    let len = square.width() * 0.07;
    let off = square.width() * 0.03;
    let stroke = Stroke::new(
        1.5 * r.zoom,
        Color32::from_rgba_unmultiplied(255, 255, 255, 128),
    );
    let inner = square.shrink(off);
    for (corner, dx, dy) in [
        (inner.left_top(), 1.0, 1.0),
        (inner.right_top(), -1.0, 1.0),
        (inner.right_bottom(), -1.0, -1.0),
        (inner.left_bottom(), 1.0, -1.0),
    ] {
        r.ui.painter()
            .line_segment([corner, corner + vec2(len * dx, 0.0)], stroke);
        r.ui.painter()
            .line_segment([corner, corner + vec2(0.0, len * dy)], stroke);
    }
}

// ----------------------------------------------------------------------------- the gestures

/// How far a point of drag moves the drift and the tilt, and how far one notch of the wheel
/// moves the zoom and the rotation — silvia's four rates, per point rather than per pixel.
const DRIFT_RATE: f32 = 0.0005;
const TILT_RATE: f32 = 0.005;
const WHEEL_STEP: f32 = 0.003;

/// Drag to drift, shift-drag to tilt, scroll to zoom, shift-scroll to rotate, double-click to
/// reset — every one of them a write to a control the node already has.
fn gestures(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(grab) = r.grab.clone() else {
        return Vec::new();
    };
    let shift = r.ui.input(|i| i.modifiers.shift);
    let node = r.node;

    if grab.double_clicked() {
        let values = [DRIFT_X, DRIFT_Y, TILT_X, TILT_Y, ZOOM, ROTATION]
            .into_iter()
            .map(|key| (key, ControlValue::Float(default(node, key))))
            .collect();
        return vec![RegionEvent::Controls(values)];
    }

    if grab.dragged() {
        // In world units, so the sensitivity is the same at any canvas zoom.
        let d = grab.drag_delta() / r.zoom.max(0.01);
        if d != vec2(0.0, 0.0) {
            let (kx, ky, rate) = if shift {
                (TILT_X, TILT_Y, TILT_RATE)
            } else {
                (DRIFT_X, DRIFT_Y, DRIFT_RATE)
            };
            return vec![RegionEvent::Controls(vec![
                write(node, kx, value(node, kx) + d.x * rate),
                write(node, ky, value(node, ky) - d.y * rate),
            ])];
        }
        return Vec::new();
    }

    if grab.hovered() {
        // Taken rather than read, exactly as a number control takes it: the canvas zooms on
        // whatever wheel input is left at the end of the frame.
        let scroll = r.ui.ctx().input_mut(|i| {
            let v = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta.y = 0.0;
            v
        });
        if scroll != 0.0 {
            let key = if shift { ROTATION } else { ZOOM };
            return vec![RegionEvent::Controls(vec![write(
                node,
                key,
                value(node, key) + scroll.signum() * WHEEL_STEP,
            )])];
        }
    }
    Vec::new()
}
