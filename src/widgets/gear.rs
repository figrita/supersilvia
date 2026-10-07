// SPDX-License-Identifier: AGPL-3.0-or-later

//! A gear's own picture: what a Master Gear or a Ratio Gear is set to, turning at its real
//! rate, and what a loop of it needs; and above a Ratio Gear's picture, its Teeth.
//!
//! **One region, two pictures**, by the gear's Display option, drawn in one fixed height so
//! the node never changes size. The **Rosette**, the default: for a ratio `p/q` a still
//! spirograph that turns once round an input cycle, so it winds `q` loops, the input cycles it
//! takes to close, and waves in and out `p` times across them, the output's cycles, its
//! petals; a tick at every petal's tip, one for each whole output cycle the curve carries,
//! `p` of them evenly round the ring since the Teeth are drawn in lowest terms, the one at
//! the top a touch longer where the first (`k = 0`) begins; and a dot riding the curve at the
//! output's phase, which in Reverse rides it the other way. Past a few dozen the ticks thin
//! to an even stride so a large `p` reads as marks, not a solid ring. A ×3 is three petals in
//! one loop, each with its tick, a ÷4 one petal wound over four, its one tick at the top.
//! A Master Gear's rosette is a ring with a clock face's twelve ticks and the dot. The
//! **Gears**: the input's gear of `k·p` teeth driving the output's of `k·q`, `k` keeping both
//! between 6 and 48 and the hub printing the ratio past that; a Master Gear is one gear of
//! twelve teeth, turning once a cycle. Both turn by the gear's own phases, never an animation
//! clock, so a paused show is still. Both draw the Teeth in lowest terms: 2 : 4 is the
//! rosette 1 : 2 is.
//!
//! Beside the picture, the ratio and what it closes in; under it, on a Master Gear, what a
//! loop of it needs, read from the chains below it (`nodes::chain::caption`). Read-only:
//! the region does not claim the pointer, and a hand carries the node by it.
//!
//! **The Teeth row** ([`TEETH`]) is a Ratio Gear's two whole numbers, `p : q`, with no port:
//! a line the height of a port row's, the word Teeth where a row's label stands, and two
//! s-numbers either side of a colon, as narrow as two and the colon need to fit where a
//! row's one control and its label do. Each is the node's own hidden control drawn by
//! `RegionUi::number`, so it types, drags and steps as any other whole number does, and
//! shows the Teeth as they were typed. Under the numbers, as wide as they are, the gear's
//! direction, Forward | Reverse, the segmented switch Free | Loop is (`widgets::segments`):
//! the Teeth's line has no room for it.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::gear::{self, Reading};
use crate::ui::{canvas, number, theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    WidgetType, pos2, vec2,
};
use std::f32::consts::{PI, TAU};

/// The region's height, in world units: the picture's square, and the caption's line under
/// it.
pub const HEIGHT: f32 = 92.0;
/// The padding round the picture and between it and the words.
const PAD: f32 = 6.0;
/// The caption's line at the foot.
const LINE: f32 = 13.0;
/// The picture's corner, xypad's.
const CORNER: f32 = 4.0;
/// A gear's teeth, by the proposal's rule: `k·p` and `k·q` inside these.
const MIN_TEETH: i64 = 6;
const MAX_TEETH: i64 = 48;
/// The ticks round a Master Gear's ring and the teeth on its gear: a clock face's twelve.
const MASTER_TICKS: i64 = 12;
/// The most event ticks a Ratio Gear's rosette draws before it thins them: past this a
/// petal's own tip is close enough to its neighbors' that every one drawn would read as a
/// solid ring.
const MAX_ROSETTE_TICKS: i64 = 24;

pub const GEAR: RegionDef = RegionDef {
    size,
    show,
    ..RegionDef::EMPTY
};

fn size(_node: &Node) -> f32 {
    HEIGHT
}

