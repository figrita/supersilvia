// SPDX-License-Identifier: AGPL-3.0-or-later

//! A count that eases toward its target instead of snapping to it. `counter`'s own actions
//! and range inputs drive a `target`, the way `counter`'s `value` does; a second state,
//! `value`, chases it every tick by silvia's exponential approach.
//!
//! Departs from silvia's node in two ways. There is no `set` (silvia's, which drives the
//! target to `max`) and no `jump` (which drops `value` onto the target, skipping the
//! smoothing) — Increment, Decrement and Reset are the whole set, though `counter` beside it
//! has silvia's Set to Max. That was a porting call at first and is now a decision: a smooth
//! counter whose two extra actions both defeat the smoothing is two nodes, and the one here
//! is `counter` plus an ease. The ends option is
//! named `ends`, `counter`'s own key, rather than silvia's `mode`. Everything else is
//! silvia's: a fresh one counts 0 to 1 in tenths, which is a fade in ten nudges and what the
//! node is for, and it publishes `normalized` beside `value` — with `target`, which silvia
//! keeps private, as a third.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "smoothcounter",
    category: Category::Control,
    icon: "🎚",
    label: "Smooth Counter",
    tooltip: "Like Counter, but smoothly interpolates toward the target value. Good for smooth fades, camera moves, etc.",
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
            key: "step",
            label: "Step",
            ty: UniformNumber,
            control: Control::num(0.1, -100.0, 100.0, 0.1, ""),
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
        InputDef {
            key: "speed",
            label: "Rate",
            ty: UniformNumber,
            control: Control::num(5.0, 0.1, 50.0, 0.1, ""),
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
        OutputDef {
            key: "target",
            label: "Target",
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
        // Read by `tick`. The outputs are uniform numbers, so this node emits no WGSL at all.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(SmoothCounter::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct SmoothCounter {
    /// The smoothed output, silvia's `current`.
    value: f32,
    /// The count `increment`/`decrement`/`reset` move, silvia's `target` — what `counter`'s
    /// own `value` is, before the smoothing.
    target: f32,
    /// False until the first tick, which starts both at `min` rather than at zero.
    started: bool,
    increment: Gate,
    decrement: Gate,
    reset: Gate,
}

impl CpuNode for SmoothCounter {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.2} -> {:.2}", self.value, self.target))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let min = ctx.input(id, "min");
        let max = ctx.input(id, "max");
        let step = ctx.input(id, "step");
        // A rate, not a time constant: negative would run the approach backward, away from
        // the target, so a cable driving it below zero is held at a stop rather than honored.
        let speed = ctx.input(id, "speed").max(0.0);

        if !self.started {
            self.started = true;
            self.target = min;
            self.value = min;
        }

        let up = ctx.downs(id, "increment", &mut self.increment) as f32;
        let down = ctx.downs(id, "decrement", &mut self.decrement) as f32;
        if ctx.downs(id, "reset", &mut self.reset) > 0 {
            self.target = min;
        }
        self.target += step * (up - down);

        let wrap = ctx.option(id, "ends") == "wrap";
        // A range that is inside out is a control mid-drag, not an error: hold still, the
        // way `counter` does.
        if !wrap && max > min {
            self.target = self.target.clamp(min, max);
        }

        // silvia's `_advanceSmoothing`: skipped across a stall or a dead frame, so a
        // tab-switch gap does not jump the value.
        if ctx.dt > 0.0 && ctx.dt < 0.2 {
            self.value += (self.target - self.value) * (1.0 - (-speed * ctx.dt).exp());
        }

        // Wrap mode wraps `value` and `target` together, only here — a nudge lets `target`
        // run unbounded as a phase accumulator, and this is the one place it and `value`
        // are folded back into the range, exactly as silvia's `_advanceSmoothing` does.
        if wrap && max > min {
            let span = max - min;
            self.value = min + (self.value - min).rem_euclid(span);
            self.target = min + (self.target - min).rem_euclid(span);
        }

        ctx.publish(id, "value", self.value);
        // silvia's second output, and `counter`'s: the smoothed value as a fraction of the
        // range, which is what a fade or an opacity takes with no arithmetic in between.
        // Clamped, the way `counter` clamps it, so a target nudged past an end while the
        // value is still chasing it never publishes a fraction outside 0..1.
        let span = max - min;
        let normalized = if span > 0.0 {
            ((self.value - min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ctx.publish(id, "normalized", normalized);
        ctx.publish(id, "target", self.target);
    }
}
