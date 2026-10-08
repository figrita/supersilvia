// SPDX-License-Identifier: AGPL-3.0-or-later

//! A folding section of a node's own controls: whatever its [`Section`] lays out under its
//! heading — a dial a hand drags, an option as a segmented switch, a number, a swatch, and a
//! button that opens a panel of more of them.
//!
//! **Not everything a node keeps needs a port.** Render 3D has five numbers a cable is for and
//! twenty things a hand sets once and looks at: those are hidden controls and in-region
//! options, drawn here and written through the bus as any row's control is. Every number is
//! the inset s-number and every color the swatch, by `RegionUi::number` and
//! `RegionUi::swatch`, so each one types, resets, learns a MIDI binding and wears its mark as
//! a knob on a row does.
//!
//! **Layout is pure.** Every part's height is a function of the column it is given, so
//! [`SECTION`]'s `size` answers without a `Ui`: an option's switch sits beside its caption
//! where its segments are wide enough, and under it, in as few even rows as fit, where they
//! are not.
//!
//! **The two dials.** The globe is a sphere in perspective, seen from a fixed eye above the
//! front, with the camera a dot on it and the light a sun beside it: a drag orbits and tilts
//! the camera, a click puts it where the globe was clicked, and the wheel moves it nearer or
//! further, in ratios, as Distance is logarithmic. Standing at a From point, the same gestures
//! carry From round the Target at its own distance. The sun disk is the sky seen from above:
//! the rim is the horizon, the middle straight up, and a press or a drag puts the sun under the
//! hand. A drag writes the same keys in the same order every frame, so it is one undo step.
//! Both draw what the render reads this frame, a cable's value where one runs into Orbit, Tilt,
//! Distance or Light, as the rows' meters do, and neither writes an axis a cable sets, since
//! the cable would overwrite it: a dial with nothing left to set is inert and drawn fainter.
//!
//! **A part with no effect is dimmed.** A [`Part::When`] whose condition does not hold draws
//! its parts in a disabled child `Ui`, which egui fades and lets no hand reach, so the
//! accessibility tree says each one is disabled, as it says of an option a cable overrides.
//!
//! **A panel is the editor's, not the document's.** Which one is open is kept in egui's memory
//! by node, one at a time; it hangs under its button in the popup's own frame and closes on a
//! click outside it — but not on one in a popup it opened, a color picker or a range editor —
//! on `Escape`, or on its button again.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node, PortType};
use crate::nodes::{Control, Dial, Globe, NodeDef, Part, Popout, Region, Section};
use crate::ui::{canvas, number};
use eframe::egui::{
    Align2, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Shape, Stroke, UiBuilder, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};
use std::f32::consts::TAU;

/// The body a section asks for: room for a dial and a column of controls beside it.
pub const WIDTH: f32 = 240.0;

pub const SECTION: RegionDef = RegionDef {
    size,
    show,
    claims_pointer: false,
    width: Some(WIDTH),
};

/// Where a section's parts start and stop across the body: a row's label inset on the left,
/// and on the right where a row's control stops.
const LEFT: f32 = crate::ui::node_widget::LABEL_INSET;
const RIGHT: f32 = canvas::ROW_BLOCK_INSET + 8.0;
/// Air over the first part and under the last, and between two.
const TOP: f32 = 4.0;
const FOOT: f32 = 6.0;
const GAP: f32 = 4.0;
/// One line of a [`Part::Note`]: the tiny type and a little air.
const NOTE_LINE: f32 = 12.0;
/// One line of a part beside its caption: an s-number's height.
const LINE: f32 = number::HEIGHT;
/// A caption standing over its switch.
const CAPTION: f32 = 12.0;
/// One row of segments, a select's height, and the gap between two rows of them.
const SEGMENT: f32 = canvas::SELECT_HEIGHT;
const SEGMENT_GAP: f32 = 2.0;
/// The narrowest a segment is drawn: a short word in the tiny type, with air.
const MIN_SEGMENT: f32 = 36.0;
/// The caption's column beside a switch.
const LABEL: f32 = 56.0;
/// The least room a number leaves its caption, and the narrowest it is drawn.
const NUMBER_LABEL: f32 = 36.0;
const MIN_NUMBER: f32 = 60.0;
/// A panel's button.
const BUTTON: f32 = 18.0;
/// A dial's side, and the gap between it and the column beside it.
const DIAL: f32 = 92.0;
const DIAL_GAP: f32 = 8.0;
/// A panel's content width, in points: a popup is drawn at the interface's size, not the
/// canvas zoom's.
const PANEL: f32 = 196.0;

