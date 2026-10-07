// SPDX-License-Identifier: AGPL-3.0-or-later

//! The crop: a rectangle of the input, and a choice of what happens outside it.
//!
//! Ported from silvia's `regionabsolute.js` and `regionsized.js`, which are the same node
//! reached by two sets of handles — four edges, or a center and a size. Everything below the
//! handles is shared here as it is written twice there: the outside modes, the soft edge and
//! the coverage output.
//!
//! Four departures from the source.
//!
//! **The background is behind the input as well as around it.** silvia's background mode is
//! `mix(bg, input, mask)`, which fills outside the rectangle and leaves the input's own
//! transparency inside it, so a Text over a checkerboard was black around its letters. Here
//! the input is composited over the background, `input · mask + bg · (1 − input.a · mask)`,
//! Porter–Duff over on premultiplied colors as every color in the graph is. With the default
//! transparent background it is `input · mask` and with an opaque input `mix(bg, input,
//! mask)`, both exactly silvia's, so only a background behind a transparent input draws
//! anything new.
//!
//! **The outside modes are branchless.** silvia guards each with the `inside` test it already
//! computed for the mask; `mod` and `clamp` are identities inside the rectangle, so the test
//! buys nothing and the branch goes. The boundary itself is where the two differ: at exactly
//! `cropMax` silvia keeps the edge pixel and the wrap here returns `cropMin`, which is the
//! same pixel of the same tile one line over.
//!
//! **A rectangle a cable turned inside out stays defined.** A tile smaller than `1e-3` and a
//! `clamp` whose ends crossed both have no single answer in WGSL, and either is reachable from a
//! cabled edge. The guards leave a rectangle anybody would draw exactly where it was, and the
//! mask is silvia's unguarded arithmetic, so an inverted rectangle still covers nothing.
//!
//! **Softness has a floor.** `smoothstep(0.0, 0.0, x)` divides by zero, and zero is the
//! default the node ships with; `1e-5` is a hard edge at every scale worldspace has.
//!
//! The absolute version's icon is `▣` and not silvia's `⯐`, which the vendored symbol face
//! has no glyph for — it is inside the block `scripts/subset-fonts.py` already cuts, so the
//! source font is what lacks it and widening the subset would not find one.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// The rectangle from four edges, in the order silvia's handles are in.
const ABSOLUTE_RECT_WGSL: &str = "    let cropMin = vec2f({left}, {bottom});
    let cropMax = vec2f({right}, {top});
    let rectCenter = (cropMin + cropMax) * 0.5;
    let rectHalf = (cropMax - cropMin) * 0.5;
";

/// The same rectangle from a center and a size. The size is the full one and is halved here,
/// so the default one-by-one box is half the absolute version's default rectangle.
const SIZED_RECT_WGSL: &str = "    let rectCenter = vec2f({centerX}, {centerY});
    let rectHalf = vec2f({width}, {height}) * 0.5;
    let cropMin = rectCenter - rectHalf;
    let cropMax = rectCenter + rectHalf;
";

/// The coverage, which is the crop softened by `softness` and nothing else.
///
/// Only background mode has an edge to soften: the other three fill the frame from the
/// rectangle, so everywhere is covered and the softness knob has nothing to bite on.
fn region_mask_wgsl(mode: &str) -> &'static str {
    if mode == "background" {
        "    let overshoot = max(vec2f(0.0), abs(uv - rectCenter) - rectHalf);
    let mask = 1.0 - smoothstep(0.0, max({softness}, 1e-5), length(overshoot));
"
    } else {
        "    let mask = 1.0;\n"
    }
}

