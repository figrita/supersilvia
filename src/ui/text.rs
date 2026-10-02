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

use crate::ui::theme::{self, Hsl, Theme};

/// What a field reports back: what was typed, if anything, and how tall the text turned out
/// once it wrapped.
pub struct Edited {
    /// `Some` on the frame a keystroke changed it.
    pub text: Option<String>,
    /// How tall the field drew itself, in world units — the wrapped text, the padding, and
    /// never less than the lines it was asked for.
    ///
    /// Measured rather than worked out from a line count. Only this function knows the font
    /// and the width, and those are what wrapping depends on; a node laid out from an assumed
    /// line height gains the difference on every line, which is a box that outgrows its text.
    pub height: f32,
}

/// The gap between the text and the field's own border, left and right.
const PAD_X: f32 = 10.0;
/// The same above and below, on a field of more than one line. A one-line field centers its
/// single row instead, so it has no use for this.
const PAD_Y: f32 = 8.0;
use eframe::egui::{FontId, Frame, Margin, Rect, TextEdit, Ui};

/// A `Margin` counts in whole points and holds them in an `i8`, so a zoomed-out padding
/// rounds to the nearest point and a zoomed-in one stops growing at 127.
fn points(v: f32) -> i8 {
    i8::try_from(v.round().clamp(0.0, 127.0) as i32).unwrap_or(i8::MAX)
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

    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
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
            field
                .id(id)
                .frame(Frame::NONE.inner_margin(pad))
                .font(font)
                .hint_text(placeholder)
                .text_color(theme.text_primary())
                // The frame's own margin is taken out of this, so the text wraps inside the
                // padding rather than up against the border.
                .desired_width(rect.width())
                .desired_rows(lines as usize)
                .clip_text(lines == 1)
                .show(ui)
        })
        .inner;
    // The text itself, and nothing else — not the widget's allocated rect, which carries
    // whatever egui rounded it up to and leaves the box a little taller on every line.
    // Never less than the lines the definition asked for, so an empty box keeps its floor.
    let content = out.galley.size().y.max(f32::from(lines) * row);
    let response = out.response;
    // Back out of the canvas zoom: a node's height is world geometry and must not move when
    // somebody zooms in.
    let drawn = 2.0f32.mul_add(pad_y, content) / zoom.max(0.01);

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
        height: drawn,
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
    let font = FontId::monospace(theme::font_size(size, zoom));
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
    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
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
