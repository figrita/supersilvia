// SPDX-License-Identifier: AGPL-3.0-or-later

//! The mix of the two decks, silvia's `MainMixer` — the draw's **mix** phase, after every
//! Output and the probes, so the decks are the frames just drawn.
//!
//! **One pipeline, made with the renderer and never replaced.** Where an Output is rebuilt by
//! every structural edit upstream of it, the mix has no upstream: [`MIX`] is written against
//! two textures rather than any graph, the method is an `i32` in its uniform block, and all
//! eight branches are in the one module. So deck B can be built, undone and relinked while
//! deck A is on air. See [docs/rendering.md](../../docs/rendering.md#the-mixer) and
//! `proposals/wgpu.md`, 1.12.
//!
//! **A ring of its own**, of [`Format::Byte`], made on the first tick and carried to a new size
//! on the tick the size changes, so a resize shows the last mix scaled rather than black. What
//! a viewer is shown follows the Outputs' rule — the newest mix whose submission has finished.
//! A mix is skipped only when every target is held or still on the GPU, which is a drop, or
//! when neither deck has a picture: two empty decks are black whatever the fade says, so once
//! one black mix is drawn nothing is drawn until a deck has a picture again, and that skip is
//! not a drop.
//!
//! **Blackout and Freeze are here, after the mix and before every viewer**, so the panel, the
//! canvas behind the graph, a picture window, NDI and Syphon all show the same held picture.
//! Freeze draws no mix, and the newest one finished goes on being shown; Blackout publishes a
//! black texel at the mix's size in its place while the mix goes on being drawn underneath, so
//! letting go shows the mix as it stands — or the frozen frame, where Freeze is still held.
//!
//! **A claim allocates nothing**: the tick's bind group names the deck's latest view, or the
//! shared black texel for an empty deck. The fragment builds its `uv` from
//! `@builtin(position)`, so the mix keeps GL's rows, bottom first, as an Output's frame does
//! (`proposals/wgpu.md`, 1.15).

use super::Published;
use super::gpu::{Gpu, Ticket};
use super::queue::Recording;
use super::ring::{Format, Ring};
use super::shared::{self, Shared};
use super::{Picture, Texture};
use crate::compile::wgsl::Sampler;
use crate::graph::NodeId;

/// What the mixer should draw this frame: the two decks and the fade between them.
///
/// A deck names an Output; one with no renderer, or no program, samples black. The mixer
/// itself carries no shader here, because it never has a new one.
#[derive(Debug, Clone, PartialEq)]
pub struct MixerJob {
    pub a: Option<NodeId>,
    pub b: Option<NodeId>,
    pub balance: f32,
    pub method: crate::mixer::Method,
    pub resolution: (u32, u32),
    /// Show black in place of the mix: [`crate::mixer::Mixer::blackout`].
    pub blackout: bool,
    /// Draw no new mix, so the last one goes on being shown: [`crate::mixer::Mixer::freeze`].
    pub freeze: bool,
}

impl Default for MixerJob {
    fn default() -> Self {
        Self {
            a: None,
            b: None,
            balance: -1.0,
            method: crate::mixer::Method::Blend,
            resolution: crate::nodes::output::DEFAULT_RESOLUTION,
            blackout: false,
            freeze: false,
        }
    }
}

/// The crossfade, ported from silvia's `MIXING_FRAGMENT_SHADER`.
///
/// Each deck is scaled about its center by the mix's aspect over its own, full height, so a
/// deck of another shape is cropped or mirrored out to the sides — the sampler mirrors — and
/// never letterboxed; the mix fills the projector, as silvia's does. The branch taken is
/// `method`, [`crate::mixer::Method::index`]: all eight are in the one pipeline and nothing
/// ever rebuilds it.
pub const MIX: &str = "
struct Mix {
    resolution: vec2f,
    balance: f32,
    aspect_a: f32,
    aspect_b: f32,
    method: i32,
    pad: vec2f,
}
@group(0) @binding(0) var<uniform> m: Mix;
@group(0) @binding(1) var deck_sampler: sampler;
@group(0) @binding(2) var channel_a: texture_2d<f32>;
@group(0) @binding(3) var channel_b: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

