// SPDX-License-Identifier: AGPL-3.0-or-later

//! Repetition: a grid of copies and a ring of them. Each folds the point into one cell, so the
//! input is evaluated once however many copies show — infinitely many cost what one does.
//!
//! **Copies vary by the repeat's own rows** ([`crate::nodes::percopy`]): an offset, a turn, a
//! scale on each axis and a color, each a knob or a list.
//!
//! **Every repeat measures its neighbors.** The fold puts a point in one cell, but the copy in
//! the next cell can be nearer: a taller copy beside a short one — from either side of the
//! short one's cell, since a point high over it is nearer a tall neighbor across the cell than
//! the copy under it — or a copy bigger than its cell, as a whole row of bars is in a ring's
//! wedge. A repeat cannot see how big its input is, so it never assumes it fits: it evaluates
//! the nearest copy and the ones either side of it on each axis it repeats along and keeps the
//! nearer, three times the cost for a row or a ring, five for a plane, seven for a grid in three
//! dimensions, and a march's step is held to the outer walls of the cells measured. A copy
//! reaching past its neighbor's cell is still cut there.
//!
//! **Count is how many copies**, centered on the axis, and a list cabled into it is a count of
//! its length: a Repeat over a list of sixteen bands draws sixteen copies.

use super::{SDF, fill, num, out, solid};
use crate::compile::CompileContext;
use crate::compile::instances::{self, Slot};
use crate::graph::NodeId;
use crate::nodes::percopy::{self, Item};
use crate::nodes::{Category, Family, NodeDef, OptionDef, TURNS};

/// The lines that skip a copy, before anything is asked of its input, where its own box — the
/// input's reach `inreach`, worked out once a point, scaled and turned by the copy's own rows and
/// moved by its offset — is further from its point `q` than the nearest copy so far, leaving the
/// distance to that box as `gap`.
pub(super) fn skip_copy(copy: &percopy::Copy) -> String {
    format!(
        "        var ch = inreach * abs({});
        if (({}) != 0.0) {{ ch = vec3f(length(ch.xz), ch.y, length(ch.xz)); }}
        let gap = sd_box_gap(q, {}, ch);
        if (gap >= best.x) {{ continue; }}
",
        copy.scale, copy.turn, copy.offset
    )
}

/// A copy's distance where its box is known: the input's as [`percopy::evaluate`] measures it —
/// where that is the bound by the smallest scale, it shrinks with the squash, and a copy pressed
/// flat would draw fat — and outside the copy's box no less than the distance to the box, which
/// is a bound too and near the true one there. Inside the box the input's alone, so a point
/// inside a copy keeps its distance below zero.
pub(super) const DISTANCE_BOUNDED: &str =
    "        let d = select(r.x * k, max(r.x * k, gap), gap > 0.0);\n";

/// The most copies a counted Repeat lists, and the most cells its grid has: past them a draw
/// measures by folding.
const GRID_ITEMS: u32 = 32768;
const GRID_CELLS: u32 = 65536;

/// The most a ring lists: Copies' own most, and cells for twice as many.
const AROUND_ITEMS: u32 = 64;
const AROUND_CELLS: u32 = 512;

/// The statements that measure a point through a listed repeat's grid where this draw's lists
/// hold every copy, returning; past them the folded measure runs.
pub(super) fn listed(ctx: &mut CompileContext, node: NodeId, slot: &Slot) -> String {
    let eval = percopy::evaluate_listed(ctx, node, "input");
    let color = if ctx.solid_color() {
        "r.yzw * c.color.xyz"
    } else {
        "r.yzw"
    };
    format!(
        "    if (sd_grid[{}u].cs.w > 0.5) {{\n{}    }}\n",
        slot.grid,
        instances::lookup(slot, &eval, color)
    )
}

