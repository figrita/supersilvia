// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the plan is built only when something it reads changed, a source is sent to the
//! renderer once, and an Output is submitted after the Outputs whose frames it samples.
//!
//! The editor paints far more often than anything the plan reads changes: every frame of a
//! scrub, and every frame nobody touches anything. A plan built and sent on each of those was
//! a whole walk of the graph for nothing. And an undo replaces the graph whole, so every
//! Output is rebuilt — but one whose source comes out as it was must not be linked again.

mod common;

use common::{add_on, connect, picture_on, two_tabs};
use emath::Pos2;
use std::collections::HashSet;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::mixer::{Channel, Method};
use supersilvia::ui::maininput::MainInputAction;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

fn add(app: &mut App, slug: &'static str, x: f32) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::new(x, 0.0),
        workspace,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// A checkerboard into an Output, with a plan built and taken.
fn patch() -> (App, NodeId, NodeId) {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard", 0.0);
    let out = add(&mut app, "output", 300.0);
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.publish_plan();
    app.tick(FRAME);
    (app, cb, out)
}

/// One editor frame's worth of the plan: the tick, then the plan as the frame asks for it.
fn frame(app: &mut App) {
    app.tick(FRAME);
    app.publish_plan();
}

#[test]
fn an_unchanged_frame_builds_and_sends_no_plan() {
    let (mut app, cb, _) = patch();
    let built = app.plans_built();
    for _ in 0..10 {
        frame(&mut app);
    }
    assert_eq!(
        app.plans_built(),
        built,
        "nothing changed, so nothing was built"
    );

    // A scrub writes a control, which is a uniform: the plan reads no value.
    for i in 0..10 {
        app.apply(Command::SetControl {
            node: cb,
            key: "frequency",
            value: ControlValue::Float(4.0 + i as f32),
        })
        .unwrap();
        frame(&mut app);
    }
    assert_eq!(app.plans_built(), built, "a scrub builds no plan");

    // A drag is a position, which the plan reads no more than a value.
    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(40.0, 40.0))],
    })
    .unwrap();
    frame(&mut app);
    assert_eq!(app.plans_built(), built, "a drag builds no plan");

    // What the plan does read, each once.
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug: "hsla",
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    frame(&mut app);
    assert_eq!(
        app.plans_built(),
        built + 1,
        "an edit to the graph's shape builds one"
    );
    frame(&mut app);
    assert_eq!(
        app.plans_built(),
        built + 1,
        "and the frame after builds none"
    );
}

/// **An option set to the value it holds is no edit**: no undo step, no graph handed to the
/// synth, no plan built and no Output rebuilt. An Output's resolution set again to its own size
/// is the case, as a picker or a select chosen twice sends it; a new size is one plan and no
/// rebuild, since the shader does not read the size.
#[test]
fn an_option_set_to_the_value_it_holds_builds_nothing() {
    let (mut app, _, out) = patch();
    frame(&mut app);
    let (steps, generation, built, sent) = (
        app.undo_len(),
        app.graph_generation(),
        app.plans_built(),
        app.sources_sent(),
    );
    let held = app.graph().get(out).unwrap().options["resolution"].clone();
    for _ in 0..3 {
        app.apply(Command::SetOption {
            node: out,
            key: "resolution",
            value: held.clone(),
        })
        .unwrap();
        frame(&mut app);
    }
    assert_eq!(app.undo_len(), steps, "no undo step");
    assert_eq!(app.graph_generation(), generation, "no graph handed over");
    assert_eq!(app.plans_built(), built, "no plan built");
    assert!(!app.needs_recompile(out), "and nothing to rebuild");

    app.apply(Command::SetOption {
        node: out,
        key: "resolution",
        value: "1920x1080".to_string(),
    })
    .unwrap();
    assert!(
        !app.needs_recompile(out),
        "a new size rebuilds no shader: the program draws at any size"
    );
    frame(&mut app);
    frame(&mut app);
    assert_eq!(app.undo_len(), steps + 1, "a new size is one step");
    assert_eq!(app.plans_built(), built + 1, "and one plan");
    assert_eq!(app.sources_sent(), sent, "with no source sent");
    assert_eq!(
        app.build_frame_job()
            .outputs
            .iter()
            .find(|o| o.node == out)
            .map(|o| o.resolution),
        Some((1920, 1080)),
        "which carries the size to the renderer"
    );
}

