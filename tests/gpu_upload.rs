// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's uploads: a CPU node's frame sampled by a consumer, two texture outputs of one node as two
//! textures, every layout a source makes — RGB in both byte orders with and without a padding
//! byte, YUV planar, semi-planar and packed, rows padded — coming out the picture RGBA would
//! be, a refused import falling back to bytes, and an import neither written nor let go early.
//! Then zero flash for sources: a frame is sampled on the tick it arrives, a change of size or
//! layout never shows an empty texture, a frame that does not fit leaves the last one up, and
//! a viewer keeps the frame it was handed.
//!
//! Wrap and filter belong to a sampler on wgpu, so the wrap tests read the uploaded texture
//! through the sampler its `Picture` names, and a change of declaration is asserted to
//! reallocate nothing.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain, job, link_all, module, rgba_of};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use supersilvia::compile::Shader;
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::nodes::{
    DmaBuf, Frame, Layout, Mapped, Pixels, Planes, TextureFilter, TextureWrap, Yuv, YuvMatrix,
};
use supersilvia::render::{FrameJob, Gpu, OutputMode, Renderer, SourceJob, shared};

const DRAW: OutputMode = OutputMode::Draw;

// ------------------------------------------------------------------------------ helpers

/// Hand the renderer one frame on one port, with nothing drawing it, declaring how it is to
/// be sampled, and let the GPU finish.
fn publish(
    gpu: &Gpu,
    renderer: &mut Renderer,
    port: PortRef,
    frame: &Arc<Frame>,
    wrap: TextureWrap,
    filter: TextureFilter,
) {
    renderer.draw(&FrameJob {
        sources: vec![SourceJob {
            port,
            frame: Arc::clone(frame),
            wrap,
            filter,
        }],
        ..FrameJob::default()
    });
    drain(gpu);
}

/// The same, sampled by the rule.
fn upload_only(gpu: &Gpu, renderer: &mut Renderer, port: PortRef, frame: &Arc<Frame>) {
    publish(
        gpu,
        renderer,
        port,
        frame,
        TextureWrap::Mirror,
        TextureFilter::Linear,
    );
}

/// The test's own stage: each pixel of the target is one texel of the source loaded whole
/// (`mode` 0), or the source sampled through `smp` at `u = base + x / width * scale` on its
/// middle row (`mode` 1). Whatever the source's format, the target is `Rgba8Unorm`, so a
/// `Bgra8Unorm` texture reads back in RGBA order.
const READ: &str = "
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct Read { base: f32, scale: f32, width: f32, mode: u32 }
@group(0) @binding(2) var<uniform> r: Read;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    if r.mode == 0u {
        return textureLoad(src, vec2i(frag_coord.xy), 0);
    }
    return textureSampleLevel(src, smp, vec2f(r.base + frag_coord.x / r.width * r.scale, 0.5), 0.0);
}
";

