// SPDX-License-Identifier: AGPL-3.0-or-later

//! An envelope over a gate. The node that proves down-and-up was the right call: it is held
//! at its sustain for as long as the gate is down, which a pulse could not express without
//! moving the duration into this node and getting it wrong the first time two sources
//! overlap.
//!
//! It is also the node that proves an event has to carry a time. A frame is 16 ms and an
//! attack is often shorter, so this integrates **in segments between the events inside the
//! frame** rather than in one lump at the end of it: a gate that opened three quarters of
//! the way through the frame gets a quarter of a frame of attack, not a whole one. Anything
//! less and every envelope in the graph is quantized to the display.
//!
//! Attack, decay, sustain, release. Its output is a `UniformNumber`, so it reaches a uniform
//! the way any number does, and the whole point of the event half is that this is
//! where it lands.
//!
//! Each moving stage is a *curve over the stage's own time* rather than a constant rate,
//! because silvia's three shapes are what give an envelope its character and two of the
//! three it ships with are not straight. The formulas and the defaults are silvia's
//! `_applyCurve` and its option defaults to the decimal, and so is the gate: a fresh gate
//! restarts the attack from zero rather than carrying on from what is left of a release.
//! See docs/decisions.md.

use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::cpu::TraceRing;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "adsr",
    category: Category::Control,
    icon: "📈",
    label: "ADSR",
    tooltip: "Rises while the gate is held and falls when it is let go. A fresh gate restarts the attack from zero, silvia's own retrigger.",
    // The envelope is the thing being edited; see docs/decisions.md#a-trace-is-a-picture-of-
    // the-shape-a-node-is-editing. 300 is `canvas::SCOPE_NODE_WIDTH`, which `nodes/` cannot
    // name — it is a graphical dependency this crate does not take.
    regions: &[crate::nodes::Region::Trace, crate::nodes::Region::Caption],
    inputs: &[
        InputDef {
            key: "gate",
            label: "Gate",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "attack",
            label: "Attack",
            ty: UniformNumber,
            control: Control::num(0.01, 0.0, 10.0, 0.001, "s"),
        },
        InputDef {
            key: "decay",
            label: "Decay",
            ty: UniformNumber,
            control: Control::num(0.1, 0.0, 10.0, 0.001, "s"),
        },
        InputDef {
            key: "sustain",
            label: "Sustain",
            ty: UniformNumber,
            control: Control::num(0.7, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "release",
            label: "Release",
            ty: UniformNumber,
            control: Control::num(0.2, 0.0, 10.0, 0.001, "s"),
        },
    ],
    outputs: &[OutputDef {
        key: "value",
        label: "Value",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "attackCurve",
            label: "Attack Curve",
            default: "linear",
            choices: &CURVES,
            // Read by `tick`. The output is a uniform number, so this node emits no WGSL
            // for an option to change.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "decayCurve",
            label: "Decay Curve",
            default: "exponential",
            choices: &CURVES,
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "releaseCurve",
            label: "Release Curve",
            default: "exponential",
            choices: &CURVES,
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_TRACE,
    ],
    cpu: Some(CpuDef {
        create: || Box::new(Adsr::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// silvia's three curve shapes, the choices on all three menus.
const CURVES: [(&str, &str); 3] = [
    ("linear", "Linear"),
    ("exponential", "Exponential"),
    ("logarithmic", "Logarithmic"),
];

/// How a stage crosses its own span: silvia's `_applyCurve`, to the constant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Curve {
    Linear,
    /// Fast at the start and trailing away: what gives a fall its taper.
    Exponential,
    /// Slow at the start and quick at the end.
    Logarithmic,
}

impl Curve {
    fn of(name: &str) -> Self {
        match name {
            "exponential" => Self::Exponential,
            "logarithmic" => Self::Logarithmic,
            // "linear", and anything a file carries that this build does not have.
            _ => Self::Linear,
        }
    }

    /// How far across its span a stage is, `t` of the way through its time.
    ///
    /// silvia's own formulas. `Exponential` reaches 0.9933 rather than 1 at a `t` of 1,
    /// which is silvia's arithmetic kept rather than corrected: the next stage starts from
    /// its own end of the span, and normalizing would make every exponential curve a
    /// different shape from the one silvia draws.
    fn at(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Exponential => 1.0 - (-5.0 * t).exp(),
            Self::Logarithmic => (1.0 + 9.0 * t).ln() / 10.0f32.ln(),
        }
    }
}

/// Which segment the envelope is in.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

impl Stage {
    /// For the Status box line.
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Attack => "attack",
            Self::Decay => "decay",
            Self::Sustain => "sustain",
            Self::Release => "release",
        }
    }

    /// For the caption under the node's trace, which is silvia's own `Phase` readout.
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Attack => "Attack",
            Self::Decay => "Decay",
            Self::Sustain => "Sustain",
            Self::Release => "Release",
        }
    }
}

/// How many seconds the trace holds: long enough to see a whole envelope without the plot
/// outrunning what a hand did a moment ago. Seconds rather than samples, since the band
/// draws by time.
const TRACE_SPAN: f32 = 3.0;

