// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's linking: a link made on a `linker` thread never takes the picture away, a
//! failure keeps the program drawing and says why, a superseded link is never shown, a source
//! drawn before comes back without a link, and an Output draws nothing before its first
//! program lands. Every wait is for a link to land, bounded only so a link that never does
//! fails the test rather than hanging it.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, link_all, module, rgba_of, solid, tick};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::compile::{self, Shader};
use supersilvia::graph::NodeId;
use supersilvia::render::link::{KEPT_PROGRAMS, Programs};
use supersilvia::render::{Gpu, OutputMode, Renderer};

const DRAW: OutputMode = OutputMode::Draw;
const NODE: NodeId = NodeId(1);
const SIZE: (u32, u32) = (32, 32);
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// How long a link may take before the test calls it lost.
const LINK_DEADLINE: Duration = Duration::from_secs(20);

/// A tick drawing `shader` on [`NODE`], sent where `send` says.
fn draw(renderer: &mut Renderer, shader: &Arc<Shader>, send: bool) {
    renderer.draw(&tick(
        0.0,
        vec![job(NODE, SIZE, shader, send, DRAW, Vec::new())],
    ));
}

/// Draw `shader` unsent until [`NODE`]'s link has landed, one way or the other.
fn draw_until_linked(renderer: &mut Renderer, shader: &Arc<Shader>) {
    let deadline = Instant::now() + LINK_DEADLINE;
    while renderer.is_linking(NODE) {
        assert!(Instant::now() < deadline, "the link never landed");
        draw(renderer, shader, false);
    }
}

/// The one colour [`NODE`]'s published picture is, once every submission has finished.
fn shown(gpu: &Gpu, renderer: &mut Renderer) -> [u8; 4] {
    drain(gpu);
    let published = renderer.publish();
    let picture = published.outputs.get(&NODE).expect("a picture is shown");
    let pixels = rgba_of(gpu, picture.texture.texture());
    let first = pixels[0];
    assert!(pixels.iter().all(|p| *p == first), "one colour: {first:?}");
    first
}

/// A renderer whose [`NODE`] draws `shader`, linked and drawn once.
fn drawing(gpu: &Gpu, shader: &Arc<Shader>) -> Renderer {
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    link_all(&mut renderer, &|send, mode| {
        vec![job(NODE, SIZE, shader, send, mode, Vec::new())]
    });
    draw(&mut renderer, shader, false);
    renderer
}

/// `shader` with a comment appended: the same picture from another source.
fn edited(shader: &Shader, n: usize) -> Arc<Shader> {
    let mut edited = shader.clone();
    writeln!(edited.body, "// edit {n}").expect("a String takes it");
    Arc::new(edited)
}

fn source(shader: &Shader) -> u64 {
    compile::source_hash(&shader.body)
}