/// A counted Repeat's copies listed ([`instances`]): copy `t` is cell `t` counted along x, then
/// y, then z, as its rows' items are. `None` where the input cannot say how big it is, or a
/// field reaches the spacing.
fn grid_slot(
    ctx: &mut CompileContext,
    node: NodeId,
    copy: &percopy::Copy,
    counts: (&str, &str, &str),
) -> Option<Slot> {
    let own = (GRID.bounds?)(node, ctx)?;
    let input = ctx.solid_bounds(node, "input")?;
    let (offset, scale, turned) = percopy::extremes(ctx, node);
    let (cx, cy, cz) = counts;
    ctx.instance_slot(node, GRID_ITEMS, GRID_CELLS, |ctx, slot| {
        let head = instances::place_head(slot, "n.x * n.y * n.z", &own, "2.0 * reach");
        let body = format!(
            "    let s = vec3f({{spacingX}}, {{spacingY}}, {{spacingZ}});
    let on = s > vec3f(1.0e-4);
    let step = select(vec3f(0.0), s, on);
    let n = select(vec3f(1.0), max(round(vec3f({cx}, {cy}, {cz})), vec3f(1.0)), on);
    let mid = (n - 1.0) * 0.5;
    let bx = {input};
    let reach = sd_copy_reach(bx, {offset}, {scale}, {turned});
{head}    let fi = f32(t);
    let cell = vec3f(fi % n.x, floor(fi / n.x) % n.y, floor(fi / (n.x * n.y)));
    let at = (cell - mid) * step + {off};
    let turn = -({turn}) * 6.2831853;
    sd_inst[{ib}u + t] = sd_instance(
        sd_copy_frame(vec3f(0.0), at, turn),
        sd_copy_frame(vec3f(1.0, 0.0, 0.0), at, turn),
        sd_copy_frame(vec3f(0.0, 1.0, 0.0), at, turn),
        sd_copy_frame(vec3f(0.0, 0.0, 1.0), at, turn),
        {sc},
        {color},
        bx,
    );",
            off = copy.offset,
            turn = copy.turn,
            sc = copy.scale,
            color = copy.color,
            ib = slot.item_base,
        );
        Some(fill(node, ctx, &body))
    })
}