/// Draw `texture` into an `Rgba8Unorm` target of `size` through [`READ`] and read it back,
/// rows as they lie.
fn read_through(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    size: (u32, u32),
    (base, scale, mode): (f32, f32, u32),
) -> Vec<u8> {
    let device = gpu.device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("read"),
        source: wgpu::ShaderSource::Wgsl(READ.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("read"),
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
    let pipeline = shared::fullscreen(
        device,
        &module,
        &layout,
        wgpu::TextureFormat::Rgba8Unorm,
        "read",
    );
    let mut block = [0u8; 16];
    block[..4].copy_from_slice(&base.to_le_bytes());
    block[4..8].copy_from_slice(&scale.to_le_bytes());
    block[8..12].copy_from_slice(&(size.0 as f32).to_le_bytes());
    block[12..].copy_from_slice(&mode.to_le_bytes());
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue().write_buffer(&uniforms, 0, &block);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("read"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
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
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("read target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("read"),
    });
    {
        let mut pass = shared::begin(
            &mut encoder,
            &target_view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            "read",
        );
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.submit([encoder.finish()]);
    bytes_of(gpu, &target)
}

/// A texture's texels in RGBA order, rows as they lie: for an upload, top row first.
fn texels_of(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let sampler = shared::sampler(
        gpu.device(),
        supersilvia::compile::wgsl::Sampler::MirrorNearest,
    );
    read_through(
        gpu,
        texture,
        &sampler,
        (texture.width(), texture.height()),
        (0.0, 0.0, 0),
    )
}

/// A source node's texture, in RGBA order.
fn uploaded(gpu: &Gpu, renderer: &Renderer, node: NodeId) -> Vec<u8> {
    texels_of(gpu, &renderer.texture_of(node).expect("uploaded"))
}

/// Planes held in vectors, counting how often any is asked for: what a mapped GStreamer
/// buffer is to the renderer.
struct Held {
    planes: Vec<Vec<u8>>,
    asked: AtomicUsize,
}

impl Planes for Held {
    fn plane(&self, i: usize) -> &[u8] {
        self.asked.fetch_add(1, Ordering::Relaxed);
        &self.planes[i]
    }
}

/// A frame in `layout` whose planes are `planes`, rows `strides` apart.
fn mapped(
    width: u32,
    height: u32,
    layout: Layout,
    strides: [u32; 3],
    yuv: Yuv,
    planes: Vec<Vec<u8>>,
) -> (Arc<Frame>, Arc<Held>) {
    let held = Arc::new(Held {
        planes,
        asked: AtomicUsize::new(0),
    });
    let frame = Frame {
        width,
        height,
        pixels: Pixels::Mapped(Mapped {
            layout,
            strides,
            yuv,
            data: Arc::clone(&held) as Arc<dyn Planes>,
        }),
    };
    (Arc::new(frame), held)
}

/// One plane of `width` by `height` texels of `texel` bytes each, rows `stride` apart, each
/// texel `at(x, y)` and every padding byte 0xab, so a stride taken wrong reads garbage.
fn plane(
    width: u32,
    height: u32,
    texel: usize,
    stride: u32,
    at: impl Fn(u32, u32) -> Vec<u8>,
) -> Vec<u8> {
    let mut out = vec![0xab; (stride * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = (y * stride) as usize + x as usize * texel;
            out[i..i + texel].copy_from_slice(&at(x, y));
        }
    }
    out
}

/// A solid I420 frame of full-range BT.601 `y`, `u`, `v`, rows padded.
fn i420(width: u32, height: u32, yuv: [u8; 3]) -> Arc<Frame> {
    i420_held(width, height, yuv).0
}

/// The same, and the planes behind it.
fn i420_held(width: u32, height: u32, [y, u, v]: [u8; 3]) -> (Arc<Frame>, Arc<Held>) {
    let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
    let (ys, cs) = (width.next_multiple_of(8) + 8, cw.next_multiple_of(8) + 8);
    let full = Yuv {
        matrix: YuvMatrix::Bt601,
        full_range: true,
    };
    mapped(
        width,
        height,
        Layout::I420,
        [ys, cs, cs],
        full,
        vec![
            plane(width, height, 1, ys, |_, _| vec![y]),
            plane(cw, ch, 1, cs, |_, _| vec![u]),
            plane(cw, ch, 1, cs, |_, _| vec![v]),
        ],
    )
}

/// Full-range BT.601's red, (254, 0, 0) to within a step.
const YUV_RED: [u8; 3] = [76, 85, 255];

fn close(got: [u8; 3], want: [u8; 3], by: u8) -> bool {
    got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= by)
}

/// An Output drawing, everywhere, the middle of the frame published on `port`.
fn viewer_of(
    port: PortRef,
) -> (
    Arc<Shader>,
    Vec<(Arc<str>, supersilvia::render::UniformValue)>,
) {
    let shader = module(
        &[],
        &[("u_src", port.node)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return textureSampleLevel(u_src, sampler_mirror_nearest, vec2f(0.5), 0.0);
}
",
    );
    let uniforms = vec![(
        Arc::<str>::from("u_src"),
        supersilvia::render::UniformValue::NodeTexture(port),
    )];
    (shader, uniforms)
}

/// The middle pixel of an Output's latest frame, RGB.
fn middle(gpu: &Gpu, renderer: &Renderer, out: NodeId) -> [u8; 3] {
    let texture = renderer.texture_of(out).expect("drawn");
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    let p = rgba_of(gpu, &texture)[(h / 2) * w + w / 2];
    [p[0], p[1], p[2]]
}

// ---------------------------------------------------------------------- source textures

/// A frame a CPU node publishes is uploaded once and sampled by a consumer's shader. This is
/// the whole camera path minus the camera: a camera cabled into an Output, compiled by
/// `compile::wgsl::build`, a solid green frame in, green pixels out.
#[test]
fn a_published_frame_is_uploaded_and_sampled() {
    let (g, cam, out) = gpu::one_node("camera");
    let (shader, mut values) = gpu::compiled(&g, out);
    let textures: Vec<Arc<str>> = shader
        .uniforms
        .iter()
        .filter(|(_, p)| matches!(p, supersilvia::compile::UniformProvider::NodeTexture { .. }))
        .map(|(n, _)| Arc::clone(n))
        .collect();
    assert_eq!(textures.len(), 1, "one texture uniform");
    values.extend(textures.into_iter().map(|name| {
        (
            name,
            supersilvia::render::UniformValue::NodeTexture(PortRef::new(cam, "frame")),
        )
    }));
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(out, (64, 64), &shader, send, mode, values.clone())];
    link_all(&mut renderer, &outputs);
    let frame = Arc::new(Frame::solid(16, 16, [0, 255, 0, 255]));
    renderer.draw(&FrameJob {
        sources: vec![SourceJob::new(PortRef::new(cam, "frame"), frame)],
        ..gpu::tick(0.0, outputs(false, DRAW))
    });
    drain(&gpu);
    assert_eq!(renderer.errors.get(&out), None);
    assert_eq!(middle(&gpu, &renderer, out), [0, 255, 0], "the frame");
}

