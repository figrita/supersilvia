// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an Output reads back off the GPU: its tap words, a thumbnail, a Snap and a render's
//! capture — and [`read_texture`], the one blocking read the tests use.
//!
//! **Every read is a staging buffer and a map, collected when its callback has fired, never
//! waited for** (`proposals/wgpu.md`, 1.6). wgpu has no persistent mapping of a buffer the GPU
//! writes: a `MAP_READ` buffer may only be a copy's destination, and it must be unmapped
//! whenever a submission uses it. So a draw records a copy into a staging buffer ([`Read`])
//! behind its pass, the renderer submits it as that Output's own submission, and
//! [`Readbacks::submitted`] asks for the map. The callback only stores into an atomic; the
//! bytes are copied out on a later tick, the buffer unmapped, and only then is it the
//! destination of another copy. A map completes when its own submission does, and each Output
//! is a submission of its own, so a cheap Output's reading waits for nothing queued behind it.
//!
//! **Taps.** A program with taps draws into one storage buffer, reset from the template by a
//! copy recorded ahead of the pass and copied into the frame's staging after it — all on the
//! one queue, in order, so the next frame's reset cannot overtake this frame's copy.
//! [`READBACKS`] records, one per frame the GPU may still hold and the one being drawn, each
//! carry a staging buffer and the source that was wanted when the frame was drawn; a reading
//! is handed over only while that source is still the one wanted, since the caller decodes
//! it in that source's layout (docs/rendering.md, tap buffers).
//!
//! **Thumbnail, Snap and capture** are a pass of [`PICTURE`] into an `Rgba8Unorm` target, then
//! a copy into staging with its rows padded to 256 bytes, dropped again as the rows are copied
//! out and flipped top first. The pass samples by `@builtin(position)`, so it writes the rows
//! GL's `glBlitFramebuffer` wrote (`proposals/wgpu.md`, 1.15): a thumbnail letterboxed on
//! black, a Snap whole, and a capture brought down to the film in linear halvings — one at 1x
//! and 2x, two at 4x through a half-float target twice the film's size, so each written pixel
//! averages all sixteen drawn behind it. A capture drops no frame: [`Readbacks::settle`] waits
//! for the previous frame's submission, bounded at two seconds, before the next is drawn.

use super::gpu::{Gpu, Ticket};
use super::ring::{Ring, Slot};
use super::shared::Shared;
use crate::compile::{TAP_TEMPLATE, TAP_WORDS};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// How big a thumbnail of an Output is, and therefore how big the file a save writes is.
/// The same 240x135 the on-node render draws at, letterboxed the same way.
pub const THUMBNAIL: (u32, u32) = (240, 135);

/// How many bytes one thumbnail is, RGBA8.
pub const THUMBNAIL_BYTES: usize = (THUMBNAIL.0 * THUMBNAIL.1 * 4) as usize;

/// How many frames' tap words an Output holds: one per draw the GPU may still be running, and
/// the one being drawn.
pub const READBACKS: usize = super::TICKS_IN_FLIGHT + 1;

/// How long a test's blocking read waits for the GPU.
const READ_WAIT: Duration = Duration::from_secs(10);

/// How long a capture waits for its previous frame, or for a read still out when it stops,
/// before deciding the GPU has hung and losing that frame rather than the session.
const CAPTURE_WAIT: Duration = Duration::from_secs(2);

/// The pass every picture read is drawn by: the frame, sampled over the rectangle the pass's
/// viewport covers, by fragment position.
pub const PICTURE: &str = "
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
struct Rect { origin: vec2f, size: vec2f }
@group(0) @binding(2) var<uniform> rect: Rect;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return textureSampleLevel(frame, frame_sampler, (frag_coord.xy - rect.origin) / rect.size, 0.0);
}
";

