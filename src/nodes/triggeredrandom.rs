// SPDX-License-Identifier: AGPL-3.0-or-later

//! Latches a fresh random number in range on every press. silvia's node holds a fixed 0.5
//! until the first trigger; here the first tick latches one instead, so the port never
//! reads a stale placeholder — the same reason `slew`'s value is `None` until its first
//! tick rather than zero.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "triggeredrandom",
    category: Category::Control,
    icon: "🎰",
    label: "Triggered Random",
    tooltip: "Outputs a new random value within the specified range each time it is triggered.",
    inputs: &[
        InputDef {
            key: "trigger",
            label: "Trigger",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "min",
            label: "Min",
            ty: UniformNumber,
            control: Control::num(0.0, -100.0, 100.0, 0.01, ""),
        },
        InputDef {
            key: "max",
            label: "Max",
            ty: UniformNumber,
            control: Control::num(1.0, -100.0, 100.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "value",
        label: "Value",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(TriggeredRandom::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct TriggeredRandom {
    value: f32,
    started: bool,
    trigger: Gate,
    rng: Rng,
}

impl CpuNode for TriggeredRandom {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.3}", self.value))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        let min = ctx.input(id, "min");
        let max = ctx.input(id, "max");

        // Changing Min/Max does not re-map or re-clamp the currently held value — only a
        // fresh trigger reads them, silvia's own behavior.
        if !self.started {
            self.started = true;
            self.value = self.rng.range(min, max);
        }
        if ctx.downs(id, "trigger", &mut self.trigger) > 0 {
            self.value = self.rng.range(min, max);
        }

        ctx.publish(id, "value", self.value);
    }
}
