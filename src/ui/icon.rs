// SPDX-License-Identifier: AGPL-3.0-or-later

//! The editor's small icons, painted: a plus, a minus, the chevrons, the triangles, a cross,
//! a warning, a dot, and the few others a button or a status line carries.
//!
//! **No icon is a text glyph.** A glyph sits in the box its face gives it: a proportional
//! face puts a `+` on its own baseline at its own advance and a `▾` falls through to a symbol
//! face with another baseline and another advance, so neither lands in the middle of the
//! button it labels. These are geometry instead, centered on the rect they are given, sized
//! from it, and stroked at a whole number of physical pixels whose parity matches where that
//! center falls, so a line is crisp at any interface size rather than smeared across two rows.
//!
//! Every helper here keeps the widget's **accessible name**: a button that shows a `+` is
//! still found by what it does, which is how a screen reader, a test and the agent-driven
//! layer find it. The marks a node draws over its picture and in its header — the `?`, the
//! close, the pop-out — are `node_widget`'s, drawn the same way at silvia's own geometry;
//! see design-system.md.

use eframe::egui::{
    self, Atom, Color32, Id, InnerResponse, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

/// One painted icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Add, or step up.
    Plus,
    /// Step down.
    Minus,
    /// A list opens below.
    ChevronDown,
    /// Fold to the left edge, or open from the right.
    ChevronLeft,
    /// Fold to the right edge, or open from the left.
    ChevronRight,
    /// A menu stands above, closed.
    TriangleUp,
    /// A menu stands above, open.
    TriangleDown,
    /// A submenu, or a jump to somewhere else.
    TriangleRight,
    /// Remove.
    Close,
    /// Something is costing more than it should, or failed.
    Warning,
    /// An upright stroke and its point, for a disc or a mark that is already a warning.
    Exclaim,
    /// On, live, in use.
    Dot,
    /// Off: the [`Icon::Dot`]'s outline at the same size.
    Ring,
    /// The project: a frame split down the middle.
    Project,
    /// A loopback: an arc that comes round to an arrowhead.
    Loopback,
}

/// The stroke weight, as a fraction of the icon's box, before it is rounded to whole pixels.
const WEIGHT: f32 = 0.1;

/// A chevron's half-width, as a fraction of the icon's box.
const CHEVRON: f32 = 0.25;

/// A triangle's circumradius, as a fraction of the icon's box.
const TRIANGLE: f32 = 0.4;

/// A dot's radius, and a ring's outer radius, as a fraction of the icon's box.
const DOT: f32 = 0.3;

/// Paint `icon` centered in `rect`, in `color`, sized from the smaller side of `rect`.
///
/// The painter's own fade applies, so a disabled `Ui` dims the icon as it dims text.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    if s <= 0.0 || color == Color32::TRANSPARENT {
        return;
    }
    let ppp = painter.pixels_per_point();
    let grid = Grid::new(rect.center(), s, ppp);
    let sw = grid.stroke;
    let c = grid.center;
    let stroke = Stroke::new(sw, color);
    match icon {
        Icon::Plus | Icon::Minus => {
            // Bars as blocks, so the plus's crossing is painted once and a faded color does
            // not darken where the two bars meet.
            let arm = grid.whole(0.26 * s);
            let half = sw * 0.5;
            block(
                painter,
                Rect::from_min_max(pos2(c.x - arm, c.y - half), pos2(c.x + arm, c.y + half)),
                color,
            );
            if icon == Icon::Plus {
                for (top, bottom) in [(c.y - arm, c.y - half), (c.y + half, c.y + arm)] {
                    block(
                        painter,
                        Rect::from_min_max(pos2(c.x - half, top), pos2(c.x + half, bottom)),
                        color,
                    );
                }
            }
        }
        Icon::ChevronDown => chevron(painter, c, vec2(0.0, 1.0), CHEVRON * s, stroke),
        Icon::ChevronLeft => chevron(painter, c, vec2(-1.0, 0.0), CHEVRON * s, stroke),
        Icon::ChevronRight => chevron(painter, c, vec2(1.0, 0.0), CHEVRON * s, stroke),
        Icon::TriangleUp => triangle(painter, rect.center(), vec2(0.0, -1.0), TRIANGLE * s, color),
        Icon::TriangleDown => triangle(painter, rect.center(), vec2(0.0, 1.0), TRIANGLE * s, color),
        Icon::TriangleRight => {
            triangle(painter, rect.center(), vec2(1.0, 0.0), TRIANGLE * s, color);
        }
        Icon::Close => {
            let d = grid.whole(0.2 * s);
            painter.line_segment([c - vec2(d, d), c + vec2(d, d)], stroke);
            painter.line_segment([c + vec2(d, -d), c - vec2(d, -d)], stroke);
        }
        Icon::Warning => warning(painter, ppp, rect.center(), s, color),
        Icon::Exclaim => exclaim(painter, ppp, rect.center(), s, color),
        Icon::Dot => {
            painter.circle_filled(rect.center(), DOT * s, color);
        }
        Icon::Ring => {
            painter.circle_stroke(rect.center(), DOT * s - sw * 0.5, stroke);
        }
        Icon::Project => {
            let frame = Rect::from_center_size(c, Vec2::splat(2.0 * grid.whole(0.34 * s)));
            painter.rect_stroke(frame, 1, stroke, StrokeKind::Inside);
            painter.line_segment([pos2(c.x, frame.min.y), pos2(c.x, frame.max.y)], stroke);
        }
        Icon::Loopback => loopback(painter, rect.center(), s, stroke),
    }
}

