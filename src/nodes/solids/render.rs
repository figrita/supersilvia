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
//!   an exponential a step. It is on while Glow Amount is above zero, and since that is a
//!   uniform the accumulation is always emitted and asked a step at a time: at zero it costs
//!   a branch the whole frame takes the same way, and adds nothing.
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
//! - **Sky**, a picture cabled in, is the whole sphere round the scene: a ray that hits nothing
//!   shows it in its direction, mapped as Texture's Sphere maps a picture — its width once round
//!   the horizon, its height pole to pole — and fog fades to it rather than to Background, as a
//!   reflection's does to the sky in the reflected direction. The picture is asked once a
//!   pixel, after the march, and once more for a reflection. Sky Tilt tips its pole about x;
//!   turning it round the pole is a Translate along X before it, so there is no number for it.
//!
//! Every one of these but the Glow and the Sky is a `Code` option, and at its default each emits
//! nothing, as the Sky unconnected does: the module is the plain march with the glow's branch,
//! which is also what every solid node's snapshot holds.
//!
//! **A ray that runs out of steps** short of the far limit is asked its distance once more
//! where it stopped, and is a hit there if that is within about a pixel; else it lands on the
//! floor where it would cross it; else it is the background. A bound that shrinks the step and
//! a ray grazing the floor toward the horizon run out before they arrive, and showed the
//! background as a ring round the surface and a ragged horizon. The ray's closest approach is
//! not used: a bound that shrinks the step under-reports the distance, so it accepted rays that
//! pass pixels clear of the surface and grew a crust round it.
//!
//! **Five numbers are ports**: Orbit, Tilt, Distance, Light and Glow, so a MIDI knob or a gear
//! flies the camera and moves the light; the Sky is a port too, being a picture. Everything else is the node's own: hidden controls and
//! in-region options, drawn in four folding sections (`Region::Section`) — Camera's globe, which
//! a hand drags to orbit and tilt and wheels to move nearer, Light's sun disk, Look and Quality
//! — and in the panels three of their buttons open. Each is still a control or an option: saved,
//! undone through the bus, a number bindable to MIDI. See `widgets::section`.

use std::fmt::Write;

use super::{SDF, color, fill, num, num_log, solid, turn, uniform};
use crate::compile::{CompileContext, SOLID_FAR};
use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::area::Applies;
use crate::nodes::{
    Category, Control, Dial, Globe, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, Part, Popout, Region, Section, TURNS,
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

/// How near a ray that runs out of steps must have come to a surface, in picture units, to be
/// shaded as a hit there: about a pixel at [`REFERENCE_HEIGHT`](crate::nodes::REFERENCE_HEIGHT).
const NEAR: &str = "0.003";

/// How much world one unit of `uv` spans at `depth` along a ray, through the lens `kind`.
fn per_unit(kind: &str, depth: &str) -> String {
    if kind == "orthographic" {
        "oh".to_string()
    } else {
        format!("(max({depth}, 0.001) / max({{lens}}, 0.001))")
    }
}

/// One march's names: its depth and hit flag, the point and direction its ray leaves along,
/// and its hit test's tolerance where the ray leaves.
struct March<'a> {
    t: &'a str,
    hit: &'a str,
    from: &'a str,
    dir: &'a str,
    tolerance: &'a str,
}

impl March<'_> {
    /// After the loop: a ray that ran out of steps short of the far limit is a hit where it
    /// ended, if it ended within [`NEAR`] of a surface; else on the floor, where `floor` and
    /// the ray come within the hit test's tolerance of it; else a miss. `scene` is the
    /// distance at [`AT`] and `unit` the world one unit of `uv` spans at the depth.
    fn exhausted(&self, scene: &str, unit: &str, floor: bool) -> String {
        let March {
            t,
            hit,
            from,
            dir,
            tolerance,
        } = *self;
        let crossing = if floor {
            format!(
                " else if ({dir}.y < 0.0) {{
            let tf = ({from}.y - fh - {tolerance}) / (0.0004 - {dir}.y);
            if (tf > 0.0 && tf <= 80.0) {{ {t} = tf; {hit} = true; }}
        }}"
            )
        } else {
            String::new()
        };
        format!(
            "    if (!{hit} && {t} <= 80.0) {{
        {AT} = {from} + {dir} * {t};
        if ({scene} < {NEAR} * {unit}) {{
            {hit} = true;
        }}{crossing}
    }}
