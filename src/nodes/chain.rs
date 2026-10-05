// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a loop of a Master Gear needs: how many of its cycles bring back everything it drives,
//! read off the graph — each node's declaration (`nodes::timing`), its mode, and the gears —
//! and whether a node on its own clock comes back over a length. The caption under a Master
//! Gear says it, `examples/loop_gifs` renders a loop that long, and so **a claim is exact and
//! the shortest that is true, or it is not made**: where the arithmetic cannot follow, the
//! caption says it cannot tell. See `docs/nodes.md#when-a-loop-closes`.
//!
//! **Everything is counted in the master's cycles, as an exact fraction** ([`Fraction`]). What
//! a cable carries is a [`Motion`]: still; a gear's count, at a rate of its cycles a master
//! cycle, as Cycles, Phase or Ping-pong; a Trigger's beats; something that comes back every so many master cycles; something that never does;
//! or something the walk cannot read. Each node turns what reaches it into what it publishes:
//!
//! - **A Ratio Gear** is its parent times its Teeth, `p ÷ q`, exactly: a count's rate in its
//!   Clock In times `p/q`, reduced, or the master's own seconds' with nothing cabled
//!   ([`Clock`]). Anything else in its Clock In — a Phase, a Ping-pong, a number that comes
//!   back — it reads as a number, and comes back when that does.
//! - **A node that moves with time** is where each axis's Time and Offset put it, a rate and
//!   things that come back on their own, read round that axis's period `P` as the graph gives
//!   it (`timing::period_in`, any positive fraction): a count at rate `r` comes back every
//!   `P ÷ r`; a Phase every gear cycle, or every `P ÷ r` where `P` divides one; a Ping-pong
//!   every two; a sequencer reads a Phase as a count, and with Step cabled advances a
//!   sixteenth of a bar a beat. Its own clock — Loop mode unplugged, or running free on its
//!   Speed's knob, at the axis's pace — is a rate in seconds over the master's seconds, read as
//!   a fraction ([`Fraction::near`]); a clip's, whose length the graph does not hold, cannot be
//!   told. Every other input it reads adds what it comes back in. A count in Speed never
//!   closes, and anything else in one cannot be told.
//! - **A Clock Divider** at ÷n turns beats every `T` into beats every `nT`.
//! - **A node with no CPU half** — arithmetic, a shader, an Output — is a function of its
//!   inputs, and comes back when all of them have. Any other CPU node keeps state the walk
//!   cannot read: it is still while nothing that moves reaches it, a device's never comes
//!   back, and otherwise it cannot be told.
//!
//! The loop is the least common multiple of what every node downstream of the master comes
//! back in, and of the master's own cycle, so whole master cycles: [`master_loop`]. A gear is
//! counted through what reads it — four on the floor on a ÷4 gear loops in one cycle, not four
//! — and a gear nothing reads by its own turn. A loop that overflows the arithmetic is one no
//! practical length closes, and is said not to close. What is claimed is the shortest the
//! graph can tell: a picture with a symmetry no declaration states — a cosine on a Ping-pong,
//! which is back every cycle and not every two — comes back sooner.

use crate::graph::{ControlValue, Graph, Node, NodeId, PortRef};
use crate::nodes::{sequencer, timing};
use std::collections::{HashMap, HashSet};

/// A ratio, a rate or a period as a fraction, reduced, its denominator positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    pub p: i64,
    pub q: i64,
}

/// The largest denominator [`Fraction::near`] reads a rate as: a loop of more master cycles
/// than this, out of a knob's rate, is no practical length.
pub const NEAR_Q: i64 = 4096;

impl Fraction {
    pub const ZERO: Self = Self { p: 0, q: 1 };
    pub const ONE: Self = Self { p: 1, q: 1 };

    /// The fraction `x` is, where it is one with a denominator up to [`NEAR_Q`]: the least `q`
    /// whose multiple of `x` is within two `f32` roundings of a whole `p`, and never more than
    /// 10⁻⁴ from it. So a Speed of 0.3 times a pace of one over a Master Gear two seconds long,
    /// `0.6000000238`, is 3/5, a period an `f64` cannot hold reads as the fraction it was
    /// written as, and a loop claimed out of it drifts by under 10⁻⁴ of a cycle — the rounding
    /// of the knob, never seen in eight bits. `None` for a number no such fraction is.
    pub fn near(x: f64) -> Option<Self> {
        if !x.is_finite() {
            return None;
        }
        (1..=NEAR_Q).find_map(|q| {
            let scaled = x * q as f64;
            let p = scaled.round();
            let slack = (2.0 * f64::from(f32::EPSILON) * p.abs()).clamp(1e-9, 1e-4);
            (p.abs() < 1e15 && (scaled - p).abs() <= slack).then(|| Self::new(p as i64, q))
        })
    }

    pub fn new(p: i64, q: i64) -> Self {
        let g = gcd(p.unsigned_abs(), q.unsigned_abs()).max(1) as i64;
        let s = if q < 0 { -1 } else { 1 };
        Self {
            p: s * p / g,
            q: s * q / g,
        }
    }

    /// `p/q` reduced, where both fit an `i64` and `q` is not zero.
    fn checked(p: i128, q: i128) -> Option<Self> {
        if q == 0 {
            return None;
        }
        let g = gcd128(p.unsigned_abs(), q.unsigned_abs()).max(1) as i128;
        let s = q.signum();
        Some(Self {
            p: i64::try_from(s * p / g).ok()?,
            q: i64::try_from(s * q / g).ok()?,
        })
    }

    /// The product, or `None` where it does not fit.
    pub fn times(self, other: Self) -> Option<Self> {
        Self::checked(
            i128::from(self.p) * i128::from(other.p),
            i128::from(self.q) * i128::from(other.q),
        )
    }

    /// The quotient, or `None` where it does not fit or `other` is zero.
    pub fn over(self, other: Self) -> Option<Self> {
        Self::checked(
            i128::from(self.p) * i128::from(other.q),
            i128::from(self.q) * i128::from(other.p),
        )
    }

    /// The sum, or `None` where it does not fit.
    pub fn plus(self, other: Self) -> Option<Self> {
        Self::checked(
            i128::from(self.p) * i128::from(other.q) + i128::from(other.p) * i128::from(self.q),
            i128::from(self.q) * i128::from(other.q),
        )
    }

    #[must_use]
    pub fn abs(self) -> Self {
        Self {
            p: self.p.abs(),
            q: self.q,
        }
    }

    /// The least positive fraction both `self` and `other`, positive, divide: the least common
    /// multiple of the numerators over the greatest common divisor of the denominators. `None`
    /// where it does not fit.
    pub fn lcm(self, other: Self) -> Option<Self> {
        let p = lcm(self.p.unsigned_abs(), other.p.unsigned_abs())?;
        let q = gcd(self.q.unsigned_abs(), other.q.unsigned_abs());
        Self::checked(i128::from(p), i128::from(q))
    }

    pub fn as_f64(self) -> f64 {
        self.p as f64 / self.q as f64
    }
}

/// The greatest common divisor of two counts; zero only where both are.
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn gcd128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The least common multiple of two positive counts, or `None` where it does not fit a `u64`.
pub fn lcm(a: u64, b: u64) -> Option<u64> {
    if a == 0 || b == 0 {
        return Some(a.max(b));
    }
    (a / gcd(a, b)).checked_mul(b)
}

/// A Ratio Gear's Teeth, `p/q`, as an exact fraction in lowest terms.
fn teeth_of(graph: &Graph, id: NodeId) -> Fraction {
    graph.get(id).map_or(Fraction::ONE, |node| {
        let (p, q) = crate::nodes::gear::teeth_of(node);
        Fraction::new(p, q)
    })
}

/// One Ratio Gear a Master Gear drives, through a chain of them: the product of every gear's
/// Teeth from the master down to it, or `None` where the product does not fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driven {
    pub node: NodeId,
    pub ratio: Option<Fraction>,
}

