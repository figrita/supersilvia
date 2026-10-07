// SPDX-License-Identifier: AGPL-3.0-or-later

//! A free-text option row: an ordinary field, open the whole time it is on screen — there is
//! nothing to pick, so there is nothing to click through to reach it. Ported from silvia's own
//! `<input type=text>` on Lyapunov's `sequence`, the one option in the library typed rather
//! than chosen.
//!
//! Live-validated, not live-enforced: the border reads invalid the moment what is typed would
//! silently fall back to the option's default, and [`edit`] hands the caller every keystroke
//! so it can decide what to do with it — Lyapunov's row sends every one straight to
//! `SetOption` regardless, because `parse_sequence` never refuses to emit *something* and the
//! color is what says which something that will be.
//!
//! [`number`] is the same field holding a number: a plain box with none of the s-number's
//! hands — no scrub, no steppers, no fill — for a number that is typed rather than performed,
//! such as the Text node's size and the ends in the range editor. It admits only what a number
//! is written with, and commits once, on Enter or when the field is left, clamped into its
//! bounds; Escape puts back what was there. One commit rather than one a keystroke, because
//! the digits on the way to 128 are sizes nobody asked for.
//!
//! A box of lines — the note's, the Text node's — breaks its text into the same lines at every
//! zoom, through [`wrapped`]. The font is a whole pixel size while the box scales smoothly, so
//! a box that wrapped at its own width fit a different number of words on a line at each zoom
//! and the text jumped between line breaks mid-zoom. A wrap width that follows the quantized
//! size ([`theme::wrap_width`]) fixes the ratio, but egui places each glyph on a whole pixel,
//! so a line ending within a pixel of the edge still broke one way at one size and the other
//! at the next. So the lines are broken once, at zoom 1's size, and each is laid out alone at
//! the zoom's. The `TextEdit` lays its text out the same way, so typing and reading break
//! alike and the caret sits on the rows drawn.

use crate::ui::theme::{self, Hsl, Theme};

/// What a field reports back: what was typed, if anything.
pub struct Edited {
    /// `Some` on the frame a keystroke changed it.
    pub text: Option<String>,
}

/// The gap between the text and the field's own border, left and right.
const PAD_X: f32 = 10.0;
/// The same above and below, on a field of more than one line. A one-line field centers its
/// single row instead, so it has no use for this.
const PAD_Y: f32 = 8.0;
use eframe::egui::{
    Color32, Context, FontId, Frame, Galley, Margin, Rect, TextEdit, Ui, text::LayoutJob,
};
use std::sync::Arc;

/// A `Margin` counts in whole points and holds them in an `i8`, so a zoomed-out padding
/// rounds to the nearest point and a zoomed-in one stops growing at 127.
fn points(v: f32) -> i8 {
    i8::try_from(v.round().clamp(0.0, 127.0) as i32).unwrap_or(i8::MAX)
}

/// What a box of lines keeps clear inside each side's [`PAD_X`], in world units: room for
/// [`points`] rounding the padding by up to half a point, and for a glyph [`wrapped`] lays out
/// landing up to an eighth of a pixel right of its place, at every zoom whose font size scales.
const PAD_SLACK: f32 = 1.0;

/// The room inside a box of lines' padding, on screen, for a box `width` points wide.
fn box_room(width: f32, zoom: f32) -> f32 {
    (width - 2.0 * f32::from(points(PAD_X * zoom))).max(0.0)
}

/// The text of a box of lines `width` points wide: [`wrapped`] inside its padding, as
/// `TextEdit`'s own layouter lays out a multi-line field, with trailing spaces kept and each
/// row a row of the font plus `spacing` tall.
fn box_galley(
    ctx: &Context,
    text: &str,
    width: f32,
    zoom: f32,
    color: Color32,
    spacing: f32,
) -> Arc<Galley> {
    let world = width / zoom - 2.0 * (PAD_X + PAD_SLACK);
    let room = box_room(width, zoom);
    wrapped(
        ctx,
        text,
        theme::FONT_TINY,
        world,
        room,
        zoom,
        |text, font, wrap| {
            let line_height = ctx.fonts_mut(|f| f.row_height(&font)) + spacing;
            let mut job = LayoutJob::simple(text.to_owned(), font, color, wrap);
            job.keep_trailing_whitespace = true;
            for section in &mut job.sections {
                section.format.line_height = Some(line_height);
            }
            job
        },
    )
}

