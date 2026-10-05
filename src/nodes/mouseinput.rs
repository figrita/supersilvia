// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pointer, as two numbers and two buttons.
//!
//! silvia's node measured against the editor page, because the editor page was the whole
//! program. Here a hand can be over three surfaces that mean different things, so
//! **Measure against** names one: a picture in a window of its own — the default, and what
//! a performer means — the mixer's preview, or the canvas. `proposals/mouseinput.md` is the
//! argument, and "options for the position framing" is read as that option: the framing
//! is the surface the position is framed in.
//!
//! The position is in that picture's own world units, so `x` and `y` drive a shape's center
//! or a `sample`'s point and land where the hand is. **Off the surface it publishes
//! nothing** — the rows go blank rather than holding the last place the hand was — and a
//! button held when the pointer leaves comes up, because an envelope it opened has to close.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, CpuDef, CpuNode, Edge, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
    TickContext,
};
use crate::pointer::Surface;

pub static DEF: NodeDef = NodeDef {
    slug: "mouseinput",
    category: Category::Source,
    icon: "🖱",
    label: "Mouse Input",
    tooltip: "Where the pointer is over a picture, in that picture's own world units, with \
              each button as a level and as an event. Publishes nothing while the pointer \
              is off the surface it measures against.",
    outputs: &[
        OutputDef {
            key: "x",
            label: "X Position",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "y",
            label: "Y Position",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "leftButton",
            label: "Left Button",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "rightButton",
            label: "Right Button",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "leftEvent",
            label: "Left Event",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "rightEvent",
            label: "Right Event",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
    ],
    options: &[OptionDef {
        key: "framing",
        label: "Measure against",
        default: "picture",
        choices: &[
            ("picture", "Picture"),
            ("preview", "Preview"),
            ("canvas", "Canvas"),
        ],
        // Read by `tick`. Every output is a uniform number or an action, so this node emits
        // no WGSL for an option to change.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    regions: &[crate::nodes::Region::Status],
    cpu: Some(CpuDef {
        create: || Box::new(MouseInput::default()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// The level and the event port for one button, in the order [`crate::pointer::Reading`]
/// holds them.
const BUTTONS: [(&str, &str); 2] = [("leftButton", "leftEvent"), ("rightButton", "rightEvent")];

#[derive(Default)]
struct MouseInput {
    /// Each button as the last tick saw it, so an edge is fired once. A button the pointer
    /// carried off the surface is read as up, which is what closes an envelope it opened.
    held: [bool; 2],
    /// Whether the last tick had a reading, for the status line.
    over: bool,
}

impl CpuNode for MouseInput {
    fn reset(&mut self) {
        self.held = [false; 2];
        self.over = false;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let surface = Surface::of(ctx.option(id, "framing"));
        let reading = ctx.pointer(surface);
        self.over = reading.is_some();

        if let Some(r) = reading {
            ctx.publish(id, "x", f64::from(r.x));
            ctx.publish(id, "y", f64::from(r.y));
            for (i, (level, _)) in BUTTONS.iter().enumerate() {
                let down = [r.left, r.right][i];
                ctx.publish(id, level, if down { 1.0 } else { 0.0 });
            }
        } else {
            // Nothing, rather than zero: a hand that is somewhere else has no position, and
            // `0.00` would be the middle of the picture. The rows draw blank, which is the
            // rule an unpublished port already follows.
            ctx.withdraw(id, "x");
            ctx.withdraw(id, "y");
            for (level, _) in BUTTONS {
                ctx.withdraw(id, level);
            }
        }

        // The gate half. A pointer off the surface holds no button, so leaving with the
        // button down is a release — which is what a `wl_pointer` leave means as well.
        for (i, (_, event)) in BUTTONS.iter().enumerate() {
            let down = reading.is_some_and(|r| [r.left, r.right][i]);
            if down != self.held[i] {
                ctx.fire(id, event, if down { Edge::Down } else { Edge::Up });
                self.held[i] = down;
            }
        }
    }

    fn status(&self) -> Option<String> {
        Some(if self.over {
            "pointer here".to_string()
        } else {
            "pointer off the surface".to_string()
        })
    }
}
