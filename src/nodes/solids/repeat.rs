// SPDX-License-Identifier: AGPL-3.0-or-later

//! Repetition: a grid of copies and a ring of them. Each folds the point into one cell, so the
//! input is evaluated once however many copies show — infinitely many cost what one does.
//!
//! **A copy has to fit its cell.** The fold looks at the nearest cell, so a copy wider than
//! the spacing is sliced flat at the cell's edge. The slicing is the signal to widen the
//! spacing.
//!
//! **Copies vary by the repeat's own rows** ([`crate::nodes::percopy`]): an offset, a turn, a
//! scale on each axis and a color, each a knob or a list. While every copy is the same shape
//! the nearest cell is the whole answer. Once a list moves, turns or sizes the copies, a copy
//! taller than the one beside it is nearer to a point in that cell than that cell's own copy,
//! and a ray stepping by the shorter one's distance runs into it — from either side of the
//! short copy's cell, since a point high over a short copy is nearer a tall neighbor across
//! the cell than the copy under it. So a repeat whose copies differ evaluates the nearest copy
//! and the ones either side of it on each axis it repeats along, and keeps the nearer: three
//! times the cost for a row or a ring, five for a plane, seven for a grid in three dimensions.
//! A Color list alone never pays for it.
//!
//! **Count is how many copies**, centered on the axis, and a list cabled into it is a count of
//! its length: a Repeat over a list of sixteen bands draws sixteen copies.

use super::{MARCH_STATE, SDF, fill, num, out, solid};
use crate::nodes::percopy::{self, Item};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

