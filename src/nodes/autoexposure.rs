// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hold a picture at a target brightness.
//!
//! Both halves. It measures its input the way `tap` does — over the unit square, at the
//! grid a tap offers no choice about — and its pass-through multiplies by a gain. The CPU
//! half reads the measurement back a frame later, computes the gain that would bring the
//! input's mean luminance to `target`, slews toward it in log space at `speed`, clamps it
//! between `min` and `max`, and publishes it — which is the uniform number the WGSL
//! multiplies by.
//!
//! Open loop on the input, deliberately: `gain = target / measured` has no dynamics to tune,
//! and the slew is the only time constant. Inside a video feedback loop the input is last
//! frame's output, so the loop is held at the target instead of running away to white or
//! dying to black.
//!
//! **The slew leads with part of each change in the gain it wants.** Inside a loop that adds
//! light, a dim source leaves the frame almost all its own past: each frame's input is the
//! last one's times the gain, so the log of the input integrates the log of the gain, and a
//! plain slew toward `target / measured` on top of that is a spring with almost no damping:
//! at a Response of 2 to 4 and Max gain 8 it swings the mean between a fifth and a half of
//! white about every two seconds. Each tick the gain also takes `√a − a` of the change in what
//! it wants, `a` the slew's fraction, which makes that spring half-damped whatever the
//! Response; off a loop it only answers a step in the input sooner, `√a` of it at once, and
//! never overshoots. The reading is a frame or more late, so a Response under a second can
//! still outrun it in a loop over a dim source; Max gain 1, where the loop cannot grow, keeps
//! that to a ripple, and the tooltip says so.

use crate::compile::{CompileContext, TAP_WORDS, TapKind};
use crate::graph::NodeId;
use crate::graph::PortType::{UniformNumber, VaryingColor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
    decompose, tap,
};

pub static DEF: NodeDef = NodeDef {
    slug: "autoexposure",
    category: Category::Effect,
    icon: "🌗",
    label: "Auto Exposure",
    tooltip: "Scales its input so its mean brightness sits at the target. Holds a feedback \
              loop at a level instead of letting it run away or die. In a loop that adds light, \
              a Response under a second can still swing, faster with a dimmer source; a slower \
              Response or a Max gain of 1 holds it.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "target",
            label: "Target",
            ty: UniformNumber,
            control: Control::num(0.3, 0.0, 4.0, 0.01, ""),
        },
        InputDef {
            key: "speed",
            label: "Response",
            ty: UniformNumber,
            control: Control::num(0.5, 0.0, 10.0, 0.01, "s"),
        },
        InputDef {
            key: "min",
            label: "Min gain",
            ty: UniformNumber,
            control: Control::num_log(0.05, 0.001, 1.0, 0.001, "x"),
        },
        InputDef {
            key: "max",
            label: "Max gain",
            ty: UniformNumber,
            control: Control::num_log(8.0, 1.0, 100.0, 0.01, "x"),
        },
    ],
    outputs: &[
        OutputDef {
            key: "output",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _func| {
                let input = ctx.input(node, "input", "uv");
                let gain = ctx.own_uniform(node, "gain");
                format!("    let color = {input};\n    return vec4f(color.rgb * {gain}, color.a);")
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "gain",
            label: "Gain",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "luma",
            label: "Luma",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || {
            Box::new(AutoExposure {
                gain: 1.0,
                luma: 0.0,
                wanted: None,
            })
        },
        integrates: true,
        live: false,
    }),
    measure_wgsl: Some(measure_wgsl),
    ..NodeDef::EMPTY
};

/// The exposure's measurement: the mean luminance of its input over the grid.
///
/// Luminance, and only luminance: an exposure is a statement about brightness, so this is
/// not the picker a `tap` carries. The expression is the same table entry the `luminosity`
/// node is, over the `color` declared here, which is the input's own channels as that node
/// reads them; the gain multiplies those same channels, so it lands the brightness it measured. No jitter either — a gain that wanders with the
/// dither of its own measurement is a gain that breathes.
fn measure_wgsl(node: NodeId, ctx: &mut CompileContext) {
    let input = ctx.input(node, "input", "p");
    let base = ctx.tap_slot(node, TapKind::Stats) * TAP_WORDS;
    let luma = decompose::find("luminosity")
        .expect("luminosity is in the table")
        .wgsl;
    let body = format!(
        "    let color = unpremultiply({input});\n{}",
        tap::stats_wgsl(base, luma, "p")
    );
    ctx.measure_grid(node, tap::DEFAULT_GRID, None, &body);
}

/// The gain that brings `luma` to `target`, clamped between `min` and `max`.
pub fn wanted_gain(luma: f32, target: f32, min: f32, max: f32) -> f32 {
    let (min, max) = (min.max(1e-6), max.max(1e-6));
    (target / luma.max(1e-6)).clamp(min, max)
}

/// One step of the controller, in log space: `gain` approaches `wanted` at `speed` seconds,
/// a fraction `a` of the gap a tick, and takes `√a − a` of the change from `last`, the gain
/// it wanted the tick before, on top. Clamped. Separate from the node so it is plain
/// arithmetic.
pub fn next_gain(
    gain: f32,
    wanted: f32,
    last: Option<f32>,
    speed: f32,
    min: f32,
    max: f32,
    dt: f32,
) -> f32 {
    let (min, max) = (min.max(1e-6), max.max(1e-6));
    let a = if speed > 0.0 {
        1.0 - (-dt / speed).exp()
    } else {
        1.0
    };
    let log = |x: f32| x.max(1e-6).ln();
    let lead = last.map_or(0.0, |last| (a.sqrt() - a) * (log(wanted) - log(last)));
    let g = (log(gain) + (log(wanted) - log(gain)) * a + lead).exp();
    g.clamp(min, max)
}

