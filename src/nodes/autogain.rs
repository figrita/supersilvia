// SPDX-License-Identifier: AGPL-3.0-or-later

//! Normalize a wandering uniform number to 0..1.
//!
//! Tracks a floor and a ceiling of its input: the ceiling rises at the attack rate and sinks
//! at the release rate, the floor the other way round. The output is where the input sits
//! between them. A microphone's `level` in a loud room and a quiet one both come out
//! touching 1 on the loud moments. `span` is the smallest gap the two are allowed, so
//! silence is not stretched to full scale.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "autogain",
    category: Category::Control,
    icon: "📶",
    label: "Auto Gain",
    tooltip: "Follows the floor and ceiling of its input and outputs where the input sits \
              between them, 0 to 1.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: UniformNumber,
            control: Control::num(0.0, -100.0, 100.0, 0.01, ""),
        },
        InputDef {
            key: "attack",
            label: "Attack",
            ty: UniformNumber,
            control: Control::num(0.05, 0.001, 5.0, 0.001, "s"),
        },
        InputDef {
            key: "release",
            label: "Release",
            ty: UniformNumber,
            control: Control::num(2.0, 0.01, 60.0, 0.01, "s"),
        },
        InputDef {
            key: "span",
            label: "Span",
            ty: UniformNumber,
            control: Control::num(0.05, 0.0001, 100.0, 0.0001, ""),
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
        create: || Box::new(AutoGain { range: None }),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// One pole toward `target`, with time constant `tau` seconds.
fn follow(current: f64, target: f64, tau: f64, dt: f64) -> f64 {
    let alpha = if tau > 0.0 {
        1.0 - (-dt / tau).exp()
    } else {
        1.0
    };
    current + (target - current) * alpha
}

/// The tracker, separated from the node so it can be tested as arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub lo: f64,
    pub hi: f64,
}

impl Range {
    /// Advance the trackers for one input and return the normalized value.
    pub fn step(&mut self, x: f64, attack: f64, release: f64, span: f64, dt: f64) -> f64 {
        // Toward a new extreme quickly, away from it slowly.
        self.hi = if x > self.hi {
            follow(self.hi, x, attack, dt)
        } else {
            follow(self.hi, x, release, dt)
        };
        self.lo = if x < self.lo {
            follow(self.lo, x, attack, dt)
        } else {
            follow(self.lo, x, release, dt)
        };
        let gap = (self.hi - self.lo).max(span.max(1e-9));
        ((x - self.lo) / gap).clamp(0.0, 1.0)
    }
}

struct AutoGain {
    /// `None` until the first tick, which starts both trackers at the input.
    range: Option<Range>,
}

impl CpuNode for AutoGain {
    fn reset(&mut self) {
        self.range = None;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let x = ctx.input(id, "input");
        let range = self.range.get_or_insert(Range { lo: x, hi: x });
        let y = range.step(
            x,
            ctx.input(id, "attack"),
            ctx.input(id, "release"),
            ctx.input(id, "span"),
            f64::from(ctx.dt),
        );
        ctx.publish(id, "output", y);
    }

    fn debug(&self) -> Option<String> {
        let r = self.range?;
        Some(format!("autogain floor {:.3} ceiling {:.3}", r.lo, r.hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn a_peak_reads_one_and_silence_reads_zero() {
        let mut r = Range { lo: 0.0, hi: 0.0 };
        let mut peak = 0.0f64;
        // Ramp up over half a second, hold, then drop to silence.
        for i in 0..30 {
            peak = peak.max(r.step(f64::from(i) / 10.0, 0.05, 2.0, 0.05, DT));
        }
        for _ in 0..30 {
            peak = peak.max(r.step(3.0, 0.05, 2.0, 0.05, DT));
        }
        assert!(peak > 0.95, "the loudest moment reads about 1: {peak}");
        let mut y = 1.0;
        for _ in 0..60 {
            y = r.step(0.0, 0.05, 2.0, 0.05, DT);
        }
        assert!(y < 0.05, "silence reads about 0: {y}");
        assert!(r.hi > 1.0, "the ceiling releases slowly: {}", r.hi);
    }

    #[test]
    fn a_quiet_room_is_not_stretched_to_full_scale() {
        let mut r = Range { lo: 0.0, hi: 0.0 };
        let mut y = 0.0;
        for i in 0..120 {
            // A wobble of a hundredth around a hundredth.
            let x = 0.01 + 0.01 * (f64::from(i) * 0.3).sin();
            y = r.step(x, 0.05, 2.0, 0.05, DT);
        }
        assert!(y < 0.5, "under the span it stays small: {y}");
    }
}
