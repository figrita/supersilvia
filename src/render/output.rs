// SPDX-License-Identifier: AGPL-3.0-or-later

//! One Output node's renderer: its ring, its programs, and one frame recorded into an encoder
//! of its own.
//!
//! Per frame: take a free slot of the ring, record one pass of the program into it, make it
//! the latest. The renderer submits the encoder on its own, so the slot's serial is that
//! submission's and a cheap Output's frame is shown, and its readings taken, as soon as it has
//! finished, whatever heavier Outputs follow it (`proposals/wgpu.md`, 1.2). A graph sampling
//! its own Output reads the latest slot from before this frame, which makes feedback a
//! one-frame delay instead of a race.
//!
//! **The bind group is made per draw**, because rings rotate what every consumer samples: the
//! uniform struct at the draw's offset into the tick's buffer, the tap buffer of the program
//! drawing, the four samplers, and each texture looked up as the draw is made — an Output
//! drawn earlier this tick as it was just drawn, one drawn later as it was last tick, and one
//! that is not there as black. Nothing names another Output's tap buffer: there is no global
//! binding state to leave one behind (`proposals/wgpu.md`, 1.7).
//!
//! What it reads back is [`Readbacks`], what it times is [`Timer`], and how its programs link
//! is [`Programs`]; each is its own file, filled by its own lane.

use super::UniformValue;
use super::gpu::{Gpu, Ticket};
use super::lease::Lease;
use super::link::Programs;
use super::readback::Readbacks;
use super::ring::{Format, Ring, Slot};
use super::shared::Shared;
use super::timing::GpuTime;
use super::timing::Timer;
use super::uniforms::Arena;
use crate::compile::Shader;
use crate::compile::wgsl::Resource;
use crate::graph::{NodeId, PortRef};
use std::collections::HashMap;
use std::sync::Arc;

/// What a draw binds its uniform struct from: the tick's buffer and the draw's offset in it.
#[derive(Clone, Copy)]
pub struct Block<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub offset: u32,
}

/// The textures a draw may sample: every CPU node's upload and simulation's picture by port,
/// and every Output's latest frame by node.
pub struct Sampled<'a> {
    pub textures: &'a HashMap<PortRef, wgpu::TextureView>,
    pub latest: &'a HashMap<NodeId, wgpu::TextureView>,
}

// Each flag is an independent question about a different part of the pipeline.
#[allow(clippy::struct_excessive_bools)]
pub struct OutputRenderer {
    programs: Programs,
    width: u32,
    height: u32,
    ring: Ring,
    /// The ring's count for the last frame `encode` drew: the frame a capture waits for.
    rendered: Option<u64>,
    /// The frame `encode` drew that has not been submitted yet.
    unsubmitted: Option<u64>,
    /// True once the latest frame has been cleared because this Output went inactive.
    dark: bool,
    /// True once the latest frame is one its program drew.
    pictured: bool,
    drops: DropCauses,
    readbacks: Readbacks,
    timer: Timer,
    /// The part of the target drawn, from the corner, where not all of it: a workspace pass
    /// drawing only its measurements.
    region: Option<(u32, u32)>,
    /// The size a uniform block says it draws at, where that is not the target's: a
    /// workspace pass, whose target is a grid of thumbnails and whose nodes should size
    /// what they measure in pixels as an Output of the default size would.
    nominal: Option<(u32, u32)>,
    /// The frame a render found, kept while it draws its own, with whether a program drew it
    /// and whether it was dark: [`OutputRenderer::keep`].
    kept: Option<(wgpu::Texture, bool, bool)>,
}

impl OutputRenderer {
    /// A renderer whose ring's first slot is cleared into `encoder`.
    pub fn new(gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, width: u32, height: u32) -> Self {
        let ring = Ring::new(gpu, encoder, width, height, Format::Half);
        let (width, height) = ring.size();
        Self {
            programs: Programs::new(Format::Half.texture_format()),
            width,
            height,
            ring,
            rendered: None,
            unsubmitted: None,
            dark: false,
            pictured: false,
            drops: DropCauses::default(),
            readbacks: Readbacks::default(),
            timer: Timer::default(),
            region: None,
            nominal: None,
            kept: None,
        }
    }

    /// Keep a copy of the latest frame for a render to hand back: [`Ring::keep`]. Once until
    /// it is put back.
    pub fn keep(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder) {
        if self.kept.is_none() {
            self.kept = Some((self.ring.keep(gpu, encoder), self.pictured, self.dark));
        }
    }

    /// The kept frame the latest again, at its own size, and say whether there is nothing left
    /// to put back: [`Ring::put_back`].
    pub fn put_back(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder) -> bool {
        let Some((texture, pictured, dark)) = &self.kept else {
            return true;
        };
        if !self.ring.put_back(gpu, encoder, texture, gpu.completed()) {
            return false;
        }
        (self.pictured, self.dark) = (*pictured, *dark);
        (self.width, self.height) = self.ring.size();
        self.kept = None;
        true
    }

