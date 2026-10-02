// SPDX-License-Identifier: AGPL-3.0-or-later

//! What one tab of a project costs, ticked at a display's rate: the tick rate reached, every
//! drawn Output's draws and drops a second, and the GPU's tick by phase.
//!
//! ```sh
//! cargo run --release --example tick_bench -- <project folder> <workspace name> [hz] [seconds]
//! ```
//!
//! Headless, on the integrated GPU, `render::adapter::Asked::integrated` — never a discrete GPU,
//! never a software adapter, unless `SUPERSILVIA_ADAPTER` names one — and on a copy of the project, since nothing here writes to it but a
//! scratch folder is cheaper than doubt. Every workspace is opened and the named one is looked at, as a person
//! with every tab open would have it; nodes that would open a device or a portal are removed
//! first. The ticks are paced to a deadline carried forward, the synth thread's own loop. The
//! first tick's wall time is printed on its own, before any of it is measured.
//!
//! `SUPERSILVIA_BENCH_EDITOR=<quads>` stands an editor beside it: a thread of its own on the
//! same device and its one queue, as eframe paints on it, painting a 3440x1440 target with that
//! many blended quads each covering it and sampling a 1080p half-float picture, at the same
//! rate and waiting for each, as a window painting to a display does — and what each frame took
//! is reported. A count rather than a time, so every run asks the GPU for the same work
//! whatever clock it is at.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::graph::NodeId;
use supersilvia::project::Active;
use supersilvia::render::Gpu;
use supersilvia::render::GpuPhase;
use supersilvia::synth::meter;
use supersilvia::{App, Command};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("a project folder"));
    let tab = args.next().expect("a workspace name");
    let hz: f64 = args
        .next()
        .map_or(100.0, |a| a.parse().expect("a rate in Hz"));
    let seconds: f64 = args.next().map_or(8.0, |a| a.parse().expect("seconds"));

    let gpu = Gpu::headless(&supersilvia::render::adapter::Asked::integrated())
        .unwrap_or_else(|e| panic!("{e}"));
    println!(
        "adapter: {}",
        supersilvia::render::adapter::describe(&gpu.adapter().get_info())
    );
    // Every editor frame's time, painted and waited for, in milliseconds.
    let painted: Arc<std::sync::Mutex<Vec<f64>>> = Arc::default();
    if let Some(quads) = std::env::var("SUPERSILVIA_BENCH_EDITOR")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        let editor_gpu = gpu.clone();
        let (ready, started) = std::sync::mpsc::channel();
        let painted = Arc::clone(&painted);
        std::thread::spawn(move || editor(&editor_gpu, quads, hz, &ready, &painted));
        let ms: f64 = started.recv().expect("the editor started");
        println!("editor: {quads} quads a frame, {ms:.2} ms alone on the GPU as it clocked then");
    }

    let mut app = App::headless();
    app.attach_gpu_on(gpu.for_synth());
    app.open_project(root);
    let live: Vec<_> = app
        .graph()
        .iter()
        .filter(|(_, n)| n.def.cpu.as_ref().is_some_and(|c| c.live))
        .map(|(id, _)| id)
        .collect();
    if !live.is_empty() {
        app.apply(Command::RemoveNodes(live)).expect("removable");
    }
    let workspaces: Vec<_> = app
        .graph()
        .workspaces()
        .iter()
        .map(|w| (w.id, w.name.clone()))
        .collect();
    for (id, _) in &workspaces {
        app.open_workspace(*id);
    }
    let (looked, _) = workspaces
        .iter()
        .find(|(_, name)| *name == tab)
        .unwrap_or_else(|| panic!("no workspace named {tab:?}"));
    app.activate(Active::Workspace(*looked));
    app.set_show_status_box(true);

    let interval = Duration::from_secs_f64(1.0 / hz);
    let mut deadline = Instant::now();
    // One tick and the sleep to its deadline, returning the tick's wall time and the thread's
    // own CPU time in it: what the wall saw and the CPU did not is the tick waiting.
    let mut tick = |app: &mut App| {
        let (wall, cpu) = (Instant::now(), meter::thread_cpu());
        app.publish_plan();
        app.tick(interval.as_secs_f32());
        let spent = (wall.elapsed(), meter::thread_cpu().saturating_sub(cpu));
        deadline += interval;
        let now = Instant::now();
        match deadline.checked_duration_since(now) {
            Some(rest) => std::thread::sleep(rest),
            None => deadline = now,
        }
        spent
    };

    let (first, _) = tick(&mut app);
    println!("first tick: {:.1} ms", first.as_secs_f64() * 1000.0);

    // Every program linked, and the GPU clocked up under the load, before anything counts.
    let started = Instant::now();
    loop {
        tick(&mut app);
        if app.snapshot().render.linking.is_empty() && started.elapsed().as_secs() >= 5 {
            break;
        }
        assert!(started.elapsed().as_secs() < 120, "never linked");
    }

    let before = app.snapshot().render.clone();
    painted.lock().expect("the editor's times").clear();
    let engine_before = supersilvia::platform::gpu::render_engine_ns();
    // Per Output: its last GPU time, and whether that reading moved while measured.
    let mut read: HashMap<NodeId, (f32, bool)> = HashMap::new();
    let mut ages: HashMap<NodeId, Vec<u64>> = HashMap::new();
    let (mut outputs_ms, mut passes_ms, mut whole_ms, mut samples) =
        (0.0_f64, 0.0_f64, 0.0_f64, 0u32);
    let (mut wall, mut cpu, mut ticks) = (Duration::ZERO, Duration::ZERO, 0u32);
    let window = Instant::now();
    while window.elapsed().as_secs_f64() < seconds {
        let (w, c) = tick(&mut app);
        wall += w;
        cpu += c;
        ticks += 1;
        let snap = app.snapshot();
        for (id, picture) in &snap.published.outputs {
            if let Some(drawn_tick) = picture.drawn_tick {
                ages.entry(*id)
                    .or_default()
                    .push(snap.published.tick.saturating_sub(drawn_tick));
            }
        }
        if let Some(g) = snap.phases.as_ref().and_then(|p| p.gpu.as_ref()) {
            outputs_ms += f64::from(g[GpuPhase::Outputs].now);
            passes_ms += f64::from(g[GpuPhase::Passes].now);
            whole_ms += f64::from(g[GpuPhase::Whole].now);
            samples += 1;
        }
        for (id, t) in &snap.render.gpu_times {
            let entry = read.entry(*id).or_insert((t.latest, false));
            entry.1 |= entry.0 != t.latest;
            entry.0 = t.latest;
        }
    }
    let took = window.elapsed().as_secs_f64();
    let after = app.snapshot().render.clone();
    let busy = (supersilvia::platform::gpu::render_engine_ns() - engine_before) as f64
        / (took * 1e9)
        * 100.0;

    println!(
        "{tab} at {hz} Hz for {took:.1} s: {:.1} ticks/s",
        f64::from(ticks) / took
    );
    // An Output drawn on every tick draws on each it does not drop.
    println!("| Output | GPU ms | draws/s | drops/s | age median/p90/max ticks |");
    println!("| --- | ---: | ---: | ---: | ---: |");
    let graph = app.graph();
    let mut own = 0.0;
    for (id, node) in graph.iter().filter(|(_, n)| n.def.is_output) {
        let dropped = after.dropped_frames.get(&id).copied().unwrap_or(0)
            - before.dropped_frames.get(&id).copied().unwrap_or(0);
        let Some((ms, moved)) = read.get(&id).copied() else {
            continue;
        };
        if !moved && dropped == 0 {
            continue;
        }
        own += ms;
        let upstream = graph
            .cables_into(id)
            .first()
            .and_then(|c| graph.get(c.from.node))
            .map_or("-", |n| n.def.slug);
        let on = node
            .workspaces
            .first()
            .and_then(|w| workspaces.iter().find(|(id, _)| id == w))
            .map_or("?", |(_, name)| name.as_str());
        let age = ages.get_mut(&id).map_or((0, 0, 0), |values| {
            values.sort_unstable();
            let at = |pct: usize| values[(values.len() - 1) * pct / 100];
            (at(50), at(90), at(100))
        });
        println!(
            "| {id} {upstream} ({on}) | {ms:.2} | {:.1} | {:.1} | {}/{}/{} |",
            (f64::from(ticks) - dropped as f64) / took,
            dropped as f64 / took,
            age.0,
            age.1,
            age.2
        );
    }
    let per_s = |a: u64, b: u64| (a - b) as f64 / took;
    println!(
        "drops/s by cause: held {:.1}, capture {:.1}; the draw waited for the GPU on {:.1} \
         ticks/s",
        per_s(after.drops.held, before.drops.held),
        per_s(after.drops.capture, before.drops.capture),
        per_s(after.gpu_waits, before.gpu_waits),
    );
    let n = f64::from(samples.max(1));
    println!(
        "GPU per tick: outputs span {:.2} ms, thumbnails {:.2} ms, whole {:.2} ms; the drawn \
         Outputs' own sum {own:.2} ms; the process {busy:.0} % of the render engine",
        outputs_ms / n,
        passes_ms / n,
        whole_ms / n,
    );
    let frames = std::mem::take(&mut *painted.lock().expect("the editor's times"));
    if !frames.is_empty() {
        let budget = 1000.0 / hz;
        println!(
            "editor: {:.1} frames/s, {:.2} ms a frame on average, {:.2} ms at worst, {} over \
             {budget:.1} ms",
            frames.len() as f64 / took,
            frames.iter().sum::<f64>() / frames.len() as f64,
            frames.iter().copied().fold(0.0, f64::max),
            frames.iter().filter(|f| **f > budget).count(),
        );
    }
    let per_tick = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(ticks.max(1));
    println!(
        "CPU per tick: {:.2} ms of which waiting {:.2} ms",
        per_tick(wall),
        per_tick(wall.saturating_sub(cpu))
    );
}

