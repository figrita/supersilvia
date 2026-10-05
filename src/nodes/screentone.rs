// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cell patterns: halftone dots, mosaic cells and ordered dither.
//!
//! Ported from silvia's `halftone.js`, `mosaic.js` and `dither.js`. All three are
//! `Category::Effect`, and they are the three that widen what that category is defined as.
//! A blur is an effect because it reads a neighborhood; these read the picture at one place
//! and still are not color maps, because **what a fragment gets depends on where it sits
//! inside a cell**: a halftone dot grows out of its cell center, a mosaic reads its cell's
//! center and paints a border the input never had, and a dither takes no picture at all — a
//! `VaryingNumber` field and two colors — and thresholds the field against a pattern over
//! the pixel grid. [decisions.md](../../../docs/decisions.md) records the rule.
//!
//! **The pixel grid comes from `uv`, not from `gl_FragCoord`.** silvia's halftone reads the
//! fragment's window coordinate directly, so its dots stand still while the picture under
//! them moves. Here one world unit is half of [`REFERENCE_HEIGHT`] pixels — the worldspace
//! height is exactly 2.0 — so the grid is derived from the coordinate the node was called
//! with and a transform upstream carries the pattern with the picture. It is also square,
//! which is what keeps a dot round. A cell is therefore a cell of a 720-high frame in every
//! Output, and an Output of another height draws the same cells at its own pixel pitch;
//! [decisions.md](../../../docs/decisions.md) states the cost.
//!
//! `halftone` sizes its dots by the color's own channels, read through the prelude's
//! `unpremultiply`, and prints its ink at the input's alpha through `premultiply`, so a
//! half-transparent picture prints the same dots at half strength. `mosaic` and `dither` only
//! mix whole colors, which is exact on premultiplied colors as they are.
//!
//! `mosaic` publishes `mask` for the cell interior and `halftone` for its ink, which are both
//! coverage; `dither`'s is its own thresholded output, 1 where the light color won.
//!
//! `mosaic`'s rows are silvia's order, Border Color under Border Width, because a border is
//! one thing dialed at one end of the node; and its hexagon drops the border outright at a
//! width of zero, which is the one lattice whose softened edge would otherwise keep a
//! hairline there.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

/// The fragment's place in the pixel grid: half of [`REFERENCE_HEIGHT`] pixels per world
/// unit, from the worldspace height of exactly 2.0.
pub(crate) fn pixel_grid_wgsl() -> String {
    format!("    let pixel = uv * ({REFERENCE_HEIGHT:?} * 0.5);\n")
}

// ---------------------------------------------------------------------------------- halftone

/// One screen's dot: the grid rotated by `a`, and the coverage of the dot in the cell the
/// fragment landed in.
///
/// A function rather than three copies of the same fifteen lines, which is what silvia's
/// three modes are.
pub const HALFTONE_DOT_WGSL: &str =
    "fn halftoneDot(pixel: vec2f, a: f32, dotSize: f32, amount: f32, smoothness: f32) -> f32 {
    let cs = cos(a);
    let sn = sin(a);
    let rot = vec2f(pixel.x * cs - pixel.y * sn, pixel.x * sn + pixel.y * cs);
    let cell = floor(rot / dotSize) * dotSize;
    let dist = length(rot - (cell + dotSize * 0.5)) / dotSize;
    let radius = sqrt(clamp(amount, 0.0, 1.0)) * 0.7;
    return 1.0 - smoothstep(radius - smoothness, radius + smoothness, dist);
}";