/// What the region is to draw, in the form both pictures read.
struct Drawn {
    /// Output cycles, unbounded.
    output: f64,
    /// Input cycles, unbounded; a Master Gear's are its own.
    input: f64,
    /// The ratio in lowest terms.
    p: i64,
    q: i64,
    /// Ticks round a Master Gear's ring, and teeth on its gear.
    ticks: Option<i64>,
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let z = r.zoom;
    let rect = r.rect;
    let side = (rect.height() - LINE * z - 2.0 * PAD * z).max(8.0);
    let square = Rect::from_min_size(rect.min + vec2(PAD * z, PAD * z), vec2(side, side));
    let text = Rect::from_min_max(
        pos2(square.max.x + PAD * z, square.min.y),
        pos2(rect.max.x - PAD * z, square.max.y),
    );
    let foot = Rect::from_min_max(
        pos2(rect.min.x + PAD * z, square.max.y),
        pos2(rect.max.x - PAD * z, rect.max.y),
    );
    let painter =
        r.ui.painter()
            .with_clip_rect(r.ui.clip_rect().intersect(rect));
    let radius = CornerRadius::same((CORNER * z).round() as u8);
    painter.rect_filled(square, radius, r.theme.bg_primary());
    painter.rect_stroke(
        square,
        radius,
        Stroke::new((1.0 * z).max(1.0), r.theme.border_normal()),
        StrokeKind::Inside,
    );

    let gears = r.node.options.get("display").is_some_and(|d| d == "gears");
    let reading = r.live.gear.copied();
    let drawn = reading.map(|reading| drawn_of(&reading));
    let inner = square.shrink(4.0 * z);
    let colors = Colors {
        curve: r.theme.port(crate::graph::PortType::UniformNumber),
        ticks: r.theme.primary(),
        faint: r.theme.border_subtle(),
        dot: r.theme.text_primary(),
        ground: r.theme.bg_secondary(),
    };
    if let Some(d) = &drawn {
        if gears {
            draw_gears(&painter, inner, d, &colors, z);
        } else {
            draw_rosette(&painter, inner, d, &colors, z);
        }
    }

    // The words beside it: what it is set to, what it closes in, what is waiting.
    let big = FontId::proportional(theme::font_size(theme::FONT_BASE, z));
    let small = FontId::proportional(theme::font_size(theme::FONT_TINY, z));
    let lines = words(reading.as_ref(), drawn.as_ref(), gears);
    let mut y = text.min.y + 2.0 * z;
    for (i, line) in lines.iter().enumerate() {
        let (font, color, step) = if i == 0 {
            (big.clone(), r.theme.readout(), 16.0)
        } else {
            (small.clone(), r.theme.text_secondary(), 12.0)
        };
        painter.text(pos2(text.min.x, y), Align2::LEFT_TOP, line, font, color);
        y += step * z;
    }
    // The caption whole on hover and in the tree, and on the node up to its reason, which
    // the node's width has no room for.
    let caption = r.live.gear_caption.unwrap_or_default();
    if !caption.is_empty() {
        let shown = caption.split(" (").next().unwrap_or(caption);
        painter.text(
            pos2(foot.min.x, foot.center().y),
            Align2::LEFT_CENTER,
            shown,
            small,
            r.theme.text_muted(),
        );
    }

    // Painted text is not in the accessibility tree: the region says what it shows.
    let name = format!("{} {}", r.name("gear"), lines.join(" · "));
    let w =
        r.ui.interact(rect, r.ui.id().with(("gear", r.id)), Sense::hover());
    crate::ui::accessible(&w, WidgetType::Label, &name);
    if !caption.is_empty() {
        let w =
            r.ui.interact(foot, r.ui.id().with(("gear-caption", r.id)), Sense::hover());
        crate::ui::accessible(
            &w,
            WidgetType::Label,
            format_args!("{} {caption}", r.name("loop")),
        );
        w.on_hover_text(caption);
    }
    Vec::new()
}

/// The colors a picture is drawn in, from the theme.
struct Colors {
    curve: Color32,
    ticks: Color32,
    faint: Color32,
    dot: Color32,
    ground: Color32,
}

fn drawn_of(reading: &Reading) -> Drawn {
    match *reading {
        Reading::Master { cycles, .. } => Drawn {
            output: cycles,
            input: cycles,
            p: 1,
            q: 1,
            ticks: Some(MASTER_TICKS),
        },
        Reading::Ratio {
            input,
            output,
            p,
            q,
        } => {
            let (p, q) = gear::reduced(p, q);
            Drawn {
                output,
                input,
                p,
                q,
                ticks: None,
            }
        }
    }
}

