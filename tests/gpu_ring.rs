// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's ring, zero flash and feedback — with the pixels the GL renderer was held to
//! where there are pixels, because an Output's texture keeps GL's rows (`proposals/wgpu.md`,
//! 1.15) — and the one queue's throttle, a GPU that cannot keep up,
//! what churn leaves on the device, and a viewer on another thread.
//!
//! What a saturated GPU is held to is what the bounds promise — no frame dropped, nothing
//! shown or read older than [`TICKS_IN_FLIGHT`] ticks — and never how often something had to
//! wait, which is a ratio of this GPU's speed to whatever else is running on it.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain, floats_of, job, link_all, module, reading, rgba_of, solid, tick};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::compile::wgsl::{self, Sampler};
use supersilvia::compile::{Shader, TAP_TEMPLATE, TAP_WORDS, TapKind};
use supersilvia::graph::NodeId;
use supersilvia::render::program::Program;
use supersilvia::render::readback::{Alpha, THUMBNAIL_BYTES};
use supersilvia::render::ring::RING;
use supersilvia::render::{
    FrameJob, Gpu, MixerJob, OutputJob, OutputMode, Published, QUEUED_AHEAD, Renderer,
    TICKS_IN_FLIGHT, UniformValue, shared, uniforms,
};

const DRAW: OutputMode = OutputMode::Draw;

/// The harness renders on the integrated GPU it asks for, not on the strongest GPU the app
/// would take: never a discrete GPU here, never a software adapter.
#[test]
fn the_harness_renders_on_the_igpu() {
    let gpu = gpu::gpu();
    let info = gpu.adapter().get_info();
    let described = supersilvia::render::adapter::describe(&info);
    assert_eq!(
        info.device_type,
        wgpu::DeviceType::IntegratedGpu,
        "{described}"
    );
    assert_ne!(
        info.vendor,
        supersilvia::render::adapter::NVIDIA,
        "{described}"
    );
}

/// **A real graph draws through the wgpu path**: a checkerboard cabled into an Output, compiled
/// by `compile::wgsl::build`, bound by `wgsl::bindings` and `uniform_layout` alone, drawn by the
/// renderer and read back. Every pixel is exactly one of the two control colours, and a
/// frequency of 8 over a square target splits them evenly, as the GL test asserts.
#[test]
fn a_checkerboard_drawn_through_the_renderer_is_pure_black_and_white() {
    let (g, _, out) = gpu::one_node("checkerboard");
    let (shader, values) = gpu::compiled(&g, out);
    assert_eq!(values.len(), 3, "frequency and two colours");
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(out, (64, 64), &shader, send, mode, values.clone())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);

    let pixels = rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    let (mut white, mut black) = (0, 0);
    for p in &pixels {
        match *p {
            [255, 255, 255, 255] => white += 1,
            [0, 0, 0, 255] => black += 1,
            other => panic!("pixel is neither control colour: {other:?}"),
        }
    }
    assert!(white > 0 && black > 0, "got {white} white, {black} black");
    assert_eq!(white, black, "an 8-square grid splits evenly");

    let published = renderer.publish();
    let picture = published
        .outputs
        .get(&out)
        .expect("published once finished");
    assert_eq!(
        picture.texture.texture(),
        &renderer.texture_of(out).expect("drawn"),
        "drained, the newest frame is the one shown"
    );
    assert_eq!((picture.width, picture.height), (64, 64));
    assert!(!picture.flip, "an Output's frame is not flipped");
}

/// A widescreen target is covered to its last row and column.
#[test]
fn a_widescreen_checkerboard_covers_the_whole_target() {
    let (g, _, out) = gpu::one_node("checkerboard");
    let (shader, values) = gpu::compiled(&g, out);
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs =
        |send: bool, mode| vec![job(out, (320, 180), &shader, send, mode, values.clone())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    let pixels = rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    assert_eq!(pixels.len(), 320 * 180);
    assert!(
        pixels.iter().all(|p| p[3] == 255),
        "every pixel was drawn, the far corner included"
    );
}

// ---------------------------------------------------------------------------- feedback

/// Feedback through an Output's own frame: a moving pattern with a zoomed, shifted and
/// brightened copy of last frame mixed in, reaching past the edge so the mirrored wrap takes
/// part. Every frame depends on every frame before it, through the linear filter.
fn self_feedback(node: NodeId) -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[("u_self", node)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let at = (uv - 0.5) * 0.93 + 0.5 + vec2f(0.013 * sin(u.u_time * 1.7), 0.021);
    let prev = textureSampleLevel(u_self, sampler_mirror_linear, at * 1.1 - 0.05, 0.0);
    let fresh = vec4f(fract(uv.x * 3.0 + u.u_time), fract(uv.y * 5.0 - u.u_time * 0.7),
                      0.5 + 0.5 * sin(u.u_time * 3.0 + uv.x * 10.0), 1.0);
    return mix(prev * 1.02, fresh, 0.15);
}
",
    )
}

/// The first of two Outputs feeding each other: it reads itself and the second.
fn cross_a(a: NodeId, b: NodeId) -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[("u_a", a), ("u_b", b)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let mine = textureSampleLevel(u_a, sampler_mirror_linear, (uv - 0.5) * 0.97 + 0.5 + vec2f(0.0, 0.011), 0.0);
    let theirs = textureSampleLevel(u_b, sampler_mirror_linear, uv.yx * 1.08 - 0.04, 0.0);
    let fresh = vec4f(0.5 + 0.5 * sin(uv.x * 9.0 + u.u_time * 2.0), fract(uv.y * 2.0 + u.u_time),
                      0.25, 1.0);
    return mix(0.6 * mine + 0.5 * theirs, fresh, 0.2);
}
",
    )
}

/// The second: it reads the first, mirrored, and nothing of its own.
fn cross_b(a: NodeId) -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[("u_a", a)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let theirs = textureSampleLevel(u_a, sampler_mirror_linear,
                                    vec2f(1.0 - uv.x, uv.y) * 1.05 + vec2f(0.01 * u.u_time, 0.0), 0.0);
    let fresh = vec4f(0.1, fract(uv.x * 4.0 - u.u_time * 0.5),
                      0.5 + 0.5 * cos(uv.y * 7.0 + u.u_time), 1.0);
    return mix(theirs, fresh, 0.25) * 1.01;
}
",
    )
}

/// An Output as a renderer without a ring would draw one: a temp target it draws into and a
/// published target the frame is copied into after, which is what every `frame` samples. The
/// reference the ring is held to, to the last bit — with the very pipeline the renderer makes.
struct Reference {
    program: Program,
    samplers: Vec<(Sampler, wgpu::Sampler)>,
    temp: wgpu::Texture,
    published: wgpu::Texture,
    size: (u32, u32),
}

impl Reference {
    fn new(gpu: &Gpu, shader: &supersilvia::compile::Shader, size: (u32, u32)) -> Self {
        let program = Program::create(gpu, shader, wgpu::TextureFormat::Rgba16Float)
            .expect("the reference links");
        let target = |label| {
            gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        Self {
            program,
            samplers: Sampler::ALL
                .iter()
                .map(|k| (*k, shared::sampler(gpu.device(), *k)))
                .collect(),
            temp: target("reference temp"),
            published: target("reference published"),
            size,
        }
    }

    /// One frame the old way: draw into temp, sampling `reads` as they are now, then copy
    /// temp into the published target.
    fn draw(&self, gpu: &Gpu, time: f32, reads: &[(&str, &wgpu::Texture)]) {
        self.draw_with(gpu, time, reads, &[]);
    }

    /// The same, with these values in the uniform block: one submission of its own.
    fn draw_with(
        &self,
        gpu: &Gpu,
        time: f32,
        reads: &[(&str, &wgpu::Texture)],
        values: &[(Arc<str>, UniformValue)],
    ) {
        use supersilvia::compile::wgsl::Resource;
        let block = uniforms::pack(&self.program.uniforms, self.size, time, values);
        let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("reference uniforms"),
            size: block.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue().write_buffer(&buffer, 0, &block);
        let views: Vec<(&str, wgpu::TextureView)> = reads
            .iter()
            .map(|(n, t)| (*n, t.create_view(&wgpu::TextureViewDescriptor::default())))
            .collect();
        let entries: Vec<wgpu::BindGroupEntry<'_>> = self
            .program
            .bindings
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match resource {
                    Resource::Uniforms => buffer.as_entire_binding(),
                    Resource::Sampler(kind) => wgpu::BindingResource::Sampler(
                        &self.samplers.iter().find(|(k, _)| k == kind).unwrap().1,
                    ),
                    Resource::Texture(name) => wgpu::BindingResource::TextureView(
                        &views.iter().find(|(n, _)| *n == &**name).unwrap().1,
                    ),
                    Resource::Taps => unreachable!("no taps here"),
                },
            })
            .collect();
        let group = gpu.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reference"),
            layout: self.program.layout(),
            entries: &entries,
        });
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let view = self
                .temp
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = shared::begin(
                &mut encoder,
                &view,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                "reference",
            );
            pass.set_pipeline(self.program.pipeline());
            pass.set_bind_group(0, &group, &[0]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_texture(
            self.temp.as_image_copy(),
            self.published.as_image_copy(),
            self.temp.size(),
        );
        gpu.submit([encoder.finish()]);
    }
}

