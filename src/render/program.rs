// SPDX-License-Identifier: AGPL-3.0-or-later

//! A pipeline made from one compiled WGSL module, and what it binds, with no reflection.
//!
//! Everything a module binds is in group 0 and follows from its `compile::Shader` alone
//! (`compile::wgsl`): the uniform struct at 0, the tap buffer at 1 where there are taps, the
//! four samplers from 2 where there is a texture, and a texture per `NodeTexture` uniform from
//! 6 up. [`Program::create`] reads `wgsl::bindings` for the group's layout and keeps
//! `wgsl::uniform_layout` to pack the struct each draw — "uniform locations are cached on
//! link" becomes "a block layout is computed on link". The uniform struct is bound with a
//! dynamic offset into the tick's one uniform buffer ([`super::uniforms`]).
//!
//! [`Program::create`] blocks for as long as the driver compiles, so the renderer calls it only
//! on a `linker` thread ([`super::link`]). A module that does not parse, or a pipeline the
//! device refuses, is caught by a validation error scope around the creation — thread-local,
//! so it holds that link's errors and no other thread's — and comes back as the message, for
//! the status line; the program before it keeps drawing. See `proposals/wgpu.md`, 1.8 and
//! 1.13.

use super::gpu::Gpu;
use crate::compile::Shader;
use crate::compile::wgsl::{self, Resource, UniformLayout};

/// A linked module: its pipeline, the layout of its one group, and where its uniforms go.
pub struct Program {
    /// `compile::source_hash` of the module it was made from.
    pub source: u64,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Where each member of the uniform struct sits, and its size.
    pub uniforms: UniformLayout,
    /// What goes at each binding, in binding order.
    pub bindings: Vec<(u32, Resource)>,
    /// How many of the tap buffer's slots each frame resets from the template: `None` for
    /// all of them. A workspace pass's thumbnails lie past its tap slots and are not reset,
    /// so a frame that draws only its measurements leaves the thumbnails the last full frame
    /// wrote rather than the template's zeros.
    pub reset: Option<usize>,
}

impl Program {
    /// The pipeline drawing `shader`'s module into a `format` target, or the error that
    /// refused it. Waits for the driver's compile.
    pub fn create(gpu: &Gpu, shader: &Shader, format: wgpu::TextureFormat) -> Result<Self, String> {
        let device = gpu.device();
        let bindings = wgsl::bindings(shader);
        let uniforms = wgsl::uniform_layout(shader);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("output"),
            source: wgpu::ShaderSource::Wgsl(shader.body.as_str().into()),
        });
        let entries: Vec<wgpu::BindGroupLayoutEntry> = bindings
            .iter()
            .map(|(binding, resource)| layout_entry(*binding, resource, &uniforms))
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output"),
            entries: &entries,
        });
        let pipeline = super::shared::fullscreen(device, &module, &layout, format, "output");
        if let Some(error) = crate::render::adapter::block_on(scope.pop()) {
            return Err(error.to_string());
        }
        Ok(Self {
            source: crate::compile::source_hash(&shader.body),
            pipeline,
            layout,
            uniforms,
            bindings,
            reset: (!shader.thumbs.is_empty()).then_some(shader.taps.len()),
        })
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Whether the module binds a tap buffer.
    pub fn taps(&self) -> bool {
        self.bindings.iter().any(|(_, r)| *r == Resource::Taps)
    }
}

/// How one binding is declared to the pipeline.
fn layout_entry(
    binding: u32,
    resource: &Resource,
    uniforms: &UniformLayout,
) -> wgpu::BindGroupLayoutEntry {
    let ty = match resource {
        Resource::Uniforms => wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(u64::from(uniforms.size)),
        },
        Resource::Taps => wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        Resource::Sampler(_) => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        Resource::Texture(_) => wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
    };
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}
