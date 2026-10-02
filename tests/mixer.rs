// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the mixer — two decks and a fade, silvia's `MainMixer` and not a node.
//!
//! What is asserted here is the shape: an Output claims a deck through an action input that
//! is also a button, so a hand and a sequencer are the same source; a claim is playing the
//! instrument rather than editing the graph, so undo never touches it; and a deck that is
//! on air stays on air whatever its workspace's tab is doing. The pixels are layer 3's.

mod common;

use common::{add_on, connect};
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::mixer::{Channel, Method, Resolution};
use supersilvia::render::Display;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    add_on(app, slug, workspace)
}

/// A checkerboard into an Output, on one workspace.
fn patched_output(app: &mut App, workspace: WorkspaceId) -> NodeId {
    let source = add_on(app, "checkerboard", workspace);
    let out = add_on(app, "output", workspace);
    connect(app, (source, "output"), (out, "input"));
    out
}

#[test]
fn the_mixer_is_not_a_node() {
    assert!(
        supersilvia::nodes::find("mixer").is_none(),
        "the mixer is a render target with two decks, not a library node"
    );
    let out = supersilvia::nodes::find("output").unwrap();
    for key in ["show_a", "show_b"] {
        let input = out.input(key).unwrap_or_else(|| panic!("output has {key}"));
        assert_eq!(input.ty, supersilvia::graph::PortType::Action);
        assert!(
            matches!(input.control, supersilvia::nodes::Control::Press),
            "{key} is a button as well as a port"
        );
    }
}

#[test]
fn show_on_a_is_a_button_that_claims_the_deck() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = patched_output(&mut app, ws);
    assert_eq!(app.mixer().a, None);

    app.press(PortRef::new(out, "show_a"), true);
    app.tick(FRAME);
    assert_eq!(
        app.mixer().a,
        Some(out),
        "the hand went down: deck A is this Output"
    );
    assert_eq!(app.mixer().b, None);

    app.press(PortRef::new(out, "show_a"), false);
    app.tick(FRAME);
    assert_eq!(app.mixer().a, Some(out), "letting go changes nothing");

    app.press(PortRef::new(out, "show_b"), true);
    app.tick(FRAME);
    assert_eq!(
        app.mixer().decks_of(out),
        (true, true),
        "one Output may be on both decks, as in silvia"
    );
}

/// A claim happens on the frame the button goes down, not on every frame it is held. If it
/// were every frame, a finger resting on `Show on A` would take the deck back from a
/// sequencer that cut to something else.
#[test]
fn holding_the_button_claims_once() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let first = patched_output(&mut app, ws);
    let second = patched_output(&mut app, ws);

    app.press(PortRef::new(first, "show_a"), true);
    app.tick(FRAME);
    app.tick(FRAME);
    assert_eq!(app.mixer().a, Some(first));

    app.show_on(Channel::A, second);
    app.tick(FRAME);
    assert_eq!(
        app.mixer().a,
        Some(second),
        "a finger still resting on the first button does not take the deck back"
    );
}

#[test]
fn a_sequencer_cuts_between_two_outputs() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let first = patched_output(&mut app, ws);
    let second = patched_output(&mut app, ws);
    app.show_on(Channel::A, first);

    // A button node is the smallest sequencer: its trigger into the Output's action input.
    let button = add(&mut app, "button");
    connect(&mut app, (button, "trigger"), (second, "show_a"));

    app.tick(FRAME);
    assert_eq!(app.mixer().a, Some(first), "nothing has fired");

    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    assert_eq!(
        app.mixer().a,
        Some(second),
        "the edge arrived down the cable inside the tick it fired"
    );
}

#[test]
fn claiming_is_playing_not_editing() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = patched_output(&mut app, ws);
    let edits = app.history().len();
    let dirty = app.dirty();

    app.show_on(Channel::A, out);
    app.set_balance(0.3);
    app.set_method(Method::RadialWipe);
    assert_eq!(app.history().len(), edits, "not a command");
    assert_eq!(app.dirty(), dirty, "not an edit");

    // An edit made after the claim, undone: the graph goes back, the decks do not.
    add(&mut app, "checkerboard");
    assert!(app.undo());
    assert_eq!(app.mixer().a, Some(out), "undo never touches the decks");
    assert_eq!(app.mixer().balance, 0.3);
    assert_eq!(app.mixer().method, Method::RadialWipe);
}

