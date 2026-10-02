// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer the app draws with, on wgpu: the synth's, the editor's viewer and paint timer,
//! and the picture windows' blits, all on one [`Gpu`] on the adapter [`adapter`] picks. No
//! `unsafe` but [`dmabuf`]'s and [`picture`]'s, the two modules the crate root's denial is
//! lifted for.
//!
//! **One device, one queue, and a counter of finished submissions** ([`gpu`]). "Has the GPU
//! finished this?" is a serial compared with [`Gpu::completed`], and the synth submits **an
//! Output, or a run of cheap ones, per submission, never far ahead of the GPU** ([`queue`]),
//! so an editor frame on the same queue waits behind about one Output's pass rather than a
//! tick.
//!
//! **A tick is three phases**, in plan order ([`Renderer::draw`]):
//!
//! 1. the **prelude** — the Outputs made, resized, blanked and linked, the **sources**
//!    uploaded ([`sources`]), the **sims** stepped ([`sims`]);
//! 2. every **Output** that draws, each sampling every Output drawn before it as it was just
//!    drawn and every other as it was last tick;
//! 3. the **coda** — the **workspace passes**, the **probes**, then the **mix** ([`mixer`]).
//!
//! **They go in as submissions of about [`SUBMISSION_MS`] of GPU time.** An Output whose last
//! pass cost more, or that has not been timed, is a submission of its own; a run of cheaper
//! ones shares one, the prelude going in with the first run and the coda with the last. A
//! submission costs the synth thread about a tenth of a millisecond in wgpu and the driver,
//! and at most [`QUEUED_AHEAD`] wait behind the one running, so a submission per Output left
//! the GPU idle after each cheap one while the thread recorded the next (`proposals/wgpu.md`,
//! the headless figures).
//!
//! The GPU timer's **marks** ([`timing`]) bound each phase. Each phase is a file of its own —
//! [`sources`], [`readback`], [`timing`], [`sims`], [`mixer`], [`viewer`], [`dmabuf`] and
//! [`link`] — and this file calls every one of them. [`publish`] is a viewer of its own, as a
//! picture window is, handing pictures to other apps over [`syphon`] and [`ndi`], and the draw
//! never calls it.

pub mod adapter;
#[allow(unsafe_code)]
pub mod dmabuf;
pub mod gpu;
pub mod lease;
pub mod link;
pub mod mixer;
pub mod ndi;
pub mod output;
pub mod picture;
pub mod program;
pub mod publish;
pub mod queue;
pub mod readback;
pub mod ring;
pub mod shared;
pub mod sims;
pub mod sources;
pub mod syphon;
pub mod timing;
pub mod uniforms;
pub mod viewer;

pub use gpu::{Gpu, Ticket};
pub use lease::Lease;
pub use output::OutputRenderer;
pub use queue::QUEUED_AHEAD;
pub use viewer::{Live, Viewer};

pub use mixer::MixerJob;
pub use output::DropCauses;
pub use sims::{SimJob, SimReadback};
pub use sources::SourceJob;
pub use timing::{GpuPhase, GpuSpans, GpuTime};
pub use viewer::Fit;

use crate::compile::Shader;
use crate::compile::wgsl::Sampler;
use crate::graph::{NodeId, PortRef, WorkspaceId};
use output::{Block, Sampled};
use queue::{Recording, Throttle};
use shared::Shared;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use timing::Mark;

/// How many draws may be on the GPU at once: the start of a draw that finds the one this many
/// before it unfinished waits for it. Two, so a tick's first submissions queue behind the end
/// of the one before rather than wait for the GPU to run dry; [`Throttle`] keeps what is queued
/// to [`QUEUED_AHEAD`] submissions whichever tick they belong to, so a tick the GPU cannot keep
/// up with waits there, and every Output slows with it.
pub const TICKS_IN_FLIGHT: usize = 2;

/// The estimated GPU milliseconds a submission of the synth's gathers before it goes in: an
/// Output joins the submission being recorded while its last pass and theirs sum to no more.
/// Near the editor's own paint (1.7 ms at 3440×1440), so what an editor frame waits behind
/// stays about one pass.
pub const SUBMISSION_MS: f32 = 2.0;

/// How long a draw waits for the draw [`TICKS_IN_FLIGHT`] before it. Reaching it means a GPU
/// that has stopped, and the synth goes on at one tick a second rather than not at all.
pub const QUEUE_WAIT: Duration = Duration::from_secs(1);

/// What the draw has recorded and not yet submitted, and whose it is.
struct Submission {
    recording: Recording,
    /// Holds the prelude, which every Output, probe and source and the mix may record into.
    prelude: bool,
    /// The Outputs drawn into it.
    outputs: Vec<NodeId>,
    /// Holds the coda: the workspace passes, the probes and the mix.
    coda: bool,
    /// The sum of its Outputs' last GPU spans, in milliseconds.
    cost: f32,
}

