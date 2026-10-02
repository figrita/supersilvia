// SPDX-License-Identifier: AGPL-3.0-or-later

//! Distortions: nodes that move the sampling coordinate a long way rather than a little.
//!
//! Ported from silvia's `wave.js`, `whirlandpinch.js`, `tunnel3d.js`, `kaleidoscope.js`,
//! `tile.js`, `repeater.js`, `domainwarp.js` and `scatter.js`, which silvia files under a
//! `Distort` category of its own. Here they are `Category::Transform`, which is defined as
//! *moves the sampling coordinate* — precisely what all eight do, and what `zoom`, `rotate`
//! and `fisheye` in `transform.rs` do. The category says what a node does to the uv, not how
//! wild the result looks; [decisions.md](../../../docs/decisions.md) has the argument.
//!
//! Every one of them reads its input through the macro's `at`, so the value spliced in is
//! `source(distortedUV)` and the transform composes into the caller's shader with no
//! machinery of its own.
//!
//! Four departures from the source.
//!
//! **A falloff instead of an early return.** silvia's `whirlandpinch` returns the untouched
//! input outside its radius and the distorted one inside, which is two samples of the same
//! input at two coordinates — and an input here is sampled at one. Its falloff is already
//! zero at the radius, so the one distorted sample *is* the untouched one out there.
//!
//! **`repeater` is branchless.** Its four early returns become one coverage product, so the
//! color and the mask share a `wgsl_common` block; silvia writes the whole grid test twice, once
//! per generator.
//!
//! **Time and Offset, and no speed.** silvia's `domainwarp` reads `u_time * timeSpeed`, and
//! its `tunnel3d` drives the camera from a CPU phase accumulator with a Start/Stop button on
//! the node. Here each reads **Time**, ambient or a gear's, with **Offset** added: the warp in
//! lattice cells, still at rest as silvia's is, with a Repeat like the noises'; the tunnel in
//! units of camera depth, at silvia's half a unit a second. **The tunnel's path is retuned so
//! the flight repeats**: every path frequency is silvia's times 5π/16, so Sine and Lissajous
//! come back every 64 units and the Helix every 16 — each a whole number of both depth wraps,
//! so the whole flight comes back every 64 under Mirror or Repeat. Depth Wrap None never does.
//!
//! **`domainwarp` publishes `value`, not `mask`.** The length of its warp vector is the raw
//! quantity the picture was made from, not coverage, and `mask` means coverage.

use crate::graph::PortType::{UniformNumber, VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::noise::{
    FBM_LOOP_WGSL, FBM_WGSL, LOOP_CIRCLE_WGSL, SIMPLEX3D_WGSL, SIMPLEX4D_WGSL, fbm_time,
    noise_ambient,
};
use crate::nodes::{
    Ambient, Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind,
};

// ------------------------------------------------------------------------------ shader helpers

/// A hash-based random number, from silvia's `shaderUtils.HASH_RANDOM`: Bob Jenkins'
/// one-at-a-time hash over the bits of the coordinate, with the result poured into a float's
/// mantissa.
///
/// `fract(sin(dot(…)))` is the alternative and it repeats visibly over the cell counts
/// `scatter` reaches.
///
/// From Spatial's Stack Overflow answer (2013), CC BY-SA 3.0; see licenses/spatial-hash.txt.
///
/// WGSL has no overloading, so the `vec2u` version of `hashBits` is `hashBits2`; and a WGSL
/// parameter is immutable, so the one that steps its argument steps a copy.
pub const HASH_RANDOM_WGSL: &str = "fn hashBits(x0: u32) -> u32 {
    var x = x0;
    x += (x << 10u);
    x ^= (x >> 6u);
    x += (x << 3u);
    x ^= (x >> 11u);
    x += (x << 15u);
    return x;
}

fn hashBits2(v: vec2u) -> u32 { return hashBits(v.x ^ hashBits(v.y)); }

fn hashRandom(v: vec2f) -> f32 {
    var m = hashBits2(bitcast<vec2u>(v));
    m &= 0x007FFFFFu;
    m |= 0x3F800000u;
    return bitcast<f32>(m) - 1.0;
}";

/// The tube's center line at a depth, in three shapes. `tw` is the twist amount.
///
/// All three are emitted together because one option chooses between them and the option is
/// a recompile boundary; three four-line functions cost less than a branch inside the march.
///
/// silvia's frequencies, 0.3 and 0.5 and the Helix's 0.4, are each multiplied by 5π/16, so the
/// three come back every 64, 64 and 16 units of depth: 3π/32 and 5π/32, and π/8.
pub const TUNNEL_PATH_WGSL: &str = "fn tunnelPathSine(z: f32, tw: f32) -> vec2f {
    return vec2f(sin(z * 0.29452431 + 1.7) * tw, cos(z * 0.49087385 + 2.3) * tw * 0.7);
}

