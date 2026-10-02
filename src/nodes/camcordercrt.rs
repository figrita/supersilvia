// SPDX-License-Identifier: AGPL-3.0-or-later

//! A camcorder pointed at its own monitor: the whole VHS/CRT look as one node, aimed from a
//! viewfinder.
//!
//! silvia's `camcordercrt.js`. Barrel curve, color fringing, scanlines and a phosphor tint,
//! a four-tap glow, brightness, vignette and the warm tape cast, and over that a feedback tap
//! the camera aims at the screen with a zoom, a roll, a perspective tilt and a drift.
//!
//! **The loop is a cable and there is no delay.** silvia taps whichever Output it was
//! compiled into and reads a chosen depth out of that Output's frame history, so the same
//! node is a different picture under two Outputs. Here `Last Frame` is an ordinary picture
//! input: cable an Output's Frame Out into it and the loop says where it comes from. There is
//! no history here and none is being built — silvia's FB Delay is gone with it, by decision: it was
//! an instantaneous delay with an old-frame input. See
//! [decisions.md](../../../docs/decisions.md).
//!
//! **Two of silvia's numbers are the resolution's and cannot be ported as they are.** No node
//! body reads `u_resolution`, so the glow's radius and the scanline pitch are measured against
//! [`REFERENCE_HEIGHT`] — the same distance at 720p, and the same field in every Output.
//! silvia's bezel darkens all four edges of the frame; worldspace is exactly 2.0 tall and as
//! wide as the Output made it, so only the top and the bottom are edges this node knows, and
//! those two are the ones it fades. The vignette and the barrel curve go round the worldspace
//! circle rather than silvia's aspect-normalized ellipse, exactly as the `vignette` node does.
//!
//! **A tap is only taken where it changes the picture.** Every read of `Input` is the whole
//! chain upstream evaluated again at another point, so silvia's seven reads a fragment — three
//! for the fringing, four for the glow — are seven Perlin noises when a Perlin is cabled in:
//! 6.1 ms a frame at 1280x720 on the Intel iGPU, when one noise on its own is 0.9. At Glow 0
//! the four glow taps would be scaled by zero, and at Aberration 0 the three fringe taps land
//! on the same point, so each set sits behind a test of its own number: one read with both
//! off, three with the fringing on, five with the glow on, seven with both. The picture is the
//! same to the bit either way; `tests/gpu_render.rs` holds silvia's seven-tap body and
//! compares the two over a feedback loop. See
//! [decisions.md](../../../docs/decisions.md#the-camcorder-and-the-flow-take-the-older-frame-as-a-cable-and-the-camcorder-is-aimed-in-a-region)
//! for what it costs when `Input` is already a texture.
//!
//! **The aiming numbers are ports with controls**, which is the rule here for anything silvia
//! saved beside a node, and the viewfinder writes them through the command bus — see
//! [`Region::Viewfinder`]. A sample the camera pushes past the edge of the frame
//! mirrors back in, which is what a loop this tight needs.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OutputDef, OutputKind, REFERENCE_HEIGHT, Region,
};

// The six aiming controls the viewfinder writes, which `widgets::viewfinder` reads from here.

/// How far the camera is pushed off center, in the frame's own units.
pub const DRIFT_X: &str = "fbDriftX";
pub const DRIFT_Y: &str = "fbDriftY";
/// How far the camera is angled relative to the screen it is pointed at.
pub const TILT_X: &str = "fbTiltX";
pub const TILT_Y: &str = "fbTiltY";
/// How much closer the camera is than the screen, and how far it is rolled.
pub const ZOOM: &str = "fbZoom";
pub const ROTATION: &str = "fbRotation";