/// **Feedback through an Output's own `frame` is a one-frame delay, to the bit.** The ring draws
/// straight into a free target and the feedback reads the latest one; a temp target copied into
/// the one `frame` samples is the same delay, and this holds the two equal over a run of ticks
/// in which each frame compounds every frame before it.
#[test]
fn self_feedback_through_the_ring_is_bit_identical_to_a_copy() {
    const SIZE: (u32, u32) = (64, 48);
    let gpu = gpu::gpu();
    let node = NodeId(7);
    let shader = self_feedback(node);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| {
        vec![job(
            node,
            SIZE,
            &shader,
            send,
            mode,
            reading(&[("u_self", node)]),
        )]
    };
    link_all(&mut renderer, &outputs);
    let reference = Reference::new(&gpu, &shader, SIZE);

    for n in 0..12 {
        let time = n as f32 * 0.37;
        renderer.draw(&tick(time, outputs(false, DRAW)));
        reference.draw(&gpu, time, &[("u_self", &reference.published)]);
        drain(&gpu);
        let drawn = renderer.texture_of(node).expect("drawn");
        assert!(
            bytes_of(&gpu, &drawn) == bytes_of(&gpu, &reference.published),
            "tick {n}: the ring's frame differs from the temp-and-copy frame"
        );
    }
    assert_eq!(renderer.dropped_frames(node), 0, "every tick was drawn");
    assert!(
        bytes_of(&gpu, &renderer.texture_of(node).unwrap())
            .iter()
            .any(|b| *b != 0),
        "and the picture is not simply black"
    );
}

/// **Two Outputs feeding each other are a one-frame delay, to the bit.** A reads itself and B,
/// B reads A, drawn A then B, each a submission of its own: A sees B's frame from the tick
/// before and B the frame A has just drawn.
#[test]
fn cross_feedback_through_the_ring_is_bit_identical_to_a_copy() {
    const SIZE: (u32, u32) = (48, 64);
    let gpu = gpu::gpu();
    let (a, b) = (NodeId(3), NodeId(5));
    let (shader_a, shader_b) = (cross_a(a, b), cross_b(a));
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| {
        vec![
            job(
                a,
                SIZE,
                &shader_a,
                send,
                mode,
                reading(&[("u_a", a), ("u_b", b)]),
            ),
            job(b, SIZE, &shader_b, send, mode, reading(&[("u_a", a)])),
        ]
    };
    link_all(&mut renderer, &outputs);
    let ref_a = Reference::new(&gpu, &shader_a, SIZE);
    let ref_b = Reference::new(&gpu, &shader_b, SIZE);

    for n in 0..12 {
        let time = n as f32 * 0.29;
        renderer.draw(&tick(time, outputs(false, DRAW)));
        ref_a.draw(
            &gpu,
            time,
            &[("u_a", &ref_a.published), ("u_b", &ref_b.published)],
        );
        ref_b.draw(&gpu, time, &[("u_a", &ref_a.published)]);
        drain(&gpu);
        for (node, reference, name) in [(a, &ref_a, "A"), (b, &ref_b, "B")] {
            let drawn = renderer.texture_of(node).expect("drawn");
            assert!(
                bytes_of(&gpu, &drawn) == bytes_of(&gpu, &reference.published),
                "tick {n}: {name}'s frame differs from the temp-and-copy frame"
            );
        }
    }
    assert_eq!(renderer.dropped_frames(a) + renderer.dropped_frames(b), 0);
}

/// A producer drawing its time as a colour, and a consumer copying the producer's frame.
fn producer() -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return vec4f(fract(u.u_time), fract(u.u_time * 0.5), 0.25, 1.0);
}
",
    )
}

fn consumer(from: NodeId) -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[("u_from", from)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return textureLoad(u_from, vec2i(frag_coord.xy), 0);
}
",
    )
}

/// **A consumer reads its producer's frame of the same tick.** Drawn after it in plan order,
/// in a later submission, it samples the frame the producer has just drawn, to the bit.
#[test]
fn a_consumer_reads_its_producers_frame_of_the_same_tick() {
    const SIZE: (u32, u32) = (16, 16);
    let gpu = gpu::gpu();
    let (p, c) = (NodeId(1), NodeId(2));
    let (shader_p, shader_c) = (producer(), consumer(p));
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| {
        vec![
            job(p, SIZE, &shader_p, send, mode, Vec::new()),
            job(c, SIZE, &shader_c, send, mode, reading(&[("u_from", p)])),
        ]
    };
    link_all(&mut renderer, &outputs);
    for n in 0..6 {
        renderer.draw(&tick(0.1 + n as f32 * 0.17, outputs(false, DRAW)));
        drain(&gpu);
        assert!(
            bytes_of(&gpu, &renderer.texture_of(p).unwrap())
                == bytes_of(&gpu, &renderer.texture_of(c).unwrap()),
            "tick {n}: the consumer read another frame than the producer's of this tick"
        );
    }
}

// ---------------------------------------------------------------------------- zero flash

/// The one published picture of `node`, once everything drawn has finished, as RGBA8.
fn shown(gpu: &Gpu, renderer: &mut Renderer, node: NodeId) -> Vec<[u8; 4]> {
    drain(gpu);
    let published = renderer.publish();
    let picture = published.outputs.get(&node).expect("a picture is shown");
    rgba_of(gpu, picture.texture.texture())
}

/// **Zero flash.** A structural edit's module is linking; the Output goes on drawing with the
/// program on screen, and what is shown is its picture, never black.
#[test]
fn recompiling_never_publishes_a_black_frame() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(node, (64, 64), &white, send, mode, Vec::new())]
    });
    renderer.draw(&tick(
        0.0,
        vec![job(node, (64, 64), &white, false, DRAW, Vec::new())],
    ));
    assert!(
        shown(&gpu, &mut renderer, node)
            .iter()
            .all(|p| *p == [255; 4])
    );

    // A different source drawing the same picture, held linking.
    let mut edited = (*white).clone();
    edited.body.push_str("// a structural edit\n");
    let edited = Arc::new(edited);
    renderer.hold_link(node, true);
    renderer.draw(&tick(
        0.0,
        vec![job(node, (64, 64), &edited, true, DRAW, Vec::new())],
    ));
    assert!(renderer.is_linking(node), "the edit is still linking");
    renderer.draw(&tick(
        0.0,
        vec![job(node, (64, 64), &edited, false, DRAW, Vec::new())],
    ));
    assert!(
        shown(&gpu, &mut renderer, node)
            .iter()
            .all(|p| *p == [255; 4]),
        "the old program still drew while the new one links"
    );
    assert_eq!(renderer.dropped_frames(node), 0);
}

/// **A module that fails leaves the working program drawing**, and says why on the status
/// line.
#[test]
fn a_module_that_fails_leaves_the_working_one_running() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(node, (32, 32), &white, send, mode, Vec::new())]
    });
    let mut broken = (*white).clone();
    broken.body.push_str("fn broken( { }\n");
    let broken = Arc::new(broken);
    renderer.draw(&tick(
        0.0,
        vec![job(node, (32, 32), &broken, true, DRAW, Vec::new())],
    ));
    // Linking happens on the linker threads, so the failure lands on some later draw.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        renderer.draw(&tick(
            0.1,
            vec![job(node, (32, 32), &broken, false, DRAW, Vec::new())],
        ));
        if !renderer.is_linking(node) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the link never landed"
        );
    }
    assert!(
        renderer.errors.contains_key(&node),
        "the failure is on the status line"
    );
    assert!(
        shown(&gpu, &mut renderer, node)
            .iter()
            .all(|p| *p == [255; 4]),
        "and the old program is still drawing"
    );
}

/// **A resize carries the last frame across.** Nothing is drawn after it — the dropped-frame
/// case — and the new target already holds the old picture, scaled, rather than black.
#[test]
fn a_resolution_change_never_publishes_a_black_frame() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(node, (64, 64), &white, send, mode, Vec::new())]
    });
    renderer.draw(&tick(
        0.0,
        vec![job(node, (64, 64), &white, false, DRAW, Vec::new())],
    ));
    renderer.draw(&tick(
        0.0,
        vec![job(
            node,
            (128, 96),
            &white,
            false,
            OutputMode::Suspended,
            Vec::new(),
        )],
    ));
    drain(&gpu);
    let latest = renderer.texture_of(node).expect("a frame");
    assert_eq!((latest.width(), latest.height()), (128, 96), "resized");
    assert!(
        rgba_of(&gpu, &latest).iter().all(|p| *p == [255; 4]),
        "the last good frame was carried into the new target, not left black"
    );
    let picture = shown(&gpu, &mut renderer, node);
    assert_eq!(
        picture.len(),
        128 * 96,
        "and it is what is shown once finished"
    );
}

/// A horizontal gradient, its red channel the frame's own `uv.x` — a vertical stripe pattern
/// taken to its continuous limit, so a resize carry's aspect-correct scaling (and any naive
/// stretch it is not doing) shows up exactly, pixel by pixel, rather than only at a stripe's
/// edge.
fn gradient() -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    return vec4f(uv.x, 0.0, 0.0, 1.0);
}
",
    )
}

