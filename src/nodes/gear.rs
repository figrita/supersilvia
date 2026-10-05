// SPDX-License-Identifier: AGPL-3.0-or-later

//! Gears: where a rate several nodes keep time by is set, changed or divided.
//!
//! A node that moves with time runs free on its own Speed, or loops on its Time: ambient time,
//! the playhead, at a rate its kind declares, unless a gear is cabled in — and a gear is where
//! the show's clock is turned into another one (`nodes::timing`). Everything a gear drives is
//! a function of what it publishes. See `proposals/time.md`, *Gears*, and
//! `docs/cpu.md#gears`.
//!
//! **The Master Gear** is the show's own clock at a length in seconds. It integrates the
//! playhead's advance over its length in `f64`, so a length turned bends from where it is and
//! never jumps, and it is born at `playhead ÷ length` — two master gears of one length agree,
//! and at the playhead's zero every one of them is at the start of its cycle.
//!
//! **The Ratio Gear** is a pure product of its parent: `±(parent × p ÷ q) + offset`, worked
//! out afresh each tick from what arrives at Clock In — a count read whole in `f64`
//! (`TickContext::count`), anything else the one `f32` it is — or from the playhead's seconds
//! with nothing cabled. Its **Teeth**, `p : q`, are two whole numbers of one or more with no
//! port, kept as they were typed and reduced only in the arithmetic: the gear turns `p` times
//! for every `q` turns of its parent. Its **direction** ([`DIRECTION`]), Forward or Reverse,
//! is the product's sign, and its **Offset**, every CPU node's, is added after it. It has no
//! position of its own and keeps no track, so a seek, a render, a relaunch and a reopened tab
//! land it on the same count to the bit, and a change of Teeth or direction puts it where it
//! would be had it always run that way: a jump, which is the behavior (`docs/decisions.md`,
//! *A Ratio Gear is a pure product of its parent*). The one thing it remembers is the
//! parent's and the Offset's last readings, to find the whole cycles its output passed in a
//! frame, going down as going up.
//!
//! Both publish the same four: Cycles, a count published whole (`TickContext::publish_count`) —
//! to a Time in `f64` on the CPU and as a whole part and a fraction in a shader, and to
//! anything else as one `f32` wrapped at [`phasor::WRAP`] centered on zero;
//! Phase, the fraction alone; Ping-pong, a triangle over two cycles; and Trigger, an event on
//! each whole cycle placed where inside the frame it fell. A Master Gear's **Hold** is a
//! toggle that freezes it where it stands; its **Reset** puts it at the start of a cycle, and
//! is a beat. A seek — the time readout's reset among them — and a render's start are a jump:
//! every gear is born again where the playhead puts it, and fires nothing on the way. A gear
//! the jump puts on a whole cycle is on that cycle's beat and fires it, so the readout's reset
//! and a render's first frame are a downbeat. A clock cabled into a Ratio Gear that its source
//! calls a jump ([`TickContext::jump`], which a gear says of its readings on a Reset or a
//! change of Teeth or direction), or anything but a count sent back more than a cycle in a
//! frame, is a jump to it too, and fires at most one downbeat, its own where this frame's
//! motion carried it past one, so a hand's Reset above it is a beat however early in a cycle
//! it comes. A count going down with no such word is a clock running backwards.
//!
//! How each is drawn is its **Display** option, a still rosette or two meshing gears, both
//! turning at the real rate: `widgets::gear`.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::phasor::{self, Step};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Gate, InputDef, NodeDef, OptionDef, OptionKind,
    OutputDef, OutputKind, Region, TickContext, Timing,
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

/// A Ratio Gear's direction: [`FORWARD`], the default, or [`REVERSE`], which negates its
/// product. Drawn by the Teeth row's region, as two segments under `p : q`
/// (`widgets::gear::TEETH`).
pub const DIRECTION: OptionDef = OptionDef {
    key: "direction",
    label: "Direction",
    default: FORWARD,
    choices: &[(FORWARD, "Forward"), (REVERSE, "Reverse")],
    kind: OptionKind::Runtime,
    in_region: true,
    ..OptionDef::EMPTY
};

/// [`DIRECTION`]'s value for a gear that turns with its parent. The default.
pub const FORWARD: &str = "forward";

