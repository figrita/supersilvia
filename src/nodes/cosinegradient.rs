// SPDX-License-Identifier: AGPL-3.0-or-later

//! A number turned into a color by a cosine per channel.
//!
//! Iñigo Quílez's palette form, `bias + amp · cos(2π(freq · t + phase))`, ported from
//! silvia's `cosinegradient.js`. One `VaryingNumber` in, one `VaryingColor` out: it is
//! the node that makes a picture out of a number, so a mask, a noise's value or an envelope
//! becomes a ramp without a color ramp editor anywhere.
//!
//! **The twelve coefficients are the node's own values, drawn as silvia draws them.** They
//! are hidden controls — no port, no cable, no row — laid out three across under R, G and B
//! by [`Region::Palette`](super::Region::Palette), over the gradient strip and the three
//! channel curves silvia puts on the same node. They were twelve `VaryingNumber` ports; the palette is the
//! one node whose whole job is to be looked at, and fifteen rows with nothing drawn was the
//! wrong shape for it. What the cells cost is what a cable into Amp G could have done, which
//! is what silvia never had either. **Time and Offset shift the palette**, in cycles of it:
//! still on a new node, as silvia's Cycle of zero is, with Offset placing it and a Speed turned
//! up, a shift every ten seconds at 1, or a gear cabled into Time drifting it.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind, Timing};

/// The twelve, as the grid lays them out: one term a row, R, G and B across.
///
/// The region reads this rather than spelling the keys again, so the node's own values and
/// the picture of them cannot fall out of step.
pub const ROWS: [(&str, [&str; 3]); 4] = [
    ("Bias", ["biasR", "biasG", "biasB"]),
    ("Amp", ["ampR", "ampG", "ampB"]),
    ("Freq", ["freqR", "freqG", "freqB"]),
    ("Phase", ["phaseR", "phaseG", "phaseB"]),
];

/// The palette at `t`, drifted by `phase`: Iñigo Quílez's form, in Rust.
///
/// The same expression the output's WGSL body is, so the strip and the curves on the node
/// are a picture of the picture rather than an approximation of it. `coefficients` is
/// [`ROWS`]' own order — bias, amp, freq, phase, each R, G and B.
pub fn eval(coefficients: &[[f32; 3]; 4], phase: f32, t: f32) -> [f32; 3] {
    std::array::from_fn(|c| {
        let [bias, amp, freq, offset] = [
            coefficients[0][c],
            coefficients[1][c],
            coefficients[2][c],
            coefficients[3][c],
        ];
        (bias + amp * (std::f32::consts::TAU * (freq * t + offset + phase)).cos()).clamp(0.0, 1.0)
    })
}

node! {
    /// The cosine palette: three waves through one number.
    COSINEGRADIENT,
    slug: "cosinegradient",
    // silvia's code point, put back now the vendored full Noto Emoji face renders it.
    icon: "🌈",
    label: "Cosine Gradient",
    category: Color,
    tooltip: "Turns a number into a color with one cosine per channel: bias + amp · \
              cos(2π(freq · t + phase)). Offsetting the three phases is what makes a ramp \
              run through the spectrum; its Speed or a gear in Time drifts all three.",
    timing: Timing::periodic(0.0).paced(0.1),
    inputs: [
        VaryingNumber "t" "Input" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
    ],
    hidden: [
        "biasR" "Bias R" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "biasG" "Bias G" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "biasB" "Bias B" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "ampR" "Amp R" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "ampG" "Amp G" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "ampB" "Amp B" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        "freqR" "Freq R" = Control::num(1.0, 0.0, 4.0, 0.01, "x"),
        "freqG" "Freq G" = Control::num(1.0, 0.0, 4.0, 0.01, "x"),
        "freqB" "Freq B" = Control::num(1.0, 0.0, 4.0, 0.01, "x"),
        "phaseR" "Phase R" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
        "phaseG" "Phase G" = Control::num(0.33, 0.0, 1.0, 0.01, ""),
        "phaseB" "Phase B" = Control::num(0.67, 0.0, 1.0, 0.01, ""),
    ],
    regions: &[crate::nodes::Region::Palette],
    outputs: [
        VaryingColor "output" "Output" = "    let t = {t};
    let bias = vec3f({biasR}, {biasG}, {biasB});
    let amp = vec3f({ampR}, {ampG}, {ampB});
    let freq = vec3f({freqR}, {freqG}, {freqB});
    let phase = vec3f({phaseR}, {phaseG}, {phaseB}) + time_periodic({clock}, {phaseOffset});
    let rgb = bias + amp * cos(2.0 * PI * (freq * t + phase));
    return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);",
    ],
}
