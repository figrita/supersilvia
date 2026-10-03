// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a tick's milliseconds go, on the CPU and on the GPU, for the Status box.
//!
//! **Waiting is not work.** A tick is timed as a run of laps from the top of one to the top of
//! the next, each on the thread's own CPU clock ([`crate::platform::clock`]):
//! [`Laps::lap`] gives the CPU time since the previous lap to the [`Work`] named, and
//! whatever no lap claimed is [`Work::Other`]. The one wall-clock span inside a tick is the
//! sleep to the deadline, [`Laps::slept`]. What the wall saw and the CPU did not is time the
//! thread was off the CPU inside a phase — blocked in the driver with the GPU's queue full,
//! almost all of it — and is [`Phases::waiting`]: the tick less the sleep less every
//! [`Work`]. So the work, the waiting and the sleep add up to the tick by construction.
//!
//! **Each figure reads like the tick's own**: this tick, and a running mean with a time
//! constant of a second. Every mean is blended with the same weight on every tick, so the
//! means add up as the figures do.
//!
//! **Measured only while the Status box is open**, which is what the `status` flag of
//! [`super::Msg::Inputs`] says; closed, no lap reads a clock, the renderer places no GPU
//! mark and everything here is forgotten, so reopening it starts fresh. The **pacing** —
//! the spread of the last [`PACING`] ticks — is one of these figures, so its window starts
//! when the box opens and no tick sorts a distribution nothing shows.
//!
//! The GPU half is the renderer's — see [`crate::render::timing`] — and beside it is the
//! whole process's share of the render engine, read through [`crate::platform::gpu`]: on Linux
//! from the kernel's own DRM client counters in `/proc/self/fdinfo`, which see the editor's
//! painting and every picture window too; a Mac gives the whole GPU's, and Windows none.

use crate::graph::NodeId;
use crate::platform::gpu::Read;
use crate::render::{GpuPhase, GpuSpans};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// What a tick's CPU did, by the thread's own clock, in the order the Status box lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    /// The editor's messages, drained at the top of the tick.
    Inbox,
    /// The MIDI queues, drained and acted on.
    Midi,
    /// The clock and every CPU node, the Main Input's capture with them.
    Nodes,
    /// The frame job built out of the plan.
    Job,
    /// The whole draw: the uploads, the simulations, every Output, the probes, the mix and
    /// the flush that submits them.
    Draw,
    /// `publish`: every Output's finished frame found by the completion serial, and the
    /// pictures named.
    Publish,
    /// Tap readings routed to their nodes.
    Readbacks,
    /// The snapshot filled and swapped.
    Snapshot,
    /// Whatever no lap claimed.
    Other,
}

impl Work {
    pub const ALL: [Self; 9] = [
        Self::Inbox,
        Self::Midi,
        Self::Nodes,
        Self::Job,
        Self::Draw,
        Self::Publish,
        Self::Readbacks,
        Self::Snapshot,
        Self::Other,
    ];
}

pub const WORKS: usize = Work::ALL.len();

/// The calling thread's own CPU time: the clock that stands still while the thread is blocked.
pub use crate::platform::clock::thread_cpu;

/// How many tick intervals the pacing is over. Ten seconds at 60 Hz: long enough to hold a
/// stall and short enough that a run of good ticks clears one.
pub const PACING: usize = 600;

/// What the last [`PACING`] ticks looked like, as a shape rather than a number.
///
/// A mean hides a stall and a single worst hides how often. What a performer asks is *how
/// consistent is this*, and only a distribution answers that: `p50` says what a tick usually
/// costs and `p99` what the bad hundredth costs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pacing {
    /// How many intervals the figures are over.
    pub frames: usize,
    /// The median interval, in seconds: what a tick usually costs.
    pub p50: f32,
    /// The 99th percentile: what the bad hundredth costs.
    pub p99: f32,
}

impl Pacing {
    /// The shape of these intervals, in seconds.
    fn of(intervals: &std::collections::VecDeque<f32>) -> Self {
        let mut sorted: Vec<f32> = intervals.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let at = |q: f32| -> f32 {
            if sorted.is_empty() {
                return 0.0;
            }
            sorted[((sorted.len() - 1) as f32 * q).round() as usize]
        };
        Self {
            frames: sorted.len(),
            p50: at(0.5),
            p99: at(0.99),
        }
    }
}