/// [`DIRECTION`]'s value for a gear that turns against its parent: `−(parent × p ÷ q)`.
pub const REVERSE: &str = "reverse";

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
    tooltip: "A clock that turns p times for every q turns of its parent, set by its Teeth: \
              2 : 1 is twice as fast, 3 : 2 three turns against two, and Reverse turns it the \
              other way. It is the parent times p ÷ q every frame, plus its Offset, so a change \
              lands at once and a seek lands where playing would. With nothing in Clock In \
              its parent is ambient seconds.",
    inputs: &[
        InputDef {
            key: "clock",
            label: "Clock In",
            ty: UniformNumber,
            // No knob: unplugged, the clock is ambient time's seconds.
            control: Control::None,
        },
        // Every CPU node's Offset, in the gear's own cycles: one cycle either way.
        crate::nodes::timing::offset_row(Timing::periodic(1.0), 0, UniformNumber),
    ],
    // Drawn on the Teeth row (`widgets::gear::TEETH`), with no port.
    hidden: &[
        InputDef {
            key: TEETH_P,
            label: "Teeth p",
            ty: UniformNumber,
            control: Control::num(1.0, 1.0, MAX_TEETH as f32, 1.0, ""),
        },
        InputDef {
            key: TEETH_Q,
            label: "Teeth q",
            ty: UniformNumber,
            control: Control::num(1.0, 1.0, MAX_TEETH as f32, 1.0, ""),
        },
    ],
    outputs: OUTPUTS,
    options: &[DIRECTION, DISPLAY],
    regions: &[Region::Teeth, Region::Gear],
    cpu: Some(CpuDef {
        create: || Box::new(RatioGear::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The key of a Ratio Gear's `p`: the turns it makes for every `q` of its parent's.
pub const TEETH_P: &str = "p";

/// The key of a Ratio Gear's `q`: the parent's turns its `p` are made in.
pub const TEETH_Q: &str = "q";

/// The most either of a Ratio Gear's Teeth reaches.
pub const MAX_TEETH: i64 = 64;

/// A Ratio Gear's Teeth as the arithmetic reads two stored numbers: each the whole number
/// nearest it, from one to [`MAX_TEETH`].
pub fn teeth(p: f32, q: f32) -> (i64, i64) {
    let whole = |v: f32| {
        if v.is_finite() {
            (v.round() as i64).clamp(1, MAX_TEETH)
        } else {
            1
        }
    };
    (whole(p), whole(q))
}

/// A Ratio Gear's Teeth on `node`, as [`teeth`] reads them.
pub fn teeth_of(node: &crate::graph::Node) -> (i64, i64) {
    let read = |key| match node.controls.get(key) {
        Some(crate::graph::ControlValue::Float(v)) => *v,
        _ => 1.0,
    };
    teeth(read(TEETH_P), read(TEETH_Q))
}

/// Whether a Ratio Gear runs in Reverse: its [`DIRECTION`].
pub fn reversed(node: &crate::graph::Node) -> bool {
    node.options
        .get(DIRECTION.key)
        .is_some_and(|d| d == REVERSE)
}

/// `p ÷ q` in lowest terms.
pub fn reduced(p: i64, q: i64) -> (i64, i64) {
    let g = crate::nodes::chain::gcd(p.unsigned_abs(), q.unsigned_abs()).max(1) as i64;
    (p / g, q / g)
}

/// A Ratio Gear's output for a parent's reading before its Offset: `parent × p ÷ q`, in lowest
/// terms, so Teeth of 2 : 4 and 1 : 2 give the same count to the bit. `p` is negative in
/// Reverse, which negates the product exactly: rounding to nearest is symmetric about zero.
pub fn product(parent: f64, p: i64, q: i64) -> f64 {
    let (p, q) = reduced(p, q);
    parent * p as f64 / q as f64
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
        /// The parent's reading.
        input: f64,
        /// Output cycles, unbounded.
        output: f64,
        /// The Teeth, `p : q`, as they are set.
        p: i64,
        q: i64,
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
    /// What the parent read on the last tick: the playhead's seconds with Clock In unplugged,
    /// the cabled clock's reading as it is published otherwise.
    raw: f64,
    /// The output cabled into Clock In on the last tick.
    source: Option<crate::graph::PortRef>,
    /// How fast the cabled clock was moving on the last tick it moved, in its cycles a second
    /// of the transport's advance, and how far off that can be from the `f32` readings it was
    /// measured on: what a frame of it is after it is thrown back.
    pace: Option<(f64, f64)>,
    born: bool,
    /// The Teeth on the last tick.
    teeth: (i64, i64),
    /// Whether it ran in Reverse on the last tick.
    reverse: bool,
    /// The Offset on the last tick, in its own cycles.
    offset: f64,
    /// Output cycles on the last tick, for the display.
    output: f64,
    trigger: Gate,
}

/// The Ratio Gear's Trigger gate: half a cycle.
const RATIO_GATE: f64 = 0.5;

impl CpuNode for RatioGear {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{:.3} cycles at {} : {}{}",
            self.output,
            self.teeth.0,
            self.teeth.1,
            if self.reverse { " in reverse" } else { "" }
        ))
    }

    fn gear(&self) -> Option<Reading> {
        Some(Reading::Ratio {
            input: self.raw,
            output: self.output,
            p: self.teeth.0,
            q: self.teeth.1,
        })
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let (p, q) = teeth(ctx.input(id, TEETH_P), ctx.input(id, TEETH_Q));
        let reverse = ctx.option(id, DIRECTION.key) == REVERSE;
        // The turns it makes for every `q` of its parent's, against it in Reverse.
        let turns = if reverse { -p } else { p };
        let ratio = product(1.0, turns, q);
        let offset = f64::from(ctx.input(id, crate::nodes::timing::OFFSET));
        let time = ctx.time;
        let source = ctx.source(id, "clock");
        let cabled = source.is_some();
        // A count published whole is read in `f64`; anything else is one `f32`, and its
        // readings are only as near as that.
        let now = if cabled {
            ctx.count(id, "clock")
        } else {
            time.playhead
        };
        let epsilon = if cabled && !ctx.counted(id, "clock") {
            f64::from(f32::EPSILON)
        } else {
            f64::EPSILON
        };
        // Negated before the Offset is added, so an Offset moves a reversed gear forwards as
        // it does any other.
        let output = product(now, turns, q) + offset;

        // Born where the parent is: on a jump, on the first tick, and where Clock In is
        // cabled, let go or moved onto another output, since the clock it counts is then
        // another one.
        let birth = time.jumped || !self.born || source != self.source;
        // How far the parent moved this frame, unwrapped where its output says it wraps.
        let delta = if cabled {
            phasor::unwrap_at(self.raw, now, ctx.wraps_at(id, "clock"))
        } else {
            now - self.raw
        };
        // A cabled clock its source says was put where it is — a gear's Reset, Teeth or
        // direction above, or a gear above born again — jumped: nothing on the way was played
        // through. So did anything but a count sent back more than a cycle in one frame. A
        // count, which only a gear or the Time node publishes, says when it jumps, so one
        // going back by any amount with no word is a clock running backwards — a fast gear in
        // Reverse passes several cycles a frame going down, as one forwards does going up —
        // and so is anything else going back by less than a cycle.
        let thrown_back = !birth
            && cabled
            && (ctx.jumped(id, "clock") || (!ctx.counted(id, "clock") && delta < -1.0));

        if birth || thrown_back {
            let first = !self.born;
            self.born = true;
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
            if let Some(since) = since {
                let (pace, error) = if cabled {
                    self.pace.unwrap_or((0.0, 0.0))
                } else {
                    (1.0, 0.0)
                };
                // How far past its downbeat the way it runs: a gear counts down where its
                // clock runs backwards or it runs in Reverse, and up where both are so.
                let going = ratio * pace;
                let past = if going < 0.0 {
                    output.ceil() - output
                } else {
                    output - output.floor()
                };
                let motion = (going * since).abs();
                let slack = ratio.abs() * (error * since + epsilon * now.abs());
                let whole = on_a_whole_cycle(output);
                if whole || past <= motion + slack + phasor::REACH {
                    let at = if whole || motion <= 0.0 || !thrown_back {
                        0.0
                    } else {
                        (1.0 - past / motion).clamp(0.0, 1.0)
                    };
                    fire(id, ctx, &mut self.trigger, vec![(at, true)]);
                }
            }
        } else {
            if cabled && time.advance > 0.0 {
                let error = epsilon * (now.abs() + self.raw.abs());
                self.pace = Some((delta / time.advance, error / time.advance));
            }
            // A change of Teeth or direction is a jump: what counts this gear is born again
            // with it.
            if (p, q) != self.teeth || reverse != self.reverse {
                jumped(id, ctx);
            }
            // Where the Offset was, unwrapped where its cable says it wraps: its motion is the
            // gear's, as a cable into any Offset moves what it offsets.
            let moved = phasor::unwrap_at(
                self.offset,
                offset,
                ctx.wraps_at(id, crate::nodes::timing::OFFSET),
            );
            let was = if offset - self.offset == moved {
                self.offset
            } else {
                offset - moved
            };
            // The frame's motion at the Teeth and direction set now, from where the parent
            // and the Offset were: the whole cycles it passed, and none for a jump the Teeth
            // or the direction made.
            let from = if now - self.raw == delta {
                product(self.raw, turns, q)
            } else {
                product(now - delta, turns, q)
            } + was;
            let mut edges = Vec::new();
            if from != output {
                edges_between(from, output, 0.0, 1.0, RATIO_GATE, &mut edges);
            }
            fire(id, ctx, &mut self.trigger, edges);
        }
        self.raw = now;
        self.teeth = (p, q);
        self.reverse = reverse;
        self.offset = offset;
        self.output = output;
        publish(id, ctx, output);
    }
}
