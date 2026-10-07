// SPDX-License-Identifier: AGPL-3.0-or-later

//! `.ssw` round-trips, and what happens when a file names what the node library does not
//! have: it opens with whatever matches, and each thing it drops is a plain warning.

use emath::Pos2;
use std::collections::BTreeSet;
use supersilvia::graph::{ControlValue, Graph, LayoutMode, NodeId, PortRef, WorkspaceKind};
use supersilvia::workspace::{
    self, Ids, LoadError, LoadWarning, SavedConnection, SavedPort, WorkspaceFile,
};
use supersilvia::{compile, nodes};

fn add(g: &mut Graph, slug: &str, at: Pos2) -> NodeId {
    nodes::add_to_graph(g, slug, at).unwrap()
}

/// A whole graph as one workspace file, which is what a one-workspace project writes.
fn file_of(g: &Graph) -> WorkspaceFile {
    let nodes: BTreeSet<NodeId> = g.iter().map(|(id, _)| id).collect();
    WorkspaceFile::of(g, &g.workspaces()[0], &nodes)
}

/// checkerboard → zoom → output, with an edited control and a non-default option.
fn graph_with_everything() -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard", Pos2::new(10.0, 20.0));
    let zoom = add(&mut g, "zoom", Pos2::new(240.0, 20.0));
    let out = add(&mut g, "output", Pos2::new(480.0, 20.0));
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.get_mut(cb)
        .unwrap()
        .controls
        .insert("frequency", ControlValue::Float(23.0));
    g.get_mut(out)
        .unwrap()
        .options
        .insert("resolution", "2000x1000".to_string());
    (g, cb, out)
}

fn round_trip(g: &Graph) -> (Graph, Vec<LoadWarning>) {
    let json = serde_json::to_string_pretty(&file_of(g)).unwrap();
    workspace::from_str(&json).unwrap()
}

/// A hidden control is a control: it is stored on the node and written to the file, so it
/// has to come back. Loading only the ports dropped an audio node's whole band tuning —
/// silently, with a warning per control, on every reload.
#[test]
fn a_hidden_control_round_trips_and_is_not_a_warning() {
    let mut g = Graph::new();
    let video = add(&mut g, "video", Pos2::ZERO);
    g.get_mut(video)
        .unwrap()
        .controls
        .insert("bassFreq", ControlValue::Float(140.0));

    let (back, warnings) = round_trip(&g);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        back.get(video).unwrap().controls.get("bassFreq"),
        Some(&ControlValue::Float(140.0)),
        "the band it was tuned to came back"
    );
}

/// A step sequencer's pattern is one of its values, so it is written into the file and comes
/// back cell for cell — as four lanes of `x` and `.` a person can read in the `.ssw` — while
/// where its playhead was is runtime state and has nowhere in the file to be.
#[test]
fn a_step_sequencers_pattern_round_trips_and_its_playhead_does_not() {
    let mut g = Graph::new();
    let seq = add(&mut g, "stepsequencer", Pos2::ZERO);
    let pattern = supersilvia::graph::Value::grid(4, 16, |lane, step| step % (lane + 2) == 0);
    g.get_mut(seq)
        .unwrap()
        .values
        .insert("pattern", pattern.clone());

    let json = serde_json::to_string_pretty(&file_of(&g)).unwrap();
    assert!(
        json.contains("\"x.x.x.x.x.x.x.x.\""),
        "lane 1 is written as the cells it lights:\n{json}"
    );
    assert!(!json.contains("playhead"), "the playhead is not saved");

    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        back.get(seq).unwrap().values.get("pattern"),
        Some(&pattern),
        "every lit cell came back"
    );
}

#[test]
fn a_workspace_round_trips_exactly() {
    let (g, cb, out) = graph_with_everything();
    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(back.len(), 3);
    assert_eq!(back.connections().len(), 2);
    assert_eq!(back.get(cb).unwrap().pos, Pos2::new(10.0, 20.0));
    assert_eq!(
        back.get(cb).unwrap().controls.get("frequency"),
        Some(&ControlValue::Float(23.0)),
        "an edited control survives"
    );
    assert_eq!(
        back.get(out).unwrap().options.get("resolution").unwrap(),
        "2000x1000",
        "a non-default option survives, a free size among them"
    );
}

