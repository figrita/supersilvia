// SPDX-License-Identifier: AGPL-3.0-or-later

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
                "    return vec4f({}, {}, {}, {});",
                ctx.input(node, "r", "uv"),
                ctx.input(node, "g", "uv"),
                ctx.input(node, "b", "uv"),
                ctx.input(node, "a", "uv"),
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
