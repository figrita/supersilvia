// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sequencers' own grid: four lanes of steps with the lit ones filled and the playhead
//! walking across them. One grid, two regions — [`STEPS`], `euclideanrhythm`'s, reads it over
//! the three numbers that shape each lane, and [`GRID`], `stepsequencer`'s, is edited a cell
//! at a time.
//!
//! silvia's `euclideanrhythm.js` and `stepsequencer.js` draw the same cell under two class
//! names, `.euc-step` and `.seq-step`: 16 px square with a 4 px gap, a brighter cell on every
//! fourth so the bar reads, the lit ones filled and a glow on the step the sequencer is on.
//! This is that, in world units and the editor's own tokens, with a lane's own step count in
//! place of silvia's fixed sixteen — so a 64-step Euclidean lane draws 64 narrower cells
//! rather than being unable to say what it is playing. The two nodes space their lanes
//! alike, which is `.euc-lane`'s 4 px margin; silvia's `.seq-lane` has none, and one grid
//! wins over two spacings.
//!
//! **Euclidean Rhythm's grid is read-only.** Its cells are a picture of what the numbers make;
//! the numbers are what a hand moves, the node's own hidden controls, drawn here by
//! `RegionUi::number` as the inset s-number every other number on a node is.
//!
//! **The Step Sequencer's grid is the pattern**, a `ValueKind::Cells` read straight off the
//! `Node`: a click on a cell turns it over as one `SetValue`, and Clear writes the whole grid
//! unlit. Neither region claims the pointer — a cell senses a click and nothing more, so a
//! drag that starts on one, or in the gaps, carries the node.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node, Value};
use crate::nodes::ValueKind;
use crate::nodes::euclideanrhythm::{self, LaneParams};
use crate::ui::number;
use eframe::egui::{
    Align2, CornerRadius, CursorIcon, Rect, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType,
    pos2, vec2,
};

/// silvia's `.euc-container { padding: 0.5rem; gap: 0.5rem }`.
const PAD: f32 = 8.0;
const GAP: f32 = 8.0;
/// silvia's `.euc-lane { gap: 0.25rem; margin-bottom: 0.25rem }`, which is also the gap
/// between two cells and between two lane slabs.
const CELL_GAP: f32 = 4.0;
/// silvia's `.euc-step { width: 16px; height: 16px }`, which a lane longer than silvia's
/// sixteen shrinks below rather than running off the node.
const CELL: f32 = 16.0;
/// The lane's own row in the grid: a cell and the margin under it.
const LANE: f32 = CELL + CELL_GAP;
/// silvia's `.euc-lane-label { min-width: 50px }`.
const LABEL: f32 = 50.0;
/// The Steps / Pulses / Rotate heading over the three columns.
const HEAD: f32 = 12.0;
/// A lane's slab: the s-number and silvia's `padding: 0.25rem` above and below it.
const SLAB: f32 = number::HEIGHT + CELL_GAP;
/// The Clear button, silvia's `.euc-controls button { padding: 5px }` around one tiny line.
const BUTTON: f32 = 24.0;

/// The body this region needs: the lane label, three s-numbers, and the gaps between them
/// inside silvia's own padding.
pub const WIDTH: f32 = PAD * 2.0 + LABEL + 3.0 * number::WIDTH + 3.0 * CELL_GAP;

/// The node's own area: the grid, the four lane slabs and the Clear button.
pub const STEPS: RegionDef = RegionDef {
    size,
    show,
    width: Some(WIDTH),
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    PAD * 2.0 + LANE * 4.0 + GAP + HEAD + SLAB * 4.0 + CELL_GAP * 3.0 + GAP + BUTTON
}

