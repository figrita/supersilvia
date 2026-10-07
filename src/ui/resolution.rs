// SPDX-License-Identifier: AGPL-3.0-or-later

//! The resolution picker: a fixed width and height picked as two things a person already
//! thinks in — a shape and a short side, "a 16:9 projector at 1080" — or typed exactly.
//!
//! **Closed**, it is one select-sized row: a painted rectangle of the picture's shape, the
//! ratio's name and the short side (`16:9 · 1080`), and the select's chevron. **Open**, a
//! popover holds a strip of ratios, each a painted rectangle of its own shape over its name,
//! with **Wide** and **Tall** above it; a row of short sides under that, each one's cost on
//! hover; a readout of the size, its megapixels and the memory an Output of that size commits
//! ([`crate::nodes::output::megabytes_at`], the figure the Output's own row prints); and a
//! width and a height to type. Changing the shape keeps the short side, changing the short
//! side keeps the shape, and **Tall** mirrors every shape and keeps the short side. A typed
//! size that is no cell on the strip is **Free**.
//!
//! Every size the strip makes is even on both sides, because a recording's encoder wants
//! even sizes, and so is a typed one. 21:9 is a marketing name rather than one ratio, so it
//! takes the size ultrawide panels are sold at where there is one, 2560x1080, and otherwise
//! falls back to the same even rounding as every other ratio.
//!
//! Every glyph is paint, as the select's chevron is: the text face is proportional, and a
//! character standing in for a shape is at the mercy of the font stack.
//!
//! It draws and returns, as everything in `ui/` does: the closed row is a `Response`, and the
//! popover says what was picked and whether it is to close.

use crate::nodes::output::{MAX_SIDE, MIN_SIDE, megabytes_at};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, WidgetType, pos2, vec2,
};

/// One cell of the strip: a ratio by name, as the long side over the short one in whole
/// numbers so the arithmetic is exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub name: &'static str,
    long: u32,
    short: u32,
    /// The long side it is sold at for a short side, ahead of the arithmetic.
    sold: &'static [(u32, u32)],
}

impl Ratio {
    const fn new(name: &'static str, long: u32, short: u32) -> Self {
        Self {
            name,
            long,
            short,
            sold: &[],
        }
    }

    /// The long side for `short`: the size it is sold at, or `short` times the ratio rounded
    /// to the nearest even number.
    pub fn long_side(&self, short: u32) -> u32 {
        if let Some(&(_, long)) = self.sold.iter().find(|(s, _)| *s == short) {
            return long;
        }
        let n = u64::from(short) * u64::from(self.long);
        let d = u64::from(self.short) * 2;
        let even = (n + d / 2) / d * 2;
        u32::try_from(even)
            .unwrap_or(MAX_SIDE)
            .clamp(MIN_SIDE, MAX_SIDE)
    }

    /// The long side over the short one.
    fn value(&self) -> f32 {
        self.long as f32 / self.short as f32
    }

    /// The name as a tall picture of this shape reads it: `9:16`, `1:1.85`.
    fn mirrored(&self) -> String {
        match self.name.split_once(':') {
            Some((long, short)) => format!("{short}:{long}"),
            None => format!("1:{}", self.name),
        }
    }
}

/// The strip, narrowest first. **Free** is the cell after the last, and is no ratio.
pub const RATIOS: [Ratio; 5] = [
    Ratio::new("1:1", 1, 1),
    Ratio::new("4:3", 4, 3),
    Ratio::new("16:10", 16, 10),
    Ratio::new("16:9", 16, 9),
    Ratio {
        name: "21:9",
        long: 64,
        short: 27,
        sold: &[(1080, 2560)],
    },
];

/// The short sides offered, in pixels: 1080 is 1920x1080 wide and 1080x1920 tall. The cost of
/// a size grows with its square, and this is glitch art rather than delivery, so the small
/// sizes below 720 earn their place beside the panel heights above it.
pub const HEIGHTS: [u32; 6] = [240, 480, 512, 720, 1080, 1200];

