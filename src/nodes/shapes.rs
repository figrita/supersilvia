// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shapes around a center: a circle, a polygon, a star, a spiral, a phyllotaxis spiral.
//!
//! Ported from silvia's `circle.js`, `polygon.js`, `star.js`, `spiral.js` and
//! `phyllotaxis.js`. silvia writes each coverage test twice — once inside the color
//! generator and once inside the mask generator — because a JS node has no way to share
//! between them; here the shared half is the macro's `wgsl_common` and each output is the one
//! line that differs.
//!
//! Two departures from the source. Every input is read into a local before it is used, so a
//! connected field is evaluated once rather than once per mention. And every divisor a cable
//! can drive to zero is guarded: a control's range excludes zero, an oscillator into the same
//! port does not, and a NaN spreads across the frame rather than staying in the node.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// The two colors a shape's coverage mixes between.
const SHAPE_COLOR: &str = "    return mix({background}, {foreground}, mask);";

const SHAPE_MASK: &str = "    return mask;";

node! {
    /// A soft-edged disc, and its coverage.
    CIRCLE,
    slug: "circle",
    icon: "🔵",
    label: "Circle",
    category: Generate,
    tooltip: "A circle with an adjustable radius, position and edge softness. Its mask is the \
              coverage that drew it.",
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "radius" "Radius" = Control::num(0.5, 0.0, 2.0, 0.01, "⬓"),
        VaryingNumber "softness" "Softness" = Control::num(0.01, 0.0, 0.5, 0.001, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    wgsl_common: "    let radius = {radius};
    let softness = {softness};
    let center = vec2f({centerX}, {centerY});
    let dist = length(uv - center);
    let mask = 1.0 - smoothstep(radius - softness, radius + softness, dist);
",
    outputs: [
        VaryingColor "color" "Color" = SHAPE_COLOR,
        VaryingNumber "mask" "Mask" = SHAPE_MASK,
    ],
}

node! {
    /// A regular polygon, and its coverage.
    POLYGON,
    slug: "polygon",
    icon: "⬟",
    label: "Polygon",
    category: Generate,
    tooltip: "A regular polygon with a chosen number of sides, radius, rotation and edge \
              softness. Its mask is the coverage that drew it.",
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "sides" "Sides" = Control::num(6.0, 3.0, 20.0, 1.0, ""),
        VaryingNumber "radius" "Radius" = Control::num(0.5, 0.0, 2.0, 0.01, "⬓"),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "softness" "Softness" = Control::num(0.01, 0.0, 0.5, 0.001, ""),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // A cable can drive `sides` below three, where the segment angle is a division by
    // something at or through zero.
    wgsl_common: "    let sides = max({sides}, 3.0);
    let radius = {radius};
    let softness = {softness};
    let rotation = ({rotation}) * 2.0 * PI;
    let cuv = uv - vec2f({centerX}, {centerY});
    let cs = cos(rotation);
    let sn = sin(rotation);
    let p = vec2f(cuv.x * cs - cuv.y * sn, cuv.x * sn + cuv.y * cs);
    let angle = atan2(p.y, p.x);
    let dist = length(p);
    let segmentAngle = PI * 2.0 / sides;
    let theta = floor(angle / segmentAngle + 0.5) * segmentAngle;
    let edge = radius * cos(PI / sides) / max(cos(angle - theta), 1e-6);
    let mask = 1.0 - smoothstep(edge - softness, edge + softness, dist);
",
    outputs: [
        VaryingColor "color" "Color" = SHAPE_COLOR,
        VaryingNumber "mask" "Mask" = SHAPE_MASK,
    ],
}

node! {
    /// A star, and its coverage.
    STAR,
    slug: "star",
    icon: "⭐",
    label: "Star",
    category: Generate,
    tooltip: "A star with a chosen number of points, inner radius and rotation. Its mask is \
              the inside/outside test that drew it.",
    // "Inner Radius" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "points" "Points" = Control::num(5.0, 5.0, 12.0, 1.0, ""),
        VaryingNumber "innerRadius" "Inner Radius" = Control::num(0.25, 0.0, 1.0, 0.01, ""),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    // Half-plane accumulation: a point is inside where fewer than two of the point normals
    // put it outside. The loop is bounded by the control's own maximum so the trip count is
    // a constant however the count arrives.
    wgsl_common: "    let points = clamp({points}, 3.0, 12.0);
    let innerRadius = {innerRadius};
    let rotation = ({rotation}) * 2.0 * PI;
    let cuv = uv - vec2f({centerX}, {centerY});
    let cs = cos(rotation);
    let sn = sin(rotation);
    let p = vec2f(cuv.x * cs - cuv.y * sn, cuv.x * sn + cuv.y * cs);
    let sweep = PI * 2.0 / points;
    var acc = 0.0;
    var a = 0.0;
    for (var i = 0; i < 12; i++) {
        if (f32(i) >= points) { break; }
        let dir = vec2f(sin(a), cos(a));
        acc += select(0.0, 0.5, dot(dir, p - dir * innerRadius) > 0.0);
        a += sweep;
    }
    let mask = step(acc, 0.9999);
",
    outputs: [
        VaryingColor "color" "Color" = SHAPE_COLOR,
        VaryingNumber "mask" "Mask" = SHAPE_MASK,
    ],
}

/// The polar frame a spiral arm is measured in, shared by all three kinds. `mask` is a
/// `var`, since the arm loop raises it.
const SPIRAL_SETUP_WGSL: &str = "    let turns = max({turns}, 0.1);
    let thickness = max({thickness}, 1e-4);
    let innerRadius = {innerRadius};
    let outerRadius = {outerRadius};
    let rotation = ({rotation}) * 2.0 * PI;
    let cuv = uv - vec2f({centerX}, {centerY});
    let cs = cos(rotation);
    let sn = sin(rotation);
    let p = vec2f(cuv.x * cs - cuv.y * sn, cuv.x * sn + cuv.y * cs);
    let angle = (atan2(p.y, p.x) + PI) / (2.0 * PI);
    let dist = length(p);
    var mask = 0.0;
";

/// r = a + b·θ: even spacing between the windings.
const SPIRAL_ARCHIMEDEAN_WGSL: &str =
    "    let b = (outerRadius - innerRadius) / (turns * 2.0 * PI);
    for (var w = 0; w < 20; w++) {
        if (f32(w) >= turns) { break; }
        let theta = (angle + f32(w)) * 2.0 * PI;
        let arm = innerRadius + b * theta;
        if (arm >= innerRadius && arm <= outerRadius) {
            mask = max(mask, 1.0 - smoothstep(0.0, thickness, abs(dist - arm)));
        }
    }
";

/// r = a·e^(b·θ): each winding a fixed ratio wider than the last.
const SPIRAL_LOGARITHMIC_WGSL: &str = "    let a = innerRadius + 0.001;
    let b = log((outerRadius + 0.001) / a) / (turns * 2.0 * PI);
    for (var w = 0; w < 20; w++) {
        if (f32(w) >= turns) { break; }
        let theta = (angle + f32(w)) * 2.0 * PI;
        let arm = a * exp(b * theta);
        if (arm >= innerRadius && arm <= outerRadius) {
            mask = max(mask, 1.0 - smoothstep(0.0, thickness, abs(dist - arm)));
        }
    }
";

/// r = a·√θ, both arms: equal area between the windings.
const SPIRAL_FERMAT_WGSL: &str = "    let a = outerRadius / sqrt(turns * 2.0 * PI);
    for (var w = 0; w < 20; w++) {
        if (f32(w) >= turns) { break; }
        let theta = (angle + f32(w)) * 2.0 * PI;
        let arm = abs(a * sqrt(theta));
        if (arm >= innerRadius && arm <= outerRadius) {
            mask = max(mask, 1.0 - smoothstep(0.0, thickness, abs(dist - arm)));
        }
    }
";

node! {
    /// A spiral arm, its coverage, and the polar angle it is drawn from.
    SPIRAL,
    slug: "spiral",
    icon: "😵",
    label: "Spiral",
    category: Generate,
    tooltip: "A spiral in one of three families. Its mask is the coverage of the arm, and its \
              angle is the polar sweep the arm is drawn along.",
    // "Inner Radius" and "Outer Radius" do not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "turns" "Turns" = Control::num(5.0, 0.5, 20.0, 0.1, ""),
        VaryingNumber "thickness" "Thickness" = Control::num(0.05, 0.01, 0.5, 0.01, ""),
        VaryingNumber "innerRadius" "Inner Radius" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "outerRadius" "Outer Radius" = Control::num(1.0, 0.0, 2.0, 0.01, ""),
        VaryingNumber "rotation" "Rotation" = Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.01, "⬓"),
    ],
    options: [
        "type" "Type" = "archimedean" [
            "archimedean" => "Archimedean",
            "logarithmic" => "Logarithmic",
            "fermat" => "Fermat",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            SPIRAL_SETUP_WGSL,
            match ctx.option(node, "type") {
                "logarithmic" => SPIRAL_LOGARITHMIC_WGSL,
                "fermat" => SPIRAL_FERMAT_WGSL,
                _ => SPIRAL_ARCHIMEDEAN_WGSL,
            },
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = SHAPE_COLOR,
        VaryingNumber "mask" "Mask" = SHAPE_MASK,
        VaryingNumber "angle" "Angle" = "    return angle;",
    ],
}

