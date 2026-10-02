// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gears: the one place a rate is set, changed or divided.
//!
//! A node that moves with time has no speed of its own. It reads ambient time, the playhead,
//! at a rate its kind declares, unless a gear is cabled into its Time — and a gear is where
//! the show's clock is turned into another one. Gears hold the only state in the time model:
//! everything they drive is a function of what they publish. See `proposals/time.md`, *Gears*,
//! and `docs/cpu.md#gears`.
//!
//! **The Master Gear** is the show's own clock at a length in seconds. It integrates the
//! playhead's advance over its length in `f64`, so a length turned bends from where it is and
//! never jumps, and it is born at `playhead ÷ length` — two master gears of one length agree,
//! and at the playhead's zero every one of them is at the start of its cycle.
//!
//! **The Ratio Gear** is a clock in and a clock out at a ratio: `ratio × ΔClock In`, a count
//! read whole in `f64` (`TickContext::count`), anything else unwrapped where its output
//! declares its wrap (`OutputDef::wraps_at`), or the playhead's seconds with nothing cabled. **A ratio change lands on the input's next whole cycle**:
//! until then the old ratio runs and the display shows the new one pending, so the output's
//! downbeat stays on the input's, and a chain of whole ratios still closes whatever phase the
//! change landed at. It is born at `ratio ×` the input's reading, the count as its source
//! publishes it, so a gear born again is where playing would have put it and its downbeat is
//! the input's. A cabled clock its source calls a jump ([`TickContext::jump`], which a gear
//! says of its readings on a Reset), or one sent back more than a cycle in a frame, is a jump
//! to it, and fires at most one downbeat, its own where this frame's motion carried it past
//! one, so a hand's Reset above it is a beat however early in a cycle it comes.
//!
//! Both publish the same four: Cycles, a count published whole (`TickContext::publish_count`) —
//! to a Time in `f64` on the CPU and as a whole part and a fraction in a shader, and to
//! anything else as one `f32` wrapped at [`phasor::WRAP`] centered on zero;
//! Phase, the fraction alone; Ping-pong, a triangle over two cycles; and Trigger, an event on
//! each whole cycle placed where inside the frame it fell. **Hold** is a toggle that freezes
//! the gear where it stands; **Reset** puts it at the start of a cycle, and is a beat. A seek
//! — the time readout's reset among them — and a render's start are a jump: every gear is
//! born again where the playhead puts it, and fires nothing on the way. A gear the jump puts
//! on a whole cycle is on that cycle's beat and fires it, so the readout's reset and a
//! render's first frame are a downbeat.
//!
//! How each is drawn is its **Display** option, a still rosette or two meshing gears, both
//! turning at the real rate: `widgets::gear`.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::phasor::{self, Step};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Gate, InputDef, NodeDef, OptionDef, OptionKind,
    OutputDef, OutputKind, Region, TickContext,
};

/// The four outputs both gears publish, in their order on the node. The Phase's key is
/// `wrapped`, which `phase`'s was.
const OUTPUTS: &[OutputDef] = &[
    OutputDef {
        key: "cycles",
        label: "Cycles",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "wrapped",
        label: "Phase",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        wraps_at: 1.0,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "pingpong",
        label: "Ping-pong",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "trigger",
        label: "Trigger",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    },
];

/// The Display option both gears carry: a still rosette, the default, or two meshing gears.
/// It changes the picture and nothing the gear publishes.
pub const DISPLAY: OptionDef = OptionDef {
    key: "display",
    label: "Display",
    default: "rosette",
    choices: &[("rosette", "Rosette"), ("gears", "Gears")],
    kind: OptionKind::Runtime,
    ..OptionDef::EMPTY
};

