// SPDX-License-Identifier: AGPL-3.0-or-later

//! Convolution edge detection.
//!
//! silvia builds the neighbor sample by string surgery on the generated expression —
//! `inputColor.replace('(uv)', '(uv_offset)')`. Here the offset coordinate is just what is
//! asked for: `ctx.input(node, "input", "uv_offset")`.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

pub static DEF: NodeDef = NodeDef {
    slug: "edgedetection",
    category: Category::Effect,
    icon: "🔪",
    label: "Edge Detection",
    tooltip: "Convolution edge detection. Sobel, Prewitt or Laplacian, each with a gray \
              variant. Sample Distance is how far the kernel reaches; at 0 every tap lands \
              on the same texel and the edges go away. There is no Invert row — an Invert \
              node one cable later gives dark lines on light.",
    // `sampleDistance`'s label does not fit the default 200.
    // See docs/decisions.md#a-node-may-declare-a-wider-body.
    width: Some(240.0),
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "strength",
            label: "Strength",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.0, 10.0, 0.01, ""),
        },
        InputDef {
            key: "sampleDistance",
            label: "Sample Distance",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.0, 20.0, 0.1, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        // The kernels are `var`s so the loop can index them by its counters.
        wgsl: |node, ctx, _func| {
            let strength = ctx.input(node, "strength", "uv");
            let distance = ctx.input(node, "sampleDistance", "uv");
            let neighbor = ctx.input(node, "input", "uv_offset");
            let mode = ctx.option(node, "mode").to_string();
            let step = if ctx.option(node, "kernel_space") == "uv" {
                format!("vec2f({distance})")
            } else {
                format!("vec2f({distance}) / {REFERENCE_HEIGHT:?}")
            };

            let combine = match mode.as_str() {
                "laplacian" => "    let edge = abs(lap);",
                "laplacian_gray" => "    let edge = vec3f(abs(lapGray));",
                "sobel_gray" | "prewitt_gray" => {
                    "    let edge = vec3f(sqrt(gxGray * gxGray + gyGray * gyGray));"
                }
                _ => "    let edge = sqrt(gx * gx + gy * gy);",
            };
            let kernels = match mode.as_str() {
                "prewitt" | "prewitt_gray" => {
                    "    var KX = mat3x3f(-1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    var KY = mat3x3f(-1.0, 0.0, 1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0);"
                }
                _ => {
                    "    var KX = mat3x3f(-1.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 1.0);
    var KY = mat3x3f(-1.0, 0.0, 1.0, -2.0, 0.0, 2.0, -1.0, 0.0, 1.0);"
                }
            };

            format!(
                "{kernels}
    var KL = mat3x3f(0.0, 1.0, 0.0, 1.0, -4.0, 1.0, 0.0, 1.0, 0.0);
    let kernelStep = {step};

    var gx = vec3f(0.0);
    var gy = vec3f(0.0);
    var lap = vec3f(0.0);
    var gxGray = 0.0;
    var gyGray = 0.0;
    var lapGray = 0.0;
    var centerAlpha = 1.0;

    for (var i = -1; i <= 1; i++) {{
        for (var j = -1; j <= 1; j++) {{
            let uv_offset = uv + vec2f(f32(i), f32(j)) * kernelStep;
            let sampled = {neighbor};
            if (i == 0 && j == 0) {{ centerAlpha = sampled.a; }}

            let wx = KX[i + 1][j + 1];
            let wy = KY[i + 1][j + 1];
            let wl = KL[i + 1][j + 1];
            let gray = dot(sampled.rgb, vec3f(0.299, 0.587, 0.114));

            gx += sampled.rgb * wx;
            gy += sampled.rgb * wy;
            lap += sampled.rgb * wl;
            gxGray += gray * wx;
            gyGray += gray * wy;
            lapGray += gray * wl;
        }}
    }}

{combine}
    return vec4f(clamp(edge * ({strength}), vec3f(0.0), vec3f(1.0)), centerAlpha);"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "mode",
            label: "Mode",
            default: "sobel",
            choices: &[
                ("sobel", "Sobel"),
                ("prewitt", "Prewitt"),
                ("laplacian", "Laplacian"),
                ("sobel_gray", "Sobel (Gray)"),
                ("prewitt_gray", "Prewitt (Gray)"),
                ("laplacian_gray", "Laplacian (Gray)"),
            ],
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "kernel_space",
            label: "Sample Space",
            default: "pixel",
            choices: &[("pixel", "Pixel"), ("uv", "UV")],
            ..OptionDef::EMPTY
        },
    ],
    ..NodeDef::EMPTY
};