/// The red channel the carry's mirrored, aspect-correct scaling predicts at `col` of a
/// `new_w`×`new_h` target, carried from a `gradient()` frame of `old_w`×`old_h` — the same
/// center-to-full-height scaling the mixer's `MIX` does for a deck of another shape, folded
/// through `MirrorRepeat` where it samples past the old frame's edge.
fn carried_red(col: u32, new_w: u32, new_h: u32, old_w: u32, old_h: u32) -> f32 {
    let new_aspect = new_w as f32 / new_h as f32;
    let old_aspect = old_w as f32 / old_h as f32;
    let uv_x = (col as f32 + 0.5) / new_w as f32;
    let mut s = (uv_x - 0.5) * (new_aspect / old_aspect) + 0.5;
    s = s.rem_euclid(2.0);
    if s > 1.0 {
        s = 2.0 - s;
    }
    s
}

/// The red channel a naive stretch (the old `frag_coord.xy / carry.size` alone) would have
/// put at `col` of `new_w` — what the carry must *not* match whenever the shape changed.
fn stretched_red(col: u32, new_w: u32) -> f32 {
    (col as f32 + 0.5) / new_w as f32
}

/// The gradient's red channel at `col`, middle row, of a `width`×`height` texture read back
/// with `floats_of` (rows bottom first; the gradient has no vertical variation, so any row
/// does).
fn red_at(floats: &[f32], width: u32, col: u32) -> f32 {
    let row = 0u32;
    floats[((row * width + col) * 4) as usize]
}

/// **A resize carries the old frame scaled about its center to full height, by the new aspect
/// over the old — the mixer's rule for a frame of another shape — never stretched.** A 16:9
/// gradient carried into a 9:16 target is cropped at the sides (every sample stays inside the
/// old frame); carried into a 21:9 target it is mirrored past the edges. Both differ sharply
/// from the naive stretch the carry used to do, and both keep the center column unmoved.
#[test]
fn a_resize_carries_the_old_frame_at_its_true_shape_not_stretched() {
    let old = (160, 90);

    // A fresh renderer per case: the carry must read the one clean gradient drawn at `old`,
    // never a frame already carried once before (which would no longer be a plain gradient to
    // predict against).
    for new in [(90, 160), (336, 144)] {
        let gpu = gpu::gpu();
        let node = NodeId(1);
        let shader = gradient();
        let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
        link_all(&mut renderer, &|send, mode| {
            vec![job(node, old, &shader, send, mode, Vec::new())]
        });
        renderer.draw(&tick(
            0.0,
            vec![job(node, old, &shader, false, DRAW, Vec::new())],
        ));
        renderer.draw(&tick(
            0.0,
            vec![job(
                node,
                new,
                &shader,
                false,
                OutputMode::Suspended,
                Vec::new(),
            )],
        ));
        drain(&gpu);
        let latest = renderer.texture_of(node).expect("a frame");
        assert_eq!((latest.width(), latest.height()), new, "resized");
        let floats = floats_of(&gpu, &latest);
        let (new_w, new_h) = new;

        let center = new_w / 2;
        assert!(
            (red_at(&floats, new_w, center) - 0.5).abs() < 0.03,
            "the center column stays centered at {new:?}"
        );

        for col in [0, new_w - 1] {
            let got = red_at(&floats, new_w, col);
            let want = carried_red(col, new_w, new_h, old.0, old.1);
            let naive = stretched_red(col, new_w);
            assert!(
                (got - want).abs() < 0.03,
                "at {new:?} col {col}: got {got}, aspect-correct predicts {want} (naive stretch would be {naive})"
            );
            assert!(
                (got - naive).abs() > 0.08,
                "at {new:?} col {col}: {got} should differ from the naive stretch {naive}, \
                 or the carry is still stretching"
            );
        }
    }
}

/// **A same-shape resize is unchanged**: the new aspect equals the old, so the carry's scale
/// factor is 1 and every sample lands exactly where the old naive stretch put it — a plain
/// upscale, pixel for pixel.
#[test]
fn a_same_shape_resize_is_a_plain_upscale() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let old = (160, 90);
    let new = (320, 180);
    let shader = gradient();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(node, old, &shader, send, mode, Vec::new())]
    });
    renderer.draw(&tick(
        0.0,
        vec![job(node, old, &shader, false, DRAW, Vec::new())],
    ));
    renderer.draw(&tick(
        0.0,
        vec![job(
            node,
            new,
            &shader,
            false,
            OutputMode::Suspended,
            Vec::new(),
        )],
    ));
    drain(&gpu);
    let latest = renderer.texture_of(node).expect("a frame");
    assert_eq!((latest.width(), latest.height()), new, "resized");
    let floats = floats_of(&gpu, &latest);
    for col in [0, new.0 / 4, new.0 / 2, new.0 - 1] {
        let got = red_at(&floats, new.0, col);
        let want = stretched_red(col, new.0);
        assert!(
            (got - want).abs() < 0.03,
            "col {col}: got {got}, a same-shape resize predicts {want} exactly as the old \
             carry did"
        );
    }
}

/// **An Output unplugged publishes black**, not the last frame it drew: another Output's
/// `frame` would go on feeding a picture from a chain already cut.
#[test]
fn unplugging_an_output_blanks_what_it_publishes() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(node, (32, 32), &white, send, mode, Vec::new())]
    });
    renderer.draw(&tick(
        0.0,
        vec![job(node, (32, 32), &white, false, DRAW, Vec::new())],
    ));
    drain(&gpu);
    assert!(
        rgba_of(&gpu, &renderer.texture_of(node).unwrap())
            .iter()
            .all(|p| *p == [255; 4])
    );
    renderer.draw(&tick(
        0.0,
        vec![job(
            node,
            (32, 32),
            &white,
            false,
            OutputMode::Dark,
            Vec::new(),
        )],
    ));
    drain(&gpu);
    assert!(
        rgba_of(&gpu, &renderer.texture_of(node).unwrap())
            .iter()
            .all(|p| p[..3] == [0, 0, 0]),
        "unplugged, it publishes black rather than the white it last drew"
    );
}

// ----------------------------------------------------------------- the ring and its leases

/// A module whose colour is its time, so every tick's frame differs from the last.
fn clock() -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return vec4f(fract(u.u_time), 0.5, 0.5, 1.0);
}
",
    )
}

/// **A target a viewer holds is never drawn into**, however many ticks go by, and the ring
/// does not grow past its cap to manage it.
#[test]
fn a_target_a_viewer_holds_is_never_drawn_into() {
    let gpu = gpu::gpu();
    let node = NodeId(4);
    let shader = self_feedback(node);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| {
        vec![job(
            node,
            (32, 32),
            &shader,
            send,
            mode,
            reading(&[("u_self", node)]),
        )]
    };
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    let held: Published = renderer.publish();
    let pinned = held.outputs[&node].texture.texture().clone();
    let before = bytes_of(&gpu, &pinned);
    for i in 1..20 {
        renderer.draw(&tick(i as f32 * 0.1, outputs(false, DRAW)));
        drain(&gpu);
        assert_ne!(
            renderer.texture_of(node).as_ref(),
            Some(&pinned),
            "tick {i} drew into a target a viewer holds"
        );
        drop(renderer.publish());
    }
    assert!(
        bytes_of(&gpu, &pinned) == before,
        "the held frame is the frame it was"
    );
    assert!(renderer.targets_of(node).len() <= RING);
    drop(held);
}

/// **Every target held drops the tick and keeps the picture.** Viewers that never let go claim
/// the whole ring; the Output then skips, counting `held`, and never draws into what they hold;
/// once they let go it draws again.
#[test]
fn held_frames_are_never_drawn_into_and_the_ring_resumes_once_let_go() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = clock();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(node, (64, 36), &shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    let mut holds = Vec::new();
    for i in 0..(RING + 2) {
        renderer.draw(&tick(0.1 + i as f32 * 0.13, outputs(false, DRAW)));
        drain(&gpu);
        let published = renderer.publish();
        let picture = published.outputs[&node].clone();
        let red = rgba_of(&gpu, picture.texture.texture())[0][0];
        holds.push((published, picture, red));
    }
    assert!(
        renderer.drops(node).held > 0,
        "with every target held, a tick was dropped"
    );
    assert!(renderer.targets_of(node).len() <= RING);
    for (_, picture, red) in &holds {
        assert_eq!(
            rgba_of(&gpu, picture.texture.texture())[0][0],
            *red,
            "a held frame was drawn into"
        );
    }
    drop(holds);
    let drops = renderer.drops(node).held;
    for i in 0..10 {
        renderer.draw(&tick(5.0 + i as f32 * 0.07, outputs(false, DRAW)));
        drain(&gpu);
        drop(renderer.publish());
    }
    assert_eq!(
        renderer.drops(node).held,
        drops,
        "let go, it draws every tick"
    );
}

/// A module that takes a while: `u_loops` rounds of a hash per pixel, so a frame is still on
/// the GPU when the test asks about it.
fn slow() -> Arc<supersilvia::compile::Shader> {
    module(
        &["u_loops"],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    var v = uv.x;
    for (var i = 0.0; i < u.u_loops; i += 1.0) {
        v = fract(sin(v * 12.9898 + uv.y * 78.233 + i) * 43758.5453);
    }
    return vec4f(v, uv, 1.0);
}
",
    )
}

fn loops(n: f32) -> Vec<(Arc<str>, UniformValue)> {
    vec![(Arc::from("u_loops"), UniformValue::Float(n))]
}