/// How many nodes the Status box names under `nodes`.
pub const TOP_NODES: usize = 5;

/// One figure, in milliseconds: this tick, and the running mean of about a second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Reading {
    pub now: f32,
    pub avg: f32,
}

/// One [`Reading`] as it is kept, each figure moving the mean by the seconds since the last.
type Meter = crate::clock::Figure;

fn reading(meter: &Meter) -> Reading {
    Reading {
        now: meter.now,
        avg: meter.avg,
    }
}

/// The GPU's phases of a draw, as the Status box shows them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuPhases(pub [Reading; GpuPhase::ALL.len()]);

impl std::ops::Index<GpuPhase> for GpuPhases {
    type Output = Reading;

    fn index(&self, phase: GpuPhase) -> &Reading {
        &self.0[phase as usize]
    }
}

/// Everything the Status box shows about where a tick went.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Phases {
    /// The wall time from the top of one tick to the top of the next.
    pub tick: Reading,
    /// Every [`Work`] on the thread's CPU clock, indexed by it.
    pub work: [Reading; WORKS],
    /// The tick less the sleep and every work: the thread off the CPU inside a phase. With
    /// `work` and `asleep` it sums to `tick`.
    pub waiting: Reading,
    /// Asleep until the deadline, on the wall clock.
    pub asleep: Reading,
    /// The costliest nodes by their mean on the thread's CPU clock, at most [`TOP_NODES`],
    /// each named as the Status box names a node. The Main Input's capture is one of them, as
    /// `main input`.
    pub nodes: Vec<(String, Reading)>,
    /// `None` until a draw's GPU marks have landed, and where there is no GPU at all.
    pub gpu: Option<GpuPhases>,
    /// The share of the render engine this whole process used over the last second, `0..=1`,
    /// summed over its DRM clients — or, where [`crate::platform::gpu::WHOLE_GPU`], the whole
    /// GPU's busy share, every app's. `None` where the machine does not say.
    pub busy: Option<f32>,
    /// The spread of the ticks timed since the box opened, the last [`PACING`] of them.
    pub pacing: Pacing,
}

