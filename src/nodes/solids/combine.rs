// SPDX-License-Identifier: AGPL-3.0-or-later

//! The booleans: union, subtraction and intersection, each with a smoothing radius that blends
//! the colors across the seam as it blends the shapes.

use super::{SDF, fill, num, out, solid};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::nodes::{Category, Family, InputDef, NodeDef};
use std::fmt::Write as _;

const SMOOTH: InputDef = num("smooth", "Smooth", 0.0, 0.0, 2.0, 0.01, "⬓");

/// `op` folded over the connected ones of `keys`, the first always.
fn fold(node: NodeId, ctx: &mut CompileContext, op: &str, keys: &[&'static str]) -> String {
    let first = ctx.input(node, keys[0], "p");
    let mut body = format!("    let k = {{smooth}};\n    var r = {first};\n");
    for key in &keys[1..] {
        if keys.len() > 2 && !ctx.connected(node, key) {
            continue;
        }
        let next = ctx.input(node, key, "p");
        let _ = writeln!(body, "    r = {op}(r, {next}, k);");
    }
    body.push_str("    return r;");
    fill(node, ctx, &body)
}

pub static UNION: NodeDef = NodeDef {
    slug: "union",
    category: Category::Solid,
    family: Some(Family::Combine),
    icon: "⊕",
    label: "Union",
    tooltip: "Up to four solids as one. Smooth melts them together over that distance.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        solid("c", "C"),
        solid("d", "D"),
        SMOOTH,
    ],
    outputs: &[out(|node, ctx, _| {
        fold(node, ctx, "sd_union", &["a", "b", "c", "d"])
    })],
    bounds: Some(|n, ctx| {
        let mut b = ctx.solid_bounds(n, "a")?;
        for key in ["b", "c", "d"] {
            if ctx.connected(n, key) {
                let next = ctx.solid_bounds(n, key)?;
                b = format!("sd_bounds_union({b}, {next})");
            }
        }
        ctx.uniform_fill(n, &format!("sd_bounds_grow({b}, {{smooth}})"))
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static SUBTRACT: NodeDef = NodeDef {
    slug: "solidsubtract",
    category: Category::Solid,
    family: Some(Family::Combine),
    icon: "⊖",
    label: "Subtract",
    tooltip: "A with B carved out of it. The cut takes B's color.",
    inputs: &[solid("a", "A"), solid("b", "B"), SMOOTH],
    outputs: &[out(|node, ctx, _| {
        fold(node, ctx, "sd_subtract", &["a", "b"])
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "a")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static INTERSECT: NodeDef = NodeDef {
    slug: "intersect",
    category: Category::Solid,
    family: Some(Family::Combine),
    icon: "⊗",
    label: "Intersect",
    tooltip: "Only where A and B overlap.",
    inputs: &[solid("a", "A"), solid("b", "B"), SMOOTH],
    outputs: &[out(|node, ctx, _| {
        fold(node, ctx, "sd_intersect", &["a", "b"])
    })],
    bounds: Some(|n, ctx| {
        let a = ctx.solid_bounds(n, "a")?;
        let b = ctx.solid_bounds(n, "b")?;
        Some(format!("sd_bounds_meet({a}, {b})"))
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