/// **A viewer is never handed a frame still being drawn.** A slow frame is published while
/// it is on the GPU: the picture handed over is the newest finished one, and once the slow one
/// has finished it is the one shown.
#[test]
fn a_viewer_is_never_handed_a_frame_still_being_drawn() {
    const SIZE: (u32, u32) = (1024, 1024);
    let gpu = gpu::gpu();
    let node = NodeId(2);
    let shader = slow();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode, n: f32| vec![job(node, SIZE, &shader, send, mode, loops(n))];
    link_all(&mut renderer, &|send, mode| outputs(send, mode, 1.0));
    renderer.draw(&tick(0.0, outputs(false, DRAW, 1.0)));

    let mut n = 500.0;
    for attempt in 0.. {
        assert!(attempt < 6, "no frame was slow enough to still be drawing");
        drain(&gpu);
        let before = renderer.publish();
        let finished = before.outputs[&node].texture.clone();
        renderer.draw(&tick(0.0, outputs(false, DRAW, n)));
        let marker = gpu.submit([]);
        let published = renderer.publish();
        gpu.poll();
        if gpu.is_done(marker.serial) {
            n *= 4.0;
            continue;
        }
        let drawing = renderer.texture_of(node).expect("drawn");
        assert_ne!(
            finished.texture(),
            &drawing,
            "the new frame went into a slot of its own"
        );
        let picture = &published.outputs[&node];
        if picture.texture.texture() == &drawing {
            // Finished between the draw and the publish, with only the marker behind it.
            n *= 4.0;
            continue;
        }
        assert_eq!(picture.texture, finished, "handed the newest finished one");
        drop(published);
        drain(&gpu);
        let after = renderer.publish();
        assert_eq!(
            after.outputs[&node].texture.texture(),
            &drawing,
            "once it has finished, it is the one shown"
        );
        break;
    }
}

// ---------------------------------------------------------------------------- one queue

/// **The synth keeps at most [`QUEUED_AHEAD`] submissions queued behind the one running.**
/// Six slow Outputs in one tick, none timed yet, are six submissions; none is made while more than
/// `QUEUED_AHEAD` earlier ones are still on the GPU, so an editor frame submitted at any moment
/// of the tick lands behind the pass running and at most that many more. Nothing is
/// dropped for it: the tick is slower, every Output still draws.
#[test]
fn the_synth_submits_one_output_at_a_time_and_little_ahead() {
    const OUTPUTS: u32 = 6;
    let gpu = gpu::gpu();
    let shader = slow();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode, n: f32| -> Vec<OutputJob> {
        (1..=OUTPUTS)
            .map(|i| job(NodeId(i), (512, 512), &shader, send, mode, loops(n)))
            .collect()
    };
    link_all(&mut renderer, &|send, mode| outputs(send, mode, 1.0));
    drain(&gpu);
    renderer.draw(&tick(0.0, outputs(false, DRAW, 2000.0)));
    drain(&gpu);
    // How many had to wait depends on how far the CPU gets ahead of the GPU, which another
    // process's load changes; what the throttle promises is the bound itself.
    assert!(
        renderer.most_queued_ahead() <= QUEUED_AHEAD,
        "{} synth submissions were queued ahead of one",
        renderer.most_queued_ahead()
    );
    for i in 1..=OUTPUTS {
        assert_eq!(renderer.dropped_frames(NodeId(i)), 0);
        assert!(
            rgba_of(&gpu, &renderer.texture_of(NodeId(i)).unwrap())
                .iter()
                .all(|p| p[3] == 255),
            "Output {i} drew"
        );
    }
}

/// **Cheap Outputs share a submission, and a costly one has its own.** Once every Output's pass
/// has been timed, three cheap ones, a slow one and three more cheap ones are three
/// submissions: the prelude with the first three, the slow one alone, and the last three with
/// the coda. Every one of them still draws.
///
/// **On Metal the cheap Output after the slow one is timed at about the slow one's cost** —
/// 12–16 ms beside its siblings' 0.02–0.4 ms on an idle M2 — so it goes alone as well, and an
/// idle GPU gives four submissions. With other tests on the GPU every span is inflated, the
/// cheap ones' past `SUBMISSION_MS` at times, and up to one submission per Output and the
/// prelude's own. Accepted, and recorded in `proposals/wgpu.md`.
#[test]
fn cheap_outputs_share_a_submission_and_a_costly_one_has_its_own() {
    let gpu = gpu::gpu();
    if !gpu
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        return;
    }
    let (cheap, slow) = (solid(0.2, 0.4, 0.6), slow());
    let heavy = NodeId(4);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| -> Vec<OutputJob> {
        (1..=7)
            .map(|i| {
                let node = NodeId(i);
                if node == heavy {
                    job(node, (512, 512), &slow, send, mode, loops(2000.0))
                } else {
                    job(node, (16, 16), &cheap, send, mode, Vec::new())
                }
            })
            .collect()
    };
    link_all(&mut renderer, &outputs);
    let deadline = Instant::now() + Duration::from_secs(10);
    while (1..=7).any(|i| renderer.gpu_time(NodeId(i)).is_none()) {
        renderer.draw(&tick(0.0, outputs(false, DRAW)));
        drain(&gpu);
        assert!(Instant::now() < deadline, "every pass is timed");
    }
    assert!(
        renderer
            .gpu_time(heavy)
            .is_some_and(|t| t.latest > supersilvia::render::SUBMISSION_MS),
        "the slow Output costs more than a submission gathers"
    );
    let before = renderer.submissions();
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    let submissions = renderer.submissions() - before;
    if gpu.adapter().get_info().backend == wgpu::Backend::Metal {
        let after = NodeId(5);
        assert!(
            renderer
                .gpu_time(after)
                .is_some_and(|t| t.latest > supersilvia::render::SUBMISSION_MS),
            "on Metal the cheap Output after the slow one measures about the slow one's time: \
             {:?}",
            renderer.gpu_time(after)
        );
        assert!(
            (4..=8).contains(&submissions),
            "on Metal it goes alone too, beside whichever others the GPU's load made dear: \
             {submissions}"
        );
    } else {
        assert_eq!(submissions, 3);
    }
    for i in 1..=7 {
        assert_eq!(renderer.dropped_frames(NodeId(i)), 0);
    }
}

