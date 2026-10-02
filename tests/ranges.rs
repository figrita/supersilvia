// SPDX-License-Identifier: AGPL-3.0-or-later

//! A control's ends belong to the node, not to its kind.
//!
//! `NumberSpec` used to be rebuilt each frame from the static `InputDef`, so min, max and
//! step were properties of a node *kind*. They are properties of an instance: a speed that
//! only ever wants 0.9 to 1.1 is unscrubbable across the range its definition declares, and a
//! fader or a lane mapped onto a control needs to know where that control's ends are *here*.
//!
//! An instance's range goes where it is put: the definition's ends are advice and bind
//! nothing, as in silvia. What is still refused is a non-finite end. The definition's range
//! is printed in the editor's header as the node's statement about
//! where its own maths is defined.

use emath::Pos2;
use supersilvia::graph::{ControlRange, ControlValue, NodeId, PortRef};
use supersilvia::nodes;
use supersilvia::{App, Command, CommandError};

/// `checkerboard.frequency` is 8 over 1..64 step 1 — a number control with room to narrow.
/// A whole graph as one workspace file, which is what a one-workspace project writes.
fn file_of(g: &supersilvia::Graph) -> supersilvia::workspace::WorkspaceFile {
    let nodes: std::collections::BTreeSet<NodeId> = g.iter().map(|(id, _)| id).collect();
    supersilvia::workspace::WorkspaceFile::of(g, &g.workspaces()[0], &nodes)
}

fn patch() -> (App, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let id = app.graph().iter().next().expect("just added").0;
    (app, id)
}

fn range(app: &App, id: NodeId, key: &str) -> ControlRange {
    let node = app.graph().get(id).expect("node is in the graph");
    let def = nodes::find(node.def.slug).expect("kind is in the registry");
    nodes::control_range(def, node, key).expect("frequency is a number control")
}

fn value(app: &App, id: NodeId, key: &str) -> f32 {
    match app.graph().get(id).and_then(|n| n.controls.get(key)) {
        Some(ControlValue::Float(v)) => *v,
        other => panic!("expected a number control, got {other:?}"),
    }
}

#[test]
fn a_control_reports_its_definitions_range_until_something_changes_it() {
    let (app, id) = patch();
    assert_eq!(
        range(&app, id, "frequency"),
        ControlRange {
            min: 1.0,
            max: 64.0,
            step: 1.0
        }
    );
    assert!(
        app.graph().get(id).is_some_and(|n| n.values.is_empty()),
        "nothing is stored until it differs, which is the case in almost every graph"
    );
}

#[test]
fn a_range_is_per_instance() {
    let (mut app, first) = patch();
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::new(300.0, 0.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let second = app
        .graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("two nodes");

    app.apply(Command::SetRange {
        node: first,
        key: "frequency",
        range: ControlRange {
            min: 4.0,
            max: 8.0,
            step: 0.5,
        },
    })
    .unwrap();

    assert_eq!(range(&app, first, "frequency").max, 8.0);
    assert_eq!(
        range(&app, second, "frequency").max,
        64.0,
        "the other node of the same kind is untouched"
    );
}

#[test]
fn narrowing_a_range_pulls_the_value_in_with_it() {
    let (mut app, id) = patch();
    assert_eq!(value(&app, id, "frequency"), 8.0);
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 1.0,
            max: 4.0,
            step: 1.0,
        },
    })
    .unwrap();
    assert_eq!(
        value(&app, id, "frequency"),
        4.0,
        "a control cannot sit outside the track it is drawn on"
    );
}

#[test]
fn a_value_is_clamped_to_the_instances_range_not_the_definitions() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 2.0,
            max: 6.0,
            step: 1.0,
        },
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: id,
        key: "frequency",
        value: ControlValue::Float(50.0),
    })
    .unwrap();
    assert_eq!(value(&app, id, "frequency"), 6.0);
}