/// Two texture outputs of **one** node reach the GPU as two textures: keyed by port, never
/// by node, or both samplers would read whichever was published last.
#[test]
fn two_texture_outputs_of_one_node_are_two_textures_on_the_gpu() {
    let (source, out) = (NodeId(1), NodeId(2));
    let (left, right) = (PortRef::new(source, "left"), PortRef::new(source, "right"));
    let shader = module(
        &[],
        &[("u_left", source), ("u_right", source)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let l = textureSampleLevel(u_left, sampler_mirror_linear, vec2f(0.5), 0.0);
    let r = textureSampleLevel(u_right, sampler_mirror_linear, vec2f(0.5), 0.0);
    return select(r, l, uv.x < 0.5);
}
",
    );
    let uniforms = vec![
        (
            Arc::<str>::from("u_left"),
            supersilvia::render::UniformValue::NodeTexture(left),
        ),
        (
            Arc::<str>::from("u_right"),
            supersilvia::render::UniformValue::NodeTexture(right),
        ),
    ];
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs =
        |send: bool, mode| vec![job(out, (64, 64), &shader, send, mode, uniforms.clone())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&FrameJob {
        sources: vec![
            SourceJob::new(left, Arc::new(Frame::solid(4, 4, [255, 0, 0, 255]))),
            SourceJob::new(right, Arc::new(Frame::solid(4, 4, [0, 255, 0, 255]))),
        ],
        ..gpu::tick(0.0, outputs(false, DRAW))
    });
    drain(&gpu);
    let px = rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    let at = |x: usize| {
        let p = px[32 * 64 + x];
        [p[0], p[1], p[2]]
    };
    assert_eq!(at(16), [255, 0, 0], "the left port is the left texture");
    assert_eq!(at(48), [0, 255, 0], "and the right port is a different one");
    let published = renderer.publish();
    assert!(published.sources.contains_key(&left) && published.sources.contains_key(&right));
    assert_ne!(
        published.sources[&left].texture, published.sources[&right].texture,
        "one texture per port"
    );
}

/// A published source is the frame's size, top row first (`flip`), and carries the sampler
/// its output declared; a node that leaves takes its texture with it.
#[test]
fn a_source_is_published_flipped_with_its_sampler_and_dropped_with_its_node() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(4), "cells");
    let frame = Arc::new(Frame::solid(6, 3, [9, 9, 9, 255]));
    publish(
        &gpu,
        &mut renderer,
        port,
        &frame,
        TextureWrap::Repeat,
        TextureFilter::Nearest,
    );
    let published = renderer.publish();
    let picture = published.sources.get(&port).expect("published");
    assert_eq!((picture.width, picture.height), (6, 3));
    assert!(
        picture.flip,
        "an upload's first row is the top of the picture"
    );
    assert_eq!(
        picture.sampler,
        supersilvia::compile::wgsl::Sampler::RepeatNearest
    );
    assert_eq!(
        published
            .picture(port.node, Some("cells"))
            .map(|p| p.texture),
        Some(picture.texture.clone())
    );

    renderer.draw(&FrameJob::default());
    assert!(
        renderer.texture_of(port.node).is_none(),
        "gone with its node"
    );
    assert!(renderer.publish().sources.is_empty());
}

// ------------------------------------------------------------------- how a texture wraps

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Four texels, each a pure colour, left to right: the 4x1 asymmetric picture every wrap test
/// reads back out of the sampler.
fn four_texels() -> Arc<Frame> {
    Arc::new(Frame {
        width: 4,
        height: 1,
        pixels: Pixels::Bytes([RED, GREEN, BLUE, WHITE].concat()),
    })
}

/// The source on `port` read through the sampler its picture names, `count` columns with `u`
/// running from `base` to `base + scale`.
fn sampled(
    gpu: &Gpu,
    renderer: &mut Renderer,
    port: PortRef,
    base: f32,
    scale: f32,
    count: u32,
) -> Vec<u8> {
    let published = renderer.publish();
    let picture = published.sources.get(&port).expect("published");
    let sampler = shared::sampler(gpu.device(), picture.sampler);
    read_through(
        gpu,
        picture.texture.texture(),
        &sampler,
        (count, 1),
        (base, scale, 1),
    )
}

/// A `u` of 0 to 2 in eight columns, each on a texel centre, where the filter has nothing to
/// blend and the wrap alone decides.
fn sampled_span(gpu: &Gpu, renderer: &mut Renderer, port: PortRef) -> Vec<u8> {
    sampled(gpu, renderer, port, 0.0, 2.0, 8)
}

fn assert_columns(sampled: &[u8], want: [[u8; 4]; 8], what: &str) {
    for (column, want) in want.into_iter().enumerate() {
        assert_eq!(
            sampled[column * 4..column * 4 + 4],
            want,
            "column {column}: {what}"
        );
    }
}

