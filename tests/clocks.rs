// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the gears and the clocks. A Master Gear at a length, a Ratio Gear its parent times
//! its Teeth to the bit, the chain a loop closes on, a Master Gear's Hold, Reset and Sync, a
//! seek re-birthing them, a beat on a render's frame firing on it, the Time node reading the
//! playhead, and the oscillator on the transport. See `docs/cpu.md#gears` and
//! `proposals/time.md`.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::transport::Command as Transport;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

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
    .unwrap();
}

fn choose(app: &mut App, node: NodeId, key: &'static str, value: &str) {
    app.apply(Command::SetOption {
        node,
        key,
        value: value.to_string(),
    })
    .unwrap();
}

fn connect(app: &mut App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    // A clock cabled into a Time: the node loops on it rather than running free.
    if supersilvia::nodes::is_time(to.1)
        && app
            .graph()
            .get(to.0)
            .is_some_and(|n| n.def.timing.is_some())
    {
        app.apply(Command::SetOption {
            node: to.0,
            key: "clockMode",
            value: "loop".to_string(),
        })
        .unwrap();
    }
    app.apply(Command::Connect {
        from: PortRef::new(from.0, from.1),
        to: PortRef::new(to.0, to.1),
    })
    .unwrap();
}

fn read(app: &App, node: NodeId, key: &'static str) -> f64 {
    app.uniform(PortRef::new(node, key))
        .unwrap_or_else(|| panic!("{key} is published"))
}

fn ticks(app: &mut App, n: u32) {
    for _ in 0..n {
        app.tick(FRAME);
    }
}

/// A Master Gear `length` seconds long.
fn master(app: &mut App, length: f32) -> NodeId {
    let id = add(app, "mastergear");
    set(app, id, "length", length);
    id
}

/// A Ratio Gear's Teeth, `p : q`.
fn teeth(app: &mut App, gear: NodeId, p: f32, q: f32) {
    set(app, gear, "p", p);
    set(app, gear, "q", q);
}

/// A Ratio Gear at `p : q`, its Clock In on `from`'s Cycles where it names one.
fn geared(app: &mut App, from: Option<NodeId>, p: f32, q: f32) -> NodeId {
    let id = add(app, "ratiogear");
    teeth(app, id, p, q);
    if let Some(from) = from {
        connect(app, (from, "cycles"), (id, "clock"));
    }
    id
}

/// A gear's Cycles, read whole.
fn count(app: &App, node: NodeId) -> f64 {
    app.uniform(PortRef::new(node, "cycles"))
        .expect("cycles is published")
}

/// **A Ratio Gear is its parent's count times p ÷ q, to the bit, however the playhead got
/// there.** One on ambient seconds at 3 : 2 and one at 7 : 5 on a Master Gear's Cycles,
/// played to a moment, sought straight to it, sought away and back, and rendered up to it:
/// each reads its parent's count times p ÷ q exactly, and the one on ambient seconds, whose
/// parent is the playhead, reads the same bits all four ways.
#[test]
fn a_ratio_gear_is_its_parent_times_p_over_q_to_the_bit() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.3);
    let ambient = geared(&mut app, None, 3.0, 2.0);
    let on_master = geared(&mut app, Some(clock), 7.0, 5.0);
    let check = |app: &App, how: &str| -> u64 {
        let t = app.transport_state().playhead;
        let m = count(app, clock);
        assert_eq!(
            count(app, on_master).to_bits(),
            (m * 7.0 / 5.0).to_bits(),
            "{how}: 7 : 5 of the master's {m}"
        );
        let a = count(app, ambient);
        assert_eq!(
            a.to_bits(),
            (t * 3.0 / 2.0).to_bits(),
            "{how}: 3 : 2 of the playhead's {t}"
        );
        a.to_bits()
    };

    ticks(&mut app, 200);
    let t = app.transport_state().playhead;
    let played = check(&app, "played");

    app.transport(Transport::Pause);
    app.transport(Transport::Seek(t));
    app.tick(FRAME);
    assert_eq!(app.transport_state().playhead, t);
    assert_eq!(
        check(&app, "sought"),
        played,
        "sought to where it played to"
    );
    let sought = count(&app, on_master).to_bits();

    app.transport(Transport::Seek(t + 1234.567));
    app.tick(FRAME);
    app.transport(Transport::Seek(t));
    app.tick(FRAME);
    assert_eq!(check(&app, "away and back"), played);
    assert_eq!(
        count(&app, on_master).to_bits(),
        sought,
        "the master's gear sought away and back"
    );

    app.transport(Transport::Play);
    app.reset_cpu();
    app.transport(Transport::Seek(t - 1.0));
    for n in 0..60 {
        app.tick_at(t - 1.0 + f64::from(n) / 60.0);
    }
    app.tick_at(t);
    assert_eq!(check(&app, "rendered"), played, "rendered up to it");
}

/// **A Teeth change lands on the parent times the new p ÷ q.** A gear at 1 : 1 on a
/// one-second Master Gear, turned to 4 : 1 ten seconds in and then to 4 : 3, reads the
/// master's count times the new ratio on the very frame after, to the bit — nothing pending,
/// nothing bent — and fires no beat for the thirty cycles the first jump went over.
#[test]
fn a_teeth_change_lands_on_the_parent_times_the_new_ratio() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = geared(&mut app, Some(clock), 1.0, 1.0);
    ticks(&mut app, 618);
    teeth(&mut app, gear, 4.0, 1.0);
    app.tick(FRAME);
    let m = count(&app, clock);
    assert!(m > 10.0 && m < 10.5, "{m}");
    assert_eq!(count(&app, gear).to_bits(), (m * 4.0).to_bits(), "4 : 1");
    let downs = app
        .edges(PortRef::new(gear, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .count();
    assert_eq!(downs, 0, "nothing fires on the way");
    ticks(&mut app, 7);
    teeth(&mut app, gear, 4.0, 3.0);
    app.tick(FRAME);
    let m = count(&app, clock);
    assert_eq!(
        count(&app, gear).to_bits(),
        (m * 4.0 / 3.0).to_bits(),
        "4 : 3"
    );
    // And on from there at the new ratio.
    ticks(&mut app, 40);
    let m = count(&app, clock);
    assert_eq!(count(&app, gear).to_bits(), (m * 4.0 / 3.0).to_bits());
}

/// A Ratio Gear's direction: Forward, or Reverse, which negates its product.
fn reverse(app: &mut App, gear: NodeId, reversed: bool) {
    choose(
        app,
        gear,
        "direction",
        if reversed { "reverse" } else { "forward" },
    );
}

/// The downs `node`'s Trigger fired on the frame just ticked.
fn downs(app: &App, node: NodeId) -> usize {
    app.edges(PortRef::new(node, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .count()
}

/// **A reversed Ratio Gear is minus its parent times p ÷ q, to the bit, however the playhead
/// got there.** One on ambient seconds at 3 : 2 and one at 7 : 5 on a Master Gear's Cycles,
/// both in Reverse, played to a moment, sought straight to it, sought away and back, and
/// rendered up to it: each reads `−(parent × p ÷ q)` exactly, the same bits all four ways.
#[test]
fn a_reversed_gear_is_minus_its_parent_times_p_over_q_to_the_bit() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.3);
    let ambient = geared(&mut app, None, 3.0, 2.0);
    let on_master = geared(&mut app, Some(clock), 7.0, 5.0);
    reverse(&mut app, ambient, true);
    reverse(&mut app, on_master, true);
    // Each reads its parent's count as the bits it is, and the one on ambient seconds,
    // whose parent is the playhead, the same bits every way.
    let check = |app: &App, how: &str| -> u64 {
        let t = app.transport_state().playhead;
        let m = count(app, clock);
        assert_eq!(
            count(app, on_master).to_bits(),
            (-(m * 7.0 / 5.0)).to_bits(),
            "{how}: minus 7 : 5 of the master's {m}"
        );
        let a = count(app, ambient);
        assert_eq!(
            a.to_bits(),
            (-(t * 3.0 / 2.0)).to_bits(),
            "{how}: minus 3 : 2 of the playhead's {t}"
        );
        a.to_bits()
    };

    ticks(&mut app, 200);
    let t = app.transport_state().playhead;
    let played = check(&app, "played");
    assert!(f64::from_bits(played) < -4.0, "it counts down");

    app.transport(Transport::Pause);
    app.transport(Transport::Seek(t));
    app.tick(FRAME);
    assert_eq!(
        check(&app, "sought"),
        played,
        "sought to where it played to"
    );
    let sought = count(&app, on_master).to_bits();

    app.transport(Transport::Seek(t + 1234.567));
    app.tick(FRAME);
    app.transport(Transport::Seek(t));
    app.tick(FRAME);
    assert_eq!(check(&app, "away and back"), played);
    assert_eq!(
        count(&app, on_master).to_bits(),
        sought,
        "the master's gear"
    );

    app.transport(Transport::Play);
    app.reset_cpu();
    app.transport(Transport::Seek(t - 1.0));
    for n in 0..60 {
        app.tick_at(t - 1.0 + f64::from(n) / 60.0);
    }
    app.tick_at(t);
    assert_eq!(check(&app, "rendered"), played, "rendered up to it");
}

/// **Switching direction jumps exactly**, as a Teeth change does: a 3 : 2 gear on a
/// one-second Master Gear switched to Reverse ten seconds in reads minus the master's count
/// times 3 ÷ 2 on the very frame after, to the bit, fires nothing for the thirty cycles it
/// went over, and says its readings jumped, so a 1 : 1 gear counting it is born again with it
/// and fires nothing either; switched back, it is the plain product again.
#[test]
fn switching_direction_jumps_exactly() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = geared(&mut app, Some(clock), 3.0, 2.0);
    let follower = geared(&mut app, Some(gear), 1.0, 1.0);
    ticks(&mut app, 618);
    reverse(&mut app, gear, true);
    app.tick(FRAME);
    let m = count(&app, clock);
    assert!(m > 10.0 && m < 10.5, "{m}");
    assert_eq!(
        count(&app, gear).to_bits(),
        (-(m * 3.0 / 2.0)).to_bits(),
        "Reverse lands on minus the product"
    );
    assert_eq!(count(&app, follower).to_bits(), count(&app, gear).to_bits());
    assert_eq!(downs(&app, gear), 0, "nothing fires on the way");
    assert_eq!(downs(&app, follower), 0, "nor on its follower's");
    ticks(&mut app, 40);
    let m = count(&app, clock);
    assert_eq!(count(&app, gear).to_bits(), (-(m * 3.0 / 2.0)).to_bits());
    reverse(&mut app, gear, false);
    app.tick(FRAME);
    let m = count(&app, clock);
    assert_eq!(
        count(&app, gear).to_bits(),
        (m * 3.0 / 2.0).to_bits(),
        "Forward lands on the product again"
    );
    assert_eq!(downs(&app, gear), 0);
    assert_eq!(downs(&app, follower), 0);
}

