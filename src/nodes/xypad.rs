// SPDX-License-Identifier: AGPL-3.0-or-later

//! A puck you throw around a square, published as X, Y, its speed and a gate on every bounce.
//!
//! Ported from silvia's `xypad.js`: drag, bounce, a spring to the middle, gravity on each axis,
//! a temperature that jitters it, the four range ends, an edge behavior per axis, gravity
//! wells and tethers dropped with a right-drag, and silvia's nine presets. The pad is
//! [`Region::XyPad`](super::Region::XyPad), a region of the node's body that **claims the
//! pointer**, so a press inside it throws the puck and never carries the node.
//!
//! **Where the hand put the puck is the document; the flight is not.** silvia keeps `x`, `y`,
//! `vx` and `vy` in `values`, and a number in `values` is a control here
//! (docs/cpu.md) — so they are four hidden controls, `padX`, `padY`, `vx` and `vy`, written
//! through the bus by the pad as a gesture and by a preset as a press, saved, undone as one
//! step and bound to a MIDI knob like any other. The tick owns the puck from there: it takes
//! each of the four the moment the document changes it, and integrates everything in between,
//! so a knob sweeping Pad X moves the puck and a throw carries on after the hand has let go.
//! What the puck did since is runtime, as silvia's `runtimeState` is — the wells, the trail
//! and where the flight has got to are not saved, and a patch reopens with the puck where
//! the hand last put it.
//!
//! **The puck lives on the pad, -1 to 1 across it and up positive.** silvia integrates in
//! the output's own units, between Min and Max; here the physics runs on the square and the
//! four range ends map it onto X and Y. At silvia's default range of -1 to 1 the two are the
//! same numbers. Anywhere else a preset plays the same motion across the pad rather than a
//! motion scaled by whatever range the outputs were given, and moving a range end moves what
//! the pad publishes without moving the puck under the hand.
//!
//! **Held, the puck does not fly.** silvia pauses its step while the pointer is down; here the
//! pad reports the hand as a level under [`PUCK`], the way a finger on an action input's
//! button is reported, and the tick reads it with `TickContext::pressed`.
//!
//! **Bounced is a one-frame gate**, `brickgame`'s shape: `Down` on the tick the puck met an
//! edge in Bounce mode and `Up` on the next.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::cpu::{Puck, Pull, Touch, Well};
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Gate, InputDef, NodeDef, OptionDef, OptionKind,
    OutputDef, OutputKind, TickContext,
};

/// Where the hand put the puck on the pad, and the velocity it left it with: the four hidden
/// controls, in the order every write a gesture makes names them — the order `history::joins`
/// needs to see to hold a whole throw in one undo step.
pub const PAD_X: &str = "padX";
pub const PAD_Y: &str = "padY";
pub const VEL_X: &str = "vx";
pub const VEL_Y: &str = "vy";
pub const HAND: [&str; 4] = [PAD_X, PAD_Y, VEL_X, VEL_Y];

/// The key the pad reports a hand holding the puck under, read with `TickContext::pressed`.
pub const PUCK: &str = "puck";

/// The node's four options, by the keys silvia saved them under.
pub const EDGE_X: &str = "edgeX";
pub const EDGE_Y: &str = "edgeY";
pub const CLICK: &str = "clickMode";
pub const PLACE: &str = "wellType";

/// The six knobs a preset writes, in the order it writes them after [`HAND`].
pub const DRAG: &str = "drag";
pub const BOUNCE: &str = "bounce";
pub const TEMPERATURE: &str = "temperature";
pub const SPRING: &str = "spring";
pub const GRAVITY_X: &str = "gravityX";
pub const GRAVITY_Y: &str = "gravityY";

/// How the puck meets one edge of the pad: silvia's four.
const EDGES: &[(&str, &str)] = &[
    ("bounce", "Bounce"),
    ("wrap", "Wrap"),
    ("clamp", "Clamp"),
    ("unbound", "Unbound"),
];

/// A number with a port, as silvia's grid of s-numbers has it.
const fn knob(key: &'static str, label: &'static str, control: Control) -> InputDef {
    InputDef {
        key,
        label,
        ty: UniformNumber,
        control,
    }
}