/// A texture that declared nothing is read mirrored outside `[0,1]`, the rule.
#[test]
fn a_sample_outside_0_1_reads_the_mirrored_texel() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    upload_only(&gpu, &mut renderer, port, &four_texels());
    assert_columns(
        &sampled_span(&gpu, &mut renderer, port),
        [RED, GREEN, BLUE, WHITE, WHITE, BLUE, GREEN, RED],
        "a tiled or clamped wrap, not a mirrored one",
    );
}

/// A texture output that declares `Repeat` tiles instead.
#[test]
fn a_texture_declared_repeat_tiles_outside_0_1() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "cells");
    publish(
        &gpu,
        &mut renderer,
        port,
        &four_texels(),
        TextureWrap::Repeat,
        TextureFilter::Nearest,
    );
    assert_columns(
        &sampled_span(&gpu, &mut renderer, port),
        [RED, GREEN, BLUE, WHITE, RED, GREEN, BLUE, WHITE],
        "a mirrored wrap, not a tiled one",
    );
}

/// **Zero flash**: a change of declaration picks another sampler and reallocates nothing. The
/// very same `Arc`, skipped by pointer, so nothing is uploaded; the texture is the same one
/// and so are its pixels.
#[test]
fn a_change_of_wrap_picks_another_sampler_and_reallocates_nothing() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    let frame = four_texels();
    upload_only(&gpu, &mut renderer, port, &frame);
    let before = renderer.texture_of(port.node).expect("uploaded");
    assert_columns(
        &sampled_span(&gpu, &mut renderer, port),
        [RED, GREEN, BLUE, WHITE, WHITE, BLUE, GREEN, RED],
        "the rule, before the declaration changed",
    );
    publish(
        &gpu,
        &mut renderer,
        port,
        &frame,
        TextureWrap::Repeat,
        TextureFilter::Nearest,
    );
    assert_eq!(
        renderer.texture_of(port.node).expect("still uploaded"),
        before,
        "the texture was reallocated under the change"
    );
    assert_columns(
        &sampled_span(&gpu, &mut renderer, port),
        [RED, GREEN, BLUE, WHITE, RED, GREEN, BLUE, WHITE],
        "the new declaration never reached the viewer's sampler",
    );
}

/// Seven tenths of the way from the first texel to the second — a `u` of 0.3 in a four-wide
/// texture — `Nearest` reads the second texel whole where `Linear` reads three parts of the
/// first to seven of the second.
#[test]
fn a_texture_declared_nearest_reads_one_texel_rather_than_a_blend() {
    const BETWEEN: f32 = 0.3;
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "cells");
    let frame = four_texels();
    publish(
        &gpu,
        &mut renderer,
        port,
        &frame,
        TextureWrap::Repeat,
        TextureFilter::Nearest,
    );
    let nearest = sampled(&gpu, &mut renderer, port, BETWEEN, 0.0, 1);
    assert_eq!(nearest[..4], GREEN, "a blend rather than the nearer texel");
    publish(
        &gpu,
        &mut renderer,
        port,
        &frame,
        TextureWrap::Repeat,
        TextureFilter::Linear,
    );
    let linear = sampled(&gpu, &mut renderer, port, BETWEEN, 0.0, 1);
    assert!(
        (51..=102).contains(&linear[0]) && (153..=204).contains(&linear[1]),
        "{linear:?} is not the blend linear filtering owes between two texels"
    );
    assert_eq!(linear[2], 0, "no third texel is in reach of this u");
}

// ----------------------------------------------------------- frames in their own layout

/// Every RGB layout goes up as it lies — both byte orders, with a real alpha or a padding
/// byte, rows padded past the width — and comes out RGBA with the rows unsheared. The padding
/// byte holds garbage and reads as opaque; a real alpha is kept.
#[test]
fn every_rgb_layout_uploads_as_rgba_with_its_own_stride() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    let (w, h, stride) = (5u32, 3u32, 5 * 4 + 12);
    // A different colour in every texel, so a sheared row or a swapped order is seen.
    let want = |x: u32, y: u32| {
        [
            (40 * x + 7) as u8,
            (60 * y + 11) as u8,
            (x * y * 13 + 3) as u8,
        ]
    };
    for (layout, alpha) in [
        (Layout::Rgba, 128u8),
        (Layout::Rgbx, 17),
        (Layout::Bgra, 128),
        (Layout::Bgrx, 17),
    ] {
        let bgr = matches!(layout, Layout::Bgra | Layout::Bgrx);
        let bytes = plane(w, h, 4, stride, |x, y| {
            let [r, g, b] = want(x, y);
            if bgr {
                vec![b, g, r, alpha]
            } else {
                vec![r, g, b, alpha]
            }
        });
        let (frame, _) = mapped(w, h, layout, [stride, 0, 0], Yuv::default(), vec![bytes]);
        upload_only(&gpu, &mut renderer, port, &frame);
        let got = uploaded(&gpu, &renderer, port.node);
        let opaque = matches!(layout, Layout::Rgbx | Layout::Bgrx);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                assert_eq!(got[i..i + 3], want(x, y), "{layout:?} at ({x}, {y})");
                let a = if opaque { 255 } else { alpha };
                assert_eq!(got[i + 3], a, "{layout:?} alpha at ({x}, {y})");
            }
        }
    }
}