/// Ids are the file's identity, not an artifact of insertion order. They have to come back
/// unchanged or the WGSL function names in a reopened graph describe different nodes.
#[test]
fn node_ids_survive_the_round_trip() {
    let (g, _, _) = graph_with_everything();
    let before: Vec<NodeId> = g.iter().map(|(id, _)| id).collect();
    let (back, _) = round_trip(&g);
    let after: Vec<NodeId> = back.iter().map(|(id, _)| id).collect();
    assert_eq!(before, after);
}

/// The whole point of reopening a file is that it still renders.
#[test]
fn a_reloaded_workspace_compiles_to_the_same_shader() {
    let (g, _, out) = graph_with_everything();
    let (back, _) = round_trip(&g);
    assert_eq!(
        compile::wgsl::build(&g, out).unwrap().source(),
        compile::wgsl::build(&back, out).unwrap().source()
    );
}

/// Adding a node after the file was written must not leave that node's controls empty.
#[test]
fn a_control_missing_from_the_file_gets_its_default() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard", Pos2::ZERO);
    let mut saved = file_of(&g);
    saved.nodes[0].controls.remove("frequency");

    let json = serde_json::to_string(&saved).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(
        back.get(cb).unwrap().controls.get("frequency"),
        Some(&ControlValue::Float(8.0)),
        "defaults are laid down before the file's values, not instead of them"
    );
}

#[test]
fn an_unknown_node_kind_is_dropped_with_its_cables_and_a_warning() {
    let (g, _, _) = graph_with_everything();
    let mut saved = file_of(&g);
    saved.nodes[1].slug = "flanger9000".to_string();

    let json = serde_json::to_string(&saved).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();

    assert_eq!(back.len(), 2, "the unknown node is gone");
    assert_eq!(
        back.connections().len(),
        0,
        "both cables touched it, so both go"
    );
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::UnknownNodeKind { .. })),
        "{warnings:?}"
    );
}

#[test]
fn a_control_the_node_no_longer_has_is_reported_not_swallowed() {
    let (g, _, _) = graph_with_everything();
    let mut saved = file_of(&g);
    saved.nodes[0]
        .controls
        .insert("wobble".to_string(), ControlValue::Float(1.0));

    let json = serde_json::to_string(&saved).unwrap();
    let (_, warnings) = workspace::from_str(&json).unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::UnknownControl { key, .. } if key == "wobble")),
        "{warnings:?}"
    );
}

/// A hand-edited file must not be able to produce a graph the editor could not.
#[test]
fn a_file_describing_a_cycle_loses_the_offending_cable() {
    let mut g = Graph::new();
    let a = add(&mut g, "zoom", Pos2::ZERO);
    let b = add(&mut g, "zoom", Pos2::ZERO);
    g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();

    let mut saved = file_of(&g);
    saved.connections.push(workspace::SavedConnection {
        from: workspace::SavedPort {
            node: b,
            key: "output".to_string(),
        },
        to: workspace::SavedPort {
            node: a,
            key: "input".to_string(),
        },
    });

    let json = serde_json::to_string(&saved).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();
    assert_eq!(
        back.connections().len(),
        1,
        "the cycle-closing edge is gone"
    );
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::DroppedConnection { .. })),
        "{warnings:?}"
    );
}

