// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the renderer reads back off the GPU — tap words, a thumbnail, a Snap and a render's
//! capture — through the `Renderer` and with real tap nodes compiled by `compile::wgsl::build`. Every read is a
//! staging buffer and a map collected on a later tick, so each test asks for a reading, draws,
//! and finds it on the draw after (`proposals/wgpu.md`, 1.6).

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, link_all, module, tick};
use std::sync::Arc;
use supersilvia::compile::{self, Shader, TAP_TEMPLATE, TAP_WORDS, TapKind, UniformProvider};
use supersilvia::graph::{ControlValue, Graph, NodeId, PortRef};
use supersilvia::nodes::{self, autoexposure, sample, tap};
use supersilvia::render::readback::{Alpha, THUMBNAIL, THUMBNAIL_BYTES};
use supersilvia::render::{
    FrameJob, OutputJob, OutputMode, PassJob, PassKey, Published, Renderer, UniformValue,
};

const DRAW: OutputMode = OutputMode::Draw;

// ------------------------------------------------------------------------------------ taps

/// What the pass measuring `g`'s taps read back after `frames` ticks, `out` drawn at `size`
/// beside it. See `gpu::pass_words`.
fn tap_words(g: &Graph, out: NodeId, frames: usize, size: (u32, u32)) -> Vec<u32> {
    gpu::pass_words(g, out, frames, size)
}

/// A grid of 64 keeps the pass these draw small; the unit square is measured whatever it is.
fn set_grid(g: &mut Graph, t: NodeId, grid: &str) {
    g.get_mut(t)
        .expect("the tap is in the graph")
        .options
        .insert("grid", grid.to_string());
}

fn set(g: &mut Graph, node: NodeId, key: &'static str, value: f32) {
    g.get_mut(node)
        .expect("in the graph")
        .controls
        .insert(key, ControlValue::Float(value));
}

/// `input` into a `tap`, the tap into an Output: the graph, the tap and the Output.
fn tapped(g: &mut Graph, input: PortRef) -> (NodeId, NodeId) {
    let t = nodes::add_to_graph(g, "tap", emath::Pos2::ZERO).unwrap();
    set_grid(g, t, "64");
    let out = nodes::add_to_graph(g, "output", emath::Pos2::ZERO).unwrap();
    g.connect(input, PortRef::new(t, "input")).unwrap();
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();
    (t, out)
}

fn stats(words: Vec<u32>) -> tap::Stats {
    assert_eq!(words.len(), TAP_WORDS, "one slot");
    let slot: [u32; TAP_WORDS] = words.try_into().unwrap();
    tap::decode(&slot)
}

/// A tap on a solid gray measures exactly that gray: every one of the 4096 points added the
/// same luminance, and the extremes are that luminance to the bit.
#[test]
fn a_tap_on_solid_gray_reads_the_gray_exactly() {
    let mut g = Graph::new();
    let gray = nodes::add_to_graph(&mut g, "rgba", emath::Pos2::ZERO).unwrap();
    for key in ["r", "g", "b"] {
        set(&mut g, gray, key, 0.5);
    }
    let (_, out) = tapped(&mut g, PortRef::new(gray, "output"));
    let s = stats(tap_words(&g, out, 2, (64, 64)));
    assert_eq!(s.count, 64 * 64, "every fragment counted once");
    assert!((s.mean - 0.5).abs() < 1e-3, "mean {}", s.mean);
    assert_eq!(s.max, 0.5, "max is exact");
    assert_eq!(s.min, 0.5, "min is exact");
    assert!(
        s.x.abs() < 0.05 && s.y.abs() < 0.05,
        "centroid ({}, {})",
        s.x,
        s.y
    );
}

/// A checkerboard's mean is a half and its extremes are the two colors.
#[test]
fn a_tap_on_a_checkerboard_reads_half_and_both_extremes() {
    let mut g = Graph::new();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", emath::Pos2::ZERO).unwrap();
    let (_, out) = tapped(&mut g, PortRef::new(cb, "output"));
    let s = stats(tap_words(&g, out, 2, (64, 64)));
    assert_eq!(s.count, 64 * 64);
    assert!((s.mean - 0.5).abs() < 1e-3, "mean {}", s.mean);
    assert_eq!(s.max, 1.0);
    assert_eq!(s.min, 0.0);
}

/// The picker decides which channel the tap reduces, each exact on a solid color.
#[test]
fn a_tap_measures_the_channel_its_picker_names() {
    let mut g = Graph::new();
    let solid = nodes::add_to_graph(&mut g, "rgba", emath::Pos2::ZERO).unwrap();
    for (key, value) in [("r", 0.25f32), ("g", 0.5), ("b", 0.75), ("a", 0.125)] {
        set(&mut g, solid, key, value);
    }
    let (t, out) = tapped(&mut g, PortRef::new(solid, "output"));
    for (which, expected) in [("red", 0.25f32), ("blue", 0.75), ("alpha", 0.125)] {
        g.get_mut(t)
            .unwrap()
            .options
            .insert("measure", which.to_string());
        let s = stats(tap_words(&g, out, 2, (64, 64)));
        assert_eq!(s.count, 64 * 64, "{which}: every fragment counted once");
        assert_eq!(s.mean, expected, "{which}: the channel, exactly");
        assert_eq!(s.max, expected, "{which}: the extremes are exact");
        assert_eq!(s.min, expected, "{which}: the extremes are exact");
    }
}

