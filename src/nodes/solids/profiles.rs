// SPDX-License-Identifier: AGPL-3.0-or-later

//! Solids made from a picture: pushed out into a slab, spun around an axis, or raised into
//! terrain.
//!
//! **A picture's value is not a distance.** A march steps by the distance a solid reports, and
//! a picture read as a shape — its brightness or its alpha over a threshold — only says inside
//! or outside. So each node here takes a Steepness: the fastest the value can change per unit
//! of the picture (whose frame is two units tall). Where the value is `v`, the edge at
//! Threshold is at least `|Threshold − v| / Steepness` away, and that is the distance each
//! node reports, scaled into the world. It is a bound as long as the picture is no steeper
//! than Steepness says; a picture steeper than that oversteps and tears at its edges, and a
//! Steepness far above the picture's own makes the march creep. A soft edge is what makes a
//! low Steepness true: a shape's own Softness, or a blurred texture such as a Text. A Blur of
//! a drawn shape is not one — it is a staircase of its taps, each step as steep as the edge. The bound never reaches further than the
//! value's own range allows, so inside the picture's frame the march steps at most
//! `Threshold / Steepness`. Outside the frame the picture counts as empty, whatever a texture's
//! wrap repeats there: the distance is the larger of the distance to the frame and the bound
//! at the frame's nearest point, so the two meet at the frame's edge without a seam a ray
//! could stop on, and a far ray crosses the empty space in a step. A Bevel rounds within the
//! same reach as the bound.
//!
//! **The picture is the shape**, so unlike Texture's it is asked in the distance alone too,
//! once a march step, wherever the point is near enough for it to matter. A heavy chain behind
//! it is paid for at every step; an Output's Frame, one texture read, is cheapest. Color from
//! the picture is read in the whole of the solid only, once a pixel
//! (`CompileContext::solid_input`).

use super::{color, fill, num, num_log, out, tint};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef};

/// The picture's frame, 16/9 wide each side of the center and one unit tall each side: the
/// signed distance from a picture-space `uv` to its edge, and the nearest point of it to `uv`,
/// which is `uv` itself inside it.
const FRAME: &str = "fn sd_profile_frame(uv: vec2f) -> f32 {
    let b = abs(uv) - vec2f(16.0 / 9.0, 1.0);
    return length(max(b, vec2f(0.0))) + min(max(b.x, b.y), 0.0);
}
fn sd_profile_clamp(uv: vec2f) -> vec2f {
    return clamp(uv, vec2f(-16.0 / 9.0, -1.0), vec2f(16.0 / 9.0, 1.0));
}";

const PROFILE: InputDef = InputDef {
    key: "profile",
    label: "Profile",
    ty: VaryingColor,
    control: Control::None,
};

const PICTURE: InputDef = InputDef {
    key: "picture",
    label: "Picture",
    ty: VaryingColor,
    control: Control::None,
};

const THRESHOLD: InputDef = num("threshold", "Threshold", 0.5, 0.0, 1.0, 0.01, "");

const READ: OptionDef = OptionDef {
    key: "read",
    label: "Read",
    default: "brightness",
    choices: &[("brightness", "Brightness"), ("alpha", "Alpha")],
    ..OptionDef::EMPTY
};

const PAINT: OptionDef = OptionDef {
    key: "paint",
    label: "Colored by",
    default: "color",
    choices: &[("color", "Color"), ("picture", "Picture")],
    ..OptionDef::EMPTY
};

