// SPDX-License-Identifier: AGPL-3.0-or-later

//! One named basis mapped onto another.
//!
//! Ported from silvia's `sliderule.js`. It is `reframerange`'s map with the four bounds
//! picked as two names instead of typed: 0 to 1, 0 to 360, -1 to 1, 0 to 255 and 0 to 2π are
//! the conversions a patch asks for over and over, and *From* and *To* are two picks where
//! the general node is four entries. The two **Invert** options are silvia's pair of
//! checkboxes and cancel the way silvia's do — one of them turns the run round, both of them
//! leave it alone — because the invert is applied to the normalized number between the two
//! bases, once, where the two answers differ.
//!
//! **Dual, like the `math` family.** Its one input is a number, so an instance fed by a knob
//! or a diamond maps a number onto a number on the CPU and publishes a uniform number. See
//! [Dual outputs](../../docs/nodes.md#dual-outputs).
//!
//! The bases are chosen at compile time rather than carried as the `int` uniforms silvia
//! passed a branching function: an option here is `OptionKind::Code`, so the shader that is
//! built is the one arithmetic the picks name and the branch never reaches the GPU.

use crate::graph::{NodeId, PortType::VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, TickContext,
};

node! {
    /// silvia's five bases, either end, with a per-side invert.
    SLIDERULE,
    slug: "sliderule",
    icon: "📏",
    label: "Slide Rule",
    category: Convert,
    tooltip: "Maps a number from one named range onto another — degrees, bytes, turns. \
              Invert on one side runs it backwards; on both it cancels.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "inBasis" "From" = "unit" [
            "unit" => "0 to 1",
            "degrees" => "0 to 360",
            "signed" => "-1 to 1",
            "byte" => "0 to 255",
            "turns" => "0 to 2π",
        ],
        "outBasis" "To" = "signed" [
            "unit" => "0 to 1",
            "degrees" => "0 to 360",
            "signed" => "-1 to 1",
            "byte" => "0 to 255",
            "turns" => "0 to 2π",
        ],
        "inInvert" "Invert From" = "off" [
            "off" => "Off",
            "on" => "On",
        ],
        "outInvert" "Invert To" = "off" [
            "off" => "Off",
            "on" => "On",
        ],
    ],
    outputs: [
        VaryingNumber "output" "Output" eval(evaluate) = varying(|node, ctx| {
            // The two bases' arithmetic: products and quotients of `f32`s and the prelude's
            // `PI`.
            let normalize = normalize_wgsl(ctx.option(node, "inBasis"));
            let expand = expand_wgsl(ctx.option(node, "outBasis"));
            // `n` is a `var` only where the invert assigns it again.
            let (binding, invert) =
                if inverted(ctx.option(node, "inInvert"), ctx.option(node, "outInvert")) {
                    ("var", "    n = 1.0 - n;\n")
                } else {
                    ("let", "")
                };
            format!(
                "    let val = {{input}};
    {binding} n = {normalize};
{invert}    return {expand};"
            )
        }),
    ],
}

/// A basis read as a normalization of `val` onto 0 to 1, in WGSL.
fn normalize_wgsl(basis: &str) -> &'static str {
    match basis {
        "degrees" => "val / 360.0",
        "signed" => "(val + 1.0) / 2.0",
        "byte" => "val / 255.0",
        "turns" => "val / (2.0 * PI)",
        // 0 to 1 is already normalized.
        _ => "val",
    }
}

/// A basis read as an expansion of the normalized `n` back out of 0 to 1, in WGSL.
fn expand_wgsl(basis: &str) -> &'static str {
    match basis {
        "degrees" => "n * 360.0",
        "signed" => "(n * 2.0) - 1.0",
        "byte" => "n * 255.0",
        "turns" => "n * (2.0 * PI)",
        _ => "n",
    }
}

/// Whether the run is turned round: one side inverted, not both.
fn inverted(in_invert: &str, out_invert: &str) -> bool {
    (in_invert == "on") != (out_invert == "on")
}

/// The body above, in Rust, for the instance whose input is a uniform number.
///
/// WGSL semantics: `PI` is the prelude's `3.14159265359`, which is `std::f32::consts::PI` to
/// every bit an `f32` carries, and every other term is a multiply or a divide that says the
/// same thing in both languages. `f32::midpoint` stands in for the shader's `(val + 1.0) /
/// 2.0`, which it rounds to the same number over any input a control holds.
fn evaluate(node: NodeId, ctx: &TickContext<'_>) -> f32 {
    use std::f32::consts::PI;

    let val = ctx.input(node, "input");
    let mut n = match ctx.option(node, "inBasis") {
        "degrees" => val / 360.0,
        "signed" => f32::midpoint(val, 1.0),
        "byte" => val / 255.0,
        "turns" => val / (2.0 * PI),
        _ => val,
    };
    if inverted(ctx.option(node, "inInvert"), ctx.option(node, "outInvert")) {
        n = 1.0 - n;
    }
    match ctx.option(node, "outBasis") {
        "degrees" => n * 360.0,
        "signed" => (n * 2.0) - 1.0,
        "byte" => n * 255.0,
        "turns" => n * (2.0 * PI),
        _ => n,
    }
}
