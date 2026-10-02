// SPDX-License-Identifier: AGPL-3.0-or-later

//! Color maps: one texel in, one texel out.
//!
//! Ported from silvia's `contrast.js`, `gamma.js`, `levels.js`, `invert.js`, `posterize.js`,
//! `vignette.js` and `simplelight.js`, which silvia files under one `Effects` heading of
//! thirty-three. Here the categories say what a node *does*: none of these looks at a second
//! texel, so all seven are `Category::Color` and the neighborhood kernels in `convolve.rs` are
//! `Category::Effect`. [decisions.md](../../../docs/decisions.md) has the argument.
//!
//! `vignette` is the one that reads `uv` — its falloff is a function of the coordinate — and
//! it is still per-texel: the picture it returns is its input at this fragment times a
//! number. It publishes that falloff as `mask`, which is what `mask` means: coverage, 1
//! inside.
//!
//! `simplelight` is the one that reads no picture to map: its normal map says which way the
//! surface faces and its own two swatches say what color it is, so what arrives is a
//! direction rather than a picture. It is still one texel in and one texel out, which is
//! what puts it here rather than beside the neighborhood kernel that writes its input.
//! It gains an **Ambient** the source has none of, because silvia's unlit side is pure
//! black; at zero it is silvia's shading exactly.
//!
//! Two departures from the source. Every divisor and every `smoothstep` span is guarded,
//! because here a cable can drive any of them to zero where silvia has only a slider with a
//! floor on it. And `posterize`'s dither is added unconditionally rather than behind an
//! `if`, since an amount of zero already adds nothing.
//!
//! **`posterize` carries silvia's Color Dither**, which is this same snap to a ladder with
//! the rounding decided by an ordered pattern rather than by a hash: `pattern` is that
//! choice, Hash Noise being what the node already did, and `scale` is how many pixels wide
//! one cell of it is. Its four other inputs are read at the cell's center, which is what
//! stops a modulated knob tearing inside a cell — at the default scale of one pixel that
//! cell is one texel of a 720-high frame, so the node is still a color map wherever nobody
//! widens it.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::screentone::{dither_threshold_wgsl, pixel_grid_wgsl};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

/// The cell one of `posterize`'s patterns quantizes to, and the 0 to 1 threshold that decides
/// which way a channel rounds inside it.
///
/// Every branch ends with `posterizeUV` and `threshold` declared: the coordinate the node
/// reads all four of its inputs at, so a modulated knob cannot tear inside a cell, and the
/// offset added before the snap. silvia's Color Dither is these five patterns over silvia's
/// Posterize, so they land here rather than as a second node that snaps to the same ladder —
/// [decisions.md](../../../docs/decisions.md) has the argument.
///
/// The three Bayer matrices are [`dither_threshold_wgsl`]'s, the `dither` node's own. Hash Noise is
/// the pattern `posterize` already had, kept expression for expression and read at the cell
/// instead of at the fragment. Hexagonal and Triangular are silvia's, and their lattices are
/// `mosaic`'s written over the pixel grid rather than over a cell frequency.
fn posterize_pattern_wgsl(pattern: &str) -> String {
    let square = format!(
        "{}    let cellPixel = floor(pixel / scale);
    let posterizeUV = (cellPixel * scale + scale * 0.5) / ({REFERENCE_HEIGHT:?} * 0.5);
",
        pixel_grid_wgsl()
    );
    let lattice =
        format!("    let cellFreq = {REFERENCE_HEIGHT:?} / scale;\n    let p = uv * cellFreq;\n");
    match pattern {
        "hex" => {
            lattice
                + "    let r = vec2f(1.0, 1.73);
    let hs = r * 0.5;
    let a = floor_mod2(p, r) - hs;
    let b = floor_mod2(p - hs, r) - hs;
    let gv = select(b, a, dot(a, a) < dot(b, b));
    let id = round(p - gv);
    let posterizeUV = id / cellFreq;
    let threshold = fract(sin(dot(id, vec2f(12.9898, 78.233))) * 43758.5453);
"
        }
        "tri" => {
            lattice
                + "    let skewed = vec2f(p.x - p.y * 0.5, p.y * 0.866);
    let cell = floor(skewed);
    let f = fract(skewed);
    let upper = step(f.x + f.y, 1.0);
    let tri = cell + (1.0 - upper);
    let middle = tri + mix(vec2f(-0.333), vec2f(0.333), upper);
    let posterizeUV = vec2f(middle.x + middle.y / 0.866 * 0.5, middle.y / 0.866) / cellFreq;
    let threshold = fract(sin(dot(tri, vec2f(12.9898, 78.233))) * 43758.5453);
"
        }
        "noise" => {
            square
                + "    let threshold = fract(sin(dot(posterizeUV * 1000.0, vec2f(12.9898, 78.233))) * 43758.5453);
"
        }
        bayer => square + dither_threshold_wgsl(bayer),
    }
}