/// Repeat Around's copies listed ([`instances`]): copy `t` is the copy `t` turns round, its
/// frame the ring's turned onto the chosen axis, `into` and `back` the turn there and back as
/// [`AROUND`] writes them. `None` where the input cannot say how big it is, or a field reaches
/// the radius or the spread.
fn around_slot(
    ctx: &mut CompileContext,
    node: NodeId,
    copy: &percopy::Copy,
    copies: &str,
    (into, back): (&str, &str),
) -> Option<Slot> {
    if ctx.is_field(node, "spread") {
        return None;
    }
    let own = (AROUND.bounds?)(node, ctx)?;
    let input = ctx.solid_bounds(node, "input")?;
    let (offset, scale, turned) = percopy::extremes(ctx, node);
    let back_m = back.replace("{v}", "m");
    let back_sc = back.replace("{v}", "sc");
    ctx.instance_slot(node, AROUND_ITEMS, AROUND_CELLS, |ctx, slot| {
        let head = instances::place_head(slot, "n", &own, "vec3f(2.0 * length(reach))");
        let body = format!(
            "    let tau = 6.2831853;
    let n = max(round({copies}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w0 = max(spread * tau / n, 1.0e-5);
    let bx = {input};
    let reach = sd_copy_reach(bx, {offset}, {scale}, {turned});
{head}    let i = f32(t);
    let at = {off};
    let turn = -({turn}) * tau;
    let sc = {sc};
    var f: array<vec3f, 4>;
    for (var e = 0; e < 4; e++) {{
        let p = vec3f(f32(e == 1), f32(e == 2), f32(e == 3));
        let l = {into};
        let xz = sd_rot2(l.xz, -i * w0);
        let m = sd_copy_frame(vec3f(xz.x - {{radius}}, l.y, xz.y), at, turn);
        f[e] = {back_m};
    }}
    sd_inst[{ib}u + t] = sd_instance(f[0], f[1], f[2], f[3], abs({back_sc}), {color}, bx);",
            off = copy.offset,
            turn = copy.turn,
            sc = copy.scale,
            color = copy.color,
            ib = slot.item_base,
        );
        Some(fill(node, ctx, &body))
    })
}

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
    bounds: Some(|node, ctx| {
        let b = ctx.solid_bounds(node, "input")?;
        let (offset, scale, turned) = percopy::extremes(ctx, node);
        let reach = format!("sd_copy_reach({b}, {offset}, {scale}, {turned})");
        let spacing = "vec3f({spacingX}, {spacingY}, {spacingZ})";
        let half = if ctx.option(node, "extent") == "counted" {
            let (cx, cy, cz) = (
                percopy::count_of(ctx, node, "countX"),
                percopy::count_of(ctx, node, "countY"),
                percopy::count_of(ctx, node, "countZ"),
            );
            format!(
                "select({reach}, (max(round(vec3f({cx}, {cy}, {cz})), vec3f(1.0)) - 1.0) * 0.5 * {spacing} + {reach}, {spacing} > vec3f(1.0e-4))"
            )
        } else {
            format!("select({reach}, vec3f(1.0e9), {spacing} > vec3f(1.0e-4))")
        };
        ctx.uniform_fill(node, &super::half(&half))
    }),
    outputs: &[out(|node, ctx, _| {
        let counted = ctx.option(node, "extent") == "counted";
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
        // The input's function first, so a repeat inside it takes its grid before this one.
        let _ = ctx.input(node, "input", "p");
        let slot = if counted {
            grid_slot(ctx, node, &copy, (&cx, &cy, &cz))
        } else {
            None
        };
        let measure_copy = percopy::evaluate(ctx, node, &copy, "input", str::to_string);
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
        // One copy's evaluation at `cell`, kept if it is the nearest so far.
        let eval = format!(
            "        let q = p - (cell - mid) * step;
{measure_copy}        let d = r.x * k;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
"
        );
        // Copies past the measured block are no nearer than its outer walls: a march's step
        // never crosses one, so a copy two cells off that stands taller than the three measured
        // is met at the wall and measured from the next cell. Only while a ray is marched
        // (`sd_march`): a soft shadow reads the distance as how near the copies come, and the
        // wall is no copy.
        let (lo, hi) = if counted {
            ("near >= vec3f(2.0)", "near <= n - 3.0")
        } else {
            ("vec3f(1.0) > vec3f(0.0)", "vec3f(1.0) > vec3f(0.0)")
        };
        let neighbors = format!(
            "    for (var j = 0; j < 7; j++) {{
        var cell = near;
        if (j > 0) {{
            let a = (j - 1) / 2;
            let axis = vec3f(f32(a == 0), f32(a == 1), f32(a == 2));
            if (dot(axis, select(vec3f(0.0), vec3f(1.0), on)) == 0.0) {{ continue; }}
            cell += axis * select(-1.0, 1.0, (j - 1) % 2 == 0);
{skip}        }}
{eval}    }}
    let below = select(vec3f(1.0e9), p - (near - mid - 1.5) * step, on & ({lo}));
    let above = select(vec3f(1.0e9), (near - mid + 1.5) * step - p, on & ({hi}));
    let wall = min(min(below.x, below.y), min(below.z, min(above.x, min(above.y, above.z))));
    if (sd_march) {{ best.x = min(best.x, wall); }}
"
        );
        let measure = match ctx.solid_bounds(node, "input") {
            Some(bounds) => {
                let (offset, scale, turned) = percopy::extremes(ctx, node);
                let skip = skip.replace("            if", "                if");
                let eval = eval
                    .replacen(
                        "        let q = p - (cell - mid) * step;\n",
                        &format!(
                            "        let q = p - (cell - mid) * step;\n{}",
                            skip_copy(&copy)
                        ),
                        1,
                    )
                    .replacen("        let d = r.x * k;\n", DISTANCE_BOUNDED, 1);
                // The cells a copy can reach from its own, at most three either side, with a
                // tenth of a cell to spare, each copy skipped where its own box is further off than
                // the nearest so far; then the march held by the boxes of the copies past them,
                // which the spare tenth keeps from holding it to nothing.
                let beyond = if counted {
                    "                if (edge < 0.0 || edge > n[ax] - 1.0) { continue; }\n"
                } else {
                    ""
                };
                format!(
                    "    let inreach = sd_bounds_reach({bounds});
    let reach = sd_copy_reach({bounds}, {offset}, {scale}, {turned});
    let kk = select(vec3f(0.0), min(max(ceil(reach / select(vec3f(1.0), s, on) - 0.4), vec3f(0.0)), vec3f(3.0)), on);
    let ki = vec3i(kk);
    for (var dz = -ki.z; dz <= ki.z; dz++) {{
    for (var dy = -ki.y; dy <= ki.y; dy++) {{
    for (var dx = -ki.x; dx <= ki.x; dx++) {{
        let cell = near + vec3f(f32(dx), f32(dy), f32(dz));
{skip}{eval}    }}
    }}
    }}
    if (sd_march) {{
        var wall = 1.0e9;
        let lim = select(reach, vec3f(1.0e9), on);
        for (var ax = 0; ax < 3; ax++) {{
            if (!on[ax]) {{ continue; }}
            for (var sd = -1; sd <= 1; sd += 2) {{
                let edge = near[ax] + f32(sd) * (kk[ax] + 1.0);
{beyond}                var g = max(abs(p - (near - mid) * step) - lim, vec3f(0.0));
                g[ax] = max(f32(sd) * ((near[ax] - mid[ax]) * step[ax] - p[ax]) + (kk[ax] + 1.0) * step[ax] - reach[ax], 0.0);
                wall = min(wall, length(g));
            }}
        }}
        if (wall > 0.0) {{ best.x = min(best.x, wall); }}
    }}
"
                )
            }
            None => neighbors,
        };
        let listing = slot
            .map(|slot| listed(ctx, node, &slot))
            .unwrap_or_default();
        let body = format!(
            "{listing}    let s = vec3f({{spacingX}}, {{spacingY}}, {{spacingZ}});
    let on = s > vec3f(1.0e-4);
    let step = select(vec3f(0.0), s, on);
    let n = {n};
    let mid = {mid};
    let near = select(vec3f(0.0), {near}, on);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
{measure}    return best;"
        );
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
    bounds: Some(|node, ctx| {
        let b = ctx.solid_bounds(node, "input")?;
        let (offset, scale, turned) = percopy::extremes(ctx, node);
        ctx.uniform_fill(
            node,
            &super::half(&format!(
                "vec3f({{radius}} + length(sd_copy_reach({b}, {offset}, {scale}, {turned})))"
            )),
        )
    }),
    outputs: &[out(|node, ctx, _| {
        // The fold runs about y; another axis is turned onto y and back.
        let (into, back) = match ctx.option(node, "axis") {
            "x" => ("vec3f(-p.y, p.x, p.z)", "vec3f({v}.y, -{v}.x, {v}.z)"),
            "z" => ("vec3f(p.x, p.z, -p.y)", "vec3f({v}.x, -{v}.z, {v}.y)"),
            _ => ("p", "{v}"),
        };
        let copies = percopy::count_of(ctx, node, "copies");
        let copy = percopy::read(
            ctx,
            node,
            Item::Counted {
                index: "i",
                count: "n",
            },
        );
        let _ = ctx.input(node, "input", "w");
        let slot = around_slot(ctx, node, &copy, &copies, (into, back));
        let measure_copy = percopy::evaluate(ctx, node, &copy, "input", |v| back.replace("{v}", v));
        let color = if ctx.solid_color() {
            format!("r.yzw * {}", copy.color)
        } else {
            "r.yzw".to_string()
        };
        // One copy's evaluation, copy `i`: placed in the ring's frame, x out from the axis, moved
        // by its rows in that frame, and turned back onto the chosen axis for the input.
        let eval = format!(
            "        let xz = sd_rot2(l.xz, -i * w0);
        let q = vec3f(xz.x - {{radius}}, l.y, xz.y);
{measure_copy}        let d = r.x * k;
        if (d < best.x) {{ best = vec4f(d, {color}); }}
"
        );
        let bounded = ctx.solid_bounds(node, "input").map(|bounds| {
            let (offset, scale, turned) = percopy::extremes(ctx, node);
            (bounds, offset, scale, turned)
        });
        // The copies that can reach the point across the ring's plane, at most eight either
        // side, each skipped where its own box is further off than the nearest so far; then
        // the march held by the upright cylinders round the nearest copies past
        // them on either side, since along a ring a copy further round is further off. Only
        // the reach across the plane decides how far round a copy reaches: a copy lifted high
        // is no wider for it.
        let body = if let Some((bounds, offset, scale, turned)) = bounded {
            let eval_bounded = eval
                .replacen(
                    "        let q = vec3f(xz.x - {radius}, l.y, xz.y);\n",
                    &format!(
                        "        let q = vec3f(xz.x - {{radius}}, l.y, xz.y);\n{}",
                        skip_copy(&copy)
                    ),
                    1,
                )
                .replacen("        let d = r.x * k;\n", DISTANCE_BOUNDED, 1);
            format!(
                "    let tau = 6.2831853;
    let n = max(round({copies}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w0 = max(spread * tau / n, 1.0e-5);
    let l = {into};
    let a = floor_mod(atan2(l.z, l.x), tau);
    let middle = (n - 1.0) * w0 * 0.5;
    let kc = round((middle + floor_mod(a - middle + 3.14159265, tau) - 3.14159265) / w0);
    let rho = max({{radius}}, 1.0e-4);
    let inreach = sd_bounds_reach({bounds});
    let reach = sd_copy_reach({bounds}, {offset}, {scale}, {turned});
    let rc = length(reach.xz);
    var kr = 8.0;
    if (rc < rho) {{ kr = min(max(ceil(asin(rc / rho) / w0 - 0.4), 0.0), 8.0); }}
    if (spread > 0.9999) {{ kr = min(kr, floor(n * 0.5)); }}
    let ki = i32(kr);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = -ki; j <= ki; j++) {{
        var i = kc + f32(j);
        if (spread > 0.9999) {{
            i = floor_mod(i, n);
        }} else if (i < 0.0 || i > n - 1.0) {{
            continue;
        }}
{eval_bounded}    }}
    if (sd_march) {{
        var wall = 1.0e9;
        for (var e = -1; e <= 1; e += 2) {{
            var i = kc + f32(e) * (kr + 1.0);
            if (spread > 0.9999) {{
                if (2.0 * kr + 1.0 >= n) {{ continue; }}
                i = floor_mod(i, n);
            }} else if (i < 0.0 || i > n - 1.0) {{
                continue;
            }}
            let ca = i * w0;
            wall = min(wall, length(l.xz - vec2f(cos(ca), sin(ca)) * rho) - rc);
        }}
        if (wall > 0.0) {{ best.x = min(best.x, wall); }}
    }}
    return best;"
            )
        } else {
            // A ring measures the nearest copy and its two neighbors, and its march is held to the two
            // half-planes through the axis that bound them, since a copy past them is no nearer than
            // they are. Near the axis, where every
            // wedge meets, the walls would hold a step to nothing, and the copies stand a Radius off.
            let body = format!(
            "    let tau = 6.2831853;
    let n = max(round({copies}), 1.0);
    let spread = clamp({{spread}}, 0.0, 1.0);
    let w0 = max(spread * tau / n, 1.0e-5);
    let l = {into};
    let a = floor_mod(atan2(l.z, l.x), tau);
    let k0 = round(a / w0);
    var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
    for (var j = 0; j < 3; j++) {{
        var i = k0 + select(0.0, select(-1.0, 1.0, j == 1), j > 0);
        if (spread > 0.9999) {{
            i = floor_mod(i, n);
        }} else {{
            i = clamp(i, 0.0, n - 1.0);
            if (j == 0 && abs(a - tau) < abs(a - i * w0)) {{ i = 0.0; }}
        }}
{eval}    }}
    if (sd_march && length(l.xz) > 0.5 * {{radius}}) {{
        let lo_ok = spread > 0.9999 || k0 >= 2.0;
        let hi_ok = spread > 0.9999 || k0 <= n - 3.0;
        var wall = 1.0e9;
        for (var e = 0; e < 2; e++) {{
            if ((e == 0 && !lo_ok) || (e == 1 && !hi_ok)) {{ continue; }}
            let th = (k0 + select(-1.5, 1.5, e == 1)) * w0;
            let dir = vec2f(cos(th), sin(th));
            let along = dot(l.xz, dir);
            wall = min(wall, select(length(l.xz), abs(dot(l.xz, vec2f(-dir.y, dir.x))), along > 0.0));
        }}
        best.x = min(best.x, wall);
    }}
    return best;"
        );
            body
        };
        let listing = slot
            .map(|slot| listed(ctx, node, &slot))
            .unwrap_or_default();
        fill(node, ctx, &format!("{listing}{body}"))
    })],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
