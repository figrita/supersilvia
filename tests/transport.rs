// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the transport, and the gears that count it.
//!
//! A playhead over the one clock that plays, pauses and seeks: ambient time. What is held here
//! is what a person would see: pause holds, a seek lands every gear where the playhead puts it
//! and fires nothing on the way, a tab reopened after a minute is in phase
//! with one that stayed open, a render is the same twice and exact at any frame rate, and a
//! project opens playing at zero. See `proposals/time.md` and `docs/cpu.md`.

use emath::Pos2;
use supersilvia::clock::{Stepper, Warmup};
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::transport::{self, Command as Transport};
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

fn add_on(app: &mut App, slug: &'static str, workspace: WorkspaceId) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    add_on(app, slug, workspace)
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap();
}

fn read(app: &App, node: NodeId, key: &'static str) -> f32 {
    app.uniform(PortRef::new(node, key))
        .unwrap_or_else(|| panic!("{key} is published"))
}

fn ticks(app: &mut App, n: u32) {
    for _ in 0..n {
        app.tick(FRAME);
    }
}

/// A Ratio Gear at `p : 1` on ambient seconds.
fn gear(app: &mut App, p: f32) -> NodeId {
    let phase = add(app, "ratiogear");
    set(app, phase, "p", p);
    phase
}

/// Pause stops the advance: a gear holds, and picks up from there on play.
#[test]
fn pause_holds() {
    let mut app = App::headless();
    let phase = gear(&mut app, 1.0);
    ticks(&mut app, 30);
    let p = read(&app, phase, "cycles");
    assert!(p > 0.4, "it ran: {p}");

    app.transport(Transport::Pause);
    ticks(&mut app, 120);
    assert_eq!(read(&app, phase, "cycles"), p, "paused, a gear holds");
    assert!(!app.transport_state().playing);

    app.transport(Transport::Play);
    ticks(&mut app, 60);
    assert!(
        (read(&app, phase, "cycles") - (p + 1.0)).abs() < 1e-3,
        "a second of play, a cycle more: {}",
        read(&app, phase, "cycles")
    );
}

/// A seek moves a gear by its ratio times the jump, landing where it would have been had the
/// show played there.
#[test]
fn a_seek_moves_a_gear_by_its_ratio_times_the_jump() {
    let mut app = App::headless();
    let phase = gear(&mut app, 2.0);
    ticks(&mut app, 10);
    let p = read(&app, phase, "cycles");
    let before = app.transport_state().playhead;

    app.transport(Transport::Seek(before + 10.0));
    app.tick(FRAME);
    let moved = app.transport_state().playhead - before;
    assert!((moved - (10.0 + f64::from(FRAME))).abs() < 1e-6, "{moved}");
    assert!(
        (f64::from(read(&app, phase, "cycles") - p) - 2.0 * moved).abs() < 1e-3,
        "rate 2 over the jump: {} from {p}",
        read(&app, phase, "cycles")
    );
}

/// A jump fires nothing on the way: a seek over forty beats is not forty beats. It lands on
/// the forty-first, a whole cycle, which is that one beat.
#[test]
fn a_jump_fires_no_crossings() {
    let mut app = App::headless();
    let clock = add(&mut app, "mastergear");
    set(&mut app, clock, "length", 0.5);
    let trigger = PortRef::new(clock, "trigger");
    ticks(&mut app, 5);

    app.transport(Transport::Seek(20.0));
    app.tick(FRAME);
    assert_eq!(
        app.edges(trigger).iter().filter(|e| e.is_down()).count(),
        1,
        "twenty seconds at 120 bpm skipped, and only the beat it landed on fired: {:?}",
        app.edges(trigger)
    );

    // Played through, the same distance fires every beat on the way, each one down.
    let mut downs = 0;
    for _ in 0..(20 * 60) {
        app.tick(FRAME);
        downs += app.edges(trigger).iter().filter(|e| e.is_down()).count();
    }
    assert_eq!(downs, 40, "forty beats in twenty seconds, played");
}

/// A workspace closed for a minute and reopened finds its gears where a minute puts them: in
/// phase with the same gears on a tab that stayed open.
#[test]
fn a_tab_closed_for_a_minute_reopens_in_phase_with_one_left_open() {
    let mut app = App::headless();
    let open = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Closed".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let closed = app.graph().workspaces().last().unwrap().id;
    app.open_workspace(closed);

    let mut pairs = Vec::new();
    for (slug, settings) in [
        ("ratiogear", &[("p", 7.0_f32), ("q", 10.0)][..]),
        ("mastergear", &[("length", 0.75)][..]),
    ] {
        let a = add_on(&mut app, slug, open);
        let b = add_on(&mut app, slug, closed);
        for &(key, value) in settings {
            set(&mut app, a, key, value);
            set(&mut app, b, key, value);
        }
        let out = "cycles";
        pairs.push((slug, a, b, out));
    }
    ticks(&mut app, 30);
    for &(slug, a, b, out) in &pairs {
        assert_eq!(read(&app, a, out), read(&app, b, out), "{slug}: together");
    }

    app.close_workspace(closed);
    ticks(&mut app, 60 * 60);
    let (_, _, b, out) = pairs[0];
    let asleep = read(&app, b, out);
    app.tick(FRAME);
    assert_eq!(read(&app, b, out), asleep, "closed, it did not tick");

    app.open_workspace(closed);
    app.tick(FRAME);
    for &(slug, a, b, out) in &pairs {
        let (x, y) = (read(&app, a, out), read(&app, b, out));
        assert!(
            (x - y).abs() < 1e-3,
            "{slug}: reopened after a minute, in phase with the open one: {x} and {y}"
        );
    }
}