/// Where a staging buffer's map stands, as its callback leaves it.
const MAPPING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// One staging buffer and the read through it.
pub(super) struct Read {
    buffer: wgpu::Buffer,
    stage: Stage,
    /// [`MAPPING`] until the callback stores [`MAPPED`] or [`FAILED`].
    landed: Arc<AtomicU8>,
    /// The submission the copy went in, once made.
    ticket: Option<Ticket>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Unmapped and not written: free to be a copy's destination.
    Idle,
    /// A copy into it is recorded and not yet submitted.
    Copied,
    /// The copy is submitted and the map asked for.
    Mapping,
}

/// What a read has come to.
pub(super) enum Landed<T> {
    /// Its map has not landed yet.
    Pending,
    Bytes(T),
    /// The map failed: the buffer went, or the device did.
    Lost,
}

impl Read {
    pub(super) fn new(gpu: &Gpu, size: u64, label: &str) -> Self {
        Self {
            buffer: gpu.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            stage: Stage::Idle,
            landed: Arc::new(AtomicU8::new(MAPPING)),
            ticket: None,
        }
    }

    /// The staging buffer, for the copy into it.
    pub(super) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub(super) fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    /// A copy into it has been recorded.
    pub(super) fn copied(&mut self) {
        self.stage = Stage::Copied;
    }

    /// The copy went in `ticket`'s submission: ask for the map.
    pub(super) fn map(&mut self, ticket: &Ticket) {
        if self.stage != Stage::Copied {
            return;
        }
        self.stage = Stage::Mapping;
        self.ticket = Some(ticket.clone());
        self.landed.store(MAPPING, Ordering::Release);
        let landed = Arc::clone(&self.landed);
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let state = if result.is_ok() { MAPPED } else { FAILED };
                landed.store(state, Ordering::Release);
            });
    }

    /// Whether its submission has finished and its callback not yet run: a poll would land
    /// it.
    fn due(&self, gpu: &Gpu) -> bool {
        self.stage == Stage::Mapping
            && self.landed.load(Ordering::Acquire) == MAPPING
            && self.ticket.as_ref().is_some_and(|t| gpu.is_done(t.serial))
    }

    /// Its bytes, through `read`, where the map has landed; then unmapped and idle.
    pub(super) fn take<T>(&mut self, read: impl FnOnce(&[u8]) -> T) -> Landed<T> {
        if self.stage != Stage::Mapping {
            return Landed::Pending;
        }
        match self.landed.load(Ordering::Acquire) {
            MAPPED => {
                let out = self
                    .buffer
                    .slice(..)
                    .get_mapped_range()
                    .ok()
                    .map(|mapped| read(&mapped));
                self.buffer.unmap();
                self.stage = Stage::Idle;
                self.ticket = None;
                out.map_or(Landed::Lost, Landed::Bytes)
            }
            FAILED => {
                self.stage = Stage::Idle;
                self.ticket = None;
                Landed::Lost
            }
            _ => Landed::Pending,
        }
    }

    /// Wait for its submission, at most `timeout`, and poll so the map's callback runs.
    fn wait(&self, gpu: &Gpu, timeout: Duration) {
        if let Some(ticket) = &self.ticket
            && gpu.wait(ticket, timeout).is_err()
        {
            return;
        }
        gpu.poll();
    }
}

/// A picture's read: RGBA8 rows, each padded to what a texture copy asks.
pub(super) fn padded_row(width: u32) -> u32 {
    (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// Rows of `width` RGBA8 texels out of a padded read, bottom first as they lie, returned top
/// first as a PNG is written.
fn unpad_flipped(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (row, padded) = ((width * 4) as usize, padded_row(width) as usize);
    let mut out = Vec::with_capacity(row * height as usize);
    for r in (0..height as usize).rev() {
        out.extend_from_slice(&bytes[r * padded..r * padded + row]);
    }
    out
}

/// A render target a picture read draws into, and the size it is.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Target {
    fn new(gpu: &Gpu, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("readback"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    /// A staging buffer this target's copy fits.
    fn read(&self, gpu: &Gpu, label: &str) -> Read {
        Read::new(gpu, u64::from(padded_row(self.width) * self.height), label)
    }

    /// Record the copy of this target into `read`.
    fn copy_into(&self, encoder: &mut wgpu::CommandEncoder, read: &mut Read) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row(self.width)),
                    rows_per_image: None,
                },
            },
            self.texture.size(),
        );
        read.copied();
    }
}

