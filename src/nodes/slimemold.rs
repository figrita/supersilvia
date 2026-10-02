// SPDX-License-Identifier: AGPL-3.0-or-later

//! Physarum: thousands of agents sniffing a scent field they are themselves depositing.
//!
//! Jeff Jones' rules (Artificial Life 16(2), 2010), silvia's implementation of them, **run on
//! the GPU**. The agents live in a storage buffer and the field in a pair of `r32f` images the
//! renderer keeps; the rules are the compute kernels below, and the picture a consumer samples
//! is an image they write — red the trail against its own running maximum, alpha where an
//! agent is standing — so nothing is read back and nothing is uploaded. The CPU half is what
//! has to be on the CPU: it reads the options, the presses and the knobs, turns this tick's
//! `dt` into a number of steps, and publishes the tick as a [`Simulation`] — see
//! [`crate::nodes::sim`] and `docs/decisions.md`, "A simulation steps on the GPU", for why
//! and what it cost on the CPU.
//!
//! **What reallocates the world is an option; what a step reads is a `UniformNumber`
//! input.** The grid and the population are `Runtime` options, because changing either means
//! a new world, and `docs/decisions.md` — "Not every CPU number is a `UniformNumber`" — is
//! why a cable may not do that once a frame. silvia's other eleven numbers are inputs with
//! controls, handed to every kernel as uniforms.
//!
//! **Steps are the clock's.** silvia steps `Speed` times in a lump thirty times a second off
//! a timer of its own; here the rate is the same — `Speed` × 30 steps a second — and the
//! steps are owed to the one clock's `dt` and paid on the tick they fall due, so the world
//! moves every tick, at the same speed whatever the display does, and an offline render
//! stepping virtual time makes the same world as live play.
//!
//! **Arrivals are counted, not summed.** Many agents land on one cell in one step, so the
//! motor kernel counts them with `atomicAdd` on a storage buffer of a `u32` per cell and the
//! scent a sensor reads is the field plus `DEPOSIT` per arrival. An integer count is exact and its total is
//! the same in whatever order the GPU ran the agents, which is what makes a run repeatable.
//!
//! **The world is a torus and its picture says so.** An agent that walks off an edge comes
//! back on the other one, its sensors sniff wrapped cells, its deposit lands on the cell it
//! came around to, and the diffusion reads across every edge; `trail` therefore declares
//! `TextureWrap::Repeat` — silvia's own `GL_REPEAT` for this texture, and the exception to
//! the mirror-wrap rule in `docs/rendering.md`, since the field is sampled at raw worldspace
//! `uv` and a mirrored tiling would fold a seam through a world that has none. The filter
//! stays `Linear`: a scent field is smooth, which is the whole of what a scent field is.
//!
//! **silvia's ways to find a look are both here, by two doors.** `Randomize` is an action
//! input, as silvia's is, and rolls Sense Angle, Turn Angle and Sense Dist to silvia's ranges
//! from the tick through `TickContext::write_control` — three uniforms, so a cable may fire
//! it — and then nudges the world. The nine presets are silvia's preset bar, a
//! [`Region::Buttons`] row: each press writes the three knobs *and* the Mode as one
//! `SetSettings`, which a tick cannot say, and pulses [`NUDGE`] so the tick nudges the world
//! as silvia's `_applyPreset` does. The nudge is silvia's `_nudgeSimulation`: the trail
//! knocked back to nine tenths and one agent in eight thrown somewhere else. The `Push` knob
//! and the pointer that drove it are not here, since pushing agents around needs pointer
//! input on a node's body.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber, VaryingColor, VaryingNumber};
use crate::nodes::rng::Rng;
use crate::nodes::sim::{Domain, Kernel, Pass, Simulation};
use crate::nodes::{
    Buttons, Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, ON, OptionDef,
    OptionKind, OutputDef, OutputKind, Region, Settings, TextureWrap, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "slimemold",
    category: Category::Generate,
    icon: "🧫",
    label: "Slime Mold",
    tooltip: "Simulates Physarum slime mold (Jones, 2010). Thousands of agents wander a grid, \
              each sniffing for scent trails with 3 forward sensors. They turn toward the \
              strongest scent (Attract) or away from it (Repel), drop scent when they move, \
              and the scent slowly diffuses and fades. Networks, veins, spots and stripes \
              emerge from just these rules. Sense Angle is how wide the sensors spread, Turn \
              Angle how sharply agents turn, Sense Dist how far ahead they sense.",
    inputs: &[
        InputDef {
            key: "trailColor",
            label: "Trail Color",
            ty: VaryingColor,
            control: Control::color("#ffcc11ff"),
        },
        InputDef {
            key: "agentColor",
            label: "Agent Color",
            ty: VaryingColor,
            control: Control::color("#ff4488ff"),
        },
        InputDef {
            key: "bgColor",
            label: "Background",
            ty: VaryingColor,
            control: Control::color("#0a0a1aff"),
        },
        InputDef {
            key: "randomSensors",
            label: "Randomize",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "scatter",
            label: "Scatter",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "clearTrails",
            label: "Clear",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "sensorAngle",
            label: "Sense Angle",
            ty: UniformNumber,
            control: Control::num(19.0, 1.0, 180.0, 0.5, "°"),
        },
        InputDef {
            key: "rotationAngle",
            label: "Turn Angle",
            ty: UniformNumber,
            control: Control::num(151.8, 1.0, 180.0, 0.5, "°"),
        },
        InputDef {
            key: "sensorOffset",
            label: "Sense Dist",
            ty: UniformNumber,
            control: Control::num(22.0, 1.0, 40.0, 1.0, ""),
        },
        InputDef {
            key: "decay",
            label: "Decay",
            ty: UniformNumber,
            control: Control::num(0.05, 0.001, 0.5, 0.001, ""),
        },
        InputDef {
            key: "jitter",
            label: "Jitter",
            ty: UniformNumber,
            control: Control::num(0.02, 0.0, 0.5, 0.005, ""),
        },
        InputDef {
            key: "trailGamma",
            label: "Gamma",
            ty: UniformNumber,
            control: Control::num(0.2, 0.1, 2.0, 0.05, ""),
        },
        InputDef {
            key: "heatmapStrength",
            label: "Heatmap",
            ty: UniformNumber,
            control: Control::num(5.0, 0.1, 100.0, 0.5, "x"),
        },
        InputDef {
            key: "stepsPerFrame",
            label: "Rate",
            ty: UniformNumber,
            control: Control::num(6.0, 1.0, 20.0, 1.0, "×30/s"),
        },
    ],
    outputs: &[
        OutputDef {
            key: "trail",
            label: "Trail",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The scent field itself, as the `PICTURE` kernel draws it: red the trail, shaped
            // by `Gamma` against its own running maximum, and alpha where an agent is
            // standing. Sampled at raw worldspace uv, silvia's own mapping, so the field
            // tiles across the world.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "trail");
                let sampler = ctx.sampler(node, "trail");
                format!("    return textureSampleLevel({tex}, {sampler}, uv, 0.0);")
            },
            // The field wraps in the simulation, so it wraps in the texture. `color` and
            // `value` sample this same texture, so they read through these parameters too.
            wrap: TextureWrap::Repeat,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "color",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            // silvia's, built from the two ticks: the background, then the trail over it by
            // the scent, then the agents over that by where they are. An input the ticks
            // leave out is never asked for, so its uniform is not declared and a
            // producer feeding it is not dragged into the shader.
            // The sibling port's texture through its own sampler, read once into a `let`.
            wgsl: |node, ctx, _func| {
                use std::fmt::Write as _;
                let background = ctx.input(node, "bgColor", "uv");
                let mut body = format!("    var c = {background};\n");
                let trails = ctx.option(node, "trails") == ON;
                let agents = ctx.option(node, "agents") == ON;
                if trails || agents {
                    let tex = ctx.texture_uniform(node, "trail");
                    let sampler = ctx.sampler(node, "trail");
                    let _ = writeln!(
                        body,
                        "    let trail = textureSampleLevel({tex}, {sampler}, uv, 0.0);"
                    );
                }
                if trails {
                    let color = ctx.input(node, "trailColor", "uv");
                    let _ = writeln!(body, "    c = mix(c, {color}, trail.r);");
                }
                if agents {
                    let color = ctx.input(node, "agentColor", "uv");
                    let _ = writeln!(body, "    c = mix(c, {color}, trail.a);");
                }
                body + "    return c;"
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "value",
            label: "Density",
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            // The raw quantity the picture was made from, which is what `value` means in the
            // vocabulary — silvia's `trail.r * heatmapStrength`, with the strength a port
            // rather than a node-local number.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "trail");
                let sampler = ctx.sampler(node, "trail");
                let heatmap = ctx.input(node, "heatmapStrength", "uv");
                format!("    return textureSampleLevel({tex}, {sampler}, uv, 0.0).r * {heatmap};")
            },
            range: Some("[0, 1] x Heatmap"),
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        OptionDef {
            key: "mode",
            label: "Mode",
            default: "attract",
            choices: &[("attract", "Attract"), ("repel", "Repel")],
            // Read by `tick` and handed to the kernels as a number: it flips which way a
            // sensor's answer turns an agent, and emits no fragment WGSL at all.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "gridScale",
            label: "Grid Scale",
            default: "12",
            // Every value silvia's own 2..16 s-number could hold, named by the grid it makes.
            choices: &[
                ("2", "32x32"),
                ("3", "48x48"),
                ("4", "64x64"),
                ("5", "80x80"),
                ("6", "96x96"),
                ("7", "112x112"),
                ("8", "128x128"),
                ("9", "144x144"),
                ("10", "160x160"),
                ("11", "176x176"),
                ("12", "192x192"),
                ("13", "208x208"),
                ("14", "224x224"),
                ("15", "240x240"),
                ("16", "256x256"),
            ],
            // Read by `tick`, which publishes the world at the new size: the renderer
            // reallocates the field and the agents on it.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "population",
            label: "Population",
            default: "20",
            // silvia's Pop % is a 1..40 s-number; these are the ladder of it, since a
            // population is an allocation and an option holds a list.
            choices: &[
                ("1", "1%"),
                ("2", "2%"),
                ("5", "5%"),
                ("10", "10%"),
                ("15", "15%"),
                ("20", "20%"),
                ("25", "25%"),
                ("30", "30%"),
                ("35", "35%"),
                ("40", "40%"),
            ],
            // Read by `tick`, which publishes the new population: the renderer grows or
            // shrinks the agents' buffer on it.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        // silvia's two layer checkboxes. `Code`, unlike every other tick here: what they
        // change is the picture's WGSL, so changing one rebuilds the shader.
        OptionDef::check("trails", "Trails", true, OptionKind::Code),
        OptionDef::check("agents", "Agents", false, OptionKind::Code),
    ],
    width: Some(240.0),
    regions: &[Region::Buttons(&PRESET_BAR)],
    cpu: Some(CpuDef {
        create: || Box::new(Mold::new()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// silvia's nine presets, in its order: Sense Angle and Turn Angle in degrees, Sense Dist in
/// cells, and the Mode.
pub const PRESETS: [(f32, f32, f32, &str); 9] = [
    (19.0, 151.8, 22.0, "attract"),
    (15.9, 5.6, 18.0, "repel"),
    (41.0, 45.4, 31.0, "attract"),
    (141.7, 68.8, 31.0, "repel"),
    (14.7, 108.4, 9.0, "attract"),
    (168.0, 44.2, 37.0, "repel"),
    (12.9, 156.4, 15.0, "attract"),
    (141.1, 5.1, 17.0, "repel"),
    (35.0, 70.0, 4.0, "attract"),
];

/// What a preset asks the tick for beside the settings it writes: silvia's nudge.
pub const NUDGE: &str = "nudge";

/// silvia's preset bar: nine buttons numbered from one, each writing its preset.
pub static PRESET_BAR: Buttons = Buttons {
    buttons: &[
        ("preset1", "1"),
        ("preset2", "2"),
        ("preset3", "3"),
        ("preset4", "4"),
        ("preset5", "5"),
        ("preset6", "6"),
        ("preset7", "7"),
        ("preset8", "8"),
        ("preset9", "9"),
    ],
    press: |index, _| preset(index),
    pulse: Some(NUDGE),
};

/// The settings preset `index` writes: the three sensing knobs and the Mode.
pub fn preset(index: usize) -> Settings {
    let Some(&(sensor, rotation, offset, mode)) = PRESETS.get(index) else {
        return Settings::default();
    };
    Settings {
        options: vec![("mode", mode.to_string())],
        controls: vec![
            ("sensorAngle", sensor),
            ("rotationAngle", rotation),
            ("sensorOffset", offset),
        ],
    }
}

/// silvia's `_randomizeParams`: Sense Angle and Turn Angle anywhere from 1 to 180 degrees, and
/// Sense Dist a whole number of cells from 1 to 39.
fn roll(rng: &mut Rng) -> [(&'static str, f32); 3] {
    [
        ("sensorAngle", 1.0 + rng.next_f32() * 179.0),
        ("rotationAngle", 1.0 + rng.next_f32() * 179.0),
        ("sensorOffset", 1.0 + (rng.next_f32() * 39.0).floor()),
    ]
}

/// silvia's grid unit: the `gridScale` option counts sixteens.
const GRID_UNIT: u32 = 16;

/// silvia's batch rate: `Speed` steps, thirty times a second.
const HZ: f64 = 30.0;

/// The most steps a `Speed` asks for per thirtieth of a second: silvia's maximum.
const MAX_SPEED: f32 = 20.0;

/// The most steps one tick pays, whatever its `dt`: a second of the fastest world. A live
/// `dt` is clamped to a tenth of that; this bounds a driven one that is not.
const MAX_STEPS_PER_TICK: u64 = 600;

/// Where the running maximum the picture is normalized against starts: silvia's.
const START_MAX: f32 = 20.0;

/// silvia's smoothing of that maximum, per thirtieth of a second: rise fast, fall slow.
const RISE: f32 = 0.3;
const FALL: f32 = 0.02;

/// The helpers every kernel shares: the deposit, a random per agent and draw, the cell a
/// position wraps to, and the scent at a cell. Its numbers are members of the renderer's
/// uniform struct `u`, written from [`PARAMS`], so it declares none of them.
const COMMON_WGSL: &str = "
const DEPOSIT: f32 = 5.0;
const TAU: f32 = 6.28318530718;

fn mold_hash(v: u32) -> u32 {
    var x = v;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

// A random in [0, 1) for agent `i`, this pass's seed, and the `k`th draw within it.
fn mold_random(i: i32, k: u32) -> f32 {
    let h = mold_hash(mold_hash(u.u_seed ^ (k * 0x9e3779b9u)) ^ u32(i));
    return f32(h >> 8u) / 16777216.0;
}

// The cell holding a position on a world that wraps. Floored, so half a cell before the
// origin is the far cell and not the first one; a floored modulo, not `%`, which truncates.
fn cell_of(p: vec2f) -> vec2i {
    let n = f32(u.u_size);
    let f = floor(p);
    let c = vec2i(f - n * floor(f / n));
    return clamp(c, vec2i(0), vec2i(u.u_size - 1));
}

fn wrapped(c: vec2i) -> vec2i {
    let n = vec2i(u.u_size);
    return (c + n) % n;
}

// The field, and every deposit made this step.
fn scent(c: vec2i) -> f32 {
    return textureLoad(field, c).r + DEPOSIT * f32(atomicLoad(&arrivals[sim_index(c)]));
}
";

/// The numbers every kernel reads beyond the pass's own, in the order the renderer lays them
/// into its uniform struct, and the names [`Simulation::params`] carries.
pub const PARAMS: [&str; 9] = [
    "u_sensor",
    "u_rotation",
    "u_offset",
    "u_decay",
    "u_jitter",
    "u_repel",
    "u_gamma",
    "u_rise",
    "u_fall",
];

/// The motor stage: every agent may jitter, steps one cell and lands. An agent that would
/// leave the grid comes back on the other side, and one comparison each way is enough, because
/// the step is one cell. The landing is counted, and the count is the deposit.
pub static MOTOR: Kernel = Kernel {
    name: "motor",
    over: Domain::Agents,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_agent(i: i32) {
    var a = agents[i];
    if (mold_random(i, 0u) < u.u_jitter) {
        a.z = mold_random(i, 1u) * TAU;
    }
    var p = a.xy + vec2f(cos(a.z), sin(a.z));
    let n = vec2f(f32(u.u_size));
    p += select(vec2f(0.0), n, p < vec2f(0.0)) - select(vec2f(0.0), n, p >= n);
    agents[i] = vec4f(p, a.zw);
    atomicAdd(&arrivals[sim_index(cell_of(p))], 1u);
}
",
    flips: false,
};

/// The sensory stage: every agent sniffs ahead and to either side, and turns. silvia's
/// rule: under Attract, carry on where forward is strongest, pick a side at random where it
/// is weakest, and otherwise turn toward the stronger side; Repel is the same with the field's
/// sign flipped. The two side sensors come from the angle addition identities rather than two
/// more `sin` and `cos`, silvia's own saving.
pub static SENSE: Kernel = Kernel {
    name: "sense",
    over: Domain::Agents,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_agent(i: i32) {
    var a = agents[i];
    let ahead = vec2f(cos(a.z), sin(a.z));
    let cs = cos(u.u_sensor);
    let ss = sin(u.u_sensor);
    let left = vec2f(ahead.x * cs - ahead.y * ss, ahead.y * cs + ahead.x * ss);
    let right = vec2f(ahead.x * cs + ahead.y * ss, ahead.y * cs - ahead.x * ss);
    let polarity = select(1.0, -1.0, u.u_repel > 0.5);
    let f = polarity * scent(cell_of(a.xy + ahead * u.u_offset));
    let l = polarity * scent(cell_of(a.xy + left * u.u_offset));
    let r = polarity * scent(cell_of(a.xy + right * u.u_offset));
    if (f > l && f > r) {
        return;
    }
    if (f < l && f < r) {
        if (mold_random(i, 0u) < 0.5) {
            a.z += u.u_rotation;
        } else {
            a.z -= u.u_rotation;
        }
    } else if (l < r) {
        a.z -= u.u_rotation;
    } else if (r < l) {
        a.z += u.u_rotation;
    }
    agents[i].z = a.z;
}
",
    flips: false,
};

/// The field diffuses and fades: a 3x3 mean with the decay folded into the divisor, wrapped
/// at the edges, into the other field — and the other arrivals zeroed for the next step, so
/// the ones this step counted stay behind as where the agents are standing. Each workgroup
/// reads the scent of its tile and the ring around it once, into shared memory, and sums
/// from there: a ninth of the reads a cell would make on its own.
pub static DIFFUSE: Kernel = Kernel {
    name: "diffuse",
    over: Domain::Tiles,
    wgsl_common: COMMON_WGSL,
    wgsl: "
const RIM: i32 = SIM_TILE + 2;
var<workgroup> tile: array<f32, RIM * RIM>;

fn run_tile(c: vec2i, inside: bool) {
    let origin = sim_group * SIM_TILE - 1;
    for (var k = sim_local_index; k < RIM * RIM; k += SIM_TILE * SIM_TILE) {
        tile[k] = scent(wrapped(origin + vec2i(k % RIM, k / RIM)));
    }
    workgroupBarrier();
    if (!inside) {
        return;
    }
    let at = sim_local + 1;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            sum += tile[(at.y + dy) * RIM + at.x + dx];
        }
    }
    textureStore(field_next, c, vec4f(sum * ((1.0 - u.u_decay) / 9.0)));
    atomicStore(&arrivals_next[sim_index(c)], 0u);
}
",
    flips: true,
};

/// The field's highest cell, into `state[0]`. A non-negative float orders as its bits do, so
/// the maximum is an integer atomic; most cells are below what has been found already and
/// never reach it.
pub static PEAK: Kernel = Kernel {
    name: "peak",
    over: Domain::Cells,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_cell(c: vec2i) {
    let v = bitcast<u32>(max(textureLoad(field, c).r, 0.0));
    if (v > atomicLoad(&state[0])) {
        atomicMax(&state[0], v);
    }
}
",
    flips: false,
};

/// The running maximum in `state[1]` moves toward the peak — silvia's rise fast, fall slow,
/// at the rate the tick's `dt` makes of it — and the peak is emptied for the next tick.
pub static SETTLE: Kernel = Kernel {
    name: "settle",
    over: Domain::Once,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_once() {
    let peak = max(bitcast<f32>(atomicLoad(&state[0])), 1.0);
    var held = bitcast<f32>(atomicLoad(&state[1]));
    held += (peak - held) * select(u.u_fall, u.u_rise, peak > held);
    atomicStore(&state[1], bitcast<u32>(held));
    atomicStore(&state[0], 0u);
}
",
    flips: false,
};

/// The picture: the field over its running maximum, shaped by `Gamma`, in red; where an
/// agent landed on the last step, in alpha.
pub static PICTURE: Kernel = Kernel {
    name: "picture",
    over: Domain::Cells,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_cell(c: vec2i) {
    let held = max(bitcast<f32>(atomicLoad(&state[1])), 1e-30);
    let v = pow(clamp(textureLoad(field, c).r / held, 0.0, 1.0), u.u_gamma);
    let standing = select(0.0, 1.0, atomicLoad(&arrivals_next[sim_index(c)]) > 0u);
    textureStore(picture, c, vec4f(v, 0.0, 0.0, standing));
}
",
    flips: false,
};

/// A world's first moment: no peak yet, and the maximum where the pass's argument starts it.
pub static INIT: Kernel = Kernel {
    name: "init",
    over: Domain::Once,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_once() {
    atomicStore(&state[0], 0u);
    atomicStore(&state[1], bitcast<u32>(u.u_arg));
}
",
    flips: false,
};

/// silvia's `Clear`: no scent, and nobody standing anywhere.
pub static CLEAR: Kernel = Kernel {
    name: "clear",
    over: Domain::Cells,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_cell(c: vec2i) {
    textureStore(field, c, vec4f(0.0));
    atomicStore(&arrivals[sim_index(c)], 0u);
    atomicStore(&arrivals_next[sim_index(c)], 0u);
}
",
    flips: false,
};

/// The scent knocked back by the pass's argument: the first half of silvia's nudge.
pub static FADE: Kernel = Kernel {
    name: "fade",
    over: Domain::Cells,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_cell(c: vec2i) {
    textureStore(field, c, textureLoad(field, c) * u.u_arg);
}
",
    flips: false,
};

/// Agents somewhere else, facing anywhere.
pub static THROW: Kernel = Kernel {
    name: "throw",
    over: Domain::Agents,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_agent(i: i32) {
    let n = f32(u.u_size);
    agents[i] = vec4f(mold_random(i, 0u) * n, mold_random(i, 1u) * n, mold_random(i, 2u) * TAU, 0.0);
}
",
    flips: false,
};

/// Agents carried onto a grid the pass's argument times the size of the one they stood on.
pub static SCALE: Kernel = Kernel {
    name: "scale",
    over: Domain::Agents,
    wgsl_common: COMMON_WGSL,
    wgsl: "
fn run_agent(i: i32) {
    let a = agents[i];
    agents[i] = vec4f(a.xy * u.u_arg, a.zw);
}
",
    flips: false,
};

/// Every kernel, for the test that compiles them all.
pub static KERNELS: [&Kernel; 11] = [
    &MOTOR, &SENSE, &DIFFUSE, &PEAK, &SETTLE, &PICTURE, &INIT, &CLEAR, &FADE, &THROW, &SCALE,
];

struct Mold {
    /// The world as the last tick left it: cells across, and agents on it. Zero before the
    /// first tick, which is what makes the first tick a birth.
    size: u32,
    agents: u32,
    /// The fraction of a step the clock has paid for and the world has not taken: the one
    /// clock's `dt`, times the rate, less every step taken.
    owed: f64,
    /// Steps taken since the world was born.
    steps: u64,
    /// Steps a second, as the last tick asked, for the Status box.
    rate: f64,
    randomize: Gate,
    nudge: Gate,
    scatter: Gate,
    clear: Gate,
    rng: Rng,
}

/// What the presses asked of the world this tick: Scatter, Clear, and the nudge a Randomize
/// or a preset ends in.
#[derive(Debug, Clone, Copy, Default)]
struct Presses {
    nudge: bool,
    scatter: bool,
    clear: bool,
}

/// Everything one tick read, so a tick's plan is a function of it and of the state alone.
#[derive(Debug, Clone, Copy)]
struct Reading {
    size: u32,
    agents: u32,
    presses: Presses,
    dt: f32,
    /// `Speed`: steps per thirtieth of a second.
    speed: f32,
    /// In radians.
    sensor: f32,
    rotation: f32,
    /// In cells.
    offset: f32,
    decay: f32,
    jitter: f32,
    repel: bool,
    gamma: f32,
}

impl Mold {
    fn new() -> Self {
        Self {
            size: 0,
            agents: 0,
            owed: 0.0,
            steps: 0,
            rate: 0.0,
            randomize: Gate::default(),
            nudge: Gate::default(),
            scatter: Gate::default(),
            clear: Gate::default(),
            rng: Rng::new(),
        }
    }

    /// This tick of the world: what changed shape, what a press asked for, the steps the
    /// clock paid for, and the picture of where that left it.
    fn plan(&mut self, r: &Reading) -> Simulation {
        let mut passes = Vec::new();
        if self.size == 0 {
            // Born: silvia's `_initSimulation`, a clean field and every agent thrown.
            self.size = r.size;
            self.agents = r.agents;
            passes.push(Pass::of(&INIT).with_arg(START_MAX));
            passes.push(Pass::of(&CLEAR));
            passes.push(
                Pass::of(&THROW)
                    .over(0, r.agents, 1)
                    .seeded(self.rng.next_u32()),
            );
        } else {
            // A new grid: the renderer resamples the field nearest-neighbor and keeps the
            // agents it can, which are in cells and so move with the grid they are on —
            // silvia's `_rescaleGrid`. Then whoever is new is thrown.
            if r.size != self.size {
                let kept = self.agents.min(r.agents);
                let ratio = r.size as f32 / self.size as f32;
                passes.push(Pass::of(&SCALE).over(0, kept, 1).with_arg(ratio));
                self.size = r.size;
            }
            if r.agents > self.agents {
                passes.push(
                    Pass::of(&THROW)
                        .over(self.agents, r.agents, 1)
                        .seeded(self.rng.next_u32()),
                );
            }
            self.agents = r.agents;
        }

        if r.presses.nudge {
            // silvia's `_nudgeSimulation`, which `Randomize` and every preset end in.
            passes.push(Pass::of(&FADE).with_arg(0.9));
            passes.push(
                Pass::of(&THROW)
                    .over(0, self.agents, 8)
                    .seeded(self.rng.next_u32()),
            );
        }
        if r.presses.scatter {
            passes.push(
                Pass::of(&THROW)
                    .over(0, self.agents, 1)
                    .seeded(self.rng.next_u32()),
            );
        }
        if r.presses.clear {
            passes.push(Pass::of(&CLEAR));
        }

        let speed = if r.speed.is_finite() {
            r.speed.round().clamp(1.0, MAX_SPEED)
        } else {
            1.0
        };
        self.rate = f64::from(speed) * HZ;
        self.owed += f64::from(r.dt.max(0.0)) * self.rate;
        let due = self.owed.floor();
        self.owed -= due;
        let steps = (due as u64).min(MAX_STEPS_PER_TICK);
        for _ in 0..steps {
            passes.push(Pass::of(&MOTOR).seeded(self.rng.next_u32()));
            passes.push(Pass::of(&SENSE).seeded(self.rng.next_u32()));
            passes.push(Pass::of(&DIFFUSE));
        }
        self.steps += steps;

        passes.push(Pass::of(&PEAK));
        passes.push(Pass::of(&SETTLE));
        passes.push(Pass::of(&PICTURE));

        let (rise, fall) = smoothing(r.dt);
        Simulation {
            size: self.size,
            agents: self.agents,
            params: PARAMS
                .into_iter()
                .zip([
                    r.sensor,
                    r.rotation,
                    r.offset,
                    r.decay,
                    r.jitter,
                    if r.repel { 1.0 } else { 0.0 },
                    r.gamma,
                    rise,
                    fall,
                ])
                .collect(),
            passes,
            steps: self.steps,
        }
    }
}

/// silvia's rise and fall of the running maximum, each a fraction per thirtieth of a second,
/// as the fraction `dt` seconds make of them.
fn smoothing(dt: f32) -> (f32, f32) {
    let batches = dt.max(0.0) * HZ as f32;
    (
        1.0 - (1.0 - RISE).powf(batches),
        1.0 - (1.0 - FALL).powf(batches),
    )
}

impl CpuNode for Mold {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        let scale: u32 = ctx.option(id, "gridScale").parse().unwrap_or(12);
        let size = scale.clamp(2, 16) * GRID_UNIT;
        let percent: f32 = ctx.option(id, "population").parse().unwrap_or(20.0);
        let agents = ((size * size) as f32 * percent.clamp(1.0, 40.0) / 100.0) as u32;
        let degrees = std::f32::consts::PI / 180.0;
        let randomize = ctx.downs(id, "randomSensors", &mut self.randomize) > 0;
        if randomize {
            // Onto the knobs, as a hand would turn them: this tick steps with what they said,
            // and the next with what was rolled.
            for (key, value) in roll(&mut self.rng) {
                ctx.write_control(id, key, value);
            }
        }
        let preset = ctx.downs(id, NUDGE, &mut self.nudge) > 0;
        let reading = Reading {
            size,
            agents,
            presses: Presses {
                nudge: randomize || preset,
                scatter: ctx.downs(id, "scatter", &mut self.scatter) > 0,
                clear: ctx.downs(id, "clearTrails", &mut self.clear) > 0,
            },
            dt: ctx.dt,
            speed: ctx.input(id, "stepsPerFrame"),
            sensor: ctx.input(id, "sensorAngle").max(0.0) * degrees,
            rotation: ctx.input(id, "rotationAngle").max(0.0) * degrees,
            offset: ctx.input(id, "sensorOffset").max(0.0),
            decay: ctx.input(id, "decay").clamp(0.0, 1.0),
            jitter: ctx.input(id, "jitter").clamp(0.0, 1.0),
            repel: ctx.option(id, "mode") == "repel",
            gamma: ctx.input(id, "trailGamma").clamp(0.01, 8.0),
        };
        let sim = self.plan(&reading);
        ctx.publish_sim(id, "trail", sim);
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "slimemold {}x{}, {} agents, {:.0} steps/s on the GPU",
            self.size, self.size, self.agents, self.rate
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading() -> Reading {
        let degrees = std::f32::consts::PI / 180.0;
        Reading {
            size: 192,
            agents: 7372,
            presses: Presses::default(),
            dt: 1.0 / 100.0,
            speed: 6.0,
            sensor: 19.0 * degrees,
            rotation: 151.8 * degrees,
            offset: 22.0,
            decay: 0.05,
            jitter: 0.02,
            repel: false,
            gamma: 0.2,
        }
    }

    fn mold() -> Mold {
        let mut mold = Mold::new();
        mold.rng.seed(NodeId(1));
        mold
    }

    fn kernels(sim: &Simulation) -> Vec<&'static str> {
        sim.passes.iter().map(|p| p.kernel.name).collect()
    }

    fn count(sim: &Simulation, kernel: &Kernel) -> usize {
        sim.passes
            .iter()
            .filter(|p| std::ptr::eq(p.kernel, kernel))
            .count()
    }

    /// Every `gridScale` and `population` choice is a number this parses, and the defaults
    /// are silvia's 192 square at a fifth full.
    #[test]
    fn every_choice_is_a_number_and_the_defaults_are_silvias() {
        for (value, _) in DEF.option("gridScale").unwrap().choices {
            let scale: u32 = value
                .parse()
                .unwrap_or_else(|_| panic!("{value} is a scale"));
            assert!(
                (2..=16).contains(&scale),
                "{value} is outside silvia's range"
            );
        }
        for (value, _) in DEF.option("population").unwrap().choices {
            let percent: f32 = value
                .parse()
                .unwrap_or_else(|_| panic!("{value} is a percent"));
            assert!((1.0..=40.0).contains(&percent), "{value} is outside 1..40");
        }
        let scale: u32 = DEF.option("gridScale").unwrap().default.parse().unwrap();
        assert_eq!(scale * GRID_UNIT, 192);
        assert_eq!(DEF.option("population").unwrap().default, "20");
    }

    /// The first tick is a birth — a starting maximum, a clean field and every agent thrown —
    /// and every tick ends in the picture of where it left the world.
    #[test]
    fn the_first_tick_is_a_birth_and_every_tick_draws() {
        let mut mold = mold();
        let first = mold.plan(&reading());
        assert_eq!((first.size, first.agents), (192, 7372));
        assert_eq!(&kernels(&first)[..3], ["init", "clear", "throw"]);
        assert_eq!(
            (
                first.passes[2].from,
                first.passes[2].to,
                first.passes[2].stride
            ),
            (0, 7372, 1),
            "every agent, once"
        );
        assert_eq!(
            &kernels(&first)[first.passes.len() - 3..],
            ["peak", "settle", "picture"]
        );
        let next = mold.plan(&reading());
        assert_eq!(count(&next, &INIT), 0, "born once");
        assert_eq!(count(&next, &THROW), 0);
        assert_eq!(count(&next, &PICTURE), 1, "and drawn every tick");
    }

    /// silvia's rate — `Speed` steps thirty times a second — paid as the clock goes rather than
    /// in a lump: a second of 100 Hz ticks at the default speed is 180 steps, one or two a tick.
    #[test]
    fn steps_are_owed_to_the_clock_and_paid_every_tick() {
        let mut mold = mold();
        let mut total = 0;
        for _ in 0..100 {
            let sim = mold.plan(&reading());
            let steps = count(&sim, &MOTOR);
            assert_eq!(steps, count(&sim, &SENSE));
            assert_eq!(steps, count(&sim, &DIFFUSE));
            assert!((1..=2).contains(&steps), "{steps} steps in one tick");
            total += steps;
        }
        assert!((179..=180).contains(&total), "{total} steps in a second");
        assert_eq!(mold.steps, total as u64);

        // A tick with no time in it takes no step, and draws the same picture again.
        let mut still = reading();
        still.dt = 0.0;
        let sim = mold.plan(&still);
        assert_eq!(count(&sim, &MOTOR), 0);
        assert_eq!(count(&sim, &PICTURE), 1);

        // A stall is paid, up to a second of the fastest world.
        let mut stalled = reading();
        stalled.dt = 1000.0;
        stalled.speed = 1000.0;
        let sim = mold.plan(&stalled);
        assert_eq!(count(&sim, &MOTOR) as u64, MAX_STEPS_PER_TICK);
    }

    /// The same seed and the same `dt`s are the same passes, seeds and all — which is what makes
    /// the GPU's world the same one twice. Another node is another world.
    #[test]
    fn the_same_clock_makes_the_same_world() {
        let dts = [0.016, 0.017, 0.1, 0.0, 0.004, 0.033];
        let run = |id: u32| {
            let mut mold = Mold::new();
            mold.rng.seed(NodeId(id));
            dts.iter()
                .map(|dt| {
                    let mut r = reading();
                    r.dt = *dt;
                    mold.plan(&r)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8), "a different node, the same seeds");
    }

    /// A new grid carries the agents onto it and throws the ones the larger world has room
    /// for; a smaller population keeps the first of them and throws nobody.
    #[test]
    fn a_new_grid_moves_the_agents_and_throws_the_new_ones() {
        let mut mold = mold();
        let mut r = reading();
        r.size = 32;
        r.agents = 204;
        mold.plan(&r);
        r.size = 64;
        r.agents = 819;
        let sim = mold.plan(&r);
        assert_eq!((sim.size, sim.agents), (64, 819));
        let scale = sim
            .passes
            .iter()
            .find(|p| p.kernel.name == "scale")
            .unwrap();
        assert_eq!((scale.from, scale.to, scale.arg), (0, 204, 2.0));
        let throw = sim
            .passes
            .iter()
            .find(|p| p.kernel.name == "throw")
            .unwrap();
        assert_eq!((throw.from, throw.to), (204, 819), "only the new ones");

        r.agents = 40;
        let sim = mold.plan(&r);
        assert_eq!(sim.agents, 40);
        assert_eq!(count(&sim, &THROW) + count(&sim, &SCALE), 0);
    }

    /// Each press is silvia's: Clear empties the field, Scatter throws everybody, and
    /// the nudge that Randomize and a preset end in knocks the scent back and throws one agent
    /// in eight.
    #[test]
    fn a_press_is_the_passes_silvia_ran() {
        let mut mold = mold();
        mold.plan(&reading());
        let press = |mold: &mut Mold, f: fn(&mut Reading)| {
            let mut r = reading();
            r.dt = 0.0;
            f(&mut r);
            mold.plan(&r)
        };
        let sim = press(&mut mold, |r| r.presses.clear = true);
        assert_eq!(kernels(&sim), ["clear", "peak", "settle", "picture"]);
        let sim = press(&mut mold, |r| r.presses.scatter = true);
        assert_eq!(
            (
                sim.passes[0].kernel.name,
                sim.passes[0].to,
                sim.passes[0].stride
            ),
            ("throw", 7372, 1)
        );
        let sim = press(&mut mold, |r| r.presses.nudge = true);
        assert_eq!(kernels(&sim)[..2], ["fade", "throw"]);
        assert_eq!(sim.passes[0].arg, 0.9);
        assert_eq!(sim.passes[1].stride, 8, "one agent in eight");
    }

    /// silvia's nine presets, as its table has them, each writing the three sensing knobs and
    /// the Mode — every value inside the knob's own range and a choice the Mode offers — and
    /// its first is the node's own defaults, as silvia's is.
    #[test]
    fn the_presets_are_silvias_and_land_on_the_knobs() {
        let silvia = [
            (19.0, 151.8, 22.0, "attract"),
            (15.9, 5.6, 18.0, "repel"),
            (41.0, 45.4, 31.0, "attract"),
            (141.7, 68.8, 31.0, "repel"),
            (14.7, 108.4, 9.0, "attract"),
            (168.0, 44.2, 37.0, "repel"),
            (12.9, 156.4, 15.0, "attract"),
            (141.1, 5.1, 17.0, "repel"),
            (35.0, 70.0, 4.0, "attract"),
        ];
        assert_eq!(PRESET_BAR.buttons.len(), silvia.len());
        for (i, (sensor, rotation, offset, mode)) in silvia.into_iter().enumerate() {
            assert_eq!(
                PRESET_BAR.buttons[i].1,
                (i + 1).to_string(),
                "numbered from one"
            );
            let settings = (PRESET_BAR.press)(i, 12345);
            assert_eq!(settings.options, vec![("mode", mode.to_string())]);
            assert_eq!(
                settings.controls,
                vec![
                    ("sensorAngle", sensor),
                    ("rotationAngle", rotation),
                    ("sensorOffset", offset),
                ]
            );
            for (key, value) in settings.controls {
                let range = crate::nodes::declared_range(&DEF, key).unwrap();
                assert!((range.min..=range.max).contains(&value), "{key} {value}");
            }
            assert!(crate::nodes::option_is_valid(&DEF, "mode", mode));
        }
        let first = preset(0);
        for (key, value) in first.controls {
            let Control::Number { default, .. } = DEF.input(key).unwrap().control else {
                panic!("{key} is a number");
            };
            assert_eq!(value, default, "{key}");
        }
        assert_eq!(DEF.option("mode").unwrap().default, "attract");
        assert_eq!(
            PRESET_BAR.pulse,
            Some(NUDGE),
            "and a preset nudges the world"
        );
        assert_eq!(preset(9), Settings::default(), "there is no tenth");
    }

    /// Randomize rolls silvia's ranges: both angles anywhere from 1 to 180, Sense Dist a whole
    /// number of cells from 1 to 39 — each reached, over enough rolls, and each inside the
    /// knob's own range.
    #[test]
    fn randomize_rolls_silvias_ranges() {
        let mut rng = Rng::new();
        rng.seed(NodeId(3));
        let mut offsets = std::collections::BTreeSet::new();
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for _ in 0..5000 {
            let [(a, sensor), (b, rotation), (c, offset)] = roll(&mut rng);
            assert_eq!((a, b, c), ("sensorAngle", "rotationAngle", "sensorOffset"));
            for angle in [sensor, rotation] {
                assert!((1.0..=180.0).contains(&angle), "{angle}");
                low = low.min(angle);
                high = high.max(angle);
            }
            assert_eq!(offset.fract(), 0.0, "{offset}");
            assert!((1.0..=39.0).contains(&offset), "{offset}");
            offsets.insert(offset as u32);
        }
        assert!(low < 3.0 && high > 178.0, "{low}..{high}");
        assert_eq!(
            offsets,
            (1..=39).collect(),
            "every distance silvia's can roll"
        );
    }

    /// The running maximum moves at silvia's rates when the ticks are silvia's thirtieths, and
    /// by as much in two ticks of half the length.
    #[test]
    fn the_maximum_settles_at_silvias_rate_whatever_the_tick() {
        let (rise, fall) = smoothing(1.0 / 30.0);
        assert!((rise - RISE).abs() < 1e-5 && (fall - FALL).abs() < 1e-5);
        let (half_rise, _) = smoothing(1.0 / 60.0);
        let twice = 1.0 - (1.0 - half_rise) * (1.0 - half_rise);
        assert!((twice - RISE).abs() < 1e-5, "{twice}");
        assert_eq!(smoothing(0.0), (0.0, 0.0), "no time, no movement");
        let first = mold().plan(&reading());
        assert_eq!(
            first.passes[0].arg, 20.0,
            "and a birth starts it where silvia does"
        );
    }

    /// Each kernel defines the function its domain calls, and names nothing it would have to
    /// declare for itself.
    #[test]
    fn every_kernel_defines_what_its_domain_calls() {
        for kernel in KERNELS {
            let entry = match kernel.over {
                Domain::Agents => "fn run_agent(i: i32)",
                Domain::Cells => "fn run_cell(c: vec2i)",
                Domain::Tiles => "fn run_tile(c: vec2i, inside: bool)",
                Domain::Once => "fn run_once()",
            };
            assert!(kernel.wgsl.contains(entry), "{} lacks {entry}", kernel.name);
            assert_eq!(
                kernel.wgsl_common, COMMON_WGSL,
                "{} shares the helpers",
                kernel.name
            );
        }
        assert_eq!(
            KERNELS.iter().filter(|k| k.flips).count(),
            1,
            "only the diffusion writes the other field"
        );
    }
}