/// The three numbers of one lane, in the order the columns stand in.
fn lane_keys(lane: usize) -> [&'static str; 3] {
    [
        euclideanrhythm::STEPS_KEYS[lane],
        euclideanrhythm::PULSES_KEYS[lane],
        euclideanrhythm::ROTATION_KEYS[lane],
    ]
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let inner = r.rect.shrink(PAD * r.zoom);
    let lanes: [LaneParams; 4] = std::array::from_fn(|i| LaneParams::of(r.node, i));
    // Where the sequencer has got to, in absolute steps. `None` before anything has stepped
    // and after a reset, which is silvia's `currentStep = -1`: nothing lit.
    let playhead = r.live.playhead.map(|p| p as i64);

    let mut top = inner.min.y;
    for lane in &lanes {
        let row = lane_row(r, inner, top);
        for (step, at) in cells(row, lane.steps.max(1)).enumerate() {
            let step = step as u32;
            let pulse = euclideanrhythm::is_pulse(i64::from(step), lane);
            paint_cell(r, at, step, pulse, on_playhead(playhead, lane.steps, step));
        }
        top += LANE * r.zoom;
    }
    top += GAP * r.zoom;

    let (zoom, left) = (r.zoom, inner.min.x + (LABEL + CELL_GAP) * r.zoom);
    let column = move |i: usize| left + (number::WIDTH + CELL_GAP) * zoom * i as f32;
    for (i, name) in ["Steps", "Pulses", "Rotate"].into_iter().enumerate() {
        r.caption(
            pos2(
                column(i) + number::WIDTH * r.zoom * 0.5,
                top + HEAD * r.zoom * 0.5,
            ),
            Align2::CENTER_CENTER,
            name,
            r.theme.text_muted(),
        );
    }
    top += HEAD * r.zoom;

    let mut out = Vec::new();
    for lane in 0..4 {
        let slab = Rect::from_min_size(pos2(inner.min.x, top), vec2(inner.width(), SLAB * r.zoom));
        // silvia's `.euc-lane-controls` slab, which is what makes a lane's three numbers read
        // as one lane's rather than as a grid of twelve.
        r.ui.painter().rect_filled(
            slab,
            CornerRadius::same((f32::from(crate::ui::theme::RADIUS_SHARP) * r.zoom) as u8),
            r.theme.bg_secondary(),
        );
        r.caption(
            pos2(slab.min.x + CELL_GAP * r.zoom, slab.center().y),
            Align2::LEFT_CENTER,
            &format!("Lane {}", lane + 1),
            r.theme.text_secondary(),
        );
        for (i, key) in lane_keys(lane).into_iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(column(i), slab.center().y - number::HEIGHT * r.zoom * 0.5),
                vec2(number::WIDTH * r.zoom, number::HEIGHT * r.zoom),
            );
            out.extend(r.number(cell, key));
        }
        top += (SLAB + CELL_GAP) * r.zoom;
    }
    top += (GAP - CELL_GAP) * r.zoom;

    // silvia's third body button. Start and Reset are action inputs here and are already
    // buttons on their own rows; Clear is not something a cable fires, so it lives here.
    let button = Rect::from_min_size(pos2(inner.min.x, top), vec2(inner.width(), BUTTON * r.zoom));
    if super::button(r, button, "Clear", "clear") {
        out.push(RegionEvent::Controls(
            euclideanrhythm::PULSES_KEYS
                .iter()
                .map(|&key| (key, ControlValue::Float(0.0)))
                .collect(),
        ));
    }
    out
}

/// The band one lane's cells stand in, `top` being where the lane starts.
fn lane_row(r: &RegionUi<'_>, inner: Rect, top: f32) -> Rect {
    Rect::from_min_size(
        pos2(inner.min.x, top + CELL_GAP * r.zoom * 0.5),
        vec2(inner.width(), CELL * r.zoom),
    )
}

/// Whether the playhead, an absolute step, stands on `step` of a lane `steps` long.
fn on_playhead(playhead: Option<i64>, steps: u32, step: u32) -> bool {
    playhead.is_some_and(|p| p.rem_euclid(i64::from(steps.max(1))) == i64::from(step))
}

/// Where each of a lane's `steps` cells goes in its row, centred.
///
/// silvia's cells are 16 with a 4 gap, which is what a sixteen-step lane draws here; past
/// that the cell and its gap shrink together, so the lane keeps its proportions and stays
/// inside the body rather than running off it.
fn cells(row: Rect, steps: u32) -> impl Iterator<Item = Rect> {
    let unit = row.width() / (steps as f32 * 1.25 - 0.25);
    let cell = unit.min(row.height());
    let gap = cell * 0.25;
    let span = cell * steps as f32 + gap * (steps - 1) as f32;
    let left = row.center().x - span * 0.5;
    (0..steps).map(move |step| {
        Rect::from_min_size(
            pos2(
                left + (cell + gap) * step as f32,
                row.center().y - cell * 0.5,
            ),
            vec2(cell, cell),
        )
    })
}

