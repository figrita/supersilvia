// SPDX-License-Identifier: AGPL-3.0-or-later

//! Seams and space: two unions with a shaped seam, an exclusive-or, and three nodes that hand
//! their input a different point — a noise warp, a fold across any plane, and a wrap of a
//! straight row into a ring.
//!
//! Stairs Union and Columns Union are Mercury's `fOpUnionStairs` and `fOpUnionColumns` from
//! hg_sdf. Stairs is a bound as it stands. Columns works on the pair `(a, b)` as a point in a
//! plane, and that pair moves up to √2 times as fast as a point in space where the two
//! surfaces meet at a shallow crease, so its seam term is divided by √2. It is evaluated
//! everywhere rather than only within Radius of both inputs, as hg_sdf gates it, so the
//! distance is continuous. The color across either seam is `sd_union`'s blend at that radius.
//!
//! Warp pushes the point by a vector value noise. Its distance is divided by
//! `1 + Amount · Frequency · 6.4952`: a component of the noise changes at most 3.75 a cell,
//! along an axis where two corners differ by the full 2 and the quintic fade is steepest, and
//! three components hashed apart reach that together, √3 · 3.75. Phase carries the noise
//! round a circle, so a whole turn comes back to where it began.
//!
//! Wrap measures angle round Y as length along X. Inside the ring it stretches space by
//! `R / ρ`, and dividing by that at the point alone is no bound — a march near the axis sees
//! the whole row crushed onto it — so the distance is the furthest step that cannot reach
//! anything the input reports: the stretch integrated along a line toward the axis, and,
//! for a march far from everything, the length of the whole row. That step shrinks to nothing
//! at the axis itself, where a ray aimed through it would creep up on it forever, so outside
//! the input a ray in the axis's thin core steps a fiftieth of the radius, which is safe for
//! anything not wrapped onto the axis. Across the seam behind the ring the input is cut unless
//! it repeats every `2πR` along X.

use super::{SDF, fill, num, num_log, out, solid, turn};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::nodes::{Category, NodeDef};

/// The seam helpers and the warp's noise.
const BLENDS: &str = "// Seams and warps.
fn sd_stairs_union(a: f32, b: f32, r: f32, n: f32) -> f32 {
    let m = min(a, b);
    if (r <= 0.0) { return m; }
    let s = r / n;
    let e = b - r;
    let w = e - a + s;
    let t = w - 2.0 * s * floor(w / (2.0 * s));
    return min(m, 0.5 * (e + a + abs(t - s)));
}
fn sd_columns_union(a: f32, b: f32, r: f32, n: f32) -> f32 {
    let m = min(a, b);
    if (r <= 0.0) { return m; }
    let c = r * 1.4142136 / ((n - 1.0) * 2.0 + 1.4142136);
    var v = vec2f(a + b, b - a) * 0.70710678;
    v.x = v.x - 0.70710678 * r + c * 1.4142136;
    if (n - 2.0 * floor(n * 0.5) > 0.5) { v.y = v.y + c; }
    v.y = v.y - 2.0 * c * floor((v.y + c) / (2.0 * c));
    return min(m, min(length(v) - c, v.x) * 0.70710678);
}
// The color across a seam Radius wide, blended as `sd_union` blends it.
fn sd_seam_tint(a: vec4f, b: vec4f, r: f32) -> vec3f {
    if (r <= 0.0) { return select(b.yzw, a.yzw, a.x < b.x); }
    let h = clamp(0.5 + 0.5 * (b.x - a.x) / r, 0.0, 1.0);
    return mix(b.yzw, a.yzw, h);
}
// Three numbers, -1 to 1, smooth in `x`: value noise on the whole-numbered lattice with a
// quintic fade.
fn sd_warp_noise(x: vec3f) -> vec3f {
    let i = floor(x);
    let f = x - i;
    let g = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x00 = mix(sd_hash3(i, 0.0), sd_hash3(i + vec3f(1.0, 0.0, 0.0), 0.0), g.x);
    let x10 = mix(sd_hash3(i + vec3f(0.0, 1.0, 0.0), 0.0), sd_hash3(i + vec3f(1.0, 1.0, 0.0), 0.0), g.x);
    let x01 = mix(sd_hash3(i + vec3f(0.0, 0.0, 1.0), 0.0), sd_hash3(i + vec3f(1.0, 0.0, 1.0), 0.0), g.x);
    let x11 = mix(sd_hash3(i + vec3f(0.0, 1.0, 1.0), 0.0), sd_hash3(i + vec3f(1.0, 1.0, 1.0), 0.0), g.x);
    return mix(mix(x00, x10, g.y), mix(x01, x11, g.y), g.z) * 2.0 - 1.0;
}";

/// A union of A and B whose seam distance is `call`, over `a`, `b`, `r` and the rounded
/// count `n`.
fn seam(node: NodeId, ctx: &mut CompileContext, count: &str, call: &str) -> String {
    let a = ctx.input(node, "a", "p");
    let b = ctx.input(node, "b", "p");
    let color = if ctx.solid_color() {
        "sd_seam_tint(a, b, r)"
    } else {
        "vec3f(0.0)"
    };
    fill(
        node,
        ctx,
        &format!(
            "    let a = {a};
    let b = {b};
    let r = max({{radius}}, 0.0);
    let n = max(round({count}), 1.0);
    return vec4f({call}, {color});"
        ),
    )
}

