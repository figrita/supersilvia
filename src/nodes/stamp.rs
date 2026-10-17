// SPDX-License-Identifier: AGPL-3.0-or-later

//! Stamp: one picture copied Count times, laid out on a grid, a ring, a line or at points, each
//! copy moved, turned, sized and colored by the Per copy rows ([`crate::nodes::percopy`]).
//!
//! **Grid, Ring and Line fold.** The copy a fragment belongs to is worked out from its uv, so
//! the picture is read once per fragment however many copies there are, and a copy that
//! reaches past its cell is cut at the cell's edge. Overlap checks the neighbors as well, the
//! two either side on a line or a ring and the eight round a grid cell, which repairs a copy
//! moved or grown a little past its cell for three or nine times the reads.
//!
//! **Points places copy i at its Offset**, so Offset X and Y are the positions, and every
//! fragment looks at every copy: a loop over the copies up to [`CAP`], with a `break` at the
//! count (docs/loop-bounding.md). A copy's box rejects a fragment outside it before its picture
//! is read, so the picture is read only where a copy is, and the cost of the loop is the
//! count's: a few reads of the rows and a box test a copy. Binning the copies on the CPU,
//! so a fragment looks only at those near it, is the step past that, for counts in the
//! thousands.
//!
//! **The last copy is in front.** Copies are composited front to back from the last,
//! premultiplied, `acc + copy · (1 − acc.a)`, and the background goes under them all; Points
//! stops once a fragment is covered.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::distort::{input, shader_out};
use crate::nodes::percopy::{self, Item};
use crate::nodes::{Category, Control, NodeDef, OptionDef};

/// The most copies Points draws, the loop's literal bound: the Count knob's top end.
pub const CAP: u32 = 256;

/// The statements that read copy `i`'s rows, place it about `center` and lay it under what
/// is in front of it; a `continue` where the fragment misses it. Indented for a loop's body.
fn stamp(bind: &str) -> String {
    format!(
        "{bind}        let reach = half * max(abs(copyScale.x), abs(copyScale.y)) * 1.4143;
        if (any(abs(uv - center - copyOffset) > vec2f(reach))) {{ continue; }}
        let stampUV = {place} / half;
        if (any(abs(stampUV) > vec2f(1.0))) {{ continue; }}
        let copy = {{input}} * copyTint;
        acc += copy * (1.0 - acc.a);
",
        place = percopy::place_2d("uv - center"),
    )
}

/// The loop over the copies a fragment may be in, for one layout.
fn layout_wgsl(layout: &str, overlap: bool, stamp: &str) -> String {
    match layout {
        "points" => format!(
            "    let shown = min(n, {CAP}.0);
    for (var j = 0; j < {CAP}; j++) {{
        if (f32(j) >= shown || acc.a >= 1.0) {{ break; }}
        let i = shown - 1.0 - f32(j);
        let center = vec2f(0.0);
{stamp}    }}
"
        ),
        "ring" => {
            // Each neighbor once, the higher copy first: past the seam at copy 0 the order
            // turns over, and under three copies a neighbor is the same copy twice.
            let (loops, order) = if overlap {
                (
                    3,
                    "    var order = vec3f(1.0, 0.0, -1.0);
    if (near == 0.0) { order = vec3f(-1.0, 1.0, 0.0); } else if (near == ring - 1.0) { order = vec3f(0.0, -1.0, 1.0); }
",
                )
            } else {
                (1, "    let order = vec3f(0.0);\n")
            };
            format!(
                "    let radius = {{radius}};
    let ring = max(n, 1.0);
    let w = 6.2831853 / ring;
    let near = floor_mod(round(floor_mod(atan2(uv.y, uv.x), 6.2831853) / w), ring);
{order}    for (var j = 0; j < {loops}; j++) {{
        let step = order[j];
        if (n < 1.0 || (n < 3.0 && step == -1.0) || (n < 2.0 && step != 0.0)) {{ continue; }}
        let i = floor_mod(near + step, ring);
        let center = radius * vec2f(cos(i * w), sin(i * w));
{stamp}    }}
"
            )
        }
        "line" => {
            let (loops, step) = if overlap {
                (3, "f32(1 - j)")
            } else {
                (1, "0.0")
            };
            format!(
                "    let spacing = max({{spacing}}, 1.0e-4);
    let mid = (n - 1.0) * 0.5;
    let near = clamp(round(uv.x / spacing + mid), 0.0, max(n - 1.0, 0.0));
    for (var j = 0; j < {loops}; j++) {{
        let i = near + {step};
        if (i < 0.0 || i > n - 1.0) {{ continue; }}
        let center = vec2f((i - mid) * spacing, 0.0);
{stamp}    }}
"
            )
        }
        _ => {
            let (loops, step) = if overlap {
                (9, "vec2f(f32(1 - j % 3), f32(1 - j / 3))")
            } else {
                (1, "vec2f(0.0)")
            };
            format!(
                "    let spacing = max({{spacing}}, 1.0e-4);
    let columns = clamp(round({{columns}}), 1.0, max(n, 1.0));
    let dims = vec2f(columns, ceil(n / columns));
    let mid = (dims - 1.0) * 0.5;
    let near = clamp(round(uv / spacing + mid), vec2f(0.0), max(dims - 1.0, vec2f(0.0)));
    for (var j = 0; j < {loops}; j++) {{
        let cell = near + {step};
        if (any(cell < vec2f(0.0)) || any(cell > dims - 1.0)) {{ continue; }}
        let i = cell.y * columns + cell.x;
        if (i > n - 1.0) {{ continue; }}
        let center = (cell - mid) * spacing;
{stamp}    }}
"
            )
        }
    }
}

