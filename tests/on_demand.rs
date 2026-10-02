// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: an Output nothing depends on is not drawn while nobody can see it — the draw rule
//! of docs/rendering.md#which-outputs-draw, a test per clause. The pixels are
//! `tests/gpu_render.rs`'s and `tests/gpu_app.rs`'s.

mod common;

use common::{add_on, connect, picture_on, two_tabs};
use supersilvia::graph::{NodeId, PortRef, WorkspaceId};
use supersilvia::mixer::Channel;
use supersilvia::project::Active;
use supersilvia::render::OutputMode;
use supersilvia::render::publish::Via;
use supersilvia::synth::Why;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

/// Whether the job the synth builds from the plan draws this Output.
fn drawn(app: &mut App, node: NodeId) -> bool {
    app.build_frame_job()
        .outputs
        .iter()
        .any(|o| o.node == node && o.mode.draws())
}

/// Whether the job draws this workspace's pass, and so measures what it holds.
fn measuring(app: &mut App, workspace: WorkspaceId) -> bool {
    app.build_frame_job()
        .passes
        .iter()
        .any(|p| p.key.workspace == workspace && p.draws)
}

#[test]
fn an_output_on_a_tab_not_looked_at_with_nothing_reading_it_does_not_draw() {
    let (mut app, first, second) = two_tabs();
    let here = picture_on(&mut app, first);
    let there = picture_on(&mut app, second);

    assert!(drawn(&mut app, here), "on the tab being looked at");
    assert_eq!(app.draws(here), Some(Why::Tab));
    assert!(!drawn(&mut app, there), "open, awake, and seen by nobody");
    assert_eq!(app.draws(there), None);

    let job = app.build_frame_job();
    let idle = job
        .outputs
        .iter()
        .find(|o| o.node == there)
        .expect("an idle Output is described, not dropped");
    assert!(
        idle.mode == OutputMode::Idle,
        "idle, not suspended: its nodes still tick and its program is kept ready"
    );
}

#[test]
fn switching_to_a_tab_draws_its_outputs_on_the_next_tick() {
    let (mut app, _, second) = two_tabs();
    let there = picture_on(&mut app, second);
    assert!(!drawn(&mut app, there));

    app.activate(Active::Workspace(second));
    assert!(
        drawn(&mut app, there),
        "the next job the synth builds draws it"
    );
    assert_eq!(app.draws(there), Some(Why::Tab));
}

#[test]
fn the_project_tab_draws_nothing_but_what_is_on_air() {
    let (mut app, first, second) = two_tabs();
    let here = picture_on(&mut app, first);
    let there = picture_on(&mut app, second);
    app.show_on(Channel::B, there);

    app.activate(Active::Project);
    assert!(!drawn(&mut app, here), "no workspace is being looked at");
    assert!(drawn(&mut app, there), "but the deck is on air");
}

#[test]
fn a_feedback_output_on_a_tab_not_looked_at_draws_every_tick() {
    let (mut app, _, second) = two_tabs();
    // A checkerboard mixed with the Output's own last frame: a loop with a history.
    let out = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    connect(&mut app, (source, "output"), (mix, "a"));
    connect(&mut app, (out, "frame"), (mix, "b"));
    connect(&mut app, (mix, "output"), (out, "input"));

    for _ in 0..5 {
        app.tick(FRAME);
        assert!(drawn(&mut app, out), "every tick, looked at or not");
    }
    assert_eq!(app.draws(out), Some(Why::Feedback));
}

