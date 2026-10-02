// SPDX-License-Identifier: AGPL-3.0-or-later

//! A node that exists only for unit tests: a minimal fixture with two `Control::None` inputs
//! on one varying number output, and carrying one option of each kind.
//! Registered in `REGISTRY` under `cfg(test)` only.
//!
//! The three options are the fixture for the recompile boundary. A `Code` option rebuilds and
//! the other two must not, and asserting that needs one node whose three options differ in
//! nothing else — the production nodes that want a `Uniform` option are the mixer and what
//! follows it, and the rule has to be tested before they are written rather than after.

use crate::graph::PortType::VaryingNumber;
use crate::nodes::{
    Accepts, Category, Control, InputDef, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
};

pub static DEF: NodeDef = NodeDef {
    slug: "probe",
    icon: "⏱",
    label: "Probe",
    tooltip: "Adds or subtracts its two inputs.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingNumber,
            // Unconnected adds nothing; connect anything and it is a call into the graph.
            control: Control::None,
        },
        InputDef {
            key: "bare",
            label: "Bare",
            ty: VaryingNumber,
            control: Control::None,
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingNumber,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let t = ctx.input(node, "input", "uv");
            let b = ctx.input(node, "bare", "uv");
            // The `Code` option picks the operator while emitting; the `Uniform` one leaves
            // both branches in the program and chooses between them with an `i32`.
            let op = match ctx.option(node, "op") {
                "sub" => "-",
                _ => "+",
            };
            let scale = ctx.option_uniform(node, "scale");
            format!("    return ({t} {op} {b}) * select(1.0, 2.0, {scale} == 1);")
        },
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "op",
            label: "Operator",
            default: "add",
            choices: &[("add", "Add"), ("sub", "Subtract")],
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "scale",
            label: "Scale",
            default: "one",
            choices: &[("one", "1x"), ("two", "2x")],
            kind: OptionKind::Uniform,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "file",
            label: "File",
            default: "",
            choices: &[],
            kind: OptionKind::Asset,
            accepts: Accepts::VIDEO,
            ..OptionDef::EMPTY
        },
    ],
    // A varying number in and a varying number out, which is what the Math group is;
    // `NodeDef::EMPTY` would otherwise file it under Effect, where a node that draws no
    // picture does not belong.
    category: Category::Math,
    ..NodeDef::EMPTY
};