pub static DEF: NodeDef = NodeDef {
    slug: "camcordercrt",
    category: Category::Effect,
    icon: "📼",
    label: "Camcorder CRT",
    tooltip: "The whole camcorder-pointed-at-a-monitor look in one node: barrel curvature, \
              color fringing, scanlines, phosphor glow, brightness and vignette, over a \
              feedback tap the camera aims. Cable an Output's Frame Out into Last Frame to \
              close the loop, then aim it in the viewfinder — drag to drift, shift-drag to \
              tilt, scroll to zoom, shift-scroll to rotate, double-click to reset. \
              Every pixel reads everything upstream once; Aberration above 0 reads it two \
              more times and Glow above 0 four more.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "lastFrame",
            label: "Last Frame",
            ty: VaryingColor,
            control: Control::color("#000000ff"),
        },
        InputDef {
            key: "curvature",
            label: "Curvature",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 2.0, 0.01, ""),
        },
        InputDef {
            key: "aberration",
            label: "Aberration",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 3.0, 0.01, ""),
        },
        InputDef {
            key: "scanlines",
            label: "Scanlines",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "glow",
            label: "Glow",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 2.0, 0.01, ""),
        },
        InputDef {
            key: "brightness",
            label: "Brightness",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.0, 2.0, 0.01, "x"),
        },
        InputDef {
            key: "vignette",
            label: "Vignette",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 2.0, 0.01, ""),
        },
        InputDef {
            key: "fbAmount",
            label: "Feedback",
            ty: VaryingNumber,
            control: Control::num(0.0, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "fbContrast",
            label: "FB Contrast",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.5, 2.0, 0.01, "x"),
        },
        InputDef {
            key: ZOOM,
            label: "Zoom",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.9, 1.2, 0.001, "x"),
        },
        InputDef {
            key: ROTATION,
            label: "Rotate",
            ty: VaryingNumber,
            control: Control::num(0.0, -0.5, 0.5, 0.001, "rad"),
        },
        InputDef {
            key: TILT_X,
            label: "Tilt X",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, ""),
        },
        InputDef {
            key: TILT_Y,
            label: "Tilt Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, ""),
        },
        InputDef {
            key: DRIFT_X,
            label: "Drift X",
            ty: VaryingNumber,
            control: Control::num(0.0, -0.1, 0.1, 0.001, ""),
        },
        InputDef {
            key: DRIFT_Y,
            label: "Drift Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -0.1, 0.1, 0.001, ""),
        },
    ],
    outputs: &[
        OutputDef {
            key: "color",
            label: "Color",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _func| [screen_wgsl(node, ctx), picture_wgsl(node, ctx)].concat(),
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "mask",
            label: "Mask",
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            range: Some("[0, 1]"),
            wgsl: |node, ctx, _func| {
                [
                    screen_wgsl(node, ctx),
                    "    return edgeFade * vigAmount;".to_string(),
                ]
                .concat()
            },
            ..OutputDef::EMPTY
        },
    ],
    regions: &[Region::Viewfinder],
    ..NodeDef::EMPTY
};

/// The glass: where the barrel curve reads from, and how much of the picture the tube leaves
/// there. Shared by the color and the mask, and it touches no input but its own two numbers —
/// so asking for the mask alone drags nothing into the shader that draws a picture.
fn screen_wgsl(node: NodeId, ctx: &mut CompileContext) -> String {
    let curvature = ctx.input(node, "curvature", "uv");
    let vignette = ctx.input(node, "vignette", "uv");
    format!(
        "    let curv = {curvature};
    let vig = {vignette};
    let r2 = dot(uv, uv);
    let crtUV = uv * (1.0 + r2 * curv * 0.3);
    let edgeFade = 1.0 - smoothstep(0.92, 1.0, abs(crtUV.y));
    let vigAmount = clamp(1.0 - r2 * vig * vig, 0.0, 1.0);
"
    )
}