/// One frame of the test pattern in `format`, at a size whose rows do not pack: 190 across
/// makes I420's luma stride 192 and its chroma 96, and NV12's interleaved chroma 192.
fn pattern_in(format: &str) -> Arc<Frame> {
    use supersilvia::video::{Camera, Source};
    let head =
        format!("videotestsrc is-live=true ! video/x-raw,format={format},width=190,height=96");
    let mut camera = Camera::open(&Source::Described(head), None).expect("a pipeline");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A source's frame goes up in the layout the source made it in — BGR, planar, semi-planar
/// and packed YUV, with padded rows — and comes out the same picture the pattern paints in
/// RGBA. Nothing was converted on the CPU: each frame is the pipeline's own buffer, mapped.
///
/// Read at the middle of each of the seven bars on three rows spread down them. A stride
/// taken wrong shears the rows, a plane or a chroma order taken wrong gives the wrong colour.
/// One source is fed every format in turn, so each frame also proves a texture that held
/// another layout is reused or replaced cleanly.
#[test]
fn a_frame_in_its_own_layout_uploads_as_the_picture_rgba_would_be() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");

    let reference = pattern_in("RGBA");
    let want = reference.bytes().expect("RGBA arrives packed").to_vec();
    let (w, rows) = (190usize, [8usize, 30, 60]);
    let bars: Vec<usize> = (0..7).map(|b| (2 * b + 1) * w / 14).collect();
    let pixel = |buf: &[u8], x: usize, y: usize| {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2]]
    };

    for (format, layout, padded) in [
        ("BGRx", Layout::Bgrx, false),
        ("I420", Layout::I420, true),
        ("NV12", Layout::Nv12, true),
        ("YUY2", Layout::Yuy2, false),
        ("UYVY", Layout::Uyvy, false),
        ("Y42B", Layout::Y42b, true),
        ("YV12", Layout::I420, true),
        ("BGRA", Layout::Bgra, false),
        ("RGBA", Layout::Rgba, false),
    ] {
        let frame = pattern_in(format);
        let Pixels::Mapped(m) = &frame.pixels else {
            panic!(
                "{format}: the buffer is held, not copied: {:?}",
                frame.pixels
            );
        };
        assert_eq!(m.layout, layout, "{format}");
        let packed = |i: usize| m.layout.plane_size(i, 190, 96).0 * m.layout.texel_bytes(i);
        let is_padded = (0..m.layout.planes()).any(|i| m.strides[i] != packed(i));
        assert_eq!(is_padded, padded, "{format}: strides {:?}", m.strides);
        upload_only(&gpu, &mut renderer, port, &frame);
        let got = uploaded(&gpu, &renderer, port.node);
        for y in rows {
            for &x in &bars {
                let (g, e) = (pixel(&got, x, y), pixel(&want, x, y));
                assert!(
                    close(g, e, 4),
                    "{format} at ({x}, {y}): {g:?}, and RGBA paints {e:?}"
                );
            }
        }
        assert!(
            got.chunks(4).all(|p| p[3] == 255),
            "{format}: every pixel is opaque"
        );
    }
}

/// A frame whose size changes is drawn at the new size, in the tick that fills it, for a YUV
/// frame as for an RGB one.
#[test]
fn a_yuv_source_that_changes_size_is_drawn_at_the_new_size() {
    use supersilvia::video::{Camera, Source};
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    upload_only(&gpu, &mut renderer, port, &pattern_in("I420"));

    let head =
        "videotestsrc is-live=true pattern=white ! video/x-raw,format=I420,width=64,height=32";
    let mut camera = Camera::open(&Source::Described(head.into()), None).expect("a pipeline");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let white = loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    upload_only(&gpu, &mut renderer, port, &white);
    let got = uploaded(&gpu, &renderer, port.node);
    assert_eq!(got.len(), 64 * 32 * 4);
    assert!(
        got.chunks(4).all(|p| p[..3].iter().all(|c| *c >= 250)),
        "white all over at 64x32"
    );
}

/// **An `Arc` already uploaded is skipped, by pointer**: its buffer is not asked for again,
/// and the renderer keeps no strong hold on it, so it goes back to its source as soon as the
/// copy is queued.
#[test]
fn a_frame_already_uploaded_is_skipped_and_its_buffer_goes_back() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    let (frame, held) = i420_held(8, 6, YUV_RED);
    upload_only(&gpu, &mut renderer, port, &frame);
    let first = held.asked.load(Ordering::Relaxed);
    assert!(first > 0, "the planes were read");
    for _ in 0..3 {
        upload_only(&gpu, &mut renderer, port, &frame);
    }
    assert_eq!(
        held.asked.load(Ordering::Relaxed),
        first,
        "the same Arc was read again"
    );
    assert_eq!(
        Arc::strong_count(&frame),
        1,
        "the renderer holds only a Weak"
    );
    let px = uploaded(&gpu, &renderer, port.node);
    assert!(
        close([px[0], px[1], px[2]], [254, 0, 0], 3),
        "{:?}",
        &px[..4]
    );
}

