// SPDX-License-Identifier: AGPL-3.0-or-later

//! The two side panels' shared chrome: a header with a fold arrow, and the spine it folds to.
//!
//! The Main Input is on the left and the Main Mixer is on the right, and both of them are
//! things you want out of the way while you are working on the patch and back the moment you
//! are not. silvia folds both; this is the same gesture, drawn once so the two agree.
//!
//! Folded, a panel is a strip the width of [`SPINE`] with its name written up it, and the
//! whole strip is the button that unfolds it — so a folded panel is never a mystery and never
//! needs a menu item to find again.
//!
//! Open, the whole header bar is the button that folds it, which is the same gesture at the
//! same size: a strip to put away, a strip to bring back. The arrow says which way the panel
//! is going rather than being the one place the click lands.
//!
//! Inside, the two share a section heading, a 16:9 picture box and a labeled number, drawn
//! here once so a deck and the Main Input's picture are the same box.

use crate::graph::{ControlRange, NodeId};
use crate::render::Fit;
use crate::ui::Thumbnail;
use crate::ui::number::{self, NumberAction, NumberSpec};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2,
};

/// How wide a folded panel is. Room for the arrow, and for the name written up it.
pub const SPINE: f32 = 24.0;

/// How tall an open panel's header bar is, and the width of the arrow's own box at its outer
/// end.
const HEADER: f32 = 18.0;

/// Which edge a panel lives on, which is the only thing that differs between the two.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The arrow that folds the panel away: it points at the edge the panel goes to.
    fn fold(self) -> &'static str {
        match self {
            Self::Left => "‹",
            Self::Right => "›",
        }
    }

    /// The arrow that brings it back, which points the other way.
    fn unfold(self) -> &'static str {
        match self {
            Self::Left => "›",
            Self::Right => "‹",
        }
    }
}

/// A panel's header: its name, and the arrow that folds it. True when it was clicked.
///
/// The whole bar is the button, as the whole spine is the one that brings the panel back —
/// a target the size of the thing it acts on, in both directions. The arrow is painted on
/// the outer edge, the side the panel folds towards, so it points where the panel is going.
///
/// A real `Response` **with a name**, like the spine: a painted rectangle a hand can click
/// and the accessibility tree cannot see is a bug, and it is also what a test and the
/// agent-driven layer find it by.
pub fn header(ui: &mut Ui, title: &str, side: Side, theme: &Theme) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER), Sense::click());
    let name = format!("Hide {title}");
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, &name);
    let response = response.on_hover_text(format!("Hide {title}"));
    // The band lights under the pointer. Without it a bar that is entirely a button looks
    // like a heading that happens to have an arrow on it.
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme.bg_hover());
    }
    let gap = ui.spacing().item_spacing.x;
    let (arrow, name_at) = match side {
        Side::Left => (
            Pos2::new(rect.left() + HEADER * 0.5, rect.center().y),
            Pos2::new(rect.left() + HEADER + gap, rect.center().y),
        ),
        Side::Right => (
            Pos2::new(rect.right() - HEADER * 0.5, rect.center().y),
            Pos2::new(rect.left() + gap, rect.center().y),
        ),
    };
    ui.painter().text(
        arrow,
        Align2::CENTER_CENTER,
        side.fold(),
        FontId::monospace(theme::FONT_BASE),
        theme.text_secondary(),
    );
    // Clipped clear of the arrow, so dragging the panel narrow runs the name under its own
    // edge rather than over the arrow.
    let room = match side {
        Side::Left => rect.with_min_x(name_at.x),
        Side::Right => rect.with_max_x(rect.right() - HEADER),
    };
    ui.painter().with_clip_rect(room).text(
        name_at,
        Align2::LEFT_CENTER,
        title,
        FontId::monospace(theme::FONT_BASE),
        theme.text_primary(),
    );
    ui.separator();
    response.clicked()
}