"
        )
    }
}

/// Statements binding `name` to the Sky input's color in the direction `dir`: the direction
/// tipped by Sky Tilt about x, then mapped over the whole sphere, the picture's width once round
/// the horizon and its height pole to pole.
fn sky_at(ctx: &mut CompileContext, node: NodeId, name: &str, dir: &str) -> String {
    let color = ctx.input(
        node,
        "sky",
        &format!(
            "vec2f(atan2({name}_d.x, {name}_d.z) / PI * (16.0 / 9.0), asin(clamp({name}_d.y, -1.0, 1.0)) / (PI * 0.5))"
        ),
    );
    format!(
        "    let {name}_t = {{skyTilt}} * tau;
    let {name}_d = vec3f({dir}.x, {dir}.y * cos({name}_t) - {dir}.z * sin({name}_t), {dir}.y * sin({name}_t) + {dir}.z * cos({name}_t));
    let {name} = {color};
"
    )
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
    let bounce = chosen(ctx, node, "reflections") == "one";
    let lens_kind = chosen(ctx, node, "projection");
    let look_at = chosen(ctx, node, "camera") == "lookat";
    let toon = shading == "toon";
    let sky = ctx.connected(node, "sky");

    let dist = ctx.solid_input(node, "solid", AT, false);
    let whole = ctx.solid_input(node, "solid", "pos", true);
    let floor = match floor_kind {
        "none" => SOLID_FAR.to_string(),
        _ => format!("({AT}.y - fh)"),
    };
    let scene = format!("min({floor}, {dist}.x)");
    let plain = floor_kind == "plain";
    let has_floor = floor_kind != "none";

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
    // The rays: how they start, where the march starts them from, and the depth fog and Depth
    // count.
    let (start, origin, seen) = match lens_kind {
        "orthographic" => (
            "    let pull = length(aim - ro);
    let oh = pull / max({lens}, 0.001);
    let org = ro - fwd * pull + (right * uv.x + up * uv.y) * oh;
    let rd = fwd;
",
            "org",
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
            "t",
        ),
        _ => (
            "    let rd = normalize(fwd * {lens} + right * uv.x + up * uv.y);\n",
            "ro",
            "t",
        ),
    };
    let unit = per_unit(lens_kind, "t");
    let main = March {
        t: "t",
        hit: "hit",
        from: origin,
        dir: "rd",
        tolerance: "0.0001",
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
        last_w = hs / {unit};
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
    // Glow is always emitted and asked at run time, since Glow Amount is a uniform: at zero
    // each step costs one branch the whole frame takes the same way, and nothing is added.
    track_init.push_str(
        "    let glowing = {glowAmount} > 0.0;
    let reach = max({glowReach}, 0.001);
    var acc = 0.0;
",
    );
    track.push_str("        if (glowing) { acc += exp(-hs / reach) * h * relax; }\n");
    after_march.push_str(
        "    var gl = vec3f(0.0);
    var gla = 0.0;
    if (glowing) {
        gl = {glowAmount} * {glowColor}.rgb * (1.0 - exp(-acc / reach));
        gla = min(max(gl.r, max(gl.g, gl.b)), 1.0);
    }
",
    );
    post.push_str("    res = vec4f(res.rgb + gl, res.a + gla * (1.0 - res.a));\n");
    let measure = format!("        let hs = {dist}.x;\n        let h = min({floor}, hs);\n");
    // The statements that end the function on the color `expr`, through the outline where
    // Toon draws one and the glow.
    let give = |expr: &str| format!("    var res = {expr};\n{post}    return res;");
    let miss = format!("    if (!hit) {{\n{}    }}\n", indent(&give("bg")));

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
        let bounce = March {
            t: "t2",
            hit: "hit2",
            from: "ro2",
            dir: "rd2",
            tolerance: "(0.0004 * t + 0.0001)",
        };
        let unit2 = per_unit(lens_kind, "t + t2");
        // What the reflected ray sees where it hits nothing, and what its fog fades to.
        let (far2, beyond2) = if sky {
            (sky_at(ctx, node, "sky2", "rd2"), "sky2")
        } else {
            (String::new(), "bg")
        };
        format!(
            "    let rd2 = reflect(rd, n);
    let ro2 = pos + n * 0.002;
    var t2 = 0.01;
    var hit2 = false;
    sd_march = true;
    for (var k = 0; k < {}; k++) {{
        {AT} = ro2 + rd2 * t2;
        let h = {scene};
        if (h < 0.0004 * (t + t2) + 0.0001) {{ hit2 = true; break; }}
        t2 += h * relax;
        if (t2 > 80.0) {{ break; }}
    }}
    sd_march = false;
{}{far2}    var rcol = {beyond2}.rgb;
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
        rcol = mix(col2, {beyond2}.rgb, 1.0 - exp(-{{fog}} * t2));
    }}
    let gloss = clamp({{reflectivity}}, 0.0, 1.0);
    let fres = gloss + (1.0 - gloss) * pow(1.0 - clamp(dot(n, -rd), 0.0, 1.0), 5.0);
    col = mix(col, rcol, fres);
",
            (steps / 4).max(1),
            bounce.exhausted(&scene, &unit2, has_floor),
        )
    } else {
        String::new()
    };
    let col_kw = if bounce { "var" } else { "let" };
    // Fog shifts a hit's color toward the background's, and never its opacity: a solid is
    // solid however far off, and a transparent background fades it toward black, the color a
    // premultiplied transparent swatch keeps.
    let lit = give("vec4f(mix(col, unpremultiply(bg).rgb, fog), 1.0)");
    let finish = match shading {
        "normals" => give("vec4f(n * 0.5 + 0.5, 1.0)"),
        "depth" => give(&format!("vec4f(vec3f(exp(-{seen} * 0.12)), 1.0)")),
        "flat" if bounce => format!("    var col = albedo;\n{reflection}{lit}"),
        "flat" => give("vec4f(mix(albedo, unpremultiply(bg).rgb, fog), 1.0)"),
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
    // Steps shading paints the count alone, so it asks nothing of a ray that ran out.
    let exhausted = if shading == "steps" {
        String::new()
    } else {
        main.exhausted(&scene, &unit, has_floor)
    };
    // With the Sky connected, what a ray that hits nothing sees and what fog fades to is the
    // sky in its direction, asked once the march is done; otherwise it is the Background.
    let (bg_before, bg_after) = if sky {
        (String::new(), sky_at(ctx, node, "bg", "rd"))
    } else {
        ("    let bg = {background};\n".to_string(), String::new())
    };
    let rest = if shading == "steps" {
        "    return vec4f(clamp(vec3f(2.0 * heat, 2.0 - 2.0 * abs(2.0 * heat - 1.0) * 1.0, 1.0 - 2.0 * heat), vec3f(0.0), vec3f(1.0)), 1.0);".to_string()
    } else {
        format!(
            "{bg_after}{miss}    let pos = {origin} + rd * t;
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
{bg_before}    let relax = {{step}};
    var {AT} = vec3f(0.0);
    var t = 0.0;
    var hit = false;
    var count = 0.0;
{track_init}    sd_march = true;
    for (var i = 0; i < {steps}; i++) {{
        {AT} = {origin} + rd * t;
{measure}        if (h < 0.0004 * t + 0.0001) {{ hit = true; break; }}
{track}        t += h * relax;
        count += 1.0;
        if (t > 80.0) {{ break; }}
    }}
    sd_march = false;
{exhausted}{after_march}    let heat = count / {steps}.0;
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
        Part::When(
            Applies::Is {
                key: "camera",
                value: "lookat",
            },
            &[
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
        ),
    ],
    reset: &["targetX", "targetY", "targetZ", "fromX", "fromY", "fromZ"],
};

static GLOW: Popout = Popout {
    key: "glowPanel",
    caption: "Glow…",
    parts: &[
        Part::Note(&[
            "Glow is on while the Glow row",
            "is above zero, cabled or not.",
        ]),
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
        Part::When(
            Applies::Is {
                key: "shading",
                value: "toon",
            },
            &[Part::Popout(&TOON)],
        ),
        Part::Swatch {
            key: "background",
            caption: "Background",
        },
        Part::When(
            Applies::Cabled("sky"),
            &[Part::Number {
                key: "skyTilt",
                caption: "Sky Tilt",
            }],
        ),
        Part::Choice {
            key: "floor",
            caption: "Floor",
        },
        Part::When(
            Applies::IsNot {
                key: "floor",
                value: "none",
            },
            &[Part::Number {
                key: "floorHeight",
                caption: "Floor Height",
            }],
        ),
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
        Part::When(
            Applies::Is {
                key: "reflections",
                value: "one",
            },
            &[Part::Number {
                key: "reflectivity",
                caption: "Reflectivity",
            }],
        ),
    ],
};

pub static RENDER: NodeDef = NodeDef {
    slug: "solidrender",
    category: Category::Solid,
    icon: "🎥",
    label: "Render 3D",
    tooltip: "Turns a solid into a picture: a camera Distance away from the target, orbiting it \
              and tilted above it — or, with Camera on Look at, standing at From — lit from \
              Light. Drag the globe to move the camera and the sun to move the light. Plug the \
              picture into an Output, or into any video node. Steps shading paints how hard each \
              pixel worked. Glow above zero lights the air near a surface, Toon shading draws \
              light in Bands with an Outline, and One bounce reflections march a second ray from \
              every surface, costing up to a quarter as much again. Plug a picture into Sky and \
              it wraps the whole sphere round the scene: what a ray that hits nothing sees, what \
              fog fades to and what reflections show. A Translate along X before it turns it, \
              and Sky Tilt tips it. The sky picture is worked out once a pixel, and once more \
              for a reflection.",
    inputs: &[
        solid("solid", "Solid"),
        uniform(turn("orbit", "Orbit", 0.08, -1.0, 1.0, 0.001)),
        uniform(num("tilt", "Tilt", 0.06, -0.24, 0.24, 0.001, TURNS)),
        uniform(num_log("distance", "Distance", 7.0, 0.5, 80.0, 0.01, "⬓")),
        uniform(turn("light", "Light", 0.12, -1.0, 1.0, 0.001)),
        uniform(num("glowAmount", "Glow", 0.0, 0.0, 4.0, 0.001, "")),
        InputDef {
            key: "sky",
            label: "Sky",
            ty: VaryingColor,
            control: Control::None,
        },
    ],
    hidden: &[
        uniform(num("targetX", "Target X", 0.0, -20.0, 20.0, 0.01, "⬓")),
        uniform(num("targetY", "Target Y", 0.0, -20.0, 20.0, 0.01, "⬓")),
        uniform(num("targetZ", "Target Z", 0.0, -20.0, 20.0, 0.01, "⬓")),
        uniform(num("fromX", "From X", 3.135, -40.0, 40.0, 0.01, "⬓")),
        uniform(num("fromY", "From Y", 2.58, -40.0, 40.0, 0.01, "⬓")),
        uniform(num("fromZ", "From Z", 5.7, -40.0, 40.0, 0.01, "⬓")),
        uniform(num("lens", "Lens", 1.6, 0.3, 6.0, 0.01, "x")),
        uniform(num(
            "lightHeight",
            "Light Height",
            0.12,
            0.0,
            0.25,
            0.001,
            TURNS,
        )),
        uniform(num(
            "floorHeight",
            "Floor Height",
            -1.0,
            -20.0,
            20.0,
            0.01,
            "⬓",
        )),
        uniform(num("fog", "Fog", 0.03, 0.0, 1.0, 0.001, "")),
        uniform(num("step", "Step", 0.9, 0.2, 1.5, 0.01, "x")),
        color("background", "Background", "#141019ff"),
        uniform(turn("skyTilt", "Sky Tilt", 0.0, -0.5, 0.5, 0.001)),
        uniform(num(
            "reflectivity",
            "Reflectivity",
            0.25,
            0.0,
            1.0,
            0.001,
            "",
        )),
        color("glowColor", "Glow Color", "#59d8ffff"),
        uniform(num_log(
            "glowReach",
            "Glow Reach",
            0.25,
            0.01,
            4.0,
            0.001,
            "⬓",
        )),
        uniform(num("bands", "Bands", 3.0, 2.0, 8.0, 1.0, "")),
        uniform(num("outline", "Outline", 0.012, 0.0, 0.1, 0.0001, "")),
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
