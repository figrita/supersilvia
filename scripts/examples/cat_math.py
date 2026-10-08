# SPDX-License-Identifier: AGPL-3.0-or-later
"""Math and control: numbers that move knobs — an oscillator, a slew, uniform arithmetic, chance,
a sequencer, counting beats, picking a picture, an envelope, a recorded gesture and a pad."""

from timekit import DUSK, INK, PASTEL, trail

ORDER = 8
CATEGORY = "Math and control"


def oscillator_knob(new):
    e = new("An Oscillator moves a knob", "A sine wave cabled into a Polygon's Radius")
    e.note(
        40,
        40,
        "A cable into a knob takes it over. The Oscillator's Output goes into the Polygon's Radius, "
        "so the hexagon breathes: Amplitude is how far it swings, Level the radius it swings "
        "around, Speed how many waves a second. Try the Waveform: Triangle, Square, Sawtooth. The "
        "trace on the Oscillator draws what it sends.",
    )
    osc = e.add("oscillator", 0, 0, controls={"speed": 0.5, "amplitude": 0.18, "offset": 0.5})
    fill = e.add("radialgradient", 0, 0, controls={"centerColor": "#ffd166", "edgeColor": "#ff4f8b", "radius": 0.7})
    shape = e.add("polygon", 0, 0, controls={"sides": 6.0, "softness": 0.006, "background": INK, "rotation": 0.25})
    e.cable(fill, "output", shape, "foreground")
    e.cable(osc, "output", shape, "radius")
    e.cable(shape, "color", e.output(0, 0), "input")
    return e


def slew(new):
    e = new("Slew smooths a jump", "A square wave raw and through a Slew, drawn side by side")
    e.note(
        40,
        40,
        "A square-wave Oscillator jumps between two levels. The gold dot follows it raw; the blue "
        "one follows it through a Slew, which eases toward each new level instead of jumping. The "
        "Output's own last frame, slid left, draws both paths. Turn the Slew's Rise and Fall, or "
        "set its Shape to Rate for a straight ramp with a corner.",
    )
    osc = e.add(
        "oscillator",
        0,
        0,
        controls={"speed": 0.5, "amplitude": 0.3},
        options={"waveform": "square"},
    )
    soft = e.add("slew", 0, 0, controls={"rise": 6.0, "fall": 6.0}, options={"shape": "ease"})
    e.cable(osc, "output", soft, "input")
    up = e.add("add", 0, 0, controls={"b": 0.45})
    down = e.add("add", 0, 0, controls={"b": -0.45})
    e.cable(osc, "output", up, "a")
    e.cable(soft, "output", down, "a")
    raw = e.add("circle", 0, 0, controls={"radius": 0.06, "softness": 0.03, "centerX": 1.35, "foreground": "#ffb347", "background": "#000000"})
    smooth = e.add("circle", 0, 0, controls={"radius": 0.06, "softness": 0.03, "centerX": 1.35, "foreground": "#5ad1ff"})
    e.cable(up, "output", raw, "centerY")
    e.cable(down, "output", smooth, "centerY")
    e.cable(raw, "color", smooth, "background")
    out = e.output(0, 0)
    blend = trail(e, out, (smooth, "color"), fade=0.985, slide=(-0.025, 0.0))
    e.cable(blend, "output", out, "input")
    return e


def reframe(new):
    e = new("Reframe a 0 to 1", "A gear's Ping-pong, 0 to 1, made into a radius and a rocking turn")
    e.note(
        40,
        40,
        "Gears and envelopes give 0 to 1, and a knob wants its own range. Reframe Range maps the "
        "Ping-pong onto a radius of 0.2 to 0.8 in one node; a Multiply then an Add does the same "
        "by hand, times 0.3 and minus 0.15 making a rock of 0.15 turn either way. Math on numbers "
        "gives numbers, the diamond ports: try Out Min above Out Max to turn the pulse round.",
    )
    master = e.master(0, 0, length=2.0)
    size = e.add("reframerange", 0, 0, controls={"outMin": 0.2, "outMax": 0.8})
    e.cable(master, "pingpong", size, "input")
    times = e.add("multiply", 0, 0, controls={"b": 0.3})
    minus = e.add("add", 0, 0, controls={"b": -0.15})
    e.cable(master, "pingpong", times, "a")
    e.cable(times, "output", minus, "a")
    fill = e.add("radialgradient", 0, 0, controls={"centerColor": "#9be564", "edgeColor": "#2a9d8f", "radius": 0.8})
    shape = e.add("polygon", 0, 0, controls={"sides": 5.0, "softness": 0.006, "background": INK})
    e.cable(fill, "output", shape, "foreground")
    e.cable(size, "output", shape, "radius")
    e.cable(minus, "output", shape, "rotation")
    e.cable(shape, "color", e.output(0, 0, duration=4.0), "input")
    return e