/// Canvas text of size `base` at `zoom`, wrapped in a room `world` units wide, never wider than
/// `room` on screen, and broken into the same lines at every zoom. `job` lays a text out in a
/// font at a wrap width.
///
/// The lines are the ones the text breaks into at zoom 1's size and [`theme::wrap_width`] less
/// a pixel, and each is laid out alone at this zoom's size. Where the floor holds the size and
/// that would be wider than `room`, the text wraps at `room` instead.
pub fn wrapped(
    ctx: &Context,
    text: &str,
    base: f32,
    world: f32,
    room: f32,
    zoom: f32,
    job: impl Fn(&str, FontId, f32) -> LayoutJob,
) -> Arc<Galley> {
    let layout = |job: LayoutJob| ctx.fonts_mut(|f| f.layout_job(job));
    let font = FontId::proportional(theme::font_size(base, zoom));
    let wrap = theme::wrap_width(world, base, zoom);
    if wrap > room {
        return layout(job(text, font, room));
    }
    let at_one = FontId::proportional(theme::font_size(base, 1.0));
    let lines = layout(job(
        text,
        at_one.clone(),
        (theme::wrap_width(world, base, 1.0) - 1.0).max(0.0),
    ));
    if at_one == font {
        return lines;
    }
    let mut chars = text.chars();
    let rows: Vec<Arc<Galley>> = lines
        .rows
        .iter()
        .map(|row| {
            let line: String = chars.by_ref().take(row.row.glyphs.len()).collect();
            if row.ends_with_newline {
                chars.next();
            }
            layout(job(&line, font.clone(), f32::INFINITY))
        })
        .collect();
    // `concat` joins galleys as paragraphs, so it ends each one's last row with a newline; a
    // line here ends with one only where the text has one.
    let mut galley = Galley::concat(
        Arc::new(job(text, font, wrap)),
        &rows,
        ctx.pixels_per_point(),
    );
    for (row, line) in galley.rows.iter_mut().zip(&lines.rows) {
        row.ends_with_newline = line.ends_with_newline;
    }
    Arc::new(galley)
}

/// silvia's own invalid-field color, `hsl(0, 60%, 40%)`, ported literally: the design system
/// carries no token for "this will not compile", and silvia's own choice is as good as any.
const INVALID: Hsl = Hsl::new(0.0, 0.6, 0.4);

/// What a field holds and how it is asked to hold it: everything about one field that is not
/// where it is or what it is called.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub value: &'a str,
    /// What an empty field says.
    pub placeholder: &'a str,
    /// What is typed would be used as it stands; the border says so when it would not.
    pub valid: bool,
    /// The floor under an empty box, in lines.
    pub rows: u8,
}