/// **A reversed gear's Trigger fires once on each whole cycle of its output, going down.** A
/// 3 : 2 gear in Reverse on a one-second Master Gear passes fifteen whole cycles of its own in
/// ten seconds, and fires fifteen beats, each on the frame its count passes a whole number
/// going down and placed where inside the frame it fell. A 1 : 64 gear on a 64 : 1 one in
/// Reverse on ambient seconds, whose count falls by more than a cycle a frame, counts down a
/// cycle a second with it: a fast clock running backwards is motion, never a jump, so it fires
/// one beat a second.
#[test]
fn a_reversed_gears_trigger_fires_once_per_whole_output_cycle() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = geared(&mut app, Some(clock), 3.0, 2.0);
    reverse(&mut app, gear, true);
    let fast = geared(&mut app, None, 64.0, 1.0);
    reverse(&mut app, fast, true);
    let slow = geared(&mut app, Some(fast), 1.0, 64.0);
    ticks(&mut app, 3);
    let (mut beats, mut slow_beats) = (0, 0);
    for _ in 0..600 {
        let (from, slow_from) = (count(&app, gear), count(&app, slow));
        app.tick(FRAME);
        let (to, slow_to) = (count(&app, gear), count(&app, slow));
        let passed = (from.floor() - to.floor()) as usize;
        let fired: Vec<f32> = app
            .edges(PortRef::new(gear, "trigger"))
            .iter()
            .filter(|e| e.is_down())
            .map(|e| e.at)
            .collect();
        assert_eq!(
            fired.len(),
            passed,
            "{from} to {to}: one beat a whole cycle passed"
        );
        if let [at] = fired[..] {
            let whole = from.floor();
            let expected = ((from - whole) / (from - to)) as f32 * FRAME;
            assert!(
                (at - expected).abs() < 1e-4,
                "the beat at {whole} falls {expected} into the frame, not {at}"
            );
        }
        beats += fired.len();
        let slow_passed = (slow_from.floor() - slow_to.floor()) as usize;
        assert_eq!(
            downs(&app, slow),
            slow_passed,
            "{slow_from} to {slow_to}: the follower of a fast reversed gear"
        );
        slow_beats += slow_passed;
    }
    assert_eq!(beats, 15, "fifteen whole cycles down in ten seconds");
    assert_eq!(slow_beats, 10, "one a second");
}

/// **A Ratio Gear's Offset is added after the product, and after its direction, to the bit,
/// however the playhead got there.** A 7 : 5 gear on a Master Gear's Cycles at an Offset of
/// 0.1, forwards and in Reverse, and a 3 : 2 one on ambient seconds at −0.375: each reads
/// `±(parent × p ÷ q) + offset` exactly, the Offset's `f32` read whole, played to a moment,
/// sought straight to it, sought away and back, and rendered up to it.
#[test]
fn a_ratio_gears_offset_is_added_after_the_product_to_the_bit() {
    for reversed in [false, true] {
        let mut app = App::headless();
        let clock = master(&mut app, 1.3);
        let ambient = geared(&mut app, None, 3.0, 2.0);
        let on_master = geared(&mut app, Some(clock), 7.0, 5.0);
        for (gear, offset) in [(ambient, -0.375_f32), (on_master, 0.1)] {
            set(&mut app, gear, "phaseOffset", offset);
            reverse(&mut app, gear, reversed);
        }
        let sign = if reversed { -1.0 } else { 1.0 };
        let check = |app: &App, how: &str| -> u64 {
            let t = app.transport_state().playhead;
            let m = count(app, clock);
            assert_eq!(
                count(app, on_master).to_bits(),
                (sign * (m * 7.0 / 5.0) + f64::from(0.1_f32)).to_bits(),
                "{how}, reversed {reversed}: 7 : 5 of the master's {m}, plus 0.1"
            );
            let a = count(app, ambient);
            assert_eq!(
                a.to_bits(),
                (sign * (t * 3.0 / 2.0) - 0.375).to_bits(),
                "{how}, reversed {reversed}: 3 : 2 of the playhead's {t}, less 0.375"
            );
            a.to_bits()
        };

        ticks(&mut app, 200);
        let t = app.transport_state().playhead;
        let played = check(&app, "played");

        app.transport(Transport::Pause);
        app.transport(Transport::Seek(t));
        app.tick(FRAME);
        assert_eq!(check(&app, "sought"), played);
        let sought = count(&app, on_master).to_bits();

        app.transport(Transport::Seek(t + 1234.567));
        app.tick(FRAME);
        app.transport(Transport::Seek(t));
        app.tick(FRAME);
        assert_eq!(check(&app, "away and back"), played);
        assert_eq!(
            count(&app, on_master).to_bits(),
            sought,
            "the master's gear"
        );

        app.transport(Transport::Play);
        app.reset_cpu();
        app.transport(Transport::Seek(t - 1.0));
        for n in 0..60 {
            app.tick_at(t - 1.0 + f64::from(n) / 60.0);
        }
        app.tick_at(t);
        assert_eq!(check(&app, "rendered"), played);
    }
}

