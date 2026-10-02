// SPDX-License-Identifier: AGPL-3.0-or-later

//! The event half: the port type that had no values.
//!
//! What is asserted here is the shape agreed in `docs/decisions.md` before the first
//! inhabitant existed — **an action is a gate, not a pulse** — and the two things that
//! follow from it: a receiver sees down *and* up, and an event is a CPU thing that never
//! reaches a shader.

use emath::Pos2;
use supersilvia::graph::{ConnectError, ControlValue, Graph, NodeId, PortRef, PortType};
use supersilvia::nodes::{self, Edge};
use supersilvia::{App, Command};

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is in the registry")
}

const FRAME: f32 = 1.0 / 60.0;

/// Add a node through the bus, which is the only way anything reaches an `App`'s graph.
fn add_to(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .expect("slug is in the registry");
    app.graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

/// A Master Gear half a second long, a beat at 120: the clock the event tests run on.
fn clock_to(app: &mut App) -> NodeId {
    let id = add_to(app, "mastergear");
    app.apply(Command::SetControl {
        node: id,
        key: "length",
        value: ControlValue::Float(0.5),
    })
    .unwrap();
    id
}

/// A graph of one half-second Master Gear, ticked once so its CPU state exists.
fn clock_app() -> (App, NodeId) {
    let mut app = App::headless();
    let id = clock_to(&mut app);
    app.tick(FRAME);
    (app, id)
}

/// A graph of one kind of node, ticked once so its CPU state exists.
fn app_with(slug: &'static str) -> (App, NodeId) {
    let mut app = App::headless();
    let id = add_to(&mut app, slug);
    app.tick(FRAME);
    (app, id)
}

/// The kinds of the events on a port, for asserting a sequence without their times.
fn kinds(app: &App, port: PortRef) -> Vec<Edge> {
    app.edges(port).iter().map(|e| e.edge).collect()
}

// ---------------------------------------------------------------- the type has values

#[test]
fn the_action_port_type_has_inhabitants() {
    let with_actions: Vec<&str> = nodes::REGISTRY
        .iter()
        .filter(|d| {
            d.inputs.iter().any(|p| p.ty == PortType::Action)
                || d.outputs.iter().any(|p| p.ty == PortType::Action)
        })
        .map(|d| d.slug)
        .collect();
    assert!(
        with_actions.len() >= 5,
        "the event half needs enough nodes to compose; found {with_actions:?}"
    );
    assert!(
        nodes::REGISTRY
            .iter()
            .any(|d| d.outputs.iter().any(|p| p.ty == PortType::Action)),
        "a type nothing emits is still a type with no values"
    );
}

#[test]
fn an_action_only_connects_to_an_action() {
    let mut g = Graph::new();
    let button = add(&mut g, "button");
    let slew = add(&mut g, "slew");
    let err = g
        .connect(PortRef::new(button, "trigger"), PortRef::new(slew, "input"))
        .unwrap_err();
    assert!(
        matches!(err, ConnectError::ActionMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn one_action_output_feeds_many_inputs_and_one_input_takes_many_sources() {
    let mut g = Graph::new();
    let clock = add(&mut g, "mastergear");
    let button = add(&mut g, "button");
    let counter = add(&mut g, "counter");
    let divider = add(&mut g, "clockdivider");

    g.connect(
        PortRef::new(clock, "trigger"),
        PortRef::new(counter, "increment"),
    )
    .unwrap();
    g.connect(
        PortRef::new(clock, "trigger"),
        PortRef::new(divider, "input"),
    )
    .unwrap();
    // A second source on an occupied action input joins it rather than replacing it, which
    // is the one place the graph's "an input takes one connection" rule does not apply.
    g.connect(
        PortRef::new(button, "trigger"),
        PortRef::new(counter, "increment"),
    )
    .unwrap();
    assert_eq!(g.connections().len(), 3);
    assert_eq!(
        g.sources_of(PortRef::new(counter, "increment")).count(),
        2,
        "both sources still feed the counter"
    );
}

// ---------------------------------------------------------------- a gate, not a pulse

#[test]
fn a_button_fires_down_when_pressed_and_up_when_released() {
    let (mut app, id) = app_with("button");
    let trigger = PortRef::new(id, "trigger");
    let press = PortRef::new(id, "press");

    assert!(app.edges(trigger).is_empty(), "nothing until a hand");

    app.press(press, true);
    app.tick(FRAME);
    assert_eq!(kinds(&app, trigger), [Edge::Down]);

    // Held is not retriggered: an envelope downstream stays in its sustain.
    app.tick(FRAME);
    assert!(
        app.edges(trigger).is_empty(),
        "a held gate fires once, not once a frame"
    );

    app.press(press, false);
    app.tick(FRAME);
    assert_eq!(
        kinds(&app, trigger),
        [Edge::Up],
        "the release is an event too — this is the half a pulse cannot express"
    );
}

#[test]
fn an_edge_lives_for_exactly_one_frame() {
    let (mut app, id) = app_with("button");
    app.press(PortRef::new(id, "press"), true);
    app.tick(FRAME);
    assert_eq!(app.edges(PortRef::new(id, "trigger")).len(), 1);
    app.tick(FRAME);
    assert!(
        app.edges(PortRef::new(id, "trigger")).is_empty(),
        "an event that outlived its frame would fire twice"
    );
}

// ---------------------------------------------------------------- it composes

#[test]
fn a_divider_takes_actions_and_emits_actions() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let divider = add_to(&mut app, "clockdivider");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(divider, "input"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: divider,
        key: "divide",
        value: ControlValue::Float(3.0),
    })
    .unwrap();

    let out = PortRef::new(divider, "trigger");
    let mut passed = 0;
    for beat in 0..9 {
        app.press(PortRef::new(button, "press"), true);
        app.tick(FRAME);
        if app.edges(out).iter().any(|e| e.is_down()) {
            passed += 1;
            assert_eq!(beat % 3, 0, "beat {beat} should have been swallowed");
        }
        app.press(PortRef::new(button, "press"), false);
        app.tick(FRAME);
    }
    assert_eq!(passed, 3, "one beat in three out of nine");
}

#[test]
fn a_divider_passes_the_up_that_belongs_to_a_down_it_passed() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let divider = add_to(&mut app, "clockdivider");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(divider, "input"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: divider,
        key: "divide",
        value: ControlValue::Float(2.0),
    })
    .unwrap();
    let out = PortRef::new(divider, "trigger");

    // First beat: passed, so its release is passed too.
    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    assert_eq!(kinds(&app, out), [Edge::Down]);
    app.press(PortRef::new(button, "press"), false);
    app.tick(FRAME);
    assert_eq!(kinds(&app, out), [Edge::Up]);

    // Second beat: swallowed, and so is its release — a gate downstream is never left half
    // open.
    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    assert!(app.edges(out).is_empty());
    app.press(PortRef::new(button, "press"), false);
    app.tick(FRAME);
    assert!(app.edges(out).is_empty());
}

/// Moving Division starts the group over, as silvia's does: the next beat through is the
/// first of the new group rather than wherever the running count happened to land.
#[test]
fn moving_a_dividers_division_starts_the_count_over() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let divider = add_to(&mut app, "clockdivider");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(divider, "input"),
    })
    .unwrap();
    let out = PortRef::new(divider, "trigger");
    let beat = |app: &mut App| {
        app.press(PortRef::new(button, "press"), true);
        app.tick(FRAME);
        let passed = app.edges(out).iter().any(|e| e.is_down());
        app.press(PortRef::new(button, "press"), false);
        app.tick(FRAME);
        passed
    };
    // Four by default: the first passes, and two more are swallowed.
    assert!(beat(&mut app), "the first beat passes");
    assert!(!beat(&mut app));
    assert!(!beat(&mut app));

    app.apply(Command::SetControl {
        node: divider,
        key: "divide",
        value: ControlValue::Float(3.0),
    })
    .unwrap();
    assert!(
        beat(&mut app),
        "the count started over, so this is the first of three"
    );
    assert!(!beat(&mut app));
    assert!(!beat(&mut app));
    assert!(beat(&mut app), "and the group is three from there");
}