#[test]
fn a_file_that_is_not_ours_is_refused_outright() {
    let err = workspace::from_str(r#"{"format":"silvia","version":1,"nodes":[],"connections":[]}"#)
        .unwrap_err();
    assert!(matches!(err, LoadError::NotAWorkspace(_)), "{err:?}");

    let err = workspace::from_str("not json at all").unwrap_err();
    assert!(matches!(err, LoadError::Json(_)), "{err:?}");

    let err = workspace::from_str(
        r#"{"format":"supersilvia-workspace","version":99,"nodes":[],"connections":[]}"#,
    )
    .unwrap_err();
    assert!(matches!(err, LoadError::UnsupportedVersion(_)), "{err:?}");
}

#[test]
fn saving_and_loading_through_the_filesystem_works() {
    let (g, _, out) = graph_with_everything();
    let dir = std::env::temp_dir().join(format!("ssw-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("workspace.ssw");

    workspace::write(&file_of(&g), &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("supersilvia-workspace"));
    assert!(text.ends_with('\n'), "files end with a newline");

    let (back, warnings) = workspace::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(warnings.is_empty());
    assert!(compile::wgsl::build(&back, out).is_some());
    std::fs::remove_dir_all(&dir).ok();
}

/// A hand-edited file cannot seat a color under a float uniform. The compiler derives the
/// WGSL type from the definition while the renderer writes the uniform block from the stored
/// value, so the mismatch would land as another type's bytes with no error path anywhere in
/// the app.
#[test]
fn a_control_value_of_the_wrong_kind_is_refused_and_reported() {
    let (g, cb, _) = graph_with_everything();
    let mut saved = file_of(&g);
    let node = saved.nodes.iter_mut().find(|n| n.id == cb).unwrap();
    node.controls.insert(
        "frequency".to_string(),
        ControlValue::Color([1.0, 0.0, 0.0, 1.0]),
    );

    let json = serde_json::to_string(&saved).unwrap();
    let (loaded, warnings) = workspace::from_str(&json).unwrap();

    assert!(
        matches!(
            loaded.get(cb).unwrap().controls.get("frequency"),
            Some(ControlValue::Float(_))
        ),
        "the definition's default stands"
    );
    assert!(
        warnings.contains(&LoadWarning::WrongValue {
            id: cb,
            key: "frequency".to_string()
        }),
        "and it is reported: {warnings:?}"
    );
}

/// The same for a value outside the declared range: a zoom of zero is a NaN frame.
#[test]
fn a_control_value_outside_its_range_is_clamped_on_load() {
    let mut g = Graph::new();
    let zoom = add(&mut g, "zoom", Pos2::ZERO);
    let mut saved = file_of(&g);
    saved.nodes[0]
        .controls
        .insert("zoom".to_string(), ControlValue::Float(-500.0));

    let json = serde_json::to_string(&saved).unwrap();
    let (loaded, _) = workspace::from_str(&json).unwrap();
    let Some(ControlValue::Float(v)) = loaded.get(zoom).unwrap().controls.get("zoom") else {
        panic!("zoom is a number control");
    };
    assert!(*v >= 0.01, "clamped into the declared range, got {v}");
}

/// A resolution that is no size would reach `parse_resolution` and silently fall back,
/// leaving a node whose picker shows a size it is not drawn at.
#[test]
fn an_option_value_that_is_not_a_choice_keeps_the_default() {
    let (g, _, out) = graph_with_everything();
    let mut saved = file_of(&g);
    let node = saved.nodes.iter_mut().find(|n| n.id == out).unwrap();
    node.options
        .insert("resolution".to_string(), "1280x".to_string());

    let json = serde_json::to_string(&saved).unwrap();
    let (loaded, warnings) = workspace::from_str(&json).unwrap();

    assert_eq!(
        loaded.get(out).unwrap().options.get("resolution").unwrap(),
        "1280x720",
        "the definition's default stands"
    );
    assert!(
        warnings.contains(&LoadWarning::WrongValue {
            id: out,
            key: "resolution".to_string()
        }),
        "and it is reported: {warnings:?}"
    );
}

/// The layout mode is the workspace's own, so it rides in the workspace file.
#[test]
fn the_layout_mode_round_trips() {
    let (mut g, _, _) = graph_with_everything();
    let ws = g.default_workspace();
    assert!(g.set_layout(ws, LayoutMode::Linear));
    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty());
    assert_eq!(back.layout_of(back.default_workspace()), LayoutMode::Linear);
}

/// Collapsed is document data: it rides in the file like a position.
#[test]
fn a_collapsed_node_stays_collapsed_across_a_round_trip() {
    let mut g = Graph::new();
    let id = add(&mut g, "checkerboard", Pos2::new(10.0, 20.0));
    g.get_mut(id).expect("the node").collapsed = true;

    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(
        back.get(id).expect("the node").collapsed,
        "the file lost it"
    );
}

/// A dragged width is document data too: the one thing a person sets about a note besides
/// what it says, so the file carries it and a reopened patch is the canvas that was left.
#[test]
fn a_dragged_width_stays_across_a_round_trip() {
    let mut g = Graph::new();
    let id = add(&mut g, "note", Pos2::new(10.0, 20.0));
    g.get_mut(id).expect("the node").dragged_width = Some(420.0);

    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        back.get(id).expect("the node").dragged_width,
        Some(420.0),
        "the file lost it"
    );
}

/// A node still the width its kind asks for writes nothing about its width at all, the way
/// an expanded node writes no collapsed flag.
#[test]
fn a_node_at_its_kinds_width_writes_no_width() {
    let mut g = Graph::new();
    let id = add(&mut g, "note", Pos2::new(10.0, 20.0));
    let json = serde_json::to_string(&file_of(&g)).unwrap();
    assert!(
        !json.contains("width"),
        "a node nobody dragged should not write a width: {json}"
    );

    let (back, warnings) = workspace::from_str(&json).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.get(id).expect("the node").dragged_width, None);
}

