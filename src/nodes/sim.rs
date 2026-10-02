// SPDX-License-Identifier: AGPL-3.0-or-later

//! A simulation a node runs on the GPU: what its CPU half publishes, as plain data.
//!
//! A node whose state is a world too big to step on the CPU once a tick — `slimemold`'s
//! thousands of agents over a scent field — keeps that world on the GPU and steps it there
//! with compute kernels it writes itself, the way a generator writes the WGSL of its outputs.
//! Its `tick` still runs on the CPU, because the tick is where the one clock, the presses and
//! the options are: it decides how many steps this tick's `dt` is worth, what the controls
//! say, and what a press or a new grid asks for, and publishes that as a [`Simulation`]
//! through `TickContext::publish_sim`. The renderer runs the passes in order, before any
//! Output draws — and the picture it leaves is bound to a consumer exactly as a CPU node's
//! uploaded frame is. Nothing comes back: no readback, no upload.
//!
//! **The resources are the renderer's and their shape is fixed**, so a kernel names them
//! without declaring them. Every kernel is compiled after a prelude (`render::sims`)
//! that holds:
//!
//! | WGSL | what | kept across a change of shape |
//! | --- | --- | --- |
//! | `agents: array<vec4f>` | one `vec4f` an agent, the node's to mean | the first `min(old, new)`; the rest are zero |
//! | `state: array<atomic<u32>>` | [`STATE_WORDS`] words for whatever the node reduces into | always |
//! | `field`, `field_next` (`texture_storage_2d<r32float, read_write>`) | the field a pass reads, and the one a flipping pass writes | `field` resampled nearest; `field_next` zero |
//! | `arrivals`, `arrivals_next` (`array<atomic<u32>>`) | a counter per cell for `atomicAdd`, and its twin | zero |
//! | `picture` (`texture_storage_2d<rgba8unorm, write>`) | what every consumer samples | scaled into the new size |
//!
//! beside one uniform struct `u` holding `u_size`, `u_agents`, `u_from`, `u_to`, `u_stride`
//! (`i32`), `u_seed` (`u32`) and `u_arg` (`f32`) from the [`Pass`], then an `f32` for each of
//! the simulation's [`Simulation::params`] — `u.u_sensor` — which the renderer writes, so a
//! kernel declares none of them; `SIM_TILE`, and `sim_index(vec2i)`, a cell's place in a
//! per-cell buffer. `state` and both arrivals are read with `atomicLoad` and written with
//! `atomicStore`; `workgroupBarrier()` is the barrier and `var<workgroup>` the shared memory,
//! and a [`Domain::Tiles`] kernel finds its workgroup and its place in it in the private
//! `sim_group`, `sim_local` and `sim_local_index`, since WGSL hands a built-in only to the
//! entry point. A kernel that [`Kernel::flips`] swaps `field` with `field_next` and
//! `arrivals` with `arrivals_next` after it runs, which is how a diffusion reads one field and
//! writes the other without a copy.
//!
//! **Deterministic by construction.** A kernel's randomness is a hash of the invocation and
//! the pass's `seed`, which the CPU half draws from its own seeded generator; arrivals are
//! counted with integer atomics, whose sum does not depend on the order the GPU ran them in —
//! on a buffer rather than an image, because agents crowd onto the same cells and Intel's
//! typed atomics under that contention cost ten times what untyped ones do;
//! and the number of steps is a function of the `dt`s the tick was handed. The same seed and
//! the same `dt` sequence make the same world, live or in an offline render at any speed.
//!
//! See [docs/rendering.md](../../docs/rendering.md#simulations) for the renderer's side.

/// How many `uint`s `state[]` holds.
pub const STATE_WORDS: usize = 16;

/// Cells on a side of the workgroup a [`Domain::Cells`] or [`Domain::Tiles`] kernel runs in,
/// which a kernel reads as `SIM_TILE`.
pub const TILE: u32 = 8;

