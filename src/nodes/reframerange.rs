// SPDX-License-Identifier: AGPL-3.0-or-later

//! One range mapped onto another.
//!
//! Ported from silvia's `reframerange.js`. Without it, matching a source's range to a
//! parameter's is three `math` nodes and a lie about what the patch is doing — and every one
//! of the four bounds is a port, so a `UniformNumber` drives them for free and the window
//! itself can be modulated.
//!
//! **Dual, like the `math` family.** Every one of its five bounds is a number, so an
//! instance fed by knobs and diamonds maps a number onto a number on the CPU and publishes a
//! uniform number — which is what makes it the node that fits a source's range to a
//! `UniformNumber` parameter's rather than only to a field's. See
//! [Dual outputs](../../docs/nodes.md#dual-outputs).
//!
//! **silvia's `sliderule` is ported beside it**, in `sliderule.rs`, and the five bases it
//! picks between are on this node too — as [`Region::Ranges`], two rows of buttons
//! that write the bounds and let go. That is where the speed of silvia's second node came
//! from, and it costs nothing here: the bounds stay knobs with ports, so a named range is a
//! starting point a cable can still drive off. This node keeps `sliderule`'s filing as well,
//! `Convert` — it converts, and a silvia patcher opens that heading first.

use crate::graph::{NodeId, PortType::VaryingNumber};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind, Region,
    TickContext,
};

// The four bounds the named ranges write, which `widgets::ranges` reads from here.

pub const IN_MIN: &str = "inMin";
pub const IN_MAX: &str = "inMax";
pub const OUT_MIN: &str = "outMin";
pub const OUT_MAX: &str = "outMax";

/// The heading over the named ranges, which is presentation and nothing else: what it changes
/// is how tall the node is drawn.
pub const SHOW_NAMED: OptionDef =
    OptionDef::heading("named", "Named Ranges", true, OptionKind::Presentation);

/// `in[min, max]` onto `out[min, max]`, extrapolating unless it is told to clamp.
///
/// Written out rather than declared with `node!`: the macro is for a node that is a port
/// list, an option list and one body per output, and this one has grown a region and the
/// heading over it.
pub static REFRAMERANGE: NodeDef = NodeDef {
    slug: "reframerange",
    icon: "⇆",
    label: "Reframe Range",
    tooltip: "Maps a number from one range onto another. Outside the input range it keeps \
              going, unless Clamp is on; putting Out Min above Out Max turns it round.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingNumber,
            control: Control::num(0.5, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: IN_MIN,
            label: "Input Min",
            ty: VaryingNumber,
            control: Control::num(0.0, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: IN_MAX,
            label: "Input Max",
            ty: VaryingNumber,
            control: Control::num(1.0, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: OUT_MIN,
            label: "Output Min",
            ty: VaryingNumber,
            control: Control::num(-1.0, -1000.0, 1000.0, 0.01, ""),
        },
        InputDef {
            key: OUT_MAX,
            label: "Output Max",
            ty: VaryingNumber,
            control: Control::num(1.0, -1000.0, 1000.0, 0.01, ""),
        },
    ],
    options: &[
        OptionDef {
            key: "clamp",
            label: "Clamp Output",
            default: crate::nodes::OFF,
            choices: crate::nodes::CHECK_CHOICES,
            checkbox: true,
            ..OptionDef::EMPTY
        },
        SHOW_NAMED,
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingNumber,
        kind: OutputKind::Shader,
        eval: Some(evaluate),
        wgsl: |node, ctx, _func| {
            let input = ctx.input(node, "input", "uv");
            let in_min = ctx.input(node, IN_MIN, "uv");
            let in_max = ctx.input(node, IN_MAX, "uv");
            let out_min = ctx.input(node, OUT_MIN, "uv");
            let out_max = ctx.input(node, OUT_MAX, "uv");
            let tail = if ctx.option(node, "clamp") == crate::nodes::ON {
                "    return clamp(mapped, min(outMin, outMax), max(outMin, outMax));"
            } else {
                "    return mapped;"
            };
            format!(
                "    let inMin = {in_min};
    let outMin = {out_min};
    let outMax = {out_max};
    let inRange = ({in_max}) - inMin;
    if (abs(inRange) < 1e-5) {{ return outMin; }}
    let mapped = mix(outMin, outMax, (({input}) - inMin) / inRange);
{tail}"
            )
        },
        ..OutputDef::EMPTY
    }],
    regions: &[Region::Ranges],
    category: Category::Convert,
    ..NodeDef::EMPTY
};

/// The body above, in Rust, for the instance whose every input is a uniform number.
///
/// WGSL semantics where they differ: `mix` extrapolates outside 0..1 the same way, and
/// `clamp`'s two bounds are already ordered by `min`/`max`, so the Rust one cannot be handed
/// an inside-out pair.
fn evaluate(node: NodeId, ctx: &TickContext<'_>) -> f32 {
    let in_min = ctx.input(node, "inMin");
    let out_min = ctx.input(node, "outMin");
    let out_max = ctx.input(node, "outMax");
    let in_range = ctx.input(node, "inMax") - in_min;
    if in_range.abs() < 1e-5 {
        return out_min;
    }
    let t = (ctx.input(node, "input") - in_min) / in_range;
    // WGSL's `mix`, term for term, rather than the algebraically equal `a + (b - a) * t`.
    let mapped = out_min * (1.0 - t) + out_max * t;
    if ctx.option(node, "clamp") == "on" {
        mapped.clamp(out_min.min(out_max), out_min.max(out_max))
    } else {
        mapped
    }
}