/// **An Offset that sways a gear moves it about its locked place.** A 1 : 1 gear on a
/// one-second Master Gear with a Number in its Offset turned from 0 to 1 over two seconds reads
/// the master's count plus the Number every frame, and its Trigger fires where that sum passes
/// a whole cycle: three beats, where the master alone passes two.
#[test]
fn an_offset_cabled_into_a_gear_sways_it() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = geared(&mut app, Some(clock), 1.0, 1.0);
    let sway = add(&mut app, "number");
    set(&mut app, sway, "value", 0.0);
    connect(&mut app, (sway, "output"), (gear, "phaseOffset"));
    ticks(&mut app, 3);
    let mut beats = 0;
    let mut passed = 0;
    for n in 1..=120 {
        let before = count(&app, gear);
        let v = n as f32 / 120.0;
        set(&mut app, sway, "value", v);
        app.tick(FRAME);
        let m = count(&app, clock);
        assert_eq!(
            count(&app, gear),
            m + f64::from(v),
            "the master plus the Number"
        );
        passed += (count(&app, gear).floor() - before.floor()) as usize;
        beats += downs(&app, gear);
    }
    assert_eq!(
        passed, 3,
        "the master's two seconds and the Number's one cycle"
    );
    assert_eq!(beats, passed, "a beat on each whole cycle the sum passed");
}

/// A Ratio Gear on a Master Gear's Cycles counts in its cycles: ÷4 of a beat is a bar. The
/// beat turned from half a second to a third, the bar bends — its slope changes and its value
/// does not jump — because the Cycles it reads are an integral of the length.
#[test]
fn a_ratio_gear_on_a_master_bends_on_a_length_change() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.5);
    let bar = geared(&mut app, Some(clock), 1.0, 4.0);

    ticks(&mut app, 120);
    let beats = read(&app, clock, "cycles");
    assert!(
        (beats - 4.0).abs() < 0.05,
        "two seconds of half-second beats is four: {beats}"
    );
    let cycles = read(&app, bar, "cycles");
    assert!(
        (cycles - 0.25 * beats).abs() < 0.02,
        "a quarter of the beats: {cycles} of {beats}"
    );

    let slope = |app: &mut App| {
        let a = read(app, bar, "cycles");
        app.tick(FRAME);
        read(app, bar, "cycles") - a
    };
    let before = slope(&mut app);
    assert!(
        (before - 0.25 * 2.0 * f64::from(FRAME)).abs() < 1e-4,
        "{before}"
    );
    let at = read(&app, bar, "cycles");
    set(&mut app, clock, "length", 1.0 / 3.0);
    app.tick(FRAME);
    let step = read(&app, bar, "cycles") - at;
    assert!(
        step > 0.0 && step < 0.25 * 3.0 * f64::from(FRAME) + 1e-4,
        "no jump across the change, one frame's motion at most: {step}"
    );
    ticks(&mut app, 2);
    let after = slope(&mut app);
    assert!(
        (after - 0.25 * 3.0 * f64::from(FRAME)).abs() < 1e-4,
        "and a steeper slope after it: {after}"
    );
}

/// **×3 of ÷3 is the input**: two Ratio Gears in a chain give back the Master Gear's cycles,
/// and ÷3 alone comes round once every three.
#[test]
fn three_times_a_third_is_the_input() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.5);
    let third = geared(&mut app, Some(clock), 1.0, 3.0);
    let back = geared(&mut app, Some(third), 3.0, 1.0);
    ticks(&mut app, 200);
    let (m, t, b) = (
        read(&app, clock, "cycles"),
        read(&app, third, "cycles"),
        read(&app, back, "cycles"),
    );
    assert!((t - m / 3.0).abs() < 1e-3, "{t} against {m}");
    assert!((b - m).abs() < 1e-3, "{b} against {m}");
}

/// **A Master Gear is born where the playhead puts it, and a seek is a birth.** Two master
/// gears of one length made at different moments agree; a Reset puts one at the start of a
/// cycle and fires a beat; the time readout's reset — a seek to zero — puts every gear back at
/// zero.
#[test]
fn a_seek_rebirths_every_gear_and_a_reset_starts_a_cycle() {
    let mut app = App::headless();
    let early = master(&mut app, 2.0);
    ticks(&mut app, 50);
    let late = master(&mut app, 2.0);
    let gear = geared(&mut app, Some(early), 3.0, 1.0);
    ticks(&mut app, 20);
    let (a, b) = (read(&app, early, "cycles"), read(&app, late, "cycles"));
    assert!((a - b).abs() < 1e-4, "made late, it agrees: {a} {b}");

    app.press(PortRef::new(late, "reset"), true);
    app.tick(FRAME);
    app.press(PortRef::new(late, "reset"), false);
    let c = read(&app, late, "cycles");
    assert!(c.abs() < 0.05, "a reset starts a cycle: {c}");
    assert!(
        app.edges(PortRef::new(late, "trigger"))
            .iter()
            .any(|e| e.is_down()),
        "and is a beat"
    );

    ticks(&mut app, 30);
    app.transport(Transport::Seek(0.0));
    app.tick(FRAME);
    for (id, name) in [(early, "early"), (late, "late"), (gear, "gear")] {
        let c = read(&app, id, "cycles");
        assert!(c.abs() < 0.05, "{name} is born again at zero: {c}");
    }
}

/// Did `node` fire a down on its Trigger on the frame just ticked?
fn beat(app: &App, node: NodeId) -> bool {
    app.edges(PortRef::new(node, "trigger"))
        .iter()
        .any(|e| e.is_down())
}

/// **A gear born on a whole cycle fires that beat.** The time readout's reset is a seek to
/// zero, where every gear is at the start of its cycle: a one-second Master Gear, a ×1 and a ÷4
/// Ratio Gear on its Cycles and a ×1 on ambient seconds each fire one down on the frame the
/// seek lands, though the show has run a frame past zero by its end. A seek to half a second
/// lands every one of them mid-cycle and fires nothing, from behind it or from ten seconds
/// past it; one to three or ten seconds lands the master, the ×1s and the ambient gear on a
/// whole cycle and the ÷4 three quarters or half way through its own.
#[test]
fn a_gear_born_on_a_whole_cycle_fires_that_beat() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let once = geared(&mut app, Some(clock), 1.0, 1.0);
    let quarter = geared(&mut app, Some(clock), 1.0, 4.0);
    let ambient = geared(&mut app, None, 1.0, 1.0);
    let gears = [
        (clock, "master"),
        (once, "×1"),
        (quarter, "÷4"),
        (ambient, "ambient"),
    ];
    ticks(&mut app, 140);
    for (seek, fires) in [
        (0.0, [true, true, true, true]),
        (0.5, [false; 4]),
        (3.0, [true, true, false, true]),
        (10.0, [true, true, false, true]),
        (0.5, [false; 4]),
    ] {
        ticks(&mut app, 17);
        app.transport(Transport::Seek(seek));
        app.tick(FRAME);
        for ((id, name), fires) in gears.iter().zip(fires) {
            assert_eq!(
                beat(&app, *id),
                fires,
                "{name} after a seek to {seek}: {}",
                read(&app, *id, "cycles")
            );
            let downs = app
                .edges(PortRef::new(*id, "trigger"))
                .iter()
                .filter(|e| e.is_down())
                .count();
            assert!(downs <= 1, "{name}: one beat, not {downs}");
        }
        app.tick(FRAME);
        for (id, name) in gears {
            assert!(!beat(&app, id), "{name}: and nothing on the frame after");
        }
    }
}

/// **A seek re-births a Ratio Gear where playing would have put it.** A ÷4 gear on a
/// one-second Master Gear's Cycles, after a seek to three seconds, is three quarters of its
/// cycle on — the count it reads times its ratio — and not a quarter of the fraction of the
/// master's cycle, which starts its bar afresh on whatever beat the seek lands on.
#[test]
fn a_seek_puts_a_ratio_gear_where_playing_puts_it() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let quarter = geared(&mut app, Some(clock), 1.0, 4.0);
    ticks(&mut app, 100);
    app.transport(Transport::Seek(3.0));
    app.tick(FRAME);
    let (m, q) = (read(&app, clock, "cycles"), read(&app, quarter, "cycles"));
    assert!((m - 3.0).abs() < 0.02, "the master is three cycles on: {m}");
    assert!(
        (q - 0.75).abs() < 0.02,
        "and the ÷4 three quarters of its own: {q}"
    );
}