/// How far the globe's eye stands from its middle, in globe radii, and how far above the
/// globe's equator it looks down, in radians.
const EYE: f32 = 4.0;
const ELEVATION: f32 = 0.45;
/// How far the sun stands off the globe, in globe radii.
const SUN_OUT: f32 = 1.3;
/// How many straight pieces one ring of the globe is drawn in.
const SEGMENTS: usize = 48;
/// How much of a turn a drag across the whole globe is.
const DRAG_TURN: f32 = 0.5;
/// How fast the wheel moves the camera: a ratio per point of scroll.
const WHEEL: f32 = 0.004;

fn section(region: Region) -> Option<&'static Section> {
    match region {
        Region::Section(section) => Some(section),
        _ => None,
    }
}

fn size(node: &Node, region: Region) -> f32 {
    let Some(section) = section(region) else {
        return 0.0;
    };
    let w = canvas::node_width(node) - LEFT - RIGHT;
    TOP + parts_height(node.def, section.parts, w) + FOOT
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(section) = section(r.region) else {
        return Vec::new();
    };
    let z = r.zoom;
    let rect = Rect::from_min_max(
        r.rect.min + vec2(LEFT * z, TOP * z),
        pos2(r.rect.max.x - RIGHT * z, r.rect.max.y - FOOT * z),
    );
    parts(r, section.parts, rect)
}

// ---------------------------------------------------------------------------------- layout

fn parts_height(def: &NodeDef, parts: &[Part], w: f32) -> f32 {
    let sum: f32 = parts.iter().map(|p| part_height(def, p, w)).sum();
    sum + GAP * parts.len().saturating_sub(1) as f32
}

fn part_height(def: &NodeDef, part: &Part, w: f32) -> f32 {
    match part {
        Part::Beside(_, column) => DIAL.max(parts_height(def, column, w - DIAL - DIAL_GAP)),
        Part::Choice { key, .. } => {
            let n = def.option(key).map_or(1, |o| o.choices.len());
            choice_layout(n, w).height()
        }
        Part::Number { .. } | Part::Swatch { .. } => LINE,
        Part::Popout(_) => BUTTON,
        Part::When(_, inner) => parts_height(def, inner, w),
        Part::Note(lines) => NOTE_LINE * lines.len() as f32,
    }
}

/// How a switch of `n` segments sits in a column `w` wide: beside its caption, or under it in
/// `rows` rows of at most `per_row`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ChoiceLayout {
    beside: bool,
    rows: usize,
    per_row: usize,
}

impl ChoiceLayout {
    fn height(self) -> f32 {
        if self.beside {
            LINE
        } else {
            CAPTION + self.rows as f32 * SEGMENT + (self.rows - 1) as f32 * SEGMENT_GAP
        }
    }
}

fn choice_layout(n: usize, w: f32) -> ChoiceLayout {
    let n = n.max(1);
    if (w - LABEL) / n as f32 >= MIN_SEGMENT {
        return ChoiceLayout {
            beside: true,
            rows: 1,
            per_row: n,
        };
    }
    let fit = (((w + SEGMENT_GAP) / (MIN_SEGMENT + SEGMENT_GAP)).floor() as usize).max(1);
    let rows = n.div_ceil(fit);
    ChoiceLayout {
        beside: false,
        rows,
        per_row: n.div_ceil(rows),
    }
}

/// How wide a number is drawn in a column `w` wide.
fn number_width(w: f32) -> f32 {
    (w - NUMBER_LABEL).clamp(MIN_NUMBER, number::WIDTH)
}

// --------------------------------------------------------------------------------- drawing

/// Draw `parts` down `rect`, a column on screen, and say what the hand did in them.
fn parts(r: &mut RegionUi<'_>, parts: &[Part], rect: Rect) -> Vec<RegionEvent> {
    let z = r.zoom;
    let w = rect.width() / z;
    let mut top = rect.min.y;
    let mut out = Vec::new();
    for part in parts {
        let h = part_height(r.node.def, part, w) * z;
        let band = Rect::from_min_max(pos2(rect.min.x, top), pos2(rect.max.x, top + h));
        out.extend(one(r, part, band));
        top += h + GAP * z;
    }
    out
}

