// SPDX-License-Identifier: AGPL-3.0-or-later

//! Worlds a node steps on the GPU, in compute kernels it writes — the draw's **sims** phase,
//! after the uploads and before any Output, so an Output samples the world this tick left.
//!
//! Per published port a
//! [`World`] holds the agents in a storage buffer, the field in a pair of `R32Float` storage
//! textures, the arrival counts in a pair of storage buffers of atomics, a few words of state,
//! and the `Rgba8Unorm` picture every consumer samples; each tick the node published runs its
//! passes in order. Nothing here knows what an agent or a scent is. What is fixed is the shape
//! of the resources and where each kernel finds them — [`source`]'s prelude.
//!
//! **Two groups.** Group 0 is the world, and each world makes two bind groups once: front to
//! back and back to front, so a pass that [`Kernel::flips`] picks the other one next rather
//! than anything being copied or rebound. Group 1 is one uniform struct `u` per pass — the
//! pass's own numbers (`u_size`, `u_from`, `u_seed`…) and then the simulation's
//! [`Simulation::params`] by name, in the order it lists them — packed for every pass of the
//! tick into one buffer and bound at a dynamic offset. The struct is written by [`source`]
//! from the names, so a kernel declares none of its uniforms and the renderer needs no
//! reflection to fill them.
//!
//! **Pipelines are the renderer's, one per kernel**, keyed by the kernel's address and the
//! names of the numbers it is handed. The first pass naming one sends it to a thread of its
//! own, `sim-linker`, since creating a compute pipeline compiles it before it returns; a tick
//! whose kernels are not all linked stays queued with the ticks after it, so a world steps late
//! on its first frames rather than skipping steps, and the same clock still makes the same
//! world. A kernel that fails is its node's status line.
//!
//! **No barrier is written here.** wgpu places one between two dispatches that touch the same
//! storage resource — a `read_write` binding is an exclusive use, never an ordered one — and
//! between the last dispatch and an Output pass sampling the picture.
//!
//! **A change of shape reallocates before the passes that asked for it**, and never shows an
//! empty picture — *zero flash*: a new field is the old one resampled nearest by a compute
//! pass (an `R32Float` texture is not filterable, so it is read with `textureLoad`), the counts
//! start at zero, the new picture is the old one scaled in by `Shared::carry`'s linear pass,
//! and a new population keeps the agents it can. The picture is drawn in place every tick, as
//! an upload is, and a viewer's blit is ordered against it by the one queue, so it sees the
//! picture before or after a tick's passes and never between. A `Published` naming an old
//! picture keeps it alive through its view.

use super::gpu::Gpu;
use super::queue::Recording;
use super::ring::Format;
use super::shared::Shared;
use super::{Picture, Published, Texture};
use crate::compile::wgsl::Sampler;
use crate::graph::{NodeId, PortRef};
use crate::nodes::sim::{Domain, Kernel, Pass, STATE_WORDS, Simulation, TILE};
use crate::nodes::{TextureFilter, TextureWrap};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::sync::mpsc;

/// One simulated texture output: this tick of its world, and how its picture is sampled —
/// carried for the reason [`super::SourceJob`] carries it.
pub struct SimJob {
    pub port: PortRef,
    pub sim: Simulation,
    pub wrap: TextureWrap,
    pub filter: TextureFilter,
}

/// What a simulation holds on the GPU, read back for a test. Rows bottom first, as the images
/// are addressed.
#[derive(Debug, Clone, PartialEq)]
pub struct SimReadback {
    pub size: u32,
    pub agents: Vec<[f32; 4]>,
    pub state: Vec<u32>,
    /// The field a kernel reads as `field`.
    pub field: Vec<f32>,
    /// `arrivals`, and `arrivals_next`: after a flipping pass, the counts of the step before.
    pub arrivals: Vec<u32>,
    pub arrivals_next: Vec<u32>,
    /// RGBA8.
    pub picture: Vec<u8>,
    /// Ticks still waiting for their kernels to link.
    pub queued: usize,
}

/// Every kernel's entry point, written by [`source`].
pub const ENTRY: &str = "cs_main";

/// Agents a workgroup of a [`Domain::Agents`] kernel runs.
const AGENT_GROUP: u32 = 64;