// -------------------------------------------------------------------------- DMA-BUF import

/// A descriptor nothing imports — no file behind it — with the flag its source reads.
fn unimportable(width: u32, height: u32) -> (Arc<Frame>, Arc<AtomicBool>) {
    let refused = Arc::new(AtomicBool::new(false));
    let frame = Frame {
        width,
        height,
        pixels: Pixels::DmaBuf(DmaBuf {
            fd: -1,
            fourcc: u32::from_le_bytes(*b"AB24"),
            modifier: 0,
            stride: width * 4,
            offset: 0,
            keep: Arc::new(()),
            refused: Some(Arc::clone(&refused)),
        }),
    };
    (Arc::new(frame), refused)
}

/// **A refused import falls back to bytes of the same size**, which are shown; and a refused
/// import after bytes leaves the bytes up.
#[test]
fn bytes_after_a_refused_import_of_the_same_size_are_shown() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(3), "frame");

    let (refused_frame, refused) = unimportable(16, 16);
    upload_only(&gpu, &mut renderer, port, &refused_frame);
    assert!(
        refused.load(Ordering::Relaxed),
        "the source is told to fall back"
    );

    let green = Arc::new(Frame::solid(16, 16, [0, 255, 0, 255]));
    upload_only(&gpu, &mut renderer, port, &green);
    let px = uploaded(&gpu, &renderer, port.node);
    assert_eq!(px[..4], [0, 255, 0, 255], "the bytes are shown");

    let before = renderer.texture_of(port.node).expect("uploaded");
    let (again, refused) = unimportable(16, 16);
    upload_only(&gpu, &mut renderer, port, &again);
    assert!(refused.load(Ordering::Relaxed));
    assert_eq!(renderer.texture_of(port.node).expect("kept"), before);
    assert_eq!(
        uploaded(&gpu, &renderer, port.node)[..4],
        [0, 255, 0, 255],
        "a refused import leaves the last frame up"
    );
}

/// **Bytes never go into an import's memory, and an import is held past the draws that
/// sampled it.** A good import, then a refused one, then bytes, then the first import again:
/// the refused one leaves the good one up, the bytes land in a texture of the source's own,
/// and the first frame imported again is its own picture rather than the bytes written into
/// it. A frame replaced goes back to its producer only once the GPU has finished the
/// submission after which nothing samples it.
///
/// Skipped where this machine exports no DMA-BUF or the wgpu import refuses the test pattern's.
#[test]
fn an_import_between_uploads_is_neither_written_nor_let_go_early() {
    use supersilvia::video::{Camera, Delivery, Source};
    if supersilvia::platform::video::dmabuf_format().is_none() {
        eprintln!("no DMA-BUF export here; skipping");
        return;
    }
    let Ok(mut camera) = Camera::open_with(&Source::Test, Some((64, 64)), Delivery::DmaBuf) else {
        eprintln!("no DMA-BUF pipeline here; skipping");
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let imported = loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let Pixels::DmaBuf(buf) = &imported.pixels else {
        panic!("delivered as a descriptor")
    };
    let refused = buf.refused.clone();

    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(5), "frame");
    upload_only(&gpu, &mut renderer, port, &imported);
    if refused.is_some_and(|r| r.load(Ordering::Relaxed))
        || renderer.texture_of(port.node).is_none()
    {
        eprintln!("the wgpu import refused the test pattern's DMA-BUF; skipping");
        return;
    }
    // The test pattern's top-left bar is white, and its first row is the texture's.
    let corner = |renderer: &Renderer| {
        let px = uploaded(&gpu, renderer, port.node);
        let i = (2 * 64 + 2) * 4;
        [px[i], px[i + 1], px[i + 2]]
    };
    let white = |px: [u8; 3]| px.iter().all(|c| *c > 240);
    assert!(white(corner(&renderer)), "the import is sampled");
    let held = Arc::strong_count(&imported);

    let (refused_frame, _) = unimportable(64, 64);
    upload_only(&gpu, &mut renderer, port, &refused_frame);
    assert!(
        white(corner(&renderer)),
        "a refused import leaves the last up"
    );

    let red = Arc::new(Frame::solid(64, 64, [255, 0, 0, 255]));
    // Not finished: nothing has asked whether the submission after the import's last sampler
    // is done.
    renderer.draw(&FrameJob {
        sources: vec![SourceJob::new(port, Arc::clone(&red))],
        ..FrameJob::default()
    });
    assert_eq!(
        Arc::strong_count(&imported),
        held,
        "replaced, and held until the GPU is done with it"
    );
    drain(&gpu);
    assert_eq!(corner(&renderer), [255, 0, 0], "the bytes are shown");
    upload_only(&gpu, &mut renderer, port, &red);
    assert_eq!(
        Arc::strong_count(&imported),
        held - 1,
        "and let go once it is"
    );

    upload_only(&gpu, &mut renderer, port, &imported);
    let px = corner(&renderer);
    assert!(
        white(px),
        "the import's own memory was never written: {px:?}"
    );
}

