// SPDX-License-Identifier: AGPL-3.0-or-later

//! Seven more primitives: a box's frame, a superellipsoid, an ellipsoid, a helix, a slab and
//! two of the regular solids.
//!
//! The box frame is Inigo Quilez's exact `sdBoxFrame`, and the slab, the layer between two
//! parallel planes, is exact. The dodecahedron and the icosahedron are hg_sdf's: the largest of
//! the distances to their face planes, exact inside and across each face, a bound beyond an
//! edge or a corner, where the nearest face plane is nearer than the shape.
//!
//! The ellipsoid is IQ's `k0 (k0 − 1) / k1` outside, which a dense search for the nearest point
//! of the surface never found claiming too much, to radii a hundred to one apart. Inside, the
//! same formula can claim more than twice the distance there is, so inside it is `(k0 − 1)`
//! times the shortest radius instead: `k0` grows no faster than one over the shortest radius a
//! unit, so that one cannot.
//!
//! The superellipsoid is `|x|ⁿ + |y|ⁿ + |z|ⁿ = 1` over the radii, and a bound at every
//! exponent, by two arguments. From 1 up, `(|x|ⁿ + |y|ⁿ + |z|ⁿ)^(1/n)` is a norm, so it grows
//! no faster than its largest value on the unit sphere, `3^(1/n − 1/2)` up to 2 and 1 beyond;
//! the distance is the norm less one, divided by that. Below 1 it is not a norm, and its
//! gradient has no bound at all on the coordinate planes, where the star's points and fins
//! are cusps. There `tⁿ` is concave and subadditive, and four bounds follow from that: the
//! sum can change by no more than `3^(1 − n/2) dⁿ` over a distance `d`, which is good far off
//! and slow near the surface; and with one, two or none of the smallest coordinates allowed
//! to fall all the way to zero and the rest held to a linear rate over half their own value,
//! three that are linear near the surface. The largest of the four is a bound, and a dense
//! search at exponents from 0.5 to 20 never found it claiming too much. The search is in the
//! scaled space; the distance is multiplied back by the shortest radius, as Stretch does.
//!
//! The helix is a spring cut flat at both ends, a tube round the curve
//! `(R cos φ, Pφ/2π, R sin φ)`. Over the angle `u` from the point's own, the squared distance
//! to the strand nearest in height is `(ρ − R)² + 4ρR sin²(u/2) + (y − Pu/2π)²`. A lower bound
//! takes `sin(u/2)` at its smallest ratio to `u/2` over the angles the nearest point can be
//! at, which Jordan's inequality first narrows; the next strand up or down, Jordan's alone. An
//! upper bound is the distance to an actual point of the curve, two Newton steps from the
//! nearest point's small-angle estimate. The tube's distance is the lower one less the tube
//! outside, the upper one less the tube inside, and zero in the sliver where the two disagree
//! about which side the point is on; within a tube's radius of the surface it is within a
//! percent of exact, and a dense search over the curve never found it claiming too much.

use super::more::{X, Y, Z, placed};
use super::{SDF, color, num, num_log, out, tint, turn};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::nodes::{Category, NodeDef};

/// A primitive's body: `prelude` over `q`, then the distance `dist` and the node's Color.
fn primitive(node: NodeId, ctx: &mut CompileContext, prelude: &str, dist: &str) -> String {
    let c = tint(node, ctx, "color");
    placed(
        node,
        ctx,
        &format!("{prelude}    return vec4f({dist}, {c});"),
    )
}

/// The superellipsoid's distance bound, in the scaled space, multiplied back by the shortest
/// radius.
const SUPERELLIPSOID_WGSL: &str =
    "// Each component of `v` raised to `n`, zero where it is zero or less.
fn sd_superellipsoid_pow(v: vec3f, n: f32) -> vec3f {
    return select(pow(max(v, vec3f(1.0e-30)), vec3f(n)), vec3f(0.0), v <= vec3f(0.0));
}
// A bound on the signed distance to |x/r.x|^n + |y/r.y|^n + |z/r.z|^n = 1.
fn sd_superellipsoid(p: vec3f, r: vec3f, n: f32) -> f32 {
    let a = abs(p) / r;
    let k = min(r.x, min(r.y, r.z));
    let hi = max(a.x, max(a.y, a.z));
    if (n >= 1.0) {
        let t = sd_superellipsoid_pow(a / max(hi, 1.0e-30), n);
        let g = hi * pow(t.x + t.y + t.z, 1.0 / n);
        return (g - 1.0) * k / pow(3.0, max(1.0 / n - 0.5, 0.0));
    }
    let lo = min(a.x, min(a.y, a.z));
    let mid = max(min(a.x, a.y), min(max(a.x, a.y), a.z));
    let w = sd_superellipsoid_pow(vec3f(lo, mid, hi), n);
    let s = w.x + w.y + w.z;
    let gap = abs(s - 1.0);
    let whole = pow(gap, 1.0 / n) / pow(3.0, 1.0 / n - 0.5);
    if (s >= 1.0) {
        let sl = n * pow(max(vec3f(lo, mid, hi) * 0.5, vec3f(1.0e-20)), vec3f(n - 1.0));
        let c0 = min(gap / (sl.x + sl.y + sl.z), lo * 0.5);
        let c1 = min((gap - w.x) / (sl.y + sl.z), mid * 0.5);
        let c2 = min((gap - w.x - w.y) / sl.z, hi * 0.5);
        return max(whole, max(c0, max(c1, c2))) * k;
    }
    let sl = n * pow(max(vec3f(lo, mid, hi), vec3f(1.0e-20)), vec3f(n - 1.0));
    let i1 = gap / (sl.x + sl.y + sl.z);
    let i2 = min(0.5 * gap / (sl.y + sl.z), pow(0.5 * gap, 1.0 / n));
    let i3 = min(0.5 * gap / sl.z, pow(0.25 * gap, 1.0 / n));
    return -max(whole, max(i1, max(i2, i3))) * k;
}";

