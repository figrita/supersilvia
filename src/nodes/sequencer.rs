// SPDX-License-Identifier: AGPL-3.0-or-later

//! The clock both step sequencers run behind: four gate lanes on sixteen steps a bar, read off
//! **Time and Offset** in bars, with silvia's Step and Gate. What a lane plays at a step is the
//! node's own question, asked through the `pulse` a [`Transport::tick`] is handed —
//! `euclideanrhythm` answers from Bjorklund's figure, `stepsequencer` from the cells a hand lit.
//!
//! **A step is a crossing of `floor(16 × cycle)`**, where the node is in bars with its Offset added
//! (`TickContext::cycle`). A new one stands still — Speed 0, a sequencer starts stopped, as
//! silvia's does — and Speed 1 is a bar every two seconds; in Loop mode a Master Gear a bar long
//! cabled into Time is the tempo, and its Hold and Reset are the play and the reset silvia's
//! Start/Stop and Reset were. Nothing is integrated here: the node remembers only last tick's
//! reading, to see what it crossed, and a cabled Time read as a count whole or unwrapped where its
//! source declares its wrap, so a gear's Phase passing one is a frame's motion and not a jump. **A
//! reading that moves more than a bar in one tick, or onto another clock as Time's cable is moved,
//! is a jump** and fires nothing, and so is the first reading and one that moves backwards on a
//! clock; running free, a negative Speed plays the steps backwards, each entered at its top
//! boundary. A step it lands exactly on, with a gear driving Time or a Speed moving it and the show
//! playing, is played at once, so a render's first frame is its bar's downbeat; any other step it
//! lands in is played on the next tick where it landed no further past it than that tick moves, so
//! a gear's Reset lands on the downbeat. Each lane reads the absolute step modulo its own length,
//! so a lane of five against sixteen keeps its phase.
//!
//! **`gate` closes each lane's own down**: a lane's gate is driven straight off whether that
//! lane is a pulse at each step boundary, `mastergear`'s down-then-up-after-a-fraction shape
//! repeated per lane, so two adjacent pulses at a long gate share one held level. A Time that
//! stands still — a gear held, the show paused — closes whatever a step opened as the Time
//! passed it, so nothing downstream is left held by a clock that is not moving; what a landing
//! opened on the step it stands on stays open until the Time moves on, so a render's Hold
//! warm-up, which lands on step 0 and stands there, keeps step 0's gate open into its first
//! kept frame.
//!
//! **`step` slaves the sequencer to an event clock**, silvia's `_manualStep`: each down
//! advances the grid by exactly one and holds every lane exactly as long as that step is a
//! pulse — the one stateful path, since an event clock (a tap, a threshold) is not a gear.
//! While something is cabled into Step the sequencer ignores its Time.
//!
//! **A sequencer comes back after the shortest run of steps every lane repeats in** ([`bars`]):
//! each lane's figure read round its own length, the least shift that leaves it as it is
//! ([`repeat`]) — four on the floor every four steps, a silent or a full lane every step — and
//! the least common multiple of the four, in bars. So a pattern that repeats inside a bar says
//! so, and a loop of it on a slow gear is as short as the pattern is (`nodes::chain`).

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::phasor::{self, REACH};
use crate::nodes::{Control, Event, Gate, InputDef, OutputDef, OutputKind, TickContext, Timing};

/// The time rows in bars, then Step as a button with a port, and Gate: silvia's s-number, as
/// a knob a cable can drive.
pub const INPUTS: &[InputDef] = crate::nodes::timing::inputs![
    TIMING;
    InputDef {
        key: "step",
        label: "Step",
        ty: Action,
        control: Control::Press,
    },
    InputDef {
        key: "gateLength",
        label: "Gate",
        ty: UniformNumber,
        control: Control::num(0.5, 0.1, 1.0, 0.05, ""),
    },
];

