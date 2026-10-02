// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: MIDI — the map, the scaling, where a knob lands in the undo history, and the
//! world going on while the editor does not.
//!
//! **No device is opened.** `App::headless` starts no sequencer and no thread; a message is
//! posted through `App::apply_midi`, which is the editor's own seam into the same queue the
//! reader thread posts to. So every rule here is checked on a box with nothing plugged in,
//! and `cargo test` never touches `/dev/snd`.
//!
//! **A message is applied by the tick, not by the frame.** So the shape of a test here is
//! post-then-step: [`send`] does both, and the tests that model a minimized editor step the
//! synth by itself with `App::step_synth` and take the snapshot once at the end.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId};
use supersilvia::midi::{Device, Kind, Message, Target, Trigger, Wire};
use supersilvia::{App, Command};

fn add(app: &mut App, slug: &'static str, workspace: WorkspaceId) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .expect("the registry's own slug");
    app.graph().iter().map(|(id, _)| id).max().expect("added")
}

fn cc(channel: u8, cc: u8, value: u8) -> Message {
    Message {
        channel,
        kind: Kind::Control { cc, value },
    }
}

fn note(channel: u8, note: u8, on: bool) -> Message {
    Message {
        channel,
        kind: Kind::Note { note, on },
    }
}

/// A frame's worth of time, which is the step every test here takes.
const FRAME: f32 = 1.0 / 60.0;

/// A message arrives and the tick that reads it runs, with the editor running around it.
fn send(app: &mut App, message: Message) {
    app.apply_midi(message);
    app.tick(FRAME);
}

/// What the synth's own copy of the graph says a control is: what the tick drew from, which
/// is ahead of the document while a knob has moved and the editor has not run.
fn ticked(app: &App, port: PortRef) -> f32 {
    match app
        .ticked()
        .expect("the synth is inline")
        .control(port)
        .expect("a control")
    {
        ControlValue::Float(f) => f,
        ControlValue::Color(_) => panic!("{} is a color", port.key),
    }
}

fn float(app: &App, node: NodeId, key: &str) -> f32 {
    match app.graph().get(node).expect("the node").controls[key] {
        ControlValue::Float(f) => f,
        ControlValue::Color(_) => panic!("{key} is a color"),
    }
}

/// A binding is addressed by channel *and* number, which is where silvia is loose: it keys
/// its map by the CC alone, so two controllers collide the moment both send CC 7.
#[test]
fn a_channel_is_part_of_the_address() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = add(&mut app, "checkerboard", ws);
    let b = add(&mut app, "checkerboard", ws);

    app.learn_midi(PortRef::new(a, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.learn_midi(PortRef::new(b, "frequency"));
    send(&mut app, cc(1, 7, 0));

    send(&mut app, cc(0, 7, 127));
    let moved_a = float(&app, a, "frequency");
    let still_b = float(&app, b, "frequency");
    send(&mut app, cc(1, 7, 127));

    assert_ne!(
        float(&app, b, "frequency"),
        still_b,
        "channel 2 moved its own control"
    );
    assert_eq!(
        float(&app, a, "frequency"),
        moved_a,
        "and left channel 1's where it was"
    );
}

/// A CC arrives as 0–127 and lands across the control's **own** range — the instance's where
/// it has one, which is the reason an instance may narrow a range at all.
#[test]
fn a_cc_lands_across_the_controls_own_range() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";

    app.apply(Command::SetRange {
        node,
        key,
        range: supersilvia::graph::ControlRange {
            min: 2.0,
            max: 4.0,
            step: 0.01,
        },
    })
    .expect("a range inside the definition's");

    app.learn_midi(PortRef::new(node, key));
    send(&mut app, cc(0, 7, 64));

    send(&mut app, cc(0, 7, 0));
    assert!(
        (float(&app, node, key) - 2.0).abs() < 1e-3,
        "zero is the narrowed floor, not the definition's"
    );

    send(&mut app, cc(0, 7, 127));
    assert!(
        (float(&app, node, key) - 4.0).abs() < 1e-3,
        "and 127 is the narrowed ceiling"
    );

    send(&mut app, cc(0, 7, 64));
    let half = float(&app, node, key);
    assert!(
        (half - 3.0).abs() < 0.05,
        "halfway across the fader is halfway across the range, not {half}"
    );
}

/// A note reaches an action input by the same press a button sends, so a hand and a
/// controller are one source to the node underneath.
#[test]
fn a_note_presses_an_action_input() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = add(&mut app, "output", ws);
    let port = PortRef::new(out, "show_a");

    app.learn_midi(port);
    send(&mut app, note(0, 60, true));
    assert_eq!(app.mixer().a, None, "learning is not also a press");

    send(&mut app, note(0, 60, true));
    assert_eq!(app.mixer().a, Some(out), "the note went down");

    send(&mut app, note(0, 60, false));
    assert_eq!(app.mixer().a, Some(out), "and letting go changes nothing");
}