/// The finest grid is 65,536 points, each adding more than half the low word, so the sums
/// carry into the high word tens of thousands of times — and the mean is still the level
/// drawn. A full HD target has a fragment for every cell.
#[test]
fn a_taps_sums_carry_across_the_finest_grid() {
    let mut g = Graph::new();
    let gray = nodes::add_to_graph(&mut g, "rgba", emath::Pos2::ZERO).unwrap();
    for key in ["r", "g", "b"] {
        set(&mut g, gray, key, 0.51);
    }
    let (t, out) = tapped(&mut g, PortRef::new(gray, "output"));
    set_grid(&mut g, t, "256");
    let s = stats(tap_words(&g, out, 2, (1920, 1080)));
    assert_eq!(s.count, 256 * 256, "every cell of the grid, once");
    assert!(
        (s.mean - 0.51).abs() <= 1.0 / tap::SCALE,
        "mean {} is 0.51 to 1/{}",
        s.mean,
        tap::SCALE
    );
    assert_eq!(s.max, s.min, "a flat field has one value");
}

/// A sample at a point reads the color there, the checkerboard's control color exactly, and
/// the point moves with its controls.
#[test]
fn a_sample_reads_the_color_at_its_point() {
    let mut g = Graph::new();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", emath::Pos2::ZERO).unwrap();
    let s = nodes::add_to_graph(&mut g, "sample", emath::Pos2::ZERO).unwrap();
    let out = nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO).unwrap();
    g.connect(PortRef::new(cb, "output"), PortRef::new(s, "input"))
        .unwrap();
    g.connect(PortRef::new(s, "output"), PortRef::new(out, "input"))
        .unwrap();
    // Frequency 8 over a square: two points in cells of opposite color.
    let mut colors = Vec::new();
    for (x, y) in [(0.125, 0.125), (0.375, 0.125)] {
        set(&mut g, s, "x", x);
        set(&mut g, s, "y", y);
        let slot: [u32; TAP_WORDS] = tap_words(&g, out, 2, (64, 64)).try_into().unwrap();
        let c = sample::decode(&slot).expect("a fragment landed on the point");
        assert!(
            c == [1.0, 1.0, 1.0, 1.0] || c == [0.0, 0.0, 0.0, 1.0],
            "a control color exactly, got {c:?}"
        );
        colors.push(c);
    }
    assert_ne!(colors[0], colors[1], "the two points are in opposite cells");
}

/// `autoexposure` closes its loop through a reading: it measures its input before the gain,
/// the gain follows the reading, and the picture comes out at the target.
#[test]
fn autoexposure_brings_a_dark_input_to_its_target() {
    let mut g = Graph::new();
    let gray = nodes::add_to_graph(&mut g, "rgba", emath::Pos2::ZERO).unwrap();
    for key in ["r", "g", "b"] {
        set(&mut g, gray, key, 0.25);
    }
    let ae = nodes::add_to_graph(&mut g, "autoexposure", emath::Pos2::ZERO).unwrap();
    set(&mut g, ae, "target", 0.5);
    let out = nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO).unwrap();
    g.connect(PortRef::new(gray, "output"), PortRef::new(ae, "input"))
        .unwrap();
    g.connect(PortRef::new(ae, "output"), PortRef::new(out, "input"))
        .unwrap();
    let (shader, controls) = gpu::compiled(&g, out);
    let gain_name: Arc<str> = shader
        .uniforms
        .iter()
        .find_map(|(n, p)| matches!(p, UniformProvider::NodeUniform { .. }).then(|| Arc::clone(n)))
        .expect("the gain uniform");

    // What it measures is its workspace's pass's, drawn after the Output each tick.
    let ws = g.default_workspace();
    let pass = Arc::new(
        compile::wgsl::build_pass(&g, ws, &[ae], false)
            .pop()
            .expect("it measures"),
    );
    let key = PassKey {
        workspace: ws,
        batch: 0,
    };
    let pass_job = |send: bool| PassJob {
        key,
        shader: send.then(|| Arc::clone(&pass)),
        source: compile::source_hash(&pass.body),
        resolution: pass.pass_size(),
        region: None,
        uniforms: gpu::resolved(&g, &pass),
        taps: pass.slots(),
        draws: true,
    };

    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs_at = |gain: f32| {
        let controls = controls.clone();
        let (shader, gain_name) = (Arc::clone(&shader), Arc::clone(&gain_name));
        move |send: bool, mode| {
            let mut uniforms = controls.clone();
            uniforms.push((Arc::clone(&gain_name), UniformValue::Float(gain)));
            vec![job(out, (64, 64), &shader, send, mode, uniforms)]
        }
    };
    link_all(&mut renderer, &outputs_at(1.0));
    let with_pass = |outputs: Vec<OutputJob>, send: bool| FrameJob {
        passes: vec![pass_job(send)],
        ..tick(0.0, outputs)
    };
    renderer.draw(&with_pass(
        outputs_at(1.0)(false, OutputMode::Suspended),
        true,
    ));
    while !renderer.pass_linked(key) {
        drain(&gpu);
        renderer.draw(&with_pass(
            outputs_at(1.0)(false, OutputMode::Suspended),
            false,
        ));
    }

    // Speed 0, so the gain snaps to the first reading.
    let mut gain = 1.0f32;
    let mut measured = None;
    for _ in 0..4 {
        drain(&gpu);
        renderer.draw(&with_pass(outputs_at(gain)(false, DRAW), false));
        if let Some((_, words)) = renderer.take_passes().pop() {
            let s = stats(words);
            measured = Some(s.mean);
            gain = autoexposure::next_gain(gain, s.mean, 0.5, 0.0, 0.05, 8.0, 1.0 / 60.0);
        }
    }
    let measured = measured.expect("a reading came back");
    assert!(
        (measured - 0.25).abs() < 1e-3,
        "it measured the input, before the gain: {measured}"
    );
    assert!((gain - 2.0).abs() < 0.01, "gain {gain}");
    renderer.draw(&tick(0.0, outputs_at(gain)(false, DRAW)));
    drain(&gpu);
    let pixels = gpu::rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    let px = pixels[32 * 64 + 32];
    assert!(
        (126..=130).contains(&px[0]) && px[0] == px[1] && px[1] == px[2],
        "a quarter gray at gain 2 is a half: {px:?}"
    );
}

