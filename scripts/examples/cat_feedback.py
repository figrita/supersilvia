# SPDX-License-Identifier: AGPL-3.0-or-later
"""Feedback: an Output's Frame Out read back into the patch that draws it, so every frame is
drawn on top of the last one — trails, tunnels, smoke, growth — and an Output as a buffer
another Output reads."""

from tef_common import CANDY, NEON, RAINBOW, drifting_noise, gear, times

ORDER = 6
CATEGORY = "Feedback"


def over(e, echo, shape):
    """`shape`'s color laid over `echo` where its mask is: a Mix whose Amount is the mask."""
    layer = e.add("mix", 0, 0)
    e.cable(echo, "output", layer, "a")
    e.cable(shape, "color", layer, "b")
    e.cable(shape, "mask", layer, "amount")
    return layer


def chain(e, first, nodes, key="output"):
    """Cable `first`'s `key` into the first of `nodes`, each into the next by Input; the last
    is returned."""
    prev, out = first, key
    for n in nodes:
        e.cable(prev, out, n, "input")
        prev, out = n, "output"
    return prev


def tunnel(new):
    e = new("Infinite tunnel", "A ring drawn over its own last frame, shrunk a little each time")
    e.note(
        40,
        40,
        "The Output's Frame Out is the frame it drew last, and here it comes back in: shrunk by "
        "the Zoom, turned a hair by the Rotate, and drawn under a new square ring each frame. "
        "Every old ring shrinks a little further, so they fall away into a tunnel, colored by "
        "the palette as it was when each was drawn. Turn the Zoom toward 1 for a longer tunnel, "
        "or the Rotate's Angle for a twist.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    shrink = e.add("zoom", 0, 0, controls={"zoom": 0.92})
    turn = e.add("rotate", 0, 0, controls={"angle": 0.004})
    fade = e.add("colorshift", 0, 0, controls={"value": 0.97})
    echo = chain(e, out, [shrink, turn, fade], key="frame")
    outer = e.add("polygon", 0, 0, controls={"sides": 4, "radius": 1.3, "softness": 0.0})
    inner = e.add("polygon", 0, 0, controls={"sides": 4, "radius": 1.22, "softness": 0.0})
    ring = e.add("subtract", 0, 0)
    e.cable(outer, "mask", ring, "a")
    e.cable(inner, "mask", ring, "b")
    hue = e.add("cosinegradient", 0, 0, controls=RAINBOW)
    e.cable(gear(e, master, 3), "cycles", hue, "clock")
    layer = e.add("mix", 0, 0)
    e.cable(echo, "output", layer, "a")
    e.cable(hue, "output", layer, "b")
    e.cable(ring, "output", layer, "amount")
    e.cable(layer, "output", out, "input")
    return e


def trails(new):
    e = new("Trails", "A dot on a looping path leaving rainbow trails that turn and fade")
    e.note(
        40,
        40,
        "A small dot runs a figure of eight, and every frame it is drawn over the last frame, "
        "turned a little, shifted in hue and dimmed, so it leaves a fading rainbow trail. The "
        "Color Shift's Value is how long the trail lasts: 0.99 is very long, 0.9 short. Turn "
        "its Hue Shift, or the Rotate's Angle the other way.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    turn = e.add("rotate", 0, 0, controls={"angle": 0.003})
    shift = e.add("colorshift", 0, 0, controls={"hue": 0.006, "value": 0.975})
    echo = chain(e, out, [turn, shift], key="frame")
    dot = e.add("circle", 0, 0, controls={"radius": 0.08, "softness": 0.02, "foreground": "#ff3b6b", "background": "#000000"})
    across = e.sway(0, 0, master, amplitude=0.9, offset=0.0)
    e.cable(across, "output", dot, "centerX")
    twice = gear(e, master, 2)
    down = e.add("oscillator", 0, 0, controls={"amplitude": 0.45})
    e.cable(twice, "cycles", down, "clock")
    e.cable(down, "output", dot, "centerY")
    e.cable(over(e, echo, dot), "output", out, "input")
    return e


def smoke(new):
    e = new("Smoke", "A spinning flame whose last frames rise, blur and fade like smoke")
    e.note(
        40,
        40,
        "The last frame comes back nudged up by the Translate, swayed by the Wave, softened by "
        "the Blur and dimmed by the Color Shift, under a small spinning star. Old frames keep "
        "rising, swaying and spreading until they fade, so the star smokes. Turn the Wave's "
        "Amplitude for wilder smoke, or the Translate's Y Offset below zero to let it fall.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    rise = e.add("translate", 0, 0, controls={"y": 0.012})
    curl = e.add("wave", 0, 0, controls={"amplitude": 0.012, "frequency": 4.0})
    e.cable(master, "wrapped", curl, "phase")
    soft = e.add("blur", 0, 0, controls={"blurX": 2.5, "blurY": 2.5}, options={"size": "5x5"})
    fade = e.add("colorshift", 0, 0, controls={"hue": -0.004, "value": 0.985})
    echo = chain(e, out, [rise, curl, soft, fade], key="frame")
    flame = e.add(
        "star",
        0,
        0,
        controls={"points": 5, "innerRadius": 0.05, "centerY": -0.6, "foreground": "#ffb703", "background": "#000000"},
    )
    e.cable(master, "wrapped", flame, "rotation")
    e.cable(over(e, echo, flame), "output", out, "input")
    return e


def kaleidoscope(new):
    e = new("Kaleidoscope feedback", "Each frame folded into six wedges and grown outward, so a dot blooms into a mandala")
    e.note(
        40,
        40,
        "The last frame goes through a Kaleidoscope before it comes back, so every echo of the "
        "wandering dot is folded into six mirrored wedges, again and again, and the pattern "
        "blooms. The Kaleidoscope's Zoom just above 1 pushes each fold outward. Turn Segments, "
        "or Twist for a spinning flower.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    fold = e.add("kaleidoscope", 0, 0, controls={"segments": 6.0, "zoom": 1.02, "twist": 0.02}, width=240)
    fade = e.add("colorshift", 0, 0, controls={"hue": 0.01, "value": 0.97})
    echo = chain(e, out, [fold, fade], key="frame")
    dot = e.add("circle", 0, 0, controls={"radius": 0.08, "softness": 0.03, "foreground": "#4cc9f0", "background": "#000000"})
    e.cable(e.sway(0, 0, master, amplitude=0.5, offset=0.0), "output", dot, "centerX")
    up = gear(e, master, 3)
    wander = e.add("oscillator", 0, 0, controls={"amplitude": 0.3}, options={"waveform": "cosine"})
    e.cable(up, "cycles", wander, "clock")
    e.cable(wander, "output", dot, "centerY")
    e.cable(over(e, echo, dot), "output", out, "input")
    return e


def reaction(new):
    e = new("Reaction-diffusion", "Blur and sharpen fed back until noise grows into a Turing pattern")
    e.note(
        40,
        40,
        "Feed a frame back through a Blur and then a hard Sharpen and the two fight: the blur "
        "spreads every spot and the sharpen pushes it back into an edge, so a little noise grows "
        "into winding stripes, the way chemicals pattern a fish's skin. The two Zooms read the "
        "loop three times bigger, which sets the stripes' width. Turn the Sharpen's Amount, or "
        "the first Zoom.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0, resolution="640x360")
    small = e.add("zoom", 0, 0, controls={"zoom": 1.0 / 3.0})
    spread = e.add("blur", 0, 0, controls={"blurX": 1.5, "blurY": 1.5}, options={"size": "5x5"})
    edge = e.add("sharpen", 0, 0, controls={"amount": 4.0, "radius": 5.0}, options={"method": "unsharp"})
    big = e.add("zoom", 0, 0, controls={"zoom": 3.0})
    tint = e.add("colorize", 0, 0, controls={"tint": "#4cc9f0", "amount": 1.0, "preserveLuminance": 1.0})
    e.cable(out, "frame", small, "input")
    e.cable(small, "output", spread, "input")
    e.cable(spread, "output", edge, "input")
    e.cable(edge, "color", big, "input")
    e.cable(big, "output", tint, "input")
    # A little fresh noise all the time, so the pattern keeps rearranging rather than freezing.
    seed = e.add("simplex", 0, 0, controls={"scale": 6.0, "foreground": "#ffd6a5", "background": "#1b263b"}, options={"repeat": "1"})
    e.cable(master, "cycles", seed, "clock")
    fresh = e.add("mix", 0, 0, controls={"amount": 0.01})
    e.cable(tint, "output", fresh, "a")
    e.cable(seed, "color", fresh, "b")
    e.cable(fresh, "output", out, "input")
    return e


def two_outputs(new):
    e = new("Two Outputs feeding each other", "Each Output draws its shape over the other's last frame")
    e.note(
        40,
        40,
        "Here are two Outputs, and each one reads the other's Frame Out: the left draws a warm "
        "circle over the right's last frame turned, the right a cool square over the left's "
        "last frame shrunk. Every frame goes back and forth between them, so each picture "
        "holds the other's echoes. Watch both Outputs, and turn either Rotate or Zoom.",
    )
    master = e.master(0, 0, length=4.0)
    left = e.output(0, 0)
    right = e.output(0, 0)
    turn = e.add("rotate", 0, 0, controls={"angle": 0.02})
    fade_l = e.add("colorshift", 0, 0, controls={"hue": 0.01, "value": 0.96})
    from_right = chain(e, right, [turn, fade_l], key="frame")
    shrink = e.add("zoom", 0, 0, controls={"zoom": 0.96})
    fade_r = e.add("colorshift", 0, 0, controls={"hue": -0.01, "value": 0.96})
    from_left = chain(e, left, [shrink, fade_r], key="frame")
    warm = e.add("circle", 0, 0, controls={"radius": 0.12, "softness": 0.02, "foreground": "#ff9f1c"})
    e.cable(e.sway(0, 0, master, amplitude=0.6, offset=0.0), "output", warm, "centerX")
    cool = e.add("polygon", 0, 0, controls={"sides": 4, "radius": 0.18, "softness": 0.01, "foreground": "#2ec4b6"})
    e.cable(master, "wrapped", cool, "rotation")
    up = e.add("oscillator", 0, 0, controls={"amplitude": 0.5}, options={"waveform": "cosine"})
    e.cable(master, "cycles", up, "clock")
    e.cable(up, "output", cool, "centerY")
    e.cable(over(e, from_right, warm), "output", left, "input")
    e.cable(over(e, from_left, cool), "output", right, "input")
    return e


def buffer(new):
    e = new("Render once, read twice", "A heavy fractal drawn once into an Output, its frame read in two places by another")
    e.note(
        40,
        40,
        "An Output is also a buffer: the deep Mandelbrot is drawn once, small, into the first "
        "Output, and the second reads its Frame Out twice, tiled behind and turning in the "
        "porthole, for the price of two texture reads instead of two fractals. Turn the "
        "Mandelbrot's Iterations and only the first Output pays for it.",
    )
    master = e.master(0, 0, length=8.0)
    fractal = e.add(
        "mandelbrot",
        0,
        0,
        controls={"centerX": -0.7436, "centerY": 0.1318, "zoom": 250.0, "iterations": 400},
    )
    color = e.add("cosinegradient", 0, 0, controls=CANDY)
    e.cable(master, "cycles", color, "clock")
    e.cable(fractal, "smooth", color, "t")
    heavy = e.output(0, 0, resolution="640x360")
    e.cable(color, "output", heavy, "input")
    tiles = e.add("tile", 0, 0, controls={"width": 1.0, "height": 1.0, "span": 3.0})
    e.cable(times(e, master, "wrapped", 3.0), "output", tiles, "offsetY")
    dim = e.add("colorshift", 0, 0, controls={"saturation": 0.6, "value": 0.45})
    behind = chain(e, heavy, [tiles, dim], key="frame")
    spin = e.add("rotate", 0, 0)
    e.cable(times(e, master, "wrapped", -1.0), "output", spin, "angle")
    e.cable(heavy, "frame", spin, "input")
    hole = e.add("circle", 0, 0, controls={"radius": 0.75, "softness": 0.01})
    show = e.add("mix", 0, 0)
    e.cable(behind, "output", show, "a")
    e.cable(spin, "output", show, "b")
    e.cable(hole, "mask", show, "amount")
    e.cable(show, "output", e.output(0, 0), "input")
    return e


def geiss(new):
    e = new("Geiss flow", "A spinning spiral dragged through a swirling flow, like ink in water")
    e.note(
        40,
        40,
        "Geiss Flow takes the last frame in its Last Frame, drags it along a swirling flow and "
        "fades it under the live picture, so the spiral pours out like ink in water. Distortion "
        "is how far a pixel travels each frame, Swirl how much the flow curls. Turn Feedback "
        "and Fade for longer-lived ink.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    arms = e.add(
        "spiral",
        0,
        0,
        controls={"turns": 2.0, "thickness": 0.1, "outerRadius": 0.8, "background": "#000000"},
    )
    e.cable(master, "wrapped", arms, "rotation")
    hue = e.add("cosinegradient", 0, 0, controls=NEON)
    e.cable(master, "cycles", hue, "clock")
    tint = e.add("layerblend", 0, 0, options={"blend_mode": "multiply"})
    e.cable(arms, "color", tint, "background")
    e.cable(hue, "output", tint, "foreground")
    flow = e.add(
        "geissflow",
        0,
        0,
        controls={"flowScale": 3.0, "distortionAmount": 0.012, "feedbackAmount": 0.95, "fadeAmount": 0.985, "swirl": 1.4},
    )
    e.cable(master, "cycles", flow, "clock")
    e.cable(tint, "output", flow, "input")
    e.cable(out, "frame", flow, "lastFrame")
    e.cable(flow, "output", out, "input")
    return e


def stargate(new):
    e = new("Star Gate", "Slit-scan: a thin slit of live noise dragged out into a corridor")
    e.note(
        40,
        40,
        "Star Gate shows the live picture only in a thin slit and its Last Frame everywhere "
        "else, pushed away from the slit on both sides. Each frame the slit's colors are "
        "dragged a little further out, so the drifting noise streams into a corridor of light, "
        "the way the last reel of 2001 was shot. Turn Angle and Drift, or add some Perspective.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    noise = drifting_noise(e, master, colors=RAINBOW, scale=3.0, contrast=1.4)
    gate = e.add("stargate", 0, 0, controls={"angle": 0.0, "width": 0.015, "speed": 3.0, "perspective": 0.01})
    e.cable(noise, "output", gate, "input")
    e.cable(out, "frame", gate, "lastFrame")
    e.cable(gate, "color", out, "input")
    return e


def camcorder(new):
    e = new("Camcorder at its own screen", "Video feedback: a CRT camcorder aimed at the screen it is showing")
    e.note(
        40,
        40,
        "Camcorder CRT can aim at its own screen: its Last Frame is the Output's Frame Out, and "
        "Feedback mixes that back in, zoomed and turned, the way a camcorder pointed at its own "
        "monitor makes a corridor of screens. Drag and scroll in its viewfinder to aim it, or "
        "turn Zoom, Rotate and Feedback.",
    )
    master = e.master(0, 0, length=4.0)
    out = e.output(0, 0)
    shape = e.add("star", 0, 0, controls={"points": 5, "innerRadius": 0.1, "foreground": "#ff9e00", "background": "#000000"})
    e.cable(master, "wrapped", shape, "rotation")
    tube = e.add(
        "camcordercrt",
        0,
        0,
        controls={
            "curvature": 0.3,
            "scanlines": 0.4,
            "glow": 0.5,
            "vignette": 0.6,
            "brightness": 0.6,
            "fbAmount": 0.75,
            "fbZoom": 1.06,
            "fbRotation": 0.04,
        },
    )
    e.cable(shape, "color", tube, "input")
    e.cable(out, "frame", tube, "lastFrame")
    e.cable(tube, "color", out, "input")
    return e


def examples(new):
    made = [
        tunnel(new),
        trails(new),
        smoke(new),
        kaleidoscope(new),
        reaction(new),
        geiss(new),
        stargate(new),
        camcorder(new),
        two_outputs(new),
        buffer(new),
    ]
    return made