/// A burst of CC is **one** undo step, and it closes when the knob stops.
///
/// A drag ends on the pointer coming up; a hardware knob has no release, so without the
/// silence the step would stay open and swallow whatever was edited next.
#[test]
fn a_knob_is_one_undo_step_that_closes_on_silence() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    app.learn_midi(PortRef::new(node, key));
    send(&mut app, cc(0, 7, 0));
    let before = app.undo_len();
    let started = float(&app, node, key);

    // A knob turning: many messages, none of them far apart.
    for value in (0..40).map(|n| u8::try_from(n * 3).expect("under 127")) {
        app.apply_midi(cc(0, 7, value));
    }
    app.tick(FRAME);
    assert_eq!(
        app.undo_len(),
        before + 1,
        "forty messages are one gesture, not forty steps"
    );

    // The hand comes off it.
    app.settle_midi_at(10.0);
    let moved = float(&app, node, key);
    assert_ne!(moved, started);

    // A second turn is a second step, and undoing it leaves the first one's value.
    send(&mut app, cc(0, 7, 10));
    assert_eq!(app.undo_len(), before + 2, "the next turn opened its own");
    app.undo();
    assert!(
        (float(&app, node, key) - moved).abs() < 1e-3,
        "one undo took back the second turn and not the first"
    );
}