/// The definition's ends are advice. A control whose range could only ever narrow cannot be
/// pushed, and pushing a parameter past where its author expected is most of what this is for.
#[test]
fn a_range_goes_where_it_is_put() {
    let (mut app, id) = patch();
    let wide = ControlRange {
        min: -1000.0,
        max: 1000.0,
        step: 1.0,
    };
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: wide,
    })
    .unwrap();
    assert_eq!(
        range(&app, id, "frequency"),
        wide,
        "the declared 1..64 bounds nothing"
    );
}

/// A non-finite end is not a decision, and falls back to what the definition declares.
#[test]
fn a_non_finite_end_falls_back_to_the_declared_one() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: f32::NAN,
            max: f32::INFINITY,
            step: 1.0,
        },
    })
    .unwrap();
    assert_eq!(
        range(&app, id, "frequency"),
        ControlRange {
            min: 1.0,
            max: 64.0,
            step: 1.0
        },
        "both ends came back from the definition"
    );
}

#[test]
fn an_inside_out_range_is_straightened_and_a_bad_step_falls_back() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        // What a half-typed pair of numbers looks like. Refusing it would mean the fields
        // could not be edited left to right.
        range: ControlRange {
            min: 20.0,
            max: 10.0,
            step: 0.0,
        },
    })
    .unwrap();
    let r = range(&app, id, "frequency");
    assert_eq!((r.min, r.max), (10.0, 20.0));
    assert!(r.step > 0.0, "a step of zero is not a quantum");
}

#[test]
fn clearing_a_range_hands_the_control_back_to_its_definition() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 2.0,
            max: 4.0,
            step: 1.0,
        },
    })
    .unwrap();
    app.apply(Command::ClearRange {
        node: id,
        key: "frequency",
    })
    .unwrap();
    assert_eq!(range(&app, id, "frequency").max, 64.0);
    assert!(app.graph().get(id).is_some_and(|n| n.values.is_empty()));
}

#[test]
fn a_range_on_a_key_that_is_not_a_number_is_refused() {
    let (mut app, id) = patch();
    for key in ["color1", "nonsense"] {
        assert!(matches!(
            app.apply(Command::SetRange {
                node: id,
                key,
                range: ControlRange {
                    min: 0.0,
                    max: 1.0,
                    step: 0.1,
                },
            }),
            Err(CommandError::NoSuchKey(..)),
        ));
    }
}

#[test]
fn a_range_is_undoable_and_takes_the_value_it_moved_with_it() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 1.0,
            max: 4.0,
            step: 1.0,
        },
    })
    .unwrap();
    assert_eq!(value(&app, id, "frequency"), 4.0);

    assert!(app.undo());
    assert_eq!(range(&app, id, "frequency").max, 64.0);
    assert_eq!(
        value(&app, id, "frequency"),
        8.0,
        "undo is a snapshot, so the value the range dragged in comes back with it"
    );
}

#[test]
fn a_range_never_recompiles() {
    let (mut app, id) = patch();
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::new(300.0, 0.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let out = app.graph().iter().map(|(i, _)| i).max().expect("two nodes");
    app.apply(Command::Connect {
        from: PortRef::new(id, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let _ = app.take_recompiles();

    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 1.0,
            max: 4.0,
            step: 1.0,
        },
    })
    .unwrap();
    assert!(
        !app.needs_recompile(out),
        "a range moves a control's ends, and the control is already a uniform"
    );
}

#[test]
fn a_range_rides_in_the_workspace_file() {
    let (mut app, id) = patch();
    let narrowed = ControlRange {
        min: 2.0,
        max: 6.0,
        step: 0.5,
    };
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: narrowed,
    })
    .unwrap();

    let json = serde_json::to_string(&file_of(app.graph())).expect("a workspace serializes");
    let (graph, warnings) = supersilvia::workspace::from_str(&json).expect("and reads back");
    assert!(warnings.is_empty(), "{warnings:?}");

    let node = graph.get(id).expect("the node round-tripped");
    assert_eq!(
        node.values
            .get("frequency")
            .and_then(supersilvia::graph::Value::range),
        Some(narrowed),
    );
}