/// **Pause freezes feedback.** An Output in a loop draws only on a tick the playhead moved:
/// while paused nothing in the loop advances, wherever it is shown — the tab being looked at,
/// another tab or a deck — and a seek while paused is a move, drawn on the tick it lands.
/// An Output outside every loop draws on as before; its picture is a function of the clock.
#[test]
fn a_feedback_output_draws_only_while_the_playhead_moves() {
    use supersilvia::transport::Command as Transport;
    let (mut app, first, second) = two_tabs();
    let looped = |app: &mut App, workspace| {
        let out = add_on(app, "output", workspace);
        let source = add_on(app, "checkerboard", workspace);
        let mix = add_on(app, "mix", workspace);
        connect(app, (source, "output"), (mix, "a"));
        connect(app, (out, "frame"), (mix, "b"));
        connect(app, (mix, "output"), (out, "input"));
        out
    };
    let here = looped(&mut app, first);
    let there = looped(&mut app, second);
    let on_air = looped(&mut app, second);
    app.show_on(Channel::A, on_air);
    let plain = picture_on(&mut app, first);
    let loops = [here, there, on_air];

    app.tick(FRAME);
    for out in loops {
        assert!(drawn(&mut app, out), "playing, a loop draws");
    }

    app.transport(Transport::Pause);
    app.tick(FRAME);
    app.tick(FRAME);
    for out in loops {
        assert!(!drawn(&mut app, out), "paused, the loop stands still");
    }
    assert!(drawn(&mut app, plain), "an Output outside a loop draws on");

    app.transport(Transport::Seek(3.0));
    app.tick(FRAME);
    for out in loops {
        assert!(drawn(&mut app, out), "a seek moves the playhead");
    }
    app.tick(FRAME);
    for out in loops {
        assert!(
            !drawn(&mut app, out),
            "and the tick after it stands still again"
        );
    }

    app.transport(Transport::Play);
    app.tick(FRAME);
    for out in loops {
        assert!(drawn(&mut app, out), "playing again, it moves");
    }
}

#[test]
fn a_loop_through_two_outputs_draws_both() {
    let (mut app, _, second) = two_tabs();
    let (a, b) = (
        add_on(&mut app, "output", second),
        add_on(&mut app, "output", second),
    );
    let source = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    // a draws the checkerboard mixed with b's frame; b draws a's frame.
    connect(&mut app, (source, "output"), (mix, "a"));
    connect(&mut app, (b, "frame"), (mix, "b"));
    connect(&mut app, (mix, "output"), (a, "input"));
    connect(&mut app, (a, "frame"), (b, "input"));

    assert!(drawn(&mut app, a) && drawn(&mut app, b));
    assert_eq!(app.draws(a), Some(Why::Feedback));
    assert_eq!(app.draws(b), Some(Why::Feedback));
}

#[test]
fn an_output_whose_frame_a_drawing_one_reads_draws() {
    let (mut app, first, second) = two_tabs();
    let hidden = picture_on(&mut app, second);
    let seen = add_on(&mut app, "output", first);
    connect(&mut app, (hidden, "frame"), (seen, "input"));

    assert!(
        drawn(&mut app, hidden),
        "its frame is sampled by one on screen"
    );
    assert_eq!(app.draws(hidden), Some(Why::Frame(seen)));

    // Read by an idle one instead, nothing is seen and neither draws.
    app.activate(Active::Project);
    assert!(!drawn(&mut app, hidden) && !drawn(&mut app, seen));
}

#[test]
fn a_deck_claimed_output_on_a_tab_not_looked_at_draws() {
    let (mut app, _, second) = two_tabs();
    let there = picture_on(&mut app, second);
    assert!(!drawn(&mut app, there));

    app.show_on(Channel::A, there);
    assert!(drawn(&mut app, there), "on air");
    assert_eq!(app.draws(there), Some(Why::Deck));
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: &str) {
    app.apply(Command::SetOption {
        node,
        key,
        value: value.to_string(),
    })
    .unwrap();
}

/// An Output published over Syphon is on air in another app: it draws with nothing on screen
/// showing it, closed tab and all, and the publisher is asked for it under its name and looks.
/// On a machine without Syphon the option a Mac's project carries publishes nothing and wakes
/// nothing.
#[test]
fn an_output_published_over_syphon_draws_on_a_tab_nobody_looks_at() {
    let (mut app, _, second) = two_tabs();
    let there = picture_on(&mut app, second);
    assert!(!drawn(&mut app, there));
    assert!(app.sent_wanted().is_empty(), "nothing is switched on");

    set(&mut app, there, "syphon", "on");
    set(&mut app, there, "transparent", "on");
    if !supersilvia::platform::syphon::available() {
        assert!(!drawn(&mut app, there), "a Mac's Syphon wakes nothing here");
        assert!(app.sent_wanted().is_empty(), "and publishes nothing");
        return;
    }
    assert!(drawn(&mut app, there), "published");
    assert_eq!(app.draws(there), Some(Why::Sent));
    let wanted = app.sent_wanted();
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].via, Via::Syphon);
    assert_eq!(wanted[0].name, format!("supersilvia Output {}", there.0));
    assert!(wanted[0].look.transparent && !wanted[0].look.flip);

    app.close_workspace(second);
    assert!(
        drawn(&mut app, there),
        "a closed tab does not take it off the air"
    );
    assert_eq!(app.draws(there), Some(Why::Sent));

    app.set_syphon_mix(true);
    let names: Vec<String> = app.sent_wanted().into_iter().map(|w| w.name).collect();
    assert_eq!(
        names,
        [format!("supersilvia Output {}", there.0), "Mix".to_string()]
    );

    set(&mut app, there, "syphon", "off");
    assert!(
        !drawn(&mut app, there),
        "suspended again once nothing publishes it"
    );
    assert_eq!(app.sent_wanted().len(), 1, "the mix alone");
}

