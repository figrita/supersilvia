// SPDX-License-Identifier: AGPL-3.0-or-later

//! A uniform number that follows its input at a bounded rate. The smallest CPU node there
//! is, and the reference for one: a `UniformNumber` input, a `UniformNumber` output, and a
//! `tick` that integrates `dt`.
//!
//! Two shapes, because both are worth having and a rate limit can only make one of them. A
//! **rate limit** leaves at a constant speed and arrives with a corner, which is what holds a
//! fast signal to a speed. An **ease** is the exponential approach silvia buried inside its
//! Smooth Counter — `1 - exp(-rate · dt)` of the gap each frame — which is quick at first and
//! slows as it lands. Rise and Fall are the two knobs in both, read as a speed in one and as
//! an approach rate in the other, so a fall can still be slower than a rise either way.

use crate::graph::PortType::UniformNumber;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "slew",
    category: Category::Control,
    icon: "🛝",
    label: "Slew",
    tooltip: "Follows its input: at a rate it cannot exceed, or easing toward it so it slows as it arrives.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: UniformNumber,
            control: Control::num(0.0, -100.0, 100.0, 0.01, ""),
        },
        InputDef {
            key: "rise",
            label: "Rise",
            ty: UniformNumber,
            control: Control::num(10.0, 0.01, 1000.0, 0.01, "/s"),
        },
        InputDef {
            key: "fall",
            label: "Fall",
            ty: UniformNumber,
            control: Control::num(10.0, 0.01, 1000.0, 0.01, "/s"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    options: &[OptionDef {
        key: "shape",
        label: "Shape",
        default: "rate",
        choices: &[("rate", "Rate Limit"), ("ease", "Ease")],
        // Read by `tick`. The output is a uniform number, so this node emits no WGSL for an
        // option to change.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Slew { value: None }),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

struct Slew {
    /// `None` until the first tick, which snaps to the input rather than ramping up from
    /// zero.
    value: Option<f64>,
}

impl CpuNode for Slew {
    fn reset(&mut self) {
        self.value = None;
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.3}", self.value.unwrap_or(0.0)))
    }

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        let target = ctx.input(id, "input");
        let current = match self.value {
            Some(v) => v,
            None => target,
        };
        // Whichever direction it is going in, over the whole frame: a control cannot move
        // inside one, so the direction cannot change inside one either.
        let rate = if target > current {
            ctx.input(id, "rise")
        } else {
            ctx.input(id, "fall")
        };
        let next = if ctx.option(id, "shape") == "ease" {
            // silvia's `_advanceSmoothing`: a fixed fraction of the gap a second, so the
            // value slows as it arrives and the ease takes the same wall time at any frame
            // rate. A negative rate would run the approach backwards, away from the target,
            // so a cable driving one below zero is held at a stop.
            current + (target - current) * (1.0 - (-rate.max(0.0) * f64::from(ctx.dt)).exp())
        } else {
            let step = rate * f64::from(ctx.dt);
            if (target - current).abs() <= step {
                target
            } else if target > current {
                current + step
            } else {
                current - step
            }
        };
        self.value = Some(next);
        ctx.publish(id, "output", next);
    }
}
