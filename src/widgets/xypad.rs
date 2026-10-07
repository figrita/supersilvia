// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xypad`'s square: a puck a hand throws, with its X and Y under it and silvia's presets
//! under those.
//!
//! silvia's `xypad.js` fills its custom area with a 350 canvas at the body's full width, the
//! two readouts, a bar of nine numbered preset buttons, its ten s-numbers and a line of help.
//! This is that, in world units and the editor's own tokens, with the ten knobs moved onto the
//! node's rows where a cable can reach them. On the square: silvia's ground, its dashed
//! center lines and its border, the wells — a ring, a dot and a cross for a gravity well, a
//! dashed circle and a string for a tether — the trail fading in behind the puck, a line for
//! its velocity, the slingshot's band while one is drawn back, and the puck.
//!
//! **The square claims the pointer**, so a press there is the puck's and never a node drag.
//! A primary press puts the puck under the hand and holds it; a drag then pulls a slingshot
//! back, or carries the puck in Cursor mode; letting go launches it, or lets it fly on at the
//! speed it was carried at. Every one of those is a write of the node's four hand controls
//! through `SetControls`, the same four keys in the same order every frame, so a whole
//! gesture is one undo step. A secondary press on a well takes it away, and anywhere else
//! pulls a new one out, which lands when it is let go: those are [`Touch`]es, since a well is
//! the node's runtime state and not the document's. While the primary button is down the
//! region says so as [`RegionEvent::Held`], and the tick keeps the puck still.
//!
//! **The readouts are the two hand controls**, drawn as the inset s-number every number on
//! a node is, so each types, resets, learns a MIDI binding and wears its mark as a knob on a
//! row does. silvia's show where the puck is; the rows for X and Y do that here, and these
//! say where the hand put it.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node, PortType};
use crate::nodes::cpu::{Puck, Pull, Touch, Well};
use crate::nodes::xypad::{self, HAND, PRESETS};
use crate::ui::{canvas, number, theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};

/// silvia's `padding: 0.5rem; gap: 0.5rem`.
const PAD: f32 = 8.0;
const GAP: f32 = 8.0;
/// The X and Y captions over the two numbers, silvia's `0.7rem` labels.
const CAPTION: f32 = 12.0;
/// A preset button, silvia's `padding: 2px 0` around a `0.85rem` digit, and the `gap: 2px`
/// between two.
const BUTTON: f32 = 20.0;
const BUTTON_GAP: f32 = 2.0;
/// The line of help at the foot, silvia's `0.75rem`.
const HINT: f32 = 12.0;
/// silvia's canvas corner, `border-radius: 4px`.
const CORNER: f32 = 4.0;
/// How near a secondary press has to be to a well to take it away, silvia's 14 px.
const WELL_HIT: f32 = 14.0;
/// A secondary drag shorter than this is a click, and drops silvia's default well.
const WELL_CLICK: f32 = 5.0;
/// silvia's slingshot, launching at five times the pull.
const SLING: f32 = 5.0;
/// silvia's cursor throw: a move is a sixtieth of a second of velocity.
const THROW: f32 = 60.0;

/// The body a pad asks for: the audio scope's, which is the widest region there is, so the
/// square is as large as any picture on a node.
pub const WIDTH: f32 = canvas::SCOPE_NODE_WIDTH;

pub const XYPAD: RegionDef = RegionDef {
    size,
    show,
    claims_pointer: true,
    width: Some(WIDTH),
};

/// The square at the body's own width inside silvia's padding, then the readouts, the
/// presets and the line of help.
fn size(node: &Node) -> f32 {
    let side = canvas::node_width(node) - 2.0 * PAD;
    PAD + side + GAP + CAPTION + number::HEIGHT + GAP + BUTTON + GAP + HINT + PAD
}

