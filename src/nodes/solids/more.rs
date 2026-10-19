// SPDX-License-Identifier: AGPL-3.0-or-later

//! More solids: five primitives, two operations and two color fields, with the fractals in
//! [`fractals`].
//!
//! The primitives' distances are Inigo Quilez's exact ones but the gyroid's; the pyramid's
//! measures its base as well, which IQ's leaves to a ground plane. A gyroid is a level set of
//! a trigonometric sum, not a shape with a nearest point anyone can solve for, so its value is
//! divided by the steepest the sum ever gets — √3 a radian — and is a bound that is close to
//! exact near the sheet. It fills all of space, so it is meant to be intersected with
//! something finite, and says how big it is only then (`solids::endless_bounds`).
//!
//! Elongate is IQ's exact `opElongate` for a solid symmetric about its own center planes, the
//! case it is defined for. It hands its input the point with the inserted slab taken out, sign
//! kept, so an asymmetric solid stays itself on each side rather than being mirrored; the
//! interior term is added only where the input's center is inside it, which is the only place
//! it means something. Its box is its input's pulled apart by X, Y and Z. Morph's is both
//! inputs': a mix of two distances is below zero only where one of them is.
//!
//! The two color fields change only color. A march reads their distance-only variant, which is
//! their input passed through untouched.

pub mod fractals;

use super::{SDF, UNSCALED, color, fill, num, num_log, out, scaled_body, solid, tint};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::nodes::{Category, Family, InputDef, NodeDef, OptionDef};

pub(crate) const X: InputDef = num("x", "X", 0.0, -10.0, 10.0, 0.01, "⬓");
pub(crate) const Y: InputDef = num("y", "Y", 0.0, -10.0, 10.0, 0.01, "⬓");
pub(crate) const Z: InputDef = num("z", "Z", 0.0, -10.0, 10.0, 0.01, "⬓");

/// A placed solid's body: `q` is the point relative to its X, Y and Z, then `body`, which
/// returns.
pub(crate) fn placed(node: NodeId, ctx: &mut CompileContext, body: &str) -> String {
    fill(
        node,
        ctx,
        &format!("    let q = p - vec3f({{x}}, {{y}}, {{z}});\n{body}"),
    )
}

/// A primitive's body: `prelude` over `q`, then the distance `dist` and the node's Color.
fn primitive(node: NodeId, ctx: &mut CompileContext, prelude: &str, dist: &str) -> String {
    let c = tint(node, ctx, "color");
    placed(
        node,
        ctx,
        &format!("{prelude}    return vec4f({dist}, {c});"),
    )
}

/// The cone at a size scaled by `sc`, exact while both widths scale alike.
const CONE_BODY: &str = "    let h = {height} * 0.5 * sc.y;
    let r1 = {bottom} * sc.x;
    let r2 = {top} * sc.x;
    let w = vec2f(length(q.xz), q.y);
    let k1 = vec2f(r2, h);
    let k2 = vec2f(r2 - r1, 2.0 * h);
    let ca = vec2f(w.x - min(w.x, select(r2, r1, w.y < 0.0)), abs(w.y) - h);
    let cb = w - k1 + k2 * clamp(dot(k1 - w, k2) / dot(k2, k2), 0.0, 1.0);
    let sg = select(1.0, -1.0, cb.x < 0.0 && ca.y < 0.0);
";
const CONE_DIST: &str = "sg * sqrt(min(dot(ca, ca), dot(cb, cb)))";

