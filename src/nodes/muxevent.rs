// SPDX-License-Identifier: AGPL-3.0-or-later

//! Switches between four colors on `next`/`prev`, silvia's own actions, with `rand` — also
//! silvia's — beside a `reset` that is ours. The CPU half keeps the channel as a wrapping
//! index and publishes it as a `UniformNumber`; the WGSL reads it back through
//! `own_uniform`, the pattern `autoexposure` uses for its gain, and picks a channel exactly
//! as silvia's own `if (channel == n)` chain does.
//!
//! **The channel counts from one**, so the published number is 1 through 4 and reads as the
//! row labels Input 1 through Input 4 do. It is an `integral` output, drawn with no decimals,
//! because an index that only ever lands on a whole number should not wear two zeroes saying
//! it might not have. The shader takes the one back off before it picks.
//!
//! **There is no crossfade**, as there is none in silvia's `muxevent.js`. One was folded in
//! from the sibling `muxnumber.js`, whose `select` is a continuous position and whose
//! `crossfade` mixes the channel either side of it by `fract(select)` — but this node's
//! index only ever moves in whole steps, on an action, so `fract` was always zero and the
//! option did nothing at all. A node that switches between four colors is a switch; the
//! continuous read belongs to `muxnumber`, whose select is a number, and that is where it
//! goes when it is ported.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber, VaryingColor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OutputDef, OutputKind,
    TickContext, rng::Rng,
};

pub static DEF: NodeDef = NodeDef {
    slug: "muxevent",
    category: Category::Color,
    icon: "👇",
    label: "Mux (Event)",
    tooltip: "Switches between 4 color inputs on Next and Prev, Random for one of the four, \
              or Reset to the first.",
    inputs: &[
        InputDef {
            key: "input0",
            label: "Input 1",
            ty: VaryingColor,
            control: Control::color("#ff0000ff"),
        },
        InputDef {
            key: "input1",
            label: "Input 2",
            ty: VaryingColor,
            control: Control::color("#00ff00ff"),
        },
        InputDef {
            key: "input2",
            label: "Input 3",
            ty: VaryingColor,
            control: Control::color("#0000ffff"),
        },
        InputDef {
            key: "input3",
            label: "Input 4",
            ty: VaryingColor,
            control: Control::color("#ffff00ff"),
        },
        InputDef {
            key: "next",
            label: "Next",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "prev",
            label: "Prev",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "rand",
            label: "Random",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
    ],
    outputs: &[
        OutputDef {
            key: "output",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            // The four in an array indexed by the channel, since WGSL has no `?:`.
            wgsl: |node, ctx, _func| {
                let c0 = ctx.input(node, "input0", "uv");
                let c1 = ctx.input(node, "input1", "uv");
                let c2 = ctx.input(node, "input2", "uv");
                let c3 = ctx.input(node, "input3", "uv");
                let index = ctx.own_uniform(node, "index");
                format!(
                    "    var colors = array<vec4f, 4>({c0}, {c1}, {c2}, {c3});
    let channel = i32(floor_mod(floor({index}) - 1.0, 4.0));
    return colors[channel];"
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "index",
            label: "Index",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            integral: true,
            ..OutputDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || Box::new(MuxEvent::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct MuxEvent {
    /// The selected channel, kept wrapped into 0..4 on every move — `rem_euclid` rather
    /// than a stored bound, so a `prev` from zero lands on three in one step. Published one
    /// higher, so the number on the row is the number in the row's label.
    index: i32,
    next: Gate,
    prev: Gate,
    rand: Gate,
    reset: Gate,
    /// silvia's `Math.random()`, seeded from the node's own id so a rerun of a patch is the
    /// same performance.
    rng: Rng,
}

impl CpuNode for MuxEvent {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("input {}", self.index + 1))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        let forward = ctx.downs(id, "next", &mut self.next) as i32;
        let back = ctx.downs(id, "prev", &mut self.prev) as i32;
        // Random first, so a frame carrying both a jump and a step lands the step on top of
        // the jump rather than throwing it away, and Reset after it, so Reset wins.
        if ctx.downs(id, "rand", &mut self.rand) > 0 {
            self.index = (self.rng.next_f32() * 4.0) as i32;
        }
        if ctx.downs(id, "reset", &mut self.reset) > 0 {
            self.index = 0;
        }
        self.index = (self.index + forward - back).rem_euclid(4);
        ctx.publish(id, "index", self.index as f32 + 1.0);
    }
}