/// What a hand is doing on the pad, kept across the frames of one gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hand {
    /// The primary button on the puck: where it went down, where the pointer has pulled to,
    /// and the last place the puck was written to.
    Puck {
        anchor: [f32; 2],
        pull: [f32; 2],
        last: [f32; 2],
    },
    /// The secondary button pulling a well out of `origin`, which was pressed at `from`.
    Well { origin: [f32; 2], from: Pos2 },
    /// The secondary button went down on a well and took it away.
    Spent,
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let z = r.zoom;
    let inner = r.rect.shrink(PAD * z);
    let square = Rect::from_min_size(inner.min, vec2(inner.width(), inner.width()));
    let puck = r.live.puck.cloned().unwrap_or_else(|| resting(r.node));

    let mut out = gestures(r, square, &puck);
    let hand = r.ui.data(|d| d.get_temp::<Hand>(hand_id(r)));
    paint(r, square, &puck, hand);

    let mut top = square.max.y + GAP * z;
    out.extend(readouts(r, inner, top));
    top += (CAPTION + number::HEIGHT + GAP) * z;
    out.extend(presets(r, inner, top));
    top += (BUTTON + GAP) * z;
    r.ui.painter().text(
        pos2(inner.center().x, top + HINT * z * 0.5),
        Align2::CENTER_CENTER,
        "Right-drag: place well / Right-click well: remove",
        FontId::proportional(theme::font_size(theme::FONT_TINY, z)),
        r.theme.text_disabled(),
    );
    out
}

/// A puck at rest where the hand controls put it, for a node that has not ticked.
fn resting(node: &Node) -> Puck {
    Puck {
        at: [value(node, HAND[0]), value(node, HAND[1])],
        ..Puck::default()
    }
}

/// One of the node's own numbers, or its definition's default where nothing is written yet.
fn value(node: &Node, key: &str) -> f32 {
    if let Some(ControlValue::Float(v)) = node.controls.get(key) {
        return *v;
    }
    match node.def.input(key).map(|p| &p.control) {
        Some(crate::nodes::Control::Number { default, .. }) => *default,
        _ => 0.0,
    }
}

/// One of the node's own options, or its definition's default.
fn option<'a>(node: &'a Node, key: &str) -> &'a str {
    node.options
        .get(key)
        .map(String::as_str)
        .or_else(|| node.def.option(key).map(|o| o.default))
        .unwrap_or_default()
}

fn hand_id(r: &RegionUi<'_>) -> eframe::egui::Id {
    r.ui.id().with(("xypad-hand", r.id))
}

/// A point on screen, in the pad's own units: -1 to 1 across the square, up positive.
fn to_pad(square: Rect, at: Pos2) -> [f32; 2] {
    let c = square.center();
    let half = square.width() * 0.5;
    // `+ 0.0` so the middle is 0 and not -0, which an s-number would print as a sign.
    [(at.x - c.x) / half + 0.0, (c.y - at.y) / half + 0.0]
}

/// A point on the pad, on screen.
fn to_screen(square: Rect, at: [f32; 2]) -> Pos2 {
    let c = square.center();
    let half = square.width() * 0.5;
    pos2(c.x + at[0] * half, c.y - at[1] * half)
}

fn clamp(at: [f32; 2]) -> [f32; 2] {
    [at[0].clamp(-1.0, 1.0), at[1].clamp(-1.0, 1.0)]
}

/// The hand's four controls, written as one edit in the order that joins a gesture's frames.
fn write(at: [f32; 2], velocity: [f32; 2]) -> RegionEvent {
    let values = [at[0], at[1], velocity[0], velocity[1]];
    RegionEvent::Controls(
        HAND.iter()
            .zip(values)
            .map(|(key, v)| (*key, ControlValue::Float(v)))
            .collect(),
    )
}

// ----------------------------------------------------------------------------- the gestures

