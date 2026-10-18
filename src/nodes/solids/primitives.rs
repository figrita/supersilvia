// SPDX-License-Identifier: AGPL-3.0-or-later

//! The primitives: a distance to one shape, each with its own place and color, so a simple
//! scene needs no transform nodes. Exact distances throughout but the octahedron's, which is
//! a bound, from Inigo Quilez's catalogue of distance functions.

use super::{SDF, color, fill, num, out, tint};
use crate::nodes::{Category, Family, InputDef, NodeDef};

const X: InputDef = num("x", "X", 0.0, -10.0, 10.0, 0.01, "⬓");
const Y: InputDef = num("y", "Y", 0.0, -10.0, 10.0, 0.01, "⬓");
const Z: InputDef = num("z", "Z", 0.0, -10.0, 10.0, 0.01, "⬓");

/// The primitive's body: `q` is the point relative to the primitive's center, `dist` the
/// distance expression over it, and `prelude` anything `dist` needs first.
fn primitive(
    node: crate::graph::NodeId,
    ctx: &mut crate::compile::CompileContext,
    prelude: &str,
    dist: &str,
) -> String {
    let c = tint(node, ctx, "color");
    fill(
        node,
        ctx,
        &format!(
            "    let q = p - vec3f({{x}}, {{y}}, {{z}});\n{prelude}    return vec4f({dist}, {c});"
        ),
    )
}

pub static SPHERE: NodeDef = NodeDef {
    slug: "sphere",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "⚪",
    label: "Sphere",
    tooltip: "A ball, at its center.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("radius", "Radius", 1.0, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#e8925aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(node, ctx, "", "length(q) - {radius}")
    })],
    bounds: Some(|n, ctx| super::shape_bounds(n, ctx, &super::half("vec3f({radius})"))),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

/// The box at a size scaled by `sc`: its sides scaled, and its Rounding by the smallest scale,
/// so a squashed box keeps round edges rather than flattened ones.
const BOX_BODY: &str = "    let half = vec3f({width}, {height}, {depth}) * 0.5 * sc;
    let r = min({rounding} * min(sc.x, min(sc.y, sc.z)), min(half.x, min(half.y, half.z)));
    let b = abs(q) - half + r;
";
const BOX_DIST: &str = "length(max(b, vec3f(0.0))) + min(max(b.x, max(b.y, b.z)), 0.0) - r";

pub static BOX: NodeDef = NodeDef {
    slug: "box",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "📦",
    label: "Box",
    tooltip: "A box, at its center, with its edges rounded by Rounding.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("width", "Width", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("height", "Height", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("depth", "Depth", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("rounding", "Rounding", 0.05, 0.0, 2.0, 0.01, "⬓"),
        color("color", "Color", "#5a8fe8ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(node, ctx, &[super::UNSCALED, BOX_BODY].concat(), BOX_DIST)
    })],
    scaled: Some(crate::nodes::ScaledDef {
        exact: "true",
        wgsl: |n, ctx| super::scaled_body(n, ctx, BOX_BODY, BOX_DIST),
    }),
    bounds: Some(|n, ctx| {
        super::shape_bounds(
            n,
            ctx,
            &super::half("vec3f({width}, {height}, {depth}) * 0.5"),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static TORUS: NodeDef = NodeDef {
    slug: "torus",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "🍩",
    label: "Torus",
    tooltip: "A ring lying flat: Ring is the distance to the middle of the tube, Tube its radius.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("ring", "Ring", 1.2, 0.01, 10.0, 0.01, "⬓"),
        num("tube", "Tube", 0.35, 0.01, 5.0, 0.01, "⬓"),
        color("color", "Color", "#d67a9eff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "",
            "length(vec2f(length(q.xz) - {ring}, q.y)) - {tube}",
        )
    })],
    bounds: Some(|n, ctx| {
        super::shape_bounds(
            n,
            ctx,
            &super::half("vec3f({ring} + {tube}, {tube}, {ring} + {tube})"),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

/// The cylinder at a size scaled by `sc`, exact while both widths scale alike.
const CYLINDER_BODY: &str =
    "    let e = abs(vec2f(length(q.xz), q.y)) - vec2f({radius} * sc.x, {height} * 0.5 * sc.y);\n";
const CYLINDER_DIST: &str = "min(max(e.x, e.y), 0.0) + length(max(e, vec2f(0.0)))";

pub static CYLINDER: NodeDef = NodeDef {
    slug: "cylinder",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "🥫",
    label: "Cylinder",
    tooltip: "An upright cylinder, capped, at its center.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("radius", "Radius", 0.8, 0.01, 10.0, 0.01, "⬓"),
        num("height", "Height", 2.0, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#8fd17aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            &[super::UNSCALED, CYLINDER_BODY].concat(),
            CYLINDER_DIST,
        )
    })],
    scaled: Some(crate::nodes::ScaledDef {
        exact: "sc.x == sc.z",
        wgsl: |n, ctx| super::scaled_body(n, ctx, CYLINDER_BODY, CYLINDER_DIST),
    }),
    bounds: Some(|n, ctx| {
        super::shape_bounds(
            n,
            ctx,
            &super::half("vec3f({radius}, {height} * 0.5, {radius})"),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static CAPSULE: NodeDef = NodeDef {
    slug: "capsule",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "💊",
    label: "Capsule",
    tooltip: "An upright pill: a cylinder of Length with a half ball on each end.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("length", "Length", 1.5, 0.0, 20.0, 0.01, "⬓"),
        num("radius", "Radius", 0.45, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#e8d35aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let half = {length} * 0.5;\n    let c = vec3f(q.x, q.y - clamp(q.y, -half, half), q.z);\n",
            "length(c) - {radius}",
        )
    })],
    bounds: Some(|n, ctx| {
        super::shape_bounds(
            n,
            ctx,
            &super::half("vec3f({radius}, {length} * 0.5 + {radius}, {radius})"),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static OCTAHEDRON: NodeDef = NodeDef {
    slug: "octahedron",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "💎",
    label: "Octahedron",
    tooltip: "Eight faces, its points Size from the center along each axis.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("size", "Size", 1.2, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#b07ae8ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let a = abs(q);\n",
            "(a.x + a.y + a.z - {size}) * 0.57735027",
        )
    })],
    bounds: Some(|n, ctx| super::shape_bounds(n, ctx, &super::half("vec3f({size})"))),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static GROUND: NodeDef = NodeDef {
    slug: "ground",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "▁",
    label: "Ground",
    tooltip: "A level plane that goes on forever, at Height. Everything below it is inside.",
    inputs: crate::nodes::solids::shape_inputs![
        num("height", "Height", -1.0, -10.0, 10.0, 0.01, "⬓"),
        color("color", "Color", "#4a4652ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        let c = tint(node, ctx, "color");
        fill(
            node,
            ctx,
            &format!("    return vec4f(p.y - {{height}}, {c});"),
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};
