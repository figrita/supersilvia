// SPDX-License-Identifier: AGPL-3.0-or-later

//! The paint surface and the brush under it: `drawingcanvas`'s custom area.
//!
//! silvia's `_createUI` stacks, inside `padding: 0.5rem` with a `0.5rem` gap: the canvas,
//! 300 px wide in a 1 px `#555` border rounded 4; a centered row of six 28 px tool buttons
//! 4 px apart, the chosen one lit; the Size s-number, the Color and Background swatches, each
//! a label and its control across the row; the symmetry select; and a line of the keys. This
//! is that, in world units and the editor's own tokens, as two regions: the surface, which
//! **claims the pointer** so a drag on it paints rather than carrying the node, and the brush,
//! which does not, so the node is still carried by the air around its buttons. The symmetry
//! select is an ordinary option row above them, which is the widget silvia's row is.
//!
//! **The surface is the picture the node publishes.** It reserves a slot the app blits the
//! node's own `output` texture into, the way a source's preview does — so what is painted is
//! what the patch gets, and the surface wears the pop-out and fullscreen marks every picture
//! does. What the surface draws itself is the frame around it, the focus ring, and a shape
//! while it is being dragged, at silvia's 0.7 alpha.
//!
//! **A stroke is an edit.** A press takes the painting off the `Node`, paints the stroke into
//! a copy with `nodes::drawingcanvas`, and hands the copy back as a `SetValue`; every frame
//! the pen moves does it again, and every one of them lands in the gesture the press opened,
//! so a stroke is one undo step. The release stamps the picture with a fresh stroke number,
//! which is what the tick fires Stroke Done on. A shape is one write, when it lets go; a fill
//! is one write, when it lands.
//!
//! Where the stroke is between frames — the tool it began with and the last point — is egui's
//! own widget memory, which is the region's to keep and nobody's document.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node, Value};
use crate::nodes::drawingcanvas::{
    self, BACKGROUND, BRUSH_COLOR, BRUSH_SIZE, OUTPUT, Sheet, TOOL, Tool,
};
use crate::ui::canvas;
use crate::ui::number;
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, pos2, vec2,
};

/// silvia's `padding: 0.5rem` and `gap: 0.5rem`, at its 12 px root.
const PAD: f32 = 6.0;
const GAP: f32 = 6.0;
/// The canvas's border, silvia's `1px solid #555`.
const BORDER: f32 = 1.0;
/// silvia's `width: 300px` canvas.
const PICTURE: f32 = 300.0;
/// A tool button, silvia's 28 px square, 4 px from the next, and the 14 px icon inside it.
const BUTTON: f32 = 28.0;
const BUTTON_GAP: f32 = 4.0;
const ICON: f32 = 14.0;
/// The keys, silvia's `font-size: 0.7rem` at `line-height: 1.4`, on two lines of their own.
const HINT_SIZE: f32 = crate::ui::theme::FONT_BASE * 0.7;
const HINT_LINE: f32 = HINT_SIZE * 1.4;
const HINT: [&str; 2] = [
    "B pen · E eraser · L line",
    "R rect · C circle · F fill · [ ] size",
];
/// silvia's shape preview, drawn at 0.7 of the brush's alpha until the pointer lets go.
const PREVIEW_ALPHA: f32 = 0.7;
/// silvia's `[` and `]`, and its wheel: one pixel a step, five with Shift.
const SIZE_STEP: f32 = 1.0;
const SIZE_STEP_SHIFT: f32 = 5.0;

/// The body the surface needs: silvia's canvas in its border and padding.
pub const WIDTH: f32 = PAD * 2.0 + BORDER * 2.0 + PICTURE;

/// The surface: the picture, painted on.
pub const PAINT: RegionDef = RegionDef {
    size: paint_size,
    show: paint_show,
    claims_pointer: true,
    width: Some(WIDTH),
};

/// The brush under it: the tools, the size, the two colors and the keys.
pub const BRUSH: RegionDef = RegionDef {
    size: brush_size,
    show: brush_show,
    claims_pointer: false,
    width: Some(WIDTH),
};