/// **A cable moved from one clock to another is a birth.** A ×1 Ratio Gear counting a
/// tenth-of-a-second Master Gear, its Clock In taken over by a seven-second one's Cycles in a
/// single edit, counts the new clock from where it is: no Trigger fires for the thirty cycles
/// between the two readings, and its Cycles are the new clock's.
#[test]
fn moving_clock_in_to_another_clock_is_a_birth() {
    let mut app = App::headless();
    let fast = master(&mut app, 0.1);
    let slow = master(&mut app, 7.0);
    let gear = geared(&mut app, Some(fast), 1.0, 1.0);
    ticks(&mut app, 180);
    connect(&mut app, (slow, "cycles"), (gear, "clock"));
    app.tick(FRAME);
    let downs = app
        .edges(PortRef::new(gear, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .count();
    assert_eq!(
        downs, 0,
        "nothing fires on the way from one clock to the other"
    );
    let (g, s) = (read(&app, gear, "cycles"), read(&app, slow, "cycles"));
    assert!((g - s).abs() < 1e-3, "it counts the new clock: {g} and {s}");
}

/// **A clock that a Reset sends back many cycles is a jump to the gear counting it.** A ×1
/// Ratio Gear on a quarter-second Master Gear, the master Reset after forty cycles: the
/// follower is born again where the master now is and fires at most the one downbeat, where
/// it ran the forty cycles back as motion and fired a Trigger for each in one frame; it goes
/// on counting with the master after, and a master held and Reset exactly onto a whole cycle
/// gives it that one downbeat.
#[test]
fn a_reset_of_the_clock_in_fires_no_burst() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.25);
    let gear = geared(&mut app, Some(clock), 1.0, 1.0);
    ticks(&mut app, 600);
    assert!(read(&app, gear, "cycles") > 39.0);
    let reset = PortRef::new(clock, "reset");
    app.press(reset, true);
    app.tick(FRAME);
    app.press(reset, false);
    let downs = app
        .edges(PortRef::new(gear, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .count();
    assert!(downs <= 1, "at most the downbeat, not {downs}");
    let (g, m) = (read(&app, gear, "cycles"), read(&app, clock, "cycles"));
    assert!(
        (g - m).abs() < 1e-4,
        "born where the master is: {g} and {m}"
    );
    ticks(&mut app, 30);
    let (g, m) = (read(&app, gear, "cycles"), read(&app, clock, "cycles"));
    assert!((g - m).abs() < 1e-4, "and counting with it: {g} and {m}");

    // Held, a Reset puts the master exactly at the start of a cycle, and the follower lands
    // on its own downbeat: the one beat.
    let hold = PortRef::new(clock, "hold");
    app.press(hold, true);
    app.tick(FRAME);
    app.press(hold, false);
    app.press(reset, true);
    app.tick(FRAME);
    app.press(reset, false);
    let downs = app
        .edges(PortRef::new(gear, "trigger"))
        .iter()
        .filter(|e| e.is_down())
        .count();
    assert_eq!(read(&app, gear, "cycles"), 0.0);
    assert_eq!(
        downs, 1,
        "a clock thrown back onto a whole cycle is a downbeat"
    );
}

/// **A hand's Reset on the clock is a beat to every gear counting it.** The hand presses
/// between two ticks, so the Master Gear ends the frame a frame's motion past zero and fires
/// its Reset beat; a follower at 1 : 1, 2 : 1 or 1 : 4 is born again just past its own
/// downbeat — the 1 : 4 too, since a master at zero is a ÷4 at zero — and fires that one down
/// on the same frame.
#[test]
fn a_hand_reset_of_the_clock_is_a_beat_to_its_followers() {
    for (p, q) in [(1.0, 1.0), (2.0, 1.0), (1.0, 4.0)] {
        let r = p / q;
        let mut app = App::headless();
        let clock = master(&mut app, 1.0);
        let gear = geared(&mut app, Some(clock), p, q);
        ticks(&mut app, 140);
        let reset = PortRef::new(clock, "reset");
        app.press(reset, true);
        app.tick(FRAME);
        app.press(reset, false);
        let downs = |port| {
            app.edges(PortRef::new(port, "trigger"))
                .iter()
                .filter(|e| e.is_down())
                .count()
        };
        assert_eq!(downs(clock), 1, "the master's Reset is a beat");
        assert_eq!(downs(gear), 1, "×{r}: and its follower's");
        let m = read(&app, clock, "cycles");
        assert!(m > 0.0 && m < 0.02, "the master is a frame past zero: {m}");
    }
}

/// **A Reset before the clock has run one cycle is a beat to its followers too.** A one-second
/// Master Gear Reset half a cycle in steps back less than a cycle, which a follower cannot tell
/// from a clock running backwards by the step alone: the master's Reset says so on its outputs,
/// and a follower at 1 : 1, 2 : 1 or 1 : 4 is born again where the master now is and fires its
/// one downbeat on the frame the master fires its own. A Number turned down a sixtieth a frame
/// is a clock running backwards, and a follower on it counts backwards with it, a beat a cycle
/// and never a birth; so is a 1 : 1 gear in Reverse on ambient seconds, and a follower on it
/// fires on the frames it does.
#[test]
fn a_reset_inside_the_first_cycle_is_a_beat_to_the_followers() {
    let downs = |app: &App, node: NodeId| {
        app.edges(PortRef::new(node, "trigger"))
            .iter()
            .filter(|e| e.is_down())
            .count()
    };
    for (p, q) in [(1.0_f32, 1.0_f32), (2.0, 1.0), (1.0, 4.0)] {
        let r = f64::from(p) / f64::from(q);
        let mut app = App::headless();
        let clock = master(&mut app, 1.0);
        let gear = geared(&mut app, Some(clock), p, q);
        ticks(&mut app, 30);
        let reset = PortRef::new(clock, "reset");
        app.press(reset, true);
        app.tick(FRAME);
        app.press(reset, false);
        assert_eq!(downs(&app, clock), 1, "the master's Reset is a beat");
        assert_eq!(downs(&app, gear), 1, "{p} : {q}: and its follower's");
        let (g, m) = (read(&app, gear, "cycles"), read(&app, clock, "cycles"));
        assert!(
            (g - r * m).abs() < 1e-5,
            "{p} : {q}: born where the master is: {g} and {m}"
        );
    }

    let mut app = App::headless();
    let back = add(&mut app, "number");
    set(&mut app, back, "value", 3.0);
    let follower = geared(&mut app, None, 1.0, 1.0);
    connect(&mut app, (back, "output"), (follower, "clock"));
    ticks(&mut app, 10);
    let mut beats = 0;
    for n in 1..=150 {
        set(&mut app, back, "value", 3.0 - n as f32 / 60.0);
        app.tick(FRAME);
        beats += downs(&app, follower);
        let (f, b) = (read(&app, follower, "cycles"), read(&app, back, "output"));
        assert!(
            (f - b).abs() < 1e-6,
            "counting backwards with it, never born again: {f} against {b}"
        );
    }
    assert_eq!(
        beats, 2,
        "two and a half cycles back from three is two whole ones"
    );

    let mut app = App::headless();
    let back = geared(&mut app, None, 1.0, 1.0);
    reverse(&mut app, back, true);
    let follower = geared(&mut app, Some(back), 1.0, 1.0);
    ticks(&mut app, 10);
    let mut beats = 0;
    for _ in 0..150 {
        app.tick(FRAME);
        assert_eq!(
            downs(&app, follower),
            downs(&app, back),
            "a beat of the reversed gear is a beat of its follower"
        );
        beats += downs(&app, follower);
        let (f, b) = (count(&app, follower), count(&app, back));
        assert_eq!(f, b, "counting backwards with it, never born again");
    }
    assert_eq!(beats, 2, "two and a half seconds back is two whole cycles");
}

/// **A cabled clock passes 2520 with no seam.** A gear's Cycles never wrap, so a Ratio Gear
/// counting them keeps going the same way across 2520, at Teeth whose ratio's denominator
/// divides 2520 or not — 1 : 2, 1 : 7, 1 : 11, 3 : 7 and 1 : 7 in Reverse — each frame its
/// share of the clock's motion to a billionth.
#[test]
fn a_cabled_clock_passes_2520_with_no_seam() {
    let mut app = App::headless();
    // Ambient seconds at 20 : 1, 2520 cycles in 126 seconds.
    let fast = geared(&mut app, None, 20.0, 1.0);
    let teeth = [(1.0, 2.0), (1.0, 7.0), (1.0, 11.0), (3.0, 7.0), (-1.0, 7.0)];
    let followers: Vec<NodeId> = teeth
        .iter()
        .map(|&(p, q): &(f32, f32)| {
            let id = geared(&mut app, Some(fast), p.abs(), q);
            reverse(&mut app, id, p < 0.0);
            id
        })
        .collect();
    app.tick(FRAME);
    app.transport(Transport::Seek(125.9));
    app.tick(FRAME);
    let mut last: Vec<f64> = followers.iter().map(|&id| count(&app, id)).collect();
    for _ in 0..30 {
        let before = read(&app, fast, "cycles");
        app.tick(FRAME);
        let after = read(&app, fast, "cycles");
        assert!(after > before, "the clock climbs: {before} to {after}");
        for ((id, (p, q)), last) in followers.iter().zip(teeth).zip(&mut last) {
            let now = count(&app, *id);
            let d = now - *last;
            assert!(
                (d - f64::from(p) / f64::from(q) * 20.0 * f64::from(FRAME)).abs() < 1e-9,
                "{p} : {q} moves by its share of the clock each frame, past 2520 too: {d}"
            );
            *last = now;
        }
    }
    assert!(count(&app, fast) > 2520.0, "the clock passed 2520");
}

/// What a node's row prints for one of its outputs: the reading the canvas is handed.
fn shown(app: &App, node: NodeId, key: &'static str) -> f64 {
    supersilvia::synth::Uniforms::of(app.snapshot())
        .get(PortRef::new(node, key))
        .unwrap_or_else(|| panic!("{key} is published"))
}

/// **A gear's Cycles never wrap.** A one-second Master Gear played past 1260, 2520, 5040 and a
/// million cycles: the number it publishes, which a Math node and a row both read, is the
/// count itself, climbing a frame's time every frame for as long as the show runs.
#[test]
fn a_gears_cycles_climb_on_and_never_wrap() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    app.tick(FRAME);
    for at in [1259.5, 2519.5, 5039.5, 1.0e6 - 0.5] {
        app.transport(Transport::Seek(at));
        app.tick(FRAME);
        let mut before = read(&app, clock, "cycles");
        for _ in 0..60 {
            app.tick(FRAME);
            let now = read(&app, clock, "cycles");
            assert_eq!(now, count(&app, clock), "the number is the count");
            assert_eq!(shown(&app, clock, "cycles"), now, "and the row reads it");
            assert!(
                (now - before - f64::from(FRAME)).abs() < 1e-9,
                "a frame's time each frame at {now}: {before} to {now}"
            );
            before = now;
        }
        assert!(before > at + 0.9, "it climbs past {at}");
    }
}

/// **Cycles × 0.37 through a Multiply never jumps.** A one-second Master Gear's Cycles times
/// 0.37, played across 2520, 5040 and on past a million cycles: what comes out is the count
/// times 0.37 to the bit every frame, moving 0.37 of a frame's cycles each frame, where a count
/// read as one `f32` wrapped at 2520 jumped by 932 there.
#[test]
fn cycles_times_0_37_through_a_multiply_never_jumps() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let product = add(&mut app, "multiply");
    connect(&mut app, (clock, "cycles"), (product, "a"));
    set(&mut app, product, "b", 0.37);
    let b = f64::from(0.37_f32);
    app.tick(FRAME);
    for at in [2519.5, 5039.5, 1.0e6 - 0.5] {
        app.transport(Transport::Seek(at));
        app.tick(FRAME);
        let mut before = read(&app, product, "output");
        for _ in 0..60 {
            app.tick(FRAME);
            let out = read(&app, product, "output");
            assert_eq!(out, count(&app, clock) * b, "the count times 0.37");
            assert!(
                (out - before - b * f64::from(FRAME)).abs() < 1e-9,
                "no jump at {}: {before} to {out}",
                count(&app, clock)
            );
            before = out;
        }
    }
    assert!(count(&app, clock) > 1.0e6, "a million cycles on");
}