node! {
    /// A gain about mid gray, with a brightness offset after it.
    CONTRAST,
    slug: "contrast",
    // silvia's code point, put back now the vendored full Noto Emoji face renders it: the
    // half-lit moon a subset font could not draw was worked around with the supersilvia-only "◐"
    // rather than the codepoint supersilvia's own registry test now clears.
    icon: "🌗",
    label: "Contrast",
    category: Color,
    tooltip: "Scales the input away from mid gray. Above 1 hardens the picture, below 1 \
              flattens it; brightness is added afterwards.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "contrast" "Contrast" = Control::num(1.0, 0.0, 3.0, 0.01, "x"),
        VaryingNumber "brightness" "Brightness" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    let adjusted = (color.rgb - 0.5) * ({contrast}) + 0.5 + ({brightness});
    return vec4f(clamp(adjusted, vec3f(0.0), vec3f(1.0)), color.a);",
    ],
}

node! {
    /// A power curve on the channels, leaving black and white where they are.
    GAMMA,
    slug: "gamma",
    icon: "☀",
    label: "Gamma",
    category: Color,
    tooltip: "Bends the midtones without moving black or white. Above 1 brightens them, \
              below 1 darkens them. Alpha is untouched.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "gamma" "Gamma" = Control::num(1.0, 0.1, 5.0, 0.01, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    let corrected = pow(max(color.rgb, vec3f(0.0)), vec3f(1.0 / max({gamma}, 1e-4)));
    return vec4f(corrected, color.a);",
    ],
}

node! {
    /// The tonal range clipped, stretched, bent and landed on a new pair of ends.
    LEVELS,
    slug: "levels",
    icon: "📊",
    label: "Levels",
    category: Color,
    tooltip: "Remaps the tonal range. In Black and In White clip and stretch what arrives, \
              Midtones bends what is between them, and Out Black and Out White are where the \
              ends land.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "inBlack" "In Black" = Control::num(0.0, 0.0, 1.0, 0.001, ""),
        VaryingNumber "inWhite" "In White" = Control::num(1.0, 0.0, 1.0, 0.001, ""),
        VaryingNumber "gamma" "Midtones" = Control::num(1.0, 0.1, 5.0, 0.01, ""),
        VaryingNumber "outBlack" "Out Black" = Control::num(0.0, 0.0, 1.0, 0.001, ""),
        VaryingNumber "outWhite" "Out White" = Control::num(1.0, 0.0, 1.0, 0.001, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    let inBlack = {inBlack};
    let outBlack = {outBlack};
    let inRange = max({inWhite} - inBlack, 1e-4);
    var t = clamp((color.rgb - inBlack) / inRange, vec3f(0.0), vec3f(1.0));
    t = pow(t, vec3f(1.0 / max({gamma}, 1e-4)));
    return vec4f(outBlack + t * (({outWhite}) - outBlack), color.a);",
    ],
}

node! {
    /// The negative, mixed against the original.
    INVERT,
    slug: "invert",
    icon: "🔳",
    label: "Invert",
    category: Color,
    tooltip: "The photographic negative. Mix fades between the original and the inverted \
              picture; alpha is untouched.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "mix" "Mix" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
    ],
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    return vec4f(mix(color.rgb, 1.0 - color.rgb, clamp({mix}, 0.0, 1.0)), color.a);",
    ],
}

node! {
    /// Each channel snapped to a ladder of levels, with noise to break the banding.
    POSTERIZE,
    slug: "posterize",
    icon: "🎭",
    label: "Posterize",
    category: Color,
    tooltip: "Snaps each channel to a ladder of levels. Gamma decides where on the ramp the \
              steps fall, Dither offsets a channel before the snap so a smooth gradient \
              breaks up instead of banding, and Pattern is what that offset comes off — a \
              hash, a Bayer grid, or a hexagonal or triangular cell. Scale is how many pixels \
              wide one cell of the pattern is.",
    inputs: [
        VaryingColor "input" "Input" at "posterizeUV" = Control::None,
        VaryingNumber "levels" "Levels" at "posterizeUV" = Control::num(4.0, 2.0, 32.0, 1.0, ""),
        VaryingNumber "dither" "Dither" at "posterizeUV" = Control::num(0.0, 0.0, 0.1, 0.001, ""),
        VaryingNumber "gamma" "Gamma" at "posterizeUV" = Control::num(1.0, 0.5, 2.0, 0.01, ""),
        VaryingNumber "scale" "Scale" = Control::num(1.0, 1.0, 16.0, 1.0, "px"),
    ],
    options: [
        "pattern" "Pattern" = "noise" [
            "noise" => "Hash Noise",
            "bayer2" => "Bayer 2x2",
            "bayer4" => "Bayer 4x4",
            "bayer8" => "Bayer 8x8",
            "hex" => "Hexagonal",
            "tri" => "Triangular",
        ],
    ],
    // The dither is added whatever its amount, where silvia guards it with an `if`: zero
    // times the threshold is zero, and a branch every fragment takes or does not is not free.
    wgsl_common: varying(|node, ctx| {
        let mut source = "    let scale = max(floor({scale}), 1.0);\n".to_string();
        source.push_str(&posterize_pattern_wgsl(ctx.option(node, "pattern")));
        source
    }),
    outputs: [
        VaryingColor "output" "Output" = "    let color = {input};
    let levels = max({levels}, 1.0);
    let gammaVal = max({gamma}, 1e-4);
    var ramped = pow(max(color.rgb, vec3f(0.0)), vec3f(gammaVal));
    ramped += (threshold - 0.5) * ({dither});
    let stepped = floor(ramped * levels + 0.5) / levels;
    let restored = pow(max(stepped, vec3f(0.0)), vec3f(1.0 / gammaVal));
    return vec4f(clamp(restored, vec3f(0.0), vec3f(1.0)), color.a);",
    ],
}