/// The picture's own width on a node of this width, in world units.
fn picture_width(node: &Node) -> f32 {
    canvas::node_width(node) - 2.0 * (PAD + BORDER)
}

/// As tall as the canvas's own aspect makes the body's width: silvia's canvas is a fixed
/// width and whatever height Canvas Size gives it.
fn paint_size(node: &Node) -> f32 {
    let (w, h) = drawingcanvas::canvas_size(node);
    PAD + 2.0 * BORDER + picture_width(node) * h as f32 / w as f32
}

fn brush_size(_node: &Node) -> f32 {
    GAP + BUTTON + 3.0 * (GAP + number::HEIGHT) + GAP + 2.0 * HINT_LINE + PAD
}

// -------------------------------------------------------------------------------- the surface

/// Where a stroke is between frames: what it was begun with, where, and where it last was, in
/// pixels of the picture.
#[derive(Debug, Clone, Copy)]
struct Stroking {
    tool: Tool,
    from: (f32, f32),
    last: (f32, f32),
}

fn paint_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let zoom = r.zoom;
    let frame = Rect::from_min_size(
        r.rect.min + vec2(PAD, PAD) * zoom,
        vec2(
            r.rect.width() - 2.0 * PAD * zoom,
            r.rect.height() - PAD * zoom,
        ),
    );
    let picture = frame.shrink(BORDER * zoom);
    let radius = CornerRadius::same(crate::ui::theme::RADIUS_SM);
    let focused = r
        .grab
        .as_ref()
        .is_some_and(eframe::egui::Response::has_focus);
    let painter = r.ui.painter().clone();
    // silvia's `background: #000` under the canvas, which is what shows before the first
    // frame lands.
    painter.rect_filled(frame, radius, r.theme.screen_off());
    let slot = RegionEvent::Picture(crate::ui::Thumbnail {
        node: r.id,
        port: Some(OUTPUT),
        rect: picture,
        slot: painter.add(Shape::Noop),
        fit: crate::render::Fit::Cover,
        corner: 0.0,
    });
    painter.rect_stroke(
        frame,
        radius,
        Stroke::new((BORDER * zoom).max(1.0), r.theme.border_normal()),
        StrokeKind::Inside,
    );
    // silvia's focus ring: `0 0 0 2px` of the theme's own hue at half strength, outside the
    // border, while the keys are the canvas's.
    if focused {
        painter.rect_stroke(
            frame,
            radius,
            Stroke::new(2.0 * zoom, r.theme.primary().gamma_multiply(0.5)),
            StrokeKind::Outside,
        );
    }
    let mut out = vec![slot];
    out.extend(gestures(r, picture));
    out
}

/// A point on screen as a point of the picture, in its pixels.
fn to_picture(node: &Node, picture: Rect, at: Pos2) -> (f32, f32) {
    let (w, h) = drawingcanvas::canvas_size(node);
    (
        (at.x - picture.min.x) / picture.width().max(1.0) * w as f32,
        (at.y - picture.min.y) / picture.height().max(1.0) * h as f32,
    )
}

/// The same point back on screen.
fn to_screen(node: &Node, picture: Rect, (x, y): (f32, f32)) -> Pos2 {
    let (w, h) = drawingcanvas::canvas_size(node);
    pos2(
        picture.min.x + x / w as f32 * picture.width(),
        picture.min.y + y / h as f32 * picture.height(),
    )
}

/// The painting, with one more thing done to it, as the value that writes it.
fn write(sheet: Sheet, stroke: u64) -> RegionEvent {
    RegionEvent::Value {
        key: drawingcanvas::PAINTING,
        value: Value::Painting(sheet.into_painting(stroke)),
    }
}

/// The stroke number the node's picture carries now, which a stroke in progress keeps.
fn held_stroke(node: &Node) -> u64 {
    drawingcanvas::painting(node).map_or(0, crate::graph::Painting::stroke)
}

