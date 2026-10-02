// SPDX-License-Identifier: AGPL-3.0-or-later

//! The wgpu renderer's GPU timer: an Output's span per frame, the draw's nine marks, and the
//! editor's paint pair, each read back through its ring without a wait
//! (`proposals/wgpu.md`, 1.14).
//!
//! Structure only, never a duration: the iGPU is shared with every other test running, so a
//! span says nothing here. What is held is that readings arrive, one per measured draw or
//! frame, their marks in order, nothing when nothing was asked for, and nothing at all on a
//! device without timestamps. Every test drains the GPU after each draw or frame, so which
//! reading has landed by when is fixed.
//!
//! **One test at a time** ([`one_at_a_time`]): on Metal the process holds at most
//! `timing::QUERY_SETS` query sets, and the test that holds more renderers than Metal has
//! counter sample buffers takes every one, which would leave a test beside it untimed.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, link_all, solid, tick};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use supersilvia::graph::NodeId;
use supersilvia::render::GpuPhase;
use supersilvia::render::timing::{self, PaintTimer};
use supersilvia::render::{FrameJob, Gpu, OutputJob, OutputMode, Renderer};

const DRAW: OutputMode = OutputMode::Draw;

/// How many draws a test measures.
const MEASURED: usize = 6;

/// How many editor frames a test paints and reads.
const FRAMES: usize = 5;

/// A frame's paint pair is resolved in the next frame and its map asked for in the one after,
/// so it is read in the third frame after its own.
const LAG: usize = 3;

/// Metal's limit on the counter sample buffers one process holds, across every device.
const METAL_SAMPLE_BUFFERS: usize = 32;

/// Held for a whole test, so no two run at once.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A flag the device raises if it is lost.
fn watch_for_loss(gpu: &Gpu) -> Arc<AtomicBool> {
    let lost = Arc::new(AtomicBool::new(false));
    let raised = Arc::clone(&lost);
    gpu.device()
        .set_device_lost_callback(move |_, _| raised.store(true, Ordering::Release));
    lost
}

/// The query sets `gpu`'s device holds now.
fn query_sets(gpu: &Gpu) -> isize {
    gpu.device().get_internal_counters().hal.query_sets.read()
}

fn timestamps(gpu: &Gpu) -> bool {
    gpu.device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
}

fn metal(gpu: &Gpu) -> bool {
    gpu.adapter().get_info().backend == wgpu::Backend::Metal
}

/// Whether the draw's marks are written: Metal writes no timestamp for an empty pass.
fn marks(gpu: &Gpu) -> bool {
    timestamps(gpu) && !metal(gpu)
}

/// The serial the next submission on `gpu` would take, found by submitting nothing.
fn serial(gpu: &Gpu) -> u64 {
    gpu.submit(std::iter::empty()).serial
}

/// A renderer with one Output drawing a solid colour, linked and undrawn.
fn one_output(gpu: &Gpu) -> (Renderer, impl Fn(OutputMode) -> FrameJob) {
    let out = NodeId(1);
    let shader = solid(0.25, 0.5, 0.75);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = move |send: bool, mode| vec![job(out, (64, 64), &shader, send, mode, vec![])];
    link_all(&mut renderer, &outputs);
    (renderer, move |mode| tick(0.0, outputs(false, mode)))
}

/// **An Output's frames are timed**: a reading once a frame has landed, its worst never below
/// its latest; gone once the Output goes dark, and absent throughout on a device without
/// timestamps.
#[test]
fn an_outputs_frames_are_timed_until_it_goes_dark() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    let (mut renderer, frame) = one_output(&gpu);
    let out = NodeId(1);
    assert_eq!(renderer.gpu_time(out), None, "nothing drawn, nothing read");
    for _ in 0..4 {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
    }
    let reading = renderer.gpu_time(out);
    if timestamps(&gpu) {
        let time = reading.expect("a landed frame has a span");
        assert!(time.latest.is_finite() && time.latest >= 0.0, "{time:?}");
        assert!(time.worst >= time.latest, "{time:?}");
    } else {
        assert_eq!(reading, None, "no timestamps, no line — never a zero");
    }

    renderer.draw(&frame(OutputMode::Dark));
    assert_eq!(
        renderer.gpu_time(out),
        None,
        "a dark Output reports no cost"
    );
    for _ in 0..3 {
        renderer.draw(&frame(OutputMode::Dark));
        drain(&gpu);
    }
    assert_eq!(
        renderer.gpu_time(out),
        None,
        "frames in flight when it went dark are let go unread"
    );
}

