// SPDX-License-Identifier: AGPL-3.0-or-later

//! The strip's own control: silvia's `#workspace-controls`, in the corner of a linear
//! canvas.
//!
//! One button at the bottom-right, an icon that gives its name when the pointer is on it —
//! silvia's `.ws-ctrl-label`, which is a width transition from nothing to the word.
//! **Auto-arrange** is the one the Workspace menu also carries, under the menu's own name
//! for it: the offer that a switch to Linear puts up answers with a button called *Arrange*,
//! and two buttons on screen with one name is a name that names neither.
//!
//! There is no button for the strip's length: a drag at the far edge makes room faster than
//! the view follows it, and the length the view is clamped to eases back to what the content
//! needs the moment nothing is asking for more.
//!
//! Only in Linear. On a plane there is nothing to arrange into a strip: a canvas pans as far
//! as you push it, and the ranks a column apart are the strip's idea of an order.
//!
//! Arranging is an edit and is a `Command` like any other.

use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2,
};

/// One button's icon box and height — silvia's 36x22.
const ICON_BOX: f32 = 36.0;
const HEIGHT: f32 = 22.0;
/// Clear space from the corner of the canvas, silvia's `bottom: 12px`.
const MARGIN: f32 = 12.0;
/// The gap between the icon and the name it opens to show, silvia's `padding-left: 8px`.
const LABEL_PAD: f32 = 8.0;
/// How long the name takes to open, in seconds — silvia's `0.2s` width transition.
const OPEN_TIME: f32 = 0.2;
/// The icon's own square inside the button, the size lucide renders these at.
const ICON: f32 = 14.0;

/// What the button is called, and what hovering it says.
const LABEL: &str = "Auto-arrange";
const HINT: &str = "A column per rank, stacked down the strip";

/// Draw it in the canvas's bottom-right corner, and say whether it was pressed: the icon
/// always, the name while the pointer is on it, opening to the left so the icon stays where
/// it was.
///
/// The canvas's own layer rather than an `Area` of its own. An `Area` sits on a layer above
/// the canvas, and a layer above the canvas is not the canvas: `contains_pointer` would go
/// false under this, and the wheel that scrolls the strip is gated on it. silvia blurs the
/// control on click for the same reason — over a control is still over the workspace.
pub fn controls(ui: &mut Ui, within: Rect, theme: &Theme) -> bool {
    let (right, y) = (within.max.x - MARGIN, within.max.y - MARGIN - HEIGHT);
    let id = Id::new(("strip-control", LABEL));
    // Last frame's hover drives this frame's width. The button under the pointer is the one
    // that was under it a frame ago in every case but the first, and the open is an
    // animation rather than a jump, so the frame of lag is not a visible one.
    let was = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let open = ui
        .ctx()
        .animate_bool_with_time(id.with("open"), was, OPEN_TIME);
    let font = FontId::proportional(theme::FONT_TINY);
    let galley = ui
        .painter()
        .layout_no_wrap(LABEL.to_owned(), font.clone(), theme.text_primary());
    let room = galley.size().x + LABEL_PAD;
    let width = ICON_BOX + open * room;
    let rect = Rect::from_min_size(Pos2::new(right - width, y), vec2(width, HEIGHT));
    let response = ui.interact(rect, id, Sense::click());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, LABEL)
    });
    let response = response.on_hover_text(HINT);

    let lit = response.hovered();
    let radius = CornerRadius::same(theme::RADIUS_SM);
    ui.painter().rect_filled(
        rect,
        radius,
        if lit {
            theme.bg_hover()
        } else {
            theme.bg_secondary()
        },
    );
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            if lit {
                theme.primary_muted()
            } else {
                theme.border_subtle()
            },
        ),
        StrokeKind::Inside,
    );
    let ink = theme.text_primary();
    // The icon keeps its box at the right-hand end, so it stays where it was while the name
    // opens to the left of it.
    let box_rect = Rect::from_min_max(
        Pos2::new(rect.max.x - ICON_BOX, rect.min.y),
        Pos2::new(rect.max.x, rect.max.y),
    );
    icon(ui, box_rect, ink);
    // The name has a box of its own, everything left of the icon's, and is clipped to it
    // rather than to the button. silvia's `.ws-ctrl-label` is `max-width: 0` and
    // `overflow: hidden`, so the word is revealed from its first letter as the box opens;
    // clipped to the button instead, it is drawn across the icon for the whole animation.
    let label_box = Rect::from_min_max(
        Pos2::new(rect.min.x + LABEL_PAD, rect.min.y),
        Pos2::new(box_rect.min.x, rect.max.y),
    );
    if label_box.is_positive() {
        ui.painter().with_clip_rect(label_box).galley(
            Pos2::new(label_box.min.x, rect.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    }
    response.clicked()
}

/// lucide's `columns-3`, drawn as its own vector geometry in the 24-unit space the source
/// SVG is in, the way the node's `?` and `✕` are. A round cap is a filled circle of the
/// stroke's own radius at each end.
fn icon(ui: &Ui, within: Rect, color: Color32) {
    let at = Rect::from_center_size(within.center(), vec2(ICON, ICON));
    let scale = ICON / 24.0;
    let stroke = Stroke::new(2.0 * scale, color);
    let p = |x: f32, y: f32| at.min + vec2(x, y) * scale;
    let mut shapes: Vec<Shape> = Vec::new();
    let line = |from: (f32, f32), to: (f32, f32), shapes: &mut Vec<Shape>| {
        let (a, b) = (p(from.0, from.1), p(to.0, to.1));
        shapes.push(Shape::line_segment([a, b], stroke));
        shapes.push(Shape::circle_filled(a, stroke.width * 0.5, color));
        shapes.push(Shape::circle_filled(b, stroke.width * 0.5, color));
    };
    // `rect x3 y3 w18 h18 rx2`, `M9 3v18`, `M15 3v18`
    shapes.push(Shape::rect_stroke(
        Rect::from_min_max(p(3.0, 3.0), p(21.0, 21.0)),
        CornerRadius::same((2.0 * scale).round() as u8),
        stroke,
        StrokeKind::Middle,
    ));
    line((9.0, 3.0), (9.0, 21.0), &mut shapes);
    line((15.0, 3.0), (15.0, 21.0), &mut shapes);
    ui.painter().extend(shapes);
}