#[test]
fn only_an_output_can_be_on_a_deck() {
    let mut app = App::headless();
    let source = add(&mut app, "checkerboard");
    app.show_on(Channel::A, source);
    assert_eq!(app.mixer().a, None);
    app.show_on(Channel::B, NodeId(99));
    assert_eq!(app.mixer().b, None);
}

#[test]
fn deleting_the_deck_output_empties_the_deck_and_undo_does_not_refill_it() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = patched_output(&mut app, ws);
    app.show_on(Channel::A, out);
    app.show_on(Channel::B, out);

    app.apply(Command::RemoveNodes(vec![out])).unwrap();
    app.tick(FRAME);
    assert_eq!(app.mixer().a, None, "the record was taken off");
    assert_eq!(app.mixer().b, None);

    assert!(app.undo());
    assert!(app.graph().get(out).is_some(), "the node is back");
    app.tick(FRAME);
    assert_eq!(
        app.mixer().a,
        None,
        "but the deck is not: putting an Output on air was never an edit"
    );
}

#[test]
fn the_frame_shows_the_mix_at_the_mixers_settings() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = patched_output(&mut app, ws);
    let b = patched_output(&mut app, ws);
    app.show_on(Channel::A, a);
    app.show_on(Channel::B, b);
    app.set_balance(0.25);
    app.set_method(Method::Checkerboard);
    app.set_mix_resolution(Resolution::Fixed(720, 1280));

    let job = app.build_frame_job();
    assert_eq!(job.display, Display::Mixer, "the preview is the mix");
    assert_eq!(job.mixer.a, Some(a));
    assert_eq!(job.mixer.b, Some(b));
    assert_eq!(job.mixer.balance, 0.25);
    assert_eq!(job.mixer.method, Method::Checkerboard);
    assert_eq!(job.mixer.resolution, (720, 1280));

    app.set_balance(9.0);
    assert_eq!(app.mixer().balance, 1.0, "the fade stops at its ends");
    app.set_mix_resolution(Resolution::Viewport);
    let (w, h) = app.build_frame_job().mixer.resolution;
    assert!(
        w > 0 && h > 0,
        "a viewport-matched mix has a size before any canvas has been drawn"
    );
}

/// Closing the live deck's tab to tidy the editor must not freeze the show.
#[test]
fn a_deck_on_a_closed_workspace_stays_on_air() {
    let mut app = App::headless();
    app.apply(Command::AddWorkspace {
        name: "Second".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        // These tests want a bare workspace; the app seeds a video tab with a patch.
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let other = app.graph().workspaces().last().unwrap().id;
    app.open_workspace(other);

    // A CPU node feeding the deck's graph, so the tick half is covered as well as the draw.
    let slew = add_on(&mut app, "slew", other);
    let source = add_on(&mut app, "checkerboard", other);
    let on_air = add_on(&mut app, "output", other);
    let offline = patched_output(&mut app, other);
    connect(&mut app, (slew, "output"), (source, "frequency"));
    connect(&mut app, (source, "output"), (on_air, "input"));
    app.show_on(Channel::A, on_air);

    app.close_workspace(other);
    let job = app.build_frame_job();
    let suspended = |id: NodeId| {
        job.outputs.iter().find(|o| o.node == id).unwrap().mode
            == supersilvia::render::OutputMode::Suspended
    };
    assert!(
        !suspended(on_air),
        "the deck's Output is drawn with its tab closed"
    );
    assert!(
        suspended(offline),
        "the other Output on that workspace is suspended"
    );

    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(4.0),
    })
    .unwrap();
    app.tick(FRAME);
    assert_eq!(
        app.uniform(PortRef::new(slew, "output")),
        Some(4.0),
        "everything upstream of the deck ticks too"
    );
}
