// SPDX-License-Identifier: AGPL-3.0-or-later

//! Effects that read their one input at many coordinates and combine what they find.
//!
//! Ported from silvia's `kuwahara.js`, `motionblur.js`, `radialblur.js`, `sincfilter.js` and
//! `supersampling.js`. `convolve.rs` is the same category's other half — a kernel over a
//! square neighborhood — and the rule it states holds here too: **a loop's bound is an
//! option, never a knob.** silvia puts Kuwahara's Radius and the Sinc Filter's Kernel Size on
//! rows a cable can drive, so the cost climbs from nine samples to a hundred and sixty-nine
//! with nothing on screen saying so; here each is a select whose labels carry the count.
//!
//! **Every step is pixels of a 720-high frame**, as [nodes.md](../../../docs/nodes.md#worldspace)
//! requires. silvia divides by `u_resolution`, which makes a motion blur's streak lean on a
//! wide frame and a Kuwahara's window change shape with the output; `nodes::REFERENCE_HEIGHT`
//! on both axes is one field rather than one field per Output. Radial Blur is the exception
//! that needed no correction: its smear is a fraction of the ray from the center, which is
//! already in world units.
//!
//! **Kuwahara works on the color's own channels**, since a variance is nonlinear in them; the
//! four blurs are averages, which are exact on premultiplied colors as they are.
//!
//! Two departures beyond that. **Super Sampling's grid is centered.** silvia offsets a cell by
//! `(i - 0.5) / n`, which is centered only at 2x2 — at 3x3 and 4x4 its samples lean toward one
//! corner and the picture shifts as the option changes. `(i + 0.5) / n - 0.5` is the same grid
//! about the fragment at every count. And **the Sinc Filter's disc is branchless**: silvia's
//! `if (dist <= kSize)` around the accumulation is a `step` on the weight, which is the same
//! number with no divergence inside the inner loop.