/// An Output sent over NDI is on air on another machine, as a Syphon one is on this: drawn with
/// its tab closed, under `supersilvia Output <id>`, its Alpha shared with Syphon and Flip not,
/// declaring the synth's rate; the mix's NDI mark is `supersilvia Mix`.
#[test]
fn an_output_sent_over_ndi_draws_on_a_tab_nobody_looks_at() {
    let (mut app, _, second) = two_tabs();
    let there = picture_on(&mut app, second);
    set(&mut app, there, "ndi", "on");
    set(&mut app, there, "syphonFlip", "on");
    set(&mut app, there, "transparent", "on");
    app.close_workspace(second);
    assert!(drawn(&mut app, there), "sent, with its tab closed");
    assert_eq!(app.draws(there), Some(Why::Sent));
    let wanted = app.sent_wanted();
    assert_eq!(wanted.len(), 1, "NDI alone: {wanted:?}");
    let Via::Ndi { rate } = wanted[0].via else {
        panic!("sent over NDI: {wanted:?}");
    };
    assert!(rate > 0, "a rate declared");
    assert_eq!(wanted[0].name, format!("supersilvia Output {}", there.0));
    assert!(wanted[0].look.transparent, "Transparent is shared");
    assert!(!wanted[0].look.flip, "Flip is Syphon's alone");

    set(&mut app, there, "syphon", "on");
    let vias: Vec<Via> = app.sent_wanted().into_iter().map(|w| w.via).collect();
    if supersilvia::platform::syphon::available() {
        assert_eq!(vias, [Via::Syphon, Via::Ndi { rate }], "both ways at once");
    } else {
        assert_eq!(vias, [Via::Ndi { rate }], "no Syphon here");
    }

    app.set_ndi_mix(true);
    let last = app.sent_wanted().pop().expect("the mix");
    assert_eq!(last.name, "supersilvia Mix");

    // Renamed on air: the one name, both ways, and the mix's own is not to be had.
    set(&mut app, there, "sendName", "warpzone");
    let names: Vec<String> = app.sent_wanted().into_iter().map(|w| w.name).collect();
    assert!(
        names
            .iter()
            .all(|n| n == "warpzone" || n == "supersilvia Mix"),
        "{names:?}"
    );
    assert_eq!(
        names.iter().filter(|n| *n == "warpzone").count(),
        vias.len()
    );
    set(&mut app, there, "sendName", "supersilvia Mix");
    assert_eq!(app.sent_wanted()[0].name, "supersilvia Mix copy");
    set(&mut app, there, "sendName", "");
    assert_eq!(
        app.sent_wanted()[0].name,
        format!("supersilvia Output {}", there.0),
        "empty goes back to the default"
    );

    set(&mut app, there, "syphon", "off");
    set(&mut app, there, "ndi", "off");
    assert!(!drawn(&mut app, there), "suspended once nothing sends it");
}

/// A tap that reaches no Output is measured by its workspace's pass, and a pass whose
/// reading a CPU node reads draws, looked at or not — with no Output drawn for it.
#[test]
fn a_tap_that_feeds_a_cpu_node_is_measured_on_a_tab_not_looked_at() {
    let (mut app, _, second) = two_tabs();
    let out = picture_on(&mut app, second);
    let source = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (tap, "input"));
    app.build_frame_job();
    assert_eq!(
        app.pass_measures(second),
        [tap],
        "its workspace's pass holds it"
    );
    assert!(
        !measuring(&mut app, second),
        "and nothing reads the reading yet"
    );

    let slew = add_on(&mut app, "slew", second);
    connect(&mut app, (tap, "mean"), (slew, "input"));
    assert!(measuring(&mut app, second), "a CPU node reads it");
    assert!(!drawn(&mut app, out), "and no Output draws to measure it");
}