/// **An Output unplugged while viewers hold every target it has blanks none of them.** Its
/// black waits for a target nothing holds, and lands on the first tick there is one.
#[test]
fn unplugging_an_output_whose_targets_are_all_held_blanks_none_of_them() {
    let gpu = gpu::gpu();
    let node = NodeId(12);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(node, (16, 16), &white, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    let step = |renderer: &mut Renderer, mode| {
        renderer.draw(&tick(0.0, outputs(false, mode)));
        drain(&gpu);
    };
    // A viewer that never lets go, until the ring can grow no further.
    let mut held = Vec::new();
    for _ in 0..2 * RING {
        step(&mut renderer, DRAW);
        held.push(renderer.publish());
    }
    assert_eq!(renderer.targets_of(node).len(), RING);
    let textures: Vec<wgpu::Texture> = held
        .iter()
        .map(|p| p.outputs[&node].texture.texture().clone())
        .collect();

    step(&mut renderer, OutputMode::Dark);
    for texture in &textures {
        assert!(
            rgba_of(&gpu, texture).iter().all(|p| *p == [255; 4]),
            "a held frame was blanked"
        );
    }

    drop(held);
    step(&mut renderer, OutputMode::Dark);
    let latest = renderer.texture_of(node).expect("a latest frame");
    assert!(
        rgba_of(&gpu, &latest).iter().all(|p| p[..3] == [0, 0, 0]),
        "black once one is free"
    );
}

/// **Every render target is half float, not 8-bit unorm**: `Rgba16Float`, every slot of the
/// ring and the frame published. The whole feedback path depends on it — at 8 bits a
/// per-frame delta under 1/255 rounds away, so a low mix amount leaves a trail that never
/// moves the channel — and a level under 1/255 drawn is read back as itself.
#[test]
fn an_outputs_render_targets_are_half_float() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let dim = solid(0.001, 0.5, 0.75);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(node, (64, 64), &dim, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    let mut held = Vec::new();
    for _ in 0..3 {
        renderer.draw(&tick(0.0, outputs(false, DRAW)));
        drain(&gpu);
        held.push(renderer.publish());
    }
    let targets = renderer.targets_of(node);
    assert!(targets.len() > 1, "the ring made more than its first slot");
    for target in &targets {
        assert_eq!(target.format(), wgpu::TextureFormat::Rgba16Float);
    }
    assert_eq!(
        held[2].outputs[&node].texture.texture().format(),
        wgpu::TextureFormat::Rgba16Float,
        "and what is published"
    );
    let red = gpu::floats_of(&gpu, &renderer.texture_of(node).expect("drawn"))[0];
    assert!(
        (red - 0.001).abs() < 1.0e-5,
        "a level under 1/255 read {red}"
    );
}

/// **A deleted Output's target outlives it while a viewer holds it**, and is freed once the
/// viewer lets go — by the claim, not by a count of ticks. Counted by the device's own texture
/// memory and views, on a device of its own: more than the renderer alone holds while the
/// viewer holds the frame, and none more once it lets go.
#[test]
fn a_deleted_outputs_target_is_freed_when_the_last_viewer_lets_go() {
    let gpu = gpu::gpu();
    let node = NodeId(9);
    let white = solid(1.0, 1.0, 1.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    // wgpu-hal's Vulkan device counts a texture it wraps and not one it makes, so what is
    // counted here is texture memory and texture views, both counted both ways. Its Metal
    // device counts views and never texture memory, which stays at zero there.
    let textures = |renderer: &mut Renderer| {
        renderer.draw(&FrameJob::default());
        drain(&gpu);
        let hal = gpu.device().get_internal_counters().hal;
        (hal.texture_memory.read(), hal.texture_views.read())
    };
    let alone = textures(&mut renderer);
    let outputs = |send: bool, mode| vec![job(node, (16, 16), &white, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    let held = renderer.publish();
    let texture = held.outputs[&node].texture.texture().clone();

    // The Output is deleted, and ticks go on well past any count a timer would have used.
    for _ in 0..12 {
        renderer.draw(&FrameJob::default());
    }
    let holding = textures(&mut renderer);
    let alive = if cfg!(target_os = "macos") {
        holding.1 > alone.1
    } else {
        holding.0 > alone.0 && holding.1 > alone.1
    };
    assert!(
        alive,
        "the held frame is alive: {holding:?} against {alone:?} for the renderer alone"
    );
    assert!(
        rgba_of(&gpu, &texture).iter().all(|p| *p == [255; 4]),
        "and still the frame"
    );

    drop((held, texture));
    assert_eq!(textures(&mut renderer), alone, "freed once nothing held it");
}

// ------------------------------------------------------------------ every Output on a busy GPU

fn timestamps(gpu: &Gpu) -> bool {
    gpu.device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
}

/// One feedback loop and three stateless Outputs heavy enough to keep the GPU behind, in the
/// order the synth submits them, which is the plan's: the three, then the loop.
struct Saturated {
    looped: NodeId,
    stateless: [NodeId; 3],
    /// Rounds of the slow module's hash per pixel, tuned so one stateless frame costs what
    /// [`Saturated::new`] was asked for on this GPU.
    loops: f32,
    slow: Arc<Shader>,
    feedback: Arc<Shader>,
}

impl Saturated {
    const SIZE: (u32, u32) = (1024, 1024);

    fn new(gpu: &Gpu, renderer: &mut Renderer, frame: Duration) -> Self {
        let looped = NodeId(4);
        let mut this = Self {
            looped,
            stateless: [NodeId(1), NodeId(2), NodeId(3)],
            loops: 16.0,
            slow: slow(),
            feedback: self_feedback(looped),
        };
        link_all(renderer, &|send, mode| this.jobs(send, mode));
        // Doubled until one stateless frame is measurable, then scaled to `frame`.
        let mut took = this.frame_ms(gpu, renderer);
        while took < 1.0 && this.loops < 1.0e7 {
            this.loops *= 2.0;
            took = this.frame_ms(gpu, renderer);
        }
        this.loops *= frame.as_secs_f32() * 1000.0 / took;
        this
    }

    /// What one stateless frame costs, drawn alone ten times so the GPU has clocked up under
    /// the load: the GPU's own timer where the device has one, and otherwise the shortest
    /// drain-to-drain wall time.
    fn frame_ms(&self, gpu: &Gpu, renderer: &mut Renderer) -> f32 {
        // The rest suspended rather than left out: an Output missing from a job is freed.
        let mut outputs = self.jobs(false, DRAW);
        for o in &mut outputs {
            if o.node != self.stateless[0] {
                o.mode = OutputMode::Suspended;
            }
        }
        let job = tick(0.0, outputs);
        let mut shortest = f32::INFINITY;
        for _ in 0..10 {
            drain(gpu);
            let start = Instant::now();
            renderer.draw(&job);
            drain(gpu);
            shortest = shortest.min(start.elapsed().as_secs_f32() * 1000.0);
        }
        renderer
            .gpu_time(self.stateless[0])
            .map_or(shortest, |t| t.latest)
    }

    /// Every Output's job this tick, in the synth's order.
    fn jobs(&self, send: bool, mode: OutputMode) -> Vec<OutputJob> {
        self.stateless
            .into_iter()
            .map(|node| job(node, Self::SIZE, &self.slow, send, mode, loops(self.loops)))
            .chain([job(
                self.looped,
                (256, 256),
                &self.feedback,
                send,
                mode,
                reading(&[("u_self", self.looped)]),
            )])
            .collect()
    }

    /// Every Output's dropped frames, the loop's first.
    fn dropped(&self, renderer: &Renderer) -> [u64; 4] {
        let [a, b, c] = self.stateless.map(|id| renderer.dropped_frames(id));
        [renderer.dropped_frames(self.looped), a, b, c]
    }
}

/// How many ticks old the frame a viewer is shown of `node` is, at this publish.
fn age(published: &Published, node: NodeId) -> u64 {
    let picture = published.outputs.get(&node).expect("a picture is shown");
    published.tick
        - picture
            .drawn_tick
            .expect("an Output's frame names its tick")
}

/// **No Output drops a frame on a GPU that cannot keep up.** Ticks are submitted back to back,
/// far faster than the GPU finishes them, three heavy stateless Outputs beside a loop. Every
/// one of the four draws on every tick; what bounds the queue is the draw waiting for the tick
/// before it; and viewers are shown each Output's frames as they finish — the frame shown is
/// never older than [`TICKS_IN_FLIGHT`] ticks, however far behind the GPU is.
#[test]
fn nothing_drops_on_a_saturated_gpu() {
    const TICKS: u64 = 60;
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let busy = Saturated::new(&gpu, &mut renderer, Duration::from_millis(6));
    let watched = [busy.looped, busy.stateless[0]];
    let before = busy.dropped(&renderer);
    let waits = renderer.waits();
    let (mut longest, mut oldest) = (Duration::ZERO, [0; 2]);
    for t in 0..TICKS {
        let start = Instant::now();
        renderer.draw(&tick(t as f32 / 60.0, busy.jobs(false, DRAW)));
        let published = renderer.publish();
        // The first tick's shown frames are the tuning's, drawn before the loop drew at all.
        if t > 0 {
            for (k, id) in watched.iter().enumerate() {
                oldest[k] = oldest[k].max(age(&published, *id));
            }
        }
        longest = longest.max(start.elapsed());
    }
    drain(&gpu);
    let waits = renderer.waits() - waits;
    let dropped: Vec<u64> = busy
        .dropped(&renderer)
        .iter()
        .zip(before)
        .map(|(a, b)| a - b)
        .collect();
    println!(
        "{TICKS} ticks back to back: dropped {dropped:?} (the loop's first), the draw waited \
         for the queue on {waits} ticks, the shown frames of the loop and a stateless Output \
         were at most {oldest:?} ticks old, and the longest tick took {longest:?}"
    );
    assert_eq!(dropped, [0; 4], "every Output drew on every tick");
    for (k, o) in oldest.iter().enumerate() {
        assert!(
            *o <= TICKS_IN_FLIGHT as u64,
            "and viewers were shown each one's frames as they finished: {k} was {o} ticks old"
        );
    }
}

/// An Output that writes which tick drew it into a tap slot, from one fragment: a whole
/// frame's reading is the template's first word plus one, and its second is the tick at sixty
/// a second. With `feedback`, it mixes in its own last frame.
fn tapped_clock(node: NodeId, feedback: bool) -> Arc<Shader> {
    let picture = if feedback {
        "mix(textureSampleLevel(u_self, sampler_mirror_linear, uv, 0.0) * 0.99, \
         vec4f(uv, fract(u.u_time), 1.0), 0.1)"
    } else {
        "vec4f(uv, fract(u.u_time), 1.0)"
    };
    let textures: &[(&str, NodeId)] = if feedback { &[("u_self", node)] } else { &[] };
    tapping(
        &[],
        textures,
        1,
        &format!(
            "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    let uv = frag_coord.xy / u.u_resolution;
    if (all(vec2i(frag_coord.xy) == vec2i(0))) {{
        atomicAdd(&tap[0], 1u);
        atomicAdd(&tap[1], u32(u.u_time * 60.0 + 0.5));
    }}
    return {picture};
}}
"
        ),
    )
}

/// A hand-written module writing `slots` tap slots: `body` holds `fs_main` and may name `tap`,
/// declared at the compiler's binding where there are any.
fn tapping(floats: &[&str], textures: &[(&str, NodeId)], slots: usize, body: &str) -> Arc<Shader> {
    let decl = if slots == 0 {
        String::new()
    } else {
        format!(
            "@group(0) @binding({}) var<storage, read_write> tap: array<atomic<u32>>;\n",
            wgsl::TAPS
        )
    };
    let mut shader = (*module(floats, textures, &format!("{decl}{body}"))).clone();
    shader.taps = vec![(NodeId(0), TapKind::Stats); slots];
    Arc::new(shader)
}

fn words_of(taken: &[(NodeId, Vec<u32>)], node: NodeId) -> Option<Vec<u32>> {
    taken
        .iter()
        .find(|(id, _)| *id == node)
        .map(|(_, w)| w.clone())
}

/// **A loop's readings arrive while the GPU stays behind.** A loop is drawn behind its previous
/// frame, so on a GPU that never catches up that frame is unfinished on most ticks; what its
/// earlier frames left — the tap words that can drive the graph, and the GPU time — is taken
/// from whichever of them has finished, rather than waiting for a tick that finds the last one
/// done. The draw bounds the queue at [`TICKS_IN_FLIGHT`] ticks, so the newest reading is never
/// older than that.
#[test]
fn a_loops_readings_arrive_on_a_saturated_gpu() {
    const TICKS: u64 = 60;
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let busy = Saturated::new(&gpu, &mut renderer, Duration::from_millis(6));
    let tapped = NodeId(9);
    let shader = tapped_clock(tapped, true);
    let jobs = |send: bool, mode| {
        let mut jobs = vec![job(
            tapped,
            (256, 256),
            &shader,
            send,
            mode,
            reading(&[("u_self", tapped)]),
        )];
        jobs.extend(busy.jobs(send, mode));
        jobs
    };
    link_all(&mut renderer, &jobs);

    let waits = renderer.waits();
    let (mut readings, mut newest, mut stalest) = (0, None, 0);
    for t in 0..TICKS {
        renderer.draw(&tick(t as f32 / 60.0, jobs(false, DRAW)));
        renderer.publish();
        if let Some(words) = words_of(&renderer.take_taps(), tapped) {
            assert_eq!(words[0], TAP_TEMPLATE[0] + 1, "a whole frame's reading");
            readings += 1;
            newest = newest.max(Some(u64::from(words[1])));
        }
        if t > TICKS_IN_FLIGHT as u64 {
            stalest = stalest.max(t - newest.unwrap_or(0));
        }
    }
    let waits = renderer.waits() - waits;
    println!(
        "{TICKS} ticks back to back: the draw waited for the queue on {waits}, the loop dropped \
         {}, its readings arrived on {readings}, and the newest was at most {stalest} ticks old",
        renderer.dropped_frames(tapped)
    );
    assert_eq!(renderer.dropped_frames(tapped), 0, "a loop drops no step");
    assert!(
        stalest <= TICKS_IN_FLIGHT as u64,
        "the newest reading was {stalest} ticks old"
    );
    assert_eq!(
        renderer.gpu_time(tapped).is_some(),
        timestamps(&gpu),
        "and what its frames cost is known, where the device can time them"
    );
}

/// **A finished frame's tap words and thumbnail are read without waiting for the queue behind
/// it.** A tapped Output draws, asked for a thumbnail, then heavy frames are queued behind it,
/// each a submission of its own, as the Outputs after it in a tick and the ticks after that
/// are. Once the tapped frame is found finished, an idle tick takes its words and its picture
/// while the heavy work queued after it is still running: the reads took what the GPU had
/// already written, and neither a wait of the renderer's nor the queue draining was needed to
/// get it. How long the reads took is printed and not held.
#[test]
fn a_finished_frames_readbacks_are_taken_without_waiting_for_the_queue_behind_it() {
    const TRIALS: u32 = 20;
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = tapped_clock(node, false);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(node, (64, 64), &shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    let heavy = Reference::new(&gpu, &slow(), (1024, 1024));
    // Doubled until one heavy frame takes 30 ms of the GPU, measured on the wall with the
    // queue drained on either side.
    let mut n = 16.0f32;
    loop {
        drain(&gpu);
        let start = Instant::now();
        heavy.draw_with(&gpu, 0.0, &[], &loops(n));
        drain(&gpu);
        if start.elapsed() >= Duration::from_millis(30) || n > 1.0e7 {
            break;
        }
        n *= 2.0;
    }

    let mut counted = 0;
    for trial in 0..TRIALS {
        drain(&gpu);
        renderer.draw(&tick(0.0, outputs(false, OutputMode::Idle)));
        renderer.take_taps();
        renderer.request_thumbnail(node);
        renderer.draw(&tick((trial + 1) as f32 / 60.0, outputs(false, DRAW)));
        let drawn = renderer.texture_of(node).expect("drawn");
        for _ in 0..4 {
            heavy.draw_with(&gpu, 0.0, &[], &loops(n));
        }
        let behind = gpu.submit([]);

        // Until the tapped frame is found finished, as a tick's publish finds it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while renderer
            .publish()
            .outputs
            .get(&node)
            .is_none_or(|p| p.texture.texture() != &drawn)
        {
            assert!(Instant::now() < deadline, "the tapped frame never finished");
            std::hint::spin_loop();
        }
        // A thread the rest of the suite kept off the CPU for the heavy frames' whole run
        // finds them finished too, and such a trial proves nothing either way: its reads are
        // taken, and not counted.
        gpu.poll();
        let behind_yet = !gpu.is_done(behind.serial);

        let (waits, throttled) = (renderer.waits(), renderer.throttled());
        let start = Instant::now();
        renderer.draw(&tick(0.0, outputs(false, OutputMode::Idle)));
        let words = words_of(&renderer.take_taps(), node);
        let picture = renderer
            .take_thumbnails()
            .into_iter()
            .find(|(id, _)| *id == node);
        let took = start.elapsed();
        gpu.poll();
        let drained = gpu.is_done(behind.serial);
        println!(
            "trial {trial}: the reads took {took:?}; the queue behind them drained: {drained}{}",
            if behind_yet {
                ""
            } else {
                " (it had before the reads; not counted)"
            }
        );
        let words = words.expect("the finished frame's words");
        let (_, picture) = picture.expect("the finished frame's thumbnail");
        assert_eq!(picture.len(), THUMBNAIL_BYTES, "a whole thumbnail");
        assert_eq!(words[0], TAP_TEMPLATE[0] + 1, "a whole frame's reading");
        assert_eq!(words[1], trial + 1, "and the frame that was drawn");
        assert_eq!(
            (renderer.waits(), renderer.throttled()),
            (waits, throttled),
            "the idle tick waited for the GPU"
        );
        if behind_yet {
            counted += 1;
            assert!(
                !drained,
                "the reads waited for the heavy frames queued behind the one they read"
            );
            if counted == 3 {
                break;
            }
        } else if trial < 3 {
            // Heavier, so the next trial's queue outlasts whatever kept this thread away.
            n *= 2.0;
        }
    }
    assert!(
        counted > 0,
        "in no trial were the heavy frames still on the GPU once the tapped one had finished"
    );
    drain(&gpu);
}

/// **Paced at 85 Hz with the GPU half again over budget, nothing drops and the tick slows
/// instead.** The loop and the three stateless Outputs each draw on every tick, so every one
/// keeps the one rate; and a light stateless Output tapping which tick drew it says the newest
/// finished frame is never older than the queue's bound. What the run measured — the rate and
/// the spread of the intervals — is printed and not held, since it is a ratio of this GPU's
/// speed to whatever else is running on it.
#[test]
fn paced_over_budget_nothing_drops_and_the_tick_slows_evenly() {
    const TICKS: u64 = 170;
    const HZ: f64 = 85.0;
    let interval = Duration::from_secs_f64(1.0 / HZ);
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    // Three stateless frames of half the interval each: half again the budget.
    let busy = Saturated::new(&gpu, &mut renderer, interval / 2);
    let clock = NodeId(9);
    let shader = tapped_clock(clock, false);
    let jobs = |send: bool, mode| {
        let mut jobs = busy.jobs(send, mode);
        jobs.push(job(clock, (64, 64), &shader, send, mode, Vec::new()));
        jobs
    };
    link_all(&mut renderer, &jobs);
    let before = busy.dropped(&renderer);
    let clock_before = renderer.dropped_frames(clock);
    let waits = renderer.waits();
    let (mut newest, mut stalest) = (None, 0);
    let mut starts = Vec::with_capacity(TICKS as usize);
    // The synth thread's loop: a deadline carried forward, re-anchored on the wall by a tick
    // that ends past it, so nothing is caught up.
    let mut deadline = Instant::now();
    for t in 0..TICKS {
        starts.push(Instant::now());
        renderer.draw(&tick(t as f32 / 60.0, jobs(false, DRAW)));
        renderer.publish();
        if let Some(words) = words_of(&renderer.take_taps(), clock) {
            assert_eq!(words[0], TAP_TEMPLATE[0] + 1, "a whole frame's reading");
            newest = newest.max(Some(u64::from(words[1])));
        }
        if t > TICKS_IN_FLIGHT as u64 {
            stalest = stalest.max(t - newest.unwrap_or(0));
        }
        deadline += interval;
        let now = Instant::now();
        match deadline.checked_duration_since(now) {
            Some(rest) => std::thread::sleep(rest),
            None => deadline = now,
        }
    }
    let took = starts[0].elapsed().as_secs_f64();
    let waits = renderer.waits() - waits;
    let dropped: Vec<u64> = busy
        .dropped(&renderer)
        .iter()
        .zip(before)
        .map(|(a, b)| a - b)
        .chain([renderer.dropped_frames(clock) - clock_before])
        .collect();
    // The intervals once the queue has filled, sorted.
    let mut intervals: Vec<f64> = starts
        .windows(2)
        .skip(2 * TICKS_IN_FLIGHT)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
        .collect();
    intervals.sort_by(f64::total_cmp);
    let at = |q: f64| intervals[((intervals.len() - 1) as f64 * q).round() as usize];
    let rate = TICKS as f64 / took;
    println!(
        "{TICKS} ticks paced at {HZ} Hz, stateless frames of {:.2} ms: {rate:.1} ticks/s, \
         intervals p50 {:.2} ms p90 {:.2} ms; the draw waited for the queue on {waits}; dropped \
         {dropped:?} (the loop's first, the clock's last); the newest finished frame at most \
         {stalest} ticks old",
        renderer
            .gpu_time(busy.stateless[0])
            .map_or(0.0, |t| t.latest),
        at(0.5),
        at(0.9),
    );
    assert!(
        dropped.iter().all(|d| *d == 0),
        "every Output drew on every tick: {dropped:?}"
    );
    assert!(
        stalest <= TICKS_IN_FLIGHT as u64,
        "the queue stayed bounded: the newest finished frame was {stalest} ticks old"
    );
}

/// On the tab being looked at: A, and B reading A's frame, beside three stateless Outputs of
/// the slow module and a feedback loop elsewhere — in the synth's order, which is the plan's:
/// A, the three, B, then the loop.
struct Pair {
    a: NodeId,
    b: NodeId,
    heavy: [NodeId; 3],
    looped: NodeId,
    /// Rounds of the slow module's hash per pixel in each heavy Output.
    loops: f32,
    /// A's, B's, the heavy Outputs' and the loop's.
    shaders: [Arc<Shader>; 4],
}

impl Pair {
    const SIZE: (u32, u32) = (64, 36);

    fn new(loops: f32) -> Self {
        let (a, looped) = (NodeId(11), NodeId(16));
        Self {
            a,
            b: NodeId(12),
            heavy: [NodeId(13), NodeId(14), NodeId(15)],
            looped,
            loops,
            shaders: [producer(), consumer(a), slow(), self_feedback(looped)],
        }
    }

    fn jobs(&self, send: bool, mode: OutputMode) -> Vec<OutputJob> {
        let [producer, consumer, slow, feedback] = &self.shaders;
        let mut jobs = vec![job(self.a, Self::SIZE, producer, send, mode, Vec::new())];
        for node in self.heavy {
            jobs.push(job(
                node,
                Saturated::SIZE,
                slow,
                send,
                mode,
                loops(self.loops),
            ));
        }
        jobs.push(job(
            self.b,
            Self::SIZE,
            consumer,
            send,
            mode,
            reading(&[("u_from", self.a)]),
        ));
        jobs.push(job(
            self.looped,
            Self::SIZE,
            feedback,
            send,
            mode,
            reading(&[("u_self", self.looped)]),
        ));
        jobs
    }

    /// Every Output's dropped frames: A, B, the three heavy ones and the loop.
    fn dropped(&self, renderer: &Renderer) -> Vec<u64> {
        [self.a, self.b]
            .into_iter()
            .chain(self.heavy)
            .chain([self.looped])
            .map(|id| renderer.dropped_frames(id))
            .collect()
    }
}

/// **A consumer and its producer draw on every tick of a saturated GPU.** Three heavy stateless
/// Outputs keep the GPU behind, ticks back to back: A, B, the heavy three and the loop each draw
/// on every tick, and B's last frame is the one A drew on the last tick.
#[test]
fn a_consumer_and_its_producer_draw_every_tick_on_a_saturated_gpu() {
    const TICKS: u64 = 90;
    let gpu = gpu::gpu();
    let loops = {
        let mut tuning = Renderer::new(gpu.clone()).expect("renderer");
        Saturated::new(&gpu, &mut tuning, Duration::from_millis(6)).loops
    };
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let pair = Pair::new(loops);
    link_all(&mut renderer, &|send, mode| pair.jobs(send, mode));
    let before = pair.dropped(&renderer);
    let waits = renderer.waits();
    for t in 0..TICKS {
        renderer.draw(&tick(0.5 + t as f32 * 0.73, pair.jobs(false, DRAW)));
        renderer.publish();
    }
    drain(&gpu);
    let waits = renderer.waits() - waits;
    let dropped: Vec<u64> = pair
        .dropped(&renderer)
        .iter()
        .zip(&before)
        .map(|(a, b)| a - b)
        .collect();
    println!(
        "{TICKS} ticks back to back: the draw waited for the queue on {waits}; dropped \
         {dropped:?} (A, B, the heavy three, the loop)"
    );
    assert!(
        dropped.iter().all(|d| *d == 0),
        "every Output drew on every tick: {dropped:?}"
    );
    let a = bytes_of(&gpu, &renderer.texture_of(pair.a).expect("A drew"));
    let b = bytes_of(&gpu, &renderer.texture_of(pair.b).expect("B drew"));
    assert!(a == b, "B holds the frame A drew on the last tick");
}

// ------------------------------------------------------- tap readings and the ring under change

/// Counts its fragments into word 0 and writes its `u_time` into word 1 from pixel (0, 0),
/// after `u_iter` rounds of arithmetic per fragment, so a reading is whole only where the first
/// word is the frame's pixel count. Red is `fract(u_time)`.
fn counting(tag: &str) -> Arc<Shader> {
    tapping(
        &["u_iter"],
        &[],
        1,
        &format!(
            "
// {tag}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    let p = frag_coord.xy / u.u_resolution;
    var a = p.x;
    for (var i = 0; i < i32(u.u_iter); i++) {{ a = sin(a * 1.0001 + p.y + f32(i) * 0.001); }}
    atomicAdd(&tap[0], 1u);
    if (all(vec2i(frag_coord.xy) == vec2i(0))) {{ atomicStore(&tap[1], bitcast<u32>(u.u_time)); }}
    return vec4f(fract(u.u_time), a * 0.001, 0.0, 1.0);
}}
"
        ),
    )
}

/// One Output drawn this tick with `u_iter` rounds where its module reads them.
fn counted(
    node: NodeId,
    size: (u32, u32),
    shader: &Arc<Shader>,
    (send, mode): (bool, OutputMode),
    iter: f32,
) -> OutputJob {
    job(
        node,
        size,
        shader,
        send,
        mode,
        vec![(Arc::from("u_iter"), UniformValue::Float(iter))],
    )
}

/// A tick of these Outputs at `time`, and a small mix of nothing.
fn tick_of(time: f32, outputs: Vec<OutputJob>) -> FrameJob {
    FrameJob {
        time,
        outputs,
        mixer: MixerJob {
            resolution: (64, 36),
            ..MixerJob::default()
        },
        ..FrameJob::default()
    }
}

/// **A resize requested every tick keeps a finished picture on screen and no more than
/// [`RING`] targets.** Once the size settles and the viewer lets go, the requested size lands.
#[test]
fn an_output_resized_every_tick_keeps_a_picture_and_a_bounded_ring() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = counting("resized");
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let job = |size, time| {
        tick_of(
            time,
            vec![counted(node, size, &shader, (false, DRAW), 600.0)],
        )
    };
    link_all(&mut r, &|send, mode| {
        vec![counted(node, (1280, 720), &shader, (send, mode), 600.0)]
    });
    let mut held: VecDeque<Published> = VecDeque::new();
    let mut most = 0;
    let before = r.drops(node).held;
    for i in 0..120u32 {
        let size = if i % 2 == 0 { (1280, 720) } else { (1296, 720) };
        r.draw(&job(size, i as f32));
        held.push_back(r.publish());
        while held.len() > 2 {
            held.pop_front();
        }
        most = most.max(r.targets_of(node).len());
    }
    drain(&gpu);
    let dropped = r.drops(node).held - before;
    let shown = held.back().and_then(|p| p.outputs.get(&node)).cloned();
    drop(held);
    let shown = shown.expect("a finished frame stays published");
    assert_eq!(
        rgba_of(&gpu, shown.texture.texture())[0][3],
        255,
        "the picture never became transparent black"
    );
    for i in 0..10 {
        r.draw(&job((1296, 720), 120.0 + i as f32));
        drop(r.publish());
    }
    drain(&gpu);
    let published = r.publish();
    let last = &published.outputs[&node];
    println!("resized every tick: {dropped} skips, at most {most} targets");
    assert_eq!((last.width, last.height), (1296, 720));
    assert!(most <= RING, "the ring passed its limit: {most}");
}