/// **An Add fed Cycles reads the count exactly, a million cycles in**, and a CPU node reading
/// that sum reads the count: a Ratio Gear at 1 : 1 on it counts what its parent does, to the
/// bit, frame after frame.
#[test]
fn an_add_fed_cycles_reads_the_count_exactly_at_a_million() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let sum = add(&mut app, "add");
    connect(&mut app, (clock, "cycles"), (sum, "a"));
    set(&mut app, sum, "b", 0.0);
    let follower = geared(&mut app, None, 1.0, 1.0);
    connect(&mut app, (sum, "output"), (follower, "clock"));
    app.tick(FRAME);
    app.transport(Transport::Pause);
    app.transport(Transport::Seek(1.0e6));
    app.tick(FRAME);
    assert_eq!(read(&app, sum, "output"), 1.0e6, "a million, exactly");
    app.transport(Transport::Play);
    for _ in 0..30 {
        app.tick(FRAME);
        let whole = count(&app, clock);
        assert_eq!(read(&app, sum, "output"), whole, "the sum is the count");
        assert_eq!(count(&app, follower), whole, "and a gear on it counts it");
    }
}

/// **A count of 10⁷ and a quarter reaches a CPU node to within a millionth.** A Master Gear
/// paused at 10⁷ + 0.25 cycles, through an Add, into a Ratio Gear's Clock In: the gear reads
/// it, where one `f32` holds no quarter there at all.
#[test]
fn ten_million_and_a_quarter_reaches_a_cpu_node_to_a_millionth() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let sum = add(&mut app, "add");
    connect(&mut app, (clock, "cycles"), (sum, "a"));
    set(&mut app, sum, "b", 0.0);
    let follower = geared(&mut app, None, 1.0, 1.0);
    connect(&mut app, (sum, "output"), (follower, "clock"));
    app.tick(FRAME);
    app.transport(Transport::Pause);
    app.transport(Transport::Seek(1.0e7 + 0.25));
    ticks(&mut app, 3);
    let got = count(&app, follower);
    assert!((got - (1.0e7 + 0.25)).abs() < 1e-6, "{got}");
}

/// **A reversed gear's count through Math stays negative and continuous.** A Ratio Gear in
/// Reverse at 20 : 1 on ambient seconds counts down twenty a second; through an Add with zero,
/// across −2520 and on past minus a million, what comes out is the count, below zero, moving
/// twenty a second's share each frame with no jump.
#[test]
fn a_reversed_gears_count_through_math_stays_negative_and_continuous() {
    let mut app = App::headless();
    let gear = geared(&mut app, None, 20.0, 1.0);
    reverse(&mut app, gear, true);
    let sum = add(&mut app, "add");
    connect(&mut app, (gear, "cycles"), (sum, "a"));
    set(&mut app, sum, "b", 0.0);
    app.tick(FRAME);
    for (at, past) in [(125.9, -2520.0), (5.0e4, -1.0e6)] {
        app.transport(Transport::Seek(at));
        app.tick(FRAME);
        let mut before = read(&app, sum, "output");
        for _ in 0..30 {
            app.tick(FRAME);
            let out = read(&app, sum, "output");
            assert_eq!(out, count(&app, gear), "the sum is the count");
            assert!(out < 0.0, "{out}: a reversed count stays negative");
            assert!(
                (out - before + 20.0 * f64::from(FRAME)).abs() < 1e-6,
                "no jump: {before} to {out}"
            );
            before = out;
        }
        assert!(before < past, "{before} is past {past}");
    }
}