/// An expanded node writes no flag at all, and reads back expanded.
#[test]
fn an_expanded_node_writes_no_collapsed_flag() {
    let mut g = Graph::new();
    let id = add(&mut g, "checkerboard", Pos2::new(10.0, 20.0));
    let json = serde_json::to_string(&file_of(&g)).unwrap();
    assert!(
        !json.contains("collapsed"),
        "an expanded node should not write the flag at all: {json}"
    );

    let (back, warnings) = workspace::from_str(&json).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!back.get(id).expect("the node").collapsed);
}

// ------------------------------------------------- the name, the kind and the ids

/// A workspace file says which workspace it is, so a project can be read back from its
/// files and an exported one arrives with the name it had.
#[test]
fn a_file_carries_its_workspace_name_and_kind() {
    let mut g = Graph::new();
    g.rename_workspace(g.default_workspace(), "Tunnel".to_string());
    add(&mut g, "checkerboard", Pos2::ZERO);

    let json = serde_json::to_string(&file_of(&g)).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.workspaces()[0].name, "Tunnel");
    assert_eq!(back.workspaces()[0].kind, WorkspaceKind::Video);
}

/// A file from somewhere else brings ids from another counter. `Ids::Fresh` gives every
/// node a new one and re-points every cable inside the file to match.
#[test]
fn fresh_ids_keep_every_connection_inside_the_file() {
    let (source, _, _) = graph_with_everything();
    let file = file_of(&source);

    // A graph that already holds a node under every id the file uses.
    let mut g = Graph::new();
    for _ in 0..3 {
        add(&mut g, "zoom", Pos2::ZERO);
    }
    let workspace = g.add_workspace("Imported".to_string(), WorkspaceKind::Video);
    let warnings = file.insert_into(&mut g, workspace, Ids::Fresh);
    assert!(warnings.is_empty(), "{warnings:?}");

    assert_eq!(g.len(), 6, "nothing was overwritten");
    assert_eq!(
        g.connections().len(),
        2,
        "both cables came with the nodes they join"
    );
    let imported: Vec<NodeId> = g
        .iter()
        .filter(|(_, n)| n.workspaces.contains(&workspace))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(imported.len(), 3);
    for c in g.connections() {
        assert!(
            imported.contains(&c.from.node) && imported.contains(&c.to.node),
            "a cable points at a node the import did not make: {c:?}"
        );
    }
}

/// The example in `examples/` is a real file, and a file that no longer opens is not an
/// example of anything.
#[test]
fn the_example_workspace_opens() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("big.ssw");
    let (graph, warnings) = workspace::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(graph.len(), 22);
    assert_eq!(graph.connections().len(), 32);
}

/// A node opened from a file lays out like the same node added from the menu: what `canvas`
/// reads — the audio scope, the wider body, the trace band — comes through the definition the
/// file's slug names, on both routes.
#[test]
fn a_loaded_node_carries_its_definitions_layout_bits() {
    let mut g = Graph::new();
    let scope = add(&mut g, "audioin", Pos2::ZERO);
    let trace = add(&mut g, "adsr", Pos2::new(300.0, 0.0));
    let wide = add(&mut g, "edgedetection", Pos2::new(600.0, 0.0));

    let json = serde_json::to_string(&file_of(&g)).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.get(scope).unwrap().def.regions.len(), 1, "the scope");
    assert_eq!(
        back.get(trace).unwrap().def.regions.len(),
        2,
        "the trace and the caption under it"
    );
    assert_eq!(back.get(wide).unwrap().def.width, Some(216.0));
}

/// A cable from Mandelbrot's `mask` is still on `mask` after a save and a reopen. Every output
/// key a file names is read as this build's own; nothing redirects it to another port.
#[test]
fn a_mandelbrot_mask_cable_survives_a_reopen() {
    let mut g = Graph::new();
    let mandelbrot = add(&mut g, "mandelbrot", Pos2::ZERO);
    let sum = add(&mut g, "add", Pos2::new(200.0, 0.0));
    let mut saved = file_of(&g);
    saved.connections.push(SavedConnection {
        from: SavedPort {
            node: mandelbrot,
            key: "mask".to_string(),
        },
        to: SavedPort {
            node: sum,
            key: "a".to_string(),
        },
    });

    let json = serde_json::to_string(&saved).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.connections().len(), 1);
    assert_eq!(back.connections()[0].from, PortRef::new(mandelbrot, "mask"));
}