fn mix_odd(x: f32) -> bool {
    return x - 2.0 * floor(x / 2.0) > 0.5;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / m.resolution;
    let viewport_aspect = m.resolution.x / m.resolution.y;
    let uv_a = vec2f((uv.x - 0.5) * (viewport_aspect / m.aspect_a) + 0.5, uv.y);
    let uv_b = vec2f((uv.x - 0.5) * (viewport_aspect / m.aspect_b) + 0.5, uv.y);
    let color_a = textureSampleLevel(channel_a, deck_sampler, uv_a, 0.0);
    let color_b = textureSampleLevel(channel_b, deck_sampler, uv_b, 0.0);

    let amount = clamp(0.5 + 0.5 * tan(clamp(m.balance, -0.999, 0.999) * 1.5707963), 0.0, 1.0);
    let method = m.method;

    if (method == 1) {
        return select(color_a, color_b, uv.x < amount);
    } else if (method == 2) {
        return select(color_a, color_b, uv.y < amount);
    } else if (method == 3) {
        let offset = uv - vec2f(0.5, 0.5);
        let max_dist = sqrt(0.25 + 0.25 * viewport_aspect * viewport_aspect);
        let dist = length(vec2f(offset.x * viewport_aspect, offset.y));
        return select(color_a, color_b, dist / max_dist < amount);
    } else if (method == 4) {
        let lum_a = dot(color_a.rgb, vec3f(0.299, 0.587, 0.114));
        return select(color_a, color_b, lum_a < amount);
    } else if (method == 5) {
        let lum_a = dot(color_a.rgb, vec3f(0.299, 0.587, 0.114));
        return select(color_a, color_b, lum_a > 1.0 - amount);
    } else if (method == 6) {
        let checker = floor(uv * 8.0);
        let show_b = select((1.0 - uv.y) < amount, uv.y < amount, mix_odd(checker.x + checker.y));
        return select(color_a, color_b, show_b);
    } else if (method == 7) {
        let checker = floor(uv * vec2f(16.0, 8.0));
        let show_b = select((1.0 - uv.x) < amount, uv.x < amount, mix_odd(checker.y));
        return select(color_a, color_b, show_b);
    }
    return mix(color_a, color_b, amount);
}
";

/// The size of `Mix` in [`MIX`], in bytes.
const BLOCK: u64 = 32;

/// A deck's picture this tick: an Output's latest frame and its size.
pub struct Deck<'a> {
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl<'a> Deck<'a> {
    /// The view, and the aspect the mix scales the deck by.
    fn split(self) -> (&'a wgpu::TextureView, f32) {
        (self.view, self.width as f32 / self.height.max(1) as f32)
    }
}

/// The mix and its ring.
pub struct Mixer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The one draw's uniform block, rewritten on the tick it draws.
    block: wgpu::Buffer,
    /// Made on the first tick.
    ring: Option<Ring>,
    /// Mixes not drawn because every target was held or on the GPU, since the run began.
    dropped: u64,
    /// The latest mix is of two empty decks, so another of them would be the same black.
    blank: bool,
    /// One opaque black texel, published in the mix's place under Blackout.
    black: Texture,
    /// The tick Blackout began on, while it holds: the black picture's `drawn_tick`, so an
    /// outlet sends it once rather than every tick.
    dark_since: Option<u64>,
    /// The tick Blackout was last let go on. No mix is published as drawn earlier than this,
    /// so a frozen frame shown again after the black is newer than the black to an outlet
    /// that sends only what is newer than what it sent — NDI.
    lit_since: u64,
}