/// **A count just below zero reads as a small negative.** A Ratio Gear whose Clock In steps
/// from zero to a hundredth below it publishes −0.01, the small negative it is, and a Phase of
/// 0.99.
#[test]
fn a_ratio_gear_stepped_below_zero_reads_a_small_negative() {
    let mut app = App::headless();
    let input = add(&mut app, "number");
    set(&mut app, input, "value", 0.0);
    let gear = add(&mut app, "ratiogear");
    connect(&mut app, (input, "output"), (gear, "clock"));
    ticks(&mut app, 3);
    assert_eq!(read(&app, gear, "cycles"), 0.0);
    set(&mut app, input, "value", -0.01);
    app.tick(FRAME);
    let c = read(&app, gear, "cycles");
    assert!((c + 0.01).abs() < 1e-5, "a hundredth back from zero: {c}");
    assert!(
        (read(&app, gear, "wrapped") - 0.99).abs() < 1e-5,
        "and its Phase is 0.99"
    );
}

/// **The Time node's Seconds and a Master Gear's Cycles before the playhead's zero** read as
/// the negative they are, to zero's precision: a render's warm-up at −0.01 s is −0.01 s, and
/// a four-second master there is −0.0025 cycles.
#[test]
fn a_clock_before_zero_reads_negative_at_zeros_precision() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    let clock = master(&mut app, 4.0);
    app.tick_at(-1.0);
    app.tick_at(-0.01);
    let seconds = read(&app, time, "seconds");
    assert_eq!(seconds, -0.01, "Seconds, to the bit");
    let cycles = read(&app, clock, "cycles");
    assert!((cycles + 0.0025).abs() < 1e-9, "{cycles}");
}

/// **A 0..1 Phase cabled into Clock In is the gear's parent like any other.** A gear's Phase
/// in a 3 : 2 gear's Clock In: the gear reads it times three halves every frame, so it comes
/// round with the Phase, up to one and a half and back to zero.
#[test]
fn a_phase_cabled_into_clock_in_is_its_parent() {
    for slug in ["ratiogear", "mastergear"] {
        let mut app = App::headless();
        let source = add(&mut app, slug);
        if slug == "mastergear" {
            set(&mut app, source, "length", 1.0);
        }
        let follower = geared(&mut app, None, 3.0, 2.0);
        connect(&mut app, (source, "wrapped"), (follower, "clock"));
        let mut top = 0.0_f64;
        for _ in 0..90 {
            app.tick(FRAME);
            let phase = read(&app, source, "wrapped");
            let c = count(&app, follower);
            assert_eq!(c, phase * 3.0 / 2.0, "{slug}.wrapped times 3 : 2");
            top = top.max(c);
        }
        assert!(
            top > 1.45 && top < 1.5,
            "{slug}: up to one and a half, {top}"
        );
    }
}

/// A Master Gear's Cycles count its cycles and its Phase the fraction of the current one;
/// its Trigger fires on each whole one, where inside the frame it fell.
#[test]
fn a_master_gear_publishes_its_cycles_its_phase_and_a_beat_each_cycle() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let mut downs = 0;
    for _ in 0..150 {
        app.tick(FRAME);
        downs += app
            .edges(PortRef::new(clock, "trigger"))
            .iter()
            .filter(|e| e.is_down())
            .count();
    }
    let cycles = read(&app, clock, "cycles");
    let phase = read(&app, clock, "wrapped");
    assert!((cycles - 2.5).abs() < 0.02, "{cycles}");
    assert!((phase - cycles.fract()).abs() < 1e-4, "{phase}");
    assert_eq!(downs, 2, "a beat at one second and at two");
}

/// **A beat that falls on a frame fires on that frame, in every loop, whatever ran before.**
/// A render steps the playhead to exact frame times. A Master Gear of 0.375 s at 40 frames a
/// second beats on every fifteenth frame, and a ÷8 Ratio Gear on its Cycles on every hundred
/// and twentieth. A fortieth of a second is no binary fraction, so what the advances add up
/// to there is a whole number only to within `f64` rounding, a hair short on some of those
/// frames: each of those beats still fires on its own frame, never a frame late, however long
/// the live show ran before. A beat late on a loop's first frame is a loop that does not close.
#[test]
fn a_beat_on_a_frame_fires_on_that_frame_in_every_loop() {
    const FPS: f64 = 40.0;
    const WARM_UP: f64 = 12.0;
    for live in [0u32, 37, 150, 361, 20000] {
        let mut app = App::headless();
        let clock = master(&mut app, 0.375);
        let gear = geared(&mut app, Some(clock), 1.0, 8.0);
        ticks(&mut app, live);
        // As a render does: every node born again, the playhead sought to twelve seconds of
        // warm-up before frame zero — thirty-two beats, a whole number of the ÷8's cycles, so
        // the ÷8 is born on a downbeat where playing puts it — and each frame at its own time.
        app.reset_cpu();
        app.transport(Transport::Seek(-WARM_UP));
        let mut late = Vec::new();
        for n in 0..=480u32 {
            app.tick_at(f64::from(n) / FPS - WARM_UP);
            for (id, name, every) in [(clock, "master", 15), (gear, "÷8", 120)] {
                let fired = app
                    .edges(PortRef::new(id, "trigger"))
                    .iter()
                    .any(|e| e.is_down());
                if n > 0 && n % every == 0 && !fired {
                    late.push((name, n));
                }
            }
        }
        assert!(
            late.is_empty(),
            "after {live} live ticks, beats a frame late on frames {late:?}"
        );
    }
}

/// The Time node is the playhead: it moves with play, holds with a pause and jumps with a
/// seek.
#[test]
fn the_time_node_reads_the_playhead() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    ticks(&mut app, 30);
    let playhead = |app: &App| app.transport_state().playhead;
    assert!((read(&app, time, "seconds") - playhead(&app)).abs() < 1e-5);
    assert!(read(&app, time, "seconds") > 0.4);

    app.transport(Transport::Pause);
    let held = read(&app, time, "seconds");
    ticks(&mut app, 30);
    assert_eq!(read(&app, time, "seconds"), held, "paused, it holds");

    app.transport(Transport::Seek(42.0));
    app.tick(FRAME);
    assert_eq!(
        read(&app, time, "seconds"),
        42.0,
        "a seek is where it reads"
    );
    app.transport(Transport::Play);
}

/// **A gear counting another reads the same a loop later, to the bit.** A 1 : 1 and a 1 : 3
/// Ratio Gear counting a one-second Master Gear, played a frame at a time from 1022 s across 1024
/// cycles, where one `f32` of a count halves its precision: the fraction a shader reads of
/// each count, every frame, is the fraction it read one of its own cycles earlier, to the bit.
#[test]
fn a_gear_counting_another_reads_the_same_a_loop_later_to_the_bit() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let once = geared(&mut app, Some(clock), 1.0, 1.0);
    let third = geared(&mut app, Some(clock), 1.0, 3.0);
    app.tick(FRAME);
    let from = 1022.0;
    app.transport(Transport::Seek(from));
    let fraction = |app: &App, id| {
        let count = app.uniform(PortRef::new(id, "cycles")).unwrap();
        supersilvia::nodes::phasor::split(count)[1].to_bits()
    };
    let mut read = Vec::new();
    for i in 0..400 {
        app.tick_at(from + f64::from(i) / 60.0);
        read.push([
            fraction(&app, clock),
            fraction(&app, once),
            fraction(&app, third),
        ]);
    }
    assert!(
        app.uniform(PortRef::new(clock, "cycles")).unwrap() > 1024.5,
        "the master passed 1024"
    );
    for (k, (name, frames)) in [("the master", 60), ("×1", 60), ("÷3", 180)]
        .into_iter()
        .enumerate()
    {
        for i in 0..read.len() - frames {
            assert_eq!(
                read[i][k],
                read[i + frames][k],
                "{name} reads at frame {} what it read at frame {i}",
                i + frames
            );
        }
    }
}

