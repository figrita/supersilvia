// SPDX-License-Identifier: AGPL-3.0-or-later

//! Neighborhood kernels: nodes that read the picture at more than one texel.
//!
//! Ported from silvia's `blur.js`, `sharpen.js`, `emboss.js`, `bloom.js`, `dilate.js`,
//! `erode.js` and `heighttonormal.js`. That is the test the whole `Effect` category is defined by, and it is what
//! splits silvia's one `Effects` heading of thirty-three in two: a color map, however
//! dramatic, is `Category::Color` and lives in `adjust.rs`; anything that has to sample a
//! neighbor is here. [decisions.md](../../../docs/decisions.md) has the argument.
//!
//! **A sampling radius is a `VaryingNumber`; a kernel size is an option.** A radius
//! scales an offset and changes no code, so it is a control a cable can drive. The
//! half-width of the loop is the loop's bound, and a bound is generated code — so it is an
//! option, which is a recompile boundary, and every loop here has a constant bound as a
//! result.
//!
//! **These are quadratic and say so.** One pass, no intermediate target, so a separable blur
//! is not available: a half-width of `r` is `(2r+1)²` samples of everything upstream. The
//! tooltips carry the sample counts for that reason.
//!
//! Four departures from the source. **One sample per iteration.** silvia writes the center
//! sample as a second call at `uv` beside the loop's call at `uv + offset`; here the center
//! is the iteration where the offset is zero, so an input is read at exactly one coordinate
//! and the macro's `at` carries it. **Branchless.** silvia's `if (brightness > threshold)`
//! inside a bloom's inner loop, and its early return in `dilate` and `erode`, become a `step`
//! and a `mix`. And **`sharpen`'s three methods are one combine**: silvia's Simple has no
//! threshold and a hand-written cross kernel, which is the unsharp mask with a flat
//! four-neighbor blur and a gain of four, so all three share the difference, the threshold
//! and the add-back. And **`heighttonormal` steps a pixel of the reference frame** where
//! silvia steps a texel of the real output, so the map it writes is the same map whatever
//! size the Output is — the rule every other kernel here already follows.

use crate::compile::CompileContext;
use crate::graph::{NodeId, PortType::VaryingColor, PortType::VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

/// Rec.601 luma, which is what all six of these weigh brightness by.
const LUMA_WGSL: &str = "vec3f(0.299, 0.587, 0.114)";

// -------------------------------------------------------------------------------------- blur

/// The half-width of a box blur's kernel, and the sample count it comes to.
///
/// The option's values are the kernel's own name rather than its half-width, because a closed
/// select on the node shows the value it holds and `1` says nothing on a row labeled Kernel.
fn box_kernel(size: &str) -> (i32, i32) {
    let r = match size {
        "5x5" => 2,
        "7x7" => 3,
        _ => 1,
    };
    (r, (2 * r + 1) * (2 * r + 1))
}

node! {
    /// A box blur, its two axes separately scaled.
    BLUR,
    slug: "blur",
    icon: "🕶",
    label: "Blur",
    category: Effect,
    tooltip: "A box blur in pixels, with X and Y scaled apart so it can smear in one \
              direction. Kernel is the half-width: 3x3 is nine samples of everything \
              upstream, 5x5 is twenty-five, 7x7 is forty-nine.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "blurX" "Blur X" = Control::num(1.0, 0.0, 50.0, 0.1, "px"),
        VaryingNumber "blurY" "Blur Y" = Control::num(1.0, 0.0, 50.0, 0.1, "px"),
    ],
    options: [
        "size" "Kernel" = "3x3" [
            "3x3" => "3x3 (9)",
            "5x5" => "5x5 (25)",
            "7x7" => "7x7 (49)",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let (r, count) = box_kernel(ctx.option(node, "size"));
            format!(
                "    let blurStep = vec2f({{blurX}}, {{blurY}}) / {REFERENCE_HEIGHT:?};
    var sum = vec4f(0.0);
    for (var y = -{r}; y <= {r}; y++) {{
        for (var x = -{r}; x <= {r}; x++) {{
            let offset = vec2f(f32(x), f32(y)) * blurStep;
            sum += {{input}};
        }}
    }}
    return sum / {count}.0;"
            )
        }),
    ],
}

// ----------------------------------------------------------------------------------- sharpen

/// The kernel one sharpening method reads: its half-width, how far a step reaches, and the
/// gain its difference is added back with.
///
/// Simple is a flat four-neighbor cross, which makes its difference the same quantity the
/// other two compute with a Gaussian. High Pass scales the whole neighborhood by the radius
/// where Unsharp Mask keeps its steps one texel apart and widens the Gaussian instead.
fn sharpen_kernel(method: &str) -> (i32, &'static str, &'static str) {
    match method {
        "simple" => (1, "1.0", "amount * 4.0"),
        "highpass" => (2, "radius", "amount"),
        _ => (3, "1.0", "amount"),
    }
}

