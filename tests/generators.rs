// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the generators' Time — ambient time at each node's own rate, or a gear's Cycles in
//! its place, with Offset added and nothing integrated.
//!
//! What a person would see: a node that moves with time is where the playhead puts it,
//! whatever the path there — played, sought, paused and played again — because it keeps no
//! state; a gear cabled into Time replaces the ambient reading; Repeat and the tunnel's depth
//! wrap bring the reading round at the picture's own period; and Repeat is a rebuild. The
//! pictures themselves are `tests/gpu_nodes.rs`' and `tests/gpu_app.rs`'. See
//! `proposals/time.md` and `docs/nodes.md#time-and-offset`.

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
    match (n.def.ambient.unwrap().period)(n) {
        Some(period) => whole.rem_euclid(period) + fraction,
        None => whole + fraction,
    }
}

/// Every node that moves with time and draws.
fn ambient_slugs() -> Vec<&'static str> {
    nodes::REGISTRY
        .iter()
        .filter(|d| d.ambient.is_some() && d.cpu.is_none())
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
        let ambient = def.ambient.unwrap();
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

/// **A gear in Time replaces the ambient reading**: the shader reads the gear's Cycles, and
/// no ambient reading at all.
#[test]
fn a_gear_in_time_replaces_the_ambient_reading() {
    let mut app = App::headless();
    let perlin = add(&mut app, "perlin");
    app.tick(FRAME);
    assert!(app.count(PortRef::new(perlin, TIME)).is_some());
    let gear = add(&mut app, "ratiogear");
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
    for key in [nodes::phasor::OFFSET, nodes::phasor::OFFSET_Y] {
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
    assert!(line.contains("cnoise3(vec3f(") && !line.contains("loopCircle(("));
    option(&mut app, perlin, "repeat", "8");
    let circle = body(&app);
    assert!(
        circle.contains("cnoise4(vec4f(") && circle.contains("loopCircle(("),
        "{circle}"
    );
}
