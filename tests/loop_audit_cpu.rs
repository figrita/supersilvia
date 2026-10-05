// SPDX-License-Identifier: AGPL-3.0-or-later

//! The loop audit of the time-driven CPU nodes and the gears: whether what a node or a chain
//! says about when it comes back is true of what it publishes. Two properties, through a
//! headless `App`, so what is measured is what a user gets:
//!
//! - **No false claim.** Where a node's `Timing::period`, `nodes::chain::closes_alone` or
//!   `nodes::chain::master_length` says a loop of `P` seconds closes, every output at `t + P`
//!   is what it was at `t` — numbers to within an `f32` hair, events the same edges at the same
//!   moments — for `t` small, large and non-integer, and with an Offset.
//! - **No early loop.** It does not come back sooner: at `P / k` for every prime `k` dividing
//!   the claim (a sequence's least period divides every period it has, so if none of those is
//!   a period the claim is the least one).
//!
//! A stateless output is sampled by a paused seek to `t` and one tick. A sequencer, a gear's
//! Trigger, a Clock Divider — anything that keeps state — is played through without a seek, its
//! events placed in absolute time (`frame start + Event::at`) and two windows a period apart
//! compared.
//!
//! Each test gathers every setting that breaks into one list and fails with it, so a failure
//! reads as the set of settings that lie. Cases that fail on purpose are real failures, left
//! in as the regression the fix has to turn green.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef, Value};
use supersilvia::nodes::{chain, timing};
use supersilvia::transport::Command as Transport;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;
const FPS: f64 = 60.0;

/// The playheads a stateless output is sampled at: inside the first second, ordinary, an
/// hour, a day, a thousand hours and three years in, none of them on a whole anything.
const TIMES: [f64; 10] = [
    0.0371,
    0.5113,
    1.9031,
    7.7707,
    123.4561,
    3_600.301_1,
    86_400.170_3,
    1_000_003.917,
    3_600_000.290_3,
    100_000_000.331_7,
];

/// How far apart two `f32` readings of one output may be and still be the same reading.
const TOL: f32 = 2e-4;

// ------------------------------------------------------------------------------ helpers

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap_or_else(|e| panic!("{key} = {value}: {e:?}"));
}

fn choose(app: &mut App, node: NodeId, key: &'static str, value: &str) {
    app.apply(Command::SetOption {
        node,
        key,
        value: value.to_string(),
    })
    .unwrap();
}

/// Loop mode, with nothing in Time: ambient time at the node's pace.
fn ambient(app: &mut App, node: NodeId) {
    choose(app, node, timing::MODE.key, timing::LOOP);
}

fn connect(app: &mut App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    if timing::is_time(to.1)
        && app
            .graph()
            .get(to.0)
            .is_some_and(|n| n.def.timing.is_some())
    {
        ambient(app, to.0);
    }
    app.apply(Command::Connect {
        from: PortRef::new(from.0, from.1),
        to: PortRef::new(to.0, to.1),
    })
    .unwrap();
}

/// A Master Gear `length` seconds long.
fn master(app: &mut App, length: f32) -> NodeId {
    let id = add(app, "mastergear");
    set(app, id, "length", length);
    id
}

/// A Ratio Gear at Teeth `p : q` on `from`'s Cycles.
fn teeth_on(app: &mut App, from: NodeId, p: f32, q: f32) -> NodeId {
    let id = add(app, "ratiogear");
    set(app, id, "p", p);
    set(app, id, "q", q);
    connect(app, (from, "cycles"), (id, "clock"));
    id
}

/// One output, by a paused seek to `t` and one tick.
fn sample(app: &mut App, port: PortRef, t: f64) -> f32 {
    app.transport(Transport::Pause);
    app.transport(Transport::Seek(t));
    app.tick(FRAME);
    app.uniform(port)
        .unwrap_or_else(|| panic!("{port:?} is published"))
}

/// The seconds a node on its own clock says it comes back after: its period over its own
/// rate (`chain::own_rate`), which `chain::closes_alone` must agree closes.
fn claimed_alone(app: &App, node: NodeId) -> f64 {
    let g = app.graph();
    let n = g.get(node).unwrap();
    let t = n.def.timing.unwrap();
    let rate = chain::own_rate(g, node, timing::Axis::X).expect("on its own clock");
    let period = (t.period)(n).expect("a node that says it repeats");
    let seconds = period / rate.abs();
    assert_eq!(
        chain::closes_alone(g, node, seconds),
        Some(true),
        "{} says it closes over its own period",
        n.def.slug
    );
    seconds
}

/// The seconds a Master Gear's caption says a loop is.
fn claimed_master(app: &App, master: NodeId) -> f64 {
    chain::master_length(app.graph(), master).unwrap_or_else(|| {
        panic!(
            "the caption says it will not close: {}",
            chain::caption(app.graph(), master)
        )
    })
}

/// The primes up to 61 dividing `n`, for the early-loop candidates `P / k`.
fn primes_of(n: u64) -> Vec<u64> {
    [
        2u64, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61,
    ]
    .into_iter()
    .filter(|p| n.is_multiple_of(*p))
    .collect()
}