    /// Draw only this much of the target from now on, from the corner, or all of it.
    pub fn set_region(&mut self, region: Option<(u32, u32)>) {
        self.region = region;
    }

    /// Tell every block from now on that it draws at `size`; see the field.
    pub fn set_nominal(&mut self, size: (u32, u32)) {
        self.nominal = Some(size);
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn drops(&self) -> DropCauses {
        self.drops
    }

    /// The latest frame drawn: what another Output's `frame` samples, and what this Output's
    /// own `frame` samples on its next draw.
    pub fn latest(&self) -> &Slot {
        self.ring.latest()
    }

    pub fn set_tick(&mut self, tick: u64) {
        self.ring.set_tick(tick);
    }

    pub fn shown_tick(&self) -> Option<u64> {
        self.ring.shown_tick()
    }

    /// The newest frame that has finished on the GPU, and a claim on it: what a viewer is
    /// shown.
    pub fn shown(&self) -> Option<(&Slot, Lease)> {
        self.ring.shown()
    }

    /// Every target this Output holds. For a test.
    pub fn targets(&self) -> Vec<wgpu::Texture> {
        self.ring.textures()
    }

    /// Mark every frame whose submission has finished, so the next publish names the newest.
    pub fn poll_frames(&mut self, completed: u64) {
        self.ring.poll(completed);
    }

    /// Drop targets of an old size that nothing reads. Once a tick.
    pub fn sweep(&mut self) {
        self.ring.sweep();
    }

    pub fn has_program(&self) -> bool {
        self.programs.has_program()
    }

    pub fn error(&self) -> Option<&String> {
        self.programs.error.as_ref()
    }

    pub fn gpu_time(&self) -> Option<GpuTime> {
        self.timer.reading()
    }

    /// Its frames' spans, as the renderer's [`super::timing::Stamps`] reads them back.
    pub fn timer(&mut self) -> &mut Timer {
        &mut self.timer
    }

    /// Replace the program with one linked from `shader`; see [`Programs::set_shader`].
    pub fn set_shader(&mut self, gpu: &Gpu, shader: &Shader) {
        self.dark = false;
        self.programs.set_shader(gpu, shader);
    }

    /// Take a job's uniforms and tap slots; see [`Programs::adopt`].
    pub fn adopt(
        &mut self,
        source: u64,
        uniforms: &[(Arc<str>, UniformValue)],
        taps: usize,
    ) -> bool {
        self.want(Some(source));
        self.programs.adopt(source, uniforms, taps)
    }

    /// Land a program that finished linking. Once a tick, before drawing.
    pub fn poll_shader(&mut self) {
        self.programs.poll();
    }

    pub fn is_linking(&self) -> bool {
        self.programs.is_linking()
    }

    /// Leave a link in flight unfinished until let go. For a test.
    pub fn hold_link(&mut self, hold: bool) {
        self.programs.held = hold;
    }

    pub fn kept_programs(&self) -> usize {
        self.programs.kept()
    }

    /// The source a tap reading must come from, from now on.
    pub fn want(&mut self, source: Option<u64>) {
        self.readbacks.want(source);
    }

    /// Stop drawing with any program.
    pub fn clear_shader(&mut self) {
        self.want(None);
        self.programs.clear();
    }

    /// Drop the program and blank the latest frame, so an unplugged Output publishes black to
    /// whatever samples its `frame`.
    pub fn go_dark(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder) {
        self.clear_shader();
        if self.dark {
            return;
        }
        self.timer.forget();
        // Not dark yet where every target is held: the next tick asks again.
        self.dark = self.ring.blank(gpu, encoder, gpu.completed());
        self.pictured = false;
    }

    /// Blank the latest frame and keep the program: a render's black warm-up.
    pub fn clear_frame(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder) {
        if !self.ring.blank(gpu, encoder, gpu.completed()) {
            log::warn!("every target of an Output is held: its frame is not blanked");
        }
        self.pictured = false;
    }

    /// Draw at a new resolution from now on, carrying the last frame across. A ring with no
    /// room for the carry stays at the old size, and the next call asks again.
    pub fn resize(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
    ) {
        if (self.width, self.height) == (width.max(1), height.max(1)) {
            return;
        }
        self.ring.resize(gpu, shared, encoder, width, height);
        (self.width, self.height) = self.ring.size();
    }

    /// An idle tick: nothing drawn, and what the last frame left collected once it finished.
    pub fn rest(&mut self, gpu: &Gpu) {
        self.readbacks.settle(gpu, &self.ring, self.rendered);
    }

    /// Whether a picture is owed that only a draw makes: a thumbnail or Snap, or a first
    /// frame from a program since the latest was made or blanked.
    pub fn wants_picture(&self) -> bool {
        self.readbacks.wants_picture() || (self.programs.has_program() && !self.pictured)
    }

    /// Pack this draw's uniform block into the tick's `arena` and say at which offset. `None`
    /// with nothing to draw with.
    pub fn push_block(&self, arena: &mut Arena, time: f32) -> Option<u32> {
        let (program, setup) = self.programs.current()?;
        let size = self.nominal.unwrap_or_else(|| self.size());
        Some(arena.push(&program.uniforms, size, time, &setup.uniforms))
    }

    /// Record one frame into `encoder`, its pass writing `timestamps` where it is timed. False
    /// where nothing was recorded: no program, every target held, or a capture's wait ran out.
    pub fn encode(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        encoder: &mut wgpu::CommandEncoder,
        block: Block<'_>,
        sampled: &Sampled<'_>,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> bool {
        let Self {
            programs,
            ring,
            readbacks,
            drops,
            rendered,
            unsubmitted,
            pictured,
            region,
            ..
        } = self;
        let Some((program, setup)) = programs.current() else {
            return false;
        };
        if !readbacks.settle(gpu, ring, *rendered) {
            log::error!("the GPU did not finish a captured frame in two seconds");
            drops.capture += 1;
            return false;
        }
        let Some(slot) = ring.free_slot(gpu, gpu.completed()) else {
            drops.held += 1;
            return false;
        };
        let slots = if program.taps() { setup.taps.max(1) } else { 0 };
        let taps = readbacks.tap_buffer(gpu, encoder, slots, program.reset, program.source);

        let entries: Vec<wgpu::BindGroupEntry<'_>> = program
            .bindings
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match resource {
                    Resource::Uniforms => wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: block.buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(u64::from(program.uniforms.size)),
                    }),
                    Resource::Taps => taps
                        .as_ref()
                        .expect("a program with taps is given a tap buffer")
                        .as_entire_binding(),
                    Resource::Sampler(kind) => {
                        wgpu::BindingResource::Sampler(shared.sampler(*kind))
                    }
                    Resource::Texture(name) => wgpu::BindingResource::TextureView(texture_for(
                        name,
                        &setup.uniforms,
                        sampled,
                        shared.black(),
                    )),
                },
            })
            .collect();
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("output"),
            layout: program.layout(),
            entries: &entries,
        });

        {
            let target = &ring.slot(slot).view;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("output"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    // Every fragment is drawn, so what the target held is never read.
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: timestamps,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(program.pipeline());
            pass.set_bind_group(0, &group, &[block.offset]);
            if let Some((w, h)) = *region {
                let (width, height) = (ring.slot(slot).width, ring.slot(slot).height);
                pass.set_scissor_rect(0, 0, w.min(width), h.min(height));
            }
            pass.draw(0..3, 0..1);
        }
        readbacks.after_pass(gpu, shared, encoder, ring.slot(slot));
        ring.commit(slot);
        let drawn = ring.latest_drawn();
        *rendered = Some(drawn);
        *unsubmitted = Some(drawn);
        *pictured = true;
        true
    }

    /// What this renderer recorded since the last submission went in `ticket`'s.
    pub fn submitted(&mut self, ticket: &Ticket) {
        self.ring.submitted(ticket.serial);
        if let Some(drawn) = self.unsubmitted.take() {
            self.readbacks.submitted(ticket, drawn);
        }
    }

    /// The readbacks, for the renderer's accessors.
    pub fn readbacks(&mut self) -> &mut Readbacks {
        &mut self.readbacks
    }

    /// The same, to ask.
    pub fn readbacks_ref(&self) -> &Readbacks {
        &self.readbacks
    }
}

