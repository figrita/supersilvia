// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cast: a field or a picture laid onto a solid by a projection.
//!
//! **A field on a solid's number is read at the point's front view**, `p.xy`, so a pattern goes
//! straight through the solid like a slide through a projector (`CompileContext::input_at_point`).
//! Cast chooses the projection instead: it reads the point a solid is asking about from
//! `sd_at`, turns it into a `uv` by its Projection — Texture's six, by the same formulas — and
//! reads its input there. In a picture there is no solid asking, and the point is the pixel on
//! the picture's plane, so Front hands its input back unchanged.
//!
//! **Box blends three projections by which way the point lies from the center**, not by the
//! surface's normal as Texture's does: a field has no solid to take a normal from. On a ball
//! the two are the same.
//!
//! Its input is read as a picture, with no solid asking, so a Cast inside a Cast projects once.

use super::{SDF, num, num_log};
use crate::compile::{CompileContext, POINT_STATE};
use crate::graph::NodeId;
use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{
    Category, Control, Family, InputDef, NodeDef, OptionDef, OutputDef, OutputKind,
};

pub static CAST: NodeDef = NodeDef {
    slug: "cast",
    category: Category::Solid,
    family: Some(Family::Surface),
    icon: "📽",
    label: "Cast",
    tooltip: "Lays a field or a picture onto a solid, for any number or color on it: Front, Top \
              and Side cast it straight through from one side, Around wraps it round the up \
              axis, Sphere round the center, and Box from all three sides at once. Scale is how \
              big it is on the solid. A field cabled into a solid without one is cast from the \
              front.",
    inputs: &[
        InputDef {
            key: "field",
            label: "Field",
            ty: VaryingNumber,
            control: Control::None,
        },
        InputDef {
            key: "picture",
            label: "Picture",
            ty: VaryingColor,
            control: Control::None,
        },
        num_log("scale", "Scale", 1.0, 0.05, 20.0, 0.01, "⬓"),
        num("offsetX", "Offset X", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("offsetY", "Offset Y", 0.0, -10.0, 10.0, 0.01, "⬓"),
    ],
    options: &[OptionDef {
        key: "projection",
        label: "Projection",
        default: "front",
        choices: &[
            ("front", "Front"),
            ("top", "Top"),
            ("side", "Side"),
            ("around", "Around"),
            ("sphere", "Sphere"),
            ("box", "Box"),
        ],
        ..OptionDef::EMPTY
    }],
    outputs: &[
        OutputDef {
            key: "value",
            label: "Value",
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _| cast(node, ctx, "field", "0.0"),
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "color",
            label: "Color",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _| cast(node, ctx, "picture", "vec4f(0.0)"),
            ..OutputDef::EMPTY
        },
    ],
    wgsl_utils: &[SDF, POINT_STATE],
    ..NodeDef::EMPTY
};

/// The body of one output: its input, `key`, read at the projection of the point. `zero` is
/// the output's own type's nothing, which a box blend starts from.
fn cast(node: NodeId, ctx: &mut CompileContext, key: &'static str, zero: &str) -> String {
    let at = |ctx: &mut CompileContext, uv: &str| ctx.input(node, key, &format!("({uv}) / s + o"));
    let read = match ctx.option(node, "projection") {
        "top" => at(ctx, "w.xz"),
        "side" => at(ctx, "w.zy"),
        "around" => at(ctx, "vec2f(atan2(w.x, w.z) / PI * (16.0 / 9.0) * s, w.y)"),
        "sphere" => at(
            ctx,
            "vec2f(atan2(w.x, w.z) / PI * (16.0 / 9.0), asin(clamp(normalize(w).y, -1.0, 1.0)) / (PI * 0.5))",
        ),
        "box" => {
            let (x, y, z) = (at(ctx, "w.zy"), at(ctx, "w.xz"), at(ctx, "w.xy"));
            format!("{zero} + {x} * b.x + {y} * b.y + {z} * b.z")
        }
        _ => at(ctx, "w.xy"),
    };
    let scale = ctx.input(node, "scale", "uv");
    let (ox, oy) = (
        ctx.input(node, "offsetX", "uv"),
        ctx.input(node, "offsetY", "uv"),
    );
    format!(
        "    let w = select(vec3f(uv, 0.0), sd_at.xyz, sd_at.w > 0.5);
    let s = max({scale}, 1.0e-4);
    let o = vec2f({ox}, {oy});
    var b = pow(abs(normalize(w + vec3f(1.0e-6))), vec3f(4.0));
    b = b / (b.x + b.y + b.z);
    let was = sd_at;
    sd_at = vec4f(0.0);
    let v = {read};
    sd_at = was;
    return v;"
    )
}
