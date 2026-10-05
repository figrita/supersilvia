// SPDX-License-Identifier: AGPL-3.0-or-later

//! The uniform number port type: one `f32` per frame on the CPU, and the tick that
//! produces it.
//!
//! What is asserted here is the asymmetry the design rests on. A uniform number feeds a
//! varying number input for free and becomes a bare uniform; a varying number cannot
//! feed a uniform number, because collapsing a field to one needs a render pass and is a
//! node.

use emath::Pos2;
use supersilvia::compile::{self, UniformProvider, UniformType};
use supersilvia::graph::{ConnectError, ControlValue, Graph, NodeId, PortRef, PortType};
use supersilvia::nodes;
use supersilvia::{App, Command};

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is in the registry")
}

// ---------------------------------------------------------------- connection rules

#[test]
fn a_uniform_feeds_a_fragment_input_for_free() {
    let mut g = Graph::new();
    let slew = add(&mut g, "slew");
    let zoom = add(&mut g, "zoom");
    g.connect(PortRef::new(slew, "output"), PortRef::new(zoom, "zoom"))
        .expect("a uniform number into a varying number is the free direction");
}

#[test]
fn a_fragment_cannot_feed_a_uniform_input() {
    let mut g = Graph::new();
    let field = add(&mut g, "luminosity");
    let slew = add(&mut g, "slew");
    let err = g
        .connect(PortRef::new(field, "output"), PortRef::new(slew, "input"))
        .unwrap_err();
    assert_eq!(
        err,
        ConnectError::TypeMismatch {
            from: PortType::VaryingNumber,
            to: PortType::UniformNumber
        },
        "a field into a uniform number costs a readback, and is a node, not a cable"
    );
}

#[test]
fn a_uniform_number_feeds_a_number_and_never_a_color() {
    let mut g = Graph::new();
    let a = add(&mut g, "slew");
    let b = add(&mut g, "slew");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();
    assert!(matches!(
        g.connect(PortRef::new(a, "output"), PortRef::new(out, "input")),
        Err(ConnectError::TypeMismatch { .. })
    ));
}

#[test]
fn uniform_edges_are_data_edges_and_cannot_cycle() {
    let mut g = Graph::new();
    let a = add(&mut g, "slew");
    let b = add(&mut g, "slew");
    g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();
    assert!(matches!(
        g.connect(PortRef::new(b, "output"), PortRef::new(a, "input")),
        Err(ConnectError::WouldCycle { .. })
    ));
}

// ---------------------------------------------------------------- delayed uniforms

/// A tap's uniforms are last frame's by construction, so a cable out of one to a node
/// upstream of the tap is feedback rather than an immediate cycle: it is accepted, the
/// shader it closes a loop through still compiles clean, and every node — including the two
/// the loop runs through — still ticks, in a total order.
#[test]
fn a_taps_mean_can_be_cabled_upstream_of_the_tap() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    let tap = add(&mut g, "tap");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(tap, "input"))
        .unwrap();
    g.connect(PortRef::new(tap, "output"), PortRef::new(out, "input"))
        .unwrap();
    // The zoom feeds the tap; this closes the loop the other way. Anywhere else a uniform
    // number does this it is `WouldCycle` — here it is accepted, because `tap.mean` is
    // delayed.
    g.connect(PortRef::new(tap, "mean"), PortRef::new(zoom, "zoom"))
        .expect("a tap's uniform number is delayed, so this is feedback, not a cycle");

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);

    let order = g.tick_order().to_vec();
    assert_eq!(order.len(), 4, "every node ticks, including the loop's two");
    let mut sorted = order;
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 4, "each exactly once: a total order");
}

/// `autoexposure.gain` is delayed the same way `tap.mean` is, so the loop it used to have to
/// close through `own_uniform` can be closed with a cable instead.
#[test]
fn autoexposures_gain_can_be_cabled_upstream_of_it() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    let ae = add(&mut g, "autoexposure");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(ae, "input"))
        .unwrap();
    g.connect(PortRef::new(ae, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.connect(PortRef::new(ae, "gain"), PortRef::new(zoom, "zoom"))
        .expect("autoexposure's gain is delayed too");

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);

    let order = g.tick_order().to_vec();
    let mut sorted = order.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), order.len(), "a total order despite the loop");
}

/// The exemption is per-port, not per-graph: a tap's delayed loop sits beside an ordinary
/// pair of uniforms in the same graph, and the ordinary pair — no delayed port between them —
/// is still an immediate cycle and is still refused.
#[test]
fn an_immediate_uniform_cycle_is_still_refused_beside_a_delayed_one() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    let tap = add(&mut g, "tap");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(tap, "input"))
        .unwrap();
    g.connect(PortRef::new(tap, "mean"), PortRef::new(zoom, "zoom"))
        .expect("the delayed loop from the test above");

    let a = add(&mut g, "slew");
    let b = add(&mut g, "slew");
    g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();
    assert_eq!(
        g.connect(PortRef::new(b, "output"), PortRef::new(a, "input")),
        Err(ConnectError::WouldCycle { from: b, to: a }),
    );
}

// ---------------------------------------------------------------- the compiler

/// A connected uniform number takes the same branch an unconnected number control does: a
/// bare uniform, and no function at all.
#[test]
fn a_connected_uniform_number_compiles_to_a_bare_uniform() {
    let mut g = Graph::new();
    let slew = add(&mut g, "slew");
    let zoom = add(&mut g, "zoom");
    let cb = add(&mut g, "checkerboard");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(slew, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let name = format!("u_float_slew{slew}_output");
    assert_eq!(
        shader.uniforms.get(name.as_str()),
        Some(&UniformProvider::NodeUniform {
            node: slew,
            port: "output",
            ty: UniformType::Float,
        }),
    );
    assert!(
        shader.body.contains(&format!("    {name}: f32,")),
        "declared as an f32 member of the uniform struct"
    );
    assert!(
        !shader.body.contains(&format!("slew{slew}_output(")),
        "a uniform number is never a function"
    );
    // And it is not also a control uniform for the input it replaced.
    assert!(
        !shader
            .uniforms
            .contains_key(format!("u_control_zoom{zoom}_zoom").as_str())
    );
    insta::assert_snapshot!(shader.body);
}

// ---------------------------------------------------------------- the tick

fn app_with_slew() -> (App, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "slew",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let slew = app.graph().iter().next().unwrap().0;
    (app, slew)
}

#[test]
fn a_cpu_node_publishes_on_tick_and_follows_its_control() {
    let (mut app, slew) = app_with_slew();
    let out = PortRef::new(slew, "output");
    assert_eq!(app.uniform(out), None, "nothing until the first tick");

    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(5.0),
    })
    .unwrap();
    // The first tick snaps to the input rather than ramping from zero.
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(out), Some(5.0));

    // Rise is 10 per second by default, so one 60 Hz frame moves a sixth of a unit.
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(50.0),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    let v = app.uniform(out).unwrap();
    assert!((v - (5.0 + 10.0 / 60.0)).abs() < 1e-4, "slewing: {v}");
}

/// The second shape: silvia's own exponential approach, which slows as it arrives where the
/// rate limit runs at one speed and stops with a corner.
#[test]
fn an_eased_slew_slows_as_it_arrives_and_a_rate_limited_one_does_not() {
    let steps = |shape: &'static str| {
        let (mut app, slew) = app_with_slew();
        let out = PortRef::new(slew, "output");
        app.apply(Command::SetOption {
            node: slew,
            key: "shape",
            value: shape.to_string(),
        })
        .unwrap();
        // Slow enough that ten frames is well short of arriving, so the rate limit is still
        // running at its own speed on the last of them.
        app.apply(Command::SetControl {
            node: slew,
            key: "rise",
            value: ControlValue::Float(2.0),
        })
        .unwrap();
        app.tick(1.0 / 60.0);
        app.apply(Command::SetControl {
            node: slew,
            key: "input",
            value: ControlValue::Float(1.0),
        })
        .unwrap();
        let mut history = vec![];
        for _ in 0..10 {
            app.tick(1.0 / 60.0);
            history.push(app.uniform(out).unwrap());
        }
        history
    };

    let rate = steps("rate");
    // Ten frames at 10 a second is a sixth of the way there, every frame the same size.
    let first = rate[0];
    assert!(
        rate.windows(2)
            .all(|w| ((w[1] - w[0]) - first).abs() < 1e-5),
        "a rate limit runs at one speed: {rate:?}"
    );

    let ease = steps("ease");
    let deltas: Vec<f32> = std::iter::once(ease[0])
        .chain(ease.windows(2).map(|w| w[1] - w[0]))
        .collect();
    assert!(
        deltas.windows(2).all(|w| w[1] < w[0]),
        "an ease slows every frame as it closes the gap: {deltas:?}"
    );
    assert!(
        ease[0] < rate[0] && ease[0] > rate[0] * 0.9,
        "it leaves at the rate limit's own slope and is already slowing inside the first \
         frame: {} against {}",
        ease[0],
        rate[0]
    );
    assert!(
        ease.last().unwrap() < &1.0,
        "without ever arriving: {:?}",
        ease.last()
    );
}

