// SPDX-License-Identifier: AGPL-3.0-or-later

//! The recompile boundary. This is the property that makes the instrument playable: changing
//! what the graph *is* costs a shader rebuild, changing what it *does* costs a float.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::{App, Command, CommandError};

fn patch() -> (App, NodeId, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::new(300.0, 0.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    (app, cb, out)
}

#[test]
fn setting_a_control_never_recompiles() {
    let (mut app, cb, out) = patch();
    // Clear the pending rebuild from wiring the graph up.
    let _ = app.take_recompiles();
    assert!(!app.needs_recompile(out));

    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(40.0),
    })
    .unwrap();

    assert!(
        !app.needs_recompile(out),
        "a parameter edit must set a uniform, not rebuild a shader",
    );
}

/// A source node feeding an Output, for the tests about option kinds. `camera` and `video`
/// are where the kinds other than `Code` actually live.
fn source_into_output(slug: &'static str, port: &'static str) -> (App, NodeId, NodeId) {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    for (s, x) in [(slug, 0.0), ("output", 300.0)] {
        app.apply(Command::AddNode {
            slug: s,
            at: Pos2::new(x, 0.0),
            workspace,
        })
        .unwrap();
    }
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let (source, out) = (ids[0], ids[1]);
    app.apply(Command::Connect {
        from: PortRef::new(source, port),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let _ = app.take_recompiles();
    (app, source, out)
}

/// A `Code` option is in the emitted WGSL, so changing it rebuilds. `camera.mirror` flips the
/// sampling coordinate in the generator, which is as baked as a choice gets.
#[test]
fn setting_an_option_does_recompile() {
    let (mut app, camera, out) = source_into_output("camera", "frame");

    app.apply(Command::SetOption {
        node: camera,
        key: "mirror",
        value: "no".to_string(),
    })
    .unwrap();

    assert!(app.needs_recompile(out));
}

/// The exemption is carried by the kind, not by the node: one camera, two options, two
/// answers. `device` is read by `tick` and `mirror` by the generator.
#[test]
fn two_kinds_on_one_node_get_different_answers() {
    let (mut app, camera, out) = source_into_output("camera", "frame");

    app.apply(Command::SetOption {
        node: camera,
        key: "device",
        value: "/dev/video1".to_string(),
    })
    .unwrap();
    assert!(
        !app.needs_recompile(out),
        "a device is opened by tick, not compiled into a shader",
    );

    app.apply(Command::SetOption {
        node: camera,
        key: "mirror",
        value: "no".to_string(),
    })
    .unwrap();
    assert!(app.needs_recompile(out));
}

/// The other half of the boundary, on the kind that was added for it: an asset reference
/// changes which texture the renderer binds and not one character of WGSL, so swapping a clip
/// must cost nothing. A rebuild here is a stutter in the middle of a set.
#[test]
fn swapping_an_asset_never_recompiles() {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug: "video",
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::new(300.0, 0.0),
        workspace,
    })
    .unwrap();
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let (video, out) = (ids[0], ids[1]);
    app.apply(Command::Connect {
        from: PortRef::new(video, "frame"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let _ = app.take_recompiles();
    assert!(!app.needs_recompile(out));

    let before = supersilvia::compile::wgsl::build(app.graph(), out)
        .expect("connected")
        .source();

    app.apply(Command::SetOption {
        node: video,
        key: "file",
        value: "clips/second.mp4".to_string(),
    })
    .unwrap();

    assert!(
        !app.needs_recompile(out),
        "an asset reference names a texture the renderer binds, not WGSL",
    );

    // Why that is safe, rather than merely cheap: the shader a swap would have rebuilt is
    // the one already running, character for character. The clip reaches the fragment
    // through the node's texture binding, so the path is never in the WGSL to go stale.
    let after = supersilvia::compile::wgsl::build(app.graph(), out)
        .expect("connected")
        .source();
    assert_eq!(
        before, after,
        "swapping an asset changed the generated WGSL"
    );
    assert!(!before.contains("second.mp4") && !before.contains(".mp4"));

    // And nothing else on a video rebuilds either: its options are an asset, two read by
    // `tick` and four ticks the canvas reads, so a clip node never costs a recompile at all.
    for (key, value) in [
        ("size", "720"),
        ("loop", "hold"),
        ("scope", "off"),
        ("preview", "off"),
    ] {
        app.apply(Command::SetOption {
            node: video,
            key,
            value: value.to_string(),
        })
        .unwrap();
        assert!(!app.needs_recompile(out), "video.{key} rebuilt a shader");
    }
}

/// A presentation option reaches no shader at all — the same answer `SetCollapsed` gives.
#[test]
fn changing_what_is_drawn_never_recompiles() {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug: "video",
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::new(300.0, 0.0),
        workspace,
    })
    .unwrap();
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let (video, out) = (ids[0], ids[1]);
    app.apply(Command::Connect {
        from: PortRef::new(video, "frame"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let _ = app.take_recompiles();

    for key in ["uniforms", "events", "scope", "preview"] {
        app.apply(Command::SetOption {
            node: video,
            key,
            value: "off".to_string(),
        })
        .unwrap();
        assert!(
            !app.needs_recompile(out),
            "how much of a node is drawn is not a fact about its shader: {key}",
        );
    }
}

#[test]
fn moving_a_node_never_recompiles() {
    let (mut app, cb, out) = patch();
    let _ = app.take_recompiles();

    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(50.0, 50.0))],
    })
    .unwrap();
    assert!(!app.needs_recompile(out));
}

