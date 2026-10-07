// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `s-color` swatch: a color over a checkerboard, so alpha is visible.
//!
//! The popup pairs egui's own picker with a hex field: a square and a hue bar are a better
//! instrument for *picking* a color than silvia's four HSLA sliders, but a hex string is how
//! one is written down, pasted, matched to a brand, and carried out of a design tool. The
//! field takes `#rgb`, `#rrggbb` and `#rrggbbaa`, with or without the `#`, commits on `Enter`
//! or focus loss, and leaves the color alone on anything unparseable.
//!
//! The field defers to the picker rather than competing with it. It does not take focus when
//! the popup opens — a picker is opened to *pick* — and it commits only a string somebody
//! actually changed. Both halves of that are one bug: a field that grabs focus on open loses
//! it again the moment the pointer enters the picker square, and a blur that commits whatever
//! the field was showing commits a string one frame behind the color the click just picked,
//! putting the old color straight back. That is a picked color reverting on every click.

use crate::ui::theme::{self, Theme};
use eframe::egui::ecolor::{Hsva, HsvaGamma};
use eframe::egui::emath::remap_clamp;
use eframe::egui::epaint::Mesh;
use eframe::egui::style::NumericColorSpace;
use eframe::egui::{
    Color32, CornerRadius, DragValue, FontId, Key, Rect, Response, Sense, Shape, Stroke, TextEdit,
    TextureId, Ui, lerp, vec2,
};

/// silvia's `background-size: 10px 10px`, a tile of four checks: each check is 5 points.
const CHECK: f32 = 5.0;

/// The grid behind a color, so alpha reads as alpha and not as whatever is under it.
///
/// Rows offset by one cell, which is what makes a checkerboard out of a grid of squares. The
/// swatch on the node and the alpha bar in the popup share it: two different checkerboards
/// would say the two controls were about different things.
fn checkers(painter: &eframe::egui::Painter, rect: Rect, step: f32, theme: &Theme) {
    let mut y = rect.min.y;
    let mut row = 0;
    while y < rect.max.y {
        let mut x = rect.min.x + if row % 2 == 0 { 0.0 } else { step };
        while x < rect.max.x {
            let cell = Rect::from_min_size(
                eframe::egui::pos2(x, y),
                vec2(step.min(rect.max.x - x), step.min(rect.max.y - y)),
            );
            painter.rect_filled(cell, 0.0, theme.bg_tertiary());
            x += step * 2.0;
        }
        y += step;
        row += 1;
    }
}

/// Draw the swatch. Clicking it opens the picker, which is drawn separately.
///
/// `varying` is the case a color has no answer for: a color that **varies** is arriving, one
/// per pixel, so there is no single color to show and `value` is one nobody is reading. The
/// swatch then paints its own ground and says [`crate::ui::number::VARYING`] across it. A
/// color is the whole of this control, so the word cannot go beside it as it can on a
/// number — it has to stand where the color was. Where the color's own picture is at hand,
/// [`swatch_picturing`] puts that there instead.
// Every argument is used, and the two flags are independent questions: a swatch can be inert
// without a varying color arriving, which is what an unpublished uniform color looks like.
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub fn swatch(
    ui: &mut Ui,
    rect: Rect,
    label: impl crate::ui::Name,
    value: [f32; 4],
    theme: &Theme,
    enabled: bool,
    varying: bool,
    zoom: f32,
) -> Response {
    swatch_picturing(ui, rect, label, value, theme, enabled, varying, None, zoom)
}