/// The helix's distance bound, before its ends are cut.
const SPRING_HELIX: &str =
    "// A bound on the signed distance to a tube `tube` in radius round the upright helix
// `radius` from its axis that rises `pitch` a turn.
fn sd_spring_helix(q: vec3f, radius: f32, pitch: f32, tube: f32) -> f32 {
    let rho = length(q.xz);
    let c = pitch * 0.15915494;
    let th = atan2(q.z, select(q.x, 1.0, rho < 1.0e-12));
    let along = q.y - c * th;
    let y = along - pitch * round(along / pitch);
    let rr = rho * radius;
    let jordan = rr * 0.40528473;
    let radial = (rho - radius) * (rho - radius);
    let reach = min(2.0 * abs(y) * c / (jordan + c * c + 1.0e-12), 3.14159265);
    let h = max(reach * 0.5, 1.0e-6);
    let ak = rr * (sin(h) / h) * (sin(h) / h);
    let nearest = y * y * ak / (ak + c * c + 1.0e-12);
    let y2 = pitch - abs(y);
    let next = y2 * y2 * jordan / (jordan + c * c + 1.0e-12);
    let lower = sqrt(radial + min(nearest, next));
    var ang = y * c / (rr + c * c + 1.0e-12);
    for (var i = 0; i < 2; i++) {
        let d1 = 2.0 * rr * sin(ang) - 2.0 * c * (y - c * ang);
        let d2 = 2.0 * rr * cos(ang) + 2.0 * c * c;
        ang = clamp(ang - d1 / max(d2, 1.0e-12), -3.14159265, 3.14159265);
    }
    let on = 2.0 * rr * (1.0 - cos(ang)) + (y - c * ang) * (y - c * ang);
    let upper = sqrt(radial + min(on, y * y));
    return min(upper - tube, 0.0) + max(lower - tube, 0.0);
}";