node! {
    /// Seeds on a golden-angle spiral, as a sunflower head arranges them.
    PHYLLOTAXIS,
    slug: "phyllotaxis",
    icon: "🌻",
    label: "Phyllotaxis",
    category: Generate,
    tooltip: "The spiral packing a sunflower head uses. The angle between seeds is what makes \
              the arms; the golden angle is 0.382 turns.",
    // "Background Color" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingNumber "count" "Count" = Control::num(100.0, 1.0, 500.0, 1.0, ""),
        VaryingNumber "radius" "Radius" = Control::num(0.9, 0.01, 2.0, 0.01, "⬓"),
        VaryingNumber "angle" "Angle" = Control::num(0.382, -2.0, 2.0, 0.001, crate::nodes::TURNS),
        VaryingNumber "dotSize" "Dot Size" = Control::num(0.06, 0.005, 0.2, 0.001, "⬓"),
        VaryingColor "bg" "Background Color" = Control::color("#ffe066ff"),
        VaryingColor "fg" "Dot Color" = Control::color("#000000ff"),
    ],
    // The nearest seed, not a coverage sum: the dots do not overlap at the sizes the control
    // allows, so the distance to the closest one is the whole picture.
    //
    // Only a seed within `dotSize` of the pixel can color it, and a seed's radius is a
    // function of its index, so the seeds that matter are a band of indices: the ring under
    // the pixel, a dot's width either side. Outside the disc the band is empty and the loop
    // runs zero times; inside it the trip count follows the dot size rather than the seed
    // count. Every seed tested was, at 500 seeds and 720p, two vsync intervals per frame on
    // an integrated GPU. The loop is still bounded by the control's maximum, as `polygon`'s
    // is, so the trip count is a constant however the count arrives.
    //
    // The edge is `1.0 - smoothstep` with its edges in order: the same curve as a reversed
    // `smoothstep(dotSize, dotSize * 0.5, …)`, which Metal leaves undefined.
    outputs: [
        VaryingColor "output" "Output" = "    let count = max({count}, 1.0);
    let maxR = max({radius}, 1e-4);
    let seedAngle = ({angle}) * 2.0 * PI;
    let dotSize = {dotSize};
    let last = max(count - 1.0, 1.0);
    let r = length(uv);
    let lo = max(r - dotSize, 0.0) / maxR;
    let hi = (r + dotSize) / maxR;
    let first = i32(floor(last * lo * lo));
    let stop = i32(min(ceil(last * hi * hi), count - 1.0));
    var minDist = 1e6;
    for (var i = 0; i < 500; i++) {
        let j = first + i;
        if (j > stop) { break; }
        let t = f32(j) / last;
        let sr = maxR * sqrt(t);
        let a = f32(j) * seedAngle;
        minDist = min(minDist, length(uv - sr * vec2f(cos(a), sin(a))));
    }
    return mix({bg}, {fg}, 1.0 - smoothstep(dotSize * 0.5, dotSize, minDist));",
    ],
}