/// A sequencer's timing: a bar a cycle, standing still when new, and a bar every two
/// seconds — 120 beats a minute. Each node's period is its own, the [`bars`] its lanes come
/// back in.
pub const TIMING: Timing = Timing::periodic(0.5).still();

/// The key of **Step**, the action input that advances the grid by one step a down.
pub const STEP: &str = "step";

/// Whether a node runs behind this clock: it reads a cabled Time unwrapped where its source
/// declares the wrap, so a gear's Phase in its Time is a count and not a fraction that jumps
/// back every cycle, and while [`STEP`] is cabled it advances one step a down on it and reads
/// nothing else of time.
pub fn is_sequencer(def: &crate::nodes::NodeDef) -> bool {
    def.slug == crate::nodes::stepsequencer::DEF.slug
        || def.slug == crate::nodes::euclideanrhythm::DEF.slug
}

/// The least shift that leaves a lane of `steps` steps as it is, read round its own length:
/// bit `i` of `figure` lit where step `i` is. A divisor of `steps`; one for a lane all silent or
/// all lit.
pub fn repeat(figure: u64, steps: u32) -> u32 {
    let steps = steps.clamp(1, 64);
    let lit = |i: u32| figure >> (i % steps) & 1 == 1;
    (1..=steps)
        .filter(|d| steps.is_multiple_of(*d))
        .find(|&d| (0..steps).all(|i| lit(i) == lit(i + d)))
        .unwrap_or(steps)
}

/// How many bars four lanes come back in: the least common multiple of each lane's
/// [`repeat`], `(figure, steps)` a lane, over the bar's sixteen steps — a fraction of a bar
/// where every lane repeats inside one.
pub fn bars(lanes: impl IntoIterator<Item = (u64, u32)>) -> f64 {
    let steps = lanes
        .into_iter()
        .map(|(figure, steps)| i64::from(repeat(figure, steps)))
        .fold(1, lcm);
    steps as f64 / BAR
}

/// The four lanes, each an action output holding its gate while its step is lit.
pub const OUTPUTS: &[OutputDef] = &[
    OutputDef {
        key: "lane1",
        label: "Lane 1",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "lane2",
        label: "Lane 2",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "lane3",
        label: "Lane 3",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    },
    OutputDef {
        key: "lane4",
        label: "Lane 4",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    },
];

/// How many lanes a sequencer runs.
pub const LANES: usize = 4;

const LANE_KEYS: [&str; LANES] = ["lane1", "lane2", "lane3", "lane4"];

/// The steps in a bar, sixteenth notes: what one cycle of Time is.
pub const STEPS_A_BAR: i64 = 16;
const BAR: f64 = STEPS_A_BAR as f64;

/// The most events one frame may produce, across all four lanes — `mastergear`'s own bound,
/// for the same reason: a `tick` never waits and never allocates without one.
const MAX_EVENTS_PER_FRAME: u32 = 64;

/// The least common multiple of two lane lengths, for [`Transport::playhead`] and [`bars`].
pub fn lcm(a: i64, b: i64) -> i64 {
    let (mut x, mut y) = (a.max(1), b.max(1));
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a.max(1) / x * b.max(1)
}

/// Where a sequencer is and which of its gates are open: the runtime half, never saved.
#[derive(Default)]
pub struct Transport {
    /// Last tick's reading, in steps: what the next one crosses from. `None` until the first.
    last: Option<f64>,
    /// What a cabled Time read on the last tick, as its source publishes it. `None` while
    /// Time is unplugged.
    raw: Option<f64>,
    /// The output cabled into Time on the last tick: a cable moved onto another clock is a
    /// landing on that clock, not a motion of this one.
    source: Option<crate::graph::PortRef>,
    /// Time's cable was plugged in, let go or moved this tick.
    moved: bool,
    /// A cabled Time unwrapped across its source's wraps, in bars: the reading steps are
    /// counted on.
    time: f64,
    /// The reading jumped, and the step it landed in is still to be played on the next tick.
    landed: bool,
    /// A landing played the step it stood on, and the Time has not moved since: what it opened
    /// stays open while the Time stands there, as a render's Hold warm-up does.
    standing: bool,
    /// The step last played, absolute, for the grid to light. `None` before anything has
    /// stepped — silvia's `currentStep = -1`, which lights nothing.
    current: Option<i64>,
    step: Gate,
    lanes: [Gate; LANES],
}

