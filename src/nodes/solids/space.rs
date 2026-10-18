// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transforms: each hands its input a different point, as `zoom` hands its input a different
//! `uv`. A transform that preserves distances — a move, a turn, a mirror — passes the distance
//! back as it is; a uniform scale multiplies it back. One that stretches space by different
//! amounts in different places divides the distance by the most it stretches, so it stays a
//! bound: Stretch by its smallest factor's reciprocal, Twist and Bend by how far the point is
//! from the axis they turn about.

use super::{SDF, fill, num, num_log, out, solid, turn};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

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
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static TWIST: NodeDef = NodeDef {
    slug: "twist",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "🌀",
    label: "Twist",
    tooltip: "Turns a solid about its upright axis by Rate turns for every unit up. Far from the \
              axis the march slows down to stay on the surface.",
    inputs: &[
        solid("input", "Solid"),
        num("rate", "Rate", 0.15, -2.0, 2.0, 0.001, TURNS),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let k = {{rate}} * 6.2831853;
    let xz = sd_rot2(p.xz, k * p.y);
    let q = vec3f(xz.x, p.y, xz.y);
    let r = {inner};
    return vec4f(r.x / sqrt(1.0 + k * k * dot(p.xz, p.xz)), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static BEND: NodeDef = NodeDef {
    slug: "bend",
    category: Category::Solid,
    family: Some(Family::Space),
    icon: "⌒",
    label: "Bend",
    tooltip: "Bends a solid along X into an arc. Curvature is one over the arc's radius.",
    inputs: &[
        solid("input", "Solid"),
        num("curvature", "Curvature", 0.3, -3.0, 3.0, 0.001, "/⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let k = {{curvature}};
    let c = cos(k * p.x);
    let s = sin(k * p.x);
    let q = vec3f(c * p.x - s * p.y, s * p.x + c * p.y, p.z);
    let r = {inner};
    return vec4f(r.x / (1.0 + abs(k) * length(p.xy)), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