use crate::compile::CompileContext;
use crate::graph::{NodeId, PortType::VaryingColor, PortType::VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

// ---------------------------------------------------------------------------------- kuwahara

/// The half-width of a Kuwahara window, and the samples one quadrant of it holds.
///
/// A quadrant is the axes plus one corner, so it is `(r + 1)²` of the `(2r + 1)²` the whole
/// window reads. The option's values name the window rather than its half-width, as `blur`'s
/// do, because a closed select shows the value it holds and `3` says nothing on a row labeled
/// Kernel.
fn kuwahara_window(size: &str) -> (i32, i32) {
    let r = match size {
        "3x3" => 1,
        "5x5" => 2,
        "9x9" => 4,
        "11x11" => 5,
        "13x13" => 6,
        _ => 3,
    };
    (r, (r + 1) * (r + 1))
}

node! {
    /// Painterly smoothing: the mean of whichever quadrant around a fragment is flattest.
    KUWAHARA,
    slug: "kuwahara",
    icon: "🎨",
    label: "Kuwahara",
    category: Effect,
    tooltip: "Takes the mean of whichever of the four quadrants around a fragment varies \
              least, so flat areas melt into blocks of color while edges stay crisp — a \
              picture that looks painted rather than soft. Kernel is the window: 3x3 is nine \
              samples of everything upstream, 13x13 is a hundred and sixty-nine. Its value is \
              how rough the quadrant it took was.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
    ],
    options: [
        "size" "Kernel" = "7x7" [
            "3x3" => "3x3 (9)",
            "5x5" => "5x5 (25)",
            "7x7" => "7x7 (49)",
            "9x9" => "9x9 (81)",
            "11x11" => "11x11 (121)",
            "13x13" => "13x13 (169)",
        ],
    ],
    // The four quadrants overlap along the axes, which is silvia's arrangement and the
    // classic one: the fragment itself is in all four, so every quadrant is (r + 1)² samples.
    // The means and variances are of the samples' own channels, and the mean taken is
    // premultiplied at the center's alpha.
    wgsl_common: varying(|node, ctx| {
        let (r, quadrant) = kuwahara_window(ctx.option(node, "size"));
        format!(
            "    let kernelStep = vec2f(1.0) / {REFERENCE_HEIGHT:?};
    let quadrant = {quadrant}.0;
    var mean = array<vec3f, 4>();
    var square = array<vec3f, 4>();
    var centerAlpha = 1.0;
    var offset = vec2f(0.0);
    for (var y = -{r}; y <= {r}; y++) {{
        for (var x = -{r}; x <= {r}; x++) {{
            offset = vec2f(f32(x), f32(y)) * kernelStep;
            let sampled = unpremultiply({{input}});
            if (x == 0 && y == 0) {{ centerAlpha = sampled.a; }}
            let c = sampled.rgb;
            if (x <= 0 && y <= 0) {{ mean[0] += c; square[0] += c * c; }}
            if (x >= 0 && y <= 0) {{ mean[1] += c; square[1] += c * c; }}
            if (x <= 0 && y >= 0) {{ mean[2] += c; square[2] += c * c; }}
            if (x >= 0 && y >= 0) {{ mean[3] += c; square[3] += c * c; }}
        }}
    }}
    var flattest = mean[0] / quadrant;
    var roughness = dot(square[0] / quadrant - flattest * flattest, vec3f(1.0));
    for (var q = 1; q < 4; q++) {{
        let m = mean[q] / quadrant;
        let v = dot(square[q] / quadrant - m * m, vec3f(1.0));
        if (v < roughness) {{ roughness = v; flattest = m; }}
    }}
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = "    return premultiply(vec4f(flattest, centerAlpha));",
        VaryingNumber "value" "Value" in "[0, 1]" = "    return clamp(roughness, 0.0, 1.0);",
    ],
}

// -------------------------------------------------------------------------------- motionblur

node! {
    /// Sixteen taps along a line, averaged: the smear a moving camera leaves.
    MOTIONBLUR,
    slug: "motionblur",
    icon: "💨",
    label: "Motion Blur",
    category: Effect,
    tooltip: "Averages sixteen samples spread evenly along a line through the fragment, so \
              the picture smears the way a moving camera leaves it. Amount is the length of \
              the line in pixels of a 720-high frame and Angle is where it points.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "amount" "Amount" = Control::num(10.0, 0.0, 100.0, 0.1, "px"),
        VaryingNumber "angle" "Angle" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
    ],
    outputs: [
        VaryingColor "output" "Output" = format!(
            "    let angle = ({{angle}}) * 2.0 * PI;
    let line = vec2f(cos(angle), sin(angle)) * ({{amount}}) / {REFERENCE_HEIGHT:?};
    var sum = vec4f(0.0);
    var offset = vec2f(0.0);
    for (var i = 0; i < 16; i++) {{
        offset = line * (f32(i) / 15.0 - 0.5);
        sum += {{input}};
    }}
    return sum / 16.0;"
        ),
    ],
}

// -------------------------------------------------------------------------------- radialblur

node! {
    /// Sixteen taps along the ray from a center point, averaged: a zoom smear, still.
    RADIALBLUR,
    slug: "radialblur",
    icon: "💫",
    label: "Radial Blur",
    category: Effect,
    tooltip: "Averages sixteen samples along the ray from the center through the fragment, so \
              the picture streaks outward from that point — a zoom smear that is still and \
              exact rather than trailing. Amount is the fraction of the ray the smear covers.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "amount" "Amount" = Control::num(0.1, 0.0, 1.0, 0.001, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // The ray is already in world units, so this is the one node here with nothing to divide
    // by the reference height: the smear is a fraction of the distance to the center.
    outputs: [
        VaryingColor "output" "Output" = "    let ray = (uv - vec2f({centerX}, {centerY})) * ({amount});
    var sum = vec4f(0.0);
    var offset = vec2f(0.0);
    for (var i = 0; i < 16; i++) {
        offset = ray * (f32(i) / 15.0 - 0.5);
        sum += {input};
    }
    return sum / 16.0;",
    ],
}

// -------------------------------------------------------------------------------- sincfilter

/// The half-width of a windowed-sinc kernel.
fn sinc_window(size: &str) -> i32 {
    match size {
        "3x3" => 1,
        "5x5" => 2,
        "9x9" => 4,
        _ => 3,
    }
}

node! {
    /// A resample weighed by a Hamming-windowed sinc, which is sharp without ringing.
    SINCFILTER,
    slug: "sincfilter",
    icon: "👾",
    label: "Sinc Filter",
    category: Effect,
    tooltip: "Weighs a disc of samples by a Hamming-windowed sinc, which resamples sharply \
              without the ringing a naive filter leaves. Cutoff and Sharpness shape the \
              curve; Kernel Size is how far it reaches — 3x3 is nine samples of everything \
              upstream, 9x9 is eighty-one.",
    // "Kernel Size" beside its widest choice does not fit the default 200.
    // See docs/decisions.md#a-node-may-declare-a-wider-body.
    width: 216.0,
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
        VaryingNumber "cutoff" "Cutoff" = Control::num(0.5, 0.1, 1.0, 0.01, ""),
        VaryingNumber "sharpness" "Sharpness" = Control::num(1.0, 0.1, 3.0, 0.01, ""),
    ],
    options: [
        "size" "Kernel Size" = "7x7" [
            "3x3" => "3x3 (9)",
            "5x5" => "5x5 (25)",
            "7x7" => "7x7 (49)",
            "9x9" => "9x9 (81)",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let r = sinc_window(ctx.option(node, "size"));
            format!(
                "    let cutoff = {{cutoff}};
    let sharpness = {{sharpness}};
    let kernelStep = vec2f(1.0) / {REFERENCE_HEIGHT:?};
    var sum = vec4f(0.0);
    var total = 0.0;
    var offset = vec2f(0.0);
    for (var y = -{r}; y <= {r}; y++) {{
        for (var x = -{r}; x <= {r}; x++) {{
            offset = vec2f(f32(x), f32(y)) * kernelStep;
            let dist = length(vec2f(f32(x), f32(y)));
            let t = PI * dist * cutoff * sharpness;
            let sinc = select(sin(t) / t, 1.0, dist == 0.0);
            let window = 0.54 + 0.46 * cos(PI * dist / {r}.0);
            let weight = sinc * window * step(dist, {r}.0);
            sum += ({{input}}) * weight;
            total += weight;
        }}
    }}
    return select(sum, sum / total, total > 0.0);"
            )
        }),
    ],
}

