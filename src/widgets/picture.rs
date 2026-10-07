// SPDX-License-Identifier: AGPL-3.0-or-later

//! The two regions that are a picture: an Output's own render, and a source showing what it
//! publishes.
//!
//! Neither draws the picture — `ui/` has no GPU. Each paints the black ground it reads
//! as before the first frame and reserves a slot in the node's own paint order, which is what
//! keeps the blit inside the node rather than over every node drawn after it. The marks and
//! the player's strip over a picture are the node body's, on every picture alike.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::Picture;
use crate::ui::canvas;
use eframe::egui::CornerRadius;

/// An Output's own render, flush with the body's bottom corners.
///
/// **No heading.** An Output's render is the node, and a triangle that closed it would turn an
/// Output into a header with ports.
pub const RENDER: RegionDef = RegionDef {
    size: render_size,
    show: render_show,
    width: Some(canvas::OUTPUT_NODE_WIDTH),
    ..RegionDef::EMPTY
};

/// The frame's own aspect, not a fixed 16:9: silvia's `.output-canvas` is the body's full
/// width with no letterboxing, so the band is exactly as tall as the width divided by whatever
/// the `resolution` option currently names — recomputed every frame rather than cached, so
/// picking a taller resolution reshapes the node on the spot instead of leaving black bars.
fn render_size(node: &Node) -> f32 {
    let (w, h) = crate::nodes::output::resolution_of(node);
    canvas::node_width(node) * h as f32 / w as f32
}

fn render_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    // The slot is sized to the Output's own resolution, so the two already agree — `Cover` is
    // what keeps a rounding error at an odd zoom a cropped hairline of the picture rather than
    // a hairline of the ground showing through.
    vec![ground(r, Picture::Render, crate::render::Fit::Cover)]
}

/// A source's own picture, behind a heading a hand closes. Which port it shows is the
/// region's name's — `Region::Preview` — so one widget draws every one of them.
pub const PREVIEW: RegionDef = RegionDef {
    size: preview_size,
    show: preview_show,
    ..RegionDef::EMPTY
};

/// A fixed 16:9 box of the body's width, where an Output's is its `resolution` option's aspect
/// exactly: a clip's shape is whatever was imported and is not known until a frame has been
/// decoded, long after the node has been laid out. The blit letterboxes into it, which is what
/// the mixer panel's channel preview already does with a deck that is not 16:9.
fn preview_size(node: &Node) -> f32 {
    canvas::node_width(node) * 9.0 / 16.0
}

fn preview_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(picture) = r.region.picture() else {
        return Vec::new();
    };
    // A clip is whatever shape it was imported at and the band is a fixed 16:9, so the picture
    // is fitted inside it rather than cropped to fill it.
    let slot = ground(r, picture, crate::render::Fit::Letterbox);
    working(r);
    vec![slot]
}

/// What the node says it is doing, across the middle of its own picture.
///
/// A source that is not showing a picture yet is showing a black band, and the black band is
/// where the eye already is — so the line saying *why* belongs there and not on a status row
/// under the rows, or behind a file button at the other end of the node. It is the node's own
/// [`CpuNode::status`](crate::nodes::CpuNode::status), so nothing here knows whether it is a
/// clip being prepared or a GIF being counted.
///
/// Drawn **after** the picture's slot is reserved, so it is legible whatever lands behind it;
/// in practice a node with something to say has published nothing but black. Read-only, like
/// the status line region: no interact, so a hand still carries the node by its picture.
fn working(r: &mut RegionUi<'_>) {
    let Some(text) = r.live.status else {
        return;
    };
    if r.rect.height() < 8.0 {
        return;
    }
    let font = eframe::egui::FontId::proportional(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        r.zoom,
    ));
    r.ui.painter().text(
        r.rect.center(),
        eframe::egui::Align2::CENTER_CENTER,
        text,
        font,
        r.theme.text_secondary(),
    );
    // Named as well as painted: what a node says about itself has to be readable by a test
    // and by the agent-driven layer, exactly as the status line region's is.
    let name = r.name("status");
    let label = format!("{name} {text}");
    let w = r.ui.interact(
        r.rect,
        r.ui.id().with(("preview-status", &name)),
        eframe::egui::Sense::hover(),
    );
    crate::ui::accessible(&w, eframe::egui::WidgetType::Label, &label);
}

/// The black ground and the slot over it: what every picture on a node shares.
///
/// Rounded only where this band is the foot of the body — `corner` is zero above another
/// region, since there is no arc there for the picture to stay inside.
fn ground(r: &mut RegionUi<'_>, picture: Picture, fit: crate::render::Fit) -> RegionEvent {
    let radius = r.corner as u8;
    r.ui.painter().rect_filled(
        r.rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: radius,
            se: radius,
        },
        r.theme.screen_off(),
    );
    RegionEvent::Picture(crate::ui::Thumbnail {
        node: r.id,
        port: picture.port(),
        rect: r.rect,
        slot: r.ui.painter().add(eframe::egui::Shape::Noop),
        fit,
        corner: r.corner,
    })
}