fn tunnelPathHelix(z: f32, tw: f32) -> vec2f {
    return vec2f(cos(z * 0.39269908) * tw, sin(z * 0.39269908) * tw);
}

fn tunnelPathLissajous(z: f32, tw: f32) -> vec2f {
    return vec2f(sin(z * 0.29452431) * tw, sin(z * 0.49087385 + 1.5708) * tw);
}";

/// How far the tunnel flies before its picture comes back, while its depth wraps.
pub const TUNNEL_REPEAT: f64 = 64.0;

/// The tunnel's period: [`TUNNEL_REPEAT`] while its depth wraps, and none with Depth Wrap None,
/// whose wall is never the same twice.
fn tunnel_period(node: &crate::graph::Node) -> Option<f64> {
    (node.options.get("wrap").map(String::as_str) != Some("none")).then_some(TUNNEL_REPEAT)
}

/// The camera's depth: Time + Offset, and while the depth wraps, Time's whole part taken
/// modulo [`TUNNEL_REPEAT`] and its fraction added before Offset is, and the sum wrapped again
/// after, where the flight comes back — so a gear's Time at 64 draws the frame at zero to the
/// bit, and a long flight keeps the precision of a short one.
fn tunnel_flight_wgsl(wrap: &str) -> &'static str {
    if wrap == "none" {
        "    let camZ = {clock}.x + {clock}.y + {phaseOffset};\n"
    } else {
        "    let flight = whole_mod({clock}.x, 64.0) + {clock}.y + {phaseOffset};\n    let camZ = flight - 64.0 * floor(flight / 64.0);\n"
    }
}

/// The one body a distortion that only moves the coordinate needs.
const PASS_THROUGH: &str = "    return {input};";

// -------------------------------------------------------------------------------------- wave

/// The wave's frame: the center, the rotation, and the coordinate measured in it.
const WAVE_SETUP_WGSL: &str = "    let amplitude = {amplitude};
    let frequency = {frequency};
    let phase = ({phase}) * 2.0 * PI;
    let rotation = ({rotation}) * 2.0 * PI;
    let center = vec2f({centerX}, {centerY});
    let cuv = uv - center;
    let cs = cos(rotation);
    let sn = sin(rotation);
    let rotUV = vec2f(cuv.x * cs + cuv.y * sn, -cuv.x * sn + cuv.y * cs);
";

/// Where along the wave this fragment sits: across the rotated frame, or out from the center.
fn wave_position_wgsl(mode: &str) -> &'static str {
    if mode == "radial" {
        "    let wavePos = length(cuv) * frequency + phase;\n"
    } else {
        "    let wavePos = rotUV.y * frequency + phase;\n"
    }
}

/// The waveform, over -1 to 1.
fn waveform_wgsl(shape: &str) -> &'static str {
    match shape {
        "triangle" => "    let wave = 2.0 * abs(floor_mod(wavePos / PI, 2.0) - 1.0) - 1.0;\n",
        "square" => "    let wave = sign(sin(wavePos));\n",
        "sawtooth" => "    let wave = 2.0 * fract(wavePos / (2.0 * PI)) - 1.0;\n",
        "noise" => "    let wave = fract(sin(wavePos * 12.9898) * 43758.5453) * 2.0 - 1.0;\n",
        _ => "    let wave = sin(wavePos);\n",
    }
}

/// Which way the wave pushes: across its own direction, or along it.
fn wave_displacement_wgsl(mode: &str, displacement: &str) -> &'static str {
    match (mode, displacement) {
        ("radial", "ripple") => "    let radialDir = select(vec2f(0.0), normalize(cuv), dot(cuv, cuv) > 0.0);
    let displacedUV = uv + radialDir * wave * amplitude;
",
        ("radial", _) => "    let radialDir = select(vec2f(0.0), normalize(cuv), dot(cuv, cuv) > 0.0);
    let displacedUV = uv + vec2f(-radialDir.y, radialDir.x) * wave * amplitude;
",
        (_, "ripple") => "    let displaced = vec2f(rotUV.x, rotUV.y + wave * amplitude);
    let displacedUV = vec2f(displaced.x * cs - displaced.y * sn, displaced.x * sn + displaced.y * cs) + center;
",
        _ => "    let displaced = vec2f(rotUV.x + wave * amplitude, rotUV.y);
    let displacedUV = vec2f(displaced.x * cs - displaced.y * sn, displaced.x * sn + displaced.y * cs) + center;
",
    }
}