/// silvia's `_onPointerDown`, `_onPointerMove` and `_onPointerUp`, over egui's pointer: what
/// the hand is doing is remembered in the context's memory between frames, keyed by the node.
fn gestures(r: &mut RegionUi<'_>, square: Rect, puck: &Puck) -> Vec<RegionEvent> {
    let Some(grab) = r.grab.clone() else {
        return Vec::new();
    };
    let key = hand_id(r);
    let hand = r.ui.data(|d| d.get_temp::<Hand>(key));
    let (primary, secondary, at, origin) = r.ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
            i.pointer.latest_pos(),
            i.pointer.press_origin(),
        )
    });
    let slingshot = option(r.node, xypad::CLICK) != "cursor";
    let mut out = Vec::new();
    // silvia's `cursor: crosshair` over the square, and while a hand has the puck anywhere.
    if hand.is_some() || (grab.hovered() && at.is_some_and(|p| square.contains(p))) {
        r.ui.ctx()
            .set_cursor_icon(eframe::egui::CursorIcon::Crosshair);
    }

    let next = match hand {
        None => {
            let pressed_here =
                grab.is_pointer_button_down_on() && origin.is_some_and(|o| square.contains(o));
            match origin.filter(|_| pressed_here) {
                Some(o) if primary => {
                    let anchor = clamp(to_pad(square, o));
                    out.push(write(anchor, [0.0, 0.0]));
                    out.push(RegionEvent::Held(xypad::PUCK));
                    Some(Hand::Puck {
                        anchor,
                        pull: anchor,
                        last: anchor,
                    })
                }
                Some(o) if secondary => {
                    if let Some(i) = well_at(r, square, puck, o) {
                        out.push(RegionEvent::Touch(Touch::Unwell(i)));
                        Some(Hand::Spent)
                    } else {
                        Some(Hand::Well {
                            origin: to_pad(square, o),
                            from: o,
                        })
                    }
                }
                // Pressed and let go inside one frame, which a quick click can be: the same
                // answer the two frames would have given, a puck put down at rest or a well
                // dropped or taken away.
                _ => {
                    let clicked = at.filter(|p| square.contains(*p));
                    if let Some(p) = clicked.filter(|_| grab.clicked()) {
                        out.push(write(clamp(to_pad(square, p)), [0.0, 0.0]));
                    } else if let Some(p) = clicked.filter(|_| grab.secondary_clicked()) {
                        out.push(RegionEvent::Touch(well_at(r, square, puck, p).map_or(
                            Touch::Well {
                                at: to_pad(square, p),
                                reach: None,
                            },
                            Touch::Unwell,
                        )));
                    }
                    None
                }
            }
        }
        Some(Hand::Puck { anchor, pull, last }) if primary => {
            out.push(RegionEvent::Held(xypad::PUCK));
            let pointer = at.map_or(pull, |p| to_pad(square, p));
            let here = clamp(pointer);
            if here != last {
                // Pulled back, a slingshot holds still until it is let go; carried, the puck
                // takes the speed of the carry, as silvia's does from each move.
                let velocity = if slingshot {
                    [0.0, 0.0]
                } else {
                    [(here[0] - last[0]) * THROW, (here[1] - last[1]) * THROW]
                };
                out.push(write(here, velocity));
            }
            Some(Hand::Puck {
                anchor,
                pull: pointer,
                last: here,
            })
        }
        Some(Hand::Puck { anchor, pull, .. }) => {
            // Let go. A slingshot snaps back to where it was drawn from and flies the other
            // way; a carried puck is already moving at the speed the carry left it with.
            if slingshot {
                let launch = [(anchor[0] - pull[0]) * SLING, (anchor[1] - pull[1]) * SLING];
                out.push(write(anchor, launch));
            }
            None
        }
        Some(Hand::Well { .. } | Hand::Spent) if secondary => hand,
        Some(Hand::Well { origin, from }) => {
            let drawn = at.map_or(0.0, |p| p.distance(from));
            let reach = (drawn >= WELL_CLICK * r.zoom).then(|| drawn / (square.width() * 0.5));
            out.push(RegionEvent::Touch(Touch::Well { at: origin, reach }));
            None
        }
        Some(Hand::Spent) => None,
    };
    r.ui.data_mut(|d| match next {
        Some(h) => {
            d.insert_temp(key, h);
        }
        None => d.remove::<Hand>(key),
    });
    out
}

/// The topmost well within silvia's reach of a point on screen, by its index in the list.
fn well_at(r: &RegionUi<'_>, square: Rect, puck: &Puck, at: Pos2) -> Option<usize> {
    puck.wells
        .iter()
        .rposition(|w| to_screen(square, w.at).distance(at) < WELL_HIT * r.zoom)
}

// ------------------------------------------------------------------------------ the picture