/// Poll until the link in flight has landed, one way or the other.
fn poll_until_linked(programs: &mut Programs) {
    let deadline = Instant::now() + LINK_DEADLINE;
    loop {
        programs.poll();
        if !programs.is_linking() {
            return;
        }
        assert!(Instant::now() < deadline, "the link never landed");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn programs() -> Programs {
    Programs::new(wgpu::TextureFormat::Rgba16Float)
}

/// **A new program links without taking the old one away.** While the edit's link is held in
/// flight, every frame is the old program's; once it lands, the new one draws, and no frame
/// was dropped for it.
#[test]
fn a_new_program_links_without_taking_the_old_one_away() {
    let gpu = gpu::gpu();
    let (red, green) = (solid(1.0, 0.0, 0.0), solid(0.0, 1.0, 0.0));
    let mut renderer = drawing(&gpu, &red);
    assert_eq!(shown(&gpu, &mut renderer), RED);

    renderer.hold_link(NODE, true);
    draw(&mut renderer, &green, true);
    for tick in 0..4 {
        draw(&mut renderer, &green, false);
        assert!(renderer.is_linking(NODE), "tick {tick}: in flight");
        assert_eq!(shown(&gpu, &mut renderer), RED, "tick {tick}: the old one");
    }

    renderer.hold_link(NODE, false);
    draw_until_linked(&mut renderer, &green);
    assert_eq!(renderer.errors.get(&NODE), None);
    draw(&mut renderer, &green, false);
    assert_eq!(shown(&gpu, &mut renderer), GREEN, "the new program draws");
    assert_eq!(renderer.dropped_frames(NODE), 0);
}

/// **A program that fails to link leaves the working one running**, whether the module does
/// not parse or the device refuses the pipeline made of it, and the failure is on the status
/// line. A source that links then replaces it and clears the error.
#[test]
fn a_program_that_fails_to_link_leaves_the_working_one_running() {
    let gpu = gpu::gpu();
    let red = solid(1.0, 0.0, 0.0);
    let mut renderer = drawing(&gpu, &red);

    let mut unparsed = (*solid(0.0, 1.0, 0.0)).clone();
    unparsed.body.push_str("fn broken( { }\n");
    // Binds a texture the `Shader` does not list, so the group's layout has no room for it.
    let refused = module(
        &[],
        &[],
        "
@group(0) @binding(9) var stray: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return textureLoad(stray, vec2i(0, 0), 0);
}
",
    );
    for broken in [Arc::new(unparsed), refused] {
        draw(&mut renderer, &broken, true);
        draw_until_linked(&mut renderer, &broken);
        let error = renderer.errors.get(&NODE).cloned();
        assert!(error.is_some(), "the failure is on the status line");
        for tick in 0..3 {
            draw(&mut renderer, &broken, false);
            assert_eq!(renderer.errors.get(&NODE), error.as_ref(), "tick {tick}");
            assert_eq!(
                shown(&gpu, &mut renderer),
                RED,
                "tick {tick}: still drawing"
            );
        }
    }

    let green = solid(0.0, 1.0, 0.0);
    draw(&mut renderer, &green, true);
    draw_until_linked(&mut renderer, &green);
    assert_eq!(renderer.errors.get(&NODE), None, "the error goes with it");
    draw(&mut renderer, &green, false);
    assert_eq!(shown(&gpu, &mut renderer), GREEN);
}

/// **A superseded link is never shown.** An edit's link is held in flight and a newer one
/// replaces it before it lands: every frame until the newer one lands is the old program's,
/// never the superseded one's.
#[test]
fn a_superseded_link_is_never_shown() {
    let gpu = gpu::gpu();
    let (red, green, blue) = (
        solid(1.0, 0.0, 0.0),
        solid(0.0, 1.0, 0.0),
        solid(0.0, 0.0, 1.0),
    );
    let mut renderer = drawing(&gpu, &red);

    renderer.hold_link(NODE, true);
    draw(&mut renderer, &green, true);
    draw(&mut renderer, &blue, true);
    renderer.hold_link(NODE, false);
    let deadline = Instant::now() + LINK_DEADLINE;
    while renderer.is_linking(NODE) {
        assert!(Instant::now() < deadline, "the link never landed");
        draw(&mut renderer, &blue, false);
        let colour = shown(&gpu, &mut renderer);
        assert!(colour == RED || colour == BLUE, "{colour:?}");
    }
    draw(&mut renderer, &blue, false);
    assert_eq!(shown(&gpu, &mut renderer), BLUE);
}

/// **Superseded links do not pile up, and none of them lands.** Thirty-two sources sent one
/// after another, with no poll between, a control drag's worth: whatever the linkers made of
/// the older ones is dropped, the newest lands, and only the program it replaced is kept.
#[test]
fn a_burst_of_superseded_links_lands_only_the_newest() {
    let gpu = gpu::gpu();
    let first = solid(1.0, 0.0, 0.0);
    let mut programs = programs();
    programs.set_shader(&gpu, &first);
    poll_until_linked(&mut programs);
    assert_eq!(programs.source(), Some(source(&first)));

    let burst: Vec<Arc<Shader>> = (0..32).map(|n| edited(&first, n)).collect();
    for shader in &burst {
        programs.set_shader(&gpu, shader);
    }
    assert!(programs.is_linking(), "only the newest is pending");
    let newest = source(burst.last().expect("32"));
    let deadline = Instant::now() + LINK_DEADLINE;
    loop {
        programs.poll();
        let on = programs.source();
        assert!(
            on == Some(source(&first)) || on == Some(newest),
            "a superseded program landed"
        );
        if !programs.is_linking() {
            break;
        }
        assert!(Instant::now() < deadline, "the link never landed");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(programs.source(), Some(newest));
    assert_eq!(programs.error, None);
    assert_eq!(programs.kept(), 1);
    assert!(programs.keeps(source(&first)));
}

/// **Going back to what is on screen drops the link in flight.** An edit is sent and undone
/// before it lands: nothing is linking, and when another edit lands later, the undone one was
/// never drawn with — only the first program is kept.
#[test]
fn an_edit_undone_before_it_lands_never_lands() {
    let gpu = gpu::gpu();
    let first = solid(1.0, 0.0, 0.0);
    let (undone, next) = (edited(&first, 1), edited(&first, 2));
    let mut programs = programs();
    programs.set_shader(&gpu, &first);
    poll_until_linked(&mut programs);

    programs.set_shader(&gpu, &undone);
    assert!(programs.is_linking());
    programs.set_shader(&gpu, &first);
    assert!(!programs.is_linking(), "back to the program on screen");

    programs.set_shader(&gpu, &next);
    poll_until_linked(&mut programs);
    assert_eq!(programs.source(), Some(source(&next)));
    assert!(
        !programs.keeps(source(&undone)),
        "the undone edit never landed"
    );
    assert_eq!(programs.kept(), 1);
}

/// **A source drawn before comes back without a link**, on the call that asks for it: an
/// undo that restores the graph before an edit links nothing. And what is kept is bounded.
#[test]
fn a_source_drawn_before_comes_back_without_a_link() {
    let gpu = gpu::gpu();
    let (g, _, out) = gpu::one_node("checkerboard");
    let (before, _) = gpu::compiled(&g, out);
    let after = solid(0.0, 1.0, 0.0);
    let mut programs = programs();
    programs.set_shader(&gpu, &before);
    poll_until_linked(&mut programs);
    programs.set_shader(&gpu, &after);
    poll_until_linked(&mut programs);
    assert_eq!(programs.kept(), 1, "the program the edit replaced is kept");

    // The undo.
    programs.set_shader(&gpu, &before);
    assert!(
        !programs.is_linking(),
        "the program it had is drawn with, not linked again"
    );
    assert_eq!(programs.source(), Some(source(&before)));
    assert_eq!(programs.error, None);

    // The same source as the one on screen sends nothing either.
    programs.set_shader(&gpu, &before);
    assert!(!programs.is_linking());

    // Six more sources, one after another: the oldest kept go, the newest stay.
    for n in 0..6 {
        programs.set_shader(&gpu, &edited(&after, n));
        poll_until_linked(&mut programs);
        assert!(programs.kept() <= KEPT_PROGRAMS, "{}", programs.kept());
    }
    assert_eq!(programs.kept(), KEPT_PROGRAMS);
    assert_eq!(programs.error, None);
}

/// **An Output draws nothing before its first program lands.** Its link is held: every tick
/// publishes no picture of it and its frame stays the ring's first clear, with no frame
/// counted dropped; once the program lands, it draws.
#[test]
fn an_output_draws_nothing_before_its_first_program_lands() {
    let gpu = gpu::gpu();
    let red = solid(1.0, 0.0, 0.0);
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    renderer.draw(&tick(
        0.0,
        vec![job(
            NODE,
            SIZE,
            &red,
            false,
            OutputMode::Suspended,
            Vec::new(),
        )],
    ));
    renderer.hold_link(NODE, true);
    draw(&mut renderer, &red, true);
    for tick in 0..4 {
        draw(&mut renderer, &red, false);
        assert!(renderer.is_linking(NODE), "tick {tick}");
        drain(&gpu);
        let published = renderer.publish();
        assert!(!published.outputs.contains_key(&NODE), "tick {tick}");
        let texture = renderer.texture_of(NODE).expect("its ring");
        assert!(
            rgba_of(&gpu, &texture).iter().all(|p| *p == [0; 4]),
            "tick {tick}: nothing drawn"
        );
    }
    assert_eq!(renderer.dropped_frames(NODE), 0);

    renderer.hold_link(NODE, false);
    draw_until_linked(&mut renderer, &red);
    draw(&mut renderer, &red, false);
    assert_eq!(shown(&gpu, &mut renderer), RED);
}

/// **A probe's module links the same way**: `compile::wgsl::build_probe`'s module, with every
/// node counting into its tap buffer, lands from a linker with no error.
#[test]
fn a_probe_module_links_on_a_linker() {
    let gpu = gpu::gpu();
    let (g, _, out) = gpu::one_node("checkerboard");
    let probe = compile::wgsl::build_probe(&g, out).expect("connected");
    assert!(!probe.taps.is_empty(), "a probe counts");
    let mut programs = programs();
    programs.set_shader(&gpu, &probe);
    assert!(programs.is_linking());
    poll_until_linked(&mut programs);
    assert_eq!(programs.error, None);
    assert!(programs.adopt(source(&probe), &[], probe.taps.len()));
    let (program, _) = programs.current().expect("landed");
    assert!(program.taps(), "and binds its tap buffer");
}