/// The lines beside the picture.
fn words(reading: Option<&Reading>, drawn: Option<&Drawn>, gears: bool) -> Vec<String> {
    let (Some(reading), Some(d)) = (reading, drawn) else {
        return vec!["—".to_string()];
    };
    let mut out = Vec::new();
    match *reading {
        Reading::Master { seconds, held, .. } => {
            out.push(format!("{seconds:.3} s"));
            out.push("a cycle".to_string());
            if held {
                out.push("held".to_string());
            }
        }
        Reading::Ratio { p, q, .. } => {
            out.push(label(d.p, d.q));
            out.push(if d.q == 1 {
                "every cycle in".to_string()
            } else {
                format!("every {} cycles in", d.q)
            });
            // Under Gears, the Teeth as the row has them, typed and unreduced, whatever the
            // wheels drawn for them carry; under Rosette, its petals and loops.
            out.push(if gears {
                format!("{p} : {q}")
            } else {
                format!(
                    "{} petal{} · {} loop{}",
                    d.p,
                    if d.p == 1 { "" } else { "s" },
                    d.q,
                    if d.q == 1 { "" } else { "s" }
                )
            });
        }
    }
    out
}

/// A ratio in lowest terms as the words beside the picture write it: `×3`, `÷4`, `3/2`.
fn label(p: i64, q: i64) -> String {
    if q == 1 {
        format!("×{p}")
    } else if p == 1 {
        format!("÷{q}")
    } else {
        format!("{p}/{q}")
    }
}

/// The teeth two meshing gears carry for a ratio `p/q`: `k·p` driving `k·q`, the smallest `k`
/// that puts both at six or more, or `None` where that puts one past forty-eight.
pub fn teeth(p: i64, q: i64) -> Option<(i64, i64)> {
    let (p, q) = (p.abs(), q.abs());
    if p == 0 || q == 0 {
        return None;
    }
    let k = (MIN_TEETH + p.min(q) - 1) / p.min(q);
    let (a, b) = (k * p, k * q);
    (a <= MAX_TEETH && b <= MAX_TEETH).then_some((a, b))
}

/// The rosette: the angle makes a full turn each input cycle, `q` loops, and the radius `p`
/// waves across them, with a tick at each of the `p` petal tips — where a whole output cycle
/// lands on the curve, `k`'s at input fraction `k·q/p` of the ring for `k` from `0` to `p − 1`
/// — the top one, `k = 0`, drawn longer, and a dot at the output's place on the curve.
fn draw_rosette(painter: &eframe::egui::Painter, rect: Rect, d: &Drawn, c: &Colors, z: f32) {
    let center = rect.center();
    let big = rect.width().min(rect.height()) * 0.5 / 1.12;
    let at = |angle: f32, rad: f32| center + vec2(angle.cos(), angle.sin()) * rad;
    let top = -PI / 2.0;
    let line = |w: f32, color: Color32| Stroke::new((w * z).max(0.8), color);

    if let Some(ticks) = d.ticks {
        // A Master Gear: a ring, its ticks — the first one long — and the dot.
        painter.circle_stroke(center, big * 0.8, line(1.4, c.curve));
        let n = ticks.clamp(1, 64);
        for k in 0..n {
            let a = top + TAU * k as f32 / n as f32;
            let (from, to) = if k == 0 { (0.62, 1.1) } else { (0.86, 1.02) };
            painter.line_segment(
                [at(a, big * from), at(a, big * to)],
                line(if k == 0 { 1.8 } else { 1.2 }, c.ticks),
            );
        }
        let a = top + TAU * crate::nodes::phasor::fraction(d.output, 1.0) as f32;
        painter.circle_filled(at(a, big * 0.8), 3.0 * z, c.dot);
        return;
    }

    painter.circle_stroke(center, big * 0.94, line(1.0, c.faint));
    rosette_curve(painter, center, big, d.p, d.q, line(1.4, c.curve));
    let q = d.q.max(1);
    // A tick at every petal tip — where a whole output cycle lands on the curve — the one at
    // the top, where the first (`k = 0`) begins, drawn a touch longer so loop start stays
    // readable.
    for (a, top_tick) in rosette_tick_angles(d.p, q) {
        let (from, to) = if top_tick { (0.94, 1.16) } else { (0.96, 1.06) };
        painter.line_segment(
            [at(a, big * from), at(a, big * to)],
            line(if top_tick { 1.8 } else { 1.3 }, c.ticks),
        );
    }
    // The dot: where the output is, as a point of the curve it runs along, a turn an input
    // cycle.
    let t = crate::nodes::phasor::fraction(d.output * q as f64 / d.p as f64, q as f64);
    let a = top + TAU * crate::nodes::phasor::fraction(t, 1.0) as f32;
    let wave = (TAU * crate::nodes::phasor::fraction(d.output, 1.0) as f32).cos();
    painter.circle_filled(at(a, big * (0.6 + 0.3 * wave)), 3.0 * z, c.dot);
}