/// Every reason a stateless output at `port` breaks a claim of `period` seconds over the
/// samples: a `t` where `t + period` reads differently, and a `period / k` it comes back at.
/// `early` lists the `k` to try.
fn audit_numbers(
    app: &mut App,
    port: PortRef,
    period: f64,
    early: &[u64],
    label: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for t in TIMES {
        let a = sample(app, port, t);
        let b = sample(app, port, t + period);
        if (a - b).abs() > TOL {
            out.push(format!(
                "{label}: FALSE at t={t}: {a} then {b} a period ({period:.6} s) on"
            ));
        }
    }
    // A shift that leaves the output alone at every one of a few dozen points spread
    // across a period, here and a day in, is a period.
    let spread: Vec<f64> = [2.3, 86_400.7]
        .iter()
        .flat_map(|base| (0..29).map(move |i| base + period * (f64::from(i) + 0.37) / 29.0))
        .collect();
    for &k in early {
        let q = period / k as f64;
        let same = spread.iter().all(|&t| {
            let a = sample(app, port, t);
            let b = sample(app, port, t + q);
            (a - b).abs() <= TOL
        });
        if same {
            out.push(format!(
                "{label}: EARLY, comes back at P/{k} = {q:.6} s, not {period:.6} s"
            ));
        }
    }
    out
}

/// One edge, at its absolute time on the playhead, on output `lane` of the ports watched.
#[derive(Debug, Clone, Copy)]
struct Fired {
    at: f64,
    lane: usize,
    down: bool,
}

/// Play from `t0` for `seconds` at `FPS`, every node born where `t0` puts it, as a render
/// does, and gather every edge on `ports`, placed at `frame start + at`.
fn play(app: &mut App, ports: &[PortRef], t0: f64, seconds: f64) -> Vec<Fired> {
    app.transport(Transport::Play);
    app.reset_cpu();
    app.transport(Transport::Seek(t0));
    let frames = (seconds * FPS).ceil() as u64;
    let mut out = Vec::new();
    let mut prev = t0;
    for n in 0..=frames {
        let t = t0 + n as f64 / FPS;
        app.tick_at(t);
        for (lane, port) in ports.iter().enumerate() {
            for e in app.edges(*port) {
                out.push(Fired {
                    at: prev + f64::from(e.at),
                    lane,
                    down: e.is_down(),
                });
            }
        }
        prev = t;
    }
    out
}

/// The edges in `[from, from + len)`, relative to `from`.
fn window(events: &[Fired], from: f64, len: f64) -> Vec<Fired> {
    events
        .iter()
        .filter(|e| e.at >= from && e.at < from + len)
        .map(|e| Fired {
            at: e.at - from,
            ..*e
        })
        .collect()
}

fn same_events(a: &[Fired], b: &[Fired]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.lane == y.lane && x.down == y.down && (x.at - y.at).abs() < 1e-5)
}

/// Every reason the events on `ports` break a claim of `period` seconds: played from `t0`
/// for three periods, the second and third windows compared, and the second against itself
/// `period / k` on for each `k` of `early`. A window starts `skew` into the second period so
/// no edge sits on its border.
fn audit_events(
    app: &mut App,
    ports: &[PortRef],
    t0: f64,
    period: f64,
    early: &[u64],
    label: &str,
) -> Vec<String> {
    let skew = 0.012_345;
    let events = play(app, ports, t0, 3.0 * period + 0.1);
    let mut out = Vec::new();
    let a = window(&events, t0 + period + skew, period);
    let b = window(&events, t0 + 2.0 * period + skew, period);
    if a.is_empty() && b.is_empty() {
        out.push(format!(
            "{label}: nothing fired in two periods, nothing proved"
        ));
        return out;
    }
    if !same_events(&a, &b) {
        let first = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x.lane != y.lane || x.down != y.down || (x.at - y.at).abs() >= 1e-5)
            .unwrap_or(a.len().min(b.len()));
        out.push(format!(
            "{label}: FALSE, {} edges then {} a period ({period:.4} s) on; first difference \
             at edge {first}: {:?} vs {:?}",
            a.len(),
            b.len(),
            a.get(first),
            b.get(first)
        ));
    }
    for &k in early {
        let q = period / k as f64;
        let c = window(&events, t0 + period + skew + q, period);
        if same_events(&a, &c) {
            out.push(format!(
                "{label}: EARLY, comes back at P/{k} = {q:.4} s, not {period:.4} s"
            ));
        }
    }
    out
}

fn lanes_of(node: NodeId) -> [PortRef; 4] {
    ["lane1", "lane2", "lane3", "lane4"].map(|k| PortRef::new(node, k))
}

fn check(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} broken:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// --------------------------------------------------------------------------- oscillator

const PERIODIC_WAVES: [&str; 6] = ["sine", "cosine", "triangle", "square", "sawtooth", "pulse"];