/// How many ticks a world may queue while its kernels link. Linking takes a fraction of a
/// second; past this something is wrong, and the oldest tick is dropped rather than the queue
/// growing for the rest of the run.
const MAX_QUEUED: usize = 1024;

/// The pass's own numbers, first in every kernel's uniform struct.
const PASS_FIELDS: [(&str, &str); 7] = [
    ("u_size", "i32"),
    ("u_agents", "i32"),
    ("u_from", "i32"),
    ("u_to", "i32"),
    ("u_stride", "i32"),
    ("u_seed", "u32"),
    ("u_arg", "f32"),
];

/// Bytes of one pass's uniform struct, at most: room for the pass's numbers and
/// [`MAX_PARAMS`] of the node's.
const SLOT: u64 = 256;

/// The most numbers a simulation may hand its kernels.
pub const MAX_PARAMS: usize = SLOT as usize / 4 - PASS_FIELDS.len();

/// The world's resources as every kernel sees them, in group 0, and its private copies of the
/// built-ins a [`Domain::Tiles`] kernel reads. See [`crate::nodes::sim`] for the table.
const RESOURCES: &str = "
@group(0) @binding(0) var<storage, read_write> agents: array<vec4f>;
@group(0) @binding(1) var<storage, read_write> state: array<atomic<u32>>;
@group(0) @binding(2) var field: texture_storage_2d<r32float, read_write>;
@group(0) @binding(3) var field_next: texture_storage_2d<r32float, read_write>;
@group(0) @binding(4) var<storage, read_write> arrivals: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> arrivals_next: array<atomic<u32>>;
@group(0) @binding(6) var picture: texture_storage_2d<rgba8unorm, write>;

// The workgroup, and the invocation's place in it — `workgroup_id`, `local_invocation_id`
// and `local_invocation_index` — which WGSL hands to the entry point alone.
var<private> sim_group: vec2i;
var<private> sim_local: vec2i;
var<private> sim_local_index: i32;

// A cell's place in a per-cell buffer.
fn sim_index(c: vec2i) -> i32 {
    return c.y * u.u_size + c.x;
}
";

/// The whole of one kernel's module: the uniform struct with `params` after the pass's own
/// numbers, the resources, the node's WGSL, and an entry point in its domain's workgroup that
/// calls the function the domain names.
pub fn source(kernel: &Kernel, params: &[&str]) -> String {
    let (layout, builtins, main) = match kernel.over {
        Domain::Agents => (
            format!("@workgroup_size({AGENT_GROUP})"),
            "@builtin(global_invocation_id) id: vec3u",
            "    let i = u.u_from + i32(id.x) * u.u_stride;
    if (i >= u.u_to) {
        return;
    }
    run_agent(i);",
        ),
        Domain::Cells => (
            format!("@workgroup_size({TILE}, {TILE})"),
            "@builtin(global_invocation_id) id: vec3u",
            "    let c = vec2i(id.xy);
    if (c.x >= u.u_size || c.y >= u.u_size) {
        return;
    }
    run_cell(c);",
        ),
        Domain::Tiles => (
            format!("@workgroup_size({TILE}, {TILE})"),
            "@builtin(global_invocation_id) id: vec3u,
    @builtin(workgroup_id) group: vec3u,
    @builtin(local_invocation_id) local: vec3u,
    @builtin(local_invocation_index) local_index: u32",
            "    sim_group = vec2i(group.xy);
    sim_local = vec2i(local.xy);
    sim_local_index = i32(local_index);
    let c = vec2i(id.xy);
    run_tile(c, c.x < u.u_size && c.y < u.u_size);",
        ),
        Domain::Once => ("@workgroup_size(1)".to_string(), "", "    run_once();"),
    };
    let mut out = String::from("struct Uniforms {\n");
    for (name, ty) in PASS_FIELDS {
        writeln!(out, "    {name}: {ty},").unwrap();
    }
    for name in params {
        writeln!(out, "    {name}: f32,").unwrap();
    }
    write!(
        out,
        "}}
@group(1) @binding(0) var<uniform> u: Uniforms;
const SIM_TILE: i32 = {TILE};
{RESOURCES}
{}
{}
@compute {layout}
fn {ENTRY}({builtins}) {{
{main}
}}
",
        kernel.wgsl_common, kernel.wgsl
    )
    .unwrap();
    out
}

