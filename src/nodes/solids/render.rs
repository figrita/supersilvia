// SPDX-License-Identifier: AGPL-3.0-or-later

//! The render: one ray per pixel into a solid, from a camera that orbits a target or looks at
//! it from a point.
//!
//! Its output is a `VaryingColor` field — a function of `uv` like any picture — so an ordinary
//! Output shows it, a frame port republishes it, the mixer takes it and every video node can
//! work on it. Nothing in the renderer or the compositor knows it is three-dimensional. The
//! cost is the one thing different about it: a picture is evaluated once a pixel, and this
//! evaluates its solid up to Steps times a pixel, then four times for the normal, up to 32 for
//! the shadow and five for the occlusion. The Steps shading paints that count.
//!
//! What else it can do is priced by the march it rides on:
//!
//! - **Glow** integrates `exp(-d / reach)` along the ray, `d` the solid's distance at each
//!   step, so a ray that grazes a surface without touching it still carries light: a halo
//!   against the background. It reads the distance the march already measured, so it costs
//!   an exponential a step, and nothing at all while the option is Off, which emits no code.
//! - **Toon** bands the light and inks an outline where the ray's distance to the solid fell
//!   to a minimum and rose again — the ray passed the solid's edge, whether it then missed
//!   everything or hit something behind. The minimum is taken in picture units (the distance
//!   over how much world one unit of `uv` spans at that depth), so the line is one width
//!   across the picture. A comparison a step.
//! - **One bounce** marches a second ray along the reflection, a quarter of Steps long, then
//!   asks for a normal and a color there: up to a quarter as much again as the main march,
//!   plus five evaluations, and it shades what it hits with diffuse light alone.
//! - **Look at**, **Orthographic** and **Fisheye** change only how the ray starts, and cost
//!   nothing. Orthographic frames the target as the perspective lens would at the same
//!   distance, so switching keeps the subject's size, and its rays start that distance again
//!   behind the camera, so a plane of origins as tall as the view does not begin under the
//!   floor; fog and Depth count from the camera. Fisheye is equidistant, `1 / Lens`
//!   radians from the middle to the top edge, and paints the background past a half turn.
//!
//! Every one of these is a `Code` option, and at its default each emits nothing: the module is
//! the plain march alone, which is also what every solid node's snapshot holds.
//!
//! **Five numbers are ports**: Orbit, Tilt, Distance, Light and Glow, so a MIDI knob or a gear
//! flies the camera and moves the light. Everything else is the node's own: hidden controls and
//! in-region options, drawn in four folding sections (`Region::Section`) — Camera's globe, which
//! a hand drags to orbit and tilt and wheels to move nearer, Light's sun disk, Look and Quality
//! — and in the panels three of their buttons open. Each is still a control or an option: saved,
//! undone through the bus, a number bindable to MIDI. See `widgets::section`.

use std::fmt::Write;

use super::{SDF, color, fill, num, num_log, solid};
use crate::compile::{CompileContext, SOLID_FAR};
use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, Dial, Globe, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind, Part, Popout,
    Region, Section, TURNS,
};

/// Where the solid is asked in the march: `pp`, a variable the body moves.
const AT: &str = "pp";

/// The value of one of [`RENDER`]'s options, as the choice it names; the default for a value
/// that names none.
fn chosen(ctx: &CompileContext, node: NodeId, key: &str) -> &'static str {
    let Some(def) = RENDER.options.iter().find(|o| o.key == key) else {
        return "";
    };
    let value = ctx.option(node, key);
    def.choices
        .iter()
        .map(|(c, _)| *c)
        .find(|c| *c == value)
        .unwrap_or(def.default)
}

/// `src` with every non-empty line indented four more spaces.
fn indent(src: &str) -> String {
    src.lines()
        .map(|l| {
            if l.is_empty() {
                "\n".to_string()
            } else {
                format!("    {l}\n")
            }
        })
        .collect()
}

/// The floor's color at the point named `at`.
fn floor_albedo(plain: bool, at: &str) -> String {
    if plain {
        "vec3f(0.30, 0.29, 0.33)".to_string()
    } else {
        format!(
            "mix(vec3f(0.34, 0.32, 0.38), vec3f(0.20, 0.19, 0.23), floor_mod(floor({at}.x) + floor({at}.z), 2.0))"
        )
    }
}

