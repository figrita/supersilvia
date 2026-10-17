// SPDX-License-Identifier: AGPL-3.0-or-later

//! The list rate: any number of values a frame, held on the CPU.
//!
//! What is asserted here is the rule the design rests on. A list feeds a list and nothing
//! else; a one-value input is offered a node that picks a value out of it, never a quiet first
//! item. And the tick: a list published by one node is read by the next in the same frame,
//! and goes with its node.

use emath::Pos2;
use supersilvia::graph::{ConnectError, ControlValue, Graph, NodeId, PortRef, PortType};
use supersilvia::nodes::{self, bridge};
use supersilvia::{App, Command};

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is in the registry")
}

fn added(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .expect("slug is in the registry");
    app.graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap();
}

fn list(app: &App, port: PortRef) -> Option<Vec<f64>> {
    app.snapshot().lists.get(&port).cloned()
}

// ---------------------------------------------------------------- connection rules

#[test]
fn a_list_feeds_a_list() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let get = add(&mut g, "get");
    g.connect(PortRef::new(range, "list"), PortRef::new(get, "list"))
        .expect("a list into a list");
}

/// A list onto a number, a field or a color is refused on type. The first two are offered the
/// menu, which picks a value out of the list; a color is offered nothing.
#[test]
fn a_list_never_lands_on_a_one_value_input_and_a_number_is_offered_the_menu() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let slew = add(&mut g, "slew");
    let zoom = add(&mut g, "zoom");
    let from = PortRef::new(range, "list");
    for (to, ty, offered) in [
        (PortRef::new(slew, "input"), PortType::UniformNumber, true),
        (PortRef::new(zoom, "zoom"), PortType::VaryingNumber, true),
    ] {
        assert_eq!(
            g.connect(from, to),
            Err(ConnectError::TypeMismatch {
                from: PortType::ListNumber,
                to: ty
            }),
            "{to:?}"
        );
        assert_eq!(bridge::can_bridge(&g, from, to), offered, "{to:?}");
    }
    let out = add(&mut g, "output");
    assert!(g.connect(from, PortRef::new(out, "input")).is_err());
    assert!(!bridge::can_bridge(&g, from, PortRef::new(out, "input")));
}

/// A single number lands on a list input as a list of one; a field does not.
#[test]
fn a_number_lands_on_a_list_input_as_a_list_of_one() {
    let mut g = Graph::new();
    let slew = add(&mut g, "slew");
    let get = add(&mut g, "get");
    g.connect(PortRef::new(slew, "output"), PortRef::new(get, "list"))
        .expect("a number is a list of one");
    let field = add(&mut g, "luminosity");
    let length = add(&mut g, "length");
    assert!(matches!(
        g.connect(PortRef::new(field, "output"), PortRef::new(length, "list")),
        Err(ConnectError::TypeMismatch { .. })
    ));
}

/// A number cabled into a list is read as a list of one.
#[test]
fn a_number_into_length_counts_one() {
    let mut app = App::headless();
    let number = added(&mut app, "number");
    let length = added(&mut app, "length");
    app.apply(Command::Connect {
        from: PortRef::new(number, "output"),
        to: PortRef::new(length, "list"),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(length, "count")), Some(1.0));
}

// ---------------------------------------------------------------- the tick

/// Range, Get and Length in one frame: the list is published before its readers tick, a Get
/// picks an item, a Length counts, and a change of Count lands on the next tick.
#[test]
fn a_range_is_read_by_get_and_length_in_the_same_frame() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let get = added(&mut app, "get");
    let length = added(&mut app, "length");
    let list_out = PortRef::new(range, "list");
    for reader in [get, length] {
        app.apply(Command::Connect {
            from: list_out,
            to: PortRef::new(reader, "list"),
        })
        .unwrap();
    }
    set(&mut app, range, "count", 5.0);
    set(&mut app, get, "index", 2.0);
    app.tick(1.0 / 60.0);

    assert_eq!(list(&app, list_out), Some(vec![0.0, 0.25, 0.5, 0.75, 1.0]));
    assert_eq!(app.uniform(PortRef::new(get, "item")), Some(0.5));
    assert_eq!(app.uniform(PortRef::new(length, "count")), Some(5.0));

    set(&mut app, range, "count", 3.0);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(length, "count")), Some(3.0));
    assert_eq!(app.uniform(PortRef::new(get, "item")), Some(1.0));
}