impl Transport {
    /// One line for the Debug readout.
    pub fn debug(&self) -> String {
        match self.current {
            Some(step) => format!("step {step}"),
            None => "at rest".to_string(),
        }
    }

    /// Which cell the grid on the node lights: the absolute step, not a fraction of a clip,
    /// taken modulo the largest multiple of `period` an `f32` holds every whole number below,
    /// so a lane whose length divides `period` reads its own step exactly however long the
    /// show has run.
    pub fn playhead(&self, period: i64) -> Option<f32> {
        const EXACT: i64 = 1 << f32::MANTISSA_DIGITS;
        let period = period.clamp(1, EXACT);
        self.current
            .map(|step| step.rem_euclid(period * (EXACT / period)) as f32)
    }

    /// One frame. `pulse(lane, step)` says whether a lane is lit at an absolute step, which
    /// the node answers from its own pattern.
    pub fn tick(
        &mut self,
        id: NodeId,
        ctx: &mut TickContext<'_>,
        pulse: impl Fn(usize, i64) -> bool,
    ) {
        // A manual step advances the grid by exactly one, whatever drives it — a hand, or an
        // event clock's own cable — and holds each lane exactly as long as that one step is a
        // pulse.
        let mut step_events = ctx.edges(id, "step");
        if let Some(edge) = self.step.set(ctx.pressed(id, "step")) {
            step_events.push(Event::now(edge));
        }
        for event in step_events {
            if !event.is_down() {
                continue;
            }
            let step_index = self.current.map_or(0, |c| c + 1);
            self.current = Some(step_index);
            for (lane_idx, key) in LANE_KEYS.iter().enumerate() {
                let active = pulse(lane_idx, step_index);
                if let Some(edge) = self.lanes[lane_idx].set(active) {
                    ctx.fire_at(id, key, edge, event.at);
                }
            }
        }
        // An event clock in Step is the whole of the clock.
        if ctx.connected(id, "step") {
            self.last = None;
            return;
        }

        let now = BAR * self.time(id, ctx);
        let Some(p0) = self.last.replace(now) else {
            // The first reading, after a reset among them, is a landing too.
            self.land(id, ctx, now, &pulse);
            return;
        };
        let p1 = now;
        // Running free, a negative Speed plays the steps backwards; on a clock, a reading
        // that goes back is a jump.
        let free = ctx.runs_free(id);
        if ctx.time.jumped || self.moved || (!free && p1 < p0 - REACH) || (p1 - p0).abs() > BAR {
            // A jump plays nothing on the way, and closes what it left open.
            self.close(id, ctx);
            self.land(id, ctx, now, &pulse);
            return;
        }
        if (p1 - p0).abs() <= REACH {
            // Standing still — at rest, held, paused — says nothing, and holds nothing held
            // but what a landing on this very step opened, which waits for the Time to move.
            if !self.standing {
                self.close(id, ctx);
            }
            return;
        }
        self.standing = false;
        let landed = std::mem::take(&mut self.landed);
        let gate = f64::from(ctx.input(id, "gateLength").clamp(0.001, 1.0));
        let dt = ctx.dt;
        let moment = |p: f64| (((p - p0) / (p1 - p0)).clamp(0.0, 1.0) as f32) * dt;
        if p1 < p0 {
            self.walk_back(id, ctx, (p0, p1), gate, &moment, &pulse);
            return;
        }

        let mut fired = 0u32;
        let mut boundary = p0.floor();
        while boundary <= p1 + REACH && fired < MAX_EVENTS_PER_FRAME {
            let step_index = boundary as i64;
            // Crossed, or the step a jump landed in, where it landed no further past it than
            // a tick moves: a gear's Reset lands on its downbeat mid-frame.
            let arrives = boundary > p0 + REACH
                || (landed && boundary <= p0 + REACH && p0 - boundary <= p1 - p0 + REACH);
            if arrives {
                self.current = Some(step_index);
            }
            for (lane_idx, key) in LANE_KEYS.iter().enumerate() {
                let want_down = pulse(lane_idx, step_index);
                for (p, level, due) in [
                    (boundary, want_down, arrives),
                    (boundary + gate, false, boundary + gate > p0 + REACH),
                ] {
                    if due
                        && p <= p1 + REACH
                        && let Some(edge) = self.lanes[lane_idx].set(level)
                    {
                        ctx.fire_at(id, key, edge, moment(p));
                        fired += 1;
                    }
                }
            }
            boundary += 1.0;
        }
    }

