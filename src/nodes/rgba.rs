// SPDX-License-Identifier: AGPL-3.0-or-later

//! Four numbers into a color. The numbers are the color's own channels, as a picker's are, and
//! the color is premultiplied as it is built, so an Alpha of zero is transparent black
//! whatever the other three say. See
//! [decisions.md](../../../docs/decisions.md#colors-in-the-graph-are-premultiplied).
//!
//! **Its output is dual**, as the maths nodes' are: one color a frame while every input is a
//! knob or a number, so it reaches a solid's Color; a color list while a list is cabled in, one
//! color per item; a picture while a field is. `eval_color` is the body in Rust.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

pub static DEF: NodeDef = NodeDef {
    slug: "rgba",
    category: Category::Color,
    icon: "🪢",
    label: "RGBA",
    tooltip: "Combines four numbers into a color.",
    inputs: &[
        InputDef {
            key: "r",
            label: "R",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "g",
            label: "G",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "b",
            label: "B",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "a",
            label: "A",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.0, 1.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            format!(
                "    return premultiply(vec4f({}, {}, {}, {}));",
                ctx.input(node, "r", "uv"),
                ctx.input(node, "g", "uv"),
                ctx.input(node, "b", "uv"),
                ctx.input(node, "a", "uv"),
            )
        },
        eval_color: Some(|node, ctx| {
            let c = |k| ctx.input(node, k) as f32;
            [c("r"), c("g"), c("b"), c("a")]
        }),
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