/// A Get with nothing cabled in publishes nothing, which is a row that draws nothing, and a
/// Length counts nothing as zero.
#[test]
fn an_unplugged_get_publishes_nothing_and_an_unplugged_length_counts_zero() {
    let mut app = App::headless();
    let get = added(&mut app, "get");
    let length = added(&mut app, "length");
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(get, "item")), None);
    assert_eq!(app.uniform(PortRef::new(length, "count")), Some(0.0));
}

#[test]
fn a_list_goes_with_its_node() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    app.tick(1.0 / 60.0);
    assert_eq!(
        list(&app, PortRef::new(range, "list")).map(|l| l.len()),
        Some(8)
    );
    app.apply(Command::RemoveNodes(vec![range])).unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(list(&app, PortRef::new(range, "list")), None);
}

/// The menu's Item row is one step that lands a Get between the list and the number.
#[test]
fn the_item_row_lands_a_get_between_a_list_and_a_number() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let slew = added(&mut app, "slew");
    let workspace = app.graph().default_workspace();
    let row = bridge::bridges(PortType::ListNumber, PortType::UniformNumber)
        .iter()
        .find(|b| b.label == "Item")
        .expect("an Item row");
    app.apply(Command::Bridge {
        from: PortRef::new(range, "list"),
        to: PortRef::new(slew, "input"),
        slug: row.def.slug,
        inputs: row.inputs,
        output: row.output,
        options: row.options,
        at: Pos2::new(100.0, 0.0),
        workspace,
    })
    .expect("a list bridges to a number through a Get");
    let get = app.graph().iter().map(|(id, _)| id).max().unwrap();
    assert_eq!(app.graph().get(get).map(|n| n.def.slug), Some("get"));
    assert_eq!(app.graph().connections().len(), 2);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(slew, "output")), Some(0.0));
}

// ---------------------------------------------------------------- maths per item

fn cable(app: &mut App, from: PortRef, to: PortRef) {
    app.apply(Command::Connect { from, to }).unwrap();
}

fn ty_out(app: &App, port: PortRef) -> PortType {
    app.graph()
        .get(port.node)
        .and_then(|n| n.output(port.key))
        .expect("an output")
        .ty
}

fn ty_in(app: &App, port: PortRef) -> PortType {
    app.graph()
        .get(port.node)
        .and_then(|n| n.input(port.key))
        .expect("an input")
        .ty
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// A list into a Multiply makes it a list: one product per item, the knob on B reaching every
/// one of them, and its ports drawn as lists.
#[test]
fn a_list_into_dual_maths_runs_it_per_item() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let multiply = added(&mut app, "multiply");
    set(&mut app, range, "count", 4.0);
    set(&mut app, range, "to", 3.0);
    set(&mut app, multiply, "b", 2.0);
    cable(
        &mut app,
        PortRef::new(range, "list"),
        PortRef::new(multiply, "a"),
    );
    let out = PortRef::new(multiply, "output");
    assert_eq!(ty_out(&app, out), PortType::ListNumber);
    assert_eq!(
        ty_in(&app, PortRef::new(multiply, "b")),
        PortType::ListNumber
    );
    app.tick(1.0 / 60.0);
    assert_eq!(list(&app, out), Some(vec![0.0, 2.0, 4.0, 6.0]));
    assert_eq!(app.uniform(out), None, "a list publishes no single number");

    // Unplugged, it is a number again, and the list goes from the frame.
    app.apply(Command::Disconnect {
        to: PortRef::new(multiply, "a"),
    })
    .unwrap();
    assert_eq!(ty_out(&app, out), PortType::UniformNumber);
    app.tick(1.0 / 60.0);
    assert_eq!(list(&app, out), None);
    assert_eq!(app.uniform(out), Some(2.0));
}

/// Three items beside twelve is AAAA BBBB CCCC: the output is the longest input's length, and
/// a shorter one is stretched across it.
#[test]
fn a_shorter_list_is_stretched_over_the_longer() {
    let mut app = App::headless();
    let short = added(&mut app, "range");
    let long = added(&mut app, "range");
    let add = added(&mut app, "add");
    set(&mut app, short, "count", 3.0);
    set(&mut app, short, "from", 10.0);
    set(&mut app, short, "to", 30.0);
    set(&mut app, long, "count", 12.0);
    set(&mut app, long, "to", 11.0);
    cable(
        &mut app,
        PortRef::new(short, "list"),
        PortRef::new(add, "a"),
    );
    cable(&mut app, PortRef::new(long, "list"), PortRef::new(add, "b"));
    app.tick(1.0 / 60.0);
    let want: Vec<f64> = (0..12)
        .map(|i| [10.0, 20.0, 30.0][i / 4] + i as f64)
        .collect();
    let got = list(&app, PortRef::new(add, "output")).unwrap();
    assert!(close(&got, &want), "{got:?}");
}

