// SPDX-License-Identifier: AGPL-3.0-or-later

//! A travel from one value to another over a duration, and how it comes back.
//!
//! silvia's node. Two ends, a duration and two curves: the approach is how it gets there and
//! the return is how it comes home, or whether it comes home at all — `stay` holds at the far
//! end after one pass and `jump` snaps back and goes again. Everything else ping-pongs over
//! two cycles with the return curve mirrored.
//!
//! The phase is a `phasor` at `1/duration` cycles a second with the same 50 ms glide
//! `oscillator` uses, so dragging the duration bends the travel instead of teleporting it.
//! It starts **stopped**, holding the start value, because an animation is a thing a
//! performer fires rather than something already running when the node lands. It is a
//! stateful node, like `adsr`: a person or an event starts it, and it keeps its own start.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::cpu::TraceRing;
use crate::nodes::phasor::{Birth, Phasor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};
use std::f32::consts::PI;

pub static DEF: NodeDef = NodeDef {
    slug: "animation",
    category: Category::Control,
    icon: "🐰",
    label: "Animation",
    tooltip: "Travels from start to end over its duration, and comes back however the return curve says.",
    // The travel is the thing being edited, as an envelope is on `adsr`; see
    // docs/decisions.md#a-trace-is-a-picture-of-the-shape-a-node-is-editing. 300 is
    // `canvas::SCOPE_NODE_WIDTH`, which `nodes/` cannot name.
    regions: &[crate::nodes::Region::Trace],
    inputs: &[
        InputDef {
            key: "startValue",
            label: "Start Value",
            ty: UniformNumber,
            control: Control::num(0.0, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: "endValue",
            label: "End Value",
            ty: UniformNumber,
            control: Control::num(1.0, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: "duration",
            label: "Duration",
            ty: UniformNumber,
            control: Control::num(1.0, 0.01, 60.0, 0.01, "s"),
        },
        InputDef {
            key: "startStop",
            label: "Start/Stop",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "restart",
            label: "Restart",
            ty: Action,
            control: Control::Press,
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
            key: "approach_curve",
            label: "Approach Curve",
            default: "smooth",
            choices: &CURVES,
            // Read by `tick`. The output is a uniform number, so this node emits no
            // WGSL for an option to change.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "return_curve",
            label: "Return Curve",
            default: "smooth",
            choices: &RETURNS,
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_TRACE,
    ],
    cpu: Some(CpuDef {
        create: || Box::new(Animation::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The four curves both ends share.
const CURVES: [(&str, &str); 4] = [
    ("linear", "Linear"),
    ("smooth", "Smooth"),
    ("ease_in", "Ease-in"),
    ("ease_out", "Ease-out"),
];

/// The same four, plus the two that are not curves at all but what happens at the end of a
/// pass: `jump` starts the next one from the near end, `stay` never leaves the far one.
const RETURNS: [(&str, &str); 6] = [
    CURVES[0],
    CURVES[1],
    CURVES[2],
    CURVES[3],
    ("jump", "Jump"),
    ("stay", "Stay"),
];

/// silvia's `transitionDuration` for this node: the time constant the speed is followed with.
const GLIDE: f64 = 0.05;

/// How many seconds the trace holds: silvia's `historySize` of 200 for the same picture, at
/// the 60 Hz it was drawn at. Seconds rather than samples, since the band draws by time.
const TRACE_SPAN: f32 = 200.0 / 60.0;

/// The shortest duration the phase is computed from, silvia's own floor. A duration of zero
/// is a divide by it.
const MIN_DURATION: f32 = 0.01;

struct Animation {
    /// Cycles since the last restart, one cycle one pass from start to end, at a speed of one
    /// over the duration, glided.
    phase: Phasor,
    /// silvia's `isRunning`, false until something fires: the value holds at the start.
    running: bool,
    start_stop: Gate,
    restart: Gate,
    value: f64,
    /// The last `TRACE_SPAN` seconds of published values, dated, for the trace on the body.
    history: TraceRing,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            phase: Phasor::new(Birth::Zero).gliding(GLIDE),
            running: false,
            start_stop: Gate::default(),
            restart: Gate::default(),
            value: 0.0,
            history: TraceRing::new(TRACE_SPAN),
        }
    }
}

/// One of the four curves, applied to a `t` in 0..1. silvia's formulas; `jump` and `stay` are
/// decided before this and travel linearly.
fn curve(t: f32, name: &str) -> f32 {
    match name {
        "smooth" => -((PI * t).cos() - 1.0) / 2.0,
        "ease_in" => t * t,
        "ease_out" => ((t * PI) / 2.0).sin(),
        // "linear", "jump", "stay" and anything a file carries that this build does not have.
        _ => t,
    }
}

impl CpuNode for Animation {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{} {:.3}",
            if self.running { "running" } else { "stopped" },
            self.value
        ))
    }

    fn trace(&self) -> Option<&TraceRing> {
        Some(&self.history)
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let start = ctx.input(id, "startValue");
        let end = ctx.input(id, "endValue");
        let duration = ctx.input(id, "duration").max(f64::from(MIN_DURATION));
        let return_curve = ctx.option(id, "return_curve");
        let approach_curve = ctx.option(id, "approach_curve");

        // silvia's `_toggleAnimation`: a down starts it, and a down while it is already
        // running pauses it — except in `stay`, where there is nothing to pause towards, so
        // the button is the replay.
        if ctx.downs(id, "startStop", &mut self.start_stop) % 2 == 1 {
            if !self.running {
                self.running = true;
            } else if return_curve == "stay" {
                self.phase.set(0.0);
            } else {
                self.running = false;
            }
        }

        // silvia's `_restartAnimation`: back to zero and running, at the moment inside the
        // frame the event fell the way `phase`'s reset does.
        let restart = ctx
            .last_down(id, "restart", &mut self.restart)
            .map(|at| ctx.fraction(at));
        // A pass a duration.
        let rate = 1.0 / duration;
        let mut walk = self.phase.walk(rate, &ctx.time.carried());
        walk.hold(!self.running);
        if let Some(at) = restart {
            walk.to(at);
            walk.set(0.0);
            walk.hold(false);
            self.running = true;
        }
        walk.finish();
        let cycles = self.phase.phase();

        self.value = if self.running {
            // Where in a pass it is, and which way it is going. `stay` clamps at the far end
            // after one pass, `jump` saws back to the near one, and everything else is a
            // ping-pong over two cycles with the second one mirrored.
            let (t, approaching) = match return_curve {
                "stay" if cycles >= 1.0 => (1.0, true),
                "stay" | "jump" => ((cycles % 1.0) as f32, true),
                _ => {
                    let cycle = (cycles % 2.0) as f32;
                    if cycle <= 1.0 {
                        (cycle, true)
                    } else {
                        (cycle - 1.0, false)
                    }
                }
            };
            let tweened = if approaching {
                curve(t, approach_curve)
            } else {
                1.0 - curve(t, return_curve)
            };
            start + (end - start) * f64::from(tweened)
        } else {
            // Stopped, it holds the near end: an animation nobody has fired is at its start.
            start
        };
        ctx.publish(id, "output", self.value);
        self.history.push(ctx.elapsed, self.value as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// silvia's four formulas, at the ends and in the middle. Each is 0 at 0 and 1 at 1, so
    /// the travel always reaches both of its values whichever curve it takes.
    #[test]
    fn every_curve_runs_from_zero_to_one() {
        for name in ["linear", "smooth", "ease_in", "ease_out"] {
            assert!(curve(0.0, name).abs() < 1e-6, "{name} starts at zero");
            assert!((curve(1.0, name) - 1.0).abs() < 1e-6, "{name} ends at one");
            let half = curve(0.5, name);
            assert!((0.0..=1.0).contains(&half), "{name} at half is {half}");
        }
        // Smooth is symmetric about its middle, which is the whole reason to pick it.
        assert!((curve(0.5, "smooth") - 0.5).abs() < 1e-6);
        // Ease-in is slow to leave and ease-out is slow to arrive.
        assert!(curve(0.5, "ease_in") < 0.5);
        assert!(curve(0.5, "ease_out") > 0.5);
    }
}