/// The free cell's name.
const FREE: &str = "Free";

/// What a size is, as the picker reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// The strip's cell, an index into [`RATIOS`]; `None` is Free.
    pub ratio: Option<usize>,
    /// The short side.
    pub short: u32,
    /// Taller than wide. A square is wide.
    pub tall: bool,
}

/// `ratio` at `short` on its short side, wide or tall.
pub fn size_of(ratio: &Ratio, short: u32, tall: bool) -> (u32, u32) {
    let short = short.clamp(MIN_SIDE, MAX_SIDE);
    let long = ratio.long_side(short);
    if tall { (short, long) } else { (long, short) }
}

/// Which cell `size` is: the ratio whose long side at this short side is exactly this one,
/// or Free.
pub fn shape_of((w, h): (u32, u32)) -> Shape {
    let (short, long) = (w.min(h), w.max(h));
    Shape {
        ratio: RATIOS.iter().position(|r| r.long_side(short) == long),
        short,
        tall: h > w,
    }
}

/// `v` rounded to the nearest even number inside the bounds.
pub fn even(v: f64) -> u32 {
    let lo = f64::from(MIN_SIDE);
    let hi = f64::from(MAX_SIDE);
    ((v.clamp(lo, hi) / 2.0).round() * 2.0) as u32
}

/// `size` at a new short side, keeping its shape: the ratio's own long side where it is on
/// the strip and `free` is not asked, and its own proportion otherwise.
pub fn with_short(size: (u32, u32), short: u32, free: bool) -> (u32, u32) {
    let shape = shape_of(size);
    if let Some(i) = shape.ratio.filter(|_| !free) {
        return size_of(&RATIOS[i], short, shape.tall);
    }
    let long = f64::from(size.0.max(size.1)) * f64::from(short) / f64::from(shape.short.max(1));
    let (short, long) = (even(f64::from(short)), even(long));
    if shape.tall {
        (short, long)
    } else {
        (long, short)
    }
}

/// A typed width and height as the option holds them: each even and inside the bounds.
pub fn typed(w: f32, h: f32) -> (u32, u32) {
    (even(f64::from(w)), even(f64::from(h)))
}

/// The ratio's name as `size` reads it: mirrored when it is tall, Free off the strip.
pub fn ratio_name(size: (u32, u32)) -> String {
    let shape = shape_of(size);
    match shape.ratio {
        Some(i) if shape.tall => RATIOS[i].mirrored(),
        Some(i) => RATIOS[i].name.to_string(),
        None => FREE.to_string(),
    }
}

/// What the closed row says: the ratio and the short side, `16:9 · 1080`, or the size itself
/// where it is no ratio on the strip.
pub fn caption(size: (u32, u32)) -> String {
    let shape = shape_of(size);
    match shape.ratio {
        Some(_) => format!("{} · {}", ratio_name(size), shape.short),
        None => format!("{}×{}", size.0, size.1),
    }
}

/// `size` in megapixels as the readout prints it: two places under one, one above it.
fn megapixels((w, h): (u32, u32)) -> String {
    let mp = f64::from(w) * f64::from(h) / 1e6;
    if mp < 1.0 {
        format!("{mp:.2}")
    } else {
        format!("{mp:.1}")
    }
}

/// The readout under the short sides: the size, then its megapixels and its memory.
pub fn readout(size: (u32, u32)) -> (String, String) {
    (
        format!("{}×{}", size.0, size.1),
        format!(
            " · {} MP · {} MB",
            megapixels(size),
            megabytes_at(size.0, size.1)
        ),
    )
}

/// The size a strip or a short side starts from while an entry above the strip is chosen
/// whose size is not known.
const FALLBACK: (u32, u32) = (1920, 1080);

/// What the closed row shows: a size, or the name of an entry above the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown<'a> {
    Size(u32, u32),
    Named(&'a str),
}