fn one(r: &mut RegionUi<'_>, part: &Part, band: Rect) -> Vec<RegionEvent> {
    let z = r.zoom;
    let mut out = Vec::new();
    match *part {
        Part::Beside(dial, column) => {
            let side = DIAL * z;
            let square = Rect::from_min_size(
                pos2(band.min.x, band.center().y - side * 0.5),
                vec2(side, side),
            );
            out.extend(match dial {
                Dial::Globe(globe) => self::globe(r, globe, square),
                Dial::Sun {
                    azimuth,
                    height,
                    camera,
                } => sun(r, azimuth, height, camera, square),
            });
            let w = (band.width() / z - DIAL - DIAL_GAP) * z;
            let h = parts_height(r.node.def, column, w / z) * z;
            let left = band.max.x - w;
            let col = Rect::from_min_size(pos2(left, band.center().y - h * 0.5), vec2(w, h));
            out.extend(parts(r, column, col));
        }
        Part::Choice { key, caption } => out.extend(choice(r, key, caption, band)),
        Part::Number { key, caption } => {
            label(r, band, caption);
            let nw = number_width(band.width() / z) * z;
            let cell = Rect::from_min_max(pos2(band.max.x - nw, band.min.y), band.max);
            out.extend(r.number(cell, key));
        }
        Part::Swatch { key, caption } => {
            label(r, band, caption);
            let nw = number_width(band.width() / z) * z;
            let cell = Rect::from_min_max(pos2(band.max.x - nw, band.min.y), band.max);
            out.extend(r.swatch(cell, key));
        }
        Part::Popout(popout) => out.extend(popout_button(r, popout, band)),
        Part::When(applies, inner) => {
            if applies.holds(r.node, |key| r.source(key).is_some()) {
                out.extend(parts(r, inner, band));
            } else {
                dimmed(r, inner, band);
            }
        }
        Part::Note(lines) => {
            for (i, line) in lines.iter().enumerate() {
                r.caption(
                    pos2(band.min.x, band.min.y + (i as f32 + 0.5) * NOTE_LINE * z),
                    Align2::LEFT_CENTER,
                    line,
                    r.theme.text_muted(),
                );
            }
        }
    }
    out
}

/// Parts that have no effect as the node is set: drawn in a `Ui` of their own that egui fades
/// and lets no hand reach, so each one is in the accessibility tree disabled, as an option a
/// cable overrides is.
fn dimmed(r: &mut RegionUi<'_>, inner: &[Part], band: Rect) {
    let mut ui = r.ui.new_child(
        UiBuilder::new()
            .id_salt("section-dimmed")
            .max_rect(band)
            .disabled(),
    );
    let mut faded = RegionUi {
        ui: &mut ui,
        region: r.region,
        node: r.node,
        id: r.id,
        rect: r.rect,
        zoom: r.zoom,
        corner: r.corner,
        theme: r.theme,
        live: r.live,
        grab: None,
        lock_cursor: r.lock_cursor,
        learning: r.learning,
        bindings: r.bindings,
    };
    // Deaf: whatever a faded part says is dropped, though egui lets none of them hear a hand.
    let _ = parts(&mut faded, inner, band);
}

/// A part's caption, where a row's label stands: the tiny type, at the left of its line.
fn label(r: &RegionUi<'_>, band: Rect, caption: &str) {
    r.caption(
        pos2(band.min.x, band.min.y + LINE * r.zoom * 0.5),
        Align2::LEFT_CENTER,
        caption,
        r.theme.text_secondary(),
    );
}

/// An option as a segmented switch, beside its caption or under it.
fn choice(r: &mut RegionUi<'_>, key: &str, caption: &str, band: Rect) -> Vec<RegionEvent> {
    let z = r.zoom;
    let Some(option) = r.node.def.option(key) else {
        return Vec::new();
    };
    let choices = option.choices;
    let layout = choice_layout(choices.len(), band.width() / z);
    let node = r.node;
    let chosen = node
        .options
        .get(option.key)
        .map_or(option.default, String::as_str);
    let name = r.name(option.key);
    let mut out = Vec::new();
    let mut pick = |r: &mut RegionUi<'_>, frame: Rect, slice: crate::nodes::Choices| {
        if let Some(i) = super::segments(r.ui, frame, slice, chosen, &name, z, r.theme) {
            out.push(RegionEvent::Option {
                key: option.key,
                value: slice[i].0,
            });
        }
    };
    if layout.beside {
        label(r, band, caption);
        let frame = Rect::from_min_max(
            pos2(band.min.x + LABEL * z, band.center().y - SEGMENT * z * 0.5),
            pos2(band.max.x, band.center().y + SEGMENT * z * 0.5),
        );
        pick(r, frame, choices);
    } else {
        r.caption(
            pos2(band.min.x, band.min.y + CAPTION * z * 0.5),
            Align2::LEFT_CENTER,
            caption,
            r.theme.text_secondary(),
        );
        let mut top = band.min.y + CAPTION * z;
        for row in choices.chunks(layout.per_row) {
            // A short last row keeps the others' segment width, so its segments line up.
            let w = band.width() * row.len() as f32 / layout.per_row as f32;
            let frame = Rect::from_min_size(pos2(band.min.x, top), vec2(w, SEGMENT * z));
            pick(r, frame, row);
            top += (SEGMENT + SEGMENT_GAP) * z;
        }
    }
    out
}

