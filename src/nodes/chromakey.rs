// SPDX-License-Identifier: AGPL-3.0-or-later

//! A color keyed out of a picture, and the key published beside the composite.
//!
//! Ported from silvia's `chromakey.js`. The distance is silvia's: the fragment and the key
//! color both go to YCbCr, the chroma difference is the length of the Cb/Cr pair, and a fifth
//! of the luminance difference is added to it so a dark green and a bright green are not the
//! same color. One `smoothstep` about Threshold turns that distance into the key, Softness is
//! how wide the step is, and Spill Suppression pulls the dominant channel of the key out of
//! whatever survived.
//!
//! It is `Category::Color`, not `Effect`, by the test in [nodes.md]: its answer is a function
//! of the texel under the fragment — two of them, the picture's and the background's — and
//! nothing else. silvia files it under Effects, and `mix` and `layerblend` are the two
//! neighbors it belongs beside.
//!
//! `mask` is the key, 1 where the picture is kept and 0 where the background shows through,
//! which is what a `mask` means here: it drives Mix, Layer Blend or anything else that takes
//! coverage, so the node does not have to be the one that stacks.
//!
//! [nodes.md]: ../../../docs/nodes.md
//!
//! **The key reads the picture's own color, and what it keeps goes over the background.** A
//! picture is premultiplied, so its color is unpremultiplied before the distance and the spill
//! pull, which are nonlinear in it; the kept picture, its alpha times the key, is premultiplied
//! again and laid over the background by Porter–Duff over, `kept + bg·(1 − kept.a)`. silvia's
//! `mix(bg, picture, key)` is the same over an opaque picture; over a transparent one the
//! background shows through wherever the picture does not cover, as it does around a keyed
//! color.
//!
//! Three departures from the source besides, all the shader's. The Background keeps silvia's magenta
//! default — `every_control_default_is_representable` now holds magenta to its own hex
//! spelling rather than refusing it, since that text parses by being the color's own name. The
//! softness is floored, because here a cable can drive it to zero where a `smoothstep` of
//! equal edges is undefined rather than a hard edge. The spill pull is not behind silvia's
//! `alpha > 0.01` guard: its own weight already goes to
//! zero with the key, and the pixels the guard excluded are the ones the composite replaces
//! with the background entirely. And the dominant channel is found with `>=` rather than
//! silvia's `==` against the maximum, because a cable can drive Key Color from a picture and
//! an interpolated channel need not land exactly on its own maximum.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

node! {
    /// Everything near a key color replaced by a background, with the key published beside it.
    DEF,
    slug: "chromakey",
    icon: "🟢",
    label: "Chroma Key",
    category: Color,
    tooltip: "Keys a color out of the picture and puts the background behind it. Threshold \
              is how close a color has to be to count as the key, Softness feathers the edge, \
              and Spill Suppression pulls the key's own channel off what is left. Its mask is \
              the key, 1 where the picture stays.",
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingColor "background" "Background" = Control::color("#ff00ffff"),
        VaryingColor "keyColor" "Key Color" = Control::color("#00ff00ff"),
        VaryingNumber "threshold" "Threshold" = Control::num(0.4, 0.0, 1.0, 0.01, ""),
        VaryingNumber "softness" "Softness" = Control::num(0.2, 0.0, 1.0, 0.01, ""),
        VaryingNumber "spill" "Spill Suppression" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
    ],
    // The chroma distance, weighted against the luminance difference, stepped about the
    // threshold. `distance` is a WGSL builtin, so the quantity carries the node's own name.
    wgsl_common: "    let picture = {input};
    let foreground = unpremultiply(picture);
    let key = unpremultiply({keyColor});
    let thresh = {threshold};
    let soft = max({softness}, 1e-4);
    let keyY = dot(key.rgb, vec3f(0.299, 0.587, 0.114));
    let keyChroma = vec2f(0.564 * (key.b - keyY), 0.713 * (key.r - keyY));
    let fgY = dot(foreground.rgb, vec3f(0.299, 0.587, 0.114));
    let fgChroma = vec2f(0.564 * (foreground.b - fgY), 0.713 * (foreground.r - fgY));
    let keyDistance = length(fgChroma - keyChroma) + abs(fgY - keyY) * 0.2;
    let mask = smoothstep(thresh - soft, thresh + soft, keyDistance);
",
    outputs: [
        VaryingColor "color" "Color" = "    let bg = {background};
    var despilled = foreground.rgb;
    let maxChannel = max(key.r, max(key.g, key.b));
    if (key.g >= maxChannel) {
        despilled.g = min(despilled.g, (despilled.r + despilled.b) * 0.5);
    } else if (key.b >= maxChannel) {
        despilled.b = min(despilled.b, (despilled.r + despilled.g) * 0.5);
    }
    despilled = mix(foreground.rgb, despilled, clamp({spill}, 0.0, 1.0) * (1.0 - mask));
    let kept = premultiply(vec4f(despilled, picture.a * mask));
    return kept + bg * (1.0 - kept.a);",
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
    ],
}