/// **Seconds counts on.** Past 1260 s, 2520 s and a million, the Time node's Seconds goes on:
/// the number it publishes, which a Math node or a row reads, is the playhead to the bit, and a
/// Ratio Gear counting it moves a frame's time each frame.
#[test]
fn seconds_count_on_and_never_wrap() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    let gear = add(&mut app, "ratiogear");
    connect(&mut app, (time, "seconds"), (gear, "clock"));
    app.tick(FRAME);
    for at in [1259.5, 2519.5, 1.0e6] {
        app.transport(Transport::Seek(at));
        app.tick(FRAME);
        let mut last = count(&app, gear);
        for _ in 0..60 {
            app.tick(FRAME);
            let playhead = app.transport_state().playhead;
            assert_eq!(
                read(&app, time, "seconds"),
                playhead,
                "the number is the playhead"
            );
            let now = count(&app, gear);
            assert!(
                (now - last - f64::from(FRAME)).abs() < 1e-9,
                "a gear counting it moves a frame's time: {}",
                now - last
            );
            last = now;
        }
        assert!(app.transport_state().playhead > at + 0.9, "it passed {at}");
    }
}

/// An oscillator's position in its cycle, read back from a sawtooth: `2x` over the first
/// half of a cycle and `2x − 2` over the second.
fn sawtooth_phase(value: f64) -> f64 {
    if value >= 0.0 {
        value / 2.0
    } else {
        1.0 + value / 2.0
    }
}

fn sawtooth(app: &mut App, hz: f32) -> NodeId {
    let osc = add(app, "oscillator");
    choose(app, osc, "waveform", "sawtooth");
    // Its frequency is a gear's: a Ratio Gear on ambient seconds at `hz : 1`, into Time.
    let gear = geared(app, None, hz, 1.0);
    connect(app, (gear, "cycles"), (osc, "clock"));
    osc
}

/// Paused, an oscillator holds; a seek moves it by what the playhead moved at its gear's rate,
/// and it is where it would be had it played there.
#[test]
fn a_seek_moves_an_oscillator_by_its_rate_times_the_jump_and_pause_holds_it() {
    let mut app = App::headless();
    let osc = sawtooth(&mut app, 1.0);
    ticks(&mut app, 30);
    app.transport(Transport::Pause);
    app.tick(FRAME);
    let before = sawtooth_phase(read(&app, osc, "output"));
    ticks(&mut app, 60);
    assert_eq!(
        sawtooth_phase(read(&app, osc, "output")),
        before,
        "paused, it holds"
    );

    let at = app.transport_state().playhead;
    app.transport(Transport::Seek(at + 0.4));
    app.tick(FRAME);
    let after = sawtooth_phase(read(&app, osc, "output"));
    let moved = (after - before).rem_euclid(1.0);
    assert!((moved - 0.4).abs() < 1e-4, "1 Hz over 0.4 s: {moved}");
}

/// **Offset is added to Time**, in waves, either way: a sine a quarter of a wave on is a
/// cosine, and so is one three quarters of a wave back; at zero the wave is silvia's, a wave a
/// second on ambient time.
#[test]
fn the_oscillators_offset_is_added_to_its_time() {
    let mut app = App::headless();
    let sine = add(&mut app, "oscillator");
    set(&mut app, sine, "phaseOffset", 0.25);
    let back = add(&mut app, "oscillator");
    set(&mut app, back, "phaseOffset", -0.75);
    let cosine = add(&mut app, "oscillator");
    choose(&mut app, cosine, "waveform", "cosine");
    let plain = add(&mut app, "oscillator");
    for _ in 0..40 {
        app.tick(FRAME);
        let (a, b) = (read(&app, sine, "output"), read(&app, cosine, "output"));
        assert!((a - b).abs() < 1e-4, "{a} against {b}");
        let c = read(&app, back, "output");
        assert!((c - b).abs() < 1e-4, "{c} against {b}, from behind");
    }
    let t = app.transport_state().playhead;
    let expected = (t * std::f64::consts::TAU).sin();
    let got = read(&app, plain, "output");
    assert!((got - expected).abs() < 1e-3, "{got} against {expected}");
}

// ------------------------------------------------------------------------------- sync

/// A hand's press of one of `node`'s buttons: down for one tick, then let go.
fn tap(app: &mut App, node: NodeId, key: &'static str) {
    app.press(PortRef::new(node, key), true);
    app.tick(FRAME);
    app.press(PortRef::new(node, key), false);
}

/// The edges `node` fired on `port` on the frame just ticked: how many downs and how many ups.
fn fired(app: &App, node: NodeId, port: &'static str) -> (usize, usize) {
    let events = app.edges(PortRef::new(node, port));
    let downs = events.iter().filter(|e| e.is_down()).count();
    (downs, events.len() - downs)
}

/// Where a Master Gear `length` seconds long stands on the show's time: the playhead over its
/// length.
fn on_time(app: &App, length: f64) -> f64 {
    app.transport_state().playhead / length
}

/// **Sync puts a Master Gear back on the show's time, and lets go of Hold.** A two-second gear
/// held for three quarters of a second, Reset, or bent by a Length turned from two seconds to
/// a half reads the playhead over its Length to the bit on the frame Sync is pressed, and runs
/// on from there: a held one is let go.
#[test]
fn sync_puts_a_master_gear_back_on_the_shows_time() {
    // Held.
    let mut app = App::headless();
    let clock = master(&mut app, 2.0);
    ticks(&mut app, 30);
    tap(&mut app, clock, "hold");
    ticks(&mut app, 45);
    let held = count(&app, clock);
    assert!(
        on_time(&app, 2.0) - held > 0.3,
        "held, it falls behind the show: {held}"
    );
    tap(&mut app, clock, "sync");
    assert_eq!(count(&app, clock), on_time(&app, 2.0), "synced from a hold");
    ticks(&mut app, 10);
    let c = count(&app, clock);
    assert!(
        (c - on_time(&app, 2.0)).abs() < 1e-12,
        "and let go, it runs with the show: {c}"
    );

    // Reset.
    let mut app = App::headless();
    let clock = master(&mut app, 2.0);
    ticks(&mut app, 100);
    tap(&mut app, clock, "reset");
    ticks(&mut app, 7);
    assert!(count(&app, clock) < 0.1, "reset, it starts a cycle");
    tap(&mut app, clock, "sync");
    assert_eq!(
        count(&app, clock),
        on_time(&app, 2.0),
        "synced from a reset"
    );

    // Bent by a Length turned.
    let mut app = App::headless();
    let clock = master(&mut app, 2.0);
    ticks(&mut app, 40);
    set(&mut app, clock, "length", 0.5);
    ticks(&mut app, 20);
    let bent = count(&app, clock);
    assert!(
        (on_time(&app, 0.5) - bent).abs() > 0.5,
        "a Length turned bends it from where it was: {bent}"
    );
    tap(&mut app, clock, "sync");
    assert_eq!(
        count(&app, clock),
        on_time(&app, 0.5),
        "synced at the new length"
    );
}

/// **A synced gear is where a gear born that frame is, to the bit, at any Length.** A Master
/// Gear of 0.7 s held for a while, and one of the same length made on the frame the first is
/// synced, which is born at the playhead over its length: the two read the same bits, and keep
/// agreeing after.
#[test]
fn a_synced_gear_reads_what_a_gear_born_then_reads() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.7);
    ticks(&mut app, 20);
    tap(&mut app, clock, "hold");
    ticks(&mut app, 33);
    let twin = master(&mut app, 0.7);
    tap(&mut app, clock, "sync");
    assert_eq!(count(&app, clock), count(&app, twin));
    for _ in 0..30 {
        app.tick(FRAME);
        let (a, b) = (count(&app, clock), count(&app, twin));
        assert!((a - b).abs() < 1e-12, "running together: {a} and {b}");
    }
}

