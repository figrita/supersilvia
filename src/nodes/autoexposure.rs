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
//! Open loop on the input, deliberately: `gain = target / measured` has no dynamics to tune
//! and cannot oscillate, and the slew is the only time constant. Inside a video feedback
//! loop the input is last frame's output, so the loop is held at the target instead of
//! running away to white or dying to black.

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
              loop at a level instead of letting it run away or die.",
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
/// node is, over the `color` declared here. No jitter either — a gain that wanders with the
/// dither of its own measurement is a gain that breathes.
fn measure_wgsl(node: NodeId, ctx: &mut CompileContext) {
    let input = ctx.input(node, "input", "p");
    let base = ctx.tap_slot(node, TapKind::Stats) * TAP_WORDS;
    let luma = decompose::find("luminosity")
        .expect("luminosity is in the table")
        .wgsl;
    let body = format!(
        "    let color = {input};\n{}",
        tap::stats_wgsl(base, luma, "p")
    );
    ctx.measure_grid(node, tap::DEFAULT_GRID, None, &body);
}

/// One step of the controller: the gain that brings `luma` to `target`, approached in log
/// space at `speed` seconds and clamped. Separate from the node so it is plain arithmetic.
pub fn next_gain(
    gain: f32,
    luma: f32,
    target: f32,
    speed: f32,
    min: f32,
    max: f32,
    dt: f32,
) -> f32 {
    let (min, max) = (min.max(1e-6), max.max(1e-6));
    let wanted = (target / luma.max(1e-6)).clamp(min, max);
    let alpha = if speed > 0.0 {
        1.0 - (-dt / speed).exp()
    } else {
        1.0
    };
    let g = (gain.max(1e-6).ln() + (wanted.ln() - gain.max(1e-6).ln()) * alpha).exp();
    g.clamp(min, max)
}

struct AutoExposure {
    gain: f32,
    luma: f32,
}

impl CpuNode for AutoExposure {
    fn reset(&mut self) {
        self.gain = 1.0;
        self.luma = 0.0;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // Nothing read back yet: leave the gain where it is rather than chasing a zero.
        if let Some(s) = ctx.readback(id).map(tap::decode)
            && s.count > 0
        {
            self.luma = s.mean;
            self.gain = next_gain(
                self.gain,
                s.mean,
                ctx.input(id, "target"),
                ctx.input(id, "speed"),
                ctx.input(id, "min"),
                ctx.input(id, "max"),
                ctx.dt,
            );
        }
        ctx.publish(id, "gain", self.gain);
        ctx.publish(id, "luma", self.luma);
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

    #[test]
    fn the_gain_converges_on_target_over_measured() {
        let mut g = 1.0;
        for _ in 0..600 {
            g = next_gain(g, 0.25, 0.5, 0.5, 0.05, 8.0, 1.0 / 60.0);
        }
        assert!((g - 2.0).abs() < 0.01, "0.25 to 0.5 needs a gain of 2: {g}");
    }

    #[test]
    fn the_gain_is_clamped_and_black_does_not_blow_up() {
        let g = next_gain(1.0, 0.0, 0.5, 0.0, 0.05, 8.0, 1.0 / 60.0);
        assert_eq!(g, 8.0, "black asks for infinity and gets the maximum");
        let g = next_gain(1.0, 100.0, 0.5, 0.0, 0.05, 8.0, 1.0 / 60.0);
        assert_eq!(g, 0.05);
    }

    #[test]
    fn speed_zero_snaps() {
        assert!((next_gain(1.0, 0.25, 0.5, 0.0, 0.05, 8.0, 0.0) - 2.0).abs() < 1e-6);
    }
}
