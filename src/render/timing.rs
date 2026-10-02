// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GPU's own measurement: one span per Output's frame, a draw's phases, and the editor's
//! painting (`proposals/wgpu.md`, 1.14).
//!
//! **Timestamps at pass boundaries.** An Output's render pass carries `timestamp_writes` at
//! its beginning and end ([`Stamps::span`]). Each of the draw's ten marks is an empty compute
//! pass whose end writes one timestamp into the encoder of the phase it bounds
//! ([`Stamps::mark`]). A pass-boundary timestamp needs `TIMESTAMP_QUERY` alone, so a mark goes
//! wherever an Output's span does, and on Vulkan both are written at the bottom of the pipe,
//! after every command recorded before them. A span is the GPU's clock between two marks of
//! the synth's own stream, so it holds whatever else the GPU fitted in between them.
//!
//! **One query set per renderer, and few in the process on Metal.** A draw's marks and every
//! Output span it records are one range of the renderer's single query set: the ten marks,
//! then a pair per Output drawn, for the first [`TIMED`] to draw in plan order. Plan order
//! holds from tick to tick, so an Output past them is never timed while they all draw: its
//! cost is unknown, and it goes in a submission of its own. Metal holds at most 32 counter
//! sample buffers in a process, across every device, and wgpu loses the device that asks for
//! a 33rd, so on Metal every query set is made under a claim on [`QUERY_SETS`]: a renderer
//! that finds them all held draws untimed rather than lose its device, and asks again at each
//! draw until one is given back; a paint timer that finds them held is not made. Other
//! backends have no such limit and take no claim.
//!
//! **Read back under the timer's rule.** A draw's range is resolved into a buffer and copied
//! into a staging buffer in the same recording as its last timestamp, and read once the
//! staging buffer's `map_async` has fired — asked, never waited for. The map is asked for at
//! the next collect, since the submission that carries the copy is made after the last mark
//! and a buffer with a map pending may not be submitted. A ring of [`DEPTH`] ranges, oldest
//! first: a draw that finds its range still in flight places no marks and times no Output
//! rather than overwrite one, and a range still in flight holds back the ones after it.
//!
//! **Where the device has no `TIMESTAMP_QUERY` there is no line, never a zero**: no ring is
//! made, nothing is marked and every reading is `None`. The same holds for a timestamp the GPU
//! did not write, which resolves to zero: a draw or frame holding one gives no reading. Metal
//! writes none for a pass with no work in it, so on a Mac every mark reads zero and a draw has
//! no breakdown, while an Output's span, whose pass draws, is written. A mark that dispatched
//! work would be written, but Metal orders passes only by the resources they share, and an
//! Output's pass then runs outside the marks around it — before `Start`, at times — so the
//! phases would not be what they name. A Vulkan timestamp reads zero only on the tick its
//! counter wraps, which loses that one reading rather than show a wrong one. The editor's
//! [`PaintTimer`] also needs `TIMESTAMP_QUERY_INSIDE_PASSES`, since its end mark sits inside
//! egui's render pass; Apple GPUs sample counters only at stage boundaries and do not offer it.
//!
//! **Where each mark goes.** A tick is three kinds of submission: the prelude — `Start`,
//! `UploadsFrom`, the uploads, `UploadsTo`, the simulations, `SimsTo` and `OutputsFrom` — then
//! one per Output, then the coda — `OutputsTo`, the workspace passes, `PassesTo`, the probes,
//! `ProbesTo`, the mix, `MixTo` and `End`. A mark lands in the recording it is handed, and a mark not placed asks nothing of
//! it, so a phase with nothing in it still submits nothing.

use super::gpu::Gpu;
use super::queue::Recording;
use crate::clock::Figure;
use crate::graph::NodeId;
use std::ops::Range;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A part of a draw on the GPU, in the order the Status box lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPhase {
    /// The sources: every upload and every conversion pass.
    Uploads,
    /// Every simulation's passes: its compute dispatches and a reallocation's resamples.
    Sims,
    /// The Outputs, first to last: the per-Output spans and whatever lies between them — a
    /// tap's copy, a thumbnail's pass, a capture's read.
    Outputs,
    /// The workspace passes: every thumbnail on the workspace being looked at.
    Passes,
    Probes,
    Mix,
    /// The whole less the six above: the GPU time of the draw that no part names.
    Between,
    /// The whole draw, from its first mark to its last.
    Whole,
}