/// Which invocations a kernel runs over, and so which function of its own it defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// One invocation per agent, `u_from` to `u_to` every `u_stride`th: the kernel defines
    /// `fn run_agent(i: i32)`.
    Agents,
    /// One invocation per cell of the field: `fn run_cell(c: vec2i)`.
    Cells,
    /// One invocation per cell in workgroups of [`TILE`] by [`TILE`] that share memory:
    /// `fn run_tile(c: vec2i, inside: bool)`, called for **every** invocation of a group,
    /// inside the world or past its edge, so the kernel may call `workgroupBarrier()` — a
    /// diffusion that reads each neighborhood once into `var<workgroup>` memory rather than
    /// nine times from the image.
    Tiles,
    /// One invocation in all, for the bookkeeping between two passes over everything:
    /// `fn run_once()`.
    Once,
}

/// One compute kernel of a node's simulation.
///
/// A `static`, so the renderer compiles each once for the life of the run and knows it by its
/// address.
#[derive(Debug)]
pub struct Kernel {
    /// What the node calls it, for the log and a link error.
    pub name: &'static str,
    pub over: Domain,
    /// What every kernel of the node shares — its hash, its cell mapping — pasted ahead of
    /// `wgsl`.
    pub wgsl_common: &'static str,
    /// The function [`Domain`] names — `fn run_agent(i: i32)`, `fn run_cell(c: vec2i)`,
    /// `fn run_tile(c: vec2i, inside: bool)` or `fn run_once()` — and whatever it calls,
    /// compiled after `render::sims`'s prelude.
    pub wgsl: &'static str,
    /// Swap `field` and `arrivals` with their `_next` twins after this pass.
    pub flips: bool,
}

/// One dispatch of one kernel.
#[derive(Debug, Clone, Copy)]
pub struct Pass {
    pub kernel: &'static Kernel,
    /// The agents an [`Domain::Agents`] pass visits: `from`, then every `stride`th, short of
    /// `to`. Ignored by the other domains.
    pub from: u32,
    pub to: u32,
    pub stride: u32,
    /// What the kernel hashes its randomness from. Drawn by the CPU half, so the sequence is
    /// its seeded generator's.
    pub seed: u32,
    /// One number the pass carries for itself — a scale, a fade.
    pub arg: f32,
}

impl Pass {
    /// A pass over the whole domain, with nothing of its own.
    pub fn of(kernel: &'static Kernel) -> Self {
        Self {
            kernel,
            from: 0,
            to: u32::MAX,
            stride: 1,
            seed: 0,
            arg: 0.0,
        }
    }

    #[must_use]
    pub fn seeded(self, seed: u32) -> Self {
        Self { seed, ..self }
    }

    #[must_use]
    pub fn with_arg(self, arg: f32) -> Self {
        Self { arg, ..self }
    }

    /// Agents `from` to `to`, every `stride`th.
    #[must_use]
    pub fn over(self, from: u32, to: u32, stride: u32) -> Self {
        Self {
            from,
            to,
            stride: stride.max(1),
            ..self
        }
    }
}

impl PartialEq for Pass {
    /// The same kernel — by address, since a kernel is a `static` — over the same agents with
    /// the same seed and argument.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.kernel, other.kernel)
            && self.from == other.from
            && self.to == other.to
            && self.stride == other.stride
            && self.seed == other.seed
            && self.arg.to_bits() == other.arg.to_bits()
    }
}

/// What a node publishes for its simulation on one tick.
///
/// The shape is the world's as the tick left it; the renderer reallocates to it before the
/// passes run. The passes are this tick's work, in order, and are run once: a tick that
/// publishes none leaves the world where it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Simulation {
    /// Cells across, which is also cells down.
    pub size: u32,
    pub agents: u32,
    /// The numbers every kernel may read, by uniform name.
    pub params: Vec<(&'static str, f32)>,
    pub passes: Vec<Pass>,
    /// How many steps the world has taken since it was born, for the Status box and a test.
    pub steps: u64,
}