/// **Every periodic waveform comes back every wave, and not before**, on ambient time in
/// Loop mode, running free at Speeds from a hundredth to the knob's ends either way, and on a
/// Master Gear's Cycles, with Offsets inside and at the edge of its range and Level and
/// Amplitude moved.
#[test]
fn oscillator_periodic_waveforms_come_back_every_wave_and_not_before() {
    let mut failures = Vec::new();
    for wave in PERIODIC_WAVES {
        for offset in [0.0f32, 0.37, -0.81, 1.0] {
            // Ambient, in Loop mode.
            let mut app = App::headless();
            let osc = add(&mut app, "oscillator");
            choose(&mut app, osc, "waveform", wave);
            set(&mut app, osc, "phaseOffset", offset);
            set(&mut app, osc, "amplitude", 2.5);
            set(&mut app, osc, "offset", -1.25);
            ambient(&mut app, osc);
            let p = claimed_alone(&app, osc);
            failures.extend(audit_numbers(
                &mut app,
                PortRef::new(osc, "output"),
                p,
                &[2, 3, 5, 7],
                &format!("{wave} ambient offset {offset}"),
            ));
            // Free, at Speeds across the knob.
            for speed in [1.0f32, 0.3, -2.5, 4.0, -4.0, 0.01, 0.07] {
                let mut app = App::headless();
                let osc = add(&mut app, "oscillator");
                choose(&mut app, osc, "waveform", wave);
                set(&mut app, osc, "phaseOffset", offset);
                set(&mut app, osc, "speed", speed);
                let p = claimed_alone(&app, osc);
                failures.extend(audit_numbers(
                    &mut app,
                    PortRef::new(osc, "output"),
                    p,
                    &[2, 3],
                    &format!("{wave} free speed {speed} offset {offset}"),
                ));
            }
        }
        // On a Master Gear's Cycles, whose caption says a loop of one cycle.
        for length in [2.0f32, 0.37, 1.7, 13.0] {
            let mut app = App::headless();
            let m = master(&mut app, length);
            let osc = add(&mut app, "oscillator");
            choose(&mut app, osc, "waveform", wave);
            set(&mut app, osc, "phaseOffset", 0.29);
            connect(&mut app, (m, "cycles"), (osc, timing::TIME));
            let p = claimed_master(&app, m);
            failures.extend(audit_numbers(
                &mut app,
                PortRef::new(osc, "output"),
                p,
                &[2, 3],
                &format!("{wave} on a {length} s master"),
            ));
        }
    }
    check(&failures);
}

/// **Noise says it never comes back, and does not**: a fresh value each wave keyed by
/// `floor(Time + Offset)` repeats only after 2³⁰ waves, which no loop is, so its period is none
/// — the loop meter open, the Offset knob a wave either way — `closes_alone` says no length
/// closes it, ambient or free, and on a Master Gear the caption says it will not close. Nor
/// does it come back at a whole number of waves a loop could be.
#[test]
fn oscillator_noise_comes_back_when_it_says() {
    let mut failures = Vec::new();
    let mut waves = |app: &mut App, osc: NodeId, rate: f64, label: &str| {
        let n = app.graph().get(osc).unwrap();
        if (n.def.timing.unwrap().period)(n).is_some() {
            failures.push(format!("{label}: claims a period"));
        }
        for k in [1.0, 2.0, 3.0, 4.0, 7.0] {
            let seconds = k / rate;
            if chain::closes_alone(app.graph(), osc, seconds) != Some(false) {
                failures.push(format!("{label}: closes_alone({seconds}) does not say no"));
            }
            let port = PortRef::new(osc, "output");
            let same = TIMES[..6]
                .iter()
                .all(|&t| (sample(app, port, t) - sample(app, port, t + seconds)).abs() <= TOL);
            if same {
                failures.push(format!("{label}: comes back after {k} waves"));
            }
        }
    };
    for offset in [0.0f32, 0.37] {
        let mut app = App::headless();
        let osc = add(&mut app, "oscillator");
        choose(&mut app, osc, "waveform", "noise");
        set(&mut app, osc, "phaseOffset", offset);
        ambient(&mut app, osc);
        waves(
            &mut app,
            osc,
            1.0,
            &format!("noise ambient offset {offset}"),
        );
        let mut app = App::headless();
        let osc = add(&mut app, "oscillator");
        choose(&mut app, osc, "waveform", "noise");
        set(&mut app, osc, "phaseOffset", offset);
        set(&mut app, osc, "speed", 0.5);
        waves(
            &mut app,
            osc,
            0.5,
            &format!("noise free speed 0.5 offset {offset}"),
        );
    }
    let mut app = App::headless();
    let m = master(&mut app, 1.5);
    let osc = add(&mut app, "oscillator");
    choose(&mut app, osc, "waveform", "noise");
    connect(&mut app, (m, "cycles"), (osc, timing::TIME));
    let cap = chain::caption(app.graph(), m);
    if chain::master_length(app.graph(), m).is_some() || !cap.contains("will not close") {
        failures.push(format!("noise on a master: {cap}"));
    }
    check(&failures);
}

// --------------------------------------------------------------------------- sequencers

/// A Euclidean Rhythm's four lanes, `(steps, pulses, rotation)` each.
type Lanes = [(f32, f32, f32); 4];

/// Set a Euclidean Rhythm's four lanes: `(steps, pulses, rotation)` each.
fn lanes(app: &mut App, node: NodeId, lanes: Lanes) {
    use supersilvia::nodes::euclideanrhythm::{PULSES_KEYS, ROTATION_KEYS, STEPS_KEYS};
    for (i, (steps, pulses, rotation)) in lanes.into_iter().enumerate() {
        set(app, node, STEPS_KEYS[i], steps);
        set(app, node, PULSES_KEYS[i], pulses);
        set(app, node, ROTATION_KEYS[i], rotation);
    }
}

/// A Euclidean Rhythm in Loop mode on ambient time, shaped by `l`, its Offset at `offset`.
fn euclid(l: Lanes, offset: f32) -> (App, NodeId) {
    let mut app = App::headless();
    let e = add(&mut app, "euclideanrhythm");
    lanes(&mut app, e, l);
    set(&mut app, e, "phaseOffset", offset);
    ambient(&mut app, e);
    (app, e)
}