/// A hand-written module writing `slots` tap slots: `body` holds `fs_main` and may name
/// `tap`, declared at the compiler's binding.
fn tapping(floats: &[&str], slots: usize, body: &str) -> Arc<Shader> {
    let decl = format!(
        "@group(0) @binding({}) var<storage, read_write> tap: array<atomic<u32>>;\n",
        compile::wgsl::TAPS
    );
    let mut shader = (*module(floats, &[], &format!("{decl}{body}"))).clone();
    shader.taps = vec![(NodeId(0), TapKind::Stats); slots];
    Arc::new(shader)
}

/// Adds `adds[i]` into slot `i`'s first word from pixel (0, 0) alone, so a whole frame's
/// reading is the template's word plus exactly that. `tag` keeps two sources apart.
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

/// Counts its fragments into word 0 and writes its `u_time` into word 1 from pixel (0, 0),
/// after `u_iter` rounds of arithmetic per fragment.
fn counting(tag: &str) -> Arc<Shader> {
    tapping(
        &["u_iter"],
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

/// An Output's job with the shader's tap slots and `u_iter` at `iter`.
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

fn words_of(taken: &[(NodeId, Vec<u32>)], node: NodeId) -> Option<Vec<u32>> {
    taken
        .iter()
        .find(|(id, _)| *id == node)
        .map(|(_, w)| w.clone())
}

/// **A reading is never handed over on the tick that drew it**, and arrives on the tick after:
/// the words are mapped once the frame's submission has finished, and collected by the next
/// draw's settle rather than waited for.
#[test]
fn a_tap_reading_arrives_a_tick_late_and_is_never_waited_for() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let shader = tap_writer("late", &[3]);
    let out = NodeId(1);
    let outputs = |send: bool, mode| vec![job(out, (64, 64), &shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    assert!(
        renderer.take_taps().is_empty(),
        "the tick that drew the first frame has no reading of it"
    );
    drain(&gpu);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    let words = words_of(&renderer.take_taps(), out).expect("the frame before is collected");
    assert_eq!(words.len(), TAP_WORDS);
    assert_eq!(
        words[0],
        TAP_TEMPLATE[0] + 3,
        "reset from the template, then added to"
    );
    assert_eq!(&words[1..], &TAP_TEMPLATE[1..], "and nothing else written");
    assert!(
        renderer.take_taps().is_empty(),
        "each reading is taken once"
    );

    // An idle tick collects what the last frame left without drawing another.
    drain(&gpu);
    renderer.draw(&tick(0.0, outputs(false, OutputMode::Idle)));
    let words = words_of(&renderer.take_taps(), out).expect("collected on an idle tick");
    assert_eq!(words[0], TAP_TEMPLATE[0] + 3);
}

/// Every frame's words are its own: each draw resets the buffer from the template, so a
/// reading is one frame's sum and never a running total, whatever is still in flight.
#[test]
fn every_reading_is_one_frames_words() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let shader = counting("one frame");
    let out = NodeId(1);
    let size = (96, 54);
    let outputs = |send: bool, mode| vec![counted(out, size, &shader, (send, mode), 4.0)];
    link_all(&mut renderer, &outputs);
    let mut readings = Vec::new();
    for i in 0..12 {
        renderer.draw(&tick(i as f32, outputs(false, DRAW)));
        if i % 3 == 2 {
            drain(&gpu);
        }
        readings.extend(words_of(&renderer.take_taps(), out));
    }
    assert!(
        !readings.is_empty(),
        "readings arrive without waiting on every tick"
    );
    let mut times = Vec::new();
    for w in &readings {
        assert_eq!(w[0], size.0 * size.1, "a whole frame's fragments, once");
        times.push(f32::from_bits(w[1]));
    }
    assert!(
        times.windows(2).all(|t| t[0] < t[1]),
        "each reading is newer than the one before: {times:?}"
    );
}

/// The readings handed over across a change of source from one writing `before` to one
/// writing `after`, with frames of the old program still on the GPU and one drawn by it on the
/// tick the new source arrives. `settle` waits for that tick's frame before the next is drawn.
fn readings_across(before: &[u32], after: &[u32], settle: bool) -> Vec<Option<Vec<u32>>> {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let out = NodeId(1);
    let (before, after) = (tap_writer("before", before), tap_writer("after", after));
    let outputs = |send: bool, mode| vec![job(out, (64, 64), &before, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    for _ in 0..3 {
        renderer.draw(&tick(0.0, outputs(false, DRAW)));
    }
    renderer.take_taps();
    // Not waited for: frames are on the GPU when the source changes, as on a live synth.
    let next = |send: bool| vec![job(out, (64, 64), &after, send, DRAW, Vec::new())];
    renderer.draw(&tick(0.0, next(true)));
    if settle {
        drain(&gpu);
    }
    // Eight ticks at least, and four after the new program has landed, however long the link
    // takes on a loaded box.
    let mut log = Vec::new();
    let mut landed = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while log.len() < 8 || landed < 4 {
        assert!(
            std::time::Instant::now() < deadline,
            "the new program never linked"
        );
        renderer.draw(&tick(0.0, next(false)));
        drain(&gpu);
        log.push(words_of(&renderer.take_taps(), out));
        if !renderer.is_linking(out) {
            landed += 1;
        }
    }
    log
}

/// The first word of each slot of every reading.
fn firsts(log: &[Option<Vec<u32>>]) -> Vec<Option<Vec<u32>>> {
    log.iter()
        .map(|w| {
            w.as_ref()
                .map(|w| w.iter().step_by(TAP_WORDS).copied().collect())
        })
        .collect()
}

/// **A tap reading is delivered only from the program whose layout the job names.** The
/// source changes from one slot to two, in another order: slot 0 was X adding 1, and is now Y
/// adding 7 with X in slot 1. None of the old program's frames is handed over, and the new
/// program's are.
#[test]
fn a_tap_reading_is_delivered_only_from_the_program_whose_layout_the_job_names() {
    let log = firsts(&readings_across(&[1], &[7, 1], false));
    let want = vec![TAP_TEMPLATE[0] + 7, TAP_TEMPLATE[0] + 1];
    assert!(
        log.iter().flatten().all(|w| *w == want),
        "every reading handed over is the new program's: {log:?}"
    );
    assert_eq!(
        log.last().cloned().flatten(),
        Some(want),
        "and they arrive: {log:?}"
    );
}

/// The same with the slot count unchanged, and the old program's last frame finished before
/// the next draw collects it: `OutputRenderer::encode` tags each frame with the source of the
/// program that drew it, so the old program's frame is never read in the new layout.
#[test]
fn a_tap_reading_of_the_same_slot_count_is_delivered_only_from_the_program_named() {
    let log = firsts(&readings_across(&[1, 5], &[7, 1], true));
    let want = vec![TAP_TEMPLATE[0] + 7, TAP_TEMPLATE[0] + 1];
    assert!(
        log.iter().flatten().all(|w| *w == want),
        "every reading handed over is the new program's: {log:?}"
    );
    assert_eq!(
        log.last().cloned().flatten(),
        Some(want),
        "and they arrive: {log:?}"
    );
}

/// **A frame blanked in place does not hand over the taps of an unfinished frame.** With every
/// other target held, a render's black warm-up clears the latest target — a heavy frame still
/// on the GPU — in place. The heavy frame's reading comes from its own submission's map, so it
/// is handed over whole once that has finished, and every reading after is a whole frame.
#[test]
fn a_frame_blanked_in_place_does_not_hand_over_an_unfinished_frames_taps() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let out = NodeId(1);
    let size = (1280, 720);
    let shader = counting("blanked");
    let outputs = |send: bool, mode| vec![counted(out, size, &shader, (send, mode), 1.0)];
    link_all(&mut renderer, &outputs);
    let at = |time: f32, iter: f32, clear: bool| {
        let mut o = counted(out, size, &shader, (false, DRAW), iter);
        o.clear = clear;
        tick(time, vec![o])
    };
    // Four finished frames held: with the heavy frame in a fifth, the ring is at its limit.
    let mut holds: Vec<Published> = Vec::new();
    for i in 0..4 {
        renderer.draw(&at(1.0 + i as f32, 1.0, false));
        drain(&gpu);
        holds.push(renderer.publish());
    }
    renderer.take_taps();
    renderer.draw(&at(100.0, 200.0, false));
    // The latest is cleared in place, and nothing is drawn: every other target is held.
    renderer.draw(&at(101.0, 1.0, true));
    let mut readings: Vec<Vec<u32>> = words_of(&renderer.take_taps(), out).into_iter().collect();
    drop(holds);
    for i in 0..3 {
        drain(&gpu);
        renderer.draw(&at(102.0 + i as f32, 1.0, false));
        readings.extend(words_of(&renderer.take_taps(), out));
    }
    let full = size.0 * size.1;
    for w in &readings {
        assert_eq!(w[0], full, "a reading is of a whole frame");
    }
    assert!(
        readings.iter().any(|w| f32::from_bits(w[1]) == 100.0),
        "the heavy frame's reading is handed over once it has finished: {:?}",
        readings
            .iter()
            .map(|w| f32::from_bits(w[1]))
            .collect::<Vec<_>>()
    );
}

// ------------------------------------------------------------------------ pictures

/// A module filling the frame with opaque white.
fn white() -> Arc<Shader> {
    gpu::solid(1.0, 1.0, 1.0)
}

/// A module whose gray runs from black at the left edge to white at the right.
fn gradient() -> Arc<Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let x = frag_coord.x / u.u_resolution.x;
    return vec4f(vec3f(x), 1.0);
}
",
    )
}

