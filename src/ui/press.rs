// SPDX-License-Identifier: AGPL-3.0-or-later

//! The button an action input falls back to when nothing is connected: the whole row, as
//! silvia's is.
//!
//! It is drawn in the action port's own color, spanning the row from the port's label to
//! the row's right edge, with the caption centered the way silvia centers its button text —
//! the target is the row, not a small square hanging off the end of it with a hand's width
//! of margin before it. The action port itself, drawn in the port's shape at the row's left
//! edge by `node_widget::port`, is what still says which port a cable lands on; this is the
//! control that fires it.
//!
//! It reports a **level**, not a click. An action is a gate, so what `tick` needs to know is
//! whether a finger is on it this frame; down and up are the app's to derive.

use crate::ui::theme::{self, Theme};
use eframe::egui::{Align2, FontId, Rect, Sense, Ui, WidgetType};

/// The glyph's box, at the scale of a stepper — what `SIZE` used to be the whole button.
pub const SIZE: f32 = 18.0;

/// What a press button reported this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Nothing: no finger on it.
    Idle,
    /// A finger is on it this frame. An action is a gate, so what `tick` needs is the level.
    Held,
    /// `Alt` + click: bind this action input to the next MIDI message. It is **not** also
    /// held — binding a button that fired on the way would cut to a deck while teaching a
    /// note to cut to it.
    Learn,
}

/// Draw the button and say what it reported.
///
/// `caption` is the port's label, painted inside the button the way silvia paints it as the
/// button's own text; `name` is the accessible name — `{slug}{id}.{key}` — kept exactly as
/// it was when the button was a small square, so a test or an agent script still finds
/// `output1.show_a` by that name. While `learning`, the name ends in [`crate::ui::LEARNING`].
// Every argument is used; a struct would only move the list to the single call site.
#[allow(clippy::too_many_arguments)]
pub fn button(
    ui: &mut Ui,
    rect: Rect,
    caption: &str,
    name: impl crate::ui::Name,
    theme: &Theme,
    zoom: f32,
    // How brightly this input is throbbing because something fired it: 1 on the frame it
    // fired and decaying to 0. A sequencer lane, a MIDI note and a finger all arrive here,
    // which is the point — the button says the action happened however it was asked for.
    fire: f32,
    // This input is waiting for a MIDI message: the button wears `ui::learning_ring`.
    learning: bool,
) -> Press {
    let response = ui.interact(
        rect,
        ui.id().with(("press", &name)),
        Sense::click_and_drag(),
    );
    // `Alt` + click learns a MIDI binding instead of pressing.
    let learn =
        ui.ctx().input(|i| i.modifiers.alt) && (response.clicked() || response.drag_started());
    // Held: the pointer is down on this button, **or** a whole click happened inside this
    // frame. `is_pointer_button_down_on` alone loses a fast click entirely — press and
    // release inside one 16 ms frame leave it false at the end of the frame, the level never
    // rises, and the gate never opens. Reporting the click as one frame of hold turns it into
    // a down on the next tick and an up on the one after, which is what a tap is.
    let held = !learn && (response.is_pointer_button_down_on() || response.clicked());

    let action = theme.port(crate::graph::PortType::Action);
    let painter = ui.painter();
    // **The throb is the press this button would have shown.** A firing that arrived down a
    // cable or off a controller is the same event a finger sends, so it wears the same
    // chrome — the whole button in the action color — faded in over a held one rather than
    // drawn as a second mark beside it. Held is still the exact press, so a finger on the
    // button looks as it always did.
    let glow = if held { 0.0 } else { fire.clamp(0.0, 1.0) };
    let fill = if held {
        action
    } else if response.hovered() {
        theme.bg_hover()
    } else {
        theme.bg_interactive()
    };
    let fill = fill.lerp_to_gamma(action, glow);
    let border = if held {
        action
    } else {
        theme.border_normal().lerp_to_gamma(action, glow)
    };
    crate::ui::field(painter, rect, fill, None, border);
    if learning {
        crate::ui::learning_ring(ui, rect, zoom, theme);
    }
    let text = if held {
        theme.bg_primary()
    } else {
        theme
            .text_secondary()
            .lerp_to_gamma(theme.bg_primary(), glow)
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::monospace(theme::font_size(theme::FONT_TINY, zoom)),
        text,
    );

    if learning {
        crate::ui::accessible(
            &response,
            WidgetType::Button,
            format_args!("{name} {}", crate::ui::LEARNING),
        );
    } else {
        crate::ui::accessible(&response, WidgetType::Button, &name);
    }
    match (learn, held) {
        (true, _) => Press::Learn,
        (false, true) => Press::Held,
        (false, false) => Press::Idle,
    }
}

