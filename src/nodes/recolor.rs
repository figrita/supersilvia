// SPDX-License-Identifier: AGPL-3.0-or-later

//! Recolorings: a picture read for a quantity of its own and painted back from it.
//!
//! Ported from silvia's `colorize.js`, `colormapping.js` and `colorshift.js`. All three are
//! one texel in and one texel out, so all three are `Category::Color` beside `adjust.rs`;
//! what separates them from a `mix` against a swatch is that each takes the picture apart
//! first — into its brightness, or into hue, saturation and value — and puts it back.
//!
//! **One weighting for brightness.** silvia's `rgb2lum` is `0.299, 0.587, 0.114`; the two
//! bodies here carry the weights the `luminosity` conversion already uses, and a test below
//! reads them against the table so they cannot drift from it. A Colorize holding a different
//! brightness than the `luminosity` node one cable away would be two answers to one question.
//!
//! **Hue, saturation and value are the table's.** `colorshift` reads HSV out of the color
//! with [`crate::nodes::decompose::HELPERS_WGSL`] and the `hue` conversion's own expression.
//! Saturation and value are HSV's — `delta / maxc` and `maxc` — and not the `saturation` and
//! `lightness` nodes', which are HSL's: a shifter that took a picture apart in one space and
//! rebuilt it in another would move colors nobody asked it to. `colormapping`'s saturation
//! hold is HSV's for the same reason, since it is silvia's `rgb2hsv().y`.
//!
//! **No global for three lines.** The write back to RGB is written into `colorshift`'s own
//! body rather than emitted as a `wgsl_utils` function. A util is one global per shader, so
//! a name is a claim on every shader the node appears in, and this is the only node in the
//! file that converts in that direction.
//!
//! **All three read the color's own channels.** Each is nonlinear in them, so the picture goes
//! in through the prelude's `unpremultiply` and the answer comes back through `premultiply`
//! at the input's alpha; the swatches a grade or a tint is painted from are read for their
//! own channels the same way, so a swatch's alpha is never part of the color it lends. See
//! [decisions.md](../../../docs/decisions.md#colors-in-the-graph-are-premultiplied).
//!
//! Two departures from the source. `colorshift`'s unconnected input falls through to the
//! default picture like every other node here, where silvia substitutes black on this one
//! node alone. And every divisor is guarded, because a cable can drive any of these where a
//! slider could not.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::decompose;
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

// --------------------------------------------------------------------------------- colorize

/// The gray, the tint, the mix back, and the brightness handed back by as much as the knob
/// asks. `newLum` is guarded: a tint toward black takes the result's brightness to zero.
const COLORIZE_BODY_WGSL: &str = "    let color = unpremultiply({input});
    let originalLum = dot(color.rgb, vec3f(0.2126, 0.7152, 0.0722));
    let tinted = vec3f(originalLum) * unpremultiply({tint}).rgb;
    var result = mix(color.rgb, tinted, {amount});
    let newLum = dot(result, vec3f(0.2126, 0.7152, 0.0722));
    result = mix(result, result * (originalLum / max(newLum, 0.001)), {preserveLuminance});
    return premultiply(vec4f(clamp(result, vec3f(0.0), vec3f(1.0)), color.a));";

node! {
    /// The picture grayed, tinted, mixed back in, and handed its own brightness again.
    COLORIZE,
    slug: "colorize",
    icon: "🖌",
    label: "Colorize",
    category: Color,
    tooltip: "Tints the picture: its own brightness times the tint color, mixed back in by \
              Amount. Preserve Luminance puts back the brightness the tint cost it, which is \
              what a mix against a solid color cannot do.",
    // "Preserve Luminance" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingColor "tint" "Tint Color" = Control::color("#ff00ffff"),
        VaryingNumber "amount" "Amount" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "preserveLuminance" "Preserve Luminance"
            = Control::num(0.5, 0.0, 1.0, 0.01, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = COLORIZE_BODY_WGSL,
    ],
}

// ---------------------------------------------------------------------------- color mapping

/// The brightness the grade is indexed by, which is also the node's second port.
const MAPPING_COMMON_WGSL: &str = "    let color = unpremultiply({input});
    let lum = dot(color.rgb, vec3f(0.2126, 0.7152, 0.0722));
";