pub static GRID: NodeDef = NodeDef {
    slug: "repeatgrid",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "▦",
    label: "Repeat",
    tooltip: "Copies a solid on a grid, Spacing apart on each axis; a spacing of 0 leaves that \
              axis alone. Forever goes on without end at the cost of one copy; Counted lays Count \
              copies centered on each axis, and a list into Count makes as many copies as it has \
              items. Per copy moves, turns, sizes and colors each copy, from a knob or a list.",
    inputs: &[
        solid("input", "Solid"),
        num("spacingX", "Spacing X", 3.0, 0.0, 20.0, 0.01, "⬓"),
        num("spacingY", "Spacing Y", 0.0, 0.0, 20.0, 0.01, "⬓"),
        num("spacingZ", "Spacing Z", 3.0, 0.0, 20.0, 0.01, "⬓"),
        percopy::count("countX", "Count X", 5.0, 100.0),
        percopy::count("countY", "Count Y", 5.0, 100.0),
        percopy::count("countZ", "Count Z", 5.0, 100.0),
        percopy::OFFSET_X,
        percopy::OFFSET_Y,
        percopy::OFFSET_Z,
        percopy::TURN,
        percopy::SCALE_X,
        percopy::SCALE_Y,
        percopy::SCALE_Z,
        percopy::COLOR,
    ],
    options: &[
        OptionDef {
            key: "extent",
            label: "Extent",
            default: "forever",
            choices: &[("forever", "Forever"), ("counted", "Counted")],
            ..OptionDef::EMPTY
        },
        percopy::HEADING,
    ],
    row_headings: percopy::ROW_HEADINGS,
    outputs: &[out(|node, ctx, _| {
        let counted = ctx.option(node, "extent") == "counted";
        let inner = ctx.input(node, "input", "m");
        let differ = percopy::moves_differ(ctx, node);
        let (cx, cy, cz) = (
            percopy::count_of(ctx, node, "countX"),
            percopy::count_of(ctx, node, "countY"),
            percopy::count_of(ctx, node, "countZ"),
        );
        let item = if counted {
            Item::Counted {
                index: "cell.x + n.x * (cell.y + n.y * cell.z)",
                count: "n.x * n.y * n.z",
            }
        } else {
            Item::Endless {
                sum: "cell.x + cell.y + cell.z",
            }
        };
        let copy = percopy::read(ctx, node, item);
        let place = percopy::place(&copy);
        let color = if ctx.solid_color() {
            format!("r.yzw * {}", copy.color)
        } else {
            "r.yzw".to_string()
        };
        // Counted: n copies an axis, the cell index 0 to n - 1, centered. Forever: the cell is
        // the nearest lattice point and nothing bounds it.
        let (n, mid, near, skip) = if counted {
            (
                format!("select(vec3f(1.0), max(round(vec3f({cx}, {cy}, {cz})), vec3f(1.0)), on)"),
                "(n - 1.0) * 0.5",
                "clamp(round(p / select(vec3f(1.0), s, on) + mid), vec3f(0.0), n - 1.0)",
                "            if (any(cell < vec3f(0.0)) || any(cell > n - 1.0)) { continue; }\n",
            )
        } else {
            (
                "vec3f(1.0)".to_string(),
                "vec3f(0.0)",
                "round(p / select(vec3f(1.0), s, on))",
                "",
            )
        };
        let loops = if differ { 7 } else { 1 };
        // Copies past the measured block are no nearer than its outer walls: a march's step
        // never crosses one, so a copy two cells off that stands taller than the three measured
        // is met at the wall and measured from the next cell. Only while a ray is marched
        // (`MARCH_STATE`): a soft shadow reads the distance as how near the copies come, and the
        // wall is no copy.
        let beyond = if differ {
            let (lo, hi) = if counted {
                ("near >= vec3f(2.0)", "near <= n - 3.0")
            } else {
                ("vec3f(1.0) > vec3f(0.0)", "vec3f(1.0) > vec3f(0.0)")
            };
            format!(
                "    let below = select(vec3f(1.0e9), p - (near - mid - 1.5) * step, on & ({lo}));
    let above = select(vec3f(1.0e9), (near - mid + 1.5) * step - p, on & ({hi}));
    let wall = min(min(below.x, below.y), min(below.z, min(above.x, min(above.y, above.z))));
    if (sd_march) {{ best.x = min(best.x, wall); }}
"
            )
        } else {
            String::new()
        };
        let body = format!(
            "    let s = vec3f({{spacingX}}, {{spacingY}}, {{spacingZ}});
    let on = s > vec3f(1.0e-4);
    let step = select(vec3f(0.0), s, on);
    let n = {n};
    let mid = {mid};
    let near = select(vec3f(0.0), {near}, on);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = 0; j < {loops}; j++) {{
        var cell = near;
        if (j > 0) {{
            let a = (j - 1) / 2;
            let axis = vec3f(f32(a == 0), f32(a == 1), f32(a == 2));
            if (dot(axis, select(vec3f(0.0), vec3f(1.0), on)) == 0.0) {{ continue; }}
            cell += axis * select(-1.0, 1.0, (j - 1) % 2 == 0);
{skip}        }}
        let q = p - (cell - mid) * step;
{place}        let r = {inner};
        let d = r.x * k;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
    }}
{beyond}    return best;"
        );
        fill(node, ctx, &body)
    })],
    wgsl_utils: &[SDF, MARCH_STATE],
    ..NodeDef::EMPTY
};