node! {
    /// A periodic displacement of the coordinate, along or across the wave's own direction.
    WAVE,
    slug: "wave",
    icon: "〰",
    label: "Wave",
    category: Transform,
    tooltip: "Displaces the input along a wave. Rotation turns the wave fronts, amplitude is \
              how far it pushes, frequency how many waves across the frame; Wiggle pushes \
              across the wave and Ripple along it.",
    inputs: [
        VaryingColor "input" "Input" at "displacedUV" = Control::None,
        VaryingNumber "amplitude" "Amplitude" = Control::num(0.1, 0.0, 1.0, 0.001, "⬓"),
        VaryingNumber "frequency" "Frequency" = Control::num_log(10.0, 0.1, 100.0, 0.1, "/⬓"),
        VaryingNumber "phase" "Phase" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, -1.0, 1.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "waveform" "Waveform" = "sine" [
            "sine" => "Sine",
            "triangle" => "Triangle",
            "square" => "Square",
            "sawtooth" => "Sawtooth",
            "noise" => "Noise",
        ],
        "mode" "Mode" = "linear" [
            "linear" => "Linear",
            "radial" => "Radial",
        ],
        "displacement" "Displacement" = "wiggle" [
            "wiggle" => "Wiggle",
            "ripple" => "Ripple",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        let mode = ctx.option(node, "mode");
        [
            WAVE_SETUP_WGSL,
            wave_position_wgsl(mode),
            waveform_wgsl(ctx.option(node, "waveform")),
            wave_displacement_wgsl(mode, ctx.option(node, "displacement")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = PASS_THROUGH,
    ],
}

// -------------------------------------------------------------------------- whirl and pinch

node! {
    /// A twist and a squeeze inside one disc, both fading to nothing at its edge.
    WHIRLANDPINCH,
    slug: "whirlandpinch",
    icon: "🌌",
    label: "Whirl & Pinch",
    category: Transform,
    tooltip: "Twists the input around a center and pulls it in or out, both fading to nothing \
              at the radius. Its mask is that falloff. Negative pinch is cool.",
    // "Pinch Amount" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "whirledUV" = Control::None,
        VaryingNumber "whirl" "Whirl Angle" = Control::num(0.25, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "pinch" "Pinch Amount" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
        VaryingNumber "radius" "Radius" = Control::num(0.5, 0.01, 1.5, 0.01, "⬓"),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // The falloff is zero at and beyond the radius, so outside the disc the angle and the
    // distance are unchanged and the polar round trip lands back on `uv`. That is what
    // replaces silvia's early return, which would sample the input at a second coordinate.
    wgsl_common: "    let center = vec2f({centerX}, {centerY});
    let cuv = uv - center;
    let dist = length(cuv);
    let radius = max({radius}, 1e-6);
    let mask = 1.0 - smoothstep(0.0, radius, dist);
",
    // The polar round trip belongs to the color alone, so the mask function neither reads
    // the whirl and pinch uniforms nor builds a coordinate nothing samples at.
    outputs: [
        VaryingColor "output" "Output" = "    let angle = atan2(cuv.y, cuv.x) + ({whirl}) * 2.0 * PI * mask;
    let pinched = dist + ({pinch}) * mask;
    let whirledUV = vec2f(cos(angle), sin(angle)) * pinched + center;
    return {input};",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

// ------------------------------------------------------------------------------ kaleidoscope

/// The polar frame every kaleidoscope style works in.
const KALEIDOSCOPE_SETUP_WGSL: &str = "    let segments = max({segments}, 2.0);
    let offset = {offset};
    let twist = {twist};
    var zoom = {zoom};
    if (abs(zoom) < 1e-6) { zoom = select(1e-6, -1e-6, zoom < 0.0); }
    let center = vec2f({centerX}, {centerY});
    let p = uv - center;
    let r = length(p);
    let a = atan2(p.y, p.x) + r * twist;
    let segmentAngle = 2.0 * PI / segments;
    let sourceIndex = floor(clamp({sourceSegment}, 1.0, segments) - 1.0);
";

/// Every other wedge mirrored, which is what a tube of mirrors does. The half-height is
/// silvia's, and it is what makes the classic look.
const KALEIDOSCOPE_CLASSIC_WGSL: &str = "    var scaled = r / zoom;
    var localAngle = floor_mod(a, segmentAngle);
    if (floor_mod(floor(a / segmentAngle), 2.0) == 1.0) { localAngle = segmentAngle - localAngle; }
    localAngle += sourceIndex * segmentAngle;
    scaled += offset;
    let sampleUV = vec2f(scaled * cos(localAngle), scaled * sin(localAngle) * 0.5) + center;
";

/// The same wedge repeated without mirroring: a fan rather than a flower.
const KALEIDOSCOPE_CONTINUOUS_WGSL: &str = "    let scaled = r / zoom + offset;
    let localAngle = floor_mod(a, segmentAngle) + sourceIndex * segmentAngle;
    let sampleUV = vec2f(scaled * cos(localAngle), scaled * sin(localAngle)) + center;
";

/// Polar coordinates unwrapped onto the axes: the angle across, the inverse radius down.
///
/// The two axes are `spiralU` and `spiralV` rather than `u` and `v`, since `u` is the uniform
/// struct every `u.u_…` reads.
const KALEIDOSCOPE_SPIRAL_WGSL: &str =
    "    let spiralU = fract((a + sourceIndex * segmentAngle) / (2.0 * PI) * segments);
    let spiralV = (1.0 / max(r, 0.001)) / zoom + offset;
    let sampleUV = vec2f(spiralU * 2.0 - 1.0, spiralV * 2.0 - 1.0) + center;
";

node! {
    /// The frame folded into wedges around a center.
    KALEIDOSCOPE,
    slug: "kaleidoscope",
    icon: "🌸",
    label: "Kaleidoscope",
    category: Transform,
    tooltip: "Folds the input into wedges around a center. Classic mirrors every other wedge, \
              Continuous repeats one without mirroring, and Spiral unwraps the wedge into a \
              tunnel. Source Segment picks which wedge is copied.",
    // "Source Segment" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "sampleUV" = Control::None,
        VaryingNumber "segments" "Segments" = Control::num(6.0, 2.0, 32.0, 1.0, ""),
        VaryingNumber "sourceSegment" "Source Segment"
            = Control::num_capped(1.0, 1.0, 6.0, 1.0, "", "segments"),
        VaryingNumber "offset" "Shift" = Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
        VaryingNumber "twist" "Twist" = Control::num(0.0, -5.0, 5.0, 0.01, ""),
        VaryingNumber "zoom" "Zoom" = Control::num_log(1.0, 0.1, 5.0, 0.01, "x"),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "style" "Style" = "classic" [
            "classic" => "Classic",
            "continuous" => "Continuous",
            "spiral" => "Spiral",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            KALEIDOSCOPE_SETUP_WGSL,
            match ctx.option(node, "style") {
                "continuous" => KALEIDOSCOPE_CONTINUOUS_WGSL,
                "spiral" => KALEIDOSCOPE_SPIRAL_WGSL,
                _ => KALEIDOSCOPE_CLASSIC_WGSL,
            },
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = PASS_THROUGH,
    ],
}

// -------------------------------------------------------------------------------------- tile

/// Which axis the slide staggers, and by which index. Either line steps `scaledUV` in
/// place, which the body declares a `var` for.
fn tile_slide(direction: &str) -> &'static str {
    if direction == "vertical" {
        "    scaledUV.y += ({slide}) * tileHeight * floor((scaledUV.x + halfWidth) / tileWidth);\n"
    } else {
        "    scaledUV.x += ({slide}) * tileWidth * floor((scaledUV.y + halfHeight) / tileHeight);\n"
    }
}

node! {
    /// The whole plane wrapped onto one rectangle of it, over and over.
    TILE,
    slug: "tile",
    icon: "🧱",
    label: "Tile",
    category: Transform,
    tooltip: "Wraps the coordinate onto one rectangle, so the input repeats across the frame. \
              Slide staggers every row or column, which is how a brick bond is made.",
    // "Slide Direction" beside "Horizontal (Rows)" needs 246 of the 200 a body has by
    // default, and a direction that reads "Horizont…" closed is the one setting that says
    // which way the bond runs.
    width: 250.0,
    inputs: [
        VaryingColor "input" "Input" at "tiledUV" = Control::None,
        VaryingNumber "width" "Width" = Control::num(2.0, 0.1, 10.0, 0.1, "⬓"),
        VaryingNumber "height" "Height" = Control::num(2.0, 0.1, 10.0, 0.1, "⬓"),
        VaryingNumber "span" "Span" = Control::num(1.0, 0.1, 5.0, 0.01, "x"),
        VaryingNumber "offsetX" "Offset X" = Control::num(0.0, -5.0, 5.0, 0.01, "⬓"),
        VaryingNumber "offsetY" "Offset Y" = Control::num(0.0, -5.0, 5.0, 0.01, "⬓"),
        VaryingNumber "slide" "Slide" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
    ],
    options: [
        "slideDirection" "Slide Direction" = "horizontal" [
            "horizontal" => "Horizontal (Rows)",
            "vertical" => "Vertical (Columns)",
        ],
    ],
    // A cable can drive either side to zero, where the modulo is undefined.
    wgsl_common: varying(|node, ctx| {
        [
            "    let tileWidth = max({width}, 1e-3);
    let tileHeight = max({height}, 1e-3);
    let halfWidth = tileWidth * 0.5;
    let halfHeight = tileHeight * 0.5;
    var scaledUV = uv * ({span}) + vec2f({offsetX}, {offsetY});
",
            tile_slide(ctx.option(node, "slideDirection")),
            "    let tiledUV = vec2f(
        floor_mod(scaledUV.x + halfWidth, tileWidth) - halfWidth,
        floor_mod(scaledUV.y + halfHeight, tileHeight) - halfHeight);
",
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = PASS_THROUGH,
    ],
}

// ---------------------------------------------------------------------------------- repeater

/// Where the corner of the grid sits, from the grid's own size.
fn repeater_origin_wgsl(alignment: &str) -> &'static str {
    match alignment {
        "top-left" => "    let gridOffset = vec2f(-1.0, 1.0 - totalHeight);\n",
        "top-right" => "    let gridOffset = vec2f(1.0 - totalWidth, 1.0 - totalHeight);\n",
        "bottom-left" => "    let gridOffset = vec2f(-1.0, -1.0);\n",
        "bottom-right" => "    let gridOffset = vec2f(1.0 - totalWidth, -1.0);\n",
        _ => "    let gridOffset = vec2f(-totalWidth, -totalHeight) * 0.5;\n",
    }
}

/// The grid, and whether this fragment lands on a copy of the source rectangle.
///
/// silvia asks that in four early returns; a product of `step`s answers the same question
/// with no branch and, being a float, is the mask as well.
const REPEATER_GRID_WGSL: &str = "    let sourceWidth = max({sourceWidth}, 1e-3);
    let sourceHeight = max({sourceHeight}, 1e-3);
    let rows = max(floor({rows}), 1.0);
    let columns = max(floor({columns}), 1.0);
    let spacing = max({spacing}, 0.0);
    let cellWidth = sourceWidth + spacing;
    let cellHeight = sourceHeight + spacing;
    let totalWidth = columns * cellWidth - spacing;
    let totalHeight = rows * cellHeight - spacing;
";

const REPEATER_CELL_WGSL: &str = "    let gridUV = uv - gridOffset;
    let cell = floor(gridUV / vec2f(cellWidth, cellHeight));
    let cellUV = floor_mod2(gridUV, vec2f(cellWidth, cellHeight));
    let mask = step(0.0, gridUV.x) * step(gridUV.x, totalWidth)
        * step(0.0, gridUV.y) * step(gridUV.y, totalHeight)
        * step(cell.x, columns - 1.0) * step(cell.y, rows - 1.0)
        * step(cellUV.x, sourceWidth) * step(cellUV.y, sourceHeight);
";

node! {
    /// One rectangle of the input, laid out in a grid of copies.
    REPEATER,
    slug: "repeater",
    icon: "🪩",
    label: "Repeater",
    category: Transform,
    tooltip: "Copies one rectangle of the input into a grid of rows and columns, with spacing \
              between them and the background showing through. Grid Mask is where a copy is.",
    // "Source Height" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "sourceUV" = Control::None,
        VaryingColor "bgColor" "Background" = Control::color("#00000000"),
        VaryingNumber "sourceX" "Source X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "sourceY" "Source Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "sourceWidth" "Source Width" = Control::num(0.5, 0.01, 4.0, 0.01, "⬓"),
        VaryingNumber "sourceHeight" "Source Height" = Control::num(0.5, 0.01, 4.0, 0.01, "⬓"),
        VaryingNumber "rows" "Rows" = Control::num(3.0, 1.0, 20.0, 1.0, ""),
        VaryingNumber "columns" "Columns" = Control::num(3.0, 1.0, 20.0, 1.0, ""),
        VaryingNumber "spacing" "Spacing" = Control::num(0.0, 0.0, 1.0, 0.01, "⬓"),
    ],
    options: [
        "alignment" "Alignment" = "center" [
            "center" => "Center",
            "top-left" => "Top-Left",
            "top-right" => "Top-Right",
            "bottom-left" => "Bottom-Left",
            "bottom-right" => "Bottom-Right",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            REPEATER_GRID_WGSL,
            repeater_origin_wgsl(ctx.option(node, "alignment")),
            REPEATER_CELL_WGSL,
        ]
        .concat()
    }),
    // The source coordinate is declared in the color body alone, so the mask function
    // neither reads the source position's uniforms nor calls the input's producer.
    outputs: [
        VaryingColor "output" "Output" = "    let sourceUV = vec2f({sourceX}, {sourceY}) + cellUV
        - vec2f(sourceWidth, sourceHeight) * 0.5;
    return mix({bgColor}, {input}, mask);",
        VaryingNumber "mask" "Grid Mask" = "    return mask;",
    ],
}

// -------------------------------------------------------------------------------- domain warp

/// The two offsets each warp iteration reads its noise at, far enough apart to decorrelate.
const WARP_OFFSETS_WGSL: [(&str, &str); 3] = [
    ("vec2f(0.0, 0.0)", "vec2f(5.2, 1.3)"),
    ("vec2f(1.7, 9.2)", "vec2f(8.3, 2.8)"),
    ("vec2f(3.1, 7.4)", "vec2f(6.9, 4.6)"),
];

/// One warp iteration: the noise field read at the coordinate the last one displaced, by
/// `fbm`, the fractal sum [`fbm_time`] names.
fn warp_iteration_wgsl(fbm: &str, x_offset: &str, y_offset: &str, shape: &str) -> String {
    format!(
        "    warp = vec2f(
        {fbm}(uv * frequency + warp * amplitude + seedOffset + {x_offset}, frequency, octaves, lacunarity, gain, t, {shape}),
        {fbm}(uv * frequency + warp * amplitude + seedOffset + {y_offset}, frequency, octaves, lacunarity, gain, t, {shape}));
"
    )
}

/// The `fbmNoise` shape a Type names, and how many iterations the Iterations option asks for.
fn warp_options(ty: &str, iterations: &str) -> (&'static str, usize) {
    let shape = match ty {
        "turbulence" => "1",
        "ridged" => "2",
        _ => "0",
    };
    let count = match iterations {
        "2" => 2,
        "3" => 3,
        _ => 1,
    };
    (shape, count)
}

node! {
    /// The coordinate displaced by a noise field, fed back into itself.
    DOMAINWARP,
    slug: "domainwarp",
    icon: "🫠",
    label: "Domain Warp",
    category: Transform,
    tooltip: "Displaces the input by a fractal noise field. Each iteration reads the noise at \
              the coordinate the last one moved, which is what makes the distortion organic \
              rather than wavy. Value is how far the coordinate moved: the length of the \
              displacement, and the only field this node has beside its picture.",
    ambient: noise_ambient(0.0),
    inputs: [
        VaryingColor "input" "Input" at "warpedUV" = Control::None,
        UniformNumber "clock" "Time" = Control::None,
        VaryingNumber "phaseOffset" "Offset" = crate::nodes::phasor::offset_control(),
        VaryingNumber "amplitude" "Amplitude" = Control::num(0.5, 0.0, 2.0, 0.01, "⬓"),
        VaryingNumber "frequency" "Frequency" = Control::num_log(3.0, 0.1, 20.0, 0.1, "/⬓"),
        VaryingNumber "octaves" "Octaves" = Control::num(4.0, 1.0, 8.0, 1.0, ""),
        VaryingNumber "lacunarity" "Lacunarity" = Control::num(2.0, 1.0, 4.0, 0.1, "x"),
        VaryingNumber "gain" "Gain" = Control::num(0.5, 0.1, 1.0, 0.01, "x"),
        VaryingNumber "seed" "Seed" = Control::num(0.0, 0.0, 1000.0, 1.0, ""),
    ],
    options: [
        "type" "Type" = "standard" [
            "standard" => "Standard",
            "turbulence" => "Turbulence",
            "ridged" => "Ridged",
        ],
        "iterations" "Iterations" = "1" [
            "1" => "1",
            "2" => "2",
            "3" => "3",
        ],
        "repeat" "Repeat" = "never" [
            "never" => "Never",
            "1" => "Every 1",
            "2" => "Every 2",
            "4" => "Every 4",
            "8" => "Every 8",
            "16" => "Every 16",
        ],
    ],
    wgsl_utils: [SIMPLEX3D_WGSL, FBM_WGSL, SIMPLEX4D_WGSL, FBM_LOOP_WGSL, LOOP_CIRCLE_WGSL],
    wgsl_common: varying(|node, ctx| {
        let (shape, count) =
            warp_options(ctx.option(node, "type"), ctx.option(node, "iterations"));
        let (fbm, time) = fbm_time(ctx.option(node, "repeat"));
        let mut source = format!(
            "    let amplitude = {{amplitude}};
    let frequency = {{frequency}};
    let octaves = i32(clamp({{octaves}}, 1.0, 8.0));
    let lacunarity = {{lacunarity}};
    let gain = {{gain}};
    let t = {time};
    let seed = {{seed}};
    let seedOffset = vec2f(seed * 0.1317, seed * 0.0741);
    var warp = vec2f(0.0);
",
        );
        for (x_offset, y_offset) in &WARP_OFFSETS_WGSL[..count] {
            source.push_str(&warp_iteration_wgsl(fbm, x_offset, y_offset, shape));
        }
        source
    }),
    outputs: [
        VaryingColor "output" "Output" = "    let warpedUV = uv + warp * amplitude;
    return {input};",
        VaryingNumber "value" "Value" = "    return clamp(length(warp), 0.0, 1.0);",
    ],
}

// ----------------------------------------------------------------------------------- scatter

/// The neighborhood a fragment searches for copies that might cover it.
///
/// One cell is three samples per pixel and leaves a seam wherever a copy crosses a cell
/// boundary; the 3×3 ring is twenty-seven and has none.
fn scatter_neighborhood(overlap: &str) -> (&'static str, &'static str) {
    if overlap == "on" {
        ("-1", "1")
    } else {
        ("0", "0")
    }
}

/// The bounding-box reject, which is only worth its instruction count when the ring is nine
/// cells wide.
fn scatter_reject(overlap: &str) -> &'static str {
    if overlap == "on" {
        "            if (abs(uv.x - copyCenter.x) > halfExtent * 1.5
                || abs(uv.y - copyCenter.y) > halfExtent * 1.5) { continue; }
"
    } else {
        ""
    }
}