impl GpuPhase {
    pub const ALL: [Self; 8] = [
        Self::Uploads,
        Self::Sims,
        Self::Outputs,
        Self::Passes,
        Self::Probes,
        Self::Mix,
        Self::Between,
        Self::Whole,
    ];
}

/// The GPU time of one draw, by phase, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuSpans(pub [f32; GpuPhase::ALL.len()]);

impl std::ops::Index<GpuPhase> for GpuSpans {
    type Output = f32;

    fn index(&self, phase: GpuPhase) -> &f32 {
        &self.0[phase as usize]
    }
}

/// What the GPU spent on one Output's frame, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuTime {
    /// The most recent frame's span.
    pub latest: f32,
    /// The longest span in the last two seconds.
    pub worst: f32,
}

/// Where a mark is placed in the draw. Indices into a set's queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Start = 0,
    UploadsFrom,
    UploadsTo,
    /// The end of the simulations, which begin where the uploads end.
    SimsTo,
    OutputsFrom,
    /// The end of the Outputs and the start of the workspace passes.
    OutputsTo,
    /// The end of the workspace passes and the start of the probes.
    PassesTo,
    /// The end of the probes and the start of the mix.
    ProbesTo,
    MixTo,
    End,
}

const MARKS: usize = Mark::End as usize + 1;

/// How many draws' marks and spans may be in flight at once.
pub const DEPTH: usize = super::TICKS_IN_FLIGHT + 1;

/// How many editor frames' marks may be in flight at once. One more stage than a draw's: the
/// end mark is inside egui's pass, so a frame's pair is resolved in the next frame's encoder.
const PAINT_DEPTH: usize = 4;

/// The most query sets this process holds at once on Metal, across every device and renderer:
/// one per [`Stamps`] and one per [`PaintTimer`]. Metal's own limit is 32 and the 33rd loses
/// the device; the rest is slack for sets dropped whose memory wgpu frees only at its next
/// poll. No other backend is held to it.
pub const QUERY_SETS: usize = 16;

/// The most Outputs one draw times: the first to draw, in plan order.
pub const TIMED: usize = 512;

/// A draw's first Output span, past its marks: a resolve lands at a multiple of
/// `QUERY_RESOLVE_BUFFER_ALIGNMENT` bytes into its buffer.
const SPANS_FROM: u32 = (wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT / size_of::<u64>() as u64) as u32;

/// The queries one draw's range holds: its marks, then a pair per timed Output.
const DRAW_QUERIES: u32 = SPANS_FROM + 2 * TIMED as u32;

const _: () = assert!(MARKS as u32 <= SPANS_FROM);
const _: () = assert!(DEPTH as u32 * DRAW_QUERIES <= wgpu::QUERY_SET_MAX_QUERIES);

/// Metal query sets held now, under [`Claim`]s.
static LIVE_SETS: AtomicUsize = AtomicUsize::new(0);

/// How many query sets the process holds on Metal now: never more than [`QUERY_SETS`].
pub fn live_query_sets() -> usize {
    LIVE_SETS.load(Ordering::Acquire)
}

/// Leave to hold one query set, given back when dropped.
struct Claim;