/// A ÷4 Ratio Gear on a closed tab, counting a Master Gear on an open one, wakes where a
/// twin of it that stayed open has got to: a minute and a half on, the same quarter of its
/// bar, and not a quarter of where the master happens to be in its own cycle.
#[test]
fn a_ratio_gear_on_a_closed_tab_reopens_in_phase_with_its_twin() {
    let mut app = App::headless();
    let open = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Closed".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let closed = app.graph().workspaces().last().unwrap().id;
    app.open_workspace(closed);

    let clock = add_on(&mut app, "mastergear", open);
    set(&mut app, clock, "length", 1.0);
    let twins = [open, closed].map(|ws| {
        let gear = add_on(&mut app, "ratiogear", ws);
        set(&mut app, gear, "q", 4.0);
        app.apply(Command::Connect {
            from: PortRef::new(clock, "cycles"),
            to: PortRef::new(gear, "clock"),
        })
        .unwrap();
        gear
    });
    ticks(&mut app, 30);
    app.close_workspace(closed);
    ticks(&mut app, 61 * 60);
    app.open_workspace(closed);
    app.tick(FRAME);
    let [a, b] = twins.map(|gear| read(&app, gear, "wrapped"));
    assert!(
        (a - b).abs() < 1e-3,
        "reopened, the ÷4 is where its open twin is: {b} and {a}"
    );
}

/// A stall is advance: a gear lands where the clock says, and a stateful node takes at most
/// `MAX_DT` of it.
#[test]
fn a_stall_is_caught_up_by_a_gear_and_clamped_for_a_slew() {
    let mut app = App::headless();
    let phase = gear(&mut app, 1.0);
    let slew = add(&mut app, "slew");
    set(&mut app, slew, "rise", 1.0);
    ticks(&mut app, 2);
    set(&mut app, slew, "input", 10.0);
    app.tick(FRAME);
    let (p, s) = (read(&app, phase, "cycles"), read(&app, slew, "output"));
    app.tick(0.3);
    assert!(
        (read(&app, phase, "cycles") - p - 0.3).abs() < 1e-4,
        "the whole stall"
    );
    assert!(
        (read(&app, slew, "output") - s - transport::MAX_DT).abs() < 1e-4,
        "a slew steps MAX_DT of it: {} from {s}",
        read(&app, slew, "output")
    );
}

/// Run a render's time over a fresh app: a seek to the first frame, then the transport
/// driven to each, and what the nodes published on the last.
fn render(fps: f64, seconds: f64, warmup: Warmup) -> Vec<f32> {
    let mut app = App::headless();
    let phase = add(&mut app, "ratiogear");
    set(&mut app, phase, "p", 13.0);
    set(&mut app, phase, "q", 10.0);
    let perlin = add(&mut app, "perlin");
    let time = add(&mut app, "time");
    let osc = add(&mut app, "oscillator");
    let frames = (seconds * fps).round() as u32 + 1;
    let stepper = Stepper::new(fps, frames, warmup);
    app.transport(Transport::Seek(stepper.time_of(0)));
    for i in 0..stepper.len() {
        app.tick_at(stepper.time_of(i));
    }
    vec![
        read(&app, phase, "cycles"),
        app.count(PortRef::new(perlin, "clock")).unwrap() as f32,
        read(&app, time, "seconds"),
        read(&app, osc, "output"),
    ]
}

/// A render is the same twice, to the bit, and a constant rate is exact at any frame rate.
#[test]
fn a_render_is_identical_twice_and_exact_at_any_fps() {
    let once = render(60.0, 2.0, Warmup::Run(30));
    assert_eq!(once, render(60.0, 2.0, Warmup::Run(30)), "the same twice");

    for fps in [24.0, 30.0, 60.0, 144.0] {
        let got = render(fps, 2.0, Warmup::Black);
        assert!((got[0] - 2.6).abs() < 1e-4, "{fps} fps: phase {}", got[0]);
        assert!((got[1] - 1.0).abs() < 1e-4, "{fps} fps: perlin {}", got[1]);
        assert!((got[2] - 2.0).abs() < 1e-4, "{fps} fps: time {}", got[2]);
    }
    // Frame zero is every gear's and every generator's zero: after a warm-up, a render's
    // first kept frame has them and the playhead at zero.
    let got = render(30.0, 0.0, Warmup::Run(30));
    assert!(got[1].abs() < 1e-5, "{}", got[1]);
    assert!(got[0].abs() < 1e-5, "{}", got[0]);
    assert!(got[2].abs() < 1e-5, "{}", got[2]);
}

/// **Nothing of the transport is saved**: a project's file says nothing of it, and a project
/// opened into an app that was paused away from zero opens playing, at zero.
#[test]
fn a_project_opens_playing_at_zero() {
    let mut app = App::headless();
    app.save_project().unwrap();
    let root = app.project().root().to_path_buf();
    let manifest = root.join(supersilvia::project::MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(json.get("transport").is_none(), "nothing of it is written");

    let mut back = App::headless();
    ticks(&mut back, 30);
    back.transport(Transport::Pause);
    back.open_project(root.clone());
    back.tick(FRAME);
    let state = back.transport_state();
    assert!(state.playing, "playing");
    assert!(
        state.playhead >= 0.0 && state.playhead < 0.05,
        "at zero: {}",
        state.playhead
    );
    std::fs::remove_dir_all(&root).ok();
}