/// One Output of `shader` at `size`, linked and drawn once.
fn one_output(shader: &Arc<Shader>, size: (u32, u32)) -> (supersilvia::render::Gpu, Renderer) {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(NodeId(1), size, shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    (gpu, renderer)
}

/// Draw with a finished GPU each time until `take` has something, at most ten seconds.
fn until<T>(
    gpu: &supersilvia::render::Gpu,
    renderer: &mut Renderer,
    job: &dyn Fn() -> supersilvia::render::FrameJob,
    mut take: impl FnMut(&mut Renderer) -> Option<T>,
) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        drain(gpu);
        renderer.draw(&job());
        if let Some(t) = take(renderer) {
            return t;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the read never landed"
        );
    }
}

/// A thumbnail is read back **without the synth ever waiting on the GPU**: the tick that
/// issues the read returns with nothing, and a later one has the picture — 240x135,
/// letterboxed like the on-node render, so a square frame has black bars beside it.
#[test]
fn a_thumbnail_is_read_back_a_frame_late_and_never_waits() {
    let out = NodeId(1);
    let shader = white();
    let (gpu, mut renderer) = one_output(&shader, (64, 64));
    let frame = || {
        tick(
            0.0,
            vec![job(out, (64, 64), &shader, false, DRAW, Vec::new())],
        )
    };

    renderer.request_thumbnail(out);
    assert!(renderer.thumbnail_pending(out));
    assert!(renderer.wants_picture(out), "a thumbnail is owed a draw");
    renderer.draw(&frame());
    assert!(
        renderer.take_thumbnails().is_empty(),
        "the tick that issued the read does not have it: it was issued, not waited on"
    );
    assert!(
        !renderer.wants_picture(out),
        "issued, it wants no further draw"
    );
    let bytes = until(&gpu, &mut renderer, &frame, |r| {
        r.take_thumbnails().pop().map(|(_, b)| b)
    });
    assert_eq!(bytes.len(), THUMBNAIL_BYTES, "240x135 RGBA8");
    assert!(
        !renderer.thumbnail_pending(out),
        "and nothing is outstanding"
    );

    let at = |x: u32, y: u32| {
        let i = ((y * THUMBNAIL.0 + x) * 4) as usize;
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    };
    assert_eq!(
        at(THUMBNAIL.0 / 2, THUMBNAIL.1 / 2),
        [255, 255, 255, 255],
        "the white render is in the middle of the thumbnail"
    );
    assert_eq!(
        at(0, THUMBNAIL.1 / 2),
        [0, 0, 0, 255],
        "and the bar beside it is black"
    );
    assert_eq!(at(THUMBNAIL.0 - 1, 0), [0, 0, 0, 255], "on both sides");
    // 135 tall and square: the picture is 135 wide, centred.
    let left = (THUMBNAIL.0 - THUMBNAIL.1) / 2;
    assert_eq!(
        at(left, 0)[0],
        255,
        "the picture starts where the bar stops"
    );
    assert_eq!(at(left - 1, 0)[0], 0);
    assert_eq!(at(left + THUMBNAIL.1 - 1, THUMBNAIL.1 - 1)[0], 255);
    assert_eq!(at(left + THUMBNAIL.1, THUMBNAIL.1 - 1)[0], 0);
}