/// [`PICTURE`]'s pipelines, one per target format, and the two samplers it reads through. Made
/// once with the renderer and kept in its [`super::shared::Shared`], so no Output makes a
/// pipeline on the synth thread the first time a picture is asked of it.
pub struct Blit {
    layout: wgpu::BindGroupLayout,
    byte: wgpu::RenderPipeline,
    half: wgpu::RenderPipeline,
    /// Clamped at the edges, as `glBlitFramebuffer` read: linear where the picture is scaled,
    /// nearest where it is copied one to one.
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
}

impl Blit {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("picture"),
            source: wgpu::ShaderSource::Wgsl(PICTURE.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline =
            |format| super::shared::fullscreen(device, &module, &layout, format, "picture");
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("picture"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            byte: pipeline(wgpu::TextureFormat::Rgba8Unorm),
            half: pipeline(wgpu::TextureFormat::Rgba16Float),
            layout,
            linear: sampler(wgpu::FilterMode::Linear),
            nearest: sampler(wgpu::FilterMode::Nearest),
        }
    }

    /// Record a pass clearing `to` to opaque black and drawing all of `from`, `from_size`,
    /// into the rectangle at `origin` of `size` in it.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        from: &wgpu::TextureView,
        from_size: (u32, u32),
        to: &Target,
        origin: (u32, u32),
        size: (u32, u32),
    ) {
        let pipeline = if to.texture.format() == wgpu::TextureFormat::Rgba16Float {
            &self.half
        } else {
            &self.byte
        };
        // A copy at one to one takes no filter at all; anything else is fitted.
        let sampler = if size == from_size {
            &self.nearest
        } else {
            &self.linear
        };
        let uniforms = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        let mut block = [0u8; 16];
        for (i, v) in [origin.0, origin.1, size.0, size.1].into_iter().enumerate() {
            block[i * 4..i * 4 + 4].copy_from_slice(&(v as f32).to_le_bytes());
        }
        uniforms
            .slice(..)
            .get_mapped_range_mut()
            .expect("a buffer mapped at creation")
            .copy_from_slice(&block);
        uniforms.unmap();
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(from),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniforms.as_entire_binding(),
                },
            ],
        });
        let mut pass = super::shared::begin(
            encoder,
            &to.view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            "picture",
        );
        pass.set_viewport(
            origin.0 as f32,
            origin.1 as f32,
            size.0 as f32,
            size.1 as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// What one frame leaves to be read once it has finished: its tap words.
#[derive(Default)]
struct InFlight {
    /// The ring's count for the frame that last used this record, until its words are
    /// collected.
    drawn: Option<u64>,
    /// The source wanted when that frame was drawn. Its words are delivered only while it is
    /// still the one wanted.
    source: Option<u64>,
    /// The staging buffer the frame's tap words are copied into. `None` with no tap slots.
    staging: Option<Read>,
}

/// The tap buffer the pass binds, the template it is reset from, and the slots both are
/// sized for.
struct Taps {
    slots: usize,
    buffer: wgpu::Buffer,
    template: wgpu::Buffer,
}

impl Taps {
    fn bytes(&self) -> u64 {
        (self.slots * TAP_WORDS * 4) as u64
    }
}

/// A thumbnail's or a Snap's target, its read, and the last one that landed.
struct Picture {
    target: Target,
    read: Read,
    ready: Option<Vec<u8>>,
}

impl Picture {
    fn new(gpu: &Gpu, width: u32, height: u32, label: &str) -> Self {
        let target = Target::new(gpu, width, height, wgpu::TextureFormat::Rgba8Unorm);
        let read = target.read(gpu, label);
        Self {
            target,
            read,
            ready: None,
        }
    }

    /// Collect the read where it has landed. Never waits.
    fn poll(&mut self) {
        let (w, h) = (self.target.width, self.target.height);
        if let Landed::Bytes(bytes) = self.read.take(|b| unpad_flipped(b, w, h)) {
            self.ready = Some(bytes);
        }
    }

    fn pending(&self) -> bool {
        !self.read.is_idle() || self.ready.is_some()
    }
}

/// Whether a read takes the whole frame or letterboxes it into a target of another shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Whole,
    /// The picture's aspect inside the target, centred on black.
    Letterbox,
}