/// silvia's `_draw`, in the editor's tokens: the ground, the center lines and the border, the
/// wells, the well being pulled out, the trail, the velocity, the slingshot and the puck.
fn paint(r: &mut RegionUi<'_>, square: Rect, puck: &Puck, hand: Option<Hand>) {
    let z = r.zoom;
    let painter =
        r.ui.painter()
            .with_clip_rect(r.ui.clip_rect().intersect(square));
    let radius = CornerRadius::same((CORNER * z).round() as u8);
    painter.rect_filled(square, radius, r.theme.bg_primary());

    let hairline = Stroke::new((1.0 * z).max(1.0), r.theme.border_subtle());
    let c = square.center();
    for line in [
        [pos2(c.x, square.min.y), pos2(c.x, square.max.y)],
        [pos2(square.min.x, c.y), pos2(square.max.x, c.y)],
    ] {
        painter.extend(Shape::dashed_line(&line, hairline, 4.0 * z, 4.0 * z));
    }
    painter.rect_stroke(
        square,
        radius,
        Stroke::new((1.0 * z).max(1.0), r.theme.border_normal()),
        StrokeKind::Inside,
    );

    let held = match hand {
        Some(Hand::Puck { anchor, last, .. }) => Some((anchor, last)),
        _ => None,
    };
    // Held, the puck is where the hand has it this frame rather than where the last tick left
    // it, so it does not trail the pointer by a frame.
    let at = held.map_or(puck.at, |(_, last)| last);

    for well in &puck.wells {
        draw_well(r, &painter, square, well, at, 1.0);
    }
    if let (Some(Hand::Well { origin, from }), Some(pointer)) =
        (hand, r.ui.input(|i| i.pointer.latest_pos()))
    {
        let drawn = pointer.distance(from);
        let reach = (drawn >= WELL_CLICK * z).then(|| drawn / (square.width() * 0.5));
        let pull = if option(r.node, xypad::PLACE) == "tether" {
            Pull::Tether(reach.unwrap_or(0.0).max(0.05))
        } else {
            Pull::Gravity(reach.map_or(2.0, |r| r * 3.0))
        };
        let ghost = Well { at: origin, pull };
        draw_well(r, &painter, square, &ghost, at, 0.5);
    }

    trail(r, &painter, square, &puck.path);

    let force = r.theme.port(PortType::VaryingColor);
    let dot = to_screen(square, at);
    let [vx, vy] = puck.velocity;
    if held.is_none() && (vx.abs() > 0.01 || vy.abs() > 0.01) {
        let half = square.width() * 0.5;
        painter.line_segment(
            [dot, dot + vec2(vx * 0.15 * half, -vy * 0.15 * half)],
            Stroke::new(1.0 * z, force.gamma_multiply(0.5)),
        );
    }
    if let Some((anchor, pull)) = held.filter(|_| option(r.node, xypad::CLICK) != "cursor") {
        let (anchor, pull) = (to_screen(square, anchor), to_screen(square, pull));
        painter.line_segment(
            [anchor, pull],
            Stroke::new(2.0 * z, force.gamma_multiply(0.6)),
        );
        let launch = anchor + (anchor - pull) * 0.5;
        painter.extend(Shape::dashed_line(
            &[anchor, launch],
            Stroke::new(1.0 * z, force.gamma_multiply(0.25)),
            4.0 * z,
            4.0 * z,
        ));
        painter.circle_filled(anchor, 4.0 * z, force.gamma_multiply(0.7));
    }
    let ink = r.theme.text_primary();
    if held.is_some() {
        painter.circle_filled(dot, 12.0 * z, ink.gamma_multiply(0.15));
    }
    painter.circle_filled(dot, 5.0 * z, ink);
}

/// Where the puck has been, fading in toward it, in the uniform number's own hue — silvia's
/// `--number-hue` at 60% — and broken wherever a wrap jumped it across the pad.
fn trail(r: &RegionUi<'_>, painter: &eframe::egui::Painter, square: Rect, path: &[[f32; 2]]) {
    if path.len() < 2 {
        return;
    }
    let hue = r.theme.port_anchor(PortType::UniformNumber);
    let color = hue.with(hue.s, 0.6).color();
    let n = path.len() as f32;
    for (i, pair) in path.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        if (b[0] - a[0]).abs() > 1.0 || (b[1] - a[1]).abs() > 1.0 {
            continue;
        }
        let alpha = (i + 1) as f32 / n * 0.6;
        painter.line_segment(
            [to_screen(square, a), to_screen(square, b)],
            Stroke::new(1.5 * r.zoom, color.gamma_multiply(alpha)),
        );
    }
}