/// The pixel grid an icon is drawn on: a stroke a whole number of physical pixels wide, and a
/// center moved by at most half a pixel so that stroke lands on whole pixels.
struct Grid {
    center: Pos2,
    stroke: f32,
    odd: bool,
    ppp: f32,
}

impl Grid {
    /// An odd stroke is centered on a pixel's middle and an even one on the line between two.
    /// The stroke takes the parity of where the rect's center already is when that costs it
    /// less than a third of a pixel of weight, so a rect on the pixel grid keeps its exact
    /// center; otherwise the weight wins and the center moves by half a pixel.
    fn new(center: Pos2, side: f32, ppp: f32) -> Self {
        let base = (side * WEIGHT * ppp).max(1.0);
        let f = (center.y * ppp).rem_euclid(1.0);
        let wants_odd = (0.25..0.75).contains(&f);
        let nearest = base.round();
        let other = if base >= nearest {
            nearest + 1.0
        } else {
            (nearest - 1.0).max(1.0)
        };
        let px = if (nearest % 2.0 == 1.0) == wants_odd || (other - base).abs() > 1.0 / 3.0 {
            nearest
        } else {
            other
        };
        let odd = px % 2.0 == 1.0;
        let snap = |v: f32| {
            if odd {
                ((v * ppp).floor() + 0.5) / ppp
            } else {
                (v * ppp).round() / ppp
            }
        };
        Self {
            center: pos2(snap(center.x), snap(center.y)),
            stroke: px / ppp,
            odd,
            ppp,
        }
    }

    /// A half-length from the center that ends on the line between two pixels, so the end of
    /// a bar is as crisp as its sides.
    fn whole(&self, length: f32) -> f32 {
        let px = length * self.ppp;
        let px = if self.odd {
            (px - 0.5).round().max(0.0) + 0.5
        } else {
            px.round().max(1.0)
        };
        px / self.ppp
    }
}

/// An open chevron of half-width `a`, its point `dir` of the center: the bounding box of what
/// is drawn, the miter at the point included, is what sits on the center.
fn chevron(painter: &Painter, c: Pos2, dir: Vec2, a: f32, stroke: Stroke) {
    let side = dir.rot90();
    // A right-angled miter reaches `w/√2` past its vertex and a butt end `w/(2√2)` short of the
    // ends' line, so the stroked shape is longer on the point's side by the difference.
    let c = c - dir * (stroke.width * (0.707 - 0.354) * 0.5);
    let back = c - dir * (a * 0.5);
    painter.add(Shape::line(
        vec![back + side * a, c + dir * (a * 0.5), back - side * a],
        stroke,
    ));
}

