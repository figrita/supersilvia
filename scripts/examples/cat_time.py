# SPDX-License-Identifier: AGPL-3.0-or-later
"""Time and gears: Speed and Offset, Free and Loop, one Master Gear for everything, Ratio Gears,
a loop that renders seamless, the playhead beside the time of day, copies a phase apart, and a
gear's Trigger."""

from timekit import CANDY, EMBER, INK, RAINBOW, gear, trail

ORDER = 7
CATEGORY = "Time and gears"


def speed_and_offset(new):
    e = new("Speed and Offset", "A Julia set drifting on its own Speed, and its Offset to place it")
    e.note(
        40,
        40,
        "Everything that moves has a Timing heading, open here on the Julia Set, whose Time slides "
        "the rainbow round the set's orbits. Speed is how fast it runs, a multiple of its own pace: 2 is twice as fast, 0 holds it still and -1 runs it "
        "backwards. Offset is where it is in its cycle, 0 to 1. Set Speed to 0 and drag Offset to "
        "scrub through the whole drift by hand.",
    )
    world = e.add("worldcoordinates", 0, 0)
    rainbow = e.add("cosinegradient", 0, 0, controls=RAINBOW)
    julia = e.add(
        "juliaset",
        0,
        0,
        controls={"zoom": 0.9, "iterations": 120.0, "speed": 1.0, "strength": 0.6},
        options={"timing": "on"},
    )
    vig = e.add("vignette", 0, 0, controls={"strength": 0.6, "radius": 0.9})
    e.cable(world, "x", rainbow, "t")
    e.cable(rainbow, "output", julia, "input")
    e.cable(julia, "map", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def free_and_loop(new):
    e = new("Free and Loop", "One dot runs free with its Speed bent, the other is locked to a gear")
    e.note(
        40,
        40,
        "Two Oscillators bob two dots, and the Output's own last frame, slid left, draws their "
        "paths. The top one is Free, and an Oscillator swings its Speed, so it rushes and lags; "
        "the bottom one is Loop, its Time a gear's count, exactly four waves a loop. Both do four "
        "waves a loop, so they meet again at the end. Click Free or Loop on either Timing heading "
        "to swap them.",
    )
    master = e.master(0, 0, length=4.0, display="gears")
    four = gear(e, 4, 1, clock=master)
    swing = e.add("oscillator", 0, 0, controls={"amplitude": 0.6, "offset": 1.0}, options={"trace": "off"})
    e.cable(master, "cycles", swing, "clock")
    free = e.add("oscillator", 0, 0, controls={"amplitude": 0.32, "offset": 0.48}, options={"timing": "on"})
    e.cable(swing, "output", free, "speed")
    locked = e.add("oscillator", 0, 0, controls={"amplitude": 0.32, "offset": -0.48}, options={"timing": "on"})
    e.cable(four, "cycles", locked, "clock")
    top = e.add("circle", 0, 0, controls={"radius": 0.07, "softness": 0.03, "centerX": 1.35, "foreground": "#ffb347", "background": "#000000"})
    bottom = e.add("circle", 0, 0, controls={"radius": 0.07, "softness": 0.03, "centerX": 1.35, "foreground": "#5ad1ff"})
    e.cable(free, "output", top, "centerY")
    e.cable(locked, "output", bottom, "centerY")
    e.cable(top, "color", bottom, "background")
    out = e.output(0, 0)
    blend = trail(e, out, (bottom, "color"), fade=0.985, slide=(-0.025, 0.0))
    e.cable(blend, "output", out, "input")
    return e


def one_master(new):
    e = new("One Master Gear", "One clock drives the noise, the palette and the turn, so they move as one")
    e.note(
        40,
        40,
        "One Master Gear, four seconds long, drives three things: its Cycles walk the noise a cell "
        "and slide the palette once a loop, and its Phase turns the kaleidoscope once. Set the "
        "gear's Length to 2 and all three go twice as fast, together. Press Hold and everything "
        "stops on the same frame; Sync puts it back on the show's time.",
    )
    master = e.master(0, 0, display="gears")
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 1.6, "octaves": 4.0, "gain": 0.55, "contrast": 1.2},
        options={"type": "ridged", "repeat": "1"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=RAINBOW)
    spin = e.add("rotate", 0, 0)
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0, "zoom": 0.8})
    vig = e.add("vignette", 0, 0, controls={"strength": 0.8, "radius": 0.85})
    e.cable(master, "cycles", noise, "clock")
    e.cable(master, "cycles", grad, "clock")
    e.cable(master, "wrapped", spin, "angle")
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", spin, "input")
    e.cable(spin, "output", kal, "input")
    e.cable(kal, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def orbit(e, clock, radius, color, background):
    """A dot going round a circle once a cycle of `clock`, on a Sine and a Cosine Oscillator."""
    x = e.add("oscillator", 0, 0, controls={"amplitude": radius}, options={"trace": "off"})
    y = e.add("oscillator", 0, 0, controls={"amplitude": radius}, options={"waveform": "cosine", "trace": "off"})
    e.cable(clock, "cycles", x, "clock")
    e.cable(clock, "cycles", y, "clock")
    dot = e.add("circle", 0, 0, controls={"radius": 0.06, "softness": 0.03, "foreground": color, "background": background})
    e.cable(x, "output", dot, "centerX")
    e.cable(y, "output", dot, "centerY")
    return dot


def three_against_two(new):
    e = new("Three against two", "Two Ratio Gears on one Master Gear: three turns against two")
    e.note(
        40,
        40,
        "Two Ratio Gears count the same Master Gear: Teeth 3 : 1 turn the outer comet three times a "
        "loop and 2 : 1 turn the inner one twice, three against two. They only meet at the top, "
        "once a loop. Change either gear's Teeth, to 5 : 1 say, and it lands at once where it would "
        "be had it always run so; click Reverse to send it the other way.",
    )
    master = e.master(0, 0, display="gears")
    three = gear(e, 3, 1, clock=master)
    two = gear(e, 2, 1, clock=master)
    outer = orbit(e, three, 0.78, "#ffb347", "#000000")
    inner = orbit(e, two, 0.45, "#ff5fa2", "#000000")
    both = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(outer, "color", both, "background")
    e.cable(inner, "color", both, "foreground")
    out = e.output(0, 0)
    blend = trail(e, out, (both, "output"), fade=0.95)
    e.cable(blend, "output", out, "input")
    return e


def seamless(new):
    e = new("A loop you can render", "Noise that repeats, on one gear, so the render closes to the frame")
    e.note(
        40,
        40,
        "Noise never repeats unless its Repeat says so. Here the Fractal and the Domain Warp both "
        "repeat every cell and the Master Gear moves them a cell a loop, so its caption says the "
        "loop closes in one cycle, 4 s: render the Output for that long and it plays round with no "
        "seam. Set the Fractal's Repeat to Never and the caption says it will not close.",
    )
    master = e.master(0, 0)
    frac = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 1.4, "octaves": 3.0, "gain": 0.5, "contrast": 1.1},
        options={"type": "ridged", "repeat": "1"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=EMBER)
    warp = e.add(
        "domainwarp",
        0,
        0,
        controls={"amplitude": 0.3, "frequency": 1.2, "octaves": 2.0},
        options={"repeat": "1"},
    )
    vig = e.add("vignette", 0, 0, controls={"strength": 0.7, "radius": 0.95})
    e.cable(master, "cycles", frac, "clock")
    e.cable(master, "cycles", warp, "clock")
    e.cable(master, "cycles", grad, "clock")
    e.cable(frac, "value", grad, "t")
    e.cable(grad, "output", warp, "input")
    e.cable(warp, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def hand(e, angle, length, width, color, background):
    """A clock hand: a dot stretched long and thin, slid up so it starts at the middle, turned
    by `angle` in turns. Stretch/Skew scales the sampling coordinate, so the dot is drawn as
    wide as its Stretch X divides it to and as long as its Stretch Y does."""
    r = round(width * 1.4, 4)
    dot = e.add("circle", 0, 0, controls={"radius": r, "softness": 0.01, "foreground": color, "background": background})
    thin = e.add("stretchskew", 0, 0, controls={"stretchX": 2.8, "stretchY": round(r / (length / 2), 4)})
    up = e.add("translate", 0, 0, controls={"y": length / 2})
    turn = e.add("rotate", 0, 0)
    e.cable(dot, "color", thin, "input")
    e.cable(thin, "output", up, "input")
    e.cable(up, "output", turn, "input")
    e.cable(angle[0], angle[1], turn, "angle")
    return turn


def show_and_wall(new):
    e = new("Show time and wall time", "The Time node's playhead and the Clock's time of day, each turning a hand")
    e.note(
        40,
        40,
        "The gold hand is the Time node, the show's playhead in seconds, a turn every four "
        "seconds through its Multiply. The white one is the Clock, the time of day, "
        "sweeping once a minute. Pause the show with the button by the time readout, or F8: the "
        "gold hand stops and the white one carries on. Back to zero puts only the gold one at the top.",
    )
    time = e.add("time", 0, 0)
    # Rotate turns a positive angle anticlockwise, so both hands are turned by minus theirs.
    quarter = e.add("multiply", 0, 0, controls={"b": -0.25})
    e.cable(time, "seconds", quarter, "a")
    wall = e.add("clock", 0, 0, options={"mode": "seconds"})
    sweep = e.add("multiply", 0, 0, controls={"b": -1.0})
    e.cable(wall, "normalized", sweep, "a")
    face = e.add("circle", 0, 0, controls={"radius": 0.92, "softness": 0.01, "foreground": "#1d1840", "background": INK})
    ring = e.add("circle", 0, 0, controls={"radius": 0.06, "softness": 0.02, "foreground": "#ffffff"})
    e.cable(face, "color", ring, "background")
    show = hand(e, (quarter, "output"), 0.55, 0.05, "#ffc04d", "#000000")
    minute = hand(e, (sweep, "output"), 0.82, 0.03, "#f4f1ff", "#000000")
    over = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(ring, "color", over, "background")
    e.cable(minute, "output", over, "foreground")
    top = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(over, "output", top, "background")
    e.cable(show, "output", top, "foreground")
    e.cable(top, "output", e.output(0, 0), "input")
    return e


def phase_apart(new):
    e = new("Copies a phase apart", "Six Oscillators on one gear, each Offset a sixth further: a wave")
    e.note(
        40,
        40,
        "Six copies of one Oscillator on one Master Gear, each with its Offset a sixth of a wave "
        "further on, bob six dots. Together they make a wave that travels across. Set every Offset "
        "to 0 and they bob as one; spread them by a twelfth instead and the wave is twice as long.",
    )
    master = e.master(0, 0, length=2.0)
    colors = ["#ff5fa2", "#ff7b7b", "#ffa45c", "#ffd166", "#9be564", "#5ad1ff"]
    last = None
    for k, color in enumerate(colors):
        osc = e.add(
            "oscillator",
            0,
            0,
            controls={"amplitude": 0.45, "phaseOffset": round(-k / 6, 4)},
            options={"timing": "on", "trace": "off"},
        )
        e.cable(master, "cycles", osc, "clock")
        dot = e.add(
            "circle",
            0,
            0,
            controls={"radius": 0.13, "softness": 0.02, "centerX": round(-1.25 + 0.5 * k, 3), "foreground": color, "background": INK},
        )
        e.cable(osc, "output", dot, "centerY")
        if last is not None:
            e.cable(last, "color", dot, "background")
        last = dot
    e.cable(last, "color", e.output(0, 0, duration=4.0), "input")
    return e


def on_the_beat(new):
    e = new("Events on the beat", "A Master Gear's Trigger fires an envelope every beat, a Ratio Gear's every bar")
    e.note(
        40,
        40,
        "A gear's Trigger fires an event on every whole cycle. The Master Gear is a beat, half a "
        "second: each Trigger fires an ADSR that kicks the kaleidoscope's zoom. A Ratio Gear at "
        "1 : 4 is a bar, and its Trigger flashes the colour once every four beats. Turn the Master "
        "Gear's Length and both keep the beat; try the bar gear at 1 : 2.",
    )
    beat = e.master(0, 0, length=0.5, display="gears")
    bar = gear(e, 1, 4, clock=beat)
    kick = e.add("adsr", 0, 0, controls={"attack": 0.005, "decay": 0.28, "sustain": 0.0, "release": 0.05})
    flash = e.add("adsr", 0, 0, controls={"attack": 0.005, "decay": 0.6, "sustain": 0.0, "release": 0.1})
    e.cable(beat, "trigger", kick, "gate")
    e.cable(bar, "trigger", flash, "gate")
    zoom = e.add("reframerange", 0, 0, controls={"outMin": 1.0, "outMax": 0.55})
    e.cable(kick, "value", zoom, "input")
    bright = e.add("reframerange", 0, 0, controls={"outMin": 1.0, "outMax": 1.9})
    e.cable(flash, "value", bright, "input")
    noise = e.add("simplex", 0, 0, controls={"scale": 2.5, "contrast": 1.3}, options={"repeat": "1"})
    e.cable(bar, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=CANDY)
    e.cable(noise, "value", grad, "t")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 10.0})
    e.cable(grad, "output", kal, "input")
    e.cable(zoom, "output", kal, "zoom")
    shine = e.add("colorshift", 0, 0)
    e.cable(kal, "output", shine, "input")
    e.cable(bright, "output", shine, "value")
    e.cable(shine, "output", e.output(0, 0, duration=4.0), "input")
    return e


def examples(new):
    return [
        speed_and_offset(new),
        free_and_loop(new),
        one_master(new),
        three_against_two(new),
        seamless(new),
        show_and_wall(new),
        phase_apart(new),
        on_the_beat(new),
    ]