node! {
    /// The frame darkened toward its corners, by a falloff the node also publishes.
    VIGNETTE,
    slug: "vignette",
    icon: "🔦",
    label: "Vignette",
    category: Color,
    tooltip: "Darkens the frame away from its center. Radius is where the falloff starts, \
              softness how wide it is, and a negative strength brightens instead. Its mask is \
              that falloff, 1 in the middle.",
    inputs: [
        VaryingColor "input" "Input" = Control::None,
        VaryingNumber "strength" "Strength" = Control::num(0.8, -2.0, 2.0, 0.01, ""),
        VaryingNumber "radius" "Radius" = Control::num(0.8, 0.0, 3.0, 0.01, "⬓"),
        VaryingNumber "softness" "Softness" = Control::num(0.5, 0.01, 2.0, 0.01, "⬓"),
    ],
    // A cable can drive the softness to zero, where a `smoothstep` of equal edges is
    // undefined rather than a hard edge.
    wgsl_common: "    let radius = {radius};
    let softness = max({softness}, 1e-4);
    let mask = 1.0 - smoothstep(radius - softness, radius + softness, length(uv));
",
    outputs: [
        VaryingColor "color" "Color" = "    let color = {input};
    return vec4f(color.rgb * (1.0 - (1.0 - mask) * ({strength})), color.a);",
        VaryingNumber "mask" "Mask" = "    return mask;",
    ],
}

node! {
    /// A surface shaded by one lamp, from the direction a normal map says it faces.
    SIMPLELIGHT,
    slug: "simplelight",
    icon: "💡",
    label: "Simple Light",
    category: Color,
    tooltip: "Reads a normal map — a picture whose color says which way each point faces, as \
              Height to Normal writes one — and shades the surface as if a lamp sat where \
              Light X, Light Y and Light Z put it. Ambient is how lit the side facing away \
              from the lamp still is. Its value is the diffuse term.",
    // "Surface Color" does not fit the default 200 beside its swatch.
    // See docs/decisions.md#a-node-may-declare-a-wider-body.
    width: 240.0,
    inputs: [
        VaryingColor "normalMap" "Normal Map" = Control::None,
        VaryingColor "surfaceColor" "Surface Color" = Control::color("#ffffffff"),
        VaryingColor "lightColor" "Light Color" = Control::color("#ffffffff"),
        VaryingNumber "lightX" "Light X" = Control::num(0.5, -2.0, 2.0, 0.01, ""),
        VaryingNumber "lightY" "Light Y" = Control::num(0.5, -2.0, 2.0, 0.01, ""),
        VaryingNumber "lightZ" "Light Z" = Control::num(1.0, 0.0, 5.0, 0.01, ""),
        VaryingNumber "ambient" "Ambient" = Control::num(0.1, 0.0, 1.0, 0.01, ""),
    ],
    // Both directions are normalized by hand against a guarded length: a mid-gray normal map
    // and three lamp coordinates at zero are each a vector of length zero, which `normalize`
    // leaves undefined, and a cable can drive either there.
    wgsl_common: "    let unpacked = ({normalMap}).rgb * 2.0 - 1.0;
    let normal = unpacked / max(length(unpacked), 1e-4);
    let lamp = vec3f({lightX}, {lightY}, {lightZ});
    let lightDir = lamp / max(length(lamp), 1e-4);
    let diffuse = max(dot(normal, lightDir), 0.0);
",
    outputs: [
        VaryingColor "color" "Color" = "    let surface = {surfaceColor};
    let ambient = clamp({ambient}, 0.0, 1.0);
    let lit = ambient + (1.0 - ambient) * diffuse;
    return vec4f(surface.rgb * ({lightColor}).rgb * lit, surface.a);",
        VaryingNumber "value" "Value" in "[0, 1]" = "    return diffuse;",
    ],
}