/// `↺`: three quarters of a circle running anticlockwise from the upper right, an arrowhead
/// on its end, the bounding box of the two on `c`.
fn loopback(painter: &Painter, c: Pos2, s: f32, stroke: Stroke) {
    use std::f32::consts::PI;
    let r = 0.28 * s;
    let (from, to) = (-PI / 3.0, PI * 7.0 / 6.0);
    let steps = 18;
    let mut arc: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            pos2(a.cos(), a.sin()) * r
        })
        .collect();
    // The head on the arc's first point, aimed the way the arc runs out of it there.
    let end = arc[0];
    let radial = vec2(from.cos(), from.sin());
    let along = vec2(from.sin(), -from.cos());
    let head = 0.2 * s;
    let mut tip = vec![
        end + along * head,
        end + radial * (head * 0.75),
        end - radial * (head * 0.75),
    ];
    let bounds = Rect::from_points(&arc).union(Rect::from_points(&tip));
    let shift = c - bounds.center();
    for p in arc.iter_mut().chain(tip.iter_mut()) {
        *p += shift;
    }
    painter.add(Shape::line(arc, stroke));
    painter.add(Shape::convex_polygon(tip, stroke.color, Stroke::NONE));
}

/// A filled equilateral triangle of circumradius `r` pointing along `dir`, its corners
/// rounded, its bounding box on `c`.
fn triangle(painter: &Painter, c: Pos2, dir: Vec2, r: f32, color: Color32) {
    use std::f32::consts::TAU;
    let angle = dir.y.atan2(dir.x);
    let v: Vec<Pos2> = (0..3)
        .map(|i| {
            let a = angle + i as f32 * TAU / 3.0;
            pos2(r * a.cos(), r * a.sin())
        })
        .collect();
    // Each corner pulled in along its bisector a little, cut by a short chord: a soft corner
    // at the size these are drawn, without a spike.
    let cut = r * 0.12;
    let mut points = Vec::with_capacity(6);
    for i in 0..3 {
        let p = v[i];
        let to_prev = (v[(i + 2) % 3] - p).normalized();
        let to_next = (v[(i + 1) % 3] - p).normalized();
        points.push(p + to_prev * cut);
        points.push(p + to_next * cut);
    }
    let bounds = Rect::from_points(&points);
    let shift = c - bounds.center();
    painter.add(Shape::convex_polygon(
        points.into_iter().map(|p| p + shift).collect(),
        color,
        Stroke::NONE,
    ));
}

/// A filled triangle `s` across with an exclamation point cut out of it, so the mark reads
/// on any ground without knowing the ground's color.
///
/// The cut is made by painting the triangle in five pieces around it: the cap above the
/// stroke, the two flanks beside it, and the two blocks between the stroke, its point and the
/// base. Every edge two pieces share is horizontal or vertical and on the line between two
/// pixels, so the anti-aliasing of the one meets the other's with no seam; only the
/// triangle's own slanted sides are smoothed.
fn warning(painter: &Painter, ppp: f32, c: Pos2, s: f32, color: Color32) {
    let edge = |v: f32| (v * ppp).round() / ppp;
    let top = c.y - 0.42 * s;
    let bottom = edge(c.y + 0.42 * s);
    let half = 0.5 * s;
    let height = bottom - top;
    // The stroke's width in whole pixels, its center on a pixel's middle or on the line
    // between two to suit, so both its sides are on lines between pixels.
    let px = (s * 0.13 * ppp).round().max(1.0);
    let x = if px % 2.0 == 1.0 {
        ((c.x * ppp).floor() + 0.5) / ppp
    } else {
        (c.x * ppp).round() / ppp
    };
    let (l, r) = (x - px * 0.5 / ppp, x + px * 0.5 / ppp);
    // Laid out from the base up, each part at least a pixel: the margin under the point, the
    // point as tall as the stroke is wide, the gap, and the stroke up to a third of the way.
    let one = 1.0 / ppp;
    let point_bottom = edge(bottom - (0.14 * height).max(one));
    let point_top = point_bottom - px / ppp;
    let stroke_bottom = edge(point_top - (0.1 * height).max(one));
    let stroke_top = edge(top + 0.3 * height).min(stroke_bottom - one);
    let side = |y: f32| half * (y - top) / height;
    let pieces = [
        vec![
            pos2(x, top),
            pos2(x + side(stroke_top), stroke_top),
            pos2(x - side(stroke_top), stroke_top),
        ],
        vec![
            pos2(x - side(stroke_top), stroke_top),
            pos2(l, stroke_top),
            pos2(l, bottom),
            pos2(x - half, bottom),
        ],
        vec![
            pos2(r, stroke_top),
            pos2(x + side(stroke_top), stroke_top),
            pos2(x + half, bottom),
            pos2(r, bottom),
        ],
        vec![
            pos2(l, stroke_bottom),
            pos2(r, stroke_bottom),
            pos2(r, point_top),
            pos2(l, point_top),
        ],
        vec![
            pos2(l, point_bottom),
            pos2(r, point_bottom),
            pos2(r, bottom),
            pos2(l, bottom),
        ],
    ];
    painter.extend(
        pieces
            .into_iter()
            .map(|points| Shape::convex_polygon(points, color, Stroke::NONE)),
    );
}