/// **One resize under load keeps the old picture while a viewer holds two frames**; after the
/// viewer lets go, the new size is drawn without a black transition.
#[test]
fn one_resize_under_load_keeps_the_old_picture_until_the_new_one_lands() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = counting("resized once");
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let job = |size, time| {
        tick_of(
            time,
            vec![counted(node, size, &shader, (false, DRAW), 600.0)],
        )
    };
    link_all(&mut r, &|send, mode| {
        vec![counted(node, (1280, 720), &shader, (send, mode), 600.0)]
    });
    let mut held: VecDeque<Published> = VecDeque::new();
    let mut step = |r: &mut Renderer, size, time| {
        r.draw(&job(size, time));
        held.push_back(r.publish());
        while held.len() > 2 {
            held.pop_front();
        }
    };
    for i in 0..30 {
        step(&mut r, (1280, 720), i as f32);
    }
    let before = r.drops(node).held;
    let mut log = Vec::new();
    for i in 0..10 {
        step(&mut r, (1920, 1080), 30.0 + i as f32);
        log.push((r.targets_of(node).len(), r.drops(node).held - before));
    }
    drain(&gpu);
    let shown = held.back().and_then(|p| p.outputs.get(&node)).cloned();
    drop(held);
    let shown = shown.expect("a finished frame remains visible");
    assert_eq!(
        rgba_of(&gpu, shown.texture.texture())[0][3],
        255,
        "the resize showed transparent black"
    );
    for i in 0..10 {
        r.draw(&job((1920, 1080), 40.0 + i as f32));
        drop(r.publish());
    }
    drain(&gpu);
    let published = r.publish();
    let picture = &published.outputs[&node];
    println!("after one resize under load, (targets, skips) per tick: {log:?}");
    assert_eq!((picture.width, picture.height), (1920, 1080));
    assert_eq!(rgba_of(&gpu, picture.texture.texture())[0][3], 255);
    assert!(log.iter().all(|(targets, _)| *targets <= RING));
}