impl Shown<'_> {
    fn text(self) -> String {
        match self {
            Shown::Size(w, h) => caption((w, h)),
            Shown::Named(label) => label.to_string(),
        }
    }
}

/// An entry above the strip that is no size of its own but follows something: the mixer's
/// *Match display* and *Match viewport*.
pub struct Entry<'a> {
    /// The last word of its accessible name, `{name} {key}`.
    pub key: &'a str,
    pub label: &'a str,
    /// It is the one chosen.
    pub on: bool,
    /// The size it stands for, where that is known: what the strip and the short sides start
    /// from while it is chosen, and what the readout reads.
    pub size: Option<(u32, u32)>,
    /// What the readout says while it is chosen and its size is not known.
    pub note: &'a str,
}

// Geometry, in points at zoom 1.

/// The select's inner padding, its chevron's half-width and the gap before it — the
/// select's own, `node_widget::select_button`'s.
const PAD: f32 = 6.0;
const CHEVRON: f32 = 3.5;
const CHEVRON_GAP: f32 = 6.0;
/// The closed row's glyph box, and the gap after it.
const GLYPH: (f32, f32) = (14.0, 10.0);
const GLYPH_GAP: f32 = 5.0;

/// The strip's cells, every ratio and Free, one row: a glyph box over a name.
const COLUMNS: usize = RATIOS.len() + 1;
const CELL: (f32, f32) = (38.0, 40.0);
const CELL_GLYPH: (f32, f32) = (28.0, 16.0);
const GAP: f32 = 3.0;
/// The popover's content width: the strip's.
pub const WIDTH: f32 = CELL.0 * COLUMNS as f32 + GAP * (COLUMNS - 1) as f32;
/// A short side's button, an entry's row and a section's title row.
const ROW: f32 = 22.0;
const TITLE: f32 = 18.0;
/// The space either side of a section's rule.
const RULE_GAP: f32 = 6.0;
/// A typed side's field, the s-number's height.
const FIELD: (f32, f32) = (64.0, 20.0);

/// How wide the closed row is drawn for what it shows, before the room the row leaves it.
pub fn closed_width(ctx: &eframe::egui::Context, shown: Shown<'_>, zoom: f32) -> f32 {
    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, zoom));
    let text = shown.text();
    let text_w = ctx.fonts_mut(|f| text.chars().map(|c| f.glyph_width(&font, c)).sum::<f32>());
    let glyph = match shown {
        Shown::Size(..) => GLYPH.0 + GLYPH_GAP,
        Shown::Named(_) => 0.0,
    };
    text_w + (PAD * 2.0 + glyph + CHEVRON * 2.0 + CHEVRON_GAP) * zoom
}