node! {
    /// Copies of the input strewn across three interleaved lattices.
    SCATTER,
    slug: "scatter",
    icon: "👣",
    label: "Scatter",
    category: Transform,
    tooltip: "Strews randomized copies of the input across the plane on three lattices at \
              once, so the spacing looks scattered rather than gridded. Each copy is jittered, \
              rotated, flipped and scaled by its own cell's hash.",
    inputs: [
        VaryingColor "input" "Input" at "scatterUV" = Control::None,
        VaryingColor "bgColor" "Background" = Control::color("#00000000"),
        VaryingNumber "density" "Density" = Control::num(5.0, 1.0, 50.0, 0.1, "/⬓"),
        VaryingNumber "presence" "Presence" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "jitter" "Jitter" = Control::num(0.3, 0.0, 1.0, 0.01, ""),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "scale" "Scale" = Control::num(0.7, 0.01, 2.0, 0.01, "x"),
        VaryingNumber "scaleVariation" "Scale Var" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "flipChance" "Flip" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "seed" "Seed" = Control::num(0.0, 0.0, 1000.0, 1.0, ""),
    ],
    options: [
        "overlap" "Overlap" = "off" [
            "off" => "Off (3 samples)",
            "on" => "On (27 samples)",
        ],
    ],
    wgsl_utils: [HASH_RANDOM_WGSL],
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let overlap = ctx.option(node, "overlap");
            let (from, to) = scatter_neighborhood(overlap);
            format!(
                "    let baseSize = 2.0 / max({{density}}, 1.0);
    let seed = {{seed}};
    let seedOffset = vec2f(seed * 0.1317, seed * 0.0741);
    let presence = {{presence}};
    let jitter = {{jitter}};
    let rotation = {{rotation}};
    let scale = {{scale}};
    let scaleVariation = {{scaleVariation}};
    let flipChance = {{flipChance}};
    var result = {{bgColor}};
    var layerScale = array<f32, 3>(4.0, 2.0, 1.0);
    var layerChance = array<f32, 3>(0.12, 0.20, 0.35);
    var layerOffset = array<f32, 3>(0.0, 10000.0, 20000.0);

    for (var layer = 0; layer < 3; layer++) {{
        let cellSize = baseSize * layerScale[layer];
        let baseCell = floor(uv / cellSize);
        for (var dy = {from}; dy <= {to}; dy++) {{
        for (var dx = {from}; dx <= {to}; dx++) {{
            let neighbor = vec2f(f32(dx), f32(dy));
            let key = baseCell + neighbor + layerOffset[layer] + seedOffset;
            let h1 = hashRandom(key);
            let h2 = hashRandom(key + 1.0);
            let h3 = hashRandom(key + 2.0);
            let h4 = hashRandom(key + 3.0);
            let h5 = hashRandom(key + 4.0);
            let h6 = hashRandom(key + 5.0);
            if (h6 > presence * layerChance[layer]) {{ continue; }}

            let copyCenter = (baseCell + neighbor + 0.5) * cellSize
                + (vec2f(h1, h2) - 0.5) * cellSize * jitter;
            let halfExtent = 0.2 * max(scale * (1.0 - scaleVariation + scaleVariation * h5 * 2.0), 0.01);
{reject}
            let angle = (h3 - 0.5) * rotation * 2.0 * PI;
            let cv = cos(-angle);
            let sv = sin(-angle);
            var scatterUV = mat2x2f(cv, -sv, sv, cv) * (uv - copyCenter) / halfExtent;
            if (h4 < flipChance) {{ scatterUV.x = -scatterUV.x; }}
            if (abs(scatterUV.x) > 1.0 || abs(scatterUV.y) > 1.0) {{ continue; }}

            let sampled = {{input}};
            result = mix(result, sampled, sampled.a);
        }}
        }}
    }}
    return result;",
                reject = scatter_reject(overlap),
            )
        }),
    ],
}