/// The block that asks the picture input `key` at `uv` and runs `set` with its value in `v`,
/// 0 to 1, where `near` holds; and the color the solid takes from it, `c` being the picture's
/// premultiplied color. The whole variant colored by the picture asks it unconditionally,
/// since its color is wanted wherever the ray lands; the distance stays the same in both.
fn ask(
    node: NodeId,
    ctx: &mut CompileContext,
    key: &'static str,
    uv: &str,
    near: &str,
    set: &str,
) -> (String, String) {
    let value = match ctx.option(node, "read") {
        "alpha" => "c.a",
        _ => "dot(c.rgb, vec3f(0.2126, 0.7152, 0.0722))",
    };
    let sample = ctx.input(node, key, uv);
    let painted = ctx.solid_color() && ctx.option(node, "paint") == "picture";
    let block = if painted {
        format!(
            "    let c = {sample};
    if ({near}) {{
        let v = clamp({value}, 0.0, 1.0);
{set}    }}
"
        )
    } else {
        format!(
            "    if ({near}) {{
        let c = {sample};
        let v = clamp({value}, 0.0, 1.0);
{set}    }}
"
        )
    };
    let color = if painted {
        "unpremultiply(c).rgb".to_string()
    } else {
        tint(node, ctx, "color")
    };
    (block, color)
}