/// A node whose controls all sit at their definition's range writes no `ranges` at all,
/// which is almost every node in almost every graph.
#[test]
fn a_node_with_no_range_of_its_own_writes_none() {
    let (app, id) = patch();
    let json = serde_json::to_string(&file_of(app.graph())).expect("a workspace serializes");
    assert!(
        !json.contains("values"),
        "an empty map is not written, so a graph without one is byte-identical: {json}"
    );

    let (graph, warnings) = supersilvia::workspace::from_str(&json).expect("reads back");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(graph.get(id).is_some_and(|n| n.values.is_empty()));
}

/// `Ctrl` on a stepper changes the step, which is a third of a range and so document data:
/// it rides in the file and it is one undo step, unlike the scrub the same button performs.
#[test]
fn a_changed_step_rides_in_the_file_and_undoes_in_one() {
    let (mut app, id) = patch();
    let steps = app.undo_len();
    let coarser = ControlRange {
        min: 1.0,
        max: 64.0,
        step: 10.0,
    };
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: coarser,
    })
    .unwrap();
    assert_eq!(range(&app, id, "frequency").step, 10.0);
    assert_eq!(
        value(&app, id, "frequency"),
        8.0,
        "a coarser quantum does not move the value that is already dialed"
    );

    let json = serde_json::to_string(&file_of(app.graph())).expect("a workspace serializes");
    let (graph, warnings) = supersilvia::workspace::from_str(&json).expect("and reads back");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        graph
            .get(id)
            .and_then(|n| n.values.get("frequency"))
            .and_then(supersilvia::graph::Value::range),
        Some(coarser),
        "the step the stepper dialed is in the file"
    );

    assert!(app.undo());
    assert_eq!(app.undo_len(), steps);
    assert_eq!(
        range(&app, id, "frequency").step,
        1.0,
        "one undo, not one per notch: a step is not a scrub"
    );
}

#[test]
fn a_duplicate_carries_the_range_it_was_given() {
    let (mut app, id) = patch();
    app.apply(Command::SetRange {
        node: id,
        key: "frequency",
        range: ControlRange {
            min: 2.0,
            max: 6.0,
            step: 0.5,
        },
    })
    .unwrap();
    app.apply(Command::Duplicate {
        nodes: vec![id],
        offset: emath::vec2(24.0, 24.0),
    })
    .unwrap();

    let copy = app.graph().iter().map(|(i, _)| i).max().expect("a copy");
    assert_ne!(copy, id);
    assert_eq!(
        range(&app, copy, "frequency").max,
        6.0,
        "a duplicate that lost the range someone dialed in is a new node with extra steps"
    );
}

/// A kaleidoscope's Source Segment picks one of the wedges Segments cut, so its top end is
/// that count and a knob running past it is travel that picks the last wedge again.
///
/// The range is the node's own, written by the same command that moved the count, so one
/// undo puts both back — which is silvia's behavior, where the listener on Segments writes
/// the `max` attribute Source Segment is serialized with.
#[test]
fn source_segment_follows_the_segment_count() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "kaleidoscope",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let id = app.graph().iter().next().expect("just added").0;
    assert_eq!(
        range(&app, id, "sourceSegment").max,
        6.0,
        "the declared top end is the default count"
    );

    let set = |app: &mut App, key: &'static str, v: f32| {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    };

    set(&mut app, "segments", 12.0);
    assert_eq!(range(&app, id, "sourceSegment").max, 12.0);
    set(&mut app, "sourceSegment", 11.0);
    assert_eq!(value(&app, id, "sourceSegment"), 11.0);

    set(&mut app, "segments", 4.0);
    assert_eq!(range(&app, id, "sourceSegment").max, 4.0);
    assert_eq!(
        value(&app, id, "sourceSegment"),
        4.0,
        "the value comes down with the count rather than sitting off its own track"
    );

    assert!(app.undo());
    assert_eq!(range(&app, id, "sourceSegment").max, 12.0);
    assert_eq!(
        value(&app, id, "sourceSegment"),
        11.0,
        "the range and the value it dragged are one step"
    );
}