/// **An import whose fourth byte is padding is drawn opaque, into a texture of the source's
/// own.** wgpu has no alpha swizzle, so the test pattern's frame relabelled as the `x` twin of
/// its fourcc — the same memory, its fourth byte now padding — goes through the conversion pass
/// rather than being sampled where it lies: what is published is a texture the pass drew,
/// holding the import's colours with alpha one, and the frame stays claimed by what was
/// published until that is let go.
///
/// Skipped where this machine exports no DMA-BUF or the wgpu import refuses the relabelled one.
#[test]
fn an_import_with_a_padding_byte_is_drawn_opaque_into_a_texture_of_its_own() {
    use supersilvia::render::dmabuf::{AB24, AR24, XB24, XR24};
    use supersilvia::video::{Camera, Delivery, Source};
    if supersilvia::platform::video::dmabuf_format().is_none() {
        eprintln!("no DMA-BUF export here; skipping");
        return;
    }
    let Ok(mut camera) = Camera::open_with(&Source::Test, Some((64, 64)), Delivery::DmaBuf) else {
        eprintln!("no DMA-BUF pipeline here; skipping");
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let imported = loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let Pixels::DmaBuf(buf) = &imported.pixels else {
        panic!("delivered as a descriptor")
    };
    let padded = match buf.fourcc {
        AB24 => XB24,
        AR24 => XR24,
        other => {
            eprintln!("the test pattern is {other:#x}, which has no padded twin; skipping");
            return;
        }
    };
    let refused = Arc::new(AtomicBool::new(false));
    let relabelled = Arc::new(Frame {
        width: imported.width,
        height: imported.height,
        pixels: Pixels::DmaBuf(DmaBuf {
            fd: buf.fd,
            fourcc: padded,
            modifier: buf.modifier,
            stride: buf.stride,
            offset: buf.offset,
            keep: Arc::clone(&buf.keep),
            refused: Some(Arc::clone(&refused)),
        }),
    });

    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(5), "frame");
    upload_only(&gpu, &mut renderer, port, &imported);
    let Some(direct) = renderer.texture_of(port.node) else {
        eprintln!("the wgpu import refused the test pattern's DMA-BUF; skipping");
        return;
    };
    assert!(
        !direct
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "a frame with an alpha byte is sampled where it lies"
    );
    let colours = uploaded(&gpu, &renderer, port.node);

    upload_only(&gpu, &mut renderer, port, &relabelled);
    if refused.load(Ordering::Relaxed) {
        eprintln!("the wgpu import refused the padded fourcc; skipping");
        return;
    }
    let drawn = renderer.texture_of(port.node).expect("uploaded");
    assert!(
        drawn
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "a frame with a padding byte is drawn into a texture of the source's own"
    );
    assert_eq!(drawn.format(), wgpu::TextureFormat::Rgba8Unorm);
    let opaque = uploaded(&gpu, &renderer, port.node);
    for (i, (a, b)) in colours
        .as_chunks::<4>()
        .0
        .iter()
        .zip(opaque.as_chunks::<4>().0)
        .enumerate()
    {
        assert_eq!(a[..3], b[..3], "texel {i}: the import's own colour");
        assert_eq!(b[3], 255, "texel {i}: alpha one");
    }

    let published = renderer.publish();
    assert_eq!(
        published.imports.len(),
        1,
        "what is published holds the frame its texture was drawn from"
    );
    let red = Arc::new(Frame::solid(64, 64, [255, 0, 0, 255]));
    upload_only(&gpu, &mut renderer, port, &red);
    upload_only(&gpu, &mut renderer, port, &red);
    assert!(
        Arc::strong_count(&relabelled) > 1,
        "a frame a Published still claims does not go back"
    );
    drop(published);
    upload_only(&gpu, &mut renderer, port, &red);
    upload_only(&gpu, &mut renderer, port, &red);
    assert_eq!(
        Arc::strong_count(&relabelled),
        1,
        "and goes back once nothing claims it"
    );
}

// ---------------------------------------------------------------------------- zero flash