/// An empty list makes the answer empty, whatever is beside it.
#[test]
fn an_empty_list_into_maths_gives_an_empty_list() {
    let mut app = App::headless();
    let none = added(&mut app, "range");
    let five = added(&mut app, "range");
    let add = added(&mut app, "add");
    set(&mut app, none, "count", 0.0);
    set(&mut app, five, "count", 5.0);
    cable(&mut app, PortRef::new(none, "list"), PortRef::new(add, "a"));
    cable(&mut app, PortRef::new(five, "list"), PortRef::new(add, "b"));
    app.tick(1.0 / 60.0);
    assert_eq!(list(&app, PortRef::new(add, "output")), Some(Vec::new()));
}

/// Range → Sine → Multiply → Get: a chain of dual nodes is a list all the way down, and
/// settles in one frame.
#[test]
fn a_chain_of_dual_nodes_carries_a_list_through_in_one_frame() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let sine = added(&mut app, "sine");
    let multiply = added(&mut app, "multiply");
    let get = added(&mut app, "get");
    set(&mut app, range, "count", 4.0);
    set(&mut app, range, "to", 1.0);
    app.apply(Command::SetOption {
        node: range,
        key: "ends",
        value: "from".to_string(),
    })
    .unwrap();
    set(&mut app, multiply, "b", 10.0);
    set(&mut app, get, "index", 1.0);
    cable(
        &mut app,
        PortRef::new(range, "list"),
        PortRef::new(sine, "input"),
    );
    cable(
        &mut app,
        PortRef::new(sine, "output"),
        PortRef::new(multiply, "a"),
    );
    cable(
        &mut app,
        PortRef::new(multiply, "output"),
        PortRef::new(get, "list"),
    );
    app.tick(1.0 / 60.0);
    let got = list(&app, PortRef::new(multiply, "output")).unwrap();
    assert!(close(&got, &[0.0, 10.0, 0.0, -10.0]), "{got:?}");
    let item = app.uniform(PortRef::new(get, "item")).unwrap();
    assert!((item - 10.0).abs() < 1e-9);
}

/// **A node fed a list and a field at once would be both**, so the second cable is refused,
/// whichever came first. A list arriving second is offered the menu, which picks a number out
/// of it; a field arriving second has nothing to offer.
#[test]
fn a_list_and_a_field_into_one_node_are_refused_at_the_second_cable() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let field = add(&mut g, "luminosity");
    let multiply = add(&mut g, "multiply");
    let list_out = PortRef::new(range, "list");
    let field_out = PortRef::new(field, "output");
    let (a, b) = (PortRef::new(multiply, "a"), PortRef::new(multiply, "b"));

    g.connect(list_out, a).expect("a list into a number");
    assert_eq!(
        g.connect(field_out, b),
        Err(ConnectError::TypeMismatch {
            from: PortType::VaryingNumber,
            to: PortType::ListNumber
        })
    );
    assert!(!bridge::can_bridge(&g, field_out, b));

    g.disconnect(a);
    g.connect(field_out, a).expect("a field into a number");
    assert_eq!(
        g.connect(list_out, b),
        Err(ConnectError::TypeMismatch {
            from: PortType::ListNumber,
            to: PortType::VaryingNumber
        })
    );
    assert!(bridge::can_bridge(&g, list_out, b));
    // Onto the one input the field is in, the list replaces it, and the node is a list.
    g.connect(list_out, a).expect("a list in the field's place");
    let out = g.get(multiply).unwrap().output("output").unwrap().ty;
    assert_eq!(out, PortType::ListNumber);
}

/// A Multiply whose number something reads takes no list: the list would reach a one-value
/// input through it. Its inputs offer the menu instead, as the reader's own would.
#[test]
fn a_dual_node_read_as_a_number_takes_no_list() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let multiply = add(&mut g, "multiply");
    let sum = add(&mut g, "add");
    let slew = add(&mut g, "slew");
    g.connect(PortRef::new(multiply, "output"), PortRef::new(sum, "a"))
        .unwrap();
    g.connect(PortRef::new(sum, "output"), PortRef::new(slew, "input"))
        .unwrap();
    let from = PortRef::new(range, "list");
    for to in [PortRef::new(multiply, "a"), PortRef::new(sum, "b")] {
        assert!(
            matches!(
                g.connect(from, to),
                Err(ConnectError::TypeMismatch {
                    from: PortType::ListNumber,
                    ..
                })
            ),
            "{to:?}"
        );
        assert!(bridge::can_bridge(&g, from, to), "{to:?}");
    }
}