def chance(new):
    e = new("Random on the beat", "A beat count picks a Random number and rerolls a Random Hurl")
    e.note(
        40,
        40,
        "Random gives the same number for the same Seed, so a seed that steps gives a new number "
        "each step. Here a half-second gear's Cycles, floored and wrapped round 8 by Modulo, is "
        "the seed: three Randoms place and size the dot, and Random Hurl, a random color a pixel, "
        "rerolls the static inside it. Every eighth beat the pattern comes back; set the Modulo to 16 for a longer one.",
    )
    beat = e.master(0, 0, length=0.5)
    whole = e.add("floor", 0, 0)
    e.cable(beat, "cycles", whole, "input")
    count = e.add("modulo", 0, 0, controls={"b": 8.0})
    e.cable(whole, "output", count, "a")
    size = e.add("random", 0, 0, controls={"min": 0.1, "max": 0.42})
    e.cable(count, "output", size, "seed")
    seed_x = e.add("add", 0, 0, controls={"b": 37.0})
    seed_y = e.add("add", 0, 0, controls={"b": 71.0})
    e.cable(count, "output", seed_x, "a")
    e.cable(count, "output", seed_y, "a")
    x = e.add("random", 0, 0, controls={"min": -1.2, "max": 1.2})
    y = e.add("random", 0, 0, controls={"min": -0.5, "max": 0.5})
    e.cable(seed_x, "output", x, "seed")
    e.cable(seed_y, "output", y, "seed")
    snow = e.add("randomhurl", 0, 0)
    e.cable(count, "output", snow, "seed")
    tint = e.add("colorize", 0, 0, controls={"tint": "#ff5fa2", "amount": 0.8, "preserveLuminance": 0.6})
    e.cable(snow, "color", tint, "input")
    dot = e.add("circle", 0, 0, controls={"softness": 0.01, "background": INK})
    e.cable(tint, "output", dot, "foreground")
    e.cable(size, "output", dot, "radius")
    e.cable(x, "output", dot, "centerX")
    e.cable(y, "output", dot, "centerY")
    e.cable(dot, "color", e.output(0, 0, duration=4.0), "input")
    return e


def sequencer(new):
    e = new("Step Sequencer", "Four lanes of a sequencer fire four envelopes, a light each")
    e.note(
        40,
        40,
        "A Master Gear two seconds long is a bar, and the Step Sequencer walks its sixteen steps "
        "across it. Each lane fires an ADSR when it reaches a lit cell, and each envelope pops one "
        "light. Click cells in the grid to change the rhythm; set the Master Gear's Length to 1 "
        "for double time.",
    )
    bar = e.master(0, 0, length=2.0, display="gears")
    seq = e.add(
        "stepsequencer",
        0,
        0,
        controls={"gateLength": 0.5},
        values={
            "pattern": {
                "Cells": [
                    "x...x...x...x...",
                    "....x.......x..x",
                    "..x...x...x...x.",
                    "x..x..x...x..x..",
                ]
            }
        },
    )
    e.cable(bar, "cycles", seq, "clock")
    colors = ["#ff5fa2", "#ffb347", "#9be564", "#5ad1ff"]
    last = None
    for k, color in enumerate(colors):
        env = e.add("adsr", 0, 0, controls={"attack": 0.005, "decay": 0.45, "sustain": 0.0, "release": 0.05}, options={"trace": "off"})
        e.cable(seq, f"lane{k + 1}", env, "gate")
        size = e.add("reframerange", 0, 0, controls={"outMin": 0.1, "outMax": 0.36})
        e.cable(env, "value", size, "input")
        light = e.add(
            "circle",
            0,
            0,
            controls={"softness": 0.02, "centerX": round(-1.2 + 0.8 * k, 2), "foreground": color, "background": INK},
        )
        e.cable(size, "output", light, "radius")
        if last is not None:
            e.cable(last, "color", light, "background")
        last = light
    glow = e.add("bloom", 0, 0, controls={"threshold": 0.4, "intensity": 1.2, "radius": 0.05})
    e.cable(last, "color", glow, "input")
    e.cable(glow, "color", e.output(0, 0, duration=4.0), "input")
    return e