/// A folded panel: its name up the spine, the whole strip clickable. True when clicked.
///
/// The strip is a real `Response` **with a name**, like every control here: a painted
/// rectangle a hand can click and the accessibility tree cannot see is a bug, and it is also
/// what a test and the agent-driven layer find it by.
pub fn spine(ui: &mut Ui, title: &str, side: Side, theme: &Theme) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ui.available_height().max(SPINE)),
        Sense::click(),
    );
    let name = format!("Show {title}");
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, &name);
    let response = response.on_hover_text(format!("Show {title}"));
    let ink = if response.hovered() {
        theme.text_primary()
    } else {
        theme.text_muted()
    };
    let galley =
        ui.painter()
            .layout_no_wrap(title.to_string(), FontId::monospace(theme::FONT_BASE), ink);
    // A quarter turn anticlockwise, so the name reads upwards — the way a book's spine does.
    // `TextShape`'s angle rotates about the position given, so the anchor is the text's
    // bottom-left once turned, which is the center of the strip plus half the line's length.
    let at = rect.center() + vec2(-theme::FONT_BASE * 0.5, galley.size().x * 0.5);
    ui.painter().add(Shape::Text(
        eframe::egui::epaint::TextShape::new(at, galley, ink)
            .with_angle(-std::f32::consts::FRAC_PI_2),
    ));
    // The arrow that brings it back, at the top of the strip: the same affordance the header
    // has, pointing the other way.
    ui.painter().text(
        rect.center_top() + vec2(0.0, 10.0),
        Align2::CENTER_CENTER,
        side.unfold(),
        FontId::monospace(theme::FONT_TINY),
        ink,
    );
    response.clicked()
}

/// A section's heading inside a panel: small, monospace, secondary.
pub fn heading(ui: &mut Ui, text: &str, theme: &Theme) {
    ui.label(
        eframe::egui::RichText::new(text)
            .font(FontId::monospace(theme::FONT_BASE))
            .color(theme.text_secondary()),
    );
}

/// A picture 16:9 across the panel: the screen's black ground and a border, and where
/// `shown` names a picture, the slot `App` fills with a paint callback blitting it.
///
/// The box is a fixed 16:9 and a source may be any shape, so the whole frame is shown with
/// bars rather than cropped to the box. With nothing to show it is the same black, so the
/// panel's shape never changes.
pub fn picture(
    ui: &mut Ui,
    shown: Option<(NodeId, Option<&'static str>)>,
    theme: &Theme,
) -> Option<Thumbnail> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS_SM, theme.screen_off());
    let thumbnail = shown.map(|(node, port)| Thumbnail {
        port,
        node,
        rect,
        slot: painter.add(Shape::Noop),
        fit: Fit::Letterbox,
        // The box draws its own square-cornered frame; nothing to round.
        corner: 0.0,
    });
    painter.rect_stroke(
        rect,
        theme::RADIUS_SM,
        Stroke::new(1.0, theme.border_subtle()),
        StrokeKind::Inside,
    );
    thumbnail
}

/// What a hand did to a panel's number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scrubbed {
    /// A value, from a drag, a typed entry, a reset or an `Escape`: a panel's control is not
    /// an edit, so the value a drag began at is simply a value.
    Set(f32),
    /// `Alt` + click: bind it to the next MIDI message.
    Learn,
}

/// A panel's own number in `rect`: the `s-number` every control gets, with no range of its
/// own to edit and no step to change. `learning` is whether it is waiting for a MIDI
/// message, which draws the learning ring.
#[allow(clippy::too_many_arguments)]
pub fn number(
    ui: &mut Ui,
    rect: Rect,
    name: &str,
    value: f32,
    default: f32,
    range: ControlRange,
    theme: &Theme,
    lock_cursor: bool,
    learning: bool,
) -> Option<Scrubbed> {
    ghosted(
        ui,
        rect,
        name,
        (value, default),
        range,
        theme,
        lock_cursor,
        learning,
        None,
    )
}

/// [`number`], wearing the ghost mark where a bound fader is out of soft takeover's pick-up:
/// the Main Mixer's fade.
#[allow(clippy::too_many_arguments)]
pub fn ghosted(
    ui: &mut Ui,
    rect: Rect,
    name: &str,
    (value, default): (f32, f32),
    range: ControlRange,
    theme: &Theme,
    lock_cursor: bool,
    learning: bool,
    ghost: Option<f32>,
) -> Option<Scrubbed> {
    let spec = NumberSpec {
        value,
        default,
        range,
        declared: range,
        unit: "",
        log: false,
        varying: false,
        learning,
        ladder: false,
        ghost,
    };
    match number::scrub(ui, rect, name, &spec, theme, true, lock_cursor, 1.0)? {
        NumberAction::Set(v) | NumberAction::Cancel(v) => Some(Scrubbed::Set(v)),
        NumberAction::ResetAll => Some(Scrubbed::Set(default)),
        NumberAction::Learn => Some(Scrubbed::Learn),
        NumberAction::SetStep(_) | NumberAction::OpenRange => None,
    }
}