/// The line in the body, which is the whole of what silvia's divider says about itself:
/// where it is in the group, a mark on the beat that got through, and *Ready* before
/// anything has arrived.
#[test]
fn a_divider_says_where_it_is_in_its_cycle() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let divider = add_to(&mut app, "clockdivider");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(divider, "input"),
    })
    .unwrap();
    app.tick(FRAME);
    assert_eq!(
        app.node_status(divider),
        Some("Ready ÷4"),
        "nothing has arrived yet"
    );

    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    assert_eq!(
        app.node_status(divider),
        Some("● 1 of 4"),
        "the beat that passed is marked"
    );
    app.press(PortRef::new(button, "press"), false);

    // The mark outlives the tick that set it, or a frame could paint between two ticks and
    // miss the flash entirely.
    app.tick(FRAME);
    assert_eq!(app.node_status(divider), Some("● 1 of 4"));
    for _ in 0..10 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.node_status(divider),
        Some("1 of 4"),
        "and then it goes out"
    );
}

// ---------------------------------------------------------------- events become numbers

#[test]
fn a_counter_turns_edges_into_a_uniform() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let counter = add_to(&mut app, "counter");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(counter, "increment"),
    })
    .unwrap();
    // A fresh counter is silvia's fade — 0 to 1 in hundredths — so counting whole numbers
    // over 0..8 is something this test asks for rather than something it inherits.
    app.apply(Command::SetControl {
        node: counter,
        key: "step",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: counter,
        key: "max",
        value: ControlValue::Float(8.0),
    })
    .unwrap();

    let value = PortRef::new(counter, "value");
    app.tick(FRAME);
    assert_eq!(app.uniform(value), Some(0.0), "a counter starts at its min");

    for expected in 1..=3u8 {
        app.press(PortRef::new(button, "press"), true);
        app.tick(FRAME);
        app.press(PortRef::new(button, "press"), false);
        app.tick(FRAME);
        assert_eq!(
            app.uniform(value),
            Some(f32::from(expected)),
            "the release must not count as a second beat"
        );
    }
    // 0..8 as set above, so three of eight.
    assert_eq!(
        app.uniform(PortRef::new(counter, "normalized")),
        Some(0.375)
    );
}

