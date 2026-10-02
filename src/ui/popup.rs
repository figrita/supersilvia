// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one popup: an `Area` in the foreground at a fixed point, a `Frame::popup` in the
//! theme's popup ground and hairline, and the click-away rule every popup dismisses by. The
//! color picker, the range editor, the select, asset and `!` lists, the browser, the
//! conversion menu and the Nodes menu are all drawn in it.
//!
//! **Not movable.** An `Area` is movable by default, and a movable one senses a *drag* rather
//! than a click, so a click on the popup's own padding never registered as hitting it and fell
//! through to whatever the canvas had underneath. A popup anchored to the control that opened
//! it has no business being dragged off it either.
//!
//! **`contains_pointer`, not `hovered`.** egui gives `hovered` to the topmost widget under the
//! pointer, so a click on a field, a row or a default *inside* a popup leaves the popup's own
//! response unhovered, and a rule written on `hovered` reads that click as one made outside
//! it — the range editor closed on every click into it by exactly that.

use crate::ui::theme::Theme;
use eframe::egui::{
    Area, Color32, Context, Frame, Id, Margin, Order, Pos2, Response, Stroke, Style, Ui,
};

/// A popup's `Area`: foreground, at `at`, and not movable.
pub fn area(id: Id, at: Pos2) -> Area {
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(at)
        .movable(false)
}

/// A popup's frame: `Frame::popup`, on `bg_secondary` with a one-point hairline in `edge`.
pub fn frame(style: &Style, theme: &Theme, edge: fn(&Theme) -> Color32) -> Frame {
    Frame::popup(style)
        .fill(theme.bg_secondary())
        .stroke(Stroke::new(1.0, edge(theme)))
}

/// Whether this frame's click landed outside `response`, the popup's own area.
pub fn clicked_away(ctx: &Context, response: &Response) -> bool {
    ctx.input(|i| i.pointer.any_click()) && !response.contains_pointer()
}

/// One popup, described before it is shown.
pub struct Popup {
    id: Id,
    at: Pos2,
    edge: fn(&Theme) -> Color32,
    margin: Option<Margin>,
    radius: Option<u8>,
}

/// What a popup's contents returned, and whether a click landed outside it.
pub struct Shown<R> {
    pub inner: R,
    pub clicked_away: bool,
}

impl Popup {
    /// A popup at `at`, edged in `border_normal`, with `Frame::popup`'s margin and radius.
    pub fn new(id: Id, at: Pos2) -> Self {
        Self {
            id,
            at,
            edge: Theme::border_normal,
            margin: None,
            radius: None,
        }
    }

    /// The hairline's token.
    #[must_use]
    pub fn edge(self, edge: fn(&Theme) -> Color32) -> Self {
        Self { edge, ..self }
    }

    #[must_use]
    pub fn margin(self, margin: impl Into<Margin>) -> Self {
        Self {
            margin: Some(margin.into()),
            ..self
        }
    }

    #[must_use]
    pub fn radius(self, radius: u8) -> Self {
        Self {
            radius: Some(radius),
            ..self
        }
    }

    pub fn show<R>(self, ctx: &Context, theme: &Theme, add: impl FnOnce(&mut Ui) -> R) -> Shown<R> {
        let shown = area(self.id, self.at).show(ctx, |ui| {
            let mut frame = frame(ui.style(), theme, self.edge);
            if let Some(margin) = self.margin {
                frame = frame.inner_margin(margin);
            }
            if let Some(radius) = self.radius {
                frame = frame.corner_radius(radius);
            }
            frame.show(ui, add).inner
        });
        Shown {
            clicked_away: clicked_away(ctx, &shown.response),
            inner: shown.inner,
        }
    }
}
