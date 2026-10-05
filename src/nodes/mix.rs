// SPDX-License-Identifier: AGPL-3.0-or-later

//! Blending two colors: the crossfade, and the nine layer modes.
//!
//! `mix` is the node that makes feedback expressible — mix a source with an Output's own
//! previous frame and the picture accumulates. It is a straight crossfade between two
//! colors by one amount, alpha included, which is exact on premultiplied colors as they are.
//!
//! `layerblend` is silvia's `layerblend.js`, and it sits **beside** `mix` rather than
//! absorbing it. It composites the way a layer stack does: the foreground, scaled by the
//! opacity, over the background. A crossfade driven from an oscillator wants none of that, and
//! it is the one thing in this file that a feedback patch reaches for every time.
//!
//! **The composite is the W3C's** (*Compositing and Blending Level 1*), on premultiplied
//! colors: `fg·(1 − bg.a) + bg·(1 − fg.a) + fg.a·bg.a·B(Cb, Cs)`, alpha `fg.a + bg.a·(1 −
//! fg.a)`, where `B` is the mode's blend function of the two layers' own colors,
//! unpremultiplied. Normal's `B` is the foreground, which makes it Porter–Duff over, `fg + bg·(1
//! − fg.a)`, and it is written so. A mode blends only where both layers cover: over nothing,
//! every mode is the foreground.
//!
//! Three departures from the source. silvia mixes straight colors by the foreground's alpha
//! times the opacity, each mode weighting by it in a way of its own; here the formula above
//! does it once for all nine, so a Text's soft edge is not darkened a second time. Opaque layers
//! at full opacity draw silvia's blend exactly, and an opacity below one lerps toward it in every
//! mode, Difference included, where silvia's was `|bg − fg·w|`. Every mode is one expression
//! rather than a chain of per-channel `if`s — Overlay and Hard Light are `mix` over a `step`,
//! which is the same function without three branches per fragment. And the opacity is clamped,
//! because here a cable can drive it past its ends.
//!
//! **`method`** carries the three of silvia's eight crossfades that need no screen to sweep
//! across: the plain mix and the two luminance fades, whose answer is a function of the
//! color under the fragment. The five that do sweep — the wipes, the checkers — are the
//! Main Mixer's, which has all eight over its own screen space (`mixer::Method`); a node
//! body reads no resolution, and a wipe here would have been a different picture in every
//! Output. **`curve`** carries silvia's fader warp,
//! `mixer::mix_amount` fed `2·amount − 1` so the default `linear` leaves `amount` as the even
//! 0…1 it always was. Both are `OptionKind::Uniform`: every branch is already in the program
//! and a change writes one uniform, not a rebuild — the case
//! [decisions.md](../../../docs/decisions.md) names for the kind. `amount` stays the port; the
//! fader only changes what a value drawn from it means.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
};