/// The screens one halftone mode prints, and the ink they come to.
fn halftone_screens_wgsl(mode: &str) -> &'static str {
    match mode {
        "cmyk" => {
            "    let cmy = 1.0 - clamp(color.rgb, vec3f(0.0), vec3f(1.0));
    let k = min(min(cmy.r, cmy.g), cmy.b);
    let amounts = vec4f((cmy - k) / max(1.0 - k, 1e-3), k);
    let screens = vec4f(15.0, 75.0, 0.0, 45.0) / 180.0 * PI + angle;
    var dots = vec4f(0.0);
    for (var i = 0; i < 4; i++) {
        dots[i] = halftoneDot(pixel, screens[i], dotSize, amounts[i], smoothness);
    }
    var ink = vec3f(1.0);
    ink = mix(ink, vec3f(0.0, 1.0, 1.0), dots.r);
    ink = mix(ink, vec3f(1.0, 0.0, 1.0), dots.g);
    ink = mix(ink, vec3f(1.0, 1.0, 0.0), dots.b);
    ink = mix(ink, vec3f(0.0), dots.a);
    let mask = max(max(dots.r, dots.g), max(dots.b, dots.a));
"
        }
        "rgb" => {
            "    let amounts = clamp(color.rgb, vec3f(0.0), vec3f(1.0));
    let screens = vec3f(15.0, 75.0, 45.0) / 180.0 * PI + angle;
    var dots = vec3f(0.0);
    for (var i = 0; i < 3; i++) {
        dots[i] = halftoneDot(pixel, screens[i], dotSize, amounts[i], smoothness);
    }
    let ink = dots;
    let mask = max(max(dots.r, dots.g), dots.b);
"
        }
        _ => {
            "    let gray = dot(color.rgb, vec3f(0.299, 0.587, 0.114));
    let mask = halftoneDot(pixel, angle, dotSize, gray, smoothness);
    let ink = vec3f(mask);
"
        }
    }
}

node! {
    /// A print screen: one dot per cell, sized by what is under it.
    HALFTONE,
    slug: "halftone",
    icon: "🦸",
    label: "Halftone",
    category: Effect,
    tooltip: "Prints the picture as dots on a rotated screen, each dot sized by the \
              brightness under it. CMYK and RGB print one screen per ink at the angles a \
              press uses. Its mask is the ink coverage.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "dotSize" "Dot Size" = Control::num(5.0, 2.0, 40.0, 0.1, "px"),
        VaryingNumber "angle" "Angle" = Control::num(0.125, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "smoothness" "Smoothness" = Control::num(0.1, 0.01, 0.5, 0.01, ""),
    ],
    options: [
        "mode" "Mode" = "mono" [
            "mono" => "Monochrome",
            "cmyk" => "CMYK",
            "rgb" => "RGB",
        ],
    ],
    wgsl_utils: [HALFTONE_DOT_WGSL],
    wgsl_common: varying(|node, ctx| {
        let grid = pixel_grid_wgsl();
        [
            "    let color = unpremultiply({input});
    let dotSize = max({dotSize}, 1.0);
    let angle = ({angle}) * 2.0 * PI;
    let smoothness = max({smoothness}, 1e-3);
",
            grid.as_str(),
            halftone_screens_wgsl(ctx.option(node, "mode")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return premultiply(vec4f(ink, color.a));",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

// ------------------------------------------------------------------------------------ mosaic

/// One 0 to 1 pair per cell, for jittering a cell's center off the lattice.
pub const MOSAIC_JITTER_WGSL: &str = "fn mosaicJitter(cell: vec2f) -> vec2f {
    return vec2f(
        fract(sin(dot(cell, vec2f(12.9898, 78.233))) * 43758.5453),
        fract(sin(dot(cell, vec2f(269.5, 183.3))) * 43758.5453));
}";

/// The lattice one mosaic shape lays down: where the cell's center is, and how far inside the
/// cell the fragment sits.
///
/// Every shape ends with `cellCenter` and `edge` declared, which is what lets the color and
/// the mask share a body and the input be read through the macro's `at`.
fn mosaic_lattice_wgsl(shape: &str) -> &'static str {
    match shape {
        "hexagon" => {
            "    let r = vec2f(1.0, 1.73);
    let hs = r * 0.5;
    let a = floor_mod2(p, r) - hs;
    let b = floor_mod2(p - hs, r) - hs;
    let gv = select(b, a, dot(a, a) < dot(b, b));
    let id = round(p - gv);
    let cellCenter = (id + (mosaicJitter(id) - 0.5) * randomness) / cellSize;
    let absGv = abs(gv);
    let edge = 0.5 - max(dot(absGv, normalize(vec2f(1.0, 1.73))), absGv.x);
"
        }
        "triangle" => {
            "    let skewed = vec2f(p.x - p.y * 0.5, p.y * 0.866);
    let cell = floor(skewed);
    let f = fract(skewed);
    let upper = step(f.x + f.y, 1.0);
    let tri = cell + (1.0 - upper);
    let g = mix(1.0 - f, f, upper);
    let middle = tri + vec2f(0.5, 0.5 + mix(0.166, -0.166, upper))
        + (mosaicJitter(tri) - 0.5) * randomness;
    let cellCenter = vec2f(middle.x + middle.y * 0.5, middle.y / 0.866) / cellSize;
    let edge = min(min(g.x, g.y), 1.0 - g.x - g.y);
"
        }
        "voronoi" => {
            "    let base = floor(p);
    let f = fract(p);
    var nearest = vec2f(0.0);
    var minDist = 10.0;
    var secondDist = 10.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let neighbor = vec2f(f32(x), f32(y));
            let cell = base + neighbor;
            let jitter = mosaicJitter(cell);
            let d = length(neighbor + mix(vec2f(0.5), jitter, randomness) - f);
            if (d < minDist) {
                secondDist = minDist;
                minDist = d;
                nearest = (cell + jitter) / cellSize;
            } else if (d < secondDist) {
                secondDist = d;
            }
        }
    }
    let cellCenter = nearest;
    let edge = (secondDist - minDist) * 0.5;
"
        }
        _ => {
            "    let cell = floor(p);
    let f = fract(p);
    let cellCenter = (cell + 0.5 + (mosaicJitter(cell) - 0.5) * randomness) / cellSize;
    let edge = min(min(f.x, f.y), min(1.0 - f.x, 1.0 - f.y));
"
        }
    }
}

/// The cell's interior as coverage, from how far inside it the fragment sits.
///
/// The hexagon's border **goes entirely** at a width of zero, which is silvia's own
/// `borderWidth > 0.0 ? … : 1.0`: its `edge` is a hex distance that the smoothing straddles
/// at zero, so the softened step still paints a hairline along every cell wall where the
/// other three lattices leave none.
fn mosaic_border_wgsl(shape: &str) -> &'static str {
    if shape == "hexagon" {
        "    let mask = select(
        1.0,
        smoothstep(borderWidth - smoothing * 0.5, borderWidth + smoothing * 0.5, edge),
        borderWidth > 0.0);
"
    } else {
        "    let mask = smoothstep(borderWidth - smoothing * 0.5, borderWidth + smoothing * 0.5, edge);
"
    }
}