/// Every Ratio Gear `master` drives through its Cycles into a Clock In, and on down through
/// theirs, each with the product of the Teeth on its way. A gear a cycle of ratio gears
/// reaches twice is listed once, at the first product found.
pub fn driven(graph: &Graph, master: NodeId) -> Vec<Driven> {
    let mut out: Vec<Driven> = Vec::new();
    let mut stack = vec![(master, Some(Fraction::ONE))];
    while let Some((from, product)) = stack.pop() {
        for to in graph.targets_of(PortRef::new(from, "cycles")) {
            let Some(node) = graph.get(to.node) else {
                continue;
            };
            if node.def.slug != crate::nodes::gear::RATIO.slug
                || to.key != "clock"
                || out.iter().any(|d| d.node == to.node)
            {
                continue;
            }
            let ratio = product.and_then(|a| a.times(teeth_of(graph, to.node)));
            out.push(Driven {
                node: to.node,
                ratio,
            });
            stack.push((to.node, ratio));
        }
    }
    out.sort_by_key(|d| d.node);
    out
}

/// What a loop of a Master Gear needs: how many of its cycles bring every node downstream of
/// it back to where it started, the one that asks for the most, what never closes and what
/// cannot be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterLoop {
    pub cycles: u64,
    /// The gear or node that asks for the most cycles, and how many, where one asks for more
    /// than one.
    pub why: Option<(NodeId, i64)>,
    /// Nodes that never come back, or not in any length the arithmetic can count: a picture
    /// that never repeats on a moving clock, a count in a Speed, a device.
    pub open: Vec<NodeId>,
    /// Nodes past which the walk cannot tell: a cable in a Speed or a Divide, a count through
    /// arithmetic, a CPU node that keeps state of its own, a clip on its own clock.
    pub unsure: Vec<NodeId>,
}

/// A gear's count: `rate` of its cycles a master cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub rate: Fraction,
}

/// Which of a gear's readings a cable carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Cycles, the count.
    Count,
    /// Phase, the count's fraction.
    Phase,
    /// Ping-pong, a triangle over two cycles.
    PingPong,
}

/// How a value moves with the show, in the master's cycles: what a cable carries and what a
/// node does, as the loop arithmetic reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// The same every frame.
    Still,
    /// A gear's reading.
    Clock(Clock, Shape),
    /// Comes back every `period` master cycles; `why` asks for the most of them, and how many.
    Every {
        period: Fraction,
        why: (NodeId, i64),
    },
    /// One beat every `period` master cycles: a Trigger, or one divided.
    Beats {
        period: Fraction,
        why: (NodeId, i64),
    },
    /// Never comes back, because of this node.
    Never(NodeId),
    /// Cannot be told, past this node.
    Unknown(NodeId),
}

/// How many master cycles `h(count)` takes to come back, for `h` coming back every `p` of the
/// count's own cycles (`None`: never), on clock `c`: `p ÷ rate`. `Err` where it never comes
/// back, or the arithmetic overflows.
fn period_of(c: Clock, p: Option<Fraction>) -> Result<Fraction, ()> {
    p.ok_or(())?.over(c.rate.abs()).ok_or(())
}

/// Everything that comes back, gathered over a node's inputs: the least common multiple of
/// their periods and the one that asks for the most master cycles, or why it does not close.
#[derive(Default)]
struct Gather {
    period: Option<Fraction>,
    why: Option<(NodeId, i64)>,
    broke: Option<Motion>,
}

impl Gather {
    /// A period `node` asks for.
    fn every(&mut self, period: Fraction, node: NodeId) {
        self.with(period, (node, period.p));
    }

    fn with(&mut self, period: Fraction, why: (NodeId, i64)) {
        let period = period.abs();
        match self.period.map_or(Some(period), |p| p.lcm(period)) {
            Some(p) => self.period = Some(p),
            None => self.fail(Motion::Never(why.0)),
        }
        if why.1 > 1 && self.why.is_none_or(|(_, n)| why.1 > n) {
            self.why = Some(why);
        }
    }

    /// Something that does not close, the first that never does before the first that cannot
    /// be told.
    fn fail(&mut self, m: Motion) {
        match (self.broke, m) {
            (None, _) | (Some(Motion::Unknown(_)), Motion::Never(_)) => self.broke = Some(m),
            _ => {}
        }
    }

    /// What a node reads as a number, from what reaches `at`: a reading that comes back on its
    /// own, a beat, a Phase or Ping-pong — but a count is a number that grows without bound.
    fn reading(&mut self, m: Motion, at: NodeId) {
        match m {
            Motion::Still => {}
            Motion::Every { period, why } | Motion::Beats { period, why } => self.with(period, why),
            Motion::Clock(_, Shape::Count) => self.fail(Motion::Unknown(at)),
            Motion::Clock(c, shape) => {
                let p = if shape == Shape::Phase { 1 } else { 2 };
                match period_of(c, Some(Fraction::new(p, 1))) {
                    Ok(period) => self.every(period, at),
                    Err(()) => self.fail(Motion::Never(at)),
                }
            }
            Motion::Never(_) | Motion::Unknown(_) => self.fail(m),
        }
    }

    fn done(self) -> Motion {
        if let Some(m) = self.broke {
            return m;
        }
        match self.period {
            Some(period) => Motion::Every {
                period,
                why: self.why.unwrap_or((NodeId(0), 1)),
            },
            None => Motion::Still,
        }
    }
}

/// The walk under one Master Gear: every node's [`Motion`] in its cycles, worked out once.
struct Walk<'g> {
    graph: &'g Graph,
    master: NodeId,
    /// One cycle of the master in seconds, where its Length is a knob: what a node's own clock
    /// is counted against.
    seconds: Option<f64>,
    motions: HashMap<NodeId, Motion>,
    clocks: HashMap<NodeId, Result<Clock, Motion>>,
    /// Nodes being worked out: a walk that comes back to one has gone round a loop through a
    /// frame, which nothing here reads.
    busy: HashSet<NodeId>,
}

fn is_gear(node: &Node) -> bool {
    node.def.slug == crate::nodes::gear::MASTER.slug
        || node.def.slug == crate::nodes::gear::RATIO.slug
}

impl<'g> Walk<'g> {
    fn new(graph: &'g Graph, master: NodeId) -> Self {
        Self {
            graph,
            master,
            seconds: master_seconds(graph, master),
            motions: HashMap::new(),
            clocks: HashMap::new(),
            busy: HashSet::new(),
        }
    }

    /// A rate in a node's own cycles a second, in its cycles a master cycle.
    fn per_cycle(&self, id: NodeId, per_second: f64) -> Result<Fraction, Motion> {
        if per_second == 0.0 {
            return Ok(Fraction::ZERO);
        }
        let seconds = self.seconds.ok_or(Motion::Unknown(id))?;
        Fraction::near(per_second * seconds).ok_or(Motion::Never(id))
    }

    /// What arrives at `id`'s input `key`: still with nothing cabled, what its one source
    /// carries, and on an action input with several, still while all of them are.
    fn into(&mut self, id: NodeId, key: &'static str) -> Motion {
        let sources: Vec<PortRef> = self.graph.sources_of(PortRef::new(id, key)).collect();
        match sources.as_slice() {
            [] => Motion::Still,
            [one] => self.port(*one),
            many => {
                let mut g = Gather::default();
                for m in many.iter().map(|s| self.port(*s)) {
                    match m {
                        Motion::Still => {}
                        Motion::Never(_) | Motion::Unknown(_) => g.fail(m),
                        _ => g.fail(Motion::Unknown(id)),
                    }
                }
                g.broke.unwrap_or(Motion::Still)
            }
        }
    }