/// The angles a Ratio Gear's rosette ticks sit at, from `top = -PI / 2`, clockwise same as
/// [`rosette_curve`]'s own parametrization: one for each whole output cycle the curve
/// carries, output cycle `k` at input fraction `k·q/p` of the ring, so `p` of them, evenly
/// spaced since the Teeth arrive in lowest terms, and `k = 0`, the top one where every loop
/// begins, is among them. Past [`MAX_ROSETTE_TICKS`] a large `p` is thinned to an even stride
/// so they read as marks, not a solid ring; the top tick is always kept. The `bool` says which
/// is the top one, drawn longer.
fn rosette_tick_angles(p: i64, q: i64) -> Vec<(f32, bool)> {
    let p = p.max(1);
    let q = q.max(1);
    let stride = ((p + MAX_ROSETTE_TICKS - 1) / MAX_ROSETTE_TICKS).max(1);
    (0..p)
        .step_by(stride as usize)
        .map(|k| (-PI / 2.0 + TAU * (k as f32 * q as f32 / p as f32), k == 0))
        .collect()
}

/// One rosette's curve.
fn rosette_curve(
    painter: &eframe::egui::Painter,
    center: Pos2,
    big: f32,
    p: i64,
    q: i64,
    stroke: Stroke,
) {
    let q = q.max(1);
    let n = (240 * q.unsigned_abs() + 40 * p.unsigned_abs()).min(4000) as usize;
    let points: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = q as f32 * i as f32 / n as f32;
            let a = -PI / 2.0 + TAU * t;
            let rad = big * (0.6 + 0.3 * (TAU * p as f32 * t / q as f32).cos());
            center + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    painter.add(Shape::line(points, stroke));
}

/// The gears: two meshing, the input's driving the output's, or a Master Gear's one.
fn draw_gears(painter: &eframe::egui::Painter, rect: Rect, d: &Drawn, c: &Colors, z: f32) {
    let stroke = |color: Color32| Stroke::new((1.0 * z).max(0.8), color);
    if let Some(ticks) = d.ticks {
        let teeth = ticks.clamp(3, MAX_TEETH);
        let r = rect.width().min(rect.height()) * 0.5 * 0.78;
        let turn = -PI / 2.0 + TAU * crate::nodes::phasor::fraction(d.output, 1.0) as f32;
        gear(
            painter,
            rect.center(),
            r,
            teeth,
            turn,
            c.ground,
            stroke(c.ticks),
            z,
        );
        return;
    }
    let fits = teeth(d.p, d.q);
    let (ti, to) = fits.unwrap_or((12, 12));
    // Radii proportional to teeth, the pair as wide as the square allows.
    let module = (rect.width() / (2.0 * (ti + to) as f32 + 4.0))
        .min(rect.height() / (2.0 * ti.max(to) as f32 + 4.0));
    let (ri, ro) = (ti as f32 * module, to as f32 * module);
    let y = rect.center().y;
    let left = rect.center().x - (ri + ro);
    let (ci, co) = (pos2(left + ri, y), pos2(left + 2.0 * ri + ro, y));
    let turn_in = TAU * crate::nodes::phasor::fraction(d.input, 1.0) as f32;
    let turn_out =
        -TAU * crate::nodes::phasor::fraction(d.output, 1.0) as f32 + PI + PI / to as f32;
    gear(painter, ci, ri, ti, turn_in, c.ground, stroke(c.ticks), z);
    gear(painter, co, ro, to, turn_out, c.ground, stroke(c.curve), z);
    if fits.is_none() {
        painter.text(
            co,
            Align2::CENTER_CENTER,
            label(d.p, d.q),
            FontId::proportional(theme::font_size(theme::FONT_TINY, z)),
            c.dot,
        );
    }
}