impl Claim {
    /// One of the [`QUERY_SETS`], where one is left.
    fn take() -> Option<Self> {
        LIVE_SETS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < QUERY_SETS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        LIVE_SETS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Whether the device writes pass-boundary timestamps.
fn timestamps(gpu: &Gpu) -> bool {
    gpu.device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
}

/// A staging buffer's `map_async`, as its callback leaves it.
const MAP_PENDING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;

/// Where one range of timestamps is in its round trip.
enum State {
    /// Nothing in it waits to be read.
    Free,
    /// Being marked.
    Open,
    /// Every mark placed, the range unresolved: a paint pair, whose end is inside egui's pass.
    Ended,
    /// Resolved and copied into its staging buffer, in a submission made or about to be.
    Closed,
    /// The staging buffer's map is asked for; the callback stores into the flag.
    Mapping(Arc<AtomicU8>),
}

/// One range of the ring's queries: the buffer they resolve into, the buffer that is read,
/// and where it is.
struct Set {
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    state: State,
    /// Read and dropped rather than kept: marked before the ring was last forgotten.
    discard: bool,
    /// How many timestamps from the range's first the last resolve covered.
    read: u32,
}

impl Set {
    fn new(device: &wgpu::Device, queries: u32, label: &str) -> Self {
        let size = u64::from(queries) * size_of::<u64>() as u64;
        Self {
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: State::Free,
            discard: false,
            read: 0,
        }
    }

    /// Resolve `written`, each relative to the range's first query `base`, and copy the result
    /// where it is read.
    fn resolve(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queries: &wgpu::QuerySet,
        base: u32,
        written: &[Range<u32>],
    ) {
        let Some(read) = written.iter().map(|r| r.end).max() else {
            return;
        };
        for range in written {
            let offset = u64::from(range.start) * size_of::<u64>() as u64;
            debug_assert_eq!(offset % wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT, 0);
            encoder.resolve_query_set(
                queries,
                base + range.start..base + range.end,
                &self.resolve,
                offset,
            );
        }
        let bytes = u64::from(read) * size_of::<u64>() as u64;
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, bytes);
        self.read = read;
        self.state = State::Closed;
    }

    /// Ask for the staging buffer's map. Only once the submission carrying the copy is made.
    fn map(&mut self) {
        let flag = Arc::new(AtomicU8::new(MAP_PENDING));
        let stored = Arc::clone(&flag);
        self.staging.map_async(wgpu::MapMode::Read, .., move |r| {
            stored.store(
                if r.is_ok() { MAP_DONE } else { MAP_FAILED },
                Ordering::Release,
            );
        });
        self.state = State::Mapping(flag);
    }

    /// Free set `at` once its map has fired, putting its timestamps in `ready` where the map
    /// succeeded and the set is not discarded. False while it is still in flight.
    fn land(&mut self, at: usize, ready: &mut Vec<(usize, Vec<u64>)>) -> bool {
        let State::Mapping(flag) = &self.state else {
            return false;
        };
        let read = match flag.load(Ordering::Acquire) {
            MAP_PENDING => return false,
            MAP_DONE => {
                let bytes = u64::from(self.read) * size_of::<u64>() as u64;
                let read = self
                    .staging
                    .slice(..bytes)
                    .get_mapped_range()
                    .ok()
                    .map(|bytes| {
                        bytes
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|b| u64::from_le_bytes(*b))
                            .collect()
                    });
                self.staging.unmap();
                read
            }
            _ => None,
        };
        self.state = State::Free;
        let discard = std::mem::take(&mut self.discard);
        ready.extend(read.filter(|_| !discard).map(|t| (at, t)));
        true
    }
}

/// A ring of ranges of one query set, `stride` queries a range, each read once its map has
/// fired and never sooner, oldest first.
struct Ring {
    queries: wgpu::QuerySet,
    /// Leave for `queries` on Metal, given back after them.
    _claim: Option<Claim>,
    stride: u32,
    sets: Vec<Set>,
    /// The range the next draw or frame marks into, which is also the oldest.
    next: usize,
    /// Nanoseconds per timestamp tick.
    period: f64,
    /// Every range's timestamps read since the owner last took them, by range, oldest first.
    ready: Vec<(usize, Vec<u64>)>,
}