// ----------------------------------------------------------------------------- supersampling

/// The side of the sub-pixel grid a quality reads, and the samples it comes to.
fn supersampling_grid(quality: &str) -> (i32, i32) {
    let n = match quality {
        "2" => 2,
        "3" => 3,
        "4" => 4,
        _ => 1,
    };
    (n, n * n)
}

/// A grid of reads inside one pixel, averaged. The whole body, because the passthrough at 1x
/// and the loop above it are one option apart.
fn supersampling_body_wgsl(node: NodeId, ctx: &CompileContext<'_>) -> String {
    let (n, count) = supersampling_grid(ctx.option(node, "quality"));
    if n <= 1 {
        return "    let offset = vec2f(0.0);
    return {input};"
            .to_string();
    }
    format!(
        "    let subpixel = vec2f(1.0) / {REFERENCE_HEIGHT:?} / {n}.0;
    var sum = vec4f(0.0);
    var offset = vec2f(0.0);
    for (var y = 0; y < {n}; y++) {{
        for (var x = 0; x < {n}; x++) {{
            offset = (vec2f(f32(x), f32(y)) + 0.5 - {n}.0 * 0.5) * subpixel;
            sum += {{input}};
        }}
    }}
    return sum / {count}.0;"
    )
}

node! {
    /// Several reads inside one pixel, averaged, so a hard edge stops crawling.
    SUPERSAMPLING,
    slug: "supersampling",
    icon: "✨",
    label: "Super Sampling",
    category: Effect,
    tooltip: "Reads everything upstream several times inside each pixel and averages them, so \
              the stair steps on a zoomed-out fractal or a tight tiling settle down and stop \
              crawling. Quality is the grid and the sample count with it; at 1x the node is a \
              passthrough and costs nothing.",
    inputs: [
        VaryingColor "input" "Input" at "uv + offset" = Control::None,
    ],
    options: [
        "quality" "Quality" = "1" [
            "1" => "1x (Off)",
            "2" => "2x2 (4)",
            "3" => "3x3 (9)",
            "4" => "4x4 (16)",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(supersampling_body_wgsl),
    ],
}
