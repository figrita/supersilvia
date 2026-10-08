// SPDX-License-Identifier: AGPL-3.0-or-later

//! Surface modifiers: what a solid's skin is — rounder, hollow, a color, rippled, a picture.
//!
//! **Texture** is the door a picture comes in by. A solid has a point and no `uv`, so the node
//! makes one, by projecting the point onto a plane, round a cylinder or onto a sphere, and asks
//! its picture input at that `uv`. The input is any `VaryingColor`: an Output's `frame` from a
//! video workspace, a camera, a clip, or a whole video chain, which is then computed where the
//! ray lands. It is sampled in the whole of the solid only, so a march pays for it once a
//! pixel. Where it is wired decides whether it sticks: right after a primitive it sees that
//! primitive's own point and moves with it; after a transform it sees the transform's input
//! space, and near the render it is a projector the solid moves through.

use super::{SDF, color, fill, num, num_log, out, solid};
use crate::graph::PortType::{UniformNumber, VaryingColor};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, TURNS};

pub static ROUND: NodeDef = NodeDef {
    slug: "inflate",
    category: Category::Solid,
    icon: "◍",
    label: "Round",
    tooltip: "Grows a solid outward by Radius, rounding every edge and corner by as much.",
    inputs: &[
        solid("input", "Solid"),
        num("radius", "Radius", 0.1, 0.0, 2.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        fill(
            node,
            ctx,
            &format!("    let r = {inner};\n    return vec4f(r.x - {{radius}}, r.yzw);"),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static SHELL: NodeDef = NodeDef {
    slug: "shell",
    category: Category::Solid,
    icon: "🥚",
    label: "Shell",
    tooltip: "Hollows a solid into a skin Thickness thick. Cut it open with Subtract to see in.",
    inputs: &[
        solid("input", "Solid"),
        num("thickness", "Thickness", 0.1, 0.005, 2.0, 0.005, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};\n    return vec4f(abs(r.x) - {{thickness}} * 0.5, r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static PAINT: NodeDef = NodeDef {
    slug: "paint",
    category: Category::Solid,
    icon: "🖌",
    label: "Paint",
    tooltip: "Gives a whole solid one color.",
    inputs: &[
        solid("input", "Solid"),
        color("color", "Color", "#e8e2d6ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        if !ctx.solid_color() {
            return format!("    return {inner};");
        }
        fill(
            node,
            ctx,
            &format!("    let r = {inner};\n    return vec4f(r.x, {{color}}.rgb);"),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static DISPLACE: NodeDef = NodeDef {
    slug: "displace",
    category: Category::Solid,
    icon: "〰",
    label: "Displace",
    tooltip: "Ripples a solid's surface in and out by Amount. Cable a gear's Cycles into Phase to \
              set it moving. The more it ripples, the slower the march creeps up on it.",
    inputs: &[
        solid("input", "Solid"),
        num("amount", "Amount", 0.08, 0.0, 1.0, 0.001, "⬓"),
        num("frequency", "Frequency", 4.0, 0.1, 30.0, 0.01, "/⬓"),
        InputDef {
            key: "phase",
            label: "Phase",
            ty: UniformNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.001, TURNS),
        },
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let f = {{frequency}};
    let a = {{amount}};
    let t = {{phase}} * 6.2831853;
    let n = sin(f * p.x + t) * sin(f * p.y + t * 2.0) * sin(f * p.z - t);
    return vec4f((r.x + a * n) / (1.0 + a * f * 1.7320508), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static TEXTURE: NodeDef = NodeDef {
    slug: "surfacetexture",
    category: Category::Solid,
    icon: "🖼",
    label: "Texture",
    tooltip: "Puts a picture on a solid's surface: an Output's Frame from another workspace, a \
              camera, a clip, or any picture. Box wraps it on from all sides; the others \
              project it one way. Scale is how big one picture is on the surface.",
    inputs: &[
        solid("input", "Solid"),
        InputDef {
            key: "picture",
            label: "Picture",
            ty: VaryingColor,
            control: Control::None,
        },
        num_log("scale", "Scale", 1.0, 0.05, 20.0, 0.01, "⬓"),
        num("offsetX", "Offset X", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("offsetY", "Offset Y", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("mix", "Mix", 1.0, 0.0, 1.0, 0.01, ""),
    ],
    options: &[OptionDef {
        key: "projection",
        label: "Mapping",
        default: "box",
        choices: &[
            ("box", "Box"),
            ("front", "Front"),
            ("top", "Top"),
            ("side", "Side"),
            ("around", "Around"),
            ("sphere", "Sphere"),
        ],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "p");
        if !ctx.solid_color() {
            return format!("    return {inner};");
        }
        let at = |ctx: &mut crate::compile::CompileContext, uv: &str| {
            ctx.input(node, "picture", &format!("({uv}) / s + o"))
        };
        let picture = match ctx.option(node, "projection") {
            "front" => at(ctx, "p.xy"),
            "top" => at(ctx, "p.xz"),
            "side" => at(ctx, "p.zy"),
            "around" => at(ctx, "vec2f(atan2(p.x, p.z) / PI * (16.0 / 9.0) * s, p.y)"),
            "sphere" => at(
                ctx,
                "vec2f(atan2(p.x, p.z) / PI * (16.0 / 9.0), asin(clamp(normalize(p).y, -1.0, 1.0)) / (PI * 0.5))",
            ),
            _ => {
                let taps: Vec<String> = ["e.xyy", "e.yyx", "e.yxy", "e.xxx"]
                    .iter()
                    .map(|e| ctx.solid_input(node, "input", &format!("p + {e}"), false))
                    .collect();
                let (x, y, z) = (at(ctx, "p.zy"), at(ctx, "p.xz"), at(ctx, "p.xy"));
                format!(
                    "box_tex(p, {x}, {y}, {z}, {a}.x, {b}.x, {c}.x, {d}.x)",
                    a = taps[0],
                    b = taps[1],
                    c = taps[2],
                    d = taps[3]
                )
            }
        };
        // The box's normal comes from the solid's own distance, four taps round the point.
        let head = if picture.starts_with("box_tex") {
            "    let e = vec2f(0.002, -0.002);\n"
        } else {
            ""
        };
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let s = max({{scale}}, 1.0e-4);
    let o = vec2f({{offsetX}}, {{offsetY}});
{head}    let c = unpremultiply({picture});
    return vec4f(r.x, mix(r.yzw, c.rgb, c.a * {{mix}}));"
            ),
        )
    })],
    wgsl_utils: &[SDF, BOX_TEX],
    ..NodeDef::EMPTY
};

/// A box projection's blend: three pictures, one along each axis, weighted by how squarely the
/// surface faces it. The normal is the four taps of the distance at `p`.
const BOX_TEX: &str =
    "fn box_tex(p: vec3f, x: vec4f, y: vec4f, z: vec4f, a: f32, b: f32, c: f32, d: f32) -> vec4f {
    let e = vec2f(1.0, -1.0);
    let n = normalize(e.xyy * a + e.yyx * b + e.yxy * c + e.xxx * d);
    var w = pow(abs(n), vec3f(4.0));
    w = w / (w.x + w.y + w.z);
    return x * w.x + y * w.y + z * w.z;
}";
