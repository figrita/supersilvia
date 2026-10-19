// SPDX-License-Identifier: AGPL-3.0-or-later

//! The axes chip and its popover: which of a solid's axes an input reads along
//! ([`crate::graph::Axes`]).
//!
//! **The chip** sits on the input's line, left of its control: a small cube split like a Y
//! into its three faces, the face the input's plane is lit — the left one Front, X and Y, the
//! top one Top, X and Z, the right one Side, Z and Y — and the axes as text beside it, `XY`,
//! `X−Z`. The lit face is solid where neither axis runs backwards, striped one way where the
//! across axis does, the other way where the up axis does, and diagonally where both do.
//!
//! **The popover** has the three planes as presets, each its own cube, and under them a row
//! for each of the picture's axes: X, Y or Z, the one the other row took greyed out, and a
//! minus that runs it backwards.

use crate::graph::{Axes, Axis, Plane};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, WidgetType, pos2, vec2,
};

/// The popover's content width.
const WIDTH: f32 = 156.0;
/// A preset's cell, a row's height, the gap between cells.
const PRESET: f32 = 46.0;
const ROW: f32 = 22.0;
const GAP: f32 = 3.0;
/// The space a row's name takes before its cells.
const NAME: f32 = 44.0;

/// The cube's six outer corners and its center, about `c`, `r` from the center to a corner:
/// top, upper right, lower right, bottom, lower left, upper left.
fn corners(c: Pos2, r: f32) -> ([Pos2; 6], Pos2) {
    let w = r * 0.866;
    (
        [
            pos2(c.x, c.y - r),
            pos2(c.x + w, c.y - r * 0.5),
            pos2(c.x + w, c.y + r * 0.5),
            pos2(c.x, c.y + r),
            pos2(c.x - w, c.y + r * 0.5),
            pos2(c.x - w, c.y - r * 0.5),
        ],
        c,
    )
}

/// One face: its corner where across and up start, and the edges along across and along up.
fn face(c: Pos2, r: f32, plane: Plane) -> (Pos2, eframe::egui::Vec2, eframe::egui::Vec2) {
    let ([top, ur, lr, bottom, ll, ul], center) = corners(c, r);
    let _ = top;
    match plane {
        Plane::Front => (ll, bottom - ll, ul - ll),
        Plane::Side => (bottom, lr - bottom, center - bottom),
        Plane::Top => (center, ur - center, ul - center),
    }
}

/// The cube in `rect`: every face outlined, `plane`'s lit, striped as `axes`' flips say.
pub fn cube(painter: &Painter, rect: Rect, axes: Option<Axes>, lit: Color32, theme: &Theme) {
    let r = rect.height().min(rect.width() / 0.866 * 0.5) * 0.5;
    let c = rect.center();
    let edge = Stroke::new((r * 0.12).clamp(0.75, 1.5), theme.text_muted());
    for plane in [Plane::Front, Plane::Top, Plane::Side] {
        let (o, a, u) = face(c, r, plane);
        let points = vec![o, o + a, o + a + u, o + u];
        let on = axes.is_some_and(|x| x.plane() == plane);
        let fill = if on { lit } else { theme.bg_tertiary() };
        painter.add(Shape::convex_polygon(points, fill, edge));
        let Some(axes) = axes.filter(|_| on) else {
            continue;
        };
        let stripe = Stroke::new((r * 0.1).clamp(0.6, 1.2), theme.bg_primary());
        let at = |s: f32, t: f32| o + a * s + u * t;
        let parts = [0.25, 0.5, 0.75];
        match (axes.across.flipped, axes.up.flipped) {
            (false, false) => {}
            // Across backwards: lines along up, across the across.
            (true, false) => {
                for s in parts {
                    painter.line_segment([at(s, 0.0), at(s, 1.0)], stripe);
                }
            }
            (false, true) => {
                for t in parts {
                    painter.line_segment([at(0.0, t), at(1.0, t)], stripe);
                }
            }
            (true, true) => {
                for k in [0.4, 0.8, 1.2, 1.6] {
                    let (s0, s1) = ((k - 1.0_f32).max(0.0), k.min(1.0));
                    painter.line_segment([at(s0, k - s0), at(s1, k - s1)], stripe);
                }
            }
        }
    }
}

/// How wide the chip is for `axes` at `zoom`, in screen points.
pub fn chip_width(ctx: &eframe::egui::Context, axes: Axes, height: f32, zoom: f32) -> f32 {
    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, zoom));
    let text = ctx.fonts_mut(|f| {
        axes.label()
            .chars()
            .map(|c| f.glyph_width(&font, c))
            .sum::<f32>()
    });
    height + text + 6.0 * zoom
}

/// The chip in `rect`: a pressable cube and the axes as text, named `name` for a pointer and a
/// screen reader. Clicked opens the popover.
pub fn chip(ui: &mut Ui, rect: Rect, name: &str, axes: Axes, theme: &Theme, zoom: f32) -> Response {
    let response = ui.interact(rect, ui.id().with(("axes-chip", name)), Sense::click());
    crate::ui::cursor(&response, CursorIcon::PointingHand);
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let bg = if response.hovered() {
        theme.bg_hover()
    } else {
        theme.bg_secondary()
    };
    painter.rect(
        rect,
        radius,
        bg,
        Stroke::new(1.0, theme.border_subtle()),
        StrokeKind::Inside,
    );
    let glyph =
        Rect::from_min_size(rect.min, vec2(rect.height(), rect.height())).shrink(2.0 * zoom);
    cube(painter, glyph, Some(axes), theme.primary(), theme);
    painter.text(
        pos2(rect.min.x + rect.height() + 1.0 * zoom, rect.center().y),
        Align2::LEFT_CENTER,
        axes.label(),
        FontId::proportional(theme::font_size(theme::FONT_TINY, zoom)),
        theme.text_secondary(),
    );
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(
            WidgetType::Button,
            true,
            format!("{name} {}", axes.label()),
        )
    });
    response
}