    /// A frame run backwards, from `p0` down to `p1`, in steps: each boundary `b` crossed
    /// going down enters step `b − 1`, whose lanes open there and close `gate` of a step
    /// further down — the steps in reverse order, each held as long as forwards.
    fn walk_back(
        &mut self,
        id: NodeId,
        ctx: &mut TickContext<'_>,
        (p0, p1): (f64, f64),
        gate: f64,
        moment: &impl Fn(f64) -> f32,
        pulse: &impl Fn(usize, i64) -> bool,
    ) {
        let mut fired = 0u32;
        let mut boundary = (p0 - REACH).ceil();
        while boundary >= p1 - REACH && fired < MAX_EVENTS_PER_FRAME {
            let step_index = boundary as i64 - 1;
            let arrives = boundary < p0 - REACH;
            if arrives {
                self.current = Some(step_index);
            }
            for (lane_idx, key) in LANE_KEYS.iter().enumerate() {
                let want_down = pulse(lane_idx, step_index);
                for (p, level, due) in [
                    (boundary, want_down, arrives),
                    (boundary - gate, false, boundary - gate < p0 - REACH),
                ] {
                    if due
                        && p >= p1 - REACH
                        && let Some(edge) = self.lanes[lane_idx].set(level)
                    {
                        ctx.fire_at(id, key, edge, moment(p));
                        fired += 1;
                    }
                }
            }
            boundary -= 1.0;
        }
    }

    /// Where the node is in bars, its Offset added ([`TickContext::cycle`]). Running free, or
    /// unplugged, it is that reading; cabled, it is what arrives read as a Ratio Gear's Clock
    /// In is — a count whole, anything else unwrapped where its source declares its wrap
    /// (`TickContext::wraps_at`), a gear's Phase at one — so a wrap is one frame's motion,
    /// and a jump, or a cable plugged in, let go or moved onto another output, puts it back
    /// at the reading as published, and is a jump.
    fn time(&mut self, id: NodeId, ctx: &mut TickContext<'_>) -> f64 {
        let source = ctx.source(id, crate::nodes::TIME);
        self.moved = self.last.is_some() && source != self.source;
        self.source = source;
        let raw = ctx.cycle(id);
        if ctx.runs_free(id) || !ctx.connected(id, crate::nodes::TIME) {
            self.raw = None;
            return raw;
        }
        self.time = match self.raw {
            Some(was) if !ctx.time.jumped && !self.moved => {
                self.time + phasor::unwrap_at(was, raw, ctx.wraps_at(id, crate::nodes::TIME))
            }
            _ => raw,
        };
        self.raw = Some(raw);
        self.time
    }