/// A button that reports a click rather than a level, for a thing done once — starting a
/// render, canceling it — drawn in the press button's chrome so the two read as one family.
/// `progress`, 0 to 1, fills the button from the left in the action color behind the
/// caption: the button *is* the progress bar, which is where silvia's offline output puts
/// its own, under the same buttons.
pub fn click(
    ui: &mut Ui,
    rect: Rect,
    caption: &str,
    name: impl crate::ui::Name,
    theme: &Theme,
    zoom: f32,
    progress: Option<f32>,
) -> bool {
    let response = ui.interact(rect, ui.id().with(("click", &name)), Sense::click());
    let action = theme.port(crate::graph::PortType::Action);
    let painter = ui.painter();
    let fill = if response.hovered() {
        theme.bg_hover()
    } else {
        theme.bg_interactive()
    };
    crate::ui::field(
        painter,
        rect,
        fill,
        progress.map(|p| (p, action)),
        theme.border_normal(),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::monospace(theme::font_size(theme::FONT_TINY, zoom)),
        theme.text_primary(),
    );
    crate::ui::accessible(&response, WidgetType::Button, &name);
    response.clicked()
}

/// How far a choice's caption sits in from each side of its segment, in world units.
const CHOICE_PAD: f32 = 6.0;

/// How wide a [`choice`] over these captions is drawn: each segment its own caption and the
/// air either side, so a long word beside a short one is read whole.
pub fn choice_width(ctx: &eframe::egui::Context, captions: &[&str], zoom: f32) -> f32 {
    let advance = crate::ui::node_widget::advance(
        ctx,
        &FontId::monospace(theme::font_size(theme::FONT_TINY, zoom)),
    );
    captions
        .iter()
        .map(|c| c.chars().count() as f32 * advance + 2.0 * CHOICE_PAD * zoom)
        .sum()
}

/// One of an option's values, picked from all of them side by side: the field the click
/// button wears, cut into a segment per caption, the chosen one raised and the others a click
/// away. What a yes-or-no is drawn as where each answer has a name of its own — Opaque and
/// Transparent — rather than a tick that names only one of them.
///
/// Returns the segment clicked when it is not the chosen one: a click is an edit, and
/// choosing what is already chosen is none. Each segment is named `{name} {caption}` and says
/// whether it is the one chosen.
pub fn choice(
    ui: &mut Ui,
    rect: Rect,
    captions: &[&str],
    chosen: usize,
    name: impl crate::ui::Name,
    theme: &Theme,
    zoom: f32,
) -> Option<usize> {
    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
    let advance = crate::ui::node_widget::advance(ui.ctx(), &font);
    let natural: Vec<f32> = captions
        .iter()
        .map(|c| c.chars().count() as f32 * advance + 2.0 * CHOICE_PAD * zoom)
        .collect();
    // What is left over, or short, is shared evenly, so the segments fill the rect exactly.
    let spare = (rect.width() - natural.iter().sum::<f32>()) / captions.len().max(1) as f32;
    crate::ui::field(
        ui.painter(),
        rect,
        theme.bg_interactive(),
        None,
        theme.border_normal(),
    );
    let radius = eframe::egui::CornerRadius::same(theme::RADIUS_SM);
    let mut picked = None;
    let mut x = rect.min.x;
    for (i, (caption, width)) in captions.iter().zip(natural).enumerate() {
        let segment = Rect::from_min_max(
            eframe::egui::pos2(x, rect.min.y),
            eframe::egui::pos2(x + width + spare, rect.max.y),
        );
        x = segment.max.x;
        let on = i == chosen;
        let response = ui.interact(segment, ui.id().with(("choice", &name, i)), Sense::click());
        let painter = ui.painter();
        if on {
            painter.rect(
                segment,
                radius,
                theme.bg_active(),
                eframe::egui::Stroke::new(1.0, theme.border_strong()),
                eframe::egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment.shrink(1.0), radius, theme.bg_hover());
        }
        painter.text(
            segment.center(),
            Align2::CENTER_CENTER,
            caption,
            font.clone(),
            if on {
                theme.text_primary()
            } else {
                theme.text_muted()
            },
        );
        response.widget_info(|| {
            eframe::egui::WidgetInfo::selected(
                WidgetType::RadioButton,
                true,
                on,
                format!("{name} {caption}"),
            )
        });
        if response.clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}