pub static DEF: NodeDef = NodeDef {
    slug: "xypad",
    category: Category::Control,
    icon: "🕹",
    label: "XY Pad",
    tooltip: "A puck you throw around a square, with physics. Drag to throw it, or to place it \
              in Cursor mode; right-drag to drop a gravity well or a tether, the drag setting \
              its strength or its length; right-click a well to take it away. X and Y are \
              where it is between the range ends, Speed how fast it is going, and Bounced \
              fires on every edge it bounces off.",
    inputs: &[
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
        knob(DRAG, "Drag", Control::num(0.02, 0.0, 0.1, 0.001, "")),
        knob(BOUNCE, "Bounce", Control::num(0.8, 0.0, 1.5, 0.01, "")),
        knob(
            TEMPERATURE,
            "Temp",
            Control::num_log(0.0, 0.0, 5.0, 0.01, ""),
        ),
        knob(SPRING, "Spring", Control::num(0.0, 0.0, 20.0, 0.1, "")),
        knob(GRAVITY_X, "Grav X", Control::num(0.0, -5.0, 5.0, 0.01, "")),
        knob(GRAVITY_Y, "Grav Y", Control::num(0.0, -5.0, 5.0, 0.01, "")),
        knob("minX", "Min X", Control::num(-1.0, -100.0, 100.0, 0.01, "")),
        knob("maxX", "Max X", Control::num(1.0, -100.0, 100.0, 0.01, "")),
        knob("minY", "Min Y", Control::num(-1.0, -100.0, 100.0, 0.01, "")),
        knob("maxY", "Max Y", Control::num(1.0, -100.0, 100.0, 0.01, "")),
    ],
    // Where the hand put the puck, across the pad, and what it was moving at when it let go.
    // Pad X and Pad Y are drawn under the pad as silvia's X and Y readouts are; the velocity
    // is written by a throw and a preset and read by nothing a hand dials, so MIDI does not
    // offer it.
    hidden: &[
        knob(PAD_X, "Pad X", Control::num(0.0, -1.0, 1.0, 0.001, "")),
        knob(PAD_Y, "Pad Y", Control::num(0.0, -1.0, 1.0, 0.001, "")),
        knob(
            VEL_X,
            "Velocity X",
            Control::num(0.0, -100.0, 100.0, 0.01, ""),
        ),
        knob(
            VEL_Y,
            "Velocity Y",
            Control::num(0.0, -100.0, 100.0, 0.01, ""),
        ),
    ],
    unbindable: &[VEL_X, VEL_Y],
    outputs: &[
        OutputDef {
            key: "x",
            label: "X",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "y",
            label: "Y",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "speed",
            label: "Speed",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "bounced",
            label: "Bounced",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        OptionDef {
            key: EDGE_X,
            label: "Edge X",
            default: "bounce",
            choices: EDGES,
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: EDGE_Y,
            label: "Edge Y",
            default: "bounce",
            choices: EDGES,
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: CLICK,
            label: "Click",
            default: "slingshot",
            choices: &[("slingshot", "Slingshot"), ("cursor", "Cursor")],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: PLACE,
            label: "Place Mode",
            default: "gravity",
            choices: &[("gravity", "Gravity Well"), ("tether", "Tether")],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
    ],
    regions: &[crate::nodes::Region::XyPad],
    cpu: Some(CpuDef {
        create: || Box::new(Pad::new()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// One of silvia's nine: where the puck starts, how it is moving, the knobs that shape its
/// flight, the edges and the wells.
pub struct Preset {
    pub name: &'static str,
    pub at: [f32; 2],
    pub velocity: [f32; 2],
    pub drag: f32,
    pub gravity: [f32; 2],
    pub bounce: f32,
    pub edges: [&'static str; 2],
    pub wells: &'static [Well],
}

const fn gravity(x: f32, y: f32, strength: f32) -> Well {
    Well {
        at: [x, y],
        pull: Pull::Gravity(strength),
    }
}

/// silvia's presets, in silvia's order and with silvia's numbers — which are pad units here,
/// the same numbers at the default range.
pub const PRESETS: [Preset; 9] = [
    Preset {
        name: "DVD",
        at: [-0.7, 0.5],
        velocity: [0.8, 0.6],
        drag: 0.0,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["bounce", "bounce"],
        wells: &[],
    },
    Preset {
        name: "Drop",
        at: [0.0, 0.9],
        velocity: [0.4, 0.0],
        drag: 0.003,
        gravity: [0.0, -3.0],
        bounce: 0.85,
        edges: ["bounce", "bounce"],
        wells: &[],
    },
    Preset {
        name: "Orbit",
        at: [0.7, 0.0],
        velocity: [0.0, 1.6],
        drag: 0.0,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["unbound", "unbound"],
        wells: &[gravity(0.0, 0.0, 2.0)],
    },
    Preset {
        name: "Spiral",
        at: [0.8, 0.0],
        velocity: [0.0, 1.4],
        drag: 0.003,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["unbound", "unbound"],
        wells: &[gravity(0.0, 0.0, 2.0)],
    },
    Preset {
        name: "Figure 8",
        at: [0.0, 0.05],
        velocity: [1.35, 0.0],
        drag: 0.001,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["unbound", "unbound"],
        wells: &[gravity(-0.4, 0.0, 2.0), gravity(0.4, 0.0, 2.0)],
    },
    Preset {
        name: "Pendulum",
        at: [0.71, -0.21],
        velocity: [0.0, 0.0],
        drag: 0.001,
        gravity: [0.0, -3.0],
        bounce: 1.0,
        edges: ["unbound", "unbound"],
        wells: &[Well {
            at: [0.0, 0.5],
            pull: Pull::Tether(1.0),
        }],
    },
    Preset {
        name: "Pong",
        at: [-0.8, 0.3],
        velocity: [0.5, 0.8],
        drag: 0.0,
        gravity: [0.0, -1.0],
        bounce: 1.0,
        edges: ["wrap", "bounce"],
        wells: &[],
    },
    Preset {
        name: "Drift",
        at: [0.0, 0.0],
        velocity: [0.35, 0.2],
        drag: 0.0,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["wrap", "wrap"],
        wells: &[],
    },
    Preset {
        name: "3-Body",
        at: [0.0, -0.5],
        velocity: [1.3, 0.3],
        drag: 0.0,
        gravity: [0.0, 0.0],
        bounce: 1.0,
        edges: ["unbound", "unbound"],
        wells: &[
            gravity(0.0, 0.5, 1.2),
            gravity(-0.45, -0.3, 1.2),
            gravity(0.45, -0.3, 1.2),
        ],
    },
];

impl Preset {
    /// The controls a press writes, as one list: the hand's four in [`HAND`]'s order, then the
    /// six knobs — silvia's `_applyPreset`, which zeroes the temperature and the spring.
    pub fn controls(&self) -> [(&'static str, f32); 10] {
        [
            (PAD_X, self.at[0]),
            (PAD_Y, self.at[1]),
            (VEL_X, self.velocity[0]),
            (VEL_Y, self.velocity[1]),
            (DRAG, self.drag),
            (TEMPERATURE, 0.0),
            (SPRING, 0.0),
            (GRAVITY_X, self.gravity[0]),
            (GRAVITY_Y, self.gravity[1]),
            (BOUNCE, self.bounce),
        ]
    }
}

/// How many points of trail the pad keeps, silvia's.
const PATH: usize = 500;
/// The longest step one tick takes, silvia's: a late frame is a tenth of a second of flight,
/// not however long the frame was.
const MAX_DT: f32 = 0.1;
/// A slower velocity than this is none, silvia's, so a puck under drag comes to rest.
const REST: f32 = 0.0001;

/// What one tick integrates with, read once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Physics {
    pub drag: f32,
    pub bounce: f32,
    pub temperature: f32,
    pub spring: f32,
    pub gravity: [f32; 2],
    pub edges: [EdgeMode; 2],
}

/// silvia's four edge behaviors, per axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    Bounce,
    Wrap,
    Clamp,
    Unbound,
}

impl EdgeMode {
    fn of(key: &str) -> Self {
        match key {
            "wrap" => Self::Wrap,
            "clamp" => Self::Clamp,
            "unbound" => Self::Unbound,
            _ => Self::Bounce,
        }
    }
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            drag: 0.02,
            bounce: 0.8,
            temperature: 0.0,
            spring: 0.0,
            gravity: [0.0, 0.0],
            edges: [EdgeMode::Bounce; 2],
        }
    }
}

/// The pad's own state: the puck in flight, the wells, the trail.
#[derive(Debug)]
pub struct Pad {
    at: [f32; 2],
    velocity: [f32; 2],
    wells: Vec<Well>,
    path: std::collections::VecDeque<[f32; 2]>,
    /// The four hand controls as the tick last took them, so a change the document makes —
    /// a gesture, a preset, a knob, an undo — is taken the tick it lands and nothing else is.
    seen: Option<[f32; 4]>,
    /// Whether a hand held the puck last tick, so a slingshot's release can clear the trail.
    held: bool,
    /// Bounced fired `Down` last tick and owes an `Up`.
    owed: bool,
    reset: Gate,
    rng: Rng,
}

impl Pad {
    fn new() -> Self {
        Self {
            at: [0.0, 0.0],
            velocity: [0.0, 0.0],
            wells: Vec::new(),
            path: std::collections::VecDeque::with_capacity(PATH),
            seen: None,
            held: false,
            owed: false,
            reset: Gate::default(),
            rng: Rng::new(),
        }
    }

    /// Take whichever of the hand's four controls the document changed since last tick.
    fn take(&mut self, hand: [f32; 4]) {
        let seen = self.seen.unwrap_or([f32::NAN; 4]);
        for i in 0..4 {
            if hand[i] != seen[i] {
                if i < 2 {
                    self.at[i] = hand[i];
                } else {
                    self.velocity[i - 2] = hand[i];
                }
            }
        }
        self.seen = Some(hand);
    }

    /// Start one of the presets from the top: the puck, its velocity and its wells.
    fn preset(&mut self, index: usize) {
        let Some(p) = PRESETS.get(index) else { return };
        self.at = p.at;
        self.velocity = p.velocity;
        self.wells = p.wells.to_vec();
        self.path.clear();
    }

    /// Drop a well where a hand let go of one: a gravity well as strong as three times the
    /// drag, or silvia's 2 for a click, or a tether as long as the drag, never shorter than
    /// silvia's 0.05.
    fn drop_well(&mut self, at: [f32; 2], reach: Option<f32>, tether: bool) {
        let pull = if tether {
            Pull::Tether(reach.unwrap_or(0.0).max(0.05))
        } else {
            Pull::Gravity(reach.map_or(2.0, |r| r * 3.0))
        };
        self.wells.push(Well { at, pull });
    }

    /// silvia's `_step`, on the pad: gravity, the spring to the middle, the wells, the
    /// temperature, the drag, the move, the tethers and the edges, in that order. Returns
    /// whether the puck bounced off an edge.
    fn step(&mut self, dt: f32, p: &Physics) -> bool {
        let dt = dt.clamp(0.0, MAX_DT);
        let [mut vx, mut vy] = self.velocity;
        vx += p.gravity[0] * dt;
        vy += p.gravity[1] * dt;
        if p.spring > 0.0 {
            vx -= self.at[0] * p.spring * dt;
            vy -= self.at[1] * p.spring * dt;
        }
        for w in &self.wells {
            let Pull::Gravity(strength) = w.pull else {
                continue;
            };
            let (dx, dy) = (w.at[0] - self.at[0], w.at[1] - self.at[1]);
            let dist = dx.hypot(dy);
            if dist > 0.001 {
                let force = strength / (dist + 0.05);
                vx += dx / dist * force * dt;
                vy += dy / dist * force * dt;
            }
        }
        if p.temperature > 0.0 {
            let jitter = p.temperature * dt.sqrt();
            vx += self.gauss() * jitter;
            vy += self.gauss() * jitter;
        }
        let drag = (1.0 - p.drag.clamp(0.0, 1.0)).powf(dt * 60.0);
        vx *= drag;
        vy *= drag;
        if vx.abs() < REST {
            vx = 0.0;
        }
        if vy.abs() < REST {
            vy = 0.0;
        }
        let [mut x, mut y] = self.at;
        x += vx * dt;
        y += vy * dt;

        for w in &self.wells {
            let Pull::Tether(length) = w.pull else {
                continue;
            };
            let (dx, dy) = (x - w.at[0], y - w.at[1]);
            let dist = dx.hypot(dy);
            if dist < 0.0001 {
                x = w.at[0] + length;
                vx = 0.0;
                vy = 0.0;
                continue;
            }
            let (nx, ny) = (dx / dist, dy / dist);
            x = w.at[0] + nx * length;
            y = w.at[1] + ny * length;
            let radial = vx * nx + vy * ny;
            vx -= radial * nx;
            vy -= radial * ny;
        }

        let bounced_x = edge(&mut x, &mut vx, p.edges[0], p.bounce);
        let bounced_y = edge(&mut y, &mut vy, p.edges[1], p.bounce);
        self.at = [x, y];
        self.velocity = [vx, vy];
        if self.path.len() == PATH {
            self.path.pop_front();
        }
        self.path.push_back(self.at);
        bounced_x || bounced_y
    }

    /// A standard normal sample, Box-Muller, silvia's `_gaussRandom`.
    fn gauss(&mut self) -> f32 {
        let u = self.rng.next_f32().max(f32::MIN_POSITIVE);
        let v = self.rng.next_f32();
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// One axis meeting the pad's edges at -1 and 1, silvia's `_applyEdgeAxis`. Returns whether
/// it bounced.
fn edge(at: &mut f32, velocity: &mut f32, mode: EdgeMode, bounce: f32) -> bool {
    match mode {
        EdgeMode::Unbound => false,
        EdgeMode::Wrap => {
            if !(-1.0..=1.0).contains(at) {
                *at = (*at + 1.0).rem_euclid(2.0) - 1.0;
            }
            false
        }
        EdgeMode::Clamp => {
            if *at <= -1.0 || *at >= 1.0 {
                *at = at.clamp(-1.0, 1.0);
                *velocity = 0.0;
            }
            false
        }
        EdgeMode::Bounce => {
            if *at <= -1.0 {
                *at = -1.0;
                *velocity = velocity.abs() * bounce;
                true
            } else if *at >= 1.0 {
                *at = 1.0;
                *velocity = -velocity.abs() * bounce;
                true
            } else {
                false
            }
        }
    }
}

/// A point on the pad, -1 to 1, onto the range between two ends: from the middle of the
/// range, so at silvia's -1 to 1 a pad unit is exactly an output unit.
pub fn map(at: f64, min: f64, max: f64) -> f64 {
    f64::midpoint(min, max) + at * (max - min) * 0.5
}

impl CpuNode for Pad {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        if std::mem::take(&mut self.owed) {
            ctx.fire(id, "bounced", Edge::Up);
        }

        if ctx.downs(id, "reset", &mut self.reset) > 0 {
            self.at = [0.0, 0.0];
            self.velocity = [0.0, 0.0];
            self.path.clear();
        }
        let tether = ctx.option(id, PLACE) == "tether";
        for touch in ctx.touches(id).to_vec() {
            match touch {
                Touch::Well { at, reach } => self.drop_well(at, reach, tether),
                Touch::Unwell(i) => {
                    if i < self.wells.len() {
                        self.wells.remove(i);
                    }
                }
                Touch::Preset(i) => self.preset(i),
            }
        }

        // The hand's four hidden controls, which hold `f32`s.
        let hand = HAND.map(|key| ctx.input(id, key) as f32);
        let held = ctx.pressed(id, PUCK);
        let mut bounced = false;
        if held {
            // The puck is under the hand: where it was put, moving at what the gesture says,
            // and going nowhere until it is let go.
            self.at = [hand[0], hand[1]];
            self.velocity = [hand[2], hand[3]];
            self.seen = Some(hand);
        } else {
            // A slingshot let go starts a new flight, and the trail with it.
            if self.held && ctx.option(id, CLICK) == "slingshot" {
                self.path.clear();
            }
            self.take(hand);
            let physics = Physics {
                drag: ctx.input(id, DRAG) as f32,
                bounce: ctx.input(id, BOUNCE) as f32,
                temperature: ctx.input(id, TEMPERATURE) as f32,
                spring: ctx.input(id, SPRING) as f32,
                gravity: [
                    ctx.input(id, GRAVITY_X) as f32,
                    ctx.input(id, GRAVITY_Y) as f32,
                ],
                edges: [
                    EdgeMode::of(ctx.option(id, EDGE_X)),
                    EdgeMode::of(ctx.option(id, EDGE_Y)),
                ],
            };
            bounced = self.step(ctx.dt, &physics);
        }
        self.held = held;

        let (min_x, max_x) = (ctx.input(id, "minX"), ctx.input(id, "maxX"));
        let (min_y, max_y) = (ctx.input(id, "minY"), ctx.input(id, "maxY"));
        ctx.publish(id, "x", map(f64::from(self.at[0]), min_x, max_x));
        ctx.publish(id, "y", map(f64::from(self.at[1]), min_y, max_y));
        // In the outputs' own units, as silvia's is: a pad unit is half the range.
        let speed = (f64::from(self.velocity[0]) * (max_x - min_x) * 0.5)
            .hypot(f64::from(self.velocity[1]) * (max_y - min_y) * 0.5);
        ctx.publish(id, "speed", speed);
        if bounced {
            ctx.fire(id, "bounced", Edge::Down);
            self.owed = true;
        }
    }

    fn puck(&self) -> Option<Puck> {
        Some(Puck {
            at: self.at,
            velocity: self.velocity,
            wells: self.wells.clone(),
            path: self.path.iter().copied().collect(),
        })
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "xypad at ({:.3}, {:.3}) moving ({:.3}, {:.3}), {} wells",
            self.at[0],
            self.at[1],
            self.velocity[0],
            self.velocity[1],
            self.wells.len(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still() -> Physics {
        Physics {
            drag: 0.0,
            ..Physics::default()
        }
    }

    /// A pad at rest with nothing pulling on it stays where the hand put it.
    #[test]
    fn a_puck_nothing_pulls_on_stays_put() {
        let mut pad = Pad::new();
        pad.take([0.25, -0.5, 0.0, 0.0]);
        for _ in 0..120 {
            assert!(!pad.step(1.0 / 60.0, &Physics::default()));
        }
        assert_eq!(pad.at, [0.25, -0.5]);
    }

    /// A thrown puck carries on, and meets the wall: in Bounce it turns around there, loses
    /// what Bounce takes and says so; in Clamp it stops dead; in Wrap it comes in the other
    /// side; unbound it keeps going.
    #[test]
    fn each_edge_does_what_silvia_says() {
        let fly = |edges: [EdgeMode; 2]| {
            let mut pad = Pad::new();
            pad.take([0.9, 0.0, 3.0, 0.0]);
            let p = Physics {
                edges,
                bounce: 0.5,
                ..still()
            };
            let bounced = (0..6).any(|_| pad.step(1.0 / 60.0, &p));
            (pad, bounced)
        };
        let (pad, bounced) = fly([EdgeMode::Bounce; 2]);
        assert!(bounced, "the wall said nothing");
        assert!(pad.velocity[0] < 0.0 && pad.velocity[0] > -3.0, "{pad:?}");
        assert!(pad.at[0] <= 1.0);

        let (pad, bounced) = fly([EdgeMode::Clamp; 2]);
        assert!(!bounced);
        assert_eq!((pad.at[0], pad.velocity[0]), (1.0, 0.0));

        let (pad, bounced) = fly([EdgeMode::Wrap; 2]);
        assert!(!bounced);
        assert!(pad.at[0] < 0.0, "came back in the other side: {pad:?}");

        let (pad, _) = fly([EdgeMode::Unbound; 2]);
        assert!(pad.at[0] > 1.0, "{pad:?}");
    }

    /// Gravity pulls, and drag slows without reversing.
    #[test]
    fn gravity_pulls_and_drag_slows() {
        let mut pad = Pad::new();
        pad.take([0.0, 0.0, 0.0, 0.0]);
        let p = Physics {
            gravity: [0.0, -3.0],
            edges: [EdgeMode::Unbound; 2],
            ..still()
        };
        pad.step(0.1, &p);
        assert!(pad.velocity[1] < 0.0 && pad.at[1] < 0.0, "{pad:?}");

        let mut pad = Pad::new();
        pad.take([0.0, 0.0, 1.0, 0.0]);
        pad.step(
            1.0 / 60.0,
            &Physics {
                drag: 0.1,
                ..Physics::default()
            },
        );
        assert!((pad.velocity[0] - 0.9).abs() < 1e-5, "{pad:?}");
    }

    /// A tether holds the puck at its length, however it is moving.
    #[test]
    fn a_tether_holds_its_length() {
        let mut pad = Pad::new();
        pad.preset(5); // Pendulum
        let p = Physics {
            gravity: [0.0, -3.0],
            drag: 0.001,
            edges: [EdgeMode::Unbound; 2],
            ..Physics::default()
        };
        for _ in 0..200 {
            pad.step(1.0 / 60.0, &p);
            let (dx, dy) = (pad.at[0], pad.at[1] - 0.5);
            assert!((dx.hypot(dy) - 1.0).abs() < 1e-4, "{:?}", pad.at);
        }
    }

    /// A gravity well keeps an orbit bounded.
    #[test]
    fn an_orbit_stays_near_its_well() {
        let mut pad = Pad::new();
        pad.preset(2); // Orbit
        let p = Physics {
            drag: 0.0,
            edges: [EdgeMode::Unbound; 2],
            ..Physics::default()
        };
        for _ in 0..600 {
            pad.step(1.0 / 60.0, &p);
            assert!(pad.at[0].hypot(pad.at[1]) < 2.0, "flew off: {:?}", pad.at);
        }
    }

    /// A change the document makes is taken — only the part it changed — and the flight in
    /// between is left alone.
    #[test]
    fn only_what_the_document_changed_is_taken() {
        let mut pad = Pad::new();
        pad.take([0.0, 0.0, 1.0, 0.0]);
        pad.step(0.1, &still());
        let flying = pad.at;
        pad.take([0.0, 0.0, 1.0, 0.0]);
        assert_eq!(pad.at, flying, "nothing changed, so the flight goes on");
        pad.take([0.0, 0.5, 1.0, 0.0]);
        assert_eq!(pad.at, [flying[0], 0.5], "Pad Y moved, and only Pad Y");
    }

    /// A click drops silvia's default well; a drag sets its strength or its length.
    #[test]
    fn a_dropped_well_takes_its_strength_from_the_drag() {
        let mut pad = Pad::new();
        pad.drop_well([0.1, 0.2], None, false);
        pad.drop_well([0.1, 0.2], Some(0.5), false);
        pad.drop_well([0.1, 0.2], None, true);
        pad.drop_well([0.1, 0.2], Some(0.5), true);
        let pulls: Vec<Pull> = pad.wells.iter().map(|w| w.pull).collect();
        assert_eq!(
            pulls,
            [
                Pull::Gravity(2.0),
                Pull::Gravity(1.5),
                Pull::Tether(0.05),
                Pull::Tether(0.5)
            ]
        );
    }

    /// The pad maps onto the range ends, and at silvia's default range the pad's units are
    /// the outputs'.
    #[test]
    fn the_pad_maps_onto_the_range_ends() {
        assert_eq!(map(-1.0, -1.0, 1.0), -1.0);
        assert_eq!(map(0.3, -1.0, 1.0), 0.3);
        assert_eq!(map(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(map(1.0, 0.0, 10.0), 10.0);
        assert_eq!(map(0.0, 0.0, 10.0), 5.0);
    }

    /// Every preset starts inside the pad and inside the controls' own ranges, so the press
    /// that writes it is not clamped into another motion.
    #[test]
    fn every_preset_fits_the_controls_it_writes() {
        for p in &PRESETS {
            for (key, v) in p.controls() {
                let range = crate::nodes::declared_range(&DEF, key).expect(key);
                assert!(
                    (range.min..=range.max).contains(&v),
                    "{}: {key} {v} is outside {range:?}",
                    p.name
                );
            }
            for e in p.edges {
                assert!(EDGES.iter().any(|(k, _)| *k == e), "{}: {e}", p.name);
            }
        }
    }
}
