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
//! 1 to 500, it picks the same copy. **Where its input can say how big it is**, it measures
//! every copy that can reach the point instead, as Repeat does: a copy's box, seen from the
//! sphere's center, lies within an angle of its own direction, so the copies within that angle
//! and one lattice spacing of the point's direction are found among the band of heights that
//! covers and measured, each skipped where its box is further off than the nearest so far, and
//! a march's step is held to the spacing's angle, which every other copy is further round than.
//! Where it cannot say, the nearest copy alone is measured, and a copy wider than the spacing
//! is sliced.
//!
//! **Noise Displace divides by the noise's steepest slope.** The noise is gradient noise with
//! unit gradients hashed from the lattice corners by `sd_hash3`, integer arithmetic, so it is
//! the same on every GPU. With a quintic fade its slope is never more than 2.7923 a lattice
//! unit, reached at a cell's center whatever the gradients are; doubled to make Amount about
//! the tallest bump, that is 5.6. The octaves' slopes add, each scaled by its frequency and
//! weight, so the bound is exact for one octave and loose for many.

use super::{SDF, fill, num, out, solid, turn};
use crate::nodes::{Category, Family, NodeDef, OptionDef};

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
              its top facing out. A list into Copies makes as many copies as it has items. Per \
              copy moves, turns, sizes and colors each copy, from a knob or a list.",
    inputs: &[
        solid("input", "Solid"),
        crate::nodes::percopy::count("copies", "Copies", 20.0, 500.0),
        num("radius", "Radius", 3.0, 0.0, 20.0, 0.01, "⬓"),
        crate::nodes::percopy::OFFSET_X,
        crate::nodes::percopy::OFFSET_Y,
        crate::nodes::percopy::OFFSET_Z,
        crate::nodes::percopy::TURN,
        crate::nodes::percopy::SCALE_X,
        crate::nodes::percopy::SCALE_Y,
        crate::nodes::percopy::SCALE_Z,
        crate::nodes::percopy::COLOR,
    ],
    options: &[crate::nodes::percopy::HEADING],
    row_headings: crate::nodes::percopy::ROW_HEADINGS,
    outputs: &[out(|node, ctx, _| {
        use crate::nodes::percopy::{self, Item};
        let copies = percopy::count_of(ctx, node, "copies");
        // Copies count from the top of the sphere down, so a list runs top to bottom.
        let copy = percopy::read(
            ctx,
            node,
            Item::Counted {
                index: "i",
                count: "n",
            },
        );
        let measure_copy = percopy::evaluate(ctx, node, &copy, "input", str::to_string);
        let color = if ctx.solid_color() {
            format!("r.yzw * {}", copy.color)
        } else {
            "r.yzw".to_string()
        };
        // Copy `i`'s frame: x and z across the sphere, y out of it, and the point in it, `q`.
        let frame = "        let up = sd_fibonacci_point(i, n);
        let sg = select(-1.0, 1.0, up.z >= 0.0);
        let a = -1.0 / (sg + up.z);
        let b = up.x * up.y * a;
        let side = vec3f(1.0 + sg * up.x * up.x * a, sg * b, -sg * up.x);
        let back = vec3f(b, sg + up.y * up.y * a, -up.y);
        let dp = p - up * {radius};
        let q = vec3f(dot(dp, side), dot(dp, up), -dot(dp, back));
";
        let Some(bounds) = ctx.solid_bounds(node, "input") else {
            return fill(
                node,
                ctx,
                &format!(
                    "    let n = clamp(round({copies}), 1.0, 500.0);
    let i = sd_fibonacci_nearest(p / max(length(p), 1.0e-6), n);
{frame}{measure_copy}    return vec4f(r.x * k, {color});"
                ),
            );
        };
        // The copies that can reach the point: a copy's box, seen from the sphere's center,
        // lies within `spread` of its own direction while its nearest side stays out past the
        // center, so every copy within `spread` and a lattice spacing of the point's direction
        // is measured, found among the band of heights that covers, and each skipped where its
        // box is further off than the nearest so far. A copy further round is a spacing's angle
        // off the point's direction at least, which a march holds its step to.
        let (offset, scale, turned) = percopy::extremes(ctx, node);
        let skip = super::repeat::skip_copy(&copy);
        let distance = super::repeat::DISTANCE_BOUNDED;
        fill(
            node,
            ctx,
            &format!(
                "    let n = clamp(round({copies}), 1.0, 500.0);
    let lp = length(p);
    let v = p / max(lp, 1.0e-6);
    let bx = {bounds};
    let inreach = sd_bounds_reach(bx);
    let reach = sd_copy_reach(bx, {offset}, {scale}, {turned});
    let low = {{radius}} + min(bx.lo.y * ({scale}).y, 0.0) - ({offset}).y;
    let margin = min(sqrt(12.566371 / n), 1.5707963);
    var within = 3.1415927;
    if (low > 0.0) {{ within = min(atan2(length(reach.xz), low) + margin, 3.1415927); }}
    let cmin = cos(within);
    let pv = acos(clamp(v.y, -1.0, 1.0));
    let i0 = max(floor((1.0 - cos(max(pv - within, 0.0))) * n * 0.5 - 0.5), 0.0);
    let i1 = min(ceil((1.0 - cos(min(pv + within, 3.1415927))) * n * 0.5 - 0.5), n - 1.0);
    let sv = sqrt(max(1.0 - v.y * v.y, 0.0));
    let av = atan2(v.z, v.x);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var i = i0; i <= i1; i += 1.0) {{
        let yi = 1.0 - (2.0 * i + 1.0) / n;
        let ai = 6.2831853 * fract(i * 0.61803399);
        if (yi * v.y + sqrt(max(1.0 - yi * yi, 0.0)) * sv * cos(ai - av) < cmin) {{ continue; }}
{frame}{skip}{measure_copy}{distance}        if (d < best.x) {{ best = vec4f(d, {color}); }}
    }}
    if (sd_march && within < 3.1415927) {{ best.x = min(best.x, lp * sin(margin)); }}
    return best;"
            ),
        )
    })],
    bounds: Some(|node, ctx| {
        let b = ctx.solid_bounds(node, "input")?;
        let (offset, scale, turned) = crate::nodes::percopy::extremes(ctx, node);
        ctx.uniform_fill(
            node,
            &super::half(&format!(
                "vec3f({{radius}} + length(sd_copy_reach({b}, {offset}, {scale}, {turned})))"
            )),
        )
    }),
    wgsl_utils: &[SDF, super::MARCH_STATE, FIBONACCI],
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