/// The map rides in `project.ssp`, which is what the tier test says: a binding names a node
/// in *this* project, so it is neither a preference nor a workspace's.
#[test]
fn the_map_round_trips_through_the_project_file() {
    let root = std::env::temp_dir().join(format!("ssw-midi-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(node, "frequency"));
    send(&mut app, cc(3, 74, 0));
    app.new_project(root.clone());

    // `new_project` starts an empty one, so the binding is made after it and saved from there.
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(node, "frequency"));
    send(&mut app, cc(3, 74, 0));
    app.save_project().unwrap();

    let (project, graph, warnings) =
        supersilvia::project::Project::open(root).expect("what was just written");
    assert!(warnings.is_empty(), "{warnings:?}");
    let bindings = project.midi();
    assert_eq!(bindings.len(), 1);
    let trigger = Trigger::Control { channel: 3, cc: 74 };
    let target = bindings
        .get(trigger)
        .expect("the binding")
        .target
        .port()
        .expect("a port");
    assert_eq!(target.key, "frequency");
    assert!(
        graph.get(target.node).is_some(),
        "and it names a node the file brought back"
    );
}

/// A binding whose node is deleted is not written to the file, and undoing the delete brings
/// the knob back with the node: the map keeps it, and only the live half of it drives, lists
/// or saves.
#[test]
fn undoing_a_delete_brings_its_binding_back() {
    let root = std::env::temp_dir().join(format!("ssw-midi-undo-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let mut app = App::headless();
    app.new_project(root.clone());
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let port = PortRef::new(node, "frequency");
    app.learn_midi(port);
    send(&mut app, cc(0, 7, 0));
    let started = float(&app, node, "frequency");

    app.apply(Command::RemoveNodes(vec![node]))
        .expect("a node that is there");
    // On the frame it went, beside the deck that was showing it.
    app.tick(FRAME);
    app.save_project().unwrap();
    let (saved, _, warnings) =
        supersilvia::project::Project::open(root).expect("what was just written");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(
        saved.midi().is_empty(),
        "the file does not carry a binding to a node it does not hold"
    );

    assert!(app.undo());
    assert_eq!(
        app.midi_trigger_of(port),
        Some(Trigger::Control { channel: 0, cc: 7 }),
        "the node came back with its knob"
    );
    send(&mut app, cc(0, 7, 127));
    assert_ne!(
        float(&app, node, "frequency"),
        started,
        "and the knob drives it again"
    );
}

/// Two knobs turned together, as a hand does on a desk, are one undo step for the stretch of
/// movement — not a step per knob per frame, which emptied the ring in two seconds.
#[test]
fn two_knobs_turned_together_are_one_undo_step() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = add(&mut app, "checkerboard", ws);
    let b = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(a, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.learn_midi(PortRef::new(b, "frequency"));
    send(&mut app, cc(0, 8, 0));
    let started = (float(&app, a, "frequency"), float(&app, b, "frequency"));
    app.settle_midi_at(1000.0);
    let before = app.undo_len();

    for v in 1..=30u8 {
        app.apply_midi(cc(0, 7, v * 4));
        app.apply_midi(cc(0, 8, 127 - v * 4));
        app.tick(FRAME);
    }
    assert_eq!(app.undo_len(), before + 1, "thirty frames of two knobs");

    // Still, and then one of them again: the next stretch is the next step.
    app.settle_midi_at(2000.0);
    send(&mut app, cc(0, 7, 5));
    assert_eq!(app.undo_len(), before + 2);

    app.undo();
    app.undo();
    assert_eq!(
        (float(&app, a, "frequency"), float(&app, b, "frequency")),
        started,
        "two undos take both knobs back to where they began"
    );
}

/// Knobs turned without a pause cannot empty the undo ring: six hundred frames of two of them,
/// more than twice the ring's depth, are one step, and the edits before them are all still
/// there to undo.
#[test]
fn turning_knobs_cannot_empty_the_undo_ring() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = add(&mut app, "checkerboard", ws);
    let b = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(a, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.learn_midi(PortRef::new(b, "frequency"));
    send(&mut app, cc(0, 8, 0));
    app.settle_midi_at(1000.0);
    let before = app.undo_len();

    for i in 0..600u16 {
        let v = u8::try_from(i % 128).expect("under 128");
        app.apply_midi(cc(0, 7, v));
        app.apply_midi(cc(0, 8, 127 - v));
        app.tick(FRAME);
    }
    assert_eq!(
        app.undo_len(),
        before + 1,
        "six hundred frames of two knobs"
    );
}

/// A node dragged while a knob turns is one step, and one undo takes back both: the drag and
/// the knob's stretch of movement are one gesture, and a redo brings both back.
#[test]
fn a_drag_while_a_knob_turns_is_one_step() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let knob = add(&mut app, "checkerboard", ws);
    let dragged = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(knob, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.settle_midi_at(1000.0);
    let before = app.undo_len();
    let turned_from = float(&app, knob, "frequency");
    let dragged_from = app.graph().get(dragged).expect("the node").pos;

    for v in 1..=30u8 {
        app.apply(Command::MoveNodes {
            moves: vec![(dragged, Pos2::new(f32::from(v), 0.0))],
        })
        .expect("a node that is there");
        app.apply_midi(cc(0, 7, v * 4));
        app.tick(FRAME);
    }
    // The pointer comes up while the knob is still turning, and the knob turns on.
    app.end_gesture();
    send(&mut app, cc(0, 7, 125));
    app.settle_midi_at(2000.0);
    assert_eq!(
        app.undo_len(),
        before + 1,
        "one drag and one knob, together"
    );
    let turned_to = float(&app, knob, "frequency");
    let dragged_to = Pos2::new(30.0, 0.0);

    assert!(app.undo());
    assert_eq!(float(&app, knob, "frequency"), turned_from);
    assert_eq!(
        app.graph().get(dragged).expect("the node").pos,
        dragged_from,
        "one undo took back the drag with the knob"
    );
    assert!(app.redo());
    assert_eq!(float(&app, knob, "frequency"), turned_to);
    assert_eq!(app.graph().get(dragged).expect("the node").pos, dragged_to);

    // Both let go: the next edit is a step of its own.
    app.apply(Command::MoveNodes {
        moves: vec![(dragged, Pos2::new(99.0, 0.0))],
    })
    .expect("a node that is there");
    assert_eq!(app.undo_len(), before + 2);
}

/// A knob falling silent does not split a scrub: the pointer is still down, so the gesture
/// the knob joined carries on as the one step it is.
#[test]
fn a_knob_falling_silent_does_not_split_a_scrub() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let knob = add(&mut app, "checkerboard", ws);
    let scrubbed = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(knob, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.settle_midi_at(1000.0);
    send(&mut app, cc(0, 7, 90));
    let before = app.undo_len();

    let scrub = |app: &mut App, v: f32| {
        app.apply(Command::SetControl {
            node: scrubbed,
            key: "frequency",
            value: ControlValue::Float(v),
        })
        .expect("a control that is there");
    };
    scrub(&mut app, 3.0);
    app.settle_midi_at(2000.0);
    scrub(&mut app, 4.0);
    assert_eq!(app.undo_len(), before, "the scrub joined the knob's step");
    app.end_gesture();
    scrub(&mut app, 5.0);
    assert_eq!(
        app.undo_len(),
        before + 1,
        "and let go, the next is its own"
    );
}

/// `Escape` on a scrub drops the open step whole, so a knob turned while it was open goes
/// back with it, exactly as an undo of the step would take it, and no step is left behind.
#[test]
fn escape_on_a_scrub_takes_back_a_knob_turned_during_it() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let knob = add(&mut app, "checkerboard", ws);
    let scrubbed = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(knob, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.settle_midi_at(1000.0);
    let before = app.undo_len();
    let began = float(&app, scrubbed, "frequency");
    let turned_from = float(&app, knob, "frequency");

    app.apply(Command::SetControl {
        node: scrubbed,
        key: "frequency",
        value: ControlValue::Float(began + 2.0),
    })
    .expect("a control that is there");
    send(&mut app, cc(0, 7, 90));
    assert_ne!(float(&app, knob, "frequency"), turned_from);

    assert!(app.cancel_control_drag(scrubbed, "frequency", began));
    assert_eq!(float(&app, scrubbed, "frequency"), began);
    assert_eq!(float(&app, knob, "frequency"), turned_from);
    assert_eq!(app.undo_len(), before, "nothing left in the ring");
}

/// The thing the whole seam is for: a knob turned while the editor is not running moves the
/// world on the tick the message arrived, and the document catches up in one undo step when
/// the editor runs again.
#[test]
fn a_knob_moves_the_world_while_the_editor_is_not_running() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    let port = PortRef::new(node, key);
    app.learn_midi(port);
    send(&mut app, cc(0, 7, 0));
    let before = app.undo_len();
    let started = float(&app, node, key);

    // Minimized: the synth ticks, nothing takes the snapshot and nothing reconciles.
    let mut applied = started;
    for value in [20u8, 60, 100, 127] {
        app.apply_midi(cc(0, 7, value));
        app.step_synth(FRAME);
        let now = ticked(&app, port);
        assert_ne!(now, applied, "the tick that read the message applied it");
        applied = now;
    }
    assert_eq!(
        float(&app, node, key),
        started,
        "and the document has heard none of it"
    );

    // The editor comes back.
    app.tick(FRAME);
    assert!(
        (float(&app, node, key) - applied).abs() < 1e-3,
        "the document caught up to where the knob left it"
    );
    assert_eq!(
        app.undo_len(),
        before + 1,
        "four ticks of knob are one undo step, not four"
    );
    app.settle_midi_at(100.0);
    assert_eq!(app.undo_len(), before + 1, "and the silence closed it");
}

/// A graph the editor built **before** it saw a knob turn must not undo the turn.
///
/// The decks' own rule, on controls: the synth adopts an arriving graph's value only where it
/// differs from the one the editor last sent, so a write it made since then survives — and
/// stops surviving the moment the editor's own value says it has been answered for.
#[test]
fn a_graph_built_before_the_turn_does_not_undo_it() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    let port = PortRef::new(node, key);
    app.learn_midi(port);
    send(&mut app, cc(0, 7, 0));

    // The knob moves with nobody watching.
    app.apply_midi(cc(0, 7, 127));
    app.step_synth(FRAME);
    let turned = ticked(&app, port);
    assert_ne!(
        turned,
        float(&app, node, key),
        "the editor has not heard yet"
    );

    // An edit the editor makes from the graph it still believes in, which republishes that
    // graph — with the knob's control at the value it had before the turn.
    add(&mut app, "checkerboard", ws);
    app.step_synth(FRAME);
    assert!(
        (ticked(&app, port) - turned).abs() < 1e-3,
        "a stale graph did not take the turn back"
    );

    // And now the editor reconciles, republishes, and the two agree.
    app.tick(FRAME);
    app.tick(FRAME);
    assert!(
        (float(&app, node, key) - turned).abs() < 1e-3,
        "the document is where the knob left it"
    );
    assert!(
        (ticked(&app, port) - turned).abs() < 1e-3,
        "and the tick is drawing from the same value"
    );
}

/// A note bound to an action input fires inside the tick it arrived on, with no editor.
#[test]
fn a_note_while_the_editor_is_not_running_fires_that_tick() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = add(&mut app, "output", ws);
    let port = PortRef::new(out, "show_a");
    app.learn_midi(port);
    send(&mut app, note(0, 60, true));
    assert_eq!(app.mixer().a, None, "learning is not also a press");

    app.apply_midi(note(0, 60, true));
    app.step_synth(FRAME);
    assert_eq!(
        app.ticked().expect("the synth is inline").decks().0,
        Some(out),
        "the mix being rendered cut to it on the tick the note arrived"
    );
    assert_eq!(app.mixer().a, None, "the project's mixer has not heard yet");

    app.tick(FRAME);
    assert_eq!(
        app.mixer().a,
        Some(out),
        "and it catches up when the editor runs"
    );
}