/// One gear of `teeth` teeth round a pitch circle of radius `r`, turned by `turn`: a fan
/// from its center, which a gear's outline is star-shaped about, its outline, its hub and a
/// spoke that shows it turning.
// A gear is its center, its size, its teeth and its turn, and the three ways it is painted.
#[allow(clippy::too_many_arguments)]
fn gear(
    painter: &eframe::egui::Painter,
    center: Pos2,
    r: f32,
    teeth: i64,
    turn: f32,
    fill: Color32,
    stroke: Stroke,
    z: f32,
) {
    let step = TAU / teeth as f32;
    // A tooth is the size a twelve-tooth gear's would be, or smaller where there are more of
    // them, so a gear of four teeth has four teeth and not four lobes.
    let pitch = step.min(TAU / 12.0);
    let depth = (r * pitch * 0.42).clamp(0.9 * z, 8.0 * z);
    let (outer, inner) = (r + depth / 2.0, r - depth / 2.0);
    let mut outline = Vec::with_capacity(teeth as usize * 6);
    for i in 0..teeth {
        let a = turn + i as f32 * step;
        for (rad, da) in [
            (inner, -0.27 * pitch),
            (outer, -0.14 * pitch),
            (outer, 0.14 * pitch),
            (inner, 0.27 * pitch),
        ] {
            let an = a + da;
            outline.push(center + vec2(an.cos(), an.sin()) * rad);
        }
        // Round to the next tooth along the root, where the teeth are far apart.
        let (from, to) = (a + 0.27 * pitch, a + step - 0.27 * pitch);
        let arcs = ((to - from) / (TAU / 48.0)).floor() as i32;
        for k in 1..arcs {
            let an = from + (to - from) * k as f32 / arcs as f32;
            outline.push(center + vec2(an.cos(), an.sin()) * inner);
        }
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, fill);
    for p in &outline {
        mesh.colored_vertex(*p, fill);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::closed_line(outline, stroke));
    painter.circle_filled(center, (r * 0.16).max(1.2 * z), stroke.color);
    painter.line_segment(
        [
            center + vec2(turn.cos(), turn.sin()) * r * 0.25,
            center + vec2(turn.cos(), turn.sin()) * r * 0.72,
        ],
        stroke,
    );
}

// ------------------------------------------------------------------------------- teeth

/// A Ratio Gear's Teeth: the word, and its two numbers either side of a colon, on one row,
/// and under the numbers its direction, Forward | Reverse.
pub const TEETH: RegionDef = RegionDef {
    size: teeth_size,
    show: teeth_show,
    ..RegionDef::EMPTY
};

/// Each of the Teeth's two s-numbers: narrower than a row's, so two and the colon fit where
/// a row's one control and its label do, and wide enough to keep both steppers.
const TEETH_NUMBER: f32 = 60.0;
/// The colon's room between them.
const COLON: f32 = 16.0;
/// Where a row's control stops short of the body's right edge: the input block's inset and
/// the control's own.
const ROW_RIGHT: f32 = canvas::ROW_BLOCK_INSET + 8.0;
/// Between the numbers and the switch under them.
const SWITCH_GAP: f32 = 4.0;

/// The Teeth's line, the height of a port row's, then the switch's: as far under the numbers
/// as the numbers are under the region's top.
fn teeth_size(_node: &Node) -> f32 {
    canvas::CONTROL_ROW_PITCH + SWITCH_GAP + super::SEGMENTS_HEIGHT
}

/// What the Teeth say in words: `Turns 3 times for every 2 turns of its parent.`, and in
/// Reverse `…of its parent, backwards.`
pub fn teeth_words(p: i64, q: i64, reverse: bool) -> String {
    format!(
        "Turns {p} time{} for every {q} turn{} of its parent{}.",
        if p == 1 { "" } else { "s" },
        if q == 1 { "" } else { "s" },
        if reverse { ", backwards" } else { "" }
    )
}