#[test]
fn producers_tick_before_consumers_within_one_frame() {
    let mut app = App::headless();
    for _ in 0..2 {
        app.apply(Command::AddNode {
            slug: "slew",
            at: Pos2::ZERO,
            workspace: app.graph().default_workspace(),
        })
        .unwrap();
    }
    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    // Wire the *later* id into the *earlier* one, so id order and dependency order differ.
    let (consumer, producer) = (ids[0], ids[1]);
    app.apply(Command::Connect {
        from: PortRef::new(producer, "output"),
        to: PortRef::new(consumer, "input"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: producer,
        key: "input",
        value: ControlValue::Float(7.0),
    })
    .unwrap();

    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(PortRef::new(consumer, "output")),
        Some(7.0),
        "the consumer saw this frame's value, not last frame's"
    );
}

#[test]
fn a_published_number_reaches_its_uniform_and_dies_with_its_node() {
    let (mut app, slew) = app_with_slew();
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(3.0),
    })
    .unwrap();
    app.tick(0.0);
    assert_eq!(app.uniform(PortRef::new(slew, "output")), Some(3.0));

    app.apply(Command::RemoveNodes(vec![slew])).unwrap();
    app.tick(0.0);
    assert_eq!(
        app.uniform(PortRef::new(slew, "output")),
        None,
        "a removed node's value must not keep feeding a uniform"
    );
}

#[test]
fn undo_across_a_cpu_node_keeps_it_ticking() {
    let (mut app, slew) = app_with_slew();
    app.tick(0.0);
    assert!(app.uniform(PortRef::new(slew, "output")).is_some());
    app.apply(Command::RemoveNodes(vec![slew])).unwrap();
    app.tick(0.0);
    assert!(app.undo());
    app.tick(0.0);
    assert!(
        app.uniform(PortRef::new(slew, "output")).is_some(),
        "restored under the same id, it publishes again"
    );
}

#[test]
fn a_uniform_control_is_clamped_like_any_other() {
    let (mut app, slew) = app_with_slew();
    app.apply(Command::SetControl {
        node: slew,
        key: "rise",
        value: ControlValue::Float(1e9),
    })
    .unwrap();
    let ControlValue::Float(v) = app.graph().get(slew).unwrap().controls["rise"] else {
        panic!("rise is a number");
    };
    assert!(v <= 1000.0);
}

// ---------------------------------------------------------------- autogain

/// A quiet input and a loud one both normalize to a signal that touches 1 at their peak.
#[test]
fn autogain_normalizes_whatever_range_it_is_given() {
    for scale in [0.1f32, 10.0] {
        let mut app = App::headless();
        app.apply(Command::AddNode {
            slug: "autogain",
            at: Pos2::ZERO,
            workspace: app.graph().default_workspace(),
        })
        .unwrap();
        let ag = app.graph().iter().next().unwrap().0;
        let out = PortRef::new(ag, "output");
        let mut peak = 0.0f32;
        // A slow sine, a few periods, at this scale.
        for i in 0..600 {
            let x = scale * (0.5 + 0.5 * (i as f32 * 0.05).sin());
            app.apply(Command::SetControl {
                node: ag,
                key: "input",
                value: ControlValue::Float(x),
            })
            .unwrap();
            app.tick(1.0 / 60.0);
            if i > 300 {
                peak = peak.max(app.uniform(out).unwrap());
            }
        }
        assert!(peak > 0.9, "scale {scale}: peak reads {peak}");
        let v = app.uniform(out).unwrap();
        assert!((0.0..=1.0).contains(&v));
    }
}

// ---------------------------------------------------------------- own uniforms

/// A node's WGSL may read a uniform number its own tick published, as the same uniform a
/// connected one would be. That is how autoexposure closes its loop without a cable.
#[test]
fn a_node_reads_its_own_published_number_as_a_uniform() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let ae = add(&mut g, "autoexposure");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(ae, "input"))
        .unwrap();
    g.connect(PortRef::new(ae, "output"), PortRef::new(out, "input"))
        .unwrap();
    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let name = format!("u_float_autoexposure{ae}_gain");
    assert_eq!(
        shader.uniforms.get(name.as_str()),
        Some(&UniformProvider::NodeUniform {
            node: ae,
            port: "gain",
            ty: UniformType::Float,
        }),
    );
    assert!(shader.taps.is_empty(), "the picture measures nothing");
    let pass = compile::wgsl::build_pass(&g, g.default_workspace(), &[ae], false)
        .pop()
        .expect("its workspace's pass measures it");
    assert_eq!(pass.taps.len(), 1, "through one slot");
    assert_eq!(pass.taps[0].0, ae);
}

/// In a headless app there is no readback, so the gain holds at 1 rather than chasing a
/// zero reading up to the maximum.
#[test]
fn autoexposure_holds_its_gain_until_it_has_measured_something() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "autoexposure",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let ae = app.graph().iter().next().unwrap().0;
    for _ in 0..10 {
        app.tick(1.0 / 60.0);
    }
    assert_eq!(app.uniform(PortRef::new(ae, "gain")), Some(1.0));
}

// ---------------------------------------------------------------- measurements with no reading

/// A tap nothing measures publishes nothing, and says so.
///
/// A headless app has no renderer, so there is no readback and no pass ran the tap's
/// measurement — the shape of a tap on a closed workspace nothing awake reads. Its five ports
/// are withdrawn, so their rows draw nothing: a reading of nothing is not a reading of zero.
#[test]
fn a_tap_with_no_readback_publishes_nothing_and_says_it_is_not_measuring() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "tap",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let tap = app.graph().iter().next().unwrap().0;
    for _ in 0..3 {
        app.tick(1.0 / 60.0);
    }
    for port in ["mean", "max", "min", "x", "y"] {
        assert_eq!(
            app.uniform(PortRef::new(tap, port)),
            None,
            "{port} has nothing to say",
        );
    }
    let report = app.cpu_report(tap);
    assert!(
        report.contains("not measuring"),
        "the node says which Output measures it, or that none does: {report}",
    );
}

/// A tap publishes a mean color beside its numbers, and withdraws it with them.
#[test]
fn a_tap_with_no_readback_publishes_no_color_either() {
    let mut app = App::headless();
    let tap = added(&mut app, "tap");
    for _ in 0..3 {
        app.tick(1.0 / 60.0);
    }
    assert_eq!(app.uniform_color(PortRef::new(tap, "color")), None);
    assert!(app.cpu_report(tap).contains("not measuring"));
}

/// A triggered color's `color` is a uniform color now, not a field assembled in WGSL — so it
/// still lands on a color input, and it carries the four channels it publishes beside it.
#[test]
fn a_triggered_colors_color_is_a_uniform_that_still_lands_on_a_picture() {
    let mut app = App::headless();
    let tc = added(&mut app, "triggeredcolor");
    let out = added(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(tc, "color"),
        to: PortRef::new(out, "input"),
    })
    .expect("a uniform color into a varying color input");

    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform_color(PortRef::new(tc, "color")),
        Some([1.0, 0.0, 1.0, 1.0]),
        "silvia's power-on magenta, published as one color"
    );
    let channel = |key: &'static str| app.uniform(PortRef::new(tc, key)).unwrap();
    assert_eq!(
        [channel("r"), channel("g"), channel("b"), channel("a")],
        [1.0, 0.0, 1.0, 1.0],
        "and the channels say the same thing"
    );
}

/// And a sample the same, port for port.
#[test]
fn a_sample_with_no_readback_publishes_nothing() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "sample",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let sample = app.graph().iter().next().unwrap().0;
    for _ in 0..3 {
        app.tick(1.0 / 60.0);
    }
    for port in ["r", "g", "b", "a", "luma"] {
        assert_eq!(app.uniform(PortRef::new(sample, port)), None, "{port}");
    }
    for port in ["hue", "saturation", "lightness"] {
        assert_eq!(app.uniform(PortRef::new(sample, port)), None, "{port}");
    }
    // The color goes with them: a color of nothing is not transparent black.
    assert_eq!(app.uniform_color(PortRef::new(sample, "color")), None);
    assert!(app.cpu_report(sample).contains("not measuring"));
}