    /// What a cable from `from` carries.
    fn port(&mut self, from: PortRef) -> Motion {
        let Some(node) = self.graph.get(from.node) else {
            return Motion::Still;
        };
        if is_gear(node) {
            return match self.clock(from.node) {
                Err(m) => m,
                Ok(c) => match from.key {
                    "cycles" => Motion::Clock(c, Shape::Count),
                    "wrapped" => Motion::Clock(c, Shape::Phase),
                    "pingpong" => Motion::Clock(c, Shape::PingPong),
                    "trigger" => match Fraction::ONE.over(c.rate.abs()) {
                        Some(period) => Motion::Beats {
                            period,
                            why: (from.node, period.p),
                        },
                        None => Motion::Never(from.node),
                    },
                    _ => Motion::Unknown(from.node),
                },
            };
        }
        if node.def.slug == crate::nodes::clockdivider::DEF.slug {
            return self.divided(from.node);
        }
        self.motion(from.node)
    }

    /// A Clock Divider's Divided Out: beats every `T` in, every `n × T` out, at a Divide that
    /// is a knob and with nothing moving in its Reset.
    fn divided(&mut self, id: NodeId) -> Motion {
        let Some(node) = self.graph.get(id) else {
            return Motion::Still;
        };
        if self.graph.source_of(PortRef::new(id, "divide")).is_some() {
            return Motion::Unknown(id);
        }
        let n = match node.controls.get("divide") {
            Some(ControlValue::Float(v)) => v.max(1.0).round() as i64,
            _ => return Motion::Unknown(id),
        };
        match self.into(id, "reset") {
            Motion::Still => {}
            m @ (Motion::Never(_) | Motion::Unknown(_)) => return m,
            _ => return Motion::Unknown(id),
        }
        match self.into(id, "input") {
            Motion::Beats { period, .. } => match period.times(Fraction::new(n, 1)) {
                Some(period) => Motion::Beats {
                    period,
                    why: (id, period.p),
                },
                None => Motion::Never(id),
            },
            m @ (Motion::Still | Motion::Never(_) | Motion::Unknown(_)) => m,
            _ => Motion::Unknown(id),
        }
    }

    /// A gear's count, or what it publishes where that is not a clock: still on a still
    /// clock or at ×0, and otherwise why it does not close.
    fn clock(&mut self, id: NodeId) -> Result<Clock, Motion> {
        if let Some(c) = self.clocks.get(&id) {
            return *c;
        }
        if !self.busy.insert(id) {
            return Err(Motion::Unknown(id));
        }
        let c = self.work_out_clock(id);
        self.busy.remove(&id);
        self.clocks.insert(id, c);
        c
    }

    fn work_out_clock(&mut self, id: NodeId) -> Result<Clock, Motion> {
        let graph = self.graph;
        let node = graph.get(id).ok_or(Motion::Still)?;
        let master = node.def.slug == crate::nodes::gear::MASTER.slug;
        let rate = if id == self.master {
            Fraction::ONE
        } else if master {
            // Another master is a clock of its own: its cycles over this one's.
            let theirs = master_seconds(graph, id).ok_or(Motion::Unknown(id))?;
            self.per_cycle(id, 1.0 / theirs)?
        } else {
            let input = match graph.source_of(PortRef::new(id, "clock")) {
                // Ambient seconds.
                None => self.per_cycle(id, 1.0)?,
                Some(src) => match self.port(src) {
                    Motion::Clock(c, Shape::Count) => c.rate,
                    // A number times p ÷ q comes back when the number does.
                    m => {
                        let mut g = Gather::default();
                        g.reading(m, id);
                        return Err(g.done());
                    }
                },
            };
            input.times(teeth_of(graph, id)).ok_or(Motion::Never(id))?
        };
        if master {
            if graph.source_of(PortRef::new(id, "gate")).is_some() {
                return Err(Motion::Unknown(id));
            }
            // A master is the yardstick: one reset or held is not a whole number of anything.
            for key in ["reset", "hold"] {
                match self.into(id, key) {
                    Motion::Still => {}
                    m @ (Motion::Never(_) | Motion::Unknown(_)) => return Err(m),
                    _ => return Err(Motion::Unknown(id)),
                }
            }
        }
        if rate.p == 0 {
            return Err(Motion::Still);
        }
        Ok(Clock { rate })
    }

    /// What `id` does, in the master's cycles.
    fn motion(&mut self, id: NodeId) -> Motion {
        let graph = self.graph;
        let Some(node) = graph.get(id) else {
            return Motion::Still;
        };
        if is_gear(node) {
            return match self.clock(id) {
                Err(m) => m,
                Ok(c) => match period_of(c, Some(Fraction::ONE)) {
                    Ok(period) => Motion::Every {
                        period,
                        why: (id, period.p),
                    },
                    Err(()) => Motion::Never(id),
                },
            };
        }
        if let Some(m) = self.motions.get(&id) {
            return *m;
        }
        if !self.busy.insert(id) {
            return Motion::Unknown(id);
        }
        let m = self.work_out(id, node);
        self.busy.remove(&id);
        self.motions.insert(id, m);
        m
    }

    fn work_out(&mut self, id: NodeId, node: &Node) -> Motion {
        if node.def.slug == crate::nodes::clockdivider::DEF.slug {
            return match self.divided(id) {
                Motion::Beats { period, why } => Motion::Every { period, why },
                m => m,
            };
        }
        if let Some(t) = node.def.timing {
            return self.timed(id, node, t);
        }
        let cables: Vec<PortRef> = self.graph.cables_into(id).iter().map(|c| c.from).collect();
        let mut g = Gather::default();
        match node.def.cpu.as_ref() {
            // A function of its inputs.
            None => {
                for from in cables {
                    let m = self.port(from);
                    g.reading(m, id);
                }
                g.done()
            }
            Some(cpu) if cpu.live => Motion::Never(id),
            Some(cpu) => {
                let moves = cpu.integrates || node.def.slug == crate::nodes::time::DEF.slug;
                for from in cables {
                    match self.port(from) {
                        Motion::Still => {}
                        m @ (Motion::Never(_) | Motion::Unknown(_)) => g.fail(m),
                        _ => g.fail(Motion::Unknown(id)),
                    }
                }
                if moves {
                    g.fail(Motion::Unknown(id));
                }
                g.broke.unwrap_or(Motion::Still)
            }
        }
    }