impl Ring {
    /// `depth` ranges of `stride` queries on `gpu`'s device. `None` where it has no
    /// `TIMESTAMP_QUERY`, or on Metal where the process holds [`QUERY_SETS`] already.
    fn new(gpu: &Gpu, depth: usize, stride: u32, label: &str) -> Option<Self> {
        if !timestamps(gpu) {
            return None;
        }
        let claim = if gpu.adapter().get_info().backend == wgpu::Backend::Metal {
            Some(Claim::take()?)
        } else {
            None
        };
        let device = gpu.device();
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some(label),
                ty: wgpu::QueryType::Timestamp,
                count: depth as u32 * stride,
            }),
            _claim: claim,
            stride,
            sets: (0..depth)
                .map(|_| Set::new(device, stride, label))
                .collect(),
            next: 0,
            period: f64::from(gpu.queue().get_timestamp_period()),
            ready: Vec::new(),
        })
    }

    fn queries(&self) -> &wgpu::QuerySet {
        &self.queries
    }

    /// The first query of range `set`.
    fn base(&self, set: usize) -> u32 {
        set as u32 * self.stride
    }

    /// Read every range that has landed, oldest first, then ask for the map of every range
    /// whose submission is made. A range still in flight holds back the ones after it, which
    /// were placed later and cannot have landed sooner. Asks rather than waits.
    fn collect(&mut self) {
        let depth = self.sets.len();
        for i in 0..depth {
            let at = (self.next + i) % depth;
            let set = &mut self.sets[at];
            match set.state {
                State::Free | State::Open => {}
                State::Ended | State::Closed => break,
                State::Mapping(_) => {
                    if !set.land(at, &mut self.ready) {
                        break;
                    }
                }
            }
        }
        for set in &mut self.sets {
            if matches!(set.state, State::Closed) {
                set.map();
            }
        }
    }

    /// Begin marking the next range, if it is free: which range it is and its first query. A
    /// range still marking is marked again.
    fn open(&mut self) -> Option<(usize, u32)> {
        let at = self.next;
        let set = &mut self.sets[at];
        match set.state {
            State::Free => set.state = State::Open,
            State::Open => {}
            _ => return None,
        }
        Some((at, self.base(at)))
    }

    /// The range being marked and its first query, where one is.
    fn marking(&self) -> Option<(usize, u32)> {
        let at = self.next;
        matches!(self.sets[at].state, State::Open).then(|| (at, self.base(at)))
    }

    /// End marking: the range waits to be resolved, and the next one is up. `None` where
    /// nothing was marking.
    fn end(&mut self) -> Option<usize> {
        let at = self.next;
        let set = &mut self.sets[at];
        if !matches!(set.state, State::Open) {
            return None;
        }
        set.state = State::Ended;
        self.next = (self.next + 1) % self.sets.len();
        Some(at)
    }

    /// End marking and resolve `written` into `encoder`, after the last mark.
    fn close(&mut self, encoder: &mut wgpu::CommandEncoder, written: &[Range<u32>]) {
        if let Some(at) = self.end() {
            let base = self.base(at);
            self.sets[at].resolve(encoder, &self.queries, base, written);
        }
    }

    /// End marking a range nothing was written into: it is free again, and the next one is
    /// up.
    fn let_go(&mut self) {
        if let Some(at) = self.end() {
            self.sets[at].state = State::Free;
        }
    }

    /// Resolve `written` of every ended range into `encoder`.
    fn resolve_ended(&mut self, encoder: &mut wgpu::CommandEncoder, written: &[Range<u32>]) {
        for at in 0..self.sets.len() {
            if matches!(self.sets[at].state, State::Ended) {
                let base = self.base(at);
                self.sets[at].resolve(encoder, &self.queries, base, written);
            }
        }
    }

    /// Every range read since the last call, by range, as nanoseconds. A timestamp the GPU
    /// never wrote stays zero.
    fn take(&mut self) -> Vec<(usize, Vec<u64>)> {
        let period = self.period;
        std::mem::take(&mut self.ready)
            .into_iter()
            .map(|(at, t)| {
                (
                    at,
                    t.into_iter().map(|t| (t as f64 * period) as u64).collect(),
                )
            })
            .collect()
    }

    /// Let every range go unread, and what was read with them: marks placed before a pause
    /// are not a reading of anything after it. A range whose map is asked for is dropped once
    /// it lands, since its buffer is not free until then.
    fn forget(&mut self) {
        for set in &mut self.sets {
            match set.state {
                State::Mapping(_) => set.discard = true,
                _ => set.state = State::Free,
            }
        }
        self.ready.clear();
    }
}