/// The renderer's own stage for a change of shape: the old field resampled nearest into the
/// new, each new cell the old cell its center falls in — a nearest blit, which an `R32Float`
/// texture cannot be filtered for.
pub const RESAMPLE: &str = "
@group(0) @binding(0) var old_field: texture_2d<f32>;
@group(0) @binding(1) var new_field: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3u) {
    let new_size = textureDimensions(new_field);
    if (id.x >= new_size.x || id.y >= new_size.y) {
        return;
    }
    let old_size = textureDimensions(old_field);
    let at = vec2u((vec2f(id.xy) + 0.5) * vec2f(old_size) / vec2f(new_size));
    let value = textureLoad(old_field, vec2i(min(at, old_size - 1u)), 0);
    textureStore(new_field, vec2i(id.xy), value);
}
";

// ------------------------------------------------------------------------------ the kernels

/// A kernel's pipeline, by the kernel's address and the set of names its numbers were laid out
/// from.
type Key = (usize, usize);

fn address(kernel: &'static Kernel) -> usize {
    std::ptr::from_ref(kernel) as usize
}

enum Program {
    Linking,
    Ready(wgpu::ComputePipeline),
    Failed(String),
}

/// A kernel on its way to the linker.
struct Request {
    key: Key,
    name: &'static str,
    source: String,
}

/// The `sim-linker` thread: requests in, pipelines out.
struct Linker {
    requests: mpsc::Sender<Request>,
    linked: mpsc::Receiver<(Key, Result<wgpu::ComputePipeline, String>)>,
}

