// SPDX-License-Identifier: AGPL-3.0-or-later

//! Iterated solids: a kaleidoscopic fold, copies spread over a sphere, and noise on a surface.
//! Each evaluates its input once, at a point it works out, so what it adds costs the same
//! however much detail shows.
//!
//! **Fold is the demoscene's kaleidoscopic IFS.** Each iteration folds space onto one wedge of
//! a symmetry — mirrors, so never a stretch — turns it and scales it about the Offset point.
//! The input is evaluated once at the final point and its distance divided by the scale
//! accumulated on the way, which makes the whole a bound: folds and turns never lengthen a
//! step and the division undoes every scale. The iteration count is a Code option, so the loop
//! unrolls to a fixed length and a cable cannot drive the cost up.
//!
//! **Scatter on Sphere finds the nearest copy without looking at the others.** The copies sit
//! on a spherical Fibonacci lattice, and Keinert, Innmann, Sänger and Stamminger's inverse
//! mapping (*Spherical Fibonacci Mapping*, 2015, as in Inigo Quilez's shadertoy "inverse
//! spherical fibonacci") turns a direction into the four lattice points round it, of which
//! the nearest is the copy. Checked against the nearest of all of them, for every count from
//! 1 to 500, it picks the same copy. As with Repeat, a copy has to fit its cell: nothing
//! checks the neighbors, so a copy wider than the spacing is sliced.
//!
//! **Noise Displace divides by the noise's steepest slope.** The noise is gradient noise with
//! unit gradients hashed from the lattice corners by `sd_hash3`, integer arithmetic, so it is
//! the same on every GPU. With a quintic fade its slope is never more than 2.7923 a lattice
//! unit, reached at a cell's center whatever the gradients are; doubled to make Amount about
//! the tallest bump, that is 5.6. The octaves' slopes add, each scaled by its frequency and
//! weight, so the bound is exact for one octave and loose for many.

use super::{SDF, fill, num, out, solid, turn};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

/// Iteration counts 1 to 12, for an option whose loop length is written into the code.
const COUNTS: &[(&str, &str)] = &[
    ("1", "1"),
    ("2", "2"),
    ("3", "3"),
    ("4", "4"),
    ("5", "5"),
    ("6", "6"),
    ("7", "7"),
    ("8", "8"),
    ("9", "9"),
    ("10", "10"),
    ("11", "11"),
    ("12", "12"),
];

/// A count option's value, `fallback` when it is not a whole number in `1..=max`.
fn count(value: &str, max: u32, fallback: u32) -> u32 {
    value
        .parse()
        .ok()
        .filter(|n| (1..=max).contains(n))
        .unwrap_or(fallback)
}