/// **Zero flash for sources.** An Output sampling a source on every tick, while the source
/// arrives, changes size, changes layout — packed bytes, a padded BGR frame with a padding
/// byte, planar YUV, back to bytes — and comes back: on every tick the Output draws the frame
/// published that tick, never black and never the one before, because a new or resized
/// texture is made and written in the prelude that precedes the Output's submission.
#[test]
fn a_source_is_drawn_on_the_tick_it_arrives_whatever_its_size_or_layout() {
    let (src, out) = (NodeId(1), NodeId(2));
    let port = PortRef::new(src, "frame");
    let (shader, uniforms) = viewer_of(port);
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs =
        |send: bool, mode| vec![job(out, (32, 32), &shader, send, mode, uniforms.clone())];
    link_all(&mut renderer, &outputs);

    let bgrx = |w: u32, h: u32, [r, g, b]: [u8; 3]| {
        let stride = w * 4 + 16;
        mapped(
            w,
            h,
            Layout::Bgrx,
            [stride, 0, 0],
            Yuv::default(),
            vec![plane(w, h, 4, stride, |_, _| vec![b, g, r, 0])],
        )
        .0
    };
    let frames: Vec<(Arc<Frame>, [u8; 3])> = vec![
        (
            Arc::new(Frame::solid(16, 16, [200, 10, 10, 255])),
            [200, 10, 10],
        ),
        (
            Arc::new(Frame::solid(16, 16, [10, 200, 10, 255])),
            [10, 200, 10],
        ),
        (
            Arc::new(Frame::solid(40, 20, [10, 10, 200, 255])),
            [10, 10, 200],
        ),
        (bgrx(40, 20, [220, 120, 20]), [220, 120, 20]),
        (bgrx(12, 30, [20, 120, 220]), [20, 120, 220]),
        (i420(18, 10, YUV_RED), [254, 0, 0]),
        (i420(64, 48, YUV_RED), [254, 0, 0]),
        (
            Arc::new(Frame::solid(8, 8, [90, 90, 90, 255])),
            [90, 90, 90],
        ),
        (i420(8, 8, YUV_RED), [254, 0, 0]),
    ];
    for (tick, (frame, want)) in frames.iter().enumerate() {
        renderer.draw(&FrameJob {
            sources: vec![SourceJob::new(port, Arc::clone(frame))],
            ..gpu::tick(tick as f32, outputs(false, DRAW))
        });
        drain(&gpu);
        let got = middle(&gpu, &renderer, out);
        assert!(
            close(got, *want, 3),
            "tick {tick}: drew {got:?}, published {want:?}"
        );
    }
}

/// **A frame of the same size and layout is written in place**: no texture is made per frame.
/// A frame that does not fit — too few bytes, a buffer shorter than its stride claims, a
/// stride that is not whole texels — changes nothing: the texture and the frame in it stay.
#[test]
fn a_frame_that_does_not_fit_leaves_the_last_one_up() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    upload_only(
        &gpu,
        &mut renderer,
        port,
        &Arc::new(Frame::solid(8, 8, [1, 2, 3, 255])),
    );
    let texture = renderer.texture_of(port.node).expect("uploaded");
    upload_only(
        &gpu,
        &mut renderer,
        port,
        &Arc::new(Frame::solid(8, 8, [30, 60, 90, 255])),
    );
    assert_eq!(
        renderer.texture_of(port.node).expect("uploaded"),
        texture,
        "written in place"
    );

    let short = Arc::new(Frame {
        width: 8,
        height: 8,
        pixels: Pixels::Bytes(vec![255; 8 * 8 * 4 - 4]),
    });
    let (truncated, _) = mapped(
        8,
        8,
        Layout::Rgba,
        [32, 0, 0],
        Yuv::default(),
        vec![vec![255; 32 * 7]],
    );
    let (sheared, _) = mapped(
        8,
        8,
        Layout::Bgra,
        [34, 0, 0],
        Yuv::default(),
        vec![vec![255; 34 * 8]],
    );
    let (thin_chroma, _) = mapped(
        8,
        8,
        Layout::I420,
        [8, 4, 4],
        Yuv::default(),
        vec![vec![255; 64], vec![255; 16], vec![255; 8]],
    );
    for bad in [short, truncated, sheared, thin_chroma] {
        upload_only(&gpu, &mut renderer, port, &bad);
        assert_eq!(
            renderer.texture_of(port.node).expect("kept"),
            texture,
            "{:?}",
            bad.pixels
        );
        assert_eq!(
            uploaded(&gpu, &renderer, port.node)[..4],
            [30, 60, 90, 255],
            "{:?}",
            bad.pixels
        );
    }
}

/// **A viewer keeps the frame it was handed.** A `Published` names the source's texture; a
/// frame of another size replaces the texture rather than writing it, so the one the viewer
/// holds still holds its frame, whole, while the renderer's is the new one.
#[test]
fn a_viewer_holding_a_source_keeps_its_frame_across_a_resize() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let port = PortRef::new(NodeId(1), "frame");
    upload_only(
        &gpu,
        &mut renderer,
        port,
        &Arc::new(Frame::solid(8, 8, [10, 20, 30, 255])),
    );
    let held = renderer.publish();
    upload_only(&gpu, &mut renderer, port, &i420(20, 12, YUV_RED));
    let picture = held.sources.get(&port).expect("published");
    let kept = texels_of(&gpu, picture.texture.texture());
    assert_eq!(kept.len(), 8 * 8 * 4);
    assert!(
        kept.chunks(4).all(|p| p == [10, 20, 30, 255]),
        "the held frame changed under its viewer"
    );
    let now = uploaded(&gpu, &renderer, port.node);
    assert_eq!(now.len(), 20 * 12 * 4);
    assert!(close([now[0], now[1], now[2]], [254, 0, 0], 3));
    assert_ne!(
        renderer.publish().sources[&port].texture,
        picture.texture,
        "the new frame is a texture of its own"
    );
}
