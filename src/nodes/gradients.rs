// SPDX-License-Identifier: AGPL-3.0-or-later

//! Two ramps between two colors: along a direction, and out from a point.
//!
//! Ported from silvia's `lineargradient.js` and `radialgradient.js`, which share their
//! `loop_mode`: what happens past the end of the ramp. Neither publishes a field. The ramp
//! *is* the picture, and a `VaryingNumber` beside it would be the same number twice — the
//! case `docs/decisions.md` names when it says the rule is not "every generator publishes a
//! mask".

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// What the ramp does outside 0..1: hold, repeat, or fold back on itself.
///
/// Reads `raw` and declares `t`.
fn loop_mode_wgsl(mode: &str) -> &'static str {
    match mode {
        "repeat" => "    let t = fract(raw);\n",
        "mirror" => {
            "    let t = select(fract(raw), 1.0 - fract(raw), floor_mod(floor(raw), 2.0) == 1.0);\n"
        }
        _ => "    let t = clamp(raw, 0.0, 1.0);\n",
    }
}

node! {
    /// A ramp along a direction.
    LINEARGRADIENT,
    slug: "lineargradient",
    icon: "▧",
    label: "Linear Gradient",
    category: Generate,
    tooltip: "A ramp between two colors along an angle. Frequency stretches or repeats it; \
              loop mode says what happens past the end.",
    inputs: [
        VaryingColor "startColor" "Start Color" = Control::color("#ffffffff"),
        VaryingColor "endColor" "End Color" = Control::color("#000000ff"),
        VaryingNumber "angle" "Angle" = Control::num(0.25, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "frequency" "Frequency" = Control::num(1.0, 0.1, 20.0, 0.1, "/⬓"),
        VaryingNumber "offset" "Shift" = Control::num(0.0, -5.0, 5.0, 0.01, "⬓"),
        VaryingNumber "center" "Center" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "loop_mode" "Loop Mode" = "once" [
            "once" => "Once (Clamp)",
            "repeat" => "Repeat",
            "mirror" => "Mirror (Ping-Pong)",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            "    let angleRad = ({angle}) * 2.0 * PI;
    let dir = vec2f(cos(angleRad), sin(angleRad));
    let raw = (dot(uv, dir) - ({center})) * ({frequency}) + ({offset});
",
            loop_mode_wgsl(ctx.option(node, "loop_mode")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = "    return mix({startColor}, {endColor}, t);",
    ],
}

node! {
    /// A ramp out from a point.
    RADIALGRADIENT,
    slug: "radialgradient",
    icon: "⭕",
    label: "Radial Gradient",
    category: Generate,
    tooltip: "A ramp between two colors out from a center. Loop mode says what happens past \
              the radius.",
    // "Center Color" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "centerColor" "Center Color" = Control::color("#ffffffff"),
        VaryingColor "edgeColor" "Edge Color" = Control::color("#000000ff"),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "radius" "Radius" = Control::num(1.0, 0.1, 5.0, 0.01, "⬓"),
        VaryingNumber "offset" "Shift" = Control::num(0.0, -5.0, 5.0, 0.01, "⬓"),
    ],
    options: [
        "loop_mode" "Loop Mode" = "once" [
            "once" => "Once (Clamp)",
            "repeat" => "Repeat",
            "mirror" => "Mirror (Ping-Pong)",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            "    let dist = length(uv - vec2f({centerX}, {centerY}));
    let radius = {radius};
    let raw = dist / select(radius, 1e-6, abs(radius) < 1e-6) + ({offset});
",
            loop_mode_wgsl(ctx.option(node, "loop_mode")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = "    return mix({centerColor}, {edgeColor}, t);",
    ],
}
