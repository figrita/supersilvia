// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reframe Range's named ranges: the common conversions as two rows of buttons.
//!
//! silvia has no such rows. It has a second node — `sliderule`, ported here beside
//! `reframerange` — where the bounds are picked as two names out of five fixed bases, and the
//! conversions a patch asks for over and over are two picks there where they are four typed
//! numbers on the general node. What that speed costs in silvia is a node whose bounds
//! nothing can drive.
//!
//! This is the speed without the cost: a press **writes the two knobs and lets go**. The
//! bounds stay knobs with ports, so the window is still drivable, and a hand that then scrubs
//! one of them is not fighting a mode — there is no stored pick to contradict, because a
//! preset here is a gesture and not a state.
//!
//! The five bases are `sliderule`'s five, by the same keys, so a name cannot come to mean two
//! things in two places. **Swap** exchanges Output Min and Output Max, which is what Slide
//! Rule's pair of Invert boxes did: an inside-out output range is how this node runs a map
//! backwards, and the tooltip already says so.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{ControlValue, Node};
use crate::nodes::reframerange::{IN_MAX, IN_MIN, OUT_MAX, OUT_MIN};
use crate::ui::{canvas, press, theme};
use eframe::egui::{Align2, FontId, Rect, pos2, vec2};

/// The two rows of presets and the swap, behind a heading a hand closes. The four bounds it
/// writes are the node's, named once in `nodes::reframerange` and read from there, so a
/// rename cannot leave the region writing a control that is not there.
pub const RANGES: RegionDef = RegionDef {
    size,
    show,
    claims_pointer: true,
    ..RegionDef::EMPTY
};

/// silvia's five bases, by `sliderule`'s own keys. The caption is what fits a fifth of the
/// body at the tiny monospace the rest of a node's chrome is set in; the name the key spells
/// out is what the row above it says.
const BASES: &[(&str, &str, f32, f32)] = &[
    ("unit", "0–1", 0.0, 1.0),
    ("degrees", "0–360", 0.0, 360.0),
    ("signed", "±1", -1.0, 1.0),
    ("byte", "0–255", 0.0, 255.0),
    ("turns", "0–2π", 0.0, std::f32::consts::TAU),
];

/// One caption line, one row of buttons, and the air between them, in world units. A button
/// is the height of a number control's own stepper, so the rows read as part of the body
/// rather than as a panel dropped into it.
const CAPTION: f32 = 12.0;
const BUTTON: f32 = 16.0;
const GAP: f32 = 4.0;
/// How far the rows sit in from the body's edge — an option row's own label inset — and how
/// far apart two buttons in a row are.
const INSET: f32 = 8.0;
const BUTTON_GAP: f32 = 3.0;

/// Two captioned rows, the swap under them, and the body's own pad below it, which is the
/// air any other node has at its foot.
fn size(_node: &Node) -> f32 {
    2.0 * (CAPTION + BUTTON + GAP) + BUTTON + canvas::BODY_PAD
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let z = r.zoom;
    let mut y = r.rect.top();
    let mut out = Vec::new();

    for (caption, min_key, max_key) in [
        ("Input Range", IN_MIN, IN_MAX),
        ("Output Range", OUT_MIN, OUT_MAX),
    ] {
        label(r, y, caption);
        y += CAPTION * z;
        if let Some(picked) = row(r, y, min_key, max_key) {
            out.push(picked);
        }
        y += (BUTTON + GAP) * z;
    }

    // Swap, across the width the five buttons take together.
    let rect = Rect::from_min_size(
        pos2(r.rect.left() + INSET * z, y),
        vec2(r.rect.width() - 2.0 * INSET * z, BUTTON * z),
    );
    let name = r.name("swap");
    if press::click(r.ui, rect, "Swap Output Min/Max", &name, r.theme, z, None) {
        out.push(RegionEvent::Controls(vec![
            write(r.node, OUT_MIN, value(r.node, OUT_MAX)),
            write(r.node, OUT_MAX, value(r.node, OUT_MIN)),
        ]));
    }
    out
}

/// The tiny label over a row of buttons, at an option row's own inset.
fn label(r: &mut RegionUi<'_>, top: f32, text: &str) {
    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, r.zoom));
    r.ui.painter().text(
        pos2(r.rect.left() + INSET * r.zoom, top + CAPTION * r.zoom * 0.5),
        Align2::LEFT_CENTER,
        text,
        font,
        r.theme.text_secondary(),
    );
}

/// One row of five presets, and the pair of writes the pressed one asks for.
fn row(
    r: &mut RegionUi<'_>,
    top: f32,
    min_key: &'static str,
    max_key: &'static str,
) -> Option<RegionEvent> {
    let z = r.zoom;
    let span = r.rect.width() - 2.0 * INSET * z;
    let pitch = span / BASES.len() as f32;
    let mut picked = None;
    for (i, (key, caption, min, max)) in BASES.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(r.rect.left() + INSET * z + pitch * i as f32, top),
            vec2(pitch - BUTTON_GAP * z, BUTTON * z),
        );
        let name = r.name(&format!("{min_key}.{key}"));
        if press::click(r.ui, rect, caption, &name, r.theme, z, None) {
            picked = Some(RegionEvent::Controls(vec![
                write(r.node, min_key, *min),
                write(r.node, max_key, *max),
            ]));
        }
    }
    picked
}

/// One control's value on this node, or its definition's default where nothing has been
/// written yet.
fn value(node: &Node, key: &str) -> f32 {
    if let Some(ControlValue::Float(v)) = node.controls.get(key) {
        return *v;
    }
    match node.def.input(key).map(|p| &p.control) {
        Some(crate::nodes::Control::Number { default, .. }) => *default,
        _ => 0.0,
    }
}

/// A control written to, fitted to the range it actually has on this node: a knob whose ends
/// have been narrowed by hand cannot be pushed outside them by a preset.
fn write(node: &Node, key: &'static str, v: f32) -> (&'static str, ControlValue) {
    let fitted = match crate::nodes::control_range(node.def, node, key) {
        Some(range) => v.clamp(range.min, range.max),
        None => v,
    };
    (key, ControlValue::Float(fitted))
}