// ---------------------------------------------------------------------------------- panels

/// The node's open panel, by the key of the [`Popout`] that opened it.
fn open_id(r: &RegionUi<'_>) -> Id {
    Id::new(("section-panel", r.id))
}

fn popout_button(r: &mut RegionUi<'_>, popout: &'static Popout, band: Rect) -> Vec<RegionEvent> {
    let state = open_id(r);
    // A button drawn dimmed closes its panel: what is in it has no effect.
    let open =
        r.ui.is_enabled() && r.ui.data(|d| d.get_temp::<&'static str>(state)) == Some(popout.key);
    let clicked = super::lit_button(r, band, popout.caption, popout.key, open);
    let open = if clicked { !open } else { open };
    r.ui.data_mut(|d| {
        if open {
            d.insert_temp(state, popout.key);
        } else if d.get_temp::<&'static str>(state) == Some(popout.key) {
            d.remove::<&'static str>(state);
        }
    });
    if !open {
        return Vec::new();
    }
    let (out, keep) = panel(r, popout, band, clicked);
    if !keep {
        r.ui.data_mut(|d| d.remove::<&'static str>(state));
    }
    out
}

/// The panel a popout opens, hung under its button: its parts, a Reset under them where it
/// has one, and whether it stays open.
fn panel(
    r: &mut RegionUi<'_>,
    popout: &'static Popout,
    button: Rect,
    opened_now: bool,
) -> (Vec<RegionEvent>, bool) {
    let ctx = r.ui.ctx().clone();
    let id = Id::new(("section-popout", r.id, popout.key));
    let def = r.node.def;
    let reset = !popout.reset.is_empty();
    let h = parts_height(def, popout.parts, PANEL) + if reset { GAP + BUTTON } else { 0.0 };
    let (region, node, node_id, theme, live, lock_cursor, learning, bindings) = (
        r.region,
        r.node,
        r.id,
        r.theme,
        r.live,
        r.lock_cursor,
        r.learning,
        r.bindings,
    );
    let shown = crate::ui::popup::Popup::new(id, button.left_bottom() + vec2(0.0, 2.0)).show(
        &ctx,
        theme,
        |ui| {
            let (rect, w) = ui.allocate_exact_size(vec2(PANEL, h), Sense::hover());
            crate::ui::accessible(
                &w,
                WidgetType::Other,
                format_args!("{}{node_id}.{}.panel", def.slug, popout.key),
            );
            let mut inner = RegionUi {
                ui,
                region,
                node,
                id: node_id,
                rect,
                zoom: 1.0,
                corner: 0.0,
                theme,
                live,
                grab: None,
                lock_cursor,
                learning,
                bindings,
            };
            let column = Rect::from_min_size(
                rect.min,
                vec2(PANEL, parts_height(def, popout.parts, PANEL)),
            );
            let mut out = parts(&mut inner, popout.parts, column);
            if reset {
                let at = Rect::from_min_max(pos2(rect.min.x, rect.max.y - BUTTON), rect.max);
                let part = format!("{}.reset", popout.key);
                if super::button(&mut inner, at, "Reset", &part) {
                    out.push(RegionEvent::Controls(
                        popout
                            .reset
                            .iter()
                            .filter_map(|key| match def.input(key)?.control {
                                Control::Number { default, .. } => {
                                    Some((*key, ControlValue::Float(default)))
                                }
                                _ => None,
                            })
                            .collect(),
                    ));
                }
            }
            out
        },
    );
    let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    // A click in a popup this panel opened — a picker, a range editor — is not a click away.
    let elsewhere = ctx
        .pointer_latest_pos()
        .and_then(|p| ctx.layer_id_at(p))
        .is_some_and(|layer| {
            layer.id != id && matches!(layer.order, Order::Foreground | Order::Tooltip)
        });
    let away = shown.clicked_away && !opened_now && !elsewhere;
    (shown.inner, !(escape || away))
}

// ---------------------------------------------------------------------------------- values

/// One of the node's own numbers, or its definition's default where nothing is written yet.
fn value(node: &Node, key: &str) -> f32 {
    if let Some(ControlValue::Float(v)) = node.controls.get(key) {
        return *v;
    }
    match node.def.input(key).map(|p| &p.control) {
        Some(Control::Number { default, .. }) => *default,
        _ => 0.0,
    }
}

fn option<'a>(node: &'a Node, key: &str) -> &'a str {
    node.options
        .get(key)
        .map(String::as_str)
        .or_else(|| node.def.option(key).map(|o| o.default))
        .unwrap_or_default()
}