/// A Multiply feeding a list input takes a list and refuses a field, which would reach the
/// list input through it.
#[test]
fn a_dual_node_read_as_a_list_takes_a_list_and_refuses_a_field() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let field = add(&mut g, "luminosity");
    let multiply = add(&mut g, "multiply");
    let length = add(&mut g, "length");
    g.connect(
        PortRef::new(multiply, "output"),
        PortRef::new(length, "list"),
    )
    .expect("a number into a list");
    assert!(matches!(
        g.connect(PortRef::new(field, "output"), PortRef::new(multiply, "a")),
        Err(ConnectError::TypeMismatch { .. })
    ));
    g.connect(PortRef::new(range, "list"), PortRef::new(multiply, "a"))
        .expect("a list through to a list");
}

/// A Multiply fed a list elsewhere refuses a field on the dual node upstream of it, which
/// would make it both.
#[test]
fn a_field_upstream_of_a_list_mode_node_is_refused() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let field = add(&mut g, "luminosity");
    let upstream = add(&mut g, "add");
    let multiply = add(&mut g, "multiply");
    g.connect(PortRef::new(range, "list"), PortRef::new(multiply, "a"))
        .unwrap();
    g.connect(
        PortRef::new(upstream, "output"),
        PortRef::new(multiply, "b"),
    )
    .expect("a number into a list-mode node");
    assert!(matches!(
        g.connect(PortRef::new(field, "output"), PortRef::new(upstream, "a")),
        Err(ConnectError::TypeMismatch { .. })
    ));
    g.connect(PortRef::new(range, "list"), PortRef::new(upstream, "a"))
        .expect("a list upstream of a list is a list");
}

// ---------------------------------------------------------------- gathering and reordering

/// A list of indices on a Get's Index gathers: the items they name, in their order, an index
/// past the end wrapping as a single one does.
#[test]
fn a_list_on_gets_index_gathers() {
    let mut app = App::headless();
    let items = added(&mut app, "range");
    let indices = added(&mut app, "range");
    let get = added(&mut app, "get");
    set(&mut app, items, "count", 4.0);
    set(&mut app, items, "from", 10.0);
    set(&mut app, items, "to", 40.0);
    set(&mut app, indices, "count", 3.0);
    set(&mut app, indices, "from", 5.0);
    set(&mut app, indices, "to", 3.0);
    cable(
        &mut app,
        PortRef::new(items, "list"),
        PortRef::new(get, "list"),
    );
    cable(
        &mut app,
        PortRef::new(indices, "list"),
        PortRef::new(get, "index"),
    );
    let item = PortRef::new(get, "item");
    assert_eq!(ty_out(&app, item), PortType::ListNumber);
    app.tick(1.0 / 60.0);
    assert_eq!(list(&app, item), Some(vec![20.0, 10.0, 40.0]));
    assert_eq!(app.uniform(item), None);

    app.apply(Command::Disconnect {
        to: PortRef::new(get, "index"),
    })
    .unwrap();
    assert_eq!(ty_out(&app, item), PortType::UniformNumber);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(item), Some(10.0));
    assert_eq!(list(&app, item), None);
}

/// Sort's Order on a Get's Index puts a sibling list in the sorted order: the one Order
/// reorders X, Y and a size alike.
#[test]
fn sorts_order_reorders_a_sibling_list_through_get() {
    let mut app = App::headless();
    let keys = added(&mut app, "randomlist");
    let sibling = added(&mut app, "range");
    let sort = added(&mut app, "sort");
    let get = added(&mut app, "get");
    set(&mut app, keys, "count", 6.0);
    set(&mut app, keys, "seed", 3.0);
    set(&mut app, sibling, "count", 6.0);
    set(&mut app, sibling, "to", 5.0);
    cable(
        &mut app,
        PortRef::new(keys, "list"),
        PortRef::new(sort, "list"),
    );
    cable(
        &mut app,
        PortRef::new(sort, "order"),
        PortRef::new(get, "index"),
    );
    cable(
        &mut app,
        PortRef::new(sibling, "list"),
        PortRef::new(get, "list"),
    );
    app.tick(1.0 / 60.0);
    let keys = list(&app, PortRef::new(keys, "list")).unwrap();
    let sorted = list(&app, PortRef::new(sort, "sorted")).unwrap();
    let order = list(&app, PortRef::new(sort, "order")).unwrap();
    assert!(sorted.windows(2).all(|w| w[0] <= w[1]));
    // The sibling is 0 to 5, so gathered by the Order it is the Order itself.
    assert_eq!(list(&app, PortRef::new(get, "item")), Some(order.clone()));
    let regathered: Vec<f64> = order.iter().map(|i| keys[*i as usize]).collect();
    assert_eq!(regathered, sorted);
}