/// The closed row in `rect`: the select's ground and border, the shape, the caption and the
/// chevron, the border and the chevron in `primary` while the popover is up. Named
/// `{name} {caption}`, as a select is named by what it shows.
#[allow(clippy::too_many_arguments)] // every one is drawn; a struct would only move the list
pub fn closed(
    ui: &mut Ui,
    rect: Rect,
    name: impl crate::ui::Name,
    shown: Shown<'_>,
    open: bool,
    theme: &Theme,
    zoom: f32,
) -> Response {
    let response = ui.interact(rect, ui.id().with(("resolution", &name)), Sense::click());
    crate::ui::cursor(&response, CursorIcon::PointingHand);
    let border = if open {
        theme.primary()
    } else if response.hovered() {
        theme.primary_muted()
    } else {
        theme.border_normal()
    };
    let painter = ui.painter();
    if open {
        painter.rect_stroke(
            rect.expand(2.0 * zoom),
            CornerRadius::same(theme::RADIUS_MD),
            Stroke::new(2.0 * zoom, theme.primary().gamma_multiply(0.2)),
            StrokeKind::Outside,
        );
    }
    crate::ui::field(painter, rect, theme.bg_interactive(), None, border);

    let painter = painter.with_clip_rect(rect.intersect(ui.clip_rect()));
    let mut x = rect.min.x + PAD * zoom;
    if let Shown::Size(w, h) = shown {
        let glyph = Rect::from_min_size(
            pos2(x, rect.center().y - GLYPH.1 * zoom * 0.5),
            vec2(GLYPH.0, GLYPH.1) * zoom,
        );
        outline(
            &painter,
            glyph,
            size_ratio((w, h)),
            1.0 * zoom,
            theme.text_secondary(),
        );
        x = glyph.max.x + GLYPH_GAP * zoom;
    }
    let text = shown.text();
    painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        &text,
        FontId::proportional(theme::font_size(theme::FONT_TINY, zoom)),
        theme.text_primary(),
    );
    let cx = rect.max.x - PAD * zoom - CHEVRON * zoom;
    let cy = rect.center().y;
    let (w, h) = (CHEVRON * zoom, CHEVRON * 0.75 * zoom);
    painter.add(eframe::egui::Shape::convex_polygon(
        vec![
            pos2(cx - w, cy - h * 0.5),
            pos2(cx + w, cy - h * 0.5),
            pos2(cx, cy + h),
        ],
        if open {
            theme.primary()
        } else {
            theme.text_muted()
        },
        Stroke::NONE,
    ));
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(WidgetType::ComboBox, true, format!("{name} {text}"))
    });
    response
}

/// What the popover was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// An entry above the strip, by its place in the list it was given.
    Entry(usize),
    Size(u32, u32),
}

/// One frame of the popover.
pub struct Popped {
    pub picked: Option<Pick>,
    /// A click landed outside it, or Escape was pressed with no field typing.
    pub dismissed: bool,
}