/// A sample's `color` is the reading itself, and its channels are that same reading taken
/// apart — so a cable from `color` lands on a color input and one from `r` does not.
#[test]
fn a_samples_color_carries_what_its_channels_take_apart() {
    let mut g = Graph::new();
    let sample = add(&mut g, "sample");
    let checker = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");

    g.connect(
        PortRef::new(sample, "color"),
        PortRef::new(checker, "color1"),
    )
    .expect("a sampled color is a uniform color, and feeds a varying color");
    g.connect(PortRef::new(sample, "r"), PortRef::new(zoom, "zoom"))
        .expect("a channel is still a number");
    assert!(
        matches!(
            g.connect(PortRef::new(sample, "r"), PortRef::new(checker, "color2")),
            Err(ConnectError::TypeMismatch { .. })
        ),
        "a channel is not a color"
    );

    let ty = |key: &str| {
        g.get(sample)
            .and_then(|n| n.output(key))
            .map(|p| p.ty)
            .unwrap()
    };
    assert_eq!(ty("color"), PortType::UniformColor);
    assert_eq!(ty("luma"), PortType::UniformNumber);
}

// ------------------------------------------------ measurements are their workspace's pass's

/// Add a node through the bus on the first workspace, and hand back its id.
fn add_to(app: &mut App, slug: &'static str) -> NodeId {
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

fn cable(app: &mut App, from: PortRef, to: PortRef) {
    // A clock cabled into a Time: the node loops on it rather than running free.
    if supersilvia::nodes::is_time(to.key)
        && app
            .graph()
            .get(to.node)
            .is_some_and(|n| n.def.timing.is_some())
    {
        app.apply(Command::SetOption {
            node: to.node,
            key: "clockMode",
            value: "loop".to_string(),
        })
        .unwrap();
    }
    app.apply(Command::Connect { from, to }).expect("legal");
}

/// A tap is measured by its workspace's pass whatever its chain reaches: cabled into
/// nothing, beside an Output drawing something else, or into that Output.
#[test]
fn a_tap_is_measured_by_its_workspaces_pass_whatever_it_reaches() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let cb = add_to(&mut app, "checkerboard");
    let out = add_to(&mut app, "output");
    let gradient = add_to(&mut app, "radialgradient");
    let tap = add_to(&mut app, "tap");
    cable(
        &mut app,
        PortRef::new(cb, "output"),
        PortRef::new(out, "input"),
    );
    cable(
        &mut app,
        PortRef::new(gradient, "output"),
        PortRef::new(tap, "input"),
    );
    app.publish_plan();
    assert_eq!(app.pass_measures(ws), [tap], "a tap on nothing");

    cable(
        &mut app,
        PortRef::new(tap, "output"),
        PortRef::new(out, "input"),
    );
    app.publish_plan();
    assert_eq!(app.pass_measures(ws), [tap], "and the same tap drawn");
}

/// A workspace with no Output at all still measures: the pass is nobody's picture.
#[test]
fn a_workspace_with_no_output_still_measures() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let gradient = add_to(&mut app, "radialgradient");
    let tap = add_to(&mut app, "tap");
    cable(
        &mut app,
        PortRef::new(gradient, "output"),
        PortRef::new(tap, "input"),
    );
    app.publish_plan();
    assert_eq!(app.pass_measures(ws), [tap]);
    app.tick(FRAME);
    assert!(
        app.cpu_report(tap).contains("not measuring"),
        "headless, nothing reads a slot back, so the node says so",
    );
}

/// Which Output is on air moves nothing: the measurement is the workspace's, and a cut is
/// playing.
#[test]
fn a_cut_moves_no_measurement() {
    let mut app = App::headless();
    let ws = app.graph().default_workspace();
    let cb = add_to(&mut app, "checkerboard");
    let first = add_to(&mut app, "output");
    let second = add_to(&mut app, "output");
    let tap = add_to(&mut app, "tap");
    for out in [first, second] {
        cable(
            &mut app,
            PortRef::new(cb, "output"),
            PortRef::new(out, "input"),
        );
    }
    app.publish_plan();
    assert_eq!(app.pass_measures(ws), [tap]);

    app.press(PortRef::new(second, "show_a"), true);
    app.tick(FRAME);
    app.build_frame_job();
    assert_eq!(app.pass_measures(ws), [tap], "the cut moves nothing");
}

// ---------------------------------------------------------------- phase

const FRAME: f32 = 1.0 / 60.0;

fn app_with_phase() -> (App, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "ratiogear",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let phase = app.graph().iter().next().unwrap().0;
    (app, phase)
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap();
}

#[test]
fn phase_accumulates_dt_times_rate() {
    let (mut app, phase) = app_with_phase();
    let out = PortRef::new(phase, "cycles");
    set(&mut app, phase, "ratio", 2.0);
    for _ in 0..60 {
        app.tick(FRAME);
    }
    let v = app.uniform(out).unwrap();
    assert!(
        (v - 2.0).abs() < 1e-3,
        "two cycles per second for a second: {v}"
    );
}

#[test]
fn a_reset_restarts_the_phase() {
    let (mut app, phase) = app_with_phase();
    let out = PortRef::new(phase, "cycles");
    set(&mut app, phase, "ratio", 1.0);
    for _ in 0..60 {
        app.tick(FRAME);
    }
    assert!(app.uniform(out).unwrap() > 0.9);

    app.press(PortRef::new(phase, "reset"), true);
    app.tick(FRAME);
    let v = app.uniform(out).unwrap();
    assert!(
        (v - FRAME).abs() < 1e-4,
        "restarted at the press, leaving this frame's advance: {v}"
    );

    // Held is not a retrigger: the phase runs on from where the reset left it.
    app.tick(FRAME);
    let v = app.uniform(out).unwrap();
    assert!((v - 2.0 * FRAME).abs() < 1e-4, "running again: {v}");
}

/// Hold is a toggle: the gear stops where it is and picks up from there on the next press,
/// rather than winding the ratio down to zero and losing it.
#[test]
fn a_hold_freezes_the_phase_and_lets_it_go_from_where_it_stopped() {
    let (mut app, phase) = app_with_phase();
    let out = PortRef::new(phase, "cycles");
    set(&mut app, phase, "ratio", 1.0);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    let stopped = app.uniform(out).unwrap();
    // The press is a moment at the top of its frame, so the frame it lands on adds nothing.

    tap_button(&mut app, phase, "hold");
    for _ in 0..59 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(out),
        Some(stopped),
        "a whole second of frames adds nothing while it is held"
    );

    tap_button(&mut app, phase, "hold");
    for _ in 0..29 {
        app.tick(FRAME);
    }
    let after = app.uniform(out).unwrap();
    assert!(
        (after - (stopped + 30.0 * FRAME)).abs() < 1e-3,
        "picked up from where it stopped rather than from where it would have been: {after}"
    );
}

/// A hold and a reset in the same frame are two moments, and which came first is the answer.
#[test]
fn a_reset_inside_a_hold_leaves_the_phase_at_zero() {
    let (mut app, phase) = app_with_phase();
    let out = PortRef::new(phase, "cycles");
    set(&mut app, phase, "ratio", 1.0);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    app.press(PortRef::new(phase, "hold"), true);
    app.tick(FRAME);
    app.press(PortRef::new(phase, "reset"), true);
    app.tick(FRAME);
    app.press(PortRef::new(phase, "reset"), false);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(out),
        Some(0.0),
        "reset while held, and held is still held"
    );
}

// ---------------------------------------------------------------- time on the CPU

/// Add a node, hold the button on one of its action inputs for one frame, and let go.
///
/// The hand is a level the canvas reports and `tick` turns into an edge, so a press has to
/// span a tick to be seen at all — and has to be released before it can be seen again.
fn tap_button(app: &mut App, node: NodeId, key: &'static str) {
    app.press(PortRef::new(node, key), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, key), false);
}

fn option(app: &mut App, node: NodeId, key: &'static str, value: &str) {
    app.apply(Command::SetOption {
        node,
        key,
        value: value.to_string(),
    })
    .expect("the option is the node's and the value is one of its choices");
}

/// `wrapped` is the cycle's own fraction and never leaves 0..1 — including under a negative
/// ratio, where a signed `fract` would fold it onto the wrong side of the wrap. A gear at −×1
/// from its birth is born at minus the playhead and counts down from zero, through three
/// whole cycles below it.
#[test]
fn phases_wrapped_output_stays_inside_one_cycle() {
    for rate in [1.0, -1.0] {
        let (mut app, phase) = app_with_phase();
        let (cycles, wrapped) = (
            PortRef::new(phase, "cycles"),
            PortRef::new(phase, "wrapped"),
        );
        set(&mut app, phase, "ratio", rate);
        let mut lowest = f32::MAX;
        for _ in 0..180 {
            app.tick(FRAME);
            let (c, v) = (app.uniform(cycles).unwrap(), app.uniform(wrapped).unwrap());
            lowest = lowest.min(c);
            assert!((0.0..1.0).contains(&v), "at rate {rate}: {v}");
            let fraction = c.rem_euclid(1.0);
            assert!(
                (v - fraction).abs() < 1e-4 || (v - fraction).abs() > 1.0 - 1e-4,
                "at rate {rate}, {c} cycles is a Phase of {fraction}, not {v}"
            );
        }
        if rate < 0.0 {
            assert!(lowest < -2.9, "the count went below zero: {lowest}");
        }
    }
}

