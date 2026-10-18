// SPDX-License-Identifier: AGPL-3.0-or-later

//! Color fields: nodes that paint a solid by where its surface is, and leave its shape alone.
//!
//! Each one changes only color, so its distance-only variant is its input passed through
//! untouched and a march never pays for it. What it measures is measured in the whole variant
//! alone, once a pixel, where the ray landed:
//!
//! - **Proximity** and **Contours** read a second solid, B, for its distance alone. B lends no
//!   shape — the result's distance is A's — so only B's distance-only variant is called, and
//!   it costs its cheapest form.
//! - **Facing** needs the surface's normal, which it takes the way Texture's Box projection
//!   does: four taps of its input's distance round the point (`surface`). The taps go to the
//!   distance-only variant, so the normal costs four distances and no color.

use super::{SDF, color, fill, num, num_log, out, solid};
use crate::nodes::{Category, Family, NodeDef, OptionDef};

pub static CHECKER: NodeDef = NodeDef {
    slug: "checker3d",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "🏁",
    label: "Checker 3D",
    tooltip: "Paints a solid in a checkerboard of cubes Size wide, in two colors. Mix is how \
              much of the solid's own color is painted over. The shape is untouched.",
    inputs: &[
        solid("input", "Solid"),
        color("colorA", "Color A", "#e8e2d6ff"),
        color("colorB", "Color B", "#3a3644ff"),
        num_log("size", "Size", 0.5, 0.01, 20.0, 0.01, "⬓"),
        num("mix", "Mix", 1.0, 0.0, 1.0, 0.01, ""),
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
    let cell = floor(p / max({{size}}, 1.0e-4));
    let k = cell.x + cell.y + cell.z;
    let odd = k - 2.0 * floor(k * 0.5);
    let c = mix({{colorA}}.rgb, {{colorB}}.rgb, odd);
    return vec4f(r.x, mix(r.yzw, c, clamp({{mix}}, 0.0, 1.0)));"
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "input")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static PROXIMITY: NodeDef = NodeDef {
    slug: "proximity",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "◎",
    label: "Proximity",
    tooltip: "Paints solid A with Color wherever it comes within Reach of solid B, fading out \
              over Falloff beyond it. B is not drawn; only A's shape is.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        color("color", "Color", "#ffb84aff"),
        num("reach", "Reach", 0.3, 0.0, 10.0, 0.01, "⬓"),
        num("falloff", "Falloff", 0.3, 0.0, 10.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let a = ctx.input(node, "a", "p");
        if !ctx.solid_color() {
            return format!("    return {a};");
        }
        let b = ctx.solid_input(node, "b", "p", false);
        fill(
            node,
            ctx,
            &format!(
                "    let r = {a};
    let d = {b}.x;
    let reach = max({{reach}}, 0.0);
    let t = 1.0 - smoothstep(reach, reach + max({{falloff}}, 1.0e-4), d);
    return vec4f(r.x, mix(r.yzw, {{color}}.rgb, t));"
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "a")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static FACING: NodeDef = NodeDef {
    slug: "facing",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "⇅",
    label: "Facing",
    tooltip: "Paints a solid by which way its surface faces: Up Color on top, Side Color on \
              the sides, Down Color underneath, or the direction itself as a color. Mix is how \
              much is painted over. The shape is untouched.",
    inputs: &[
        solid("input", "Solid"),
        color("colorUp", "Up Color", "#f2efe6ff"),
        color("colorSide", "Side Color", "#8a7f74ff"),
        color("colorDown", "Down Color", "#2e2a36ff"),
        num("mix", "Mix", 1.0, 0.0, 1.0, 0.01, ""),
    ],
    options: &[OptionDef {
        key: "mode",
        label: "Colors",
        default: "updown",
        choices: &[("updown", "Up/Side/Down"), ("normal", "Normal as color")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        if !ctx.solid_color() {
            return format!("    return {inner};");
        }
        let taps: Vec<String> = ["e.xyy", "e.yyx", "e.yxy", "e.xxx"]
            .iter()
            .map(|e| ctx.solid_input(node, "input", &format!("p + {e}"), false))
            .collect();
        let paint = match ctx.option(node, "mode") {
            "normal" => "n * 0.5 + 0.5",
            _ => {
                "{colorSide}.rgb * (1.0 - abs(n.y)) + {colorUp}.rgb * max(n.y, 0.0) \
                 + {colorDown}.rgb * max(-n.y, 0.0)"
            }
        };
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let e = vec2f(0.002, -0.002);
    let n = sd_facing_normal({}.x, {}.x, {}.x, {}.x);
    let c = {paint};
    return vec4f(r.x, mix(r.yzw, c, clamp({{mix}}, 0.0, 1.0)));",
                taps[0], taps[1], taps[2], taps[3]
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "input")),
    wgsl_utils: &[SDF, FACING_NORMAL],
    ..NodeDef::EMPTY
};

/// The unit normal from four taps of a distance at the corners of a tetrahedron round a point,
/// in the order `e.xyy`, `e.yyx`, `e.yxy`, `e.xxx`. Zero where the four agree.
const FACING_NORMAL: &str = "fn sd_facing_normal(a: f32, b: f32, c: f32, d: f32) -> vec3f {
    let e = vec2f(1.0, -1.0);
    let g = e.xyy * a + e.yyx * b + e.yxy * c + e.xxx * d;
    return g / max(length(g), 1.0e-8);
}";

pub static CONTOURS: NodeDef = NodeDef {
    slug: "distancebands",
    category: Category::Solid,
    family: Some(Family::Color),
    icon: "🗺",
    label: "Contours",
    tooltip: "Paints solid A in bands of two colors by how far each point is from solid B, \
              each Width thick, like the lines on a map. Offset moves them outward. B is not \
              drawn; only A's shape is.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        color("colorA", "Color A", "#e8e2d6ff"),
        color("colorB", "Color B", "#3a5ae8ff"),
        num("width", "Width", 0.2, 0.01, 10.0, 0.01, "⬓"),
        num("offset", "Offset", 0.0, -10.0, 10.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let a = ctx.input(node, "a", "p");
        if !ctx.solid_color() {
            return format!("    return {a};");
        }
        let b = ctx.solid_input(node, "b", "p", false);
        fill(
            node,
            ctx,
            &format!(
                "    let r = {a};
    let band = floor(({b}.x - {{offset}}) / max({{width}}, 1.0e-4));
    let odd = band - 2.0 * floor(band * 0.5);
    return vec4f(r.x, mix({{colorA}}.rgb, {{colorB}}.rgb, odd));"
            ),
        )
    })],
    bounds: Some(|n, ctx| ctx.solid_bounds(n, "a")),
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