/// An exclamation point `s` tall on `c`: the stroke, a gap, and its point, square. The stroke
/// is as many whole pixels as an eighth of the height rounds to, centered so its sides fall
/// between pixels, and every height is whole pixels too.
fn exclaim(painter: &Painter, ppp: f32, c: Pos2, s: f32, color: Color32) {
    let px = (s * 0.12 * ppp).round().max(1.0);
    let x = if px % 2.0 == 1.0 {
        ((c.x * ppp).floor() + 0.5) / ppp
    } else {
        (c.x * ppp).round() / ppp
    };
    let w = px / ppp;
    let edge = |v: f32| (v * ppp).round() / ppp;
    let (top, bottom) = (edge(c.y - 0.36 * s), edge(c.y + 0.36 * s));
    let gap = edge(0.1 * s).max(1.0 / ppp);
    let point = w.max(edge(0.14 * s));
    for (from, to) in [(top, bottom - point - gap), (bottom - point, bottom)] {
        block(
            painter,
            Rect::from_min_max(pos2(x - w * 0.5, from), pos2(x + w * 0.5, to)),
            color,
        );
    }
}

/// A rect on whole pixels, filled as two bare triangles: no anti-aliasing, which is exact on
/// whole pixels and which epaint would otherwise draw a block a pixel wide as a thin line.
fn block(painter: &Painter, rect: Rect, color: Color32) {
    if rect.is_positive() {
        let mut mesh = egui::Mesh::default();
        mesh.add_colored_rect(rect, color);
        painter.add(Shape::mesh(mesh));
    }
}

/// The ink egui would give text on this widget: the override where the style sets one, the
/// state's own color otherwise.
fn ink(ui: &Ui, response: &Response) -> Color32 {
    ui.visuals()
        .override_text_color
        .unwrap_or_else(|| ui.style().interact(response).text_color())
}

/// The side of the square an icon is drawn in beside text of the button style: the cap
/// height's neighbourhood rather than the line's, so it reads as one of the words.
fn text_side(ui: &Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Button) * 0.8).round()
}

/// The square an icon-only button is: egui's own button height, as wide as it is tall.
fn square(ui: &Ui) -> (Vec2, Vec2) {
    let h = ui.spacing().interact_size.y;
    let pad = ui.spacing().button_padding;
    let atom = vec2((h - 2.0 * pad.x).max(0.0), (h - 2.0 * pad.y).max(0.0));
    (vec2(h, h), atom)
}

/// The box an icon is painted in on an icon-only button: the button's square less its
/// padding, on the button's own center.
fn inside(ui: &Ui, rect: Rect) -> Rect {
    let pad = ui.spacing().button_padding.y;
    Rect::from_center_size(rect.center(), Vec2::splat(rect.height() - 2.0 * pad))
}

/// Name `response` as a button, enabled as its `Ui` is.
fn named(ui: &Ui, response: &Response, name: &str) {
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
}

/// An egui button showing only `icon`, square, named `name`.
pub fn button(ui: &mut Ui, icon: Icon, name: &str) -> Response {
    let (size, atom) = square(ui);
    let response = ui.add(egui::Button::new(Atom::custom(Id::new("icon"), atom)).min_size(size));
    paint(
        ui.painter(),
        inside(ui, response.rect),
        icon,
        ink(ui, &response),
    );
    named(ui, &response, name);
    response
}

/// A menu button showing only `icon`, square, named `name`, opening `add_contents` below it.
pub fn menu_button<R>(
    ui: &mut Ui,
    icon: Icon,
    name: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let (size, atom) = square(ui);
    let button = egui::Button::new(Atom::custom(Id::new("icon"), atom)).min_size(size);
    let (response, inner) =
        egui::containers::menu::MenuButton::from_button(button).ui(ui, add_contents);
    paint(
        ui.painter(),
        inside(ui, response.rect),
        icon,
        ink(ui, &response),
    );
    named(ui, &response, name);
    InnerResponse::new(inner.map(|i| i.inner), response)
}

