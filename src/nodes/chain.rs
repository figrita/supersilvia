// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a loop of a Master Gear needs: the chains of Ratio Gears below it, the nodes those
//! chains drive through their Time, and how many of its cycles bring every one of them back;
//! and whether a node on its own clock comes back over a length. It reads each node's
//! declaration (`nodes::timing`) — rate, pace, period, mode — and nothing about a node kind.
//!
//! A Ratio Gear's ratio is a fraction; a chain of them from a master multiplies them; a loop
//! of `m` cycles of the master closes every gear on it where `m` is a multiple of every
//! chain's denominator — so a ÷4 below asks for four cycles. A node whose Time a gear's
//! Cycles drive comes back where `m` times the chain's product is a whole number of its own
//! period (`Timing::period`): a noise at Repeat 4 on a ×1 asks for four, the tunnel's 64 on
//! a ×32 for two. A gear's Phase in a Time comes back every cycle of that gear, except in a
//! sequencer, which reads it as a count. A node that never repeats, one reached through
//! something the walk cannot read — a Math node between a gear and a Time — and one with a
//! clock cabled into its Speed never close. A node on its own clock — Loop mode with nothing
//! in its Time, or running free with nothing in its Speed — closes over a length `L` where its
//! rate, or its Speed times its pace, times `L` over its period is whole ([`closes_alone`]).
//! The caption under a Master Gear says so, and `examples/loop_gifs` renders a loop that long.
//! See `docs/nodes.md#when-a-loop-closes`.

use crate::graph::{ControlValue, Graph, NodeId};
use crate::nodes::timing;

/// A ratio as a fraction, reduced, its denominator positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    pub p: i64,
    pub q: i64,
}

impl Fraction {
    pub const ONE: Self = Self { p: 1, q: 1 };

    /// The fraction a ratio is, with a denominator up to 64, or `None`.
    pub fn of(r: f64) -> Option<Self> {
        let (p, q) = crate::nodes::gear::ladder::fraction(r, 64)?;
        Some(Self::new(p, q))
    }

    pub fn new(p: i64, q: i64) -> Self {
        let g = crate::nodes::gear::ladder::gcd(p.unsigned_abs(), q.unsigned_abs()).max(1) as i64;
        let s = if q < 0 { -1 } else { 1 };
        Self {
            p: s * p / g,
            q: s * q / g,
        }
    }

    #[must_use]
    pub fn times(self, other: Self) -> Self {
        Self::new(self.p * other.p, self.q * other.q)
    }

    /// Whether the ratio is a whole ×n or ÷n.
    pub fn simple(self) -> bool {
        self.q == 1 || self.p.abs() == 1 || self.p == 0
    }
}

/// The least common multiple of two positive counts.
pub fn lcm(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        return a.max(b);
    }
    a / crate::nodes::gear::ladder::gcd(a, b) * b
}

/// A Ratio Gear's ratio as a fraction, where its control is one and nothing is cabled into it.
fn ratio_of(graph: &Graph, id: NodeId) -> Option<Fraction> {
    let node = graph.get(id)?;
    if graph
        .source_of(crate::graph::PortRef::new(id, "ratio"))
        .is_some()
    {
        return None;
    }
    match node.controls.get("ratio") {
        Some(ControlValue::Float(v)) => Fraction::of(f64::from(*v)),
        _ => None,
    }
}

/// One Ratio Gear a Master Gear drives, through a chain of them: the product of every ratio
/// from the master down to it, or `None` where one on the way is not a fraction or has a cable
/// in its Ratio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driven {
    pub node: NodeId,
    pub ratio: Option<Fraction>,
}