pub static AROUND: NodeDef = NodeDef {
    slug: "repeataround",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "🎡",
    label: "Repeat Around",
    tooltip: "Copies a solid round an axis, Radius out from it. Spread is how much of a turn \
              the copies share; under a whole turn they make an arc. A list into Copies makes as \
              many copies as it has items. Per copy moves, turns, sizes and colors each copy, \
              from a knob or a list: a Range into Offset Y climbs a spiral stair.",
    inputs: &[
        solid("input", "Solid"),
        percopy::count("copies", "Copies", 8.0, 64.0),
        num("radius", "Radius", 2.0, 0.0, 20.0, 0.01, "⬓"),
        num("spread", "Spread", 1.0, 0.0, 1.0, 0.001, TURNS),
        percopy::OFFSET_X,
        percopy::OFFSET_Y,
        percopy::OFFSET_Z,
        percopy::TURN,
        percopy::SCALE_X,
        percopy::SCALE_Y,
        percopy::SCALE_Z,
        percopy::COLOR,
    ],
    options: &[
        OptionDef {
            key: "axis",
            label: "Axis",
            default: "y",
            choices: &[("y", "Y"), ("x", "X"), ("z", "Z")],
            ..OptionDef::EMPTY
        },
        percopy::HEADING,
    ],
    row_headings: percopy::ROW_HEADINGS,
    outputs: &[out(|node, ctx, _| {
        // The fold runs about y; another axis is turned onto y and back.
        let (into, back) = match ctx.option(node, "axis") {
            "x" => ("vec3f(-p.y, p.x, p.z)", "vec3f(m.y, -m.x, m.z)"),
            "z" => ("vec3f(p.x, p.z, -p.y)", "vec3f(m.x, -m.z, m.y)"),
            _ => ("p", "m"),
        };
        let inner = ctx.input(node, "input", "w");
        let differ = percopy::moves_differ(ctx, node);
        let copies = percopy::count_of(ctx, node, "copies");
        let copy = percopy::read(
            ctx,
            node,
            Item::Counted {
                index: "i",
                count: "n",
            },
        );
        let place = percopy::place(&copy);
        let color = if ctx.solid_color() {
            format!("r.yzw * {}", copy.color)
        } else {
            "r.yzw".to_string()
        };
        let loops = if differ { 3 } else { 1 };
        // The ring's measured wedges end at two half-planes through the axis, and a copy past
        // them is no nearer than they are; near the axis, where every wedge meets, the walls
        // would hold a step to nothing, and the copies stand a Radius off.
        let beyond = if differ {
            "    if (sd_march && n > 3.0 && length(l.xz) > 0.5 * {radius}) {
        let lo_ok = spread > 0.9999 || k0 >= 2.0;
        let hi_ok = spread > 0.9999 || k0 <= n - 3.0;
        var wall = 1.0e9;
        for (var e = 0; e < 2; e++) {
            if ((e == 0 && !lo_ok) || (e == 1 && !hi_ok)) { continue; }
            let th = (k0 + select(-1.5, 1.5, e == 1)) * w0;
            let dir = vec2f(cos(th), sin(th));
            let along = dot(l.xz, dir);
            wall = min(wall, select(length(l.xz), abs(dot(l.xz, vec2f(-dir.y, dir.x))), along > 0.0));
        }
        best.x = min(best.x, wall);
    }
"
            .to_string()
        } else {
            String::new()
        };
        // The copy is placed in the ring's frame, x out from the axis; its rows then move it in
        // that frame, and the result is turned back onto the chosen axis for the input.
        let body = format!(
            "    let tau = 6.2831853;
    let n = max(round({copies}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w0 = max(spread * tau / n, 1.0e-5);
    let l = {into};
    let a = floor_mod(atan2(l.z, l.x), tau);
    let k0 = round(a / w0);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = 0; j < {loops}; j++) {{
        var i = k0 + select(0.0, select(-1.0, 1.0, j == 1), j > 0);
        if (spread > 0.9999) {{
            i = floor_mod(i, n);
        }} else {{
            i = clamp(i, 0.0, n - 1.0);
            if (j == 0 && abs(a - tau) < abs(a - i * w0)) {{ i = 0.0; }}
        }}
        let xz = sd_rot2(l.xz, -i * w0);
        let q = vec3f(xz.x - {{radius}}, l.y, xz.y);
{place}        let w = {back};
        let r = {inner};
        let d = r.x * k;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
    }}
{beyond}    return best;"
        );
        fill(node, ctx, &body)
    })],
    wgsl_utils: &[SDF, MARCH_STATE],
    ..NodeDef::EMPTY
};