/// A `lyapunov` file naming a control the node does not have, Iterations, opens at its fixed
/// count: the node and its other numbers land, and the saved count and the cable into it are
/// dropped and reported, as any control or port a node does not have.
#[test]
fn a_saved_lyapunov_drops_its_iterations_and_keeps_the_rest() {
    let mut g = Graph::new();
    let noise = add(&mut g, "perlin", Pos2::ZERO);
    let fractal = add(&mut g, "lyapunov", Pos2::new(240.0, 0.0));
    let mut saved = file_of(&g);
    let node = saved.nodes.iter_mut().find(|n| n.id == fractal).unwrap();
    node.controls
        .insert("iterations".to_string(), ControlValue::Float(80.0));
    node.controls
        .insert("contrast".to_string(), ControlValue::Float(7.0));
    saved.connections.push(SavedConnection {
        from: SavedPort {
            node: noise,
            key: "value".to_string(),
        },
        to: SavedPort {
            node: fractal,
            key: "iterations".to_string(),
        },
    });
    let json = serde_json::to_string(&saved).unwrap();

    let (back, warnings) = workspace::from_str(&json).unwrap();

    let node = back.get(fractal).expect("the node itself still opens");
    assert_eq!(node.controls["contrast"], ControlValue::Float(7.0));
    assert!(!node.controls.contains_key("iterations"));
    assert!(back.connections().is_empty(), "{:?}", back.connections());
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::UnknownControl { id, key }
                if *id == fractal && key == "iterations")),
        "the saved count says so: {warnings:?}"
    );
    assert!(
        warnings.iter().any(
            |w| matches!(w, LoadWarning::DroppedConnection { reason } if reason.contains("iterations"))
        ),
        "and so does its cable: {warnings:?}"
    );
}

// ---------------------------------------------------------------- dual outputs

/// A saved `add` opens in the mode its cables say, whichever way round they are.
#[test]
fn a_saved_add_loads_in_the_mode_its_cables_say() {
    use supersilvia::graph::PortType;

    let mut g = Graph::new();
    let sum = add(&mut g, "add", Pos2::ZERO);
    let zoom = add(&mut g, "zoom", Pos2::new(240.0, 0.0));
    g.connect(PortRef::new(sum, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();

    let ty = |g: &Graph, id: NodeId| g.get(id).unwrap().output("output").unwrap().ty;

    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        ty(&back, sum),
        PortType::UniformNumber,
        "two knobs, so it comes back a number"
    );

    // The same file with a field into it comes back a field.
    let luma = add(&mut g, "luminosity", Pos2::new(-240.0, 0.0));
    g.connect(PortRef::new(luma, "output"), PortRef::new(sum, "a"))
        .unwrap();
    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ty(&back, sum), PortType::VaryingNumber);
}

/// **A bulk change settles rather than refusing cable by cable.**
///
/// A file listing both `luminosity → add.a` and `add.output → slew.input` describes a graph
/// no gesture could have made: the editor refuses the first cable while the second is there.
/// Which one a load would refuse would otherwise depend on the order the file listed them
/// in, so the types are resolved once every cable is in and the cable that reads a field as
/// a number is the one that goes.
#[test]
fn a_file_that_demotes_a_read_number_drops_the_consumer_cable() {
    use supersilvia::graph::PortType;

    let mut g = Graph::new();
    let luma = add(&mut g, "luminosity", Pos2::ZERO);
    let sum = add(&mut g, "add", Pos2::new(240.0, 0.0));
    let slew = add(&mut g, "slew", Pos2::new(480.0, 0.0));
    // Legal while the `add` is a diamond, which it is until the field arrives.
    g.connect(PortRef::new(sum, "output"), PortRef::new(slew, "input"))
        .unwrap();

    // Hand-built: the cable the editor would have refused, written into the file directly.
    let mut saved = file_of(&g);
    saved.connections.push(SavedConnection {
        from: SavedPort {
            node: luma,
            key: "output".to_string(),
        },
        to: SavedPort {
            node: sum,
            key: "a".to_string(),
        },
    });
    let json = serde_json::to_string(&saved).unwrap();

    for order in [false, true] {
        let mut file: WorkspaceFile = serde_json::from_str(&json).unwrap();
        if order {
            file.connections.reverse();
        }
        let (back, warnings) = workspace::from_str(&serde_json::to_string(&file).unwrap()).unwrap();

        assert_eq!(
            back.get(sum).unwrap().output("output").unwrap().ty,
            PortType::VaryingNumber,
            "the field won, whichever order the cables were listed in"
        );
        assert_eq!(
            back.source_of(PortRef::new(sum, "a")),
            Some(PortRef::new(luma, "output")),
            "and it is still connected"
        );
        assert!(
            back.source_of(PortRef::new(slew, "input")).is_none(),
            "the cable that read it as a number is the one that went: {:?}",
            back.connections()
        );
        assert_eq!(
            warnings.len(),
            1,
            "one cable went, and it said so: {warnings:?}"
        );
        let LoadWarning::DroppedConnection { reason } = &warnings[0] else {
            panic!("{warnings:?}");
        };
        // Whichever order: no dual output's type is known until the file settles, so the
        // consumer cable is seated either way and `settle` is what takes it back out.
        assert!(reason.contains("reads as a number"), "{reason:?}");
    }
}