/// A claim in seconds as whole bars, two seconds a bar on ambient time.
fn bars_in(seconds: f64, seconds_a_bar: f64) -> u64 {
    let bars = seconds / seconds_a_bar;
    assert!((bars - bars.round()).abs() < 1e-9, "{bars} bars");
    bars.round() as u64
}

/// A claim in seconds as whole steps, sixteen a bar: a rhythm whose lanes repeat inside a bar
/// comes back in a fraction of one.
fn steps_in(seconds: f64, seconds_a_bar: f64) -> u64 {
    let steps = 16.0 * seconds / seconds_a_bar;
    assert!((steps - steps.round()).abs() < 1e-9, "{steps} steps");
    steps.round() as u64
}

/// **A Euclidean Rhythm whose lanes are truly coprime comes back when it says**: lanes whose
/// figures do not repeat inside their own length, ambient, free at Speeds either way, with
/// Offsets, a day and a thousand hours in. Its events — every lane's downs and ups — are the
/// same in every period, and not before at any whole bar.
#[test]
fn euclidean_rhythm_with_honest_lanes_comes_back_when_it_says() {
    let configs: [Lanes; 4] = [
        // The default: E(4,16) E(3,16) E(5,16) E(2,16), a bar.
        [
            (16.0, 4.0, 0.0),
            (16.0, 3.0, 0.0),
            (16.0, 5.0, 0.0),
            (16.0, 2.0, 0.0),
        ],
        // 5, 3 and 7 against 16: 105 bars, every figure its own length.
        [
            (16.0, 3.0, 0.0),
            (5.0, 2.0, 1.0),
            (3.0, 1.0, -1.0),
            (7.0, 3.0, 2.0),
        ],
        // Rotations at the knob's ends, lanes of 9 and 11.
        [
            (9.0, 4.0, 8.0),
            (11.0, 5.0, -8.0),
            (16.0, 7.0, 3.0),
            (16.0, 1.0, -2.0),
        ],
        // 64 steps against 16: four bars.
        [
            (64.0, 13.0, 5.0),
            (16.0, 3.0, 0.0),
            (16.0, 3.0, 0.0),
            (16.0, 3.0, 0.0),
        ],
    ];
    let mut failures = Vec::new();
    for (i, l) in configs.iter().enumerate() {
        for (offset, t0) in [(0.0f32, 0.0), (0.37, 86_400.0), (-0.81, 3_600_000.0)] {
            let (mut app, e) = euclid(*l, offset);
            let p = claimed_alone(&app, e);
            let bars = bars_in(p, 2.0);
            failures.extend(audit_events(
                &mut app,
                &lanes_of(e),
                t0,
                p,
                &primes_of(bars),
                &format!("euclid config {i} ({bars} bars) offset {offset} t0 {t0}"),
            ));
        }
        // Free, forwards and backwards, on the short figures: a long one backwards is the
        // same code.
        if i == 0 || i == 3 {
            for speed in [1.0f32, -1.0, 2.5, -0.37] {
                let mut app = App::headless();
                let e = add(&mut app, "euclideanrhythm");
                lanes(&mut app, e, *l);
                set(&mut app, e, "speed", speed);
                let p = claimed_alone(&app, e);
                let bars = bars_in(p, 2.0 / f64::from(speed.abs()));
                failures.extend(audit_events(
                    &mut app,
                    &lanes_of(e),
                    1_000.0,
                    p,
                    &primes_of(bars),
                    &format!("euclid config {i} free speed {speed}"),
                ));
            }
        }
    }
    // On a 3 : 2 gear under a master a quarter second long: lanes of 5 against 16 are five
    // bars, a bar is two thirds of a cycle, so the caption asks for ten cycles.
    let mut app = App::headless();
    let m = master(&mut app, 0.25);
    let g = teeth_on(&mut app, m, 3.0, 2.0);
    let e = add(&mut app, "euclideanrhythm");
    lanes(
        &mut app,
        e,
        [
            (16.0, 3.0, 0.0),
            (5.0, 2.0, 0.0),
            (16.0, 5.0, 0.0),
            (16.0, 1.0, 0.0),
        ],
    );
    connect(&mut app, (g, "cycles"), (e, timing::TIME));
    let p = claimed_master(&app, m);
    let cycles = (p / 0.25).round() as u64;
    assert_eq!(cycles, 10, "{}", chain::caption(app.graph(), m));
    failures.extend(audit_events(
        &mut app,
        &lanes_of(e),
        7.0,
        p,
        &primes_of(cycles),
        "euclid lanes of 5 on a 3 : 2 gear",
    ));
    check(&failures);
}

