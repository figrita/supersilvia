// SPDX-License-Identifier: AGPL-3.0-or-later

//! A waveform over time, on the CPU. silvia's node, which is where it belongs: the value is
//! one number per frame, so it is a `UniformNumber` — and a uniform number feeds a
//! `VaryingNumber` for free, so it still reaches a shader with no plumbing of its own.
//!
//! **It reads Time and Offset, in waves, and keeps nothing.** Unplugged, Time is ambient time
//! at one wave a second, silvia's default Frequency of 1 Hz; a gear cabled in replaces it, and
//! is where a frequency is set, turned, stopped or restarted — silvia's Frequency, Start/Stop
//! and Reset are a Ratio Gear's Ratio, Hold and Reset now. Offset is added, in waves: a slow
//! wave on it modulates the phase of this one. A one-shot is `animation`'s. The value is a
//! function of `Time + Offset`, so it is the same wherever the show was sought or played to.
//!
//! **The waveform formulas are silvia's**, not the ones the field version emitted: a triangle
//! starts at −1, a sawtooth starts at 0 rising, a square is a comparison against π rather
//! than `sign(sin)` and so never reads 0 at a crossing, and a pulse is a quarter duty. **Noise
//! draws a fresh value each wave**, keyed by `floor(Time + Offset)` and the node's id, so it
//! holds for a wave, loops when its Time does and is the same twice. The field version — an
//! oscillator whose frequency and phase could themselves be pictures — is cut, and recorded
//! in `proposals/field-oscillator.md`. **Level** is silvia's Offset, the level added to the
//! wave, relabelled so Offset means one thing across the library.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::cpu::TraceRing;
use crate::nodes::{
    Ambient, Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind,
    OutputDef, OutputKind, TickContext, phasor, rng::Rng,
};
use std::f32::consts::{PI, TAU};