/// The fade and the crossfade method cross on their own: every frame of a fade is a new
/// balance, and nothing a plan works out reads it.
#[test]
fn a_fade_builds_no_plan_and_reaches_the_mix() {
    let (mut app, _, _) = patch();
    let built = app.plans_built();
    for i in 0..10 {
        app.set_balance(-1.0 + i as f32 / 5.0);
        frame(&mut app);
    }
    app.set_method(Method::Checkerboard);
    frame(&mut app);
    assert_eq!(app.plans_built(), built, "a fade builds no plan");

    let job = app.build_frame_job();
    assert!(
        (job.mixer.balance - 0.8).abs() < 1e-6,
        "{}",
        job.mixer.balance
    );
    assert_eq!(job.mixer.method, Method::Checkerboard);
}

/// What the editor is reading — the Status box, the Main Input panel's picture — is the
/// tick's business and not the plan's.
#[test]
fn the_status_box_and_the_main_input_panel_build_no_plan() {
    let (mut app, _, _) = patch();
    let built = app.plans_built();
    for i in 0..10 {
        app.set_show_status_box(i % 2 == 0);
        app.handle_main_input(MainInputAction::SetCollapsed(i % 2 == 1));
        frame(&mut app);
    }
    assert_eq!(app.plans_built(), built);
}

/// Which pass measures a workspace's tap does not read the mixer, so a cut, which is playing
/// rather than editing, rebuilds nothing.
#[test]
fn a_cut_moves_no_measurement() {
    let (mut app, first, _) = two_tabs();
    let a = picture_on(&mut app, first);
    let b = picture_on(&mut app, first);
    let source = add_on(&mut app, "checkerboard", first);
    let tap = add_on(&mut app, "tap", first);
    connect(&mut app, (source, "output"), (tap, "input"));
    app.publish_plan();
    assert_eq!(app.pass_measures(first), [tap], "its workspace's pass");
    let sent = app.sources_sent();

    for channel in [Channel::A, Channel::B] {
        app.show_on(channel, b);
        assert!(!app.needs_recompile(a) && !app.needs_recompile(b));
        app.publish_plan();
        assert_eq!(app.pass_measures(first), [tap]);
    }
    assert_eq!(app.sources_sent(), sent, "nothing relinks");
}

#[test]
fn a_tab_closed_and_opened_builds_a_plan_each_way() {
    let (mut app, _, out) = patch();
    let second = {
        app.add_workspace(supersilvia::graph::WorkspaceKind::Video);
        app.graph().workspaces().last().unwrap().id
    };
    frame(&mut app);
    let built = app.plans_built();
    let first = app.graph().default_workspace();

    app.close_workspace(first);
    frame(&mut app);
    assert_eq!(app.plans_built(), built + 1);
    let job = app.build_frame_job();
    assert!(
        job.outputs
            .iter()
            .any(|o| o.node == out && o.mode == supersilvia::render::OutputMode::Suspended),
        "the plan it built suspends the Output on the closed tab"
    );

    app.open_workspace(first);
    frame(&mut app);
    assert_eq!(app.plans_built(), built + 2);
    let _ = second;
}

#[test]
fn an_undo_of_a_move_sends_no_shader() {
    let (mut app, cb, _) = patch();
    let sent = app.sources_sent();
    assert!(
        sent >= 2,
        "the Output's shader and its probe went out once each: {sent}"
    );

    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(120.0, 80.0))],
    })
    .unwrap();
    app.end_gesture();
    frame(&mut app);
    assert!(app.undo());
    frame(&mut app);
    assert_eq!(
        app.sources_sent(),
        sent,
        "the graph came back whole, and every Output compiled to what the renderer holds"
    );
    let job = app.build_frame_job();
    assert!(job.outputs.iter().all(|o| o.shader.is_none()));
    assert!(job.probes.iter().all(|p| p.shader.is_none()));

    // The redo, likewise.
    assert!(app.redo());
    frame(&mut app);
    assert_eq!(app.sources_sent(), sent);
}