/// A long sweep of one knob must not push another knob's last value off the edge.
///
/// The write that crosses is one entry per control rather than a window of writes: whatever
/// happens on B while the editor is away, A's last value is still there when it comes back.
/// A window would have lost it *and* gone on restoring it against every graph, so the world
/// and the file would have disagreed for the rest of the session.
#[test]
fn a_busy_knob_does_not_evict_a_quiet_ones_last_value() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = add(&mut app, "checkerboard", ws);
    let b = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(a, "frequency"));
    send(&mut app, cc(0, 7, 0));
    app.learn_midi(PortRef::new(b, "frequency"));
    send(&mut app, cc(0, 8, 0));

    // A is turned once and then left alone; B is swept for far longer than any window.
    app.apply_midi(cc(0, 7, 127));
    app.step_synth(FRAME);
    let left_at = ticked(&app, PortRef::new(a, "frequency"));
    for value in (0..300).map(|n| u8::try_from(n % 128).expect("under 128")) {
        app.apply_midi(cc(0, 8, value));
    }
    app.step_synth(FRAME);

    app.tick(FRAME);
    assert!(
        (float(&app, a, "frequency") - left_at).abs() < 1e-3,
        "the quiet knob reached the document"
    );
}

/// The hand is later than the write and wins.
///
/// A write the editor has not read yet is in a snapshot it is about to take; dragging the
/// same control in between must not be undone by that write a frame later.
#[test]
fn a_hand_on_the_control_beats_a_write_the_editor_had_not_read() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    app.learn_midi(PortRef::new(node, key));
    send(&mut app, cc(0, 7, 0));

    // The knob moves, and the editor does not run.
    app.apply_midi(cc(0, 7, 127));
    app.step_synth(FRAME);

    // The hand moves the same control, through the bus, before any frame read that write.
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(7.5),
    })
    .expect("a control that is there");

    for _ in 0..4 {
        app.tick(FRAME);
    }
    assert!(
        (float(&app, node, key) - 7.5).abs() < 1e-3,
        "the drag stands, rather than being taken back by the older write"
    );
    assert!(
        (ticked(&app, PortRef::new(node, key)) - 7.5).abs() < 1e-3,
        "and the tick agrees with it"
    );
}