fn body(node: NodeId, ctx: &mut CompileContext) -> String {
    let shading = chosen(ctx, node, "shading");
    let floor_kind = chosen(ctx, node, "floor");
    let soft = chosen(ctx, node, "shadows") == "soft";
    let steps: u32 = chosen(ctx, node, "steps").parse().unwrap_or(128);
    let glow = chosen(ctx, node, "glow") == "on";
    let bounce = chosen(ctx, node, "reflections") == "one";
    let lens_kind = chosen(ctx, node, "projection");
    let look_at = chosen(ctx, node, "camera") == "lookat";
    let toon = shading == "toon";

    let dist = ctx.solid_input(node, "solid", AT, false);
    let whole = ctx.solid_input(node, "solid", "pos", true);
    let floor = match floor_kind {
        "none" => SOLID_FAR.to_string(),
        _ => format!("({AT}.y - fh)"),
    };
    let scene = format!("min({floor}, {dist}.x)");
    let plain = floor_kind == "plain";

    let camera = if look_at {
        "    let aim = vec3f({targetX}, {targetY}, {targetZ});
    let ro = vec3f({fromX}, {fromY}, {fromZ});
    let ahead = aim - ro;
    let fwd = select(vec3f(0.0, 0.0, -1.0), normalize(ahead), dot(ahead, ahead) > 1.0e-10);
    let side = cross(fwd, vec3f(0.0, 1.0, 0.0));
    let right = select(vec3f(1.0, 0.0, 0.0), normalize(side), dot(side, side) > 1.0e-10);
"
    } else {
        "    let orbit = {orbit} * tau;
    let tilt = {tilt} * tau;
    let aim = vec3f({targetX}, {targetY}, {targetZ});
    let ro = aim + {distance} * vec3f(cos(tilt) * sin(orbit), sin(tilt), cos(tilt) * cos(orbit));
    let fwd = normalize(aim - ro);
    let right = normalize(cross(fwd, vec3f(0.0, 1.0, 0.0)));
"
    };
    // The rays: how they start, where the march starts them from, how much world one unit of
    // `uv` spans at depth `t`, and the depth fog and Depth count.
    let (start, origin, per_unit, seen) = match lens_kind {
        "orthographic" => (
            "    let pull = length(aim - ro);
    let oh = pull / max({lens}, 0.001);
    let org = ro - fwd * pull + (right * uv.x + up * uv.y) * oh;
    let rd = fwd;
",
            "org",
            "oh",
            "max(t - pull, 0.0)",
        ),
        "fisheye" => (
            "    let r = length(uv);
    let theta = r / max({lens}, 0.001);
    if (theta > 3.1415927) { return {background}; }
    let across = select(vec2f(0.0), uv / r, r > 1.0e-6);
    let rd = fwd * cos(theta) + (right * across.x + up * across.y) * sin(theta);
",
            "ro",
            "(max(t, 0.001) / max({lens}, 0.001))",
            "t",
        ),
        _ => (
            "    let rd = normalize(fwd * {lens} + right * uv.x + up * uv.y);\n",
            "ro",
            "(max(t, 0.001) / max({lens}, 0.001))",
            "t",
        ),
    };

    let mut track_init = String::new();
    let mut track = String::new();
    let mut after_march = String::new();
    let mut post = String::new();
    if toon {
        track_init
            .push_str("    var rim = 1.0e9;\n    var last_hs = 1.0e9;\n    var last_w = 1.0e9;\n");
        let _ = write!(
            track,
            "        if (hs > last_hs) {{ rim = min(rim, last_w); }}
        last_hs = hs;
        last_w = hs / {per_unit};
"
        );
        after_march.push_str(
            "    if (!hit) { rim = min(rim, last_w); }
    let ow = max({outline}, 1.0e-5);
    let ink = 1.0 - smoothstep(0.6 * ow, ow, rim);
",
        );
        post.push_str("    res = mix(res, {outlineColor}, ink);\n");
    }
    if glow {
        track_init.push_str("    let reach = max({glowReach}, 0.001);\n    var acc = 0.0;\n");
        track.push_str("        acc += exp(-hs / reach) * h * relax;\n");
        after_march.push_str(
            "    let gl = max({glowAmount}, 0.0) * {glowColor}.rgb * (1.0 - exp(-acc / reach));
    let gla = min(max(gl.r, max(gl.g, gl.b)), 1.0);
",
        );
        post.push_str("    res = vec4f(res.rgb + gl, res.a + gla * (1.0 - res.a));\n");
    }
    let measure = if toon || glow {
        format!("        let hs = {dist}.x;\n        let h = min({floor}, hs);\n")
    } else {
        format!("        let h = {scene};\n")
    };
    // The statements that end the function on the color `expr`, through the outline and the
    // glow where they are on.
    let give = |expr: &str| {
        if post.is_empty() {
            format!("    return {expr};")
        } else {
            format!("    var res = {expr};\n{post}    return res;")
        }
    };
    let miss = if post.is_empty() {
        "    if (!hit) { return bg; }\n".to_string()
    } else {
        format!("    if (!hit) {{\n{}    }}\n", indent(&give("bg")))
    };

    let shadow = if soft {
        format!(
            "    var sh = 1.0;
    var st = 0.03;
    for (var j = 0; j < 32; j++) {{
        {AT} = pos + n * 0.01 + light * st;
        let h = {scene};
        sh = min(sh, 10.0 * h / st);
        st += clamp(h, 0.02, 0.6);
        if (sh < 0.01 || st > 20.0) {{ break; }}
    }}
    sh = clamp(sh, 0.0, 1.0);
"
        )
    } else {
        "    let sh = 1.0;\n".to_string()
    };
    let reflection = if bounce {
        let whole2 = ctx.solid_input(node, "solid", "p2", true);
        let albedo2 = floor_albedo(plain, "p2");
        format!(
            "    let rd2 = reflect(rd, n);
    let ro2 = pos + n * 0.002;
    var t2 = 0.01;
    var hit2 = false;
    for (var k = 0; k < {}; k++) {{
        {AT} = ro2 + rd2 * t2;
        let h = {scene};
        if (h < 0.0004 * (t + t2) + 0.0001) {{ hit2 = true; break; }}
        t2 += h * relax;
        if (t2 > 80.0) {{ break; }}
    }}
    var rcol = bg.rgb;
    if (hit2) {{
        let p2 = ro2 + rd2 * t2;
        let e2 = vec2f(1.0, -1.0) * 0.0005 * max(1.0, (t + t2) * 0.25);
        {AT} = p2 + e2.xyy;
        let m0 = {scene};
        {AT} = p2 + e2.yyx;
        let m1 = {scene};
        {AT} = p2 + e2.yxy;
        let m2 = {scene};
        {AT} = p2 + e2.xxx;
        let m3 = {scene};
        let n2 = normalize(e2.xyy * m0 + e2.yyx * m1 + e2.yxy * m2 + e2.xxx * m3);
        {AT} = p2;
        var albedo2 = {albedo2};
        if (({floor}) >= {dist}.x) {{ albedo2 = {whole2}.yzw; }}
        let dif2 = max(dot(n2, light), 0.0);
        let amb2 = 0.5 + 0.5 * n2.y;
        let col2 = pow(pow(albedo2, vec3f(2.2)) * (dif2 * 1.25 + amb2 * 0.30), vec3f(1.0 / 2.2));
        rcol = mix(col2, bg.rgb, 1.0 - exp(-{{fog}} * t2));
    }}
    let gloss = clamp({{reflectivity}}, 0.0, 1.0);
    let fres = gloss + (1.0 - gloss) * pow(1.0 - clamp(dot(n, -rd), 0.0, 1.0), 5.0);
    col = mix(col, rcol, fres);
",
            (steps / 4).max(1)
        )
    } else {
        String::new()
    };
    let col_kw = if bounce { "var" } else { "let" };
    let lit = give("mix(vec4f(col, 1.0), bg, fog)");
    let finish = match shading {
        "normals" => give("vec4f(n * 0.5 + 0.5, 1.0)"),
        "depth" => give(&format!("vec4f(vec3f(exp(-{seen} * 0.12)), 1.0)")),
        "flat" if bounce => format!("    var col = albedo;\n{reflection}{lit}"),
        "flat" => give("mix(vec4f(albedo, 1.0), bg, fog)"),
        "toon" => format!(
            "{shadow}    let dif = max(dot(n, light), 0.0) * sh;
    let bands = max(round({{bands}}), 2.0);
    let tone = min(floor(dif * bands), bands - 1.0) / (bands - 1.0);
    let shine = step(0.96, dot(n, normalize(light - rd))) * step(0.5, tone);
    {col_kw} col = albedo * (0.3 + 0.7 * tone) + vec3f(0.3 * shine);
{reflection}{lit}"
        ),
        _ => format!(
            "{shadow}    var occ = 0.0;
    for (var j = 1; j <= 5; j++) {{
        let hk = 0.07 * f32(j);
        {AT} = pos + n * hk;
        occ += (hk - {scene}) / f32(j);
    }}
    let ao = clamp(1.0 - 2.5 * occ, 0.0, 1.0);
    let lin = pow(albedo, vec3f(2.2));
    let dif = max(dot(n, light), 0.0) * sh;
    let amb = (0.5 + 0.5 * n.y) * ao;
    let spe = pow(max(dot(n, normalize(light - rd)), 0.0), 40.0) * dif * 0.35;
    {col_kw} col = pow(lin * (dif * 1.25 + amb * 0.30) + spe, vec3f(1.0 / 2.2));
{reflection}{lit}"
        ),
    };
    let albedo_floor = floor_albedo(plain, "pos");
    let rest = if shading == "steps" {
        "    return vec4f(clamp(vec3f(2.0 * heat, 2.0 - 2.0 * abs(2.0 * heat - 1.0) * 1.0, 1.0 - 2.0 * heat), vec3f(0.0), vec3f(1.0)), 1.0);".to_string()
    } else {
        format!(
            "{miss}    let pos = {origin} + rd * t;
    let e = vec2f(1.0, -1.0) * 0.0005 * max(1.0, t * 0.25);
    {AT} = pos + e.xyy;
    let n0 = {scene};
    {AT} = pos + e.yyx;
    let n1 = {scene};
    {AT} = pos + e.yxy;
    let n2 = {scene};
    {AT} = pos + e.xxx;
    let n3 = {scene};
    let n = normalize(e.xyy * n0 + e.yyx * n1 + e.yxy * n2 + e.xxx * n3);
    {AT} = pos;
    let floor_hit = ({floor}) < {dist}.x;
    var albedo = {albedo_floor};
    if (!floor_hit) {{ albedo = {whole}.yzw; }}
    let fog = 1.0 - exp(-{{fog}} * {seen});
    let la = {{light}} * tau;
    let lh = {{lightHeight}} * tau;
    let light = normalize(vec3f(cos(lh) * sin(la), sin(lh), cos(lh) * cos(la)));
{finish}"
        )
    };
    fill(
        node,
        ctx,
        &format!(
            "    let tau = 6.2831853;
{camera}    let up = cross(right, fwd);
{start}    let fh = {{floorHeight}};
    let bg = {{background}};
    let relax = {{step}};
    var {AT} = vec3f(0.0);
    var t = 0.0;
    var hit = false;
    var count = 0.0;
{track_init}    for (var i = 0; i < {steps}; i++) {{
        {AT} = {origin} + rd * t;
{measure}        if (h < 0.0004 * t + 0.0001) {{ hit = true; break; }}
{track}        t += h * relax;
        count += 1.0;
        if (t > 80.0) {{ break; }}
    }}
{after_march}    let heat = count / {steps}.0;
{rest}"
        ),
    )
}