/// A submenu entry: `title`, and a painted arrow at the row's far end where egui would type
/// one.
pub fn submenu_button<R>(
    ui: &mut Ui,
    title: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let side = text_side(ui);
    let button =
        egui::Button::new(title).right_text(Atom::custom(Id::new("arrow"), Vec2::splat(side)));
    let (response, inner) =
        egui::containers::menu::SubMenuButton::from_button(button).ui(ui, add_contents);
    let pad = ui.spacing().button_padding.x;
    let rect = response.rect;
    let at = Rect::from_center_size(
        pos2(rect.right() - pad - side * 0.5, rect.center().y),
        Vec2::splat(side),
    );
    paint(ui.painter(), at, Icon::TriangleRight, ink(ui, &response));
    InnerResponse::new(inner.map(|i| i.inner), response)
}

/// An egui button with `icon` before `text`, named by its text. `color` is the ink for both
/// where the caller sets one, as a `RichText` color would.
pub fn labeled(
    ui: &mut Ui,
    icon: Icon,
    text: &str,
    color: Option<Color32>,
    frame: bool,
) -> Response {
    let side = text_side(ui);
    let id = Id::new("icon");
    let mut words = egui::RichText::new(text);
    if let Some(color) = color {
        words = words.color(color);
    }
    let laid = egui::Button::new((Atom::custom(id, Vec2::splat(side)), words))
        .frame(frame)
        .atom_ui(ui);
    if let Some(at) = laid.rect(id) {
        let ink = color.unwrap_or_else(|| ink(ui, &laid.response));
        paint(ui.painter(), at, icon, ink);
    }
    laid.response
}

/// A selectable row with `icon` before `text`, named by its text, as `Ui::selectable_value`
/// would draw it: assigns `value` to `current` when clicked.
pub fn selectable_value<V: PartialEq>(
    ui: &mut Ui,
    current: &mut V,
    value: V,
    icon: Icon,
    text: &str,
) -> Response {
    let side = text_side(ui);
    let id = Id::new("icon");
    let selected = *current == value;
    let laid =
        egui::Button::selectable(selected, (Atom::custom(id, Vec2::splat(side)), text)).atom_ui(ui);
    if let Some(at) = laid.rect(id) {
        let ink = ui
            .style()
            .interact_selectable(&laid.response, selected)
            .text_color();
        paint(ui.painter(), at, icon, ink);
    }
    let mut response = laid.response;
    if response.clicked() && !selected {
        *current = value;
        response.mark_changed();
    }
    response
}

/// A painted icon standing alone in a `Ui`'s flow, the height of a line of text, named
/// `name` so the tree can find what it says.
pub fn label(ui: &mut Ui, icon: Icon, color: Color32, name: &str) -> Response {
    let side = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    paint(ui.painter(), rect, icon, color);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    response
}

#[cfg(test)]
mod tests {
    use super::Grid;
    use eframe::egui::pos2;

    /// A center on the line between two pixels takes an even stroke and keeps its place; one
    /// in a pixel's middle takes an odd stroke and keeps its place; one anywhere else moves by
    /// less than half a pixel.
    #[test]
    fn the_stroke_lands_on_whole_pixels() {
        let g = Grid::new(pos2(10.0, 10.0), 16.0, 1.0);
        assert_eq!((g.center, g.stroke), (pos2(10.0, 10.0), 2.0));
        let g = Grid::new(pos2(10.5, 10.5), 12.0, 1.0);
        assert_eq!((g.center, g.stroke), (pos2(10.5, 10.5), 1.0));
        let g = Grid::new(pos2(10.0, 10.0), 12.0, 1.0);
        assert_eq!(
            g.stroke, 1.0,
            "the weight wins over the center by more than a third"
        );
        assert!((g.center.x - 10.0).abs() <= 0.5 && (g.center.y - 10.0).abs() <= 0.5);
        let g = Grid::new(pos2(10.5, 10.5), 14.0, 1.0);
        assert_eq!(
            (g.center, g.stroke),
            (pos2(10.5, 10.5), 1.0),
            "1.4 takes the center's 1"
        );
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let g = Grid::new(pos2(10.0, 10.0), 16.0, ppp);
            let px = g.stroke * ppp;
            assert!((px - px.round()).abs() < 1e-4, "{ppp}: {px} px");
            let end = (g.center.x + g.whole(4.0)) * ppp;
            assert!(
                (end - end.round()).abs() < 1e-3,
                "{ppp}: a bar ends at {end}"
            );
        }
    }
}