/// **Every measured draw gives one reading of its phases**, oldest first, each with its marks
/// in order — a set whose marks ran backwards gives none — and the whole never shorter than any
/// part. A draw placed before measuring began, or after it stopped, gives none, and the sets in
/// flight when it stopped still land. On Metal, whose empty passes write no timestamp, there is
/// no reading at all — never one of zeros.
#[test]
fn every_measured_draw_gives_one_reading_of_its_phases() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    let (mut renderer, frame) = one_output(&gpu);
    for _ in 0..3 {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
    }
    assert!(renderer.take_gpu_spans().is_empty(), "nothing was measured");

    renderer.set_measuring(true);
    let mut spans = Vec::new();
    for _ in 0..MEASURED {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
        spans.extend(renderer.take_gpu_spans());
    }
    renderer.set_measuring(false);
    for _ in 0..3 {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
        spans.extend(renderer.take_gpu_spans());
    }

    if !marks(&gpu) {
        assert!(
            spans.is_empty(),
            "no marks written, no breakdown — never a zero: {spans:?}"
        );
        return;
    }
    assert_eq!(
        spans.len(),
        MEASURED,
        "one reading per measured draw, and none for any other"
    );
    for s in &spans {
        assert!(s[GpuPhase::Whole] > 0.0, "the marks were written: {s:?}");
        for phase in GpuPhase::ALL {
            assert!(
                s[phase].is_finite() && s[phase] >= 0.0,
                "{phase:?} in {s:?}"
            );
            assert!(s[GpuPhase::Whole] >= s[phase], "{phase:?} in {s:?}");
        }
    }
    for _ in 0..3 {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
    }
    assert!(
        renderer.take_gpu_spans().is_empty(),
        "each reading is taken once"
    );
}

/// **A mark not placed asks nothing of its recording**: a draw with nothing to do submits
/// nothing while unmeasured, and measured it submits one submission, the prelude and the coda
/// that carry the marks going in together with no Output between them.
#[test]
fn an_empty_draw_submits_only_what_its_marks_need() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let empty = FrameJob::default();
    renderer.draw(&empty);
    drain(&gpu);

    let before = serial(&gpu);
    renderer.draw(&empty);
    let after = serial(&gpu);
    assert_eq!(
        after - before,
        1,
        "an unmeasured empty draw submits nothing"
    );

    renderer.set_measuring(true);
    let before = serial(&gpu);
    renderer.draw(&empty);
    let after = serial(&gpu);
    let marks = u64::from(timestamps(&gpu));
    assert_eq!(
        after - before,
        1 + marks,
        "a measured one submits the prelude and the coda together, where it can mark"
    );
    drain(&gpu);
}

/// One editor frame into a small target: the start ahead of the pass, the end inside it.
fn paint(gpu: &Gpu, timer: &PaintTimer, target: &wgpu::TextureView) {
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor"),
        });
    timer.start(&mut encoder);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("editor"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        timer.end(&mut pass);
    }
    gpu.submit([encoder.finish()]);
    drain(gpu);
}

/// **The editor's painting is timed frame by frame**: one span per frame once its pair has
/// landed, each taken once; forgotten pairs are never read, and marking goes on after them.
/// Absent where the device cannot write a timestamp inside a pass.
#[test]
fn the_editors_painting_is_timed_frame_by_frame() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    let inside = gpu
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
    let Some(timer) = PaintTimer::new(&gpu) else {
        assert!(!inside, "a device with both features has a paint timer");
        return;
    };
    let target = gpu
        .device()
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("editor"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());

    let mut spans = Vec::new();
    for _ in 0..FRAMES + LAG {
        paint(&gpu, &timer, &target);
        spans.extend(timer.take());
    }
    assert_eq!(spans.len(), FRAMES, "one span per frame read so far");
    assert!(
        spans.iter().all(|s| s.is_finite() && *s >= 0.0),
        "{spans:?}"
    );
    assert!(timer.take().is_empty(), "each span is taken once");

    timer.forget();
    assert!(timer.take().is_empty());
    for _ in 0..LAG {
        paint(&gpu, &timer, &target);
    }
    assert!(
        timer.take().is_empty(),
        "no pair marked before the forget is read after it"
    );
    paint(&gpu, &timer, &target);
    assert_eq!(timer.take().len(), 1, "marking goes on after a forget");
}

