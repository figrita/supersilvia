// SPDX-License-Identifier: AGPL-3.0-or-later

//! A knob's movement, recorded and played back: the one node in the library that can be
//! taught something by hand.
//!
//! Everything else that shapes a number over time — `adsr`, `phase`, `slew` — makes its shape
//! out of knobs. This makes one out of a gesture. Press Record, move Input Value for as long
//! as Duration says, and the curve is kept; press Play and it runs, once or on a loop, mapped
//! into Min and Max, with Start Trim cutting the dead air off the front.
//!
//! **The recording is one of the node's own values and the transport is not.** The curve is a
//! `Value::Points` in `Node::values` — saved with the patch, undoable, and written by the
//! tick through `TickContext::write_value` when a recording stops, which is once per
//! recording and so one step of the undo history. Whether it is recording or playing, and
//! where the playhead is, live here and are dropped on save: reopening a patch gets the curve
//! back with the transport stopped, which is what a performer means by *the automation is
//! still there*. See docs/decisions.md and proposals/automation.md.
//!
//! silvia's arithmetic, kept: a point is stored whenever the input changes, the value it
//! keeps is normalized 0 to 1, the time is seconds from the start of the recording, and
//! playback interpolates linearly between the two points either side of where it is.

use crate::graph::PortType::{Action, UniformNumber};
use crate::graph::{NodeId, Point, Value};
use crate::nodes::phasor::{self, Phasor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext, ValueDef, ValueKind,
};

/// What the recording is stored under, which is also what the region that draws it finds.
pub const RECORDING: &str = "recording";