/// The tick's laps, and the meters they are folded into.
#[derive(Default)]
pub struct Laps {
    on: bool,
    /// The top of the tick being timed.
    started: Option<Instant>,
    /// Where the last lap ended, on the thread's CPU clock.
    last_cpu: Duration,
    /// This tick's sleep to the deadline, on the wall clock.
    asleep: Duration,
    work_spans: [Duration; WORKS],
    nodes: Vec<(NodeId, &'static str, Duration)>,
    main_input: Duration,
    /// Seconds the last whole tick took: the weight every figure is blended with.
    dt: f32,
    /// The last [`PACING`] ticks' wall intervals, in seconds, newest last.
    intervals: std::collections::VecDeque<f32>,
    tick: Meter,
    work: [Meter; WORKS],
    waiting: Meter,
    asleep_meter: Meter,
    /// Each node timed on the last tick, and whether it was seen on this one.
    node_meters: HashMap<NodeId, (&'static str, Meter, bool)>,
    main_input_meter: Meter,
    /// Every [`GpuPhase`], in its order.
    gpu: Option<[Meter; GpuPhase::ALL.len()]>,
    process: ProcessGpu,
}

impl Laps {
    /// Whether this tick is being timed.
    pub fn on(&self) -> bool {
        self.on
    }

    /// The top of a tick: fold the one that just ended, and start timing this one — or stop,
    /// and forget, where nothing reads it.
    pub fn begin(&mut self, on: bool) {
        if on || self.on {
            self.begin_at(on, Instant::now(), thread_cpu());
        }
    }

    /// [`Self::begin`] at a wall time and a CPU time given.
    pub fn begin_at(&mut self, on: bool, now: Instant, cpu: Duration) {
        if !on {
            *self = Self::default();
            return;
        }
        if self.on {
            if let Some(started) = self.started {
                self.fold(now - started, cpu);
            }
        } else {
            *self = Self::default();
            self.on = true;
        }
        self.started = Some(now);
        self.last_cpu = cpu;
        self.asleep = Duration::ZERO;
        self.work_spans = [Duration::ZERO; WORKS];
        self.nodes.clear();
        self.main_input = Duration::ZERO;
    }

    /// The CPU time since the last lap was this work's.
    pub fn lap(&mut self, work: Work) {
        if self.on {
            self.lap_at(work, thread_cpu());
        }
    }

    /// [`Self::lap`] at a CPU time given.
    pub fn lap_at(&mut self, work: Work, cpu: Duration) {
        self.work_spans[work as usize] += cpu.saturating_sub(self.last_cpu);
        self.last_cpu = cpu;
    }

    /// The thread slept `took` to the deadline since the last lap, which was no work at all.
    pub fn slept(&mut self, took: Duration) {
        if self.on {
            self.slept_at(took, thread_cpu());
        }
    }

    /// [`Self::slept`] at a CPU time given.
    pub fn slept_at(&mut self, took: Duration, cpu: Duration) {
        self.asleep += took;
        self.last_cpu = cpu;
    }

    /// One node's tick took this long on the thread's CPU clock.
    pub fn node(&mut self, id: NodeId, slug: &'static str, took: Duration) {
        self.nodes.push((id, slug, took));
    }

    /// The Main Input's capture took this long on the thread's CPU clock.
    pub fn main_input(&mut self, took: Duration) {
        self.main_input += took;
    }

    /// Readings the renderer's GPU marks returned.
    pub fn gpu(&mut self, readings: &[GpuSpans]) {
        if !self.on {
            return;
        }
        let dt = self.dt;
        for r in readings {
            let meters = self.gpu.get_or_insert_with(Default::default);
            for (meter, ms) in meters.iter_mut().zip(r.0) {
                meter.record(ms, dt);
            }
        }
    }

    /// Read the process's render engine counters, at most once a second.
    pub fn poll_process(&mut self) {
        if self.on {
            self.process.poll(Instant::now());
        }
    }

    /// Close the tick that took `tick`, the CPU clock standing at `cpu`: every figure
    /// recorded, and the CPU time since the last lap given to `other`.
    fn fold(&mut self, tick: Duration, cpu: Duration) {
        self.lap_at(Work::Other, cpu);
        let dt = tick.as_secs_f32();
        self.dt = dt;
        if self.intervals.len() == PACING {
            self.intervals.pop_front();
        }
        self.intervals.push_back(dt);
        self.tick.record(ms(tick), dt);
        self.asleep_meter.record(ms(self.asleep), dt);
        // The tick less the sleep less every work.
        let mut waiting = ms(tick) - ms(self.asleep);
        for (meter, span) in self.work.iter_mut().zip(self.work_spans) {
            meter.record(ms(span), dt);
            waiting -= ms(span);
        }
        self.waiting.record(waiting, dt);
        for (_, _, seen) in self.node_meters.values_mut() {
            *seen = false;
        }
        for (id, slug, took) in self.nodes.drain(..) {
            let entry = self
                .node_meters
                .entry(id)
                .or_insert((slug, Meter::default(), true));
            entry.0 = slug;
            entry.1.record(ms(took), dt);
            entry.2 = true;
        }
        // A node that did not tick is suspended or gone, and costs nothing.
        self.node_meters.retain(|_, (_, _, seen)| *seen);
        self.main_input_meter.record(ms(self.main_input), dt);
    }

    /// What the Status box shows. `None` until a whole tick has been timed.
    pub fn report(&self) -> Option<Phases> {
        if !self.on || !self.tick.seeded() {
            return None;
        }
        let mut nodes: Vec<(String, Reading)> = self
            .node_meters
            .iter()
            .map(|(id, (slug, meter, _))| (format!("{slug}{id}"), reading(meter)))
            .chain(
                (self.main_input_meter.avg > 0.0)
                    .then(|| ("main input".to_string(), reading(&self.main_input_meter))),
            )
            .collect();
        nodes.sort_by(|a, b| b.1.avg.total_cmp(&a.1.avg).then_with(|| a.0.cmp(&b.0)));
        nodes.truncate(TOP_NODES);
        Some(Phases {
            tick: reading(&self.tick),
            work: self.work.map(|m| reading(&m)),
            waiting: reading(&self.waiting),
            asleep: reading(&self.asleep_meter),
            nodes,
            gpu: self
                .gpu
                .map(|meters| GpuPhases(meters.map(|m| reading(&m)))),
            busy: self.process.busy,
            pacing: Pacing::of(&self.intervals),
        })
    }
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

/// How often the process's counters are read.
const PROCESS_EVERY: Duration = Duration::from_secs(1);
/// How often the process's descriptors are searched again for DRM ones.
const PROCESS_RESCAN: Duration = Duration::from_secs(5);

/// The whole process's use of the render engine, from the operating system's per-client
/// counters — the kernel's DRM fdinfo, on Linux; see [`crate::platform::gpu`].
///
/// The engine's cumulative nanoseconds over the process's clients, differenced across a
/// second, are the share of the engine this process kept busy: the synth, the editor's
/// painting and every picture window alike. A Mac gives the whole GPU's share instead, every
/// app's, already a share and taken as it is.
#[derive(Default)]
struct ProcessGpu {
    /// What to read, found again every [`PROCESS_RESCAN`].
    found: crate::platform::gpu::Clients,
    scanned: Option<Instant>,
    /// The clients the last total was summed over. A total over another set is not
    /// comparable with it.
    clients: Vec<u64>,
    last: Option<(Instant, u64)>,
    busy: Option<f32>,
}

impl ProcessGpu {
    fn poll(&mut self, now: Instant) {
        if self
            .last
            .is_some_and(|(at, _)| now.saturating_duration_since(at) < PROCESS_EVERY)
        {
            return;
        }
        if self
            .scanned
            .is_none_or(|at| now.saturating_duration_since(at) >= PROCESS_RESCAN)
        {
            self.found = crate::platform::gpu::Clients::scan();
            self.scanned = Some(now);
        }
        let (total, clients) = match self.found.read() {
            Some(Read::EngineNs(total, clients)) => (total, clients),
            Some(Read::WholeBusy(busy)) => {
                // Nothing to difference: the time is kept to wait out `PROCESS_EVERY`.
                self.busy = Some(busy);
                self.clients.clear();
                self.last = Some((now, 0));
                return;
            }
            None => {
                self.busy = None;
                self.last = None;
                return;
            }
        };
        if clients == self.clients
            && let Some((at, before)) = self.last
        {
            let wall = now.saturating_duration_since(at).as_nanos() as f32;
            self.busy = Some(total.saturating_sub(before) as f32 / wall.max(1.0));
        }
        self.clients = clients;
        self.last = Some((now, total));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On the thread's CPU clock, the work, the waiting and the sleep add up to the tick, with
    /// `other` holding the CPU time no lap named.
    #[test]
    fn work_waiting_and_sleep_sum_to_the_tick() {
        let t0 = Instant::now();
        let at = |us: u64| t0 + Duration::from_micros(us);
        let cpu = Duration::from_micros;
        let mut laps = Laps::default();
        laps.begin_at(true, at(0), cpu(0));
        for tick in 0..3u64 {
            let top = tick * 16_000;
            // The draw spends 1.5 of its six milliseconds on the CPU and the rest blocked.
            let c = tick * 4_300;
            laps.lap_at(Work::Inbox, cpu(c + 100));
            laps.lap_at(Work::Midi, cpu(c + 150));
            laps.lap_at(Work::Nodes, cpu(c + 2_150));
            laps.lap_at(Work::Job, cpu(c + 2_400));
            laps.lap_at(Work::Draw, cpu(c + 3_900));
            laps.lap_at(Work::Publish, cpu(c + 4_000));
            laps.lap_at(Work::Readbacks, cpu(c + 4_050));
            laps.lap_at(Work::Snapshot, cpu(c + 4_290));
            laps.slept_at(Duration::from_micros(7_100), cpu(c + 4_295));
            laps.begin_at(true, at(top + 16_000), cpu(c + 4_300));
        }
        let report = laps.report().expect("three ticks timed");
        assert!((report.tick.now - 16.0).abs() < 1e-3, "{}", report.tick.now);
        assert!((report.asleep.now - 7.1).abs() < 1e-3);
        let work = |w: Work| report.work[w as usize];
        assert!((work(Work::Draw).now - 1.5).abs() < 1e-3);
        assert!((work(Work::Nodes).now - 2.0).abs() < 1e-3);
        // Five microseconds between the sleep and the top of the next tick.
        assert!((work(Work::Other).now - 0.005).abs() < 1e-3);
        // The tick less the sleep less 4.295 ms of work: the 5 µs going to sleep are no one's.
        assert!(
            (report.waiting.now - 4.605).abs() < 1e-3,
            "{}",
            report.waiting.now
        );
        let picks: [fn(Reading) -> f32; 2] = [|r| r.now, |r| r.avg];
        for pick in picks {
            let total = pick(report.waiting)
                + pick(report.asleep)
                + report.work.iter().map(|r| pick(*r)).sum::<f32>();
            assert!(
                (total - pick(report.tick)).abs() < 1e-3,
                "work, waiting and sleep add up to the tick: {total} against {}",
                pick(report.tick)
            );
        }
    }

    /// The pacing is over the ticks timed since the box opened: closing forgets them, so a
    /// stall before it does not stand in the spread after.
    #[test]
    fn the_pacing_starts_when_the_box_opens() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut laps = Laps::default();
        laps.begin_at(true, t0, Duration::ZERO);
        laps.begin_at(true, t0 + ms(500), Duration::ZERO);
        let stalled = laps.report().unwrap().pacing;
        assert_eq!((stalled.frames, stalled.p99), (1, 0.5));
        laps.begin_at(false, t0 + ms(510), Duration::ZERO);
        let mut at = t0 + ms(600);
        laps.begin_at(true, at, Duration::ZERO);
        for _ in 0..3 {
            at += ms(10);
            laps.begin_at(true, at, Duration::ZERO);
        }
        let pacing = laps.report().unwrap().pacing;
        assert_eq!(pacing.frames, 3, "{pacing:?}");
        assert!((pacing.p99 - 0.01).abs() < 1e-6, "{pacing:?}");
        assert!((pacing.p50 - 0.01).abs() < 1e-6, "{pacing:?}");
    }

    /// Closed, nothing is kept: reopened, it starts over from the next whole tick.
    #[test]
    fn closing_forgets() {
        let t0 = Instant::now();
        let mut laps = Laps::default();
        laps.begin_at(true, t0, Duration::ZERO);
        laps.begin_at(true, t0 + Duration::from_millis(16), Duration::ZERO);
        assert!(laps.report().is_some());
        laps.begin_at(false, t0 + Duration::from_millis(32), Duration::ZERO);
        assert!(!laps.on());
        assert!(laps.report().is_none());
        laps.begin_at(true, t0 + Duration::from_millis(48), Duration::ZERO);
        assert!(
            laps.report().is_none(),
            "no whole tick has been timed since"
        );
    }

    #[test]
    fn the_costliest_nodes_are_named_first() {
        let t0 = Instant::now();
        let mut laps = Laps::default();
        laps.begin_at(true, t0, Duration::ZERO);
        for (i, us) in [10u64, 900, 40, 300, 5, 70, 2].into_iter().enumerate() {
            laps.node(NodeId(i as u32 + 1), "lfo", Duration::from_micros(us));
        }
        laps.begin_at(true, t0 + Duration::from_millis(16), Duration::ZERO);
        let names: Vec<String> = laps
            .report()
            .unwrap()
            .nodes
            .into_iter()
            .map(|n| n.0)
            .collect();
        assert_eq!(names, ["lfo2", "lfo4", "lfo6", "lfo3", "lfo1"]);
    }

    /// A node is timed on the thread's CPU clock, as its parent row is: one that blocks does
    /// not count the time it was off the CPU, so the named nodes never outweigh `CPU nodes`.
    #[test]
    fn a_blocking_node_costs_only_its_cpu_time() {
        let started = thread_cpu();
        std::thread::sleep(Duration::from_millis(20));
        let took = thread_cpu().saturating_sub(started);
        assert!(
            took < Duration::from_millis(10),
            "asleep is not on the CPU: {took:?}"
        );
    }
}