/// **Every Output of a renderer is timed out of its one query set**: more Outputs than Metal
/// has counter sample buffers draw measured, the device stays whole, and each has a span.
#[test]
fn many_outputs_are_timed_out_of_one_query_set() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    let lost = watch_for_loss(&gpu);
    let shader = solid(0.5, 0.25, 0.125);
    let count = METAL_SAMPLE_BUFFERS as u32 + 8;
    let outputs = |send: bool, mode| -> Vec<OutputJob> {
        (1..=count)
            .map(|i| job(NodeId(i), (8, 8), &shader, send, mode, vec![]))
            .collect()
    };
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &outputs);
    renderer.set_measuring(true);
    let mut phases = 0;
    for _ in 0..4 {
        renderer.draw(&tick(0.0, outputs(false, DRAW)));
        drain(&gpu);
        phases += renderer.take_gpu_spans().len();
    }
    assert!(!lost.load(Ordering::Acquire), "the device is lost");
    assert!(
        query_sets(&gpu) <= 1,
        "one query set for the renderer, however many Outputs: {}",
        query_sets(&gpu)
    );
    assert_eq!(phases > 0, marks(&gpu), "the draws' phases are read");
    if timestamps(&gpu) {
        for i in 1..=count {
            assert!(
                renderer.gpu_time(NodeId(i)).is_some(),
                "Output {i} of {count} has a span"
            );
        }
    }
}

/// **More renderers than Metal has counter sample buffers never lose the device**: past
/// `timing::QUERY_SETS` a renderer is made untimed — no span and no breakdown, never a zero —
/// and draws as any other, and the sets come back once the renderers holding them are dropped.
/// Metal's alone: no other backend is held to the budget.
#[test]
fn renderers_past_the_query_set_budget_draw_untimed() {
    let _one = one_at_a_time();
    let gpu = gpu::gpu();
    if !metal(&gpu) {
        eprintln!("not Metal: no query set budget to hold; skipping");
        return;
    }
    let lost = watch_for_loss(&gpu);
    let before = timing::live_query_sets();
    let mut renderers: Vec<_> = (0..=METAL_SAMPLE_BUFFERS)
        .map(|_| one_output(&gpu))
        .collect();
    assert!(
        timing::live_query_sets() <= timing::QUERY_SETS,
        "{} query sets live",
        timing::live_query_sets()
    );
    let out = NodeId(1);
    let mut timed = 0;
    for (renderer, frame) in &mut renderers {
        renderer.set_measuring(true);
        let mut phases = 0;
        for _ in 0..4 {
            renderer.draw(&frame(DRAW));
            drain(&gpu);
            phases += renderer.take_gpu_spans().len();
        }
        match renderer.gpu_time(out) {
            Some(_) => {
                assert_eq!(phases > 0, marks(&gpu), "a timed renderer reads its phases");
                timed += 1;
            }
            None => assert_eq!(phases, 0, "an untimed renderer has no breakdown"),
        }
    }
    assert!(!lost.load(Ordering::Acquire), "the device is lost");
    assert!(timed <= timing::QUERY_SETS, "{timed} renderers timed");
    if timestamps(&gpu) {
        assert!(timed > 0, "the renderers within the budget are timed");
    }
    assert!(
        query_sets(&gpu) <= timing::QUERY_SETS as isize,
        "{} query sets on the device",
        query_sets(&gpu)
    );

    drop(renderers);
    drain(&gpu);
    assert_eq!(timing::live_query_sets(), before, "every set is given back");
    let (mut renderer, frame) = one_output(&gpu);
    for _ in 0..4 {
        renderer.draw(&frame(DRAW));
        drain(&gpu);
    }
    assert_eq!(
        renderer.gpu_time(out).is_some(),
        timestamps(&gpu),
        "a renderer made after them is timed again"
    );
}
