// SPDX-License-Identifier: AGPL-3.0-or-later

//! Solids: shapes in three dimensions, and the node that turns one into a picture.
//!
//! A `Solid` port is `fn f(p: vec3f) -> vec4f`: the signed distance from `p` to the surface in
//! `.x` and the color there in `.yzw`. Everything here is the video graph's rule one axis up —
//! every output is a function and every cable is a call. A primitive measures a distance, a
//! boolean combines two, a transform hands its input a different point the way `zoom` hands
//! its input a different `uv`, and [`render`] marches a ray per pixel into whatever arrives.
//! Its own output is an ordinary `VaryingColor` field, so an ordinary Output shows a solid and
//! every video node can take the picture from there. See
//! [docs/nodes.md](../../../docs/nodes.md#solids).
//!
//! **Each solid compiles twice**, whole and as its distance alone, so a march pays for color
//! once a pixel rather than once a step (`CompileContext::solid_input`).
//!
//! **A distance stays a bound.** Sphere tracing steps by the distance a solid reports, so a
//! node that stretches space — a non-uniform scale, a twist, a bend, a displacement — divides
//! its distance by how far it can stretch. A march then never steps through a surface; it
//! takes more steps instead, which the render's Steps shading shows.
//!
//! **Every number input of a solid takes a field**, as a video node's does. A solid has a point
//! and a field a `uv`, so a field is read at the point's front view, `p.xy`
//! (`CompileContext::input_at_point`), and Cast reads it by another projection. Render 3D's
//! numbers are uniforms: its camera is not inside any solid.

pub mod blends;
pub mod cast;
pub mod colorfields;
pub mod combine;
pub mod iterate;
pub mod more;
pub mod primitives;
pub mod profiles;
pub mod render;
pub mod repeat;
pub mod shapes;
pub mod space;
pub mod surface;

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::{Solid, UniformColor, UniformNumber, VaryingNumber};
use crate::nodes::{Control, GenFn, InputDef, OptionDef, OptionKind, OutputDef, OutputKind};
use std::fmt::Write as _;