pub static MASTER: NodeDef = NodeDef {
    slug: "mastergear",
    category: Category::Gear,
    icon: "⚙",
    label: "Master Gear",
    tooltip: "The show's clock at a length in seconds. Cable its Cycles into a node's Time, \
              or into Ratio Gears, and everything on it closes together. Hold freezes it and \
              Reset starts a cycle.",
    inputs: &[
        InputDef {
            key: "length",
            label: "Length",
            ty: UniformNumber,
            control: Control::num(2.0, 0.01, 256.0, 0.01, "s"),
        },
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "hold",
            label: "Hold",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "gate",
            label: "Gate",
            ty: UniformNumber,
            control: Control::num(0.5, 0.01, 1.0, 0.01, ""),
        },
    ],
    outputs: OUTPUTS,
    options: &[DISPLAY],
    regions: &[Region::Gear],
    cpu: Some(CpuDef {
        create: || Box::new(MasterGear::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

pub static RATIO: NodeDef = NodeDef {
    slug: "ratiogear",
    category: Category::Gear,
    icon: "⚙",
    label: "Ratio Gear",
    tooltip: "A clock in and a clock out at a ratio: ×3, ÷4, 3/2, or a sign to run it \
              backwards. A change lands on the input's next whole cycle, so the downbeat stays \
              on the input's. With nothing in Clock In it counts ambient seconds.",
    inputs: &[
        InputDef {
            key: "clock",
            label: "Clock In",
            ty: UniformNumber,
            // No knob: unplugged, the clock is ambient time's seconds.
            control: Control::None,
        },
        InputDef {
            key: "ratio",
            label: "Ratio",
            ty: UniformNumber,
            control: Control::num(1.0, -64.0, 64.0, 0.001, ""),
        },
        InputDef {
            key: "reset",
            label: "Reset",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "hold",
            label: "Hold",
            ty: Action,
            control: Control::Press,
        },
    ],
    outputs: OUTPUTS,
    options: &[DISPLAY],
    regions: &[Region::Gear],
    cpu: Some(CpuDef {
        create: || Box::new(RatioGear::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// Whether `key` on `def` is a Ratio Gear's ratio: the control drawn and typed on the
/// [`ladder`], which a MIDI knob sweeps too.
pub fn is_ratio(def: &NodeDef, key: &str) -> bool {
    def.slug == RATIO.slug && key == "ratio"
}

/// How many seconds one cycle of a Master Gear is: its Length, kept off zero.
pub fn seconds_a_cycle(length: f64) -> f64 {
    length.max(1e-3)
}

/// What a gear is doing, for the region that draws it: [`CpuNode::gear`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    Master {
        /// Cycles since it was born, unbounded.
        cycles: f64,
        /// How long one is.
        seconds: f64,
        held: bool,
    },
    Ratio {
        /// Input cycles, unwrapped.
        input: f64,
        /// Output cycles, unbounded.
        output: f64,
        /// The ratio running.
        ratio: f64,
        /// A ratio waiting for the input's next whole cycle.
        pending: Option<f64>,
        held: bool,
    },
}

/// The most edges one frame may fire, however fast the gear turns: a `tick` never allocates
/// without a bound, and past this nothing downstream can act on them anyway.
const MAX_EDGES_PER_FRAME: usize = 64;

/// What happened inside a frame, with the fraction of its advance it happened at.
#[derive(Debug, Clone, Copy)]
enum Moment {
    /// Hold was pressed: a toggle.
    Hold,
    /// Back to the start of a cycle.
    Reset,
}

/// Every Hold and Reset a frame holds, in the order they happened: the hand at the top of
/// the frame and each cable's own downs where they fell.
fn moments(
    id: NodeId,
    ctx: &TickContext<'_>,
    hold: &mut Gate,
    reset: &mut Gate,
) -> Vec<(f64, Moment)> {
    let mut out = Vec::new();
    if hold.set(ctx.pressed(id, "hold")) == Some(Edge::Down) {
        out.push((0.0, Moment::Hold));
    }
    if reset.set(ctx.pressed(id, "reset")) == Some(Edge::Down) {
        out.push((0.0, Moment::Reset));
    }
    for (key, moment) in [("hold", Moment::Hold), ("reset", Moment::Reset)] {
        for event in ctx.edges(id, key) {
            if event.is_down() {
                out.push((ctx.fraction(event.at), moment));
            }
        }
    }
    // Stable, so two things at one moment stay in the order they were gathered.
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The whole cycles and the gate's ends a gear's output passed in one stretch of a frame,
/// from `from` to `to` cycles over the frame's fractions `f0..f1`: `(fraction, down)`.
fn edges_between(from: f64, to: f64, f0: f64, f1: f64, gate: f64, out: &mut Vec<(f64, bool)>) {
    let step = Step {
        from,
        to,
        jumped: false,
    };
    let at = |p: f64| f0 + (f1 - f0) * ((p - from) / (to - from)).clamp(0.0, 1.0);
    for (offset, down) in [(0.0, true), (gate, false)] {
        for (n, _) in step.crossings(offset) {
            if out.len() >= MAX_EDGES_PER_FRAME {
                return;
            }
            out.push((at(n + offset), down));
        }
    }
}

/// Fire a frame's gathered edges on Trigger in the order they fell, through the gear's own
/// gate so a level never repeats.
fn fire(id: NodeId, ctx: &mut TickContext<'_>, trigger: &mut Gate, mut edges: Vec<(f64, bool)>) {
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (at, down) in edges {
        // A beat retriggers: the gate closes first, so a receiver sees a fresh down.
        if down && let Some(edge) = trigger.set(false) {
            ctx.fire_at(id, "trigger", edge, ctx.moment(at));
        }
        if let Some(edge) = trigger.set(down) {
            ctx.fire_at(id, "trigger", edge, ctx.moment(at));
        }
    }
}

/// Whether a count stands on a whole cycle, to within [`phasor::REACH`]: a gear born there is
/// on that cycle's beat.
fn on_a_whole_cycle(cycles: f64) -> bool {
    (cycles - cycles.round()).abs() <= phasor::REACH
}

/// Say that a gear's three readings were put where they are this frame rather than moved
/// there, so a gear counting one is born again with them: [`TickContext::jump`].
fn jumped(id: NodeId, ctx: &mut TickContext<'_>) {
    for port in ["cycles", "wrapped", "pingpong"] {
        ctx.jump(id, port);
    }
}

/// Publish a gear's three readings of `cycles`: Cycles a count, whole, which a Time reads to
/// `f64`'s precision, and as one `f32` wrapped centered on zero for anything else.
fn publish(id: NodeId, ctx: &mut TickContext<'_>, cycles: f64) {
    ctx.publish_count(id, "cycles", cycles, phasor::wrap_count(cycles) as f32);
    let phase = phasor::fraction(cycles, 1.0) as f32;
    ctx.publish(id, "wrapped", if phase < 1.0 { phase } else { 0.0 });
    // A triangle over two cycles: up across the first, back across the second.
    ctx.publish(
        id,
        "pingpong",
        (1.0 - (phasor::fraction(cycles, 2.0) - 1.0).abs()) as f32,
    );
}

// ------------------------------------------------------------------------------ master

#[derive(Default)]
struct MasterGear {
    /// Cycles since it was born, unbounded.
    cycles: f64,
    born: bool,
    held: bool,
    /// How long a cycle was on the last tick, for the display.
    seconds: f64,
    hold: Gate,
    reset: Gate,
    trigger: Gate,
}

impl CpuNode for MasterGear {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{:.3} cycles{}",
            self.cycles,
            if self.held { " held" } else { "" }
        ))
    }

    fn gear(&self) -> Option<Reading> {
        Some(Reading::Master {
            cycles: self.cycles,
            seconds: self.seconds,
            held: self.held,
        })
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.seconds = seconds_a_cycle(f64::from(ctx.input(id, "length")));
        let rate = 1.0 / self.seconds;
        let gate = f64::from(ctx.input(id, "gate").clamp(0.01, 1.0));
        let moments = moments(id, ctx, &mut self.hold, &mut self.reset);
        let time = ctx.time;

        // A jump — a seek, a render's start, a tab reopened — is a birth where the playhead
        // puts it, and fires nothing on the way. A seek that lands on a whole cycle lands on
        // that cycle's beat, and the frame's motion since it runs on from there.
        if time.jumped || !self.born {
            let first = !self.born;
            self.born = true;
            self.cycles = rate * time.playhead;
            for (_, moment) in moments {
                if let Moment::Hold = moment {
                    self.held = !self.held;
                }
            }
            let landed = time.landed.or(first.then_some(time.playhead));
            if let Some(landed) = landed
                && !self.held
            {
                let from = rate * landed;
                let mut edges = Vec::new();
                if on_a_whole_cycle(from) {
                    edges.push((0.0, true));
                }
                if self.cycles != from {
                    edges_between(from, self.cycles, 0.0, 1.0, gate, &mut edges);
                }
                fire(id, ctx, &mut self.trigger, edges);
            }
            publish(id, ctx, self.cycles);
            return;
        }

        // The frame walked from moment to moment, the advance before each integrated at the
        // rate, and nothing added while held.
        let mut edges = Vec::new();
        let mut done = 0.0;
        for at in moments.into_iter().map(Some).chain([None]) {
            let to = at.map_or(1.0, |(f, _)| f.clamp(done, 1.0));
            if !self.held {
                let from = self.cycles;
                self.cycles += rate * time.advance * (to - done);
                if self.cycles != from {
                    edges_between(from, self.cycles, done, to, gate, &mut edges);
                }
            }
            done = to;
            match at.map(|(_, m)| m) {
                Some(Moment::Hold) => {
                    self.held = !self.held;
                    // A gear that stops closes the gate it was holding open, so nothing
                    // downstream is left held by a clock that is not moving.
                    if self.held {
                        edges.push((to, false));
                    }
                }
                Some(Moment::Reset) => {
                    self.cycles = 0.0;
                    // A reset is the start of a cycle, and a beat.
                    edges.push((to, true));
                    jumped(id, ctx);
                }
                None => {}
            }
        }
        fire(id, ctx, &mut self.trigger, edges);
        publish(id, ctx, self.cycles);
    }
}

// ------------------------------------------------------------------------------- ratio

#[derive(Default)]
struct RatioGear {
    /// Output cycles, unbounded.
    output: f64,
    /// Input cycles, unwrapped: the playhead's seconds with Clock In unplugged, the cabled
    /// clock's readings summed across its wraps otherwise.
    input: f64,
    /// What the cabled clock read on the last tick, as it is published.
    raw: Option<f64>,
    /// The output cabled into Clock In on the last tick.
    source: Option<crate::graph::PortRef>,
    /// How fast the cabled clock was moving on the last tick it moved, in its cycles a second
    /// of the transport's advance, and how far off that can be from the `f32` readings it was
    /// measured on: what a frame of it is after it is thrown back.
    pace: Option<(f64, f64)>,
    born: bool,
    ratio: f64,
    /// A ratio waiting for the input's next whole cycle.
    pending: Option<f64>,
    held: bool,
    hold: Gate,
    reset: Gate,
    trigger: Gate,
}

/// The Ratio Gear's Trigger gate: half a cycle.
const RATIO_GATE: f64 = 0.5;

impl RatioGear {
    /// Run the input from `from` to `to`, over the frame's fractions `f0..f1`: the output
    /// follows at the ratio, a pending ratio lands where the input crosses a whole cycle, and
    /// the output's own whole cycles are gathered as edges.
    fn run(&mut self, from: f64, to: f64, f0: f64, f1: f64, edges: &mut Vec<(f64, bool)>) {
        let at = |p: f64| {
            if to == from {
                f1
            } else {
                f0 + (f1 - f0) * ((p - from) / (to - from)).clamp(0.0, 1.0)
            }
        };
        let mut here = from;
        loop {
            let landing = self.pending.and_then(|p| {
                let step = Step {
                    from: here,
                    to,
                    jumped: false,
                };
                step.crossings(0.0).next().map(|(n, _)| (n, p))
            });
            let (until, next) = match landing {
                Some((n, p)) => (n, Some(p)),
                None => (to, None),
            };
            if !self.held && until != here {
                let before = self.output;
                self.output += self.ratio * (until - here);
                if self.output != before {
                    edges_between(before, self.output, at(here), at(until), RATIO_GATE, edges);
                }
            }
            here = until;
            match next {
                Some(p) => {
                    self.ratio = p;
                    self.pending = None;
                }
                None => break,
            }
        }
    }
}

impl CpuNode for RatioGear {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{:.3} cycles at {}{}{}",
            self.output,
            ladder::label(self.ratio),
            self.pending
                .map_or_else(String::new, |p| format!(" → {}", ladder::label(p))),
            if self.held { " held" } else { "" }
        ))
    }

    fn gear(&self) -> Option<Reading> {
        Some(Reading::Ratio {
            input: self.input,
            output: self.output,
            ratio: self.ratio,
            pending: self.pending,
            held: self.held,
        })
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let knob = ladder::exact(f64::from(ctx.input(id, "ratio")));
        let time = ctx.time;
        let moments = moments(id, ctx, &mut self.hold, &mut self.reset);
        let source = ctx.source(id, "clock");
        // A count published whole is read in `f64`; anything else is one `f32`, and its
        // readings are only as near as that.
        let raw = source.is_some().then(|| ctx.count(id, "clock"));
        let epsilon = if raw.is_some() && !ctx.counted(id, "clock") {
            f64::from(f32::EPSILON)
        } else {
            f64::EPSILON
        };

        // Born where the input is: on a jump, on the first tick, and where Clock In is cabled,
        // let go or moved onto another output, since the clock it counts is then another one.
        let birth = time.jumped || !self.born || source != self.source;

        // How far the input moved this frame: the playhead's advance, or the cabled clock's
        // step, unwrapped where its output says it wraps.
        let delta = match (raw, self.raw) {
            (Some(now), Some(was)) => phasor::unwrap_at(was, now, ctx.wraps_at(id, "clock")),
            _ => time.advance,
        };
        // A cabled clock its source says was put where it is — a gear's Reset above, or a
        // gear above born again — or one sent back more than a cycle in one frame jumped:
        // nothing on the way was played through. A step back of less than a cycle with no
        // word from the source is a clock running backwards; forwards, a fast gear crosses
        // several cycles a frame, which is motion.
        let thrown_back = raw.is_some() && (delta < -1.0 || ctx.jumped(id, "clock"));

        if birth || thrown_back {
            let first = !self.born;
            self.born = true;
            self.ratio = knob;
            self.pending = None;
            if let Some(raw) = raw {
                self.input = raw;
                self.output = knob * raw;
            } else {
                self.input = time.playhead;
                self.output = knob * time.playhead;
            }
            self.raw = raw;
            for (_, moment) in moments {
                if let Moment::Hold = moment {
                    self.held = !self.held;
                }
            }
            // How long the clock has run since it was put where it is: since the seek landed,
            // on a birth the transport made; a whole frame since a Reset above, which lands at
            // the top of the frame; nothing on the first tick. A cable moved is no beat.
            let kept = source == self.source || first;
            let since = if time.jumped {
                time.landed.filter(|_| kept).map(|l| time.playhead - l)
            } else if thrown_back {
                Some(time.advance)
            } else if first {
                Some(0.0)
            } else {
                None
            };
            if source != self.source || first {
                self.pace = None;
            }
            // Put somewhere the transport did not put everything: what counts this gear
            // jumps with it.
            if !time.jumped && !first {
                jumped(id, ctx);
            }
            self.source = source;
            // A gear put down on a whole cycle of its own is on its downbeat, as a Reset is a
            // beat: that one beat, and nothing for the cycles it went back over. The clock
            // has run on since, so the downbeat counts where it falls inside that run at the
            // pace the clock was going: one a second on ambient seconds.
            if let Some(since) = since
                && !self.held
            {
                let (pace, error) = if raw.is_some() {
                    self.pace.unwrap_or((0.0, 0.0))
                } else {
                    (1.0, 0.0)
                };
                // How far past its downbeat the way it runs: a gear counts down where its
                // ratio or its clock runs backwards.
                let past = if knob * pace < 0.0 || (pace == 0.0 && knob < 0.0) {
                    self.output.ceil() - self.output
                } else {
                    self.output - self.output.floor()
                };
                let motion = (knob * pace * since).abs();
                let slack = knob.abs() * (error * since + epsilon * self.input.abs());
                let whole = on_a_whole_cycle(self.output);
                if whole || past <= motion + slack + phasor::REACH {
                    let at = if whole || motion <= 0.0 || !thrown_back {
                        0.0
                    } else {
                        (1.0 - past / motion).clamp(0.0, 1.0)
                    };
                    fire(id, ctx, &mut self.trigger, vec![(at, true)]);
                }
            }
            publish(id, ctx, self.output);
            return;
        }
        if let (Some(now), Some(was)) = (raw, self.raw)
            && time.advance > 0.0
        {
            let error = epsilon * (now.abs() + was.abs());
            self.pace = Some((delta / time.advance, error / time.advance));
        }
        self.raw = raw;
        self.pending = (knob != self.ratio).then_some(knob);

        let start = self.input;
        let mut edges = Vec::new();
        let mut done = 0.0;
        for at in moments.into_iter().map(Some).chain([None]) {
            let to = at.map_or(1.0, |(f, _)| f.clamp(done, 1.0));
            self.run(
                start + delta * done,
                start + delta * to,
                done,
                to,
                &mut edges,
            );
            done = to;
            match at.map(|(_, m)| m) {
                Some(Moment::Hold) => {
                    self.held = !self.held;
                    if self.held {
                        edges.push((to, false));
                    }
                }
                Some(Moment::Reset) => {
                    self.output = 0.0;
                    edges.push((to, true));
                    jumped(id, ctx);
                }
                None => {}
            }
        }
        self.input = start + delta;
        fire(id, ctx, &mut self.trigger, edges);
        publish(id, ctx, self.output);
    }
}

