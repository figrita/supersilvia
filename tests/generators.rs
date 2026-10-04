// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the generators' Time — ambient time at each node's own rate, or a gear's Cycles in
//! its place, with Offset added and nothing integrated.
//!
//! What a person would see: a node that moves with time is where the playhead puts it,
//! whatever the path there — played, sought, paused and played again — because it keeps no
//! state; a gear cabled into Time replaces the ambient reading; Repeat and the tunnel's depth
//! wrap bring the reading round at the picture's own period; and Repeat is a rebuild. The
//! pictures themselves are `tests/gpu_nodes.rs`' and `tests/gpu_app.rs`'. See
//! `proposals/time.md` and `docs/nodes.md#timing`.

use emath::Pos2;
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::nodes::{self, TIME};
use supersilvia::transport::Command as Transport;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

fn option(app: &mut App, node: NodeId, key: &'static str, value: &str) {
    app.apply(Command::SetOption {
        node,
        key,
        value: value.to_string(),
    })
    .unwrap();
}

/// What a node's Time reads this tick: the ambient reading published under its own key.
fn time(app: &App, node: NodeId) -> f64 {
    let count = app
        .count(PortRef::new(node, TIME))
        .unwrap_or_else(|| panic!("node {node}'s Time is published"));
    read_as_the_shader_does(app, node, count)
}

/// A count as a node's shader reads it: split into its whole part and its fraction, the whole
/// part taken modulo the node's period and the fraction added, or the two added where the
/// picture never repeats.
fn read_as_the_shader_does(app: &App, node: NodeId, count: f64) -> f64 {
    let n = app.graph().get(node).unwrap();
    let [whole, fraction] = nodes::phasor::split(count);
    let (whole, fraction) = (f64::from(whole), f64::from(fraction));
    match (n.def.timing.unwrap().period)(n) {
        Some(period) => whole.rem_euclid(period) + fraction,
        None => whole + fraction,
    }
}

/// Every node that moves with time and draws.
fn ambient_slugs() -> Vec<&'static str> {
    nodes::REGISTRY
        .iter()
        .filter(|d| d.timing.is_some() && d.cpu.is_none())
        .map(|d| d.slug)
        .collect()
}

/// **Unplugged, Time is ambient time**: the playhead times the node's own rate, published as
/// a count, which the shader reads modulo its period — one cycle for a periodic node — or
/// whole, for a picture that never repeats.
#[test]
fn unplugged_time_is_the_playhead_at_the_nodes_rate() {
    let slugs = ambient_slugs();
    assert_eq!(slugs.len(), 12, "{slugs:?}");
    let mut app = App::headless();
    let ids: Vec<(NodeId, &str)> = slugs.iter().map(|s| (add(&mut app, s), *s)).collect();
    app.transport(Transport::Seek(37.25));
    app.tick(FRAME);
    let playhead = app.transport_state().playhead;
    for (id, slug) in ids {
        let def = nodes::find(slug).unwrap();
        let ambient = def.timing.unwrap();
        let node = app.graph().get(id).unwrap();
        let expected = match (ambient.period)(node) {
            Some(period) => (playhead * ambient.rate).rem_euclid(period),
            None => playhead * ambient.rate,
        };
        let got = time(&app, id);
        assert!(
            (got - expected).abs() < 1e-4,
            "{slug}: Time reads {got}, the playhead at its rate is {expected}"
        );
    }
}

/// **A CPU node's unplugged Time is published too**, under its own key, before its Offset:
/// the playhead at its rate in Loop mode, its own playhead at its pace in Free mode — what the
/// loop meter on its Time row reads.
#[test]
fn a_cpu_nodes_unplugged_time_is_published_under_its_key() {
    let mut app = App::headless();
    let osc = add(&mut app, "oscillator");
    let seq = add(&mut app, "stepsequencer");
    for id in [osc, seq] {
        option(&mut app, id, "clockMode", "loop");
    }
    app.apply(Command::SetControl {
        node: osc,
        key: nodes::timing::OFFSET,
        value: supersilvia::graph::ControlValue::Float(0.25),
    })
    .unwrap();
    app.transport(Transport::Seek(37.25));
    app.tick(FRAME);
    let playhead = app.transport_state().playhead;
    let at = |app: &App, id| app.count(PortRef::new(id, TIME)).unwrap();
    assert!(
        (at(&app, osc) - playhead).abs() < 1e-9,
        "a cycle a second, Offset left out"
    );
    assert_eq!(
        at(&app, seq),
        0.0,
        "a sequencer sits still in Loop mode with nothing in it"
    );
    option(&mut app, seq, "clockMode", "free");
    app.apply(Command::SetControl {
        node: seq,
        key: nodes::timing::SPEED,
        value: supersilvia::graph::ControlValue::Float(1.0),
    })
    .unwrap();
    app.tick(FRAME);
    assert!(
        (at(&app, seq) - app.transport_state().playhead * 0.5).abs() < 1e-6,
        "free at Speed 1, its playhead at its pace of half a bar a second"
    );
}

