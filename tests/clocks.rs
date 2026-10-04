// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the gears and the clocks. A Master Gear at a length, a Ratio Gear turning one
//! clock into another and landing a change on the input's next whole cycle, the chain a loop
//! closes on, Hold and Reset, a seek re-birthing them, a beat on a render's frame firing on it,
//! the Time node reading the playhead, and the oscillator on the transport. See
//! `docs/cpu.md#gears` and `proposals/time.md`.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::nodes::gear::ladder;
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

fn read(app: &App, node: NodeId, key: &'static str) -> f32 {
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

/// A Ratio Gear at `ratio`, its Clock In on `from`'s Cycles where it names one.
fn ratio(app: &mut App, from: Option<NodeId>, ratio: f32) -> NodeId {
    let id = add(app, "ratiogear");
    set(app, id, "ratio", ratio);
    if let Some(from) = from {
        connect(app, (from, "cycles"), (id, "clock"));
    }
    id
}

/// A Ratio Gear on a Master Gear's Cycles counts in its cycles: ÷4 of a beat is a bar. The
/// beat turned from half a second to a third, the bar bends — its slope changes and its value
/// does not jump — because the Cycles it reads are an integral of the length.
#[test]
fn a_ratio_gear_on_a_master_bends_on_a_length_change() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.5);
    let bar = ratio(&mut app, Some(clock), 0.25);

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
    assert!((before - 0.25 * 2.0 * FRAME).abs() < 1e-4, "{before}");
    let at = read(&app, bar, "cycles");
    set(&mut app, clock, "length", 1.0 / 3.0);
    app.tick(FRAME);
    let step = read(&app, bar, "cycles") - at;
    assert!(
        step > 0.0 && step < 0.25 * 3.0 * FRAME + 1e-4,
        "no jump across the change, one frame's motion at most: {step}"
    );
    ticks(&mut app, 2);
    let after = slope(&mut app);
    assert!(
        (after - 0.25 * 3.0 * FRAME).abs() < 1e-4,
        "and a steeper slope after it: {after}"
    );
}

/// **A ratio change lands on the input's next whole cycle.** A ×1 gear on a one-second Master
/// Gear, turned to ×2 at 1.3 s, runs on at ×1 — the change pending — until the master's
/// second cycle begins at 2 s, and at ×2 from there: its downbeat lands on the master's.
#[test]
fn a_ratio_change_lands_on_the_inputs_next_whole_cycle() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let gear = ratio(&mut app, Some(clock), 1.0);
    ticks(&mut app, 78);
    set(&mut app, gear, "ratio", 2.0);
    ticks(&mut app, 30);
    let input = f64::from(read(&app, clock, "cycles"));
    let output = f64::from(read(&app, gear, "cycles"));
    assert!(input > 1.7 && input < 2.0, "{input}");
    assert!(
        (output - input).abs() < 0.03,
        "until the master's next cycle it runs at the old ratio: {output} against {input}"
    );
    ticks(&mut app, 30);
    let input = f64::from(read(&app, clock, "cycles"));
    let output = f64::from(read(&app, gear, "cycles"));
    assert!(input > 2.2, "{input}");
    let landed = input.floor();
    assert!(
        (output - (landed + 2.0 * (input - landed))).abs() < 0.03,
        "from the whole cycle on, twice as fast: {output} at {input}"
    );
    // The downbeat is the master's: where the master is at a whole cycle, so is the gear.
    let mut both = 0;
    for _ in 0..120 {
        let (a, b) = (read(&app, clock, "cycles"), read(&app, gear, "cycles"));
        app.tick(FRAME);
        let (c, d) = (read(&app, clock, "cycles"), read(&app, gear, "cycles"));
        if c.floor() > a.floor() {
            assert!(d.floor() > b.floor(), "a master beat is a gear beat");
            both += 1;
        }
    }
    assert!(both >= 1);
}