/// An angle in turns, a whole turn at a time back inside its control's range where the range
/// spans one: an orbit dragged past its end goes on round.
fn wrap(node: &Node, key: &str, v: f32) -> f32 {
    let Some(range) = crate::nodes::control_range(node.def, node, key) else {
        return v;
    };
    if range.max - range.min < 1.0 {
        return v;
    }
    let mut v = v;
    while v > range.max {
        v -= 1.0;
    }
    while v < range.min {
        v += 1.0;
    }
    v
}

fn clamp_to(node: &Node, key: &str, v: f32) -> f32 {
    crate::nodes::control_range(node.def, node, key).map_or(v, |r| v.clamp(r.min, r.max))
}

fn float(key: &'static str, v: f32) -> (&'static str, ControlValue) {
    (key, ControlValue::Float(v))
}

// ---------------------------------------------------------------------------------- vectors

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l < 1.0e-9 {
        [0.0, 0.0, 1.0]
    } else {
        scale(a, 1.0 / l)
    }
}

/// The direction an orbit and a tilt in turns point, as the render's own camera reads them.
fn direction(orbit: f32, tilt: f32) -> V3 {
    let (o, t) = (orbit * TAU, tilt * TAU);
    [t.cos() * o.sin(), t.sin(), t.cos() * o.cos()]
}

/// An orbit and a tilt in turns, out of a direction.
fn angles(d: V3) -> (f32, f32) {
    let d = normalize(d);
    (d[0].atan2(d[2]) / TAU, d[1].clamp(-1.0, 1.0).asin() / TAU)
}

// ----------------------------------------------------------------------------------- globe

/// The globe's eye: across, up and toward it, the eye above the front of the globe.
fn basis() -> (V3, V3, V3) {
    let (s, c) = ELEVATION.sin_cos();
    ([1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c])
}

/// Where a point in globe radii lands on screen, round the globe's middle `c` drawn `radius`
/// across, and how far toward the eye it is.
fn project(c: Pos2, radius: f32, p: V3) -> (Pos2, f32) {
    let (right, up, toward) = basis();
    let (x, y, depth) = (dot(p, right), dot(p, up), dot(p, toward));
    // Scaled so the globe's own outline is `radius` across.
    let k = (EYE * EYE - 1.0).sqrt() / EYE;
    let s = EYE / (EYE - depth) * k * radius;
    (c + vec2(x * s, -y * s), depth)
}

/// Whether a point on the globe faces the eye.
fn facing(depth: f32) -> bool {
    depth > 1.0 / EYE
}

/// The point on the globe under a point on screen: where the eye's ray through it first meets
/// the globe, or the outline's nearest point where it misses.
fn pick(c: Pos2, radius: f32, at: Pos2) -> V3 {
    let (right, up, toward) = basis();
    let k = (EYE * EYE - 1.0).sqrt() / EYE;
    let (qx, qy) = ((at.x - c.x) / radius / k, -(at.y - c.y) / radius / k);
    let eye = scale(toward, EYE);
    let d = normalize(sub(add(scale(right, qx), scale(up, qy)), eye));
    let b = dot(eye, d);
    let disc = b * b - (EYE * EYE - 1.0);
    let t = if disc < 0.0 { -b } else { -b - disc.sqrt() };
    normalize(add(eye, scale(d, t)))
}

/// Where the camera is, as an orbit, a tilt and a distance — each what the render reads this
/// frame, a cable's where one runs in — or From round Target while the camera stands at From.
fn camera(r: &RegionUi<'_>, g: &Globe) -> (f32, f32, f32, bool) {
    if option(r.node, g.mode) == g.look_at {
        let at = |keys: [&str; 3]| keys.map(|k| r.number_now(k));
        let off = sub(at(g.from), at(g.target));
        let (o, t) = angles(off);
        (o, t, length(off), true)
    } else {
        (
            r.number_now(g.orbit),
            r.number_now(g.tilt),
            r.number_now(g.distance),
            false,
        )
    }
}