/// Learning binds from a message read **after** the asking, never from one the tick had
/// already driven a control with — which would move the old target and bind in one gesture.
#[test]
fn learning_binds_from_the_first_message_after_the_asking() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let a = add(&mut app, "checkerboard", ws);
    let b = add(&mut app, "checkerboard", ws);
    app.learn_midi(PortRef::new(a, "frequency"));
    send(&mut app, cc(0, 7, 0));

    // A tick reads the message and drives A with it. *Then* the hand asks to learn: the
    // message is on the snapshot the next frame takes, unseen, and binding from it would move
    // A and bind B in one gesture.
    app.apply_midi(cc(0, 7, 127));
    app.step_synth(FRAME);
    app.learn_midi(PortRef::new(b, "frequency"));
    app.tick(FRAME);
    assert_eq!(
        app.midi_trigger_of(PortRef::new(b, "frequency")),
        None,
        "a message already driven is not the one that binds"
    );

    send(&mut app, cc(0, 9, 64));
    assert_eq!(
        app.midi_trigger_of(PortRef::new(b, "frequency")),
        Some(Trigger::Control { channel: 0, cc: 9 }),
        "the next one is"
    );
}

/// Two notes bound to one action input are two hands on one button: the first to be let go
/// does not release it while the other is still down.
#[test]
fn two_notes_on_one_target_hold_it_until_both_let_go() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = add(&mut app, "output", ws);
    let port = PortRef::new(out, "show_a");
    // The learn gesture moves a binding rather than doubling it, so the pair comes the one
    // way a pair can: a file's rows, which keep both. See `Bindings::bind_unchecked`.
    for note in [60, 62] {
        app.bind_midi_unchecked(Trigger::Note { channel: 0, note }, port);
    }
    app.tick(FRAME);

    send(&mut app, note(0, 60, true));
    send(&mut app, note(0, 62, true));
    assert!(app.is_held(port), "both keys are down");

    send(&mut app, note(0, 60, false));
    assert!(
        app.is_held(port),
        "one let go, the other is still holding it"
    );

    send(&mut app, note(0, 62, false));
    assert!(!app.is_held(port), "and now it comes up");
}

/// Writes arriving over several frames are still one undo step: the gesture closes on
/// silence, and a knob turning is not silent.
#[test]
fn writes_over_several_frames_are_one_undo_step() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    app.learn_midi(PortRef::new(node, key));
    send(&mut app, cc(0, 7, 0));
    let before = app.undo_len();

    for value in [10u8, 40, 70, 100, 127] {
        send(&mut app, cc(0, 7, value));
    }
    assert_eq!(
        app.undo_len(),
        before + 1,
        "five frames of a turning knob are one step"
    );
    app.settle_midi_at(100.0);
    send(&mut app, cc(0, 7, 20));
    assert_eq!(app.undo_len(), before + 2, "and the next turn is the next");
}