// ------------------------------------------------------------------------------------ tunnel

/// The center-line function this tunnel's path option names.
fn tunnel_path(path: &str) -> &'static str {
    match path {
        "helix" => "tunnelPathHelix",
        "lissajous" => "tunnelPathLissajous",
        _ => "tunnelPathSine",
    }
}

/// How the depth coordinate behaves past the end of the texture.
fn tunnel_wrap_wgsl(wrap: &str) -> &'static str {
    match wrap {
        "repeat" => "    depth = fract(depth * 0.5 + 0.5) * 2.0 - 1.0;\n",
        "none" => "",
        _ => "    depth = abs(floor_mod(depth + 1.0, 4.0) - 2.0) - 1.0;\n",
    }
}

/// Where on the input a point of the tube wall reads from.
fn tunnel_mapping_wgsl(mapping: &str) -> &'static str {
    match mapping {
        "polar" => {
            "    let tunnelUV = vec2f(sin(wallAngle), cos(wallAngle)) * ((depth + 1.0) * 0.5);\n"
        }
        "cartesian_mirror" => "    let tunnelUV = vec2f(abs(wallAngle / PI) * 2.0 - 1.0, depth);\n",
        _ => "    let tunnelUV = vec2f(wallAngle / PI, depth);\n",
    }
}