/// One well: a gravity well is a ring as wide as it is strong, a dot and a cross, in the
/// event hue silvia's amber stands for; a tether is its dashed circle, a dot and the string
/// to the puck, in the editor's own.
fn draw_well(
    r: &RegionUi<'_>,
    painter: &eframe::egui::Painter,
    square: Rect,
    well: &Well,
    puck: [f32; 2],
    opacity: f32,
) {
    let z = r.zoom;
    let at = to_screen(square, well.at);
    let line =
        |alpha: f32, tint: Color32| Stroke::new(1.0 * z, tint.gamma_multiply(alpha * opacity));
    match well.pull {
        Pull::Gravity(strength) => {
            let tint = r.theme.port(PortType::Action);
            painter.circle_stroke(at, (strength * 10.0).min(50.0) * z, line(0.25, tint));
            painter.circle_filled(at, 4.0 * z, tint.gamma_multiply(0.9 * opacity));
            let arm = 7.0 * z;
            painter.line_segment([at - vec2(arm, 0.0), at + vec2(arm, 0.0)], line(0.5, tint));
            painter.line_segment([at - vec2(0.0, arm), at + vec2(0.0, arm)], line(0.5, tint));
        }
        Pull::Tether(length) => {
            let tint = r.theme.primary();
            let radius = (length * square.width() * 0.5).max(1.0);
            let ring: Vec<Pos2> = (0..=64)
                .map(|i| {
                    let a = i as f32 / 64.0 * std::f32::consts::TAU;
                    at + vec2(a.cos(), a.sin()) * radius
                })
                .collect();
            painter.extend(Shape::dashed_line(
                &ring,
                line(0.35, tint),
                3.0 * z,
                4.0 * z,
            ));
            painter.circle_filled(at, 3.0 * z, tint.gamma_multiply(0.9 * opacity));
            painter.line_segment([at, to_screen(square, puck)], line(0.25, tint));
        }
    }
}

// --------------------------------------------------------------------- under the square

/// The two hand controls, captioned X and Y, each centered in its half: silvia's two
/// readouts, as the node's own s-numbers.
fn readouts(r: &mut RegionUi<'_>, inner: Rect, top: f32) -> Vec<RegionEvent> {
    let z = r.zoom;
    let mut out = Vec::new();
    let half = inner.width() * 0.5;
    for (i, (caption, key)) in [("X", xypad::PAD_X), ("Y", xypad::PAD_Y)]
        .into_iter()
        .enumerate()
    {
        let middle = inner.min.x + half * (i as f32 + 0.5);
        r.caption(
            pos2(middle, top + CAPTION * z * 0.5),
            Align2::CENTER_CENTER,
            caption,
            r.theme.text_muted(),
        );
        let cell = Rect::from_center_size(
            pos2(middle, top + (CAPTION + number::HEIGHT * 0.5) * z),
            vec2(number::WIDTH * z, number::HEIGHT * z),
        );
        out.extend(r.number(cell, key));
    }
    out
}

/// silvia's bar of nine, numbered, each named for its preset on hover. A press writes the
/// preset's edges and its numbers through the bus and starts its flight and its wells with a
/// touch, so the controls it leaves are saved and undone and the wells, as silvia's, are not.
fn presets(r: &mut RegionUi<'_>, inner: Rect, top: f32) -> Vec<RegionEvent> {
    let z = r.zoom;
    let n = PRESETS.len() as f32;
    let pitch = (inner.width() + BUTTON_GAP * z) / n;
    let mut out = Vec::new();
    for (i, preset) in PRESETS.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(inner.min.x + pitch * i as f32, top),
            vec2(pitch - BUTTON_GAP * z, BUTTON * z),
        );
        let name = r.name(&format!("preset{}", i + 1));
        let w = r.ui.interact(
            rect,
            r.ui.id().with(("xypad-preset", &name)),
            Sense::click(),
        );
        r.ui.painter().rect(
            rect,
            CornerRadius::same((CORNER * z).round() as u8),
            if w.hovered() {
                r.theme.bg_hover()
            } else {
                r.theme.bg_interactive()
            },
            Stroke::new((1.0 * z).max(1.0), r.theme.border_normal()),
            StrokeKind::Inside,
        );
        r.ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            format!("{}", i + 1),
            FontId::proportional(theme::font_size(theme::FONT_BASE, z)),
            r.theme.text_secondary(),
        );
        crate::ui::accessible(&w, eframe::egui::WidgetType::Button, &name);
        if w.clone().on_hover_text(preset.name).clicked() {
            out.push(RegionEvent::Touch(Touch::Preset(i)));
            // The first edge, the numbers, the second edge: each joins the one before it in
            // one undo step, which is `history::joins`' rule for a region writing a node's
            // controls and its options with one press.
            out.push(RegionEvent::Option {
                key: xypad::EDGE_X,
                value: preset.edges[0],
            });
            out.push(RegionEvent::Controls(
                preset
                    .controls()
                    .into_iter()
                    .map(|(key, v)| (key, ControlValue::Float(v)))
                    .collect(),
            ));
            out.push(RegionEvent::Option {
                key: xypad::EDGE_Y,
                value: preset.edges[1],
            });
        }
    }
    out
}
