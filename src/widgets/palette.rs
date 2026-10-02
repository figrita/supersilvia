// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cosine palette drawn as itself: a gradient strip, the three channel curves under it,
//! and the twelve coefficients as a grid three across.
//!
//! silvia's `cosinegradient.js` fills its `customArea` with exactly these three things — a
//! 16 px canvas of the ramp, a 72 px plot of the three waves, and a
//! `grid-template-columns: 28px 1fr 1fr 1fr` of `<s-number>`s under R, G and B headings — and
//! this is that, in world units and in the editor's own tokens. The geometry is silvia's
//! number for number: the strip's height, the plot's height, the label column, the gap
//! between cells and the four term rows down the side.
//!
//! **The twelve are the node's own values, not ports.** They are hidden controls: stored on
//! the node, written to the file, undone through the bus, resolved by the compiler as the
//! same `u_control_…` uniform a row's control was — and with no port, so nothing can cable
//! one and the node is a palette rather than a column of fifteen rows. Each cell is the inset
//! s-number the library already draws, by `RegionUi::number`, so a coefficient answers every
//! gesture a knob on a row does.
//!
//! The region does not claim the pointer: the cells register their own interacts and so take
//! the presses that land on them, and a drag anywhere else — the strip, the plot, the gaps —
//! carries the node, exactly as a drag on an audio scope does.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::cosinegradient;
use crate::ui::number;
use eframe::egui::{Align2, Color32, CornerRadius, Rect, Shape, Stroke, pos2, vec2};

/// silvia's own `padding: 4px` and `gap: 4px` on the palette's column.
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;
/// The label column, silvia's `28px`.
const LABEL: f32 = 28.0;
/// The R/G/B heading over the three columns, silvia's `font-size: 10px` line.
const HEAD: f32 = 12.0;
/// The ramp, silvia's `height: 16px`, and the wave plot, silvia's `height: 72px`.
const STRIP: f32 = 16.0;
const WAVES: f32 = 72.0;
/// The plot's own inset, silvia's `pad = 4`: where 0 and 1 are drawn, with room above and
/// below for a curve that reaches them.
const PLOT_PAD: f32 = 4.0;
/// How many samples the ramp and the curves are drawn from. silvia's canvas is 280 px wide
/// and samples one per pixel; this is that resolution at zoom 1, and the shapes are
/// interpolated between them at any other.
const SAMPLES: usize = 280;

/// The body this region needs: the label column, three s-numbers and the gaps between them,
/// inside silvia's own padding. 348, which is what its node comes out at.
pub const WIDTH: f32 = PAD * 2.0 + LABEL + 3.0 * number::WIDTH + 3.0 * GAP;

/// The whole of the node's own area: the ramp, the curves and the grid, in one band.
///
/// One region rather than three, because the three are one picture of one palette — the
/// strip is the plot's own x axis and the grid is what moves both — and a heading over any of
/// them would offer to hide half of a node whose whole job is to be looked at.
pub const PALETTE: RegionDef = RegionDef {
    size,
    show,
    width: Some(WIDTH),
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    PAD * 2.0 + STRIP + GAP + WAVES + GAP + HEAD + 4.0 * number::HEIGHT + 3.0 * GAP
}

/// The twelve as they stand on this node, in grid order.
fn coefficients(node: &Node) -> [[f32; 3]; 4] {
    std::array::from_fn(|row| {
        std::array::from_fn(|channel| {
            let key = cosinegradient::ROWS[row].1[channel];
            match node.controls.get(key) {
                Some(&crate::graph::ControlValue::Float(v)) => v,
                _ => 0.0,
            }
        })
    })
}

/// A channel of the palette as the editor paints it: the same 0-to-1-into-a-byte the swatch
/// and the color picker use, so one number means one color wherever it is drawn.
fn byte(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let coefficients = coefficients(r.node);
    // Where Time and Offset have carried the palette: what the shader reads, so the strip
    // drifts with the picture rather than beside it. Zero before the node has ticked, which is
    // where a palette at rest sits anyway.
    let drift = r.live.playhead.unwrap_or(0.0);
    let inner = r.rect.shrink(PAD * r.zoom);
    let mut top = inner.min.y;

    let strip = Rect::from_min_size(pos2(inner.min.x, top), vec2(inner.width(), STRIP * r.zoom));
    ramp(r, strip, &coefficients, drift);
    top = strip.max.y + GAP * r.zoom;

    let plot = Rect::from_min_size(pos2(inner.min.x, top), vec2(inner.width(), WAVES * r.zoom));
    waves(r, plot, &coefficients, drift);
    top = plot.max.y + GAP * r.zoom;

    // The R, G and B headings, in the three colors a channel is always drawn in here —
    // `Theme::band`'s own triple, which is silvia's convention and the one place a literal
    // color is right.
    let (zoom, left) = (r.zoom, inner.min.x + (LABEL + GAP) * r.zoom);
    let column = move |i: usize| left + (number::WIDTH + GAP) * zoom * i as f32;
    for (i, name) in ["R", "G", "B"].into_iter().enumerate() {
        r.caption(
            pos2(
                column(i) + number::WIDTH * r.zoom * 0.5,
                top + HEAD * r.zoom * 0.5,
            ),
            Align2::CENTER_CENTER,
            name,
            r.theme.band(i),
        );
    }
    top += HEAD * r.zoom;

    let mut out = Vec::new();
    for (row, (term, keys)) in cosinegradient::ROWS.iter().enumerate() {
        let y = top + (number::HEIGHT + GAP) * r.zoom * row as f32;
        r.caption(
            pos2(inner.min.x, y + number::HEIGHT * r.zoom * 0.5),
            Align2::LEFT_CENTER,
            term,
            r.theme.text_secondary(),
        );
        for (i, key) in keys.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(column(i), y),
                vec2(number::WIDTH * r.zoom, number::HEIGHT * r.zoom),
            );
            out.extend(r.number(cell, key));
        }
    }
    out
}