/// Draw the field and return what it holds on the frame a keystroke changes it.
///
/// `name` is the row's own accessible name, matching every other control's `{slug}{id}.{key}`
/// so the field is findable the same way a number or a select is.
pub fn edit(
    ui: &mut Ui,
    rect: Rect,
    name: impl crate::ui::Name,
    field: Field<'_>,
    theme: &Theme,
    zoom: f32,
) -> Edited {
    let Field {
        value,
        placeholder,
        valid,
        rows,
    } = field;
    let id = ui.id().with(("text", &name));
    // While focused the field owns its own draft, so a keystroke this frame is never
    // overwritten by the value last frame's command bus wrote back before this one lands.
    // Unfocused, it always shows the node's own value, which is what lets an external change
    // — undo, another view of the same node — reach it.
    let focused = ui.memory(|m| m.has_focus(id));
    let mut draft = if focused {
        ui.data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    };

    let border = if valid {
        theme.border_normal()
    } else {
        INVALID.color()
    };
    crate::ui::field(ui.painter(), rect, theme.bg_interactive(), None, border);

    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, zoom));
    let row = ui.ctx().fonts_mut(|f| f.row_height(&font));
    // A single-line field centers its one row in whatever height the row was given. A
    // multi-line one starts at the top and fills downwards, because its height is the rows
    // it asked for and centring would only push the first line off the middle.
    let lines = rows.max(1);
    let pad_y = if lines > 1 {
        PAD_Y * zoom
    } else {
        (rect.height() - row) * 0.5
    };
    // The padding goes on the frame, not in `TextEdit::margin`. `margin` is only read when a
    // field draws egui's own frame; hand it one of your own — which this does, because the
    // background and border above are painted by hand — and the margin is dropped on the
    // floor. It silently did nothing here until a note made the gap worth looking at.
    let pad = Margin::symmetric(points(PAD_X * zoom), points(pad_y.max(0.0)));
    // A box of lines lays its text out as `box_galley` does rather than at the width
    // `TextEdit` offers, and the caret and a selection are placed on that same galley.
    let color = theme.text_primary();
    let spacing = ui.spacing().extra_text_line_spacing;
    let width = rect.width();
    let mut layouter = |ui: &Ui, text: &dyn eframe::egui::TextBuffer, _offered: f32| {
        box_galley(ui.ctx(), text.as_str(), width, zoom, color, spacing)
    };
    let out = ui
        .scope_builder(eframe::egui::UiBuilder::new().max_rect(rect), |ui| {
            // Nothing draws outside the box. The node is sized from what this field
            // reported *last* frame, so the frame a paste lands on has text taller than
            // the node holding it — and without this, that frame puts the overflow on
            // the canvas below.
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            // One line is a single-line field: Enter commits it rather than opening a second
            // line, which is what a one-line row means.
            let field = if lines == 1 {
                TextEdit::singleline(&mut draft)
            } else {
                TextEdit::multiline(&mut draft)
            };
            let field = field
                .id(id)
                .frame(Frame::NONE.inner_margin(pad))
                .font(font)
                .hint_text(placeholder)
                .text_color(color)
                // The frame's own margin is taken out of this, so the text wraps inside the
                // padding rather than up against the border.
                .desired_width(rect.width())
                .desired_rows(lines as usize)
                .clip_text(lines == 1);
            if lines == 1 {
                return field.show(ui);
            }
            // A box of lines keeps the height the layout gave it, and what is typed past it
            // scrolls: the box is never sized from its text. See `canvas::value_height`.
            let field = field.layouter(&mut layouter);
            eframe::egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(rect.height())
                .auto_shrink(false)
                .show(ui, |ui| field.min_size(rect.size()).show(ui))
                .inner
        })
        .inner;
    let response = out.response;

    // The row names itself, the way every other control does. It had not, since the typed
    // option landed: a `TextEdit` inside a `scope_builder` carries whatever egui gives it,
    // which is no name at all, so Lyapunov's `sequence` and every field after it were
    // reachable by a pointer and invisible to the tree. A hand-painted thing a hand can use
    // and the accessibility tree cannot see is a bug by this project's own rule.
    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::TextEdit,
        format_args!("{name} {draft}"),
    );

    let changed = response.changed();
    if changed {
        ui.data_mut(|d| d.insert_temp(id, draft.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    Edited {
        text: changed.then_some(draft),
    }
}

/// The gap between a number field's text and its border, left and right, on a row: a
/// select's.
const ROW_PAD_X: f32 = 6.0;
/// The gap between a number and its unit.
const UNIT_GAP: f32 = 4.0;

/// How a number field is framed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    /// A row's field, flat, as the free-text row is.
    Row,
    /// Recessed in the s-number's own bevel, with the number centered: the range editor,
    /// whose fields edit the control's numbers and read as the same family.
    Inset,
}

/// Draw a plain number field holding `value`, and return the number on the frame it is
/// committed.
///
/// `value` is the text shown while nobody is typing, which is the caller's to format; a value
/// the field would not commit as it stands wears the invalid border, as a draft does.
#[allow(clippy::too_many_arguments)]
pub fn number(
    ui: &mut Ui,
    rect: Rect,
    name: impl crate::ui::Name,
    value: &str,
    bounds: crate::nodes::NumberField,
    chrome: Chrome,
    theme: &Theme,
    zoom: f32,
) -> Option<f32> {
    use eframe::egui::{Align, Align2, Event, Key, Pos2};

    let id = ui.id().with(("number", &name));
    let focused = ui.memory(|m| m.has_focus(id));
    let stored = ui.data_mut(|d| d.get_temp::<String>(id));
    let mut draft = match (&stored, focused) {
        (Some(s), true) => s.clone(),
        _ => value.to_string(),
    };

    // What cannot be part of a number never reaches the field: typed and pasted text is
    // filtered before the `TextEdit` reads it, so a letter is not shown for a frame and then
    // taken away.
    if focused {
        ui.input_mut(|i| {
            for event in &mut i.events {
                if let Event::Text(t) | Event::Paste(t) = event {
                    t.retain(|c| bounds.admits(c));
                }
            }
        });
    }

    let radius = eframe::egui::CornerRadius::same(theme::RADIUS_SM);
    ui.painter()
        .rect_filled(rect, radius, theme.bg_interactive());

    // A row's size on a row, and the popup's own on an inset field, so its number reads the
    // same size as the default printed beside it.
    let size = match chrome {
        Chrome::Row => theme::FONT_TINY,
        Chrome::Inset => theme::FONT_BASE,
    };
    let font = FontId::proportional(theme::font_size(size, zoom));
    let row = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let pad_x = match chrome {
        Chrome::Row => ROW_PAD_X * zoom,
        Chrome::Inset => 2.0 * zoom,
    };
    // The unit sits inside the field's right edge, muted, and the text is laid out in what
    // is left of it, against it: `64 px`, with the number's own digits ending at the gap.
    let mut text_rect = rect;
    if !bounds.unit.is_empty() {
        let unit = ui.painter().text(
            Pos2::new(rect.max.x - pad_x, rect.center().y),
            Align2::RIGHT_CENTER,
            bounds.unit,
            font.clone(),
            theme.text_muted(),
        );
        text_rect.max.x = unit.min.x - UNIT_GAP * zoom + pad_x;
    }
    let pad = Margin::symmetric(points(pad_x), points((rect.height() - row) * 0.5));
    let response = ui
        .scope_builder(eframe::egui::UiBuilder::new().max_rect(text_rect), |ui| {
            ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
            TextEdit::singleline(&mut draft)
                .id(id)
                .frame(Frame::NONE.inner_margin(pad))
                .font(font)
                .horizontal_align(match chrome {
                    Chrome::Row => Align::Max,
                    Chrome::Inset => Align::Center,
                })
                .text_color(theme.text_primary())
                .desired_width(text_rect.width())
                .show(ui)
        })
        .inner
        .response;

    // The border last, from the state the field ended the frame in: a select's three —
    // `border_normal` at rest, `primary_muted` under the pointer, `primary` while it has the
    // keyboard — and silvia's invalid red over all of them while the draft would not commit
    // as it stands. The inset field keeps its bevel at rest.
    let border = if !bounds.holds(&draft) {
        Some(INVALID.color())
    } else if response.has_focus() {
        Some(theme.primary())
    } else if response.hovered() {
        Some(theme.primary_muted())
    } else if chrome == Chrome::Row {
        Some(theme.border_normal())
    } else {
        None
    };
    if chrome == Chrome::Inset {
        crate::ui::number::bevel_border(ui.painter(), rect, zoom, theme);
    }
    if let Some(border) = border {
        ui.painter().rect_stroke(
            rect,
            radius,
            eframe::egui::Stroke::new(1.0, border),
            eframe::egui::StrokeKind::Inside,
        );
    }
    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::TextEdit,
        format_args!("{name} {draft}"),
    );

    // What was being typed, if anything: this frame's draft while the field had the
    // keyboard, or the one it kept when the keyboard left between frames — a Tab away.
    let pending = if focused { Some(draft.clone()) } else { stored };
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        ui.data_mut(|d| d.remove::<String>(id));
        return None;
    }
    // A click anywhere else commits on the frame it lands, rather than on the next one where
    // egui reports the focus lost: the popup this field sits in may close on that same
    // click, and a field that is not drawn again is never told.
    let clicked_away =
        response.has_focus() && ui.input(|i| i.pointer.any_click()) && !response.contains_pointer();
    if (response.lost_focus() || clicked_away)
        && let Some(text) = pending
    {
        response.surrender_focus();
        ui.data_mut(|d| d.remove::<String>(id));
        return bounds.read(&text);
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, draft));
    }
    None
}