/// The line that declares the weight of the sample at `(x, y)` in one sharpening method's
/// kernel.
fn sharpen_weight_wgsl(method: &str) -> &'static str {
    match method {
        "simple" => "            let weight = select(0.0, 1.0, abs(f32(x)) + abs(f32(y)) == 1.0);",
        "highpass" => "            let weight = exp(-d2 / (2.0 * radius * radius));",
        _ => "            let weight = select(exp(-d2 / (2.0 * radius * radius)), 0.0, d2 > 9.0);",
    }
}

/// The neighborhood gathered: the center, its blurred neighborhood, and how far apart they
/// are. Shared by the picture and by the detail field beside it.
fn sharpen_common_wgsl(node: NodeId, ctx: &CompileContext<'_>) -> String {
    let method = ctx.option(node, "method");
    let (r, reach, _) = sharpen_kernel(method);
    let weight = sharpen_weight_wgsl(method);
    format!(
        "    let amount = {{amount}};
    let radius = max({{radius}}, 1e-3);
    let texel = vec2f({reach}) / {REFERENCE_HEIGHT:?};
    var center = vec4f(0.0);
    var blurred = vec4f(0.0);
    var total = 0.0;
    for (var y = -{r}; y <= {r}; y++) {{
        for (var x = -{r}; x <= {r}; x++) {{
            let offset = vec2f(f32(x), f32(y)) * texel;
            let sampled = {{input}};
            if (x == 0 && y == 0) {{ center = sampled; }}
            let d2 = f32(x * x + y * y);
{weight}
            blurred += sampled * weight;
            total += weight;
        }}
    }}
    blurred /= max(total, 1e-6);
    let detail = center.rgb - blurred.rgb;
    let edge = length(detail);
"
    )
}

node! {
    /// Detail added back to the picture: the difference between a texel and its neighborhood.
    SHARPEN,
    slug: "sharpen",
    icon: "🪒",
    label: "Sharpen",
    category: Effect,
    tooltip: "Adds a texel's difference from its neighborhood back onto it. Simple is a \
              nine-sample cross, Unsharp Mask a forty-nine-sample Gaussian whose radius is \
              its width, High Pass a twenty-five-sample one whose radius is its reach. \
              Threshold leaves flat areas alone; its value is the detail it found.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "amount" "Amount" = Control::num(1.0, 0.0, 10.0, 0.1, "x"),
        VaryingNumber "radius" "Radius" = Control::num(1.0, 0.5, 5.0, 0.1, "px"),
        VaryingNumber "threshold" "Threshold" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "method" "Method" = "unsharp" [
            "simple" => "Simple",
            "unsharp" => "Unsharp Mask",
            "highpass" => "High Pass",
        ],
    ],
    wgsl_common: varying(sharpen_common_wgsl),
    outputs: [
        VaryingColor "color" "Color" = varying(|node, ctx| {
            let (_, _, gain) = sharpen_kernel(ctx.option(node, "method"));
            format!(
                "    let threshold = {{threshold}};
    let keep = smoothstep(threshold, threshold + 0.01, edge);
    return vec4f(clamp(center.rgb + detail * ({gain}) * keep, vec3f(0.0), vec3f(1.0)), center.a);"
            )
        }),
        VaryingNumber "value" "Value" = "    return clamp(edge, 0.0, 1.0);",
    ],
}

// ------------------------------------------------------------------------------------ emboss

node! {
    /// A relief lit from one direction: the slope of the luminance along it.
    EMBOSS,
    slug: "emboss",
    icon: "⛰",
    label: "Emboss",
    category: Effect,
    tooltip: "Reads the picture as a height field and lights it from an angle, so edges \
              across the light stand up and the flat parts go to the offset gray. Deboss \
              lights it from the other side.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "strength" "Strength" = Control::num(1.0, 0.0, 5.0, 0.1, "x"),
        VaryingNumber "angle" "Light Angle" = Control::num(0.125, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "offset" "Gray" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "mode" "Mode" = "emboss" [
            "emboss" => "Emboss",
            "deboss" => "Deboss",
        ],
    ],
    // Three samples along the light's own direction, the middle one at the fragment. The
    // relief is the picture here, so there is no field to publish beside it.
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let sign = if ctx.option(node, "mode") == "deboss" {
                "-"
            } else {
                ""
            };
            format!(
                "    let angle = ({{angle}}) * 2.0 * PI;
    let lightDir = vec2f(cos(angle), sin(angle)) * 2.0 / {REFERENCE_HEIGHT:?};
    var lum: array<f32, 3>;
    var alpha = 1.0;
    for (var k = -1; k <= 1; k++) {{
        let offset = lightDir * f32(k);
        let sampled = {{input}};
        if (k == 0) {{ alpha = sampled.a; }}
        lum[k + 1] = dot(sampled.rgb, {LUMA_WGSL});
    }}
    let relief = {sign}(lum[2] - lum[0]) * ({{strength}});
    return vec4f(vec3f(clamp(relief + ({{offset}}), 0.0, 1.0)), alpha);"
            )
        }),
    ],
}

