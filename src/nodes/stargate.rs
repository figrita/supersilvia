// SPDX-License-Identifier: AGPL-3.0-or-later

//! Time displacement through a slit: the live picture inside it, last frame dragged out of it.
//!
//! Ported from silvia's `stargate.js`. A thin line across the frame shows the live picture;
//! everywhere else the node reads last frame, shifted sideways along the slit's own normal,
//! in opposite directions on the two sides of it. Cabled into itself a frame at a time, the
//! picture pours out of the slit like a tunnel.
//!
//! **Last frame is a port, not a private history.** silvia samples a `u_frame_history` array
//! belonging to whichever output it is drawn into; here an Output's Frame Out is a cable, so
//! the second picture input is where it arrives — which also means the node can drag *any*
//! picture through its slit rather than only the one it happens to sit in. Nothing on the node
//! remembers anything: the memory is the cable, which is how feedback works here
//! ([decisions.md](../../../docs/decisions.md)).
//!
//! **Speed is a step per frame, and so has no accumulator.** The drag is not a position
//! computed from elapsed time — each frame moves what it was handed by one step and hands it
//! on, and the integral lives in the loop. There is no hidden accumulator to remove and no
//! `phase` to publish: a phase here would be a number the body has no use for.
//!
//! Three departures from the source. The step is in pixels of a 720-high frame rather than
//! one texel of the real output, so the drag holds its speed at any size — silvia's shift is
//! `1.0 / u_resolution` and no node body here reads the resolution. The zoom the perspective
//! makes is floored, since a cable can drive Perspective past its own slider into a division
//! by zero. And the slit's coverage is clamped: the two `smoothstep`s silvia subtracts cross
//! over below a width of 0.01 and the difference goes negative, which is not a coverage.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind, REFERENCE_HEIGHT};

/// The slit, its coverage, and where last frame is read from: shared by the picture and by
/// the mask beside it.
///
/// Worldspace is 2.0 tall, so a pixel of a 720-high frame is `1.0 / (720 * 0.5)` of a world
/// unit, and both axes take that same step — the step is square, as every size in pixels here
/// is.
fn stargate_common_wgsl() -> String {
    format!(
        "    let zoom = max(1.0 + ({{perspective}}) * 2.0, 1e-3);
    let zoomedUV = uv / zoom;
    let angle = ({{angle}}) * 2.0 * PI;
    let s = sin(angle);
    let c = cos(angle);
    let slit = (mat2x2f(c, -s, s, c) * zoomedUV).y;
    let center = {{offset}};
    let halfWidth = {{width}};
    let mask = clamp(
        smoothstep(center - halfWidth, center - halfWidth + 0.01, slit)
            - smoothstep(center + halfWidth - 0.01, center + halfWidth, slit),
        0.0,
        1.0);
    let drag = vec2f(s, -c) * sign(slit - center) * (({{speed}}) / ({REFERENCE_HEIGHT:?} * 0.5));
    let lastFrameUV = zoomedUV + drag;
"
    )
}

node! {
    /// The live picture in a moving slit, last frame dragged sideways out of it.
    DEF,
    slug: "stargate",
    icon: "🌌",
    label: "Star Gate",
    category: Effect,
    tooltip: "Shows the live picture inside a thin slit and last frame everywhere else, \
              shifted away from the slit in opposite directions on its two sides — the \
              slit-scan tunnel. Cable an Output's Frame Out into Last Frame to close the \
              loop. Angle turns the slit, Position slides it, Drift is the drag in pixels per \
              frame, and Perspective zooms what it reads.",
    inputs: [
        VaryingColor "input" "Input" at "zoomedUV" = Control::None,
        VaryingColor "lastFrame" "Last Frame" at "lastFrameUV" = Control::None,
        VaryingNumber "angle" "Angle" = Control::num(0.25, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "offset" "Position" = Control::num(0.0, -1.0, 1.0, 0.001, "⬓"),
        VaryingNumber "width" "Slit Width" = Control::num(0.02, 0.0, 0.5, 0.001, "⬓"),
        VaryingNumber "speed" "Drift" = Control::num(1.0, -10.0, 10.0, 0.01, "px"),
        VaryingNumber "perspective" "Perspective" = Control::num(0.0, 0.0, 0.05, 0.001, ""),
    ],
    wgsl_common: stargate_common_wgsl(),
    outputs: [
        VaryingColor "color" "Color" = "    return mix({lastFrame}, {input}, mask);",
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
    ],
}
