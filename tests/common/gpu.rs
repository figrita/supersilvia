// SPDX-License-Identifier: AGPL-3.0-or-later

//! The wgpu tests' harness: one instance and one adapter per test process, a device per test,
//! and the helpers every `tests/gpu_*.rs` file shares — a hand-written module in the compiler's
//! conventions, a node graph's module and values, jobs, and reading a texture back.
//!
//! **One instance per process**, in a `OnceLock`: two threads in `vkCreateInstance` at once
//! segfaulted the Vulkan loader (`tests/ui.rs`'s module doc). The adapter is the integrated GPU,
//! [`adapter::Asked::integrated`] — a test's choice and not the app's, whose default is the
//! strongest GPU — so a GPU test never runs on a discrete GPU, nor on a software adapter.
//! **Every device panics on an uncaptured error**, so a validation error anywhere fails the
//! test that caused it.
//!
//! A test file takes it with `#[path = "common/gpu.rs"] mod gpu;`, so the files that do not
//! draw do not compile it.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use supersilvia::compile::wgsl::{self, Sampler};
use supersilvia::compile::{self, Shader, UniformProvider, UniformType};
use supersilvia::graph::{ControlValue, Graph, NodeId, PortRef};
use supersilvia::render::adapter;
use supersilvia::render::{
    FrameJob, Gpu, OutputJob, OutputMode, PassJob, PassKey, Renderer, UniformValue, readback,
};

/// The process's instance and adapter.
fn adapter() -> &'static (wgpu::Instance, wgpu::Adapter) {
    static ADAPTER: OnceLock<(wgpu::Instance, wgpu::Adapter)> = OnceLock::new();
    ADAPTER.get_or_init(|| {
        let (instance, adapter, _) =
            adapter::headless(&adapter::Asked::integrated()).unwrap_or_else(|e| panic!("{e}"));
        (instance, adapter)
    })
}

/// A device of its own on the process's adapter, which panics on any validation error.
pub fn gpu() -> Gpu {
    let (instance, adapter) = adapter();
    let gpu = Gpu::open(instance.clone(), adapter.clone()).expect("the adapter opens a device");
    gpu.device()
        .on_uncaptured_error(Arc::new(|e| panic!("wgpu validation: {e}")));
    gpu
}

/// Wait until the GPU has finished everything submitted on `gpu`.
pub fn drain(gpu: &Gpu) {
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finishes");
}

/// A texture's bytes as they lie, rows bottom first. Waits.
pub fn bytes_of(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    readback::read_texture(gpu, texture)
}

/// An IEEE half float's value.
pub fn half(bits: u16) -> f32 {
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

/// An `Rgba16Float` texture's channels as floats, rows bottom first.
pub fn floats_of(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<f32> {
    bytes_of(gpu, texture)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| half(u16::from_ne_bytes(*b)))
        .collect()
}

/// An `Rgba16Float` texture as RGBA8, rounded as GL's `read_pixels` into bytes rounds it, rows
/// bottom first.
pub fn rgba_of(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    floats_of(gpu, texture)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect()
}

/// A hand-written module in the compiler's conventions: `body` — which holds `fs_main` —
/// after the prelude, a uniform struct `u` with the standard two and one `f32` per name in
/// `floats`, the four samplers where there is a texture, and one `texture_2d<f32>` per name in
/// `textures`, the latest frame of that Output. Bindings follow `compile::wgsl::bindings`,
/// since the `Shader` built here is what the renderer reads them from.
pub fn module(floats: &[&str], textures: &[(&str, NodeId)], body: &str) -> Arc<Shader> {
    let mut uniforms: BTreeMap<Arc<str>, UniformProvider> = BTreeMap::new();
    for name in floats {
        uniforms.insert(
            Arc::from(*name),
            UniformProvider::Control {
                node: NodeId(0),
                key: "value",
                ty: UniformType::Float,
            },
        );
    }
    for (name, node) in textures {
        uniforms.insert(
            Arc::from(*name),
            UniformProvider::NodeTexture {
                node: *node,
                port: "frame",
            },
        );
    }
    let mut src = String::from(wgsl::PRELUDE);
    src.push_str("\nstruct Uniforms {\n    u_resolution: vec2f,\n    u_time: f32,\n");
    for (name, provider) in &uniforms {
        if let Some(ty) = provider.ty().wgsl() {
            writeln!(src, "    {name}: {ty},").unwrap();
        }
    }
    src.push_str("}\n@group(0) @binding(0) var<uniform> u: Uniforms;\n");
    if !textures.is_empty() {
        for (sampler, binding) in Sampler::ALL.iter().zip(wgsl::SAMPLERS..) {
            writeln!(
                src,
                "@group(0) @binding({binding}) var {}: sampler;",
                sampler.name()
            )
            .unwrap();
        }
    }
    let texture_names = uniforms
        .iter()
        .filter(|(_, p)| p.ty() == UniformType::Sampler2D)
        .map(|(n, _)| n);
    for (name, binding) in texture_names.zip(wgsl::TEXTURES..) {
        writeln!(
            src,
            "@group(0) @binding({binding}) var {name}: texture_2d<f32>;"
        )
        .unwrap();
    }
    src.push_str(body);
    Arc::new(Shader {
        body: src,
        uniforms,
        diagnostics: Vec::new(),
        taps: Vec::new(),
        thumbs: Vec::new(),
        grid: 0,
    })
}

/// A module drawing one colour everywhere.
pub fn solid(r: f32, g: f32, b: f32) -> Arc<Shader> {
    module(
        &[],
        &[],
        &format!(
            "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    return vec4f({r:?}, {g:?}, {b:?}, 1.0);
}}
"
        ),
    )
}