/// Which of Orbit and Tilt a hand on the globe can set: standing at From, both, since it
/// writes From; else each that no cable runs into, as a cable's value overwrites a write.
fn steerable(r: &RegionUi<'_>, g: &Globe, look: bool) -> [bool; 2] {
    if look {
        [true; 2]
    } else {
        [g.orbit, g.tilt].map(|key| r.source(key).is_none())
    }
}

/// The edit that puts the camera at an orbit, a tilt and a distance, writing only the axes
/// `free` names: `None` where it would write nothing.
fn place(
    node: &Node,
    g: &Globe,
    (orbit, tilt, distance): (f32, f32, f32),
    look: bool,
    free: [bool; 2],
) -> Option<RegionEvent> {
    let tilt = clamp_to(node, g.tilt, tilt);
    if look {
        let target = g.target.map(|k| value(node, k));
        let from = add(target, scale(direction(orbit, tilt), distance));
        return Some(RegionEvent::Controls(vec![
            float(g.from[0], from[0]),
            float(g.from[1], from[1]),
            float(g.from[2], from[2]),
        ]));
    }
    let writes: Vec<_> = [
        float(g.orbit, wrap(node, g.orbit, orbit)),
        float(g.tilt, tilt),
    ]
    .into_iter()
    .zip(free)
    .filter_map(|(write, free)| free.then_some(write))
    .collect();
    (!writes.is_empty()).then_some(RegionEvent::Controls(writes))
}

/// How bright a dial is drawn while cables set everything a drag on it would.
const INERT: f32 = 0.6;

fn globe(r: &mut RegionUi<'_>, g: &'static Globe, square: Rect) -> Vec<RegionEvent> {
    let (orbit, tilt, distance, look) = camera(r, g);
    let free = steerable(r, g, look);
    let steers = free.contains(&true);
    let nears = look || r.source(g.distance).is_none();
    let w = r.ui.interact(
        square,
        r.ui.id().with(("section-globe", r.id)),
        if steers {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let name = r.name("globe");
    w.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Other, steers, &name);
        info.current_text_value = Some(format!(
            "orbit {orbit:.3} tilt {tilt:.3} distance {distance:.3}"
        ));
        info
    });
    let w = w.on_hover_text(match (free, nears) {
        ([true, true], true) => {
            "Drag to orbit and tilt, click to put the camera there, wheel to move nearer"
        }
        ([true, true], false) => "Drag to orbit and tilt, click to put the camera there",
        ([false, true], true) => "Drag up and down to tilt, wheel to move nearer: Orbit is cabled",
        ([false, true], false) => "Drag up and down to tilt: Orbit is cabled",
        ([true, false], true) => "Drag across to orbit, wheel to move nearer: Tilt is cabled",
        ([true, false], false) => "Drag across to orbit: Tilt is cabled",
        ([false, false], true) => "Wheel to move nearer: Orbit and Tilt are cabled",
        ([false, false], false) => "Orbit, Tilt and Distance are cabled",
    });
    if steers {
        crate::ui::cursor(
            &w,
            if w.dragged() {
                eframe::egui::CursorIcon::Grabbing
            } else {
                eframe::egui::CursorIcon::Grab
            },
        );
    } else {
        crate::ui::refused(&w);
    }
    let c = square.center();
    let radius = square.width() * 0.5 / SUN_OUT;

    let mut out = Vec::new();
    let d = w.drag_delta();
    if w.dragged() && d != Vec2::ZERO {
        let per = DRAG_TURN / square.width();
        let to = (orbit + d.x * per, tilt - d.y * per, distance);
        out.extend(place(r.node, g, to, look, free));
    } else if w.clicked()
        && let Some(at) = w.interact_pointer_pos()
    {
        let (o, t) = angles(pick(c, radius, at));
        out.extend(place(r.node, g, (o, t, distance), look, free));
    }
    if w.hovered() && nears {
        // Taken rather than read, as a number control takes it: the canvas zooms on whatever
        // wheel input is left at the end of the frame.
        let scroll = r.ui.ctx().input_mut(|i| {
            let v = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta.y = 0.0;
            v
        });
        if scroll != 0.0 {
            let further = distance * (-scroll * WHEEL).exp();
            if look {
                out.extend(place(
                    r.node,
                    g,
                    (orbit, tilt, further.max(0.01)),
                    true,
                    free,
                ));
            } else {
                out.push(RegionEvent::Controls(vec![float(
                    g.distance,
                    clamp_to(r.node, g.distance, further),
                )]));
            }
        }
    }

    paint_globe(r, g, square, radius, (orbit, tilt), steers);
    out
}