/// Every Ratio Gear `master` drives through its Cycles or its Phase into a Clock In, and on
/// down through theirs, each with the product of the ratios on its way. A gear a cycle of
/// ratio gears reaches twice is listed once, at the first product found.
pub fn driven(graph: &Graph, master: NodeId) -> Vec<Driven> {
    let mut out: Vec<Driven> = Vec::new();
    let mut stack = vec![(master, Some(Fraction::ONE))];
    while let Some((from, product)) = stack.pop() {
        for key in ["cycles", "wrapped"] {
            for to in graph.targets_of(crate::graph::PortRef::new(from, key)) {
                let Some(node) = graph.get(to.node) else {
                    continue;
                };
                if node.def.slug != crate::nodes::gear::RATIO.slug
                    || to.key != "clock"
                    || out.iter().any(|d| d.node == to.node)
                {
                    continue;
                }
                let ratio = product
                    .zip(ratio_of(graph, to.node))
                    .map(|(a, b)| a.times(b));
                out.push(Driven {
                    node: to.node,
                    ratio,
                });
                stack.push((to.node, ratio));
            }
        }
    }
    out.sort_by_key(|d| d.node);
    out
}

/// What a loop of a Master Gear needs: how many of its cycles bring every gear and every node
/// it drives back to where it started, the one that asks for the most, and what never closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterLoop {
    pub cycles: u64,
    /// The gear or node that asks for the most cycles, and how many, where one asks for more
    /// than one.
    pub why: Option<(NodeId, i64)>,
    /// Gears whose ratio is not a fraction or has a cable in it, nodes that never repeat, and
    /// nodes whose Time is reached through something that is not a gear.
    pub open: Vec<NodeId>,
}

/// [`MasterLoop`] for one Master Gear.
pub fn master_loop(graph: &Graph, master: NodeId) -> MasterLoop {
    let mut l = MasterLoop {
        cycles: 1,
        why: None,
        open: Vec::new(),
    };
    // `node` comes back after `f`'s denominator of the master's cycles, or never.
    let mut ask = |node: NodeId, f: Option<Fraction>| match f {
        Some(f) => {
            l.cycles = lcm(l.cycles, f.q.unsigned_abs());
            if f.q > 1 && l.why.is_none_or(|(_, q)| f.q > q) {
                l.why = Some((node, f.q));
            }
        }
        None => {
            if !l.open.contains(&node) {
                l.open.push(node);
            }
        }
    };
    let gears = driven(graph, master);
    for d in &gears {
        ask(d.node, d.ratio);
    }
    let clocks = std::iter::once((master, Some(Fraction::ONE)))
        .chain(gears.iter().map(|d| (d.node, d.ratio)));
    for (clock, product) in clocks {
        for key in ["cycles", "wrapped"] {
            for to in graph.targets_of(crate::graph::PortRef::new(clock, key)) {
                let Some(node) = graph.get(to.node) else {
                    continue;
                };
                if node.def.slug == crate::nodes::gear::RATIO.slug && to.key == "clock" {
                    // A gear on a gear: `driven`'s.
                    continue;
                }
                match node.def.timing {
                    Some(timing) if timing::is_time(to.key) => {
                        let period = if key == "wrapped"
                            && !crate::nodes::sequencer::reads_unwrapped(node.def)
                        {
                            Some(1)
                        } else {
                            (timing.period)(node).and_then(whole)
                        };
                        ask(
                            to.node,
                            product
                                .zip(period)
                                .map(|(r, p)| Fraction::new(r.p, r.q * p)),
                        );
                    }
                    // A clock in a Speed is a rate that keeps changing: it never closes.
                    Some(_) if timing::is_speed(to.key) => ask(to.node, None),
                    _ => {
                        for reached in unfollowed(graph, to.node) {
                            ask(reached, None);
                        }
                    }
                }
            }
        }
    }
    l.open.sort();
    l
}

/// A period that is a whole number of a node's own units, one or more.
fn whole(period: f64) -> Option<i64> {
    let n = period.round();
    ((period - n).abs() < 1e-9 && (1.0..1e9).contains(&n)).then_some(n as i64)
}