    /// The reading jumped, or is the first, to `at`. A step it stands on has been crossed by
    /// nobody, so where a gear drives Time and the show plays it is played here, at the top of
    /// the frame: a render's first frame is its bar's downbeat, and what it opens stays open
    /// until the Time moves on, through a Hold warm-up's frames at zero. Anywhere else it is left to
    /// the next tick, which plays the step it landed in where that tick moves at least as far.
    fn land(
        &mut self,
        id: NodeId,
        ctx: &mut TickContext<'_>,
        at: f64,
        pulse: &impl Fn(usize, i64) -> bool,
    ) {
        let whole = at.round();
        // On a clock, ambient or a gear's, or running free at a Speed that moves.
        let moving = !ctx.runs_free(id) || ctx.input(id, crate::nodes::timing::SPEED) != 0.0;
        let driven = moving && ctx.time.playing;
        self.landed = !(driven && (at - whole).abs() <= REACH);
        self.standing = !self.landed;
        if self.landed {
            return;
        }
        let step_index = whole as i64;
        self.current = Some(step_index);
        for (lane_idx, key) in LANE_KEYS.iter().enumerate() {
            if pulse(lane_idx, step_index)
                && let Some(edge) = self.lanes[lane_idx].set(true)
            {
                ctx.fire_at(id, key, edge, 0.0);
            }
        }
    }

    /// Close every open lane at the top of the frame.
    fn close(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        for (lane, &key) in self.lanes.iter_mut().zip(LANE_KEYS.iter()) {
            if let Some(edge) = lane.set(false) {
                ctx.fire_at(id, key, edge, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid's playhead is the step a lane reads however far the show has run: a step past
    /// what an `f32` holds whole still names its column in every lane whose length divides
    /// the period, and a step before zero names the one it always did.
    #[test]
    fn the_playhead_names_every_lanes_step_however_far_it_has_run() {
        let periods = [16, lcm(lcm(64, 63), lcm(61, 59)), lcm(5, 11)];
        for step in [0_i64, 17, -3, (1 << 24) + 5, 123_456_789_013, -987_654_321] {
            let at = Transport {
                current: Some(step),
                ..Transport::default()
            };
            for period in periods {
                let p = at.playhead(period).unwrap();
                assert_eq!(p.fract(), 0.0, "{p}");
                for lane in [16, 64, 63, 61, 59, 11, 5] {
                    if period % lane == 0 {
                        assert_eq!(
                            (p as i64).rem_euclid(lane),
                            step.rem_euclid(lane),
                            "step {step} in a lane of {lane}, period {period}"
                        );
                    }
                }
            }
        }
        assert_eq!(lcm(lcm(64, 63), lcm(61, 59)), 14_511_168);
        assert_eq!(
            Transport {
                current: Some(17),
                ..Transport::default()
            }
            .playhead(16),
            Some(17.0),
            "a step an f32 holds is itself"
        );
    }

    /// A lane comes back after the least shift that leaves its figure alone: four on the
    /// floor every four steps, a figure with no shorter repeat its whole length, a silent or a
    /// full lane every step, and a lane of 64 lit at 0 and 32 every 32.
    #[test]
    fn a_lane_repeats_at_its_least_shift() {
        let figure = |s: &str| {
            s.chars()
                .enumerate()
                .fold(0u64, |m, (i, c)| if c == 'x' { m | 1 << i } else { m })
        };
        assert_eq!(repeat(figure("x...x...x...x..."), 16), 4);
        assert_eq!(repeat(figure("x..x.x....x..x.."), 16), 16);
        assert_eq!(repeat(figure("x.x.x.x.x.x.x.x."), 16), 2);
        assert_eq!(repeat(0, 5), 1, "silent");
        assert_eq!(repeat(0b11111, 5), 1, "full");
        assert_eq!(repeat(figure("x.x.."), 5), 5);
        assert_eq!(repeat(1 | 1 << 32, 64), 32);
        assert_eq!(repeat(u64::MAX, 64), 1);
        // Lanes of 4, 5 and 8 steps meet after 40, two and a half bars; four on the floor in
        // every lane is a quarter of one.
        let floor = figure("x...x...x...x...");
        assert_eq!(
            bars([
                (floor, 16),
                (figure("x.x.."), 5),
                (figure("x......."), 8),
                (0, 3)
            ]),
            2.5
        );
        assert_eq!(bars([(floor, 16); 4]), 0.25);
    }
}
