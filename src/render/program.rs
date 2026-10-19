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
//!
//! **A module whose repeats list their copies** (`compile::instances`) also makes a compute
//! pipeline per pass of the list's build, over the same group, and the program owns the four
//! buffers they write and the picture reads, sized from the `Shader`. A draw dispatches the
//! passes before its picture ([`Program::build_instances`]).

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
    /// The listed copies' build, where the module lists any.
    instances: Option<Instances>,
}

/// A module's listed copies: a compute pipeline per pass, in `instances::ENTRIES` order, their
/// workgroups, and the buffers at `instances::INSTANCES` up.
struct Instances {
    pipelines: Vec<wgpu::ComputePipeline>,
    passes: Vec<(usize, (u32, u32, u32))>,
    buffers: [wgpu::Buffer; 4],
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
        let instances = shader
            .instancing
            .any()
            .then(|| instances(device, &module, &layout, shader.instancing));
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
            instances,
        })
    }

    /// The buffer a listed-copies binding names. Only asked of a program that lists them.
    pub fn instance_buffer(&self, resource: &Resource) -> &wgpu::Buffer {
        let Some(instances) = &self.instances else {
            unreachable!("a binding of listed copies in a module that lists none")
        };
        let at = match resource {
            Resource::Instances => 0,
            Resource::Grids => 1,
            Resource::Counts => 2,
            _ => 3,
        };
        &instances.buffers[at]
    }

    /// Record the passes that list every repeat's copies, over `group` bound at `offset`: a
    /// compute pass each, so each reads what the one before wrote.
    pub fn build_instances(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        group: &wgpu::BindGroup,
        offset: u32,
    ) {
        let Some(instances) = &self.instances else {
            return;
        };
        for (entry, (x, y, z)) in &instances.passes {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("instances"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&instances.pipelines[*entry]);
            pass.set_bind_group(0, group, &[offset]);
            pass.dispatch_workgroups(*x, *y, *z);
        }
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

/// A module's listed copies: a pipeline per entry point over `layout`, and the buffers.
fn instances(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    instancing: crate::compile::instances::Instancing,
) -> Instances {
    use crate::compile::instances::ENTRIES;
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("instances"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let pipelines = ENTRIES
        .iter()
        .map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
        .collect();
    let passes = instancing.dispatches();
    let buffers = instancing.sizes().map(|size| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    });
    Instances {
        pipelines,
        passes,
        buffers,
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
        Resource::Instances | Resource::Grids | Resource::Counts | Resource::Lists => {
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            }
        }
    };
    // The compute passes listing a repeat's copies bind the picture's group.
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}