/// The popover at `at`, for the size `size` holds, with `entries` above the strip; `None`
/// where one of them is chosen instead. Its controls are named `{name} 16:9`, `{name} Free`,
/// `{name} Wide`, `{name} 1080`, `{name} width`, `{name} height` and `{name} {key}` for each
/// entry, none of them the closed row's own name.
pub fn popover(
    ui: &mut Ui,
    at: Pos2,
    name: &str,
    size: Option<(u32, u32)>,
    entries: &[Entry<'_>],
    theme: &Theme,
) -> Popped {
    let id = ui.id().with(("resolution-popover", name));
    // Free, picked with the strip's last cell while the size is still a ratio on it: what a
    // short side keeps from then on is the size's own proportion. Forgotten on close.
    let free_id = id.with("free");
    let typing = ui.ctx().egui_wants_keyboard_input();
    let mut picked = None;
    let shown = crate::ui::popup::Popup::new(id, at).show(ui.ctx(), theme, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let free_asked = ui.data(|d| d.get_temp::<bool>(free_id)).unwrap_or(false);
        let chosen = entries.iter().find(|e| e.on && size.is_none());
        let base = size
            .or_else(|| chosen.and_then(|e| e.size))
            .unwrap_or(FALLBACK);
        let shape = shape_of(base);
        let free = size.is_some() && (free_asked || shape.ratio.is_none());

        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                ui.add_space(GAP);
            }
            let on = entry.on && size.is_none();
            let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::hover());
            if cell(ui, rect, &format!("{name} {}", entry.key), on, theme).clicked() && !on {
                picked = Some(Pick::Entry(i));
            }
            paint_text(
                ui.painter(),
                rect.center(),
                Align2::CENTER_CENTER,
                entry.label,
                cell_ink(ui, rect, on, theme),
            );
        }
        if !entries.is_empty() {
            rule(ui, theme);
        }

        // Shape: the title, Wide and Tall at its right end, and the strip.
        let (title, _) = ui.allocate_exact_size(vec2(WIDTH, TITLE), Sense::hover());
        section_title(ui.painter(), title, "Shape", None, theme);
        let captions = ["Wide", "Tall"];
        let toggle_w = crate::ui::press::choice_width(ui.ctx(), &captions, 1.0);
        let toggle = Rect::from_min_size(
            pos2(title.max.x - toggle_w, title.min.y + 1.0),
            vec2(toggle_w, TITLE - 2.0),
        );
        let tall = usize::from(shape.tall);
        if crate::ui::press::choice(ui, toggle, &captions, tall, name, theme, 1.0).is_some()
            && base.0 != base.1
        {
            picked = Some(Pick::Size(base.1, base.0));
        }
        ui.add_space(4.0);
        let rows = (RATIOS.len() + 1).div_ceil(COLUMNS);
        let (strip, _) = ui.allocate_exact_size(
            vec2(WIDTH, CELL.1 * rows as f32 + GAP * (rows - 1) as f32),
            Sense::hover(),
        );
        for i in 0..=RATIOS.len() {
            let (col, row) = (i % COLUMNS, i / COLUMNS);
            let rect = Rect::from_min_size(
                strip.min + vec2(col as f32 * (CELL.0 + GAP), row as f32 * (CELL.1 + GAP)),
                vec2(CELL.0, CELL.1),
            );
            let ratio = RATIOS.get(i);
            let label = ratio.map_or(FREE, |r| r.name);
            let on = match ratio {
                Some(_) => size.is_some() && !free && shape.ratio == Some(i),
                None => free,
            };
            let response = cell(ui, rect, &format!("{name} {label}"), on, theme);
            let ink = cell_ink(ui, rect, on, theme);
            let glyph = Rect::from_center_size(
                pos2(rect.center().x, rect.min.y + 3.0 + CELL_GLYPH.1 * 0.5),
                vec2(CELL_GLYPH.0, CELL_GLYPH.1),
            );
            match ratio {
                Some(r) => {
                    let value = if shape.tall {
                        1.0 / r.value()
                    } else {
                        r.value()
                    };
                    outline(ui.painter(), glyph, value, 1.5, ink);
                }
                None => dashed_square(ui.painter(), glyph, ink),
            }
            paint_text(
                ui.painter(),
                pos2(rect.center().x, rect.max.y - 3.0),
                Align2::CENTER_BOTTOM,
                label,
                ink,
            );
            let response = match ratio {
                Some(r) => {
                    let to = size_of(r, shape.short, shape.tall);
                    response.on_hover_text(format!("{}×{}", to.0, to.1))
                }
                None => response.on_hover_text("Type a width and a height below"),
            };
            if response.clicked() {
                if let Some(r) = ratio {
                    ui.data_mut(|d| d.remove::<bool>(free_id));
                    let to = size_of(r, shape.short, shape.tall);
                    if size != Some(to) {
                        picked = Some(Pick::Size(to.0, to.1));
                    }
                } else {
                    ui.data_mut(|d| d.insert_temp(free_id, true));
                    ui.memory_mut(|m| m.request_focus(field_id(ui, name, "width")));
                }
            }
        }
        rule(ui, theme);

        // Size: the short sides in one row, each one's cost on hover, and the readout.
        let (title, _) = ui.allocate_exact_size(vec2(WIDTH, TITLE), Sense::hover());
        section_title(ui.painter(), title, "Size", Some("short side"), theme);
        ui.add_space(4.0);
        let (heights, _) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::hover());
        let width = (WIDTH - GAP * (HEIGHTS.len() - 1) as f32) / HEIGHTS.len() as f32;
        for (i, short) in HEIGHTS.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                heights.min + vec2(i as f32 * (width + GAP), 0.0),
                vec2(width, ROW),
            );
            let on = size.is_some() && shape.short == short;
            let to = with_short(base, short, free);
            let response = cell(ui, rect, &format!("{name} {short}"), on, theme);
            let ink = cell_ink(ui, rect, on, theme);
            paint_text(
                ui.painter(),
                rect.center(),
                Align2::CENTER_CENTER,
                &short.to_string(),
                ink,
            );
            let response = response.on_hover_text(format!(
                "{}×{} · {} MB",
                to.0,
                to.1,
                megabytes_at(to.0, to.1)
            ));
            if response.clicked() && size != Some(to) {
                picked = Some(Pick::Size(to.0, to.1));
            }
        }
        ui.add_space(RULE_GAP);
        let (line, _) = ui.allocate_exact_size(vec2(WIDTH, 14.0), Sense::hover());
        let font = FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0));
        match size.or_else(|| chosen.and_then(|e| e.size)) {
            Some(size) => {
                let (dims, rest) = readout(size);
                let first = ui.painter().text(
                    line.left_center(),
                    Align2::LEFT_CENTER,
                    dims,
                    font.clone(),
                    theme.text_primary(),
                );
                ui.painter().text(
                    first.right_center(),
                    Align2::LEFT_CENTER,
                    rest,
                    font,
                    theme.text_muted(),
                );
            }
            None => {
                ui.painter().text(
                    line.left_center(),
                    Align2::LEFT_CENTER,
                    chosen.map_or("", |e| e.note),
                    font,
                    theme.text_muted(),
                );
            }
        }
        rule(ui, theme);

        // The exact size, typed: a width, a height, and what typing does.
        let (fields, _) = ui.allocate_exact_size(vec2(WIDTH, FIELD.1), Sense::hover());
        let bounds = crate::nodes::NumberField {
            min: MIN_SIDE as f32,
            max: MAX_SIDE as f32,
            integer: true,
            unit: "",
        };
        let w_rect = Rect::from_min_size(fields.min, vec2(FIELD.0, FIELD.1));
        let times = pos2(w_rect.max.x + 8.0, fields.center().y);
        let h_rect = Rect::from_min_size(
            pos2(w_rect.max.x + 16.0, fields.min.y),
            vec2(FIELD.0, FIELD.1),
        );
        paint_text(
            ui.painter(),
            times,
            Align2::CENTER_CENTER,
            "×",
            theme.text_muted(),
        );
        let chrome = crate::ui::text::Chrome::Inset;
        let w_name = format!("{name} width");
        let h_name = format!("{name} height");
        let typed_w = crate::ui::text::number(
            ui,
            w_rect,
            &w_name,
            &base.0.to_string(),
            bounds,
            chrome,
            theme,
            1.0,
        );
        let typed_h = crate::ui::text::number(
            ui,
            h_rect,
            &h_name,
            &base.1.to_string(),
            bounds,
            chrome,
            theme,
            1.0,
        );
        let to = match (typed_w, typed_h) {
            (Some(w), _) => Some(typed(w, base.1 as f32)),
            (None, Some(h)) => Some(typed(base.0 as f32, h)),
            (None, None) => None,
        };
        if let Some(to) = to
            && size != Some(to)
        {
            picked = Some(Pick::Size(to.0, to.1));
        }
        ui.painter().text(
            pos2(fields.max.x, fields.center().y),
            Align2::RIGHT_CENTER,
            "typing picks Free",
            FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0)),
            theme.text_muted(),
        );
    });
    let escaped = !typing && ui.input(|i| i.key_pressed(eframe::egui::Key::Escape));
    let dismissed = shown.clicked_away || escaped;
    if dismissed {
        ui.data_mut(|d| d.remove::<bool>(free_id));
    }
    Popped { picked, dismissed }
}

