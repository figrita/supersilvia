// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ambient time as a number a patch can cable: **Seconds**, the playhead, published in `f64`
//! and never wrapped, so it counts on: everything that reads it reads the playhead to the bit.
//! A playhead before zero reads as the negative it is. It jumps with a seek, holds with a pause
//! and goes back to zero with the time readout's reset, because it *is* the playhead — a clip
//! cued to a moment reads it.
//!
//! It has no inputs and nothing of its own: a rate against a clock is a gear's, and a loop's
//! cycles are a Master Gear's. A file from when it had more opens with its cables out of
//! Seconds kept, and each port it dropped is reported by the loader. See `docs/cpu.md`.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::{Category, CpuDef, CpuNode, NodeDef, OutputDef, OutputKind, TickContext};

pub static DEF: NodeDef = NodeDef {
    slug: "time",
    category: Category::Gear,
    icon: "⏲",
    label: "Time",
    tooltip: "Ambient time: the playhead in seconds.",
    outputs: &[OutputDef {
        key: "seconds",
        label: "Seconds",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Time),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

struct Time;

impl CpuNode for Time {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        ctx.publish(id, "seconds", ctx.time.playhead);
    }
}