// ---------------------------------------------------------------- the menu's readings

/// Each of the menu's readings lands a Reduce with its mode set, as one undo step, and reads
/// the list the way its row says.
#[test]
fn the_menus_readings_land_a_reduce_set_to_each_one() {
    let rows = bridge::bridges(PortType::ListNumber, PortType::UniformNumber);
    for (label, want) in [
        ("Average", 2.5),
        ("Highest", 4.0),
        ("Lowest", 1.0),
        ("Sum", 10.0),
        ("Loudest Index", 3.0),
    ] {
        let mut app = App::headless();
        let range = added(&mut app, "range");
        let slew = added(&mut app, "slew");
        set(&mut app, range, "count", 4.0);
        set(&mut app, range, "from", 1.0);
        set(&mut app, range, "to", 4.0);
        let before = app.graph().len();
        let row = rows.iter().find(|b| b.label == label).expect("a row");
        let workspace = app.graph().default_workspace();
        app.apply(Command::Bridge {
            from: PortRef::new(range, "list"),
            to: PortRef::new(slew, "input"),
            slug: row.def.slug,
            inputs: row.inputs,
            output: row.output,
            options: row.options,
            at: Pos2::new(100.0, 0.0),
            workspace,
        })
        .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let reduce = app.graph().iter().map(|(id, _)| id).max().unwrap();
        assert_eq!(app.graph().get(reduce).unwrap().def.slug, "reduce");
        app.tick(1.0 / 60.0);
        assert_eq!(
            app.uniform(PortRef::new(reduce, "value")),
            Some(want),
            "{label}"
        );
        assert!(app.undo());
        assert_eq!(app.graph().len(), before, "{label}: one undo takes it all");
        assert!(app.graph().connections().is_empty(), "{label}");
    }
}

// ---------------------------------------------------------------- the list nodes in a tick

/// Circle Points and Grid Points publish their parallel lists in one frame, and a List
/// publishes its own bars between Min and Max.
#[test]
fn the_layouts_and_the_hand_made_list_publish_their_lists() {
    let mut app = App::headless();
    let circle = added(&mut app, "circlepoints");
    let grid = added(&mut app, "gridpoints");
    let hand = added(&mut app, "list");
    set(&mut app, circle, "count", 4.0);
    set(&mut app, grid, "columns", 2.0);
    set(&mut app, grid, "rows", 3.0);
    set(&mut app, hand, "count", 3.0);
    set(&mut app, hand, "min", -1.0);
    set(&mut app, hand, "max", 1.0);
    app.apply(Command::SetValue {
        node: hand,
        key: "values",
        value: supersilvia::graph::Value::Numbers(vec![1.0, 0.5]),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    let x = list(&app, PortRef::new(circle, "x")).unwrap();
    assert!(close(&x, &[0.5, 0.0, -0.5, 0.0]), "{x:?}");
    assert_eq!(
        list(&app, PortRef::new(circle, "angle")),
        Some(vec![0.0, 0.25, 0.5, 0.75])
    );
    assert_eq!(
        list(&app, PortRef::new(grid, "column")),
        Some(vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0])
    );
    assert_eq!(
        list(&app, PortRef::new(grid, "row")),
        Some(vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0])
    );
    assert_eq!(
        list(&app, PortRef::new(hand, "list")),
        Some(vec![1.0, 0.0, -1.0]),
        "a bar past the stored ones is at Min"
    );
}

// ---------------------------------------------------------------- the file

fn round_trip(g: &Graph) -> Graph {
    let nodes: std::collections::BTreeSet<NodeId> = g.iter().map(|(id, _)| id).collect();
    let file = supersilvia::workspace::WorkspaceFile::of(g, &g.workspaces()[0], &nodes);
    let json = serde_json::to_string_pretty(&file).unwrap();
    let (back, warnings) = supersilvia::workspace::from_str(&json).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    back
}