/// The id [`crate::ui::text::number`] keys a typed side's field by in `ui`.
fn field_id(ui: &Ui, name: &str, side: &str) -> eframe::egui::Id {
    ui.id().with(("number", &format!("{name} {side}")))
}

/// The long side over the short, inverted for a tall size: the shape a glyph draws.
fn size_ratio((w, h): (u32, u32)) -> f32 {
    w.max(1) as f32 / h.max(1) as f32
}

/// A rectangle of `ratio` (width over height), as large as fits in `bounds` and centered in
/// it, outlined in `ink`.
fn outline(painter: &Painter, bounds: Rect, ratio: f32, width: f32, ink: Color32) {
    let scale = (bounds.width() / ratio).min(bounds.height());
    let size = vec2((ratio * scale).max(3.0), scale.max(3.0));
    painter.rect_stroke(
        Rect::from_center_size(bounds.center(), size),
        CornerRadius::same(1),
        Stroke::new(width, ink),
        StrokeKind::Inside,
    );
}

/// Free's glyph: a dashed square, the shape that is no shape.
fn dashed_square(painter: &Painter, bounds: Rect, ink: Color32) {
    let side = bounds.height().min(bounds.width()) - 1.5;
    let r = Rect::from_center_size(bounds.center(), vec2(side, side));
    let path = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.extend(eframe::egui::Shape::dashed_line(
        &path,
        Stroke::new(1.5, ink),
        2.5,
        2.0,
    ));
}