/// **A node that moves with time keeps no state**: sought to a moment, played through to it,
/// or sought away and back, its Time reads the same there — so its picture is the same, since
/// nothing else of it moved.
#[test]
fn the_same_moment_reads_the_same_whatever_the_path() {
    let at = 12.5;
    let reading = |path: &dyn Fn(&mut App)| {
        let mut app = App::headless();
        let perlin = add(&mut app, "perlin");
        let turn = add(&mut app, "rotozoom");
        path(&mut app);
        (time(&app, perlin), time(&app, turn))
    };
    let sought = reading(&|app| {
        app.transport(Transport::Seek(at));
        app.tick_at(at);
    });
    let played = reading(&|app| {
        app.transport(Transport::Seek(0.0));
        let frames = (at * 60.0).round() as u32;
        for i in 0..=frames {
            app.tick_at(f64::from(i) / 60.0);
        }
    });
    let wandered = reading(&|app| {
        app.transport(Transport::Seek(3.0));
        app.tick_at(3.0);
        app.transport(Transport::Seek(400.0));
        app.tick_at(400.0);
        app.transport(Transport::Seek(at));
        app.tick_at(at);
    });
    assert_eq!(sought, played, "played through, it is where a seek puts it");
    assert_eq!(sought, wandered, "and wherever it has been since");
    assert!((sought.0 - 0.5 * at).abs() < 1e-5, "{sought:?}");
}

/// **Paused, Time holds**, because it is the playhead's.
#[test]
fn a_pause_holds_time() {
    let mut app = App::headless();
    let perlin = add(&mut app, "perlin");
    for _ in 0..30 {
        app.tick(FRAME);
    }
    app.transport(Transport::Pause);
    app.tick(FRAME);
    let held = time(&app, perlin);
    for _ in 0..60 {
        app.tick(FRAME);
    }
    assert_eq!(time(&app, perlin), held);
}

fn speed(app: &mut App, node: NodeId, value: f32) {
    app.apply(Command::SetControl {
        node,
        key: "speed",
        value: supersilvia::graph::ControlValue::Float(value),
    })
    .unwrap();
}

/// The count a node's Time is published as, unwrapped.
fn count(app: &App, node: NodeId) -> f64 {
    app.count(PortRef::new(node, TIME)).unwrap()
}