/// Every frame an Output draws, read back for an offline render.
struct Capture {
    /// The film, the drawn frame divided by the supersampling.
    film: Target,
    /// The half-way target a 4x render passes through, twice the film's size. `None` at 1x
    /// and 2x.
    mid: Option<Target>,
    /// Reads in flight, oldest first.
    reads: VecDeque<Read>,
    /// Reads collected, ready to be copied into again.
    spare: Vec<Read>,
}

impl Capture {
    fn new(gpu: &Gpu, width: u32, height: u32, scale: u32) -> Self {
        let film = Target::new(gpu, width, height, wgpu::TextureFormat::Rgba8Unorm);
        // Only 4x stops half way, at the precision of the frame it halves.
        let mid = (scale > 2)
            .then(|| Target::new(gpu, width * 2, height * 2, wgpu::TextureFormat::Rgba16Float));
        Self {
            film,
            mid,
            reads: VecDeque::new(),
            spare: Vec::new(),
        }
    }

    /// Collect every read that has landed, oldest first, stopping at the first that has not —
    /// or, `wait`ing, waiting for each, bounded by [`CAPTURE_WAIT`].
    fn collect(&mut self, gpu: &Gpu, captured: &mut VecDeque<Vec<u8>>, wait: bool) {
        let (w, h) = (self.film.width, self.film.height);
        while let Some(read) = self.reads.front_mut() {
            if read.stage == Stage::Copied {
                // Recorded and never submitted: there is nothing to wait for.
                self.reads.pop_front();
                continue;
            }
            if wait {
                read.wait(gpu, CAPTURE_WAIT);
            } else if read.due(gpu) {
                gpu.poll();
            }
            match read.take(|b| unpad_flipped(b, w, h)) {
                Landed::Pending => {
                    if wait {
                        log::error!("the GPU did not finish a capture read in two seconds");
                        self.reads.pop_front();
                        continue;
                    }
                    return;
                }
                Landed::Bytes(bytes) => captured.push_back(bytes),
                Landed::Lost => log::error!("a captured frame's read was lost"),
            }
            if let Some(read) = self.reads.pop_front() {
                self.spare.push(read);
            }
        }
    }
}

/// One Output's readbacks.
#[derive(Default)]
pub struct Readbacks {
    /// The source a tap reading must come from to be delivered.
    wanted: Option<u64>,
    taps: Option<Taps>,
    frames: [InFlight; READBACKS],
    /// The record the next draw uses: the oldest.
    front: usize,
    /// The record the frame being drawn uses, until it is submitted.
    current: Option<usize>,
    /// The newest collected frame's tap words, until taken.
    ready: Option<Vec<u32>>,
    /// The last frame submitted and its submission: what a capture waits for.
    last: Option<(u64, Ticket)>,
    wants_thumbnail: bool,
    thumb: Option<Picture>,
    wants_snap: bool,
    snap: Option<Picture>,
    wants_capture: bool,
    capture_scale: u32,
    capture: Option<Capture>,
    captures_issued: u64,
    captured: VecDeque<Vec<u8>>,
}

