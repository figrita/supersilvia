// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 3: a picture only ever holds its own node's pixels.
//!
//! Two CPU nodes publish pictures that cannot be mistaken for each other — the brick game's is
//! grey, the automaton's has no blue in it, and the solid frames standing in for them are grey
//! and red — and every place either picture lands is read back: the source texture itself,
//! the Output drawing it, and a viewer on another thread showing both beside every Output. A
//! texel of one in the other is the bug. What was seen in a long session was specks at the
//! top-left of the brick game's texture, preview and Output, and a red bar along the top of
//! the automaton's preview; each shape below is one that session had: the synth's renderer on
//! a thread of its own with the editor's viewer churning textures and buffers as egui does,
//! Outputs that flip between two inputs, a project opened over another whose ids name other
//! kinds, and a tab closed and opened again. On wgpu every one of them is on one device and
//! its one queue. See [docs/rendering.md](../docs/rendering.md#every-window-is-a-viewer).

mod common;
#[path = "common/gpu.rs"]
mod gpu;

use common::{add_on, connect};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::Duration;
use supersilvia::compile::Shader;
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId};
use supersilvia::nodes::Frame;
use supersilvia::project::Active;
use supersilvia::render::Fit;
use supersilvia::render::viewer::Viewport;
use supersilvia::render::{
    FrameJob, Gpu, OutputJob, OutputMode, Published, Renderer, SourceJob, UniformValue, Viewer,
    readback,
};
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

/// Ticks the threaded tests run for.
const TICKS: u32 = 120;

/// The side of one picture in a viewer's capture, which holds four across and three down.
const TILE: i32 = 200;

/// An Output latching red wherever its grey source ever held a texel that is not grey. Its
/// own last frame is `u_self`.
const WATCH_GREY: &str = "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let s = textureSampleLevel(u_src, sampler_mirror_linear, uv, 0.0);
    let wrong = select(0.0, 1.0, abs(s.r - s.g) > 0.02 || abs(s.g - s.b) > 0.02);
    let before = textureSampleLevel(u_self, sampler_mirror_linear, uv, 0.0).r;
    return vec4f(max(before, wrong), 1.0, 0.0, 1.0);
}
";
/// An Output latching red wherever its red source ever held a texel that is not red.
const WATCH_RED: &str = "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let s = textureSampleLevel(u_src, sampler_mirror_linear, uv, 0.0);
    let wrong = select(0.0, 1.0, s.r < 0.95 || s.g > 0.05);
    let before = textureSampleLevel(u_self, sampler_mirror_linear, uv, 0.0).r;
    return vec4f(max(before, wrong), 1.0, 0.0, 1.0);
}
";
/// An Output that is red on even `u_n` and green on odd.
const FLIP: &str = "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    if u.u_n % 2.0 < 1.0 {
        return vec4f(1.0, 0.0, 0.0, 1.0);
    }
    return vec4f(0.0, 1.0, 0.0, 1.0);
}
";

/// A target of four tiles across and three down: where a viewer draws.
fn capture(gpu: &Gpu) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("capture"),
        size: wgpu::Extent3d {
            width: (4 * TILE) as u32,
            height: (3 * TILE) as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// The two tiles along the top of the capture, read back and waited for, rows top first.
fn top_pair(gpu: &Gpu, capture: &wgpu::Texture) -> Vec<u8> {
    let whole = gpu::bytes_of(gpu, capture);
    let row = (4 * TILE * 4) as usize;
    let pair = (2 * TILE * 4) as usize;
    (0..TILE as usize)
        .flat_map(|r| whole[r * row..r * row + pair].to_vec())
        .collect()
}

/// Tile `i` of the capture, four across, from the top.
fn tile(i: i32) -> Viewport {
    Viewport {
        left: (i % 4) * TILE,
        top: (i / 4) * TILE,
        width: TILE,
        height: TILE,
    }
}

/// Every picture in `published`, in one submission: the two sources on tiles 0 and 1, then
/// each Output.
fn show_all(
    gpu: &Gpu,
    viewer: &Viewer,
    target: &wgpu::TextureView,
    published: &Published,
    a: PortRef,
    b: PortRef,
) {
    let size = ((4 * TILE) as u32, (3 * TILE) as u32);
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewer"),
        });
    supersilvia::render::shared::clear(&mut encoder, target, wgpu::Color::BLACK);
    viewer.show(
        &mut encoder,
        target,
        size,
        &published.sources[&a],
        tile(0),
        Fit::Cover,
        0.0,
    );
    viewer.show(
        &mut encoder,
        target,
        size,
        &published.sources[&b],
        tile(1),
        Fit::Cover,
        0.0,
    );
    for (i, picture) in (2..).zip(published.outputs.values()) {
        viewer.show(
            &mut encoder,
            target,
            size,
            picture,
            tile(i),
            Fit::Letterbox,
            0.0,
        );
    }
    gpu.submit([encoder.finish()]);
}