/// **A Euclidean Rhythm does not come back before the steps it says**, where a lane's figure
/// repeats inside its own length — E(2, 32) every 16 steps, E(6, 30) every 5 — or is silent
/// (no pulses) or full (pulses at or above its steps). The fourth is the configuration
/// `chain::tests::a_sequencer_loops_when_every_lane_does` counts: forty steps, two and a half
/// bars, so the claim is read in steps and the early loops looked for at every prime of them.
#[test]
fn euclidean_rhythm_does_not_come_back_before_the_bars_it_says() {
    let configs: [(&str, Lanes); 5] = [
        (
            "E(2,32) repeats every bar",
            [
                (32.0, 2.0, 0.0),
                (16.0, 3.0, 0.0),
                (16.0, 5.0, 0.0),
                (16.0, 2.0, 0.0),
            ],
        ),
        (
            "a silent lane of 5",
            [
                (5.0, 0.0, 0.0),
                (16.0, 3.0, 0.0),
                (16.0, 5.0, 0.0),
                (16.0, 2.0, 0.0),
            ],
        ),
        (
            "a full lane of 5",
            [
                (5.0, 5.0, 0.0),
                (16.0, 3.0, 0.0),
                (16.0, 5.0, 0.0),
                (16.0, 2.0, 0.0),
            ],
        ),
        (
            "chain.rs's own 5 and 3, the 3 full",
            [
                (16.0, 4.0, 0.0),
                (5.0, 3.0, 0.0),
                (3.0, 3.0, 0.0),
                (16.0, 2.0, 0.0),
            ],
        ),
        (
            "E(6,30) repeats every 5 steps",
            [
                (30.0, 6.0, 0.0),
                (16.0, 3.0, 0.0),
                (16.0, 5.0, 0.0),
                (16.0, 2.0, 0.0),
            ],
        ),
    ];
    let mut failures = Vec::new();
    for (name, l) in configs {
        let (mut app, e) = euclid(l, 0.0);
        let p = claimed_alone(&app, e);
        let steps = steps_in(p, 2.0);
        failures.extend(audit_events(
            &mut app,
            &lanes_of(e),
            10.0,
            p,
            &primes_of(steps),
            &format!("{name}: claims {steps} steps"),
        ));
    }
    check(&failures);
}

/// The step sequencer's pattern as four strings.
fn pattern(app: &mut App, node: NodeId, lanes: [&str; 4]) {
    app.apply(Command::SetValue {
        node,
        key: supersilvia::nodes::stepsequencer::PATTERN,
        value: Value::Cells(lanes.iter().map(|s| (*s).to_string()).collect()),
    })
    .unwrap();
}

/// **A Step Sequencer comes back every bar**, a pattern that does not repeat inside one, on
/// ambient time, free either way, and on a Master Gear a bar long, with Offsets and a Gate at
/// both ends.
#[test]
fn step_sequencer_comes_back_every_bar() {
    let lit = [
        "x..x.x....x..x..",
        "....x.......x.x.",
        "xx.x..x.x...x..x",
        "..............x.",
    ];
    let mut failures = Vec::new();
    for (offset, gate, t0) in [
        (0.0f32, 0.5f32, 0.0),
        (0.37, 1.0, 86_400.0),
        (-0.81, 0.1, 3_600_000.0),
    ] {
        let mut app = App::headless();
        let s = add(&mut app, "stepsequencer");
        pattern(&mut app, s, lit);
        set(&mut app, s, "phaseOffset", offset);
        set(&mut app, s, "gateLength", gate);
        ambient(&mut app, s);
        let p = claimed_alone(&app, s);
        failures.extend(audit_events(
            &mut app,
            &lanes_of(s),
            t0,
            p,
            &[2],
            &format!("step sequencer ambient offset {offset} gate {gate} t0 {t0}"),
        ));
    }
    for speed in [1.0f32, -1.0, 3.0, -0.25] {
        let mut app = App::headless();
        let s = add(&mut app, "stepsequencer");
        pattern(&mut app, s, lit);
        set(&mut app, s, "speed", speed);
        let p = claimed_alone(&app, s);
        failures.extend(audit_events(
            &mut app,
            &lanes_of(s),
            500.0,
            p,
            &[2],
            &format!("step sequencer free speed {speed}"),
        ));
    }
    for (out, length) in [("cycles", 2.0f32), ("wrapped", 1.3)] {
        let mut app = App::headless();
        let m = master(&mut app, length);
        let s = add(&mut app, "stepsequencer");
        pattern(&mut app, s, lit);
        connect(&mut app, (m, out), (s, timing::TIME));
        let p = claimed_master(&app, m);
        failures.extend(audit_events(
            &mut app,
            &lanes_of(s),
            40.0,
            p,
            &[2],
            &format!("step sequencer on a master's {out}"),
        ));
    }
    check(&failures);
}

/// **A pattern that repeats inside a bar, on a gear slower than the bar, comes back when the
/// caption says**: four on the floor, or E(4, 16)-shaped lanes, on a ÷2 or ÷4 gear under a
/// master, against the cycles the caption asks for.
#[test]
fn a_pattern_inside_a_bar_on_a_slow_gear_comes_back_when_the_caption_says() {
    let mut failures = Vec::new();
    for (slug, divide) in [
        ("stepsequencer", 4.0f32),
        ("stepsequencer", 2.0),
        ("euclideanrhythm", 4.0),
    ] {
        let mut app = App::headless();
        let m = master(&mut app, 0.5);
        let g = teeth_on(&mut app, m, 1.0, divide);
        let s = add(&mut app, slug);
        if slug == "stepsequencer" {
            let floor = "x...x...x...x...";
            pattern(&mut app, s, [floor, "..x...x...x...x.", floor, ""]);
        } else {
            lanes(
                &mut app,
                s,
                [
                    (16.0, 4.0, 0.0),
                    (16.0, 4.0, 2.0),
                    (8.0, 2.0, 1.0),
                    (4.0, 1.0, 0.0),
                ],
            );
        }
        connect(&mut app, (g, "cycles"), (s, timing::TIME));
        let p = claimed_master(&app, m);
        let cap = chain::caption(app.graph(), m);
        let cycles = (p / 0.5).round() as u64;
        failures.extend(audit_events(
            &mut app,
            &lanes_of(s),
            3.0,
            p,
            &primes_of(cycles),
            &format!("{slug} on ÷{divide}: {cap}"),
        ));
    }
    check(&failures);
}