#[test]
fn a_scrub_is_one_undo_step() {
    let (mut app, cb, _) = patch();
    let before = app.history().len();

    for v in 1..=20 {
        app.apply(Command::SetControl {
            node: cb,
            key: "frequency",
            value: ControlValue::Float(v as f32),
        })
        .unwrap();
    }

    assert_eq!(
        app.history().len(),
        before + 1,
        "twenty frames of drag, one entry"
    );
    assert_eq!(
        app.graph().get(cb).unwrap().controls["frequency"],
        ControlValue::Float(20.0),
    );
}

/// The bug this coalescing exists to prevent, in the shape that got past it: a band handle
/// on the audio scope carries a frequency and a Q, and one command a frame for each was two
/// undo steps a frame — a second of dragging filled the whole ring.
#[test]
fn dragging_a_two_axis_handle_is_one_undo_step() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "audioin",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let mic = app.graph().iter().map(|(id, _)| id).last().unwrap();
    let before = app.history().len();

    for step in 1..=20 {
        app.apply(Command::SetControls {
            node: mic,
            values: vec![
                ("bassFreq", ControlValue::Float(100.0 + step as f32)),
                ("bassQ", ControlValue::Float(1.0 + step as f32 * 0.1)),
            ],
        })
        .unwrap();
    }

    assert_eq!(
        app.history().len(),
        before + 1,
        "twenty frames of drag, one entry"
    );
    let node = app.graph().get(mic).unwrap();
    assert_eq!(node.controls["bassFreq"], ControlValue::Float(120.0));
    assert_eq!(node.controls["bassQ"], ControlValue::Float(3.0));
    assert!(app.undo(), "and one undo puts the whole drag back");
    assert_eq!(
        app.graph().get(mic).unwrap().controls["bassFreq"],
        ControlValue::Float(100.0),
    );
}

/// A different handle is a different gesture, so it opens its own step rather than joining
/// the one before it.
#[test]
fn a_handle_writing_other_keys_opens_its_own_step() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "audioin",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let mic = app.graph().iter().map(|(id, _)| id).last().unwrap();
    let before = app.history().len();

    for keys in [["bassFreq", "bassQ"], ["midFreq", "midQ"]] {
        app.apply(Command::SetControls {
            node: mic,
            values: keys
                .iter()
                .map(|k| (*k, ControlValue::Float(2.0)))
                .collect(),
        })
        .unwrap();
    }
    assert_eq!(app.history().len(), before + 2);
}