/// A deck on a closed tab reads a tap's reading; the tap is awake because the deck reads it,
/// so its workspace keeps a pass for it, closed or not.
#[test]
fn a_tap_feeding_a_deck_on_a_closed_tab_is_measured_by_its_workspaces_pass() {
    let (mut app, _, second) = two_tabs();
    let source = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (tap, "input"));
    let deck = add_on(&mut app, "output", second);
    let pattern = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    connect(&mut app, (pattern, "output"), (mix, "a"));
    connect(&mut app, (tap, "mean"), (mix, "amount"));
    connect(&mut app, (mix, "output"), (deck, "input"));
    app.show_on(Channel::A, deck);
    app.close_workspace(second);

    assert!(drawn(&mut app, deck), "on air");
    assert!(measuring(&mut app, second), "and what it reads is measured");
    assert_eq!(app.pass_measures(second), [tap]);
    let job = app.build_frame_job();
    let pass = job
        .passes
        .iter()
        .find(|p| p.key.workspace == second)
        .unwrap();
    assert!(
        pass.region.is_some(),
        "a closed tab shows no thumbnails: only the measurement is drawn"
    );
}

/// The same, with the tap on an Output's own chain — and read through a dual node the tick
/// evaluates, which is only a formula of the reading. The Output it is cabled into need not
/// draw: the pass measures the tap's input itself.
#[test]
fn a_tap_on_an_outputs_chain_whose_reading_reaches_a_cpu_node_is_measured() {
    let (mut app, _, second) = two_tabs();
    let out = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (tap, "input"));
    connect(&mut app, (tap, "output"), (out, "input"));
    assert!(!measuring(&mut app, second));

    let add = add_on(&mut app, "add", second);
    let slew = add_on(&mut app, "slew", second);
    connect(&mut app, (tap, "mean"), (add, "a"));
    connect(&mut app, (add, "output"), (slew, "input"));
    assert!(measuring(&mut app, second));
    assert!(!drawn(&mut app, out));
}

/// A reading a drawing Output's shader uses is measured every tick, by its pass — here on a
/// tab not looked at, read by an Output on the one that is.
#[test]
fn a_tap_read_by_a_drawing_outputs_shader_is_measured_every_tick() {
    let (mut app, first, second) = two_tabs();
    let source = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (tap, "input"));
    assert!(!measuring(&mut app, second));

    let seen = add_on(&mut app, "output", first);
    let pattern = add_on(&mut app, "checkerboard", first);
    let mix = add_on(&mut app, "mix", first);
    connect(&mut app, (pattern, "output"), (mix, "a"));
    connect(&mut app, (tap, "mean"), (mix, "amount"));
    connect(&mut app, (mix, "output"), (seen, "input"));
    assert!(
        measuring(&mut app, second),
        "the picture on screen reads what it measures"
    );
}

/// A tap shown on the tab being looked at shows its reading on its own row, so it is
/// measured — by the pass of the first open tab it is on, which is that one here.
#[test]
fn a_tap_on_the_tab_being_looked_at_is_measured() {
    let (mut app, first, second) = two_tabs();
    let out = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (tap, "input"));
    connect(&mut app, (tap, "output"), (out, "input"));
    assert!(!measuring(&mut app, second));

    app.apply(Command::ShowOn {
        nodes: vec![tap],
        workspace: first,
    })
    .unwrap();
    app.build_frame_job();
    assert_eq!(
        app.pass_measures(first),
        [tap],
        "the first tab it is on in project order"
    );
    assert!(app.pass_measures(second).is_empty(), "and only there");
    assert!(measuring(&mut app, first), "its number is on screen");
    assert!(!drawn(&mut app, out), "and the Output it feeds stays idle");
}

/// A measurement's own memory reads it: an `autoexposure` slews toward its reading, so it is
/// measured on every tick whether or not anyone looks.
#[test]
fn an_autoexposure_is_measured_every_tick() {
    let (mut app, _, second) = two_tabs();
    let out = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let exposure = add_on(&mut app, "autoexposure", second);
    connect(&mut app, (source, "output"), (exposure, "input"));
    connect(&mut app, (exposure, "output"), (out, "input"));
    assert!(measuring(&mut app, second));
    assert!(!drawn(&mut app, out));
}

