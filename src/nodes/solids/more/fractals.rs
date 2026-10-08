// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fractals: a Menger sponge and a Mandelbulb, the two solids here whose cost is a choice.
//!
//! Each runs a loop at every point the march asks, so Iterations multiplies the whole render's
//! work: a step, the normal's four taps, the shadow's and the occlusion's all pay it. It is a
//! Code option rather than a number, so the loop's length is written into the code and a cable
//! cannot drive the cost up a frame at a time.
//!
//! **The sponge is a bound.** Each level cuts a cross out of the box, measured in the
//! per-axis way IQ's sponge measures it, whose slope is never more than one; the intersection
//! of bounds is a bound.
//!
//! **The bulb is an estimate.** `0.5 · ln r · r / dr`, the standard escape-time estimate, is
//! right in the limit near the set and can overshoot away from it, so it is scaled down by a
//! quarter. Outside the ball the set is known to lie in — radius 2^(1/(Power − 1)), since
//! any orbit starting further out grows — the distance to that ball is used instead, which is
//! a true bound and cheaper.
//!
//! Both colors come from the iteration: the sponge darkens with the depth of the hole a face
//! belongs to, the bulb with how close its orbit came to the center. The distance-only
//! variant tracks neither.

use super::{X, Y, Z, placed};
use crate::nodes::solids::{SDF, color, num, num_log, out, tint};
use crate::nodes::{Category, Family, NodeDef, OptionDef};

pub static MENGER: NodeDef = NodeDef {
    slug: "menger",
    category: Category::Solid,
    family: Some(Family::Fractals),
    icon: "🧽",
    label: "Menger Sponge",
    tooltip: "A cube with a cross cut through it, and through each smaller cube left, \
              Iterations deep. Every level multiplies what a render costs.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num("size", "Size", 2.0, 0.01, 20.0, 0.01, "⬓"),
        color("color", "Color", "#d6c08aff"),
    ],
    options: crate::nodes::solids::shape_options![OptionDef {
        key: "iterations",
        label: "Iterations",
        default: "3",
        choices: &[("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("5", "5")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let n = match ctx.option(node, "iterations") {
            "1" => 1,
            "2" => 2,
            "4" => 4,
            "5" => 5,
            _ => 3,
        };
        let c = tint(node, ctx, "color");
        let (keep, shade) = if ctx.solid_color() {
            (
                "if (cut > d) { d = cut; lvl = f32(m + 1); }",
                format!("{c} * mix(1.0, 0.45, lvl / {n}.0)"),
            )
        } else {
            ("d = max(d, cut);", c)
        };
        placed(
            node,
            ctx,
            &format!(
                "    let sz = max({{size}}, 1.0e-4) * 0.5;
    let v = q / sz;
    let bx = abs(v) - vec3f(1.0);
    var d = length(max(bx, vec3f(0.0))) + min(max(bx.x, max(bx.y, bx.z)), 0.0);
    var s = 1.0;
    var lvl = 0.0;
    for (var m = 0; m < {n}; m++) {{
        let a = floor_mod3(v * s, vec3f(2.0)) - 1.0;
        s *= 3.0;
        let r = abs(1.0 - 3.0 * abs(a));
        let da = max(r.x, r.y);
        let db = max(r.y, r.z);
        let dc = max(r.z, r.x);
        let cut = (min(da, min(db, dc)) - 1.0) / s;
        {keep}
    }}
    return vec4f(d * sz, {shade});"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};

pub static MANDELBULB: NodeDef = NodeDef {
    slug: "mandelbulb",
    category: Category::Solid,
    family: Some(Family::Fractals),
    icon: "🥦",
    label: "Mandelbulb",
    tooltip: "The three-dimensional Mandelbrot set, scaled by Size, Power its order. \
              Its distance is an estimate the march creeps up on, and Iterations multiplies \
              what a render costs.",
    inputs: crate::nodes::solids::shape_inputs![
        X,
        Y,
        Z,
        num_log("size", "Size", 1.5, 0.05, 20.0, 0.01, "⬓"),
        num("power", "Power", 8.0, 2.0, 16.0, 0.01, ""),
        color("color", "Color", "#7ab8e8ff"),
    ],
    options: crate::nodes::solids::shape_options![OptionDef {
        key: "iterations",
        label: "Iterations",
        default: "6",
        choices: &[("4", "4"), ("6", "6"), ("8", "8"), ("10", "10")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let n = match ctx.option(node, "iterations") {
            "4" => 4,
            "8" => 8,
            "10" => 10,
            _ => 6,
        };
        let c = tint(node, ctx, "color");
        let (track, shade) = if ctx.solid_color() {
            (
                "trap = min(trap, m);",
                format!(
                    "mix({c} * 0.35, {c}, clamp(sqrt(trap), 0.0, 1.0)) + {c}.zxy * 0.15 * (1.0 - clamp(trap, 0.0, 1.0))"
                ),
            )
        } else {
            ("", c.clone())
        };
        placed(
            node,
            ctx,
            &format!(
                "    let sz = max({{size}}, 1.0e-4);
    let v = q / sz;
    let n = clamp({{power}}, 2.0, 16.0);
    let rb = pow(2.0, 1.0 / (n - 1.0));
    let lv = length(v);
    if (lv > rb + 0.1) {{ return vec4f((lv - rb) * sz, {c}); }}
    var w = v;
    var m = dot(w, w);
    var dz = 1.0;
    var trap = 1.0e10;
    for (var i = 0; i < {n}; i++) {{
        let r = max(sqrt(m), 1.0e-6);
        dz = n * pow(r, n - 1.0) * dz + 1.0;
        let th = n * acos(clamp(w.y / r, -1.0, 1.0));
        let ph = n * atan2(w.x, w.z);
        w = v + pow(r, n) * vec3f(sin(th) * sin(ph), cos(th), sin(th) * cos(ph));
        m = dot(w, w);
        {track}
        if (m > 256.0) {{ break; }}
    }}
    let mm = max(m, 1.0e-12);
    return vec4f(0.75 * 0.25 * log(mm) * sqrt(mm) / dz * sz, {shade});"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    row_headings: crate::nodes::solids::ROW_HEADINGS,
    ..NodeDef::EMPTY
};
