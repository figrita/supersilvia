// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sample a color at one point, where it flows.
//!
//! Like `tap`, the picture passes through, and the reading is a measurement beside it: one
//! call, made by the fragment at the corner of the frame, evaluating this node's input at
//! `(x, y)` exactly. No atomics and no tolerance — one writer, at the point asked for, in
//! the node's own coordinates, so a transform downstream moves the picture and not the
//! point. `x` and `y` are uniform number inputs, so the point can be driven. The CPU half
//! reads the slot a frame later and publishes the channels.
//!
//! The reading is published twice over: as one `color`, which is what was measured, and as
//! numbers beside it, for the arithmetic that wants one. A cable from `color` lands on any
//! color input and shows the sampled color on its swatch; the channels stay because a hue is
//! not a number and `r` is.
//!
//! The numbers are the four channels, a luma, and **hue, saturation and lightness**, which
//! are [`decompose::hsl`] over the color the slot already holds. They are here rather than
//! left to the `hue`, `saturation` and `lightness` nodes because those are shaders: each
//! turns a picture into a *field*, and collapsing one back to a uniform number takes a `tap`
//! over sixteen thousand grid points. That is the wrong instrument for a question about one
//! pixel the CPU is already holding.
//!
//! `luma` and `lightness` are both here and are not the same number: luma is Rec.709
//! perceived brightness, lightness is the midpoint of the brightest and darkest channel.

use crate::compile::{CompileContext, TAP_WORDS, TapKind};
use crate::graph::NodeId;
use crate::graph::PortType::{UniformColor, UniformNumber, VaryingColor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
    decompose, tap,
};

/// The sample's measurement: the color of its input at the point its two uniform numbers
/// name. The buffer's words are atomic, so a plain write is an `atomicStore`.
fn measure_wgsl(node: NodeId, ctx: &mut CompileContext) {
    let x = ctx.input(node, "x", "p");
    let y = ctx.input(node, "y", "p");
    let input = ctx.input(node, "input", &format!("vec2f(({x}), ({y}))"));
    let base = ctx.tap_slot(node, TapKind::Sample) * TAP_WORDS;
    ctx.measure_once(
        node,
        &format!(
            "    let c = {input};
    atomicStore(&tap[{base}u], 1u);
    atomicStore(&tap[{base}u + 1u], bitcast<u32>(c.r));
    atomicStore(&tap[{base}u + 2u], bitcast<u32>(c.g));
    atomicStore(&tap[{base}u + 3u], bitcast<u32>(c.b));
    atomicStore(&tap[{base}u + 4u], bitcast<u32>(c.a));"
        ),
    );
}

pub static DEF: NodeDef = NodeDef {
    slug: "sample",
    category: Category::Tap,
    icon: "🎯",
    label: "Sample",
    tooltip: "Passes its input through and reads the color at one point, as a uniform color \
              with its channels and its hue, saturation and lightness, one frame later.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "x",
            label: "X",
            ty: UniformNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
        InputDef {
            key: "y",
            label: "Y",
            ty: UniformNumber,
            control: Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
        },
    ],
    outputs: &[
        OutputDef {
            key: "output",
            // The tap's label, for the tap's reason: the row below it is a color too, and
            // this one is the input untouched.
            label: "Pass-Through",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _func| {
                let input = ctx.input(node, "input", "uv");
                format!("    return {input};")
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "color",
            label: "Color",
            ty: UniformColor,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "r",
            label: "R",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "g",
            label: "G",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "b",
            label: "B",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "a",
            label: "A",
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
        OutputDef {
            key: "hue",
            label: "Hue",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "saturation",
            label: "Saturation",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "lightness",
            label: "Lightness",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || {
            Box::new(Sample {
                status: tap::DORMANT.to_string(),
            })
        },
        integrates: false,
        live: false,
    }),
    measure_wgsl: Some(measure_wgsl),
    ..NodeDef::EMPTY
};

/// Decode one slot: the color, if the measurement ran.
pub fn decode(w: &[u32; TAP_WORDS]) -> Option<[f32; 4]> {
    (w[0] != 0).then(|| {
        [
            f32::from_bits(w[1]),
            f32::from_bits(w[2]),
            f32::from_bits(w[3]),
            f32::from_bits(w[4]),
        ]
    })
}

struct Sample {
    status: String,
}

/// The numbers a sample publishes beside its color: the four channels, then the four
/// readings of them, in the order `tick` computes them.
const PORTS: [&str; 8] = ["r", "g", "b", "a", "luma", "hue", "saturation", "lightness"];

impl CpuNode for Sample {
    fn reset(&mut self) {
        self.status = tap::DORMANT.to_string();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.status = if ctx.readback(id).is_some() {
            tap::MEASURING
        } else {
            tap::DORMANT
        }
        .to_string();
        // Nothing measured is nothing to say: the ports are withdrawn and their rows draw
        // nothing, rather than holding a color from a pass that no longer runs.
        let Some(color) = ctx.readback(id).and_then(decode) else {
            for port in PORTS {
                ctx.withdraw(id, port);
            }
            ctx.withdraw_color(id, "color");
            return;
        };
        let [r, g, b, a] = color;
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let [hue, saturation, lightness] = decompose::hsl(color);
        ctx.publish_color(id, "color", color);
        for (port, value) in PORTS
            .iter()
            .zip([r, g, b, a, luma, hue, saturation, lightness])
        {
            ctx.publish(id, port, value);
        }
    }

    fn status(&self) -> Option<String> {
        Some(self.status.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_slot_decodes_to_its_color_and_an_unwritten_one_to_nothing() {
        let mut w = crate::compile::TAP_TEMPLATE;
        w[0] = 1;
        w[1] = 0.25f32.to_bits();
        w[2] = 0.5f32.to_bits();
        w[3] = 1.0f32.to_bits();
        w[4] = 1.0f32.to_bits();
        assert_eq!(decode(&w), Some([0.25, 0.5, 1.0, 1.0]));
        assert_eq!(decode(&crate::compile::TAP_TEMPLATE), None);
    }
}
