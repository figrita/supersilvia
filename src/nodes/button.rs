// SPDX-License-Identifier: AGPL-3.0-or-later

//! A hand on an action port. The hello-world of the event half, and the smallest node that
//! shows what an action *is*: press and it fires down, let go and it fires up.
//!
//! Its input is an action port with a button for a control, so a sequencer can drive it too
//! — the hand and the cable are both sources, which is what many-to-many means, and the
//! output is held while either of them holds it.

use crate::graph::PortType::Action;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "button",
    category: Category::Control,
    icon: "🔘",
    label: "Button",
    tooltip: "Fires while it is held. Press to fire down, release to fire up.",
    inputs: &[InputDef {
        key: "press",
        label: "Press",
        ty: Action,
        control: Control::Press,
    }],
    outputs: &[OutputDef {
        key: "trigger",
        label: "Trigger",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Button::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct Button {
    /// What the cables are holding, remembered between frames — an edge lives one frame and
    /// a gate lives until it is released.
    upstream: bool,
    out: Gate,
}

impl CpuNode for Button {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(if self.out.is_down() { "down" } else { "up" }.to_string())
    }

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        let hand = ctx.pressed(id, "press");
        // The hand is only known to the frame, so its change is at the top of it. An event
        // arriving on a cable keeps the moment its source gave it: a button in the middle of
        // a chain must not round a beat to the frame it was noticed in.
        if let Some(edge) = self.out.set(hand || self.upstream) {
            ctx.fire(id, "trigger", edge);
        }
        for event in ctx.edges(id, "press") {
            self.upstream = event.is_down();
            if let Some(edge) = self.out.set(hand || self.upstream) {
                ctx.fire_at(id, "trigger", edge, event.at);
            }
        }
    }
}