/// One cell of the grid: lit or not, the downbeat brighter, the playhead ringed.
fn paint_cell(r: &RegionUi<'_>, at: Rect, step: u32, lit: bool, on_playhead: bool) {
    let radius = CornerRadius::same((3.0 * r.zoom).round() as u8);
    // A lit step is the gate this lane will fire, so it is drawn in the action port's own
    // hue: the cell and the cable it opens are the same color. An unlit one is the body's
    // interactive ground, a step brighter on every fourth so the bar can be counted —
    // silvia's `nth-child(4n+1)`.
    let fill = if lit {
        r.theme.port(crate::graph::PortType::Action)
    } else if step.is_multiple_of(4) {
        r.theme.bg_hover()
    } else {
        r.theme.bg_interactive()
    };
    r.ui.painter().rect(
        at,
        radius,
        fill,
        Stroke::new(
            (1.0 * r.zoom).max(1.0),
            if on_playhead {
                r.theme.text_primary()
            } else {
                r.theme.border_normal()
            },
        ),
        StrokeKind::Inside,
    );
    // silvia's `box-shadow: 0 0 5px #fffc` on the step it is standing on: one ring of
    // light outside the cell, which is what makes the playhead findable at a glance
    // across four lanes of different lengths.
    if on_playhead {
        r.ui.painter().rect_stroke(
            at.expand(1.5 * r.zoom),
            radius,
            Stroke::new(
                (1.0 * r.zoom).max(1.0),
                r.theme.text_primary().gamma_multiply(0.35),
            ),
            StrokeKind::Outside,
        );
    }
}

/// The body the Step Sequencer's grid needs: silvia's sixteen cells and their gaps inside
/// silvia's own padding.
pub const GRID_WIDTH: f32 = PAD * 2.0 + 16.0 * CELL + 15.0 * CELL_GAP;

/// The Step Sequencer's own area: the grid a hand lights, and Clear under it.
pub const GRID: RegionDef = RegionDef {
    size: grid_size,
    show: grid_show,
    width: Some(GRID_WIDTH),
    ..RegionDef::EMPTY
};

/// The grid this node declares as a value: its key, and how many lanes of how many steps.
fn declared(node: &Node) -> Option<(&'static str, usize, usize)> {
    node.def.values.iter().find_map(|v| match v.kind {
        ValueKind::Cells { lanes, steps } => Some((v.key, usize::from(lanes), usize::from(steps))),
        _ => None,
    })
}

fn grid_size(node: &Node) -> f32 {
    let lanes = declared(node).map_or(0, |(_, lanes, _)| lanes);
    PAD * 2.0 + LANE * lanes as f32 + GAP + BUTTON
}

fn grid_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some((key, lanes, steps)) = declared(r.node) else {
        return Vec::new();
    };
    let pattern = r.node.values.get(key);
    let lit = |lane: usize, step: usize| pattern.is_some_and(|p| p.lit(lane, step));
    let playhead = r.live.playhead.map(|p| p as i64);
    let inner = r.rect.shrink(PAD * r.zoom);

    let mut out = Vec::new();
    let mut top = inner.min.y;
    for lane in 0..lanes {
        let row = lane_row(r, inner, top);
        for (step, at) in cells(row, steps.max(1) as u32).enumerate() {
            let on = lit(lane, step);
            let name = r.name(&format!("lane{}.step{}", lane + 1, step + 1));
            let w =
                r.ui.interact(at, r.ui.id().with(("cell", &name)), Sense::click());
            w.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, &name));
            // silvia's `.seq-step { cursor: pointer }`.
            let w = w.on_hover_cursor(CursorIcon::PointingHand);
            paint_cell(
                r,
                at,
                step as u32,
                on,
                on_playhead(playhead, steps as u32, step as u32),
            );
            if w.clicked() {
                out.push(RegionEvent::Value {
                    key,
                    value: Value::grid(lanes, steps, |l, s| lit(l, s) != (l == lane && s == step)),
                });
            }
        }
        top += LANE * r.zoom;
    }
    top += GAP * r.zoom;

    // silvia's Clear. Start and Reset are action inputs here and are already buttons on their
    // own rows; Clear is an edit to the pattern rather than a gate, so it lives here.
    let button = Rect::from_min_size(pos2(inner.min.x, top), vec2(inner.width(), BUTTON * r.zoom));
    if super::button(r, button, "Clear", "clear") {
        out.push(RegionEvent::Value {
            key,
            value: Value::grid(lanes, steps, |_, _| false),
        });
    }
    out
}