/// Where this fragment reads the input, and what the node returns having read it.
///
/// The sample coordinate is declared in the color body alone, so the mask function neither
/// wraps a coordinate it has no use for nor calls the input's producer.
fn region_color_wgsl(mode: &str) -> &'static str {
    match mode {
        "tile" => {
            "    let tileSize = max(cropMax - cropMin, vec2f(1e-3));
    let sampleUV = cropMin + floor_mod2(uv - cropMin, tileSize);
    return {input};"
        }
        "mirror" => {
            "    let tileSize = max(cropMax - cropMin, vec2f(1e-3));
    let tileUV = floor_mod2(uv - cropMin, tileSize * 2.0);
    let sampleUV = cropMin + tileSize - abs(tileUV - tileSize);
    return {input};"
        }
        "clamp" => {
            "    let sampleUV = clamp(uv, min(cropMin, cropMax), max(cropMin, cropMax));
    return {input};"
        }
        _ => {
            "    let sampleUV = uv;
    let fg = {input} * mask;
    return fg + {bgColor} * (1.0 - fg.a);"
        }
    }
}

node! {
    /// A rectangle of the input set by its four edges, and a choice of what is outside it.
    REGIONABSOLUTE,
    slug: "regionabsolute",
    icon: "▣",
    label: "Region (Absolute)",
    category: Transform,
    tooltip: "Crops the input to a rectangle set by its four edges. Outside it: the background \
              color, the rectangle tiled, the rectangle mirror-tiled, or the edge smeared \
              outward. The background color also shows through the input where it is \
              transparent. Edge Softness bites in background mode only.",
    // "Outside Mode" beside "Background Color" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "input" "Input" at "sampleUV" = Control::None,
        VaryingNumber "top" "Top" = Control::num(1.0, -4.0, 4.0, 0.01, "⬓"),
        VaryingNumber "bottom" "Bottom" = Control::num(-1.0, -4.0, 4.0, 0.01, "⬓"),
        VaryingNumber "left" "Left" = Control::num(-1.0, -4.0, 4.0, 0.01, "⬓"),
        VaryingNumber "right" "Right" = Control::num(1.0, -4.0, 4.0, 0.01, "⬓"),
        VaryingNumber "softness" "Edge Softness" = Control::num(0.0, 0.0, 0.2, 0.001, "⬓"),
        VaryingColor "bgColor" "Background" = Control::color("#00000000"),
    ],
    options: [
        "mode" "Outside Mode" = "background" [
            "background" => "Background Color",
            "tile" => "Tile",
            "mirror" => "Mirror Tile",
            "clamp" => "Clamp",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [ABSOLUTE_RECT_WGSL, region_mask_wgsl(ctx.option(node, "mode"))].concat()
    }),
    outputs: [
        VaryingColor "output" "Output"
            = varying(|node, ctx| region_color_wgsl(ctx.option(node, "mode")).to_string()),
        VaryingNumber "mask" "Crop Mask" in "[0, 1]" = "    return mask;",
    ],
}

node! {
    /// The same crop, set by a center and a size.
    REGIONSIZED,
    slug: "regionsized",
    icon: "⛶",
    label: "Region (Sized)",
    category: Transform,
    tooltip: "Crops the input to a rectangle set by its center and its size. Outside it: the \
              background color, the rectangle tiled, the rectangle mirror-tiled, or the edge \
              smeared outward. The background color also shows through the input where it \
              is transparent. Edge Softness bites in background mode only.",
    // "Outside Mode" beside "Background Color" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "input" "Input" at "sampleUV" = Control::None,
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "width" "Width" = Control::num(1.0, 0.01, 4.0, 0.01, "⬓"),
        VaryingNumber "height" "Height" = Control::num(1.0, 0.01, 4.0, 0.01, "⬓"),
        VaryingNumber "softness" "Edge Softness" = Control::num(0.0, 0.0, 0.2, 0.001, "⬓"),
        VaryingColor "bgColor" "Background" = Control::color("#00000000"),
    ],
    options: [
        "mode" "Outside Mode" = "background" [
            "background" => "Background Color",
            "tile" => "Tile",
            "mirror" => "Mirror Tile",
            "clamp" => "Clamp",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [SIZED_RECT_WGSL, region_mask_wgsl(ctx.option(node, "mode"))].concat()
    }),
    outputs: [
        VaryingColor "output" "Output"
            = varying(|node, ctx| region_color_wgsl(ctx.option(node, "mode")).to_string()),
        VaryingNumber "mask" "Crop Mask" in "[0, 1]" = "    return mask;",
    ],
}