/// A list-mode Multiply and a gathering Get open as they were saved, with every cable, and a
/// List's bars come back.
#[test]
fn a_saved_list_mode_patch_reopens_the_same() {
    let mut g = Graph::new();
    let range = add(&mut g, "range");
    let multiply = add(&mut g, "multiply");
    let get = add(&mut g, "get");
    let hand = add(&mut g, "list");
    let length = add(&mut g, "length");
    g.connect(PortRef::new(range, "list"), PortRef::new(multiply, "a"))
        .unwrap();
    g.connect(PortRef::new(multiply, "output"), PortRef::new(get, "index"))
        .unwrap();
    g.connect(PortRef::new(hand, "list"), PortRef::new(get, "list"))
        .unwrap();
    g.connect(PortRef::new(get, "item"), PortRef::new(length, "list"))
        .unwrap();
    g.get_mut(hand).unwrap().values.insert(
        "values",
        supersilvia::graph::Value::Numbers(vec![0.25, 0.75]),
    );

    let back = round_trip(&g);
    assert_eq!(back.connections().len(), 4);
    for (node, key) in [(multiply, "output"), (get, "item")] {
        assert_eq!(
            back.get(node).unwrap().output(key).unwrap().ty,
            PortType::ListNumber,
            "{key}"
        );
    }
    assert_eq!(
        back.get(multiply).unwrap().input("b").unwrap().ty,
        PortType::ListNumber
    );
    assert_eq!(
        back.get(hand)
            .unwrap()
            .values
            .get("values")
            .and_then(supersilvia::graph::Value::numbers),
        Some(&[0.25, 0.75][..])
    );
}

// ---------------------------------------------------------------- color lists

fn out_ty(app: &App, node: NodeId, key: &str) -> PortType {
    app.graph().get(node).unwrap().output(key).unwrap().ty
}

/// RGBA and HSLA are dual, as the maths is: one color a frame from knobs, so a solid's Color
/// takes it; a color list from a list, one color per item; a picture from a field.
#[test]
fn hsla_is_one_color_from_knobs_a_color_list_from_a_list_and_a_picture_from_a_field() {
    let mut app = App::headless();
    let hsla = added(&mut app, "hsla");
    assert_eq!(out_ty(&app, hsla, "output"), PortType::UniformColor);
    let paint = added(&mut app, "paint");
    app.apply(Command::Connect {
        from: PortRef::new(hsla, "output"),
        to: PortRef::new(paint, "color"),
    })
    .expect("knobs make one color, which a solid's Color takes");

    let mut app = App::headless();
    let hsla = added(&mut app, "hsla");
    let range = added(&mut app, "range");
    set(&mut app, range, "count", 4.0);
    set(&mut app, range, "to", 0.75);
    set(&mut app, hsla, "s", 1.0);
    app.apply(Command::Connect {
        from: PortRef::new(range, "list"),
        to: PortRef::new(hsla, "h"),
    })
    .unwrap();
    assert_eq!(out_ty(&app, hsla, "output"), PortType::ListColor);
    app.tick(1.0 / 60.0);
    let colors = app
        .snapshot()
        .color_lists
        .get(&PortRef::new(hsla, "output"))
        .cloned()
        .expect("a color list");
    assert_eq!(colors.len(), 4);
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
    assert!(near(colors[0], [1.0, 0.0, 0.0, 1.0]), "{:?}", colors[0]);
    assert!(near(colors[2], [0.0, 1.0, 1.0, 1.0]), "{:?}", colors[2]);

    let mut app = App::headless();
    let rgba = added(&mut app, "rgba");
    let field = added(&mut app, "luminosity");
    app.apply(Command::Connect {
        from: PortRef::new(field, "output"),
        to: PortRef::new(rgba, "r"),
    })
    .unwrap();
    assert_eq!(out_ty(&app, rgba, "output"), PortType::VaryingColor);
}

/// A color list into a repeat's Color row lands, and compiles.
#[test]
fn a_color_list_lands_on_a_repeats_color() {
    let mut app = App::headless();
    let hsla = added(&mut app, "hsla");
    let range = added(&mut app, "range");
    let repeat = added(&mut app, "repeatgrid");
    app.apply(Command::Connect {
        from: PortRef::new(range, "list"),
        to: PortRef::new(hsla, "h"),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(hsla, "output"),
        to: PortRef::new(repeat, "copyColor"),
    })
    .expect("a color list into a repeat's Color");
}