impl Linker {
    fn spawn(gpu: &Gpu, layout: &wgpu::PipelineLayout) -> Option<Self> {
        let (requests, inbox) = mpsc::channel::<Request>();
        let (outbox, linked) = mpsc::channel();
        let (gpu, layout) = (gpu.clone(), layout.clone());
        let spawned = std::thread::Builder::new()
            .name("sim-linker".to_string())
            .spawn(move || {
                // Ends when the renderer drops its sender, or stops listening.
                for request in inbox {
                    let linked = link(&gpu, &layout, request.name, &request.source);
                    if outbox.send((request.key, linked)).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(_) => Some(Self { requests, linked }),
            Err(e) => {
                log::error!("no simulation linker thread: {e}");
                None
            }
        }
    }
}

/// One kernel's pipeline, or why the device refused it. Waits for the compile: the linker's.
fn link(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    name: &str,
    source: &str,
) -> Result<wgpu::ComputePipeline, String> {
    let device = gpu.device();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(name),
        layout: Some(layout),
        module: &module,
        entry_point: Some(ENTRY),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    match crate::render::adapter::block_on(scope.pop()) {
        Some(error) => Err(format!("kernel {name}: {error}")),
        None => Ok(pipeline),
    }
}

/// Every kernel a simulation has asked for.
#[derive(Default)]
struct Kernels {
    programs: HashMap<Key, (&'static str, Program)>,
    /// Every set of names a simulation has handed its kernels, by the index a [`Key`] holds.
    param_sets: Vec<Vec<&'static str>>,
    linker: Option<Linker>,
}

impl Kernels {
    /// The index of the set of names `params` carries, interned.
    fn set_of(&mut self, params: &[(&'static str, f32)]) -> usize {
        let same = |set: &Vec<&'static str>| {
            set.len() == params.len() && set.iter().zip(params).all(|(a, (b, _))| a == b)
        };
        if let Some(at) = self.param_sets.iter().position(same) {
            return at;
        }
        self.param_sets
            .push(params.iter().map(|(name, _)| *name).collect());
        self.param_sets.len() - 1
    }

    /// Send every kernel `sim` names that has not been sent. Never waits.
    fn want(&mut self, gpu: &Gpu, layout: &wgpu::PipelineLayout, sim: &Simulation) {
        let set = self.set_of(&sim.params);
        for pass in &sim.passes {
            let key = (address(pass.kernel), set);
            if self.programs.contains_key(&key) {
                continue;
            }
            let program = if sim.params.len() > MAX_PARAMS {
                Program::Failed(format!(
                    "kernel {}: {} numbers, of at most {MAX_PARAMS}",
                    pass.kernel.name,
                    sim.params.len()
                ))
            } else {
                if self.linker.is_none() {
                    self.linker = Linker::spawn(gpu, layout);
                }
                let request = Request {
                    key,
                    name: pass.kernel.name,
                    source: source(pass.kernel, &self.param_sets[set]),
                };
                match &self.linker {
                    Some(linker) if linker.requests.send(request).is_ok() => Program::Linking,
                    _ => Program::Failed(format!("kernel {}: no linker", pass.kernel.name)),
                }
            };
            self.programs.insert(key, (pass.kernel.name, program));
        }
    }

    /// Land every link that has finished. Never waits.
    fn poll(&mut self) {
        let Some(linker) = &self.linker else {
            return;
        };
        while let Ok((key, linked)) = linker.linked.try_recv() {
            let Some((_, program)) = self.programs.get_mut(&key) else {
                continue;
            };
            *program = match linked {
                Ok(pipeline) => Program::Ready(pipeline),
                Err(e) => {
                    log::error!("simulation {e}");
                    Program::Failed(e)
                }
            };
        }
    }

    /// Whether every kernel `sim` names can run: `Ok(true)` when all have linked, `Ok(false)`
    /// while one is still linking, and the first failure otherwise.
    fn ready(&mut self, sim: &Simulation) -> Result<bool, String> {
        let set = self.set_of(&sim.params);
        let mut all = true;
        for pass in &sim.passes {
            match self.programs.get(&(address(pass.kernel), set)) {
                Some((_, Program::Ready(_))) => {}
                Some((_, Program::Failed(e))) => return Err(e.clone()),
                Some((_, Program::Linking)) | None => all = false,
            }
        }
        Ok(all)
    }

    fn pipeline(&self, kernel: &'static Kernel, set: usize) -> Option<&wgpu::ComputePipeline> {
        match self.programs.get(&(address(kernel), set)) {
            Some((_, Program::Ready(pipeline))) => Some(pipeline),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------------------- the world

/// Bytes a count per cell takes on a world `size` on a side.
fn cell_bytes(size: u32) -> u64 {
    u64::from(size) * u64::from(size) * 4
}

/// Bytes the agents' buffer takes for `agents` of them.
fn agent_bytes(agents: u32) -> u64 {
    u64::from(agents) * 16
}

/// A storage buffer of `bytes`, zeroed — wgpu zeroes what it allocates — and never empty,
/// since a binding of an array holds at least one element.
fn buffer(gpu: &Gpu, label: &str, bytes: u64) -> wgpu::Buffer {
    gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A texture `size` on a side, zeroed.
fn texture(
    gpu: &Gpu,
    label: &str,
    size: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// One of the two fields: read and written by the kernels, read by a resample, read back by a
/// test.
fn field(gpu: &Gpu, size: u32) -> wgpu::Texture {
    texture(
        gpu,
        "sim field",
        size,
        wgpu::TextureFormat::R32Float,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    )
}

/// The picture: written by the kernels, sampled by every consumer, the target of a resize's
/// carry, read back by a test.
fn picture(gpu: &Gpu, size: u32) -> wgpu::Texture {
    texture(
        gpu,
        "sim picture",
        size,
        Format::Byte.texture_format(),
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
    )
}

fn view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The stage [`RESAMPLE`] runs in.
struct Resample {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl Resample {
    fn new(gpu: &Gpu) -> Self {
        let device = gpu.device();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim resample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sim resample"),
            source: wgpu::ShaderSource::Wgsl(RESAMPLE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim resample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("sim resample"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(ENTRY),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Record `from` resampled into `to`, which is `size` on a side.
    fn record(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        from: &wgpu::Texture,
        to: &wgpu::Texture,
        size: u32,
    ) {
        let (from, to) = (view(from), view(to));
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sim resample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&from),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&to),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sim resample"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        let side = size.div_ceil(TILE);
        pass.dispatch_workgroups(side, side, 1);
    }
}

/// The world's resources at binding `binding` of group 0.
fn storage_buffer(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_texture(
    binding: u32,
    access: wgpu::StorageTextureAccess,
    format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

/// One port's world on the GPU.
struct World {
    size: u32,
    agents: u32,
    agent_buffer: wgpu::Buffer,
    state_buffer: wgpu::Buffer,
    fields: [wgpu::Texture; 2],
    arrivals: [wgpu::Buffer; 2],
    /// Which of each pair is the one a pass reads: `fields[front]` is `field`.
    front: usize,
    picture: wgpu::Texture,
    picture_view: wgpu::TextureView,
    /// Group 0 with `fields[i]` as `field`: the one a pass binds is `groups[front]`.
    groups: [wgpu::BindGroup; 2],
    wrap: TextureWrap,
    filter: TextureFilter,
    /// Ticks published and not yet run, oldest first.
    queued: VecDeque<Simulation>,
}

impl World {
    /// A world of this shape, all of it zero.
    fn new(
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        size: u32,
        agents: u32,
        wrap: TextureWrap,
        filter: TextureFilter,
    ) -> Self {
        let size = size.max(1);
        let agent_buffer = buffer(gpu, "sim agents", agent_bytes(agents));
        let state_buffer = buffer(gpu, "sim state", STATE_WORDS as u64 * 4);
        let fields = [field(gpu, size), field(gpu, size)];
        let arrivals = [
            buffer(gpu, "sim arrivals", cell_bytes(size)),
            buffer(gpu, "sim arrivals", cell_bytes(size)),
        ];
        let picture = picture(gpu, size);
        let picture_view = view(&picture);
        let groups = groups(
            gpu,
            layout,
            &agent_buffer,
            &state_buffer,
            &fields,
            &arrivals,
            &picture_view,
        );
        Self {
            size,
            agents,
            agent_buffer,
            state_buffer,
            fields,
            arrivals,
            front: 0,
            picture,
            picture_view,
            groups,
            wrap,
            filter,
            queued: VecDeque::new(),
        }
    }

    /// Queue one tick of this world, and take the sampling it declares.
    fn publish(&mut self, sim: &Simulation, wrap: TextureWrap, filter: TextureFilter) {
        self.wrap = wrap;
        self.filter = filter;
        let unchanged = self
            .queued
            .back()
            .map_or((self.size, self.agents), |q| (q.size.max(1), q.agents))
            == (sim.size.max(1), sim.agents);
        if sim.passes.is_empty() && unchanged {
            return;
        }
        if self.queued.len() >= MAX_QUEUED {
            log::warn!("a simulation's kernels have not linked: dropping its oldest tick");
            self.queued.pop_front();
        }
        self.queued.push_back(sim.clone());
    }

    /// Take every queued tick whose kernels have linked, oldest first, stopping at the first
    /// that must wait. A kernel that failed empties the queue and says why.
    fn take_ready(&mut self, kernels: &mut Kernels) -> Result<Vec<Simulation>, String> {
        let mut ready = Vec::new();
        while let Some(sim) = self.queued.front() {
            match kernels.ready(sim) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    self.queued.clear();
                    return Err(e);
                }
            }
            ready.extend(self.queued.pop_front());
        }
        Ok(ready)
    }

    /// Make the world this shape, keeping what can be kept, recording the copies into
    /// `encoder`. What is replaced is dropped here; wgpu keeps it until the commands naming it
    /// have run, and a `Published` naming the old picture keeps that one for as long as it is
    /// held.
    fn reshape(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        stage: &Stages<'_>,
        encoder: &mut wgpu::CommandEncoder,
        size: u32,
        agents: u32,
    ) {
        let size = size.max(1);
        let reshaped = size != self.size || agents != self.agents;
        if size != self.size {
            let fields = [field(gpu, size), field(gpu, size)];
            stage
                .resample
                .record(gpu, encoder, &self.fields[self.front], &fields[0], size);
            let picture = picture(gpu, size);
            let picture_view = view(&picture);
            shared.carry(
                gpu,
                encoder,
                &self.picture_view,
                &picture_view,
                (size, size),
                Format::Byte,
            );
            self.fields = fields;
            self.arrivals = [
                buffer(gpu, "sim arrivals", cell_bytes(size)),
                buffer(gpu, "sim arrivals", cell_bytes(size)),
            ];
            self.front = 0;
            self.picture = picture;
            self.picture_view = picture_view;
            self.size = size;
        }
        if agents != self.agents {
            let fresh = buffer(gpu, "sim agents", agent_bytes(agents));
            let kept = agent_bytes(agents.min(self.agents));
            if kept > 0 {
                encoder.copy_buffer_to_buffer(&self.agent_buffer, 0, &fresh, 0, kept);
            }
            self.agent_buffer = fresh;
            self.agents = agents;
        }
        if reshaped {
            self.groups = groups(
                gpu,
                stage.world,
                &self.agent_buffer,
                &self.state_buffer,
                &self.fields,
                &self.arrivals,
                &self.picture_view,
            );
        }
    }

    /// One pass: its numbers into `bytes` at the next slot, and the dispatch over its domain
    /// with the world bound the way round its front says.
    fn dispatch(
        &mut self,
        compute: &mut wgpu::ComputePass<'_>,
        pipeline: &wgpu::ComputePipeline,
        pass: &Pass,
        params: &[(&'static str, f32)],
        slots: &mut Slots<'_>,
    ) {
        let groups = match pass.kernel.over {
            Domain::Agents => {
                let to = pass.to.min(self.agents);
                let from = pass.from.min(to);
                let visited = (to - from).div_ceil(pass.stride.max(1));
                (visited.div_ceil(AGENT_GROUP), 1)
            }
            Domain::Cells | Domain::Tiles => {
                let side = self.size.div_ceil(TILE);
                (side, side)
            }
            Domain::Once => (1, 1),
        };
        if groups.0 > 0 {
            let offset = slots.bytes.len();
            let to = pass.to.min(self.agents);
            let words: [[u8; 4]; 7] = [
                (self.size as i32).to_le_bytes(),
                (self.agents as i32).to_le_bytes(),
                (pass.from.min(to) as i32).to_le_bytes(),
                (to as i32).to_le_bytes(),
                (pass.stride.max(1) as i32).to_le_bytes(),
                pass.seed.to_le_bytes(),
                pass.arg.to_le_bytes(),
            ];
            for word in words {
                slots.bytes.extend_from_slice(&word);
            }
            for (_, value) in params {
                slots.bytes.extend_from_slice(&value.to_le_bytes());
            }
            slots.bytes.resize(offset + slots.stride as usize, 0);
            compute.set_pipeline(pipeline);
            compute.set_bind_group(0, &self.groups[self.front], &[]);
            compute.set_bind_group(1, slots.group, &[offset as u32]);
            compute.dispatch_workgroups(groups.0, groups.1, 1);
        }
        if pass.kernel.flips {
            self.front = 1 - self.front;
        }
    }

    /// What the world holds, read back and waited for. Rows bottom first, as the textures are
    /// addressed.
    fn read(&self, gpu: &Gpu) -> SimReadback {
        let cells = cell_bytes(self.size);
        let words = |bytes: Vec<u8>| -> Vec<u32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect()
        };
        let field = words(super::readback::read_texture(gpu, &self.fields[self.front]))
            .into_iter()
            .map(f32::from_bits)
            .collect();
        let picture = super::readback::read_texture(gpu, &self.picture);
        let agents = words(read_buffer(
            gpu,
            &self.agent_buffer,
            agent_bytes(self.agents),
        ))
        .as_chunks::<4>()
        .0
        .iter()
        .map(|a| a.map(f32::from_bits))
        .collect();
        SimReadback {
            size: self.size,
            agents,
            state: words(read_buffer(gpu, &self.state_buffer, STATE_WORDS as u64 * 4)),
            field,
            arrivals: words(read_buffer(gpu, &self.arrivals[self.front], cells)),
            arrivals_next: words(read_buffer(gpu, &self.arrivals[1 - self.front], cells)),
            picture,
            queued: self.queued.len(),
        }
    }
}

/// Group 0 both ways round: `[front 0, front 1]`.
fn groups(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    agents: &wgpu::Buffer,
    state: &wgpu::Buffer,
    fields: &[wgpu::Texture; 2],
    arrivals: &[wgpu::Buffer; 2],
    picture: &wgpu::TextureView,
) -> [wgpu::BindGroup; 2] {
    let views = [view(&fields[0]), view(&fields[1])];
    [0, 1].map(|front| {
        let back = 1 - front;
        gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sim world"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: agents.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[front]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[back]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: arrivals[front].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: arrivals[back].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(picture),
                },
            ],
        })
    })
}

/// `bytes` of a buffer, copied out and waited for. A test's.
fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer, bytes: u64) -> Vec<u8> {
    if bytes == 0 {
        return Vec::new();
    }
    let staging = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("sim read"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sim read"),
        });
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, bytes);
    let ticket = gpu.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| {
        r.expect("the staging buffer maps");
    });
    gpu.wait(&ticket, super::QUEUE_WAIT)
        .expect("the GPU finishes a read");
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device polls");
    let out = staging
        .slice(..)
        .get_mapped_range()
        .expect("the staging buffer is mapped")
        .to_vec();
    staging.unmap();
    out
}

// ---------------------------------------------------------------------------------- the phase

/// The pieces a reshape needs from [`Sims`] while a world is borrowed from it.
struct Stages<'a> {
    world: &'a wgpu::BindGroupLayout,
    resample: &'a Resample,
}

/// The tick's uniform slots as a sync fills them: the group binding them, the bytes so far,
/// and the bytes between two slots.
struct Slots<'a> {
    group: &'a wgpu::BindGroup,
    bytes: &'a mut Vec<u8>,
    stride: u64,
}

/// The tick's uniform buffer, one slot a pass, and the group binding it.
struct Uniforms {
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    capacity: u64,
}

/// Every world, by the port its picture is published on.
pub struct Sims {
    worlds: HashMap<PortRef, World>,
    /// The live worlds, put aside while a render grows worlds of its own: [`Sims::park`].
    parked: Option<HashMap<PortRef, World>>,
    kernels: Kernels,
    world_layout: wgpu::BindGroupLayout,
    uniform_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    resample: Resample,
    uniforms: Option<Uniforms>,
    /// Bytes between two passes' slots: [`SLOT`] rounded up to the device's offset alignment.
    stride: u64,
    /// The tick's uniforms, before they go up.
    bytes: Vec<u8>,
}

impl Sims {
    pub fn new(gpu: &Gpu) -> Self {
        let device = gpu.device();
        let world_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim world"),
            entries: &[
                storage_buffer(0),
                storage_buffer(1),
                storage_texture(
                    2,
                    wgpu::StorageTextureAccess::ReadWrite,
                    wgpu::TextureFormat::R32Float,
                ),
                storage_texture(
                    3,
                    wgpu::StorageTextureAccess::ReadWrite,
                    wgpu::TextureFormat::R32Float,
                ),
                storage_buffer(4),
                storage_buffer(5),
                storage_texture(
                    6,
                    wgpu::StorageTextureAccess::WriteOnly,
                    Format::Byte.texture_format(),
                ),
            ],
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim"),
            bind_group_layouts: &[Some(&world_layout), Some(&uniform_layout)],
            immediate_size: 0,
        });
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        Self {
            worlds: HashMap::new(),
            parked: None,
            kernels: Kernels::default(),
            world_layout,
            uniform_layout,
            pipeline_layout,
            resample: Resample::new(gpu),
            uniforms: None,
            stride: SLOT.next_multiple_of(alignment.max(1)),
            bytes: Vec::new(),
        }
    }

