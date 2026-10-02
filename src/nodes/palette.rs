// SPDX-License-Identifier: AGPL-3.0-or-later

//! One color in, eight that go with it out.
//!
//! Ported from silvia's `palette.js`. The input is read into OKLab, turned to polar — a
//! chroma and a hue — and each of the eight outputs is that pixel with its hue moved along
//! an arc, its lightness and its chroma fanned around the original. It reads the one texel
//! under it, so a picture in gives eight recolored versions of that picture rather than
//! eight swatches, and it is `Category::Color`.
//!
//! silvia has no table of preset palettes behind this: the eight are computed from the
//! input color and the five knobs, so nothing here is baked at compile time and every knob
//! takes a cable.
//!
//! The eight outputs are `a` through `h`, which is the one set of port names in the library
//! outside the vocabulary in [nodes.md](../../../docs/nodes.md). They are deliberate:
//! nothing distinguishes the eight but their place along the arc, so there is no name for
//! any one of them that is not its index.
//!
//! `SRGB2OKLAB_WGSL` and `OKLAB2SRGB_WGSL` are silvia's own, `pub` the way `hsla`'s
//! `HSL2RGB_WGSL` is, so
//! the next node wanting a perceptual space reaches these rather than inlining a copy. Both
//! use 2.2 as the transfer curve, as silvia does, rather than the piecewise sRGB one.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::node;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

/// How many colors come out, and so how far apart along the arc they sit.
const SLOTS: usize = 8;

/// One slot's body, with `$T` its place from 0 to 1 across the eight and `$TC` the same
/// place centered, -1 at A and +1 at H.
const SLOT_WGSL: &str = "    let src = {input};
    var lab = srgb2oklab(src.rgb);
    var C = length(lab.yz);
    var H = atan2(lab.z, lab.y);
    let shaped = pow($T, max({curve}, 0.01));
    H += ({shift}) * 6.28318530718 + (shaped - 0.5) * ({spread}) * 6.28318530718;
    lab.x = clamp(lab.x + ($TC) * ({lSpread}) * 0.5, 0.0, 1.0);
    C = max(C + ($TC) * ({cSpread}) * 0.2, 0.0);
    lab = vec3f(lab.x, C * cos(H), C * sin(H));
    return vec4f(oklab2srgb(lab), src.a);";

/// sRGB to OKLab. silvia's `shaderUtils.SRGB2OKLAB`.
pub const SRGB2OKLAB_WGSL: &str = "fn srgb2oklab(c: vec3f) -> vec3f {
    let lin = pow(max(c, vec3f(0.0)), vec3f(2.2));
    var l = 0.4122214708 * lin.r + 0.5363325363 * lin.g + 0.0514459929 * lin.b;
    var m = 0.2119034982 * lin.r + 0.6806995451 * lin.g + 0.1073969566 * lin.b;
    var s = 0.0883024619 * lin.r + 0.2817188376 * lin.g + 0.6299787005 * lin.b;
    l = pow(max(l, 0.0), 1.0 / 3.0);
    m = pow(max(m, 0.0), 1.0 / 3.0);
    s = pow(max(s, 0.0), 1.0 / 3.0);
    return vec3f(
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s
    );
}";

/// OKLab back to sRGB. silvia's `shaderUtils.OKLAB2SRGB`.
pub const OKLAB2SRGB_WGSL: &str = "fn oklab2srgb(lab: vec3f) -> vec3f {
    var l = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    var m = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    var s = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    l = l * l * l;
    m = m * m * m;
    s = s * s * s;
    let lin = vec3f(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    );
    return pow(clamp(lin, vec3f(0.0), vec3f(1.0)), vec3f(1.0 / 2.2));
}";

/// [`SLOT_WGSL`] with this slot's two constants in it.
fn slot_wgsl(index: usize) -> String {
    let t = index as f64 / (SLOTS - 1) as f64;
    // The centered one first: `$T` is a prefix of `$TC`.
    SLOT_WGSL
        .replace("$TC", &format!("{:.6}", t * 2.0 - 1.0))
        .replace("$T", &format!("{t:.6}"))
}

node! {
    /// One color read into OKLCH and fanned out into eight that belong together.
    PALETTE,
    slug: "palette",
    icon: "🎨",
    label: "Palette",
    category: Color,
    tooltip: "Turns one color into eight that go together, per pixel. Spread opens the hue \
              arc, Shift rotates the set, Curve clusters them near the input or pushes them \
              apart, and L and C Spread fan lightness and chroma across A to H.",
    inputs: [
        VaryingColor "input" "Input" = Control::color("#4488ccff"),
        VaryingNumber "spread" "Spread" = Control::num(1.0, 0.0, 4.0, 0.01, ""),
        VaryingNumber "shift" "Shift" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "curve" "Curve" = Control::num(1.0, 0.1, 4.0, 0.01, ""),
        VaryingNumber "lSpread" "L Spread" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "cSpread" "C Spread" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
    ],
    wgsl_utils: [SRGB2OKLAB_WGSL, OKLAB2SRGB_WGSL],
    outputs: [
        VaryingColor "a" "A" = slot_wgsl(0),
        VaryingColor "b" "B" = slot_wgsl(1),
        VaryingColor "c" "C" = slot_wgsl(2),
        VaryingColor "d" "D" = slot_wgsl(3),
        VaryingColor "e" "E" = slot_wgsl(4),
        VaryingColor "f" "F" = slot_wgsl(5),
        VaryingColor "g" "G" = slot_wgsl(6),
        VaryingColor "h" "H" = slot_wgsl(7),
    ],
}