/// A Snap is the frame in front of you at the Output's own size, read back a tick late like a
/// thumbnail — whole, with nothing letterboxed or scaled, rows top first — and the PNG it
/// becomes is the same pixels back.
#[test]
fn a_snap_is_the_whole_frame_at_full_size_and_the_png_is_lossless() {
    const W: u32 = 96;
    const H: u32 = 48;
    let out = NodeId(1);
    let shader = gradient();
    let (gpu, mut renderer) = one_output(&shader, (W, H));
    let frame = || {
        tick(
            0.0,
            vec![job(out, (W, H), &shader, false, DRAW, Vec::new())],
        )
    };

    renderer.request_snap(out);
    assert!(renderer.snap_pending(out));
    renderer.draw(&frame());
    assert!(
        renderer.take_snaps().is_empty(),
        "the tick that issued the read does not have it"
    );
    let (w, h, bytes) = until(&gpu, &mut renderer, &frame, |r| {
        r.take_snaps().pop().map(|(_, w, h, b)| (w, h, b))
    });
    assert_eq!(
        (w, h),
        (W, H),
        "the Output's own resolution, not a thumbnail"
    );
    assert_eq!(bytes.len() as u32, W * H * 4, "RGBA8, every pixel of it");
    assert!(!renderer.snap_pending(out), "and nothing is outstanding");

    // The Output's own frame, flipped top first and rounded to bytes, pixel for pixel.
    let drawn = gpu::rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    let flipped: Vec<u8> = drawn
        .chunks(W as usize)
        .rev()
        .flatten()
        .flatten()
        .copied()
        .collect();
    assert_eq!(bytes, flipped, "the frame exactly, one texel to one pixel");
    let at = |x: u32, y: u32| bytes[((y * W + x) * 4) as usize];
    assert!(at(0, 0) < 16, "the left edge is dark: {}", at(0, 0));
    assert!(
        at(W - 1, H - 1) > 239,
        "the right edge is bright: {}",
        at(W - 1, H - 1)
    );

    let path =
        std::env::temp_dir().join(format!("supersilvia-wgpu-snap-{}.png", std::process::id()));
    let image = supersilvia::video::png::Image {
        width: W,
        height: H,
        rgba: bytes.clone(),
    };
    supersilvia::video::png::write(&path, &image).expect("write the snap");
    let back = supersilvia::video::png::read(&path).expect("read it back");
    std::fs::remove_file(&path).ok();
    assert_eq!((back.width, back.height), (W, H));
    assert_eq!(back.rgba, bytes, "the PNG is lossless");
}