// ------------------------------------------------------------------------------------- bloom

/// How many directions and how many rings a bloom gathers over, and the product.
fn bloom_rings(quality: &str) -> (i32, i32, i32) {
    match quality {
        "low" => (8, 2, 16),
        "high" => (16, 4, 64),
        _ => (8, 3, 24),
    }
}

node! {
    /// The bright parts of a picture smeared outward and added back.
    BLOOM,
    slug: "bloom",
    icon: "🌟",
    label: "Bloom",
    category: Effect,
    tooltip: "Gathers everything brighter than the threshold from a disc around each fragment \
              and adds it back, so highlights glow. Quality is the sample count — 16, 24 or \
              64 reads of everything upstream. Its value is how much glow landed here.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "threshold" "Threshold" = Control::num(0.7, 0.0, 1.0, 0.01, ""),
        VaryingNumber "intensity" "Intensity" = Control::num(1.0, 0.0, 3.0, 0.01, "x"),
        VaryingNumber "radius" "Radius" = Control::num(0.01, 0.001, 0.5, 0.001, "⬓"),
    ],
    options: [
        "quality" "Quality" = "medium" [
            "low" => "Low (16)",
            "medium" => "Medium (24)",
            "high" => "High (64)",
        ],
    ],
    // The mean is over the samples that cleared the threshold, as silvia's is, so one bright
    // neighbor glows as brightly as a field of them. `step` counts them without a branch.
    wgsl_common: varying(|node, ctx| {
        let (arms, rings, _) = bloom_rings(ctx.option(node, "quality"));
        format!(
            "    let threshold = {{threshold}};
    let radius = max({{radius}}, 1e-4);
    var glow = vec4f(0.0);
    var lit = 0.0;
    var offset = vec2f(0.0);
    for (var i = 0; i < {arms}; i++) {{
        let a = f32(i) * 2.0 * PI / {arms}.0;
        let dir = vec2f(cos(a), sin(a));
        for (var j = 1; j <= {rings}; j++) {{
            offset = dir * radius * f32(j) / {rings}.0;
            let sampled = {{input}};
            let brightness = dot(sampled.rgb, {LUMA_WGSL});
            glow += sampled * max(brightness - threshold, 0.0);
            lit += step(threshold, brightness);
        }}
    }}
    glow /= max(lit, 1.0);
    glow *= ({{intensity}});
    offset = vec2f(0.0);
    let original = {{input}};
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return original + glow;",
        VaryingNumber "value" "Value" = "    return clamp(dot(glow.rgb, vec3f(0.299, 0.587, 0.114)), 0.0, 1.0);",
    ],
}

// --------------------------------------------------------------------- dilate and erode

/// The cells a morphology skips: none for a box, the diagonals for a cross.
fn morphology_skip_wgsl(shape: &str) -> &'static str {
    if shape == "box" {
        ""
    } else {
        "            if (abs(f32(x)) + abs(f32(y)) > 1.0) { continue; }\n"
    }
}

/// One morphology's neighborhood: the center, and whichever neighbor `pick` says wins.
fn morphology_wgsl(shape: &str, pick: &str) -> String {
    let skip = morphology_skip_wgsl(shape);
    format!(
        "    let radius = {{radius}};
    var offset = vec2f(0.0);
    let original = {{input}};
    var winner = original;
    var best = dot(original.rgb, {LUMA_WGSL});
    for (var y = -1; y <= 1; y++) {{
        for (var x = -1; x <= 1; x++) {{
{skip}            offset = vec2f(f32(x), f32(y)) * radius;
            let sampled = {{input}};
            let brightness = dot(sampled.rgb, {LUMA_WGSL});
            if (brightness {pick} best) {{ best = brightness; winner = sampled; }}
        }}
    }}
"
    )
}

