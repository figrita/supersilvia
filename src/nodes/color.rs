// SPDX-License-Identifier: AGPL-3.0-or-later

//! One color, everywhere: silvia's `color.js`.
//!
//! A swatch and the port that carries it, so a patch has somewhere to keep a color it uses
//! in several places, and a cable into the swatch's own port overrides it. Both ports are
//! **uniform** colors: what the node holds is one color a frame, not a picture, and saying
//! so on the row is what lets every swatch downstream draw the color arriving at it. The
//! `cpu` half is the whole of the node — a tick that reads the swatch and publishes it.

use crate::graph::NodeId;
use crate::graph::PortType::UniformColor;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "color",
    category: Category::Generate,
    icon: "🎨",
    label: "Color",
    tooltip: "One color, everywhere. Pick it on the swatch, or cable a color in.",
    inputs: &[InputDef {
        key: "color",
        label: "Color",
        ty: UniformColor,
        control: Control::color("#ff0080ff"),
    }],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: UniformColor,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Color),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// No state of its own: the color is the node's control, or whatever is cabled into it, and
/// both are read fresh each tick.
struct Color;

impl CpuNode for Color {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let color = ctx.color(id, "color");
        ctx.publish_color(id, "output", color);
    }
}