/// `pingpong` is a triangle over two cycles: out at one, home at two, and the cycles beside
/// it still count without bound. Three outputs, no mode — the way `counter` publishes both
/// its readings.
#[test]
fn phases_pingpong_is_out_at_one_cycle_and_home_at_two() {
    let (mut app, phase) = app_with_phase();
    let (cycles, pingpong) = (
        PortRef::new(phase, "cycles"),
        PortRef::new(phase, "pingpong"),
    );
    set(&mut app, phase, "ratio", 1.0);

    for _ in 0..60 {
        app.tick(FRAME);
    }
    assert!((app.uniform(cycles).unwrap() - 1.0).abs() < 1e-3);
    let out = app.uniform(pingpong).unwrap();
    assert!((out - 1.0).abs() < 1e-3, "one cycle is the far end: {out}");

    let mut lowest = out;
    for _ in 0..60 {
        app.tick(FRAME);
        lowest = lowest.min(app.uniform(pingpong).unwrap());
    }
    assert!((app.uniform(cycles).unwrap() - 2.0).abs() < 1e-3);
    let home = app.uniform(pingpong).unwrap();
    assert!(
        (home - 0.0).abs() < 1e-3,
        "two cycles is home again: {home}"
    );
    assert!(
        lowest >= -1e-6,
        "and it never went below the near end: {lowest}"
    );
}

/// A sine at 1 Hz crosses zero twice a cycle, at the half second and the second, and the
/// frame it happens on is the frame the phase reached it.
#[test]
fn an_oscillator_crosses_zero_twice_a_cycle() {
    let mut app = App::headless();
    let osc = add_to(&mut app, "oscillator");
    let out = PortRef::new(osc, "output");

    let mut crossings = Vec::new();
    let mut previous: Option<f32> = None;
    for frame in 1..=90 {
        app.tick(FRAME);
        let v = app.uniform(out).unwrap();
        if let Some(p) = previous
            && p * v <= 0.0
        {
            crossings.push(frame as f32 * FRAME);
        }
        previous = Some(v);
    }

    // A crossing is noticed on the frame the sign changed on, and the time recorded is that
    // frame's end, so the moment itself is inside the frame before it.
    assert!(
        crossings.len() >= 2,
        "two crossings in 1.5 s: {crossings:?}"
    );
    assert!(
        (crossings[0] - 0.5).abs() <= FRAME * 1.001,
        "the falling crossing: {crossings:?}"
    );
    assert!(
        (crossings[1] - 1.0).abs() <= FRAME * 1.001,
        "and the rising one: {crossings:?}"
    );
}

/// The trace on an oscillator's body is dated by the one clock: each tick records what it
/// published at the clock's own `elapsed`, so a long frame is a wide gap on the band and
/// not a kink. A late frame here is one tick three times the length of the others.
#[test]
fn the_trace_is_dated_by_the_clock_not_counted_in_frames() {
    let mut app = App::headless();
    let osc = add_to(&mut app, "oscillator");
    let out = PortRef::new(osc, "output");

    // `App::tick` moves the one clock by the `dt` it is handed, and a tick's date is the
    // clock's `elapsed`.
    let step = |app: &mut App, dt: f32| app.tick(dt);
    for _ in 0..10 {
        step(&mut app, FRAME);
    }
    step(&mut app, 3.0 * FRAME);
    step(&mut app, FRAME);

    let ring = app.trace(osc).expect("an oscillator keeps a trace");
    let newest = ring.newest().unwrap();
    assert!(
        (newest.at - app.clock().elapsed()).abs() < 1e-6,
        "the newest sample is now: {} against {}",
        newest.at,
        app.clock().elapsed()
    );
    assert_eq!(
        newest.value,
        app.uniform(out).unwrap(),
        "and is what was published"
    );

    let at: Vec<f64> = ring.iter().map(|s| s.at).collect();
    let gaps: Vec<f64> = at.windows(2).map(|w| w[1] - w[0]).collect();
    let long = gaps[gaps.len() - 2];
    let short = gaps[gaps.len() - 1];
    assert!(
        (long - 3.0 * f64::from(FRAME)).abs() < 1e-6 && (short - f64::from(FRAME)).abs() < 1e-6,
        "the late frame is three frames wide on the band: {gaps:?}"
    );
}

/// A frequency turned on the gear driving an oscillator lands on a whole wave, so the wave
/// never jumps: the value moves no faster than the new frequency's own slope, before, across
/// and after the change.
#[test]
fn a_ratio_turned_on_its_gear_leaves_the_oscillator_continuous() {
    let mut app = App::headless();
    let osc = add_to(&mut app, "oscillator");
    let gear = add_to(&mut app, "ratiogear");
    cable(
        &mut app,
        PortRef::new(gear, "cycles"),
        PortRef::new(osc, supersilvia::nodes::TIME),
    );
    let out = PortRef::new(osc, "output");
    let (from, to) = (1.0, 5.0);
    set(&mut app, gear, "ratio", from);

    let mut history = Vec::new();
    for frame in 0..180 {
        if frame == 50 {
            set(&mut app, gear, "ratio", to);
        }
        app.tick(FRAME);
        history.push(app.uniform(out).unwrap());
    }

    // A sine of unit amplitude at `to` Hz changes by at most `2π · to` a second, so one
    // frame of it is the most any step between samples can be.
    let bound = std::f32::consts::TAU * to * FRAME;
    let steps: Vec<f32> = history.windows(2).map(|w| w[1] - w[0]).collect();
    let largest = steps.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(
        largest <= bound * 1.01,
        "a step of {largest} over the slope's {bound}"
    );
    assert!(largest > bound / 4.0, "the wave moved: {largest}");
    assert!(
        history.iter().all(|v| (-1.001..=1.001).contains(v)),
        "inside its own amplitude"
    );
}

/// An animation holds its start value until something fires it, which is silvia's
/// `isRunning: false`: a travel is a thing a performer starts.
#[test]
fn an_animation_holds_its_start_until_it_is_started() {
    let mut app = App::headless();
    let anim = add_to(&mut app, "animation");
    let out = PortRef::new(anim, "output");
    set(&mut app, anim, "startValue", 0.25);
    for _ in 0..60 {
        app.tick(FRAME);
    }
    let v = app.uniform(out).unwrap();
    assert!((v - 0.25).abs() < 1e-6, "its start, and nothing else: {v}");

    tap_button(&mut app, anim, "startStop");
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert!(app.uniform(out).unwrap() > 0.3, "started, it travels");
}

/// The three things a return curve can mean, over the same duration: come back, snap back,
/// or stay. Whichever it is, the output never leaves the two values it was given.
#[test]
fn an_animations_return_curve_decides_what_happens_at_the_far_end() {
    /// A started animation of one second in one return mode, sampled every frame for two
    /// cycles. Index `n` is the value after frame `n + 1`.
    fn travel(mode: &str) -> Vec<f32> {
        let mut app = App::headless();
        let anim = add_to(&mut app, "animation");
        let out = PortRef::new(anim, "output");
        option(&mut app, anim, "return_curve", mode);
        tap_button(&mut app, anim, "startStop");
        let mut history = vec![app.uniform(out).unwrap()];
        for _ in 0..149 {
            app.tick(FRAME);
            history.push(app.uniform(out).unwrap());
        }
        history
    }

    for mode in ["smooth", "jump", "stay"] {
        let history = travel(mode);
        assert!(
            history.iter().all(|v| (-1e-6..=1.000_001).contains(v)),
            "{mode} leaves its range: {:?}",
            history.iter().fold(0.0f32, |a, b| a.max(*b))
        );
        // Inside the first second — sixty frames — every mode arrives at the far end.
        let reached = history[..60].iter().fold(0.0f32, |a, b| a.max(*b));
        assert!(
            reached > 0.99,
            "{mode} reaches the end of its first pass: {reached}"
        );
    }

    // A ping-pong comes back over the second cycle and is home at the end of it.
    let ping = travel("smooth");
    assert!(ping[61] > 0.99, "still at the far end: {}", ping[61]);
    assert!(ping[89] < 0.55, "coming back: {}", ping[89]);
    assert!(ping[119] < 0.01, "home at two cycles: {}", ping[119]);

    // Jump starts the next pass from the near end instead.
    let jump = travel("jump");
    assert!(jump[61] < 0.01, "snapped back: {}", jump[61]);
    assert!(jump[89] > 0.4, "and climbing again: {}", jump[89]);

    // Stay never leaves the far end.
    let stay = travel("stay");
    assert!(
        stay[61..].iter().all(|v| *v > 0.999),
        "stay holds: {:?}",
        stay[61..].iter().fold(1.0f32, |a, b| a.min(*b))
    );
}