/// The globe, with the camera at `orbit` and `tilt`: faded while `steers` is false.
fn paint_globe(
    r: &RegionUi<'_>,
    g: &Globe,
    square: Rect,
    radius: f32,
    (orbit, tilt): (f32, f32),
    steers: bool,
) {
    let z = r.zoom;
    let c = square.center();
    let mut painter =
        r.ui.painter()
            .with_clip_rect(r.ui.clip_rect().intersect(square.expand(2.0 * z)));
    if !steers {
        painter.multiply_opacity(INERT);
    }
    let hair = (1.0 * z).max(1.0);
    painter.circle_filled(c, radius, r.theme.bg_primary());

    // The graticule, every thirty degrees: the far half dim behind the near.
    let near = r.theme.border_normal();
    let far = r.theme.border_subtle().gamma_multiply(0.5);
    let ring = |points: &mut dyn Iterator<Item = V3>, bright: bool| {
        let pts: Vec<(Pos2, f32)> = points.map(|p| project(c, radius, p)).collect();
        for pair in pts.windows(2) {
            let front = facing(f32::midpoint(pair[0].1, pair[1].1));
            let tint = match (front, bright) {
                (true, true) => near,
                (true, false) => near.gamma_multiply(0.55),
                (false, _) => far,
            };
            painter.line_segment([pair[0].0, pair[1].0], Stroke::new(hair, tint));
        }
    };
    for lat in [-60.0_f32, -30.0, 0.0, 30.0, 60.0] {
        let phi = lat.to_radians();
        ring(
            &mut (0..=SEGMENTS).map(|i| {
                let theta = i as f32 / SEGMENTS as f32 * TAU;
                [phi.cos() * theta.sin(), phi.sin(), phi.cos() * theta.cos()]
            }),
            lat == 0.0,
        );
    }
    for lon in 0..6 {
        let theta = lon as f32 * TAU / 12.0;
        ring(
            &mut (0..=SEGMENTS).map(|i| {
                let phi = i as f32 / SEGMENTS as f32 * TAU;
                [phi.cos() * theta.sin(), phi.sin(), phi.cos() * theta.cos()]
            }),
            false,
        );
    }
    painter.circle_stroke(c, radius, Stroke::new(hair, r.theme.border_normal()));

    // The target, a small cross in the middle.
    let arm = 3.0 * z;
    let ink = r.theme.text_muted();
    painter.line_segment(
        [c - vec2(arm, 0.0), c + vec2(arm, 0.0)],
        Stroke::new(hair, ink),
    );
    painter.line_segment(
        [c - vec2(0.0, arm), c + vec2(0.0, arm)],
        Stroke::new(hair, ink),
    );

    // The sun, off the globe in the light's direction, dim where the globe hides it.
    let light = direction(r.number_now(g.light[0]), r.number_now(g.light[1]));
    let (at, depth) = project(c, radius, scale(light, SUN_OUT));
    let hidden = depth < 0.0 && at.distance(c) < radius;
    sun_mark(r, &painter, at, 3.0 * z, if hidden { 0.35 } else { 1.0 });

    // The camera: a line from the target and a dot, a ring where it is round the back.
    let (dot, depth) = project(c, radius, direction(orbit, tilt));
    let tint = r.theme.primary();
    if facing(depth) {
        painter.line_segment([c, dot], Stroke::new(1.5 * z, tint.gamma_multiply(0.7)));
        painter.circle_filled(dot, 4.5 * z, tint);
    } else {
        painter.extend(Shape::dashed_line(
            &[c, dot],
            Stroke::new(hair, tint.gamma_multiply(0.4)),
            3.0 * z,
            3.0 * z,
        ));
        painter.circle_stroke(dot, 4.0 * z, Stroke::new(1.5 * z, tint.gamma_multiply(0.6)));
    }
}

/// A sun: a disc in the event hue with eight short rays.
fn sun_mark(r: &RegionUi<'_>, painter: &eframe::egui::Painter, at: Pos2, size: f32, alpha: f32) {
    let tint = r.theme.port(PortType::Action).gamma_multiply(alpha);
    painter.circle_filled(at, size, tint);
    let stroke = Stroke::new((size * 0.4).max(1.0), tint);
    for i in 0..8 {
        let a = i as f32 * TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        painter.line_segment([at + d * size * 1.5, at + d * size * 2.3], stroke);
    }
}

