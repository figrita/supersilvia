// SPDX-License-Identifier: AGPL-3.0-or-later

//! A game controller: six numbers and eight gates.
//!
//! silvia polled the browser's Gamepad API in an animation frame and could not place a
//! press better than the frame that noticed it. Here the device is read on a thread of its
//! own — see [`crate::gamepad`] — and an edge carries the moment inside the frame it
//! happened, the way an audio threshold does. The other difference is the event half's own:
//! a button is a **gate**, held open while the finger is down, so it opens an envelope
//! directly rather than needing a press and a release wired separately.
//!
//! The mapping is silvia's and standard: both sticks, both triggers, the four face buttons
//! and the four on the d-pad, with up positive on both sticks and silvia's dead zone on
//! them. `proposals/gamepad.md` is the argument.

use crate::gamepad::{AXES, AXIS_LABELS, BUTTONS, Pad};
use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

/// A uniform number output that is one of the six axes.
const fn axis(key: &'static str, label: &'static str) -> OutputDef {
    OutputDef {
        key,
        label,
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }
}

/// An action output that is one of the eight buttons.
const fn button(key: &'static str, label: &'static str) -> OutputDef {
    OutputDef {
        key,
        label,
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    }
}

pub static DEF: NodeDef = NodeDef {
    slug: "gamepad",
    category: Category::Source,
    icon: "🎮",
    label: "Gamepad",
    tooltip: "A game controller: both sticks and both triggers as numbers, and eight \
              buttons as gates held open while the finger is down.",
    inputs: &[InputDef {
        key: "rescan",
        label: "Rescan",
        ty: Action,
        // The button silvia put under its rows, on the port that carries what it fires:
        // a controller a desktop noticed late is looked for again from here, and a
        // sequencer can ask for the same thing.
        control: Control::Press,
    }],
    outputs: &[
        axis("leftStickX", AXIS_LABELS[0]),
        axis("leftStickY", AXIS_LABELS[1]),
        axis("rightStickX", AXIS_LABELS[2]),
        axis("rightStickY", AXIS_LABELS[3]),
        axis("leftTrigger", AXIS_LABELS[4]),
        axis("rightTrigger", AXIS_LABELS[5]),
        button("buttonA", "Button A (Cross)"),
        button("buttonB", "Button B (Circle)"),
        button("buttonX", "Button X (Square)"),
        button("buttonY", "Button Y (Triangle)"),
        button("dpadUp", "D-pad Up"),
        button("dpadDown", "D-pad Down"),
        button("dpadLeft", "D-pad Left"),
        button("dpadRight", "D-pad Right"),
    ],
    options: &[OptionDef {
        key: "device",
        label: "Device",
        default: "auto",
        choices: &[
            ("auto", "First connected"),
            ("1", "Controller 1"),
            ("2", "Controller 2"),
            ("3", "Controller 3"),
            ("4", "Controller 4"),
        ],
        // Read by `tick`, which opens the device on it.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    regions: &[crate::nodes::Region::Status],
    cpu: Some(CpuDef {
        create: || Box::new(Gamepad::default()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// The six axis outputs, in [`AXIS_LABELS`]' order.
const AXIS_PORTS: [&str; AXES] = [
    "leftStickX",
    "leftStickY",
    "rightStickX",
    "rightStickY",
    "leftTrigger",
    "rightTrigger",
];

/// The eight button outputs, in the device thread's order.
const BUTTON_PORTS: [&str; BUTTONS] = [
    "buttonA",
    "buttonB",
    "buttonX",
    "buttonY",
    "dpadUp",
    "dpadDown",
    "dpadLeft",
    "dpadRight",
];

#[derive(Default)]
struct Gamepad {
    pad: Option<Pad>,
    /// The device the option last asked for, so the thread is rebuilt only when it changes.
    opened: String,
    /// The hand on *Rescan*, as the one place a held button becomes an edge.
    rescan: crate::nodes::Gate,
    name: Option<String>,
    error: Option<String>,
}

impl CpuNode for Gamepad {
    fn reset(&mut self) {
        // The device stays: it is what was expensive to acquire. What goes is every level
        // the last run left, which a fresh read replaces on the next tick.
        self.rescan = crate::nodes::Gate::default();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let wanted = ctx.option(id, "device").to_string();
        // A rescan is the device dropped and opened again, which is the whole of what a
        // rescan can be when the device's own hotplug is already watching.
        let rescan = ctx.downs(id, "rescan", &mut self.rescan) > 0;
        if self.pad.is_none() || self.opened != wanted || rescan {
            // Dropped before the next is opened, so one node never holds two threads.
            self.pad = None;
            self.pad = Some(Pad::open(&wanted));
            self.opened = wanted;
        }

        let Some(pad) = self.pad.as_ref() else { return };
        let reading = pad.take();
        self.name = reading.name;
        self.error = reading.error;

        for (i, key) in AXIS_PORTS.iter().enumerate() {
            ctx.publish(id, key, f64::from(reading.axes[i]));
        }

        // An edge carries where inside this frame it happened: the thread stamped it when
        // it read it, so how long ago that was is known, and `dt` minus that is the moment.
        // A stamp older than the frame is an event from before it, which lands at zero.
        let now = std::time::Instant::now();
        for event in reading.events {
            let Some(port) = BUTTON_PORTS.get(event.button) else {
                continue;
            };
            let age = now.saturating_duration_since(event.at).as_secs_f32();
            let at = (ctx.dt - age).clamp(0.0, ctx.dt);
            let edge = if event.down { Edge::Down } else { Edge::Up };
            ctx.fire_at(id, port, edge, at);
        }
        if reading.dropped > 0 {
            log::debug!("gamepad{id}: {} edges were read too late", reading.dropped);
        }
    }

    fn status(&self) -> Option<String> {
        self.name
            .clone()
            .or_else(|| self.error.clone())
            .or_else(|| Some("scanning…".to_string()))
    }

    fn debug(&self) -> Option<String> {
        Some(match &self.name {
            Some(name) => format!("gamepad {name}"),
            None => "no gamepad".to_string(),
        })
    }
}
