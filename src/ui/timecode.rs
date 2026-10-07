// SPDX-License-Identifier: AGPL-3.0-or-later

//! The time readout: ambient time, the playhead, with the only two transport controls there
//! are — pause and reset to zero.
//!
//! It sits at the right end of the menu bar, immediately left of the frame-rate meter, and
//! where the menu bar is the operating system's, at the end of the tab row beside the meter.
//! View ▸ Time shows and hides it, a preference like the meter's, on by default since pause
//! lives here. `F8`, the key a Mac's keyboard prints ⏯ on, is pause's, read by `App` whatever is
//! showing; `Space` is not, since a stray one would stop the show.
//!
//! Like [`crate::ui::tabs::show`], this draws and returns; it never mutates. What it asks for
//! is a [`crate::transport::Command`] that `App::transport` sends: a hand on the instrument,
//! never an edit, and never saved.
//!
//! **Nothing on it moves.** The time is laid out for its widest reading, three digits of
//! minutes, so the playhead crossing ten or a hundred minutes shifts nothing beside it. See
//! [docs/ui.md](../../docs/ui.md#the-time-readout).

use crate::transport::{Command, Report};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    self, Align2, Color32, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, WidgetType, pos2,
    vec2,
};

/// Characters the time is laid out for: `000:00.00`.
const TIME_CHARS: usize = 9;
/// Horizontal padding either side of the time.
const PAD: f32 = 6.0;
/// Space between two pieces.
const GAP: f32 = 2.0;

/// The accessible names of the readout's three pieces.
pub const PAUSE: &str = "time.pause";
pub const RESET: &str = "time.reset";
pub const TIME: &str = "time.playhead";

/// A time as the readout shows it: `mm:ss.ff`, hundredths of a second, with a sign, the
/// minutes growing past 99 rather than wrapping into hours. A reading that rounds to zero
/// drops its sign.
pub fn clock(t: f64) -> String {
    if !t.is_finite() {
        return "--:--.--".to_string();
    }
    let hundredths = (t.abs() * 100.0).round() as u64;
    let sign = if t < 0.0 && hundredths > 0 { "-" } else { "" };
    let (minutes, rest) = (hundredths / 6000, hundredths % 6000);
    format!("{sign}{minutes:02}:{:02}.{:02}", rest / 100, rest % 100)
}

/// Draw the readout into a right-to-left layout — reset, then the time, then pause, so it
/// reads pause, time, reset from the left — and return what a hand asked for.
///
/// `enabled` is false while a render owns the playhead: the readout shows the render's time
/// and neither button does anything.
pub fn show(ui: &mut Ui, report: Report, enabled: bool, theme: &Theme) -> Option<Command> {
    let mut asked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        if !enabled {
            ui.disable();
        }
        let height = ui.spacing().interact_size.y;
        if reset(ui, height, theme).clicked() {
            asked = Some(Command::Seek(0.0));
        }
        time(ui, report.playhead, height, theme);
        let playing = report.playing;
        if pause(ui, playing, height, theme).clicked() {
            asked = Some(if playing {
                Command::Pause
            } else {
                Command::Play
            });
        }
    });
    asked
}

/// The width `chars` monospace characters take, and the padding either side.
fn text_width(ui: &Ui, chars: usize) -> f32 {
    let font = FontId::proportional(theme::FONT_BASE);
    let advance = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    advance * chars as f32 + PAD * 2.0
}

/// A piece's ground and hairline, the tab's own: sunken at rest, lit on a hover.
fn ground(ui: &Ui, rect: Rect, response: &Response, theme: &Theme) {
    let fill = if response.hovered() && ui.is_enabled() {
        theme.bg_secondary()
    } else {
        theme.bg_sunken()
    };
    let radius = egui::CornerRadius::same(theme::RADIUS_SM);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme.border_subtle()),
        StrokeKind::Inside,
    );
}

/// The ink a piece's text or icon is drawn in.
fn ink(ui: &Ui, theme: &Theme) -> Color32 {
    if ui.is_enabled() {
        theme.text_primary()
    } else {
        theme.text_disabled()
    }
}

/// A square button with an icon painted by `icon`, named for the tree.
fn square(
    ui: &mut Ui,
    name: &str,
    height: f32,
    theme: &Theme,
    icon: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(height, height), Sense::click());
    crate::ui::pointing(&response);
    crate::ui::accessible(&response, WidgetType::Button, name);
    ground(ui, rect, &response, theme);
    icon(ui.painter(), rect.shrink(height * 0.3), ink(ui, theme));
    response
}

/// Back to zero: a bar and a triangle pointing at it.
fn reset(ui: &mut Ui, height: f32, theme: &Theme) -> Response {
    square(ui, RESET, height, theme, |p, r, c| {
        p.rect_filled(
            Rect::from_min_max(r.min, pos2(r.min.x + 1.5, r.max.y)),
            0.0,
            c,
        );
        p.add(egui::Shape::convex_polygon(
            vec![
                pos2(r.max.x, r.min.y),
                pos2(r.max.x, r.max.y),
                pos2(r.min.x + 2.5, r.center().y),
            ],
            c,
            Stroke::NONE,
        ));
    })
    .on_hover_text("Back to zero: every gear starts its cycle again")
}

/// Pause, whose icon is a play glyph while paused: what a press does. The play glyph is in
/// the accent, since a show that is not moving is the state somebody needs to see from across
/// a room.
fn pause(ui: &mut Ui, playing: bool, height: f32, theme: &Theme) -> Response {
    let paused = (!playing && ui.is_enabled()).then(|| theme.accent());
    square(ui, PAUSE, height, theme, |p, r, c| {
        let c = paused.unwrap_or(c);
        if playing {
            let w = r.width() * 0.3;
            p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), 0.0, c);
            p.rect_filled(
                Rect::from_min_size(pos2(r.max.x - w, r.min.y), vec2(w, r.height())),
                0.0,
                c,
            );
        } else {
            p.add(egui::Shape::convex_polygon(
                vec![r.min, pos2(r.min.x, r.max.y), pos2(r.max.x, r.center().y)],
                c,
                Stroke::NONE,
            ));
        }
    })
    .on_hover_text(if playing { "Pause (F8)" } else { "Play (F8)" })
}

/// The playhead, at a fixed width.
fn time(ui: &mut Ui, playhead: f64, height: f32, theme: &Theme) {
    let width = text_width(ui, TIME_CHARS);
    let shown = clock(playhead);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    crate::ui::accessible(&response, WidgetType::Label, format_args!("{TIME} {shown}"));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        &shown,
        FontId::proportional(theme::FONT_BASE),
        ink(ui, theme),
    );
    response.on_hover_text("Ambient time: the playhead every node keeps time by");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hundredths of a second, minutes past 99 in three digits, a sign only where it shows.
    #[test]
    fn a_time_reads_as_minutes_seconds_and_hundredths() {
        assert_eq!(clock(0.0), "00:00.00");
        assert_eq!(clock(83.456), "01:23.46");
        assert_eq!(clock(5999.99), "99:59.99");
        assert_eq!(clock(6000.0), "100:00.00");
        assert_eq!(clock(-1.5), "-00:01.50");
        assert_eq!(clock(-0.001), "00:00.00");
        assert_eq!(clock(f64::NAN), "--:--.--");
    }
}
