// SPDX-License-Identifier: AGPL-3.0-or-later

//! One number chosen by a seed. Same seed, same number, forever: the repeatable half of
//! silvia's random family, beside `triggeredrandom` and `randomfire`, which pick afresh
//! when fired. silvia's is a GLSL hash of the seed that answers one number for the whole
//! frame, which is what a uniform number is, so it lives on the CPU here and its three
//! knobs take a cable: a counter walks the seed, and the choice survives a reload.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "random",
    category: Category::Math,
    icon: "🎲",
    label: "Random",
    tooltip: "One number between Min and Max, chosen by Seed. The same seed gives the same \
              number every time; walk the seed to choose another.",
    inputs: &[
        InputDef {
            key: "seed",
            label: "Seed",
            ty: UniformNumber,
            control: Control::num(0.0, 0.0, 1000.0, 1.0, ""),
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
        key: "output",
        label: "Output",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Random),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// silvia's hash, `fract(sin(dot(vec2(seed, seed * 1.1), vec2(12.9898, 78.233))) * 43758.5453)`,
/// in single precision so a seed picks the same number here that it picked there, near
/// enough: the two `sin`s differ in their last bits, and so may the number.
pub fn hash(seed: f32) -> f32 {
    let x = (seed * 12.9898 + seed * 1.1 * 78.233).sin() * 43_758.547;
    x - x.floor()
}

struct Random;

impl CpuNode for Random {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let min = ctx.input(id, "min");
        let max = ctx.input(id, "max");
        let value = min + hash(ctx.input(id, "seed")) * (max - min);
        ctx.publish(id, "output", value);
    }
}