    /// A node that moves with time: where each axis's Time and Offset put it, round its
    /// period, and everything else it reads.
    fn timed(&mut self, id: NodeId, node: &Node, t: timing::Timing) -> Motion {
        let graph = self.graph;
        let sequencer = sequencer::is_sequencer(node.def);
        let stepped = sequencer
            && graph
                .sources_of(PortRef::new(id, sequencer::STEP))
                .next()
                .is_some();
        let mut g = Gather::default();
        for axis in t.axes() {
            // Its period on this axis, with what a cable drives read as anything it could be.
            let given = timing::period_in(graph, id, *axis);
            let period = given.and_then(|p| Fraction::near(p).filter(|f| f.p > 0));
            // A picture that reads further and further into its input comes back when the input
            // does, which nothing here can know.
            if (given.is_some() && period.is_none()) || (given.is_none() && (t.open)(node)) {
                g.fail(Motion::Unknown(id));
                continue;
            }
            let pace = t.pace_of(*axis);
            // The rate it moves at, in its cycles a master cycle, and what comes back on its
            // own.
            let mut rate = Fraction::ZERO;
            let mut every = Gather::default();
            if stepped {
                // A sixteenth of a bar a beat, and nothing else of time.
                match self.into(id, sequencer::STEP) {
                    Motion::Still => {}
                    Motion::Beats { period, .. } => {
                        match Fraction::new(1, sequencer::STEPS_A_BAR).over(period) {
                            Some(r) => rate = r,
                            None => g.fail(Motion::Never(id)),
                        }
                    }
                    m @ (Motion::Never(_) | Motion::Unknown(_)) => g.fail(m),
                    _ => g.fail(Motion::Unknown(id)),
                }
            } else {
                let own = if timing::runs_free(node) {
                    match graph.source_of(PortRef::new(id, axis.speed)) {
                        // A rate that keeps growing never comes round.
                        Some(src) => match self.port(src) {
                            Motion::Clock(_, Shape::Count) => Err(Motion::Never(id)),
                            m @ (Motion::Never(_) | Motion::Unknown(_)) => Err(m),
                            _ => Err(Motion::Unknown(id)),
                        },
                        None => match node.controls.get(axis.speed) {
                            Some(ControlValue::Float(speed)) => {
                                self.own(id, t, f64::from(*speed) * pace)
                            }
                            _ => Ok(Fraction::ZERO),
                        },
                    }
                } else {
                    match graph.source_of(PortRef::new(id, axis.time)) {
                        None => self.own(id, t, pace),
                        Some(src) => {
                            let m = self.port(src);
                            Self::place(m, period, sequencer, id, &mut rate, &mut every);
                            Ok(Fraction::ZERO)
                        }
                    }
                };
                match own.and_then(|own| rate.plus(own).ok_or(Motion::Never(id))) {
                    Ok(r) => rate = r,
                    Err(m) => g.fail(m),
                }
                if let Some(src) = graph.source_of(PortRef::new(id, axis.offset)) {
                    let m = self.port(src);
                    Self::place(m, period, sequencer, id, &mut rate, &mut every);
                }
            }
            if rate.p != 0 {
                match period.and_then(|p| p.over(rate.abs())) {
                    Some(p) => every.every(p, id),
                    None => every.fail(Motion::Never(id)),
                }
            }
            match every.done() {
                Motion::Still => {}
                Motion::Every { period, why } => g.with(period, why),
                m => g.fail(m),
            }
        }
        // Everything else it reads: Gate, Amplitude, a lane's knobs, a clip's tuning.
        let cables: Vec<PortRef> = graph
            .cables_into(id)
            .iter()
            .filter(|c| {
                !(timing::is_time_row(c.to.key) || (sequencer && c.to.key == sequencer::STEP))
            })
            .map(|c| c.from)
            .collect();
        for from in cables {
            let m = self.port(from);
            g.reading(m, id);
        }
        g.done()
    }

    /// A node's own clock at `per_second` of its cycles a second, in its cycles a master
    /// cycle: a clip's play is a length the graph does not hold.
    fn own(&self, id: NodeId, t: timing::Timing, per_second: f64) -> Result<Fraction, Motion> {
        if per_second == 0.0 {
            return Ok(Fraction::ZERO);
        }
        if t.clip {
            return Err(Motion::Unknown(id));
        }
        self.per_cycle(id, per_second)
    }

    /// Add what a cable into a Time or an Offset of node `id`, period `period`, carries to
    /// where the node is: a count's rate to `rate`, and anything that comes back on its own to
    /// `every`. A Phase is a count where the period divides one, and in a sequencer, which
    /// reads it unwrapped.
    fn place(
        m: Motion,
        period: Option<Fraction>,
        unwrapped: bool,
        id: NodeId,
        rate: &mut Fraction,
        every: &mut Gather,
    ) {
        let divides_one = period.is_some_and(|p| p.p == 1);
        match m {
            Motion::Still => {}
            Motion::Clock(c, shape) => {
                let counts =
                    shape == Shape::Count || (shape == Shape::Phase && (unwrapped || divides_one));
                let reading = if counts {
                    period
                } else if shape == Shape::Phase {
                    Some(Fraction::ONE)
                } else {
                    Some(Fraction::new(2, 1))
                };
                if counts {
                    match rate.plus(c.rate) {
                        Some(r) => *rate = r,
                        None => every.fail(Motion::Never(id)),
                    }
                } else {
                    match period_of(c, reading) {
                        Ok(p) => every.every(p, id),
                        Err(()) => every.fail(Motion::Never(id)),
                    }
                }
            }
            Motion::Every { period, why } => every.with(period, why),
            Motion::Never(_) | Motion::Unknown(_) => every.fail(m),
            Motion::Beats { .. } => every.fail(Motion::Unknown(id)),
        }
    }
}

/// Whether anything reads one of `node`'s outputs.
fn read(graph: &Graph, node: &Node, id: NodeId) -> bool {
    node.def
        .outputs
        .iter()
        .any(|o| graph.targets_of(PortRef::new(id, o.key)).next().is_some())
}

/// Every node downstream of `from`, `from` among them, in id order.
fn downstream(graph: &Graph, from: NodeId) -> Vec<NodeId> {
    let mut seen = HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        let Some(node) = graph.get(id) else {
            continue;
        };
        for output in node.def.outputs {
            for to in graph.targets_of(PortRef::new(id, output.key)) {
                if seen.insert(to.node) {
                    stack.push(to.node);
                }
            }
        }
    }
    let mut out: Vec<NodeId> = seen.into_iter().collect();
    out.sort();
    out
}

/// [`MasterLoop`] for one Master Gear: every node downstream of it, each with what it comes
/// back in by the rules in the module doc. A gear is counted through what reads it, so a ÷4
/// under four on the floor costs nothing, and one whose readings reach nothing by its own
/// turn, so a ÷4 left alone asks for four.
pub fn master_loop(graph: &Graph, master: NodeId) -> MasterLoop {
    let mut l = MasterLoop {
        cycles: 1,
        why: None,
        open: Vec::new(),
        unsure: Vec::new(),
    };
    let mut walk = Walk::new(graph, master);
    let push = |list: &mut Vec<NodeId>, node: NodeId| {
        if !list.contains(&node) {
            list.push(node);
        }
    };
    for id in downstream(graph, master) {
        // A gear is counted by what reads it, and one nothing reads by its own turn.
        if graph
            .get(id)
            .is_some_and(|n| is_gear(n) && read(graph, n, id))
        {
            continue;
        }
        match walk.motion(id) {
            Motion::Still => {}
            Motion::Every { period, why } | Motion::Beats { period, why } => {
                match lcm(l.cycles, period.p.unsigned_abs()) {
                    Some(cycles) => l.cycles = cycles,
                    None => push(&mut l.open, why.0),
                }
                if why.1 > 1 && l.why.is_none_or(|(_, n)| why.1 > n) {
                    l.why = Some(why);
                }
            }
            Motion::Never(node) => push(&mut l.open, node),
            Motion::Unknown(node) => push(&mut l.unsure, node),
            Motion::Clock(..) => push(&mut l.unsure, id),
        }
    }
    l.open.sort();
    l.unsure.sort();
    l
}

/// How many of its own cycles a second `node`'s Time runs at on its own clock, on one axis:
/// in Loop mode with nothing in that Time, the axis's pace; running free with nothing in that
/// Speed, its Speed's knob times that pace (`nodes::timing`). `None` where a cable drives it,
/// and on a clip, whose pace is one play over a length the graph does not hold.
pub fn own_rate(graph: &Graph, node: NodeId, axis: timing::Axis) -> Option<f64> {
    let n = graph.get(node)?;
    let t = n.def.timing.filter(|t| !t.clip)?;
    if timing::runs_free(n) {
        if graph.source_of(PortRef::new(node, axis.speed)).is_some() {
            return None;
        }
        match n.controls.get(axis.speed) {
            Some(ControlValue::Float(speed)) => Some(f64::from(*speed) * t.pace_of(axis)),
            _ => None,
        }
    } else {
        graph
            .source_of(PortRef::new(node, axis.time))
            .is_none()
            .then_some(t.pace_of(axis))
    }
}