/// **A viewer painting at a third of the synth rate can keep two older pictures** while the
/// synth still needs latest, shown and a free target: nothing is dropped for it.
#[test]
fn two_older_viewer_frames_do_not_starve_the_ring() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = clock();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut r, &|send, mode| {
        vec![job(node, (64, 36), &shader, send, mode, Vec::new())]
    });
    let mut held: VecDeque<Published> = VecDeque::new();
    let before = r.drops(node).held;
    for i in 0..60 {
        r.draw(&tick_of(
            i as f32 * 0.01,
            vec![job(node, (64, 36), &shader, false, DRAW, Vec::new())],
        ));
        drain(&gpu);
        held.push_back(r.publish());
        while held.len() > 3 {
            held.pop_front();
        }
    }
    let drops = r.drops(node).held - before;
    let targets = r.targets_of(node).len();
    drop(held);
    assert_eq!(drops, 0, "a slower viewer starved the draw target");
    assert!(targets <= RING);
}

/// **Under a slow GPU every tap reading is whole, in order and recent.** Every fragment of a
/// heavy Output counts itself and one records the tick's time: each reading handed over has
/// the whole frame's count, never goes back in time, and is of a frame drawn no more than the
/// queue's bound and one before.
#[test]
fn readings_under_a_slow_gpu_are_whole_in_order_and_recent() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let size = (960, 540);
    let shader = counting("slow");
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut r, &|send, mode| {
        vec![counted(node, size, &shader, (send, mode), 400.0)]
    });
    r.take_taps();
    let full = size.0 * size.1;
    let (mut last, mut readings, mut bad) = (-1.0f32, 0, Vec::new());
    for t in 1..=200u32 {
        let now = t as f32;
        r.draw(&tick_of(
            now,
            vec![counted(node, size, &shader, (false, DRAW), 400.0)],
        ));
        drop(r.publish());
        if let Some(w) = words_of(&r.take_taps(), node) {
            readings += 1;
            let at = f32::from_bits(w[1]);
            if w[0] != full || at < last || now - at > (TICKS_IN_FLIGHT + 1) as f32 || at > now {
                bad.push((t, w[0], at));
            }
            last = at;
        }
    }
    println!(
        "{readings} readings over 200 ticks; the queue waited on {}",
        r.waits()
    );
    assert!(readings > 100, "readings arrive: {readings} in 200 ticks");
    assert!(
        bad.is_empty(),
        "torn, stale or out-of-order readings: {bad:?}"
    );
}