/// **Running free, a node moves at its Speed times its pace**, integrated against the
/// transport: Perlin at Speed 1 walks half a cell a second; at 2, twice that, gliding there
/// rather than jumping; at 0 it stands; paused it holds whatever its Speed; and a seek moves it
/// by its Speed times the jump — backwards for a negative Speed.
#[test]
fn a_free_node_runs_at_its_speed_times_its_pace() {
    let mut app = App::headless();
    let perlin = add(&mut app, "perlin");
    app.transport(Transport::Seek(0.0));
    app.tick_at(0.0);
    let mut t = 0.0;
    let run = |app: &mut App, t: &mut f64, seconds: f64| {
        for _ in 0..(seconds * 60.0).round() as u32 {
            *t += 1.0 / 60.0;
            app.tick_at(*t);
        }
    };
    run(&mut app, &mut t, 1.0);
    assert!(
        (count(&app, perlin) - 0.5).abs() < 1e-9,
        "{}",
        count(&app, perlin)
    );

    speed(&mut app, perlin, 2.0);
    let before = count(&app, perlin);
    run(&mut app, &mut t, 1.0);
    let moved = count(&app, perlin) - before;
    assert!(
        moved < 1.0 && moved > 0.95,
        "it bends to the new pace: {moved}"
    );

    speed(&mut app, perlin, 0.0);
    run(&mut app, &mut t, 1.0);
    let still = count(&app, perlin);
    run(&mut app, &mut t, 1.0);
    assert!((count(&app, perlin) - still).abs() < 1e-9, "at 0 it stands");

    speed(&mut app, perlin, 1.0);
    run(&mut app, &mut t, 1.0);
    app.transport(Transport::Pause);
    app.tick_at(t);
    let held = count(&app, perlin);
    for _ in 0..60 {
        app.tick_at(t);
    }
    assert_eq!(count(&app, perlin), held, "paused, it holds");
    app.transport(Transport::Play);

    speed(&mut app, perlin, -1.0);
    run(&mut app, &mut t, 1.0);
    let before = count(&app, perlin);
    t += 10.0;
    app.transport(Transport::Seek(t));
    app.tick_at(t);
    let jumped = count(&app, perlin) - before;
    assert!(
        (jumped + 5.0).abs() < 1e-6,
        "a seek of 10 s at Speed −1 is −5 cells: {jumped}"
    );
}

/// **A stepped Speed lands on the same place at any frame rate**: the glide is integrated in
/// closed form, so Speed stepped from 1 to 3 at one second reads the same three seconds in
/// at 30 and at 144 frames a second.
#[test]
fn a_stepped_speed_lands_on_the_same_place_at_30_and_144_fps() {
    let at = |fps: f64| {
        let mut app = App::headless();
        let fractal = add(&mut app, "mandelbrot");
        app.transport(Transport::Seek(0.0));
        app.tick_at(0.0);
        let frames = (3.0 * fps).round() as u32;
        for i in 1..=frames {
            if i == fps as u32 + 1 {
                speed(&mut app, fractal, 3.0);
            }
            app.tick_at(f64::from(i) / fps);
        }
        count(&app, fractal)
    };
    let (slow, fast) = (at(30.0), at(144.0));
    assert!((slow - fast).abs() < 1e-9, "{slow} and {fast}");
    assert!(slow > 0.5 + 2.0 && slow < 0.5 + 3.0, "{slow}");
}

/// **A Speed knob turned while a node loops does nothing, and Loop reads ambient time**:
/// switched to Loop, Mandelbrot is where the playhead at its rate puts it, whatever its Speed
/// did before; switched back, it is born there again.
#[test]
fn loop_mode_reads_ambient_time_whatever_the_speed_did() {
    let mut app = App::headless();
    let fractal = add(&mut app, "mandelbrot");
    app.transport(Transport::Seek(0.0));
    app.tick_at(0.0);
    speed(&mut app, fractal, 4.0);
    for i in 1..=120 {
        app.tick_at(f64::from(i) / 60.0);
    }
    assert!(count(&app, fractal) > 3.5, "it ran at four times its pace");
    option(&mut app, fractal, "clockMode", "loop");
    app.tick_at(121.0 / 60.0);
    assert!((count(&app, fractal) - 0.5 * 121.0 / 60.0).abs() < 1e-9);
    option(&mut app, fractal, "clockMode", "free");
    app.tick_at(122.0 / 60.0);
    assert!(
        (count(&app, fractal) - 4.0 * 0.5 * 122.0 / 60.0).abs() < 1e-9,
        "born again where the playhead puts it, at its Speed"
    );
}