pub static EXTRUDE: NodeDef = NodeDef {
    slug: "extrude",
    category: Category::Solid,
    icon: "⬛",
    label: "Extrude",
    tooltip: "Pushes a picture out into a slab Depth deep: a Text, a shape, an Output's Frame, \
              a camera. It is solid where the picture's Brightness or Alpha is over Threshold, \
              and its frame is Scale × 2 tall. Steepness is the fastest that value changes per \
              unit of the picture, about one over the width of its soft edge: too low and the \
              edges tear, too high and the march crawls. A soft edge — a shape's Softness, or \
              a blurred Text — lets a low Steepness draw smooth and fast, and Bevel rounds as \
              far as that soft edge reaches; a Blur of a drawn shape speckles instead. Colored by Picture puts the picture's own colors on the front and \
              back. The picture is computed at every step of the march, so a heavy chain is \
              slow; an Output's Frame is cheapest.",
    inputs: crate::nodes::solids::shape_inputs![
        PROFILE,
        num_log("scale", "Scale", 1.0, 0.05, 20.0, 0.01, "⬓"),
        num("depth", "Depth", 0.4, 0.0, 10.0, 0.01, "⬓"),
        THRESHOLD,
        num_log("steepness", "Steepness", 10.0, 0.5, 1000.0, 0.1, "/⬓"),
        num("bevel", "Bevel", 0.02, 0.0, 1.0, 0.001, "⬓"),
        color("color", "Color", "#e8e2d6ff"),
    ],
    options: crate::nodes::solids::shape_options![READ, PAINT],
    outputs: &[out(|node, ctx, _| {
        let (block, picture) = ask(
            node,
            ctx,
            "profile",
            "sd_profile_clamp(uv)",
            "frame <= t * s / k && gap <= t * s / k",
            "        d2 = max((t - v) * s / k, frame);\n",
        );
        // Colored by the picture, the side walls keep Color.
        let c = if ctx.solid_color() && ctx.option(node, "paint") == "picture" {
            let side = tint(node, ctx, "color");
            format!("select({side}, {picture}, gap > d2)")
        } else {
            picture
        };
        fill(
            node,
            ctx,
            &format!(
                "    let s = max({{scale}}, 1.0e-4);
    let k = max({{steepness}}, 1.0e-3);
    let t = clamp({{threshold}}, 0.0, 1.0);
    let half = max({{depth}}, 0.0) * 0.5;
    let r = clamp({{bevel}}, 0.0, min(half, 0.5 * (1.0 - t) * s / k));
    let uv = p.xy / s;
    let frame = sd_profile_frame(uv) * s;
    let gap = abs(p.z) - half;
    var d2 = frame;
{block}    let w = vec2f(d2 + r, gap + r);
    return vec4f(min(max(w.x, w.y), 0.0) + length(max(w, vec2f(0.0))) - r, {c});"
            ),
        )
    })],
    wgsl_utils: &[super::SDF, FRAME],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static REVOLVE: NodeDef = NodeDef {
    slug: "revolve",
    category: Category::Solid,
    icon: "🏺",
    label: "Revolve",
    tooltip: "Spins a picture around the upright axis, as a lathe turns a vase: the picture's \
              right half is the outline, its center on the axis, or Offset out from it for a \
              ring. It is solid where the picture's Brightness or Alpha is over Threshold, and \
              its frame is Scale × 2 tall. Steepness is the fastest that value changes per unit \
              of the picture, about one over the width of its soft edge: too low and the edges \
              tear, too high and the march crawls. Colored by Picture takes the outline's own \
              colors. The picture is computed at every step of the march, so a heavy chain is \
              slow; an Output's Frame is cheapest.",
    inputs: crate::nodes::solids::shape_inputs![
        PROFILE,
        num_log("scale", "Scale", 1.0, 0.05, 20.0, 0.01, "⬓"),
        num("offset", "Offset", 0.0, 0.0, 10.0, 0.01, "⬓"),
        THRESHOLD,
        num_log("steepness", "Steepness", 10.0, 0.5, 1000.0, 0.1, "/⬓"),
        color("color", "Color", "#d6a35aff"),
    ],
    options: crate::nodes::solids::shape_options![READ, PAINT],
    outputs: &[out(|node, ctx, _| {
        let (block, c) = ask(
            node,
            ctx,
            "profile",
            "sd_profile_clamp(uv)",
            "frame <= t * s / k",
            "        d = max((t - v) * s / k, frame);\n",
        );
        fill(
            node,
            ctx,
            &format!(
                "    let s = max({{scale}}, 1.0e-4);
    let k = max({{steepness}}, 1.0e-3);
    let t = clamp({{threshold}}, 0.0, 1.0);
    let uv = vec2f(length(p.xz) - {{offset}}, p.y) / s;
    let frame = sd_profile_frame(uv) * s;
    var d = frame;
{block}    return vec4f(d, {c});"
            ),
        )
    })],
    wgsl_utils: &[super::SDF, FRAME],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static HEIGHTFIELD: NodeDef = NodeDef {
    slug: "heightfield",
    category: Category::Solid,
    icon: "⛰",
    label: "Heightfield",
    tooltip: "Raises a picture into terrain: the ground stands at Base plus Height times the \
              picture's Brightness or Alpha, the picture lying flat with its top away from the \
              front and Scale × 2 deep. Any video chain becomes a landscape, and a sound-driven \
              one moves it. Steepness is the fastest the picture changes per unit of it: too low \
              and the slopes tear, too high and the march crawls; a smooth picture lets \
              Steepness come down. Colored by Picture paints the picture onto \
              the ground. The picture is computed at every step of the march, so a heavy chain \
              is slow; an Output's Frame is cheapest.",
    inputs: crate::nodes::solids::shape_inputs![
        PICTURE,
        num_log("scale", "Scale", 3.0, 0.05, 50.0, 0.01, "⬓"),
        num("base", "Base", -1.0, -10.0, 10.0, 0.01, "⬓"),
        num("height", "Height", 0.6, -10.0, 10.0, 0.01, "⬓"),
        num_log("steepness", "Steepness", 4.0, 0.1, 1000.0, 0.1, "/⬓"),
        color("color", "Color", "#7a9a6aff"),
    ],
    options: crate::nodes::solids::shape_options![READ, PAINT],
    outputs: &[out(|node, ctx, _| {
        let (block, c) = ask(
            node,
            ctx,
            "picture",
            "uv",
            "d <= abs(hgt)",
            "        d = max((p.y - ground - hgt * v) / lip, p.y - top);\n",
        );
        fill(
            node,
            ctx,
            &format!(
                "    let s = max({{scale}}, 1.0e-4);
    let hgt = {{height}};
    let ground = {{base}};
    let top = ground + max(hgt, 0.0);
    let g = hgt * max({{steepness}}, 0.0) / s;
    let lip = sqrt(1.0 + g * g);
    let uv = vec2f(p.x, -p.z) / s;
    var d = p.y - top;
{block}    return vec4f(d, {c});"
            ),
        )
    })],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    wgsl_utils: &[super::SDF],
    ..NodeDef::EMPTY
};