/// Press to begin, drag to paint, let go to finish; and while the surface has focus, the
/// tools' keys, the size's brackets and the wheel.
fn gestures(r: &mut RegionUi<'_>, picture: Rect) -> Vec<RegionEvent> {
    let Some(grab) = r.grab.clone() else {
        return Vec::new();
    };
    let node = r.node;
    // silvia's canvas blurs when a hand goes down anywhere else, and so does this — or on
    // Escape — so the editor's own keys are the editor's again.
    let (pressed_elsewhere, escape) = r.ui.input(|i| {
        (
            i.pointer.any_pressed() && !grab.contains_pointer(),
            i.key_pressed(Key::Escape),
        )
    });
    if grab.has_focus() && (pressed_elsewhere || escape) {
        grab.surrender_focus();
    }
    let mut out = keys(r, &grab);
    if grab.hovered() {
        r.ui.ctx()
            .set_cursor_icon(eframe::egui::CursorIcon::Crosshair);
    }
    let memory = grab.id.with("stroke");
    let stroking: Option<Stroking> = r.ui.data(|d| d.get_temp(memory));
    let held = grab.is_pointer_button_down_on();
    let on_screen = grab
        .interact_pointer_pos()
        .or_else(|| r.ui.input(|i| i.pointer.latest_pos()));
    let at = on_screen.map(|p| to_picture(node, picture, p));
    let brush = drawingcanvas::brush(node);
    let symmetry = drawingcanvas::symmetry(node);

    match (stroking, held, at) {
        // A press on the picture begins a stroke. One on the frame around it does not.
        (None, true, Some(at)) => {
            let pressed = r.ui.input(|i| i.pointer.press_origin());
            if !pressed.is_some_and(|p| picture.contains(p)) {
                return out;
            }
            grab.request_focus();
            let tool = drawingcanvas::tool(node);
            r.ui.data_mut(|d| {
                d.insert_temp(
                    memory,
                    Stroking {
                        tool,
                        from: at,
                        last: at,
                    },
                );
            });
            match tool {
                Tool::Pen | Tool::Eraser => {
                    let mut sheet = Sheet::of(node);
                    sheet.segment(&brush, symmetry, at, at);
                    out.push(write(sheet, held_stroke(node)));
                }
                Tool::Fill => {
                    let mut sheet = Sheet::of(node);
                    sheet.fill(brush.color, at);
                    out.push(write(sheet, drawingcanvas::next_stroke()));
                }
                Tool::Line | Tool::Rect | Tool::Circle => {}
            }
        }
        (Some(s), true, Some(at)) => match s.tool {
            Tool::Pen | Tool::Eraser if at != s.last => {
                let mut sheet = Sheet::of(node);
                sheet.segment(&brush, symmetry, s.last, at);
                out.push(write(sheet, held_stroke(node)));
                r.ui.data_mut(|d| d.insert_temp(memory, Stroking { last: at, ..s }));
            }
            Tool::Line | Tool::Rect | Tool::Circle => preview(r, picture, s.tool, s.from, at),
            Tool::Pen | Tool::Eraser | Tool::Fill => {}
        },
        // Let go: the stroke is finished, and says so by the number it stamps.
        (Some(s), false, at) => {
            r.ui.data_mut(|d| d.remove::<Stroking>(memory));
            let at = at.unwrap_or(s.last);
            match s.tool {
                Tool::Pen | Tool::Eraser => {
                    if at == s.last {
                        if let Some(painting) = drawingcanvas::painting(node) {
                            out.push(RegionEvent::Value {
                                key: drawingcanvas::PAINTING,
                                value: Value::Painting(
                                    painting.with_stroke(drawingcanvas::next_stroke()),
                                ),
                            });
                        }
                    } else {
                        let mut sheet = Sheet::of(node);
                        sheet.segment(&brush, symmetry, s.last, at);
                        out.push(write(sheet, drawingcanvas::next_stroke()));
                    }
                }
                Tool::Line | Tool::Rect | Tool::Circle => {
                    let mut sheet = Sheet::of(node);
                    sheet.shape(s.tool, &brush, symmetry, s.from, at);
                    out.push(write(sheet, drawingcanvas::next_stroke()));
                }
                Tool::Fill => {}
            }
        }
        // A press and its release inside one frame — a quick click at a low frame rate — is a
        // whole stroke that no frame saw held: a dot, a fill, or a shape of no size.
        (None, false, Some(at)) if grab.clicked() => {
            if !on_screen.is_some_and(|p| picture.contains(p)) {
                return out;
            }
            grab.request_focus();
            let mut sheet = Sheet::of(node);
            match drawingcanvas::tool(node) {
                Tool::Pen | Tool::Eraser => sheet.segment(&brush, symmetry, at, at),
                Tool::Fill => sheet.fill(brush.color, at),
                tool @ (Tool::Line | Tool::Rect | Tool::Circle) => {
                    sheet.shape(tool, &brush, symmetry, at, at);
                }
            }
            out.push(write(sheet, drawingcanvas::next_stroke()));
        }
        _ => {}
    }
    out
}