/// The ramp's two halves meet at mid gray with a kink in them. That is silvia's, and it is
/// part of how a grade of its looks; smoothing it would move every ported patch.
///
/// The hold is HSV saturation, silvia's `rgb2hsv().y`, guarded on a black pixel having none.
///
/// The ramp is an `if` rather than a `select`, so only the half the brightness lands in reads
/// its swatch.
const MAPPING_COLOR_WGSL: &str = "    let midColor = unpremultiply({midtones});
    var graded: vec3f;
    if (lum < 0.5) {
        graded = mix(unpremultiply({shadows}).rgb, midColor.rgb, lum * 2.0);
    } else {
        graded = mix(midColor.rgb, unpremultiply({highlights}).rgb, (lum - 0.5) * 2.0);
    }
    let maxc = max(max(color.r, color.g), color.b);
    let minc = min(min(color.r, color.g), color.b);
    let saturation = select((maxc - minc) / maxc, 0.0, maxc <= 0.0);
    let result = mix(graded, color.rgb, saturation * 0.3);
    return premultiply(vec4f(clamp(result, vec3f(0.0), vec3f(1.0)), color.a));";

node! {
    /// Brightness mapped onto three swatches, with the source's own color held back in.
    COLORMAPPING,
    slug: "colormapping",
    icon: "👥",
    label: "Color Mapping",
    category: Color,
    tooltip: "Grades the picture by its brightness: dark toward Shadows, bright toward \
              Highlights, the middle blending through Midtones. A colorful source keeps some \
              of its own color, by how saturated it already is. Its value is that brightness.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingColor "shadows" "Shadows" = Control::color("#0000ffff"),
        VaryingColor "midtones" "Midtones" = Control::color("#808080ff"),
        VaryingColor "highlights" "Highlights" = Control::color("#ffff00ff"),
    ],
    wgsl_common: MAPPING_COMMON_WGSL,
    outputs: [
        VaryingColor "color" "Color" = MAPPING_COLOR_WGSL,
        VaryingNumber "value" "Value" in "[0, 1]" = "    return lum;",
    ],
}

// ------------------------------------------------------------------------------- color shift

/// The read, up to the hue expression the `hue` conversion is.
const SHIFT_HEAD_WGSL: &str = "    let color = unpremultiply({input});\n";

/// What that expression is written against, and the name it lands in.
const SHIFT_HUE_WGSL: &str = "\n    var hue = ";

/// The three shifts, and the write back to RGB. `hue` is in turns: one is the full wheel.
const SHIFT_TAIL_WGSL: &str = ";
    var sat = select(delta / maxc, 0.0, maxc <= 0.0);
    hue = fract(hue + ({hue}));
    sat = clamp(sat * ({saturation}), 0.0, 1.0);
    let val = clamp(maxc * ({value}), 0.0, 1.0);
    let wheel = abs(fract(vec3f(hue) + vec3f(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    let rgb = val * mix(vec3f(1.0), clamp(wheel - 1.0, vec3f(0.0), vec3f(1.0)), sat);
    return premultiply(vec4f(rgb, color.a));";

node! {
    /// Hue turned, saturation and value scaled, in the color space the library already reads.
    COLORSHIFT,
    slug: "colorshift",
    icon: "🧪",
    label: "Color Shift",
    category: Color,
    tooltip: "Turns the picture's hue around the wheel and scales how colorful and how \
              bright it is. One turn of Hue Shift is the whole wheel.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "hue" "Hue Shift" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "saturation" "Saturation" = Control::num(1.0, 0.0, 2.0, 0.01, ""),
        VaryingNumber "value" "Value" = Control::num(1.0, 0.0, 2.0, 0.01, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(|_node, _ctx| {
            let hue = decompose::find("hue").expect("hue is in the table").wgsl;
            [
                SHIFT_HEAD_WGSL,
                decompose::HELPERS_WGSL,
                SHIFT_HUE_WGSL,
                hue,
                SHIFT_TAIL_WGSL,
            ]
            .concat()
        }),
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The weights both bodies read brightness at are the `luminosity` conversion's, so a
    /// change to the table is a failure here rather than two brightnesses in one library.
    #[test]
    fn the_brightness_is_the_luminosity_nodes() {
        let luminosity = decompose::find("luminosity").expect("luminosity is in the table");
        let weights = luminosity
            .wgsl
            .strip_prefix("dot(color.rgb, ")
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("the conversion is a dot against a weight vector");
        for body in [COLORIZE_BODY_WGSL, MAPPING_COMMON_WGSL] {
            assert!(body.contains(weights), "{body} lost {weights}");
        }
    }
}
