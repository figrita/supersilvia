// SPDX-License-Identifier: AGPL-3.0-or-later

//! Passes one beat in every N. Actions in, actions out.
//!
//! The composability test for the event half: if this node is awkward to write, the event
//! system is wrong. It is nine lines, because a gate arrives as two edges and a divider only
//! has to decide which downs to let through — the up that follows a passed down goes with
//! it, so a gate is never left half open downstream.
//!
//! The ports are silvia's — Clock In and Divided Out, which is the whole node in four words
//! — and so are the two things it says about itself: a line in the body counting where it is
//! in the group, flashing as each beat gets through, and a count that starts over when
//! Division moves, so the next beat through is the first of the new group rather than
//! wherever the running count happened to land.

use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Event, Gate, InputDef, NodeDef, OutputDef,
    OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "clockdivider",
    category: Category::Control,
    icon: "➗",
    label: "Clock Divider",
    tooltip: "Passes one trigger in every N and swallows the rest.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Clock In",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "divide",
            label: "Divide",
            ty: UniformNumber,
            control: Control::num(4.0, 1.0, 64.0, 1.0, ""),
        },
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
    ],
    outputs: &[OutputDef {
        key: "trigger",
        label: "Divided Out",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    }],
    regions: &[crate::nodes::Region::Status],
    cpu: Some(CpuDef {
        create: || Box::new(ClockDivider::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

#[derive(Default)]
struct ClockDivider {
    /// How many downs have arrived since the last one passed.
    count: u32,
    /// Whether the gate now open downstream is one we opened.
    passing: bool,
    /// The button, which is a source like any other and arrives in the same list.
    hand: Gate,
    /// The division the count is against, so a hand moving the knob starts the group over
    /// rather than applying a new size to a count already part way through the old one.
    /// `None` until the first tick, which is not a change.
    divide: Option<u32>,
    /// Ticks left of the mark on the status line. A beat passing is one tick of the synth's
    /// and the editor reads a snapshot when it paints, so a flash that lasted one tick is a
    /// flash the screen could miss. Ticks rather than seconds on purpose: this node does not
    /// integrate time and must not start.
    flash: u8,
    /// Whether any beat has arrived at all, which is the difference between *ready* and
    /// *none of this group yet*.
    seen: bool,
}

/// How many ticks the mark stays up: about a tenth of a second at any rate a display runs
/// at, which is a flash rather than a state.
const FLASH_TICKS: u8 = 6;

impl CpuNode for ClockDivider {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{}{}",
            self.count,
            if self.passing { " open" } else { "" }
        ))
    }

    /// The line in the body: where the group is, and a mark on the beat that got through.
    ///
    /// silvia's own status line, which is what tells you a divider on a slow clock is alive
    /// and how far off the next pass is.
    fn status(&self) -> Option<String> {
        let divide = self.divide.unwrap_or(1).max(1);
        let mark = if self.flash > 0 {
            format!("{} ", crate::nodes::cpu::FLASH)
        } else {
            String::new()
        };
        Some(if self.seen {
            // The count is how many of the group have been *used*, so a beat that just
            // passed reads "1 of 4" rather than "0 of 4".
            let at = if self.count == 0 { divide } else { self.count };
            format!("{mark}{at} of {divide}")
        } else {
            format!("Ready ÷{divide}")
        })
    }

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        if ctx.pressed(id, "reset") || ctx.edges(id, "reset").iter().any(|e| e.is_down()) {
            self.count = 0;
            // Back to what a fresh node says, because that is what a reset divider is: the
            // next beat is the first of a group, and none of this one has arrived.
            self.seen = false;
        }

        let divide = ctx.input(id, "divide").max(1.0).round() as u32;
        // A division that moved starts the group over, the way silvia's does: going from
        // four to three live is a clean change rather than one you nudge back into place.
        if self.divide.replace(divide).is_some_and(|had| had != divide) {
            self.count = 0;
            self.seen = false;
        }
        self.flash = self.flash.saturating_sub(1);
        let mut events = ctx.edges(id, "input");
        // The button is one more source, and it is turned into events here rather than
        // treated as a special case below. A hand knows only the frame it pressed in.
        if let Some(edge) = self.hand.set(ctx.pressed(id, "input")) {
            events.push(Event::now(edge));
        }
        for event in events {
            // Every event that survives keeps the moment it arrived with. A divider is a
            // filter, not a re-clock: it must not round a beat to the frame it noticed it in.
            match event.edge {
                Edge::Down => {
                    let pass = self.count.is_multiple_of(divide);
                    self.count = (self.count + 1) % divide.max(1);
                    self.seen = true;
                    if pass {
                        self.passing = true;
                        self.flash = FLASH_TICKS;
                        ctx.fire_at(id, "trigger", Edge::Down, event.at);
                    }
                }
                Edge::Up => {
                    if self.passing {
                        self.passing = false;
                        ctx.fire_at(id, "trigger", Edge::Up, event.at);
                    }
                }
            }
        }
    }
}