// -------------------------------------------------------------------------------- gears

/// **A Ratio Gear at p : q comes back in q ÷ gcd(p, q) of the master's cycles, and not
/// before**, across a sweep of Teeth — one to one, twice, 3 : 2 and 4 : 3, 7 : 5, 16 : 9 and
/// 1 : 64, ratios an `f64` cannot hold, and two typed unreduced — on a master 78 frames long:
/// its Phase and a sawtooth on its Cycles, sought to as far as three years in, and its Trigger
/// played through, against the loop the caption claims.
#[test]
fn a_ratio_gear_comes_back_in_its_denominator_and_not_before() {
    let mut failures = Vec::new();
    for (p, q) in [
        (1.0f32, 1.0f32),
        (2.0, 1.0),
        (3.0, 2.0),
        (4.0, 3.0),
        (7.0, 5.0),
        (16.0, 9.0),
        (1.0, 64.0),
        (1.0, 3.0),
        (5.0, 7.0),
        (63.0, 64.0),
        (1.0, 10.0),
        (17.0, 11.0),
        (12.0, 1.0),
        (64.0, 1.0),
        (2.0, 4.0),
        (6.0, 4.0),
    ] {
        let mut app = App::headless();
        let m = master(&mut app, 1.3);
        let g = teeth_on(&mut app, m, p, q);
        let osc = add(&mut app, "oscillator");
        choose(&mut app, osc, "waveform", "sawtooth");
        connect(&mut app, (g, "cycles"), (osc, timing::TIME));
        let period = claimed_master(&app, m);
        let cycles = (period / f64::from(1.3f32)).round() as u64;
        let label = format!("{p} : {q} ({cycles} cycles)");
        let reduced = (q as u64) / gcd(p as u64, q as u64);
        if cycles != reduced {
            failures.push(format!(
                "{label}: the caption claims {cycles}, not {reduced}"
            ));
        }
        let early = primes_of(cycles);
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(g, "wrapped"),
            period,
            &early,
            &format!("{label} Phase"),
        ));
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(osc, "output"),
            period,
            &early,
            &format!("sawtooth on {label}"),
        ));
        failures.extend(audit_events(
            &mut app,
            &[PortRef::new(g, "trigger")],
            11.0,
            period,
            &early,
            &format!("{label} Trigger"),
        ));
    }
    check(&failures);
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// **A Master Gear's Phase and Trigger come back every cycle**, at lengths from a hundredth
/// to its top; its Trigger's beats fall at the same moments every cycle.
#[test]
fn a_master_gears_phase_and_trigger_come_back_every_cycle() {
    let mut failures = Vec::new();
    for length in [2.0f32, 0.37, 1.7, 13.0, 256.0, 0.05] {
        let mut app = App::headless();
        let m = master(&mut app, length);
        let p = claimed_master(&app, m);
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(m, "wrapped"),
            p,
            &[],
            &format!("master {length} s Phase"),
        ));
        if length < 20.0 {
            failures.extend(audit_events(
                &mut app,
                &[PortRef::new(m, "trigger")],
                77.0,
                p,
                &[],
                &format!("master {length} s Trigger"),
            ));
        }
    }
    check(&failures);
}

/// **A gear's Ping-pong, a triangle over two of its cycles, comes back when the caption
/// says**: a sine on a master's Ping-pong through its Time, and a still sine with it in its
/// Offset.
#[test]
fn ping_pong_comes_back_when_the_caption_says() {
    let mut failures = Vec::new();
    for length in [1.0f32, 2.0, 0.7] {
        let mut app = App::headless();
        let m = master(&mut app, length);
        let osc = add(&mut app, "oscillator");
        connect(&mut app, (m, "pingpong"), (osc, timing::TIME));
        let p = claimed_master(&app, m);
        let cap = chain::caption(app.graph(), m);
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(osc, "output"),
            p,
            &[],
            &format!("sine on a {length} s master's Ping-pong: {cap}"),
        ));
        // And straight off the gear, into an Offset.
        let mut app = App::headless();
        let m = master(&mut app, length);
        let osc = add(&mut app, "oscillator");
        set(&mut app, osc, "speed", 0.0);
        connect(&mut app, (m, "pingpong"), (osc, timing::OFFSET));
        let p = claimed_master(&app, m);
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(osc, "output"),
            p,
            &[],
            &format!("still sine, Ping-pong in Offset, {length} s master"),
        ));
    }
    check(&failures);
}