impl Submission {
    fn new(gpu: &Gpu) -> Self {
        Self {
            recording: Recording::new(gpu, "synth"),
            prelude: false,
            outputs: Vec::new(),
            coda: false,
            cost: 0.0,
        }
    }
}

/// A concrete uniform value, resolved on the CPU before the tick is handed to the GPU.
#[derive(Debug, Clone, PartialEq)]
pub enum UniformValue {
    Float(f32),
    /// Which branch a `Uniform` option selects.
    Int(i32),
    /// A count as a Time reads it: its whole part and its fraction.
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    /// A texture published by a node: an Output's frame, or a CPU node's upload.
    NodeTexture(PortRef),
}

/// What one Output does this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Every workspace showing it is closed, or a render holds the graph: it keeps its
    /// targets, its program and its last frame, and nothing is drawn or linked. Closing a
    /// workspace and reopening it must not reallocate anything, because a reallocation is a
    /// frame nobody asked for.
    Suspended,
    /// Nothing is connected: its frame goes black once and it has no program.
    Dark,
    /// Awake and not drawn this tick. Kept ready to draw — a new program is linked, a resize
    /// lands — and what its last frame left is collected; it publishes that last frame.
    Idle,
    /// Drawn this tick, whether or not the GPU has finished its previous frame. See
    /// [docs/rendering.md](../../docs/rendering.md#draws-and-skips).
    Draw,
}

impl OutputMode {
    /// Drawn this tick.
    pub fn draws(self) -> bool {
        self == Self::Draw
    }
}

/// What the main window's preview shows.
///
/// Read by the viewer that paints the preview rather than by the renderer: the synth has no
/// surface to put a picture on, so nothing it draws reaches a screen on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Nothing,
    /// One Output's published frame.
    Output(NodeId),
    /// The mix of the two decks.
    Mixer,
}

/// How many pixels a probe draws: enough to average over the picture, few enough to be
/// free. What a count is divided by to get evaluations per pixel.
pub const PROBE: (u32, u32) = (16, 9);

/// What one Output does this tick: a pipeline is made from the `Shader`, which says what it
/// binds.
pub struct OutputJob {
    pub node: NodeId,
    pub resolution: (u32, u32),
    /// `Some` when the shader changed and the program must be rebuilt: a WGSL module from
    /// `compile::wgsl::build`. `None` leaves the program alone.
    pub shader: Option<Arc<Shader>>,
    /// `compile::source_hash` of the module `uniforms` and `taps` were compiled for.
    pub source: u64,
    pub mode: OutputMode,
    pub uniforms: Vec<(Arc<str>, UniformValue)>,
    /// How many tap slots the module writes.
    pub taps: usize,
    /// Blank the published frame before drawing, keeping the program.
    pub clear: bool,
}

/// An Output's cost probe: its module with every node counting, drawn at [`PROBE`] pixels.
pub struct ProbeJob {
    pub output: NodeId,
    /// `Some` on the tick the probe module was built: `compile::wgsl::build_probe`.
    pub shader: Option<Arc<Shader>>,
    pub source: u64,
    pub uniforms: Vec<(Arc<str>, UniformValue)>,
    pub taps: usize,
}

/// Which pass: a workspace's, and which of its batches — one, unless the workspace binds
/// more textures than one pass may (`compile::PASS_TEXTURES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PassKey {
    pub workspace: WorkspaceId,
    pub batch: usize,
}

impl std::fmt::Display for PassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace {} batch {}", self.workspace, self.batch)
    }
}

/// One workspace's pass: its measurements, and every varying output on it evaluated on a
/// grid of thumbnails, `compile::wgsl::build_pass`. Drawn in the coda, after every Output,
/// where `draws`.
pub struct PassJob {
    pub key: PassKey,
    /// `Some` on the tick the module was built. One-shot, as an Output's is.
    pub shader: Option<Arc<Shader>>,
    pub source: u64,
    /// The target: one fragment per cell.
    pub resolution: (u32, u32),
    /// Where only the measurements' square at the corner is drawn: its size.
    pub region: Option<(u32, u32)>,
    pub uniforms: Vec<(Arc<str>, UniformValue)>,
    pub taps: usize,
    pub draws: bool,
}

/// Everything the renderer needs for one tick. Built by the synth out of its plan.
#[derive(Default)]
pub struct FrameJob {
    pub time: f32,
    /// In the order they are submitted: the plan's.
    pub outputs: Vec<OutputJob>,
    /// Drawn after every Output, for those that draw this tick.
    pub probes: Vec<ProbeJob>,
    /// Drawn after every Output and before the probes, for those whose `draws` says so.
    pub passes: Vec<PassJob>,
    pub display: Display,
    pub mixer: MixerJob,
    pub sources: Vec<SourceJob>,
    pub sims: Vec<SimJob>,
}