/// **A thumbnail and a Snap of an idle Output are owed a draw**, and only one: the synth draws
/// an idle Output on a tick it wants a picture, and after that draw it is idle again.
#[test]
fn a_thumbnail_and_a_snap_of_an_idle_output_want_one_draw() {
    let out = NodeId(1);
    let shader = white();
    let (gpu, mut renderer) = one_output(&shader, (64, 36));
    let at = |mode| {
        tick(
            0.0,
            vec![job(out, (64, 36), &shader, false, mode, Vec::new())],
        )
    };
    drain(&gpu);
    renderer.draw(&at(OutputMode::Idle));
    assert!(
        !renderer.wants_picture(out),
        "an idle Output with its picture wants nothing"
    );

    renderer.request_thumbnail(out);
    renderer.request_snap(out);
    renderer.draw(&at(OutputMode::Idle));
    assert!(
        renderer.wants_picture(out),
        "an idle tick does not issue what only a draw makes"
    );
    renderer.draw(&at(DRAW));
    assert!(!renderer.wants_picture(out), "one draw issues both");
    let idle = || at(OutputMode::Idle);
    let thumb = until(&gpu, &mut renderer, &idle, |r| r.take_thumbnails().pop());
    assert_eq!(thumb.1.len(), THUMBNAIL_BYTES);
    let snap = until(&gpu, &mut renderer, &idle, |r| r.take_snaps().pop());
    assert_eq!((snap.1, snap.2), (64, 36), "collected on idle ticks");
    assert!(!renderer.wants_picture(out));
}

/// The pattern a captured frame is checked by: `uv` in red and green and `u_time` in blue,
/// so a frame says which frame it is and where its corners are.
fn uv_and_time() -> Arc<Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    return vec4f(uv, u.u_time, 1.0);
}
",
    )
}