/// **What a gear's Trigger drives comes back when the caption says**: a Step Sequencer on
/// it through Step, a Clock Divider at ÷3, and a Clock Divider at ÷3 on a 3 : 2 Ratio Gear's
/// Trigger, against what the caption asks for.
#[test]
fn what_a_trigger_drives_comes_back_when_the_caption_says() {
    let mut failures = Vec::new();

    // Into a Step Sequencer's Step.
    let mut app = App::headless();
    let m = master(&mut app, 0.25);
    let s = add(&mut app, "stepsequencer");
    pattern(&mut app, s, ["x..x.x....x..x..", "", "", ""]);
    connect(&mut app, (m, "trigger"), (s, "step"));
    let p = claimed_master(&app, m);
    let cap = chain::caption(app.graph(), m);
    failures.extend(audit_events(
        &mut app,
        &lanes_of(s),
        5.0,
        p,
        &[],
        &format!("step sequencer on a Trigger: {cap}"),
    ));

    // Into a Clock Divider.
    let mut app = App::headless();
    let m = master(&mut app, 0.5);
    let d = add(&mut app, "clockdivider");
    set(&mut app, d, "divide", 3.0);
    connect(&mut app, (m, "trigger"), (d, "input"));
    let p = claimed_master(&app, m);
    let cap = chain::caption(app.graph(), m);
    failures.extend(audit_events(
        &mut app,
        &[PortRef::new(d, "trigger")],
        5.0,
        p,
        &[],
        &format!("clock divider ÷3 on a Trigger: {cap}"),
    ));

    // A 3 : 2 gear's Trigger into a Clock Divider at ÷3: a beat every two of the master's
    // cycles.
    let mut app = App::headless();
    let m = master(&mut app, 0.5);
    let g = teeth_on(&mut app, m, 3.0, 2.0);
    let d = add(&mut app, "clockdivider");
    set(&mut app, d, "divide", 3.0);
    connect(&mut app, (g, "trigger"), (d, "input"));
    let p = claimed_master(&app, m);
    let cap = chain::caption(app.graph(), m);
    failures.extend(audit_events(
        &mut app,
        &[PortRef::new(d, "trigger")],
        5.0,
        p,
        &primes_of((p / f64::from(0.5f32)).round() as u64),
        &format!("clock divider ÷3 on a 3 : 2 gear's Trigger: {cap}"),
    ));
    check(&failures);
}

/// **A gear's Cycles in an input that is not a Time comes back when the caption says**: in a
/// Euclidean Rhythm's Offset, its lanes five bars long. In an oscillator's Amplitude it is a
/// number that grows without bound, which no loop closes, and the caption says it cannot tell
/// rather than claim one.
#[test]
fn a_count_into_an_offset_or_an_amplitude_comes_back_when_the_caption_says() {
    let mut failures = Vec::new();

    let mut app = App::headless();
    let m = master(&mut app, 0.5);
    let e = add(&mut app, "euclideanrhythm");
    lanes(
        &mut app,
        e,
        [
            (16.0, 3.0, 0.0),
            (5.0, 2.0, 0.0),
            (16.0, 5.0, 0.0),
            (16.0, 2.0, 0.0),
        ],
    );
    connect(&mut app, (m, "cycles"), (e, timing::OFFSET));
    let p = claimed_master(&app, m);
    let cap = chain::caption(app.graph(), m);
    failures.extend(audit_events(
        &mut app,
        &lanes_of(e),
        3.0,
        p,
        &[],
        &format!("euclid, Cycles in Offset: {cap}"),
    ));

    let mut app = App::headless();
    let m = master(&mut app, 0.5);
    let osc = add(&mut app, "oscillator");
    connect(&mut app, (m, "cycles"), (osc, timing::TIME));
    connect(&mut app, (m, "cycles"), (osc, "amplitude"));
    let cap = chain::caption(app.graph(), m);
    if chain::master_length(app.graph(), m).is_some() || !cap.starts_with("can't tell") {
        failures.push(format!("sine, Cycles in Amplitude: claims {cap}"));
    }
    check(&failures);
}

/// **A node on its own clock in a Master Gear's graph closes with the length its caption
/// gives** (`chain::master_length`, what `examples/loop_gifs` renders): a sine on the
/// master's Cycles whose Amplitude is an oscillator running free at Speed 0.3.
#[test]
fn a_node_on_its_own_clock_beside_a_master_closes_with_it() {
    let mut app = App::headless();
    let m = master(&mut app, 2.0);
    let osc = add(&mut app, "oscillator");
    connect(&mut app, (m, "cycles"), (osc, timing::TIME));
    let lfo = add(&mut app, "oscillator");
    set(&mut app, lfo, "speed", 0.3);
    set(&mut app, lfo, "offset", 2.0);
    connect(&mut app, (lfo, "output"), (osc, "amplitude"));
    let p = claimed_master(&app, m);
    let cap = chain::caption(app.graph(), m);
    check(&audit_numbers(
        &mut app,
        PortRef::new(osc, "output"),
        p,
        &[],
        &format!("sine with a free LFO in Amplitude: {cap}"),
    ));
}