/// An empty compute pass whose end writes timestamp `index` of `queries` into `encoder`.
fn stamp(encoder: &mut wgpu::CommandEncoder, queries: &wgpu::QuerySet, index: u32) {
    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("mark"),
        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index: Some(index),
        }),
    });
}

/// A span in milliseconds between two timestamps in nanoseconds. `None` where either was
/// never written or they do not run forward.
fn between(from: u64, to: u64) -> Option<f32> {
    (from != 0 && to >= from).then(|| (to - from) as f32 / 1.0e6)
}

/// A draw's marks and its Outputs' spans, in flight and landed.
#[derive(Default)]
pub struct Stamps {
    draws: Option<Draws>,
    /// Refused a query set on Metal, every one of [`QUERY_SETS`] held: asked again at each
    /// draw's start until one is given back.
    waiting: bool,
}

/// The ring behind [`Stamps`], and what each of its ranges holds.
struct Draws {
    ring: Ring,
    /// Per range: whether the draw placed its marks.
    marked: Vec<bool>,
    /// Per range: whose frame each of its pairs is, the Output and its timer's
    /// [`Timer::id`].
    timed: Vec<Vec<(NodeId, u64)>>,
    /// Every draw's phases read since the last take, oldest first.
    phases: Vec<GpuSpans>,
    /// Every Output's frame read since the last take, oldest first.
    frames: Vec<Frame>,
}

impl Draws {
    /// The ring and its bookkeeping. `None` where [`Ring::new`] makes no ring.
    fn new(gpu: &Gpu) -> Option<Self> {
        Some(Self {
            ring: Ring::new(gpu, DEPTH, DRAW_QUERIES, "draw timer")?,
            marked: vec![false; DEPTH],
            timed: vec![Vec::new(); DEPTH],
            phases: Vec::new(),
            frames: Vec::new(),
        })
    }
}

/// One Output's frame, read back: whose it is and its span in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub node: NodeId,
    /// The [`Timer::id`] of the timer that asked for it.
    pub timer: u64,
    pub ms: f32,
}

impl Stamps {
    /// The ring, made once. Without `TIMESTAMP_QUERY` there is none, and then there is no GPU
    /// breakdown and no Output span — never a zero. On Metal with every one of [`QUERY_SETS`]
    /// held there is none until [`Self::begin`] finds one given back.
    pub fn new(gpu: &Gpu) -> Self {
        if !timestamps(gpu) {
            log::warn!("the GPU writes no timestamps: the Status box has no GPU lines");
            return Self::default();
        }
        let draws = Draws::new(gpu);
        let waiting = draws.is_none();
        if waiting {
            log::warn!(
                "the process holds all {QUERY_SETS} of its timestamp query sets: this renderer \
                 is untimed until one is given back"
            );
        }
        Self { draws, waiting }
    }

    /// Collect whatever ranges have landed, then open this draw's where it is free. Once a
    /// draw, before it places anything, on the device the renderer was made on.
    pub fn begin(&mut self, gpu: &Gpu) {
        if self.waiting {
            self.draws = Draws::new(gpu);
            self.waiting = self.draws.is_none();
        }
        let Some(d) = &mut self.draws else {
            return;
        };
        d.ring.collect();
        for (at, t) in d.ring.take() {
            if d.marked[at] {
                d.phases.extend(t.first_chunk().and_then(spans));
            }
            for (i, &(node, timer)) in d.timed[at].iter().enumerate() {
                let from = SPANS_FROM as usize + 2 * i;
                if let (Some(&a), Some(&b)) = (t.get(from), t.get(from + 1))
                    && let Some(ms) = between(a, b)
                {
                    d.frames.push(Frame { node, timer, ms });
                }
            }
        }
        if let Some((at, _)) = d.ring.open() {
            d.marked[at] = false;
            d.timed[at].clear();
        }
    }