/// **×3 of ÷3 is the input**: two Ratio Gears in a chain give back the Master Gear's cycles,
/// and ÷3 alone comes round once every three.
#[test]
fn three_times_a_third_is_the_input() {
    let mut app = App::headless();
    let clock = master(&mut app, 0.5);
    let third = ratio(&mut app, Some(clock), 1.0 / 3.0);
    let back = ratio(&mut app, Some(third), 3.0);
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
    let gear = ratio(&mut app, Some(early), 3.0);
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
    let once = ratio(&mut app, Some(clock), 1.0);
    let quarter = ratio(&mut app, Some(clock), 0.25);
    let ambient = ratio(&mut app, None, 1.0);
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
    let quarter = ratio(&mut app, Some(clock), 0.25);
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
    let gear = ratio(&mut app, Some(fast), 1.0);
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
    let gear = ratio(&mut app, Some(clock), 1.0);
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
/// its Reset beat; a follower at ×1, ×2 or ÷4 is born again just past its own downbeat — the
/// ÷4 too, since a master at zero is a ÷4 at zero — and fires that one down on the same frame.
#[test]
fn a_hand_reset_of_the_clock_is_a_beat_to_its_followers() {
    for r in [1.0, 2.0, 0.25] {
        let mut app = App::headless();
        let clock = master(&mut app, 1.0);
        let gear = ratio(&mut app, Some(clock), r);
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
/// and a follower at ×1, ×2 or ÷4 is born again where the master now is and fires its one
/// downbeat on the frame the master fires its own. A Ratio Gear at −×1 upstream counts
/// backwards, and a follower on it counts backwards with it, a beat a cycle and never a
/// birth; its own Reset half a cycle in, a step forwards, is a beat to that follower as well.
#[test]
fn a_reset_inside_the_first_cycle_is_a_beat_to_the_followers() {
    let downs = |app: &App, node: NodeId| {
        app.edges(PortRef::new(node, "trigger"))
            .iter()
            .filter(|e| e.is_down())
            .count()
    };
    for r in [1.0_f32, 2.0, 0.25] {
        let mut app = App::headless();
        let clock = master(&mut app, 1.0);
        let gear = ratio(&mut app, Some(clock), r);
        ticks(&mut app, 30);
        let reset = PortRef::new(clock, "reset");
        app.press(reset, true);
        app.tick(FRAME);
        app.press(reset, false);
        assert_eq!(downs(&app, clock), 1, "the master's Reset is a beat");
        assert_eq!(downs(&app, gear), 1, "×{r}: and its follower's");
        let (g, m) = (read(&app, gear, "cycles"), read(&app, clock, "cycles"));
        assert!(
            (g - r * m).abs() < 1e-5,
            "×{r}: born where the master is: {g} and {m}"
        );
    }

    let mut app = App::headless();
    let back = ratio(&mut app, None, -1.0);
    let follower = ratio(&mut app, Some(back), 1.0);
    ticks(&mut app, 10);
    let offset = read(&app, follower, "cycles") - read(&app, back, "cycles");
    let mut beats = 0;
    for _ in 0..150 {
        app.tick(FRAME);
        assert_eq!(
            downs(&app, follower),
            downs(&app, back),
            "a beat of the backwards clock is a beat of its follower"
        );
        beats += downs(&app, follower);
        let d = read(&app, follower, "cycles") - read(&app, back, "cycles");
        assert!(
            (d - offset).abs() < 1e-4,
            "counting backwards with it, never born again: {d} against {offset}"
        );
    }
    assert_eq!(beats, 2, "two and a half seconds back is two whole cycles");
    ticks(&mut app, 25);
    let reset = PortRef::new(back, "reset");
    app.press(reset, true);
    app.tick(FRAME);
    app.press(reset, false);
    assert_eq!(downs(&app, back), 1, "the backwards gear's Reset is a beat");
    assert_eq!(downs(&app, follower), 1, "and its follower's");
}

/// A count's one `f32` wraps, and a Time reads past the wrap: a gear's Cycles read as one
/// `f32` step from 1260 to −1260, and a Ratio Gear counting them keeps going the same way, at a
/// ratio whose denominator divides 2520 or not — ×½, ÷7, ÷11, backwards at ÷7 — each frame
/// its share of the clock's motion to a billionth, since it reads the count whole.
#[test]
fn a_cabled_clock_passes_its_wrap_with_no_seam() {
    let mut app = App::headless();
    // Ambient seconds at ×20: 1260 cycles in 63 seconds.
    let fast = ratio(&mut app, None, 20.0);
    let rates = [0.5, 1.0 / 7.0, 1.0 / 11.0, -1.0 / 7.0];
    let followers: Vec<NodeId> = rates
        .iter()
        .map(|&r| ratio(&mut app, Some(fast), r))
        .collect();
    app.tick(FRAME);
    app.transport(Transport::Seek(62.9));
    app.tick(FRAME);
    let count = |app: &App, id| app.count(PortRef::new(id, "cycles")).unwrap();
    let mut last: Vec<f64> = followers.iter().map(|&id| count(&app, id)).collect();
    let mut wrapped = false;
    for _ in 0..30 {
        let before = read(&app, fast, "cycles");
        app.tick(FRAME);
        let after = read(&app, fast, "cycles");
        if after < before {
            wrapped = true;
            assert!(
                before > 1259.0 && after < -1259.0,
                "the count wraps from 1260 to −1260: {before} to {after}"
            );
        }
        for ((id, rate), last) in followers.iter().zip(rates).zip(&mut last) {
            let now = count(&app, *id);
            let d = now - *last;
            assert!(
                (d - ladder::exact(f64::from(rate)) * 20.0 * f64::from(FRAME)).abs() < 1e-9,
                "×{rate} moves by its share of the clock each frame, across the wrap too: {d}"
            );
            *last = now;
        }
    }
    assert!(wrapped, "the clock did wrap");
}

/// **A count just below zero reads as a small negative.** A Ratio Gear reset and then stepped
/// back a hundredth of a cycle by its Clock In publishes −0.01, not 2519.99: the wrap is
/// centered on zero, where an `f32` resolves finest.
#[test]
fn a_ratio_gear_reset_and_stepped_back_reads_a_small_negative() {
    let mut app = App::headless();
    let input = add(&mut app, "number");
    set(&mut app, input, "value", 5.0);
    let gear = add(&mut app, "ratiogear");
    connect(&mut app, (input, "output"), (gear, "clock"));
    ticks(&mut app, 3);
    let reset = PortRef::new(gear, "reset");
    app.press(reset, true);
    app.tick(FRAME);
    app.press(reset, false);
    app.tick(FRAME);
    assert_eq!(read(&app, gear, "cycles"), 0.0, "a reset starts a cycle");
    set(&mut app, input, "value", 4.99);
    app.tick(FRAME);
    let c = read(&app, gear, "cycles");
    assert!((c + 0.01).abs() < 1e-5, "a hundredth back from zero: {c}");
    assert!(
        (read(&app, gear, "wrapped") - 0.99).abs() < 1e-5,
        "and its Phase is 0.99, as it always was"
    );
}

/// **The Time node's Seconds and a Master Gear's Cycles before the playhead's zero** read as
/// the negative they are, to zero's precision: a render's warm-up at −0.01 s is −0.01 s, and
/// a four-second master there is −0.0025 cycles, where 2520 less a little would step in
/// quarter-thousandths of a cycle.
#[test]
fn a_clock_before_zero_reads_negative_at_zeros_precision() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    let clock = master(&mut app, 4.0);
    app.tick_at(-1.0);
    app.tick_at(-0.01);
    let seconds = read(&app, time, "seconds");
    assert_eq!(seconds, -0.01_f32, "Seconds, to the bit");
    let cycles = read(&app, clock, "cycles");
    assert!((f64::from(cycles) + 0.0025).abs() < 1e-9, "{cycles}");
}

/// **A 0..1 Phase cabled into Clock In runs forward across its wrap.** A gear's Phase
/// declares that it wraps at one, and Clock In unwraps it there, so a gear following one at
/// ×1 counts the cycles it passed: three and a half in three and a half seconds.
#[test]
fn a_phase_cabled_into_clock_in_runs_forward_across_its_wrap() {
    for slug in ["ratiogear", "mastergear"] {
        let mut app = App::headless();
        let source = add(&mut app, slug);
        if slug == "mastergear" {
            set(&mut app, source, "length", 1.0);
        }
        let follower = ratio(&mut app, None, 1.0);
        connect(&mut app, (source, "wrapped"), (follower, "clock"));
        app.tick(FRAME);
        let start = read(&app, follower, "cycles");
        ticks(&mut app, 210);
        let counted = read(&app, follower, "cycles") - start;
        assert!(
            (counted - 3.5).abs() < 0.05,
            "{slug}.wrapped: three and a half cycles in three and a half seconds, not {counted}"
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
        let gear = ratio(&mut app, Some(clock), 0.125);
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

/// Hold is a toggle: the gear stands where it is until it is pressed again, and runs on from
/// there.
#[test]
fn hold_freezes_a_ratio_gear_until_it_is_pressed_again() {
    let mut app = App::headless();
    let gear = ratio(&mut app, None, 1.0);
    ticks(&mut app, 30);
    let hold = PortRef::new(gear, "hold");
    app.press(hold, true);
    app.tick(FRAME);
    app.press(hold, false);
    let held = read(&app, gear, "cycles");
    ticks(&mut app, 60);
    assert_eq!(read(&app, gear, "cycles"), held, "held");
    app.press(hold, true);
    app.tick(FRAME);
    app.press(hold, false);
    ticks(&mut app, 30);
    let moved = read(&app, gear, "cycles") - held;
    assert!(
        (moved - 31.0 * FRAME).abs() < 0.02,
        "and on from there: {moved}"
    );
}

/// The Time node is the playhead: it moves with play, holds with a pause and jumps with a
/// seek.
#[test]
fn the_time_node_reads_the_playhead() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    ticks(&mut app, 30);
    let playhead = |app: &App| app.transport_state().playhead as f32;
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

/// **A gear counting another reads the same a loop later, to the bit.** A ×1 and a ÷3 Ratio
/// Gear counting a one-second Master Gear, played a frame at a time from 1022 s across 1024
/// cycles, where one `f32` of a count halves its precision: the fraction a shader reads of
/// each count, every frame, is the fraction it read one of its own cycles earlier, to the bit.
#[test]
fn a_gear_counting_another_reads_the_same_a_loop_later_to_the_bit() {
    let mut app = App::headless();
    let clock = master(&mut app, 1.0);
    let once = ratio(&mut app, Some(clock), 1.0);
    let third = ratio(&mut app, Some(clock), 1.0 / 3.0);
    app.tick(FRAME);
    let from = 1022.0;
    app.transport(Transport::Seek(from));
    let fraction = |app: &App, id| {
        let count = app.count(PortRef::new(id, "cycles")).unwrap();
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
        app.count(PortRef::new(clock, "cycles")).unwrap() > 1024.5,
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

/// **Seconds counts on.** Past 1260 s, where a count read as one `f32` once stepped to −1260,
/// the Time node's Seconds goes on: read whole it is the playhead to the bit, a Ratio Gear
/// counting it moves a frame's time each frame, and its one `f32` — what a Math node or a row
/// reads — is the playhead to an `f32`'s step there, an eighth of a millisecond.
#[test]
fn seconds_count_on_past_1260() {
    let mut app = App::headless();
    let time = add(&mut app, "time");
    let gear = add(&mut app, "ratiogear");
    connect(&mut app, (time, "seconds"), (gear, "clock"));
    app.tick(FRAME);
    app.transport(Transport::Seek(1259.5));
    app.tick(FRAME);
    let seconds = PortRef::new(time, "seconds");
    let cycles = PortRef::new(gear, "cycles");
    let mut last = app.count(cycles).unwrap();
    for _ in 0..60 {
        app.tick(FRAME);
        let playhead = app.transport_state().playhead;
        assert_eq!(
            app.count(seconds),
            Some(playhead),
            "read whole, it is the playhead"
        );
        let one = f64::from(read(&app, time, "seconds"));
        assert!(
            (one - playhead).abs() < 1.5e-4,
            "as one f32 it counts on too: {one} at {playhead}"
        );
        let now = app.count(cycles).unwrap();
        assert!(
            (now - last - f64::from(FRAME)).abs() < 1e-9,
            "a gear counting it moves a frame's time: {}",
            now - last
        );
        last = now;
    }
    assert!(app.transport_state().playhead > 1260.4, "it passed 1260 s");
}

/// An oscillator's position in its cycle, read back from a sawtooth: `2x` over the first
/// half of a cycle and `2x − 2` over the second.
fn sawtooth_phase(value: f32) -> f32 {
    if value >= 0.0 {
        value / 2.0
    } else {
        1.0 + value / 2.0
    }
}

fn sawtooth(app: &mut App, hz: f32) -> NodeId {
    let osc = add(app, "oscillator");
    choose(app, osc, "waveform", "sawtooth");
    // Its frequency is a gear's: a Ratio Gear on ambient seconds at `hz`, into Time.
    let gear = ratio(app, None, hz);
    connect(app, (gear, "cycles"), (osc, "clock"));
    osc
}

/// Paused, an oscillator holds; a seek moves it by what the playhead moved at its gear's rate,
/// and it is where it would be had it played there.
#[test]
fn a_seek_moves_an_oscillator_by_its_rate_times_the_jump_and_pause_holds_it() {
    let mut app = App::headless();
    let osc = sawtooth(&mut app, 0.5);
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
    assert!((moved - 0.2).abs() < 1e-4, "0.5 Hz over 0.4 s: {moved}");
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
    let expected = (t * std::f64::consts::TAU).sin() as f32;
    let got = read(&app, plain, "output");
    assert!((got - expected).abs() < 1e-3, "{got} against {expected}");
}
