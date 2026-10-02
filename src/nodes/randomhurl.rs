// SPDX-License-Identifier: AGPL-3.0-or-later

//! Chroma snow: three unrelated random channels at every pixel.
//!
//! Ported from silvia's `randomhurl.js`. Every other noise in the library rolls one number
//! per point and mixes two colors by it, so whatever the controls say the picture lands on
//! the line between those two colors; this one rolls the three channels apart and so is the
//! only noise here with color in it.
//!
//! Two departures from the source. silvia writes the three rolls twice, once for the color
//! and once for the field, which is how the two could drift apart; here the shared half is
//! the macro's `common`. And silvia's second output is `mask`, which means coverage
//! everywhere else here — it carries the snow's brightness, so it is `value`, the raw
//! quantity the picture was made from.
//!
//! There is no time row, as in silvia: the picture holds perfectly still until Seed is
//! driven, and `static` is the noise that moves on its own.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::distort::HASH_RANDOM_WGSL;
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

node! {
    /// Uncorrelated RGB noise.
    DEF,
    slug: "randomhurl",
    icon: "🤢",
    label: "Random Hurl",
    category: Generate,
    tooltip: "Pure chroma snow: an unrelated random value per color channel at every pixel. \
              It holds still until Seed moves.",
    inputs: [
        VaryingNumber "seed" "Seed" = Control::num(0.0, 0.0, 1000.0, 1.0, ""),
    ],
    wgsl_utils: [HASH_RANDOM_WGSL],
    // Each channel is hashed at a point of its own, because the same point hashed three
    // times is one number three times: the offsets are what breaks the correlation.
    wgsl_common: "    let seed = {seed};
    let r = hashRandom(uv + vec2f(seed, 0.0));
    let g = hashRandom(uv + vec2f(0.0, seed + 1.0));
    let b = hashRandom(uv + vec2f(seed + 2.0, seed + 3.0));
",
    outputs: [
        VaryingColor "color" "Color" = "    return vec4f(r, g, b, 1.0);",
        VaryingNumber "value" "Value" in "[0, 1]"
            = "    return dot(vec3f(r, g, b), vec3f(0.299, 0.587, 0.114));",
    ],
}
