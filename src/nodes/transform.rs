// SPDX-License-Identifier: AGPL-3.0-or-later

//! UV transforms: nodes that resample their input at a different coordinate.
//!
//! These are the first nodes to pass a custom `uv` expression to `ctx.input`. The value that
//! comes back is `source_func(zoomedUV)` rather than `source_func(uv)`, so the transform
//! composes into the caller's shader with no extra machinery.
//!
//! `zoom`, `rotate` and `fisheye` are ported from silvia's `zoom.js`, `rotate.js` and
//! `fisheye.js`; the seven below them from `translate.js`, `mirror.js`, `stretchskew.js`,
//! `perspective.js`, `polarcoords.js`, `rotozoom.js` and `shakycam.js`. None of the seven
//! computes a field on its way to the picture — each one builds a coordinate and hands it to
//! its input — so none publishes anything beside `output`.
//!
//! Three departures from the source. **`mirror` drops silvia's ninth mode**, Quadrant, which
//! emits the same fold as Mirror X+Y, and gains the center pair `zoom`, `rotate` and
//! `fisheye` already share, so the mirror line is placeable; at the default center it is
//! silvia's. **`mirror` and `polarcoords` guard what silvia leaves open** — the polar scale
//! divides, and a control's range excludes zero where a cabled field does not.
//! **`rotozoom` and `shakycam` read Time and Offset in their own cycle**, where silvia reads
//! `u_time` inside the body, and a cycle is the least time in which every wave she runs comes
//! back: on Rotozoom and on Shaky Cam's X the 20π over which her rates line up, which Speed 1 or
//! ambient time runs at one a minute, her speeds of one rounded to whole seconds; on Shaky Cam's
//! Y the quarter of that over which her 0.8 and 1.2 line up, at four a minute, so the shake on
//! screen is hers. Rotozoom's **Turns**, a whole number, is how many turns one cycle makes, so
//! the node still comes back on every cycle; a sign turns it the other way and zero is zoom
//! alone. A coefficient at zero stills its waves, and what moves then may come back within a
//! cycle, which the period says: half a cycle on Rotozoom with its cosines off and Turns even. See
//! `docs/decisions.md#silvias-other-seven-transforms-and-the-two-that-keep-time`.

use crate::graph::PortType::{UniformNumber, VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, Timing,
};

/// The center offset every transform shares.
macro_rules! center_inputs {
    () => {
        InputDef {
            key: "centerX",
            label: "Center X",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        }
    };
}

