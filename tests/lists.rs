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
    let colorize = add(&mut g, "rgba");
    let from = PortRef::new(range, "list");
    for (to, ty, offered) in [
        (PortRef::new(slew, "input"), PortType::UniformNumber, true),
        (PortRef::new(zoom, "zoom"), PortType::VaryingNumber, true),
        (PortRef::new(colorize, "r"), PortType::VaryingNumber, true),
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