pub static FOLD: NodeDef = NodeDef {
    slug: "fold",
    category: Category::Solid,
    family: Some(Family::Fractals),
    icon: "❄",
    label: "Fold",
    tooltip: "Folds space like a kaleidoscope, turns it and scales it about Offset, Iterations \
              times, then puts the solid in the folded space: a fractal of its copies. Tint \
              colors it by the path each point takes through the folds. Every iteration \
              adds to what a render costs.",
    inputs: &[
        solid("input", "Solid"),
        num("scale", "Scale", 2.0, 1.0, 4.0, 0.001, "x"),
        num("offsetX", "Offset X", 1.0, -4.0, 4.0, 0.001, "⬓"),
        num("offsetY", "Offset Y", 1.0, -4.0, 4.0, 0.001, "⬓"),
        num("offsetZ", "Offset Z", 1.0, -4.0, 4.0, 0.001, "⬓"),
        turn("turnX", "Turn X", 0.02, -0.5, 0.5, 0.001),
        turn("turnY", "Turn Y", 0.0, -0.5, 0.5, 0.001),
        turn("turnZ", "Turn Z", 0.04, -0.5, 0.5, 0.001),
        num("tint", "Tint", 0.4, 0.0, 1.0, 0.01, ""),
    ],
    options: &[
        OptionDef {
            key: "symmetry",
            label: "Symmetry",
            default: "tetrahedron",
            choices: &[
                ("cube", "Cube"),
                ("tetrahedron", "Tetrahedron"),
                ("octahedron", "Octahedron"),
            ],
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "iterations",
            label: "Iterations",
            default: "6",
            choices: COUNTS,
            ..OptionDef::EMPTY
        },
    ],
    outputs: &[out(|node, ctx, _| {
        if !ctx.connected(node, "input") {
            return format!("    return {};", ctx.input(node, "input", "p"));
        }
        let n = count(ctx.option(node, "iterations"), 12, 6);
        let fold = match ctx.option(node, "symmetry") {
            "cube" => "        q = abs(q);",
            "octahedron" => {
                "        q = abs(q);
        if (q.x < q.y) { q = q.yxz; }
        if (q.x < q.z) { q = q.zyx; }
        if (q.y < q.z) { q = q.xzy; }"
            }
            _ => {
                "        if (q.x + q.y < 0.0) { q = vec3f(-q.y, -q.x, q.z); }
        if (q.x + q.z < 0.0) { q = vec3f(-q.z, q.y, -q.x); }
        if (q.y + q.z < 0.0) { q = vec3f(q.x, -q.z, -q.y); }"
            }
        };
        let inner = ctx.input(node, "input", "q");
        let (start, trap, color) = if ctx.solid_color() {
            (
                "\n    var trap = 0.0;\n    var tw = 1.0;",
                "\n        trap += tw * length(q);\n        tw *= 0.5;",
                "mix(r.yzw, sd_hue(trap / (2.0 - 2.0 * tw)), {tint})",
            )
        } else {
            ("", "", "r.yzw")
        };
        fill(
            node,
            ctx,
            &format!(
                "    let a = vec3f({{turnY}}, {{turnX}}, {{turnZ}}) * 6.2831853;
    let turn = mat3x3f(sd_turn(vec3f(1.0, 0.0, 0.0), a), sd_turn(vec3f(0.0, 1.0, 0.0), a), sd_turn(vec3f(0.0, 0.0, 1.0), a));
    let k = clamp({{scale}}, 1.0, 4.0);
    let o = vec3f({{offsetX}}, {{offsetY}}, {{offsetZ}}) * (k - 1.0);
    var q = p;
    var s = 1.0;{start}
    for (var i = 0; i < {n}; i++) {{
{fold}
        q = turn * q * k - o;
        s *= k;{trap}
    }}
    let r = {inner};
    return vec4f(r.x / s, {color});"
            ),
        )
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static SCATTER_SPHERE: NodeDef = NodeDef {
    slug: "scattersphere",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "🦠",
    label: "Scatter on Sphere",
    tooltip: "Spreads Copies of a solid evenly over a sphere of Radius, each standing on it with \
              its top facing out. Each can turn, shrink and take a hue of its own. A copy wider \
              than the gap between copies is sliced.",
    inputs: &[
        solid("input", "Solid"),
        num("copies", "Copies", 20.0, 1.0, 500.0, 1.0, ""),
        num("radius", "Radius", 3.0, 0.0, 20.0, 0.01, "⬓"),
        num("spin", "Spin", 0.0, 0.0, 1.0, 0.01, TURNS),
        num("size", "Size", 0.0, 0.0, 0.9, 0.01, ""),
        num("tint", "Tint", 0.0, 0.0, 1.0, 0.01, ""),
        num("seed", "Seed", 0.0, 0.0, 999.0, 1.0, ""),
    ],
    outputs: &[out(|node, ctx, _| {
        let inner = ctx.input(node, "input", "q");
        let color = if ctx.solid_color() {
            "mix(r.yzw, sd_hue(sd_hash3(cell + 47.0, {seed}).z), {tint})"
        } else {
            "r.yzw"
        };
        // Copies count from the top of the sphere, so Order runs top to bottom.
        let (before, after) = super::copies::enter(
            ctx,
            node,
            "",
            "SdCopy(i, n, sd_hash3(cell + 59.0, {seed}).x, vec3f(i / max(n - 1.0, 1.0), 0.0, 0.0))",
        );
        fill(
            node,
            ctx,
            &format!(
                "    let n = clamp(round({{copies}}), 1.0, 500.0);
    let i = sd_fibonacci_nearest(p / max(length(p), 1.0e-6), n);
    let up = sd_fibonacci_point(i, n);
    let sg = select(-1.0, 1.0, up.z >= 0.0);
    let a = -1.0 / (sg + up.z);
    let b = up.x * up.y * a;
    let side = vec3f(1.0 + sg * up.x * up.x * a, sg * b, -sg * up.x);
    let back = vec3f(b, sg + up.y * up.y * a, -up.y);
    let d = p - up * {{radius}};
    var q = vec3f(dot(d, side), dot(d, up), -dot(d, back));
    let cell = vec3f(i, 0.0, 0.0);
    let xz = sd_rot2(q.xz, (sd_hash3(cell + 17.0, {{seed}}).x - 0.5) * {{spin}} * 6.2831853);
    q = vec3f(xz.x, q.y, xz.y);
    let g = 1.0 - clamp({{size}}, 0.0, 0.9) * sd_hash3(cell + 31.0, {{seed}}).y;
    q = q / g;
{before}    let r = {inner};
{after}    return vec4f(r.x * g, {color});"
            ),
        )
    })],
    wgsl_utils: &[SDF, FIBONACCI],
    ..NodeDef::EMPTY
};

/// The spherical Fibonacci lattice of `n` points, its pole on y, and the inverse mapping from
/// a direction to the nearest of them.
const FIBONACCI: &str =
    "// Point `i` of `n` on the unit sphere's Fibonacci lattice, the first nearest +y.
fn sd_fibonacci_point(i: f32, n: f32) -> vec3f {
    let phi = 6.2831853 * fract(i * 0.61803399);
    let y = 1.0 - (2.0 * i + 1.0) / n;
    let r = sqrt(max(1.0 - y * y, 0.0));
    return vec3f(cos(phi) * r, y, sin(phi) * r);
}
// The index of the lattice point nearest the unit direction `v`: the four lattice points round
// it, from Keinert et al.'s inverse mapping, and the nearest of those.
fn sd_fibonacci_nearest(v: vec3f, n: f32) -> f32 {
    let golden = 1.61803399;
    let m = 1.0 - 1.0 / n;
    let phi = atan2(v.z, v.x);
    let y = clamp(v.y, -1.0, 1.0);
    let k = max(2.0, floor(log(n * 7.0248147 * max(1.0 - y * y, 1.0e-6)) / 0.96242365));
    let fk = pow(golden, k) / 2.2360680;
    let f = vec2f(round(fk), round(fk * golden));
    let ka = 2.0 * f / n;
    let kb = 6.2831853 * (fract((f + 1.0) * golden) - 0.61803399);
    let det = ka.y * kb.x - ka.x * kb.y;
    let c = floor(vec2f(ka.y * phi + kb.y * (y - m), -ka.x * phi - kb.x * (y - m)) / det);
    var best = 8.0;
    var j = 0.0;
    for (var s = 0; s < 4; s++) {
        let corner = vec2f(f32(s % 2), f32(s / 2));
        let i = clamp(dot(f, corner + c), 0.0, n - 1.0);
        let e = sd_fibonacci_point(i, n) - v;
        let dd = dot(e, e);
        if (dd < best) {
            best = dd;
            j = i;
        }
    }
    return j;
}";

pub static NOISE_DISPLACE: NodeDef = NodeDef {
    slug: "noisedisplace",
    category: Category::Solid,
    family: Some(Family::Surface),
    icon: "🪨",
    label: "Noise Displace",
    tooltip: "Roughens a solid's surface with noise, for rocks and terrain: Amount is about the \
              tallest bump, Frequency how many there are, and Roughness how much each finer \
              octave adds. Cable a gear's Cycles into Phase to set it churning. The more Amount, \
              the slower the march.",
    inputs: &[
        solid("input", "Solid"),
        num("amount", "Amount", 0.15, 0.0, 1.0, 0.001, "⬓"),
        num("frequency", "Frequency", 1.5, 0.1, 30.0, 0.01, "/⬓"),
        num("roughness", "Roughness", 0.5, 0.0, 1.0, 0.01, ""),
        turn("phase", "Phase", 0.0, 0.0, 1.0, 0.001),
    ],
    options: &[OptionDef {
        key: "octaves",
        label: "Octaves",
        default: "4",
        choices: &[
            ("1", "1"),
            ("2", "2"),
            ("3", "3"),
            ("4", "4"),
            ("5", "5"),
            ("6", "6"),
        ],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        if !ctx.connected(node, "input") {
            return format!("    return {};", ctx.input(node, "input", "p"));
        }
        let n = count(ctx.option(node, "octaves"), 6, 4);
        let inner = ctx.input(node, "input", "p");
        fill(
            node,
            ctx,
            &format!(
                "    let r = {inner};
    let t = {{phase}} * 6.2831853;
    let w = vec3f(cos(t), sin(t), 0.0);
    let f = {{frequency}};
    let gain = clamp({{roughness}}, 0.0, 1.0);
    var x = p * f;
    var amp = 1.0;
    var fr = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    var slope = 0.0;
    for (var o = 0; o < {n}; o++) {{
        sum += amp * sd_gradient_noise3(x + w);
        norm += amp;
        slope += amp * fr;
        x = mat3x3f(0.0, 0.8, 0.6, -0.8, 0.36, -0.48, -0.6, -0.48, 0.64) * x * 2.0;
        amp *= gain;
        fr *= 2.0;
    }}
    let a = {{amount}};
    return vec4f((r.x + a * 2.0 * sum / norm) / (1.0 + abs(a * f) * 5.6 * slope / norm), r.yzw);"
            ),
        )
    })],
    wgsl_utils: &[SDF, NOISE],
    ..NodeDef::EMPTY
};

/// Gradient noise from `sd_hash3`, one lattice unit a cell.
const NOISE: &str =
    "// One corner's contribution: its hashed unit gradient against the point's offset from it.
fn sd_noise_corner(i: vec3f, f: vec3f, c: vec3f) -> f32 {
    let g = sd_hash3(i + c, 0.0) * 2.0 - 1.0;
    return dot(g * inverseSqrt(max(dot(g, g), 1.0e-12)), f - c);
}
// Gradient noise, at most 0.87 either way and never steeper than 2.7923, the same on every GPU.
fn sd_gradient_noise3(x: vec3f) -> f32 {
    let i = floor(x);
    let f = x - i;
    let h = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let e = vec2f(0.0, 1.0);
    let a = mix(sd_noise_corner(i, f, e.xxx), sd_noise_corner(i, f, e.yxx), h.x);
    let b = mix(sd_noise_corner(i, f, e.xyx), sd_noise_corner(i, f, e.yyx), h.x);
    let c = mix(sd_noise_corner(i, f, e.xxy), sd_noise_corner(i, f, e.yxy), h.x);
    let d = mix(sd_noise_corner(i, f, e.xyy), sd_noise_corner(i, f, e.yyy), h.x);
    return mix(mix(a, b, h.y), mix(c, d, h.y), h.z);
}";
