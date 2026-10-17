// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hue, saturation, lightness and alpha into a color, premultiplied as it is built, as
//! `rgba`'s is, and dual as `rgba`'s is: one color a frame from knobs and numbers, a color list
//! from lists, a picture from fields. [`hsl2rgb`] is the WGSL's twin, held to it by a test.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

/// HSL to RGB. silvia inlines this per node with a chain of branches; here it is a shader
/// util, so a graph with several HSLA nodes emits it once.
pub const HSL2RGB_WGSL: &str = "fn hsl2rgb_channel(p: f32, q: f32, t0: f32) -> f32 {
    var t = t0;
    if (t < 0.0) { t += 1.0; }
    if (t > 1.0) { t -= 1.0; }
    if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
    if (t < 1.0 / 2.0) { return q; }
    if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl2rgb(h: f32, s: f32, l: f32) -> vec3f {
    if (s == 0.0) { return vec3f(l); }
    let q = select(l + s - l * s, l * (1.0 + s), l < 0.5);
    let p = 2.0 * l - q;
    return vec3f(
        hsl2rgb_channel(p, q, h + 1.0 / 3.0),
        hsl2rgb_channel(p, q, h),
        hsl2rgb_channel(p, q, h - 1.0 / 3.0)
    );
}";

/// [`HSL2RGB_WGSL`]'s `hsl2rgb` in Rust, for the color a tick works out.
pub fn hsl2rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    fn channel(p: f32, q: f32, t0: f32) -> f32 {
        let mut t = t0;
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    }
    if s == 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [
        channel(p, q, h + 1.0 / 3.0),
        channel(p, q, h),
        channel(p, q, h - 1.0 / 3.0),
    ]
}

pub static DEF: NodeDef = NodeDef {
    slug: "hsla",
    category: Category::Color,
    icon: "🌈",
    label: "HSLA",
    tooltip: "Combines hue, saturation, lightness and alpha into a color.",
    wgsl_utils: &[HSL2RGB_WGSL],
    inputs: &[
        InputDef {
            key: "h",
            label: "Hue",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "s",
            label: "Saturation",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "l",
            label: "Lightness",
            ty: VaryingNumber,
            control: Control::num(0.5, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "a",
            label: "Alpha",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.0, 1.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            format!(
                "    return premultiply(vec4f(hsl2rgb({}, {}, {}), {}));",
                ctx.input(node, "h", "uv"),
                ctx.input(node, "s", "uv"),
                ctx.input(node, "l", "uv"),
                ctx.input(node, "a", "uv"),
            )
        },
        eval_color: Some(|node, ctx| {
            let v = |k| ctx.input(node, k) as f32;
            let [r, g, b] = hsl2rgb(v("h"), v("s"), v("l"));
            [r, g, b, v("a")]
        }),
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};

#[cfg(test)]
mod tests {
    use super::hsl2rgb;

    #[test]
    fn the_primaries_and_a_gray() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(near(hsl2rgb(0.0, 1.0, 0.5), [1.0, 0.0, 0.0]));
        assert!(near(hsl2rgb(1.0 / 3.0, 1.0, 0.5), [0.0, 1.0, 0.0]));
        assert!(near(hsl2rgb(2.0 / 3.0, 1.0, 0.5), [0.0, 0.0, 1.0]));
        assert!(near(hsl2rgb(0.3, 0.0, 0.25), [0.25; 3]));
    }
}