pub static DEF: NodeDef = NodeDef {
    slug: "automation",
    category: Category::Control,
    icon: "📈",
    label: "Automation",
    tooltip: "Records the Input Value knob moving and plays the movement back, once or on a loop.",
    // The curve is the thing being performed, so it is drawn at the width a performance can
    // be read at — 300, `canvas::SCOPE_NODE_WIDTH`, which `nodes/` cannot name.
    regions: &[crate::nodes::Region::Curve, crate::nodes::Region::Caption],
    inputs: &[
        InputDef {
            key: "record",
            label: "Record",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "play",
            label: "Play",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "restart",
            label: "Restart",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "clear",
            label: "Clear",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "input",
            label: "Input Value",
            ty: UniformNumber,
            control: Control::num(0.5, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "duration",
            label: "Duration",
            ty: UniformNumber,
            control: Control::num(4.0, 0.1, 60.0, 0.1, "s"),
        },
        InputDef {
            key: "trim",
            label: "Start Trim",
            ty: UniformNumber,
            // silvia sets this knob's top end from Duration on every change; `capped_by` is
            // that listener as a declaration.
            control: Control::num_capped(0.0, 0.0, 60.0, 0.01, "s", "duration"),
        },
        InputDef {
            key: "min",
            label: "Min Value",
            ty: UniformNumber,
            control: Control::num(0.0, -10.0, 10.0, 0.01, ""),
        },
        InputDef {
            key: "max",
            label: "Max Value",
            ty: UniformNumber,
            control: Control::num(1.0, -10.0, 10.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "loop",
            label: "Loop Mode",
            default: "loop",
            choices: &[("once", "Once"), ("loop", "Loop")],
            // Read by `tick`. The output is a uniform number, so this node emits no WGSL.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_TRACE,
    ],
    values: &[ValueDef {
        key: RECORDING,
        // No label. The curve is a band of its own and a caption reading "Recording" over it
        // would say what the picture says.
        label: "",
        // What a recorded value is: silvia's own normalized 0 to 1, mapped into Min and Max
        // on the way out, so moving those ends re-reads the recording rather than rewriting
        // it.
        kind: ValueKind::Points { min: 0.0, max: 1.0 },
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Automation::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// What the transport is doing. Runtime state: none of this is saved.
#[derive(Default, Clone, Copy, PartialEq)]
enum Transport {
    #[default]
    Stopped,
    /// Seconds since Record was pressed.
    Recording(f32),
    /// How far through the recording playback is, 0 to 1 — silvia's phase accumulator, which
    /// runs at one over the duration: [`Automation::playback`], as the tick last left it.
    Playing(f32),
}

#[derive(Default)]
struct Automation {
    /// The curve as this node holds it, which is the document's plus whatever is being
    /// performed right now.
    points: Vec<Point>,
    /// The document's own copy, as this node last saw it. What makes a file just opened, an
    /// undo and a write of its own tell themselves apart: anything else in `Node::values` is
    /// news, and is adopted.
    seen: Vec<Point>,
    transport: Transport,
    /// Playback's phase, on the transport's advance at one over the duration.
    playback: Phasor,
    /// The last value recorded, so a knob standing still costs no points — silvia's
    /// `lastInputValue`.
    last: Option<f32>,
    /// What the band is drawn across, which is the duration the recording was made into.
    span: f32,
    record: Gate,
    play: Gate,
    restart: Gate,
    clear: Gate,
}

/// The knobs, read once a frame.
#[derive(Clone, Copy)]
struct Shape {
    input: f32,
    duration: f32,
    trim: f32,
    min: f32,
    max: f32,
    looping: bool,
}

impl Automation {
    /// The recording read at `t` of the way through it, 0 to 1, with the trim taken off the
    /// front. silvia's `_interpolateAutomation`.
    fn at(&self, t: f32, shape: Shape) -> f32 {
        if self.points.is_empty() {
            return 0.0;
        }
        let target = Self::target(t, shape);
        let mut left: Option<Point> = None;
        let mut right: Option<Point> = None;
        for p in &self.points {
            if p.time <= target {
                left = Some(*p);
            }
            if p.time >= target && right.is_none() {
                right = Some(*p);
                break;
            }
        }
        match (left, right) {
            (Some(a), Some(b)) if b.time > a.time => {
                let k = (target - a.time) / (b.time - a.time);
                a.value + k * (b.value - a.value)
            }
            (Some(a), _) => a.value,
            (None, Some(b)) => b.value,
            (None, None) => 0.0,
        }
    }

    /// Where in the recording `t` of the way through playback lands, in seconds.
    fn target(t: f32, shape: Shape) -> f32 {
        let effective = (shape.duration - shape.trim).max(0.0);
        shape.trim + t.clamp(0.0, 1.0) * effective
    }

    /// Start a fresh recording: silvia throws the old curve away on the way in.
    fn start_recording(&mut self) {
        self.points.clear();
        self.last = None;
        self.transport = Transport::Recording(0.0);
    }

    /// Stop recording and put the curve in the document, which is the one write this node
    /// makes.
    fn stop_recording(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.transport = Transport::Stopped;
        self.commit(id, ctx);
    }

    fn commit(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.seen.clone_from(&self.points);
        ctx.write_value(id, RECORDING, Value::Points(self.points.clone()));
    }
}

impl CpuNode for Automation {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{} points", self.points.len()))
    }

    /// silvia's status label, as the cells under the curve.
    fn caption(&self) -> Vec<(&'static str, String)> {
        let state = match self.transport {
            Transport::Recording(_) => "Recording",
            Transport::Playing(_) => "Playing",
            Transport::Stopped => "Ready",
        };
        vec![
            ("state", state.to_string()),
            ("points", self.points.len().to_string()),
        ]
    }

    fn curve(&self) -> Option<crate::nodes::cpu::Curve> {
        Some(crate::nodes::cpu::Curve {
            points: self.points.clone(),
            head: match self.transport {
                Transport::Recording(elapsed) => Some(elapsed),
                Transport::Playing(phase) => Some(phase * self.span),
                Transport::Stopped => None,
            },
            recording: matches!(self.transport, Transport::Recording(_)),
            span: self.span,
        })
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let shape = Shape {
            input: ctx.input(id, "input") as f32,
            duration: (ctx.input(id, "duration") as f32).max(0.01),
            trim: (ctx.input(id, "trim") as f32).max(0.0),
            min: ctx.input(id, "min") as f32,
            max: ctx.input(id, "max") as f32,
            looping: ctx.option(id, "loop") != "once",
        };
        self.span = shape.duration;

        // A curve in the document this node has not seen is news: a file just opened, an
        // undo, or a hand pasting a node. Its own writes come back here identical and are
        // therefore not adopted.
        let document = ctx
            .value(id, RECORDING)
            .and_then(Value::points)
            .unwrap_or_default();
        if document != self.seen.as_slice() {
            self.points = document.to_vec();
            self.seen.clone_from(&self.points);
        }

        // The four buttons. A count, so two presses in one frame are two presses — and a
        // toggle flips on an odd one, wherever the press came from.
        let record = ctx.downs(id, "record", &mut self.record);
        let play = ctx.downs(id, "play", &mut self.play);
        let restart = ctx.downs(id, "restart", &mut self.restart);
        let clear = ctx.downs(id, "clear", &mut self.clear);

        if record % 2 == 1 {
            match self.transport {
                Transport::Recording(_) => self.stop_recording(id, ctx),
                _ => self.start_recording(),
            }
        }
        if play % 2 == 1 {
            self.transport = match self.transport {
                Transport::Playing(_) => Transport::Stopped,
                // Nothing to play is nothing to start, which is silvia's own guard.
                _ if self.points.is_empty() => Transport::Stopped,
                _ => {
                    self.playback.set(0.0);
                    Transport::Playing(0.0)
                }
            };
        }
        if restart > 0 && !self.points.is_empty() {
            self.playback.set(0.0);
            self.transport = Transport::Playing(0.0);
        }
        if clear > 0 {
            self.points.clear();
            self.last = None;
            self.transport = Transport::Stopped;
            self.commit(id, ctx);
        }

        // A play a duration.
        let rate = 1.0 / f64::from(shape.duration);
        match self.transport {
            Transport::Recording(elapsed) => {
                let elapsed = elapsed + ctx.dt;
                let value = shape.input.clamp(0.0, 1.0);
                // A point where the knob moved, and one at the head of the recording so a
                // performance that starts from rest still has somewhere to start from.
                if self.last != Some(value) && elapsed <= shape.duration {
                    self.points.push(Point {
                        time: elapsed.min(shape.duration),
                        value,
                    });
                    self.last = Some(value);
                }
                if elapsed >= shape.duration {
                    // silvia's own timeout: a recording is exactly Duration long.
                    self.stop_recording(id, ctx);
                } else {
                    self.transport = Transport::Recording(elapsed);
                }
            }
            Transport::Playing(_) => {
                self.playback.step(rate, &ctx.time.carried());
                let phase = self.playback.phase();
                self.transport = if shape.looping {
                    let phase = phasor::fraction(phase, 1.0);
                    self.playback.set(phase);
                    Transport::Playing(phase as f32)
                } else if phase >= 1.0 {
                    Transport::Stopped
                } else {
                    Transport::Playing(phase as f32)
                };
            }
            Transport::Stopped => {}
        }

        // Playing reads the curve; anything else passes the knob through, across the same
        // ends. That is what makes recording audible: what is being performed is what is out.
        let normalized = match self.transport {
            Transport::Playing(phase) => self.at(phase, shape),
            _ => shape.input.clamp(0.0, 1.0),
        };
        ctx.publish(
            id,
            "output",
            f64::from(shape.min + normalized * (shape.max - shape.min)),
        );
    }
}
