// SPDX-License-Identifier: AGPL-3.0-or-later

//! Varying color to varying number: the conversions that read one number out of a
//! picture, per pixel.
//!
//! These are the cheap half of "going down". A field of `vec4f` becomes a field of `f32`
//! with no readback and no frame of latency, because the answer is still a field — one
//! number per pixel, not one number per frame. Collapsing a field to a `UniformNumber` is a
//! different and far more expensive thing, and it is not here.
//!
//! `CONVERSIONS` is the one table: a slug, a label and an `f32` expression over `color` for
//! each of the eleven. The nodes are a macro over it, like `math`, and a `tap`'s `measure`
//! picker is the same table read as choices — so a conversion is written once and the two
//! cannot say different things about it. The table's order is the picker's, which is
//! channels first; the registry's order is the Nodes menu's, which is perceptual first.
//!
//! **A number read out of a color is the color's own.** Every conversion reads its input
//! through the prelude's `unpremultiply`, so half-transparent red reads a red of one and an
//! alpha of a half, as a picker would say; see
//! [decisions.md](../../../docs/decisions.md#colors-in-the-graph-are-premultiplied).
//!
//! `channelsplitter` is silvia's, and it reads the same table: one color in and four floats
//! out, where the four single-channel nodes are four nodes and four cables. Its expressions
//! are `CONVERSIONS`' entries for `red`, `green`, `blue` and `alpha`, looked up by slug at
//! compile time, so it cannot drift from the nodes beside it.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

/// One way of reading a number out of a color.
pub struct Conversion {
    /// The slug of the `Convert` node that does it, and the value of a `measure` option.
    pub slug: &'static str,
    pub label: &'static str,
    /// An `f32` expression in WGSL over `color`, and over the `maxc`, `minc` and `delta`
    /// that [`HELPERS_WGSL`] declares. `color` holds the color's own channels: whoever
    /// declares it reads the input through the prelude's `unpremultiply`.
    pub wgsl: &'static str,
    /// What the expression bounds the answer to, drawn on the output's row. Beside the
    /// expression, because it is a fact about the expression: a channel and a ratio of
    /// channels are 0 to 1, and the three that are neither say nothing.
    pub range: Option<&'static str>,
}

/// Every conversion there is, in picker order.
///
/// Hue, saturation and chroma are guarded on `delta`: a gray pixel has no hue, and dividing
/// by its zero chroma is a NaN across every gray region of the frame rather than a wrong
/// color in one.
pub const CONVERSIONS: &[Conversion] = &[
    Conversion {
        slug: "red",
        label: "Red",
        wgsl: "color.r",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "green",
        label: "Green",
        wgsl: "color.g",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "blue",
        label: "Blue",
        wgsl: "color.b",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "alpha",
        label: "Alpha",
        wgsl: "color.a",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "hue",
        label: "Hue",
        wgsl: "select(fract(select(select(
            (color.r - color.g) / delta + 4.0,
            (color.b - color.r) / delta + 2.0,
            maxc == color.g),
        (color.g - color.b) / delta,
        maxc == color.r) / 6.0), 0.0, delta <= 0.0)",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "saturation",
        label: "Saturation",
        wgsl: "select(delta / max(1.0 - abs(maxc + minc - 1.0), 1e-6), 0.0, delta <= 0.0)",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "lightness",
        label: "Lightness",
        wgsl: "(maxc + minc) * 0.5",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "luminosity",
        label: "Luminosity",
        wgsl: "dot(color.rgb, vec3f(0.2126, 0.7152, 0.0722))",
        range: Some("[0, 1]"),
    },
    Conversion {
        slug: "value",
        label: "Value",
        wgsl: "maxc",
        range: None,
    },
    Conversion {
        slug: "average",
        label: "Average",
        wgsl: "(color.r + color.g + color.b) / 3.0",
        range: None,
    },
    Conversion {
        slug: "chroma",
        label: "Chroma",
        wgsl: "delta",
        range: None,
    },
];

/// The table as an option's choices: every conversion, by slug and label, in table order.
pub const CHOICES: [(&str, &str); CONVERSIONS.len()] = {
    let mut out = [("", ""); CONVERSIONS.len()];
    let mut i = 0;
    while i < CONVERSIONS.len() {
        out[i] = (CONVERSIONS[i].slug, CONVERSIONS[i].label);
        i += 1;
    }
    out
};

