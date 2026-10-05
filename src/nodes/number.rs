// SPDX-License-Identifier: AGPL-3.0-or-later

//! One knob, one number, cabled wherever it is wanted.
//!
//! Ported from silvia's `number.js`, with silvia's range: 0, -100 to 100, a hundredth a
//! step. `add` with its second knob left at zero is the same value and has been all along;
//! what was missing is the name a person reaches for, and a knob that is there to be
//! ignored is a knob that gets read anyway.
//!
//! silvia's is a field — a `float` generator returning the control — and this one is a
//! uniform number: the value is the same across the frame by construction, so the CPU holds
//! it and every consumer reads one uniform. That also makes it a number a CPU node can
//! read, which silvia's could not be.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "number",
    category: Category::Control,
    icon: "🔢",
    label: "Number",
    tooltip: "One number, set in one place and read wherever a cable reaches.",
    inputs: &[InputDef {
        key: "value",
        label: "Value",
        ty: UniformNumber,
        control: Control::num(0.0, -100.0, 100.0, 0.01, ""),
    }],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Number::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The last value published, for the Status box. A cable into the knob overrides it, the
/// way it does on every other control, so the node passes a number on as readily as it
/// holds one.
#[derive(Default)]
struct Number {
    value: f64,
}

impl CpuNode for Number {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.3}", self.value))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.value = ctx.input(id, "value");
        ctx.publish(id, "output", self.value);
    }
}