/// The four headings over Render 3D's sections, each an ordinary option.
pub const CAMERA_HEADING: OptionDef =
    OptionDef::heading("cameraSection", "Camera", true, OptionKind::Presentation);
pub const LIGHT_HEADING: OptionDef =
    OptionDef::heading("lightSection", "Light", true, OptionKind::Presentation);
pub const LOOK_HEADING: OptionDef =
    OptionDef::heading("lookSection", "Look", true, OptionKind::Presentation);
pub const QUALITY_HEADING: OptionDef =
    OptionDef::heading("qualitySection", "Quality", false, OptionKind::Presentation);

/// The camera's controls, as the globe reads and writes them.
static GLOBE: Globe = Globe {
    orbit: "orbit",
    tilt: "tilt",
    distance: "distance",
    light: ["light", "lightHeight"],
    mode: "camera",
    look_at: "lookat",
    from: ["fromX", "fromY", "fromZ"],
    target: ["targetX", "targetY", "targetZ"],
};

static AIM: Popout = Popout {
    key: "aim",
    caption: "Target & From…",
    parts: &[
        Part::Number {
            key: "targetX",
            caption: "Target X",
        },
        Part::Number {
            key: "targetY",
            caption: "Target Y",
        },
        Part::Number {
            key: "targetZ",
            caption: "Target Z",
        },
        Part::Number {
            key: "fromX",
            caption: "From X",
        },
        Part::Number {
            key: "fromY",
            caption: "From Y",
        },
        Part::Number {
            key: "fromZ",
            caption: "From Z",
        },
    ],
    reset: &["targetX", "targetY", "targetZ", "fromX", "fromY", "fromZ"],
};