pub static CONE: NodeDef = NodeDef {
    slug: "cone",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "🔺",
    label: "Cone",
    tooltip: "An upright cone, capped, at its center: Bottom and Top are the radii of its two \
              ends, so a Top of 0 comes to a point.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("bottom", "Bottom", 1.0, 0.0, 10.0, 0.01, "⬓"),
        num("top", "Top", 0.0, 0.0, 10.0, 0.01, "⬓"),
        num("height", "Height", 2.0, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#e8b45aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(node, ctx, &[UNSCALED, CONE_BODY].concat(), CONE_DIST)
    })],
    scaled: Some(crate::nodes::ScaledDef {
        exact: "sc.x == sc.z",
        wgsl: |n, ctx| scaled_body(n, ctx, CONE_BODY, CONE_DIST),
    }),
    bounds: Some(|n, ctx| {
        crate::nodes::solids::shape_bounds(
            n,
            ctx,
            &crate::nodes::solids::half(
                "vec3f(max({bottom}, {top}), {height} * 0.5, max({bottom}, {top}))",
            ),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

/// The hex prism at a size scaled by `sc`, exact while both widths scale alike.
const HEX_PRISM_BODY: &str = "    let k = vec3f(-0.8660254, 0.5, 0.57735027);
    let r = {radius} * sc.x;
    let a = abs(q);
    var f = a.xz;
    f -= 2.0 * min(dot(k.xy, f), 0.0) * k.xy;
    let e = vec2f(length(f - vec2f(clamp(f.x, -k.z * r, k.z * r), r)) * sign(f.y - r), a.y - {height} * 0.5 * sc.y);
";
const HEX_PRISM_DIST: &str = "min(max(e.x, e.y), 0.0) + length(max(e, vec2f(0.0)))";

pub static HEX_PRISM: NodeDef = NodeDef {
    slug: "hexprism",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "⬢",
    label: "Hex Prism",
    tooltip: "An upright six-sided column, at its center. Radius is from the middle to the \
              middle of a side.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("radius", "Radius", 0.9, 0.01, 10.0, 0.01, "⬓"),
        num("height", "Height", 1.6, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#5ad1c4ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            &[UNSCALED, HEX_PRISM_BODY].concat(),
            HEX_PRISM_DIST,
        )
    })],
    scaled: Some(crate::nodes::ScaledDef {
        exact: "sc.x == sc.z",
        wgsl: |n, ctx| scaled_body(n, ctx, HEX_PRISM_BODY, HEX_PRISM_DIST),
    }),
    bounds: Some(|n, ctx| {
        crate::nodes::solids::shape_bounds(
            n,
            ctx,
            &crate::nodes::solids::half(
                "vec3f({radius} * 1.1547005, {height} * 0.5, {radius} * 1.1547005)",
            ),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

/// The pyramid at a size scaled by `sc`, exact while both widths scale alike. IQ's pyramid, one
/// wide, measures its four sides and the base is the square under them; the point is scaled
/// into it and the distance back.
const PYRAMID_BODY: &str = "    let sb = max({base} * sc.x, 1.0e-4);
    let ht = {height} * sc.y;
    let ph = ht / sb;
    let m2 = ph * ph + 0.25;
    let v = (q + vec3f(0.0, ht * 0.5, 0.0)) / sb;
    var f = abs(v.xz);
    if (f.y > f.x) { f = f.yx; }
    f -= vec2f(0.5);
    let w = vec3f(f.y, ph * v.y - 0.5 * f.x, ph * f.x + 0.5 * v.y);
    let s = max(-w.x, 0.0);
    let t = clamp((w.y - 0.5 * f.y) / (m2 + 0.25), 0.0, 1.0);
    let da = m2 * (w.x + s) * (w.x + s) + w.y * w.y;
    let db = m2 * (w.x + 0.5 * t) * (w.x + 0.5 * t) + (w.y - m2 * t) * (w.y - m2 * t);
    let d2 = select(min(da, db), 0.0, min(w.y, -w.x * m2 - w.y * 0.5) > 0.0);
    let under = max(abs(v.xz) - vec2f(0.5), vec2f(0.0));
    let floor_d = sqrt(dot(under, under) + v.y * v.y);
";
const PYRAMID_DIST: &str = "min(sqrt((d2 + w.z * w.z) / m2), floor_d) * sign(max(w.z, -v.y)) * sb";

pub static PYRAMID: NodeDef = NodeDef {
    slug: "pyramid",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "△",
    label: "Pyramid",
    tooltip: "A pyramid on a square base Base wide, Height tall, placed by the point halfway up.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("base", "Base", 1.8, 0.01, 20.0, 0.01, "⬓"),
        num("height", "Height", 1.6, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#e8d08aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(node, ctx, &[UNSCALED, PYRAMID_BODY].concat(), PYRAMID_DIST)
    })],
    scaled: Some(crate::nodes::ScaledDef {
        exact: "sc.x == sc.z",
        wgsl: |n, ctx| scaled_body(n, ctx, PYRAMID_BODY, PYRAMID_DIST),
    }),
    bounds: Some(|n, ctx| {
        crate::nodes::solids::shape_bounds(
            n,
            ctx,
            &crate::nodes::solids::half("vec3f({base} * 0.5, {height} * 0.5, {base} * 0.5)"),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static LINK: NodeDef = NodeDef {
    slug: "link",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "🔗",
    label: "Link",
    tooltip: "A chain link standing upright: a ring of Ring radius pulled apart by Length, \
              made of a tube Tube in radius.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("length", "Length", 0.8, 0.0, 20.0, 0.01, "⬓"),
        num("ring", "Ring", 0.6, 0.01, 10.0, 0.01, "⬓"),
        num("tube", "Tube", 0.18, 0.01, 5.0, 0.01, "⬓"),
        color("color", "Color", "#c9ccd6ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let w = vec3f(q.x, max(abs(q.y) - {length} * 0.5, 0.0), q.z);\n",
            "length(vec2f(length(w.xy) - {ring}, w.z)) - {tube}",
        )
    })],
    bounds: Some(|n, ctx| {
        crate::nodes::solids::shape_bounds(
            n,
            ctx,
            &crate::nodes::solids::half(
                "vec3f({ring} + {tube}, {length} * 0.5 + {ring} + {tube}, {tube})",
            ),
        )
    }),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static GYROID: NodeDef = NodeDef {
    slug: "gyroid",
    category: Category::Solid,
    family: Some(Family::Shapes),
    icon: "≋",
    label: "Gyroid",
    tooltip: "A curved lattice that fills all of space, Scale the size of one cell, its sheet \
              Thickness thick. Intersect it with a Box or a Sphere to cut a piece out.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num_log("scale", "Scale", 2.0, 0.05, 20.0, 0.01, "⬓"),
        num("thickness", "Thickness", 0.12, 0.005, 2.0, 0.005, "⬓"),
        color("color", "Color", "#e88a7aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let s = 6.2831853 / max({scale}, 1.0e-3);
    let g = dot(sin(q * s), cos(q.zxy * s));
",
            "abs(g) / (s * 1.7320508) - {thickness} * 0.5",
        )
    })],
    bounds: Some(super::endless_bounds),
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static ELONGATE: NodeDef = NodeDef {
    slug: "elongate",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "⟷",
    label: "Elongate",
    tooltip: "Pulls a solid apart through its center and fills the gap, adding X, Y and Z to \
              its width, height and depth. Exact for a solid symmetric about its center.",
    inputs: &[
        solid("input", "Solid"),
        num("x", "X", 1.0, 0.0, 20.0, 0.01, "⬓"),
        num("y", "Y", 0.0, 0.0, 20.0, 0.01, "⬓"),
        num("z", "Z", 0.0, 0.0, 20.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let h = max(vec3f({{x}}, {{y}}, {{z}}), vec3f(0.0)) * 0.5;
    let q = p - clamp(p, -h, h);
    let r = {inner};
    let e = abs(p) - h;
    let fix = select(0.0, min(max(e.x, max(e.y, e.z)), 0.0), r.x < 0.0);
    return vec4f(r.x + fix, r.yzw);"
            ),
        )
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        let h = "max(vec3f({x}, {y}, {z}), vec3f(0.0)) * 0.5";
        ctx.uniform_fill(n, &format!("SdBounds({b}.lo - {h}, {b}.hi + {h})"))
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static MORPH: NodeDef = NodeDef {
    slug: "morph",
    category: Category::Solid,
    family: Some(Family::Combine),
    icon: "◐",
    label: "Morph",
    tooltip: "Blends solid A into solid B by Amount, shape and color both. The blend is a safe \
              distance only where both inputs are.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        num("amount", "Amount", 0.5, 0.0, 1.0, 0.001, ""),
    ],
    outputs: &[out(|node, ctx, _| {
        let a = ctx.input(node, "a", "p");
        let b = ctx.input(node, "b", "p");
        fill(
            node,
            ctx,
            &format!("    return mix({a}, {b}, clamp({{amount}}, 0.0, 1.0));"),
        )
    })],
    bounds: Some(|n, ctx| {
        let a = ctx.solid_bounds(n, "a")?;
        let b = ctx.solid_bounds(n, "b")?;
        Some(format!("sd_bounds_union({a}, {b})"))
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static STRIPES: NodeDef = NodeDef {
    slug: "stripes3d",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "▤",
    label: "Stripes 3D",
    tooltip: "Paints a solid in bands of two colors, each Width thick, across the chosen axis. \
              The shape is untouched.",
    inputs: &[
        solid("input", "Solid"),
        color("colorA", "Color A", "#e8e2d6ff"),
        color("colorB", "Color B", "#3a3644ff"),
        num("width", "Width", 0.25, 0.01, 10.0, 0.01, "⬓"),
        num("offset", "Offset", 0.0, -10.0, 10.0, 0.01, "⬓"),
    ],
    options: &[OptionDef {
        key: "axis",
        label: "Axis",
        default: "y",
        choices: &[("x", "X"), ("y", "Y"), ("z", "Z")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        if !ctx.solid_color() {
            return format!("    return {inner};");
        }
        let along = match ctx.option(node, "axis") {
            "x" => "p.x",
            "z" => "p.z",
            _ => "p.y",
        };
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let band = floor(({along} - {{offset}}) / max({{width}}, 1.0e-4));
    let odd = band - 2.0 * floor(band * 0.5);
    return vec4f(r.x, mix({{colorA}}.rgb, {{colorB}}.rgb, odd));"
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "input")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static HEIGHT_GRADIENT: NodeDef = NodeDef {
    slug: "heightgradient",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "🌅",
    label: "Height Gradient",
    tooltip: "Paints a solid from Bottom Color at the height Bottom to Top Color at the height \
              Top, each held beyond its end. The shape is untouched.",
    inputs: &[
        solid("input", "Solid"),
        color("colorBottom", "Bottom Color", "#3a5ae8ff"),
        color("colorTop", "Top Color", "#e8d35aff"),
        num("bottom", "Bottom", -1.0, -20.0, 20.0, 0.01, "⬓"),
        num("top", "Top", 1.0, -20.0, 20.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        if !ctx.solid_color() {
            return format!("    return {inner};");
        }
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let span = {{top}} - {{bottom}};
    let t = clamp((p.y - {{bottom}}) / select(span, 1.0e-4, abs(span) < 1.0e-4), 0.0, 1.0);
    return vec4f(r.x, mix({{colorBottom}}.rgb, {{colorTop}}.rgb, t));"
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "input")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