pub static DEF: NodeDef = NodeDef {
    slug: "oscillator",
    category: Category::Control,
    icon: "👋",
    label: "Oscillator",
    tooltip: "A waveform, a wave a second or as a gear in Time turns it, with a trace of what \
              it published. Offset shifts it along, in waves; Level lifts it.",
    // The waveform is the thing being edited; see docs/decisions.md#a-trace-is-a-picture-of-
    // the-shape-a-node-is-editing. 300 is `canvas::SCOPE_NODE_WIDTH`, which `nodes/` cannot
    // name — it is a graphical dependency this crate does not take.
    regions: &[crate::nodes::Region::Trace],
    ambient: Some(Ambient::periodic(1.0)),
    inputs: &[
        InputDef {
            key: crate::nodes::TIME,
            label: "Time",
            ty: UniformNumber,
            // No knob: unplugged, Time is ambient time at a wave a second.
            control: Control::None,
        },
        InputDef {
            key: phasor::OFFSET,
            label: "Offset",
            ty: UniformNumber,
            // In waves, added to Time: zero is silvia's wave.
            control: phasor::offset_control(),
        },
        InputDef {
            key: "amplitude",
            label: "Amplitude",
            ty: UniformNumber,
            control: Control::num(1.0, 0.0, 10.0, 0.01, ""),
        },
        InputDef {
            key: "offset",
            label: "Level",
            ty: UniformNumber,
            control: Control::num(0.0, -10.0, 10.0, 0.01, ""),
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
            key: "waveform",
            label: "Waveform",
            default: "sine",
            choices: &[
                ("sine", "Sine"),
                ("cosine", "Cosine"),
                ("triangle", "Triangle"),
                ("square", "Square"),
                ("sawtooth", "Sawtooth"),
                ("pulse", "Pulse"),
                ("noise", "Noise"),
            ],
            // Read by `tick`. The output is a uniform number, so this node emits no
            // WGSL for an option to change.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_TRACE,
        crate::nodes::SHOW_TIME,
    ],
    row_headings: &[crate::nodes::SHOW_TIME.key],
    cpu: Some(CpuDef {
        create: || Box::new(Oscillator::default()),
        // The trace is a ring of every frame before this one.
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// How many seconds the trace holds: silvia's `historySize` of 300 for the same picture, at
/// the 60 Hz it was drawn at. Seconds rather than samples, since the band draws by time.
const TRACE_SPAN: f32 = 5.0;

struct Oscillator {
    value: f32,
    /// The last `TRACE_SPAN` seconds of published values, dated, for the trace on the body.
    history: TraceRing,
}

impl Default for Oscillator {
    fn default() -> Self {
        Self {
            value: 0.0,
            history: TraceRing::new(TRACE_SPAN),
        }
    }
}

/// The waveform at `t` waves, silvia's formulas exactly; noise one value a wave, keyed by the
/// wave and the node.
fn wave(waveform: &str, t: f64, id: NodeId) -> f32 {
    let p = phasor::fraction(t, 1.0) as f32 * TAU;
    // Cycles within the current one, which the triangle and the sawtooth are written in.
    let x = p / TAU;
    match waveform {
        "cosine" => p.cos(),
        "triangle" => 2.0 * (2.0 * (x - (x + 0.5).floor())).abs() - 1.0,
        "square" => {
            if p < PI {
                1.0
            } else {
                -1.0
            }
        }
        "sawtooth" => 2.0 * (x - (x + 0.5).floor()),
        "pulse" => {
            if p % TAU < PI * 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        "noise" => {
            // A word of its own for each wave: the wave's number mixed with the node's id.
            let n = (t.floor() as i64).rem_euclid(1 << 30) as u32;
            let mut rng = Rng::from_word(n ^ id.0.wrapping_mul(0x9E37_79B9));
            rng.next_f32() * 2.0 - 1.0
        }
        _ => p.sin(),
    }
}

impl CpuNode for Oscillator {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{:.3}", self.value))
    }

    fn trace(&self) -> Option<&TraceRing> {
        Some(&self.history)
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let amplitude = ctx.input(id, "amplitude");
        let level = ctx.input(id, "offset");
        let t = ctx.clock(id) + f64::from(ctx.input(id, phasor::OFFSET));
        self.value = wave(ctx.option(id, "waveform"), t, id) * amplitude + level;
        ctx.publish(id, "output", self.value);
        self.history.push(ctx.elapsed, self.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trace is empty until the node ticks: the band is *nothing yet*, not a zero.
    #[test]
    fn the_trace_is_empty_before_the_first_tick() {
        let node = Oscillator::default();
        assert!(
            node.trace()
                .expect("an oscillator always answers")
                .is_empty()
        );
    }

    /// Where silvia's formulas start, which is not where the field version's did: a triangle
    /// at −1, a sawtooth at 0, a square at +1 rather than at `sign(0)`.
    #[test]
    fn every_waveform_starts_where_silvias_does() {
        for (waveform, expected) in [
            ("sine", 0.0),
            ("cosine", 1.0),
            ("triangle", -1.0),
            ("square", 1.0),
            ("sawtooth", 0.0),
            ("pulse", 1.0),
        ] {
            let value = wave(waveform, 0.0, NodeId(1));
            assert!(
                (value - expected).abs() < 1e-6,
                "{waveform} starts at {value}, expected {expected}"
            );
        }
    }

    /// Noise holds one value a wave and draws a fresh one the next, inside the range every
    /// other waveform keeps, and is the same value at the same wave however it got there.
    #[test]
    fn noise_is_one_fresh_value_a_wave_and_the_same_twice() {
        let id = NodeId(7);
        let waves: Vec<f32> = (0..8)
            .map(|n| wave("noise", f64::from(n) + 0.3, id))
            .collect();
        assert!(waves.iter().all(|v| (-1.0..=1.0).contains(v)), "{waves:?}");
        assert!(
            waves.windows(2).all(|w| w[0] != w[1]),
            "a fresh value each wave: {waves:?}"
        );
        assert_eq!(
            wave("noise", 3.1, id),
            wave("noise", 3.9, id),
            "held for a wave"
        );
        assert_eq!(wave("noise", 3.1, id), waves[3], "and the same twice");
        assert_ne!(
            wave("noise", 3.1, NodeId(8)),
            waves[3],
            "another node, another noise"
        );
    }
}