fn teeth_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let z = r.zoom;
    let rect = r.rect;
    let y = rect.min.y + canvas::CONTROL_ROW_PITCH * 0.5 * z;
    // The slab a lone input row stands on, under both lines: short of the far edge, its two
    // outer corners round.
    let slab = rect.with_max_x(rect.max.x - canvas::ROW_BLOCK_INSET * z);
    let round = (canvas::ROW_BLOCK_RADIUS * z).round() as u8;
    r.ui.painter().rect_filled(
        slab,
        CornerRadius {
            ne: round,
            se: round,
            ..Default::default()
        },
        r.theme.bg_secondary(),
    );
    r.ui.painter().text(
        pos2(rect.min.x + crate::ui::node_widget::LABEL_INSET * z, y),
        Align2::LEFT_CENTER,
        "Teeth",
        FontId::proportional(theme::font_size(theme::FONT_TINY, z)),
        r.theme.text_secondary(),
    );
    let size = vec2(TEETH_NUMBER, number::HEIGHT) * z;
    let q_rect = Rect::from_min_size(
        pos2(rect.max.x - ROW_RIGHT * z - size.x, y - size.y * 0.5),
        size,
    );
    let colon = Rect::from_min_max(
        pos2(q_rect.min.x - COLON * z, q_rect.min.y),
        q_rect.left_bottom(),
    );
    let p_rect = Rect::from_min_size(pos2(colon.min.x - size.x, q_rect.min.y), size);
    r.ui.painter().text(
        colon.center(),
        Align2::CENTER_CENTER,
        ":",
        FontId::proportional(theme::font_size(theme::FONT_BASE, z)),
        r.theme.text_secondary(),
    );
    let mut out = Vec::new();
    out.extend(r.number(p_rect, gear::TEETH_P));
    out.extend(r.number(q_rect, gear::TEETH_Q));

    // The direction, as wide as `p : q` and under it, on whole pixels: the numbers' foot is a
    // half-point, and a hairline from there bleeds into the pixel above.
    let ppp = r.ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let top = snap(p_rect.max.y + SWITCH_GAP * z);
    let switch = Rect::from_min_max(
        pos2(snap(p_rect.min.x), top),
        pos2(snap(q_rect.max.x), top + snap(super::SEGMENTS_HEIGHT * z)),
    );
    let direction = gear::DIRECTION;
    let chosen = r
        .node
        .options
        .get(direction.key)
        .map_or(direction.default, String::as_str);
    let name = r.name(direction.key);
    if let Some(i) = super::segments(r.ui, switch, direction.choices, chosen, &name, z, r.theme) {
        out.push(RegionEvent::Option {
            key: direction.key,
            value: direction.choices[i].0,
        });
    }

    // Painted text is not in the accessibility tree: the row says what it is set to, and its
    // words say it whole on hover.
    let (p, q) = gear::teeth_of(r.node);
    let reverse = chosen == gear::REVERSE;
    let words = Rect::from_min_max(rect.min, pos2(p_rect.min.x, rect.max.y));
    let w =
        r.ui.interact(words, r.ui.id().with(("teeth", r.id)), Sense::hover());
    crate::ui::accessible(
        &w,
        WidgetType::Label,
        format_args!(
            "{} {p} : {q}{}",
            r.name("teeth"),
            if reverse { " in reverse" } else { "" }
        ),
    );
    w.on_hover_text(teeth_words(p, q, reverse));
    let w =
        r.ui.interact(colon, r.ui.id().with(("teeth-colon", r.id)), Sense::hover());
    w.on_hover_text(teeth_words(p, q, reverse));
    out
}

#[cfg(test)]
mod tests {
    use super::{MAX_ROSETTE_TICKS, Reading, drawn_of, rosette_tick_angles, teeth};
    use std::f32::consts::{PI, TAU};

    /// The input's `k·p` teeth over the output's `k·q`, both between six and forty-eight.
    #[test]
    fn teeth_are_the_ratio_reduced_and_bounded() {
        assert_eq!(teeth(3, 1), Some((18, 6)));
        assert_eq!(teeth(1, 4), Some((6, 24)));
        assert_eq!(teeth(3, 2), Some((9, 6)));
        assert_eq!(teeth(1, 1), Some((6, 6)));
        assert_eq!(teeth(1, 8), Some((6, 48)));
        assert_eq!(teeth(1, 16), None, "past forty-eight, the hub prints it");
        assert_eq!(teeth(-2, 1), Some((12, 6)));
        assert_eq!(teeth(0, 1), None);
    }