/// The gate the picture is mixed through: how far the winner moved the brightness, against
/// the threshold. It belongs to the color alone, so the field beside it does not compute it.
fn morphology_gate_wgsl(order: &str) -> String {
    format!(
        "    let moved = {order};
    let take = step({{threshold}}, moved) * clamp({{intensity}}, 0.0, 1.0);
    return mix(original, winner, take);"
    )
}

/// The ports every morphology carries, since dilate and erode differ only in a comparison.
macro_rules! morphology_node {
    ($name:ident, $slug:literal, $icon:literal, $label:literal, $tip:literal,
     $pick:literal, $order:literal) => {
        node! {
            $name,
            slug: $slug,
            icon: $icon,
            label: $label,
            category: Effect,
            tooltip: $tip,
            inputs: [
                VaryingColor "input" "Input" at "uv + offset" = Control::None,
                VaryingNumber "radius" "Radius" = Control::num(0.005, 0.001, 0.2, 0.001, "⬓"),
                VaryingNumber "threshold" "Threshold" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
                VaryingNumber "intensity" "Intensity" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
            ],
            options: [
                "shape" "Shape" = "cross" [
                    "cross" => "Cross (5)",
                    "box" => "Box (9)",
                ],
            ],
            wgsl_common: varying(|node, ctx| morphology_wgsl(ctx.option(node, "shape"), $pick)),
            outputs: [
                VaryingColor "color" "Color" = morphology_gate_wgsl($order),
                VaryingNumber "value" "Value" = "    return clamp(best, 0.0, 1.0);",
            ],
        }
    };
}

morphology_node!(
    DILATE,
    "dilate",
    "⊕",
    "Dilate",
    "Spreads the brightest texel of each neighborhood over it, so bright areas grow and thin \
     dark lines close up. Threshold is how much brighter the neighbor has to be before it \
     wins. Its value is the brightness that won.",
    ">",
    "best - dot(original.rgb, vec3f(0.299, 0.587, 0.114))"
);

morphology_node!(
    ERODE,
    "erode",
    "⊖",
    "Erode",
    "Spreads the darkest texel of each neighborhood over it, so bright areas shrink and thin \
     bright lines break up. Threshold is how much darker the neighbor has to be before it \
     wins. Its value is the brightness that won.",
    "<",
    "dot(original.rgb, vec3f(0.299, 0.587, 0.114)) - best"
);

// -------------------------------------------------------------------------- heighttonormal

/// The nine luminances of the neighborhood, the Sobel slope across them, and the direction
/// the surface faces. Shared by the map and by the height beside it.
///
/// A function rather than a literal because the step is in pixels of the reference frame,
/// which is a number this file interpolates rather than spells.
fn height_to_normal_common_wgsl() -> String {
    format!(
        "    let texel = vec2f(1.0) / {REFERENCE_HEIGHT:?};
    var h: array<f32, 9>;
    var offset = vec2f(0.0);
    for (var y = -1; y <= 1; y++) {{
        for (var x = -1; x <= 1; x++) {{
            offset = vec2f(f32(x), f32(y)) * texel;
            h[(y + 1) * 3 + (x + 1)] = dot(({{input}}).rgb, {LUMA_WGSL});
        }}
    }}
    let strength = {{strength}};
    let dx = (h[8] + 2.0 * h[5] + h[2]) - (h[6] + 2.0 * h[3] + h[0]);
    let dy = (h[0] + 2.0 * h[1] + h[2]) - (h[6] + 2.0 * h[7] + h[8]);
    let normal = normalize(vec3f(-dx * strength, dy * strength, 1.0));
"
    )
}

node! {
    /// The direction a height field faces at every point, written out as a color.
    HEIGHTTONORMAL,
    slug: "heighttonormal",
    icon: "🌄",
    label: "Height to Normal",
    category: Effect,
    tooltip: "Reads the picture's brightness as a height field and hands back the direction \
              the surface faces at every point, packed the way a normal map packs it. \
              Strength is how steep it treats the slopes. Its value is the height it read.",
    inputs: [
        VaryingColor "input" "Input (Height)" at "uv + offset" = Control::None,
        VaryingNumber "strength" "Strength" = Control::num(2.0, 0.0, 10.0, 0.1, "x"),
    ],
    wgsl_common: height_to_normal_common_wgsl(),
    // The map is opaque, as silvia's is: a direction has no coverage, and the alpha of the
    // height that made it means nothing about the surface.
    outputs: [
        VaryingColor "color" "Normal Map" = "    return vec4f(normal * 0.5 + 0.5, 1.0);",
        VaryingNumber "value" "Height" in "[0, 1]" = "    return clamp(h[4], 0.0, 1.0);",
    ],
}
