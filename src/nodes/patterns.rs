// SPDX-License-Identifier: AGPL-3.0-or-later

//! Patterns that repeat across the frame: stripes, a grid, polka dots, houndstooth, flags.
//!
//! Ported from silvia's `stripes.js`, `grid.js`, `polkadot.js`, `houndstooth.js` and
//! `prideflag.js`. The Progress Pride chevron is from
//! <https://godotshaders.com/author/jtad/>, CC0, as silvia has it.
//!
//! `stripes` centers on `uv - 0.5` before rotating, as silvia does: `frequency` counts bands
//! per world unit, twice across the frame, and a non-zero `rotation` turns the pattern about
//! the frame's center rather than its origin.
//!
//! `grid` and `polkadot` publish the coverage they drew; `stripes` and `houndstooth` do not,
//! because there the choice between the two colors *is* the picture and a mask beside it
//! would be the same field twice. A flag has no field at all.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};
use std::fmt::Write as _;

node! {
    /// Parallel bands at an angle, with a duty cycle.
    STRIPES,
    slug: "stripes",
    icon: "💈",
    label: "Stripes",
    category: Generate,
    tooltip: "Parallel bands at an angle. Phase is the duty cycle: how much of each cycle the \
              first color takes.",
    inputs: [
        VaryingNumber "frequency" "Frequency" = Control::num(8.0, 1.0, 64.0, 1.0, "/⬓"),
        VaryingNumber "phase" "Phase" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingColor "color1" "Color 1" = Control::color("#ffffffff"),
        VaryingColor "color2" "Color 2" = Control::color("#000000ff"),
        VaryingNumber "smoothing" "Smoothing" = Control::num_log(0.01, 0.0, 1.0, 0.001, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = "    let freq = {frequency};
    let duty = {phase};
    let rot = ({rotation}) * 2.0 * PI;
    let smoothing = {smoothing};
    let centered = uv - 0.5;
    let rotated = vec2f(centered.x * cos(rot) - centered.y * sin(rot), centered.x * sin(rot) + centered.y * cos(rot));
    let cycle = fract((rotated.x + 0.5) * freq);
    let halfDuty = duty * 0.5;
    let edge = smoothing * freq * 0.05;
    let band = smoothstep(0.5 - halfDuty - edge, 0.5 - halfDuty + edge, cycle)
        * (1.0 - smoothstep(0.5 + halfDuty - edge, 0.5 + halfDuty + edge, cycle));
    return mix({color2}, {color1}, band);",
    ],
}

node! {
    /// Ruled lines in two directions, and the ink they lay down.
    GRID,
    slug: "grid",
    icon: "⊞",
    label: "Grid",
    category: Generate,
    tooltip: "Ruled lines in both directions, with independent cell counts. Its mask is the \
              ink the lines lay down.",
    // "Line Thickness" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "cellsX" "Cells X" = Control::num(10.0, 1.0, 100.0, 1.0, ""),
        VaryingNumber "cellsY" "Cells Y" = Control::num(10.0, 1.0, 100.0, 1.0, ""),
        VaryingNumber "thickness" "Line Thickness" = Control::num(0.05, 0.0, 0.5, 0.01, ""),
        VaryingNumber "offsetX" "Offset X" = Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
        VaryingNumber "offsetY" "Offset Y" = Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
        VaryingNumber "smoothing" "Smoothing" = Control::num_log(0.01, 0.0, 1.0, 0.001, ""),
    ],
    // Thickness is scaled per axis by that axis's cell count, so a line is the same width in
    // pixels whether there are ten cells across or a hundred.
    wgsl_common: "    let cells = vec2f({cellsX}, {cellsY});
    let thickness = {thickness};
    let smoothing = {smoothing};
    let cell = fract((uv + vec2f({offsetX}, {offsetY})) * cells);
    let halfWidth = thickness * cells * 0.05;
    let blur = smoothing * cells * 0.05;
    let line = max(
        1.0 - smoothstep(halfWidth - blur, halfWidth + blur, cell),
        1.0 - smoothstep(halfWidth - blur, halfWidth + blur, 1.0 - cell)
    );
    let mask = max(line.x, line.y);
",
    outputs: [
        VaryingColor "color" "Color" = "    return mix({background}, {foreground}, mask);",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

node! {
    /// Discs on a lattice, with alternate rows or columns offset.
    POLKADOT,
    slug: "polkadot",
    icon: "👙",
    label: "Polka Dot",
    category: Generate,
    tooltip: "Discs on a lattice, with alternate rows or columns offset. Radius, softness and \
              the dot color are sampled at the center of the dot, so a field feeding them \
              varies per dot rather than per pixel.",
    inputs: [
        VaryingColor "input" "Input" at "cellCenter" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "frequency" "Frequency" = Control::num(8.0, 1.0, 50.0, 0.5, "/⬓"),
        VaryingNumber "radius" "Radius" at "cellCenter" = Control::num(0.35, 0.0, 1.0, 0.01, ""),
        VaryingNumber "softness" "Softness" at "cellCenter" = Control::num_log(0.02, 0.0, 0.5, 0.001, ""),
        VaryingNumber "stagger" "Stagger" at "preCenter" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "staggerDirection" "Stagger Direction" = "horizontal" [
            "horizontal" => "Horizontal (Rows)",
            "vertical" => "Vertical (Columns)",
        ],
    ],
    // Two passes over the lattice: the first finds the cell a fragment is in so the stagger
    // can be sampled at its center, the second applies the stagger and finds the cell again.
    wgsl_common: varying(|node, ctx| {
        let (offset, undo) = match ctx.option(node, "staggerDirection") {
            "vertical" => (
                "    p.y += stagger * floor_mod(floor(p.x), 2.0);\n",
                "    cellCenter.y -= stagger * floor_mod(cell.x, 2.0);\n",
            ),
            _ => (
                "    p.x += stagger * floor_mod(floor(p.y), 2.0);\n",
                "    cellCenter.x -= stagger * floor_mod(cell.y, 2.0);\n",
            ),
        };
        [
            "    let freq = max({frequency}, 1e-3);
    var p = uv * freq;
    let preCenter = (floor(p) + 0.5) / freq;
    let stagger = {stagger};
",
            offset,
            "    let cell = floor(p);
    var cellCenter = cell + 0.5;
",
            undo,
            "    cellCenter /= freq;
    let radius = {radius};
    let softness = {softness};
    let dist = length(fract(p) - 0.5);
    let mask = 1.0 - smoothstep(radius - softness, radius + softness, dist);
",
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return mix({background}, {input}, mask);",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

node! {
    /// The four-quadrant weave that makes a houndstooth check.
    HOUNDSTOOTH,
    slug: "houndstooth",
    icon: "🦷",
    label: "Houndstooth",
    category: Generate,
    tooltip: "The broken check woven into tweed: two solid quadrants and two striped ones, \
              tiled.",
    inputs: [
        VaryingColor "colorA" "Color A" = Control::color("#ffffffff"),
        VaryingColor "colorB" "Color B" = Control::color("#000000ff"),
        VaryingNumber "scale" "Frequency" = Control::num(1.0, 0.1, 10.0, 0.01, "/⬓"),
        VaryingNumber "offsetX" "Offset X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "offsetY" "Offset Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // The two striped quadrants take opposite ends of the same 45° stripe, which is the
    // break that turns a check into a houndstooth.
    outputs: [
        VaryingColor "output" "Output" = "    let a = {colorA};
    let b = {colorB};
    let tile = fract((uv + vec2f({offsetX}, {offsetY})) * ({scale}));
    if (tile.x < 0.5 && tile.y > 0.5) { return a; }
    if (tile.x > 0.5 && tile.y < 0.5) { return b; }
    let stripe = step(0.5, fract((tile.x + tile.y) * 2.0));
    if (tile.x < 0.5 && tile.y < 0.5) { return mix(a, b, stripe); }
    return mix(b, a, stripe);",
    ],
}

/// One flag's stripes, top to bottom, as the four components of each color, which a
/// `vec4f` makes a constant of.
///
/// Flags are data rather than an input list: the colors are the flag, and a control that
/// let one be edited would be a different node. Ten sets of them are shorter as a table than
/// as ten bodies.
const FLAGS: &[(&str, &[&str])] = &[
    (
        "lesbian",
        &[
            "0.839, 0.161, 0.0, 1.0",
            "0.937, 0.463, 0.153, 1.0",
            "1.0, 0.608, 0.333, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.831, 0.380, 0.651, 1.0",
            "0.710, 0.337, 0.565, 1.0",
            "0.647, 0.0, 0.384, 1.0",
        ],
    ),
    (
        "gay_mlm",
        &[
            "0.031, 0.553, 0.439, 1.0",
            "0.149, 0.808, 0.667, 1.0",
            "0.596, 0.910, 0.757, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.482, 0.678, 0.886, 1.0",
            "0.314, 0.286, 0.800, 1.0",
            "0.239, 0.102, 0.471, 1.0",
        ],
    ),
    (
        "bi",
        &[
            "0.839, 0.008, 0.439, 1.0",
            "0.608, 0.310, 0.588, 1.0",
            "0.0, 0.220, 0.659, 1.0",
        ],
    ),
    (
        "transgender",
        &[
            "0.333, 0.804, 0.988, 1.0",
            "0.969, 0.659, 0.722, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.969, 0.659, 0.722, 1.0",
            "0.333, 0.804, 0.988, 1.0",
        ],
    ),
    (
        "rainbow",
        &[
            "1.0, 0.0, 0.0, 1.0",
            "1.0, 0.600, 0.0, 1.0",
            "1.0, 0.996, 0.075, 1.0",
            "0.024, 0.624, 0.176, 1.0",
            "0.004, 0.310, 0.910, 1.0",
            "0.569, 0.004, 0.631, 1.0",
        ],
    ),
    (
        "pan",
        &[
            "1.0, 0.106, 0.553, 1.0",
            "1.0, 0.855, 0.0, 1.0",
            "0.106, 0.702, 1.0, 1.0",
        ],
    ),
    (
        "asexual",
        &[
            "0.0, 0.0, 0.0, 1.0",
            "0.643, 0.643, 0.643, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.506, 0.0, 0.506, 1.0",
        ],
    ),
    (
        "nonbinary",
        &[
            "1.0, 0.957, 0.188, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.612, 0.349, 0.820, 1.0",
            "0.0, 0.0, 0.0, 1.0",
        ],
    ),
    (
        "gilbert_baker",
        &[
            "0.992, 0.412, 0.702, 1.0",
            "1.0, 0.0, 0.0, 1.0",
            "1.0, 0.553, 0.027, 1.0",
            "1.0, 0.988, 0.024, 1.0",
            "0.012, 0.553, 0.0, 1.0",
            "0.0, 0.761, 0.761, 1.0",
            "0.251, 0.012, 0.475, 1.0",
            "0.557, 0.004, 0.051, 1.0",
        ],
    ),
    (
        "progress",
        &[
            "0.890, 0.016, 0.016, 1.0",
            "1.0, 0.549, 0.0, 1.0",
            "1.0, 0.929, 0.0, 1.0",
            "0.0, 0.502, 0.149, 1.0",
            "0.0, 0.302, 1.0, 1.0",
            "0.459, 0.0, 0.529, 1.0",
            "1.0, 1.0, 1.0, 1.0",
            "0.961, 0.659, 0.722, 1.0",
            "0.361, 0.816, 0.980, 1.0",
            "0.353, 0.192, 0.055, 1.0",
            "0.0, 0.0, 0.0, 1.0",
        ],
    ),
];

/// The stripes a flag name selects, falling back to the rainbow.
fn stripes_of(name: &str) -> &'static [&'static str] {
    FLAGS
        .iter()
        .find(|(key, _)| *key == name)
        .map_or(FLAGS[4].1, |(_, stripes)| *stripes)
}

/// Horizontal bands, top to bottom, as a chain of thresholds on the vertical axis.
fn banded(stripes: &[&str]) -> String {
    let n = stripes.len();
    let mut out = String::from("    let y = 1.0 - fract((uv.y + 1.0) * 0.5);\n");
    for (i, color) in stripes.iter().enumerate() {
        if i + 1 == n {
            let _ = write!(out, "    return vec4f({color});");
        } else {
            let numerator = i + 1;
            let _ = writeln!(
                out,
                "    if (y < {numerator}.0/{n}.0) {{ return vec4f({color}); }}"
            );
        }
    }
    out
}

/// Six bands with the five-color chevron laid over their left edge.
fn progress(stripes: &[&str]) -> String {
    let mut out = String::from(
        "    let x = (uv.x + 1.0) * 0.5;
    let y = (uv.y + 1.0) * 0.5;
    var color: vec4f;
",
    );
    for (i, color) in stripes[..6].iter().rev().enumerate() {
        if i == 5 {
            let _ = writeln!(out, "    else {{ color = vec4f({color}); }}");
        } else {
            let branch = if i == 0 { "if" } else { "else if" };
            let numerator = i + 1;
            let _ = writeln!(
                out,
                "    {branch} (y < {numerator}.0/6.0) {{ color = vec4f({color}); }}"
            );
        }
    }
    out.push_str("    let chevron = 1.0 - (abs(0.5 - (1.0 - y)) + x * 1.20 + 0.5);\n");
    for (i, color) in stripes[6..].iter().enumerate() {
        let threshold = (4.5 - 0.9 * i as f32) / 6.0;
        let branch = if i == 0 { "if" } else { "else if" };
        let _ = writeln!(
            out,
            "    {branch} (chevron > {threshold:.4}) {{ color = vec4f({color}); }}"
        );
    }
    out.push_str("    return color;");
    out
}

/// The body of the flag the node's option names. WGSL's `if` takes a braced block, never a
/// bare statement, so every branch is braced.
fn flag(node: NodeId, ctx: &CompileContext<'_>) -> String {
    let flag = ctx.option(node, "flagType");
    let stripes = stripes_of(flag);
    if flag == "progress" {
        progress(stripes)
    } else {
        banded(stripes)
    }
}

node! {
    /// Ten pride flags, as horizontal bands or a chevron.
    PRIDEFLAG,
    slug: "prideflag",
    icon: "🏳",
    label: "Pride Flag",
    category: Generate,
    tooltip: "Ten pride flags, drawn as horizontal bands. Progress Pride adds the chevron.",
    inputs: [],
    options: [
        "flagType" "Flag Type" = "rainbow" [
            "lesbian" => "Lesbian",
            "gay_mlm" => "Gay/MLM",
            "bi" => "Bisexual",
            "transgender" => "Transgender",
            "rainbow" => "Rainbow",
            "pan" => "Pansexual",
            "asexual" => "Asexual",
            "nonbinary" => "Nonbinary",
            "gilbert_baker" => "8-stripe Gilbert Baker",
            "progress" => "Progress Pride",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(flag),
    ],
}