def counting(new):
    e = new("Counter and Clock Divider", "A beat steps a dot; every fourth beat adds a side")
    e.note(
        40,
        40,
        "A quarter-second Master Gear fires its Trigger every beat. One Counter counts the beats "
        "round 0 to 3 and steps the dot across. A Clock Divider passes one beat in four to a "
        "second Counter, which counts the polygon's sides round 3 to 6. Set the Divide to 2, or the "
        "second Counter's Ends to Clamp to stop at the top.",
    )
    beat = e.master(0, 0, length=0.25)
    steps = e.add("counter", 0, 0, controls={"step": 1.0, "min": 0.0, "max": 4.0}, options={"ends": "wrap"})
    e.cable(beat, "trigger", steps, "increment")
    across = e.add("reframerange", 0, 0, controls={"inMin": 0.0, "inMax": 3.0, "outMin": -0.9, "outMax": 0.9})
    e.cable(steps, "value", across, "input")
    bars = e.add("clockdivider", 0, 0, controls={"divide": 4.0})
    e.cable(beat, "trigger", bars, "input")
    sides = e.add("counter", 0, 0, controls={"step": 1.0, "min": 3.0, "max": 7.0}, options={"ends": "wrap"})
    e.cable(bars, "trigger", sides, "increment")
    shape = e.add(
        "polygon",
        0,
        0,
        controls={"radius": 0.5, "softness": 0.01, "centerY": 0.12, "foreground": "#5ad1ff", "background": INK},
    )
    e.cable(sides, "value", shape, "sides")
    dot = e.add("circle", 0, 0, controls={"radius": 0.07, "softness": 0.02, "centerY": -0.75, "foreground": "#ffd166"})
    e.cable(shape, "color", dot, "background")
    e.cable(across, "output", dot, "centerX")
    e.cable(dot, "color", e.output(0, 0, duration=4.0), "input")
    return e


def four_pictures(e):
    """Four patterns in one palette, for a mux to choose among."""
    a = e.add("stripes", 0, 0, controls={"frequency": 10.0, "rotation": 0.125, "color1": "#ff5fa2", "color2": "#2b1d5c", "smoothing": 0.2})
    b = e.add("polkadot", 0, 0, controls={"frequency": 7.0, "radius": 0.38, "stagger": 0.5, "input": "#ffd166", "background": "#2b1d5c"})
    c = e.add("checkerboard", 0, 0, controls={"frequency": 6.0, "color1": "#5ad1ff", "color2": "#2b1d5c"})
    d = e.add("spiral", 0, 0, controls={"turns": 6.0, "thickness": 0.25, "outerRadius": 2.0, "foreground": "#9be564", "background": "#2b1d5c"})
    return [(a, "output"), (b, "color"), (c, "output"), (d, "color")]


def mux_number(new):
    e = new("Mux by number", "A number picks one of four pictures, or blends between them")
    e.note(
        40,
        40,
        "Mux (Number) shows one of its four inputs by its Select: 0 is the first, 3 the last. A "
        "gear's Ping-pong, times 3, sweeps it up and back down, and in Crossfade mode each picture "
        "melts into the next. Set the Mode to Switch and it cuts instead. Drag Select yourself "
        "after pulling its cable.",
    )
    master = e.master(0, 0, length=2.0)
    sweep = e.add("multiply", 0, 0, controls={"b": 3.0})
    e.cable(master, "pingpong", sweep, "a")
    mux = e.add("muxnumber", 0, 0, options={"mode": "crossfade"})
    for k, (n, key) in enumerate(four_pictures(e)):
        e.cable(n, key, mux, f"input{k}")
    e.cable(sweep, "output", mux, "select")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.6, "radius": 0.9})
    e.cable(mux, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0, duration=4.0), "input")
    return e


def mux_event(new):
    e = new("Mux by event", "A gear's Trigger steps through four effects; a Button jumps at random")
    e.note(
        40,
        40,
        "Mux (Event) holds one of four inputs and moves on an event: Next, Prev, Random or Reset. "
        "Here a one-second gear's Trigger presses Next, so the picture steps through four effects "
        "of one pattern, a beat each. Press the Button to jump to one at random, or Reset on the "
        "mux to go back to the first.",
    )
    master = e.master(0, 0, length=1.0)
    world = e.add("worldcoordinates", 0, 0)
    far = e.add("pythagorean", 0, 0)
    e.cable(world, "x", far, "a")
    e.cable(world, "y", far, "b")
    rings = e.add("multiply", 0, 0, controls={"b": 1.5})
    e.cable(far, "output", rings, "a")
    grad = e.add("cosinegradient", 0, 0, controls=PASTEL)
    e.cable(rings, "output", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    tiles = e.add("tile", 0, 0, controls={"width": 0.8, "height": 0.8})
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0, "zoom": 1.4})
    wave = e.add("wave", 0, 0, controls={"amplitude": 0.12, "frequency": 6.0}, options={"mode": "radial", "displacement": "ripple"})
    for n in (tiles, kal, wave):
        e.cable(grad, "output", n, "input")
    mux = e.add("muxevent", 0, 0)
    e.cable(grad, "output", mux, "input0")
    e.cable(tiles, "output", mux, "input1")
    e.cable(kal, "output", mux, "input2")
    e.cable(wave, "output", mux, "input3")
    e.cable(master, "trigger", mux, "next")
    button = e.add("button", 0, 0)
    e.cable(button, "trigger", mux, "rand")
    e.cable(mux, "output", e.output(0, 0, duration=4.0), "input")
    return e