/// An Output's job: `shader`, sent this tick where `send` says, drawn or not by `mode`, with
/// these values and no taps.
pub fn job(
    node: NodeId,
    size: (u32, u32),
    shader: &Arc<Shader>,
    send: bool,
    mode: OutputMode,
    uniforms: Vec<(Arc<str>, UniformValue)>,
) -> OutputJob {
    OutputJob {
        node,
        resolution: size,
        shader: send.then(|| Arc::clone(shader)),
        source: compile::source_hash(&shader.body),
        mode,
        uniforms,
        taps: shader.taps.len(),
        clear: false,
    }
}

/// The job's uniforms reading every texture name in `reads` from that Output's frame.
pub fn reading(reads: &[(&str, NodeId)]) -> Vec<(Arc<str>, UniformValue)> {
    reads
        .iter()
        .map(|(name, node)| {
            (
                Arc::<str>::from(*name),
                UniformValue::NodeTexture(PortRef::new(*node, "frame")),
            )
        })
        .collect()
}

/// A tick of `outputs` at `time`.
pub fn tick(time: f32, outputs: Vec<OutputJob>) -> FrameJob {
    FrameJob {
        time,
        outputs,
        ..FrameJob::default()
    }
}

/// Send every Output's module, on a tick no program can draw yet, then draw with every one
/// suspended until all have landed, so no frame is drawn before the test's first tick.
pub fn link_all(renderer: &mut Renderer, outputs: &dyn Fn(bool, OutputMode) -> Vec<OutputJob>) {
    renderer.draw(&tick(0.0, outputs(true, OutputMode::Draw)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        renderer.draw(&tick(0.0, outputs(false, OutputMode::Suspended)));
        if outputs(false, OutputMode::Suspended)
            .iter()
            .all(|o| !renderer.awaiting_shader(o.node) && !renderer.is_linking(o.node))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the modules never linked"
        );
    }
    for o in outputs(false, OutputMode::Suspended) {
        assert_eq!(renderer.errors.get(&o.node), None);
    }
}

/// `slug`'s first output cabled into an Output: the graph, and the two nodes.
pub fn one_node(slug: &str) -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let src = supersilvia::nodes::add_to_graph(&mut g, slug, emath::Pos2::ZERO).expect("node");
    let out =
        supersilvia::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO).expect("output");
    let key = g.get(src).expect("node").outputs[0].key;
    g.connect(PortRef::new(src, key), PortRef::new(out, "input"))
        .expect("connect");
    (g, src, out)
}