/// A capture reads back **every** frame, at the Output's full size, in order, and drops none.
/// The frames go through with no wait between them but the capture's own, and the last are
/// collected once the capture is turned off.
#[test]
fn a_capture_reads_back_every_frame_at_full_size_and_in_order() {
    const W: u32 = 960;
    const H: u32 = 540;
    const FRAMES: usize = 8;
    let out = NodeId(1);
    let shader = uv_and_time();
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(out, (W, H), &shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);

    assert!(renderer.set_capturing(out, true, 1, Alpha::Straight));
    let before = renderer.drops(out).total();
    let mut frames = Vec::new();
    for i in 0..FRAMES {
        renderer.draw(&tick(i as f32 / FRAMES as f32, outputs(false, DRAW)));
        frames.extend(renderer.take_captured(out));
    }
    assert_eq!(renderer.drops(out).total(), before, "no frame was dropped");
    assert_eq!(
        renderer.captures_issued(out),
        FRAMES as u64,
        "one read per frame"
    );
    renderer.set_capturing(out, false, 1, Alpha::Straight);
    // The next tick's poll collects the rest, waiting for them now the capture is off.
    renderer.draw(&tick(0.0, outputs(false, OutputMode::Suspended)));
    frames.extend(renderer.take_captured(out));
    assert!(
        !renderer.capture_pending(out),
        "nothing is outstanding once the capture is off"
    );
    assert_eq!(frames.len(), FRAMES, "every frame came back");

    let stride = (W * 4) as usize;
    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(
            frame.len(),
            (W * H * 4) as usize,
            "frame {i} is {W}x{H} RGBA8"
        );
        let at = |x: usize, y: usize| {
            let p = y * stride + x * 4;
            [frame[p], frame[p + 1], frame[p + 2], frame[p + 3]]
        };
        let top_left = at(0, 0);
        let bottom_right = at(W as usize - 1, H as usize - 1);
        // Rows top first: the top row is uv.y near one.
        assert!(
            top_left[0] <= 1 && top_left[1] >= 254,
            "frame {i} top-left {top_left:?}"
        );
        assert!(
            bottom_right[0] >= 254 && bottom_right[1] <= 1,
            "frame {i} bottom-right {bottom_right:?}"
        );
        let want = (i as f32 / FRAMES as f32 * 255.0).round() as i32;
        let got = i32::from(top_left[2]);
        assert!(
            (got - want).abs() <= 1,
            "frame {i} carries its own time in blue: {got} for {want}"
        );
        assert_eq!(top_left[3], 255);
    }
}

/// **A supersampled capture comes back at the film's size and averaged.** A hard diagonal,
/// black one side and white the other, drawn at the film's size is a stair step with nothing
/// between; drawn at two or four times it, the pixels the edge crosses come back as the
/// average of what it crossed.
#[test]
fn a_supersampled_capture_comes_back_at_the_films_size_and_averaged() {
    const W: u32 = 320;
    const H: u32 = 320;
    let out = NodeId(1);
    let shader = module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let e = step(frag_coord.y, frag_coord.x);
    return vec4f(e, e, e, 1.0);
}
",
    );
    let gpu = gpu::gpu();
    let film = |scale: u32| -> Vec<u8> {
        let size = (W * scale, H * scale);
        let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
        let outputs = |send: bool, mode| vec![job(out, size, &shader, send, mode, Vec::new())];
        link_all(&mut renderer, &outputs);
        assert!(renderer.set_capturing(out, true, scale, Alpha::Straight));
        renderer.draw(&tick(0.0, outputs(false, DRAW)));
        renderer.set_capturing(out, false, scale, Alpha::Straight);
        renderer.draw(&tick(0.0, outputs(false, OutputMode::Suspended)));
        let mut frames = renderer.take_captured(out);
        assert_eq!(frames.len(), 1, "one frame, whatever it was drawn at");
        let frame = frames.remove(0);
        assert_eq!(
            frame.len(),
            (W * H * 4) as usize,
            "the film is the Output's size at {scale}x"
        );
        frame
    };
    // Pixels neither black nor white: what a hard edge leaves once it is averaged.
    let between = |frame: &[u8]| {
        frame
            .iter()
            .step_by(4)
            .filter(|v| (16..=239).contains(*v))
            .count()
    };
    assert_eq!(between(&film(1)), 0, "at 1x the edge is a stair step");
    let twice = between(&film(2));
    assert!(
        twice > (H as usize) / 2,
        "at 2x the edge is averaged: {twice} between"
    );
    let four = between(&film(4));
    assert!(four > (H as usize) / 2, "and at 4x: {four} between");
}

// ---------------------------------------------------------------------------------- alpha

/// A module whose left half is a half-transparent red and right half transparent black, both
/// premultiplied as every color in the graph is: `(0.5, 0, 0, 0.5)` is full red at half
/// coverage.
fn half_red_and_nothing() -> Arc<Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return select(vec4f(0.0), vec4f(0.5, 0.0, 0.0, 0.5), frag_coord.x < u.u_resolution.x * 0.5);
}
",
    )
}

/// One RGBA8 pixel of `bytes`, rows `width` wide.
fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
}

/// Whether `got` is `want` to within one in every channel: the frame's own rounding of a half.
fn close(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1)
}