/// A cell's ground and border, and its response: `bg_tertiary` at rest, `bg_hover` under the
/// pointer, and `primary` around a tint of it while chosen. Named `name`, selected while it
/// is the one chosen.
fn cell(ui: &mut Ui, rect: Rect, name: &str, on: bool, theme: &Theme) -> Response {
    let response = ui.interact(
        rect,
        ui.id().with(("resolution-cell", &name)),
        Sense::click(),
    );
    crate::ui::cursor(&response, CursorIcon::PointingHand);
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let painter = ui.painter();
    if on {
        painter.rect(
            rect,
            radius,
            theme.primary().gamma_multiply(0.12),
            Stroke::new(1.0, theme.primary()),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, radius, theme.bg_hover());
    } else {
        painter.rect_filled(rect, radius, theme.bg_tertiary());
    }
    response.widget_info(|| eframe::egui::WidgetInfo::selected(WidgetType::Button, true, on, name));
    response
}

/// A cell's ink: `primary` while chosen, `text_primary` under the pointer, `text_muted` at
/// rest.
fn cell_ink(ui: &Ui, rect: Rect, on: bool, theme: &Theme) -> Color32 {
    if on {
        theme.primary()
    } else if ui.rect_contains_pointer(rect) {
        theme.text_primary()
    } else {
        theme.text_muted()
    }
}

fn paint_text(painter: &Painter, at: Pos2, align: Align2, text: &str, ink: Color32) {
    painter.text(
        at,
        align,
        text,
        FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0)),
        ink,
    );
}

/// A section's title, with a muted note after it.
fn section_title(painter: &Painter, rect: Rect, title: &str, note: Option<&str>, theme: &Theme) {
    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0));
    let first = painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        title,
        font.clone(),
        theme.text_secondary(),
    );
    if let Some(note) = note {
        painter.text(
            first.right_center() + vec2(4.0, 0.0),
            Align2::LEFT_CENTER,
            format!("· {note}"),
            font,
            theme.text_muted(),
        );
    }
}