/// `R` on a control clears its range and puts the value back, and that is one edit: two undo
/// steps would mean pressing it once and having to undo twice.
#[test]
fn resetting_a_control_is_one_undo_step() {
    let (mut app, cb, _) = patch();
    app.apply(Command::SetRange {
        node: cb,
        key: "frequency",
        range: supersilvia::graph::ControlRange {
            min: 1.0,
            max: 4.0,
            step: 0.5,
        },
    })
    .unwrap();
    let before = app.history().len();

    app.apply(Command::ClearRange {
        node: cb,
        key: "frequency",
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(8.0),
    })
    .unwrap();

    assert_eq!(app.history().len(), before + 1);
    assert!(app.undo());
    let node = app.graph().get(cb).unwrap();
    assert!(
        node.values.contains_key("frequency"),
        "one undo puts the range back as well as the value"
    );
}

#[test]
fn editing_two_controls_in_turn_keeps_both_entries() {
    let (mut app, cb, _) = patch();
    let before = app.history().len();

    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(2.0),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: cb,
        key: "color1",
        value: ControlValue::Color([1.0, 0.0, 0.0, 1.0]),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(3.0),
    })
    .unwrap();

    // Coalescing only collapses consecutive edits of the *same* control.
    assert_eq!(app.history().len(), before + 3);
}

#[test]
fn an_unknown_control_key_is_an_error_not_a_panic() {
    let (mut app, cb, _) = patch();
    assert_eq!(
        app.apply(Command::SetControl {
            node: cb,
            key: "nope",
            value: ControlValue::Float(1.0),
        }),
        Err(CommandError::NoSuchKey(cb, "nope")),
    );
}

#[test]
fn control_edits_reach_the_generated_uniforms() {
    let (mut app, cb, out) = patch();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(40.0),
    })
    .unwrap();

    let shader = supersilvia::compile::wgsl::build(app.graph(), out).expect("connected");
    // The shader still names the uniform; the value travels beside it, not inside it.
    assert!(
        shader
            .uniforms
            .contains_key("u_control_checkerboard1_frequency")
    );
    assert!(
        !shader.body.contains("40"),
        "a control value must never be baked into the WGSL"
    );
}

// ---------------------------------------------------------------- value coercion

/// A control value is fitted to the definition before it reaches the graph. Nothing
/// downstream clamps: the compiler takes the WGSL type from the definition while the
/// renderer writes the uniform block from the stored variant.
#[test]
fn a_control_value_is_clamped_to_its_declared_range() {
    let (mut app, cb, _) = patch();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(1.0e9),
    })
    .unwrap();

    let stored = *app
        .graph()
        .get(cb)
        .unwrap()
        .controls
        .get("frequency")
        .unwrap();
    let ControlValue::Float(v) = stored else {
        panic!("frequency is a number control");
    };
    assert!(v <= 64.0, "clamped to the definition's maximum, got {v}");
}

/// The wrong variant would compile to an `f32` member and be written as four floats, which
/// nothing in the app reports.
#[test]
fn a_color_cannot_be_stored_under_a_number_control() {
    let (mut app, cb, _) = patch();
    let err = app
        .apply(Command::SetControl {
            node: cb,
            key: "frequency",
            value: ControlValue::Color([1.0, 0.0, 0.0, 1.0]),
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::NoSuchKey(..)), "got {err:?}");
    assert!(matches!(
        app.graph().get(cb).unwrap().controls.get("frequency"),
        Some(ControlValue::Float(_))
    ));
}