    /// Every tick sits at a petal tip, the curve's own maximum: `rad ∝ cos(TAU·p·t/q)` at
    /// `t = k·q/p` is `cos(TAU·k) = 1`, its largest value, for every `k` the angles carry.
    fn angle_is_a_petal_tip(p: i64, q: i64, angle: f32) {
        let t = q as f32 * (angle - (-PI / 2.0)) / TAU;
        let rad = (TAU * p as f32 * t / q as f32).cos();
        assert!(
            rad > 0.999,
            "p={p} q={q} angle={angle} is not a petal tip (cos={rad})"
        );
    }

    /// ×5: five teeth over one, so five ticks, evenly a fifth of a turn apart, the top one
    /// (`k = 0`) among them and marked long.
    #[test]
    fn five_ticks_for_a_times_five() {
        let ticks = rosette_tick_angles(5, 1);
        assert_eq!(ticks.len(), 5);
        assert_eq!(ticks.iter().filter(|(_, top)| *top).count(), 1);
        assert!(ticks[0].1, "k = 0 is first and is the top tick");
        for (k, (a, _)) in ticks.iter().enumerate() {
            let want = -PI / 2.0 + TAU * k as f32 / 5.0;
            assert!((a - want).abs() < 1e-5, "tick {k}: {a} != {want}");
            angle_is_a_petal_tip(5, 1, *a);
        }
    }

    /// ÷4: one tooth over four, so one petal and one tick, at the top, where the loop (and
    /// the only output cycle in it) begins.
    #[test]
    fn one_tick_at_the_top_for_a_divide_four() {
        let ticks = rosette_tick_angles(1, 4);
        assert_eq!(ticks.len(), 1);
        let (a, top) = ticks[0];
        assert!(top);
        assert!((a - (-PI / 2.0)).abs() < 1e-5);
        angle_is_a_petal_tip(1, 4, a);
    }

    /// 3 : 2: three output cycles land across the two input cycles the rosette winds, each at
    /// its own petal tip, a third of a turn from the last.
    #[test]
    fn three_ticks_for_three_over_two() {
        let ticks = rosette_tick_angles(3, 2);
        assert_eq!(ticks.len(), 3);
        assert_eq!(ticks.iter().filter(|(_, top)| *top).count(), 1);
        assert!(ticks[0].1);
        for (k, (a, _)) in ticks.iter().enumerate() {
            let want = -PI / 2.0 + TAU * (k as f32 * 2.0 / 3.0);
            assert!((a - want).abs() < 1e-5, "tick {k}: {a} != {want}");
            angle_is_a_petal_tip(3, 2, *a);
        }
    }

    /// Reverse carries no sign into the Teeth a `Reading::Ratio` draws with — only its
    /// `output` runs backwards — so `drawn_of` hands the rosette the same `p, q` whichever way
    /// the gear turns, and the ticks it draws from them do not move.
    #[test]
    fn ticks_do_not_move_in_reverse() {
        let forward = drawn_of(&Reading::Ratio {
            input: 1.25,
            output: 3.75,
            p: 3,
            q: 2,
        });
        let reverse = drawn_of(&Reading::Ratio {
            input: 1.25,
            output: -3.75,
            p: 3,
            q: 2,
        });
        assert_eq!((forward.p, forward.q), (reverse.p, reverse.q));
        assert_eq!(
            rosette_tick_angles(forward.p, forward.q),
            rosette_tick_angles(reverse.p, reverse.q)
        );
    }

    /// Past [`MAX_ROSETTE_TICKS`] a large `p` thins to an even stride rather than drawing
    /// every petal's tick, so they stay marks and not a solid ring; the top tick is always
    /// among them.
    #[test]
    fn large_p_thins_instead_of_filling_a_ring() {
        let ticks = rosette_tick_angles(64, 1);
        assert!(
            (ticks.len() as i64) <= MAX_ROSETTE_TICKS,
            "{} ticks drawn for p = 64",
            ticks.len()
        );
        assert!(!ticks.is_empty());
        assert!(ticks[0].1, "the top tick survives thinning");
        for (a, _) in &ticks {
            angle_is_a_petal_tip(64, 1, *a);
        }
    }
}