/// The palette across the whole range of the input, as the node would draw it.
fn ramp(r: &mut RegionUi<'_>, rect: Rect, coefficients: &[[f32; 3]; 4], drift: f32) {
    let painter = r.ui.painter();
    // A mesh, so the ramp is interpolated by the GPU between its samples rather than being a
    // row of hairline rectangles that seam at any zoom.
    let mut mesh = eframe::egui::Mesh::default();
    for i in 0..=SAMPLES {
        let t = i as f32 / SAMPLES as f32;
        let [red, green, blue] = cosinegradient::eval(coefficients, drift, t);
        let color = Color32::from_rgb(byte(red), byte(green), byte(blue));
        let x = rect.min.x + rect.width() * t;
        mesh.colored_vertex(pos2(x, rect.min.y), color);
        mesh.colored_vertex(pos2(x, rect.max.y), color);
        if i < SAMPLES {
            let left = i as u32 * 2;
            mesh.add_triangle(left, left + 1, left + 2);
            mesh.add_triangle(left + 1, left + 2, left + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
    // silvia's `border-radius: 2px` on the canvas. The ramp itself is a mesh and a mesh has
    // no radius, so the corner is the band's own hairline: the sharp radius is two points,
    // which is a corner softened rather than a shape cut.
    let radius = CornerRadius::same((f32::from(crate::ui::theme::RADIUS_SHARP) * r.zoom) as u8);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new((1.0 * r.zoom).max(1.0), r.theme.border_subtle()),
        eframe::egui::StrokeKind::Inside,
    );
}

/// The three channel curves over the same range, on silvia's own grid.
fn waves(r: &mut RegionUi<'_>, rect: Rect, coefficients: &[[f32; 3]; 4], drift: f32) {
    let radius = CornerRadius::same((f32::from(crate::ui::theme::RADIUS_SHARP) * r.zoom) as u8);
    let painter = r.ui.painter().clone();
    // silvia's plot sits on its own ground, a step darker than the node: the screen a picture
    // is drawn on when nothing is on it.
    painter.rect_filled(rect, radius, r.theme.screen_off());
    let hairline = (1.0 * r.zoom).max(1.0);
    // Quarters, silvia's `rgba(255,255,255,0.06)`, in the editor's own dimmest border.
    for i in 1..4 {
        let y = rect.min.y + rect.height() * i as f32 / 4.0;
        painter.line_segment(
            [pos2(rect.min.x, y), pos2(rect.max.x, y)],
            Stroke::new(hairline, r.theme.border_subtle().gamma_multiply(0.45)),
        );
    }
    // The 0 and 1 references, dashed as silvia dashes them.
    let pad = PLOT_PAD * r.zoom;
    for y in [rect.min.y + pad, rect.max.y - pad] {
        painter.add(Shape::dashed_line(
            &[pos2(rect.min.x, y), pos2(rect.max.x, y)],
            Stroke::new(hairline, r.theme.border_normal()),
            2.0 * r.zoom,
            4.0 * r.zoom,
        ));
    }
    let plot = rect.height() - pad * 2.0;
    for channel in 0..3 {
        let points: Vec<_> = (0..=SAMPLES)
            .map(|i| {
                let t = i as f32 / SAMPLES as f32;
                let value = cosinegradient::eval(coefficients, drift, t)[channel];
                pos2(
                    rect.min.x + rect.width() * t,
                    rect.min.y + pad + (1.0 - value) * plot,
                )
            })
            .collect();
        painter.add(Shape::line(
            points,
            Stroke::new(1.5 * r.zoom, r.theme.band(channel).gamma_multiply(0.8)),
        ));
    }
}