/// [`swatch`], with the picture of a varying color standing where the word would: `picture`
/// is the thumbnail of the port the color arrives from, painted over the swatch's whole rect
/// with its rounding ([`picture`]). Read only while `varying`; without one the swatch says
/// the word.
#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
pub fn swatch_picturing(
    ui: &mut Ui,
    rect: Rect,
    label: impl crate::ui::Name,
    value: [f32; 4],
    theme: &Theme,
    enabled: bool,
    varying: bool,
    picture: Option<TextureId>,
    zoom: f32,
) -> Response {
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let painter = ui.painter();
    let picture = picture.filter(|_| varying);

    if varying {
        painter.rect_filled(rect, radius, theme.bg_interactive());
        if let Some(texture) = picture {
            self::picture(painter, rect, radius, texture);
        }
    } else {
        // Alpha backing, so a transparent color does not read as the node body.
        painter.rect_filled(rect, radius, theme.bg_interactive());
        checkers(painter, rect, CHECK * zoom.max(0.5), theme);
        painter.rect_filled(rect, radius, to_color32(value));
    }
    let color = to_color32(value);
    if enabled {
        // silvia's `2px inset border-normal` — the same recessed bevel the s-number's own
        // value sits in, so a color and a number read as the same family of control.
        crate::ui::number::bevel_border(painter, rect, zoom, theme);
    } else {
        // silvia's `s-color[disabled] { border: 2px dashed border-normal }`: dashed rather
        // than the inset bevel, which is disabled's own signal — a connection overrides the
        // swatch, and "recessed" would still say "usable" where "dashed" says it is not.
        let stroke = Stroke::new((1.5 * zoom).max(1.0), theme.border_subtle());
        // One edge at a time, so each side starts its own dash rather than carrying the
        // phase around a corner; `dashed_line_many` appends into one vector where
        // `dashed_line` would return four.
        let mut dashes = Vec::new();
        for [a, b] in [
            [rect.left_top(), rect.right_top()],
            [rect.right_top(), rect.right_bottom()],
            [rect.right_bottom(), rect.left_bottom()],
            [rect.left_bottom(), rect.left_top()],
        ] {
            eframe::egui::Shape::dashed_line_many(
                &[a, b],
                stroke,
                4.0 * zoom,
                3.0 * zoom,
                &mut dashes,
            );
        }
        painter.add(eframe::egui::Shape::Vec(dashes));
    }

    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    // The word, over the ground and inside the dashed border the `enabled` arm just drew.
    // Fitted to the swatch rather than set at a fixed size: the same control is a full-width
    // slab on a node row and a small square in a readout slot.
    if varying && picture.is_none() {
        let size = (rect.height() * 0.5).min(rect.width() / 5.0);
        painter.text(
            rect.center(),
            eframe::egui::Align2::CENTER_CENTER,
            crate::ui::number::VARYING,
            eframe::egui::FontId::proportional(size),
            theme.text_secondary(),
        );
    }

    let response = ui.interact(rect, ui.id().with(("col", &label)), sense);
    response.widget_info(|| {
        // Unmultiplied, so the readout is the value the node holds. `Color32`'s channels are
        // premultiplied, so printing them reports a color darkened by its own alpha.
        let [hr, hg, hb, ha] = color.to_srgba_unmultiplied();
        eframe::egui::WidgetInfo::labeled(
            eframe::egui::WidgetType::ColorButton,
            true,
            if varying {
                format!("{label} {}", crate::ui::number::VARYING)
            } else {
                format!("{label} #{hr:02x}{hg:02x}{hb:02x}{ha:02x}")
            },
        )
    });
    response
}

/// A port thumbnail filling `rect` with `radius`, as a swatch fills it with a color: the
/// middle of the picture at the rect's own aspect, cropped across the long side and never
/// stretched. Shared by the swatch and the s-number's trough, the two places a varying value
/// arriving at a control shows what it is.
pub(crate) fn picture(
    painter: &eframe::egui::Painter,
    rect: Rect,
    radius: CornerRadius,
    texture: TextureId,
) {
    use crate::compile::{THUMB_H, THUMB_W};
    let picture = THUMB_W as f32 / THUMB_H as f32;
    let aspect = rect.width() / rect.height().max(f32::EPSILON);
    let uv = if aspect > picture {
        let keep = picture / aspect;
        Rect::from_min_max(
            eframe::egui::pos2(0.0, 0.5 - keep / 2.0),
            eframe::egui::pos2(1.0, 0.5 + keep / 2.0),
        )
    } else {
        let keep = aspect / picture;
        Rect::from_min_max(
            eframe::egui::pos2(0.5 - keep / 2.0, 0.0),
            eframe::egui::pos2(0.5 + keep / 2.0, 1.0),
        )
    };
    painter.add(
        eframe::egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(texture, uv),
    );
}