/// The hairline between two sections, with its air either side.
fn rule(ui: &mut Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, RULE_GAP * 2.0 + 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, theme.border_subtle()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio(name: &str) -> &'static Ratio {
        RATIOS.iter().find(|r| r.name == name).unwrap()
    }

    /// A ratio and a short side make the sizes screens are sold at, wide and tall.
    #[test]
    fn a_ratio_and_a_short_side_make_a_size() {
        assert_eq!(size_of(ratio("16:9"), 1080, false), (1920, 1080));
        assert_eq!(size_of(ratio("16:9"), 1080, true), (1080, 1920));
        assert_eq!(size_of(ratio("16:9"), 720, false), (1280, 720));
        assert_eq!(size_of(ratio("16:9"), 512, false), (910, 512));
        assert_eq!(size_of(ratio("4:3"), 480, false), (640, 480));
        assert_eq!(size_of(ratio("16:10"), 1200, false), (1920, 1200));
        assert_eq!(size_of(ratio("1:1"), 1080, true), (1080, 1080));
        assert_eq!(size_of(ratio("1:1"), 512, false), (512, 512));
    }

    /// 21:9 is the size ultrawide panels are sold at where a short side remains on the strip —
    /// only 1080, since 1440 and 2160 are no longer offered — and the arithmetic otherwise.
    #[test]
    fn twenty_one_by_nine_is_what_ultrawides_are_sold_at() {
        assert_eq!(size_of(ratio("21:9"), 1080, false), (2560, 1080));
        assert_eq!(size_of(ratio("21:9"), 1080, true), (1080, 2560));
        assert_eq!(size_of(ratio("21:9"), 720, false), (1706, 720));
        assert_eq!(size_of(ratio("21:9"), 1440, false), (3414, 1440));
        assert_eq!(size_of(ratio("21:9"), 2160, false), (5120, 2160));
    }

    /// Every size the strip makes is even on both sides, which is what an encoder wants.
    #[test]
    fn every_size_the_strip_makes_is_even() {
        assert_eq!(size_of(ratio("16:9"), 480, false), (854, 480));
        assert_eq!(size_of(ratio("16:9"), 1200, false), (2134, 1200));
        for r in &RATIOS {
            for short in HEIGHTS {
                for tall in [false, true] {
                    let (w, h) = size_of(r, short, tall);
                    assert!(w % 2 == 0 && h % 2 == 0, "{} at {short}: {w}x{h}", r.name);
                }
            }
        }
        assert_eq!(typed(1001.0, 701.4), (1002, 702));
        assert_eq!(typed(3.0, 99_999.0), (MIN_SIDE, MAX_SIDE));
    }

    /// A size the strip makes reads back as its own cell, wide or tall; anything else is Free.
    #[test]
    fn a_size_off_the_strip_is_free() {
        for (i, r) in RATIOS.iter().enumerate() {
            for short in HEIGHTS {
                for tall in [false, true] {
                    let shape = shape_of(size_of(r, short, tall));
                    assert_eq!(shape.short, short);
                    assert_eq!(shape.tall, tall && r.name != "1:1", "{} at {short}", r.name);
                    assert_eq!(shape.ratio, Some(i), "{} at {short}", r.name);
                }
            }
        }
        assert_eq!(
            shape_of((1024, 768)).ratio,
            Some(1),
            "4:3 off the row of short sides"
        );
        assert_eq!(shape_of((1000, 700)).ratio, None);
        assert_eq!(shape_of((1921, 1080)).ratio, None);
        assert_eq!(
            shape_of((3440, 1440)).ratio,
            None,
            "21:9 at 1440 is Free now the sold table only holds 1080"
        );
        assert_eq!(caption((1000, 700)), "1000×700");
        assert_eq!(caption((1920, 1080)), "16:9 · 1080");
        assert_eq!(caption((1080, 1920)), "9:16 · 1080");
        assert_eq!(caption((2560, 1080)), "21:9 · 1080");
    }

    /// A short side keeps the shape: the ratio's own long side on the strip, the size's own
    /// proportion off it or where Free was asked.
    #[test]
    fn a_short_side_keeps_the_shape() {
        assert_eq!(with_short((1920, 1080), 1440, false), (2560, 1440));
        assert_eq!(with_short((2560, 1080), 1440, false), (3414, 1440));
        assert_eq!(with_short((1080, 1920), 720, false), (720, 1280));
        assert_eq!(with_short((1000, 500), 1080, false), (2160, 1080));
        assert_eq!(with_short((2560, 1080), 1440, true), (3414, 1440));
    }

    /// The readout's figure is the Output's own.
    #[test]
    fn the_readout_prints_the_outputs_memory_figure() {
        assert_eq!(
            readout((1280, 720)),
            ("1280×720".to_string(), " · 0.92 MP · 15 MB".to_string())
        );
        assert_eq!(readout((3840, 2160)).1, " · 8.3 MP · 127 MB".to_string());
    }
}