/// The Output's WGSL module and the values the app would resolve from its nodes' controls.
pub fn compiled(g: &Graph, out: NodeId) -> (Arc<Shader>, Vec<(Arc<str>, UniformValue)>) {
    let shader = wgsl::build(g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let values = shader
        .uniforms
        .iter()
        .filter_map(|(name, p)| {
            let UniformProvider::Control { node, key, .. } = p else {
                return None;
            };
            let value = match g.get(*node)?.controls.get(key)? {
                ControlValue::Float(v) => UniformValue::Float(*v),
                ControlValue::Color(v) => {
                    UniformValue::Vec4(supersilvia::nodes::alpha::premultiply(*v))
                }
            };
            Some((Arc::clone(name), value))
        })
        .collect();
    (Arc::new(shader), values)
}

/// The values the app would resolve for `shader` from `g`: every control and option, every
/// texture from its port, and every published uniform at zero.
pub fn resolved(g: &Graph, shader: &Shader) -> Vec<(Arc<str>, UniformValue)> {
    shader
        .uniforms
        .iter()
        .filter_map(|(name, provider)| {
            let value = match provider {
                UniformProvider::Control { node, key, .. } => {
                    match g.get(*node)?.controls.get(key)? {
                        ControlValue::Float(v) => UniformValue::Float(*v),
                        ControlValue::Color(v) => {
                            UniformValue::Vec4(supersilvia::nodes::alpha::premultiply(*v))
                        }
                    }
                }
                UniformProvider::NodeTexture { node, port } => {
                    UniformValue::NodeTexture(PortRef::new(*node, port))
                }
                UniformProvider::NodeUniform { ty, .. } => match ty {
                    UniformType::Vec4 => UniformValue::Vec4([0.0; 4]),
                    _ => UniformValue::Float(0.0),
                },
                UniformProvider::NodeCount { .. } => UniformValue::Vec2([0.0; 2]),
                UniformProvider::Option { node, key } => {
                    let n = g.get(*node)?;
                    let value = n.options.get(key).map_or("", |v| v.as_str());
                    UniformValue::Int(n.def.option(key)?.index_of(value))
                }
            };
            Some((Arc::clone(name), value))
        })
        .collect()
}

/// Every node of `g` with a measurement, in id order: what its workspace's pass measures.
pub fn measured(g: &Graph) -> Vec<NodeId> {
    g.iter()
        .filter(|(_, n)| n.def.measure_wgsl.is_some())
        .map(|(id, _)| id)
        .collect()
}

/// The words a workspace's pass reads back: `g`'s default workspace measuring every node of
/// `measured`'s, with no thumbnail, drawn for `frames` ticks after `out` at `size` — which a
/// measurement may sample — each waited for. A reading arrives a tick late, so two ticks is
/// the least that gives one.
pub fn pass_words(g: &Graph, out: NodeId, frames: usize, size: (u32, u32)) -> Vec<u32> {
    let workspace = g.default_workspace();
    let pass = Arc::new(
        wgsl::build_pass(g, workspace, &measured(g), false)
            .pop()
            .expect("the graph measures"),
    );
    let key = PassKey {
        workspace,
        batch: 0,
    };
    assert!(pass.diagnostics.is_empty(), "{:?}", pass.diagnostics);
    let picture = wgsl::build(g, out).map(Arc::new);
    let gpu = gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let job_of = |send: bool| {
        let outputs = picture
            .iter()
            .map(|s| job(out, size, s, send, OutputMode::Draw, resolved(g, s)))
            .collect();
        FrameJob {
            outputs,
            passes: vec![PassJob {
                key,
                shader: send.then(|| Arc::clone(&pass)),
                source: compile::source_hash(&pass.body),
                resolution: pass.pass_size(),
                region: None,
                uniforms: resolved(g, &pass),
                taps: pass.slots(),
                draws: true,
            }],
            ..FrameJob::default()
        }
    };
    renderer.draw(&job_of(true));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !renderer.pass_linked(key)
        || picture.is_some() && (renderer.awaiting_shader(out) || renderer.is_linking(out))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the modules never linked"
        );
        drain(&gpu);
        renderer.draw(&job_of(false));
    }
    let mut words = Vec::new();
    for _ in 0..frames {
        drain(&gpu);
        renderer.draw(&job_of(false));
        if let Some((_, w)) = renderer.take_passes().pop() {
            words = w;
        }
    }
    words
}
