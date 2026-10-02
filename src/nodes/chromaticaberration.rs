// SPDX-License-Identifier: AGPL-3.0-or-later

//! A lens fringe: red and blue read a little off from green.
//!
//! Ported from silvia's `chromaticaberration.js`. Three modes, and each is a different way of
//! building the one offset vector, so `mode` is a `Code` option: Radial pushes the two
//! channels along the ray from the center and harder toward the edge, Linear pushes them all
//! one way at an angle, and Barrel scales the three channels by slightly different amounts.
//!
//! Three departures from the source. The center is a pair of inputs like every other lens
//! node here, defaulting to the middle of the picture — silvia's is fixed at `vec2(0.5)` in a
//! uv whose middle is elsewhere, so its radial fringe is lopsided and its barrel bulges off
//! center. The alpha comes from the green read rather than a fourth read at `uv`, which is
//! the same expression at the same coordinate and one sample fewer. And Barrel asks for no
//! angle at all, so that mode's shader neither declares the uniform nor calls whatever drives
//! it.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

pub static DEF: NodeDef = NodeDef {
    slug: "chromaticaberration",
    category: Category::Effect,
    icon: "🔭",
    label: "Chromatic Aberration",
    tooltip: "Separates the color channels the way a lens does. Radial pushes them out from \
              the center and harder toward the edge, Linear pushes them one way at an angle, \
              and Barrel scales each channel by a little more or less.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "offset",
            label: "Spread",
            ty: VaryingNumber,
            control: Control::num(0.01, 0.0, 0.1, 0.001, "⬓"),
        },
        InputDef {
            key: "angle",
            label: "Angle",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        },
        InputDef {
            key: "centerX",
            label: "Center X",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
        InputDef {
            key: "centerY",
            label: "Center Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let offset = ctx.input(node, "offset", "uv");
            let mode = ctx.option(node, "mode").to_string();
            let coordinates = match mode.as_str() {
                "linear" => {
                    let angle = ctx.input(node, "angle", "uv");
                    format!(
                        "    let offset = {offset};
    let angle = ({angle}) * 2.0 * PI;
    let dir = vec2f(cos(angle), sin(angle));
    let uvR = uv - dir * offset;
    let uvG = uv;
    let uvB = uv + dir * offset;"
                    )
                }
                "barrel" => {
                    let cx = ctx.input(node, "centerX", "uv");
                    let cy = ctx.input(node, "centerY", "uv");
                    format!(
                        "    let offset = {offset};
    let center = vec2f({cx}, {cy});
    let toCenter = uv - center;
    let uvR = center + toCenter * (1.0 + offset * 0.5);
    let uvG = center + toCenter;
    let uvB = center + toCenter * (1.0 - offset * 0.5);"
                    )
                }
                _ => {
                    let angle = ctx.input(node, "angle", "uv");
                    let cx = ctx.input(node, "centerX", "uv");
                    let cy = ctx.input(node, "centerY", "uv");
                    format!(
                        "    let offset = {offset};
    let angle = ({angle}) * 2.0 * PI;
    let center = vec2f({cx}, {cy});
    let toCenter = uv - center;
    let dist = length(toCenter);
    let dir = select(toCenter / dist, vec2f(1.0, 0.0), dist < 0.001);
    let cs = cos(angle);
    let sn = sin(angle);
    let rotated = vec2f(dir.x * cs - dir.y * sn, dir.x * sn + dir.y * cs);
    let push = rotated * offset * dist;
    let uvR = uv - push;
    let uvG = uv;
    let uvB = uv + push;"
                    )
                }
            };
            let red = ctx.input(node, "input", "uvR");
            let green = ctx.input(node, "input", "uvG");
            let blue = ctx.input(node, "input", "uvB");
            format!(
                "{coordinates}
    let r = ({red}).r;
    let g = {green};
    let b = ({blue}).b;
    return vec4f(r, g.g, b, g.a);"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[OptionDef {
        key: "mode",
        label: "Mode",
        default: "radial",
        choices: &[
            ("radial", "Radial"),
            ("linear", "Linear"),
            ("barrel", "Barrel Distortion"),
        ],
        ..OptionDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