// ------------------------------------------------------------------------------------- sun

/// The highest a sun stands, in turns: straight up.
const ZENITH: f32 = 0.25;

/// Where a light at `azimuth` and `height` in turns is on a disk round `c`, `radius` across:
/// the camera's front at the foot, as the globe shows it.
fn on_disk(c: Pos2, radius: f32, azimuth: f32, height: f32) -> Pos2 {
    let a = azimuth * TAU;
    let out = (1.0 - height.clamp(0.0, ZENITH) / ZENITH) * radius;
    c + vec2(a.sin(), a.cos()) * out
}

fn sun(
    r: &mut RegionUi<'_>,
    azimuth: &'static str,
    height: &'static str,
    globe: &'static Globe,
    square: Rect,
) -> Vec<RegionEvent> {
    let z = r.zoom;
    let (now_a, now_h) = (r.number_now(azimuth), r.number_now(height));
    // Each of the two a hand can set, being the one no cable runs into.
    let free = [azimuth, height].map(|key| r.source(key).is_none());
    let steers = free.contains(&true);
    let w = r.ui.interact(
        square,
        r.ui.id().with(("section-sun", r.id)),
        if steers {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let name = r.name("sun");
    w.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Other, steers, &name);
        info.current_text_value = Some(format!("light {now_a:.3} height {now_h:.3}"));
        info
    });
    let w = w.on_hover_text(match free {
        [true, true] => "Drag the sun: the rim is the horizon, the middle straight up",
        [false, true] => "Drag in and out to raise the sun: Light is cabled",
        [true, false] => "Drag round to turn the sun: its Height is cabled",
        [false, false] => "Light and its Height are cabled",
    });
    if steers {
        crate::ui::cursor(&w, eframe::egui::CursorIcon::Crosshair);
    } else {
        crate::ui::refused(&w);
    }
    let c = square.center();
    let radius = square.width() * 0.5 - 4.0 * z;

    let mut out = Vec::new();
    if (w.is_pointer_button_down_on() || w.clicked())
        && let Some(at) = w.interact_pointer_pos()
    {
        let off = at - c;
        let out_of = (off.length() / radius).min(1.0);
        let h = (1.0 - out_of) * ZENITH;
        let a = if off.length() < 1.0e-3 {
            now_a
        } else {
            wrap(r.node, azimuth, off.x.atan2(off.y) / TAU)
        };
        let writes: Vec<_> = [(azimuth, a, now_a), (height, h, now_h)]
            .into_iter()
            .zip(free)
            .filter(|((_, to, now), free)| *free && (to - now).abs() > 1.0e-6)
            .map(|((key, to, _), _)| float(key, to))
            .collect();
        if !writes.is_empty() {
            out.push(RegionEvent::Controls(writes));
        }
    }

    let mut painter =
        r.ui.painter()
            .with_clip_rect(r.ui.clip_rect().intersect(square.expand(2.0 * z)));
    if !steers {
        painter.multiply_opacity(INERT);
    }
    let hair = (1.0 * z).max(1.0);
    painter.circle_filled(c, radius, r.theme.bg_primary());
    for ring in [1.0 / 3.0, 2.0 / 3.0] {
        painter.circle_stroke(c, radius * ring, Stroke::new(hair, r.theme.border_subtle()));
    }
    let faint = Stroke::new(hair, r.theme.border_subtle().gamma_multiply(0.6));
    painter.line_segment([c - vec2(radius, 0.0), c + vec2(radius, 0.0)], faint);
    painter.line_segment([c - vec2(0.0, radius), c + vec2(0.0, radius)], faint);
    painter.circle_stroke(c, radius, Stroke::new(hair, r.theme.border_normal()));

    // The camera, on the horizon where it stands round its target.
    let (orbit, ..) = camera(r, globe);
    let cam = on_disk(c, radius, orbit, 0.0);
    painter.circle_filled(cam, 3.5 * z, r.theme.primary());

    let at = on_disk(c, radius, now_a, now_h);
    let amber = r.theme.port(PortType::Action);
    painter.line_segment([c, at], Stroke::new(hair, amber.gamma_multiply(0.4)));
    sun_mark(r, &painter, at, 4.0 * z, 1.0);
    out
}