/// A texture a published picture names, and the view a viewer samples it through.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Texture {
    pub fn new(texture: wgpu::Texture, view: wgpu::TextureView) -> Self {
        Self { texture, view }
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

/// One picture a viewer may be shown.
#[derive(Debug, Clone)]
pub struct Picture {
    pub texture: Texture,
    pub width: u32,
    pub height: u32,
    /// True for a texture whose first row is the top of the picture — everything a CPU node
    /// uploads. False for one an Output drew, whose first row is the bottom.
    pub flip: bool,
    /// What a viewer samples it through: a `cellularautomata` grid is nearest.
    pub sampler: Sampler,
    /// The synth tick that drew an Output or mix frame.
    pub drawn_tick: Option<u64>,
}

/// What the last tick left for viewers to sample: views, which keep their textures alive, a
/// claim on every Output's and the mix's slot named, which is not drawn into while any clone
/// of this is held, and a claim on every imported frame a source's texture samples, which does
/// not go back to its producer while any clone is held.
#[derive(Debug, Clone, Default)]
pub struct Published {
    pub tick: u64,
    pub outputs: HashMap<NodeId, Picture>,
    pub sources: HashMap<PortRef, Picture>,
    pub mixer: Option<Picture>,
    pub holds: Vec<Lease>,
    pub imports: Vec<dmabuf::Held>,
}

impl Published {
    /// One node's picture: `None` is an Output's own frame, `Some(key)` a CPU node's texture
    /// on that port.
    pub fn picture(&self, node: NodeId, port: Option<&'static str>) -> Option<Picture> {
        match port {
            None => self.outputs.get(&node).cloned(),
            Some(key) => self.sources.get(&PortRef::new(node, key)).cloned(),
        }
    }
}

pub struct Renderer {
    gpu: Gpu,
    shared: Shared,
    outputs: HashMap<NodeId, OutputRenderer>,
    /// One tiny renderer per probed Output, keyed by that Output.
    probes: HashMap<NodeId, OutputRenderer>,
    /// One renderer per workspace pass, keyed by its workspace and batch.
    passes: HashMap<PassKey, OutputRenderer>,
    /// The link error each pass last logged.
    pass_errors: HashMap<PassKey, String>,
    sources: sources::Sources,
    sims: sims::Sims,
    /// The next draw keeps each Output's latest frame for a render: [`Self::park_live`].
    keeping: bool,
    /// The next draw puts each kept frame back, and the ones after it until all are:
    /// [`Self::restore_live`].
    putting_back: bool,
    mixer: mixer::Mixer,
    pub mixer_error: Option<String>,
    stamps: timing::Stamps,
    uniforms: uniforms::Arena,
    throttle: Throttle,
    /// Errors keyed by node, for status lines.
    pub errors: HashMap<NodeId, String>,
    /// Whether a draw places GPU marks.
    measuring: bool,
    /// The last submission of each of the last [`TICKS_IN_FLIGHT`] draws, oldest first.
    ticks: VecDeque<Ticket>,
    /// Draws since the renderer was made.
    tick: u64,
    /// Draws that found the GPU [`TICKS_IN_FLIGHT`] ticks behind and waited.
    waits: u64,
    /// Waits that ran out, of which the first is logged.
    timeouts: u64,
}

impl Renderer {
    /// A renderer on `gpu`, which it keeps and draws through for its whole life.
    pub fn new(gpu: Gpu) -> Result<Self, String> {
        dmabuf::serve(gpu.device());
        Ok(Self {
            shared: Shared::new(&gpu),
            outputs: HashMap::new(),
            probes: HashMap::new(),
            passes: HashMap::new(),
            pass_errors: HashMap::new(),
            sources: sources::Sources::new(&gpu),
            sims: sims::Sims::new(&gpu),
            keeping: false,
            putting_back: false,
            mixer: mixer::Mixer::new(&gpu),
            mixer_error: None,
            stamps: timing::Stamps::new(&gpu),
            uniforms: uniforms::Arena::new(&gpu),
            throttle: Throttle::default(),
            errors: HashMap::new(),
            measuring: false,
            ticks: VecDeque::with_capacity(TICKS_IN_FLIGHT),
            tick: 0,
            waits: 0,
            timeouts: 0,
            gpu,
        })
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// Run one tick: every Output, every probe and the mix. **Nothing here reaches a
    /// screen**; what a window shows is a viewer blitting what [`Self::publish`] names.
    ///
    /// What each draw samples is the latest frame of every Output, looked up as the draw is
    /// made: one drawn earlier this tick as it was just drawn, one drawn later — or the Output
    /// itself — as it was last tick. Before submitting anything the draw waits for the tick
    /// [`TICKS_IN_FLIGHT`] before it where that is unfinished, and each submission waits for
    /// room in [`Throttle`], so a GPU that cannot keep up slows the synth rather than queueing
    /// ahead of the editor. See [docs/rendering.md](../../docs/rendering.md#draws-and-skips).
    pub fn draw(&mut self, job: &FrameJob) {
        self.tick += 1;
        self.wait_for_the_tick_before();
        self.gpu.poll();
        self.stamps.begin(&self.gpu);
        for frame in self.stamps.take_frames() {
            if let Some(o) = self.outputs.get_mut(&frame.node) {
                o.timer().record(&frame);
            }
        }
        let measuring = self.measuring;
        let mut last = None;

        // The prelude: what the Outputs are, then the sources and the simulations, which every
        // Output after it samples as this tick left them.
        let mut next = Submission::new(&self.gpu);
        next.prelude = true;
        let prelude = &mut next.recording;
        self.stamps.mark(prelude, Mark::Start, measuring);
        self.sync(job, prelude);
        self.stamps.mark(prelude, Mark::UploadsFrom, measuring);
        self.sources.sync(
            &self.gpu,
            &self.shared,
            prelude,
            &job.sources,
            self.gpu.completed(),
        );
        self.stamps.mark(prelude, Mark::UploadsTo, measuring);
        self.sims.sync(
            &self.gpu,
            &self.shared,
            prelude,
            &job.sims,
            &mut self.errors,
        );
        self.stamps.mark(prelude, Mark::SimsTo, measuring);
        self.stamps.mark(prelude, Mark::OutputsFrom, measuring);

        // Every draw's uniforms, in one buffer, before the first of them is submitted.
        let drawing: HashSet<NodeId> = job
            .outputs
            .iter()
            .filter(|o| o.mode.draws())
            .map(|o| o.node)
            .collect();
        let (offsets, probe_offsets, pass_offsets) = self.pack_uniforms(job, &drawing);

        let textures: HashMap<PortRef, wgpu::TextureView> =
            self.sources.views().chain(self.sims.views()).collect();
        let mut latest: HashMap<NodeId, wgpu::TextureView> = self
            .outputs
            .iter()
            .map(|(id, o)| (*id, o.latest().view.clone()))
            .collect();

        // The Outputs in plan order: one costing more than `SUBMISSION_MS`, or never timed, in
        // a submission of its own, and a run of cheaper ones in one between them.
        let completed = self.gpu.completed();
        for out in &job.outputs {
            let Some(renderer) = self.outputs.get_mut(&out.node) else {
                continue;
            };
            match out.mode {
                OutputMode::Suspended | OutputMode::Dark => continue,
                OutputMode::Idle => {
                    renderer.want(Some(out.source));
                    renderer.poll_frames(completed);
                    renderer.rest(&self.gpu);
                    continue;
                }
                OutputMode::Draw => {}
            }
            let cost = renderer.gpu_time().map_or(f32::INFINITY, |t| t.latest);
            if next.recording.used() && next.cost + cost > SUBMISSION_MS {
                self.submit(&mut next, &mut last);
            }
            let (Some(renderer), Some(&offset), Some(buffer)) = (
                self.outputs.get_mut(&out.node),
                offsets.get(&out.node),
                self.uniforms.buffer(),
            ) else {
                continue;
            };
            let timestamps = self.stamps.span(out.node, renderer.timer().id());
            let drew = renderer.encode(
                &self.gpu,
                &self.shared,
                next.recording.encoder(),
                Block { buffer, offset },
                &Sampled {
                    textures: &textures,
                    latest: &latest,
                },
                timestamps,
            );
            if !drew {
                self.stamps.withdraw(out.node);
                continue;
            }
            latest.insert(out.node, renderer.latest().view.clone());
            next.outputs.push(out.node);
            next.cost += cost;
        }

        // The coda, in the last Outputs' submission: the workspace passes, sampling every
        // frame as this tick left it, the probes of the Outputs drawn this tick, then the
        // mix of the frames just drawn.
        next.coda = true;
        let coda = &mut next.recording;
        self.stamps.mark(coda, Mark::OutputsTo, measuring);
        for pass in &job.passes {
            let (Some(renderer), Some(&offset), Some(buffer)) = (
                self.passes.get_mut(&pass.key),
                pass_offsets.get(&pass.key),
                self.uniforms.buffer(),
            ) else {
                continue;
            };
            renderer.encode(
                &self.gpu,
                &self.shared,
                coda.encoder(),
                Block { buffer, offset },
                &Sampled {
                    textures: &textures,
                    latest: &latest,
                },
                None,
            );
        }
        self.stamps.mark(coda, Mark::PassesTo, measuring);
        for probe in &job.probes {
            if !drawing.contains(&probe.output) {
                continue;
            }
            let (Some(renderer), Some(&offset), Some(buffer)) = (
                self.probes.get_mut(&probe.output),
                probe_offsets.get(&probe.output),
                self.uniforms.buffer(),
            ) else {
                continue;
            };
            renderer.encode(
                &self.gpu,
                &self.shared,
                coda.encoder(),
                Block { buffer, offset },
                &Sampled {
                    textures: &textures,
                    latest: &latest,
                },
                None,
            );
        }
        self.stamps.mark(coda, Mark::ProbesTo, measuring);
        let deck = |id: Option<NodeId>| {
            let o = self.outputs.get(&id?)?;
            o.has_program().then(|| {
                let (width, height) = o.size();
                mixer::Deck {
                    view: &o.latest().view,
                    width,
                    height,
                }
            })
        };
        let (a, b) = (deck(job.mixer.a), deck(job.mixer.b));
        self.mixer
            .draw(&self.gpu, &self.shared, coda, &job.mixer, a, b);
        self.stamps.mark(coda, Mark::MixTo, measuring);
        self.stamps.mark(coda, Mark::End, measuring);
        self.stamps.close(coda);
        self.submit(&mut next, &mut last);

        if let Some(ticket) = last {
            while self.ticks.len() >= TICKS_IN_FLIGHT {
                self.ticks.pop_front();
            }
            self.ticks.push_back(ticket);
        }
    }

    /// Submit what `next` holds once [`Throttle`] has room, tell everything recorded into it
    /// which submission carries it, and leave `next` empty.
    fn submit(&mut self, next: &mut Submission, last: &mut Option<Ticket>) {
        let done = std::mem::replace(next, Submission::new(&self.gpu));
        let Some(ticket) = self.throttle.submit(&self.gpu, done.recording) else {
            return;
        };
        if done.prelude {
            for o in self
                .outputs
                .values_mut()
                .chain(self.probes.values_mut())
                .chain(self.passes.values_mut())
            {
                o.submitted(&ticket);
            }
            self.sources.submitted(&ticket);
            self.mixer.submitted(&ticket);
        } else {
            for id in &done.outputs {
                if let Some(o) = self.outputs.get_mut(id) {
                    o.submitted(&ticket);
                }
            }
        }
        if done.coda {
            for o in self.probes.values_mut().chain(self.passes.values_mut()) {
                o.submitted(&ticket);
            }
            self.mixer.submitted(&ticket);
        }
        *last = Some(ticket);
    }

    /// Wait for the draw [`TICKS_IN_FLIGHT`] before this one, where it is unfinished.
    fn wait_for_the_tick_before(&mut self) {
        if self.ticks.len() < TICKS_IN_FLIGHT {
            return;
        }
        let Some(oldest) = self.ticks.front() else {
            return;
        };
        if self.gpu.is_done(oldest.serial) {
            return;
        }
        self.gpu.poll();
        if self.gpu.is_done(oldest.serial) {
            return;
        }
        self.waits += 1;
        if self.gpu.wait(oldest, QUEUE_WAIT).is_err() {
            self.timeouts += 1;
            if self.timeouts == 1 {
                log::warn!(
                    "the GPU did not finish a tick in {} ms; the synth goes on without it",
                    QUEUE_WAIT.as_millis()
                );
            }
        }
    }

    /// Pack every drawing Output's, probe's and pass's uniform block into the tick's buffer
    /// and put it up, returning each Output's offset, each probe's by the Output it probes,
    /// and each pass's by its key.
    fn pack_uniforms(
        &mut self,
        job: &FrameJob,
        drawing: &HashSet<NodeId>,
    ) -> (
        HashMap<NodeId, u32>,
        HashMap<NodeId, u32>,
        HashMap<PassKey, u32>,
    ) {
        self.uniforms.clear();
        let mut offsets = HashMap::new();
        let mut probe_offsets = HashMap::new();
        let mut pass_offsets = HashMap::new();
        for pass in &job.passes {
            if !pass.draws {
                continue;
            }
            let Some(renderer) = self.passes.get_mut(&pass.key) else {
                continue;
            };
            if !renderer.adopt(pass.source, &pass.uniforms, pass.taps) {
                continue;
            }
            if let Some(offset) = renderer.push_block(&mut self.uniforms, job.time) {
                pass_offsets.insert(pass.key, offset);
            }
        }
        for out in &job.outputs {
            if !out.mode.draws() {
                continue;
            }
            let Some(renderer) = self.outputs.get_mut(&out.node) else {
                continue;
            };
            renderer.set_tick(self.tick);
            if !renderer.adopt(out.source, &out.uniforms, out.taps) {
                continue;
            }
            if let Some(offset) = renderer.push_block(&mut self.uniforms, job.time) {
                offsets.insert(out.node, offset);
            }
        }
        for probe in &job.probes {
            if !drawing.contains(&probe.output) {
                continue;
            }
            let Some(renderer) = self.probes.get_mut(&probe.output) else {
                continue;
            };
            if !renderer.adopt(probe.source, &probe.uniforms, probe.taps) {
                continue;
            }
            if let Some(offset) = renderer.push_block(&mut self.uniforms, job.time) {
                probe_offsets.insert(probe.output, offset);
            }
        }
        self.uniforms.upload(&self.gpu);
        (offsets, probe_offsets, pass_offsets)
    }

    /// Make, resize and drop `OutputRenderer`s to match the job, land and send programs, and
    /// make or resize the mix, recording what that needs into the prelude.
    fn sync(&mut self, job: &FrameJob, prelude: &mut Recording) {
        let wanted: HashSet<NodeId> = job.outputs.iter().map(|o| o.node).collect();
        self.outputs.retain(|id, _| wanted.contains(id));

        if let Err(err) = self
            .mixer
            .sync(&self.gpu, &self.shared, prelude, &job.mixer, self.tick)
        {
            self.mixer_error = Some(err);
        } else {
            self.mixer_error = None;
        }

        // A probe lives exactly as long as its job.
        let probed: HashSet<NodeId> = job.probes.iter().map(|p| p.output).collect();
        self.probes.retain(|id, _| probed.contains(id));
        for probe in &job.probes {
            let renderer = self.probes.entry(probe.output).or_insert_with(|| {
                OutputRenderer::new(&self.gpu, prelude.encoder(), PROBE.0, PROBE.1)
            });
            renderer.poll_shader();
            if let Some(shader) = &probe.shader {
                renderer.set_shader(&self.gpu, shader);
            }
        }

        // A pass lives as long as its workspace is open, drawn or not, so a tab looked at
        // again finds its program linked.
        let passing: HashSet<PassKey> = job.passes.iter().map(|p| p.key).collect();
        self.passes.retain(|id, _| passing.contains(id));
        self.pass_errors.retain(|id, _| passing.contains(id));
        for pass in &job.passes {
            let (width, height) = pass.resolution;
            let renderer = self.passes.entry(pass.key).or_insert_with(|| {
                let mut r = OutputRenderer::new(&self.gpu, prelude.encoder(), width, height);
                r.set_nominal(crate::nodes::output::DEFAULT_RESOLUTION);
                r
            });
            renderer.poll_shader();
            renderer.resize(&self.gpu, &self.shared, prelude.encoder(), width, height);
            renderer.set_region(pass.region);
            if let Some(shader) = &pass.shader {
                renderer.set_shader(&self.gpu, shader);
            }
            if !pass.draws {
                renderer.rest(&self.gpu);
            }
            // Logged once: nothing on screen names a workspace's pass, and its thumbnails
            // standing still say the rest.
            match renderer.error() {
                Some(err) if self.pass_errors.get(&pass.key) != Some(err) => {
                    log::warn!("the pass of {} did not link: {err}", pass.key);
                    self.pass_errors.insert(pass.key, err.clone());
                }
                Some(_) => {}
                None => {
                    self.pass_errors.remove(&pass.key);
                }
            }
        }

        let mut put_back = true;
        for out in &job.outputs {
            let renderer = self.outputs.entry(out.node).or_insert_with(|| {
                OutputRenderer::new(
                    &self.gpu,
                    prelude.encoder(),
                    out.resolution.0,
                    out.resolution.1,
                )
            });
            // A render's frames go into the live show's rings: the frame each found is kept
            // first, and put back first when it ends.
            if self.keeping {
                renderer.keep(&self.gpu, prelude.encoder());
            }
            if self.putting_back {
                put_back &= renderer.put_back(&self.gpu, prelude.encoder());
            }
            renderer.set_tick(self.tick);
            // Before anything else: land a program that finished linking since the last tick,
            // so the tick a link completes on is the tick that uses it.
            renderer.poll_shader();
            renderer.readbacks().poll(&self.gpu);
            renderer.sweep();
            renderer.resize(
                &self.gpu,
                &self.shared,
                prelude.encoder(),
                out.resolution.0,
                out.resolution.1,
            );
            if out.clear {
                renderer.clear_frame(&self.gpu, prelude.encoder());
            }
            match out.mode {
                OutputMode::Suspended => {}
                OutputMode::Dark => renderer.go_dark(&self.gpu, prelude.encoder()),
                OutputMode::Idle | OutputMode::Draw => {
                    if let Some(shader) = &out.shader {
                        renderer.set_shader(&self.gpu, shader);
                    }
                }
            }
            match renderer.error() {
                Some(err) => {
                    self.errors.insert(out.node, err.clone());
                }
                None => {
                    self.errors.remove(&out.node);
                }
            }
        }
        self.keeping = false;
        self.putting_back &= !put_back;
    }

    /// What viewers may be shown now: for each Output and the mix, the newest frame whose
    /// submission has finished, and each source's texture. Chosen without waiting: every
    /// frame still on the GPU is asked once, through the completed serial.
    pub fn publish(&mut self) -> Published {
        self.gpu.poll();
        let completed = self.gpu.completed();
        for o in self.outputs.values_mut() {
            o.poll_frames(completed);
        }
        self.mixer.poll_frames(completed);
        let mut out = Published {
            tick: self.tick,
            ..Published::default()
        };
        for (id, o) in &self.outputs {
            if !o.has_program() {
                continue;
            }
            let Some((slot, lease)) = o.shown() else {
                continue;
            };
            out.holds.push(lease);
            out.outputs.insert(
                *id,
                Picture {
                    texture: Texture::new(slot.texture.clone(), slot.view.clone()),
                    width: slot.width,
                    height: slot.height,
                    flip: false,
                    sampler: Sampler::MirrorLinear,
                    drawn_tick: o.shown_tick(),
                },
            );
        }
        self.sources.publish(&mut out);
        self.sims.publish(&mut out);
        self.mixer.publish(&mut out);
        out
    }

    /// Let go of everything that belongs to the project's nodes: every Output, probe, upload
    /// and world. What is left is the run's — the mix and the shared stages.
    pub fn forget_project(&mut self) {
        self.outputs.clear();
        self.probes.clear();
        self.passes.clear();
        self.sources.forget(self.gpu.completed());
        self.sims.forget();
        self.errors.clear();
    }

    /// Put the live show's GPU state aside for a render: every simulation's world
    /// ([`sims::Sims::park`]) now, and every Output's latest frame, which feedback reads next,
    /// copied in the prelude of the next draw, before the render's first frame blanks or
    /// draws over it ([`OutputRenderer::keep`]).
    pub fn park_live(&mut self) {
        self.sims.park();
        self.keeping = true;
    }

    /// What [`Self::park_live`] put aside, back as the render found it: the worlds now, and
    /// each Output's kept frame the latest again in the prelude of the next draw, before any
    /// Output samples it ([`OutputRenderer::put_back`]).
    pub fn restore_live(&mut self) {
        self.sims.restore();
        self.keeping = false;
        self.putting_back = true;
    }

    // ------------------------------------------------------------------ what callers ask

    /// The texture a node last drew or uploaded: an Output's latest frame, a CPU node's
    /// upload or a simulation's picture. For tests that read pixels back.
    pub fn texture_of(&self, node: NodeId) -> Option<wgpu::Texture> {
        self.outputs
            .get(&node)
            .map(|o| o.latest().texture.clone())
            .or_else(|| self.sources.texture_of(node))
            .or_else(|| self.sims.texture_of(node))
    }

    /// Every target this Output holds. For a test asserting that something allocated nothing.
    pub fn targets_of(&self, node: NodeId) -> Vec<wgpu::Texture> {
        self.outputs
            .get(&node)
            .map(OutputRenderer::targets)
            .unwrap_or_default()
    }

    /// **Test accessor.** An Output's latest frame as it lies, `Rgba16Float`, rows bottom
    /// first, read back and waited for.
    #[doc(hidden)]
    pub fn read_output(&self, node: NodeId) -> Option<(u32, u32, Vec<u8>)> {
        let texture = self.outputs.get(&node)?.latest().texture.clone();
        let bytes = readback::read_texture(&self.gpu, &texture);
        Some((texture.width(), texture.height(), bytes))
    }

    /// **Test accessor.** What one simulation holds on the GPU, read back and waited for.
    #[doc(hidden)]
    pub fn read_simulation(&self, port: PortRef) -> Option<SimReadback> {
        self.sims.read(&self.gpu, port)
    }

    pub fn mixer_texture(&self) -> Option<wgpu::Texture> {
        self.mixer.texture()
    }

    pub fn mixer_targets(&self) -> Vec<wgpu::Texture> {
        self.mixer.targets()
    }

    pub fn mixer_size(&self) -> Option<(u32, u32)> {
        self.mixer.size()
    }

    /// Every Output's tap words from the last frame that finished, each taken once.
    pub fn take_taps(&mut self) -> Vec<(NodeId, Vec<u32>)> {
        self.outputs
            .iter_mut()
            .filter_map(|(id, o)| o.readbacks().take_taps().map(|w| (*id, w)))
            .collect()
    }

    /// Every probe's counts from the last frame that finished, keyed by the Output probed.
    pub fn take_probes(&mut self) -> Vec<(NodeId, Vec<u32>)> {
        self.probes
            .iter_mut()
            .filter_map(|(id, o)| o.readbacks().take_taps().map(|w| (*id, w)))
            .collect()
    }

    /// Every workspace pass's words from the last frame that finished, by its key.
    pub fn take_passes(&mut self) -> Vec<(PassKey, Vec<u32>)> {
        self.passes
            .iter_mut()
            .filter_map(|(id, o)| o.readbacks().take_taps().map(|w| (*id, w)))
            .collect()
    }

    /// **Test accessor.** Whether this workspace's pass has a program to draw with.
    #[doc(hidden)]
    pub fn pass_linked(&self, key: PassKey) -> bool {
        self.passes
            .get(&key)
            .is_some_and(OutputRenderer::has_program)
    }

    /// True when this workspace's pass has no program, none linking and no error.
    pub fn pass_awaiting(&self, key: PassKey) -> bool {
        self.passes
            .get(&key)
            .is_none_or(|o| !o.has_program() && !o.is_linking() && o.error().is_none())
    }

    /// True when this Output's probe has no program, none linking and no error.
    pub fn probe_awaiting(&self, output: NodeId) -> bool {
        self.probes
            .get(&output)
            .is_none_or(|o| !o.has_program() && !o.is_linking() && o.error().is_none())
    }

    /// Ask this Output for a picture of what it publishes; it lands through
    /// [`Self::take_thumbnails`], and nothing waits for it.
    pub fn request_thumbnail(&mut self, node: NodeId) {
        if let Some(o) = self.outputs.get_mut(&node) {
            o.readbacks().request_thumbnail();
        }
    }

    pub fn thumbnail_pending(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_some_and(|o| o.readbacks_ref().thumbnail_pending())
    }

    pub fn take_thumbnails(&mut self) -> Vec<(NodeId, Vec<u8>)> {
        self.outputs
            .iter_mut()
            .filter_map(|(id, o)| o.readbacks().take_thumbnail().map(|b| (*id, b)))
            .collect()
    }

    /// Ask this Output for a Snap: the frame it publishes, at its own size.
    pub fn request_snap(&mut self, node: NodeId) {
        if let Some(o) = self.outputs.get_mut(&node) {
            o.readbacks().request_snap();
        }
    }

    pub fn snap_pending(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_some_and(|o| o.readbacks_ref().snap_pending())
    }

    pub fn take_snaps(&mut self) -> Vec<(NodeId, u32, u32, Vec<u8>)> {
        self.outputs
            .iter_mut()
            .filter_map(|(id, o)| {
                let (w, h, bytes) = o.readbacks().take_snap()?;
                Some((*id, w, h, bytes))
            })
            .collect()
    }

    /// Whether a picture has been asked of this Output that only a draw makes.
    pub fn wants_picture(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_some_and(OutputRenderer::wants_picture)
    }

    /// Read back every frame this Output publishes from now on, or stop. False for an Output
    /// with no renderer yet.
    pub fn set_capturing(&mut self, node: NodeId, on: bool, scale: u32) -> bool {
        match self.outputs.get_mut(&node) {
            Some(o) => {
                o.readbacks().set_capturing(on, scale);
                true
            }
            None => false,
        }
    }

    pub fn captures_issued(&self, node: NodeId) -> u64 {
        self.outputs
            .get(&node)
            .map_or(0, |o| o.readbacks_ref().captures_issued())
    }

    pub fn capture_pending(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_some_and(|o| o.readbacks_ref().capture_pending())
    }

    pub fn take_captured(&mut self, node: NodeId) -> Vec<Vec<u8>> {
        self.outputs
            .get_mut(&node)
            .map(|o| o.readbacks().take_captured())
            .unwrap_or_default()
    }

    pub fn dropped_frames(&self, node: NodeId) -> u64 {
        self.drops(node).total()
    }

    pub fn drops(&self, node: NodeId) -> DropCauses {
        self.outputs
            .get(&node)
            .map_or_else(DropCauses::default, OutputRenderer::drops)
    }

    /// Every drop in the project, by cause, the mix's in `held`.
    pub fn all_drops(&self) -> DropCauses {
        let mut total = DropCauses::default();
        for o in self.outputs.values() {
            total.add(o.drops());
        }
        total.held += self.mixer.dropped();
        total
    }

    pub fn mixer_drops(&self) -> u64 {
        self.mixer.dropped()
    }

    /// Draws that found the GPU [`TICKS_IN_FLIGHT`] ticks behind and waited.
    pub fn waits(&self) -> u64 {
        self.waits
    }

    /// Submissions that found [`QUEUED_AHEAD`] of the synth's earlier ones still on the GPU
    /// and waited for room.
    pub fn throttled(&self) -> u64 {
        self.throttle.waits
    }

    /// The most synth submissions ever still on the GPU when another was submitted.
    pub fn most_queued_ahead(&self) -> usize {
        self.throttle.most_ahead
    }

    /// Submissions the synth has made since the renderer was made.
    pub fn submissions(&self) -> u64 {
        self.throttle.submissions
    }

    pub fn gpu_time(&self, node: NodeId) -> Option<GpuTime> {
        self.outputs.get(&node).and_then(OutputRenderer::gpu_time)
    }

    pub fn is_linking(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_some_and(OutputRenderer::is_linking)
    }

    /// **Test accessor.** Leave this Output's link in flight unfinished until let go.
    #[doc(hidden)]
    pub fn hold_link(&mut self, node: NodeId, hold: bool) {
        if let Some(o) = self.outputs.get_mut(&node) {
            o.hold_link(hold);
        }
    }

    /// True when this Output has no program, none linking and no error to explain it: the
    /// caller sends the source again.
    pub fn awaiting_shader(&self, node: NodeId) -> bool {
        self.outputs
            .get(&node)
            .is_none_or(|o| !o.has_program() && !o.is_linking() && o.error().is_none())
    }

    /// Time each draw's phases from now on, or stop.
    pub fn set_measuring(&mut self, on: bool) {
        self.measuring = on;
    }

    pub fn take_gpu_spans(&mut self) -> Vec<GpuSpans> {
        self.stamps.take()
    }
}