/// Distance fog, which is what makes the tube read as receding. A WGSL swizzle cannot be
/// assigned to, so the fog rebuilds the whole color.
fn tunnel_shading_wgsl(shading: &str) -> &'static str {
    match shading {
        "light" => "    wall = vec4f(wall.rgb * exp(-marched * 0.08), wall.a);\n",
        "heavy" => "    wall = vec4f(wall.rgb * exp(-marched * 0.2), wall.a);\n",
        _ => "",
    }
}

node! {
    /// A curving tube, ray-marched, with the input wrapped around its wall.
    TUNNEL3D,
    slug: "tunnel3d",
    icon: "🚇",
    label: "Tunnel",
    category: Transform,
    tooltip: "Flies a camera down a curving tube with the input wrapped around its inside, \
              half a unit a second, or as a gear cabled into Time flies it; Offset moves it \
              along. The flight comes back every 64 units while the depth wraps. Twist is how \
              far the tube wanders and zoom is the lens.",
    ambient: Ambient {
        rate: 0.5,
        period: tunnel_period,
    },
    inputs: [
        VaryingColor "input" "Texture" at "tunnelUV" = Control::None,
        UniformNumber "clock" "Time" = Control::None,
        VaryingNumber "phaseOffset" "Offset" = crate::nodes::phasor::offset_control(),
        VaryingNumber "twist" "Twist" = Control::num(1.5, 0.0, 4.0, 0.01, ""),
        VaryingNumber "radius" "Radius" = Control::num(1.0, 0.3, 3.0, 0.01, "⬓"),
        VaryingNumber "zoom" "Zoom" = Control::num(1.5, 0.3, 4.0, 0.01, "x"),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "path" "Twist Path" = "sine" [
            "sine" => "Sine",
            "helix" => "Helix",
            "lissajous" => "Lissajous",
        ],
        "mapping" "Texture Mapping" = "cartesian" [
            "cartesian" => "Cartesian",
            "cartesian_mirror" => "Cart. Mirror",
            "polar" => "Polar",
        ],
        "wrap" "Depth Wrap" = "mirror" [
            "mirror" => "Mirror",
            "repeat" => "Repeat",
            "none" => "None",
        ],
        "shading" "Shading" = "none" [
            "none" => "None",
            "light" => "Light",
            "heavy" => "Heavy",
        ],
    ],
    wgsl_utils: [TUNNEL_PATH_WGSL],
    // Sphere tracing along the ray, thirty-two steps at seven tenths of the distance to the
    // wall. The camera looks half a unit further down its own path, so the tube bends toward
    // the viewer rather than sliding sideways.
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let path = tunnel_path(ctx.option(node, "path"));
            format!(
                "    let p = uv - vec2f({{centerX}}, {{centerY}});
    let twist = {{twist}};
    let tubeRadius = max({{radius}}, 1e-3);
{flight}    let origin = vec3f({path}(camZ, twist), camZ);
    let lookZ = camZ + 0.5;
    let lookAt = vec3f({path}(lookZ, twist), lookZ);
    let forward = normalize(lookAt - origin);
    let right = normalize(cross(vec3f(0.0, 1.0, 0.0), forward));
    let up = cross(forward, right);
    let ray = normalize(p.x * right + p.y * up + ({{zoom}}) * forward);

    var marched = 0.0;
    for (var i = 0; i < 32; i++) {{
        let probe = origin + ray * marched;
        let toWall = tubeRadius - length(probe.xy - {path}(probe.z, twist));
        if (toWall < 0.001 || marched > 30.0) {{ break; }}
        marched += toWall * 0.7;
    }}

    let hit = origin + ray * marched;
    let axis = {path}(hit.z, twist);
    let wallAngle = atan2(hit.x - axis.x, hit.y - axis.y);
    var depth = hit.z * 0.5;
{wrap}{mapping}    var wall = {{input}};
{shading}    return wall;",
                flight = tunnel_flight_wgsl(ctx.option(node, "wrap")),
                wrap = tunnel_wrap_wgsl(ctx.option(node, "wrap")),
                mapping = tunnel_mapping_wgsl(ctx.option(node, "mapping")),
                shading = tunnel_shading_wgsl(ctx.option(node, "shading")),
            )
        }),
    ],
}