impl Readbacks {
    /// Read taps only from `source` from now on; a reading collected under another is
    /// dropped.
    pub fn want(&mut self, source: Option<u64>) {
        self.wanted = source;
    }

    /// Collect what every finished frame left, and say whether this Output may draw: false
    /// where a capture's wait for its own previous frame, `rendered`, ran out.
    pub fn settle(&mut self, gpu: &Gpu, ring: &Ring, rendered: Option<u64>) -> bool {
        let mut finished = true;
        if self.wants_capture
            && let Some(drawn) = rendered
            && !ring.finished(drawn, gpu.completed())
        {
            finished = match &self.last {
                Some((last, ticket)) if *last == drawn => gpu.wait(ticket, CAPTURE_WAIT).is_ok(),
                _ => {
                    gpu.poll();
                    ring.finished(drawn, gpu.completed())
                }
            };
        }
        self.collect_taps(gpu);
        finished
    }

    /// Take the tap words of every record whose map has landed, oldest first, up to the first
    /// still on the GPU, so what is left ready is the newest finished frame's.
    fn collect_taps(&mut self, gpu: &Gpu) {
        if self
            .frames
            .iter()
            .any(|f| f.drawn.is_some() && f.staging.as_ref().is_some_and(|s| s.due(gpu)))
        {
            gpu.poll();
        }
        for k in 0..READBACKS {
            let frame = &mut self.frames[(self.front + k) % READBACKS];
            if frame.drawn.is_none() {
                continue;
            }
            let Some(staging) = &mut frame.staging else {
                frame.drawn = None;
                continue;
            };
            let words = staging.take(|bytes| {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_ne_bytes(*c))
                    .collect::<Vec<u32>>()
            });
            match words {
                Landed::Pending => break,
                Landed::Bytes(words) => {
                    if frame.source.is_some() && frame.source == self.wanted {
                        self.ready = Some(words);
                    }
                }
                Landed::Lost => {}
            }
            frame.drawn = None;
        }
    }

    /// The tap buffer to bind for a program with `slots` tap slots, the first `reset` of them
    /// — all, where `None` — reset from the template in `encoder` ahead of the pass. `None`
    /// for a program with none.
    pub fn tap_buffer(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        slots: usize,
        reset: Option<usize>,
        source: u64,
    ) -> Option<wgpu::Buffer> {
        self.current = None;
        if slots == 0 {
            return None;
        }
        if self.taps.as_ref().is_none_or(|t| t.slots != slots) {
            self.size_taps(gpu, slots);
        }
        let taps = self.taps.as_ref()?;
        let bytes = taps.bytes();
        let at = self.front;
        self.front = (at + 1) % READBACKS;
        let frame = &mut self.frames[at];
        // The oldest record: collected by the settle before this, or still out where the GPU
        // fell a whole tick behind, when its frame's words are lost rather than waited for.
        if frame.staging.as_ref().is_none_or(|s| !s.is_idle()) {
            frame.staging = Some(Read::new(gpu, bytes, "taps staging"));
        }
        frame.drawn = None;
        // The program that draws this frame, not the one wanted: on the tick a new source
        // arrives the old program still draws, and its words are in the old layout.
        frame.source = Some(source);
        match reset.map(|n| (n.min(slots) * TAP_WORDS * 4) as u64) {
            None => encoder.copy_buffer_to_buffer(&taps.template, 0, &taps.buffer, 0, None),
            Some(0) => {}
            Some(bytes) => {
                encoder.copy_buffer_to_buffer(&taps.template, 0, &taps.buffer, 0, Some(bytes));
            }
        }
        self.current = Some(at);
        Some(taps.buffer.clone())
    }

    /// Make the tap buffer, its template and every record's staging for `slots` slots. A
    /// frame still in flight in a buffer let go here reads nothing.
    fn size_taps(&mut self, gpu: &Gpu, slots: usize) {
        let words: Vec<u8> = TAP_TEMPLATE
            .iter()
            .cycle()
            .take(slots * TAP_WORDS)
            .flat_map(|w| w.to_ne_bytes())
            .collect();
        let make = |label, usage| {
            gpu.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: words.len() as u64,
                usage,
                mapped_at_creation: false,
            })
        };
        let template = make(
            "tap template",
            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        gpu.queue().write_buffer(&template, 0, &words);
        let buffer = make(
            "taps",
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        );
        for frame in &mut self.frames {
            *frame = InFlight {
                staging: Some(Read::new(gpu, words.len() as u64, "taps staging")),
                ..InFlight::default()
            };
        }
        self.ready = None;
        self.taps = Some(Taps {
            slots,
            buffer,
            template,
        });
    }

    /// Issue the reads a draw owes after its pass: a thumbnail, a Snap or a capture of the
    /// slot just drawn, and the copy of its tap words into staging.
    pub fn after_pass(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        encoder: &mut wgpu::CommandEncoder,
        slot: &Slot,
    ) {
        if let (Some(at), Some(taps)) = (self.current, &self.taps)
            && let Some(staging) = &mut self.frames[at].staging
        {
            encoder.copy_buffer_to_buffer(&taps.buffer, 0, &staging.buffer, 0, None);
            staging.copied();
        }
        if !(self.wants_thumbnail || self.wants_snap || self.wants_capture) {
            return;
        }
        let blit = shared.picture();
        self.issue_thumbnail(gpu, blit, encoder, slot);
        self.issue_snap(gpu, blit, encoder, slot);
        self.issue_capture(gpu, blit, encoder, slot);
    }

    /// The frame drawn at ring count `drawn` went in `ticket`'s submission.
    pub fn submitted(&mut self, ticket: &Ticket, drawn: u64) {
        if let Some(at) = self.current.take() {
            let frame = &mut self.frames[at];
            if let Some(staging) = &mut frame.staging {
                staging.map(ticket);
                frame.drawn = Some(drawn);
            }
        }
        for picture in [&mut self.thumb, &mut self.snap].into_iter().flatten() {
            picture.read.map(ticket);
        }
        if let Some(c) = &mut self.capture {
            for read in &mut c.reads {
                read.map(ticket);
            }
        }
        self.last = Some((drawn, ticket.clone()));
    }

    /// The newest finished frame's tap words, from the source wanted, once.
    pub fn take_taps(&mut self) -> Option<Vec<u32>> {
        self.ready.take()
    }

    /// Ask for a picture of what this Output publishes: the next draw issues the read and a
    /// later [`Readbacks::poll`] collects it. Asking twice before one lands is asking once.
    pub fn request_thumbnail(&mut self) {
        self.wants_thumbnail = true;
    }

    /// True while a picture has been asked for and not yet taken.
    pub fn thumbnail_pending(&self) -> bool {
        self.wants_thumbnail || self.thumb.as_ref().is_some_and(Picture::pending)
    }

    /// RGBA8 at [`THUMBNAIL`], rows top first.
    pub fn take_thumbnail(&mut self) -> Option<Vec<u8>> {
        self.thumb.as_mut().and_then(|t| t.ready.take())
    }

    /// silvia's Snap: a thumbnail at the Output's own size.
    pub fn request_snap(&mut self) {
        self.wants_snap = true;
    }

    pub fn snap_pending(&self) -> bool {
        self.wants_snap || self.snap.as_ref().is_some_and(Picture::pending)
    }

    /// Size and RGBA8, rows top first.
    pub fn take_snap(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.snap.as_mut()?;
        let bytes = t.ready.take()?;
        Some((t.target.width, t.target.height, bytes))
    }

    /// Read back every frame from now on, in order, dropping none — or stop. `scale` is how
    /// much larger than the film the Output is drawn: 1, or 2 or 4 for a supersampled render.
    /// Turning on starts from nothing, the count at zero and the queue empty.
    pub fn set_capturing(&mut self, on: bool, scale: u32) {
        if on && !self.wants_capture {
            self.captures_issued = 0;
            self.captured.clear();
        }
        self.capture_scale = scale.max(1);
        self.wants_capture = on;
    }

    /// Capture reads issued since the capture was turned on: one per frame drawn.
    pub fn captures_issued(&self) -> u64 {
        self.captures_issued
    }

    /// True while a captured frame has landed and not been taken, or is still landing.
    pub fn capture_pending(&self) -> bool {
        !self.captured.is_empty() || self.capture.as_ref().is_some_and(|c| !c.reads.is_empty())
    }

    /// Every captured frame that has landed, oldest first, each taken once. RGBA8 at the
    /// film's size, rows top first.
    pub fn take_captured(&mut self) -> Vec<Vec<u8>> {
        self.captured.drain(..).collect()
    }

    /// Whether a picture has been asked for that only a draw makes: a thumbnail or a Snap
    /// not yet issued.
    pub fn wants_picture(&self) -> bool {
        self.wants_thumbnail || self.wants_snap
    }

    /// Collect thumbnails and captured frames whose reads have landed, on every tick whether
    /// or not the Output draws; once the capture is off, wait for the rest and let it go.
    pub fn poll(&mut self, gpu: &Gpu) {
        let due = [&self.thumb, &self.snap]
            .into_iter()
            .flatten()
            .any(|p| p.read.due(gpu))
            || self
                .capture
                .as_ref()
                .is_some_and(|c| c.reads.front().is_some_and(|r| r.due(gpu)));
        if due {
            gpu.poll();
        }
        for picture in [&mut self.thumb, &mut self.snap].into_iter().flatten() {
            picture.poll();
        }
        if let Some(c) = &mut self.capture {
            c.collect(gpu, &mut self.captured, !self.wants_capture);
        }
        if !self.wants_capture {
            self.capture = None;
        }
    }

    /// Letterbox the frame just drawn into the thumbnail target and start a read of it.
    fn issue_thumbnail(
        &mut self,
        gpu: &Gpu,
        blit: &Blit,
        encoder: &mut wgpu::CommandEncoder,
        slot: &Slot,
    ) {
        if !self.wants_thumbnail {
            return;
        }
        let thumb = self
            .thumb
            .get_or_insert_with(|| Picture::new(gpu, THUMBNAIL.0, THUMBNAIL.1, "thumbnail"));
        // A read still out keeps the request for a later frame.
        if !thumb.read.is_idle() {
            return;
        }
        issue_picture(gpu, blit, encoder, slot, thumb, Frame::Letterbox);
        self.wants_thumbnail = false;
    }

    /// Copy the frame just drawn, whole, into the Snap target and start a read of it.
    fn issue_snap(
        &mut self,
        gpu: &Gpu,
        blit: &Blit,
        encoder: &mut wgpu::CommandEncoder,
        slot: &Slot,
    ) {
        if !self.wants_snap {
            return;
        }
        // A resize between the ask and the issue leaves a target of the wrong size.
        if self.snap.as_ref().is_some_and(|t| {
            t.read.is_idle() && (t.target.width, t.target.height) != (slot.width, slot.height)
        }) {
            self.snap = None;
        }
        let snap = self
            .snap
            .get_or_insert_with(|| Picture::new(gpu, slot.width, slot.height, "snap"));
        if !snap.read.is_idle() {
            return;
        }
        issue_picture(gpu, blit, encoder, slot, snap, Frame::Whole);
        self.wants_snap = false;
    }

    /// Bring the frame just drawn down to the film in linear halvings and start a read of it.
    fn issue_capture(
        &mut self,
        gpu: &Gpu,
        blit: &Blit,
        encoder: &mut wgpu::CommandEncoder,
        slot: &Slot,
    ) {
        if !self.wants_capture {
            return;
        }
        let scale = self.capture_scale.max(1);
        let (cw, ch) = ((slot.width / scale).max(1), (slot.height / scale).max(1));
        let halves = scale > 2;
        if self
            .capture
            .as_ref()
            .is_some_and(|c| (c.film.width, c.film.height) != (cw, ch) || c.mid.is_some() != halves)
            && let Some(mut old) = self.capture.take()
        {
            old.collect(gpu, &mut self.captured, true);
        }
        let c = self
            .capture
            .get_or_insert_with(|| Capture::new(gpu, cw, ch, scale));
        // Collected rather than waited for: the settle before this draw waited for the frame
        // before it, whose read is the newest out.
        c.collect(gpu, &mut self.captured, false);
        let drawn = (slot.width, slot.height);
        let (from, from_size) = match &c.mid {
            Some(mid) => {
                let size = (mid.width, mid.height);
                blit.draw(gpu, encoder, &slot.view, drawn, mid, (0, 0), size);
                (&mid.view, size)
            }
            None => (&slot.view, drawn),
        };
        let film = (c.film.width, c.film.height);
        blit.draw(gpu, encoder, from, from_size, &c.film, (0, 0), film);
        let mut read = c.spare.pop().unwrap_or_else(|| c.film.read(gpu, "capture"));
        c.film.copy_into(encoder, &mut read);
        c.reads.push_back(read);
        self.captures_issued += 1;
    }
}