/// The declarations a conversion expression is written against, beside `color` itself.
///
/// Emitted by whoever declares `color`: the eleven nodes, and a `tap` measuring by its
/// picker.
pub const HELPERS_WGSL: &str = "    let maxc = max(max(color.r, color.g), color.b);
    let minc = min(min(color.r, color.g), color.b);
    let delta = maxc - minc;";

/// Hue, saturation and lightness of one color, on the CPU: the `hue`, `saturation` and
/// `lightness` rows of [`CONVERSIONS`], written out in Rust.
///
/// For a reading that is already on the CPU. A `sample` holds one pixel of its input, and
/// asking the GPU for that pixel's hue would mean a `hue` node and a `tap` over sixteen
/// thousand grid points to collapse the field it produces back to one number — the shader
/// route is the wrong instrument for a question the CPU can already answer.
///
/// This is a **transcription**, so it is held to the table it was transcribed from:
/// `tests/gpu_nodes.rs` renders the three nodes over a color and compares their pixels
/// with what this returns, which is the only way two spellings of one formula can be kept
/// honest.
pub fn hsl(color: [f32; 4]) -> [f32; 3] {
    let [r, g, b, _] = color;
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let delta = maxc - minc;
    // A gray pixel has no hue and no saturation, and dividing by `delta` there is a
    // division by zero. The guard is the table's own, kept rather than reinvented.
    let hue = if delta <= 0.0 {
        0.0
    } else {
        let sextant = if maxc == r {
            (g - b) / delta
        } else if maxc == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        let sixth = sextant / 6.0;
        sixth - sixth.floor()
    };
    let saturation = if delta <= 0.0 {
        0.0
    } else {
        delta / (1.0 - (maxc + minc - 1.0).abs()).max(1e-6)
    };
    [hue, saturation, f32::midpoint(maxc, minc)]
}

/// The conversion a slug names, or `None` where it names none.
///
/// `const`, because each of the eleven definitions takes its own slug, label and expression
/// from the table at compile time: a slug that is not in it is a build failure rather than a
/// node that draws nothing.
pub const fn find(slug: &str) -> Option<&'static Conversion> {
    let mut i = 0;
    while i < CONVERSIONS.len() {
        if str_eq(CONVERSIONS[i].slug, slug) {
            return Some(&CONVERSIONS[i]);
        }
        i += 1;
    }
    None
}

/// `==` over two strings, in a `const` context, where the operator is not available.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// A node taking one color and returning one conversion of it, named by its slug in
/// `CONVERSIONS`.
macro_rules! decompose {
    ($name:ident, $slug:literal, $icon:literal, $tip:literal) => {
        pub static $name: NodeDef = NodeDef {
            slug: $slug,
            icon: $icon,
            label: match find($slug) {
                Some(c) => c.label,
                None => panic!("a Convert node's slug is a conversion in the table"),
            },
            tooltip: $tip,
            inputs: &[InputDef {
                key: "input",
                label: "Input",
                ty: VaryingColor,
                control: Control::None,
            }],
            outputs: &[OutputDef {
                key: "output",
                label: "Output",
                ty: VaryingNumber,
                kind: OutputKind::Shader,
                wgsl: |node, ctx, _func| {
                    let input = ctx.input(node, "input", "uv");
                    let expr = find($slug).expect("checked at the definition").wgsl;
                    format!("    let color = unpremultiply({input});\n{HELPERS_WGSL}\n    return {expr};")
                },
                range: match find($slug) {
                    Some(c) => c.range,
                    None => panic!("a Convert node's slug is a conversion in the table"),
                },
                ..OutputDef::EMPTY
            }],
            category: Category::Convert,
            ..NodeDef::EMPTY
        };
    };
}

// --- perceptual -----------------------------------------------------------------------

decompose!(
    LUMINOSITY,
    "luminosity",
    "🕯",
    "Perceived brightness. Rec.709 weights, so green counts for most and blue for least."
);

decompose!(
    LIGHTNESS,
    "lightness",
    "💡",
    "HSL lightness: the midpoint of the brightest and darkest channel."
);

decompose!(
    VALUE,
    "value",
    "🔆",
    "HSV value: the brightest channel. Unlike lightness, a saturated red is fully bright."
);

decompose!(
    AVERAGE,
    "average",
    "⚖",
    "The unweighted mean of red, green and blue. Ignores how the eye weights them."
);

// --- hue and chroma -------------------------------------------------------------------

