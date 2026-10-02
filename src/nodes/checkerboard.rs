// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

pub static DEF: NodeDef = NodeDef {
    slug: "checkerboard",
    category: Category::Generate,
    icon: "🏁",
    label: "Checkerboard",
    tooltip: "Alternating checkerboard pattern. Frequency sets grid resolution.",
    inputs: &[
        InputDef {
            key: "frequency",
            label: "Frequency",
            ty: VaryingNumber,
            control: Control::num(8.0, 1.0, 64.0, 1.0, "/⬓"),
        },
        InputDef {
            key: "color1",
            label: "Odd Color",
            ty: VaryingColor,
            control: Control::color("#ffffffff"),
        },
        InputDef {
            key: "color2",
            label: "Even Color",
            ty: VaryingColor,
            control: Control::color("#000000ff"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let frequency = ctx.input(node, "frequency", "uv");
            let c1 = ctx.input(node, "color1", "uv");
            let c2 = ctx.input(node, "color2", "uv");
            format!(
                "    let frequency = {frequency};
    let scaled_uv = uv * (frequency / 2.0);
    let grid = floor(scaled_uv);
    let checker = floor_mod(grid.x + grid.y, 2.0);
    let c1 = {c1};
    let c2 = {c2};
    return mix(c1, c2, checker);"
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