/// What the popover did this frame.
pub struct Popped {
    pub picked: Option<Axes>,
    /// A click landed outside it, or Escape was pressed.
    pub dismissed: bool,
}

/// A cell of the popover, raised where `on`, refused where not `enabled`.
fn cell(ui: &mut Ui, rect: Rect, name: &str, on: bool, enabled: bool, theme: &Theme) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(("axes-cell", name)), sense);
    if enabled {
        crate::ui::cursor(&response, CursorIcon::PointingHand);
    }
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
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, radius, theme.bg_hover());
    } else {
        painter.rect_filled(rect, radius, theme.bg_tertiary());
    }
    response
        .widget_info(|| eframe::egui::WidgetInfo::selected(WidgetType::Button, enabled, on, name));
    response
}

/// A cell's ink.
fn ink(ui: &Ui, rect: Rect, on: bool, enabled: bool, theme: &Theme) -> Color32 {
    if !enabled {
        theme.text_disabled()
    } else if on {
        theme.primary()
    } else if ui.rect_contains_pointer(rect) {
        theme.text_primary()
    } else {
        theme.text_muted()
    }
}

/// One of the two rows: `label`, the cells X, Y and Z — the one `taken` by the other row
/// refused — and a minus. What a click makes of `axis`, where one landed.
fn axis_row(
    ui: &mut Ui,
    name: &str,
    label: &str,
    axis: Axis,
    taken: u8,
    theme: &Theme,
) -> Option<Axis> {
    let (row, _) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::hover());
    let font = FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0));
    ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        label,
        font.clone(),
        theme.text_secondary(),
    );
    let cells = WIDTH - NAME;
    let w = (cells - GAP * 4.0) / 4.0;
    let mut out = None;
    for (i, letter) in ['X', 'Y', 'Z'].into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(row.min.x + NAME + i as f32 * (w + GAP), row.min.y),
            vec2(w, ROW),
        );
        let of = i as u8;
        let (on, enabled) = (axis.of == of, of != taken);
        if cell(
            ui,
            rect,
            &format!("{name} {label} {letter}"),
            on,
            enabled,
            theme,
        )
        .clicked()
            && !on
        {
            out = Some(Axis::new(of, axis.flipped));
        }
        let ink = ink(ui, rect, on, enabled, theme);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            letter,
            font.clone(),
            ink,
        );
    }
    let rect = Rect::from_min_size(
        pos2(row.min.x + NAME + 3.0 * (w + GAP) + GAP, row.min.y),
        vec2(w - GAP, ROW),
    );
    let on = axis.flipped;
    if cell(
        ui,
        rect,
        &format!("{name} {label} backwards"),
        on,
        true,
        theme,
    )
    .on_hover_text("Run it backwards")
    .clicked()
    {
        out = Some(Axis::new(axis.of, !axis.flipped));
    }
    let ink = ink(ui, rect, on, true, theme);
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, "\u{2212}", font, ink);
    out
}

/// The popover at `at` for `axes`. Its controls are named `{name} Front`, `{name} Top`,
/// `{name} Side`, `{name} Across X` … and `{name} Up backwards`.
pub fn popover(ui: &mut Ui, at: Pos2, name: &str, axes: Axes, theme: &Theme) -> Popped {
    let id = ui.id().with(("axes-popover", name));
    let mut picked = None;
    let shown = crate::ui::popup::Popup::new(id, at).show(ui.ctx(), theme, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (presets, _) = ui.allocate_exact_size(vec2(WIDTH, PRESET), Sense::hover());
        let w = (WIDTH - GAP * 2.0) / 3.0;
        let font = FontId::proportional(theme::font_size(theme::FONT_TINY, 1.0));
        for (i, (label, preset)) in [
            ("Front", Axes::FRONT),
            ("Top", Axes::TOP),
            ("Side", Axes::SIDE),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::from_min_size(
                presets.min + vec2(i as f32 * (w + GAP), 0.0),
                vec2(w, PRESET),
            );
            let on = axes == preset;
            if cell(ui, rect, &format!("{name} {label}"), on, true, theme).clicked() && !on {
                picked = Some(preset);
            }
            let ink = ink(ui, rect, on, true, theme);
            let glyph =
                Rect::from_center_size(pos2(rect.center().x, rect.min.y + 17.0), vec2(26.0, 26.0));
            let lit = if on || ui.rect_contains_pointer(rect) {
                theme.primary()
            } else {
                theme.text_muted()
            };
            cube(ui.painter(), glyph, Some(preset), lit, theme);
            ui.painter().text(
                pos2(rect.center().x, rect.max.y - 4.0),
                Align2::CENTER_BOTTOM,
                label,
                font.clone(),
                ink,
            );
        }
        ui.add_space(8.0);
        if let Some(across) = axis_row(ui, name, "Across", axes.across, axes.up.of, theme) {
            picked = Some(Axes { across, ..axes });
        }
        ui.add_space(GAP);
        if let Some(up) = axis_row(ui, name, "Up", axes.up, axes.across.of, theme) {
            picked = Some(Axes { up, ..axes });
        }
    });
    let escaped = ui.input(|i| i.key_pressed(eframe::egui::Key::Escape));
    Popped {
        picked,
        dismissed: shown.clicked_away || escaped,
    }
}