/// **A seek carries a running animation across.** A five-second Linear animation started at
/// a hundred seconds in and two seconds into its pass reads the same just after the time
/// readout's reset — a seek to zero — as just before it, and goes on from there inside its
/// two ends, in Stay and in a ping-pong alike: the seek is a hand on the show, not a hundred
/// seconds of the animation's pass run backwards.
#[test]
fn a_seek_carries_a_running_animation_across() {
    for mode in ["stay", "linear"] {
        let mut app = App::headless();
        let anim = add_to(&mut app, "animation");
        let out = PortRef::new(anim, "output");
        set(&mut app, anim, "duration", 5.0);
        option(&mut app, anim, "approach_curve", "linear");
        option(&mut app, anim, "return_curve", mode);
        app.transport(supersilvia::transport::Command::Seek(100.0));
        app.tick(FRAME);
        tap_button(&mut app, anim, "startStop");
        for _ in 0..120 {
            app.tick(FRAME);
        }
        let before = app.uniform(out).unwrap();
        assert!(
            (before - 0.4).abs() < 0.02,
            "{mode}: two seconds in: {before}"
        );
        app.transport(supersilvia::transport::Command::Seek(0.0));
        app.tick(FRAME);
        let after = app.uniform(out).unwrap();
        assert!(
            (after - before).abs() < 1e-6,
            "{mode}: the seek leaves it where it was: {before} then {after}"
        );
        for _ in 0..600 {
            app.tick(FRAME);
            let v = app.uniform(out).unwrap();
            assert!(
                (-1e-6..=1.000_001).contains(&v),
                "{mode}: inside its ends after the seek: {v}"
            );
        }
    }
}

/// **A seek carries a playing automation across** as it does an animation: a two-second ramp
/// recorded a hundred seconds in and played once, half way through, reads the same just after
/// a seek to zero as just before it, where a hundred seconds run backwards put its playback
/// before the start of the take.
#[test]
fn a_seek_carries_a_playing_automation_across() {
    let mut app = App::headless();
    let node = add_to(&mut app, "automation");
    let out = PortRef::new(node, "output");
    set(&mut app, node, "duration", 2.0);
    option(&mut app, node, "loop", "once");
    app.transport(supersilvia::transport::Command::Seek(100.0));
    app.tick(FRAME);
    tap_button(&mut app, node, "record");
    for i in 0..=130 {
        set(&mut app, node, "input", (i as f32 / 120.0).min(1.0));
        app.tick(FRAME);
    }
    set(&mut app, node, "input", 0.0);
    tap_button(&mut app, node, "play");
    for _ in 0..60 {
        app.tick(FRAME);
    }
    let before = app.uniform(out).unwrap();
    assert!((before - 0.5).abs() < 0.05, "half way through: {before}");
    app.transport(supersilvia::transport::Command::Seek(0.0));
    app.tick(FRAME);
    let after = app.uniform(out).unwrap();
    assert!(
        (after - before).abs() < 1e-6,
        "the seek leaves it where it was: {before} then {after}"
    );
}

// ---------------------------------------------------------------- dual-mode math

/// Add a node through the command bus and return its id.
fn added(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// An `add` with two knobs is a constant: no `cpu` half, and the tick publishes its formula
/// as a number the same way a CPU node publishes one.
#[test]
fn an_add_with_two_knobs_publishes_their_sum() {
    let mut app = App::headless();
    let a = added(&mut app, "add");
    set(&mut app, a, "a", 1.5);
    set(&mut app, a, "b", -0.25);

    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(a, "output")), Some(1.25));

    // And it follows its knobs, as any published uniform number does.
    set(&mut app, a, "b", 2.0);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(a, "output")), Some(3.5));
}

/// `number` is the same value spelled the way a person asks for it: one knob, one uniform,
/// silvia's range — and the knob is a port like any other, so a cable overrides it and the
/// node passes a number on.
#[test]
fn a_number_publishes_its_knob_and_yields_to_a_cable() {
    let mut app = App::headless();
    let n = added(&mut app, "number");
    let out = PortRef::new(n, "output");
    assert_eq!(app.uniform(out), None, "nothing until the first tick");

    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(out), Some(0.0), "silvia's default");

    set(&mut app, n, "value", 12.5);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(out), Some(12.5));

    // silvia's range: -100 to 100, and a control is clamped to the range it declares.
    set(&mut app, n, "value", 1000.0);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(out), Some(100.0));

    let source = added(&mut app, "number");
    set(&mut app, source, "value", -7.0);
    app.apply(Command::Connect {
        from: PortRef::new(source, "output"),
        to: PortRef::new(n, "value"),
    })
    .expect("a uniform number into a uniform number input");
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(out),
        Some(-7.0),
        "the cable overrides the knob, producers first, in one tick"
    );
}

/// A chain of duals resolves inside one frame, because the tick runs producers first.
#[test]
fn a_chain_of_duals_resolves_in_one_frame() {
    let mut app = App::headless();
    let a = added(&mut app, "add");
    let m = added(&mut app, "multiply");
    set(&mut app, a, "a", 2.0);
    set(&mut app, a, "b", 3.0);
    set(&mut app, m, "b", 4.0);
    app.apply(Command::Connect {
        from: PortRef::new(a, "output"),
        to: PortRef::new(m, "a"),
    })
    .unwrap();

    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(PortRef::new(m, "output")),
        Some(20.0),
        "(2 + 3) * 4 in one tick, not two"
    );
}

/// **The point of the whole stage.** Uniform number arithmetic drives a `UniformNumber`
/// input: an `add` of two `phase` outputs is a number, so `phase.rate` takes it — and where
/// the same `add` also feeds a field, the shader reads it as a bare uniform with no function
/// of its own.
#[test]
fn an_add_of_two_uniforms_drives_a_rate_and_compiles_to_a_uniform() {
    let mut g = Graph::new();
    let p = add(&mut g, "ratiogear");
    let q = add(&mut g, "ratiogear");
    let sum = add(&mut g, "add");
    let driven = add(&mut g, "ratiogear");
    g.connect(PortRef::new(p, "cycles"), PortRef::new(sum, "a"))
        .unwrap();
    g.connect(PortRef::new(q, "cycles"), PortRef::new(sum, "b"))
        .unwrap();
    g.connect(PortRef::new(sum, "output"), PortRef::new(driven, "ratio"))
        .expect("a diamond add feeds a uniform number input, which is what it is for");
    assert_eq!(
        g.get(sum).unwrap().input("a").unwrap().ty,
        PortType::UniformNumber,
        "the rate reads its number, so the add is pinned — and a pinned node is still a \
         uniform to the compiler, which is what the rest of this asserts"
    );

    // The same number, seen from a shader.
    let zoom = add(&mut g, "zoom");
    let cb = add(&mut g, "checkerboard");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(sum, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let uniform = format!("u_float_add{sum}_output");
    assert_eq!(
        shader.uniforms.get(uniform.as_str()),
        Some(&UniformProvider::NodeUniform {
            node: sum,
            port: "output",
            ty: UniformType::Float,
        }),
    );
    assert!(
        !shader.body.contains(&format!("add{sum}_output(")),
        "a diamond emits no function: {}",
        shader.body
    );
}

/// A dual that flips to a field withdraws what it published. A field has no one reading, so
/// a number left behind would be a reading of nothing rather than of zero.
#[test]
fn a_flipped_dual_withdraws_its_uniform() {
    let mut app = App::headless();
    let a = added(&mut app, "add");
    set(&mut app, a, "a", 1.0);
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(a, "output")), Some(1.0));

    let luma = added(&mut app, "luminosity");
    app.apply(Command::Connect {
        from: PortRef::new(luma, "output"),
        to: PortRef::new(a, "a"),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(PortRef::new(a, "output")),
        None,
        "a field publishes nothing"
    );

    // And it comes back when the field goes.
    app.apply(Command::Disconnect {
        to: PortRef::new(a, "a"),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(a, "output")), Some(1.0));
}

/// A `reframerange` in diamond mode reads its `clamp` option through the tick, not through a
/// generator: there is no shader to bake it into.
#[test]
fn a_diamond_reframerange_reads_its_clamp_option() {
    let mut app = App::headless();
    let r = added(&mut app, "reframerange");
    for (key, value) in [
        ("input", 2.0),
        ("inMin", 0.0),
        ("inMax", 1.0),
        ("outMin", 0.0),
        ("outMax", 10.0),
    ] {
        set(&mut app, r, key, value);
    }
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(PortRef::new(r, "output")),
        Some(20.0),
        "unclamped, it keeps going past the top of the range"
    );

    app.apply(Command::SetOption {
        node: r,
        key: "clamp",
        value: "on".to_string(),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(PortRef::new(r, "output")), Some(10.0));
}