    /// Place `mark` in `recording`, where this draw is marking: `measuring` says whether the
    /// Status box is open. A draw whose `Start` is not placed places none of the others. A
    /// mark not placed asks nothing of the recording, so a phase with nothing in it still
    /// submits nothing.
    pub fn mark(&mut self, recording: &mut Recording, mark: Mark, measuring: bool) {
        let Some(d) = &mut self.draws else {
            return;
        };
        let Some((at, base)) = d.ring.marking() else {
            return;
        };
        if !measuring || (mark != Mark::Start && !d.marked[at]) {
            return;
        }
        d.marked[at] = true;
        stamp(recording.encoder(), d.ring.queries(), base + mark as u32);
    }

    /// The timestamps `node`'s pass writes this draw, whose reading goes to the timer
    /// `timer`: `None` where the draw is not being timed or [`TIMED`] Outputs already are. An
    /// Output that then draws nothing gives them back with [`Self::withdraw`].
    pub fn span(
        &mut self,
        node: NodeId,
        timer: u64,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let d = self.draws.as_mut()?;
        let (at, base) = d.ring.marking()?;
        let timed = &mut d.timed[at];
        if timed.len() >= TIMED {
            return None;
        }
        let from = base + SPANS_FROM + 2 * timed.len() as u32;
        timed.push((node, timer));
        Some(wgpu::RenderPassTimestampWrites {
            query_set: d.ring.queries(),
            beginning_of_pass_write_index: Some(from),
            end_of_pass_write_index: Some(from + 1),
        })
    }

    /// Give back the span `node` was handed this draw, if it was the last handed out: its
    /// Output recorded no pass. An Output handed none gives nothing back.
    pub fn withdraw(&mut self, node: NodeId) {
        if let Some(d) = &mut self.draws
            && let Some((at, _)) = d.ring.marking()
            && d.timed[at].last().is_some_and(|&(last, _)| last == node)
        {
            d.timed[at].pop();
        }
    }

    /// End this draw's range and resolve what it wrote into `recording`, after its last pass
    /// and its last mark. A draw that wrote nothing asks nothing of the recording.
    pub fn close(&mut self, recording: &mut Recording) {
        let Some(d) = &mut self.draws else {
            return;
        };
        let Some((at, _)) = d.ring.marking() else {
            return;
        };
        let mut written = Vec::with_capacity(2);
        if d.marked[at] {
            written.push(0..MARKS as u32);
        }
        if !d.timed[at].is_empty() {
            written.push(SPANS_FROM..SPANS_FROM + 2 * d.timed[at].len() as u32);
        }
        if written.is_empty() {
            d.ring.let_go();
        } else {
            d.ring.close(recording.encoder(), &written);
        }
    }

    /// Every draw's phases whose marks have landed since the last call, oldest first.
    pub fn take(&mut self) -> Vec<GpuSpans> {
        self.draws
            .as_mut()
            .map(|d| std::mem::take(&mut d.phases))
            .unwrap_or_default()
    }

    /// Every Output's frame whose span has landed since the last call, oldest first.
    pub fn take_frames(&mut self) -> Vec<Frame> {
        self.draws
            .as_mut()
            .map(|d| std::mem::take(&mut d.frames))
            .unwrap_or_default()
    }
}

/// Where the next [`Timer`]'s id comes from.
static TIMERS: AtomicU64 = AtomicU64::new(0);

/// One Output's span per frame, read back by the renderer's [`Stamps`] and handed here.
pub struct Timer {
    /// Which of the renderer's frames are this timer's: a fresh one each time it forgets, so a
    /// frame asked for before that is never read into it.
    id: u64,
    /// Each span in milliseconds, the latest and the longest in the recent past. `None` until
    /// a span has been read back, and again once the Output goes dark.
    spans: Option<Figure>,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            id: TIMERS.fetch_add(1, Ordering::Relaxed),
            spans: None,
        }
    }
}