struct AutoExposure {
    gain: f32,
    luma: f32,
    /// The gain the last reading wanted, which the next one's lead is measured from.
    wanted: Option<f32>,
}

impl CpuNode for AutoExposure {
    fn reset(&mut self) {
        self.gain = 1.0;
        self.luma = 0.0;
        self.wanted = None;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // Nothing read back yet: leave the gain where it is rather than chasing a zero.
        if let Some(s) = ctx.readback(id).map(tap::decode)
            && s.count > 0
        {
            self.luma = s.mean;
            let (min, max) = (ctx.input(id, "min") as f32, ctx.input(id, "max") as f32);
            let wanted = wanted_gain(s.mean, ctx.input(id, "target") as f32, min, max);
            self.gain = next_gain(
                self.gain,
                wanted,
                self.wanted,
                ctx.input(id, "speed") as f32,
                min,
                max,
                ctx.dt,
            );
            self.wanted = Some(wanted);
        }
        ctx.publish(id, "gain", f64::from(self.gain));
        ctx.publish(id, "luma", f64::from(self.luma));
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "autoexposure luma {:.3} gain {:.3}",
            self.luma, self.gain
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tick of the node's controller on a reading of `luma`.
    fn tick(gain: f32, last: &mut Option<f32>, luma: f32, speed: f32, max: f32) -> f32 {
        let wanted = wanted_gain(luma, 0.5, 0.05, max);
        let g = next_gain(gain, wanted, *last, speed, 0.05, max, 1.0 / 60.0);
        *last = Some(wanted);
        g
    }

    #[test]
    fn the_gain_converges_on_target_over_measured() {
        let (mut g, mut last) = (1.0, None);
        for _ in 0..600 {
            g = tick(g, &mut last, 0.25, 0.5, 8.0);
        }
        assert!((g - 2.0).abs() < 0.01, "0.25 to 0.5 needs a gain of 2: {g}");
    }

    #[test]
    fn the_gain_is_clamped_and_black_does_not_blow_up() {
        let g = next_gain(
            1.0,
            wanted_gain(0.0, 0.5, 0.05, 8.0),
            None,
            0.0,
            0.05,
            8.0,
            1.0 / 60.0,
        );
        assert_eq!(g, 8.0, "black asks for infinity and gets the maximum");
        let g = next_gain(
            1.0,
            wanted_gain(100.0, 0.5, 0.05, 8.0),
            None,
            0.0,
            0.05,
            8.0,
            1.0 / 60.0,
        );
        assert_eq!(g, 0.05);
    }

    #[test]
    fn speed_zero_snaps() {
        let wanted = wanted_gain(0.25, 0.5, 0.05, 8.0);
        assert!((next_gain(1.0, wanted, Some(1.0), 0.0, 0.05, 8.0, 0.0) - 2.0).abs() < 1e-6);
    }

    /// A step in the input is answered at once in part and never past where it lands: the
    /// gain falls monotonically from 2 to 1 when the input doubles.
    #[test]
    fn a_step_off_a_loop_never_overshoots() {
        for speed in [0.05, 0.5, 2.0, 4.0] {
            let (mut g, mut last) = (1.0, None);
            for _ in 0..2000 {
                g = tick(g, &mut last, 0.25, speed, 8.0);
            }
            let mut before = g;
            for _ in 0..2000 {
                g = tick(g, &mut last, 0.5, speed, 8.0);
                assert!(
                    g <= before + 1e-6 && g >= 1.0 - 1e-4,
                    "{speed}: {before} then {g}"
                );
                before = g;
            }
            assert!((g - 1.0).abs() < 1e-3, "{speed}: {g}");
        }
    }

    /// **A loop that adds light holds at its target.** A dim source added to last frame's
    /// output through the gain, measured two frames late: the frame is nearly all its own
    /// past, so the log of the input integrates the log of the gain. At Max gain 8 and a
    /// Response of 1 to 4 seconds a plain slew swings the mean across two orders of
    /// magnitude; with its lead it settles within a percent of the target.
    #[test]
    fn a_feedback_loop_that_adds_light_settles() {
        let late = 2;
        for source in [0.02, 0.005, 0.001] {
            for speed in [1.0, 2.0, 4.0] {
                let (mut g, mut last) = (1.0, None);
                let (mut out, mut inputs) = (0.0_f32, Vec::new());
                let mut tail = (f32::MAX, f32::MIN);
                let frames = 60 * 60;
                for frame in 0..frames {
                    let input = source + out;
                    out = g * input;
                    inputs.push(input);
                    if frame >= late {
                        g = tick(g, &mut last, inputs[frame - late], speed, 8.0);
                    }
                    if frame >= frames - 600 {
                        tail = (tail.0.min(out), tail.1.max(out));
                    }
                }
                assert!(
                    tail.0 > 0.495 && tail.1 < 0.505,
                    "source {source}, Response {speed}: the last ten seconds swing {tail:?}"
                );
            }
        }
    }
}
