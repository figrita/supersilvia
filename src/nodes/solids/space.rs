// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transforms: each hands its input a different point, as `zoom` hands its input a different
//! `uv`. A transform that preserves distances — a move, a turn, a mirror — passes the distance
//! back as it is; a uniform scale multiplies it back. One that stretches space by different
//! amounts in different places divides the distance by the most it stretches, so it stays a
//! bound: Stretch by its smallest factor's reciprocal, Twist and Bend by how far the point is
//! from the axis they turn about.
//!
//! **Each says how big its solid is** from its input's box. Stretch scales the box. Mirror
//! Space keeps the box's far side on each axis it folds and reflects it for the near side.
//! Twist and Bend turn a point about an axis through the origin by an angle that depends on
//! the point, which keeps its distance from that axis: the box is the input's along the axis
//! and the circle round the input's furthest reach across it.

use super::{SDF, fill, num, num_log, out, solid, turn};
use crate::compile::CompileContext;
use crate::graph::{Axes, NodeId};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

/// The axes a node's `input` line was given: the plane a Bend curves in, a Twist turns in.
fn frame_of(ctx: &CompileContext, node: NodeId, start: Axes) -> Axes {
    ctx.node(node).map_or(start, |n| n.axes_of("input"))
}

/// The bounds of a solid turned about the third axis of `axes`, its input's `b`: every point
/// keeps its distance from that axis, so the plane's two axes reach the input's reach across
/// them both ways, and the third keeps the input's span.
fn about_third(b: &str, axes: Axes) -> String {
    let (a, u, t) = (axes.across.of, axes.up.of, axes.third());
    let c = |i: u8| ['x', 'y', 'z'][usize::from(i)];
    let m = format!(
        "length(vec2f(sd_bounds_reach({b}).{}, sd_bounds_reach({b}).{}))",
        c(a),
        c(u)
    );
    let side = |lo: bool| {
        let part = |i: u8| {
            if i == t {
                format!("{b}.{}.{}", if lo { "lo" } else { "hi" }, c(i))
            } else if lo {
                format!("-{m}")
            } else {
                m.clone()
            }
        };
        format!("vec3f({}, {}, {})", part(0), part(1), part(2))
    };
    format!("SdBounds({}, {})", side(true), side(false))
}

/// Where a Twist starts: turning in the plane of X and Z, about Y.
const TWIST_START: Axes = Axes::TOP;