// ------------------------------------------------------------------------------ churn

/// Adds `adds[i]` into slot `i`'s first word from pixel (0, 0) alone. `tag` keeps two sources
/// with the same writes apart.
fn tap_writer(tag: &str, adds: &[u32]) -> Arc<Shader> {
    use std::fmt::Write as _;
    let mut writes = String::new();
    for (i, add) in adds.iter().enumerate() {
        writeln!(
            writes,
            "        atomicAdd(&tap[{}], {add}u);",
            i * TAP_WORDS
        )
        .unwrap();
    }
    tapping(
        &[],
        &[],
        adds.len(),
        &format!(
            "
// {tag}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    if (all(vec2i(frag_coord.xy) == vec2i(0))) {{
{writes}    }}
    return vec4f(1.0);
}}
"
        ),
    )
}

/// What the device holds, by kind, once everything submitted has finished and been freed.
/// Textures by their bytes: wgpu-hal's Vulkan device counts a texture it wraps and not one it
/// makes, so its texture count only ever falls.
fn census(gpu: &Gpu) -> [isize; 8] {
    drain(gpu);
    let hal = gpu.device().get_internal_counters().hal;
    [
        hal.texture_memory.read(),
        hal.texture_views.read(),
        hal.buffers.read(),
        hal.bind_groups.read(),
        hal.render_pipelines.read(),
        hal.shader_modules.read(),
        hal.query_sets.read(),
        hal.buffer_memory.read(),
    ]
}

/// **GPU objects do not grow with churn.** Three thousand ticks of four Outputs coming and
/// going, resizing, changing source and tap count, going dark, idle and suspended, asked for
/// thumbnails, Snaps and captures, under a viewer that sometimes holds five frames: the
/// textures, views, buffers, bind groups, pipelines, modules and query sets alive at the end
/// are what they were early on. Counted by the device's own counters, on a device of its own.
#[test]
fn gpu_objects_do_not_grow_with_churn() {
    const KINDS: [&str; 8] = [
        "texture bytes",
        "texture views",
        "buffers",
        "bind groups",
        "render pipelines",
        "shader modules",
        "query sets",
        "buffer bytes",
    ];
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let sources: Vec<Arc<Shader>> = (0..6)
        .map(|k| tap_writer(&format!("churn {k}"), &vec![1; k % 3]))
        .collect();
    let mut held: VecDeque<Published> = VecDeque::new();
    let mut counts = Vec::new();
    let mut rng = 0x1234_5678u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    // Per Output: which source it draws, and whether its renderer has it.
    let mut which = [0usize; 4];
    let mut sent = [false; 4];
    for t in 0..3000u32 {
        let mut outputs = Vec::new();
        for slot in 0..4usize {
            // The fourth comes and goes every 37 ticks; the others stay.
            if slot == 3 && (t / 37) % 2 == 1 {
                sent[3] = false;
                continue;
            }
            let roll = next();
            let mut send = false;
            if !sent[slot] || roll % 97 == 0 {
                which[slot] = (next() as usize) % sources.len();
                send = true;
                sent[slot] = true;
            }
            let size = if roll % 53 == 0 { (80, 45) } else { (64, 36) };
            let mode = match roll % 41 {
                0 => OutputMode::Suspended,
                1 => OutputMode::Idle,
                _ => DRAW,
            };
            let node = NodeId(slot as u32 + 1);
            let mut out = job(node, size, &sources[which[slot]], send, mode, Vec::new());
            if slot == 2 && roll % 29 == 0 {
                out.mode = OutputMode::Dark;
                out.shader = None;
                sent[2] = false;
            }
            outputs.push(out);
        }
        if t % 13 == 0 {
            r.request_thumbnail(NodeId(1));
        }
        if t % 17 == 0 {
            r.request_snap(NodeId(2));
        }
        if t % 400 == 100 {
            r.set_capturing(NodeId(1), true, 1, Alpha::Straight);
        }
        if t % 400 == 130 {
            r.set_capturing(NodeId(1), false, 1, Alpha::Straight);
        }
        r.draw(&tick_of(t as f32 * 0.01, outputs));
        r.take_thumbnails();
        r.take_snaps();
        r.take_captured(NodeId(1));
        r.take_taps();
        held.push_back(r.publish());
        // A viewer that sometimes sits on old frames for a while.
        let keep = if (t / 200) % 3 == 0 { 5 } else { 2 };
        while held.len() > keep {
            held.pop_front();
        }
        if t % 500 == 499 {
            counts.push(census(&gpu));
        }
    }
    drop(held);
    println!("{KINDS:?} every 500 ticks: {counts:?}");
    let (first, last) = (counts[1], *counts.last().expect("counted"));
    for (k, kind) in KINDS.iter().enumerate() {
        // Eight objects of slack for a count, an eighth for bytes: the Outputs alive at each
        // count differ, and a staging buffer follows its tap slots.
        let slack = if kind.ends_with("bytes") {
            first[k] / 8
        } else {
            8
        };
        assert!(
            last[k] <= first[k] + slack,
            "{kind} grew from {} to {}",
            first[k],
            last[k]
        );
    }
}

// ------------------------------------------------------------------ a viewer on another thread

/// **A viewer on another thread samples a published frame, and it is the frame it leased.**
/// The synth thread publishes, hands the `Published` to a viewer thread and keeps drawing; the
/// viewer reads what it was handed, again and again, through the one device; no tick draws into
/// that target while the viewer holds it; and every read is the frame the synth published.
/// Nothing orders the two threads but the one queue (`proposals/wgpu.md`, 1.5).
#[test]
fn a_viewer_on_another_thread_samples_the_frame_it_holds_while_the_synth_draws_on() {
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let shader = clock();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(node, (64, 36), &shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.25, outputs(false, DRAW)));
    drain(&gpu);
    let published = renderer.publish();
    let pinned = published.outputs[&node].texture.texture().clone();
    let expected = rgba_of(&gpu, &pinned);
    assert!(expected.iter().all(|p| *p == [64, 128, 128, 255]));

    let (hand, take) = std::sync::mpsc::channel::<Published>();
    let viewer_gpu = gpu.clone();
    let viewer = std::thread::spawn(move || {
        let published = take.recv().expect("a Published");
        let texture = published.outputs[&node].texture.texture().clone();
        // `published` is dropped at the end, after the submissions carrying every read.
        (0..30)
            .map(|_| rgba_of(&viewer_gpu, &texture))
            .collect::<Vec<_>>()
    });
    hand.send(published).expect("the viewer takes it");
    let mut ticks = 0;
    while !viewer.is_finished() {
        ticks += 1;
        renderer.draw(&tick(0.25 + ticks as f32 * 0.1, outputs(false, DRAW)));
        drop(renderer.publish());
        assert_ne!(
            renderer.texture_of(node).as_ref(),
            Some(&pinned),
            "tick {ticks} drew into the target the viewer holds"
        );
    }
    let reads = viewer.join().expect("the viewer thread");
    for (k, read) in reads.iter().enumerate() {
        assert!(read == &expected, "read {k} is not the frame published");
    }
    println!("the synth drew {ticks} ticks while the viewer read");
}