/// Clicking one cable removes that edge. `Disconnect` clears everything feeding the input,
/// which is what clicking the *port* does.
#[test]
fn disconnecting_one_edge_leaves_the_rest_of_the_input_alone() {
    let (mut app, cb, out) = patch();
    assert_eq!(app.graph().connections().len(), 1);
    app.apply(Command::DisconnectEdge {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    assert_eq!(app.graph().connections().len(), 0);

    let err = app
        .apply(Command::DisconnectEdge {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap_err();
    assert!(matches!(err, CommandError::NotConnected(_)), "got {err:?}");
}

/// What right-clicking a port does: `Disconnect` only ever reaches a port from its input
/// side, and a right-click on an *output* dot has to clear the same way.
#[test]
fn disconnecting_a_port_reaches_an_output_and_recompiles_its_targets() {
    let (mut app, cb, out) = patch();
    let _ = app.take_recompiles();

    app.apply(Command::DisconnectPort(PortRef::new(cb, "output")))
        .unwrap();
    assert!(app.graph().connections().is_empty());
    assert!(
        app.needs_recompile(out),
        "the Output lost its input and its shader no longer describes the graph",
    );

    let err = app
        .apply(Command::DisconnectPort(PortRef::new(cb, "output")))
        .unwrap_err();
    assert!(
        matches!(err, CommandError::NotConnected(_)),
        "a bare port has nothing to clear, got {err:?}"
    );
}

/// A control is a uniform however many of them are reset at once, so `Reset controls` sets
/// floats and rebuilds nothing. `Disconnect all` changes the shape of the expression, so it
/// does.
#[test]
fn resetting_controls_never_recompiles_and_disconnecting_all_does() {
    let (mut app, cb, out) = patch();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(40.0),
    })
    .unwrap();
    let _ = app.take_recompiles();

    app.apply(Command::ResetControls(vec![cb, out])).unwrap();
    assert!(
        !app.needs_recompile(out),
        "a reset is a uniform per control, not a rebuild",
    );

    app.apply(Command::DisconnectAll(vec![cb])).unwrap();
    assert!(
        app.needs_recompile(out),
        "the Output lost its input and its shader no longer describes the graph",
    );
}

// --------------------------------------------------- a measurement is its workspace's pass's

/// A workspace whose Output draws a checkerboard, with a gradient beside it waiting for a
/// tap that reaches nothing. The tap is measured by the workspace's pass; no Output's shader
/// holds its chain.
fn loose_tap() -> (App, NodeId, NodeId, NodeId) {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    for slug in ["checkerboard", "output", "radialgradient", "tap"] {
        app.apply(Command::AddNode {
            slug,
            at: Pos2::ZERO,
            workspace,
        })
        .unwrap();
    }
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let (cb, out, gradient, tap) = (ids[0], ids[1], ids[2], ids[3]);
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.publish_plan();
    let _ = app.take_recompiles();
    assert_eq!(
        app.pass_measures(workspace),
        [tap],
        "the tap reaches no Output, and its workspace's pass measures it",
    );
    (app, gradient, tap, out)
}

/// Add one node to the first workspace and hand back its id.
fn added(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

/// A cable upstream of a measurement is upstream of its pass and of no Output: the Output
/// beside it, downstream of nothing that moved, is not rebuilt.
#[test]
fn a_cable_upstream_of_a_loose_tap_marks_no_output() {
    let (mut app, gradient, tap, _out) = loose_tap();
    app.apply(Command::Connect {
        from: PortRef::new(gradient, "output"),
        to: PortRef::new(tap, "input"),
    })
    .unwrap();
    assert!(
        app.take_recompiles().is_empty(),
        "the pass evaluates the gradient now; no Output's shader changed",
    );
}

/// And the cheap side of the boundary is unmoved: a control upstream of a measurement is a
/// uniform.
#[test]
fn a_control_upstream_of_a_loose_tap_marks_nothing() {
    let (mut app, gradient, tap, _out) = loose_tap();
    app.apply(Command::Connect {
        from: PortRef::new(gradient, "output"),
        to: PortRef::new(tap, "input"),
    })
    .unwrap();
    let _ = app.take_recompiles();

    app.apply(Command::SetControl {
        node: gradient,
        key: "radius",
        value: ControlValue::Float(0.5),
    })
    .unwrap();
    assert!(
        app.take_recompiles().is_empty(),
        "a parameter edit sets a float, measured or not",
    );
}

/// A cable on a chain the tap cannot reach rebuilds only the Output it reaches.
#[test]
fn a_cable_on_an_unrelated_chain_marks_only_its_output() {
    let (mut app, _gradient, _tap, out) = loose_tap();
    let solid = added(&mut app, "rgba");
    let other = added(&mut app, "output");
    let _ = app.take_recompiles();

    app.apply(Command::Connect {
        from: PortRef::new(solid, "output"),
        to: PortRef::new(other, "input"),
    })
    .unwrap();
    assert_eq!(
        app.take_recompiles(),
        vec![other],
        "only the Output the cable reaches",
    );
    assert!(!app.needs_recompile(out));
}

/// Cabling the tap into an Output rebuilds that Output by the ordinary rule, and the
/// measurement stays where it was: in the pass, never in the Output's shader.
#[test]
fn cabling_a_loose_tap_into_an_output_leaves_its_measurement_in_the_pass() {
    let (mut app, gradient, tap, out) = loose_tap();
    app.apply(Command::Connect {
        from: PortRef::new(gradient, "output"),
        to: PortRef::new(tap, "input"),
    })
    .unwrap();
    let other = added(&mut app, "output");
    let _ = app.take_recompiles();

    app.apply(Command::Connect {
        from: PortRef::new(tap, "output"),
        to: PortRef::new(other, "input"),
    })
    .unwrap();
    assert_eq!(
        app.take_recompiles(),
        vec![other],
        "the Output the tap now reaches"
    );
    assert!(!app.needs_recompile(out));
    app.publish_plan();
    assert_eq!(app.pass_measures(app.graph().default_workspace()), [tap]);
}

// ------------------------------------------------------------- a node's own buttons

/// A node's value under one option, as the document holds it.
fn option_of(app: &App, node: NodeId, key: &str) -> String {
    app.graph().get(node).unwrap().options[key].clone()
}

/// A number control's value, as the document holds it.
fn control_of(app: &App, node: NodeId, key: &str) -> f32 {
    match app.graph().get(node).unwrap().controls.get(key) {
        Some(ControlValue::Float(v)) => *v,
        other => panic!("{key} is a number, not {other:?}"),
    }
}

/// The file a one-workspace project writes, and the graph it opens as.
fn saved_and_opened(app: &App) -> supersilvia::graph::Graph {
    use supersilvia::workspace::{self, WorkspaceFile};
    let g = app.graph();
    let nodes = g.iter().map(|(id, _)| id).collect();
    let file = WorkspaceFile::of(g, &g.workspaces()[0], &nodes);
    let text = workspace::to_text(&file).unwrap();
    let (back, warnings) = workspace::from_str(&text).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    back
}

/// **Random Seq is a rebuild, one undo step, and saved.** The press writes `sequence`, which is
/// in the shader, so the Output downstream rebuilds exactly as a pick from the row would; the
/// whole press is one step, and the sequence it rolled is in the file.
#[test]
fn random_seq_rebuilds_the_shader_in_one_undo_step_and_is_saved() {
    let (mut app, lyapunov, out) = source_into_output("lyapunov", "color");
    let steps = app.history().len();
    let sequence = supersilvia::nodes::fractals::random_sequence(99);
    assert_ne!(sequence, "A6B6", "a roll that moves it");

    let settings = (supersilvia::nodes::fractals::RANDOM_SEQ.press)(0, 99);
    app.apply(Command::SetSettings {
        node: lyapunov,
        options: settings.options,
        controls: Vec::new(),
    })
    .unwrap();

    assert_eq!(option_of(&app, lyapunov, "sequence"), sequence);
    assert!(app.needs_recompile(out), "a `Code` option is in the shader");
    assert_eq!(app.history().len(), steps + 1, "one press, one step");
    assert_eq!(
        saved_and_opened(&app).get(lyapunov).unwrap().options["sequence"],
        sequence,
        "and the roll is what the file says"
    );

    let _ = app.take_recompiles();
    assert!(app.undo());
    assert_eq!(option_of(&app, lyapunov, "sequence"), "A6B6");
    assert!(app.needs_recompile(out), "and undoing it rebuilds back");
}

/// **A preset writes three knobs and a `Runtime` option, and rebuilds nothing.** Sense Angle,
/// Turn Angle and Sense Dist are uniforms and Mode is read by the tick, so silvia's preset bar
/// costs the shader nothing — and all four go back in one undo.
#[test]
fn a_slime_mold_preset_writes_its_knobs_and_mode_in_one_step_without_a_rebuild() {
    let (mut app, mold, out) = source_into_output("slimemold", "color");
    let steps = app.history().len();
    let preset = supersilvia::nodes::slimemold::preset(3);
    app.apply(Command::SetSettings {
        node: mold,
        options: preset.options,
        controls: preset
            .controls
            .into_iter()
            .map(|(key, v)| (key, ControlValue::Float(v)))
            .collect(),
    })
    .unwrap();

    assert_eq!(option_of(&app, mold, "mode"), "repel");
    assert_eq!(control_of(&app, mold, "sensorAngle"), 141.7);
    assert_eq!(control_of(&app, mold, "rotationAngle"), 68.8);
    assert_eq!(control_of(&app, mold, "sensorOffset"), 31.0);
    assert!(!app.needs_recompile(out), "uniforms and a tick's option");
    assert_eq!(app.history().len(), steps + 1);
    let saved = saved_and_opened(&app);
    assert_eq!(saved.get(mold).unwrap().options["mode"], "repel");
    assert_eq!(
        saved.get(mold).unwrap().controls.get("sensorOffset"),
        Some(&ControlValue::Float(31.0))
    );

    assert!(app.undo());
    assert_eq!(option_of(&app, mold, "mode"), "attract");
    assert_eq!(control_of(&app, mold, "sensorAngle"), 19.0);
    assert_eq!(control_of(&app, mold, "rotationAngle"), 151.8);
    assert_eq!(control_of(&app, mold, "sensorOffset"), 22.0);
}

/// **A press that names anything its node cannot take writes nothing.** Every option and
/// every control is checked before any is written, so a bad key or a value the option does
/// not offer leaves the node, the history and the shader exactly as they were.
#[test]
fn settings_with_one_bad_entry_write_none_of_them() {
    let (mut app, mold, out) = source_into_output("slimemold", "color");
    let steps = app.history().len();
    let good = ("sensorAngle", ControlValue::Float(90.0));
    for (options, controls, expected) in [
        (
            vec![("mode", "sideways".to_string())],
            vec![good],
            CommandError::NoSuchChoice(mold, "mode"),
        ),
        (
            vec![("nonsense", "on".to_string())],
            vec![good],
            CommandError::NoSuchKey(mold, "nonsense"),
        ),
        (
            vec![("mode", "repel".to_string())],
            vec![good, ("nonsense", ControlValue::Float(1.0))],
            CommandError::NoSuchKey(mold, "nonsense"),
        ),
    ] {
        assert_eq!(
            app.apply(Command::SetSettings {
                node: mold,
                options,
                controls,
            }),
            Err(expected)
        );
        assert_eq!(option_of(&app, mold, "mode"), "attract");
        assert_eq!(control_of(&app, mold, "sensorAngle"), 19.0);
    }
    assert_eq!(app.history().len(), steps);
    assert!(!app.needs_recompile(out));

    // A value outside a knob's range is fitted, as any write is, rather than refused.
    app.apply(Command::SetSettings {
        node: mold,
        options: Vec::new(),
        controls: vec![("sensorOffset", ControlValue::Float(400.0))],
    })
    .unwrap();
    assert_eq!(control_of(&app, mold, "sensorOffset"), 40.0);
}