impl Mixer {
    /// The pipeline, made here once and never again.
    pub fn new(gpu: &Gpu) -> Self {
        let device = gpu.device();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mix"),
            source: wgpu::ShaderSource::Wgsl(MIX.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mix"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(BLOCK),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2),
                texture(3),
            ],
        });
        let pipeline = shared::fullscreen(
            device,
            &module,
            &layout,
            Format::Byte.texture_format(),
            "mix",
        );
        let block = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mix"),
            size: BLOCK,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let black = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blackout"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Format::Byte.texture_format(),
            // Readable as the mix itself is, so whatever reads the mix back reads this too.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        gpu.queue().write_texture(
            black.as_image_copy(),
            &[0, 0, 0, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            black.size(),
        );
        let view = black.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            pipeline,
            layout,
            block,
            ring: None,
            dropped: 0,
            blank: false,
            black: Texture::new(black, view),
            dark_since: None,
            lit_since: 0,
        }
    }

    /// Make the ring on the first tick, or carry it to the job's resolution, recording into
    /// `recording`. `Err` explains a mix that could not be made at that size; the ring stays
    /// at the size it has.
    pub fn sync(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        job: &MixerJob,
        tick: u64,
    ) -> Result<(), String> {
        match (job.blackout, self.dark_since) {
            (true, None) => self.dark_since = Some(tick),
            (false, Some(_)) => {
                self.dark_since = None;
                self.lit_since = tick;
            }
            _ => {}
        }
        let (width, height) = (job.resolution.0.max(1), job.resolution.1.max(1));
        let largest = gpu.device().limits().max_texture_dimension_2d;
        let fits = width <= largest && height <= largest;
        match &mut self.ring {
            Some(ring) => {
                ring.set_tick(tick);
                ring.sweep();
                if fits && ring.size() != (width, height) {
                    ring.resize(gpu, shared, recording.encoder(), width, height);
                }
            }
            None if fits => {
                let mut ring = Ring::new(gpu, recording.encoder(), width, height, Format::Byte);
                ring.set_tick(tick);
                self.ring = Some(ring);
            }
            None => {}
        }
        if fits {
            Ok(())
        } else {
            Err(format!(
                "a {width}x{height} mix is larger than this GPU's largest texture, {largest}"
            ))
        }
    }

    /// Record this tick's mix of `a` and `b` into `recording`, where there is a target free.
    /// An empty deck is black. Under Freeze nothing is drawn, and the newest finished mix
    /// goes on being the one shown.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        job: &MixerJob,
        a: Option<Deck<'_>>,
        b: Option<Deck<'_>>,
    ) {
        if job.freeze {
            return;
        }
        let Some(ring) = &mut self.ring else {
            return;
        };
        let (a, b) = (a.map(Deck::split), b.map(Deck::split));
        let blank = a.is_none() && b.is_none();
        if blank && self.blank {
            return;
        }
        let Some(slot) = ring.free_slot(gpu, gpu.completed()) else {
            self.dropped += 1;
            self.blank = false;
            return;
        };
        let (width, height) = ring.size();
        let aspect = |d: Option<(&wgpu::TextureView, f32)>| d.map_or(1.0, |(_, aspect)| aspect);
        let mut block = [0u8; BLOCK as usize];
        let words: [[u8; 4]; 6] = [
            (width as f32).to_le_bytes(),
            (height as f32).to_le_bytes(),
            job.balance.clamp(-1.0, 1.0).to_le_bytes(),
            aspect(a).to_le_bytes(),
            aspect(b).to_le_bytes(),
            job.method.index().to_le_bytes(),
        ];
        for (at, word) in words.iter().enumerate() {
            block[at * 4..at * 4 + 4].copy_from_slice(word);
        }
        gpu.queue().write_buffer(&self.block, 0, &block);
        let view_a = a.map_or(shared.black(), |(view, _)| view);
        let view_b = b.map_or(shared.black(), |(view, _)| view);
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(shared.sampler(Sampler::MirrorLinear)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(view_b),
                },
            ],
        });
        {
            let mut pass = shared::begin(
                recording.encoder(),
                &ring.slot(slot).view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                "mix",
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        ring.commit(slot);
        self.blank = blank;
    }

    /// What was recorded since the last submission went in `ticket`'s.
    pub fn submitted(&mut self, ticket: &Ticket) {
        if let Some(ring) = &mut self.ring {
            ring.submitted(ticket.serial);
        }
    }

    /// Mark every mix whose submission has finished.
    pub fn poll_frames(&mut self, completed: u64) {
        if let Some(ring) = &mut self.ring {
            ring.poll(completed);
        }
    }

    /// Put the newest finished mix into `out`, with a claim on it — or, under Blackout, a
    /// black picture of the same size, which needs none.
    pub fn publish(&self, out: &mut Published) {
        let Some(ring) = &self.ring else {
            return;
        };
        if let Some(since) = self.dark_since {
            let (width, height) = ring.size();
            out.mixer = Some(Picture {
                texture: self.black.clone(),
                width,
                height,
                flip: false,
                sampler: Sampler::MirrorLinear,
                drawn_tick: Some(since),
            });
            return;
        }
        let Some((slot, lease)) = ring.shown() else {
            return;
        };
        out.holds.push(lease);
        out.mixer = Some(Picture {
            texture: Texture::new(slot.texture.clone(), slot.view.clone()),
            width: slot.width,
            height: slot.height,
            flip: false,
            sampler: Sampler::MirrorLinear,
            drawn_tick: ring.shown_tick().map(|t| t.max(self.lit_since)),
        });
    }

    /// The mix drawn last. For a test.
    pub fn texture(&self) -> Option<wgpu::Texture> {
        self.ring.as_ref().map(|r| r.latest().texture.clone())
    }

    /// Every target the mix holds. For a test.
    pub fn targets(&self) -> Vec<wgpu::Texture> {
        self.ring.as_ref().map(Ring::textures).unwrap_or_default()
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.ring.as_ref().map(Ring::size)
    }

    /// Mixes not drawn because every target was held or on the GPU.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