/// **A pinned dual node still reads what arrives.** With a `slew` on its output the `add`'s
/// inputs are `UniformNumber` — a field cannot land on them — and the tick reads them
/// exactly as it reads any input, so it goes on publishing the sum of the two `phase`s
/// driving it.
#[test]
fn a_pinned_add_publishes_the_sum_of_its_uniforms() {
    let mut app = App::headless();
    let p = added(&mut app, "ratiogear");
    let q = added(&mut app, "ratiogear");
    let sum = added(&mut app, "add");
    let slew = added(&mut app, "slew");
    set(&mut app, q, "ratio", 4.0);
    for (from, to) in [
        (PortRef::new(p, "cycles"), PortRef::new(sum, "a")),
        (PortRef::new(q, "cycles"), PortRef::new(sum, "b")),
        (PortRef::new(sum, "output"), PortRef::new(slew, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }

    assert_eq!(
        app.graph()
            .get(sum)
            .unwrap()
            .inputs
            .iter()
            .map(|i| i.ty)
            .collect::<Vec<_>>(),
        [PortType::UniformNumber, PortType::UniformNumber],
        "the slew reads its number, so both inputs are pinned"
    );

    app.tick(1.0 / 60.0);
    let a = app.uniform(PortRef::new(p, "cycles")).expect("p ran");
    let b = app.uniform(PortRef::new(q, "cycles")).expect("q ran");
    assert!(b > a, "two rates, two phases: {a} and {b}");
    assert_eq!(
        app.uniform(PortRef::new(sum, "output")),
        Some(a + b),
        "the sum arrived in the same tick the phases were published in"
    );
    assert_eq!(
        app.uniform(PortRef::new(slew, "output")),
        Some(a + b),
        "and the slew read it"
    );
}

// ---------------------------------------------------------------- triggeredrandom

/// The first tick already latches a random value, so the port never reads a stale zero;
/// after that, only a trigger moves it, and every value it latches stays in `[min, max]`.
#[test]
fn a_triggered_random_latches_only_on_a_down_and_stays_in_range() {
    let mut app = App::headless();
    let tr = added(&mut app, "triggeredrandom");
    set(&mut app, tr, "min", 5.0);
    set(&mut app, tr, "max", 6.0);
    let value = PortRef::new(tr, "value");

    app.tick(1.0 / 60.0);
    let first = app
        .uniform(value)
        .expect("the first tick already latched one");
    assert!((5.0..=6.0).contains(&first), "inside [min, max]: {first}");

    // Held steady with no trigger: the value does not move on its own.
    for _ in 0..5 {
        app.tick(1.0 / 60.0);
    }
    assert_eq!(app.uniform(value), Some(first), "no trigger, no change");

    // Fire several triggers: every latched value lands in range, and at least one differs
    // from the last — a fresh random each time, not the same number replayed.
    let mut seen = vec![first];
    for _ in 0..8 {
        app.press(PortRef::new(tr, "trigger"), true);
        app.tick(1.0 / 60.0);
        app.press(PortRef::new(tr, "trigger"), false);
        app.tick(1.0 / 60.0);
        let v = app.uniform(value).unwrap();
        assert!((5.0..=6.0).contains(&v), "inside [min, max]: {v}");
        seen.push(v);
    }
    assert!(
        seen.windows(2).any(|w| w[0] != w[1]),
        "a fresh random per trigger: {seen:?}"
    );
}

// ---------------------------------------------------------------- triggeredcolor

/// Magenta — silvia's power-on color — until the first trigger, then a fresh color on
/// every down, alpha always opaque.
#[test]
fn a_triggered_color_latches_a_fresh_color_on_a_down() {
    let mut app = App::headless();
    let tc = added(&mut app, "triggeredcolor");
    let (r, g, b, a) = (
        PortRef::new(tc, "r"),
        PortRef::new(tc, "g"),
        PortRef::new(tc, "b"),
        PortRef::new(tc, "a"),
    );

    app.tick(1.0 / 60.0);
    assert_eq!(
        (
            app.uniform(r),
            app.uniform(g),
            app.uniform(b),
            app.uniform(a)
        ),
        (Some(1.0), Some(0.0), Some(1.0), Some(1.0)),
        "magenta before the first trigger"
    );

    app.press(PortRef::new(tc, "trigger"), true);
    app.tick(1.0 / 60.0);
    let rolled = (
        app.uniform(r).unwrap(),
        app.uniform(g).unwrap(),
        app.uniform(b).unwrap(),
    );
    assert_ne!(
        rolled,
        (1.0, 0.0, 1.0),
        "a fresh color, not the power-on magenta held forever"
    );
    for v in [rolled.0, rolled.1, rolled.2] {
        assert!((0.0..=1.0).contains(&v), "{rolled:?}");
    }
    assert_eq!(app.uniform(a), Some(1.0), "alpha stays opaque, not rolled");

    // Held, not retriggered: the color does not move on its own.
    app.tick(1.0 / 60.0);
    assert_eq!(
        (
            app.uniform(r).unwrap(),
            app.uniform(g).unwrap(),
            app.uniform(b).unwrap()
        ),
        rolled,
        "no trigger, no change"
    );
}

// ---------------------------------------------------------------- the uniform color

/// The fourth cell of kind × rate obeys the same asymmetry the first three do: one color a
/// frame broadcasts across every pixel for nothing, and the reverse is a readback.
#[test]
fn a_uniform_color_feeds_a_fragment_color_and_never_a_number() {
    let mut g = Graph::new();
    let color = add(&mut g, "color");
    let checker = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    g.connect(
        PortRef::new(color, "output"),
        PortRef::new(checker, "color1"),
    )
    .expect("a uniform color into a varying color is the free direction");
    assert!(
        matches!(
            g.connect(PortRef::new(color, "output"), PortRef::new(zoom, "zoom")),
            Err(ConnectError::TypeMismatch { .. })
        ),
        "a color is not a number at either rate"
    );
}

#[test]
fn a_fragment_color_cannot_feed_a_uniform_color_input() {
    let mut g = Graph::new();
    let checker = add(&mut g, "checkerboard");
    let color = add(&mut g, "color");
    let err = g
        .connect(
            PortRef::new(checker, "output"),
            PortRef::new(color, "color"),
        )
        .unwrap_err();
    assert_eq!(
        err,
        ConnectError::TypeMismatch {
            from: PortType::VaryingColor,
            to: PortType::UniformColor
        },
        "a picture into a swatch costs a readback, and is a node, not a cable"
    );
}

/// The whole of the `color` node: a swatch read each tick and published as one color.
#[test]
fn the_color_node_publishes_its_swatch() {
    let mut app = App::headless();
    let color = added(&mut app, "color");
    let out = PortRef::new(color, "output");

    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform_color(out),
        Some(nodes::parse_hex_rgba("#ff0080ff")),
        "silvia's own default pink, published without a hand touching it"
    );

    app.apply(Command::SetControl {
        node: color,
        key: "color",
        value: ControlValue::Color([0.0, 0.25, 1.0, 1.0]),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform_color(out),
        Some([0.0, 0.25, 1.0, 1.0]),
        "a swatch moved is a color published on the next tick"
    );
}

/// One `color` into another: the downstream node's swatch is overridden by the cable, which
/// is the same override a number control takes.
#[test]
fn a_cable_into_a_swatch_overrides_it() {
    let mut app = App::headless();
    let a = added(&mut app, "color");
    let b = added(&mut app, "color");
    app.apply(Command::SetControl {
        node: a,
        key: "color",
        value: ControlValue::Color([0.0, 1.0, 0.5, 1.0]),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(a, "output"),
        to: PortRef::new(b, "color"),
    })
    .unwrap();

    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform_color(PortRef::new(b, "output")),
        Some([0.0, 1.0, 0.5, 1.0]),
        "what arrives, not the swatch underneath it"
    );
}

/// A published color reaches the shader by the name silvia gives it, declared as the `vec4`
/// it is rather than the `float` a uniform number would be.
#[test]
fn a_published_color_is_a_vec4_uniform() {
    let mut g = Graph::new();
    let color = add(&mut g, "color");
    let checker = add(&mut g, "checkerboard");
    let out = add(&mut g, "output");
    g.connect(
        PortRef::new(color, "output"),
        PortRef::new(checker, "color1"),
    )
    .unwrap();
    g.connect(PortRef::new(checker, "output"), PortRef::new(out, "input"))
        .unwrap();

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let name = format!("u_color_color{color}_output");
    assert_eq!(
        shader.uniforms.get(name.as_str()),
        Some(&UniformProvider::NodeUniform {
            node: color,
            port: "output",
            ty: UniformType::Vec4,
        }),
    );
    assert!(
        shader.body.contains(&format!("    {name}: vec4f,")),
        "declared as a vec4f member of the uniform struct:\n{}",
        shader.body
    );
}

/// A color is straight on the CPU and premultiplied in the shader, converted where the synth
/// resolves it: a swatch of `#ff000080` and a Color node publishing the same both reach the
/// module as half of red at half alpha, and the Color node's row still reads what was picked.
#[test]
fn a_color_reaches_its_shader_premultiplied() {
    const HALF_RED: [f32; 4] = [1.0, 0.0, 0.0, 0.5];
    let mut app = App::headless();
    let color = add_to(&mut app, "color");
    let checker = add_to(&mut app, "checkerboard");
    let out = add_to(&mut app, "output");
    for (node, key) in [(color, "color"), (checker, "color2")] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Color(HALF_RED),
        })
        .unwrap();
    }
    for (from, to) in [
        (
            PortRef::new(color, "output"),
            PortRef::new(checker, "color1"),
        ),
        (PortRef::new(checker, "output"), PortRef::new(out, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform_color(PortRef::new(color, "output")),
        Some(HALF_RED),
        "published straight"
    );

    let shader = compile::wgsl::build(app.graph(), out).expect("connected");
    let resolved = app.resolve_uniforms(&shader);
    let value = |name: String| {
        let (_, v) = resolved
            .iter()
            .find(|(n, _)| **n == *name)
            .unwrap_or_else(|| panic!("{name} is resolved"));
        v.clone()
    };
    let premultiplied = supersilvia::render::UniformValue::Vec4([0.5, 0.0, 0.0, 0.5]);
    assert_eq!(
        value(format!("u_color_color{color}_output")),
        premultiplied,
        "a published color"
    );
    assert_eq!(
        value(format!("u_control_checkerboard{checker}_color2")),
        premultiplied,
        "a color control"
    );
}

// ---------------------------------------------------------------------------- random

/// `random` is a choice, not a shake: the same seed answers the same number on every tick
/// and after a reset, a different seed answers a different one, and every answer is inside
/// Min and Max.
#[test]
fn a_random_is_repeatable_from_its_seed_and_inside_its_range() {
    use supersilvia::graph::PortRef;

    let mut app = App::headless();
    let node = add_to(&mut app, "random");
    set(&mut app, node, "min", 2.0);
    set(&mut app, node, "max", 3.0);
    set(&mut app, node, "seed", 47.0);
    app.tick(1.0 / 60.0);
    let first = app
        .uniform(PortRef::new(node, "output"))
        .expect("published");
    app.tick(1.0 / 60.0);
    let again = app
        .uniform(PortRef::new(node, "output"))
        .expect("published");
    assert_eq!(
        first, again,
        "the same seed answers the same number on every tick"
    );
    assert!(
        (2.0..=3.0).contains(&first),
        "{first} is inside Min and Max"
    );

    set(&mut app, node, "seed", 48.0);
    app.tick(1.0 / 60.0);
    let other = app
        .uniform(PortRef::new(node, "output"))
        .expect("published");
    assert_ne!(first, other, "a different seed answers a different number");

    set(&mut app, node, "seed", 47.0);
    app.tick(1.0 / 60.0);
    let back = app
        .uniform(PortRef::new(node, "output"))
        .expect("published");
    assert_eq!(first, back, "seed 47 is the same choice it was");
}

// ---------------------------------------------------------------- the pointer

/// A synthetic pointer through the context: `x` and `y` are the hand's place in the
/// picture's own world units, and off the surface the node publishes nothing at all.
///
/// The surface is [`Surface::Canvas`] here because a headless app has no picture window and
/// no preview panel; what is asserted is the seam all three write through, which is the one
/// a `wl_pointer` on the pictures thread fills for a popped-out picture.
#[test]
fn a_pointer_reaches_the_graph_and_goes_quiet_off_the_surface() {
    use supersilvia::pointer::{Reading, Surface};

    let mut app = App::headless();
    let node = add_to(&mut app, "mouseinput");
    app.apply(Command::SetOption {
        node,
        key: "framing",
        value: "canvas".to_string(),
    })
    .unwrap();

    let (x, y) = (PortRef::new(node, "x"), PortRef::new(node, "y"));
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(x), None, "no hand, no position");

    app.set_pointer(
        Surface::Canvas,
        Some(Reading {
            x: 0.75,
            y: -0.5,
            left: true,
            right: false,
        }),
    );
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(x), Some(0.75));
    assert_eq!(app.uniform(y), Some(-0.5));
    assert_eq!(app.uniform(PortRef::new(node, "leftButton")), Some(1.0));
    assert_eq!(app.uniform(PortRef::new(node, "rightButton")), Some(0.0));

    // Another surface is another hand: the node is framed against the canvas and says
    // nothing about a pointer over the preview.
    app.set_pointer(Surface::Canvas, None);
    app.set_pointer(Surface::Preview, Some(Reading::default()));
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.uniform(x),
        None,
        "off the framed surface it withdraws rather than holding where the hand was"
    );
    assert_eq!(app.uniform(PortRef::new(node, "leftButton")), None);
}