pub static BOX_FRAME: NodeDef = NodeDef {
    slug: "boxframe",
    category: Category::Solid,
    icon: "⬚",
    label: "Box Frame",
    tooltip: "The twelve edges of a box, at its center, each a bar Edge thick.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("width", "Width", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("height", "Height", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("depth", "Depth", 1.6, 0.01, 20.0, 0.01, "⬓"),
        num("edge", "Edge", 0.12, 0.0, 5.0, 0.005, "⬓"),
        color("color", "Color", "#7ab8e8ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let b = max(vec3f({width}, {height}, {depth}), vec3f(0.0)) * 0.5;
    let e = clamp({edge}, 0.0, 2.0 * min(b.x, min(b.y, b.z))) * 0.5;
    let f = abs(q) - b;
    let g = abs(f + e) - e;
    let bx = length(max(vec3f(f.x, g.y, g.z), vec3f(0.0))) + min(max(f.x, max(g.y, g.z)), 0.0);
    let by = length(max(vec3f(g.x, f.y, g.z), vec3f(0.0))) + min(max(g.x, max(f.y, g.z)), 0.0);
    let bz = length(max(vec3f(g.x, g.y, f.z), vec3f(0.0))) + min(max(g.x, max(g.y, f.z)), 0.0);
",
            "min(bx, min(by, bz))",
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static SUPERELLIPSOID: NodeDef = NodeDef {
    slug: "superellipsoid",
    category: Category::Solid,
    icon: "▢",
    label: "Superellipsoid",
    tooltip: "A shape between a star, a ball and a box, its radii along each axis. Exponent 2 \
              is an ellipsoid, 1 an octahedron, under 1 a six-pointed star, and higher squares \
              it off toward a box. Its distance is a bound, so a march takes more steps, most \
              of all at a low Exponent.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("rx", "Radius X", 1.0, 0.01, 10.0, 0.01, "⬓"),
        num("ry", "Radius Y", 1.0, 0.01, 10.0, 0.01, "⬓"),
        num("rz", "Radius Z", 1.0, 0.01, 10.0, 0.01, "⬓"),
        num_log("exponent", "Exponent", 4.0, 0.5, 20.0, 0.01, ""),
        color("color", "Color", "#e8a05aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "",
            "sd_superellipsoid(q, max(vec3f({rx}, {ry}, {rz}), vec3f(1.0e-3)), clamp({exponent}, 0.5, 20.0))",
        )
    })],
    wgsl_utils: &[SDF, SUPERELLIPSOID_WGSL],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static ELLIPSOID: NodeDef = NodeDef {
    slug: "ellipsoid",
    category: Category::Solid,
    icon: "🏉",
    label: "Ellipsoid",
    tooltip: "A ball with its own radius along each axis, at its center. Its distance is a \
              bound: close to exact outside, and inside only as good as the shortest radius \
              allows, so a march from inside takes more steps.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("rx", "Radius X", 1.3, 0.01, 10.0, 0.01, "⬓"),
        num("ry", "Radius Y", 0.7, 0.01, 10.0, 0.01, "⬓"),
        num("rz", "Radius Z", 0.9, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#d1a07aff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let r = max(vec3f({rx}, {ry}, {rz}), vec3f(1.0e-3));
    let k0 = length(q / r);
    let k1 = max(length(q / (r * r)), 1.0e-20);
",
            "select(k0 * (k0 - 1.0) / k1, (k0 - 1.0) * min(r.x, min(r.y, r.z)), k0 < 1.0)",
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static HELIX: NodeDef = NodeDef {
    slug: "helix",
    category: Category::Solid,
    icon: "➰",
    label: "Helix",
    tooltip: "An upright spring, at its center: a tube Tube in radius wound Radius from the axis, \
              rising Pitch a turn, cut flat at Height. Its distance is a bound, within a percent \
              of exact near the tube.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("radius", "Radius", 0.9, 0.0, 10.0, 0.01, "⬓"),
        num("pitch", "Pitch", 0.6, 0.01, 10.0, 0.01, "⬓"),
        num("tube", "Tube", 0.12, 0.01, 5.0, 0.01, "⬓"),
        num("height", "Height", 2.4, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#c9ccd6ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let coil = sd_spring_helix(q, max({radius}, 0.0), max({pitch}, 1.0e-3), {tube});\n",
            "max(coil, abs(q.y) - {height} * 0.5)",
        )
    })],
    wgsl_utils: &[SDF, SPRING_HELIX],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static SLAB: NodeDef = NodeDef {
    slug: "slab",
    category: Category::Solid,
    icon: "▬",
    label: "Slab",
    tooltip: "A flat layer at any angle. Its top face is a plane through X, Y and Z, tipped by \
              Pitch, turned by Yaw and moved Offset along its facing; the layer goes Thickness \
              below it. A Thickness of 0 has no bottom: everything below the plane is inside.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        turn("yaw", "Yaw", 0.0, -1.0, 1.0, 0.001),
        turn("pitch", "Pitch", 0.0, -1.0, 1.0, 0.001),
        num("offset", "Offset", 0.0, -10.0, 10.0, 0.01, "⬓"),
        num("thickness", "Thickness", 0.5, 0.0, 20.0, 0.01, "⬓"),
        color("color", "Color", "#a8a2b8ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let tip = {pitch} * 6.2831853;
    let spin = {yaw} * 6.2831853;
    let facing = vec3f(-sin(spin) * sin(tip), cos(tip), cos(spin) * sin(tip));
    let h = dot(q, facing) - {offset};
    let thick = {thickness};
",
            "select(max(h, -h - thick), h, thick <= 0.0)",
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static DODECAHEDRON: NodeDef = NodeDef {
    slug: "dodecahedron",
    category: Category::Solid,
    icon: "⬠",
    label: "Dodecahedron",
    tooltip: "Twelve pentagon faces, its corners Size from the center. Exact inside and across \
              each face, a bound beyond its edges.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("size", "Size", 1.2, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#7ae8a8ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let a = abs(q);
    let faces = max(dot(a, vec3f(0.0, 1.618034, 1.0)), max(dot(a, vec3f(1.0, 0.0, 1.618034)), dot(a, vec3f(1.618034, 1.0, 0.0))));
",
            "faces * 0.52573111 - {size} * 0.79465447",
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static ICOSAHEDRON: NodeDef = NodeDef {
    slug: "icosahedron",
    category: Category::Solid,
    icon: "◬",
    label: "Icosahedron",
    tooltip: "Twenty triangle faces, its corners Size from the center. Exact inside and across \
              each face, a bound beyond its edges.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("size", "Size", 1.2, 0.01, 10.0, 0.01, "⬓"),
        color("color", "Color", "#e87ab0ff"),
    ],
    outputs: &[out(|node, ctx, _| {
        primitive(
            node,
            ctx,
            "    let a = abs(q);
    let corner = (a.x + a.y + a.z) * 0.57735027;
    let side = max(dot(a, vec3f(0.0, 1.0, 2.618034)), max(dot(a, vec3f(2.618034, 0.0, 1.0)), dot(a, vec3f(1.0, 2.618034, 0.0)))) * 0.35682209;
",
            "max(corner, side) - {size} * 0.79465447",
        )
    })],
    wgsl_utils: &[SDF],
    options: crate::nodes::solids::shape_options![],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};