/// The helpers every solid node shares, emitted once into any module that holds one.
pub const SDF: &str = "// Solids.
fn sd_rot2(v: vec2f, a: f32) -> vec2f {
    let c = cos(a);
    let s = sin(a);
    return vec2f(c * v.x - s * v.y, s * v.x + c * v.y);
}
// Turn a point by yaw (around y), pitch (around x) and roll (around z), in radians.
fn sd_turn(p: vec3f, a: vec3f) -> vec3f {
    let xz = sd_rot2(p.xz, a.x);
    var q = vec3f(xz.x, p.y, xz.y);
    let yz = sd_rot2(q.yz, a.y);
    q = vec3f(q.x, yz.x, yz.y);
    let xy = sd_rot2(q.xy, a.z);
    return vec3f(xy.x, xy.y, q.z);
}
// Smooth minimum with its blend carried into the color, so a seam between two colors is as
// soft as the seam between the two shapes. A radius of zero is the hard union.
fn sd_union(a: vec4f, b: vec4f, k: f32) -> vec4f {
    if (k <= 0.0) { return select(b, a, a.x < b.x); }
    let h = clamp(0.5 + 0.5 * (b.x - a.x) / k, 0.0, 1.0);
    return vec4f(mix(b.x, a.x, h) - k * h * (1.0 - h), mix(b.yzw, a.yzw, h));
}
// `a` with `b` carved out of it. The cut face takes `b`'s color.
fn sd_subtract(a: vec4f, b: vec4f, k: f32) -> vec4f {
    if (k <= 0.0) { return select(a, vec4f(-b.x, b.yzw), -b.x > a.x); }
    let h = clamp(0.5 - 0.5 * (a.x + b.x) / k, 0.0, 1.0);
    return vec4f(mix(a.x, -b.x, h) + k * h * (1.0 - h), mix(a.yzw, b.yzw, h));
}
fn sd_intersect(a: vec4f, b: vec4f, k: f32) -> vec4f {
    if (k <= 0.0) { return select(b, a, a.x > b.x); }
    let h = clamp(0.5 - 0.5 * (b.x - a.x) / k, 0.0, 1.0);
    return vec4f(mix(b.x, a.x, h) + k * h * (1.0 - h), mix(b.yzw, a.yzw, h));
}
// Three numbers, 0 to 1, from a whole-numbered cell and a seed: integer arithmetic only, so a
// cell scatters the same way on every GPU and in every session.
fn sd_hash3(cell: vec3f, seed: f32) -> vec3f {
    var v = bitcast<vec3u>(vec3i(cell) + vec3i(i32(seed) * 1597, i32(seed) * 3571, i32(seed) * 7919));
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3u(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3f(v) * (1.0 / 4294967295.0);
}
// A fully saturated hue, 0 to 1 round the wheel.
fn sd_hue(h: f32) -> vec3f {
    return clamp(abs(fract(h + vec3f(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3f(0.0), vec3f(1.0));
}";

/// **The chain.** Every shape node — a primitive, a fractal, a profile — takes the solid built
/// so far on its first row, [`ONTO`], and combines its own shape with it by [`MODE`] and
/// [`BLEND`], so a scene is a row of shapes cabled one into the next rather than a tree of
/// unions. It also turns and scales its own shape about its own center under a folded
/// [`TRANSFORM`] heading. A shape's generator writes only its own shape; the compiler wraps
/// it ([`chain`]), so the wrapping is written once for every shape. A shape node declares its
/// inputs with [`shape_inputs!`], its options with [`shape_options!`] and [`ROW_HEADINGS`].
pub const ONTO: InputDef = solid("onto", "Onto");

/// How far the shape melts into what it lands on.
pub const BLEND: InputDef = num("blend", "Blend", 0.0, 0.0, 2.0, 0.01, "⬓");

/// How the shape lands on what came before: added, carved out of it, or kept only where both
/// are. With nothing plugged into Onto it is the shape alone, whatever the mode.
pub const MODE: OptionDef = OptionDef {
    key: "mode",
    label: "Mode",
    default: "union",
    choices: &[
        ("union", "Union"),
        ("subtract", "Subtract"),
        ("intersect", "Intersect"),
    ],
    ..OptionDef::EMPTY
};

/// The heading the turn and the scale fold under: closed on a new node.
pub const TRANSFORM: OptionDef =
    OptionDef::heading("transform", "Transform", false, OptionKind::Presentation);

/// The rows [`TRANSFORM`] folds: a turn about the shape's own center and a scale.
pub const T_YAW: InputDef = turn("tYaw", "Yaw", 0.0, -1.0, 1.0, 0.001);
pub const T_PITCH: InputDef = turn("tPitch", "Pitch", 0.0, -1.0, 1.0, 0.001);
pub const T_ROLL: InputDef = turn("tRoll", "Roll", 0.0, -1.0, 1.0, 0.001);
pub const T_SCALE: InputDef = num_log("tScale", "Scale", 1.0, 0.05, 20.0, 0.01, "x");
pub const TRANSFORM_ROWS: [InputDef; 4] = [T_YAW, T_PITCH, T_ROLL, T_SCALE];

/// A shape node's inputs: [`ONTO`], its own, then [`BLEND`] and the [`TRANSFORM_ROWS`].
macro_rules! shape_inputs {
    ($($own:expr),* $(,)?) => {
        &[
            $crate::nodes::solids::ONTO,
            $($own,)*
            $crate::nodes::solids::BLEND,
            $crate::nodes::solids::T_YAW,
            $crate::nodes::solids::T_PITCH,
            $crate::nodes::solids::T_ROLL,
            $crate::nodes::solids::T_SCALE,
        ]
    };
}
pub(crate) use shape_inputs;

/// A shape node's options: its own, then [`CHAIN_OPTIONS`]'s.
macro_rules! shape_options {
    ($($own:expr),* $(,)?) => {
        &[$($own,)* $crate::nodes::solids::MODE, $crate::nodes::solids::TRANSFORM]
    };
}
pub(crate) use shape_options;

/// The row headings a shape node declares: the one [`TRANSFORM`].
pub const ROW_HEADINGS: &[&str] = &[TRANSFORM.key];

/// Whether an input folds under the Transform heading.
pub fn is_transform_row(key: &str) -> bool {
    TRANSFORM_ROWS.iter().any(|r| r.key == key)
}

/// Whether a node is a shape in the chain: it declares [`ONTO`].
pub fn is_shape(def: &crate::nodes::NodeDef) -> bool {
    def.inputs
        .iter()
        .any(|i| i.key == ONTO.key && i.ty.is_solid())
}

/// A shape node's function around its own shape, whose function is `own`: the point turned
/// and scaled about the shape's center — its X, Y and Z where it has them — the shape's
/// distance scaled back, and the result combined with Onto by the mode. Called by the
/// compiler for every node [`is_shape`] says is one.
pub fn chain(node: NodeId, ctx: &mut CompileContext, own: &str) -> String {
    let Some(def) = ctx.node(node).map(|n| n.def) else {
        return format!("    return {own}(p);");
    };
    let has = |key: &str| def.inputs.iter().any(|i| i.key == key);
    let pivot = if has("x") && has("y") && has("z") {
        "vec3f({x}, {y}, {z})"
    } else {
        "vec3f(0.0)"
    };
    let mut body = format!(
        "    let pivot = {pivot};
    let s = max({{tScale}}, 1.0e-4);
    let local = pivot + sd_turn(p - pivot, -vec3f({{tYaw}}, {{tPitch}}, {{tRoll}}) * 6.2831853) / s;
    let o = {own}(local);
    let me = vec4f(o.x * s, o.yzw);
"
    );
    if ctx.connected(node, ONTO.key) {
        let op = match ctx.option(node, MODE.key) {
            "subtract" => "sd_subtract",
            "intersect" => "sd_intersect",
            _ => "sd_union",
        };
        let onto = ctx.input(node, ONTO.key, "p");
        let _ = write!(body, "    return {op}({onto}, me, {{blend}});");
    } else {
        body.push_str("    return me;");
    }
    fill(node, ctx, &body)
}

/// A number input on a solid node: a control, a CPU node's number, or a field read at the
/// point.
pub(crate) const fn num(
    key: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &'static str,
) -> InputDef {
    InputDef {
        key,
        label,
        ty: VaryingNumber,
        control: Control::num(default, min, max, step, unit),
    }
}

/// An angle on a solid node that its body reads round a whole turn, through a rotation, a
/// sine or a cosine alone: [`Control::turn`].
pub(crate) const fn turn(
    key: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
) -> InputDef {
    InputDef {
        key,
        label,
        ty: VaryingNumber,
        control: Control::turn(default, min, max, step),
    }
}

/// A number input scrubbed in log space, for a scale that spans decades.
pub(crate) const fn num_log(
    key: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &'static str,
) -> InputDef {
    InputDef {
        key,
        label,
        ty: VaryingNumber,
        control: Control::num_log(default, min, max, step, unit),
    }
}

/// The same input as one value a frame, for a node outside every solid — Render 3D, whose
/// camera is one camera for the picture — or one whose number its CPU half reads.
pub(crate) const fn uniform(input: InputDef) -> InputDef {
    InputDef {
        ty: UniformNumber,
        ..input
    }
}

/// A color input on a solid node.
pub(crate) const fn color(
    key: &'static str,
    label: &'static str,
    default: &'static str,
) -> InputDef {
    InputDef {
        key,
        label,
        ty: UniformColor,
        control: Control::color(default),
    }
}

/// A solid input. Unconnected, it is nothing, a long way off.
pub(crate) const fn solid(key: &'static str, label: &'static str) -> InputDef {
    InputDef {
        key,
        label,
        ty: Solid,
        control: Control::None,
    }
}

/// The one output every solid node has.
pub(crate) const fn out(wgsl: GenFn) -> OutputDef {
    OutputDef {
        key: "solid",
        label: "Solid",
        ty: Solid,
        kind: OutputKind::Shader,
        wgsl,
        ..OutputDef::EMPTY
    }
}

/// Fill every `{key}` hole in `src` naming a number or color input of `node`, a port's or a
/// hidden control's, with its value.
/// A solid input is not filled here: each generator calls it at the point it means.
pub(crate) fn fill(node: NodeId, ctx: &mut CompileContext, src: &str) -> String {
    let Some(def) = ctx.node(node).map(|n| n.def) else {
        return src.to_string();
    };
    let mut out = src.to_string();
    for input in def.inputs.iter().chain(def.hidden) {
        let hole = ["{", input.key, "}"].concat();
        if input.ty.is_solid() || !out.contains(&hole) {
            continue;
        }
        let expr = ctx.input_at_point(node, input.key, "p");
        out = out.replace(&hole, &expr);
    }
    out
}

/// A color input's rgb, in the whole of a solid; zero in its distance alone, which never reads
/// it.
pub(crate) fn tint(node: NodeId, ctx: &mut CompileContext, key: &'static str) -> String {
    if ctx.solid_color() {
        format!("{}.rgb", ctx.input(node, key, "p"))
    } else {
        "vec3f(0.0)".to_string()
    }
}