decompose!(
    HUE,
    "hue",
    "🦜",
    "Position on the color wheel, 0 to 1. Gray has no hue and reads 0."
);

decompose!(
    SATURATION,
    "saturation",
    "🧂",
    "HSL saturation: how far the color is from gray at its lightness."
);

decompose!(
    CHROMA,
    "chroma",
    "💎",
    "The spread between the brightest and darkest channel. Saturation without the lightness \
     correction."
);

// --- channels -------------------------------------------------------------------------

decompose!(RED, "red", "🔴", "The red channel on its own.");

decompose!(GREEN, "green", "🟢", "The green channel on its own.");

decompose!(BLUE, "blue", "🔵", "The blue channel on its own.");

decompose!(ALPHA, "alpha", "🌫", "The alpha channel on its own.");

// --- all four at once -----------------------------------------------------------------

/// One channel port, reading its expression out of `CONVERSIONS` by slug.
///
/// The label is short — `R`, not `Red` — because four of them sit in one column; the lookup
/// is still by the table's own slug, so a name that is not in it is a build failure.
macro_rules! channel {
    ($key:literal, $label:literal, $slug:literal) => {
        OutputDef {
            key: $key,
            label: match find($slug) {
                Some(_) => $label,
                None => panic!("a channel port's slug is a conversion in the table"),
            },
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _func| {
                let input = ctx.input(node, "input", "uv");
                let expr = find($slug).expect("checked at the definition").wgsl;
                format!(
                    "    let color = unpremultiply({input});\n{HELPERS_WGSL}\n    return {expr};"
                )
            },
            range: match find($slug) {
                Some(c) => c.range,
                None => panic!("a channel port's slug is a conversion in the table"),
            },
            ..OutputDef::EMPTY
        }
    };
}

pub static CHANNELSPLITTER: NodeDef = NodeDef {
    slug: "channelsplitter",
    icon: "\u{2702}",
    label: "Channel Splitter",
    tooltip: "Splits a color into its four channels at once, so isolating one of them is a \
              port rather than a node.",
    inputs: &[InputDef {
        key: "input",
        label: "Input",
        ty: VaryingColor,
        control: Control::None,
    }],
    outputs: &[
        channel!("r", "R", "red"),
        channel!("g", "G", "green"),
        channel!("b", "B", "blue"),
        channel!("a", "A", "alpha"),
    ],
    category: Category::Convert,
    ..NodeDef::EMPTY
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The table and the registry are the same eleven, plus the splitter, `sliderule` and
    /// `reframerange`. A conversion the macro never makes a node of would be a `measure`
    /// choice with no node beside it; a node reading a quantity out of a color that is not in
    /// the table cannot exist, since the macro reads its label out of it. The last two read
    /// no color at all — they convert one number's range to another's — so they are named
    /// here rather than counted among them.
    #[test]
    fn every_conversion_is_a_convert_node_and_the_other_way_round() {
        let nodes: Vec<&str> = crate::nodes::REGISTRY
            .iter()
            .filter(|d| d.category == Category::Convert)
            .map(|d| d.slug)
            .collect();
        assert_eq!(nodes.len(), CONVERSIONS.len() + 3);
        for c in CONVERSIONS {
            assert!(nodes.contains(&c.slug), "{} has no node", c.slug);
        }
        assert!(nodes.contains(&"channelsplitter"));
        assert!(nodes.contains(&"sliderule"));
        assert!(nodes.contains(&"reframerange"));
    }

    /// The splitter's four ports are the four channel conversions, in order. It exists to be
    /// the same answers in one node, so a port reading something the node beside it does not
    /// is the failure worth catching.
    #[test]
    fn the_splitter_publishes_the_four_channel_conversions() {
        let ports: Vec<&str> = CHANNELSPLITTER.outputs.iter().map(|p| p.key).collect();
        assert_eq!(ports, ["r", "g", "b", "a"]);
        for (port, slug) in ports.iter().zip(["red", "green", "blue", "alpha"]) {
            let conversion = find(slug).expect("a channel is in the table");
            assert_eq!(conversion.wgsl, format!("color.{port}"));
        }
    }

    #[test]
    fn a_slug_that_names_no_conversion_is_none() {
        assert_eq!(find("banana").map(|c| c.slug), None);
        assert_eq!(find("re").map(|c| c.slug), None);
        assert_eq!(find("red").map(|c| c.slug), Some("red"));
    }
}
