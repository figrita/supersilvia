// SPDX-License-Identifier: AGPL-3.0-or-later

//! Geiss' flow field: the returning frame dragged along a swirl rather than smeared along a
//! trail.
//!
//! silvia's `geissflow.js`. Six knobs shape one vector field — Flow Speed, Flow Scale and
//! Swirl turn it, Distortion says how far a pixel is dragged along it, Feedback and Fade say
//! how much of the older picture survives — and the field is what makes a loop move like
//! liquid instead of piling up.
//!
//! **The older frame is a cable.** silvia reads its Output's frame history behind your back,
//! one frame deep, so the same node is a different picture under two Outputs. Here `Last
//! Frame` is an ordinary picture input: cable an Output's Frame Out into it and the loop says
//! out loud where it comes from. There is no history and no delay knob — see
//! [decisions.md](../../../docs/decisions.md).
//!
//! **The flow takes its time from Time and Offset**, in cycles of the 20π over which silvia's
//! five wave rates line up. silvia multiplies `u_time` by Flow Speed inside the body, so
//! turning the speed jumps the field; here Speed 1 or ambient time runs it at a cycle every
//! 160 s — silvia's 0.4 over 20π, rounded to whole seconds — and its Speed, integrated, or a
//! gear cabled into Time turns it.
//!
//! Two conversions from silvia's coordinates, both because it works in the frame's own 0 to 1
//! and this works in worldspace: the flow offset is doubled, since worldspace is 2.0 tall
//! where a screen is 1.0, and the sample that leaves the frame mirrors back in through the
//! texture's own wrap rather than being clamped to the edge.
//!
//! **The floor stays.** `max(result, live × (1 − feedback))` is what stops the loop
//! swallowing the live picture at full feedback, and it is most of why this reads as flow.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind, Timing};

node! {
    /// A swirling vector field that drags the returning frame around under the live one.
    DEF,
    slug: "geissflow",
    icon: "🦙",
    label: "Geiss Flow",
    category: Transform,
    tooltip: "Drags a returning frame along a swirling flow field and fades it under the live \
              picture — feedback that moves like liquid rather than smearing. Cable an \
              Output's Frame Out into Last Frame to close the loop. Flow Scale and Swirl \
              shape the field, Distortion is how far a pixel travels along it, and Feedback \
              and Fade say how much of the older picture survives.",
    timing: Timing::periodic(1.0 / 160.0),
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingColor "lastFrame" "Last Frame" at "flowed" = Control::color("#000000ff"),
    ],
    after_time: [
        VaryingNumber "flowScale" "Flow Scale" = Control::num(3.5, 0.1, 20.0, 0.1, "/⬓"),
        VaryingNumber "distortionAmount" "Distortion" = Control::num(0.015, 0.0, 0.1, 0.001, "⬓"),
        VaryingNumber "feedbackAmount" "Feedback" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "fadeAmount" "Fade" = Control::num(0.9, 0.8, 1.0, 0.01, ""),
        VaryingNumber "swirl" "Swirl" = Control::num(1.1, -3.0, 3.0, 0.01, ""),
    ],
    // The field, and the unit vector along it. `normalize` of a vector that cancelled to
    // nothing is a division by zero, which silvia leaves in and a cable here can reach.
    wgsl_common: "    let flowCoord = uv * {flowScale};
    let t = fract(time_periodic({clock}, {phaseOffset})) * 20.0 * PI;
    let swirl = {swirl};
    let noise1 = sin(flowCoord.x * 2.0 + t) * cos(flowCoord.y * 1.5 + t * 0.7);
    let noise2 = cos(flowCoord.x * 1.3 + t * 0.8) * sin(flowCoord.y * 2.2 + t * 0.5);
    let swirl1 = sin(length(flowCoord) * 3.0 + t) * swirl;
    let swirl2 = cos(length(flowCoord * 0.7) * 2.0 - t * 1.2) * swirl * 0.7;
    let around = atan2(flowCoord.y, flowCoord.x);
    var flowVector = vec2f(
        noise1 + swirl1 * cos(around + t),
        noise2 + swirl2 * sin(around + t)
    );
    let fromCenter = length(uv);
    let radialFlow = vec2f(uv.y, -uv.x) * (1.0 / (fromCenter + 0.1)) * swirl * 0.3;
    flowVector += radialFlow;
    let flowLength = length(flowVector);
    let flowDir = select(vec2f(0.0), flowVector / flowLength, flowLength > 1e-6);
",
    outputs: [
        // A screen fraction is twice as many world units, worldspace being 2.0 tall, so
        // silvia's Distortion means the same distance here as it does there.
        VaryingColor "output" "Output" = "    let flowed = uv + flowDir * ({distortionAmount} * 2.0);
    let feedback = {feedbackAmount};
    let older = {lastFrame} * {fadeAmount};
    let live = {input};
    let result = mix(older, live, 1.0 - feedback);
    return max(result, live * (1.0 - feedback));",
        VaryingNumber "angle" "Angle" in "[0, 1]" = "    return fract(atan2(flowDir.y, flowDir.x) / (2.0 * PI) + 0.5);",
    ],
}