/// A pinned `add` loads pinned: the pin is a function of the whole graph, and `settle` is
/// where a file's is worked out, so the file carries the cables and nothing about the ports.
#[test]
fn a_saved_pinned_add_loads_pinned() {
    use supersilvia::graph::PortType;

    let mut g = Graph::new();
    let sum = add(&mut g, "add", Pos2::ZERO);
    let slew = add(&mut g, "slew", Pos2::new(240.0, 0.0));
    g.connect(PortRef::new(sum, "output"), PortRef::new(slew, "input"))
        .unwrap();

    let in_tys = |g: &Graph, id: NodeId| -> Vec<PortType> {
        g.get(id).unwrap().inputs.iter().map(|p| p.ty).collect()
    };
    assert_eq!(
        in_tys(&g, sum),
        [PortType::UniformNumber, PortType::UniformNumber]
    );

    let (back, warnings) = round_trip(&g);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        in_tys(&back, sum),
        [PortType::UniformNumber, PortType::UniformNumber],
        "the slew still reads its number, so the inputs are still numbers"
    );
}

/// A file written while the twelve coefficients were ports still opens with its palette.
///
/// They are the node's own values now — hidden controls, with no port — and a hidden control
/// is a control: it is stored under the same key, so the numbers land where they always did
/// and no alias is needed for them. What the file can no longer have is a *cable* into one,
/// and that goes the way every illegal cable in a file goes: dropped, with a warning saying
/// so, while the value it was overriding stays.
#[test]
fn a_palette_saved_with_cables_on_its_coefficients_keeps_its_twelve_numbers() {
    let mut g = Graph::new();
    let palette = add(&mut g, "cosinegradient", Pos2::ZERO);
    let knob = add(&mut g, "number", Pos2::ZERO);
    for (key, value) in [("phaseG", 0.2), ("ampR", 0.9), ("freqB", 3.0)] {
        g.get_mut(palette)
            .unwrap()
            .controls
            .insert(key, ControlValue::Float(value));
    }
    let mut saved = file_of(&g);
    // The cable a file from before could carry: something driving Amp R.
    saved.connections.push(workspace::SavedConnection {
        from: workspace::SavedPort {
            node: knob,
            key: "output".to_string(),
        },
        to: workspace::SavedPort {
            node: palette,
            key: "ampR".to_string(),
        },
    });

    let json = serde_json::to_string(&saved).unwrap();
    let (back, warnings) = workspace::from_str(&json).unwrap();

    let node = back.get(palette).unwrap();
    for (key, value) in [("phaseG", 0.2), ("ampR", 0.9), ("freqB", 3.0)] {
        assert_eq!(
            node.controls.get(key),
            Some(&ControlValue::Float(value)),
            "{key} did not come back"
        );
    }
    assert_eq!(back.connections().len(), 0, "the cable had nowhere to land");
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::DroppedConnection { .. })),
        "{warnings:?}"
    );
}

/// The same for the sequencer's twelve lane numbers, which were ports on the same day.
#[test]
fn a_sequencer_saved_with_cables_on_its_lanes_keeps_its_twelve_numbers() {
    let mut g = Graph::new();
    let seq = add(&mut g, "euclideanrhythm", Pos2::ZERO);
    g.get_mut(seq)
        .unwrap()
        .controls
        .insert("lane3rotation", ControlValue::Float(-3.0));

    let (back, warnings) = round_trip(&g);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        back.get(seq).unwrap().controls.get("lane3rotation"),
        Some(&ControlValue::Float(-3.0)),
        "the rotation the lane was set to came back"
    );
}