struct Adsr {
    stage: Stage,
    /// Seconds into the current stage. A stage is a curve over its own time, so this is what
    /// the level is a function of.
    elapsed: f32,
    value: f32,
    /// The level the release started from — silvia's `lastValue`, so a gate let go in the
    /// middle of an attack falls from where it had reached.
    from: f32,
    /// Whether the gate is down, over every source at once: the hand and the cables summed.
    gated: bool,
    /// What the cables are holding. An edge lives one frame; a gate lives until it is let go.
    upstream: bool,
    hand: Gate,
    /// The last `TRACE_SPAN` seconds of values, dated, for the trace on the node's body.
    history: TraceRing,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            stage: Stage::default(),
            elapsed: 0.0,
            value: 0.0,
            from: 0.0,
            gated: false,
            upstream: false,
            hand: Gate::default(),
            history: TraceRing::new(TRACE_SPAN),
        }
    }
}

/// The four times, the level and the three curves, read once a frame. A control cannot move
/// inside a frame, so neither can these.
#[derive(Clone, Copy)]
struct Shape {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    attack_curve: Curve,
    decay_curve: Curve,
    release_curve: Curve,
}

impl Adsr {
    /// The level a stage is at, `t` of the way through its own time.
    fn level(&self, t: f32, shape: Shape) -> f32 {
        match self.stage {
            Stage::Idle => 0.0,
            Stage::Attack => shape.attack_curve.at(t),
            Stage::Decay => 1.0 - shape.decay_curve.at(t) * (1.0 - shape.sustain),
            Stage::Sustain => shape.sustain,
            Stage::Release => self.from * (1.0 - shape.release_curve.at(t)),
        }
    }

    /// Run the envelope forward by `dt` seconds, through as many stages as that crosses.
    ///
    /// A stage that finishes part way through a segment hands the rest of it to the next one,
    /// so an attack and a decay shorter than a frame both happen inside it instead of costing
    /// a frame each, and a stage whose time is zero arrives at once rather than dividing by
    /// it.
    fn advance(&mut self, mut dt: f32, shape: Shape) {
        // Attack, decay and release each end at most once and the two resting stages return,
        // so the loop cannot spin.
        for _ in 0..4 {
            let duration = match self.stage {
                Stage::Idle => {
                    self.value = 0.0;
                    return;
                }
                Stage::Sustain => {
                    self.value = shape.sustain;
                    return;
                }
                Stage::Attack => shape.attack,
                Stage::Decay => shape.decay,
                Stage::Release => shape.release,
            };
            self.elapsed += dt.max(0.0);
            if duration > 0.0 && self.elapsed < duration {
                self.value = self.level(self.elapsed / duration, shape);
                return;
            }
            // The stage is over: what is left of the segment runs in the next one.
            dt = self.elapsed - duration.max(0.0);
            self.elapsed = 0.0;
            self.stage = match self.stage {
                Stage::Attack => Stage::Decay,
                Stage::Decay => Stage::Sustain,
                // Attack, decay and release are the only three that reach here.
                _ => Stage::Idle,
            };
        }
    }

    /// What a change of gate does, wherever in the frame it happened.
    ///
    /// silvia's `_gateOn` and `_gateOff`: a fresh gate starts the attack from zero, and
    /// letting go releases from wherever the envelope had reached. Only a *change* of the
    /// summed level does anything, so a hand let go while a cable still holds the gate
    /// leaves the envelope alone rather than retriggering it.
    fn set_gate(&mut self, held: bool) {
        if held == self.gated {
            return;
        }
        self.gated = held;
        self.elapsed = 0.0;
        if held {
            self.stage = Stage::Attack;
            self.value = 0.0;
        } else if matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Sustain) {
            self.from = self.value;
            self.stage = Stage::Release;
        }
    }
}

impl CpuNode for Adsr {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!("{} {:.3}", self.stage.label(), self.value))
    }

    fn trace(&self) -> Option<&TraceRing> {
        Some(&self.history)
    }

    /// silvia's two readouts under its envelope graph: whether the gate is on, and which
    /// stage the envelope is in.
    fn caption(&self) -> Vec<(&'static str, String)> {
        vec![
            ("gate", if self.gated { "ON" } else { "OFF" }.to_string()),
            ("stage", self.stage.name().to_string()),
        ]
    }

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        let shape = Shape {
            attack: ctx.input(id, "attack"),
            decay: ctx.input(id, "decay"),
            sustain: ctx.input(id, "sustain").clamp(0.0, 1.0),
            release: ctx.input(id, "release"),
            attack_curve: Curve::of(ctx.option(id, "attackCurve")),
            decay_curve: Curve::of(ctx.option(id, "decayCurve")),
            release_curve: Curve::of(ctx.option(id, "releaseCurve")),
        };

        // A hand knows only which frame it moved in, so it acts at the top of one.
        if self.hand.set(ctx.pressed(id, "gate")).is_some() {
            self.set_gate(self.hand.is_down() || self.upstream);
        }

        // Then the frame is walked event by event, integrating the gap before each. This is
        // the whole reason an event carries a time.
        let mut t = 0.0;
        for event in ctx.edges(id, "gate") {
            let at = event.at.clamp(t, ctx.dt);
            self.advance(at - t, shape);
            self.upstream = event.is_down();
            self.set_gate(self.upstream || self.hand.is_down());
            t = at;
        }
        self.advance(ctx.dt - t, shape);

        ctx.publish(id, "value", self.value);
        self.history.push(ctx.elapsed, self.value);
    }
}