// ------------------------------------------------------------------------------ ladder

/// The Ratio Gear's ratio as a hand reads and moves it: a ladder a drag walks, a label, and
/// the typed forms it takes.
///
/// **A drag walks ÷16, ÷8, ÷6, ÷4, ÷3, ÷2, ×1, ×2, ×3, ×4, ×6, ×8, ×12, ×16**, and on through
/// ×0 into the same rungs reversed. **Typed, it takes anything**: `×5`, `÷7`, `3/2`, `0.3`,
/// or a sign in front for reverse (`-×1`). A ratio that is not a whole ×n or ÷n is allowed;
/// a loop of the master above it is then as many cycles as its denominator
/// (`nodes::chain`).
pub mod ladder {
    /// The rungs above ×0, in order.
    pub const RUNGS: [f64; 14] = [
        1.0 / 16.0,
        1.0 / 8.0,
        1.0 / 6.0,
        1.0 / 4.0,
        1.0 / 3.0,
        1.0 / 2.0,
        1.0,
        2.0,
        3.0,
        4.0,
        6.0,
        8.0,
        12.0,
        16.0,
    ];

    /// The top rung's index: rungs run from `-TOP` through ×0 at zero to `TOP`.
    pub const TOP: i32 = RUNGS.len() as i32;

    /// The ratio on rung `i`.
    pub fn at(i: i32) -> f64 {
        let i = i.clamp(-TOP, TOP);
        if i == 0 {
            0.0
        } else {
            RUNGS[(i.unsigned_abs() - 1) as usize].copysign(f64::from(i))
        }
    }