static GLOW: Popout = Popout {
    key: "glowPanel",
    caption: "Glow…",
    parts: &[
        Part::Choice {
            key: "glow",
            caption: "Glow",
        },
        Part::Swatch {
            key: "glowColor",
            caption: "Color",
        },
        Part::Number {
            key: "glowReach",
            caption: "Reach",
        },
    ],
    reset: &[],
};

static TOON: Popout = Popout {
    key: "toonPanel",
    caption: "Toon…",
    parts: &[
        Part::Number {
            key: "bands",
            caption: "Bands",
        },
        Part::Number {
            key: "outline",
            caption: "Outline",
        },
        Part::Swatch {
            key: "outlineColor",
            caption: "Outline Color",
        },
    ],
    reset: &[],
};

static CAMERA: Section = Section {
    heading: &CAMERA_HEADING,
    parts: &[Part::Beside(
        Dial::Globe(&GLOBE),
        &[
            Part::Choice {
                key: "camera",
                caption: "Camera",
            },
            Part::Choice {
                key: "projection",
                caption: "Lens Type",
            },
            Part::Number {
                key: "lens",
                caption: "Lens",
            },
            Part::Popout(&AIM),
        ],
    )],
};

static LIGHT: Section = Section {
    heading: &LIGHT_HEADING,
    parts: &[Part::Beside(
        Dial::Sun {
            azimuth: "light",
            height: "lightHeight",
            camera: &GLOBE,
        },
        &[
            Part::Choice {
                key: "shadows",
                caption: "Shadows",
            },
            Part::Number {
                key: "lightHeight",
                caption: "Height",
            },
            Part::Number {
                key: "fog",
                caption: "Fog",
            },
            Part::Popout(&GLOW),
        ],
    )],
};