/// The Main Mixer's fade is the one control that is not a node's port and still answers to
/// a knob: −1 at 0, +1 at 127, moved on the tick the fader moved and landed on the editor's
/// mixer a frame later — directly, since moving the fade is playing rather than editing.
#[test]
fn a_cc_on_the_fade_moves_the_mixer_and_never_the_history() {
    let mut app = App::headless();
    let before = app.undo_len();
    app.learn_midi(Target::Balance);
    send(&mut app, cc(0, 1, 127));
    assert_eq!(
        app.midi_trigger_of(Target::Balance),
        Some(Trigger::Control { channel: 0, cc: 1 }),
        "the first message binds the fade"
    );
    assert_eq!(app.midi_learning(), None);

    send(&mut app, cc(0, 1, 127));
    app.tick(FRAME);
    assert!(
        (app.mixer().balance - 1.0).abs() < 1e-6,
        "127 is deck B alone, got {}",
        app.mixer().balance
    );
    send(&mut app, cc(0, 1, 0));
    app.tick(FRAME);
    assert!(
        (app.mixer().balance + 1.0).abs() < 1e-6,
        "0 is deck A alone, got {}",
        app.mixer().balance
    );
    send(&mut app, cc(0, 1, 64));
    app.tick(FRAME);
    assert!(
        app.mixer().balance.abs() < 0.02,
        "the middle of the fader is the middle of the fade, got {}",
        app.mixer().balance
    );
    assert_eq!(app.undo_len(), before, "the fade is played, not edited");
}

/// A hand on the fade is later than a fader's write the synth made before it saw the move,
/// and the hand wins — the rule every bound control keeps.
#[test]
fn a_hand_on_the_fade_beats_a_stale_write() {
    let mut app = App::headless();
    app.learn_midi(Target::Balance);
    send(&mut app, cc(0, 1, 0));
    send(&mut app, cc(0, 1, 127));
    app.tick(FRAME);
    assert!((app.mixer().balance - 1.0).abs() < 1e-6, "the fader landed");

    app.set_balance(0.25);
    app.tick(FRAME);
    app.tick(FRAME);
    assert!(
        (app.mixer().balance - 0.25).abs() < 1e-6,
        "the hand's move stands, got {}",
        app.mixer().balance
    );
    assert!(
        (app.build_frame_job().mixer.balance - 0.25).abs() < 1e-6,
        "and the mix is drawn where the hand put it"
    );
}