pub static DEF: NodeDef = NodeDef {
    slug: "mix",
    category: Category::Color,
    icon: "🎚",
    label: "Mix",
    tooltip: "Blends two colors. 0 is all A, 1 is all B. Method is how it gets there — a \
              mix or a luminance fade — and Curve is whether Amount is linear or silvia's \
              hard-cut fader.",
    inputs: &[
        InputDef {
            key: "a",
            label: "A",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "b",
            label: "B",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "amount",
            label: "Amount",
            ty: VaryingNumber,
            control: Control::num(0.5, 0.0, 1.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        // `f` is `amount` after the chosen curve, and `lum` the luminance the two fades
        // cross the picture in the order of: A's as it is, premultiplied, which is how bright
        // it shows over black, so a transparent pixel is dark.
        wgsl: |node, ctx, _func| {
            let a = ctx.input(node, "a", "uv");
            let b = ctx.input(node, "b", "uv");
            let amount = ctx.input(node, "amount", "uv");
            let method = ctx.option_uniform(node, "method");
            let curve = ctx.option_uniform(node, "curve");
            format!(
                "    let a = {a};
    let b = {b};
    let amt = clamp({amount}, 0.0, 1.0);
    let f = select(
        amt,
        clamp(0.5 + 0.5 * tan(clamp(2.0 * amt - 1.0, -0.999, 0.999) * 1.5707963), 0.0, 1.0),
        {curve} == 1);
    let lum = dot(a.rgb, vec3f(0.299, 0.587, 0.114));
    let m = {method};
    if (m == 1) {{
        return select(a, b, lum < f);
    }} else if (m == 2) {{
        return select(a, b, lum > 1.0 - f);
    }} else {{
        return mix(a, b, f);
    }}"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "method",
            label: "Method",
            default: "blend",
            choices: &[
                ("blend", "Simple mix"),
                ("dark_fade", "Dark fade first"),
                ("light_fade", "Light fade first"),
            ],
            kind: OptionKind::Uniform,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "curve",
            label: "Curve",
            default: "linear",
            choices: &[("linear", "Linear"), ("fader", "Fader")],
            kind: OptionKind::Uniform,
            ..OptionDef::EMPTY
        },
    ],
    ..NodeDef::EMPTY
};

/// The body after `bg` and `fg`, premultiplied, for one mode: Normal is Porter–Duff over, and
/// every other mode the W3C's general formula over `blended`, its blend function of the
/// layers' own colors `b` and `s`.
fn blend_mode_wgsl(mode: &str) -> String {
    let blended = match mode {
        "add" => "b + s",
        "multiply" => "b * s",
        "screen" => "1.0 - (1.0 - b) * (1.0 - s)",
        "overlay" => "mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(vec3f(0.5), b))",
        "soft" => "(1.0 - 2.0 * s) * b * b + 2.0 * b * s",
        "hard" => "mix(2.0 * s * b, 1.0 - 2.0 * (1.0 - s) * (1.0 - b), step(vec3f(0.5), s))",
        "difference" => "abs(b - s)",
        "exclusion" => "b + s - 2.0 * b * s",
        _ => return "    return fg + bg * (1.0 - fg.a);".to_string(),
    };
    format!(
        "    let b = unpremultiply(bg).rgb;
    let s = unpremultiply(fg).rgb;
    let blended = {blended};
    return vec4f(
        fg.rgb * (1.0 - bg.a) + bg.rgb * (1.0 - fg.a) + fg.a * bg.a * blended,
        fg.a + bg.a * (1.0 - fg.a));"
    )
}

node! {
    /// A layer stack's blend modes, weighted by the foreground's alpha and an opacity.
    LAYERBLEND,
    slug: "layerblend",
    // silvia's ⿻ (U+2FFB) is an Ideographic Description Character, not an emoji — neither
    // vendored face carries it, so `every_character_the_registry_draws_has_a_glyph` still
    // draws it as `◻` and the icon stays supersilvia's own rather than shipping a tofu box.
    icon: "🥞",
    label: "Layer Blend",
    category: Color,
    tooltip: "Composites a foreground over a background through one of nine layer modes, \
              weighted by the foreground's own alpha times the opacity. Unlike Mix, which \
              crossfades, this is the stack a paint program draws.",
    inputs: [
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingNumber "opacity" "Opacity" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "blend_mode" "Blend Mode" = "normal" [
            "normal" => "Normal",
            "add" => "Add (Linear Dodge)",
            "multiply" => "Multiply",
            "screen" => "Screen",
            "overlay" => "Overlay",
            "soft" => "Soft Light",
            "hard" => "Hard Light",
            "difference" => "Difference",
            "exclusion" => "Exclusion",
        ],
    ],
    outputs: [
        VaryingColor "output" "Output" = varying(|node, ctx| {
            let mut body = String::from(
                "    let bg = {background};
    let fg = {foreground} * clamp({opacity}, 0.0, 1.0);
",
            );
            body.push_str(&blend_mode_wgsl(ctx.option(node, "blend_mode")));
            body
        }),
    ],
}

#[cfg(test)]
mod tests {
    /// Every crossfade the node offers is one of the Main Mixer's, under the mixer's own
    /// name: one vocabulary for the same three operations, whichever surface performs them.
    /// Nothing else holds the two lists together, so renaming one shows up here.
    #[test]
    fn its_crossfades_are_named_as_the_main_mixer_names_them() {
        let node: Vec<&str> = super::DEF
            .option("method")
            .expect("mix has one")
            .choices
            .iter()
            .map(|(_, label)| *label)
            .collect();
        let mixer: Vec<&str> = crate::mixer::Method::ALL
            .iter()
            .map(|m| m.label())
            .collect();
        assert_eq!(node.len(), 3, "the three that need no screen: {node:?}");
        for label in node {
            assert!(
                mixer.contains(&label),
                "{label:?} is not one of the mixer's"
            );
        }
    }
}
