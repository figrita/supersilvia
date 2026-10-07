// SPDX-License-Identifier: AGPL-3.0-or-later

//! What every draw of the renderer shares, made once with it: the four samplers a module's
//! textures read through, the black a sampler reads where its texture is not there, and the
//! pass that carries a frame into a target of another size.
//!
//! **Wrap and filter belong to a sampler in wgpu**, not to a texture as `glTexParameteri` made
//! them. The four a module can name — mirror or repeat, linear or nearest, in
//! `compile::wgsl::Sampler::ALL`'s order — are made here once and bound into every group, so a
//! texture output's declaration picks one and allocates nothing (`proposals/wgpu.md`, 1.8).
//!
//! **The carry reads by `@builtin(position)`, never by an interpolated vertex coordinate**, so
//! it writes the same rows GL's `glBlitFramebuffer` did (`proposals/wgpu.md`, 1.15). There is
//! no blit in wgpu: a resize is a pass into the new target, the old frame scaled about its
//! center to full height by the new aspect over the old, exactly as the mixer scales a deck of
//! another shape ([`super::mixer::MIX`]) — cropped or mirrored out to the sides, never
//! stretched, never letterboxed. The sampler mirrors for the same reason.

use super::gpu::Gpu;
use super::ring::Format;
use crate::compile::wgsl::Sampler;

/// The renderer's own stage for a resize's carry: the old frame, scaled about its center to
/// full height into the new target, by the new aspect over the old.
pub const CARRY: &str = "
@group(0) @binding(0) var old_frame: texture_2d<f32>;
@group(0) @binding(1) var old_sampler: sampler;
struct Carry { size: vec2f, old_size: vec2f }
@group(0) @binding(2) var<uniform> carry: Carry;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / carry.size;
    let new_aspect = carry.size.x / carry.size.y;
    let old_aspect = carry.old_size.x / carry.old_size.y;
    let scaled = vec2f((uv.x - 0.5) * (new_aspect / old_aspect) + 0.5, uv.y);
    return textureSampleLevel(old_frame, old_sampler, scaled, 0.0);
}
";

/// A sampler reading as `kind` says: mirrored or repeated outside the texture, and linear or
/// nearest between its texels. No texture has mips, so the mip filter never matters.
pub fn sampler(device: &wgpu::Device, kind: Sampler) -> wgpu::Sampler {
    let (address, filter) = match kind {
        Sampler::MirrorLinear => (wgpu::AddressMode::MirrorRepeat, wgpu::FilterMode::Linear),
        Sampler::MirrorNearest => (wgpu::AddressMode::MirrorRepeat, wgpu::FilterMode::Nearest),
        Sampler::RepeatLinear => (wgpu::AddressMode::Repeat, wgpu::FilterMode::Linear),
        Sampler::RepeatNearest => (wgpu::AddressMode::Repeat, wgpu::FilterMode::Nearest),
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(kind.name()),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// A fullscreen pass's pipeline and the layout of its one group.
struct Stage {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

pub struct Shared {
    /// In `Sampler::ALL` order.
    samplers: [wgpu::Sampler; 4],
    /// One opaque black texel: what a sampler reads where its texture is not there.
    black: wgpu::TextureView,
    carry_half: Stage,
    carry_byte: Stage,
    /// What a thumbnail, a Snap and a capture are drawn with.
    picture: super::readback::Blit,
}

impl Shared {
    pub fn new(gpu: &Gpu) -> Self {
        let device = gpu.device();
        let samplers = Sampler::ALL.map(|kind| sampler(device, kind));
        let black = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("black"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let black = black.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            samplers,
            black,
            carry_half: carry(device, Format::Half.texture_format()),
            carry_byte: carry(device, Format::Byte.texture_format()),
            picture: super::readback::Blit::new(device),
        }
    }

    /// The pass a thumbnail, a Snap and a capture are drawn by.
    pub fn picture(&self) -> &super::readback::Blit {
        &self.picture
    }

    /// The sampler a module binds for `kind`.
    pub fn sampler(&self, kind: Sampler) -> &wgpu::Sampler {
        let at = Sampler::ALL
            .iter()
            .position(|s| *s == kind)
            .expect("every sampler is in ALL");
        &self.samplers[at]
    }

    /// One opaque black texel.
    pub fn black(&self) -> &wgpu::TextureView {
        &self.black
    }

    /// Record a pass carrying `from`, of `old_size`, into `to`, which is `size` and of
    /// `format` — scaled about its center to full height, as a frame of another shape always
    /// is.
    pub fn carry(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        from: (&wgpu::TextureView, (u32, u32)),
        to: &wgpu::TextureView,
        size: (u32, u32),
        format: Format,
    ) {
        let (from, old_size) = from;
        let stage = match format {
            Format::Half => &self.carry_half,
            Format::Byte => &self.carry_byte,
        };
        let mut block = [0u8; 16];
        block[..4].copy_from_slice(&(size.0 as f32).to_le_bytes());
        block[4..8].copy_from_slice(&(size.1 as f32).to_le_bytes());
        block[8..12].copy_from_slice(&(old_size.0 as f32).to_le_bytes());
        block[12..16].copy_from_slice(&(old_size.1 as f32).to_le_bytes());
        let uniforms = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("carry"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue().write_buffer(&uniforms, 0, &block);
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("carry"),
            layout: &stage.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(from),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(self.sampler(Sampler::MirrorLinear)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniforms.as_entire_binding(),
                },
            ],
        });
        let mut pass = begin(encoder, to, wgpu::LoadOp::Load, "carry");
        pass.set_pipeline(&stage.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A render pass over the whole of `target`, loading or clearing it.
pub fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    label: &str,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Record a pass clearing `target` to `color`.
pub fn clear(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, color: wgpu::Color) {
    drop(begin(encoder, target, wgpu::LoadOp::Clear(color), "clear"));
}

fn carry(device: &wgpu::Device, format: wgpu::TextureFormat) -> Stage {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("carry"),
        source: wgpu::ShaderSource::Wgsl(CARRY.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("carry"),
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
    let pipeline = fullscreen(device, &module, &layout, format, "carry");
    Stage { pipeline, layout }
}

/// A pipeline drawing one triangle over the whole target from `module`'s `vs_main` and
/// `fs_main`, with nothing blended.
pub fn fullscreen(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    fullscreen_from(
        device,
        module,
        layout,
        format,
        label,
        crate::compile::wgsl::FRAGMENT_ENTRY,
    )
}

/// [`fullscreen`] with the fragment stage `module`'s `fragment` entry point.
pub fn fullscreen_from(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    label: &str,
    fragment: &str,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(crate::compile::wgsl::VERTEX_ENTRY),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}