/// Draw a one-line text field holding `value`, and return what it holds on the frame it is
/// committed — Enter, or the field left — never on a keystroke. Escape puts back what was
/// there. The text scrolls inside the field rather than wrapping, so the row never grows.
///
/// A row's field, framed as [`number`]'s is on a row: `border_normal` at rest, `primary_muted`
/// under the pointer, `primary` while it has the keyboard. `hint` is what the field says while
/// what is typed is empty. A name a sender goes out under, which restarts the sender on every
/// commit.
#[allow(clippy::too_many_arguments)]
pub fn line(
    ui: &mut Ui,
    rect: Rect,
    name: impl crate::ui::Name,
    value: &str,
    hint: &str,
    hover: &str,
    theme: &Theme,
    zoom: f32,
) -> Option<String> {
    let id = ui.id().with(("line", &name));
    let focused = ui.memory(|m| m.has_focus(id));
    let stored = ui.data_mut(|d| d.get_temp::<String>(id));
    let mut draft = match (&stored, focused) {
        (Some(s), true) => s.clone(),
        _ => value.to_string(),
    };

    let radius = eframe::egui::CornerRadius::same(theme::RADIUS_SM);
    ui.painter()
        .rect_filled(rect, radius, theme.bg_interactive());
    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, zoom));
    let row = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let pad = Margin::symmetric(
        points(ROW_PAD_X * zoom),
        points((rect.height() - row) * 0.5),
    );
    let response = ui
        .scope_builder(eframe::egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            TextEdit::singleline(&mut draft)
                .id(id)
                .frame(Frame::NONE.inner_margin(pad))
                .font(font)
                .hint_text(hint)
                .text_color(theme.text_primary())
                .desired_width(rect.width())
                .clip_text(true)
                .show(ui)
        })
        .inner
        .response
        .response;

    let border = if response.has_focus() {
        theme.primary()
    } else if response.hovered() {
        theme.primary_muted()
    } else {
        theme.border_normal()
    };
    ui.painter().rect_stroke(
        rect,
        radius,
        eframe::egui::Stroke::new(1.0, border),
        eframe::egui::StrokeKind::Inside,
    );
    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::TextEdit,
        format_args!("{name} {draft}"),
    );
    let response = if response.has_focus() {
        response
    } else {
        response.on_hover_text(hover)
    };

    // What was being typed, if anything: this frame's draft while the field had the
    // keyboard, or the one it kept when the keyboard left between frames.
    let pending = if focused { Some(draft.clone()) } else { stored };
    if ui.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
        ui.data_mut(|d| d.remove::<String>(id));
        return None;
    }
    let clicked_away =
        response.has_focus() && ui.input(|i| i.pointer.any_click()) && !response.contains_pointer();
    if (response.lost_focus() || clicked_away)
        && let Some(text) = pending
    {
        response.surrender_focus();
        ui.data_mut(|d| d.remove::<String>(id));
        return Some(text);
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, draft));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::canvas;
    use eframe::egui::{Pos2, vec2};

    const PARAGRAPH: &str = "A note is a box of text on the canvas. It connects to nothing and \
        changes nothing, and it is sized by hand: a label beside one node, or a paragraph \
        across the top of a canvas that says what the patch does, where the camera goes in \
        and which fader to reach for when the chorus lands.\n\nZooming in or out must never \
        move a word from one line to the next.";

    /// The width of a value box at zoom 1 on a fresh node of `slug`, as the canvas lays it out.
    fn box_width(slug: &str) -> f32 {
        let mut g = crate::graph::Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, slug, Pos2::ZERO).expect("a registry slug");
        let body = Rect::from_min_size(
            Pos2::ZERO,
            vec2(canvas::node_width(g.get(id).expect("the node")), 200.0),
        );
        canvas::row_block(body, canvas::Row::Value(0), 0.0, 100.0).width()
    }

    /// Each row's length in characters, newline included, which is where the lines break,
    /// and how wide the widest row is.
    fn breaks(ctx: &Context, width: f32, zoom: f32) -> (Vec<usize>, f32) {
        let galley = box_galley(ctx, PARAGRAPH, width * zoom, zoom, Color32::WHITE, 0.0);
        let rows: Vec<usize> = galley
            .rows
            .iter()
            .map(|r| r.char_count_including_newline().0)
            .collect();
        assert_eq!(
            rows.iter().sum::<usize>(),
            PARAGRAPH.chars().count(),
            "the rows at zoom {zoom} hold every character once, so a caret maps onto them",
        );
        (rows, galley.size().x)
    }

    /// A box of lines breaks its text in the same places at every zoom whose font size scales
    /// with it, from where the floor lets go up to the canvas's closest, and the text is never
    /// wider than the room inside the box's padding at any zoom at all.
    #[test]
    fn a_box_of_lines_breaks_in_the_same_places_at_every_zoom() {
        let ctx = Context::default();
        // The fonts `apply` sets are the ones the frame after it lays text out in.
        for _ in 0..2 {
            let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ctx| {
                theme::apply(ctx, &Theme::default());
            });
            output.textures_delta.clear();
        }

        // Where `font_size` stops holding the floor: 5.5 px rounds up to 6.
        let scales = 5.5 / theme::FONT_TINY;
        // The note's own box, the Text node's, and a note dragged wide.
        for width in [box_width("note"), box_width("text"), 480.0] {
            let (at_one, _) = breaks(&ctx, width, 1.0);
            assert!(at_one.len() > 3, "the paragraph wraps in a {width} box");
            let mut zoom = canvas::MIN_ZOOM;
            while zoom <= canvas::MAX_ZOOM {
                let (rows, painted) = breaks(&ctx, width, zoom);
                let room = box_room(width * zoom, zoom);
                // A galley's size is rounded up to egui's own rounding of a point.
                assert!(
                    painted <= room + eframe::emath::GUI_ROUNDING,
                    "at zoom {zoom} a {width} box's text is {painted} wide in {room} of room",
                );
                if zoom >= scales {
                    assert_eq!(
                        rows, at_one,
                        "a {width} box breaks elsewhere at zoom {zoom}"
                    );
                }
                zoom += 0.0025;
            }
        }
    }
}
