// SPDX-License-Identifier: AGPL-3.0-or-later

//! A key drawn as a key: a rounded cap lifted over a darker base, so a key the hand presses
//! reads as one at a glance rather than as a word.
//!
//! **Only a key with a legend printed on it is a cap** — a letter, a digit, a symbol, a
//! modifier, Esc, Tab, Enter, an arrow, a function key. The space bar carries no legend, so
//! `Space` stays a word, and so does every gesture of the pointer's: *drag* and *click* are
//! not keys. docs/design-system.md, "A key is a cap", has the rest.

use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Color32, CornerRadius, FontId, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, vec2,
};

/// How far the face stands above its base: the darker band under the cap.
pub const LIFT: f32 = 2.0;
/// The legend's room either side, and above and below.
const PAD_X: f32 = 5.0;
const PAD_Y: f32 = 1.0;
/// A one-character cap is square-ish rather than a sliver.
const MIN_WIDTH: f32 = 18.0;

/// One cap with `legend` on it.
pub fn keycap(ui: &mut Ui, legend: &str, theme: &Theme) {
    let galley = ui.painter().layout_no_wrap(
        legend.to_owned(),
        FontId::proportional(theme::FONT_BASE),
        theme.text_primary(),
    );
    let size = vec2(
        (galley.size().x + 2.0 * PAD_X).max(MIN_WIDTH),
        galley.size().y + 2.0 * PAD_Y + LIFT,
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let outline = Stroke::new(1.0, theme.border_normal());
    let painter = ui.painter();
    // The base is the whole cap in the sunken tone; the face sits on it, short by the lift, so
    // the base shows only as the thicker, darker edge along the bottom.
    painter.rect(rect, radius, theme.bg_sunken(), outline, StrokeKind::Inside);
    let face = Rect::from_min_max(rect.min, rect.max - vec2(0.0, LIFT));
    painter.rect(
        face,
        radius,
        theme.bg_interactive(),
        outline,
        StrokeKind::Inside,
    );
    painter.galley(
        face.center() - galley.size() / 2.0,
        galley,
        theme.text_primary(),
    );
}

/// What sits between caps or after them, in the muted ink: a `+`, a `/`, a gesture's word.
pub fn word(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(
        eframe::egui::RichText::new(text)
            .font(FontId::proportional(theme::FONT_BASE))
            .color(color),
    );
}

/// A key's legend as its cap prints it, or `None` for a key with no legend on it.
pub fn legend(key: Key, said: impl FnOnce() -> String) -> Option<String> {
    Some(match key {
        Key::Space => return None,
        Key::ArrowUp => "↑".to_owned(),
        Key::ArrowDown => "↓".to_owned(),
        Key::ArrowLeft => "←".to_owned(),
        Key::ArrowRight => "→".to_owned(),
        Key::Escape => "Esc".to_owned(),
        _ => said(),
    })
}

/// The modifiers held, one cap's legend each, in the order the machine writes them: `⌃ ⌥ ⇧ ⌘`
/// on a Mac, `Ctrl Shift Alt` elsewhere.
pub fn modifier_legends(ui: &Ui, held: Modifiers) -> Vec<String> {
    let ctx = ui.ctx();
    let name = |one: Modifiers| ctx.format_modifiers(one);
    let mut legends = Vec::new();
    if ctx.os().is_mac() {
        if held.ctrl {
            legends.push(name(Modifiers::CTRL));
        }
        if held.alt {
            legends.push(name(Modifiers::ALT));
        }
        if held.shift {
            legends.push(name(Modifiers::SHIFT));
        }
        if held.command || held.mac_cmd {
            legends.push(name(Modifiers::MAC_CMD));
        }
    } else {
        if held.ctrl || held.command {
            legends.push(name(Modifiers::CTRL));
        }
        if held.shift {
            legends.push(name(Modifiers::SHIFT));
        }
        if held.alt {
            legends.push(name(Modifiers::ALT));
        }
    }
    legends
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The space bar and nothing else of the keys has no legend; the arrows and Escape print
    /// what their caps do.
    #[test]
    fn only_the_space_bar_has_no_legend() {
        assert_eq!(legend(Key::Space, || unreachable!()), None);
        assert_eq!(
            legend(Key::ArrowUp, || unreachable!()).as_deref(),
            Some("↑")
        );
        assert_eq!(
            legend(Key::Escape, || unreachable!()).as_deref(),
            Some("Esc")
        );
        assert_eq!(legend(Key::Z, || "Z".to_owned()).as_deref(), Some("Z"));
    }
}
