// SPDX-License-Identifier: AGPL-3.0-or-later

//! Blending two colors: the crossfade, and the nine layer modes.
//!
//! `mix` is the node that makes feedback expressible — mix a source with an Output's own
//! previous frame and the picture accumulates. It is a straight crossfade between two
//! colors by one amount, alpha included.
//!
//! `layerblend` is silvia's `layerblend.js`, and it sits **beside** `mix` rather than
//! absorbing it. Its Normal mode is not the crossfade: it weights by the foreground's own
//! alpha times an opacity, `mix(bg, fg, fg.a * opacity)`, and composites the alphas the way a
//! layer stack does. A crossfade driven from an oscillator wants neither of those, and it is
//! the one thing in this file that a feedback patch reaches for every time.
//!
//! Two departures from the source. Every mode is one expression rather than a chain of
//! per-channel `if`s — Overlay and Hard Light are `mix` over a `step`, which is the same
//! function without three branches per fragment. And the opacity is clamped, because here a
//! cable can drive it past its ends.
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
        // cross the picture in the order of.
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

/// The blended color one mode makes, over `bg`, `fg` and the coverage `w`.
fn blend_mode_wgsl(mode: &str) -> &'static str {
    match mode {
        "add" => "    let blended = bg.rgb + fg.rgb * w;\n",
        "multiply" => "    let blended = bg.rgb * mix(vec3f(1.0), fg.rgb, w);\n",
        "screen" => "    let blended = 1.0 - (1.0 - bg.rgb) * (1.0 - fg.rgb * w);\n",
        "overlay" => {
            "    let lo = 2.0 * bg.rgb * fg.rgb;
    let hi = 1.0 - 2.0 * (1.0 - bg.rgb) * (1.0 - fg.rgb);
    let blended = mix(bg.rgb, mix(lo, hi, step(vec3f(0.5), bg.rgb)), w);
"
        }
        "soft" => {
            "    let soft = (1.0 - 2.0 * fg.rgb) * bg.rgb * bg.rgb + 2.0 * bg.rgb * fg.rgb;
    let blended = mix(bg.rgb, soft, w);
"
        }
        "hard" => {
            "    let lo = 2.0 * fg.rgb * bg.rgb;
    let hi = 1.0 - 2.0 * (1.0 - fg.rgb) * (1.0 - bg.rgb);
    let blended = mix(bg.rgb, mix(lo, hi, step(vec3f(0.5), fg.rgb)), w);
"
        }
        "difference" => "    let blended = abs(bg.rgb - fg.rgb * w);\n",
        "exclusion" => "    let blended = bg.rgb + fg.rgb * w - 2.0 * bg.rgb * fg.rgb * w;\n",
        _ => "    let blended = mix(bg.rgb, fg.rgb, w);\n",
    }
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
            [
                "    let bg = {background};
    let fg = {foreground};
    let w = fg.a * clamp({opacity}, 0.0, 1.0);
",
                blend_mode_wgsl(ctx.option(node, "blend_mode")),
                "    return vec4f(blended, w + bg.a * (1.0 - w));",
            ]
            .concat()
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