pub static ZOOM: NodeDef = NodeDef {
    slug: "zoom",
    category: Category::Transform,
    icon: "🔎",
    label: "Zoom",
    tooltip: "Scales the input. Above 1 zooms in, below 1 zooms out.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        // Log scrub: this range spans four decades, and a linear drag would spend almost all
        // of its travel below 1.
        InputDef {
            key: "zoom",
            label: "Zoom",
            ty: VaryingNumber,
            control: Control::num_log(1.0, 0.01, 100.0, 0.01, "x"),
        },
        center_inputs!(),
        InputDef {
            key: "centerY",
            label: "Center Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let cx = ctx.input(node, "centerX", "uv");
            let cy = ctx.input(node, "centerY", "uv");
            let zoom = ctx.input(node, "zoom", "uv");
            let source = ctx.input(node, "input", "zoomedUV");
            format!(
                "    let center = vec2f({cx}, {cy});
    var zoomAmount = ({zoom});
    if (abs(zoomAmount) < 1e-6) {{ zoomAmount = select(1e-6, -1e-6, zoomAmount < 0.0); }}
    let zoomedUV = (uv - center) / zoomAmount + center;
    return {source};"
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};

pub static ROTATE: NodeDef = NodeDef {
    slug: "rotate",
    category: Category::Transform,
    icon: "🔄",
    label: "Rotate",
    tooltip: "Rotates the input around a center. Angle in turns: 1.0 is a full turn, 0.5 \
              is 180°, positive clockwise and negative counter-clockwise.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "angle",
            label: "Angle",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        },
        center_inputs!(),
        InputDef {
            key: "centerY",
            label: "Center Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        // `mat2x2f` takes its arguments column by column.
        wgsl: |node, ctx, _func| {
            let angle = ctx.input(node, "angle", "uv");
            let cx = ctx.input(node, "centerX", "uv");
            let cy = ctx.input(node, "centerY", "uv");
            let source = ctx.input(node, "input", "rotatedUV");
            format!(
                "    let angleRad = ({angle}) * 2.0 * PI;
    let center = vec2f({cx}, {cy});
    let s = sin(angleRad);
    let c = cos(angleRad);
    let rotatedUV = mat2x2f(c, -s, s, c) * (uv - center) + center;
    return {source};"
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};

pub static FISHEYE: NodeDef = NodeDef {
    slug: "fisheye",
    category: Category::Transform,
    icon: "🐠",
    label: "Fisheye Lens",
    tooltip: "Lens distortion. Positive barrels, negative pincushions.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "distortion",
            label: "Distortion",
            ty: VaryingNumber,
            control: Control::num(0.5, -1.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "radius",
            label: "Radius",
            ty: VaryingNumber,
            control: Control::num(1.0, 0.01, 3.0, 0.01, "⬓"),
        },
        center_inputs!(),
        InputDef {
            key: "centerY",
            label: "Center Y",
            ty: VaryingNumber,
            control: Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let cx = ctx.input(node, "centerX", "uv");
            let cy = ctx.input(node, "centerY", "uv");
            let distortion = ctx.input(node, "distortion", "uv");
            let radius = ctx.input(node, "radius", "uv");
            let straight = ctx.input(node, "input", "uv");
            let distorted = ctx.input(node, "input", "distortedUV");
            format!(
                "    let center = vec2f({cx}, {cy});
    let centered = uv - center;
    let r = length(centered);
    if (r == 0.0) {{ return {straight}; }}

    let lensRadius = max(({radius}), 1e-6);
    let normalizedR = r / lensRadius;
    let amount = ({distortion}) * (1.0 - smoothstep(0.7, 1.0, normalizedR));
    let distortedR = pow(normalizedR, 1.0 + amount) * lensRadius;
    let distortedUV = centered * (distortedR / r) + center;
    return {distorted};"
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};

// ------------------------------------------------------------------------------- translate

node! {
    /// The coordinate slid by an offset.
    TRANSLATE,
    slug: "translate",
    // silvia's `✥` has no glyph in the vendored faces; this is the nearest that does.
    icon: "⬄",
    label: "Translate",
    category: Transform,
    tooltip: "Slides the input across the frame. The offset is subtracted from the sampling \
              coordinate, so a positive X moves the picture left and a positive Y moves it \
              down.",
    inputs: [
        VaryingColor "input" "Input" at "translatedUV" = Control::None,
        VaryingNumber "x" "X Offset" = Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
        VaryingNumber "y" "Y Offset" = Control::num(0.0, -1.0, 1.0, 0.01, "⬓"),
    ],
    wgsl_common: "    let translatedUV = uv - vec2f({x}, {y});
",
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

// ---------------------------------------------------------------------------------- mirror

/// The fold or the flip one mode asks for, written on `folded`, the coordinate measured from
/// the mirror's own center, which is a `var` in the body these lines land in.
fn mirror_fold(mode: &str) -> &'static str {
    match mode {
        "mirror_y" => "    folded.y = abs(folded.y);\n",
        "mirror_xy" => "    folded = abs(folded);\n",
        "flip_x" => "    folded.x = -folded.x;\n",
        "flip_y" => "    folded.y = -folded.y;\n",
        "flip_xy" => "    folded = -folded;\n",
        "mirror_x_flip_y" => "    folded.x = abs(folded.x);\n    folded.y = -folded.y;\n",
        "mirror_y_flip_x" => "    folded.y = abs(folded.y);\n    folded.x = -folded.x;\n",
        _ => "    folded.x = abs(folded.x);\n",
    }
}

node! {
    /// A fold or a flip about a line through the center, slid in by a mix of the two
    /// coordinates.
    MIRROR,
    slug: "mirror",
    icon: "🪞",
    label: "Mirror",
    category: Transform,
    tooltip: "Folds or flips the input about a line through the center. Mix slides the \
              coordinate from the untouched one to the folded one rather than blending two \
              pictures, so the picture walks into its own reflection.",
    // "Mirror Y + Flip X" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "finalUV" = Control::None,
        VaryingNumber "mix" "Mix" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "mode" "Mode" = "mirror_x" [
            "mirror_x" => "Mirror X",
            "mirror_y" => "Mirror Y",
            "mirror_xy" => "Mirror X+Y",
            "flip_x" => "Flip X",
            "flip_y" => "Flip Y",
            "flip_xy" => "Flip X+Y",
            "mirror_x_flip_y" => "Mirror X + Flip Y",
            "mirror_y_flip_x" => "Mirror Y + Flip X",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            "    let center = vec2f({centerX}, {centerY});
    var folded = uv - center;
",
            mirror_fold(ctx.option(node, "mode")),
            "    let finalUV = mix(uv, folded + center, {mix});
",
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

// ---------------------------------------------------------------------------- stretch/skew

node! {
    /// A scale per axis and a shear, both written on the sampling coordinate.
    STRETCHSKEW,
    slug: "stretchskew",
    // silvia's `⚟` has no glyph in the vendored faces; a leaning box says the same thing.
    icon: "▱",
    label: "Stretch/Skew",
    category: Transform,
    tooltip: "Scales each axis on its own and leans the picture over. The numbers read \
              backwards, because they scale the sampling coordinate: a Stretch X above 1 \
              squashes the picture rather than widening it.",
    inputs: [
        VaryingColor "input" "Input" at "skewedUV" = Control::None,
        VaryingNumber "stretchX" "Stretch X" = Control::num(1.0, 0.1, 3.0, 0.01, ""),
        VaryingNumber "stretchY" "Stretch Y" = Control::num(1.0, 0.1, 3.0, 0.01, ""),
        VaryingNumber "skewX" "Skew X" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
        VaryingNumber "skewY" "Skew Y" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
    ],
    // Both skews read the stretched coordinate rather than each other, which is silvia's
    // order: one shear applied at once, not two composed.
    wgsl_common: "    let stretchedUV = vec2f(uv.x * ({stretchX}), uv.y * ({stretchY}));
    let skewedUV = vec2f(
        stretchedUV.x + stretchedUV.y * ({skewX}),
        stretchedUV.y + stretchedUV.x * ({skewY}));
",
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

// ----------------------------------------------------------------------------- perspective

node! {
    /// The coordinate divided by a plane that leans with two tilts.
    PERSPECTIVE,
    slug: "perspective",
    icon: "⏢",
    label: "Perspective",
    category: Transform,
    tooltip: "Lays the picture back into depth. The sampling coordinate is divided by a plane \
              that leans with the two tilts, so one edge runs away from you and the other \
              comes forward.",
    inputs: [
        VaryingColor "input" "Input" at "tiltedUV" = Control::None,
        VaryingNumber "tiltX" "Tilt X" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "tiltY" "Tilt Y" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
    ],
    // silvia's own guard, kept: past the horizon the divisor crosses zero and turns negative,
    // which folds the picture through itself rather than sending it away.
    wgsl_common: "    let w = max(1.0 + uv.x * ({tiltX}) + uv.y * ({tiltY}), 0.001);
    let tiltedUV = uv / w;
",
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

// ----------------------------------------------------------------------- polar coordinates

/// The remap one mode asks for, written on `sampleUV`. `center` and `scale` are in scope.
fn polar_remap_wgsl(mode: &str) -> &'static str {
    match mode {
        "from_polar" => {
            "    let theta = uv.x * PI;
    let r = (uv.y + 1.0) * 0.5 * scale;
    let sampleUV = vec2f(cos(theta), sin(theta)) * r + center;
"
        }
        "to_log_polar" => {
            "    let cuv = uv - center;
    let logR = log(max(length(cuv), 0.001)) / scale;
    let theta = atan2(cuv.y, cuv.x) / PI;
    let sampleUV = vec2f(theta, logR);
"
        }
        // The angle read through a cosine rather than straight across, so the input's width runs
        // out along the top and back along the bottom and turns at each end with no slope: one
        // sample, a curved mirror where the straight mapping cuts the input's two edges together.
        "to_polar_smooth" => {
            "    let cuv = uv - center;
    let r = length(cuv) / scale;
    let theta = cos(atan2(cuv.y, cuv.x));
    let sampleUV = vec2f(theta, r * 2.0 - 1.0);
"
        }
        "to_log_polar_smooth" => {
            "    let cuv = uv - center;
    let logR = log(max(length(cuv), 0.001)) / scale;
    let theta = cos(atan2(cuv.y, cuv.x));
    let sampleUV = vec2f(theta, logR);
"
        }
        "from_log_polar" => {
            "    let theta = uv.x * PI;
    let r = exp(uv.y * scale);
    let sampleUV = vec2f(cos(theta), sin(theta)) * r + center;
"
        }
        _ => {
            "    let cuv = uv - center;
    let r = length(cuv) / scale;
    let theta = atan2(cuv.y, cuv.x) / PI;
    let sampleUV = vec2f(theta, r * 2.0 - 1.0);
"
        }
    }
}

node! {
    /// The frame turned inside out between straight and round coordinates, four ways.
    POLARCOORDS,
    slug: "polarcoords",
    icon: "🎯",
    label: "Polar Coordinates",
    category: Transform,
    tooltip: "Remaps between straight and round coordinates. Into polar, a row of stripes \
              becomes a sunburst; back out of it, a sunburst flattens into stripes. The \
              log-polar pair turns a repeating pattern into a spiral.",
    // "Cartesian → Log-Polar" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "input" "Input" at "sampleUV" = Control::None,
        VaryingNumber "scale" "Scale" = Control::num(1.0, 0.1, 5.0, 0.01, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "mode" "Mode" = "to_polar" [
            "to_polar" => "Cartesian → Polar",
            "to_polar_smooth" => "Cartesian → Polar, Smooth",
            "from_polar" => "Polar → Cartesian",
            "to_log_polar" => "Cartesian → Log-Polar",
            "to_log_polar_smooth" => "Cartesian → Log-Polar, Smooth",
            "from_log_polar" => "Log-Polar → Cartesian",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            "    let center = vec2f({centerX}, {centerY});
    var scale = {scale};
    if (abs(scale) < 1e-6) { scale = select(1e-6, -1e-6, scale < 0.0); }
",
            polar_remap_wgsl(ctx.option(node, "mode")),
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

// -------------------------------------------------------------------------------- rotozoom

node! {
    /// A turn and a scale riding on the same two waves at different rates.
    ROTOZOOM,
    slug: "rotozoom",
    icon: "🪂",
    label: "Rotozoom",
    category: Transform,
    tooltip: "Turns and scales the input on one pair of interlocking waves, which is what \
              makes the two breathe together. Base Zoom multiplies the sampling coordinate, \
              so a larger one pulls the picture away — the opposite sense of the Zoom node.",
    // "Sin Coefficient" does not fit the default 200.
    width: 240.0,
    timing: Timing::repeating(1.0 / 60.0, |n| Some(rotozoom_period(n))),
    inputs: [
        VaryingColor "input" "Input" at "rotozoomedUV" = Control::None,
    ],
    after_time: [
        UniformNumber "turns" "Turns" = Control::num(5.0, -10.0, 10.0, 1.0, ""),
        VaryingNumber "sinCoeff" "Sin Coefficient" = Control::num(1.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "cosCoeff" "Cos Coefficient" = Control::num(1.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "baseZoom" "Base Zoom" = Control::num(1.0, 0.1, 3.0, 0.01, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // One cycle is silvia's 20π of time, over which her 1, 0.7, 0.8 and 1.2 line up, and in
    // which her half-rate angle at equal speeds turns five times: Turns, a whole number, so
    // the turn and the waves come back together every cycle. Both are wrapped to one cycle
    // before they are scaled, so an `f32` keeps its step however long the show. Zoom is
    // clamped positive, as silvia clamps it: a cabled coefficient reaches past the knob's
    // range, and a negative scale turns the picture inside out.
    wgsl_common: "    let cycle = time_periodic({clock}, {phaseOffset});
    let wave = fract(cycle) * 20.0 * PI;
    var angle = fract(cycle * round({turns})) * 2.0 * PI;
    angle += sin(wave) * ({sinCoeff}) * 0.5;
    angle += cos(wave * 0.7) * ({cosCoeff}) * 0.3;
    var zoom = {baseZoom};
    zoom += sin(wave * 0.8) * ({sinCoeff}) * 0.3;
    zoom += cos(wave * 1.2) * ({cosCoeff}) * 0.2;
    zoom = max(0.1, zoom);
    let c = cos(angle);
    let s = sin(angle);
    let center = vec2f({centerX}, {centerY});
    let rotozoomedUV = mat2x2f(c, -s, s, c) * (uv - center) * zoom + center;
",
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

/// How many times a cycle each of Rotozoom's four waves runs: the angle's sine and cosine,
/// then the zoom's, the sines under Sin Coefficient and the cosines under Cos Coefficient.
const ROTOZOOM_WAVES: [u64; 4] = [10, 7, 8, 12];

/// Rotozoom's period: one cycle, or a half where the cosines are off and Turns is even, or
/// `1 / |Turns|` where both coefficients are — the least shift that brings back every wave that
/// moves and the turn. Turns is read rounded as the shader rounds it, half to even, and a
/// Turns a cable drives as any whole number.
fn rotozoom_period(node: &crate::graph::Node) -> f64 {
    use crate::nodes::timing::{known, period_of_waves};
    let [sin_angle, cos_angle, sin_zoom, cos_zoom] = ROTOZOOM_WAVES;
    let mut waves = Vec::with_capacity(5);
    if known(node, "sinCoeff") != Some(0.0) {
        waves.extend([sin_angle, sin_zoom]);
    }
    if known(node, "cosCoeff") != Some(0.0) {
        waves.extend([cos_angle, cos_zoom]);
    }
    waves.push(known(node, "turns").map_or(1, |t| t.round_ties_even().abs() as u64));
    period_of_waves(&waves)
}

// ------------------------------------------------------------------------------- shaky cam

/// How many times a cycle each of Shaky Cam's waves runs on its own axis, sine then cosine: X's
/// silvia's 1 and 0.7 over 20π, Y's her 0.8 and 1.2 over a quarter of that, where they line up.
const SHAKY_X_WAVES: [u64; 2] = [10, 7];
const SHAKY_Y_WAVES: [u64; 2] = [2, 3];

/// An axis of Shaky Cam's period: one of its cycles, or the sine's or the cosine's own where the
/// other's coefficient is zero. Standing still — no amplitude, both coefficients zero — it
/// claims one cycle.
fn shaky_period(node: &crate::graph::Node, [sine, cosine]: [u64; 2]) -> f64 {
    use crate::nodes::timing::{known, period_of_waves};
    if known(node, "amplitude") == Some(0.0) {
        return 1.0;
    }
    let mut waves = Vec::with_capacity(2);
    if known(node, "sinCoeff") != Some(0.0) {
        waves.push(sine);
    }
    if known(node, "cosCoeff") != Some(0.0) {
        waves.push(cosine);
    }
    period_of_waves(&waves)
}

node! {
    /// A handheld wobble: four fixed waves, two per axis, under one amplitude.
    SHAKYCAM,
    slug: "shakycam",
    icon: "🤳",
    label: "Shaky Cam",
    category: Transform,
    tooltip: "Slides the picture on four fixed waves, two per axis at rates that do not line \
              up. It is deterministic rather than noisy, which is what makes it read as a \
              handheld camera rather than as static. Each axis has a Time, a Speed and an \
              Offset of its own, so Y can shake alone, at a speed or on a gear of its own.",
    // "Sin Coefficient" does not fit the default 200.
    width: 240.0,
    timing_xy: Timing::repeating(1.0 / 60.0, |n| Some(shaky_period(n, SHAKY_X_WAVES)))
        .y(1.0 / 15.0, |n| Some(shaky_period(n, SHAKY_Y_WAVES))),
    inputs: [
        VaryingColor "input" "Input" at "shakenUV" = Control::None,
    ],
    after_time: [
        VaryingNumber "sinCoeff" "Sin Coefficient" = Control::num(1.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "cosCoeff" "Cos Coefficient" = Control::num(1.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "amplitude" "Amplitude" = Control::num(0.1, 0.0, 1.0, 0.01, ""),
    ],
    // Each axis turns through its own cycle, the waves running whole times in it: X 10 and 7,
    // Y 2 and 3 (`SHAKY_X_WAVES`, `SHAKY_Y_WAVES`). Each axis reads a Time and an Offset of
    // its own, so Y can run alone or on a gear of its own.
    wgsl_common: "    let tx = fract(time_periodic({clock}, {phaseOffset})) * 2.0 * PI;
    let ty = fract(time_periodic({clockY}, {phaseOffsetY})) * 2.0 * PI;
    let amplitude = {amplitude};
    var xOffset = sin(tx * 10.0) * ({sinCoeff}) * amplitude;
    xOffset += cos(tx * 7.0) * ({cosCoeff}) * amplitude * 0.5;
    var yOffset = sin(ty * 2.0) * ({sinCoeff}) * amplitude;
    yOffset += cos(ty * 3.0) * ({cosCoeff}) * amplitude * 0.5;
    let shakenUV = uv + vec2f(xOffset, yOffset);
",
    outputs: [
        VaryingColor "output" "Output" = "    return {input};",
    ],
}

#[cfg(test)]
mod tests {
    use crate::graph::{ControlValue, Graph, PortRef};
    use crate::nodes::timing::{Axis, period_in};

    fn node(g: &mut Graph, slug: &str, controls: &[(&'static str, f32)]) -> crate::graph::NodeId {
        let id = crate::nodes::add_to_graph(g, slug, emath::Pos2::ZERO).unwrap();
        for (key, value) in controls {
            g.get_mut(id)
                .unwrap()
                .controls
                .insert(key, ControlValue::Float(*value));
        }
        id
    }

    fn period(g: &Graph, id: crate::graph::NodeId, axis: Axis) -> Option<f64> {
        let n = g.get(id).unwrap();
        n.def.timing.unwrap().period_of(n, axis)
    }

    /// **Rotozoom comes back every cycle**, and sooner where a coefficient at zero stills its
    /// waves: with the cosines off its sines run 10 and 8 a cycle, so an even Turns brings it
    /// back in half a cycle and an odd one does not; with both off it is the turn alone,
    /// `1 / |Turns|`. Turns is read as the shader rounds it, half to even.
    #[test]
    fn rotozoom_reports_the_shorter_repeat_a_coefficient_at_zero_leaves() {
        let mut g = Graph::new();
        for (controls, want) in [
            (&[][..], 1.0),
            (&[("turns", 0.0)][..], 1.0),
            (&[("sinCoeff", 0.0)][..], 1.0),
            (&[("cosCoeff", 0.0)][..], 1.0),
            (&[("cosCoeff", 0.0), ("turns", 0.0)][..], 0.5),
            (&[("cosCoeff", 0.0), ("turns", 2.0)][..], 0.5),
            (&[("cosCoeff", 0.0), ("turns", -4.0)][..], 0.5),
            (&[("cosCoeff", 0.0), ("turns", 2.5)][..], 0.5),
            (&[("cosCoeff", 0.0), ("turns", 3.5)][..], 0.5),
            (
                &[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 3.0)][..],
                1.0 / 3.0,
            ),
            (
                &[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", -10.0)][..],
                0.1,
            ),
            (
                &[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 1.0)][..],
                1.0,
            ),
            (
                &[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 0.0)][..],
                1.0,
            ),
        ] {
            let id = node(&mut g, "rotozoom", controls);
            assert_eq!(period(&g, id, Axis::X), Some(want), "{controls:?}");
        }
    }

    /// **Each of Shaky Cam's axes comes back every cycle of its own**, and with one coefficient
    /// at zero after the other's wave alone: X's sine every tenth and its cosine every seventh,
    /// Y's every half and every third.
    #[test]
    fn shaky_cam_reports_each_axis_its_own_shorter_repeat() {
        let mut g = Graph::new();
        for (controls, x, y) in [
            (&[][..], 1.0, 1.0),
            (&[("cosCoeff", 0.0)][..], 0.1, 0.5),
            (&[("sinCoeff", 0.0)][..], 1.0 / 7.0, 1.0 / 3.0),
            (&[("sinCoeff", 0.0), ("cosCoeff", 0.0)][..], 1.0, 1.0),
            (&[("amplitude", 0.0)][..], 1.0, 1.0),
            (&[("sinCoeff", -2.0), ("cosCoeff", 0.37)][..], 1.0, 1.0),
        ] {
            let id = node(&mut g, "shakycam", controls);
            assert_eq!(
                (period(&g, id, Axis::X), period(&g, id, Axis::Y)),
                (Some(x), Some(y)),
                "{controls:?}"
            );
        }
        let t = crate::nodes::find("shakycam").unwrap().timing.unwrap();
        assert_eq!(t.pace_of(Axis::X), 1.0 / 60.0, "X a cycle a minute");
        assert_eq!(t.pace_of(Axis::Y), 1.0 / 15.0, "Y four");
    }

    /// **A control a cable drives is any value it could be**: Rotozoom's cosines cabled with
    /// their knob at zero may move, so `period_in` claims the cycle that holds for every value
    /// where the knob alone says half; a cabled Turns may be odd.
    #[test]
    fn a_cabled_coefficient_claims_the_period_every_value_shares() {
        let mut g = Graph::new();
        let turn = node(&mut g, "rotozoom", &[("cosCoeff", 0.0), ("turns", 2.0)]);
        assert_eq!(
            period_in(&g, turn, Axis::X),
            Some(0.5),
            "unplugged, the knob"
        );
        let number = crate::nodes::add_to_graph(&mut g, "number", emath::Pos2::ZERO).unwrap();
        g.connect(
            PortRef::new(number, "output"),
            PortRef::new(turn, "cosCoeff"),
        )
        .unwrap();
        assert_eq!(period_in(&g, turn, Axis::X), Some(1.0), "cabled, any value");
        assert_eq!(period(&g, turn, Axis::X), Some(0.5), "the knob it replaced");

        let both = node(
            &mut g,
            "rotozoom",
            &[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 4.0)],
        );
        assert_eq!(period_in(&g, both, Axis::X), Some(0.25));
        g.connect(PortRef::new(number, "output"), PortRef::new(both, "turns"))
            .unwrap();
        assert_eq!(period_in(&g, both, Axis::X), Some(1.0), "any whole Turns");

        let shaky = node(&mut g, "shakycam", &[("cosCoeff", 0.0)]);
        g.connect(
            PortRef::new(number, "output"),
            PortRef::new(shaky, "cosCoeff"),
        )
        .unwrap();
        assert_eq!(
            (period_in(&g, shaky, Axis::X), period_in(&g, shaky, Axis::Y)),
            (Some(1.0), Some(1.0))
        );
    }
}
