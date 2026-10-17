// SPDX-License-Identifier: AGPL-3.0-or-later

//! Repetition: a grid of copies and a ring of them. Each folds the point into one cell, so the
//! input is evaluated once however many copies show — infinitely many cost what one does.
//!
//! **A copy has to fit its cell.** The fold only looks at the nearest cell, so a copy wider
//! than the spacing, or pushed out of its cell by Scatter or Grow, is sliced flat at the cell's
//! edge. The slicing is the signal to widen the spacing; nothing checks the neighbors.
//!
//! **Copies differ by the cell they are in.** Only these nodes know the cell, so the variation
//! is theirs: Scatter, Spin, Size and Tint from a hash of the cell and the Seed, and on a ring
//! a step per copy. The hash is integer arithmetic, so a scatter is the same on every GPU and
//! comes back the same when a project is opened.

use super::copies::{RESTORE, SAVE};
use super::{SDF, fill, num, out, solid, turn};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

pub static GRID: NodeDef = NodeDef {
    slug: "repeatgrid",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "▦",
    label: "Repeat",
    tooltip: "Copies a solid on a grid, Spacing apart on each axis; a spacing of 0 leaves that \
              axis alone. Forever goes on without end at the cost of one copy; Counted stops \
              Count copies either side. A copy that does not fit its cell is sliced.",
    inputs: &[
        solid("input", "Solid"),
        num("spacingX", "Spacing X", 3.0, 0.0, 20.0, 0.01, "⬓"),
        num("spacingY", "Spacing Y", 0.0, 0.0, 20.0, 0.01, "⬓"),
        num("spacingZ", "Spacing Z", 3.0, 0.0, 20.0, 0.01, "⬓"),
        num("countX", "Count X", 2.0, 0.0, 50.0, 1.0, "±"),
        num("countY", "Count Y", 2.0, 0.0, 50.0, 1.0, "±"),
        num("countZ", "Count Z", 2.0, 0.0, 50.0, 1.0, "±"),
        num("scatter", "Scatter", 0.0, 0.0, 1.0, 0.01, ""),
        num("spin", "Spin", 0.0, 0.0, 1.0, 0.01, TURNS),
        num("size", "Size", 0.0, 0.0, 0.9, 0.01, ""),
        num("tint", "Tint", 0.0, 0.0, 1.0, 0.01, ""),
        num("seed", "Seed", 0.0, 0.0, 999.0, 1.0, ""),
    ],
    options: &[OptionDef {
        key: "extent",
        label: "Extent",
        default: "forever",
        choices: &[("forever", "Forever"), ("counted", "Counted")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        let counted = ctx.option(node, "extent") == "counted";
        let inner = ctx.input(node, "input", "q");
        let color = if ctx.solid_color() {
            "mix(r.yzw, sd_hue(sd_hash3(cell + 47.0, {seed}).z), {tint})"
        } else {
            "r.yzw"
        };
        let clamp = "cell = clamp(cell, -vec3f({countX}, {countY}, {countZ}), vec3f({countX}, {countY}, {countZ}));";
        let body = if ctx.reads_copy(node, "input") {
            // The nearest copy and the one on the near side of each axis, each saying which
            // copy it is: its place in the block of Count either side, which a Forever grid
            // repeats, an axis left alone being one copy wide.
            let clamp = if counted {
                format!("        {clamp}\n")
            } else {
                String::new()
            };
            format!(
                "    let s = vec3f({{spacingX}}, {{spacingY}}, {{spacingZ}});
    let on = s > vec3f(1.0e-4);
    let step = select(vec3f(0.0), s, on);
    let near = select(vec3f(0.0), round(p / select(vec3f(1.0), s, on)), on);
    let side = select(vec3f(0.0), sign(p - near * step), on);
    let half = max(round(vec3f({{countX}}, {{countY}}, {{countZ}})), vec3f(0.0));
    let span = select(vec3f(1.0), 2.0 * half + 1.0, on);
{SAVE}    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = 0; j < 4; j++) {{
        var cell = near;
        if (j > 0) {{
            let axis = vec3f(f32(j == 1), f32(j == 2), f32(j == 3));
            if (dot(axis, side) == 0.0) {{ continue; }}
            cell += axis * side;
        }}
{clamp}        var q = p - cell * step;
        q -= (sd_hash3(cell, {{seed}}) - 0.5) * {{scatter}} * step;
        let xz = sd_rot2(q.xz, (sd_hash3(cell + 17.0, {{seed}}).x - 0.5) * {{spin}} * 6.2831853);
        q = vec3f(xz.x, q.y, xz.y);
        let g = 1.0 - {{size}} * sd_hash3(cell + 31.0, {{seed}}).y;
        q = q / g;
        let at = select(vec3f(0.0), floor_mod3(cell + half, span), on);
        sd_copy = SdCopy(at.x + span.x * (at.y + span.y * at.z), span.x * span.y * span.z, sd_hash3(cell + 59.0, {{seed}}).x, at / max(span - 1.0, vec3f(1.0)));
        let r = {inner};
        let d = r.x * g;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
    }}
{RESTORE}    return best;"
            )
        } else {
            let clamp = if counted {
                format!("    {clamp}\n")
            } else {
                String::new()
            };
            format!(
                "    let s = vec3f({{spacingX}}, {{spacingY}}, {{spacingZ}});
    let on = s > vec3f(1.0e-4);
    let step = select(vec3f(0.0), s, on);
    var cell = select(vec3f(0.0), round(p / select(vec3f(1.0), s, on)), on);
{clamp}    var q = p - cell * step;
    q -= (sd_hash3(cell, {{seed}}) - 0.5) * {{scatter}} * step;
    let xz = sd_rot2(q.xz, (sd_hash3(cell + 17.0, {{seed}}).x - 0.5) * {{spin}} * 6.2831853);
    q = vec3f(xz.x, q.y, xz.y);
    let g = 1.0 - {{size}} * sd_hash3(cell + 31.0, {{seed}}).y;
    q = q / g;
    let r = {inner};
    return vec4f(r.x * g, {color});"
            )
        };
        fill(node, ctx, &body)
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};

pub static AROUND: NodeDef = NodeDef {
    slug: "repeataround",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "🎡",
    label: "Repeat Around",
    tooltip: "Copies a solid round an axis, Radius out from it. Spread is how much of a turn \
              the copies share; under a whole turn they make an arc. Each copy can rise, turn \
              and grow a step more than the one before it.",
    inputs: &[
        solid("input", "Solid"),
        num("copies", "Copies", 8.0, 1.0, 64.0, 1.0, ""),
        num("radius", "Radius", 2.0, 0.0, 20.0, 0.01, "⬓"),
        num("spread", "Spread", 1.0, 0.0, 1.0, 0.001, TURNS),
        num("rise", "Rise", 0.0, -2.0, 2.0, 0.01, "⬓"),
        turn("turn", "Turn", 0.0, -1.0, 1.0, 0.001),
        num("grow", "Grow", 0.0, -0.5, 0.5, 0.001, ""),
        num("tint", "Tint", 0.0, 0.0, 1.0, 0.01, ""),
        num("seed", "Seed", 0.0, 0.0, 999.0, 1.0, ""),
    ],
    options: &[OptionDef {
        key: "axis",
        label: "Axis",
        default: "y",
        choices: &[("y", "Y"), ("x", "X"), ("z", "Z")],
        ..OptionDef::EMPTY
    }],
    outputs: &[out(|node, ctx, _| {
        // The fold runs about y; another axis is turned onto y and back.
        let (into, back) = match ctx.option(node, "axis") {
            "x" => ("vec3f(-p.y, p.x, p.z)", "vec3f(m.y, -m.x, m.z)"),
            "z" => ("vec3f(p.x, p.z, -p.y)", "vec3f(m.x, -m.z, m.y)"),
            _ => ("p", "m"),
        };
        let inner = ctx.input(node, "input", "q");
        let color = if ctx.solid_color() {
            "mix(r.yzw, sd_hue(sd_hash3(vec3f(i, 0.0, 0.0), {seed}).x), {tint})"
        } else {
            "r.yzw"
        };
        let body = if ctx.reads_copy(node, "input") {
            // The nearest copy and the one on the near side of it round the ring.
            format!(
                "    let tau = 6.2831853;
    let n = max(round({{copies}}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w = max(spread * tau / n, 1.0e-5);
    let l = {into};
    let a = floor_mod(atan2(l.z, l.x), tau);
    let k = round(a / w);
    let toward = select(-1.0, 1.0, a > k * w);
{SAVE}    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = 0; j < 2; j++) {{
        var i = k + f32(j) * toward;
        if (spread > 0.9999) {{
            i = floor_mod(i, n);
        }} else {{
            i = clamp(i, 0.0, n - 1.0);
            if (j == 0 && abs(a - tau) < abs(a - i * w)) {{ i = 0.0; }}
        }}
        let xz = sd_rot2(l.xz, -i * w);
        var m = vec3f(xz.x - {{radius}}, l.y - i * {{rise}}, xz.y);
        let tw = sd_rot2(m.xz, i * {{turn}} * tau);
        m = vec3f(tw.x, m.y, tw.y);
        let g = max(1.0 + i * {{grow}}, 0.05);
        m = m / g;
        let q = {back};
        sd_copy = SdCopy(i, n, sd_hash3(vec3f(i, 0.0, 0.0) + 59.0, {{seed}}).x, vec3f(i / max(n - 1.0, 1.0), 0.0, 0.0));
        let r = {inner};
        let d = r.x * g;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
    }}
{RESTORE}    return best;"
            )
        } else {
            format!(
                "    let tau = 6.2831853;
    let n = max(round({{copies}}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w = max(spread * tau / n, 1.0e-5);
    let l = {into};
    let a = floor_mod(atan2(l.z, l.x), tau);
    var i = round(a / w);
    if (spread > 0.9999) {{
        i = floor_mod(i, n);
    }} else {{
        i = clamp(i, 0.0, n - 1.0);
        if (abs(a - tau) < abs(a - i * w)) {{ i = 0.0; }}
    }}
    let xz = sd_rot2(l.xz, -i * w);
    var m = vec3f(xz.x - {{radius}}, l.y - i * {{rise}}, xz.y);
    let tw = sd_rot2(m.xz, i * {{turn}} * tau);
    m = vec3f(tw.x, m.y, tw.y);
    let g = max(1.0 + i * {{grow}}, 0.05);
    m = m / g;
    let q = {back};
    let r = {inner};
    return vec4f(r.x * g, {color});"
            )
        };
        fill(node, ctx, &body)
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