/// The framing option is the whole of which surface is read, and the picture — the default
/// — falls back to the mixer's preview when no picture is popped out.
#[test]
fn the_framing_option_picks_the_surface() {
    use supersilvia::pointer::{Reading, Surface};

    let mut app = App::headless();
    let node = add_to(&mut app, "mouseinput");
    let x = PortRef::new(node, "x");
    app.set_pointer(
        Surface::Preview,
        Some(Reading {
            x: 0.25,
            ..Reading::default()
        }),
    );
    app.set_pointer(
        Surface::Canvas,
        Some(Reading {
            x: -0.9,
            ..Reading::default()
        }),
    );

    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(x), Some(0.25), "the default reads the picture");

    app.apply(Command::SetOption {
        node,
        key: "framing",
        value: "canvas".to_string(),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(app.uniform(x), Some(-0.9), "and Canvas reads the canvas");
}

// ---------------------------------------------------------------- the wall clock

/// `clock` reads the world, not the graph: what it publishes is the system's own time of day
/// to the second, and the second output is the same reading as a fraction of its period.
#[test]
fn the_clock_publishes_the_time_of_day() {
    let mut app = App::headless();
    let clock = add_to(&mut app, "clock");
    option(&mut app, clock, "mode", "daySeconds");
    app.tick(FRAME);

    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs_f64()
        .rem_euclid(86_400.0) as f32;
    let published = app
        .uniform(PortRef::new(clock, "value"))
        .expect("published");
    assert!(
        (published - seconds).abs() < 1.0,
        "the seconds of the day, within a second of the system's: {published} against {seconds}"
    );
    let fraction = app
        .uniform(PortRef::new(clock, "normalized"))
        .expect("published");
    assert!(
        (fraction - published / 86_400.0).abs() < 1e-4,
        "and the same reading over its own period: {fraction}"
    );
}

/// The offset is a knob like any other, and it is what local time is here: an hour on it is
/// an hour on the reading.
#[test]
fn the_clocks_offset_moves_the_reading_by_the_hours_it_says() {
    let mut app = App::headless();
    let clock = add_to(&mut app, "clock");
    option(&mut app, clock, "mode", "daySeconds");
    app.tick(FRAME);
    let utc = app
        .uniform(PortRef::new(clock, "value"))
        .expect("published");

    set(&mut app, clock, "offset", 3.0);
    app.tick(FRAME);
    let local = app
        .uniform(PortRef::new(clock, "value"))
        .expect("published");
    // Modulo the day, since three hours past 23:00 is 02:00 rather than 26:00.
    let moved = (local - utc).rem_euclid(86_400.0);
    assert!(
        (moved - 3.0 * 3600.0).abs() < 2.0,
        "three hours on: {utc} became {local}"
    );
}

// ---------------------------------------------------------------- the recording

/// Record a knob moving, play it back, and the output follows the curve rather than the
/// knob — which is the whole of what the node is for.
///
/// The recording itself lands in the node's own values, where the file will carry it.
#[test]
fn a_recorded_move_plays_back_from_the_nodes_own_values() {
    let mut app = App::headless();
    let node = add_to(&mut app, "automation");
    let out = PortRef::new(node, "output");
    // A one second recording, so the whole of it fits in sixty frames.
    set(&mut app, node, "duration", 1.0);

    app.press(PortRef::new(node, "record"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "record"), false);
    // A ramp from nothing to everything, one frame at a time.
    for i in 0..=60 {
        set(&mut app, node, "input", i as f32 / 60.0);
        app.tick(FRAME);
    }
    let points = app
        .graph()
        .get(node)
        .and_then(|n| n.values.get("recording"))
        .and_then(supersilvia::graph::Value::points)
        .expect("the recording is one of the node's own values")
        .to_vec();
    assert!(points.len() > 10, "a move became points: {}", points.len());
    assert!(
        points.windows(2).all(|w| w[0].time <= w[1].time),
        "oldest first"
    );

    // The knob is put back where it started; what comes out is the recording, not the knob.
    set(&mut app, node, "input", 0.0);
    app.press(PortRef::new(node, "play"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "play"), false);
    for _ in 0..40 {
        app.tick(FRAME);
    }
    let played = app.uniform(out).expect("published");
    assert!(
        played > 0.4,
        "two thirds through a ramp, with the knob at zero: {played}"
    );
}

/// The two ends are a mapping, not part of the recording: moving them re-reads the same
/// curve. And with nothing recorded the node passes its own knob through, across those same
/// ends.
#[test]
fn the_automation_maps_its_knob_into_min_and_max_while_it_is_not_playing() {
    let mut app = App::headless();
    let node = add_to(&mut app, "automation");
    let out = PortRef::new(node, "output");
    set(&mut app, node, "input", 0.5);
    set(&mut app, node, "min", 2.0);
    set(&mut app, node, "max", 4.0);
    app.tick(FRAME);
    assert_eq!(app.uniform(out), Some(3.0), "half way between the two ends");
}

/// Clear empties the recording, and empties it in the document too: what a performer means
/// by clearing it is that the file no longer carries it.
#[test]
fn clearing_the_automation_empties_the_value_the_file_carries() {
    let mut app = App::headless();
    let node = add_to(&mut app, "automation");
    set(&mut app, node, "duration", 0.2);
    app.press(PortRef::new(node, "record"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "record"), false);
    for i in 0..20 {
        set(&mut app, node, "input", i as f32 / 20.0);
        app.tick(FRAME);
    }
    assert!(
        app.graph()
            .get(node)
            .and_then(|n| n.values.get("recording"))
            .and_then(supersilvia::graph::Value::points)
            .is_some_and(|p| !p.is_empty()),
        "something was recorded"
    );

    app.press(PortRef::new(node, "clear"), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, "clear"), false);
    app.tick(FRAME);
    assert_eq!(
        app.graph()
            .get(node)
            .and_then(|n| n.values.get("recording"))
            .and_then(supersilvia::graph::Value::points)
            .map(<[supersilvia::graph::Point]>::len),
        Some(0),
        "cleared, in the document as well as in the tick"
    );
}