pub static STAIRS_UNION: NodeDef = NodeDef {
    slug: "stairsunion",
    category: Category::Solid,
    icon: "📶",
    label: "Stairs Union",
    tooltip: "A and B as one, joined by a flight of Steps stairs Radius wide.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        num("radius", "Radius", 0.5, 0.0, 2.0, 0.01, "⬓"),
        num("steps", "Steps", 4.0, 1.0, 20.0, 1.0, ""),
    ],
    outputs: &[out(|node, ctx, _| {
        seam(node, ctx, "{steps}", "sd_stairs_union(a.x, b.x, r, n)")
    })],
    wgsl_utils: &[SDF, BLENDS],
    ..NodeDef::EMPTY
};

pub static COLUMNS_UNION: NodeDef = NodeDef {
    slug: "columnsunion",
    category: Category::Solid,
    icon: "🏛",
    label: "Columns Union",
    tooltip: "A and B as one, the seam filled Radius wide and fluted with a row of Columns \
              round columns.",
    inputs: &[
        solid("a", "A"),
        solid("b", "B"),
        num("radius", "Radius", 0.5, 0.0, 2.0, 0.01, "⬓"),
        num("columns", "Columns", 4.0, 1.0, 20.0, 1.0, ""),
    ],
    outputs: &[out(|node, ctx, _| {
        seam(node, ctx, "{columns}", "sd_columns_union(a.x, b.x, r, n)")
    })],
    wgsl_utils: &[SDF, BLENDS],
    ..NodeDef::EMPTY
};

pub static XOR: NodeDef = NodeDef {
    slug: "solidxor",
    category: Category::Solid,
    icon: "⊻",
    label: "Exclusive",
    tooltip: "Where exactly one of A and B is: their overlap is hollowed out. Each part keeps \
              its own color.",
    inputs: &[solid("a", "A"), solid("b", "B")],
    outputs: &[out(|node, ctx, _| {
        let a = ctx.input(node, "a", "p");
        let b = ctx.input(node, "b", "p");
        let color = if ctx.solid_color() {
            "select(b.yzw, a.yzw, a.x < b.x)"
        } else {
            "vec3f(0.0)"
        };
        format!(
            "    let a = {a};
    let b = {b};
    return vec4f(max(min(a.x, b.x), -max(a.x, b.x)), {color});"
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static WARP: NodeDef = NodeDef {
    slug: "warp",
    category: Category::Solid,
    icon: "🌊",
    label: "Warp",
    tooltip: "Pushes space around by a smooth noise, up to Amount on each axis, Frequency cells \
              to the unit. Cable a gear's Cycles into Phase to set it flowing. Stronger warps \
              take more steps.",
    inputs: &[
        solid("input", "Solid"),
        num("amount", "Amount", 0.3, 0.0, 2.0, 0.001, "⬓"),
        num_log("frequency", "Frequency", 1.0, 0.05, 10.0, 0.01, "/⬓"),
        turn("phase", "Phase", 0.0, 0.0, 1.0, 0.001),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let a = {{amount}};
    let f = {{frequency}};
    let t = {{phase}} * 6.2831853;
    let q = p + a * sd_warp_noise(p * f + vec3f(cos(t), sin(t), 0.0) * 0.5);
    let r = {inner};
    return vec4f(r.x / (1.0 + abs(a * f) * 6.4951905), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF, BLENDS],
    ..NodeDef::EMPTY
};

pub static FOLD_PLANE: NodeDef = NodeDef {
    slug: "foldplane",
    category: Category::Solid,
    icon: "📐",
    label: "Fold Plane",
    tooltip: "Mirrors space across a plane, so the side its normal points to is drawn on both. \
              Yaw and Pitch aim the normal, from +X; Offset moves the plane along it.",
    inputs: &[
        solid("input", "Solid"),
        turn("yaw", "Yaw", 0.0, -1.0, 1.0, 0.001),
        turn("pitch", "Pitch", 0.0, -0.25, 0.25, 0.001),
        num("offset", "Offset", 0.0, -10.0, 10.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let yaw = {{yaw}} * 6.2831853;
    let pitch = {{pitch}} * 6.2831853;
    let n = vec3f(cos(pitch) * cos(yaw), sin(pitch), cos(pitch) * sin(yaw));
    let h = dot(p, n) - {{offset}};
    let q = p - 2.0 * min(h, 0.0) * n;
    return {inner};"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static WRAP: NodeDef = NodeDef {
    slug: "wrap",
    category: Category::Solid,
    icon: "🍩",
    label: "Wrap",
    tooltip: "Bends the X axis into a ring of Radius round Y; the ring's circumference is 2π × \
              Radius. Repeat along X with a spacing that divides it for a seamless ring. \
              Inside the ring the march slows down.",
    inputs: &[
        solid("input", "Solid"),
        num_log("radius", "Radius", 3.0, 0.1, 50.0, 0.01, "⬓"),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        fill(
            node,
            ctx,
            &format!(
                "    let rr = max({{radius}}, 1.0e-3);
    let rho = length(p.xz);
    let q = vec3f(atan2(p.z, select(p.x, 1.0e-9, rho < 1.0e-9)) * rr, p.y, rr - rho);
    let r = {inner};
    let d = abs(r.x);
    let m = max(rho - rr, 0.0);
    let near = select(rho - min(rho, rr) * exp((m - d) / rr), d, d <= m);
    let far = sqrt(max(d * d - 39.478418 * rr * rr, 0.0));
    let core = select(0.0, 0.02 * rr, rho < 0.02 * rr && r.x > 0.0);
    return vec4f(sign(r.x) * max(max(near, far), core), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