/// The view a texture binding samples: the upload or simulation on its port, or the Output's
/// latest frame, or black where neither is there.
fn texture_for<'a>(
    name: &str,
    uniforms: &[(Arc<str>, UniformValue)],
    sampled: &Sampled<'a>,
    black: &'a wgpu::TextureView,
) -> &'a wgpu::TextureView {
    uniforms
        .iter()
        .find(|(n, _)| &**n == name)
        .and_then(|(_, value)| match value {
            UniformValue::NodeTexture(port) => sampled
                .textures
                .get(port)
                .or_else(|| sampled.latest.get(&port.node)),
            _ => None,
        })
        .unwrap_or(black)
}

/// How many drops of each cause an Output has had. A count with no cause cannot be acted on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropCauses {
    /// A render waited two seconds for the frame before and the GPU had not finished it.
    pub capture: u64,
    /// Every target is held by a viewer and the ring is at its limit, so nothing is drawn
    /// under a read.
    pub held: u64,
}

impl DropCauses {
    /// Every cause with a count, worst first, for a line that says only what happened.
    pub fn listed(self) -> Vec<(&'static str, u64)> {
        let mut out = vec![("held", self.held), ("capture", self.capture)];
        out.retain(|(_, n)| *n > 0);
        out.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        out
    }

    /// Every drop, whatever its cause.
    pub fn total(self) -> u64 {
        self.capture + self.held
    }

    /// Both counts added into these.
    pub fn add(&mut self, other: Self) {
        self.capture += other.capture;
        self.held += other.held;
    }
}