/// The fade's binding rides in the project file beside the ports', as `{"mixer":
/// "balance"}`, and comes back without a node to resolve against.
#[test]
fn the_fades_binding_round_trips_through_the_project_file() {
    let root = std::env::temp_dir().join(format!("ssw-midi-fade-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let mut app = App::headless();
    app.new_project(root.clone());
    app.learn_midi(Target::Balance);
    send(&mut app, cc(4, 9, 0));
    app.save_project().unwrap();

    let text = std::fs::read_to_string(root.join("project.ssp")).expect("the manifest");
    assert!(text.contains(r#""mixer": "balance""#), "{text}");

    let (project, _graph, warnings) =
        supersilvia::project::Project::open(root).expect("what was just written");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        project.midi().trigger_of(Target::Balance),
        Some(Trigger::Control { channel: 4, cc: 9 })
    );
}

/// A number a node draws in a region of its own is bound, driven and saved exactly as a
/// number on a row is: `cosinegradient`'s twelve and `euclideanrhythm`'s lane numbers are
/// hidden controls — no port — and the map addresses them all the same.
#[test]
fn a_regions_own_number_is_driven_and_saved_as_a_rows_is() {
    let root = std::env::temp_dir().join(format!("ssw-midi-region-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let mut app = App::headless();
    app.new_project(root.clone());
    let ws = app.graph().default_workspace();
    let palette = add(&mut app, "cosinegradient", ws);
    let lanes = add(&mut app, "euclideanrhythm", ws);
    for (node, key, number) in [(palette, "freqG", 20), (lanes, "lane2pulses", 21)] {
        app.learn_midi(PortRef::new(node, key));
        send(&mut app, cc(0, number, 0));
        let low = float(&app, node, key);
        send(&mut app, cc(0, number, 127));
        assert!(
            float(&app, node, key) > low,
            "the knob drives {key}: {low} to {}",
            float(&app, node, key)
        );
    }
    app.save_project().unwrap();

    let (project, _graph, warnings) =
        supersilvia::project::Project::open(root).expect("what was just written");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        project.midi().trigger_of(PortRef::new(palette, "freqG")),
        Some(Trigger::Control { channel: 0, cc: 20 }),
        "and the file keeps the binding"
    );
    assert_eq!(
        project
            .midi()
            .trigger_of(PortRef::new(lanes, "lane2pulses")),
        Some(Trigger::Control { channel: 0, cc: 21 }),
    );
}

/// An Output's render numbers are the node's own too, and are left unbound on purpose: a row
/// in a file naming one is dropped, as a row naming a control that is gone is.
#[test]
fn an_outputs_render_numbers_are_not_bindable() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let output = add(&mut app, "output", ws);
    let fps = PortRef::new(output, "fps");
    let started = float(&app, output, "fps");
    app.bind_midi_unchecked(Trigger::Control { channel: 0, cc: 30 }, fps);
    send(&mut app, cc(0, 30, 127));
    assert_eq!(
        float(&app, output, "fps"),
        started,
        "the knob drives nothing"
    );
}

/// A message from a device of the test's own numbering, and the tick that reads it.
fn from(app: &mut App, device: u64, message: Message) {
    app.apply_midi_wire(Wire::Message(Device(device), message));
    app.tick(FRAME);
}

/// **Blackout and Freeze are rig controls the map addresses, as the fade is.** A note bound
/// to one flips it on the tick it arrives, its note-off changes nothing, a CC holds it on at
/// 64 and above, and the mix is drawn the way the press says — and none of it is an edit.
#[test]
fn a_note_flips_blackout_and_a_cc_holds_freeze_and_neither_is_an_edit() {
    let mut app = App::headless();
    let before = app.undo_len();
    app.learn_midi(Target::Blackout);
    send(&mut app, note(0, 36, true));
    assert_eq!(
        app.midi_trigger_of(Target::Blackout),
        Some(Trigger::Note {
            channel: 0,
            note: 36
        }),
        "the first message binds Blackout"
    );
    assert!(!app.mixer().blackout, "learning is not also a press");
    send(&mut app, note(0, 36, false));

    send(&mut app, note(0, 36, true));
    assert!(app.mixer().blackout, "the pad went down: black");
    assert!(
        app.build_frame_job().mixer.blackout,
        "and the mix is drawn black"
    );
    send(&mut app, note(0, 36, false));
    assert!(app.mixer().blackout, "letting go of the pad holds it");
    send(&mut app, note(0, 36, true));
    assert!(!app.mixer().blackout, "and the next hit lets it go");
    assert!(!app.build_frame_job().mixer.blackout);

    app.learn_midi(Target::Freeze);
    send(&mut app, cc(0, 20, 0));
    send(&mut app, cc(0, 20, 127));
    assert!(app.mixer().freeze, "a CC at 127 holds Freeze");
    assert!(app.build_frame_job().mixer.freeze);
    send(&mut app, cc(0, 20, 63));
    assert!(!app.mixer().freeze, "and below 64 lets it go");
    assert_eq!(app.undo_len(), before, "played, not edited");
}

/// A hand on Blackout is later than a note's flip the synth made before it saw the press,
/// and the hand wins — the rule the fade keeps.
#[test]
fn a_hand_on_blackout_beats_a_stale_note() {
    let mut app = App::headless();
    app.bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 36,
        },
        Target::Blackout,
    );
    app.tick(FRAME);
    send(&mut app, note(0, 36, true));
    assert!(app.mixer().blackout);

    app.set_blackout(false);
    app.tick(FRAME);
    app.tick(FRAME);
    assert!(!app.mixer().blackout, "the hand's press stands");
    assert!(!app.build_frame_job().mixer.blackout, "and the mix is lit");
}

/// Blackout's and Freeze's bindings ride in the project file by name, as the fade's does.
#[test]
fn the_holds_bindings_round_trip_through_the_project_file() {
    let root = std::env::temp_dir().join(format!("ssw-midi-holds-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let mut app = App::headless();
    app.new_project(root.clone());
    app.learn_midi(Target::Blackout);
    send(&mut app, note(9, 36, true));
    app.learn_midi(Target::Freeze);
    send(&mut app, note(9, 37, true));
    app.save_project().unwrap();

    let text = std::fs::read_to_string(root.join("project.ssp")).expect("the manifest");
    assert!(text.contains(r#""mixer": "blackout""#), "{text}");
    assert!(text.contains(r#""mixer": "freeze""#), "{text}");
    let (project, _graph, _) = supersilvia::project::Project::open(root.clone()).unwrap();
    assert_eq!(
        project.midi().trigger_of(Target::Freeze),
        Some(Trigger::Note {
            channel: 9,
            note: 37
        })
    );
    std::fs::remove_dir_all(&root).ok();
}

/// **A device that goes away lets go of what it held.** Its note-off will never come, so
/// without this an unplugged controller leaves the gate open for the rest of the run. Only
/// its own notes go: a key held on another controller still holds.
#[test]
fn a_device_that_goes_away_lets_go_of_its_notes() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = add(&mut app, "output", ws);
    let port = PortRef::new(out, "show_b");
    app.bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 60,
        },
        port,
    );
    app.tick(FRAME);

    from(&mut app, 1, note(0, 60, true));
    from(&mut app, 2, note(0, 60, true));
    assert!(app.is_held(port), "two controllers on one pad");

    app.apply_midi_wire(Wire::Gone(Device(1)));
    app.tick(FRAME);
    assert!(
        app.is_held(port),
        "the other controller is still holding it"
    );

    app.apply_midi_wire(Wire::Gone(Device(2)));
    app.tick(FRAME);
    assert!(!app.is_held(port), "and with both gone it comes up");
}

/// **Release all**, in the MIDI window: every action input a note holds comes up, for the
/// note-off that never came from a device that is still there.
#[test]
fn release_all_lets_go_of_every_held_note() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let out = add(&mut app, "output", ws);
    let (a, b) = (PortRef::new(out, "show_a"), PortRef::new(out, "show_b"));
    app.bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 60,
        },
        a,
    );
    app.bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 61,
        },
        b,
    );
    app.tick(FRAME);
    send(&mut app, note(0, 60, true));
    send(&mut app, note(0, 61, true));
    assert!(app.is_held(a) && app.is_held(b));

    app.release_midi_notes();
    app.tick(FRAME);
    assert!(!app.is_held(a) && !app.is_held(b), "both came up");

    send(&mut app, note(0, 60, true));
    assert!(app.is_held(a), "and a note pressed after it holds as ever");
}