/// Pixels whose channels differ by more than a rounding.
fn not_grey(px: &[u8]) -> usize {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0].abs_diff(p[1]) > 2 || p[1].abs_diff(p[2]) > 2)
        .count()
}

/// An Output job whose module is sent on the first tick only.
fn output(
    node: u32,
    shader: &Arc<Shader>,
    first: bool,
    uniforms: Vec<(&str, UniformValue)>,
) -> OutputJob {
    OutputJob {
        node: NodeId(node),
        resolution: (1280, 720),
        shader: first.then(|| Arc::clone(shader)),
        source: supersilvia::compile::source_hash(&shader.body),
        mode: OutputMode::Draw,
        uniforms: uniforms
            .into_iter()
            .map(|(k, v)| (Arc::from(k), v))
            .collect(),
        taps: 0,
        clear: false,
    }
}

/// Churn the device the way egui does between the pictures it shows: a texture made, filled
/// and let go, and streamed vertex and index buffers of a size that changes.
fn churn(gpu: &Gpu, n: u32) {
    let device = gpu.device();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("churn"),
        size: wgpu::Extent3d {
            width: 32,
            height: 32,
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
        texture.as_image_copy(),
        &[7u8; 32 * 32 * 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(32 * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    for usage in [wgpu::BufferUsages::VERTEX, wgpu::BufferUsages::INDEX] {
        let size = 64 * 1024 + u64::from(n % 7) * 4096;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("churn"),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue()
            .write_buffer(&buffer, 0, &vec![1u8; size as usize]);
    }
}

/// The live shape at the renderer: the synth's renderer on a thread of its own on the one
/// device, two sources of other sizes and colors re-uploaded every tick, two Outputs watching
/// them and two flipping, and the editor's viewer on this thread making, filling and freeing
/// textures and streaming buffers between the pictures it shows.
#[test]
fn a_threaded_renderer_keeps_each_source_its_own() {
    let gpu = gpu::gpu();
    let (tx, rx) = sync_channel::<Box<Published>>(1);
    let a = PortRef::new(NodeId(1), "field");
    let b = PortRef::new(NodeId(2), "cells");
    let grey = |n: u32| [(n % 200) as u8, (n % 200) as u8, (n % 200) as u8, 255];
    let red = |n: u32| [250, 3, (n % 250) as u8, 255];
    let watch_grey = gpu::module(
        &[],
        &[("u_self", NodeId(11)), ("u_src", NodeId(1))],
        WATCH_GREY,
    );
    let watch_red = gpu::module(
        &[],
        &[("u_self", NodeId(12)), ("u_src", NodeId(2))],
        WATCH_RED,
    );
    let flip = gpu::module(&["u_n"], &[], FLIP);

    let synth_gpu = gpu.for_synth();
    let synth = std::thread::spawn(move || {
        let mut r = Renderer::new(synth_gpu.clone()).expect("a renderer");
        let mut wrong = Vec::new();
        for n in 0..TICKS {
            let first = n == 0;
            let own = |node| UniformValue::NodeTexture(PortRef::new(NodeId(node), "frame"));
            let job = FrameJob {
                time: n as f32,
                outputs: vec![
                    output(
                        11,
                        &watch_grey,
                        first,
                        vec![("u_src", UniformValue::NodeTexture(a)), ("u_self", own(11))],
                    ),
                    output(
                        12,
                        &watch_red,
                        first,
                        vec![("u_src", UniformValue::NodeTexture(b)), ("u_self", own(12))],
                    ),
                    output(
                        13,
                        &flip,
                        first,
                        vec![("u_n", UniformValue::Float((n / 7) as f32))],
                    ),
                    output(
                        14,
                        &flip,
                        first,
                        vec![("u_n", UniformValue::Float((n / 5) as f32))],
                    ),
                ],
                sources: vec![
                    SourceJob::new(a, Arc::new(Frame::solid(300, 300, grey(n)))),
                    SourceJob::new(b, Arc::new(Frame::solid(64, 64, red(n)))),
                ],
                ..FrameJob::default()
            };
            r.draw(&job);
            let published = r.publish();
            let (ta, tb) = (
                published.sources[&a].texture.texture().clone(),
                published.sources[&b].texture.texture().clone(),
            );
            let _ = tx.try_send(Box::new(published));
            let pa = gpu::bytes_of(&synth_gpu, &ta);
            let pb = gpu::bytes_of(&synth_gpu, &tb);
            let wrong_a = pa
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| **p != grey(n))
                .count();
            let wrong_b = pb
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| **p != red(n))
                .count();
            if wrong_a + wrong_b > 0 {
                wrong.push(format!(
                    "tick {n}: {wrong_a} wrong in the grey source, {wrong_b} in the red"
                ));
            }
        }
        drop(tx);
        for node in [11, 12] {
            let (_, _, halves) = r.read_output(NodeId(node)).expect("drawn");
            let px = readback::half_to_rgba8(&halves);
            let latched = px.as_chunks::<4>().0.iter().filter(|p| p[0] > 128).count();
            if latched > 0 {
                wrong.push(format!(
                    "Output {node}: {latched} pixels saw another source's texel"
                ));
            }
        }
        wrong
    });

    let viewer = Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("a viewer");
    let (target, view) = capture(&gpu);
    let mut wrong = Vec::new();
    let mut held: Option<Box<Published>> = None;
    let mut n = 0;
    loop {
        match rx.recv_timeout(Duration::from_millis(3)) {
            Ok(p) => held = Some(p),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let Some(published) = &held else { continue };
        n += 1;
        churn(&gpu, n);
        show_all(&gpu, &viewer, &view, published, a, b);
        let pair = top_pair(&gpu, &target);
        let (mut wa, mut wb) = (0, 0);
        for (k, p) in pair.as_chunks::<4>().0.iter().enumerate() {
            if (k as i32) % (2 * TILE) < TILE {
                wa += usize::from(p[0].abs_diff(p[1]) > 2 || p[1].abs_diff(p[2]) > 2);
            } else {
                wb += usize::from(p[0] < 240 || p[1] > 12);
            }
        }
        if wa + wb > 0 {
            wrong.push(format!(
                "editor: {wa} wrong in the grey preview, {wb} in the red"
            ));
        }
    }
    drop(held);
    wrong.extend(synth.join().expect("the synth's thread"));
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Turn the brick game's auto-play and auto-reset on, so it plays and starts over unattended.
fn autoplay(app: &mut App, bricks: NodeId) {
    for key in ["autoPlay", "autoReset"] {
        app.apply(Command::SetControl {
            node: bricks,
            key,
            value: ControlValue::Float(1.0),
        })
        .expect("a number control");
    }
}

/// The brick game and the automaton each into an Output, and two Outputs flipping between a
/// multiplexer's inputs, through the whole synth, with an editor's viewer on a thread of its
/// own showing both previews and every Output.
#[test]
fn the_brick_game_and_the_automaton_keep_their_own_pixels() {
    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.for_synth());
    let ws = app.graph().default_workspace();
    let bricks = add_on(&mut app, "brickgame", ws);
    let out_b = add_on(&mut app, "output", ws);
    connect(&mut app, (bricks, "field"), (out_b, "input"));
    let ca = add_on(&mut app, "cellularautomata", ws);
    let out_c = add_on(&mut app, "output", ws);
    connect(&mut app, (ca, "cells"), (out_c, "input"));
    for hz in [2.0, 3.0] {
        let mux = add_on(&mut app, "muxnumber", ws);
        let osc = add_on(&mut app, "oscillator", ws);
        let out = add_on(&mut app, "output", ws);
        // Its frequency is a gear's: a Ratio Gear on ambient seconds, into its Time.
        let gear = add_on(&mut app, "ratiogear", ws);
        for (node, key, value) in [(gear, "p", hz), (osc, "amplitude", 2.0)] {
            app.apply(Command::SetControl {
                node,
                key,
                value: ControlValue::Float(value),
            })
            .expect("a number control");
        }
        connect(&mut app, (gear, "cycles"), (osc, "clock"));
        connect(&mut app, (osc, "output"), (mux, "select"));
        connect(&mut app, (mux, "output"), (out, "input"));
    }
    autoplay(&mut app, bricks);
    let (bport, cport) = (PortRef::new(bricks, "field"), PortRef::new(ca, "cells"));

    let (tx, rx) = sync_channel::<Box<Published>>(1);
    let editor_gpu = gpu.clone();
    let editor = std::thread::spawn(move || {
        let gpu = editor_gpu;
        let viewer = Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("a viewer");
        let (target, view) = capture(&gpu);
        let mut wrong = Vec::new();
        let mut held: Option<Box<Published>> = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(3)) {
                Ok(p) => held = Some(p),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let Some(published) = &held else { continue };
            if !(published.sources.contains_key(&bport) && published.sources.contains_key(&cport)) {
                continue;
            }
            show_all(&gpu, &viewer, &view, published, bport, cport);
            let pair = top_pair(&gpu, &target);
            let (mut wb, mut wc) = (0, 0);
            for (k, p) in pair.as_chunks::<4>().0.iter().enumerate() {
                if (k as i32) % (2 * TILE) < TILE {
                    wb += usize::from(p[0].abs_diff(p[1]) > 2 || p[1].abs_diff(p[2]) > 2);
                } else {
                    wc += usize::from(p[2] > 8);
                }
            }
            if wb + wc > 0 {
                wrong.push(format!(
                    "editor: {wb} wrong in the brick game's preview, {wc} in the automaton's"
                ));
            }
        }
        drop(held);
        wrong
    });

    let mut wrong = Vec::new();
    for tick in 0..TICKS {
        app.publish_plan();
        app.tick(FRAME);
        if let Some(synth) = app.ticked() {
            let _ = tx.try_send(Box::new(synth.published().clone()));
        }
        if tick % 10 == 0
            && let Some((_, _, px)) = app.read_output(out_b)
            && not_grey(&px) > 0
        {
            wrong.push(format!(
                "tick {tick}: {} not grey in the brick game's Output",
                not_grey(&px)
            ));
        }
    }
    drop(tx);
    wrong.extend(editor.join().expect("the editor's thread"));
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A scratch directory of its own for one test's project.
fn scratch(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "supersilvia-crosstalk-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    root
}

/// A project on disk holding a brick game and an automaton, each into an Output, the first
/// pair added first. Its root, and the brick game's node and Output.
fn saved_pair(name: &str, bricks_first: bool) -> (std::path::PathBuf, NodeId, NodeId) {
    let root = scratch(name);
    let mut app = App::headless();
    app.new_project(root.clone());
    let ws = app.graph().default_workspace();
    let mut pairs = [("brickgame", "field"), ("cellularautomata", "cells")];
    if !bricks_first {
        pairs.reverse();
    }
    let mut bricks = None;
    for (slug, port) in pairs {
        let node = add_on(&mut app, slug, ws);
        let out = add_on(&mut app, "output", ws);
        connect(&mut app, (node, port), (out, "input"));
        if slug == "brickgame" {
            bricks = Some((node, out));
        }
    }
    app.save_project().expect("the project saves");
    let (node, out) = bricks.expect("a brick game");
    (root, node, out)
}

/// A project opened in the app another ran in, where the ids the first gave the automaton
/// are the brick game's in the second.
#[test]
fn a_project_opened_over_another_keeps_each_source_its_own() {
    let (first, ..) = saved_pair("first", false);
    let (second, bricks, out_b) = saved_pair("second", true);

    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.clone());
    app.open_project(first);
    for _ in 0..30 {
        app.publish_plan();
        app.tick(FRAME);
    }
    app.open_project(second);
    let bport = PortRef::new(bricks, "field");
    let mut wrong = Vec::new();
    for tick in 0..60 {
        app.publish_plan();
        app.tick(FRAME);
        let published = app.ticked().expect("inline").published();
        let Some(picture) = published.sources.get(&bport).cloned() else {
            continue;
        };
        let source = not_grey(&gpu::bytes_of(&gpu, picture.texture.texture()));
        let out = app.read_output(out_b).map_or(0, |(_, _, px)| not_grey(&px));
        if source + out > 0 {
            wrong.push(format!(
                "tick {tick}: {source} not grey in the brick game's texture, {out} in its Output"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The brick game on a tab of its own, closed and opened again while the automaton's tab runs.
#[test]
fn a_tab_opened_again_draws_its_own_source() {
    let (mut app, first, second): (App, WorkspaceId, WorkspaceId) = common::two_tabs();
    app.attach_gpu_on(gpu::gpu());
    let ca = add_on(&mut app, "cellularautomata", first);
    let out_c = add_on(&mut app, "output", first);
    connect(&mut app, (ca, "cells"), (out_c, "input"));
    let bricks = add_on(&mut app, "brickgame", second);
    let out_b = add_on(&mut app, "output", second);
    connect(&mut app, (bricks, "field"), (out_b, "input"));
    autoplay(&mut app, bricks);
    let mut wrong = Vec::new();
    for round in 0..4 {
        if round % 2 == 0 {
            app.close_workspace(second);
            app.activate(Active::Workspace(first));
        } else {
            app.open_workspace(second);
            app.activate(Active::Workspace(second));
        }
        for tick in 0..15 {
            app.publish_plan();
            app.tick(FRAME);
            if let Some((_, _, px)) = app.read_output(out_b)
                && not_grey(&px) > 0
            {
                wrong.push(format!(
                    "round {round} tick {tick}: {} not grey in the brick game's Output",
                    not_grey(&px)
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