/// The value as whole channels, rounded and clamped: the one place a `[f32; 4]` becomes
/// bytes, so the swatch, the picker and [`to_hex`] cannot disagree about a color.
fn bytes(value: [f32; 4]) -> [u8; 4] {
    value.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// The color egui paints and picks with.
fn to_color32(value: [f32; 4]) -> Color32 {
    let [r, g, b, a] = bytes(value);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Back the other way. `Color32` stores premultiplied channels, so reading `r()` as the
/// control's straight red multiplies the color by its own alpha; `to_srgba_unmultiplied` is
/// what undoes that.
fn to_value(color: Color32) -> [f32; 4] {
    color.to_srgba_unmultiplied().map(|c| f32::from(c) / 255.0)
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`, the `#` optional. Anything else is not a color —
/// leaving the field alone is the caller's job, not this function's.
fn parse_hex(text: &str) -> Option<[f32; 4]> {
    let s = text.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let byte = |h: &str| u8::from_str_radix(h, 16).ok();
    // The shorthand doubles each digit: `f` is `ff`, the same rule `#rgb` follows in CSS.
    let nibble = |c: u8| (c as char).to_digit(16).map(|d| (d * 17) as u8);
    let rgba: [u8; 4] = match *s.as_bytes() {
        [r, g, b] => [nibble(r)?, nibble(g)?, nibble(b)?, 255],
        [..] if s.len() == 6 => [byte(&s[0..2])?, byte(&s[2..4])?, byte(&s[4..6])?, 255],
        [..] if s.len() == 8 => [
            byte(&s[0..2])?,
            byte(&s[2..4])?,
            byte(&s[4..6])?,
            byte(&s[6..8])?,
        ],
        _ => return None,
    };
    Some(rgba.map(|c| f32::from(c) / 255.0))
}

/// `#rrggbbaa`, canonical and lower-case: what the field shows once a value is settled,
/// whether that is the swatch's own color or one just typed and parsed.
pub fn to_hex(value: [f32; 4]) -> String {
    let [r, g, b, a] = bytes(value);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

// ---- the picker's own instruments ------------------------------------------------------

/// The picker's geometry. The square gets the height, and the two bars and the numbers stand
/// *beside* it: egui's own picker stacks all four full-width rows against a square pinned at
/// 100 points, which is where the empty column down the right of the popup came from.
const BAR: f32 = 20.0;
const GAP: f32 = 6.0;
const NUMBERS: f32 = 72.0;
/// The `INT`/`FLOAT` button. Narrower than the channel fields it labels.
const SPACE_TOGGLE: f32 = 46.0;
/// The button that puts the hex string on the clipboard, beside the field it copies. Worded
/// rather than a clipboard glyph: the app's font has no emoji, and a missing glyph renders as
/// a diamond that says nothing at all.
const COPY: f32 = 46.0;
/// How tall every column in the picker's middle row is.
///
/// The numbers column is five rows of whatever a row is in this style, so that is the height
/// the square and the bars take too. Derived rather than fixed: a square that outlives the
/// column beside it leaves a strip of bare popup under the last channel, and one that falls
/// short of it leaves the same strip under the square instead.
fn side(ui: &Ui) -> f32 {
    5.0f32.mul_add(ui.spacing().interact_size.y, 4.0 * GAP)
}

/// What the four columns and the gaps between them come to.
fn content(side: f32) -> f32 {
    side + GAP + BAR + GAP + BAR + GAP + NUMBERS
}

/// The color being edited, in the space the square and the bars are axes of.
///
/// Held across frames because a color does not carry everything the picker needs: black is
/// black at every hue, and a gray at every saturation. Recomputing HSV from the stored color
/// each frame would throw the hue away the moment the value reached zero, so dragging to the
/// bottom of the square and back would come back red. `of` is the color the angles were last
/// agreed with — anything else (the hex field, an undo, a controller) means they are stale.
#[derive(Clone, Copy)]
struct Editing {
    hsva: HsvaGamma,
    of: Color32,
}

/// Paint a gradient over `rect` by sampling `at` on an `N`-by-`N` grid of vertices.
///
/// One mesh rather than a row of thin rectangles: the gradient is interpolated across each
/// quad by the GPU, so six samples an axis is enough for a smooth ramp and the shape count
/// stays flat.
fn ramp(painter: &eframe::egui::Painter, rect: Rect, at: impl Fn(f32, f32) -> Color32) {
    const N: u32 = 6;
    let mut mesh = Mesh::default();
    for yi in 0..=N {
        for xi in 0..=N {
            let (xt, yt) = (xi as f32 / N as f32, yi as f32 / N as f32);
            mesh.colored_vertex(
                eframe::egui::pos2(
                    lerp(rect.left()..=rect.right(), xt),
                    lerp(rect.bottom()..=rect.top(), yt),
                ),
                at(xt, yt),
            );
            if xi < N && yi < N {
                let tl = yi * (N + 1) + xi;
                mesh.add_triangle(tl, tl + 1, tl + N + 1);
                mesh.add_triangle(tl + 1, tl + N + 1, tl + N + 2);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The ring that marks where in a gradient the color sits.
///
/// Stroked in the contrast of the color under it rather than a fixed light or dark, so it is
/// still visible at both ends of every axis it can be dragged along.
fn marker(painter: &eframe::egui::Painter, at: eframe::egui::Pos2, radius: f32, on: Color32) {
    let contrast = contrast(on);
    painter.circle_stroke(at, radius, Stroke::new(2.0, contrast));
    painter.circle_stroke(
        at,
        radius + 1.5,
        Stroke::new(1.0, contrast.gamma_multiply(0.4)),
    );
}

/// An instrument's rect grown into half the gap around it, which is what it answers clicks in.
///
/// The popup itself senses clicks — it has to, or they fall through to the node underneath —
/// and being registered after its own contents it wins the hit test at their very edges. That
/// left a dead rim on every instrument: a click at the top of the alpha bar, which is the
/// only way to *click* full opacity rather than drag to it, landed on the popup instead.
/// Half the gap each is exactly enough to close that and no more, so the columns still cannot
/// reach into one another.
fn reach(rect: Rect) -> Rect {
    rect.expand(GAP * 0.5)
}

/// Black or white, whichever can be seen against `on`.
///
/// A marker is drawn over the very gradient it points into, so no fixed color works: white
/// disappears at the pale end of the square and black at the dark end.
fn contrast(on: Color32) -> Color32 {
    if on.intensity() < 0.5 {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// The band that marks where along an upright bar the color sits.
///
/// A bar is one axis, so its marker spans the bar rather than floating in it — and unlike a
/// ring it cannot hang off the end when the value reaches 0 or 1, which is where a bar spends
/// much of its time: alpha at the top and hue at red are both extremes.
fn grip(painter: &eframe::egui::Painter, rect: Rect, value: f32, on: Color32) {
    let half = 2.5;
    let y = lerp((rect.bottom() - half)..=(rect.top() + half), value);
    let band = Rect::from_min_max(
        eframe::egui::pos2(rect.left() - 1.0, y - half),
        eframe::egui::pos2(rect.right() + 1.0, y + half),
    );
    painter.rect_stroke(
        band,
        CornerRadius::same(2),
        Stroke::new(1.5, contrast(on)),
        eframe::egui::StrokeKind::Middle,
    );
}

/// Saturation across, value up. The instrument the popup is opened for.
fn square(ui: &mut Ui, hsva: &mut HsvaGamma, theme: &Theme) -> Response {
    let side = side(ui);
    let (id, rect) = ui.allocate_space(vec2(side, side));
    let response = ui.interact(reach(rect), id, Sense::click_and_drag());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::Crosshair);
    if let Some(p) = response.interact_pointer_pos() {
        hsva.s = remap_clamp(p.x, rect.left()..=rect.right(), 0.0..=1.0);
        hsva.v = remap_clamp(p.y, rect.bottom()..=rect.top(), 0.0..=1.0);
    }
    let opaque = HsvaGamma { a: 1.0, ..*hsva };
    ramp(ui.painter(), rect, |s, v| {
        HsvaGamma { s, v, ..opaque }.into()
    });
    crate::ui::number::bevel_border(ui.painter(), rect, 1.0, theme);
    marker(
        ui.painter(),
        eframe::egui::pos2(
            lerp(rect.left()..=rect.right(), hsva.s),
            lerp(rect.bottom()..=rect.top(), hsva.v),
        ),
        5.0,
        HsvaGamma {
            s: hsva.s,
            v: hsva.v,
            ..opaque
        }
        .into(),
    );
    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::Other,
        "saturation and value",
    );
    response
}

/// One of the two upright bars beside the square: bottom is 0, top is 1.
///
/// Upright rather than laid across the popup because that is what buys the column — and a bar
/// as tall as the square reads as another axis of the same instrument rather than a row of
/// settings underneath it.
fn bar(
    ui: &mut Ui,
    value: &mut f32,
    label: &'static str,
    over_checkers: bool,
    theme: &Theme,
    at: impl Fn(f32) -> Color32,
) -> Response {
    let (id, rect) = ui.allocate_space(vec2(BAR, side(ui)));
    let response = ui.interact(reach(rect), id, Sense::click_and_drag());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::ResizeVertical);
    if let Some(p) = response.interact_pointer_pos() {
        *value = remap_clamp(p.y, rect.bottom()..=rect.top(), 0.0..=1.0);
    }
    if over_checkers {
        // Alpha has to be shown against something, or the transparent end of the bar is the
        // popup's own background and reads as "no color" rather than "no alpha".
        ui.painter().rect_filled(rect, 0.0, theme.bg_interactive());
        checkers(ui.painter(), rect, CHECK, theme);
    }
    ramp(ui.painter(), rect, |_, t| at(t));
    crate::ui::number::bevel_border(ui.painter(), rect, 1.0, theme);
    grip(ui.painter(), rect, *value, at(*value));
    crate::ui::accessible(&response, eframe::egui::WidgetType::Other, label);
    response
}

/// The channel numbers, stacked in the column the bars leave beside the square.
///
/// egui lays these across a row, which is what set the popup's width and left the square with
/// nothing beside it. They are the same widgets and the same color space toggle — a color
/// matched to a spec is typed in numbers as often as it is dragged to — only turned upright.
fn numbers(ui: &mut Ui, hsva: &mut HsvaGamma, theme: &Theme) {
    ui.spacing_mut().interact_size.x = NUMBERS;
    // Which space the four numbers below are in, said in the words the numbers are in rather
    // than egui's `U8`: a toggle is only useful to someone who can read what it will do to
    // them without leaning in.
    let space = ui.global_style().visuals.numeric_color_space;
    let (label, next) = match space {
        NumericColorSpace::GammaByte => ("INT", NumericColorSpace::Linear),
        NumericColorSpace::Linear => ("FLOAT", NumericColorSpace::GammaByte),
    };
    // Narrower than the channels below it: it is a label saying which space they are in, not
    // a fifth field to be read across.
    if worded(ui, label, SPACE_TOGGLE, theme)
        .on_hover_text("Whole channels 0-255, or linear 0-1")
        .clicked()
    {
        ui.ctx()
            .all_styles_mut(|s| s.visuals.numeric_color_space = next);
    }
    // Written back only on a change: the round trip every frame would round the angles the
    // square is being dragged along.
    match ui.global_style().visuals.numeric_color_space {
        NumericColorSpace::GammaByte => {
            let mut bytes = Hsva::from(*hsva).to_srgba_unmultiplied();
            if channels(ui, &mut bytes, |d| d.speed(0.5)) {
                *hsva = HsvaGamma::from(Hsva::from_srgba_unmultiplied(bytes));
            }
        }
        NumericColorSpace::Linear => {
            let mut linear = Hsva::from(*hsva).to_rgba_unmultiplied();
            if channels(ui, &mut linear, |d| {
                d.speed(0.003).range(0.0..=1.0).max_decimals(3)
            }) {
                let [r, g, b, a] = linear;
                *hsva = HsvaGamma::from(Hsva::from_rgba_unmultiplied(r, g, b, a));
            }
        }
    }
}

/// The four channel fields, `R` `G` `B` `A`, each a `DragValue` finished by `field`. True when
/// one of them changed.
fn channels<T: eframe::egui::emath::Numeric>(
    ui: &mut Ui,
    values: &mut [T; 4],
    field: impl for<'a> Fn(DragValue<'a>) -> DragValue<'a>,
) -> bool {
    let mut edited = false;
    for (channel, prefix) in values.iter_mut().zip(["R ", "G ", "B ", "A "]) {
        edited |= ui
            .add(field(DragValue::new(channel)).prefix(prefix))
            .changed();
    }
    edited
}

/// One of the picker's two worded buttons, `INT`/`FLOAT` and `COPY`: tiny text on
/// `bg_interactive`, at least `width` across.
fn worded(ui: &mut Ui, label: &str, width: f32, theme: &Theme) -> Response {
    ui.add(
        eframe::egui::Button::new(
            eframe::egui::RichText::new(label)
                .font(FontId::proportional(theme::FONT_TINY))
                .color(theme.text_secondary()),
        )
        .min_size(vec2(width, 0.0))
        .fill(theme.bg_interactive()),
    )
}

/// Has the field's text been changed away from the color it is showing?
///
/// Colors, not strings: `#f00` typed over a stored `#ff0000ff` is the same red, and
/// committing it would open an undo step that restores nothing. Unparseable text counts as
/// changed — it is certainly not what the field was showing — and the caller declines to
/// commit it for its own reason.
fn edited(text: &str, showing: [f32; 4]) -> bool {
    parse_hex(text) != parse_hex(&to_hex(showing))
}

/// The picker itself, drawn after every node so that a node painted later cannot cover it.
///
/// `color_edit_button_srgba` renders a swatch button that opens a further popup of its own,
/// so nesting it gives a swatch that appears to do nothing.
///
/// Returns the new color when it changed, and whether the user clicked away to dismiss.
pub fn picker(
    ui: &mut Ui,
    at: eframe::egui::Pos2,
    owner: impl std::hash::Hash + std::fmt::Debug,
    value: [f32; 4],
    theme: &Theme,
) -> (Option<[f32; 4]>, bool) {
    let mut changed = None;
    // Named by the control it belongs to, not a single shared id: the hex buffer below is
    // state that outlives one frame, and two different swatches opened one after another
    // must not inherit each other's typed text. `owner` is whatever names that control — a
    // node and its key on the canvas, an anchor in the preferences window — because the
    // picker is the same instrument wherever it is opened from.
    let popup_id = ui.id().with(("colpicker", owner));
    let hex_id = popup_id.with("hex");
    let state_id = popup_id.with("hsva");
    let shown = crate::ui::popup::Popup::new(popup_id, at).show(ui.ctx(), theme, |ui| {
        let content = content(side(ui));
        ui.set_width(content);
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);

        // The angles the square and the bars are axes of, carried across frames
        // so a hue survives a trip through black. Anything that moved the color
        // from outside the popup invalidates them.
        let color = to_color32(value);
        let mut hsva = ui
            .data(|d| d.get_temp::<Editing>(state_id))
            .filter(|e| e.of == color)
            .map_or_else(|| HsvaGamma::from(color), |e| e.hsva);

        ui.horizontal_top(|ui| {
            square(ui, &mut hsva, theme);
            let opaque = HsvaGamma { a: 1.0, ..hsva };
            bar(ui, &mut hsva.h, "hue", false, theme, |h| {
                HsvaGamma {
                    h,
                    s: 1.0,
                    v: 1.0,
                    a: 1.0,
                }
                .into()
            });
            bar(ui, &mut hsva.a, "alpha", true, theme, |a| {
                HsvaGamma { a, ..opaque }.into()
            });
            ui.vertical(|ui| numbers(ui, &mut hsva, theme));
        });

        // One place the angles become a color, so the square, the bars and the
        // numbers cannot disagree about what was picked.
        let picked_color = Color32::from(Hsva::from(hsva));
        let picked = picked_color != color;
        let straight = to_value(picked_color);
        if picked {
            changed = Some(straight);
        }
        ui.data_mut(|d| {
            d.insert_temp(
                state_id,
                Editing {
                    hsva,
                    of: picked_color,
                },
            );
        });
        let canonical = to_hex(straight);
        // The buffer holds a string mid-edit and nothing else. Its absence is
        // "nobody is typing", and the field then simply mirrors the color, so
        // dragging the square and reading the hex never disagree. `dismissed`,
        // below, removes it, so the next swatch opens on its own color rather
        // than whatever was typed here last.
        let mut text = ui
            .data_mut(|d| d.get_temp::<String>(hex_id))
            .unwrap_or_else(|| canonical.clone());

        // The hex field and the clipboard beside it: copying a color out means
        // copying the string, so the button belongs against the string rather
        // than up among the channel numbers, which are not what gets pasted.
        let response = ui
            .horizontal(|ui| {
                let field = TextEdit::singleline(&mut text)
                    .id(hex_id)
                    .font(FontId::proportional(theme::FONT_TINY))
                    .text_color(theme.text_primary())
                    .background_color(theme.bg_interactive())
                    .desired_width(content - COPY - GAP)
                    .show(ui)
                    .response;
                if worded(ui, "COPY", COPY, theme)
                    .on_hover_text("Copy the hex value")
                    .clicked()
                {
                    // The string, not egui's `255, 128, 64`: this popup says a
                    // color is written as hex, and the clipboard should agree.
                    ui.ctx().copy_text(canonical.clone());
                }
                field
            })
            .inner;

        let canceled = response.has_focus() && ui.ctx().input(|i| i.key_pressed(Key::Escape));
        if canceled {
            response.surrender_focus();
            text = canonical;
        } else if response.lost_focus() {
            // `Enter` surrenders a singleline's focus, so losing it is the whole
            // commit condition.
            match parse_hex(&text) {
                // Only a string somebody changed, and never over a color picked
                // this same frame: the picker was seeded from the stored color,
                // so the text is a frame behind whatever the pointer just did.
                Some(parsed) if edited(&text, value) && changed.is_none() => {
                    changed = Some(parsed);
                    text = to_hex(parsed);
                }
                // Unparseable, unchanged, or beaten by a pick. The color is left
                // alone — the same contract the s-number's typed entry has — and
                // the field still has to show *something* valid, so it shows the
                // color that is actually stored.
                _ => text = canonical,
            }
        } else if !response.has_focus() {
            text = canonical;
        }
        ui.data_mut(|d| d.insert_temp(hex_id, text));
    });
    let dismissed = shown.clicked_away;
    if dismissed {
        // So the next node's swatch opens on its own color rather than whatever was typed
        // here last, and on that color's own hue rather than the one this popup was left at.
        ui.data_mut(|d| {
            d.remove::<String>(hex_id);
            d.remove::<Editing>(state_id);
        });
    }
    (changed, dismissed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picker's storage is premultiplied. Reading the channels straight back is what
    /// darkened a semi-transparent color by its own alpha on every interaction, and
    /// compounded: 0.5 red at half alpha became 0.25, then 0.125.
    #[test]
    fn a_color_survives_a_round_trip_through_the_pickers_storage() {
        let value = [1.0f32, 0.0, 0.0, 0.5];
        let stored = to_color32(value);
        let back = to_value(stored);
        for (i, (got, want)) in back.iter().zip(value.iter()).enumerate() {
            assert!(
                (got - want).abs() < 0.01,
                "channel {i}: {got} should still be {want}"
            );
        }
        assert!(
            f32::from(stored.r()) / 255.0 < 0.6,
            "the premultiplied red is the half-strength value the old read returned"
        );
    }

    #[test]
    fn the_hex_field_takes_all_three_lengths_with_or_without_the_hash() {
        assert_eq!(parse_hex("#ff0000ff"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_hex("ff0000ff"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            parse_hex("#ff0000"),
            Some([1.0, 0.0, 0.0, 1.0]),
            "no alpha is opaque"
        );
        assert_eq!(
            parse_hex("#f00"),
            Some([1.0, 0.0, 0.0, 1.0]),
            "the shorthand doubles each digit"
        );
        assert_eq!(parse_hex("  #f00  "), Some([1.0, 0.0, 0.0, 1.0]), "trimmed");
    }

    /// The bug the `edited` guard exists for: a blur used to commit whatever string the field
    /// was showing. Clicking into the picker square *is* a blur, and on that frame the string
    /// is a frame behind the color the click just picked — so the pick was overwritten by
    /// the color it had replaced, every time.
    #[test]
    fn text_nobody_changed_is_not_an_edit() {
        let red = [1.0f32, 0.0, 0.0, 1.0];
        assert!(!edited("#ff0000ff", red), "the field's own canonical form");
        assert!(
            !edited("ff0000ff", red),
            "the same color without the hash is still not a change"
        );
        assert!(!edited("#f00", red), "nor in the shorthand");
        assert!(edited("#00ff00ff", red), "a different color is an edit");
        assert!(
            edited("nope", red),
            "unparseable is certainly not what the field was showing — the caller declines \
             it for its own reason"
        );
    }

    #[test]
    fn an_unparseable_hex_string_is_not_a_color() {
        for bad in ["", "#", "nope", "#gg0000", "#ff00", "#ff00000"] {
            assert_eq!(parse_hex(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_color_round_trips_through_its_canonical_hex() {
        let value = [1.0f32, 0.5, 0.0, 0.75];
        assert_eq!(
            parse_hex(&to_hex(value)),
            Some(value_at_hex_precision(value))
        );
    }

    /// `to_hex` rounds to whole bytes, so the round trip has to compare at that precision
    /// rather than against the original floats.
    fn value_at_hex_precision(value: [f32; 4]) -> [f32; 4] {
        bytes(value).map(|c| f32::from(c) / 255.0)
    }
}