/// Everything the tube does to a picture, in silvia's own order: fringing, glow, scanlines
/// and phosphor, brightness, vignette, the tape's warm cast, the bezel, and the camera.
///
/// The alpha is 1 from the first read to the last line, so the tube's work is carried as `rgb`
/// alone and the alpha written once at the end, rather than every step rebuilding a `vec4f`
/// around the same 1.
fn picture_wgsl(node: NodeId, ctx: &mut CompileContext) -> String {
    let aberration = ctx.input(node, "aberration", "uv");
    let scanlines = ctx.input(node, "scanlines", "uv");
    let glow = ctx.input(node, "glow", "uv");
    let brightness = ctx.input(node, "brightness", "uv");
    let fb_amount = ctx.input(node, "fbAmount", "uv");
    let fb_contrast = ctx.input(node, "fbContrast", "uv");
    let zoom = ctx.input(node, ZOOM, "uv");
    let rotation = ctx.input(node, ROTATION, "uv");
    let tilt_x = ctx.input(node, TILT_X, "uv");
    let tilt_y = ctx.input(node, TILT_Y, "uv");
    let drift_x = ctx.input(node, DRIFT_X, "uv");
    let drift_y = ctx.input(node, DRIFT_Y, "uv");
    let red = ctx.input(node, "input", "abLow");
    let green = ctx.input(node, "input", "crtUV");
    let blue = ctx.input(node, "input", "abHigh");
    let east = ctx.input(node, "input", "crtUV + vec2f(glowRadius, 0.0)");
    let west = ctx.input(node, "input", "crtUV - vec2f(glowRadius, 0.0)");
    let north = ctx.input(node, "input", "crtUV + vec2f(0.0, glowRadius)");
    let south = ctx.input(node, "input", "crtUV - vec2f(0.0, glowRadius)");
    let older = ctx.input(node, "lastFrame", "fbUV");
    format!(
        "    let aber = {aberration};
    let scan = {scanlines};
    let glw = {glow};

    let abDist = length(crtUV);
    let abDir = select(vec2f(1.0, 0.0), crtUV / abDist, abDist > 0.001);
    let abOffset = aber * abDist * 0.04;
    let abLow = crtUV - abDir * abOffset;
    let abHigh = crtUV + abDir * abOffset;
    var rgb: vec3f;
    if (aber != 0.0) {{
        rgb = vec3f(({red}).r, ({green}).g, ({blue}).b);
    }} else {{
        rgb = ({green}).rgb;
    }}

    if (glw != 0.0) {{
        let glowRadius = 6.0 / {REFERENCE_HEIGHT:?};
        let bloomSample = ({east} + {west} + {north} + {south}) * 0.25;
        rgb += max(bloomSample.rgb - rgb, vec3f(0.0)) * glw;
    }}

    let pixelY = (crtUV.y + 1.0) * 0.5 * {REFERENCE_HEIGHT:?};
    let scanLine = pow(sin(pixelY * PI) * 0.5 + 0.5, 1.5);
    rgb *= mix(vec3f(1.0), vec3f(scanLine), scan);

    let pixelX = (crtUV.x + 1.0) * 0.5 * {REFERENCE_HEIGHT:?};
    let subpixel = floor_mod(floor(pixelX), 3.0);
    let phosphorTint = vec3f(
        select(0.85, 1.0, subpixel == 0.0),
        select(0.85, 1.0, subpixel == 1.0),
        select(0.85, 1.0, subpixel == 2.0)
    );
    rgb *= mix(vec3f(1.0), phosphorTint, scan * 0.5);

    rgb *= {brightness};
    rgb *= vigAmount;
    rgb.r *= 1.05;
    rgb.b *= 0.92;
    rgb *= edgeFade;

    var camera = uv * 0.5;
    camera /= {zoom};
    let roll = {rotation};
    let cr = cos(roll);
    let sr = sin(roll);
    camera = vec2f(camera.x * cr - camera.y * sr, camera.x * sr + camera.y * cr);
    let pw = max(1.0 + camera.x * ({tilt_x}) + camera.y * ({tilt_y}), 0.001);
    camera /= pw;
    camera += vec2f({drift_x}, {drift_y});
    let fbUV = camera * 2.0;

    let prevFrame = {older};
    let prevRgb = (prevFrame.rgb - 0.5) * ({fb_contrast}) + 0.5;
    rgb += prevRgb * ({fb_amount});

    return vec4f(rgb, 1.0);"
    )
}