/// A stand-in for the editor's painting: a 3440x1440 target, cleared and painted with `quads`
/// blended quads each covering it, every one sampling a 1080p half-float picture as a viewer
/// samples an Output's frame, `hz` times a second, on the synth's own device and queue as
/// eframe paints, each frame waited for before the next and its time put in `painted`.
fn editor(
    gpu: &Gpu,
    quads: u32,
    hz: f64,
    started: &std::sync::mpsc::Sender<f64>,
    painted: &std::sync::Mutex<Vec<f64>>,
) {
    const SURFACE: (u32, u32) = (3440, 1440);
    const PICTURE: (u32, u32) = (1920, 1080);
    const SHADER: &str = "
        @group(0) @binding(0) var picture: texture_2d<f32>;
        @group(0) @binding(1) var picture_sampler: sampler;
        struct Varying {
            @builtin(position) position: vec4f,
            @location(0) uv: vec2f,
        }
        @vertex
        fn vs_main(@builtin(vertex_index) i: u32, @builtin(instance_index) q: u32) -> Varying {
            let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
            var out: Varying;
            out.position = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
            out.uv = p * (0.9 + 0.02 * f32(q)) + 0.01 * f32(q);
            return out;
        }
        @fragment
        fn fs_main(v: Varying) -> @location(0) vec4f {
            return vec4f(textureSampleLevel(picture, picture_sampler, v.uv, 0.0).rgb, 0.5);
        }
    ";
    let device = gpu.device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("editor"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("editor"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    });
    // Noise rather than a constant, so nothing samples a cleared texture's fast path.
    let mut seed = 0x2545_f491_u32;
    let texels: Vec<u8> = (0..PICTURE.0 * PICTURE.1 * 4)
        .flat_map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            half((seed >> 8) as f32 / (1 << 24) as f32).to_ne_bytes()
        })
        .collect();
    let picture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size: wgpu::Extent3d {
            width: PICTURE.0,
            height: PICTURE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue().write_texture(
        picture.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(PICTURE.0 * 8),
            rows_per_image: None,
        },
        picture.size(),
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let picture_view = picture.create_view(&wgpu::TextureViewDescriptor::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("editor"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&picture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width: SURFACE.0,
            height: SURFACE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let draw = || {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor"),
        });
        {
            let mut pass = supersilvia::render::shared::begin(
                &mut encoder,
                &view,
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.1,
                    g: 0.1,
                    b: 0.12,
                    a: 1.0,
                }),
                "editor",
            );
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            for q in 0..quads {
                pass.draw(0..3, q..q + 1);
            }
        }
        // Submitted as eframe submits, beside the synth's numbered submissions, and waited for.
        let index = gpu.queue().submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        });
    };
    for _ in 0..20 {
        draw();
    }
    let start = Instant::now();
    draw();
    let _ = started.send(start.elapsed().as_secs_f64() * 1000.0);
    let interval = Duration::from_secs_f64(1.0 / hz);
    let mut deadline = Instant::now();
    loop {
        let start = Instant::now();
        draw();
        if let Ok(mut times) = painted.lock() {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        deadline += interval;
        match deadline.checked_duration_since(Instant::now()) {
            Some(rest) => std::thread::sleep(rest),
            None => deadline = Instant::now(),
        }
    }
}

/// An `f32` in `[0, 1)` as an IEEE half float, rounded toward zero.
fn half(v: f32) -> u16 {
    let bits = v.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        return 0;
    }
    ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
}
