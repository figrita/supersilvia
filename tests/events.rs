// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: what a tick hands the editor once — a deck claim, a value it wrote onto its own
//! node, a MIDI message — lands once and in the order it happened, however many ticks the
//! editor missed.
//!
//! The shape of every test here is the review's: the synth ticks with nobody taking its
//! snapshot (`App::step_synth`), the editor takes one with no tick (`App::read_synth`), the
//! synth ticks, and the editor takes again. The buffer the editor never took comes back to
//! the synth between those, and whatever it carried must not land after what came later. Each
//! runs an even and an odd number of missed ticks, because which buffer holds what alternates.
//! `synth::thread`'s own tests hold the same for every kind of event, the probes, thumbnails
//! and Snaps a GPU would make included.

mod common;

use common::{add_on, picture_on};
use supersilvia::App;
use supersilvia::command::Command;
use supersilvia::graph::{ControlValue, NodeId, PortRef, Value};
use supersilvia::midi::{Kind, Message};

const FRAME: f32 = 1.0 / 60.0;

/// Ticks missed after the ones a test makes its events on: none, then one, so the run of
/// missed ticks is even in one case and odd in the other.
const TRAILING: [usize; 2] = [0, 1];

/// The editor misses these ticks, each `dt` long, takes once, the synth ticks once more, and
/// the editor takes again.
fn miss_then_catch_up(
    app: &mut App,
    dt: f32,
    ticks: &mut [&mut dyn FnMut(&mut App)],
    trailing: usize,
) {
    for tick in ticks.iter_mut() {
        tick(app);
        app.step_synth(dt);
    }
    for _ in 0..trailing {
        app.step_synth(dt);
    }
    app.read_synth();
    app.step_synth(dt);
    app.read_synth();
}

/// A newer deck claim is the one that stands, on the project's mixer and on the mix being
/// rendered, when the editor missed the tick of the older one.
#[test]
fn a_deck_claim_the_editor_missed_does_not_undo_a_newer_one() {
    for trailing in TRAILING {
        let mut app = App::headless();
        let ws = app.graph().default_workspace();
        let first = picture_on(&mut app, ws);
        let second = picture_on(&mut app, ws);
        app.tick(FRAME);
        let show = |out: NodeId| PortRef::new(out, "show_a");
        miss_then_catch_up(
            &mut app,
            FRAME,
            &mut [
                &mut |app: &mut App| app.press(show(first), true),
                &mut |app: &mut App| {
                    app.press(show(first), false);
                    app.press(show(second), true);
                },
            ],
            trailing,
        );
        for _ in 0..3 {
            app.tick(FRAME);
        }
        assert_eq!(
            app.mixer().a,
            Some(second),
            "the project's mixer, {trailing} trailing"
        );
        assert_eq!(
            app.ticked().expect("inline").decks().0,
            Some(second),
            "the mix being rendered, {trailing} trailing"
        );
    }
}

/// How long an automation's tick is here: its shortest Duration, and the longest tick the
/// clock allows.
const RECORDING: f32 = 0.1;

/// An automation at one tick a recording: a Duration of one [`RECORDING`] tick, so each press
/// of Record starts a recording and the tick's own timeout stops it, writing the curve.
fn one_tick_automation(app: &mut App) -> NodeId {
    let ws = app.graph().default_workspace();
    let node = add_on(app, "automation", ws);
    set(app, node, "duration", RECORDING);
    app.tick(FRAME);
    node
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .expect("a control");
}

/// Each recorded point's value, as the document holds the recording.
fn recorded(app: &App, node: NodeId) -> Vec<f32> {
    app.graph()
        .get(node)
        .and_then(|n| n.values.get("recording"))
        .and_then(Value::points)
        .expect("a recording")
        .iter()
        .map(|p| p.value)
        .collect()
}

/// Two values written onto one node end at the newer, in the document and so in the tick.
///
/// The two recordings are three ticks apart, since Record has to come up between two presses:
/// odd, so they are in different buffers when the editor misses them.
#[test]
fn two_value_writes_to_one_node_end_at_the_newer() {
    for trailing in TRAILING {
        let mut app = App::headless();
        let node = one_tick_automation(&mut app);
        let record = PortRef::new(node, "record");
        miss_then_catch_up(
            &mut app,
            RECORDING,
            &mut [
                &mut |app: &mut App| {
                    set(app, node, "input", 0.25);
                    app.press(record, true);
                },
                &mut |app: &mut App| app.press(record, false),
                &mut |_: &mut App| {},
                &mut |app: &mut App| {
                    set(app, node, "input", 0.75);
                    app.press(record, true);
                },
            ],
            trailing,
        );
        app.press(record, false);
        for _ in 0..3 {
            app.tick(FRAME);
        }
        assert_eq!(
            recorded(&app, node),
            [0.75],
            "the second recording, {trailing} trailing"
        );
    }
}

/// A recording followed by Clear ends cleared, in the document and so in the tick.
#[test]
fn a_recording_followed_by_clear_ends_cleared() {
    for trailing in TRAILING {
        let mut app = App::headless();
        let node = one_tick_automation(&mut app);
        let (record, clear) = (PortRef::new(node, "record"), PortRef::new(node, "clear"));
        miss_then_catch_up(
            &mut app,
            RECORDING,
            &mut [
                &mut |app: &mut App| {
                    set(app, node, "input", 0.5);
                    app.press(record, true);
                },
                &mut |app: &mut App| {
                    app.press(record, false);
                    app.press(clear, true);
                },
            ],
            trailing,
        );
        app.press(clear, false);
        for _ in 0..3 {
            app.tick(FRAME);
        }
        assert!(
            recorded(&app, node).is_empty(),
            "cleared, {trailing} trailing: {:?}",
            recorded(&app, node)
        );
    }
}

/// An editor away for longer than the synth keeps messages applies none of what is left,
/// rather than a run whose front is missing: a control being learned is not bound from the
/// middle of a sweep, and the first message after the loss binds it.
#[test]
fn an_overrun_applies_nothing_of_what_is_left() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add_on(&mut app, "checkerboard", ws);
    let port = PortRef::new(node, "frequency");
    app.tick(FRAME);
    app.learn_midi(port);
    let cc = |cc: u8| Message {
        channel: 0,
        kind: Kind::Control { cc, value: 64 },
    };
    // More than the synth keeps, all on one tick the editor misses.
    for _ in 0..=supersilvia::synth::events::KEPT {
        app.apply_midi(cc(7));
    }
    app.step_synth(FRAME);
    app.read_synth();
    assert_eq!(
        app.midi_learning(),
        Some(supersilvia::midi::Target::from(port)),
        "nothing of the overrun was learned from"
    );

    app.apply_midi(cc(9));
    app.tick(FRAME);
    assert_eq!(app.midi_learning(), None, "the next message is learned");
    assert!(
        app.midi_trigger_of(port).is_some(),
        "and bound to the control"
    );
}