impl Timer {
    /// What [`Stamps::span`] is told, so the frame comes back here.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// A frame's span, read back. Dropped where it was asked for before the last forget.
    pub fn record(&mut self, frame: &Frame) {
        if frame.timer == self.id {
            self.spans.get_or_insert_default().record(frame.ms, 1.0);
        }
    }

    /// The latest span and the worst of the last two seconds. `None` before the first and
    /// once the Output has gone dark.
    pub fn reading(&self) -> Option<GpuTime> {
        self.spans.map(|f| GpuTime {
            latest: f.now,
            worst: f.worst,
        })
    }

    /// Drop what was measured: the Output has gone dark. Frames still in flight are let go
    /// unread.
    pub fn forget(&mut self) {
        self.spans = None;
        self.id = TIMERS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Which end of the editor's painting a mark is placed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintMark {
    /// Ahead of every shape: in the first paint callback's `prepare`, into egui's encoder
    /// before its pass.
    Start,
    /// Above every window and tooltip: in the last paint callback's `paint`, inside egui's
    /// pass.
    End,
}

/// What the editor's own painting cost on the GPU, frame by frame: a timestamp ahead of egui's
/// render pass and another inside it, after its last shape.
///
/// The start is an empty compute pass recorded into egui's encoder from the frame's first
/// paint callback's `prepare` ([`PaintTimer::start`]), and the end a timestamp written into
/// egui's render pass from its last callback's `paint` ([`PaintTimer::end`]), so the span is
/// every mesh and picture blit egui paints that frame. Nothing runs in egui's encoder after
/// its pass, so a frame's pair is resolved in the next frame's `prepare`, and its map asked
/// for in the one after that.
///
/// **Read without waiting**, on the ring [`Stamps`] is on: [`PAINT_DEPTH`] pairs, and a frame
/// that finds its pair still in flight places no marks. A paint callback holds this through an
/// `Arc`, so the ring is behind a mutex that nothing but the frame thread ever takes.
pub struct PaintTimer {
    marks: Mutex<Ring>,
}

/// A paint pair's two timestamps, in its range.
const PAIR: Range<u32> = 0..2;

impl PaintTimer {
    /// The ring, made once on `gpu`. `None` where the device lacks `TIMESTAMP_QUERY` or
    /// `TIMESTAMP_QUERY_INSIDE_PASSES`, or on Metal the process holds every one of
    /// [`QUERY_SETS`], and then the editor has no GPU figure — never a zero.
    pub fn new(gpu: &Gpu) -> Option<Self> {
        let inside = gpu
            .device()
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
        if !inside {
            return None;
        }
        let Some(ring) = Ring::new(gpu, PAINT_DEPTH, PAIR.end, "paint timer") else {
            if timestamps(gpu) {
                log::warn!(
                    "the process holds all {QUERY_SETS} of its timestamp query sets: the \
                     editor's painting is untimed"
                );
            }
            return None;
        };
        Some(Self {
            marks: Mutex::new(ring),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Ring> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Read every pair that has landed, resolve the last frame's pair, then mark the start of
    /// this frame's painting into `encoder` if its pair is free. Called from the frame's first
    /// paint callback's `prepare`, whose encoder egui submits ahead of its pass.
    pub fn start(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut marks = self.lock();
        marks.collect();
        marks.resolve_ended(encoder, &[PAIR]);
        if let Some((_, base)) = marks.open() {
            stamp(encoder, marks.queries(), base);
        }
    }

    /// Mark the end of this frame's painting inside egui's render pass, on a frame that
    /// marked its start. Called from the frame's last paint callback's `paint`.
    pub fn end(&self, pass: &mut wgpu::RenderPass<'_>) {
        let mut marks = self.lock();
        if let Some((_, base)) = marks.marking() {
            pass.write_timestamp(marks.queries(), base + 1);
            marks.end();
        }
    }

    /// Every frame's span read since the last call, oldest first, in milliseconds.
    pub fn take(&self) -> Vec<f32> {
        self.lock()
            .take()
            .into_iter()
            .filter_map(|(_, t)| between(*t.first()?, *t.get(1)?))
            .collect()
    }

    /// Let every pair still unread go, so marking again starts from nothing: called while
    /// nothing is being marked.
    pub fn forget(&self) {
        self.lock().forget();
    }

    /// Put the paint callback that places this frame's `mark`: [`PaintMark::Start`] ahead of
    /// every shape, on the background layer before any panel is laid out, and
    /// [`PaintMark::End`] above every window, on the debug layer. The end is placed only on a
    /// frame whose start was.
    pub fn mark_on_paint(self: &Arc<Self>, ctx: &eframe::egui::Context, mark: PaintMark) {
        use eframe::egui;
        let layer = match mark {
            PaintMark::Start => egui::LayerId::background(),
            PaintMark::End => egui::LayerId::debug(),
        };
        ctx.layer_painter(layer)
            .add(egui_wgpu::Callback::new_paint_callback(
                ctx.content_rect(),
                Marking {
                    timer: Arc::clone(self),
                    mark,
                },
            ));
    }
}

/// The paint callback that places one of a frame's two marks: the start in its `prepare`, into
/// egui's encoder ahead of its pass, and the end in its `paint`, inside the pass.
struct Marking {
    timer: Arc<PaintTimer>,
    mark: PaintMark,
}

impl egui_wgpu::CallbackTrait for Marking {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if self.mark == PaintMark::Start {
            self.timer.start(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        if self.mark == PaintMark::End {
            self.timer.end(pass);
        }
    }
}

/// One draw's marks, in nanoseconds, as spans in milliseconds. `None` where a mark was never
/// written, which reads zero, or where the marks do not run forward, which a wrapped counter
/// would make them do.
fn spans(t: &[u64; MARKS]) -> Option<GpuSpans> {
    if t.contains(&0) || t.windows(2).any(|w| w[1] < w[0]) {
        return None;
    }
    let ms = |from: Mark, to: Mark| (t[to as usize] - t[from as usize]) as f32 / 1.0e6;
    let parts = [
        ms(Mark::UploadsFrom, Mark::UploadsTo),
        ms(Mark::UploadsTo, Mark::SimsTo),
        ms(Mark::OutputsFrom, Mark::OutputsTo),
        ms(Mark::OutputsTo, Mark::PassesTo),
        ms(Mark::PassesTo, Mark::ProbesTo),
        ms(Mark::ProbesTo, Mark::MixTo),
    ];
    let whole = ms(Mark::Start, Mark::End);
    let between = (whole - parts.iter().sum::<f32>()).max(0.0);
    let [uploads, sims, outputs, passes, probes, mix] = parts;
    Some(GpuSpans([
        uploads, sims, outputs, passes, probes, mix, between, whole,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spans_are_the_gaps_between_their_marks() {
        let s = spans(&[
            500_000, 1_500_000, 3_500_000, 3_750_000, 4_000_000, 14_000_000, 14_250_000,
            14_500_000, 16_500_000, 17_000_000,
        ])
        .expect("the marks run forward");
        assert_eq!(s[GpuPhase::Uploads], 2.0);
        assert_eq!(s[GpuPhase::Sims], 0.25);
        assert_eq!(s[GpuPhase::Outputs], 10.0);
        assert_eq!(s[GpuPhase::Passes], 0.25);
        assert_eq!(s[GpuPhase::Probes], 0.25);
        assert_eq!(s[GpuPhase::Mix], 2.0);
        assert_eq!(s[GpuPhase::Between], 1.75);
        assert_eq!(s[GpuPhase::Whole], 16.5);
    }

    #[test]
    fn marks_that_run_backwards_are_no_reading() {
        assert_eq!(spans(&[5, 4, 6, 7, 8, 9, 10, 11, 12, 13]), None);
    }

    #[test]
    fn an_unwritten_timestamp_is_no_reading() {
        assert_eq!(spans(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(spans(&[4, 5, 6, 7, 8, 9, 10, 11, 12, 0]), None);
        assert_eq!(between(0, 7), None);
        assert_eq!(between(5, 4), None);
        assert_eq!(between(1_000_000, 3_500_000), Some(2.5));
    }
}