/// Whether `node`, on its own clock, comes back after `seconds`: on every axis
/// `own_rate × seconds ÷ period` is whole, by the period the graph gives that axis
/// (`timing::period_in`) — which a node standing still always is and a moving picture that
/// never repeats never is. `None` for a node that does not move with time, a
/// clip, and one a cable drives, whose loop is its chain's.
pub fn closes_alone(graph: &Graph, node: NodeId, seconds: f64) -> Option<bool> {
    let n = graph.get(node)?;
    let t = n.def.timing?;
    let mut closes = true;
    for axis in t.axes() {
        let rate = own_rate(graph, node, *axis)?;
        if rate == 0.0 {
            continue;
        }
        closes &= timing::period_in(graph, node, *axis).is_some_and(|p| {
            let turns = rate * seconds / p;
            (turns - turns.round()).abs() < 1e-6
        });
    }
    Some(closes)
}

/// How long one cycle of a Master Gear is, in seconds, where its Length is a knob.
pub fn master_seconds(graph: &Graph, master: NodeId) -> Option<f64> {
    let node = graph.get(master)?;
    if graph.source_of(PortRef::new(master, "length")).is_some() {
        return None;
    }
    match node.controls.get("length") {
        Some(ControlValue::Float(v)) => Some(crate::nodes::gear::seconds_a_cycle(f64::from(*v))),
        _ => None,
    }
}

/// How long a loop of a Master Gear is, in seconds: its cycles a loop times one cycle's
/// length. `None` where something downstream never closes or cannot be told, or its length
/// is cabled.
pub fn master_length(graph: &Graph, master: NodeId) -> Option<f64> {
    let l = master_loop(graph, master);
    if !l.open.is_empty() || !l.unsure.is_empty() {
        return None;
    }
    Some(master_seconds(graph, master)? * l.cycles as f64)
}

/// The line under every Master Gear's picture that `shown` names.
pub fn captions(graph: &Graph, shown: impl Fn(NodeId) -> bool) -> HashMap<NodeId, String> {
    graph
        .iter()
        .filter(|(id, n)| n.def.slug == crate::nodes::gear::MASTER.slug && shown(*id))
        .map(|(id, _)| (id, caption(graph, id)))
        .collect()
}

/// A node as a caption names it: its slug and its id, `ratiogear12`.
fn named(graph: &Graph, node: NodeId) -> String {
    let slug = graph.get(node).map_or("node", |n| n.def.slug);
    format!("{slug}{}", node.0)
}