def envelope(new):
    e = new("ADSR from a Button", "Hold the Button: the envelope swells the light, and lets it go")
    e.note(
        40,
        40,
        "Press and hold Press on the Button. The ADSR rises over its Attack, falls to its Sustain "
        "over its Decay and holds there while you hold; let go and it falls away over its Release. "
        "Its Value grows the light and its glow, through two Reframe Ranges. Its trace draws the "
        "shape: try a long Attack or a Sustain of 0.",
    )
    button = e.add("button", 0, 0)
    env = e.add("adsr", 0, 0, controls={"attack": 0.35, "decay": 0.5, "sustain": 0.55, "release": 1.2})
    e.cable(button, "trigger", env, "gate")
    size = e.add("reframerange", 0, 0, controls={"outMin": 0.1, "outMax": 0.6})
    glow = e.add("reframerange", 0, 0, controls={"outMin": 0.3, "outMax": 2.5})
    e.cable(env, "value", size, "input")
    e.cable(env, "value", glow, "input")
    light = e.add("circle", 0, 0, controls={"softness": 0.08, "foreground": "#ffd166", "background": INK})
    e.cable(size, "output", light, "radius")
    bloom = e.add("bloom", 0, 0, controls={"threshold": 0.4, "radius": 0.08})
    e.cable(light, "color", bloom, "input")
    e.cable(glow, "output", bloom, "intensity")
    e.cable(bloom, "color", e.output(0, 0), "input")
    return e


# A gesture four seconds long, as a hand moving Input Value might leave it: a value 0 to 1 at
# each moment it moved, in seconds from the start of the recording.
GESTURE = [
    (0.0, 0.5), (0.3, 0.56), (0.6, 0.7), (0.9, 0.86), (1.2, 0.92), (1.5, 0.8), (1.8, 0.55),
    (2.1, 0.3), (2.4, 0.14), (2.7, 0.12), (3.0, 0.25), (3.3, 0.42), (3.6, 0.48), (4.0, 0.5),
]


def automation(new):
    e = new("Record a gesture", "Automation plays back a knob's movement, restarted by a gear every loop")
    e.note(
        40,
        40,
        "Automation remembers a knob moving. It holds a gesture already, twisting the "
        "kaleidoscope, and a four-second gear's Trigger presses its Restart every loop. To record "
        "your own, press Record, move Input Value for the four seconds of its Duration, and it "
        "plays back from the next loop. Min and Max scale it.",
    )
    master = e.master(0, 0, length=4.0)
    auto = e.add(
        "automation",
        0,
        0,
        controls={"duration": 4.0, "min": -1.5, "max": 1.5},
        values={"recording": {"Points": [{"time": t, "value": v} for t, v in GESTURE]}},
    )
    e.cable(master, "trigger", auto, "restart")
    lines = e.add("stripes", 0, 0, controls={"frequency": 7.0, "rotation": 0.25, "color1": "#ffd166", "color2": "#3a1c71", "smoothing": 0.35})
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0, "zoom": 1.2})
    e.cable(lines, "output", kal, "input")
    e.cable(auto, "output", kal, "twist")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.7, "radius": 0.9})
    e.cable(kal, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def xy_pad(new):
    e = new("The XY Pad", "A puck thrown round a pad steers the kaleidoscope's center")
    e.note(
        40,
        40,
        "The XY Pad is a puck in a box, with physics. Its X and Y move the kaleidoscope's center, "
        "and its Temp keeps it wandering on its own. Drag the puck to throw it; right-drag to drop "
        "a gravity well it falls toward, and right-click the well to take it away. Set Temp to 0 "
        "and it slides to rest.",
    )
    pad = e.add(
        "xypad",
        0,
        0,
        controls={"temperature": 1.2, "drag": 0.01, "bounce": 0.9, "minX": -0.6, "maxX": 0.6, "minY": -0.4, "maxY": 0.4},
    )
    noise = e.add("fractal", 0, 0, controls={"scale": 2.0, "octaves": 4.0, "contrast": 1.2}, options={"type": "turbulence"})
    grad = e.add("cosinegradient", 0, 0, controls=DUSK)
    e.cable(noise, "value", grad, "t")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6.0})
    e.cable(grad, "output", kal, "input")
    e.cable(pad, "x", kal, "centerX")
    e.cable(pad, "y", kal, "centerY")
    e.cable(kal, "output", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        oscillator_knob(new),
        slew(new),
        reframe(new),
        chance(new),
        sequencer(new),
        counting(new),
        mux_number(new),
        mux_event(new),
        envelope(new),
        automation(new),
        xy_pad(new),
    ]