/// **A gear in Time replaces the ambient reading**: the shader reads the gear's Cycles, and
/// no ambient reading at all.
#[test]
fn a_gear_in_time_replaces_the_ambient_reading() {
    let mut app = App::headless();
    let perlin = add(&mut app, "perlin");
    app.tick(FRAME);
    assert!(app.count(PortRef::new(perlin, TIME)).is_some());
    let gear = add(&mut app, "ratiogear");
    app.apply(Command::SetOption {
        node: perlin,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(gear, "cycles"),
        to: PortRef::new(perlin, TIME),
    })
    .unwrap();
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(perlin, "color"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let shader = supersilvia::compile::wgsl::build(app.graph(), out).unwrap();
    assert!(
        shader
            .body
            .contains(&format!("u.u_count_ratiogear{gear}_cycles")),
        "{}",
        shader.body
    );
    assert!(
        !shader
            .uniforms
            .contains_key(format!("u_count_perlin{perlin}_clock").as_str())
    );
}

/// **Shaky Cam has a Time per axis.** Unplugged, X and Y both read ambient time, each under its
/// own key, so the shake is what it was; a gear in Y's Time drives Y alone and leaves X on
/// ambient time, and the other way round.
#[test]
fn shaky_cam_has_a_time_per_axis() {
    let mut app = App::headless();
    let shaky = add(&mut app, "shakycam");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(shaky, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.transport(Transport::Seek(12.5));
    app.tick(FRAME);
    let x = app.count(PortRef::new(shaky, TIME));
    let y = app.count(PortRef::new(shaky, nodes::TIME_Y));
    assert!(x.is_some(), "X reads ambient time");
    assert_eq!(x, y, "unplugged, Y reads the same ambient time as X");
    let body = |app: &App| supersilvia::compile::wgsl::build(app.graph(), out).unwrap();
    let shader = body(&app);
    for key in [TIME, nodes::TIME_Y] {
        assert!(
            shader
                .uniforms
                .contains_key(format!("u_count_shakycam{shaky}_{key}").as_str()),
            "{key} is the published ambient reading"
        );
    }

    let gear = add(&mut app, "ratiogear");
    app.apply(Command::SetOption {
        node: shaky,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(gear, "cycles"),
        to: PortRef::new(shaky, nodes::TIME_Y),
    })
    .unwrap();
    let shader = body(&app);
    assert!(
        shader
            .body
            .contains(&format!("u.u_count_ratiogear{gear}_cycles")),
        "Y runs on its own gear"
    );
    assert!(
        shader
            .uniforms
            .contains_key(format!("u_count_shakycam{shaky}_{TIME}").as_str()),
        "and X stays on ambient time"
    );
    assert!(
        !shader
            .uniforms
            .contains_key(format!("u_count_shakycam{shaky}_{}", nodes::TIME_Y).as_str()),
        "Y reads the gear, not ambient time"
    );
    let def = nodes::find("shakycam").unwrap();
    for key in [nodes::timing::OFFSET, nodes::timing::OFFSET_Y] {
        assert!(def.input(key).is_some(), "an Offset per axis: {key}");
    }
}

/// **Repeat and the tunnel's depth wrap bring Time round**: a Perlin at half a cell a
/// second, repeating every four, reads 0.5 at nine seconds where Never reads 4.5; a tunnel
/// whose depth mirrors reads modulo 64, and one whose depth does not wrap goes on.
#[test]
fn repeat_and_the_tunnels_wrap_bring_time_round() {
    let mut app = App::headless();
    let never = add(&mut app, "perlin");
    let four = add(&mut app, "perlin");
    option(&mut app, four, "repeat", "4");
    let mirror = add(&mut app, "tunnel3d");
    let open = add(&mut app, "tunnel3d");
    option(&mut app, open, "wrap", "none");
    app.transport(Transport::Seek(9.0));
    app.tick_at(9.0);
    assert!((time(&app, never) - 4.5).abs() < 1e-5);
    assert!((time(&app, four) - 0.5).abs() < 1e-5);
    app.transport(Transport::Seek(140.0));
    app.tick_at(140.0);
    assert!((time(&app, mirror) - 6.0).abs() < 1e-4, "70 is 6 past 64");
    assert!((time(&app, open) - 70.0).abs() < 1e-4);
}

/// **Repeat is a rebuild**: the line through three dimensions becomes the circle through
/// four.
#[test]
fn repeat_rebuilds_a_noise_on_its_circle() {
    let mut app = App::headless();
    let perlin = add(&mut app, "perlin");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(perlin, "color"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let body = |app: &App| {
        supersilvia::compile::wgsl::build(app.graph(), out)
            .unwrap()
            .body
    };
    let line = body(&app);
    assert!(line.contains("cnoise3(vec3f(") && !line.contains("loopCircle(time_repeat("));
    option(&mut app, perlin, "repeat", "8");
    let circle = body(&app);
    assert!(
        circle.contains("cnoise4(vec4f(") && circle.contains("loopCircle(time_repeat("),
        "{circle}"
    );
}