/// **Soft takeover, a preference off by default.** On, a fader whose value disagrees with its
/// control moves nothing until it passes the control's value, and the control wears a ghost
/// where the fader is meanwhile; once it has passed, it drives the control and the ghost is
/// gone. Off, the first message writes.
#[test]
fn soft_takeover_waits_for_the_fader_to_pass_the_control() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let node = add(&mut app, "checkerboard", ws);
    let key = "frequency";
    let port = PortRef::new(node, key);
    app.apply(Command::SetRange {
        node,
        key,
        range: supersilvia::graph::ControlRange {
            min: 0.0,
            max: 127.0,
            step: 0.01,
        },
    })
    .unwrap();
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(64.0),
    })
    .unwrap();
    app.bind_midi_unchecked(Trigger::Control { channel: 0, cc: 7 }, port);
    assert!(
        !supersilvia::preferences::Preferences::default().midi_soft_takeover,
        "off by default"
    );
    app.set_soft_takeover(true);
    app.tick(FRAME);

    send(&mut app, cc(0, 7, 10));
    assert_eq!(float(&app, node, key), 64.0, "far below: nothing moves");
    assert_eq!(
        app.midi_ghost(port),
        Some(10.0),
        "and the ghost is the fader"
    );
    send(&mut app, cc(0, 7, 40));
    assert_eq!(float(&app, node, key), 64.0, "still short of it");
    assert_eq!(app.midi_ghost(port), Some(40.0));

    send(&mut app, cc(0, 7, 90));
    assert_eq!(float(&app, node, key), 90.0, "passed it: picked up");
    assert_eq!(app.midi_ghost(port), None, "and the ghost is gone");
    send(&mut app, cc(0, 7, 20));
    assert_eq!(float(&app, node, key), 20.0, "and it follows from there");

    // The control moved by hand leaves the fader behind again.
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(100.0),
    })
    .unwrap();
    app.tick(FRAME);
    send(&mut app, cc(0, 7, 21));
    assert_eq!(float(&app, node, key), 100.0, "the hand's value stands");
    assert_eq!(app.midi_ghost(port), Some(21.0));

    // Off, the fader writes at once and wears no ghost.
    app.set_soft_takeover(false);
    app.tick(FRAME);
    assert_eq!(app.midi_ghost(port), None, "off clears the ghost");
    send(&mut app, cc(0, 7, 22));
    assert_eq!(float(&app, node, key), 22.0, "off: the fader jumps it");
}

/// The fade keeps soft takeover too: a fader at the far end from the fade waits.
#[test]
fn soft_takeover_holds_the_fade_until_the_fader_meets_it() {
    let mut app = App::headless();
    app.bind_midi_unchecked(Trigger::Control { channel: 0, cc: 1 }, Target::Balance);
    app.set_soft_takeover(true);
    app.tick(FRAME);
    assert_eq!(app.mixer().balance, -1.0);

    send(&mut app, cc(0, 1, 127));
    assert_eq!(app.mixer().balance, -1.0, "the fade does not jump to B");
    assert_eq!(app.midi_ghost(Target::Balance), Some(1.0));
    send(&mut app, cc(0, 1, 0));
    assert_eq!(
        app.mixer().balance,
        -1.0,
        "passed it on the way down: picked up"
    );
    assert_eq!(app.midi_ghost(Target::Balance), None);
    send(&mut app, cc(0, 1, 127));
    assert_eq!(app.mixer().balance, 1.0, "and it drives the fade");
}
