// SPDX-License-Identifier: AGPL-3.0-or-later

//! Counts events. One of the two nodes where the event half becomes the number half — a
//! `UniformNumber` out, and a uniform number feeds a `VaryingNumber` for free, so a count
//! reaches a uniform with no plumbing at all.
//!
//! It publishes the raw count and the same count normalized into 0..1, because what a graph
//! usually wants is a fraction of the way through a range and re-deriving that is two math
//! nodes and an assumption about the ends.
//!
//! A fresh one is silvia's: 0 to 1 in hundredths, which is a fade rather than an index, and
//! Min and Max nudge in whole numbers. Four actions, silvia's own — `setmax` is the other
//! end of `reset`.

use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Gate, InputDef, NodeDef, OptionDef, OptionKind,
    OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "counter",
    category: Category::Control,
    icon: "🔢",
    label: "Counter",
    tooltip: "Counts triggers, clamping or wrapping at its ends.",
    inputs: &[
        InputDef {
            key: "increment",
            label: "Increment",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "decrement",
            label: "Decrement",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "setmax",
            label: "Set to Max",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "step",
            label: "Step",
            ty: UniformNumber,
            control: Control::num(0.01, -100.0, 100.0, 0.01, ""),
        },
        InputDef {
            key: "min",
            label: "Min",
            ty: UniformNumber,
            control: Control::num(0.0, -1000.0, 1000.0, 1.0, ""),
        },
        InputDef {
            key: "max",
            label: "Max",
            ty: UniformNumber,
            control: Control::num(1.0, -1000.0, 1000.0, 1.0, ""),
        },
    ],
    outputs: &[
        OutputDef {
            key: "value",
            label: "Value",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "normalized",
            label: "Normalized",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
    ],
    options: &[OptionDef {
        key: "ends",
        label: "At the ends",
        default: "clamp",
        choices: &[("clamp", "Clamp"), ("wrap", "Wrap")],
        // Read by `tick` when the count reaches a bound. The outputs are uniform
        // numbers, so this node emits no WGSL at all.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Counter::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct Counter {
    value: f32,
    /// False until the first tick, which starts the value at `min` rather than at zero — a
    /// counter over 4..8 reading 0 before its first trigger is lying about its range.
    started: bool,
    increment: Gate,
    decrement: Gate,
    reset: Gate,
    setmax: Gate,
}

/// How many downs arrived on one input this frame, counting the button as a source.
///
/// A count rather than a bool, because two sources firing in one frame are two events and a
/// counter that collapsed them would drift against the clock driving it.
fn downs(
    ctx: &TickContext<'_>,
    id: crate::graph::NodeId,
    key: &'static str,
    hand: &mut Gate,
) -> f32 {
    // Only the count matters: a counter publishes a uniform number, which is sampled once
    // per frame however precisely the events inside it were placed.
    let mut n = 0.0;
    for event in ctx.edges(id, key) {
        if event.is_down() {
            n += 1.0;
        }
    }
    if hand.set(ctx.pressed(id, key)) == Some(Edge::Down) {
        n += 1.0;
    }
    n
}

impl CpuNode for Counter {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.2}", self.value))
    }

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        let min = ctx.input(id, "min");
        let max = ctx.input(id, "max");
        let step = ctx.input(id, "step");

        if !self.started {
            self.started = true;
            self.value = min;
        }

        let up = downs(ctx, id, "increment", &mut self.increment);
        let down = downs(ctx, id, "decrement", &mut self.decrement);
        if downs(ctx, id, "reset", &mut self.reset) > 0.0 {
            self.value = min;
        }
        // silvia's fourth button, and the other end of `reset`: the way to the top of a
        // range from a trigger rather than by holding Increment.
        if downs(ctx, id, "setmax", &mut self.setmax) > 0.0 {
            self.value = max;
        }
        self.value += step * (up - down);

        // A range that is inside out is a control mid-drag, not an error: hold still.
        if max > min {
            self.value = match ctx.option(id, "ends") {
                "wrap" => min + (self.value - min).rem_euclid(max - min),
                _ => self.value.clamp(min, max),
            };
        }

        ctx.publish(id, "value", self.value);
        let span = max - min;
        let normalized = if span > 0.0 {
            ((self.value - min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ctx.publish(id, "normalized", normalized);
    }
}