    /// Step every world the job names, recording its passes into `recording`, and free the
    /// ones it no longer does. A kernel that failed to link is its node's entry in `errors`.
    pub fn sync(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        jobs: &[SimJob],
        errors: &mut HashMap<NodeId, String>,
    ) {
        let live: HashSet<PortRef> = jobs.iter().map(|j| j.port).collect();
        self.worlds.retain(|port, _| live.contains(port));
        for job in jobs {
            let world = self.worlds.entry(job.port).or_insert_with(|| {
                World::new(
                    gpu,
                    &self.world_layout,
                    job.sim.size,
                    job.sim.agents,
                    job.wrap,
                    job.filter,
                )
            });
            world.publish(&job.sim, job.wrap, job.filter);
        }

        // Send what the queues name, land what has linked, and take what can run, in the
        // job's order.
        for world in self.worlds.values() {
            for sim in &world.queued {
                self.kernels.want(gpu, &self.pipeline_layout, sim);
            }
        }
        self.kernels.poll();
        let mut runs: Vec<(PortRef, Vec<Simulation>)> = Vec::new();
        for job in jobs {
            let Some(world) = self.worlds.get_mut(&job.port) else {
                continue;
            };
            match world.take_ready(&mut self.kernels) {
                Ok(ticks) => {
                    errors.remove(&job.port.node);
                    if !ticks.is_empty() {
                        runs.push((job.port, ticks));
                    }
                }
                Err(e) => {
                    errors.insert(job.port.node, e);
                }
            }
        }
        if runs.is_empty() {
            return;
        }

        let passes: u64 = runs
            .iter()
            .flat_map(|(_, ticks)| ticks.iter().map(|t| t.passes.len() as u64))
            .sum();
        self.make_room(gpu, passes);
        let Self {
            worlds,
            kernels,
            world_layout,
            resample,
            uniforms,
            stride,
            bytes,
            ..
        } = self;
        let Some(uniforms) = uniforms.as_ref() else {
            return;
        };
        let stages = Stages {
            world: world_layout,
            resample,
        };
        bytes.clear();
        let mut slots = Slots {
            group: &uniforms.group,
            bytes,
            stride: *stride,
        };
        let encoder = recording.encoder();
        for (port, ticks) in runs {
            let Some(world) = worlds.get_mut(&port) else {
                continue;
            };
            for sim in ticks {
                world.reshape(gpu, shared, &stages, encoder, sim.size, sim.agents);
                if sim.passes.is_empty() {
                    continue;
                }
                let set = kernels.set_of(&sim.params);
                let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("sim"),
                    timestamp_writes: None,
                });
                for pass in &sim.passes {
                    let Some(pipeline) = kernels.pipeline(pass.kernel, set) else {
                        continue;
                    };
                    world.dispatch(&mut compute, pipeline, pass, &sim.params, &mut slots);
                }
            }
        }
        if !slots.bytes.is_empty() {
            gpu.queue().write_buffer(&uniforms.buffer, 0, slots.bytes);
        }
    }

    /// A uniform buffer with a slot for each of `passes`, growing it where it has too few.
    fn make_room(&mut self, gpu: &Gpu, passes: u64) {
        let needed = passes.max(1) * self.stride;
        if self.uniforms.as_ref().is_some_and(|u| u.capacity >= needed) {
            return;
        }
        let capacity = needed.next_power_of_two().max(self.stride * 16);
        let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim uniforms"),
            size: capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sim uniforms"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(SLOT),
                }),
            }],
        });
        self.uniforms = Some(Uniforms {
            buffer,
            group,
            capacity,
        });
    }

    /// Every world's picture, by port: what a draw's texture binding looks up first.
    pub fn views(&self) -> impl Iterator<Item = (PortRef, wgpu::TextureView)> + '_ {
        self.worlds
            .iter()
            .map(|(port, world)| (*port, world.picture_view.clone()))
    }

    /// Put every world's picture into `out`, rows top first (`flip`), with the sampler its
    /// output declared. Row zero of the world is the texture's first row, as an upload's top
    /// row is.
    pub fn publish(&self, out: &mut Published) {
        for (port, world) in &self.worlds {
            out.sources.insert(
                *port,
                Picture {
                    texture: Texture::new(world.picture.clone(), world.picture_view.clone()),
                    width: world.size,
                    height: world.size,
                    flip: true,
                    sampler: Sampler::of(world.wrap, world.filter),
                    drawn_tick: None,
                },
            );
        }
    }

    /// The picture a node's world last drew. For a test.
    pub fn texture_of(&self, node: NodeId) -> Option<wgpu::Texture> {
        self.worlds
            .iter()
            .find(|(port, _)| port.node == node)
            .map(|(_, world)| world.picture.clone())
    }

    /// **Test accessor.** What one world holds on the GPU, read back and waited for.
    pub fn read(&self, gpu: &Gpu, port: PortRef) -> Option<SimReadback> {
        Some(self.worlds.get(&port)?.read(gpu))
    }

    /// Let go of every world: the project is closing. The pipelines are the run's and stay.
    pub fn forget(&mut self) {
        self.worlds.clear();
        self.parked = None;
    }

    /// Put every world aside for a render, which grows a world of its own for each node from
    /// that node's seed, as a node born again does; [`Sims::restore`] puts the live ones back
    /// as the render found them, with the ticks they had queued.
    pub fn park(&mut self) {
        if self.parked.is_none() {
            self.parked = Some(std::mem::take(&mut self.worlds));
        }
    }

    /// The worlds [`Sims::park`] put aside, back in place of the render's own.
    pub fn restore(&mut self) {
        if let Some(parked) = self.parked.take() {
            self.worlds = parked;
        }
    }
}