/// **Sync is a jump: a beat only where it lands on a whole cycle, and never a second one.** On
/// a paused show at three seconds, a one-second Master Gear Reset and then Synced lands on its
/// third cycle and fires that downbeat; one Synced on its way to three and a half fires none,
/// and closes the gate the Reset's beat opened. A Sync with nothing to fix — the gear already
/// on the show's time, on a whole cycle or between two, paused or playing — moves nothing and
/// fires nothing.
#[test]
fn sync_fires_a_downbeat_only_on_a_whole_cycle() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    ticks(&mut app, 10);
    app.transport(Transport::Pause);
    app.transport(Transport::Seek(3.0));
    app.tick(FRAME);
    assert_eq!(count(&app, clock), 3.0);
    assert_eq!(
        fired(&app, clock, "trigger"),
        (1, 0),
        "born on its third beat"
    );
    app.tick(FRAME);

    tap(&mut app, clock, "sync");
    assert_eq!(count(&app, clock), 3.0);
    assert_eq!(
        fired(&app, clock, "trigger"),
        (0, 0),
        "a Sync with nothing to fix fires nothing, on a whole cycle too"
    );

    tap(&mut app, clock, "reset");
    assert_eq!(count(&app, clock), 0.0);
    assert_eq!(fired(&app, clock, "trigger").0, 1, "a Reset is a beat");
    tap(&mut app, clock, "sync");
    assert_eq!(count(&app, clock), 3.0);
    assert_eq!(
        fired(&app, clock, "trigger"),
        (1, 1),
        "landing on a whole cycle, Sync fires its downbeat, retriggering the Reset's"
    );

    app.transport(Transport::Seek(3.5));
    app.tick(FRAME);
    tap(&mut app, clock, "reset");
    assert_eq!(fired(&app, clock, "trigger").0, 1, "a Reset opens the gate");
    tap(&mut app, clock, "sync");
    assert_eq!(count(&app, clock), 3.5);
    assert_eq!(
        fired(&app, clock, "trigger"),
        (0, 1),
        "landing mid-cycle, Sync fires no beat and closes the gate it found open"
    );
    tap(&mut app, clock, "sync");
    assert_eq!(
        fired(&app, clock, "trigger"),
        (0, 0),
        "and a second Sync, nothing"
    );

    // Playing, a gear never moved off the show's time is on it, to within rounding of the
    // advances it integrated: a Sync there fires only what the frame's motion carries it past.
    app.transport(Transport::Play);
    for _ in 0..200 {
        let before = count(&app, clock);
        tap(&mut app, clock, "sync");
        let after = count(&app, clock);
        assert_eq!(after, on_time(&app, 1.0));
        let passed = (after.floor() - before.floor()) as usize;
        assert_eq!(
            fired(&app, clock, "trigger").0,
            passed,
            "from {before} to {after}, the beats it passed and no other"
        );
        app.tick(FRAME);
    }
}

/// **A Ratio Gear below a Sync lands on its parent times its Teeth at once.** A 3 : 4 gear
/// under a one-second Master Gear held for two thirds of a second reads the master's synced
/// count times three quarters, to the bit, on the frame of the Sync, which is the playhead
/// times three quarters; and fires no more than the one downbeat, where a run of the cycles
/// in between would fire one for each.
#[test]
fn a_ratio_gear_follows_a_sync_at_once() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = geared(&mut app, Some(clock), 3.0, 4.0);
    ticks(&mut app, 50);
    tap(&mut app, clock, "hold");
    ticks(&mut app, 200);
    tap(&mut app, clock, "sync");
    let m = count(&app, clock);
    assert_eq!(m, on_time(&app, 1.0));
    assert_eq!(count(&app, gear), m * 3.0 / 4.0, "the parent times 3 ÷ 4");
    assert!(
        fired(&app, gear, "trigger").0 <= 1,
        "two and a half cycles over in one frame fire at most the downbeat"
    );
}

/// **A sequencer below a Sync takes it as a jump, and a Sync with nothing to fix as nothing.**
/// A Euclidean rhythm whose every step is a pulse, on a Master Gear a bar of one second long:
/// held for half a second and Synced, the gear moves eight steps on in a frame, and the
/// sequencer plays none of them on the way. Running on the show's time, with lane 1's gate
/// open mid-step, a Sync says no jump: the gate stays open, where a jump would close it.
#[test]
fn a_sequencer_takes_a_sync_as_a_jump() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let rhythm = add(&mut app, "euclideanrhythm");
    set(&mut app, rhythm, "lane1steps", 2.0);
    set(&mut app, rhythm, "lane1pulses", 2.0);
    connect(&mut app, (clock, "cycles"), (rhythm, "clock"));
    ticks(&mut app, 37);
    tap(&mut app, clock, "hold");
    ticks(&mut app, 30);
    tap(&mut app, clock, "sync");
    let (downs, _) = fired(&app, rhythm, "lane1");
    assert!(
        downs <= 1,
        "eight steps over in one frame play none on the way, not {downs}"
    );

    // Run on until lane 1 is open a little way into a step, where a frame's motion reaches
    // neither the gate's end nor the next step.
    let mut open = false;
    let mut found = false;
    for _ in 0..240 {
        app.tick(FRAME);
        for event in app.edges(PortRef::new(rhythm, "lane1")) {
            open = event.is_down();
        }
        let into = (16.0 * count(&app, clock)).fract();
        if open && (0.02..0.2).contains(&into) {
            found = true;
            break;
        }
    }
    assert!(found, "lane 1 open early in a step");
    tap(&mut app, clock, "sync");
    assert_eq!(
        fired(&app, rhythm, "lane1"),
        (0, 0),
        "a Sync with nothing to fix is no jump: the open gate stays open"
    );
}

/// **Moments in one frame keep their order.** Two Ratio Gears on ambient seconds, their
/// Offsets putting their Triggers four and ten thousandths of a second after each whole
/// second, fire into a Master Gear's buttons inside one frame. Reset then Sync ends the frame
/// on the show's time, to the bit; Sync then Reset ends it at the start of a cycle; Sync then
/// Hold ends it held where the Sync put it, at the playhead of the Hold's moment.
#[test]
fn sync_keeps_its_place_among_a_frames_moments() {
    let rig = |first: &'static str, second: &'static str| {
        let mut app = App::headless();
        let clock = master(&mut app, 0.5);
        let early = geared(&mut app, None, 1.0, 1.0);
        set(&mut app, early, "phaseOffset", -0.004);
        let late = geared(&mut app, None, 1.0, 1.0);
        set(&mut app, late, "phaseOffset", -0.010);
        connect(&mut app, (early, "trigger"), (clock, first));
        connect(&mut app, (late, "trigger"), (clock, second));
        // Hold the master off the show's time, so the Sync has something to fix.
        ticks(&mut app, 20);
        tap(&mut app, clock, "hold");
        ticks(&mut app, 10);
        tap(&mut app, clock, "hold");
        // Up to the frame both fire in, the one that passes a second and ten thousandths.
        loop {
            app.tick(FRAME);
            if fired(&app, late, "trigger").0 > 0 && app.transport_state().playhead > 1.0 {
                break;
            }
        }
        assert!(
            fired(&app, early, "trigger").0 > 0,
            "both fire in one frame"
        );
        (app, clock)
    };

    let (app, clock) = rig("reset", "sync");
    assert_eq!(
        count(&app, clock),
        on_time(&app, 0.5),
        "Reset then Sync: synced"
    );

    let (app, clock) = rig("sync", "reset");
    let c = count(&app, clock);
    assert!(c < 0.05, "Sync then Reset: at the start of a cycle, {c}");

    let (mut app, clock) = rig("sync", "hold");
    let c = count(&app, clock);
    let hold = (1.010 / 0.5, on_time(&app, 0.5));
    assert!(
        (c - hold.0).abs() < 1e-3 && c < hold.1,
        "Sync then Hold: held where the Sync put it, at the Hold's moment: {c}"
    );
    ticks(&mut app, 5);
    assert_eq!(count(&app, clock), c, "and held");
}