#[test]
fn a_counter_clamps_at_its_ends_and_wraps_when_told_to() {
    let (mut app, id) = app_with("counter");
    for (key, value) in [("max", 2.0), ("step", 1.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    let press = PortRef::new(id, "increment");
    let value = PortRef::new(id, "value");

    let beat = |app: &mut App| {
        app.press(press, true);
        app.tick(FRAME);
        app.press(press, false);
        app.tick(FRAME);
    };
    beat(&mut app);
    beat(&mut app);
    assert_eq!(app.uniform(value), Some(2.0));
    beat(&mut app);
    assert_eq!(app.uniform(value), Some(2.0), "clamped at the top");

    app.apply(Command::SetOption {
        node: id,
        key: "ends",
        value: "wrap".to_string(),
    })
    .unwrap();
    beat(&mut app);
    assert_eq!(
        app.uniform(value),
        Some(1.0),
        "three beats over 0..2, wrapped, is one"
    );
}

/// silvia's own counter is a fade: 0 to 1 in hundredths, with Min and Max nudged in whole
/// numbers. A patch built around that one has to arrive as that one rather than as an
/// eight-step index, which is a different node to reach for.
#[test]
fn a_fresh_counter_is_silvias_fade() {
    let (app, id) = app_with("counter");
    let node = app.graph().get(id).expect("just added");
    for (key, want) in [("min", 0.0), ("max", 1.0), ("step", 0.01)] {
        assert_eq!(
            node.controls.get(key),
            Some(&ControlValue::Float(want)),
            "{key} opens where silvia's does"
        );
    }
    assert_eq!(app.uniform(PortRef::new(id, "value")), Some(0.0));
    assert_eq!(app.uniform(PortRef::new(id, "normalized")), Some(0.0));
}

/// silvia's fourth button: the way to the top of a range from a trigger, rather than by
/// holding Increment until it gets there.
#[test]
fn set_to_max_sends_the_count_to_the_top_of_the_range() {
    let (mut app, id) = app_with("counter");
    let value = PortRef::new(id, "value");
    assert_eq!(app.uniform(value), Some(0.0), "a counter starts at its min");

    app.press(PortRef::new(id, "setmax"), true);
    app.tick(FRAME);
    assert_eq!(app.uniform(value), Some(1.0), "straight to Max");
    assert_eq!(
        app.uniform(PortRef::new(id, "normalized")),
        Some(1.0),
        "and the fraction says so"
    );

    app.press(PortRef::new(id, "setmax"), false);
    app.press(PortRef::new(id, "reset"), true);
    app.tick(FRAME);
    assert_eq!(app.uniform(value), Some(0.0), "and Reset is the other end");
}

// ---------------------------------------------------------------- smoothcounter

/// The smoothed value eases toward the target rather than snapping to it — the whole point
/// of the node over `counter` — and settles on it once the exponential has had time to run.
#[test]
fn a_smooth_counter_approaches_its_target_and_settles() {
    let (mut app, id) = app_with("smoothcounter");
    // A whole step over the default range, so the numbers below are about the easing.
    app.apply(Command::SetControl {
        node: id,
        key: "step",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    let value = PortRef::new(id, "value");
    let target = PortRef::new(id, "target");

    app.press(PortRef::new(id, "increment"), true);
    app.tick(FRAME);
    assert_eq!(
        app.uniform(target),
        Some(1.0),
        "the target steps immediately"
    );
    let after_one_frame = app.uniform(value).unwrap();
    assert!(
        after_one_frame > 0.0 && after_one_frame < 1.0,
        "eased toward the target, not snapped to it: {after_one_frame}"
    );
    app.press(PortRef::new(id, "increment"), false);
    app.tick(FRAME);

    for _ in 0..300 {
        app.tick(FRAME);
    }
    let settled = app.uniform(value).unwrap();
    assert!(
        (settled - 1.0).abs() < 1e-3,
        "settled on the target: {settled}"
    );
}

/// `counter`'s own three actions are the whole set — there is no `jump` — so `reset` still
/// only moves the target, and the value eases back to it like any other change.
#[test]
fn a_smooth_counter_clamps_at_its_ends_like_counter() {
    let (mut app, id) = app_with("smoothcounter");
    for (key, value) in [("max", 2.0), ("step", 1.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    let target = PortRef::new(id, "target");

    let beat = |app: &mut App| {
        app.press(PortRef::new(id, "increment"), true);
        app.tick(FRAME);
        app.press(PortRef::new(id, "increment"), false);
        app.tick(FRAME);
    };
    for _ in 0..5 {
        beat(&mut app);
    }
    assert_eq!(
        app.uniform(target),
        Some(2.0),
        "clamped at the top, like counter"
    );

    app.press(PortRef::new(id, "reset"), true);
    app.tick(FRAME);
    assert_eq!(
        app.uniform(target),
        Some(0.0),
        "reset moves the target to min"
    );
}

/// Wrap mode wraps `value` and `target` together, silvia's `_advanceSmoothing`, so the
/// smoothed value follows the target's wrap rather than sweeping the long way round.
#[test]
fn a_smooth_counter_wraps_target_and_value_together() {
    let (mut app, id) = app_with("smoothcounter");
    app.apply(Command::SetOption {
        node: id,
        key: "ends",
        value: "wrap".to_string(),
    })
    .unwrap();
    for (key, value) in [("max", 2.0), ("step", 1.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    let value = PortRef::new(id, "value");
    let target = PortRef::new(id, "target");

    let beat = |app: &mut App| {
        app.press(PortRef::new(id, "increment"), true);
        app.tick(FRAME);
        app.press(PortRef::new(id, "increment"), false);
        app.tick(FRAME);
    };
    // Three beats over 0..2 is a wrap to 1, the same arithmetic `counter` wraps with.
    for _ in 0..3 {
        beat(&mut app);
    }
    for _ in 0..300 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(target),
        Some(1.0),
        "wrapped, like counter's own value"
    );
    let settled = app.uniform(value).unwrap();
    assert!(
        (settled - 1.0).abs() < 1e-3,
        "the smoothed value wraps too: {settled}"
    );
}

/// silvia's Smooth Counter is a fade too — 0 to 1 in tenths — and it publishes the fraction
/// a fade or an opacity takes, the way `counter` does. `target` is ours, and stays.
#[test]
fn a_fresh_smooth_counter_is_silvias_fade_and_publishes_its_fraction() {
    let (mut app, id) = app_with("smoothcounter");
    let node = app.graph().get(id).expect("just added");
    for (key, want) in [("min", 0.0), ("max", 1.0), ("step", 0.1)] {
        assert_eq!(
            node.controls.get(key),
            Some(&ControlValue::Float(want)),
            "{key} opens where silvia's does"
        );
    }

    app.press(PortRef::new(id, "increment"), true);
    app.tick(FRAME);
    app.press(PortRef::new(id, "increment"), false);
    for _ in 0..300 {
        app.tick(FRAME);
    }
    let value = app.uniform(PortRef::new(id, "value")).unwrap();
    let normalized = app.uniform(PortRef::new(id, "normalized")).unwrap();
    assert!((value - 0.1).abs() < 1e-3, "one nudge of a tenth: {value}");
    assert!(
        (normalized - value).abs() < 1e-6,
        "a tenth of 0..1 is the value itself: {normalized}"
    );
    let target = app.uniform(PortRef::new(id, "target")).unwrap();
    assert!(
        (target - 0.1).abs() < 1e-6,
        "and the target is still published beside it: {target}"
    );
}

/// The fraction is of the range, and never leaves 0 to 1 — silvia's own runs outside it
/// whenever the count does.
#[test]
fn a_smooth_counters_fraction_is_of_its_range_and_stays_inside_it() {
    let (mut app, id) = app_with("smoothcounter");
    for (key, value) in [("min", 2.0), ("max", 6.0), ("step", 1.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    for _ in 0..2 {
        app.press(PortRef::new(id, "increment"), true);
        app.tick(FRAME);
        app.press(PortRef::new(id, "increment"), false);
        app.tick(FRAME);
    }
    for _ in 0..300 {
        app.tick(FRAME);
    }
    let normalized = app.uniform(PortRef::new(id, "normalized")).unwrap();
    // A count held at the floor by the range and then nudged once is three of 2..6.
    assert!(
        (normalized - 0.25).abs() < 1e-3,
        "three of 2..6 is a quarter of the way: {normalized}"
    );
    assert!(
        (0.0..=1.0).contains(&normalized),
        "and it never leaves 0..1"
    );
}

#[test]
fn an_envelope_is_held_at_its_sustain_for_as_long_as_the_gate_is_down() {
    let (mut app, id) = app_with("adsr");
    for (key, v) in [("attack", 0.0), ("decay", 0.0), ("release", 0.5)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let value = PortRef::new(id, "value");
    let gate = PortRef::new(id, "gate");

    app.tick(FRAME);
    assert_eq!(app.uniform(value), Some(0.0), "idle is silent");

    app.press(gate, true);
    app.tick(FRAME);
    app.tick(FRAME);
    let sustain = app.uniform(value).unwrap();
    assert!(
        (sustain - 0.7).abs() < 1e-4,
        "reached its sustain, silvia's own 0.7: {sustain}"
    );

    // The part a pulse could not express: nothing else happens, and it stays there.
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(value),
        Some(sustain),
        "a held gate holds the envelope"
    );

    app.press(gate, false);
    for _ in 0..40 {
        app.tick(FRAME);
    }
    assert_eq!(app.uniform(value), Some(0.0), "and letting go releases it");
}

/// silvia's defaults, which is what a fresh node hands you: a hundredth of a second of
/// attack, a tenth of decay, 0.7 of sustain, two tenths of release, and knobs that move in
/// thousandths so a one-millisecond attack is a nudge away.
#[test]
fn an_envelope_opens_on_silvias_four_times_and_its_thousandth_step() {
    let def = nodes::find("adsr").expect("in the registry");
    let expected = [
        ("attack", 0.01, 0.001),
        ("decay", 0.1, 0.001),
        ("sustain", 0.7, 0.01),
        ("release", 0.2, 0.001),
    ];
    for (key, default, step) in expected {
        let input = def.input(key).expect("a control of its own");
        let nodes::Control::Number {
            default: opens_at,
            step: nudge,
            ..
        } = input.control
        else {
            panic!("{key} is a number");
        };
        assert!(
            (opens_at - default).abs() < 1e-6,
            "{key}: silvia opens at {default}, not {opens_at}"
        );
        assert!(
            (nudge - step).abs() < 1e-6,
            "{key}: silvia's knob moves in {step}, not {nudge}"
        );
    }
}

/// Every stage follows its curve over its own time. Exponential is silvia's default on the
/// decay and the release, and it is what gives a fall its taper: half way through the time,
/// an exponential fall is most of the way down and a straight one is exactly half.
#[test]
fn a_stage_follows_its_curve_rather_than_a_straight_rate() {
    let half_way = |curve: &'static str| {
        let (mut app, id) = app_with("adsr");
        for (key, v) in [
            ("attack", 0.0),
            ("decay", 0.0),
            ("sustain", 1.0),
            ("release", 20.0 * FRAME),
        ] {
            app.apply(Command::SetControl {
                node: id,
                key,
                value: ControlValue::Float(v),
            })
            .unwrap();
        }
        app.apply(Command::SetOption {
            node: id,
            key: "releaseCurve",
            value: curve.to_string(),
        })
        .unwrap();
        let gate = PortRef::new(id, "gate");
        app.press(gate, true);
        app.tick(FRAME);
        assert_eq!(app.uniform(PortRef::new(id, "value")), Some(1.0));
        app.press(gate, false);
        // Ten of the release's twenty frames.
        for _ in 0..10 {
            app.tick(FRAME);
        }
        app.uniform(PortRef::new(id, "value")).unwrap()
    };

    let linear = half_way("linear");
    assert!(
        (linear - 0.5).abs() < 0.03,
        "a straight fall is half way down half way through: {linear}"
    );
    let exponential = half_way("exponential");
    assert!(
        exponential < 0.2,
        "silvia's exponential fall is most of the way down by then: {exponential}"
    );
    let logarithmic = half_way("logarithmic");
    assert!(
        logarithmic > 0.2,
        "and its logarithmic one is barely started: {logarithmic}"
    );
}

/// silvia's retrigger, which is the choice recorded in docs/decisions.md: a fresh gate snaps
/// to zero and attacks from there rather than carrying on from what is left of a release.
#[test]
fn a_fresh_gate_restarts_the_attack_from_zero() {
    let (mut app, id) = app_with("adsr");
    for (key, v) in [
        ("attack", 20.0 * FRAME),
        ("decay", 0.0),
        ("sustain", 1.0),
        ("release", 100.0),
    ] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let value = PortRef::new(id, "value");
    let gate = PortRef::new(id, "gate");

    app.press(gate, true);
    for _ in 0..10 {
        app.tick(FRAME);
    }
    let half = app.uniform(value).unwrap();
    assert!(half > 0.3 && half < 0.7, "half way up the attack: {half}");

    // A release so long it has barely moved, then a fresh gate.
    app.press(gate, false);
    app.tick(FRAME);
    assert!(app.uniform(value).unwrap() > 0.3, "the release is slow");
    app.press(gate, true);
    app.tick(FRAME);
    let restarted = app.uniform(value).unwrap();
    assert!(
        restarted < 0.1,
        "the attack starts from zero, not from what was left: {restarted}"
    );
}

// ---------------------------------------------------------------- a clock

#[test]
fn a_clock_fires_on_the_beat_and_closes_its_gate_within_it() {
    let (mut app, id) = clock_app();
    // 120 bpm on the beat: half a second between downs.
    let trigger = PortRef::new(id, "trigger");
    let mut downs = 0;
    let mut ups = 0;
    // Two seconds of frames, and a quarter more for the last beat's gate to close.
    for _ in 0..140 {
        app.tick(FRAME);
        for event in app.edges(trigger) {
            match event.edge {
                Edge::Down => downs += 1,
                Edge::Up => ups += 1,
            }
        }
    }
    assert_eq!(downs, 4, "four beats in two seconds at 120 bpm");
    assert_eq!(ups, downs, "every gate this clock opened, it closed");
}

/// silvia's Triplet is three to the beat: a Master Gear a sixth of a second long, a third of a
/// beat at 120, counted.
#[test]
fn a_triplet_is_three_beats_to_the_quarter_note() {
    let (mut app, id) = clock_app();
    app.apply(Command::SetControl {
        node: id,
        key: "length",
        value: ControlValue::Float(1.0 / 6.0),
    })
    .unwrap();
    let trigger = PortRef::new(id, "trigger");
    let mut downs = 0;
    // Two seconds at 120 bpm is four quarter notes, so twelve triplets.
    for _ in 0..120 {
        app.tick(FRAME);
        downs += app.edges(trigger).iter().filter(|e| e.is_down()).count();
    }
    assert_eq!(downs, 12, "three to the beat over four beats");
}

/// Hold is silvia's Start/Stop, the first thing a hand reaches for, as a gear's toggle: the
/// clock holds where it stands, and the gate it was holding open closes rather than being
/// left held; a second press runs it on from there.
#[test]
fn hold_freezes_the_clock_and_closes_the_gate_it_left_open() {
    let (mut app, id) = clock_app();
    let trigger = PortRef::new(id, "trigger");
    let hold = PortRef::new(id, "hold");

    // Run until a beat's gate has just opened.
    let mut open = false;
    for _ in 0..60 {
        app.tick(FRAME);
        if app.edges(trigger).iter().any(|e| e.is_down()) {
            open = true;
            break;
        }
    }
    assert!(open, "a beat within a second");
    app.press(hold, true);
    app.tick(FRAME);
    assert_eq!(
        kinds(&app, trigger),
        [Edge::Up],
        "the gate it was holding is closed on the way down"
    );
    app.press(hold, false);
    let held = app.uniform(PortRef::new(id, "cycles")).unwrap();
    for _ in 0..120 {
        app.tick(FRAME);
        assert!(
            app.edges(trigger).is_empty(),
            "a held clock is a clock that says nothing"
        );
    }
    assert_eq!(app.uniform(PortRef::new(id, "cycles")), Some(held));

    app.press(hold, true);
    app.tick(FRAME);
    app.press(hold, false);
    let mut downs = 0;
    for _ in 0..120 {
        app.tick(FRAME);
        downs += app.edges(trigger).iter().filter(|e| e.is_down()).count();
    }
    assert!(downs >= 3, "and released it runs: {downs}");
}

#[test]
fn a_clock_drives_a_counter_through_a_divider() {
    let mut app = App::headless();
    let clock = clock_to(&mut app);
    let divider = add_to(&mut app, "clockdivider");
    let counter = add_to(&mut app, "counter");
    for (from, to) in [
        (
            PortRef::new(clock, "trigger"),
            PortRef::new(divider, "input"),
        ),
        (
            PortRef::new(divider, "trigger"),
            PortRef::new(counter, "increment"),
        ),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.apply(Command::SetControl {
        node: divider,
        key: "divide",
        value: ControlValue::Float(2.0),
    })
    .unwrap();
    // Whole beats out of a counter whose own default counts in hundredths.
    app.apply(Command::SetControl {
        node: counter,
        key: "step",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: counter,
        key: "max",
        value: ControlValue::Float(8.0),
    })
    .unwrap();

    // Four seconds at 120 bpm is eight beats, and four of them survive the divider.
    for _ in 0..240 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(PortRef::new(counter, "value")),
        Some(4.0),
        "eight beats, halved, counted — the whole chain in one assertion"
    );
}

// ---------------------------------------------------------------- and never a uniform

#[test]
fn an_action_never_reaches_the_shader() {
    let mut app = App::headless();
    let clock = clock_to(&mut app);
    let button = add_to(&mut app, "button");
    let counter = add_to(&mut app, "counter");
    let checkerboard = add_to(&mut app, "checkerboard");
    let zoom = add_to(&mut app, "zoom");
    let output = add_to(&mut app, "output");
    for (from, to) in [
        // The event half: a clock through a button into a counter.
        (
            PortRef::new(clock, "trigger"),
            PortRef::new(button, "press"),
        ),
        (
            PortRef::new(button, "trigger"),
            PortRef::new(counter, "increment"),
        ),
        // And the one door out of it, which is a number.
        (PortRef::new(counter, "value"), PortRef::new(zoom, "zoom")),
        (
            PortRef::new(checkerboard, "output"),
            PortRef::new(zoom, "input"),
        ),
        (PortRef::new(zoom, "output"), PortRef::new(output, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }

    let shader =
        supersilvia::compile::wgsl::build(app.graph(), output).expect("an output compiles");
    for name in shader.uniforms.keys() {
        assert!(
            !name.contains("button") && !name.contains("mastergear"),
            "{name}: an event is a CPU thing and has no uniform"
        );
    }
    assert!(
        !shader.source().contains("button") && !shader.source().contains("mastergear"),
        "the event half reaches no shader at all"
    );
    // The counter does, because it publishes a number. That is the whole boundary: events
    // stay on the CPU and the uniform number they produce is the one thing that crosses.
    assert!(
        shader
            .uniforms
            .keys()
            .any(|n| n.contains(&format!("counter{counter}"))),
        "the uniform number a counter publishes is a uniform: {:?}",
        shader.uniforms.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------- finer than a frame

#[test]
fn a_beat_that_falls_inside_a_frame_says_where_it_fell() {
    // A beat at 121 BPM, not a whole number of frames, so beats land mid-frame.
    let (mut app, id) = clock_app();
    app.apply(Command::SetControl {
        node: id,
        key: "length",
        value: ControlValue::Float(60.0 / 121.0),
    })
    .unwrap();
    let trigger = PortRef::new(id, "trigger");

    let mut stamps = Vec::new();
    for _ in 0..300 {
        app.tick(FRAME);
        stamps.extend(
            app.edges(trigger)
                .iter()
                .filter(|e| e.is_down())
                .map(|e| e.at),
        );
    }
    assert!(stamps.len() >= 8, "five seconds at 121 bpm: {stamps:?}");
    assert!(
        stamps.iter().all(|at| (0.0..=FRAME).contains(at)),
        "a stamp is an offset inside its own frame: {stamps:?}"
    );
    assert!(
        stamps
            .iter()
            .any(|at| *at > 0.1 * FRAME && *at < 0.9 * FRAME),
        "at this tempo most beats fall mid-frame, and saying `now` would be a lie: {stamps:?}"
    );
}

#[test]
fn a_frame_longer_than_the_interval_keeps_every_beat_in_order() {
    // A tenth of a second of frame — a hitch, or an offline render stepping coarsely — at a
    // subdivision four times faster than that. Frame-quantized, this is one beat and three
    // dropped ones.
    let (mut app, id) = clock_app();
    app.apply(Command::SetControl {
        node: id,
        key: "length",
        value: ControlValue::Float(0.025),
    })
    .unwrap();

    app.tick(0.1);
    let downs: Vec<f32> = app
        .edges(PortRef::new(id, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .map(|e| e.at)
        .collect();
    assert!(
        downs.len() >= 3,
        "a 100 ms frame holds four 25 ms beats: {downs:?}"
    );
    assert!(
        downs.windows(2).all(|w| w[0] < w[1]),
        "and they arrive in the order they happened: {downs:?}"
    );
}

#[test]
fn an_envelope_uses_the_part_of_the_frame_after_the_gate_opened() {
    // Attack of exactly one frame, from a clock whose beats land mid-frame. If the envelope
    // integrated in one lump per frame it would read 0.0 (the gate opened "at the end") or
    // 1.0 (a whole frame of attack); the truth is the fraction of the frame that was left.
    let mut app = App::headless();
    let clock = clock_to(&mut app);
    let adsr = add_to(&mut app, "adsr");
    app.apply(Command::Connect {
        from: PortRef::new(clock, "trigger"),
        to: PortRef::new(adsr, "gate"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: clock,
        key: "length",
        value: ControlValue::Float(60.0 / 121.0),
    })
    .unwrap();
    // Release of zero, so the envelope is back at rest before each beat and the frame under
    // test starts from idle rather than from whatever the last one left.
    for (key, v) in [
        ("attack", FRAME),
        ("decay", 0.0),
        ("sustain", 1.0),
        ("release", 0.0),
    ] {
        app.apply(Command::SetControl {
            node: adsr,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }

    let trigger = PortRef::new(clock, "trigger");
    let value = PortRef::new(adsr, "value");
    let mut partial = None;
    for _ in 0..300 {
        app.tick(FRAME);
        let at = app
            .edges(trigger)
            .iter()
            .find(|e| e.is_down())
            .map(|e| e.at);
        if let Some(at) = at
            && at > 0.2 * FRAME
            && at < 0.8 * FRAME
        {
            partial = Some((at, app.uniform(value).expect("the envelope published")));
            break;
        }
    }
    let (at, value) = partial.expect("a beat landed mid-frame within five seconds");
    let expected = (FRAME - at) / FRAME;
    assert!(
        (value - expected).abs() < 0.05,
        "the gate opened {at}s into a {FRAME}s frame, so the attack had {expected} of a frame \
         to run, not a whole one or none: got {value}"
    );
}

#[test]
fn a_clock_does_not_drift_against_a_frame_rate_that_does_not_divide_it() {
    let (mut app, id) = clock_app();
    app.apply(Command::SetControl {
        node: id,
        key: "length",
        value: ControlValue::Float(60.0 / 137.0),
    })
    .unwrap();
    let trigger = PortRef::new(id, "trigger");

    // Ten seconds of an awkward frame time against an awkward tempo.
    let dt = 1.0 / 59.94;
    let frames = (10.0 / dt) as u32;
    let mut downs = 0;
    for _ in 0..frames {
        app.tick(dt);
        downs += app.edges(trigger).iter().filter(|e| e.is_down()).count();
    }
    // 137 bpm for ten seconds is 22.8 beats. Phase is accumulated, not reset per beat, so
    // the count follows the tempo rather than the frame rate.
    assert!(
        (22..=23).contains(&downs),
        "expected about 23 beats in ten seconds at 137 bpm, got {downs}"
    );
}

#[test]
fn a_chain_of_actions_is_delivered_in_one_frame() {
    // The tick order has to respect action edges. If it only ordered data edges, each hop
    // would cost a frame — and which hop, and how many, would depend on node ids.
    let mut app = App::headless();
    // Added in reverse, so a tick order that ignored action edges would run them backwards.
    let counter = add_to(&mut app, "counter");
    let divider = add_to(&mut app, "clockdivider");
    let button = add_to(&mut app, "button");
    for (from, to) in [
        (
            PortRef::new(button, "trigger"),
            PortRef::new(divider, "input"),
        ),
        (
            PortRef::new(divider, "trigger"),
            PortRef::new(counter, "increment"),
        ),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.apply(Command::SetControl {
        node: divider,
        key: "divide",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: counter,
        key: "step",
        value: ControlValue::Float(1.0),
    })
    .unwrap();

    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    assert_eq!(
        app.uniform(PortRef::new(counter, "value")),
        Some(1.0),
        "two hops, one frame"
    );
}

/// A gear's Hold is an action input, so a cable works the button. A `button` into a Master
/// Gear's `hold` stops the clock from the graph and starts it again — the door the event half
/// exists to open, in the other direction from `counter`.
#[test]
fn a_cable_works_a_clocks_transport() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let clock = clock_to(&mut app);
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(clock, "hold"),
    })
    .unwrap();

    // Two beats a second at the default 120 BPM.
    let seconds = PortRef::new(clock, "cycles");
    for _ in 0..30 {
        app.tick(FRAME);
    }
    let running = app.uniform(seconds).expect("a clock starts running");
    assert!(running > 0.4, "half a second of it: {running}");

    // One down from the cable flips it. The release is an up, and an up is not a flip.
    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    app.press(PortRef::new(button, "press"), false);
    let stopped = app.uniform(seconds).unwrap();
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(app.uniform(seconds), Some(stopped), "stopped by a cable");

    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    app.press(PortRef::new(button, "press"), false);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert!(
        app.uniform(seconds).unwrap() > stopped + 0.4,
        "and started by the next one"
    );
}

// ---------------------------------------------------------------- muxevent

/// The channel wraps in both directions: `next` past the last one is the first again, and
/// `prev` from the first is the last; `reset` returns to it directly. The number published is
/// the row's own label, so the first input reads 1 and the fourth reads 4.
#[test]
fn a_mux_events_index_wraps_both_ways_and_reset_returns_to_the_first() {
    let (mut app, id) = app_with("muxevent");
    let index = PortRef::new(id, "index");
    assert_eq!(app.uniform(index), Some(1.0), "starts on the first input");

    let press = |app: &mut App, key: &'static str| {
        app.press(PortRef::new(id, key), true);
        app.tick(FRAME);
        app.press(PortRef::new(id, key), false);
        app.tick(FRAME);
    };

    for expected in [2.0, 3.0, 4.0, 1.0] {
        press(&mut app, "next");
        assert_eq!(
            app.uniform(index),
            Some(expected),
            "next wraps past the last"
        );
    }

    press(&mut app, "prev");
    assert_eq!(
        app.uniform(index),
        Some(4.0),
        "prev from the first wraps to the last"
    );

    press(&mut app, "reset");
    assert_eq!(app.uniform(index), Some(1.0), "reset returns to the first");
}

/// Random lands on one of the four and never outside them, and over enough presses it is not
/// one channel over and over: silvia's own button, seeded from the node's id so the run is
/// the same one every time.
#[test]
fn a_mux_events_random_lands_inside_the_four_and_moves() {
    let (mut app, id) = app_with("muxevent");
    let index = PortRef::new(id, "index");
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        app.press(PortRef::new(id, "rand"), true);
        app.tick(FRAME);
        app.press(PortRef::new(id, "rand"), false);
        app.tick(FRAME);
        let value = app.uniform(index).expect("the channel is published");
        assert!(
            (1.0..=4.0).contains(&value) && value.fract() == 0.0,
            "a random channel is one of the four, not {value}"
        );
        seen.insert(value as i32);
    }
    assert_eq!(
        seen.len(),
        4,
        "sixty-four presses reach every channel, not {seen:?}"
    );
}

// ---------------------------------------------------------------- randomfire

/// At full temperature it fires often, every stamp lands inside its own frame, and every
/// down this run produced was eventually followed by its own up — the pulse silvia's node
/// never had, closed here after the gate.
#[test]
fn a_random_fire_fires_within_a_frame_at_its_rate_and_pairs_every_gate() {
    let mut app = App::headless();
    let rf = add_to(&mut app, "randomfire");
    app.apply(Command::SetControl {
        node: rf,
        key: "temperature",
        value: ControlValue::Float(10.0),
    })
    .unwrap();
    let trigger = PortRef::new(rf, "trigger");

    let mut downs = 0;
    let mut ups = 0;
    let mut open = false;
    for _ in 0..600 {
        app.tick(FRAME);
        for event in app.edges(trigger) {
            assert!(
                (0.0..=FRAME).contains(&event.at),
                "a stamp is an offset inside its own frame: {event:?}"
            );
            match event.edge {
                Edge::Down => {
                    assert!(!open, "a down while the gate is already open");
                    open = true;
                    downs += 1;
                }
                Edge::Up => {
                    assert!(open, "an up with nothing open");
                    open = false;
                    ups += 1;
                }
            }
        }
    }
    assert!(downs >= 5, "ten seconds at full temperature: {downs}");
    assert_eq!(ups, downs, "every gate it opened, it closed");
}

/// **The node says whether it is running**, in silvia's own two words.
///
/// The firing is not on the body and is not meant to be: an action throbs at its port, on
/// every node alike. What a throb cannot say is the state a press of Start/Stop left behind,
/// which is exactly what silvia's pill says — a flame while it runs and a snowflake while it
/// does not.
#[test]
fn a_random_fire_says_whether_it_is_running() {
    let mut app = App::headless();
    let rf = add_to(&mut app, "randomfire");
    app.tick(FRAME);
    assert_eq!(
        app.node_status(rf),
        Some("\u{1f525} Active"),
        "it lands running, as silvia's does"
    );

    app.press(PortRef::new(rf, "start"), true);
    app.tick(FRAME);
    app.press(PortRef::new(rf, "start"), false);
    app.tick(FRAME);
    assert_eq!(
        app.node_status(rf),
        Some("\u{2744} Stopped"),
        "and Start/Stop is a state, so the body carries it"
    );
}

/// Start/Stop stops the firing and closes whatever gate is open rather than leaving a
/// receiver holding one forever.
#[test]
fn a_random_fire_closes_its_gate_when_stopped() {
    let mut app = App::headless();
    let rf = add_to(&mut app, "randomfire");
    app.apply(Command::SetControl {
        node: rf,
        key: "temperature",
        value: ControlValue::Float(10.0),
    })
    .unwrap();
    let trigger = PortRef::new(rf, "trigger");

    // Run until the gate is confirmed open.
    let mut open = false;
    for _ in 0..120 {
        app.tick(FRAME);
        for event in app.edges(trigger) {
            open = event.is_down();
        }
        if open {
            break;
        }
    }
    assert!(open, "a down within two seconds at full temperature");

    app.press(PortRef::new(rf, "start"), true);
    app.tick(FRAME);
    assert!(
        app.edges(trigger).iter().any(|e| e.edge == Edge::Up),
        "stopping closes an open gate rather than abandoning it"
    );
    app.press(PortRef::new(rf, "start"), false);
    app.tick(FRAME);

    for _ in 0..60 {
        app.tick(FRAME);
        assert!(app.edges(trigger).is_empty(), "stopped fires nothing");
    }
}

// ---------------------------------------------------------------- euclideanrhythm

/// A Master Gear at its default two seconds — a bar at 120 BPM — cabled into
/// `node`'s Time, with the playhead put back at zero so every gear is born at the bar's
/// start: the clock a sequencer plays on.
fn driven(app: &mut App, node: NodeId) -> NodeId {
    let clock = add_to(app, "mastergear");
    app.apply(Command::Connect {
        from: PortRef::new(clock, "cycles"),
        to: PortRef::new(node, supersilvia::nodes::TIME),
    })
    .unwrap();
    app.transport(supersilvia::transport::Command::Seek(0.0));
    app.tick(FRAME);
    clock
}

/// Set lane 1 to every step a pulse, so the grid fires on every sixteenth note of the Master
/// Gear driving its Time — sixteen of them in two seconds at 120 bpm — and every gate it
/// opens, it closes within its own frame's stamp; holding the gear closes the one the window's
/// edge caught mid-hold.
#[test]
fn a_running_lane_fires_on_the_grid_and_closes_its_gate_within_it() {
    let (mut app, id) = app_with("euclideanrhythm");
    for (key, v) in [("lane1steps", 4.0), ("lane1pulses", 4.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let clock = driven(&mut app, id);
    let lane1 = PortRef::new(id, "lane1");
    let mut downs = 0;
    let mut ups = 0;
    let mut count = |app: &App| {
        for event in app.edges(lane1) {
            assert!(
                (0.0..=FRAME).contains(&event.at),
                "a stamp is an offset inside its own frame: {event:?}"
            );
            match event.edge {
                Edge::Down => downs += 1,
                Edge::Up => ups += 1,
            }
        }
    };
    for _ in 0..120 {
        app.tick(FRAME);
        count(&app);
    }
    // Holding the gear stands Time still, which closes whatever gate is open: what the
    // sequencer opened, it closed.
    app.press(PortRef::new(clock, "hold"), true);
    app.tick(FRAME);
    count(&app);
    app.tick(FRAME);
    count(&app);
    assert!(
        (15..=17).contains(&downs),
        "sixteen sixteenth-notes in two seconds at 120 bpm: {downs}"
    );
    assert_eq!(ups, downs, "every gate it opened, it closed");
}

/// A Euclidean pattern for (8, 3) is the textbook tresillo, `x..x..x.`, at rotation zero,
/// and the first manual step lands on step zero, silvia's `currentStep = -1` advanced once —
/// so over the pattern's eight steps the three pulses land on steps 0, 3 and 6, each its own
/// down-then-up since no two of them are adjacent.
#[test]
fn a_step_input_advances_one_step_per_down_and_holds_each_lane_while_its_a_pulse() {
    let (mut app, id) = app_with("euclideanrhythm");
    for (key, v) in [
        ("lane1steps", 8.0),
        ("lane1pulses", 3.0),
        ("lane1rotation", 0.0),
    ] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let lane1 = PortRef::new(id, "lane1");

    let mut downs = 0;
    let mut ups = 0;
    // The pattern's own eight: the seventh closes the gate the sixth opened.
    for _ in 0..8 {
        app.press(PortRef::new(id, "step"), true);
        app.tick(FRAME);
        for event in app.edges(lane1) {
            match event.edge {
                Edge::Down => downs += 1,
                Edge::Up => ups += 1,
            }
        }
        app.press(PortRef::new(id, "step"), false);
        app.tick(FRAME);
    }
    assert_eq!(
        downs, 3,
        "the tresillo's three pulses over the pattern's eight steps"
    );
    assert_eq!(
        ups, downs,
        "no two are adjacent, so each closes before the next opens"
    );
}

/// A gear's Reset moves the sequencer's Time back, which is a jump: it closes whatever gate
/// is open rather than abandoning a receiver mid-hold, the retrigger-proof ordering the gear's
/// own reset gives.
#[test]
fn a_reset_gear_closes_an_open_gate() {
    let (mut app, id) = app_with("euclideanrhythm");
    for (key, v) in [("lane1steps", 2.0), ("lane1pulses", 2.0)] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let clock = driven(&mut app, id);
    let lane1 = PortRef::new(id, "lane1");
    let mut open = false;
    for _ in 0..60 {
        app.tick(FRAME);
        for event in app.edges(lane1) {
            open = event.is_down();
        }
        if open {
            break;
        }
    }
    assert!(open, "a down within a second with every step a pulse");
    app.press(PortRef::new(clock, "reset"), true);
    app.tick(FRAME);
    assert!(
        app.edges(lane1).iter().any(|e| e.edge == Edge::Up),
        "the jump closes an open gate rather than abandoning it"
    );
}

// ---------------------------------------------------------------- stepsequencer

/// A step sequencer holding `lanes` as its pattern, one string of `x` and `.` a lane.
fn sequencer_with(lanes: &[&str]) -> (App, NodeId) {
    let (mut app, id) = app_with("stepsequencer");
    app.apply(Command::SetValue {
        node: id,
        key: "pattern",
        value: supersilvia::graph::Value::Cells(lanes.iter().map(ToString::to_string).collect()),
    })
    .expect("the pattern is the node's own value");
    (app, id)
}

/// Every edge on a lane over `frames` ticks, dated by the playhead: where the tick began,
/// and how far inside it the edge fell.
fn lane_edges(app: &mut App, id: NodeId, frames: usize) -> [Vec<(Edge, f32)>; 4] {
    let mut out: [Vec<(Edge, f32)>; 4] = Default::default();
    for _ in 0..frames {
        let began = app.transport_state().playhead as f32;
        app.tick(FRAME);
        for (lane, edges) in out.iter_mut().enumerate() {
            let port = PortRef::new(id, ["lane1", "lane2", "lane3", "lane4"][lane]);
            for event in app.edges(port) {
                edges.push((event.edge, began + event.at));
            }
        }
    }
    out
}

/// Driven by a Master Gear, the playhead walks the sixteen a bar: a lit cell opens its lane on
/// its own step and the gate closes it half a step later, and an unlit lane says nothing. The
/// first column is the first thing heard, on the tick after the bar starts.
#[test]
fn a_lit_cell_opens_its_lane_on_its_step_for_the_gate_length() {
    let (mut app, id) = sequencer_with(&["x...x...x...x...", "....x..........."]);
    driven(&mut app, id);
    // One bar at 120 bpm is sixteen sixteenths of 0.125 s: two seconds, 120 frames.
    let lanes = lane_edges(&mut app, id, 118);
    let step = 60.0 / 120.0 / 4.0;

    let downs = |lane: usize| -> Vec<f32> {
        lanes[lane]
            .iter()
            .filter(|(e, _)| *e == Edge::Down)
            .map(|&(_, t)| t)
            .collect()
    };
    // The first column lands with the tick after the bar begins, so within a frame of it.
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3 || (b == 0.0 && a <= FRAME + 1e-3);
    let one = downs(0);
    assert_eq!(one.len(), 4, "four on the floor: {one:?}");
    for (beat, &t) in one.iter().enumerate() {
        assert!(
            near(t, beat as f32 * 4.0 * step),
            "beat {beat} lands on step {}: {t}",
            beat * 4
        );
    }
    let two = downs(1);
    assert_eq!(two.len(), 1, "lane 2's one cell: {two:?}");
    assert!(near(two[0], 4.0 * step), "on step 4: {}", two[0]);
    assert!(
        lanes[2].is_empty() && lanes[3].is_empty(),
        "no cell, no gate"
    );

    // The gate is half a step, the knob's own default, from the second beat on.
    for pair in lanes[0].chunks(2).skip(1) {
        let [(Edge::Down, down), (Edge::Up, up)] = pair else {
            panic!("a down and then its up: {pair:?}");
        };
        assert!(
            near(up - down, 0.5 * step),
            "held {} of a step",
            (up - down) / step
        );
    }
}

/// The playhead is where the gear driving it has got to, and nothing lights before it
/// moves: at rest a sequencer is still; driven, it starts on the first column and has walked
/// seventeen steps after 2.125 s; a gear's Reset starts the bar again from the first column.
#[test]
fn the_step_sequencers_playhead_walks_from_the_first_column() {
    let (mut app, id) = sequencer_with(&[]);
    let playhead = |app: &App| app.snapshot().playheads.get(&id).copied();
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(playhead(&app), None, "at rest nothing steps");

    let clock = driven(&mut app, id);
    app.tick(FRAME);
    assert_eq!(
        playhead(&app),
        Some(0.0),
        "the bar begins on the first column"
    );
    for _ in 0..127 {
        app.tick(FRAME);
    }
    let at = playhead(&app).expect("running");
    assert_eq!(at as i64, 17, "a step each eighth of a second: {at}");
    assert_eq!((at as i64).rem_euclid(16), 1, "the bar wrapped");

    app.press(PortRef::new(clock, "reset"), true);
    app.tick(FRAME);
    app.press(PortRef::new(clock, "reset"), false);
    app.tick(FRAME);
    assert_eq!(
        playhead(&app),
        Some(0.0),
        "a gear's reset starts the bar again from the first column"
    );
}

/// Step walks the pattern one column a press, and the cell under it decides the lane: a
/// lit one opens its gate, an unlit one does not.
#[test]
fn a_step_press_plays_the_next_column() {
    let (mut app, id) = sequencer_with(&["x.x.", "", "", ".x"]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        app.press(PortRef::new(id, "step"), true);
        app.tick(FRAME);
        seen.push((
            kinds(&app, PortRef::new(id, "lane1")).contains(&Edge::Down),
            kinds(&app, PortRef::new(id, "lane4")).contains(&Edge::Down),
        ));
        app.press(PortRef::new(id, "step"), false);
        app.tick(FRAME);
    }
    assert_eq!(
        seen,
        [(true, false), (false, true), (true, false), (false, false)],
        "columns 0 to 3, lane 1 and lane 4"
    );
}

/// A one-second Master Gear, with its `output` cabled into `node`'s Time.
fn seconds_gear_into(app: &mut App, node: NodeId, output: &'static str) -> NodeId {
    let clock = add_to(app, "mastergear");
    app.apply(Command::SetControl {
        node: clock,
        key: "length",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(clock, output),
        to: PortRef::new(node, supersilvia::nodes::TIME),
    })
    .unwrap();
    clock
}

/// Did `port` open a gate on the frame just ticked?
fn opened(app: &App, port: PortRef) -> bool {
    app.edges(port).iter().any(|e| e.is_down())
}

/// **A gear's Phase in Time plays on the gear's downbeats.** Phase declares that it wraps at
/// one, and the sequencer reads Time unwrapped there, so a bar's wrap is a bar's motion and
/// not a jump: lane 1, lit on step 0, opens on the very frames the Master Gear's Trigger
/// fires, at a steady 60 frames a second and at frame times alternating 1/50 and 1/80 s.
#[test]
fn a_sequencer_on_a_gears_phase_plays_on_its_downbeats() {
    for (name, dts) in [
        ("steady", [FRAME, FRAME]),
        ("uneven", [1.0 / 50.0, 1.0 / 80.0]),
    ] {
        let (mut app, id) = sequencer_with(&["x..............."]);
        let clock = seconds_gear_into(&mut app, id, "wrapped");
        let (mut beats, mut steps) = (Vec::new(), Vec::new());
        for i in 0..300 {
            app.tick(dts[i % 2]);
            if i < 10 {
                continue;
            }
            if opened(&app, PortRef::new(clock, "trigger")) {
                beats.push(i);
            }
            if opened(&app, PortRef::new(id, "lane1")) {
                steps.push(i);
            }
        }
        assert!(beats.len() >= 4, "{name}: {beats:?}");
        assert_eq!(steps, beats, "{name}: step 0 on every beat of the gear");
    }
}

/// **A gear's Cycles in Time pass 1260 with no seam.** Their one `f32` steps from 1260 to −1260
/// there, and the sequencer reads the count whole, so a Euclidean lane of eleven steps, which
/// 16 × 2520 steps is no whole number of, keeps its pulse every 11 sixteenths of a second — 41
/// or 42 frames — across it, and a lane of every step opens on the frame the `f32` wraps.
#[test]
fn a_sequencer_on_a_gears_cycles_plays_through_the_counts_wrap() {
    let (mut app, id) = app_with("euclideanrhythm");
    for (key, v) in [
        ("lane1steps", 16.0),
        ("lane1pulses", 16.0),
        ("lane2steps", 11.0),
        ("lane2pulses", 1.0),
    ] {
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(v),
        })
        .unwrap();
    }
    let clock = seconds_gear_into(&mut app, id, "cycles");
    app.tick(FRAME);
    app.transport(supersilvia::transport::Command::Seek(1255.0));
    app.tick(FRAME);
    let mut pulses = Vec::new();
    let mut wrapped = false;
    for i in 0..600 {
        let before = app.uniform(PortRef::new(clock, "cycles")).unwrap();
        app.tick(FRAME);
        let after = app.uniform(PortRef::new(clock, "cycles")).unwrap();
        if before > 1259.0 && after < -1259.0 {
            wrapped = true;
            assert!(
                opened(&app, PortRef::new(id, "lane1")),
                "a step on the frame the count wraps"
            );
        }
        if opened(&app, PortRef::new(id, "lane2")) {
            pulses.push(i);
        }
    }
    assert!(wrapped, "the count did wrap");
    let gaps: Vec<usize> = pulses.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(pulses.len() > 10, "{pulses:?}");
    assert!(
        gaps.iter().all(|g| (41..=42).contains(g)),
        "eleven sixteenths between pulses, across the wrap too: {gaps:?}"
    );
}

// ---------------------------------------------------------------- a held level

/// The leftmost inked column of the row `brickgame`'s paddle sits on, of the 300 across its
/// field. The paddle has no port of its own, so this is what says where it went.
fn paddle_at(app: &App, game: NodeId) -> usize {
    let frame = app
        .frame(PortRef::new(game, "field"))
        .expect("the game published a field");
    let bytes = frame.bytes().expect("bytes, not a descriptor");
    let row = ((1.0 - -0.85) * 0.5 * 300.0) as usize;
    (0..300)
        .find(|x| bytes[(row * 300 + x) * 4] > 0)
        .expect("the paddle is drawn")
}

/// **A paddle is a level, not a press.** `brickgame`'s paddles are `Action` inputs, because
/// what a performer patches into them is a gamepad button or a sequencer lane — but what the
/// game needs is whether the button is *down right now*, which is the edges on the port
/// folded back into a level plus the hand on its own button as one more source.
#[test]
fn a_paddle_holds_while_a_cable_holds_it_down() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let game = add_to(&mut app, "brickgame");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(game, "leftPaddle"),
    })
    .unwrap();
    // Running, so the paddle is allowed to move at all.
    app.press(PortRef::new(game, "startGame"), true);
    app.tick(FRAME);
    app.press(PortRef::new(game, "startGame"), false);

    let start = paddle_at(&app, game);
    // One press, held: the cable says down once and never again, and the paddle keeps going.
    app.press(PortRef::new(button, "press"), true);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    let held = paddle_at(&app, game);
    assert!(
        held + 10 < start,
        "a held gate did not move the paddle: {start} to {held}"
    );

    // Let go: the `Up` arrives and the paddle stops where it was.
    app.press(PortRef::new(button, "press"), false);
    app.tick(FRAME);
    let released = paddle_at(&app, game);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(
        paddle_at(&app, game),
        released,
        "the paddle kept moving after the gate closed"
    );
}

// ---------------------------------------------------------------- a controller

/// A controller's button is a gate like any other, driven here through a fake event source
/// so that nothing has to be plugged into the machine.
///
/// `gamepad::bench` installs a pad every `Pad::open` takes in place of a device, which is
/// what keeps `scripts/doctor.sh` from having to ask for a controller. It is process-wide,
/// which is why everything asserted about a controller is asserted here: two tests each
/// installing one would each be driving the other's.
#[test]
fn a_gamepad_button_fires_down_and_up_from_the_device() {
    use supersilvia::gamepad;

    let pad = gamepad::bench();
    let (mut app, id) = app_with("gamepad");
    let button = PortRef::new(id, "buttonA");
    assert!(kinds(&app, button).is_empty(), "nothing pressed yet");

    pad.press(0, true);
    app.tick(FRAME);
    assert_eq!(kinds(&app, button), vec![Edge::Down]);

    // An edge carries where inside the frame it happened, which is what silvia's
    // animation-frame poll could not say.
    let events = app.edges(button);
    assert_eq!(events.len(), 1);
    assert!(
        (0.0..=FRAME).contains(&events[0].at),
        "stamped inside this frame: {}",
        events[0].at
    );

    // An edge lives for one frame: a held button is a gate that stays open, not a stream.
    app.tick(FRAME);
    assert!(kinds(&app, button).is_empty(), "a gate is not a pulse");

    pad.press(0, false);
    app.tick(FRAME);
    assert_eq!(kinds(&app, button), vec![Edge::Up]);

    // Each button is its own port, and the axes are the same read: up positive on a stick.
    pad.press(4, true);
    pad.set_axis(1, 0.5);
    app.tick(FRAME);
    assert_eq!(kinds(&app, PortRef::new(id, "dpadUp")), vec![Edge::Down]);
    assert!(kinds(&app, button).is_empty(), "one button, one port");
    assert_eq!(app.uniform(PortRef::new(id, "leftStickY")), Some(0.5));

    gamepad::clear_bench();
}

// ---------------------------------------------------------------- a performance, kept

/// The transport is four action inputs, so a cable drives it exactly as a hand does: a
/// sequencer can start the playback a knob was recorded into.
///
/// And Once stops at the end of the recording where Loop starts it again, which is the one
/// thing the menu decides.
#[test]
fn the_automations_transport_is_driven_by_cables_and_by_the_menu() {
    let mut app = App::headless();
    let button = add_to(&mut app, "button");
    let node = add_to(&mut app, "automation");
    app.apply(Command::Connect {
        from: PortRef::new(button, "trigger"),
        to: PortRef::new(node, "play"),
    })
    .unwrap();
    let set = |app: &mut App, key: &'static str, value: f32| {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    };
    set(&mut app, "duration", 0.5);

    // Record a ramp, by hand on the node's own Record button.
    app.press(PortRef::new(node, "record"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "record"), false);
    for i in 0..=30 {
        set(&mut app, "input", i as f32 / 30.0);
        app.tick(FRAME);
    }
    set(&mut app, "input", 0.0);
    app.tick(FRAME);

    let out = PortRef::new(node, "output");
    assert_eq!(
        app.uniform(out),
        Some(0.0),
        "stopped, the knob is the value"
    );

    // The cable, not the hand: one press of the button upstream starts it.
    app.press(PortRef::new(button, "press"), true);
    app.tick(FRAME);
    app.press(PortRef::new(button, "press"), false);
    for _ in 0..10 {
        app.tick(FRAME);
    }
    let playing = app.uniform(out).expect("published");
    assert!(playing > 0.0, "an edge on the port started it: {playing}");

    // Once: past the end of the recording it stops, and the knob is the value again.
    app.apply(Command::SetOption {
        node,
        key: "loop",
        value: "once".to_string(),
    })
    .unwrap();
    for _ in 0..40 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(out),
        Some(0.0),
        "Once ends, and what is left is the knob"
    );

    // Loop: Restart sets it going again and it is still going a recording later.
    app.apply(Command::SetOption {
        node,
        key: "loop",
        value: "loop".to_string(),
    })
    .unwrap();
    app.press(PortRef::new(node, "restart"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "restart"), false);
    for _ in 0..50 {
        app.tick(FRAME);
    }
    assert!(
        app.uniform(out).expect("published") > 0.0,
        "Loop is still playing a recording later"
    );
}

// ---------------------------------------------------------------- a sequenced grid

/// The grid on the node lights nothing until the sequencer has stepped, then follows it.
///
/// silvia's `currentStep = -1` is what says *nothing yet*, and the cell it lights is the one
/// thing a hand reads while it is playing. It is a reading, not a control: `CpuNode::playhead`
/// out, and nothing back.
#[test]
fn the_grids_playhead_is_where_the_sequencer_has_got_to() {
    let (mut app, id) = app_with("euclideanrhythm");
    let playhead = |app: &App| app.snapshot().playheads.get(&id).copied();
    assert_eq!(playhead(&app), None, "nothing has stepped yet");

    app.press(PortRef::new(id, "step"), true);
    app.tick(FRAME);
    app.press(PortRef::new(id, "step"), false);
    app.tick(FRAME);
    assert_eq!(playhead(&app), Some(0.0), "one step in is the first column");
}

/// **A Time cable moved onto another clock lands on it.** A Step Sequencer with every step lit,
/// on a one-second Master Gear's Cycles, has its Time taken over by a 0.6-second one's in a
/// single edit, two thirds of a bar ahead: the move plays none of the ten steps between the
/// two readings in one frame, as it would were the move a motion, and the steps after it play
/// one at a time on the new clock.
#[test]
fn a_time_moved_onto_another_clock_lands_on_it() {
    let (mut app, id) = sequencer_with(&["xxxxxxxxxxxxxxxx"]);
    let slow = driven(&mut app, id);
    let fast = add_to(&mut app, "mastergear");
    for (node, length) in [(slow, 1.0), (fast, 0.6)] {
        app.apply(Command::SetControl {
            node,
            key: "length",
            value: ControlValue::Float(length),
        })
        .unwrap();
    }
    app.transport(supersilvia::transport::Command::Seek(0.0));
    for _ in 0..59 {
        app.tick(FRAME);
    }
    app.apply(Command::Connect {
        from: PortRef::new(fast, "cycles"),
        to: PortRef::new(id, supersilvia::nodes::TIME),
    })
    .unwrap();
    let lane1 = PortRef::new(id, "lane1");
    let downs = |app: &App| app.edges(lane1).iter().filter(|e| e.is_down()).count();
    app.tick(FRAME);
    assert!(
        downs(&app) <= 1,
        "the move is a landing, not ten steps in one frame: {}",
        downs(&app)
    );
    for _ in 0..60 {
        app.tick(FRAME);
        assert!(downs(&app) <= 1, "a step at a time after it");
    }
}