node! {
    /// The frame broken into cells, each flooded with the color at its center.
    MOSAIC,
    slug: "mosaic",
    icon: "🪟",
    label: "Mosaic",
    category: Effect,
    tooltip: "Floods each cell of a lattice with the color at the cell's center and draws a \
              border between them. Square, hexagon, triangle and Voronoi lattices; randomness \
              jitters each center off the grid. Its mask is the inside of a cell.",
    // "Border Color" and "Border Width" do not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "cellCenter" = Control::None,
        VaryingNumber "cellSize" "Cell Freq" = Control::num(10.0, 2.0, 50.0, 0.5, "/⬓"),
        VaryingNumber "randomness" "Randomness" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "borderWidth" "Border Width" = Control::num(0.1, 0.0, 0.5, 0.01, ""),
        VaryingColor "borderColor" "Border Color" = Control::color("#000000ff"),
        VaryingNumber "smoothing" "Smoothing" = Control::num_log(0.02, 0.001, 1.0, 0.001, ""),
    ],
    options: [
        "shape" "Shape" = "square" [
            "square" => "Square",
            "hexagon" => "Hexagon",
            "triangle" => "Triangle",
            "voronoi" => "Voronoi",
        ],
    ],
    wgsl_utils: [MOSAIC_JITTER_WGSL],
    // A cable can drive the frequency to zero, where the lattice divides by it, and the
    // smoothing to zero, where a `smoothstep` of equal edges is undefined.
    wgsl_common: varying(|node, ctx| {
        [
            "    let cellSize = max({cellSize}, 1e-3);
    let randomness = clamp({randomness}, 0.0, 1.0);
    let borderWidth = {borderWidth};
    let smoothing = max({smoothing}, 1e-4);
    let p = uv * cellSize;
",
            mosaic_lattice_wgsl(ctx.option(node, "shape")),
            mosaic_border_wgsl(ctx.option(node, "shape")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return mix({borderColor}, {input}, mask);",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

// ------------------------------------------------------------------------------------ dither

/// The threshold a fragment is measured against, from its cell in the pixel grid.
///
/// The three Bayer matrices are built by bit-reversal interleave rather than stored, which is
/// how silvia writes them and is one expression instead of sixty-four constants.
///
/// `posterize` reads the same three matrices for its own Pattern row, so the two nodes cannot
/// draw different Bayer grids under one name.
///
/// A WGSL shift's count is a `u32`, and WGSL will not chain a shift or mix it with `&`
/// unparenthesized, so each bit is its own parenthesized step; the 2x2 ranks are an array
/// indexed by the cell rather than a chain of conditionals.
pub(crate) fn dither_threshold_wgsl(mode: &str) -> &'static str {
    match mode {
        "bayer2" => "    let bx = i32(floor_mod(cellPixel.x, 2.0));
    let by = i32(floor_mod(cellPixel.y, 2.0));
    let idx = bx + by * 2;
    var ranks = array<f32, 4>(0.0, 2.0, 3.0, 1.0);
    let rank = ranks[idx];
    let threshold = (rank + 0.5) / 4.0;
",
        "bayer8" => "    let bx = i32(floor_mod(cellPixel.x, 8.0));
    let by = i32(floor_mod(cellPixel.y, 8.0));
    var val = 0;
    val |= ((bx ^ by) & 1) << 5u;
    val |= (by & 1) << 4u;
    val |= (((bx ^ by) & 2) >> 1u) << 3u;
    val |= ((by & 2) >> 1u) << 2u;
    val |= (((bx ^ by) & 4) >> 2u) << 1u;
    val |= (by & 4) >> 2u;
    let threshold = (f32(val) + 0.5) / 64.0;
",
        "noise" => "    let n = fract(sin(dot(cellPixel, vec2f(127.1, 311.7))) * 43758.5453);
    let threshold = fract(n * fract(sin(dot(cellPixel.yx + 31.71, vec2f(269.5, 183.3))) * 28461.7231));
",
        _ => "    let bx = i32(floor_mod(cellPixel.x, 4.0));
    let by = i32(floor_mod(cellPixel.y, 4.0));
    var val = 0;
    val |= ((bx ^ by) & 1) << 3u;
    val |= (by & 1) << 2u;
    val |= (((bx ^ by) & 2) >> 1u) << 1u;
    val |= (by & 2) >> 1u;
    let threshold = (f32(val) + 0.5) / 16.0;
",
    }
}

/// Which of the two coordinates — the fragment's or its cell's center — each input is read
/// at, from the `pixelate` option.
fn dither_coordinates_wgsl(pixelate: &str) -> &'static str {
    match pixelate {
        "mask" => "    let ditherUV = quv;\n    let colorUV = uv;\n",
        "none" => "    let ditherUV = uv;\n    let colorUV = uv;\n",
        _ => "    let ditherUV = quv;\n    let colorUV = quv;\n",
    }
}

node! {
    /// A number turned into two colors through an ordered threshold pattern.
    DITHER,
    slug: "dither",
    icon: "▩",
    label: "Dither",
    category: Effect,
    tooltip: "Turns a 0 to 1 number into two colors by thresholding it against an ordered \
              pattern, so a smooth ramp becomes a spray of dots instead of a gradient. Scale \
              is how many pixels wide one cell of the pattern is.",
    inputs: [
        VaryingNumber "value" "Value" at "ditherUV" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingColor "light" "Light" at "colorUV" = Control::color("#ffffffff"),
        VaryingColor "dark" "Dark" at "colorUV" = Control::color("#000000ff"),
        VaryingNumber "scale" "Scale" = Control::num(1.0, 1.0, 16.0, 1.0, "px"),
    ],
    options: [
        "mode" "Pattern" = "bayer4" [
            "bayer2" => "Bayer 2x2",
            "bayer4" => "Bayer 4x4",
            "bayer8" => "Bayer 8x8",
            "noise" => "Hash Noise",
        ],
        "pixelate" "Pixelate" = "all" [
            "all" => "All",
            "mask" => "Value Only",
            "none" => "None",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        let grid = pixel_grid_wgsl();
        let quantized = format!(
            "    let cellPixel = floor(pixel / scale);
    let quv = (cellPixel * scale + scale * 0.5) / ({REFERENCE_HEIGHT:?} * 0.5);
"
        );
        [
            "    let scale = max(floor({scale}), 1.0);\n",
            grid.as_str(),
            quantized.as_str(),
            dither_coordinates_wgsl(ctx.option(node, "pixelate")),
            dither_threshold_wgsl(ctx.option(node, "mode")),
            "    let mask = step(threshold, clamp({value}, 0.0, 1.0));\n",
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return mix({dark}, {light}, mask);",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}
