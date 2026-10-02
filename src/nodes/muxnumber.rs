// SPDX-License-Identifier: AGPL-3.0-or-later

//! Picks one of four colors from a number, and will fade between the two either side of it.
//!
//! silvia's `muxnumber.js`, and the sibling of [`muxevent`](super::muxevent): the same four
//! inputs with a number where that node's buttons are. It is the four-way switch a live
//! patch is built on — a counter, a sequencer or an envelope into `select` and the picture
//! changes with it — and it is where the crossfade that was cut out of `muxevent` belongs,
//! because here the selector is continuous to begin with.
//!
//! **The two modes are not symmetric, and that is silvia's shape.** Switch takes the whole
//! number and wraps it round the four, so a counter that runs on past the fourth comes back
//! to the first. Crossfade clamps instead, and mixes channel *i* into channel *i + 1* by the
//! fraction, with the fourth fading into itself — so it stops at the fourth rather than
//! sweeping back round to the first. The wrap is what a switch wants and the clamp is what a
//! fader wants, and silvia is right to have written them differently.
//!
//! `mode` is [`OptionKind::Uniform`], which is the case
//! [decisions.md](../../../docs/decisions.md) names for the kind: both branches are cheap,
//! both are already in the program, and a fade turned into a cut in the middle of a set must
//! not cost a shader rebuild. silvia recompiles here because a browser gave it no cheaper
//! answer.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
};

pub static DEF: NodeDef = NodeDef {
    slug: "muxnumber",
    category: Category::Color,
    icon: "👉",
    label: "Mux (Number)",
    tooltip: "Picks between 4 color inputs from a number. Switch wraps the number round the \
              four; Crossfade clamps it and blends the two either side of it.",
    inputs: &[
        InputDef {
            key: "input0",
            label: "Input 1",
            ty: VaryingColor,
            control: Control::color("#ff0000ff"),
        },
        InputDef {
            key: "input1",
            label: "Input 2",
            ty: VaryingColor,
            control: Control::color("#00ff00ff"),
        },
        InputDef {
            key: "input2",
            label: "Input 3",
            ty: VaryingColor,
            control: Control::color("#0000ffff"),
        },
        InputDef {
            key: "input3",
            label: "Input 4",
            ty: VaryingColor,
            control: Control::color("#ffff00ff"),
        },
        InputDef {
            key: "select",
            label: "Select",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 3.0, 1.0, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        // The four in an array indexed by the channel, since WGSL has no `?:`. The
        // crossfade's upper channel is the next one up, and the fourth's is itself.
        wgsl: |node, ctx, _func| {
            let c0 = ctx.input(node, "input0", "uv");
            let c1 = ctx.input(node, "input1", "uv");
            let c2 = ctx.input(node, "input2", "uv");
            let c3 = ctx.input(node, "input3", "uv");
            let select = ctx.input(node, "select", "uv");
            let mode = ctx.option_uniform(node, "mode");
            format!(
                "    var colors = array<vec4f, 4>({c0}, {c1}, {c2}, {c3});
    let s = {select};
    if ({mode} == 1) {{
        let f = clamp(s, 0.0, 3.0);
        let i = i32(floor(f));
        return mix(colors[i], colors[min(i + 1, 3)], fract(f));
    }}
    let channel = i32(floor_mod(floor(s), 4.0));
    return colors[channel];"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[OptionDef {
        key: "mode",
        label: "Mode",
        default: "switch",
        choices: &[("switch", "Switch"), ("crossfade", "Crossfade")],
        kind: OptionKind::Uniform,
        ..OptionDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