#[test]
fn an_undo_of_a_cable_sends_the_source_it_restores() {
    let (mut app, cb, out) = patch();
    let sent = app.sources_sent();
    let hsla = add(&mut app, "hsla", 150.0);
    frame(&mut app);
    app.apply(Command::Connect {
        from: PortRef::new(hsla, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    frame(&mut app);
    let after_edit = app.sources_sent();
    assert!(after_edit > sent, "a new source went out");

    assert!(app.undo());
    frame(&mut app);
    assert!(
        app.sources_sent() > after_edit,
        "the checkerboard's source is not what the renderer holds, so it goes back"
    );
    let _ = cb;
}

// ---------------------------------------------------------------- the order

/// The Outputs as the synth submits them this tick.
fn submitted(app: &mut App) -> Vec<NodeId> {
    app.build_frame_job()
        .outputs
        .iter()
        .map(|o| o.node)
        .collect()
}

fn wire(app: &mut App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    app.apply(Command::Connect {
        from: PortRef::new(from.0, from.1),
        to: PortRef::new(to.0, to.1),
    })
    .unwrap();
}

/// A feedback loop anywhere in the graph leaves the Outputs downstream of it in dependency
/// order: a consumer made before its producer, both stateless, reads the producer's frame of
/// the same tick.
#[test]
fn a_consumer_downstream_of_a_loop_is_submitted_after_its_producer() {
    let mut app = App::headless();
    // A loop: a checkerboard mixed with the loop's own frame.
    let l = add(&mut app, "output", 0.0);
    let cb = add(&mut app, "checkerboard", 0.0);
    let mix = add(&mut app, "mix", 0.0);
    wire(&mut app, (cb, "output"), (mix, "a"));
    wire(&mut app, (l, "frame"), (mix, "b"));
    wire(&mut app, (mix, "output"), (l, "input"));
    // Q is made before P and reads it; P reads the loop.
    let q = add(&mut app, "output", 0.0);
    let p = add(&mut app, "output", 0.0);
    wire(&mut app, (l, "frame"), (p, "input"));
    wire(&mut app, (p, "frame"), (q, "input"));

    let order = submitted(&mut app);
    let at = |id: NodeId| order.iter().position(|o| *o == id).unwrap();
    let every: HashSet<NodeId> = app.graph().iter().map(|(id, _)| id).collect();
    let loops = app.graph().in_cycles(&every);
    assert!(loops.contains(&l) && !loops.contains(&p) && !loops.contains(&q));
    assert!(at(l) < at(p), "{order:?}");
    assert!(at(p) < at(q), "Q reads P's frame of this tick: {order:?}");
}

/// **A consumer reads its producer's frame of the same tick.** The patch a person made on
/// the tab being looked at — Simplex Noise into Output A, A's frame through Height to Normal
/// and Simple Light into Output B — beside two more Outputs on the tab: B is submitted after A
/// on every tick.
#[test]
fn a_frame_read_through_nodes_keeps_its_producer_first() {
    let (mut app, first, _) = two_tabs();
    let a = add_on(&mut app, "output", first);
    let noise = add_on(&mut app, "simplex", first);
    connect(&mut app, (noise, "color"), (a, "input"));
    let normal = add_on(&mut app, "heighttonormal", first);
    let light = add_on(&mut app, "simplelight", first);
    let b = add_on(&mut app, "output", first);
    connect(&mut app, (a, "frame"), (normal, "input"));
    connect(&mut app, (normal, "color"), (light, "normalMap"));
    connect(&mut app, (light, "color"), (b, "input"));
    let others = [picture_on(&mut app, first), picture_on(&mut app, first)];

    for _ in 0..3 {
        let job = app.build_frame_job();
        let order: Vec<NodeId> = job
            .outputs
            .iter()
            .filter(|o| o.mode.draws())
            .map(|o| o.node)
            .collect();
        assert_eq!(order.len(), 4, "{order:?}");
        for id in [a, b, others[0], others[1]] {
            assert!(order.contains(&id), "{order:?}");
        }
        let at = |id: NodeId| order.iter().position(|o| *o == id).unwrap();
        assert!(at(a) < at(b), "B after A: {order:?}");
    }
}

/// **A measurement orders no Output.** A tap on an Output's Frame Out that nothing draws is
/// measured in its workspace's pass, which is drawn after every Output, so it reads this
/// tick's frame whatever the order: the Outputs are in the order their own cables give.
#[test]
fn a_tap_on_a_frame_orders_no_output() {
    let mut app = App::headless();
    let first = add(&mut app, "output", 0.0);
    let cb = add(&mut app, "checkerboard", 0.0);
    let producer = add(&mut app, "output", 0.0);
    let tap = add(&mut app, "tap", 0.0);
    wire(&mut app, (cb, "output"), (first, "input"));
    wire(&mut app, (cb, "output"), (producer, "input"));
    wire(&mut app, (producer, "frame"), (tap, "input"));
    let alone = submitted(&mut app);
    wire(&mut app, (first, "frame"), (tap, "input"));
    assert_eq!(
        submitted(&mut app),
        alone,
        "what the tap measures moves nothing"
    );
    assert_eq!(app.pass_measures(app.graph().default_workspace()), [tap]);
}

/// Over 3000 seeded random graphs of Outputs, mixes and taps, loops among them included: an
/// Output whose shader samples another Output's frame is submitted after it, unless a path
/// of cables runs back from the reader to the producer, which is a loop and reads a frame
/// back by design. A tap's measurement samples frames too, in its pass after every Output.
#[test]
fn every_frame_a_shader_samples_is_drawn_first_outside_a_loop_in_random_graphs() {
    let mut failed = Vec::new();
    let (mut pairs, mut looped) = (0, 0);
    for seed in 0..3000 {
        let graph = sampled_before_reader(seed);
        pairs += graph.pairs;
        looped += usize::from(graph.looped);
        if graph.late {
            failed.push(seed);
        }
    }
    assert!(
        failed.is_empty(),
        "{} of 3000 graphs read a producer's previous frame with no loop: seeds {failed:?}",
        failed.len()
    );
    assert!(
        pairs > 1000 && looped > 300,
        "{pairs} pairs, {looped} with an Output in a loop"
    );
}

/// One seeded graph, as the test above counts it.
struct Sampled {
    /// A reader is submitted before a producer it samples outside a loop.
    late: bool,
    /// How many reader and producer pairs outside a loop it checked.
    pairs: usize,
    /// Some Output is in a loop.
    looped: bool,
}

fn sampled_before_reader(seed: u64) -> Sampled {
    const POOL: &[&str] = &[
        "output",
        "output",
        "output",
        "checkerboard",
        "mix",
        "mix",
        "tap",
    ];
    const PICTURE_OUT: &[(&str, &str)] = &[
        ("checkerboard", "output"),
        ("mix", "output"),
        ("tap", "output"),
        ("output", "frame"),
        ("output", "frame"),
    ];
    const PICTURE_IN: &[(&str, &str)] = &[
        ("output", "input"),
        ("output", "input"),
        ("mix", "a"),
        ("mix", "b"),
        ("tap", "input"),
    ];
    let mut state: u64 = 0x0DDE_12ED_5EED ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut next = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let mut app = App::headless();
    let kinds: Vec<&'static str> = (0..4 + next(8)).map(|_| POOL[next(POOL.len())]).collect();
    let ids: Vec<NodeId> = kinds.iter().map(|slug| add(&mut app, slug, 0.0)).collect();
    let of = |slug: &str| -> Vec<NodeId> {
        (0..ids.len())
            .filter(|i| kinds[*i] == slug)
            .map(|i| ids[i])
            .collect()
    };
    for _ in 0..ids.len() * 2 {
        let ((fs, fk), (ts, tk)) = if next(5) == 0 {
            (("tap", "mean"), ("mix", "amount"))
        } else {
            (
                PICTURE_OUT[next(PICTURE_OUT.len())],
                PICTURE_IN[next(PICTURE_IN.len())],
            )
        };
        let (froms, tos) = (of(fs), of(ts));
        if froms.is_empty() || tos.is_empty() {
            continue;
        }
        let _ = app.apply(Command::Connect {
            from: PortRef::new(froms[next(froms.len())], fk),
            to: PortRef::new(tos[next(tos.len())], tk),
        });
    }

    let outputs = of("output");
    let job = app.build_frame_job();
    let order: Vec<NodeId> = job.outputs.iter().map(|o| o.node).collect();
    let at = |id: NodeId| order.iter().position(|o| *o == id).unwrap();
    let g = app.graph();
    // Over data edges, delayed ones included: a path back is a loop.
    let reaches = |from: NodeId, to: NodeId| {
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(v) = stack.pop() {
            for c in g.connections().iter().filter(|c| c.from.node == v) {
                let port = g.get(v).unwrap().output(c.from.key).unwrap();
                if !port.ty.is_data() {
                    continue;
                }
                if c.to.node == to {
                    return true;
                }
                if seen.insert(c.to.node) {
                    stack.push(c.to.node);
                }
            }
        }
        false
    };
    let every: HashSet<NodeId> = g.iter().map(|(id, _)| id).collect();
    let cyclic = g.in_cycles(&every);
    let looped = outputs.iter().any(|o| cyclic.contains(o));
    let (mut late, mut pairs) = (false, 0);
    for reader in &job.outputs {
        for value in &reader.uniforms {
            let supersilvia::render::UniformValue::NodeTexture(port) = &value.1 else {
                continue;
            };
            let p = port.node;
            if p == reader.node || !outputs.contains(&p) || reaches(reader.node, p) {
                continue;
            }
            pairs += 1;
            late |= at(p) > at(reader.node);
        }
    }
    Sampled {
        late,
        pairs,
        looped,
    }
}