pub static PLACE: NodeDef = NodeDef {
    slug: "place",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "📍",
    label: "Place",
    tooltip: "Moves, turns and scales a solid. Turns are in turns; Scale is the same on every axis.",
    inputs: &[
        solid("input", "Solid"),
        num("x", "X", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("y", "Y", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("z", "Z", 0.0, -10.0, 10.0, 0.01, "⬓"),
        turn("yaw", "Yaw", 0.0, -1.0, 1.0, 0.001),
        turn("pitch", "Pitch", 0.0, -1.0, 1.0, 0.001),
        turn("roll", "Roll", 0.0, -1.0, 1.0, 0.001),
        num_log("scale", "Scale", 1.0, 0.05, 20.0, 0.01, "x"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let s = max({{scale}}, 1.0e-4);
    let q = sd_turn(p - vec3f({{x}}, {{y}}, {{z}}), -vec3f({{yaw}}, {{pitch}}, {{roll}}) * 6.2831853) / s;
    let r = {inner};
    return vec4f(r.x * s, r.yzw);"
            ),
        )
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        ctx.uniform_fill(
            n,
            &format!(
                "sd_bounds_move(sd_bounds_about({b}, vec3f(0.0), \
                 any(vec3f({{yaw}}, {{pitch}}, {{roll}}) != vec3f(0.0)), max({{scale}}, 1.0e-4)), \
                 vec3f({{x}}, {{y}}, {{z}}))"
            ),
        )
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static STRETCH: NodeDef = NodeDef {
    slug: "stretch",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "⇔",
    label: "Stretch",
    tooltip: "Scales a solid by a different amount on each axis. The distance is divided by the \
              largest squash, so a march stays safe and takes more steps the further the three \
              differ.",
    inputs: &[
        solid("input", "Solid"),
        num_log("sx", "X", 1.0, 0.05, 20.0, 0.01, "x"),
        num_log("sy", "Y", 1.0, 0.05, 20.0, 0.01, "x"),
        num_log("sz", "Z", 1.0, 0.05, 20.0, 0.01, "x"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let s = max(vec3f({{sx}}, {{sy}}, {{sz}}), vec3f(1.0e-4));
    let q = p / s;
    let r = {inner};
    return vec4f(r.x * min(s.x, min(s.y, s.z)), r.yzw);"
            ),
        )
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        ctx.uniform_fill(
            n,
            &format!(
                "SdBounds({b}.lo * max(vec3f({{sx}}, {{sy}}, {{sz}}), vec3f(1.0e-4)), \
                 {b}.hi * max(vec3f({{sx}}, {{sy}}, {{sz}}), vec3f(1.0e-4)))"
            ),
        )
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static MIRROR: NodeDef = NodeDef {
    slug: "solidmirror",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "🪞",
    label: "Mirror Space",
    tooltip: "Folds space across the planes through the center, so one side is drawn on both.",
    inputs: &[solid("input", "Solid")],
    options: &[OptionDef {
        key: "axes",
        label: "Across",
        default: "x",
        choices: &[
            ("x", "X"),
            ("y", "Y"),
            ("z", "Z"),
            ("xz", "X and Z"),
            ("xyz", "X, Y and Z"),
        ],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let fold = match ctx.option(node, "axes") {
            "y" => "vec3f(p.x, abs(p.y), p.z)",
            "z" => "vec3f(p.x, p.y, abs(p.z))",
            "xz" => "vec3f(abs(p.x), p.y, abs(p.z))",
            "xyz" => "abs(p)",
            _ => "vec3f(abs(p.x), p.y, p.z)",
        };
        let inner = ctx.input(node, "input", "q");
        format!("    let q = {fold};\n    return {inner};")
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        let across = match ctx.option(n, "axes") {
            "y" => "false, true, false",
            "z" => "false, false, true",
            "xz" => "true, false, true",
            "xyz" => "true, true, true",
            _ => "true, false, false",
        };
        Some(format!(
            "SdBounds(select({b}.lo, -{b}.hi, vec3<bool>({across})), {b}.hi)"
        ))
    }),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static TWIST: NodeDef = NodeDef {
    slug: "twist",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "🌀",
    label: "Twist",
    tooltip: "Turns a solid about its upright axis by Rate turns for every unit up, or about \
              the axis its Solid row's cube chooses. Far from the axis the march slows down to \
              stay on the surface.",
    inputs: &[
        solid("input", "Solid"),
        num("rate", "Rate", 0.15, -2.0, 2.0, 0.001, TURNS),
    ],
    outputs: &[out(|node, ctx, _| {
        let axes = frame_of(ctx, node, TWIST_START);
        let inner = ctx.input(node, "input", "q");
        let into = axes.into_frame("p");
        let back = axes.out_of_frame("g");
        fill(
            node,
            ctx,
            &format!(
                "    let k = {{rate}} * 6.2831853;
    let f = {into};
    let g = vec3f(sd_rot2(f.xy, k * f.z), f.z);
    let q = {back};
    let r = {inner};
    return vec4f(r.x / sqrt(1.0 + k * k * dot(f.xy, f.xy)), r.yzw);"
            ),
        )
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        Some(about_third(&b, frame_of(ctx, n, TWIST_START)))
    }),
    axes: &[("input", TWIST_START)],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static BEND: NodeDef = NodeDef {
    slug: "bend",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "⌒",
    label: "Bend",
    tooltip: "Bends a solid along X into an arc curving toward Y, or along and toward the axes \
              its Solid row's cube chooses. Curvature is one over the arc's radius.",
    inputs: &[
        solid("input", "Solid"),
        num("curvature", "Curvature", 0.3, -3.0, 3.0, 0.001, "/⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let axes = frame_of(ctx, node, Axes::FRONT);
        let inner = ctx.input(node, "input", "q");
        let into = axes.into_frame("p");
        let back = axes.out_of_frame("g");
        fill(
            node,
            ctx,
            &format!(
                "    let k = {{curvature}};
    let f = {into};
    let c = cos(k * f.x);
    let s = sin(k * f.x);
    let g = vec3f(c * f.x - s * f.y, s * f.x + c * f.y, f.z);
    let q = {back};
    let r = {inner};
    return vec4f(r.x / (1.0 + abs(k) * length(f.xy)), r.yzw);"
            ),
        )
    })],
    bounds: Some(|n, ctx| {
        let b = ctx.solid_bounds(n, "input")?;
        Some(about_third(&b, frame_of(ctx, n, Axes::FRONT)))
    }),
    axes: &[("input", Axes::FRONT)],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
