// SPDX-License-Identifier: AGPL-3.0-or-later

//! The conversion menu: what a cable that cannot land can be carried by.
//!
//! silvia's `js/typeConversions.js` is the behavior — a dotted ring on a port a conversion
//! could reach, a menu on release, the converter inserted midway and wired both sides. The
//! rows are its table too, curated rather than derived: `nodes::bridge`. The one thing that
//! differs is that choosing a row is a single `Command::Bridge`, so the node and its cables
//! are one undo step.
//!
//! The popup is the node browser's shape without its search field: an `Area` of rows, the
//! keyboard on the arrows and `Enter`, `Escape` or a click elsewhere to dismiss. See
//! docs/ui.md, "The conversion menu".

use crate::graph::{Graph, PortRef, PortType};
use crate::nodes::{self, Bridge};
use crate::ui::popup;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align, Align2, FontId, Id, Key, Modifiers, Pos2, Rect, ScrollArea, Sense, Ui, vec2,
};

/// Panel width. Wide enough for a node's label with the port that distinguishes it.
pub const WIDTH: f32 = 260.0;
/// How much list is shown before it scrolls.
const LIST_HEIGHT: f32 = 300.0;
/// One row, the browser's.
const ROW_HEIGHT: f32 = 24.0;
/// Space between the target port and the panel's near edge, in screen points.
const OFFSET: f32 = 12.0;

/// A released cable waiting for a conversion to be chosen. `None` on `CanvasState` is no menu.
#[derive(Debug, Clone)]
pub struct Bridging {
    /// The output the cable came from.
    pub from: PortRef,
    /// The input it was released on, which cannot take it directly.
    pub to: PortRef,
    /// Where the panel hangs, in **world** units: a screen position captured on release
    /// detaches from the port the moment the canvas pans or zooms under it.
    at: Pos2,
    /// Index into the rows as they are *now*, which is what the arrows move.
    selected: usize,
    /// The frame it opened on. The release that opened it is not the click that dismisses it.
    fresh: bool,
    /// The selection moved by key, so the list scrolls to it.
    follow: bool,
}

impl Bridging {
    /// Where the panel hangs, in world units: the port the cable was released on.
    pub fn at(&self) -> Pos2 {
        self.at
    }

    pub fn new(from: PortRef, to: PortRef, at: Pos2) -> Self {
        Self {
            from,
            to,
            at,
            selected: 0,
            fresh: true,
            follow: false,
        }
    }
}

/// What the menu asked for this frame.
pub struct Outcome {
    /// The casting to insert, which carries the node and the ports its cables land on.
    pub chosen: Option<&'static Bridge>,
    /// Escape, a click outside it, or a port that is no longer there.
    pub dismissed: bool,
}

/// The castings a pair of ports offers: the table's row for their two types, or nothing where
/// either port has gone while the menu was open.
pub fn rows(graph: &Graph, from: PortRef, to: PortRef) -> &'static [Bridge] {
    match pair(graph, from, to) {
        Some((a, b)) => nodes::bridges(a, b),
        None => &[],
    }
}

/// The two port types a bridge would join, or `None` where either port has gone.
fn pair(graph: &Graph, from: PortRef, to: PortRef) -> Option<(PortType, PortType)> {
    let a = graph.get(from.node)?.output(from.key)?.ty;
    let b = graph.get(to.node)?.input(to.key)?.ty;
    Some((a, b))
}

/// Draw the menu and return what it asked for. `within` is the canvas, which the panel is
/// kept inside; `at` is the target port's own position on screen.
pub fn show(
    ui: &mut Ui,
    bridging: &mut Bridging,
    graph: &Graph,
    within: Rect,
    at: Pos2,
    theme: &Theme,
) -> Outcome {
    let hits = rows(graph, bridging.from, bridging.to);
    let mut out = Outcome {
        chosen: None,
        dismissed: hits.is_empty(),
    };
    if hits.is_empty() {
        return out;
    }
    bridging.selected = bridging.selected.min(hits.len() - 1);

    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            out.dismissed = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            bridging.selected = (bridging.selected + 1).min(hits.len() - 1);
            bridging.follow = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            bridging.selected = bridging.selected.saturating_sub(1);
            bridging.follow = true;
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) {
            out.chosen = hits.get(bridging.selected);
        }
    });

    // Beside the port, and kept inside the canvas: a panel opened near the right edge or the
    // bottom is a list you cannot read.
    let height = (LIST_HEIGHT + HEADER_HEIGHT).min(within.height() - 24.0);
    let anchor = Pos2::new(at.x + OFFSET, at.y - ROW_HEIGHT);
    let at = Pos2::new(
        anchor.x.min(within.max.x - WIDTH).max(within.min.x),
        anchor.y.min(within.max.y - height).max(within.min.y),
    );

    let shown = popup::Popup::new(Id::new("conversion-menu"), at)
        .edge(Theme::border_strong)
        .show(ui.ctx(), theme, |ui| {
            ui.set_width(WIDTH);
            // An `Area` hands its content last frame's size as the room it has, so
            // the room is stated every frame rather than measured — the browser's own
            // trap, and the same answer.
            ui.set_max_height(height);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                eframe::egui::RichText::new("Convert")
                    .monospace()
                    .size(theme::FONT_TINY)
                    .color(theme.text_secondary()),
            );
            ScrollArea::vertical()
                .max_height(height - HEADER_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, bridge) in hits.iter().enumerate() {
                        let hit = row(
                            ui,
                            bridge,
                            index == bridging.selected,
                            bridging.follow,
                            theme,
                        );
                        if hit {
                            out.chosen = Some(bridge);
                        }
                    }
                });
        });

    // Clicked away, as every popup is. Not on the frame it opened: that release is the one
    // that asked for it.
    if !bridging.fresh && shown.clicked_away {
        out.dismissed = true;
    }
    bridging.fresh = false;
    bridging.follow = false;
    out
}

/// The `Convert` heading's own height, which the list is shorter by.
const HEADER_HEIGHT: f32 = 18.0;

/// One row: the casting's icon and its name. True when it was clicked.
fn row(ui: &mut Ui, bridge: &Bridge, selected: bool, follow: bool, theme: &Theme) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    if selected && follow {
        ui.scroll_to_rect(rect, Some(Align::Center));
    }
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            eframe::egui::CornerRadius::same(theme::RADIUS_SM),
            if selected {
                theme.primary().gamma_multiply(0.25)
            } else {
                theme.bg_hover()
            },
        );
    }
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        bridge.icon,
        theme::icon_font(theme::FONT_BASE, 1.0),
        theme.text_primary(),
    );
    painter.text(
        rect.left_center() + vec2(28.0, 0.0),
        Align2::LEFT_CENTER,
        bridge.label,
        FontId::monospace(theme::FONT_BASE),
        theme.text_primary(),
    );
    // Named for the ports it would wire, so the tree an agent and a test read says which
    // conversion a row is rather than where it sits — and so an `rgba`'s four rows, which
    // share an output, are four different names.
    let name = format!(
        "convert with {}.{} to {}",
        bridge.def.slug,
        bridge.inputs.join("+"),
        bridge.output
    );
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, &name);
    response.clone().on_hover_text(bridge.def.tooltip);
    response.clicked()
}
