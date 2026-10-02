// SPDX-License-Identifier: AGPL-3.0-or-later

//! A ramp driven past its ceiling and folded back, the way a wavefolder treats a waveform.
//!
//! Ported from silvia's `wavefold.js`. Drive steepens what arrives and the fold turns it
//! around rather than clipping it flat, so one gradient comes out as a stack of repeating
//! bands. It reads the one texel under it, so it is `Category::Color` and not `Effect`,
//! however unlike the rest of that group it looks.
//!
//! Two menus, both `Code`: four fold shapes and the choice of folding the three channels
//! apart or only the brightness, which between them are eight different pictures and eight
//! different bodies.
//!
//! silvia emits the fold as a function per node, `hd_fold`. Here the choice is inlined into
//! the body instead: a `wgsl_utils` entry is one global per shader, and two Wavefolds in
//! different modes in one Output would be two definitions of one name.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::graph::PortType::VaryingNumber;
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// The fold itself: statements over the `vec3f` `driven`, leaving the `vec3f` `folded` in the
/// -1 to 1 the half-and-half at the end expects.
fn fold_wgsl(mode: &str) -> &'static str {
    match mode {
        "sine" => "    let folded = sin(driven * PI);\n",
        "tanh" => "    let folded = tanh(driven);\n",
        "chebyshev" => {
            "    let driven2 = driven * driven;
    let folded = clamp(16.0 * driven2 * driven2 * driven - 20.0 * driven2 * driven
        + 5.0 * driven, vec3f(-1.0), vec3f(1.0));
"
        }
        _ => "    let folded = abs(floor_mod3(driven + 1.0, vec3f(4.0)) - 2.0) - 1.0;\n",
    }
}

/// What is driven into the fold, and what is made of what comes back.
///
/// Per channel the three fold apart, which throws the color around. Per luminance only the
/// brightness folds and the pixel is scaled by the ratio, which keeps the hue.
fn channel_wgsl(channel: &str) -> (&'static str, &'static str) {
    if channel == "luminance" {
        (
            "    let lum = dot(color.rgb, vec3f(0.299, 0.587, 0.114));
    let driven = vec3f((lum - 0.5 + ({bias})) * ({drive}));
",
            "    let foldedLum = folded.x * 0.5 + 0.5;
    let lumRatio = select(1.0, foldedLum / lum, lum > 0.001);
    let result = mix(color.rgb, color.rgb * lumRatio, {mix});
",
        )
    } else {
        (
            "    let driven = (color.rgb - 0.5 + ({bias})) * ({drive});\n",
            "    let result = mix(color.rgb, folded * 0.5 + 0.5, {mix});\n",
        )
    }
}

/// The whole body, for the mode and channel this node is set to.
fn body_wgsl(node: NodeId, ctx: &CompileContext<'_>) -> String {
    let (drive, result) = channel_wgsl(ctx.option(node, "channel"));
    [
        "    let color = {input};\n",
        drive,
        fold_wgsl(ctx.option(node, "mode")),
        result,
        "    return vec4f(clamp(result, vec3f(0.0), vec3f(1.0)), color.a);",
    ]
    .concat()
}

node! {
    /// A gradient driven past its ceiling and folded back into bands.
    WAVEFOLD,
    slug: "wavefold",
    icon: "🪭",
    label: "Wavefold",
    category: Color,
    tooltip: "Steepens what arrives and folds it back where it passes the ceiling, so one \
              gradient becomes repeating bands. Drive is how many folds, Bias moves where \
              they sit, and Mix fades back toward the original.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "drive" "Drive" = Control::num(2.0, 1.0, 20.0, 0.1, "x"),
        VaryingNumber "bias" "Bias" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
        VaryingNumber "mix" "Mix" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "mode" "Mode" = "triangle" [
            "triangle" => "Triangle Fold",
            "sine" => "Sine Fold",
            "tanh" => "Soft Clip (tanh)",
            "chebyshev" => "Chebyshev T5",
        ],
        "channel" "Channel" = "rgb" [
            "rgb" => "Per Channel",
            "luminance" => "Luminance",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(body_wgsl),
    ],
}