// ---------------------------------------------------------------- the XY Pad

/// Write the pad's four hand controls as the pad does, in one edit.
fn put_puck(app: &mut App, node: NodeId, at: [f32; 2], velocity: [f32; 2]) {
    app.apply(Command::SetControls {
        node,
        values: vec![
            ("padX", ControlValue::Float(at[0])),
            ("padY", ControlValue::Float(at[1])),
            ("vx", ControlValue::Float(velocity[0])),
            ("vy", ControlValue::Float(velocity[1])),
        ],
    })
    .unwrap();
}

fn xy(app: &App, node: NodeId) -> (f32, f32) {
    (
        app.uniform(PortRef::new(node, "x"))
            .expect("X is published"),
        app.uniform(PortRef::new(node, "y"))
            .expect("Y is published"),
    )
}

/// Where the hand put the puck is what X and Y say, between the range ends: at silvia's -1
/// to 1 a pad unit is an output unit, and a range end moves what is published without moving
/// the puck.
#[test]
fn an_xy_pad_publishes_where_the_hand_put_it_between_its_range_ends() {
    let mut app = App::headless();
    let pad = add_to(&mut app, "xypad");
    app.tick(FRAME);
    assert_eq!(xy(&app, pad), (0.0, 0.0), "the middle, at rest");
    assert_eq!(app.uniform(PortRef::new(pad, "speed")), Some(0.0));

    put_puck(&mut app, pad, [0.5, -1.0], [0.0, 0.0]);
    app.tick(FRAME);
    assert_eq!(xy(&app, pad), (0.5, -1.0));

    set(&mut app, pad, "minX", 0.0);
    set(&mut app, pad, "maxX", 10.0);
    app.tick(FRAME);
    assert_eq!(
        xy(&app, pad).0,
        7.5,
        "three quarters of the way along 0 to 10"
    );
    assert_eq!(
        app.puck(pad).map(|p| p.at),
        Some([0.5, -1.0]),
        "and the puck did not move"
    );
}

/// The hand cannot put the puck off the pad, and in Clamp or Bounce a throw cannot carry it
/// off either: X and Y stay between the range ends. A bounce is a one-frame gate, `Down` on
/// the tick it hit and `Up` on the next.
#[test]
fn an_xy_pads_puck_stays_between_the_range_ends() {
    let mut app = App::headless();
    let pad = add_to(&mut app, "xypad");
    put_puck(&mut app, pad, [3.0, -7.0], [0.0, 0.0]);
    app.tick(FRAME);
    assert_eq!(
        xy(&app, pad),
        (1.0, -1.0),
        "the controls are the pad's own edges"
    );

    option(&mut app, pad, "edgeX", "clamp");
    put_puck(&mut app, pad, [0.0, 0.0], [40.0, 0.0]);
    for _ in 0..30 {
        app.tick(FRAME);
        assert!(xy(&app, pad).0 <= 1.0, "past the edge: {:?}", xy(&app, pad));
    }
    assert_eq!(xy(&app, pad).0, 1.0, "clamped at the edge");
    assert_eq!(
        app.uniform(PortRef::new(pad, "speed")),
        Some(0.0),
        "and stopped"
    );

    // Bounce: back off the wall, never past it, and it says so for exactly one frame.
    option(&mut app, pad, "edgeX", "bounce");
    set(&mut app, pad, "drag", 0.0);
    put_puck(&mut app, pad, [0.9, 0.0], [3.0, 0.0]);
    let bounced = PortRef::new(pad, "bounced");
    let mut downs = 0;
    for _ in 0..10 {
        app.tick(FRAME);
        let x = xy(&app, pad).0;
        assert!(x <= 1.0, "past the wall: {x}");
        let edges = app.edges(bounced).to_vec();
        if edges.iter().any(|e| e.is_down()) {
            downs += 1;
            app.tick(FRAME);
            assert!(
                app.edges(bounced).iter().all(|e| !e.is_down()) && !app.edges(bounced).is_empty(),
                "the gate closes on the next tick"
            );
        }
    }
    assert_eq!(downs, 1, "one wall, one bounce");
    let speed = app.uniform(PortRef::new(pad, "speed")).unwrap();
    assert!(
        (speed - 3.0 * 0.8).abs() < 1e-3,
        "Bounce keeps 0.8 of the speed: {speed}"
    );
}

/// A hand on the puck holds it: gravity does nothing until it lets go, and then it falls.
#[test]
fn a_held_puck_does_not_fly() {
    let mut app = App::headless();
    let pad = add_to(&mut app, "xypad");
    set(&mut app, pad, "gravityY", -3.0);
    let held = PortRef::new(pad, "puck");
    app.press(held, true);
    for _ in 0..20 {
        app.tick(FRAME);
    }
    assert_eq!(xy(&app, pad), (0.0, 0.0), "held where it was put");
    app.press(held, false);
    for _ in 0..20 {
        app.tick(FRAME);
    }
    assert!(
        xy(&app, pad).1 < -0.1,
        "let go, it fell: {:?}",
        xy(&app, pad)
    );
}

/// A preset starts silvia's motion — its puck, its velocity and its wells — a right-click takes
/// one well away and the next preset replaces them all, which is why the pad has no button that
/// clears them. The wells are the tick's, not the document's.
#[test]
fn an_xy_pad_preset_starts_its_flight_and_its_wells() {
    use supersilvia::nodes::cpu::{Pull, Touch};

    let mut app = App::headless();
    let pad = add_to(&mut app, "xypad");
    app.tick(FRAME);
    app.touch(pad, Touch::Preset(4)); // Figure 8: two wells.
    app.tick(FRAME);
    let puck = app.puck(pad).expect("the pad reports its puck").clone();
    assert_eq!(puck.wells.len(), 2, "{puck:?}");
    assert!(puck.velocity[0] > 1.0, "moving: {puck:?}");

    app.touch(
        pad,
        Touch::Well {
            at: [0.5, 0.5],
            reach: None,
        },
    );
    app.tick(FRAME);
    let wells = &app.puck(pad).unwrap().wells;
    assert_eq!(wells.len(), 3);
    assert_eq!(
        wells[2].pull,
        Pull::Gravity(2.0),
        "a click is silvia's default"
    );
    assert!(
        app.graph().get(pad).unwrap().values.is_empty(),
        "and nothing about a well is in the document"
    );

    app.touch(pad, Touch::Unwell(0));
    app.tick(FRAME);
    assert_eq!(app.puck(pad).unwrap().wells.len(), 2);
    app.touch(pad, Touch::Preset(0)); // DVD: no wells.
    app.tick(FRAME);
    assert!(app.puck(pad).unwrap().wells.is_empty());
    assert!(
        app.graph()
            .get(pad)
            .unwrap()
            .inputs
            .iter()
            .all(|p| p.key != "clearWells"),
        "no Clear Wells button"
    );
}
