// SPDX-License-Identifier: AGPL-3.0-or-later

//! A gear's own picture: what a Master Gear or a Ratio Gear is set to, turning at its real
//! rate, and what a loop of it needs.
//!
//! **One region, two pictures**, by the gear's Display option, drawn in one fixed height so
//! the node never changes size. The **Rosette**, the default: for a ratio `p/q` a still
//! spirograph that turns once round an input cycle, so it winds `q` loops, the input cycles it
//! takes to close, and waves in and out `p` times across them, the output's cycles, its
//! petals; a tick at the top where every loop begins; and a dot riding the curve at the
//! output's phase. A ×3 is three petals in one loop, a ÷4 one petal wound over four.
//! A Master Gear's rosette is a ring with a clock face's twelve ticks and the dot. The
//! **Gears**: the input's gear of `k·p` teeth driving the output's of `k·q`, `k` keeping both
//! between 6 and 48 and the hub printing the ratio past that; a Master Gear is one gear of
//! twelve teeth, turning once a cycle. Both turn by the gear's own phases, never an animation clock, so a
//! paused show is still. A ratio that is not a small fraction is drawn at the nearest one,
//! the rim broken where it does not close. A ratio waiting for its landing is ghosted behind.
//!
//! Beside the picture, the ratio and what it closes in; under it, on a Master Gear, what a
//! loop of it needs, read from the chains below it (`nodes::chain::caption`). Read-only:
//! the region does not claim the pointer, and a hand carries the node by it.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::gear::{Reading, ladder};
use crate::ui::theme;
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
    /// The ratio as a fraction, and whether it is exactly that.
    p: i64,
    q: i64,
    exact: bool,
    /// Ticks round a Master Gear's ring, and teeth on its gear.
    ticks: Option<i64>,
    /// A ratio waiting for its landing.
    pending: Option<(i64, i64)>,
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
    let big = FontId::monospace(theme::font_size(theme::FONT_BASE, z));
    let small = FontId::monospace(theme::font_size(theme::FONT_TINY, z));
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
            exact: true,
            ticks: Some(MASTER_TICKS),
            pending: None,
        },
        Reading::Ratio {
            input,
            output,
            ratio,
            pending,
            ..
        } => {
            let (p, q, exact) = ladder::nearest_fraction(ratio);
            Drawn {
                output,
                input,
                p,
                q,
                exact,
                ticks: None,
                pending: pending.map(|r| {
                    let (p, q, _) = ladder::nearest_fraction(r);
                    (p, q)
                }),
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
        Reading::Ratio {
            ratio,
            pending,
            held,
            ..
        } => {
            out.push(ladder::label(ratio));
            out.push(if !d.exact {
                "never closes".to_string()
            } else if d.q.abs() == 1 {
                "every cycle in".to_string()
            } else {
                format!("every {} cycles in", d.q.abs())
            });
            out.push(if gears {
                match teeth(d.p, d.q) {
                    Some((a, b)) => format!("{a} : {b} teeth"),
                    None => "teeth past 48".to_string(),
                }
            } else {
                format!(
                    "{} petal{} · {} loop{}",
                    d.p.abs(),
                    if d.p.abs() == 1 { "" } else { "s" },
                    d.q,
                    if d.q == 1 { "" } else { "s" }
                )
            });
            if let Some(p) = pending {
                out.push(format!("→ {} next", ladder::label(p)));
            } else if held {
                out.push("held".to_string());
            }
        }
    }
    out
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
/// waves across them, with a tick at the top where each loop begins, and a dot at the
/// output's place on the curve.
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

    // The ghost of a ratio waiting to land, behind the one running.
    if let Some((p, q)) = d.pending {
        rosette_curve(painter, center, big, p, q, line(1.0, c.faint));
    }
    // The rim, broken at the top where a ratio that is not the fraction drawn does not close.
    if d.exact {
        painter.circle_stroke(center, big * 0.94, line(1.0, c.faint));
    } else {
        let gap = 0.35;
        let points: Vec<Pos2> = (0..=90)
            .map(|i| at(top + gap + (TAU - 2.0 * gap) * i as f32 / 90.0, big * 0.94))
            .collect();
        painter.add(Shape::line(points, line(1.0, c.faint)));
    }
    rosette_curve(painter, center, big, d.p, d.q, line(1.4, c.curve));
    let q = d.q.max(1);
    painter.line_segment(
        [at(top, big * 0.96), at(top, big * 1.1)],
        line(1.6, c.ticks),
    );
    // The dot: where the output is, as a point of the curve it runs along, a turn an input
    // cycle.
    let t = if d.p == 0 {
        crate::nodes::phasor::fraction(d.input, q as f64)
    } else {
        crate::nodes::phasor::fraction(d.output * q as f64 / d.p as f64, q as f64)
    };
    let a = top + TAU * crate::nodes::phasor::fraction(t, 1.0) as f32;
    let wave = (TAU * crate::nodes::phasor::fraction(d.output, 1.0) as f32).cos();
    painter.circle_filled(at(a, big * (0.6 + 0.3 * wave)), 3.0 * z, c.dot);
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
    let fits = teeth(d.p, d.q).filter(|_| d.exact);
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
            ladder::label(d.p as f64 / d.q.max(1) as f64),
            FontId::monospace(theme::font_size(theme::FONT_TINY, z)),
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

#[cfg(test)]
mod tests {
    use super::teeth;

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
}