/// The shape being dragged, at every place symmetry will put it, in the brush's color at
/// silvia's 0.7 — drawn over the picture and not into it, so letting go is the only write.
fn preview(r: &RegionUi<'_>, picture: Rect, tool: Tool, from: (f32, f32), to: (f32, f32)) {
    let node = r.node;
    let brush = drawingcanvas::brush(node);
    let (w, h) = drawingcanvas::canvas_size(node);
    let scale = picture.width() / w as f32;
    let [red, green, blue, alpha] = brush.color;
    let color = Color32::from_rgba_unmultiplied(
        red,
        green,
        blue,
        (f32::from(alpha) * PREVIEW_ALPHA).round() as u8,
    );
    let stroke = Stroke::new((brush.size * scale).max(1.0), color);
    let painter = r.ui.painter().with_clip_rect(picture);
    let symmetry = drawingcanvas::symmetry(node);
    for (a, b) in symmetry
        .images(w as f32, h as f32, from)
        .into_iter()
        .zip(symmetry.images(w as f32, h as f32, to))
    {
        let (a, b) = (to_screen(node, picture, a), to_screen(node, picture, b));
        match tool {
            Tool::Line => {
                painter.line_segment([a, b], stroke);
                // A 2D canvas's `lineCap: round`.
                painter.circle_filled(a, stroke.width * 0.5, color);
                painter.circle_filled(b, stroke.width * 0.5, color);
            }
            Tool::Rect => {
                painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, stroke, StrokeKind::Middle);
            }
            Tool::Circle => {
                let r = Rect::from_two_pos(a, b);
                painter.add(Shape::ellipse_stroke(r.center(), r.size() * 0.5, stroke));
            }
            Tool::Pen | Tool::Eraser | Tool::Fill => {}
        }
    }
}

/// silvia's keys while the canvas has focus: a letter per tool, `[` and `]` for the size —
/// five at a time with Shift — and the wheel for the size too. Consumed, so a key the canvas
/// answered does not also reach the editor.
fn keys(r: &mut RegionUi<'_>, grab: &eframe::egui::Response) -> Vec<RegionEvent> {
    if !grab.has_focus() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let node = r.node;
    let chosen = r.ui.ctx().input_mut(|i| {
        Tool::ALL.into_iter().find(|tool| {
            let key = match tool.shortcut() {
                'B' => Key::B,
                'E' => Key::E,
                'L' => Key::L,
                'R' => Key::R,
                'C' => Key::C,
                _ => Key::F,
            };
            i.consume_key(Modifiers::NONE, key)
        })
    });
    if let Some(tool) = chosen {
        out.push(RegionEvent::Option {
            key: TOOL,
            value: tool.key(),
        });
    }
    let (steps, shift) = r.ui.ctx().input_mut(|i| {
        let shift = i.modifiers.shift;
        let held = i.modifiers;
        let mut steps = 0.0;
        if i.consume_key(held, Key::OpenBracket) {
            steps -= 1.0;
        }
        if i.consume_key(held, Key::CloseBracket) {
            steps += 1.0;
        }
        if grab.hovered() {
            let wheel = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta.y = 0.0;
            if wheel != 0.0 {
                steps += wheel.signum();
            }
        }
        (steps, shift)
    });
    if steps != 0.0 {
        let step = if shift { SIZE_STEP_SHIFT } else { SIZE_STEP };
        let now = drawingcanvas::brush(node).size;
        let range = crate::nodes::control_range(node.def, node, BRUSH_SIZE);
        let next = range.map_or(now + steps * step, |range| {
            (now + steps * step).clamp(range.min, range.max)
        });
        if next != now {
            out.push(RegionEvent::Controls(vec![(
                BRUSH_SIZE,
                ControlValue::Float(next),
            )]));
        }
    }
    out
}