static LOOK: Section = Section {
    heading: &LOOK_HEADING,
    parts: &[
        Part::Choice {
            key: "shading",
            caption: "Shading",
        },
        Part::Popout(&TOON),
        Part::Swatch {
            key: "background",
            caption: "Background",
        },
        Part::Choice {
            key: "floor",
            caption: "Floor",
        },
        Part::Number {
            key: "floorHeight",
            caption: "Floor Height",
        },
    ],
};

static QUALITY: Section = Section {
    heading: &QUALITY_HEADING,
    parts: &[
        Part::Choice {
            key: "steps",
            caption: "Steps",
        },
        Part::Number {
            key: "step",
            caption: "Step",
        },
        Part::Choice {
            key: "reflections",
            caption: "Reflections",
        },
        Part::Number {
            key: "reflectivity",
            caption: "Reflectivity",
        },
    ],
};

pub static RENDER: NodeDef = NodeDef {
    slug: "solidrender",
    category: Category::Solid,
    icon: "🎥",
    label: "Render 3D",
    tooltip: "Turns a solid into a picture: a camera Distance away from the target, orbiting \
              it and tilted above it — or, with Camera on Look at, standing at From — lit from \
              Light. Drag the globe to move the camera and the sun to move the light. Plug the \
              picture into an Output, or into any video node. Steps shading paints how hard \
              each pixel worked. Glow lights the air near a surface, Toon shading draws light \
              in Bands with an Outline, and One bounce reflections march a second ray from \
              every surface, costing up to a quarter as much again.",
    inputs: &[
        solid("solid", "Solid"),
        num("orbit", "Orbit", 0.08, -1.0, 1.0, 0.001, TURNS),
        num("tilt", "Tilt", 0.06, -0.24, 0.24, 0.001, TURNS),
        num_log("distance", "Distance", 7.0, 0.5, 80.0, 0.01, "⬓"),
        num("light", "Light", 0.12, -1.0, 1.0, 0.001, TURNS),
        num("glowAmount", "Glow", 0.6, 0.0, 4.0, 0.001, ""),
    ],
    hidden: &[
        num("targetX", "Target X", 0.0, -20.0, 20.0, 0.01, "⬓"),
        num("targetY", "Target Y", 0.0, -20.0, 20.0, 0.01, "⬓"),
        num("targetZ", "Target Z", 0.0, -20.0, 20.0, 0.01, "⬓"),
        num("fromX", "From X", 3.135, -40.0, 40.0, 0.01, "⬓"),
        num("fromY", "From Y", 2.58, -40.0, 40.0, 0.01, "⬓"),
        num("fromZ", "From Z", 5.7, -40.0, 40.0, 0.01, "⬓"),
        num("lens", "Lens", 1.6, 0.3, 6.0, 0.01, "x"),
        num("lightHeight", "Light Height", 0.12, 0.0, 0.25, 0.001, TURNS),
        num("floorHeight", "Floor Height", -1.0, -20.0, 20.0, 0.01, "⬓"),
        num("fog", "Fog", 0.03, 0.0, 1.0, 0.001, ""),
        num("step", "Step", 0.9, 0.2, 1.5, 0.01, "x"),
        color("background", "Background", "#141019ff"),
        num("reflectivity", "Reflectivity", 0.25, 0.0, 1.0, 0.001, ""),
        color("glowColor", "Glow Color", "#59d8ffff"),
        num_log("glowReach", "Glow Reach", 0.25, 0.01, 4.0, 0.001, "⬓"),
        num("bands", "Bands", 3.0, 2.0, 8.0, 1.0, ""),
        num("outline", "Outline", 0.012, 0.0, 0.1, 0.0001, ""),
        color("outlineColor", "Outline Color", "#0b0a10ff"),
    ],
    options: &[
        OptionDef {
            key: "shading",
            label: "Shading",
            default: "lit",
            choices: &[
                ("lit", "Lit"),
                ("toon", "Toon"),
                ("flat", "Flat"),
                ("normals", "Normals"),
                ("depth", "Depth"),
                ("steps", "Steps"),
            ],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "camera",
            label: "Camera",
            default: "orbit",
            choices: &[("orbit", "Orbit"), ("lookat", "Look at")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "projection",
            label: "Lens Type",
            default: "perspective",
            choices: &[
                ("perspective", "Persp"),
                ("orthographic", "Ortho"),
                ("fisheye", "Fisheye"),
            ],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "floor",
            label: "Floor",
            default: "checker",
            choices: &[("checker", "Checker"), ("plain", "Plain"), ("none", "None")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "shadows",
            label: "Shadows",
            default: "soft",
            choices: &[("soft", "Soft"), ("none", "None")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "reflections",
            label: "Reflections",
            default: "none",
            choices: &[("none", "None"), ("one", "One bounce")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "glow",
            label: "Glow",
            default: "off",
            choices: &[("off", "Off"), ("on", "On")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "steps",
            label: "Steps",
            default: "128",
            choices: &[("64", "64"), ("128", "128"), ("256", "256"), ("512", "512")],
            in_region: true,
            ..OptionDef::EMPTY
        },
        CAMERA_HEADING,
        LIGHT_HEADING,
        LOOK_HEADING,
        QUALITY_HEADING,
    ],
    regions: &[
        Region::Section(&CAMERA),
        Region::Section(&LIGHT),
        Region::Section(&LOOK),
        Region::Section(&QUALITY),
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: VaryingColor,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _| body(node, ctx),
        ..OutputDef::EMPTY
    }],
    wgsl_utils: &[SDF],
    ..NodeDef::EMPTY
};