/// How many of its own cycles a second `node`'s Time runs at on its own clock, on one axis:
/// in Loop mode with nothing in that Time, its pace; running free with nothing in that
/// Speed, its Speed's knob times its pace (`nodes::timing`). `None` where a cable drives it. A
/// clip's pace is one play over its length, which the graph does not hold, so a clip reads as
/// a play a second here.
pub fn own_rate(graph: &Graph, node: NodeId, axis: timing::Axis) -> Option<f64> {
    let n = graph.get(node)?;
    let t = n.def.timing?;
    if timing::runs_free(n) {
        if graph
            .source_of(crate::graph::PortRef::new(node, axis.speed))
            .is_some()
        {
            return None;
        }
        match n.controls.get(axis.speed) {
            Some(ControlValue::Float(speed)) => Some(f64::from(*speed) * t.pace),
            _ => None,
        }
    } else {
        graph
            .source_of(crate::graph::PortRef::new(node, axis.time))
            .is_none()
            .then_some(t.pace)
    }
}

/// Whether `node`, on its own clock, comes back after `seconds`: on every axis
/// `own_rate × seconds ÷ period` is whole, which a node standing still always is and a moving
/// picture that never repeats never is. `None` for a node that does not move with time or that
/// a cable drives, whose loop is its chain's.
pub fn closes_alone(graph: &Graph, node: NodeId, seconds: f64) -> Option<bool> {
    let n = graph.get(node)?;
    let t = n.def.timing?;
    let mut closes = true;
    for axis in t.axes() {
        let rate = own_rate(graph, node, *axis)?;
        if rate == 0.0 {
            continue;
        }
        closes &= (t.period)(n).is_some_and(|p| {
            let turns = rate * seconds / p;
            (turns - turns.round()).abs() < 1e-6
        });
    }
    Some(closes)
}

/// The nodes whose Time — or a Ratio Gear's Clock In — a number reaches through `from` and on
/// through whatever `from` feeds, stopping at any node that moves with time or is a gear: a
/// clock bent on the way, which the caption cannot follow.
fn unfollowed(graph: &Graph, from: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        let Some(node) = graph.get(id) else {
            continue;
        };
        if node.def.timing.is_some() || node.def.category == crate::nodes::Category::Gear {
            continue;
        }
        for output in node.def.outputs {
            for to in graph.targets_of(crate::graph::PortRef::new(id, output.key)) {
                let Some(target) = graph.get(to.node) else {
                    continue;
                };
                let clock = (timing::is_time(to.key)
                    && (target.def.timing.is_some()
                        || target.def.slug == crate::nodes::gear::RATIO.slug))
                    || (timing::is_speed(to.key) && target.def.timing.is_some());
                if clock {
                    if !out.contains(&to.node) {
                        out.push(to.node);
                    }
                } else if seen.insert(to.node) {
                    stack.push(to.node);
                }
            }
        }
    }
    out
}

/// How long one cycle of a Master Gear is, in seconds, where its Length is a knob.
pub fn master_seconds(graph: &Graph, master: NodeId) -> Option<f64> {
    let node = graph.get(master)?;
    if graph
        .source_of(crate::graph::PortRef::new(master, "length"))
        .is_some()
    {
        return None;
    }
    match node.controls.get("length") {
        Some(ControlValue::Float(v)) => Some(crate::nodes::gear::seconds_a_cycle(f64::from(*v))),
        _ => None,
    }
}

/// How long a loop of a Master Gear is, in seconds: its cycles a loop times one cycle's
/// length. `None` where a chain below it never closes or its length is cabled.
pub fn master_length(graph: &Graph, master: NodeId) -> Option<f64> {
    let l = master_loop(graph, master);
    if !l.open.is_empty() {
        return None;
    }
    Some(master_seconds(graph, master)? * l.cycles as f64)
}

/// The line under every Master Gear's picture that `shown` names.
pub fn captions(
    graph: &Graph,
    shown: impl Fn(NodeId) -> bool,
) -> std::collections::HashMap<NodeId, String> {
    graph
        .iter()
        .filter(|(id, n)| n.def.slug == crate::nodes::gear::MASTER.slug && shown(*id))
        .map(|(id, _)| (id, caption(graph, id)))
        .collect()
}