/// **A Snap and a thumbnail are written straight**, as a PNG is read: the frame's
/// premultiplied half-transparent red comes out as full red at half alpha, and transparent
/// black stays transparent black. The thumbnail's letterbox bars are opaque black.
#[test]
fn a_snap_and_a_thumbnail_of_a_premultiplied_frame_are_straight() {
    const W: u32 = 64;
    const H: u32 = 64;
    let out = NodeId(1);
    let shader = half_red_and_nothing();
    let (gpu, mut renderer) = one_output(&shader, (W, H));
    let frame = || {
        tick(
            0.0,
            vec![job(out, (W, H), &shader, false, DRAW, Vec::new())],
        )
    };
    renderer.request_snap(out);
    renderer.request_thumbnail(out);
    renderer.draw(&frame());
    let (w, _, snap) = until(&gpu, &mut renderer, &frame, |r| {
        r.take_snaps().pop().map(|(_, w, h, b)| (w, h, b))
    });
    let red = pixel(&snap, w, W / 4, H / 2);
    assert!(close(red, [255, 0, 0, 128]), "straight red: {red:?}");
    let nothing = pixel(&snap, w, 3 * W / 4, H / 2);
    assert_eq!(nothing, [0, 0, 0, 0], "transparent black stays so");

    let thumb = until(&gpu, &mut renderer, &frame, |r| {
        r.take_thumbnails().pop().map(|(_, b)| b)
    });
    let (tw, th) = THUMBNAIL;
    let left = (tw - th) / 2;
    let red = pixel(&thumb, tw, left + th / 4, th / 2);
    assert!(close(red, [255, 0, 0, 128]), "thumbnail red: {red:?}");
    let nothing = pixel(&thumb, tw, left + 3 * th / 4, th / 2);
    assert_eq!(nothing, [0, 0, 0, 0], "thumbnail transparent");
    assert_eq!(pixel(&thumb, tw, 0, th / 2), [0, 0, 0, 255], "the bar");
}

/// One frame of `shader` captured at `scale` with `alpha`, the film `size`.
fn captured(shader: &Arc<Shader>, size: (u32, u32), scale: u32, alpha: Alpha) -> Vec<u8> {
    let out = NodeId(1);
    let gpu = gpu::gpu();
    let drawn = (size.0 * scale, size.1 * scale);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(out, drawn, shader, send, mode, Vec::new())];
    link_all(&mut renderer, &outputs);
    assert!(renderer.set_capturing(out, true, scale, alpha));
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    renderer.set_capturing(out, false, scale, alpha);
    renderer.draw(&tick(0.0, outputs(false, OutputMode::Suspended)));
    let mut frames = renderer.take_captured(out);
    assert_eq!(frames.len(), 1, "one frame");
    frames.remove(0)
}

/// **A capture for a file is straight and one for a video is premultiplied**: the PNG
/// sequence and the GIF are read with their alpha, and the encoder drops alpha, so what it is
/// handed is the picture over black.
#[test]
fn a_capture_is_straight_for_a_file_and_premultiplied_for_a_video() {
    const W: u32 = 64;
    const H: u32 = 32;
    let shader = half_red_and_nothing();
    let straight = captured(&shader, (W, H), 1, Alpha::Straight);
    let red = pixel(&straight, W, W / 4, H / 2);
    assert!(close(red, [255, 0, 0, 128]), "straight: {red:?}");
    assert_eq!(pixel(&straight, W, 3 * W / 4, H / 2), [0, 0, 0, 0]);
    let over_black = captured(&shader, (W, H), 1, Alpha::Premultiplied);
    let red = pixel(&over_black, W, W / 4, H / 2);
    assert!(close(red, [128, 0, 0, 128]), "premultiplied: {red:?}");
    assert_eq!(pixel(&over_black, W, 3 * W / 4, H / 2), [0, 0, 0, 0]);
}

/// **A supersampled capture averages premultiplied colors** and unpremultiplies after: an
/// opaque red edge against transparent black comes back, where the edge is averaged, as red at
/// partial alpha — not as a red darkened by the transparent texels it was averaged with.
#[test]
fn a_supersampled_capture_averages_an_edge_premultiplied() {
    const W: u32 = 64;
    const H: u32 = 64;
    let shader = module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return vec4f(1.0, 0.0, 0.0, 1.0) * step(frag_coord.y, frag_coord.x);
}
",
    );
    for scale in [2, 4] {
        let frame = captured(&shader, (W, H), scale, Alpha::Straight);
        let edge: Vec<[u8; 4]> = frame
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .filter(|p| (16..=239).contains(&p[3]))
            .collect();
        assert!(
            edge.len() > (H as usize) / 2,
            "at {scale}x the edge is averaged: {} pixels",
            edge.len()
        );
        for p in &edge {
            assert!(
                p[0] >= 250 && p[1] == 0 && p[2] == 0,
                "at {scale}x an edge pixel is red at partial alpha: {p:?}"
            );
        }
    }
}
