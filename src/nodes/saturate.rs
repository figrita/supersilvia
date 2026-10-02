// SPDX-License-Identifier: AGPL-3.0-or-later

//! The saturation pair: the blunt knob and the gentle one.
//!
//! Ported from silvia's `saturate.js` and `vibrance.js`, which sit under its `Effects`
//! heading. Both read one texel, turn it into hue, saturation and value, move the middle
//! number and turn it back, so both are `Category::Color` beside `adjust.rs`.
//!
//! They are one file because they are one shape and share one helper. `RGB2HSV_WGSL` and
//! `HSV2RGB_WGSL` are silvia's own, `pub` here the way `hsla`'s `HSL2RGB_WGSL` is, so the
//! next node that wants a hexcone reaches for these rather than inlining a second copy. The
//! prelude carries the same hue-to-rgb arithmetic under `hsv2rgb2`, for the fallback picture
//! an unconnected color input draws; that one is the prelude's and is emitted into every
//! shader, where a `wgsl_utils` entry is emitted only into a shader that reaches a node
//! asking for it.
//!
//! The two differ in one line. Saturate multiplies the saturation, so the vivid parts move
//! furthest; Vibrance adds a weighted amount, and the weight is `1 - s` going up and `s`
//! coming down, so the dull parts move furthest one way and the vivid parts the other.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

/// RGB to HSV, branchless. silvia's `shaderUtils.RGB2HSV`.
pub const RGB2HSV_WGSL: &str = "fn rgb2hsv(c: vec3f) -> vec3f {
    let K = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4f(c.bg, K.wz), vec4f(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}";

/// HSV to RGB, branchless. silvia's `shaderUtils.HSV2RGB`.
pub const HSV2RGB_WGSL: &str = "fn hsv2rgb(c: vec3f) -> vec3f {
    let K = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3f(0.0), vec3f(1.0)), c.y);
}";

node! {
    /// The saturation scaled, with the hue and the alpha left alone.
    SATURATE,
    slug: "saturate",
    icon: "🧂",
    label: "Saturate",
    category: Color,
    tooltip: "Multiplies how saturated the picture is. Zero is grayscale, one leaves it \
              alone, above one is hypersaturated. Hue and alpha are untouched.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "amount" "Amount" = Control::num(1.0, 0.0, 2.0, 0.01, "x"),
    ],
    wgsl_utils: [RGB2HSV_WGSL, HSV2RGB_WGSL],
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    var hsv = rgb2hsv(color.rgb);
    hsv.y = clamp(hsv.y * ({amount}), 0.0, 1.0);
    return vec4f(hsv2rgb(hsv), color.a);",
    ],
}

node! {
    /// The dull colors lifted while the vivid ones stay where they are.
    VIBRANCE,
    slug: "vibrance",
    icon: "✨",
    label: "Vibrance",
    category: Color,
    tooltip: "Lifts the muted colors while the already vivid ones barely move; a negative \
              amount pulls the vivid ones down first. Zero is the identity.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "amount" "Amount" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
    ],
    wgsl_utils: [RGB2HSV_WGSL, HSV2RGB_WGSL],
    outputs: [
        // The weight is what makes this the gentle half: going up it is what is left of the
        // saturation, so a gray pixel moves fully and a vivid one not at all; coming down it
        // is the saturation itself, so the vivid ones move first.
        VaryingColor "output" "Output" = "    let color = {input};
    var hsv = rgb2hsv(color.rgb);
    let amount = {amount};
    let weight = select(hsv.y, 1.0 - hsv.y, amount >= 0.0);
    hsv.y = clamp(hsv.y + amount * weight, 0.0, 1.0);
    return vec4f(hsv2rgb(hsv), color.a);",
    ],
}