/// The line under a Master Gear's picture: what a loop of it needs. "loops in 1 cycle ·
/// 2.000 s", "loops in 4 cycles · 8.000 s (÷4 on ratiogear12)", "… (÷4 on perlin5)", or "2
/// nodes will not close".
pub fn caption(graph: &Graph, master: NodeId) -> String {
    let l = master_loop(graph, master);
    if !l.open.is_empty() {
        let n = l.open.len();
        return format!("{n} node{} will not close", if n == 1 { "" } else { "s" });
    }
    let cycles = if l.cycles == 1 {
        "1 cycle".to_string()
    } else {
        format!("{} cycles", l.cycles)
    };
    let seconds = master_seconds(graph, master)
        .map_or_else(String::new, |s| format!(" · {:.3} s", s * l.cycles as f64));
    let why = l.why.map_or_else(String::new, |(node, q)| {
        let slug = graph.get(node).map_or("node", |n| n.def.slug);
        format!(" (÷{q} on {slug}{})", node.0)
    });
    format!("loops in {cycles}{seconds}{why}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::PortRef;

    /// A node of kind `slug`, at `ratio` where it is a Ratio Gear, and on a clock — Loop
    /// mode — where it moves with time, so a gear can be cabled into its Time.
    fn gear(g: &mut Graph, slug: &str, ratio: Option<f32>) -> NodeId {
        let id = crate::nodes::add_to_graph(g, slug, emath::Pos2::ZERO).unwrap();
        loops(g, id);
        if let Some(r) = ratio {
            g.get_mut(id)
                .unwrap()
                .controls
                .insert("ratio", ControlValue::Float(r));
        }
        id
    }

    fn loops(g: &mut Graph, id: NodeId) {
        let n = g.get_mut(id).unwrap();
        if n.def.timing.is_some() {
            n.options.insert(timing::MODE.key, timing::LOOP.to_string());
        }
    }

    /// **A clock cabled into a Speed never closes**: a gear's Cycles into a free-running
    /// node's Speed is a rate that keeps growing, and through a Multiply the same.
    #[test]
    fn a_clock_in_a_speed_will_not_close() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        g.connect(
            PortRef::new(master, "cycles"),
            PortRef::new(perlin, timing::SPEED),
        )
        .unwrap();
        assert_eq!(master_loop(&g, master).open, [perlin]);
        assert_eq!(caption(&g, master), "1 node will not close");

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        g.connect(PortRef::new(master, "cycles"), PortRef::new(times, "a"))
            .unwrap();
        g.connect(
            PortRef::new(times, "output"),
            PortRef::new(perlin, timing::SPEED),
        )
        .unwrap();
        assert_eq!(master_loop(&g, master).open, [perlin]);
    }

    /// **A node on its own clock closes where its rate times the length over its period is
    /// whole**: running free, its Speed times its pace — Mandelbrot at Speed 1, half a drift a
    /// second, closes over 2 s and not 3 s, and at Speed 1.5 over 4 s — and in Loop mode with
    /// nothing in its Time, its pace. A node standing still closes on anything, a noise
    /// that never repeats closes on nothing, and a cable in its Speed leaves it to its chain.
    #[test]
    fn a_node_on_its_own_clock_closes_where_its_rate_comes_round() {
        let mut g = Graph::new();
        let fractal = crate::nodes::add_to_graph(&mut g, "mandelbrot", emath::Pos2::ZERO).unwrap();
        assert_eq!(own_rate(&g, fractal, timing::Axis::X), Some(0.5));
        assert_eq!(closes_alone(&g, fractal, 2.0), Some(true));
        assert_eq!(closes_alone(&g, fractal, 3.0), Some(false));
        g.get_mut(fractal)
            .unwrap()
            .controls
            .insert(timing::SPEED, ControlValue::Float(1.5));
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
        g.get_mut(simplex)
            .unwrap()
            .controls
            .insert(timing::SPEED, ControlValue::Float(1.0));
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
        g.connect(
            PortRef::new(lfo, "output"),
            PortRef::new(simplex, timing::SPEED),
        )
        .unwrap();
        assert_eq!(closes_alone(&g, simplex, 8.0), None, "a cable in Speed");
    }

    /// A master two seconds long with ×2 and ÷4 below it, the ÷4 under a ×3: the chains
    /// multiply, a loop is four cycles, eight seconds, and the caption names the ÷4.
    #[test]
    fn a_loop_of_a_master_is_every_chains_denominator() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        g.get_mut(master)
            .unwrap()
            .controls
            .insert("length", ControlValue::Float(2.0));
        let double = gear(&mut g, "ratiogear", Some(2.0));
        let triple = gear(&mut g, "ratiogear", Some(3.0));
        let quarter = gear(&mut g, "ratiogear", Some(0.25));
        for (from, to) in [(master, double), (master, triple), (triple, quarter)] {
            g.connect(PortRef::new(from, "cycles"), PortRef::new(to, "clock"))
                .unwrap();
        }
        let chains = driven(&g, master);
        assert_eq!(
            chains.iter().map(|d| (d.node, d.ratio)).collect::<Vec<_>>(),
            [
                (double, Fraction::of(2.0)),
                (triple, Fraction::of(3.0)),
                (quarter, Fraction::of(0.75)),
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
        // A cable into a ratio leaves its chain open.
        let knob = gear(&mut g, "slew", None);
        g.connect(PortRef::new(knob, "output"), PortRef::new(quarter, "ratio"))
            .unwrap();
        assert_eq!(caption(&g, master), "1 node will not close");
        assert_eq!(master_length(&g, master), None);
    }

    /// A master of `length` seconds with a Ratio Gear at `ratio` on its Cycles, and a node
    /// of kind `slug` on that gear's `out` through its Time.
    fn driving(g: &mut Graph, ratio: f32, slug: &str, out: &'static str) -> (NodeId, NodeId) {
        let master = gear(g, "mastergear", None);
        let ratio = gear(g, "ratiogear", Some(ratio));
        let node = gear(g, slug, None);
        g.connect(PortRef::new(master, "cycles"), PortRef::new(ratio, "clock"))
            .unwrap();
        g.connect(
            PortRef::new(ratio, out),
            PortRef::new(node, crate::nodes::TIME),
        )
        .unwrap();
        (master, node)
    }

    /// **A loop counts every node the master drives, at the node's own period.** A Perlin at
    /// Repeat 4 on a ×1 gear, or on the master itself, comes back every four of the master's
    /// cycles, and the caption names it; on that gear's Phase it comes back every cycle, with a jump; at Repeat Never
    /// it never comes back on Cycles.
    #[test]
    fn a_noise_at_repeat_four_on_a_master_loops_in_four() {
        let mut g = Graph::new();
        let (master, perlin) = driving(&mut g, 1.0, "perlin", "cycles");
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
        g.connect(
            PortRef::new(master, "cycles"),
            PortRef::new(straight, crate::nodes::TIME),
        )
        .unwrap();
        assert_eq!(master_loop(&g, master).cycles, 4, "on the master itself");
        g.get_mut(perlin)
            .unwrap()
            .options
            .insert("repeat", "never".to_string());
        assert_eq!(caption(&g, master), "1 node will not close");

        let mut g = Graph::new();
        let (master, perlin) = driving(&mut g, 1.0, "perlin", "wrapped");
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

    /// **The tunnel flies 64 units before it comes back**, so on a ×32 gear it loops in two of
    /// the master's cycles, and with its depth unwrapped it never does.
    #[test]
    fn a_tunnel_on_times_thirty_two_loops_in_two() {
        let mut g = Graph::new();
        let (master, tunnel) = driving(&mut g, 32.0, "tunnel3d", "cycles");
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.why), (2, Some((tunnel, 2))));
        g.get_mut(tunnel)
            .unwrap()
            .options
            .insert("wrap", "none".to_string());
        assert_eq!(caption(&g, master), "1 node will not close");
    }

    /// **A sequencer comes back when every lane does**: a Euclidean Rhythm with a lane of 5
    /// steps and one of 3 against sixteen a bar meets again after 240 steps, fifteen bars, so
    /// on a master a bar long a loop is fifteen cycles; at its sixteen-step lanes, and a Step
    /// Sequencer's, it loops every bar.
    #[test]
    fn a_sequencer_loops_when_every_lane_does() {
        let mut g = Graph::new();
        let (master, euclid) = driving(&mut g, 1.0, "euclideanrhythm", "cycles");
        assert_eq!(master_loop(&g, master).cycles, 1);
        for (key, steps) in [("lane2steps", 5.0), ("lane3steps", 3.0)] {
            g.get_mut(euclid)
                .unwrap()
                .controls
                .insert(key, ControlValue::Float(steps));
        }
        assert_eq!(master_loop(&g, master).cycles, 15);
        assert_eq!(master_loop(&g, master).why, Some((euclid, 15)));
        let mut g = Graph::new();
        let (master, _) = driving(&mut g, 1.0, "stepsequencer", "cycles");
        assert_eq!(master_loop(&g, master).cycles, 1);
    }

    /// **A chain the caption cannot follow will not close**: a Multiply between a gear and a
    /// node's Time, or a video holding its last frame.
    #[test]
    fn a_multiply_between_a_gear_and_a_time_will_not_close() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let perlin = gear(&mut g, "perlin", None);
        g.connect(PortRef::new(master, "cycles"), PortRef::new(times, "a"))
            .unwrap();
        g.connect(
            PortRef::new(times, "output"),
            PortRef::new(perlin, crate::nodes::TIME),
        )
        .unwrap();
        let l = master_loop(&g, master);
        assert_eq!(l.open, [perlin]);
        assert_eq!(caption(&g, master), "1 node will not close");
        assert_eq!(master_length(&g, master), None);

        let mut g = Graph::new();
        let (master, video) = driving(&mut g, 1.0, "video", "cycles");
        assert_eq!(
            master_loop(&g, master).cycles,
            1,
            "a looping clip comes back"
        );
        g.get_mut(video)
            .unwrap()
            .options
            .insert("loop", "hold".to_string());
        assert_eq!(caption(&g, master), "1 node will not close");
    }

    /// **Every Time a node has is followed**, Shaky Cam's Time Y as well as its Time X: on a
    /// ×½ gear under a master a second long it asks for two of the master's cycles, and
    /// through a Multiply off the master it will not close.
    #[test]
    fn a_shaky_cams_time_y_on_a_half_gear_loops_in_two() {
        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        g.get_mut(master)
            .unwrap()
            .controls
            .insert("length", ControlValue::Float(1.0));
        let half = gear(&mut g, "ratiogear", Some(0.5));
        let shaky = gear(&mut g, "shakycam", None);
        g.connect(PortRef::new(master, "cycles"), PortRef::new(half, "clock"))
            .unwrap();
        g.connect(
            PortRef::new(half, "cycles"),
            PortRef::new(shaky, crate::nodes::TIME_Y),
        )
        .unwrap();
        let l = master_loop(&g, master);
        assert_eq!((l.cycles, l.open.len()), (2, 0));
        assert!(
            caption(&g, master).starts_with("loops in 2 cycles · 2.000 s"),
            "{}",
            caption(&g, master)
        );

        let mut g = Graph::new();
        let master = gear(&mut g, "mastergear", None);
        let times = gear(&mut g, "multiply", None);
        let shaky = gear(&mut g, "shakycam", None);
        g.connect(PortRef::new(master, "cycles"), PortRef::new(times, "a"))
            .unwrap();
        g.connect(
            PortRef::new(times, "output"),
            PortRef::new(shaky, crate::nodes::TIME_Y),
        )
        .unwrap();
        assert_eq!(master_loop(&g, master).open, [shaky]);
        assert_eq!(caption(&g, master), "1 node will not close");
    }
}
