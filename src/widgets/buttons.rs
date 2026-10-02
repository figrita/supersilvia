// SPDX-License-Identifier: AGPL-3.0-or-later

//! A row of buttons that write their node's own settings: `lyapunov`'s Random Seq and
//! `slimemold`'s nine presets.
//!
//! silvia's own are a `downCallback` that writes `this.values` and calls
//! `refreshDownstreamOutputs`, and a `presetBar` of nine `<button>`s doing the same. Here a
//! press is an edit — one `SetSettings` through the command bus, so it is one undo step, it is
//! saved, and a `Code` option among what it writes rebuilds the shader the way a pick from
//! the option's own row does. What the button writes is the node's
//! ([`crate::nodes::Buttons::press`]); this is only where it is drawn and when it is asked.
//!
//! Drawn in the press button's own chrome, [`press::click`], across the region at Reframe
//! Range's insets, so one button is as wide as an action row and nine share the width the way
//! silvia's preset bar shares it.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::nodes::Region;
use crate::ui::{canvas, press};
use eframe::egui::{Rect, pos2, vec2};

/// The row: the buttons and the air around them, with the body's own pad at the foot.
pub const BUTTONS: RegionDef = RegionDef {
    size,
    show,
    // A drag that starts between two buttons is a missed press, not a move of the node.
    claims_pointer: true,
    ..RegionDef::EMPTY
};

/// Air above the row, between it and the last row of the node.
const ABOVE: f32 = 4.0;
/// How far the row sits in from the body's edge, and how far apart two buttons are: Reframe
/// Range's named ranges', so the two rows of presets in the library read as one family.
const INSET: f32 = 8.0;
const GAP: f32 = 3.0;

fn size(_node: &Node) -> f32 {
    ABOVE + press::SIZE + canvas::BODY_PAD
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Region::Buttons(row) = r.region else {
        return Vec::new();
    };
    let z = r.zoom;
    let count = row.buttons.len().max(1) as f32;
    let span = r.rect.width() - 2.0 * INSET * z;
    let pitch = (span + GAP * z) / count;
    let top = r.rect.top() + ABOVE * z;
    let mut out = Vec::new();
    for (i, (key, caption)) in row.buttons.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(r.rect.left() + INSET * z + pitch * i as f32, top),
            vec2((pitch - GAP * z).max(0.0), press::SIZE * z),
        );
        let name = r.name(key);
        if press::click(r.ui, rect, caption, &name, r.theme, z, None) {
            out.push(RegionEvent::Settings((row.press)(i, seed(r))));
            if let Some(pulse) = row.pulse {
                out.push(RegionEvent::Held(pulse));
            }
        }
    }
    out
}

/// A fresh number for a press that rolls dice: the moment of the press, mixed with the node's
/// id so two nodes pressed in one frame roll differently. The roll's outcome travels in the
/// command, so redo replays what was rolled rather than rolling again.
fn seed(r: &RegionUi<'_>) -> u32 {
    let time = r.ui.input(|i| i.time).to_bits();
    ((time ^ (time >> 32)) as u32) ^ r.id.0.wrapping_mul(0x9e37_79b9)
}
