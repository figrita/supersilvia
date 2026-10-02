// SPDX-License-Identifier: AGPL-3.0-or-later

//! Latches a fresh random color on every press. Both halves: the CPU side rolls three
//! fresh channels — alpha is always opaque, silvia's own choice, not a random fourth
//! channel — and publishes them twice, as one `color` and as the four `UniformNumber`s
//! beside it, the way a `sample` publishes what it measured.
//!
//! The `color` port is a `UniformColor`, published directly rather than assembled in the
//! shader out of the node's own four uniforms: one `vec4f` upload instead of four floats and
//! a `vec4f()` per fragment, a live swatch on the row, and — because a uniform color feeds a
//! varying color input for free — a cable from it lands on any color input.
//!
//! silvia's power-on color, before the first trigger, is magenta — `[1, 0, 1, 1]` — kept
//! here too rather than latching a random one on the first tick, unlike `triggeredrandom`.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformColor, UniformNumber};
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "triggeredcolor",
    category: Category::Control,
    icon: "🚦",
    label: "Triggered Color",
    tooltip: "Outputs a new random color each time it is triggered.",
    inputs: &[InputDef {
        key: "trigger",
        label: "Trigger",
        ty: Action,
        control: Control::Press,
    }],
    outputs: &[
        OutputDef {
            key: "color",
            label: "Color",
            ty: UniformColor,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "r",
            label: "R",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "g",
            label: "G",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "b",
            label: "B",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "a",
            label: "A",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || Box::new(TriggeredColor::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

struct TriggeredColor {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    trigger: Gate,
    rng: Rng,
}

impl Default for TriggeredColor {
    fn default() -> Self {
        // silvia's `currentColor` default: magenta, opaque.
        Self {
            r: 1.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
            trigger: Gate::default(),
            rng: Rng::default(),
        }
    }
}

impl CpuNode for TriggeredColor {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.2} {:.2} {:.2}", self.r, self.g, self.b))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        if ctx.downs(id, "trigger", &mut self.trigger) > 0 {
            self.r = self.rng.next_f32();
            self.g = self.rng.next_f32();
            self.b = self.rng.next_f32();
            // Alpha is always opaque — silvia rolls three channels, not four.
        }
        ctx.publish_color(id, "color", [self.r, self.g, self.b, self.a]);
        ctx.publish(id, "r", self.r);
        ctx.publish(id, "g", self.g);
        ctx.publish(id, "b", self.b);
        ctx.publish(id, "a", self.a);
    }
}