/// **The loop arithmetic does not overflow**: Ratio Gears at ÷ every prime to 53 under one
/// master, whose loop is more cycles than a `u64` holds, and a chain of eleven 63 : 64 gears,
/// whose product's denominator is 2⁶⁶. The caption either says they do not close or counts
/// a number every denominator divides, and does not panic.
#[test]
fn the_loop_arithmetic_does_not_overflow() {
    let mut failures = Vec::new();
    let result = std::panic::catch_unwind(|| {
        let mut app = App::headless();
        let m = master(&mut app, 1.0);
        let primes = [
            2.0f32, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0, 41.0, 43.0,
            47.0, 53.0,
        ];
        for q in primes {
            teeth_on(&mut app, m, 1.0, q);
        }
        let l = chain::master_loop(app.graph(), m);
        (
            l.open.is_empty(),
            l.cycles,
            primes.iter().all(|q| l.cycles.is_multiple_of(*q as u64)),
        )
    });
    match result {
        Err(_) => failures.push("÷ primes to 53: master_loop panicked".to_string()),
        Ok((closed, cycles, divides)) if closed && !divides => failures.push(format!(
            "÷ primes to 53: says {cycles} cycles, which not every denominator divides"
        )),
        Ok(_) => {}
    }
    let result = std::panic::catch_unwind(|| {
        let mut app = App::headless();
        let m = master(&mut app, 1.0);
        let mut from = m;
        for _ in 0..11 {
            from = teeth_on(&mut app, from, 63.0, 64.0);
        }
        let l = chain::master_loop(app.graph(), m);
        (l.open.is_empty(), l.cycles)
    });
    match result {
        Err(_) => failures.push("eleven 63 : 64 gears: master_loop panicked".to_string()),
        Ok((true, cycles)) => failures.push(format!(
            "eleven 63 : 64 gears: says {cycles} cycles where 2^66 are needed"
        )),
        Ok(_) => {}
    }
    check(&failures);
}

// ------------------------------------------------------------------------ clips and GIFs

/// A GIF of three frames held 0.1, 0.2 and 0.4 s: 0.7 s a play, and no frame twice.
fn gif() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-loop-audit-cpu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("three.gif");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = image::codecs::gif::GifEncoder::new(file);
    for (rgba, ms) in [
        ([220, 20, 20, 255], 100),
        ([20, 220, 20, 255], 200),
        ([20, 20, 220, 255], 400),
    ] {
        let mut picture = image::RgbaImage::new(4, 2);
        for pixel in picture.pixels_mut() {
            *pixel = image::Rgba(rgba);
        }
        encoder
            .encode_frame(image::Frame::from_parts(
                picture,
                0,
                0,
                image::Delay::from_numer_denom_ms(ms, 1),
            ))
            .unwrap();
    }
    path
}

/// An Image/GIF node showing [`gif`], decoded.
fn gif_node(app: &mut App) -> NodeId {
    let id = add(app, "imagegif");
    let path = gif();
    let reference = app.project().import_asset(&path).unwrap();
    choose(app, id, "file", &reference);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.uniform(PortRef::new(id, "frames")).unwrap_or(0.0) < 3.0 {
        app.tick(FRAME);
        assert!(
            std::time::Instant::now() < deadline,
            "the GIF never decoded"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    id
}

/// **A GIF comes back every play, and not before**, at its own pace on ambient time, free at
/// Speeds either way, and on a master's Cycles a play a cycle — read off its Frame output.
#[test]
fn a_gif_comes_back_every_play() {
    const LENGTH: f64 = 0.7;
    let mut failures = Vec::new();
    for offset in [0.0f32, 0.43, -0.6] {
        let mut app = App::headless();
        let id = gif_node(&mut app);
        set(&mut app, id, "phaseOffset", offset);
        ambient(&mut app, id);
        let p = LENGTH;
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(id, "frame"),
            p,
            &[2, 3, 5],
            &format!("gif ambient offset {offset}"),
        ));
    }
    for speed in [1.0f32, -1.0, 2.5, -0.3] {
        let mut app = App::headless();
        let id = gif_node(&mut app);
        set(&mut app, id, "speed", speed);
        let p = LENGTH / f64::from(speed.abs());
        failures.extend(audit_numbers(
            &mut app,
            PortRef::new(id, "frame"),
            p,
            &[2, 3],
            &format!("gif free speed {speed}"),
        ));
    }
    let mut app = App::headless();
    let m = master(&mut app, 1.1);
    let id = gif_node(&mut app);
    connect(&mut app, (m, "cycles"), (id, timing::TIME));
    let p = claimed_master(&app, m);
    failures.extend(audit_numbers(
        &mut app,
        PortRef::new(id, "frame"),
        p,
        &[2, 3],
        "gif on a master's Cycles",
    ));
    check(&failures);
}

/// **A clip on its own clock closes where `chain::closes_alone` says**: a 0.7 s GIF at
/// Speed 1 over every whole second it is said to close on, and a video, whose length is not
/// in the graph, said to close over none it cannot know.
#[test]
fn a_clip_on_its_own_clock_closes_where_closes_alone_says() {
    let mut failures = Vec::new();
    let mut app = App::headless();
    let id = gif_node(&mut app);
    for seconds in [1.0, 2.0, 3.0] {
        if chain::closes_alone(app.graph(), id, seconds) == Some(true) {
            failures.extend(audit_numbers(
                &mut app,
                PortRef::new(id, "frame"),
                seconds,
                &[],
                &format!("0.7 s gif at Speed 1, closes_alone({seconds})"),
            ));
        }
    }
    // A video's length is not in the graph, so no whole second is known to close it. Read
    // off the graph and not played: an import transcodes on the hardware encoder.
    let mut app = App::headless();
    let v = add(&mut app, "video");
    choose(&mut app, v, "file", "assets/a-clip-of-any-length.mp4");
    for seconds in [1.0, 2.0, 7.0] {
        if chain::closes_alone(app.graph(), v, seconds) == Some(true) {
            failures.push(format!(
                "video at Speed 1: closes_alone({seconds}) says it closes, for a clip of \
                 any length"
            ));
        }
    }
    check(&failures);
}