// ---------------------------------------------------------------------------------- the brush

fn brush_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let zoom = r.zoom;
    let mut out = Vec::new();
    let mut top = r.rect.min.y + GAP * zoom;

    // The tools, centered, silvia's `justify-content: center`.
    let row = BUTTON * Tool::ALL.len() as f32 + BUTTON_GAP * (Tool::ALL.len() - 1) as f32;
    let left = r.rect.center().x - row * zoom * 0.5;
    let chosen = drawingcanvas::tool(r.node);
    for (i, tool) in Tool::ALL.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(left + (BUTTON + BUTTON_GAP) * zoom * i as f32, top),
            vec2(BUTTON, BUTTON) * zoom,
        );
        if tool_button(r, rect, tool, tool == chosen) && tool != chosen {
            out.push(RegionEvent::Option {
                key: TOOL,
                value: tool.key(),
            });
        }
    }
    top += (BUTTON + GAP) * zoom;

    // Size, Color and Background: a label and its control across the row, silvia's
    // `justify-content: space-between`.
    let font = FontId::proportional(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    for (key, label) in [
        (BRUSH_SIZE, "Size"),
        (BRUSH_COLOR, "Color"),
        (BACKGROUND, "Background"),
    ] {
        let band = Rect::from_min_size(
            pos2(r.rect.min.x + PAD * zoom, top),
            vec2(r.rect.width() - 2.0 * PAD * zoom, number::HEIGHT * zoom),
        );
        r.ui.painter().text(
            band.left_center(),
            Align2::LEFT_CENTER,
            label,
            font.clone(),
            r.theme.text_secondary(),
        );
        let control = Rect::from_min_size(
            pos2(band.max.x - number::WIDTH * zoom, band.min.y),
            vec2(number::WIDTH, number::HEIGHT) * zoom,
        );
        let event = if key == BRUSH_SIZE {
            r.number(control, key)
        } else {
            r.swatch(control, key)
        };
        out.extend(event);
        top = band.max.y + GAP * zoom;
    }

    // The keys, silvia's dim line under everything.
    let hint = FontId::proportional(crate::ui::theme::font_size(HINT_SIZE, zoom));
    for (i, line) in HINT.into_iter().enumerate() {
        r.ui.painter().text(
            pos2(r.rect.center().x, top + HINT_LINE * zoom * (i as f32 + 0.5)),
            Align2::CENTER_CENTER,
            line,
            hint.clone(),
            r.theme.text_muted(),
        );
    }
    out
}

/// One tool's button: silvia's 28 px square, `#222` at rest and `#555` when it is the one in
/// the hand, a 1 px `#555` border rounded 4, and the tool's icon. Whether it was clicked.
fn tool_button(r: &mut RegionUi<'_>, rect: Rect, tool: Tool, chosen: bool) -> bool {
    let name = r.name(&format!("tool.{}", tool.key()));
    let w =
        r.ui.interact(rect, r.ui.id().with(("paint-tool", &name)), Sense::click());
    let ground = if chosen {
        r.theme.bg_active()
    } else if w.hovered() {
        r.theme.bg_hover()
    } else {
        r.theme.bg_interactive()
    };
    let painter = r.ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(crate::ui::theme::RADIUS_SM),
        ground,
        Stroke::new((1.0 * r.zoom).max(1.0), r.theme.border_normal()),
        StrokeKind::Inside,
    );
    icon(
        painter,
        Rect::from_center_size(rect.center(), vec2(ICON, ICON) * r.zoom),
        tool,
        if chosen {
            r.theme.text_primary()
        } else {
            r.theme.text_secondary()
        },
    );
    crate::ui::accessible(&w, eframe::egui::WidgetType::Button, &name);
    let clicked = w.clicked();
    w.on_hover_text(format!("{} ({})", tool.label(), tool.shortcut()));
    clicked
}