    /// The rung nearest `r`, measured in octaves: ×0 below half of ÷16.
    pub fn nearest(r: f64) -> i32 {
        if !r.is_finite() || r.abs() < RUNGS[0] / 2.0 {
            return 0;
        }
        let m = r.abs().ln();
        let best = RUNGS
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.ln() - m).abs().total_cmp(&(b.1.ln() - m).abs()))
            .map_or(0, |(i, _)| i as i32 + 1);
        if r < 0.0 { -best } else { best }
    }

    /// The fraction `p/q` a ratio is, with `q` up to `max_q`, where it is one to within
    /// rounding an `f32` control leaves.
    pub fn fraction(r: f64, max_q: i64) -> Option<(i64, i64)> {
        if !r.is_finite() {
            return None;
        }
        (1..=max_q).find_map(|q| {
            let p = (r * q as f64).round();
            ((r * q as f64 - p).abs() < 1e-4 * q as f64 && p.abs() < 1e6).then_some((p as i64, q))
        })
    }

    /// The nearest small fraction to a ratio, `q` up to sixteen, and whether it is the ratio
    /// itself: what a display draws for a ratio that is not a whole one.
    pub fn nearest_fraction(r: f64) -> (i64, i64, bool) {
        if let Some((p, q)) = fraction(r, 64) {
            let g = gcd(p.unsigned_abs(), q.unsigned_abs()).max(1) as i64;
            return (p / g, q / g, true);
        }
        let mut best = (r.round() as i64, 1, f64::MAX);
        for q in 1..=16i64 {
            let p = (r * q as f64).round();
            let err = (r - p / q as f64).abs();
            if err < best.2 {
                best = (p as i64, q, err);
            }
        }
        (best.0, best.1, false)
    }

    /// The exact ratio a control's `f32` stands for: the fraction it is, where it is one with
    /// a denominator up to 64, so ÷3 runs at a third and not at `0.33333334`.
    pub fn exact(r: f64) -> f64 {
        match fraction(r, 64) {
            Some((p, q)) => p as f64 / q as f64,
            None => r,
        }
    }

    pub fn gcd(mut a: u64, mut b: u64) -> u64 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    }

    /// A ratio as the control and the display write it: `×3`, `÷4`, `3/2`, `×0`, `0.37`,
    /// with a minus in front for one run backwards.
    pub fn label(r: f64) -> String {
        if !r.is_finite() {
            return "×?".to_string();
        }
        let sign = if r < 0.0 { "-" } else { "" };
        let m = r.abs();
        if m < 1e-6 {
            return "×0".to_string();
        }
        match fraction(m, 64) {
            Some((p, 1)) => format!("{sign}×{p}"),
            Some((1, q)) => format!("{sign}÷{q}"),
            Some((p, q)) => {
                let g = gcd(p as u64, q as u64).max(1) as i64;
                format!("{sign}{}/{}", p / g, q / g)
            }
            None => format!("{sign}{m:.3}"),
        }
    }

    /// A typed ratio: `×5` or `x5` or `*5`, `÷7` or `/7`, `3/2`, a plain number, each with an
    /// optional minus in front. `None` for anything else.
    pub fn parse(text: &str) -> Option<f64> {
        let text = text.trim();
        let (sign, body) = match text.strip_prefix(['-', '−']) {
            Some(rest) => (-1.0, rest.trim()),
            None => (1.0, text.strip_prefix('+').unwrap_or(text).trim()),
        };
        let number = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
        let value = if let Some(rest) = body.strip_prefix(['×', 'x', 'X', '*']) {
            number(rest)?
        } else if let Some(rest) = body.strip_prefix(['÷', '/']) {
            let d = number(rest)?;
            (d != 0.0).then(|| 1.0 / d)?
        } else if let Some((p, q)) = body.split_once('/') {
            let (p, q) = (number(p)?, number(q)?);
            (q != 0.0).then(|| p / q)?
        } else {
            number(body)?
        };
        Some(sign * value)
    }

    /// Where a MIDI knob at `t`, 0 to 1, puts a ratio: its whole sweep is the ladder.
    pub fn from_knob(t: f32) -> f64 {
        let t = f64::from(t.clamp(0.0, 1.0));
        at((f64::from(-TOP) + t * f64::from(2 * TOP)).round() as i32)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_ladder_walks_its_rungs_through_zero() {
            assert_eq!(at(0), 0.0);
            assert_eq!(at(7), 1.0);
            assert_eq!(at(1), 1.0 / 16.0);
            assert_eq!(at(TOP), 16.0);
            assert_eq!(at(-7), -1.0);
            assert_eq!(nearest(5.0), 11, "×5 is nearer ×6 than ×4, by the octave");
            assert_eq!(nearest(0.3), 5, "0.3 is nearest ÷3");
            assert_eq!(nearest(-2.0), -8);
            assert_eq!(nearest(0.01), 0);
            for i in -TOP..=TOP {
                assert_eq!(nearest(at(i)), i);
            }
        }

        #[test]
        fn a_ratio_reads_and_types_as_a_hand_writes_it() {
            assert_eq!(label(3.0), "×3");
            assert_eq!(label(0.25), "÷4");
            assert_eq!(label(f64::from(1.0f32 / 3.0)), "÷3");
            assert_eq!(label(1.5), "3/2");
            assert_eq!(label(-1.0), "-×1");
            assert_eq!(label(0.0), "×0");
            assert_eq!(label(0.37), "0.370");
            assert_eq!(parse("×5"), Some(5.0));
            assert_eq!(parse("x5"), Some(5.0));
            assert_eq!(parse("÷7"), Some(1.0 / 7.0));
            assert_eq!(parse("3/2"), Some(1.5));
            assert_eq!(parse("0.3"), Some(0.3));
            assert_eq!(parse("-×1"), Some(-1.0));
            assert_eq!(parse("÷0"), None);
            assert_eq!(parse("fast"), None);
            assert_eq!(exact(f64::from(1.0f32 / 3.0)), 1.0 / 3.0);
            assert_eq!(nearest_fraction(0.3), (3, 10, true));
            let (p, q, whole) = nearest_fraction(0.3719);
            assert!(!whole, "{p}/{q}");
            assert!((p as f64 / q as f64 - 0.3719).abs() < 0.02, "{p}/{q}");
        }
    }
}