/// Draw the frame in `slot` into `picture`'s target, whole or letterboxed, and record the copy
/// into its read.
fn issue_picture(
    gpu: &Gpu,
    blit: &Blit,
    encoder: &mut wgpu::CommandEncoder,
    slot: &Slot,
    picture: &mut Picture,
    frame: Frame,
) {
    let target = &picture.target;
    let (tw, th) = (target.width, target.height);
    let src = (slot.width, slot.height);
    let (fw, fh) = match frame {
        Frame::Whole => (tw, th),
        Frame::Letterbox => {
            let scale = f32::min(tw as f32 / src.0 as f32, th as f32 / src.1 as f32);
            (
                ((src.0 as f32 * scale).round() as u32).clamp(1, tw),
                ((src.1 as f32 * scale).round() as u32).clamp(1, th),
            )
        }
    };
    let origin = ((tw - fw) / 2, (th - fh) / 2);
    blit.draw(gpu, encoder, &slot.view, src, target, origin, (fw, fh));
    target.copy_into(encoder, &mut picture.read);
}

/// **Tests only.** A texture's bytes as they lie, rows packed with no padding, row 0 first —
/// the bottom of an Output's picture. Copies into a staging buffer, submits and waits.
pub fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let texel = texture
        .format()
        .block_copy_size(None)
        .expect("a colour format has a texel size");
    let row = width * texel;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let staging = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("read_texture"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("read_texture"),
        });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    let ticket = gpu.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| {
        r.expect("the staging buffer maps");
    });
    gpu.wait(&ticket, READ_WAIT)
        .expect("the GPU finishes a read");
    // The map's callback has run once its submission has finished and the device is polled.
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device polls");
    let mapped = staging
        .slice(..)
        .get_mapped_range()
        .expect("the staging buffer is mapped");
    let mut out = Vec::with_capacity((row * height) as usize);
    for r in 0..height {
        let at = (r * padded) as usize;
        out.extend_from_slice(&mapped[at..at + row as usize]);
    }
    drop(mapped);
    staging.unmap();
    out
}

/// **Tests only.** An `Rgba16Float` texture's bytes, as [`read_texture`] returns them, as RGBA8
/// rounded as GL's `read_pixels` into bytes rounded them, rows as they lay.
pub fn half_to_rgba8(bytes: &[u8]) -> Vec<u8> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| (half(u16::from_ne_bytes(*b)).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

/// An IEEE half float's value.
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}