/// The line under a Master Gear's picture: what a loop of it needs. "loops in 1 cycle ·
/// 2.000 s", "loops in 4 cycles · 8.000 s (÷4 on ratiogear12)", "… (÷4 on perlin5)"; "2 nodes
/// will not close (perlin5)", naming the first; or, where nothing is known never to close but
/// something cannot be told, "can't tell when 1 node closes (multiply3)".
pub fn caption(graph: &Graph, master: NodeId) -> String {
    let l = master_loop(graph, master);
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    if let Some(first) = l.open.first() {
        let n = l.open.len();
        return format!(
            "{n} node{} will not close ({})",
            plural(n),
            named(graph, *first)
        );
    }
    if let Some(first) = l.unsure.first() {
        let n = l.unsure.len();
        let verb = if n == 1 { "closes" } else { "close" };
        return format!(
            "can't tell when {n} node{} {verb} ({})",
            plural(n),
            named(graph, *first)
        );
    }
    let cycles = if l.cycles == 1 {
        "1 cycle".to_string()
    } else {
        format!("{} cycles", l.cycles)
    };
    let seconds = master_seconds(graph, master)
        .map_or_else(String::new, |s| format!(" · {:.3} s", s * l.cycles as f64));
    let why = l.why.map_or_else(String::new, |(node, q)| {
        format!(" (÷{q} on {})", named(graph, node))
    });
    format!("loops in {cycles}{seconds}{why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node of kind `slug`, at Teeth `p : q` where it is a Ratio Gear, and on a clock —
    /// Loop mode — where it moves with time, so a gear can be cabled into its Time.
    fn gear(g: &mut Graph, slug: &str, teeth: Option<(f32, f32)>) -> NodeId {
        let id = crate::nodes::add_to_graph(g, slug, emath::Pos2::ZERO).unwrap();
        loops(g, id);
        if let Some((p, q)) = teeth {
            set(g, id, crate::nodes::gear::TEETH_P, p);
            set(g, id, crate::nodes::gear::TEETH_Q, q);
        }
        id
    }

    fn loops(g: &mut Graph, id: NodeId) {
        let n = g.get_mut(id).unwrap();
        if n.def.timing.is_some() {
            n.options.insert(timing::MODE.key, timing::LOOP.to_string());
        }
    }

    fn set(g: &mut Graph, id: NodeId, key: &'static str, v: f32) {
        g.get_mut(id)
            .unwrap()
            .controls
            .insert(key, ControlValue::Float(v));
    }

    fn connect(g: &mut Graph, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
        g.connect(PortRef::new(from.0, from.1), PortRef::new(to.0, to.1))
            .unwrap();
    }

    /// **A small fraction is read back out of the float it was written as**: a third, a
    /// tenth, a Speed of 0.3 over a two-second master, and the `f32` rounding of each; a number
    /// no fraction with a denominator up to 4096 is, or one too far from its nearest, is not.
    #[test]
    fn a_fraction_is_read_out_of_its_float() {
        let f = |p, q| Some(Fraction::new(p, q));
        assert_eq!(Fraction::near(1.0 / 3.0), f(1, 3));
        assert_eq!(Fraction::near(f64::from(1.0f32 / 3.0)), f(1, 3));
        assert_eq!(Fraction::near(0.1), f(1, 10));
        assert_eq!(Fraction::near(2.5), f(5, 2));
        assert_eq!(Fraction::near(64.0), f(64, 1));
        assert_eq!(Fraction::near(f64::from(0.3f32) * 2.0), f(3, 5));
        assert_eq!(
            Fraction::near(f64::from(0.37f32) * 2.0 / 160.0),
            None,
            "37/8000"
        );
        assert_eq!(Fraction::near(-0.75), f(-3, 4));
        assert_eq!(Fraction::near(0.0), f(0, 1));
        let golden = f64::midpoint(1.0, 5f64.sqrt());
        assert_eq!(
            Fraction::near(golden),
            None,
            "the worst there is to approximate"
        );
        assert_eq!(Fraction::near(f64::NAN), None);
        assert_eq!(
            Fraction::new(3, 2).lcm(Fraction::new(5, 4)),
            f(15, 2),
            "3/2 and 5/4 meet at 15/2"
        );
        assert_eq!(lcm(u64::MAX, 2), None);
        assert_eq!(
            Fraction::new(i64::MAX, 1).times(Fraction::new(2, 1)),
            None,
            "an overflow is no fraction"
        );
    }

    /// **A clock cabled into a Speed never closes**: a gear's Cycles into a free-running
    /// node's Speed is a rate that keeps growing. Through a Multiply the walk cannot tell what
    /// the number is, and says so.
    #[test]
    fn a_clock_in_a_speed_will_not_close() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        connect(&mut g, (master, "cycles"), (perlin, timing::SPEED));
        assert_eq!(master_loop(&g, master).open, [perlin]);
        assert_eq!(
            caption(&g, master),
            format!("1 node will not close (perlin{})", perlin.0)
        );

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        connect(&mut g, (master, "cycles"), (times, "a"));
        connect(&mut g, (times, "output"), (perlin, timing::SPEED));
        let l = master_loop(&g, master);
        assert_eq!((l.open.len(), l.unsure.as_slice()), (0, [times].as_slice()));
        assert_eq!(
            caption(&g, master),
            format!("can't tell when 1 node closes (multiply{})", times.0)
        );
        assert_eq!(master_length(&g, master), None);
    }

    /// **A node on its own clock closes where its rate times the length over its period is
    /// whole**: running free, its Speed times its pace — Mandelbrot at Speed 1, half a drift a
    /// second, closes over 2 s and not 3 s, and at Speed 1.5 over 4 s — and in Loop mode with
    /// nothing in its Time, its pace. A node standing still closes on anything, a noise
    /// that never repeats closes on nothing, a cable in its Speed leaves it to its chain, and a
    /// clip's length is not in the graph.
    #[test]
    fn a_node_on_its_own_clock_closes_where_its_rate_comes_round() {
        let mut g = Graph::new();
        let fractal = crate::nodes::add_to_graph(&mut g, "mandelbrot", emath::Pos2::ZERO).unwrap();
        assert_eq!(own_rate(&g, fractal, timing::Axis::X), Some(0.5));
        assert_eq!(closes_alone(&g, fractal, 2.0), Some(true));
        assert_eq!(closes_alone(&g, fractal, 3.0), Some(false));
        set(&mut g, fractal, timing::SPEED, 1.5);
        assert_eq!(closes_alone(&g, fractal, 2.0), Some(false));
        assert_eq!(closes_alone(&g, fractal, 4.0), Some(true));
        loops(&mut g, fractal);
        assert_eq!(
            own_rate(&g, fractal, timing::Axis::X),
            Some(0.5),
            "its pace"
        );

        let simplex = crate::nodes::add_to_graph(&mut g, "simplex", emath::Pos2::ZERO).unwrap();
        assert_eq!(
            closes_alone(&g, simplex, 1.7),
            Some(true),
            "still at Speed 0"
        );
        set(&mut g, simplex, timing::SPEED, 1.0);
        assert_eq!(
            closes_alone(&g, simplex, 8.0),
            Some(false),
            "it never repeats"
        );
        g.get_mut(simplex)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        assert_eq!(
            closes_alone(&g, simplex, 8.0),
            Some(true),
            "4 cells at half a second"
        );

        let lfo = gear(&mut g, "oscillator", None);
        connect(&mut g, (lfo, "output"), (simplex, timing::SPEED));
        assert_eq!(closes_alone(&g, simplex, 8.0), None, "a cable in Speed");

        let clip = crate::nodes::add_to_graph(&mut g, "imagegif", emath::Pos2::ZERO).unwrap();
        assert_eq!(own_rate(&g, clip, timing::Axis::X), None, "a play's length");
        assert_eq!(closes_alone(&g, clip, 1.0), None);
    }

    /// A master two seconds long with 2 : 1 and 1 : 4 below it, the 1 : 4 under a 3 : 1: the
    /// chains multiply, a loop is four cycles, eight seconds, and the caption names the ÷4.
    /// Teeth of 2 : 8 are 1 : 4 to the loop, and 3 : 3 is one to one.
    #[test]
    fn a_loop_of_a_master_is_every_chains_denominator() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        set(&mut g, master, "length", 2.0);
        let double = gear(&mut g, "ratiogear", Some((2.0, 1.0)));
        let triple = gear(&mut g, "ratiogear", Some((3.0, 1.0)));
        let quarter = gear(&mut g, "ratiogear", Some((1.0, 4.0)));
        for (from, to) in [(master, double), (master, triple), (triple, quarter)] {
            connect(&mut g, (from, "cycles"), (to, "clock"));
        }
        let chains = driven(&g, master);
        assert_eq!(
            chains.iter().map(|d| (d.node, d.ratio)).collect::<Vec<_>>(),
            [
                (double, Some(Fraction::new(2, 1))),
                (triple, Some(Fraction::new(3, 1))),
                (quarter, Some(Fraction::new(3, 4))),
            ]
        );
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.why, l.open.len()), (4, Some((quarter, 4)), 0));
        assert_eq!(master_seconds(&g, master), Some(2.0));
        assert_eq!(master_length(&g, master), Some(8.0));
        assert_eq!(
            caption(&g, master),
            format!("loops in 4 cycles · 8.000 s (÷4 on ratiogear{})", quarter.0)
        );
        set(&mut g, quarter, crate::nodes::gear::TEETH_P, 2.0);
        set(&mut g, quarter, crate::nodes::gear::TEETH_Q, 8.0);
        assert_eq!(master_length(&g, master), Some(8.0), "2 : 8 is 1 : 4");
        set(&mut g, quarter, crate::nodes::gear::TEETH_P, 3.0);
        set(&mut g, quarter, crate::nodes::gear::TEETH_Q, 3.0);
        assert_eq!(master_length(&g, master), Some(2.0), "3 : 3 is one to one");
    }

    /// A master with a Ratio Gear at Teeth `p : q` on its Cycles, and a node of kind `slug`
    /// on that gear's `out` through its Time.
    fn driving(
        g: &mut Graph,
        (p, q): (f32, f32),
        slug: &str,
        out: &'static str,
    ) -> (NodeId, NodeId) {
        let master = gear(g, "mastergear", None);
        let ratio = gear(g, "ratiogear", Some((p, q)));
        let node = gear(g, slug, None);
        connect(g, (master, "cycles"), (ratio, "clock"));
        connect(g, (ratio, out), (node, crate::nodes::TIME));
        (master, node)
    }

    /// **A loop counts every node the master drives, at the node's own period.** A Perlin at
    /// Repeat 4 on a ×1 gear, or on the master itself, comes back every four of the master's
    /// cycles, and the caption names it; on that gear's Phase it comes back every cycle, with a
    /// jump; at Repeat Never it never comes back on Cycles.
    #[test]
    fn a_noise_at_repeat_four_on_a_master_loops_in_four() {
        let mut g = Graph::new();
        let (master, perlin) = driving(&mut g, (1.0, 1.0), "perlin", "cycles");
        g.get_mut(perlin)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.why, l.open.len()), (4, Some((perlin, 4)), 0));
        assert_eq!(master_length(&g, master), Some(8.0));
        assert_eq!(
            caption(&g, master),
            format!("loops in 4 cycles · 8.000 s (÷4 on perlin{})", perlin.0)
        );
        let straight = gear(&mut g, "perlin", None);
        g.get_mut(straight)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        connect(&mut g, (master, "cycles"), (straight, crate::nodes::TIME));
        assert_eq!(master_loop(&g, master).cycles, 4, "on the master itself");
        g.get_mut(perlin)
            .unwrap()
            .options
            .insert("repeat", "never".to_string());
        assert_eq!(
            caption(&g, master),
            format!("1 node will not close (perlin{})", perlin.0)
        );

        let mut g = Graph::new();
        let (master, perlin) = driving(&mut g, (1.0, 1.0), "perlin", "wrapped");
        g.get_mut(perlin)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        assert_eq!(
            master_loop(&g, master).cycles,
            1,
            "a Phase comes back every cycle"
        );
    }

    /// **The tunnel comes back every flight, its Helix every quarter of one**: a cycle is one
    /// flight of 64 units, so on a ×32 gear it comes back 32 times a master cycle and a loop
    /// is the master's one; on a ÷2 gear two; the Helix on a ÷8, a quarter of a flight every
    /// two; and with its depth unwrapped it comes back when its input does, which can't be
    /// told.
    #[test]
    fn a_tunnel_comes_back_every_flight() {
        for (teeth, path, want) in [
            ((32.0, 1.0), "sine", 1),
            ((1.0, 2.0), "sine", 2),
            ((1.0, 8.0), "helix", 2),
        ] {
            let mut g = Graph::new();
            let (master, tunnel) = driving(&mut g, teeth, "tunnel3d", "cycles");
            g.get_mut(tunnel)
                .unwrap()
                .options
                .insert("path", path.to_string());
            let l = master_loop(&g, master);
            let why = (want > 1).then_some((tunnel, want as i64));
            assert_eq!((l.cycles, l.why), (want, why), "{path} at {teeth:?}");
        }
        let mut g = Graph::new();
        let (master, tunnel) = driving(&mut g, (32.0, 1.0), "tunnel3d", "cycles");
        g.get_mut(tunnel)
            .unwrap()
            .options
            .insert("wrap", "none".to_string());
        assert_eq!(
            caption(&g, master),
            format!("can't tell when 1 node closes (tunnel3d{})", tunnel.0)
        );
    }

    /// **A sequencer comes back when every lane's figure does**: a Euclidean Rhythm's
    /// defaults meet every bar; a lane of E(3, 5) beside E(4, 16) and E(2, 16), the third lane
    /// full, meets again after forty steps, two and a half bars, so on a master a bar long a
    /// loop is five cycles — not the fifteen the lanes' lengths would give. A Step Sequencer
    /// whose grid is empty comes back every step, so every cycle of its master.
    #[test]
    fn a_sequencer_loops_when_every_lane_does() {
        let mut g = Graph::new();
        let (master, euclid) = driving(&mut g, (1.0, 1.0), "euclideanrhythm", "cycles");
        assert_eq!(master_loop(&g, master).cycles, 1);
        for (key, steps) in [("lane2steps", 5.0), ("lane3steps", 3.0)] {
            set(&mut g, euclid, key, steps);
        }
        assert_eq!(master_loop(&g, master).cycles, 5);
        assert_eq!(master_loop(&g, master).why, Some((euclid, 5)));
        let mut g = Graph::new();
        let (master, _) = driving(&mut g, (1.0, 1.0), "stepsequencer", "cycles");
        assert_eq!(master_loop(&g, master).cycles, 1);
    }

    /// **A step pattern that repeats inside a bar loops in its shortest repeat**: four on the
    /// floor on a gear slowed ÷4 under a master a bar long comes back every cycle of the
    /// master, not every four; on a ÷8 gear, where a quarter of a bar is two cycles, every
    /// two.
    #[test]
    fn four_on_the_floor_on_a_slow_gear_loops_in_its_shortest_repeat() {
        for (divide, want) in [(4.0, 1), (8.0, 2), (2.0, 1)] {
            let mut g = Graph::new();
            let (master, steps) = driving(&mut g, (1.0, divide), "stepsequencer", "cycles");
            g.get_mut(steps).unwrap().values.insert(
                crate::nodes::stepsequencer::PATTERN,
                crate::graph::Value::Cells(vec!["x...x...x...x...".to_string()]),
            );
            let l = master_loop(&g, master);
            assert_eq!(
                (l.cycles, l.open.len(), l.unsure.len()),
                (want, 0, 0),
                "÷{divide}"
            );
        }
    }

    /// **A period under one cycle is a fraction the loop counts exactly**: a node coming back
    /// every half or every tenth of its own cycle, on a gear at ÷4 or 3 : 10, asks for the
    /// master cycles that make the gear's turns a whole number of those periods.
    #[test]
    fn a_fractional_period_closes_where_its_fraction_does() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let rate = Fraction::new(1, 4);
        let half = Some(Fraction::new(1, 2));
        let clock = Clock { rate };
        assert_eq!(period_of(clock, half), Ok(Fraction::new(2, 1)));
        let tenth = Some(Fraction::new(1, 10));
        let slow = Clock {
            rate: Fraction::new(3, 10),
        };
        assert_eq!(period_of(slow, tenth), Ok(Fraction::new(1, 3)));
        assert_eq!(
            period_of(slow, Some(Fraction::new(5, 2))),
            Ok(Fraction::new(25, 3))
        );
        // Through the walk: a Euclidean Rhythm whose every lane is four on the floor comes back
        // every quarter of a bar; on a 3 : 10 gear that is five sixths of a master cycle, so
        // five cycles.
        let ratio = gear(&mut g, "ratiogear", Some((3.0, 10.0)));
        let euclid = gear(&mut g, "euclideanrhythm", None);
        for lane in 0..4 {
            set(
                &mut g,
                euclid,
                crate::nodes::euclideanrhythm::STEPS_KEYS[lane],
                16.0,
            );
            set(
                &mut g,
                euclid,
                crate::nodes::euclideanrhythm::PULSES_KEYS[lane],
                4.0,
            );
        }
        connect(&mut g, (master, "cycles"), (ratio, "clock"));
        connect(&mut g, (ratio, "cycles"), (euclid, crate::nodes::TIME));
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.why), (5, Some((euclid, 5))));
        assert_eq!(master_length(&g, master), Some(10.0));
    }

    /// **A Trigger is followed into what it drives**: into a Step Sequencer's Step, a beat a
    /// step, so a pattern sixteen steps long comes back in sixteen cycles; into a Clock Divider
    /// at ÷3, three; and a gear's Trigger beats at its own whole cycles, so a 3 : 2 gear's
    /// into a Clock Divider at ÷3 comes round every two of the master's.
    #[test]
    fn a_trigger_is_followed_into_what_it_drives() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let steps = crate::nodes::add_to_graph(&mut g, "stepsequencer", emath::Pos2::ZERO).unwrap();
        g.get_mut(steps).unwrap().values.insert(
            crate::nodes::stepsequencer::PATTERN,
            crate::graph::Value::Cells(vec!["x..x.x....x..x..".to_string()]),
        );
        connect(&mut g, (master, "trigger"), (steps, "step"));
        assert_eq!(master_loop(&g, master).cycles, 16);

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let divider = gear(&mut g, "clockdivider", None);
        set(&mut g, divider, "divide", 3.0);
        connect(&mut g, (master, "trigger"), (divider, "input"));
        assert_eq!(master_loop(&g, master).cycles, 3);
        // A divided beat into a sequencer's Step: sixteen steps of three cycles.
        let steps = crate::nodes::add_to_graph(&mut g, "stepsequencer", emath::Pos2::ZERO).unwrap();
        g.get_mut(steps).unwrap().values.insert(
            crate::nodes::stepsequencer::PATTERN,
            crate::graph::Value::Cells(vec!["x.......x.......".to_string()]),
        );
        connect(&mut g, (divider, "trigger"), (steps, "step"));
        assert_eq!(master_loop(&g, master).cycles, 24, "eight steps of three");
        let knob = gear(&mut g, "slew", None);
        connect(&mut g, (knob, "output"), (divider, "divide"));
        assert_eq!(master_loop(&g, master).unsure, [divider]);

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let geared = gear(&mut g, "ratiogear", Some((3.0, 2.0)));
        let divider = gear(&mut g, "clockdivider", None);
        set(&mut g, divider, "divide", 3.0);
        connect(&mut g, (master, "cycles"), (geared, "clock"));
        connect(&mut g, (geared, "trigger"), (divider, "input"));
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.open.len(), l.unsure.len()), (2, 0, 0));
    }

    /// **A Phase in a Ratio Gear's Clock In comes round with it**: the gear is the Phase times
    /// p ÷ q, a number that comes back every cycle of the master, whatever the Teeth.
    #[test]
    fn a_phase_in_clock_in_comes_back_with_it() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let geared = gear(&mut g, "ratiogear", Some((3.0, 2.0)));
        let saw = gear(&mut g, "oscillator", None);
        connect(&mut g, (master, "wrapped"), (geared, "clock"));
        connect(&mut g, (geared, "cycles"), (saw, crate::nodes::TIME));
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.open.len(), l.unsure.len()), (1, 0, 0));
    }

    /// **A Ping-pong comes back every two cycles of its gear**, through a Time and through an
    /// Offset, and on a ×2 gear every cycle of the master.
    #[test]
    fn a_ping_pong_comes_back_every_two_cycles() {
        for (p, key, want) in [
            (1.0, crate::nodes::TIME, 2),
            (1.0, timing::OFFSET, 2),
            (2.0, crate::nodes::TIME, 1),
        ] {
            let mut g = Graph::new();
            let master = gear(&mut g, "mastergear", None);
            let geared = gear(&mut g, "ratiogear", Some((p, 1.0)));
            let osc = gear(&mut g, "oscillator", None);
            connect(&mut g, (master, "cycles"), (geared, "clock"));
            connect(&mut g, (geared, "pingpong"), (osc, key));
            if key == timing::OFFSET {
                // Free at Speed 0: still but for its Offset.
                g.get_mut(osc)
                    .unwrap()
                    .options
                    .insert(timing::MODE.key, timing::FREE.to_string());
                set(&mut g, osc, timing::SPEED, 0.0);
            }
            assert_eq!(master_loop(&g, master).cycles, want, "×{p} into {key}");
        }
    }

    /// **A count into anything but a Time or an Offset cannot be told**, and into an Offset
    /// it adds to where the node is: a Euclidean Rhythm five bars long, still, with the
    /// master's Cycles in its Offset comes back in five.
    #[test]
    fn a_count_in_an_offset_moves_the_node_and_in_a_level_cannot_be_told() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let euclid =
            crate::nodes::add_to_graph(&mut g, "euclideanrhythm", emath::Pos2::ZERO).unwrap();
        for (key, v) in [
            ("lane2steps", 5.0),
            ("lane2pulses", 2.0),
            ("lane1pulses", 3.0),
        ] {
            set(&mut g, euclid, key, v);
        }
        connect(&mut g, (master, "cycles"), (euclid, timing::OFFSET));
        assert_eq!(master_loop(&g, master).cycles, 5);

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let osc = gear(&mut g, "oscillator", None);
        connect(&mut g, (master, "cycles"), (osc, crate::nodes::TIME));
        connect(&mut g, (master, "cycles"), (osc, "amplitude"));
        assert_eq!(master_loop(&g, master).unsure, [osc]);
        assert_eq!(master_length(&g, master), None);
        // The Phase in it comes back every cycle.
        g.disconnect(PortRef::new(osc, "amplitude"));
        connect(&mut g, (master, "wrapped"), (osc, "amplitude"));
        assert_eq!(master_length(&g, master), Some(2.0));
    }

    /// **A node on its own clock beside a master closes with it**: an LFO at Speed 0.3, every
    /// 10/3 s, in the Amplitude of a sine on a two-second master, so the two meet after ten
    /// seconds, five cycles. At a Speed no small fraction of the master, the loop is said not
    /// to close; on a clip's own clock, it cannot be told. A free node nothing downstream of the
    /// master reads is not counted.
    #[test]
    fn an_own_clock_beside_a_master_is_counted() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let osc = gear(&mut g, "oscillator", None);
        connect(&mut g, (master, "cycles"), (osc, crate::nodes::TIME));
        let lfo = crate::nodes::add_to_graph(&mut g, "oscillator", emath::Pos2::ZERO).unwrap();
        set(&mut g, lfo, timing::SPEED, 0.3);
        let apart = crate::nodes::add_to_graph(&mut g, "oscillator", emath::Pos2::ZERO).unwrap();
        set(&mut g, apart, timing::SPEED, 0.37);
        assert_eq!(
            master_length(&g, master),
            Some(2.0),
            "nothing reads the LFO"
        );
        connect(&mut g, (lfo, "output"), (osc, "amplitude"));
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.why), (5, Some((lfo, 5))));
        assert_eq!(master_length(&g, master), Some(10.0));
        set(&mut g, lfo, timing::SPEED, 0.3719);
        assert_eq!(master_loop(&g, master).open, [lfo]);
        let gif = crate::nodes::add_to_graph(&mut g, "imagegif", emath::Pos2::ZERO).unwrap();
        g.disconnect(PortRef::new(osc, "amplitude"));
        connect(&mut g, (gif, "frame"), (osc, "amplitude"));
        assert_eq!(master_loop(&g, master).unsure, [gif]);
    }

    /// **The arithmetic does not overflow**: Ratio Gears at ÷ every prime to 53 need more of
    /// the master's cycles than a `u64` holds, and a chain of eleven 63/64 gears a product an
    /// `i64` does not: both are said not to close.
    #[test]
    fn an_overflowing_loop_does_not_close() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        for q in [
            2.0f32, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0, 41.0, 43.0,
            47.0, 53.0,
        ] {
            let r = gear(&mut g, "ratiogear", Some((1.0, q)));
            connect(&mut g, (master, "cycles"), (r, "clock"));
        }
        assert_eq!(master_loop(&g, master).open.len(), 1);
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let mut from = master;
        for _ in 0..11 {
            let r = gear(&mut g, "ratiogear", Some((63.0, 64.0)));
            connect(&mut g, (from, "cycles"), (r, "clock"));
            from = r;
        }
        assert!(
            caption(&g, master).contains("will not close"),
            "{}",
            caption(&g, master)
        );
    }

    /// **A chain the caption cannot follow cannot be told**: a Multiply between a gear and a
    /// node's Time. A video holding its last frame never comes back.
    #[test]
    fn a_multiply_between_a_gear_and_a_time_cannot_be_told() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let perlin = gear(&mut g, "perlin", None);
        connect(&mut g, (master, "cycles"), (times, "a"));
        connect(&mut g, (times, "output"), (perlin, crate::nodes::TIME));
        let l = master_loop(&g, master);
        assert_eq!((l.open.len(), l.unsure.as_slice()), (0, [times].as_slice()));
        assert_eq!(master_length(&g, master), None);

        let mut g = Graph::new();
        let (master, video) = driving(&mut g, (1.0, 1.0), "video", "cycles");
        assert_eq!(
            master_loop(&g, master).cycles,
            1,
            "a looping clip comes back"
        );
        g.get_mut(video)
            .unwrap()
            .options
            .insert("loop", "hold".to_string());
        assert_eq!(
            caption(&g, master),
            format!("1 node will not close (video{})", video.0)
        );
    }

    /// **Every Time a node has is followed, each at its own pace and period**: Shaky Cam's Y
    /// on a ×½ gear comes back every two of its periods' worth of master cycles, and X left on
    /// ambient time drifts at X's own pace, which the loop counts too; with X on the master as
    /// well, the two meet where both do. Through a Multiply off the master it cannot be told.
    #[test]
    fn a_shaky_cams_time_y_on_a_half_gear_loops_in_two() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        set(&mut g, master, "length", 1.0);
        let half = gear(&mut g, "ratiogear", Some((1.0, 2.0)));
        let shaky = gear(&mut g, "shakycam", None);
        connect(&mut g, (master, "cycles"), (half, "clock"));
        connect(&mut g, (half, "cycles"), (shaky, crate::nodes::TIME_Y));
        let t = g.get(shaky).unwrap().def.timing.unwrap();
        let period = |axis| Fraction::near(timing::period_in(&g, shaky, axis).unwrap()).unwrap();
        let (x, y) = (period(timing::Axis::X), period(timing::Axis::Y));
        let on_half = y.over(Fraction::new(1, 2)).unwrap();
        // X on ambient time, a cycle every 1 ÷ its pace seconds, a master cycle being one.
        let drift = x
            .over(Fraction::near(t.pace_of(timing::Axis::X)).unwrap())
            .unwrap();
        let both = drift.lcm(on_half).unwrap();
        assert_eq!(master_loop(&g, master).cycles, both.p as u64, "X drifts");
        connect(&mut g, (master, "cycles"), (shaky, crate::nodes::TIME));
        let x_and_y = x.lcm(on_half).unwrap();
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.open.len()), (x_and_y.p as u64, 0));
        assert_eq!(l.cycles, 2, "Y's own period, one cycle, twice on the ×½");

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let shaky = gear(&mut g, "shakycam", None);
        connect(&mut g, (master, "cycles"), (times, "a"));
        connect(&mut g, (times, "output"), (shaky, crate::nodes::TIME_Y));
        connect(&mut g, (master, "cycles"), (shaky, crate::nodes::TIME));
        assert_eq!(master_loop(&g, master).unsure, [times]);
    }
}