fn stamp_wgsl(node: NodeId, ctx: &mut CompileContext, _func: &str) -> String {
    let count = percopy::count_of(ctx, node, "count");
    let copy = percopy::read_2d(
        ctx,
        node,
        Item::Counted {
            index: "i",
            count: "n",
        },
    );
    let overlap = ctx.option(node, "overlap") == "on";
    let body = layout_wgsl(
        ctx.option(node, "layout"),
        overlap,
        &stamp(&percopy::bind_2d(&copy, "        ")),
    );
    let src = format!(
        "    let n = max(round({count}), 0.0);
    let half = max({{size}}, 1.0e-4) * 0.5;
    var acc = vec4f(0.0);
{body}    return acc + {{bgColor}} * (1.0 - acc.a);"
    );
    percopy::fill_2d(node, ctx, &src, &[("input", "stampUV")])
}

/// One picture copied Count times on a layout.
pub static STAMP: NodeDef = NodeDef {
    slug: "stamp",
    icon: "🔖",
    label: "Stamp",
    category: Category::Transform,
    tooltip: "Copies the picture's middle square Count times, each Size across, over a \
              background. Grid lays them in rows of Columns from the bottom left, Spacing \
              apart; Ring round a circle of Radius; Line along x, Spacing apart; Points at their \
              Offsets, so lists into Offset X and Y are where the copies go. Per copy moves, \
              turns, sizes and colors each copy, from a knob or a list stretched over the \
              copies, and a list into Count makes as many copies as it has items. On Grid, Ring \
              and Line a copy past its cell is cut, and Overlap looks next door too. The last \
              copy is in front; Points draws at most 256.",
    width: Some(216.0),
    inputs: &[
        input("input", "Picture", VaryingColor, Control::None),
        input(
            "bgColor",
            "Background",
            VaryingColor,
            Control::color("#00000000"),
        ),
        percopy::count("count", "Count", 8.0, CAP as f32),
        input(
            "size",
            "Size",
            VaryingNumber,
            Control::num(0.4, 0.01, 2.0, 0.01, "⬓"),
        ),
        input(
            "spacing",
            "Spacing",
            VaryingNumber,
            Control::num(0.45, 0.0, 2.0, 0.01, "⬓"),
        ),
        input(
            "columns",
            "Columns",
            VaryingNumber,
            Control::num(4.0, 1.0, 32.0, 1.0, ""),
        ),
        input(
            "radius",
            "Radius",
            VaryingNumber,
            Control::num(0.6, 0.0, 2.0, 0.01, "⬓"),
        ),
        percopy::OFFSET_X,
        percopy::OFFSET_Y,
        percopy::TURN,
        percopy::SCALE_X,
        percopy::SCALE_Y,
        percopy::COLOR,
    ],
    options: &[
        OptionDef {
            key: "layout",
            label: "Layout",
            default: "grid",
            choices: &[
                ("grid", "Grid"),
                ("ring", "Ring"),
                ("line", "Line"),
                ("points", "Points"),
            ],
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "overlap",
            label: "Overlap",
            default: "off",
            choices: &[("off", "Off"), ("on", "On")],
            ..OptionDef::EMPTY
        },
        percopy::HEADING,
    ],
    row_headings: percopy::ROW_HEADINGS,
    wgsl_utils: &[percopy::PLACE_2D_WGSL],
    outputs: &[shader_out("output", "Output", VaryingColor, stamp_wgsl)],
    ..NodeDef::EMPTY
};