/// A tool's icon: silvia's Lucide set — `pencil`, `eraser`, a line, `square`, `circle` and
/// `paint-bucket` — drawn as its own vector geometry in the 24-unit space the SVGs are in, the
/// way the strip's `columns-3` is, with each curve walked as a few straight pieces.
fn icon(painter: &eframe::egui::Painter, at: Rect, tool: Tool, color: Color32) {
    let scale = at.width() / 24.0;
    let stroke = Stroke::new(2.0 * scale, color);
    let p = |x: f32, y: f32| at.min + vec2(x, y) * scale;
    let mut shapes: Vec<Shape> = Vec::new();
    let mut path = |points: &[(f32, f32)], closed: bool| {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| p(x, y)).collect();
        // Round joins and caps: a dot of the stroke's radius at every corner.
        for q in &points {
            shapes.push(Shape::circle_filled(*q, stroke.width * 0.5, color));
        }
        shapes.push(if closed {
            Shape::closed_line(points, stroke)
        } else {
            Shape::line(points, stroke)
        });
    };
    // Points around `(cx, cy)` at radius `r` from one angle to another, in degrees, y down.
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<(f32, f32)> {
        (0..=8)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / 8.0).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    match tool {
        // `M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z`
        Tool::Pen => {
            let mut points = arc(19.0, 5.0, 2.83, -135.0, 45.0);
            points.extend([(7.5, 20.5), (2.0, 22.0), (3.5, 16.5)]);
            path(&points, true);
        }
        // `m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21`,
        // `M22 21H7`, `m5 11 9 9`
        Tool::Eraser => {
            path(
                &[
                    (7.0, 21.0),
                    (2.7, 16.7),
                    (2.0, 15.0),
                    (2.7, 13.3),
                    (12.3, 3.7),
                    (14.0, 3.0),
                    (15.7, 3.7),
                    (21.3, 9.3),
                    (22.0, 11.0),
                    (21.3, 12.7),
                    (13.0, 21.0),
                ],
                false,
            );
            path(&[(22.0, 21.0), (7.0, 21.0)], false);
            path(&[(5.0, 11.0), (14.0, 20.0)], false);
        }
        // `line x1=4 y1=20 x2=20 y2=4`
        Tool::Line => path(&[(4.0, 20.0), (20.0, 4.0)], false),
        // `rect x=3 y=3 width=18 height=18 rx=1`
        Tool::Rect => {
            shapes.push(Shape::rect_stroke(
                Rect::from_min_max(p(3.0, 3.0), p(21.0, 21.0)),
                CornerRadius::same(scale.round() as u8),
                stroke,
                StrokeKind::Middle,
            ));
        }
        // `circle cx=12 cy=12 r=10`
        Tool::Circle => shapes.push(Shape::circle_stroke(p(12.0, 12.0), 10.0 * scale, stroke)),
        // `m19 11-8-8-8.6 8.6a2 2 0 0 0 0 2.8l5.2 5.2c.8.8 2 .8 2.8 0L19 11Z`, `m5 2 5 5`,
        // `M2 13h15`, and the drop `M22 20a2 2 0 1 1-4 0c0-1.6 1.7-2.4 2-4 .3 1.6 2 2.4 2 4Z`
        Tool::Fill => {
            path(
                &[
                    (19.0, 11.0),
                    (11.0, 3.0),
                    (2.4, 11.6),
                    (1.8, 13.0),
                    (2.4, 14.4),
                    (7.6, 19.6),
                    (9.0, 20.2),
                    (10.4, 19.6),
                ],
                true,
            );
            path(&[(5.0, 2.0), (10.0, 7.0)], false);
            path(&[(2.0, 13.0), (17.0, 13.0)], false);
            let mut drop = arc(20.0, 20.0, 2.0, 0.0, 180.0);
            drop.extend([(18.3, 18.2), (20.0, 16.0), (21.7, 18.2)]);
            path(&drop, true);
        }
    }
    painter.extend(shapes);
}