/// A reading cabled back upstream of what it measures closes a loop through the delayed
/// port, and a loop keeps every tick whether or not anyone looks.
#[test]
fn a_tap_whose_reading_closes_a_loop_is_measured_every_tick() {
    let (mut app, _, second) = two_tabs();
    let out = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (source, "output"), (mix, "a"));
    connect(&mut app, (mix, "output"), (tap, "input"));
    connect(&mut app, (tap, "output"), (out, "input"));
    assert!(!measuring(&mut app, second));

    connect(&mut app, (tap, "mean"), (mix, "amount"));
    assert!(
        measuring(&mut app, second),
        "the reading feeds what it measures"
    );
    assert!(
        !drawn(&mut app, out),
        "and the Output beside the loop stays idle"
    );
}

/// **A loop through a tap reading across two Outputs.** O2 draws a checkerboard mixed by a
/// tap's reading; the tap measures O2's frame. O2 at one tick depends on O2 at an earlier one
/// through a frame and a reading, neither of them an immediate edge, and O2 draws every tick
/// with the pass measuring it, on a tab nobody looks at, as a loop. O1, which only shows the
/// tap's pass-through, is not part of it.
#[test]
fn a_loop_through_a_frame_and_a_tap_reading_draws_as_a_loop() {
    let (mut app, _, second) = two_tabs();
    let o2 = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    connect(&mut app, (source, "output"), (mix, "a"));
    connect(&mut app, (mix, "output"), (o2, "input"));
    let o1 = add_on(&mut app, "output", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (o2, "frame"), (tap, "input"));
    connect(&mut app, (tap, "output"), (o1, "input"));
    assert!(!drawn(&mut app, o1) && !drawn(&mut app, o2));

    connect(&mut app, (tap, "mean"), (mix, "amount"));
    for _ in 0..3 {
        app.tick(FRAME);
        assert!(drawn(&mut app, o2), "every tick");
        assert!(
            measuring(&mut app, second),
            "and the pass measures it every tick"
        );
    }
    assert_eq!(app.draws(o2), Some(Why::Feedback));
    assert!(!drawn(&mut app, o1));
}

/// **A tap measuring a frame draws that frame.** O2 draws on a tab nobody looks at; a tap
/// measures its frame and a CPU node reads the reading, so O2 draws for the tap.
#[test]
fn a_tap_measuring_an_outputs_frame_draws_it() {
    let (mut app, _, second) = two_tabs();
    let o2 = picture_on(&mut app, second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (o2, "frame"), (tap, "input"));
    assert!(!drawn(&mut app, o2));

    let slew = add_on(&mut app, "slew", second);
    connect(&mut app, (tap, "mean"), (slew, "input"));
    assert!(drawn(&mut app, o2), "what the tap measures is drawn");
    assert_eq!(app.draws(o2), Some(Why::Tap(tap)));
    assert!(measuring(&mut app, second));
}

/// **A deck the synth claims draws what its picture reads.** An Output on a tab nobody looks
/// at reads a tap's reading nothing else reads; a press on its Show on A, answered by the
/// synth before the editor has replanned, draws the tap's pass beside it, so the deck reads
/// this tick's reading rather than a frozen one.
#[test]
fn a_deck_the_synth_claims_draws_the_pass_measuring_what_it_reads() {
    let (mut app, _, second) = two_tabs();
    let pattern = add_on(&mut app, "checkerboard", second);
    let tap = add_on(&mut app, "tap", second);
    connect(&mut app, (pattern, "output"), (tap, "input"));
    let deck = add_on(&mut app, "output", second);
    let source = add_on(&mut app, "checkerboard", second);
    let mix = add_on(&mut app, "mix", second);
    connect(&mut app, (source, "output"), (mix, "a"));
    connect(&mut app, (tap, "mean"), (mix, "amount"));
    connect(&mut app, (mix, "output"), (deck, "input"));
    app.publish_plan();
    app.tick(FRAME);
    assert_eq!(app.draws(deck), None);
    assert!(app.synth_drawing().is_empty(), "nothing is seen");
    assert!(!measuring(&mut app, second), "and nothing measured");

    app.press(PortRef::new(deck, "show_a"), true);
    app.tick(FRAME);
    assert_eq!(app.draws(deck), None, "the editor has not replanned");
    assert!(
        app.synth_drawing().contains(&deck),
        "the synth draws the deck it claimed"
    );
    let job = app.synth_job();
    assert!(
        job.passes
            .iter()
            .any(|p| p.key.workspace == second && p.draws),
        "and the pass measuring the reading the deck's picture reads"
    );
}
