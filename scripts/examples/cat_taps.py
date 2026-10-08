# SPDX-License-Identifier: AGPL-3.0-or-later
"""Taps: pictures measured into numbers — a brightness that drives a knob, the color at a point,
an exposure held steady, a reading fed back into what it reads, and the light's center turned
into motion."""

from timekit import DUSK, EMBER, INK, SPECTRUM

ORDER = 9
CATEGORY = "Taps"

# A tap measures the square from -1 to 1 each way, so a square Output shows all it reads.
SQUARE = "1080x1080"


def brightness(new):
    e = new("Brightness into a number", "A Tap's Mean brightness, through Auto Gain, sizes a light")
    e.note(
        40,
        40,
        "A Tap passes its picture through and measures it: its Mean is the average brightness, a "
        "frame later. Auto Gain follows that number's floor and ceiling and stretches it to 0 to "
        "1, and a Reframe Range makes it the light's radius: the brighter the noise, the bigger "
        "the light. Swap the noise for a Main Input to measure your camera.",
    )
    master = e.master(0, 0)
    noise = e.add("simplex", 0, 0, controls={"scale": 2.2, "contrast": 1.6}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=DUSK)
    e.cable(noise, "value", grad, "t")
    tap = e.add("tap", 0, 0)
    e.cable(grad, "output", tap, "input")
    gain = e.add("autogain", 0, 0, controls={"attack": 0.2, "release": 2.0})
    e.cable(tap, "mean", gain, "input")
    size = e.add("reframerange", 0, 0, controls={"outMin": 0.05, "outMax": 0.5})
    e.cable(gain, "output", size, "input")
    light = e.add("circle", 0, 0, controls={"softness": 0.15, "foreground": "#fff4d6", "background": "#000000"})
    e.cable(size, "output", light, "radius")
    over = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(tap, "output", over, "background")
    e.cable(light, "color", over, "foreground")
    e.cable(over, "output", e.output(0, 0, resolution=SQUARE), "input")
    return e


def sample_point(new):
    e = new("The color at a point", "Sample reads one point of a turning color wheel into a swatch")
    e.note(
        40,
        40,
        "Sample reads the color at its X and Y, a frame later, as a color and as numbers: R, G, B, "
        "hue, saturation and lightness. Two Oscillators move the point round the wheel, the small "
        "ring marks it, and the swatch in the corner is filled with what it reads. Pull the cables "
        "out of X and Y and drag them by hand.",
    )
    master = e.master(0, 0)
    world = e.add("worldcoordinates", 0, 0)
    angle = e.add("atan2", 0, 0)
    e.cable(world, "x", angle, "x")
    e.cable(world, "y", angle, "y")
    half = e.add("multiply", 0, 0, controls={"b": 0.5})
    e.cable(angle, "output", half, "a")
    wheel = e.add("cosinegradient", 0, 0, controls=SPECTRUM)
    e.cable(half, "output", wheel, "t")
    e.cable(master, "cycles", wheel, "clock")
    x = e.add("oscillator", 0, 0, controls={"amplitude": 0.62}, options={"trace": "off"})
    y = e.add("oscillator", 0, 0, controls={"amplitude": 0.62}, options={"waveform": "cosine", "trace": "off"})
    sweep = e.add("ratiogear", 0, 0, controls={"p": 3.0, "q": 1.0}, options={"display": "rosette"})
    e.cable(master, "cycles", sweep, "clock")
    e.cable(sweep, "cycles", x, "clock")
    e.cable(sweep, "cycles", y, "clock")
    probe = e.add("sample", 0, 0)
    e.cable(wheel, "output", probe, "input")
    e.cable(x, "output", probe, "x")
    e.cable(y, "output", probe, "y")
    ring = e.add("circle", 0, 0, controls={"radius": 0.07, "softness": 0.01, "foreground": "#ffffff"})
    hole = e.add("circle", 0, 0, controls={"radius": 0.05, "softness": 0.01})
    e.cable(probe, "output", ring, "background")
    e.cable(ring, "color", hole, "background")
    e.cable(probe, "color", hole, "foreground")
    for into in (ring, hole):
        e.cable(x, "output", into, "centerX")
        e.cable(y, "output", into, "centerY")
    frame = e.add("circle", 0, 0, controls={"radius": 0.3, "softness": 0.01, "centerX": 1.4, "centerY": 0.62, "foreground": "#ffffff"})
    swatch = e.add("circle", 0, 0, controls={"radius": 0.27, "softness": 0.01, "centerX": 1.4, "centerY": 0.62})
    e.cable(hole, "color", frame, "background")
    e.cable(frame, "color", swatch, "background")
    e.cable(probe, "color", swatch, "foreground")
    e.cable(swatch, "color", e.output(0, 0), "input")
    return e


def exposure(new):
    e = new("Auto Exposure holds a loop", "A feedback tunnel kept at one brightness by Auto Exposure")
    e.note(
        40,
        40,
        "The Output's own last frame, zoomed and shifted in hue, is added under a turning star, "
        "every frame. Added light only grows, so on its own the loop runs to white; Auto Exposure "
        "measures it and scales it so its mean sits at its Target. Turn the Target, or cable the "
        "Color Shift straight into the Layer Blend to watch it burn out.",
    )
    master = e.master(0, 0)
    star = e.add("star", 0, 0, controls={"points": 6.0, "innerRadius": 0.45, "foreground": "#ff7ab8", "background": "#000000"})
    e.cable(master, "wrapped", star, "rotation")
    small = e.add("zoom", 0, 0, controls={"zoom": 0.3})
    e.cable(star, "color", small, "input")
    out = e.output(0, 0, resolution=SQUARE)
    grow = e.add("zoom", 0, 0, controls={"zoom": 1.04})
    e.cable(out, "frame", grow, "input")
    hue = e.add("colorshift", 0, 0, controls={"hue": 0.02})
    e.cable(grow, "output", hue, "input")
    held = e.add("autoexposure", 0, 0, controls={"target": 0.3, "speed": 1.0, "max": 1.0})
    e.cable(hue, "output", held, "input")
    add = e.add("layerblend", 0, 0, options={"blend_mode": "add"})
    e.cable(held, "output", add, "background")
    e.cable(small, "output", add, "foreground")
    e.cable(add, "output", out, "input")
    return e


def feeding_back(new):
    e = new("A reading fed back", "A Tap reads the frame a dark disc is drawn into, and its reading sizes the disc")
    e.note(
        40,
        40,
        "A Tap's numbers are a frame old, so one may feed a node upstream of the Tap itself. Here "
        "its Mean, the frame's brightness, sizes the dark disc drawn into that frame: brighter "
        "noise grows the disc, which darkens the frame, which shrinks it again, so the disc "
        "breathes with the noise. The Slew eases each reading in; turn its Rise and Fall up and "
        "the disc answers faster.",
    )
    master = e.master(0, 0)
    noise = e.add("fractal", 0, 0, controls={"scale": 1.8, "octaves": 4.0, "contrast": 1.3}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=EMBER)
    e.cable(noise, "value", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    disc = e.add("circle", 0, 0, controls={"softness": 0.03, "foreground": INK})
    e.cable(grad, "output", disc, "background")
    tap = e.add("tap", 0, 0)
    e.cable(disc, "color", tap, "input")
    calm = e.add("slew", 0, 0, controls={"rise": 4.0, "fall": 4.0}, options={"shape": "ease"})
    e.cable(tap, "mean", calm, "input")
    size = e.add("reframerange", 0, 0, controls={"inMin": 0.2, "inMax": 0.5, "outMin": 0.05, "outMax": 0.95})
    e.cable(calm, "output", size, "input")
    e.cable(size, "output", disc, "radius")
    e.cable(tap, "output", e.output(0, 0, resolution=SQUARE), "input")
    return e


def into_motion(new):
    e = new("Light into motion", "A Tap's centroid follows the bright part of a moving noise")
    e.note(
        40,
        40,
        "A Tap's X and Y say where its quantity is, weighted by how much of it there is: the "
        "center of the light. Here the Tap measures the bright blobs a Smoothstep cuts out of "
        "the noise, through its Number input, and the diamond is put where they are, so it drifts "
        "after the light. Move the Smoothstep's edges to choose how bright counts.",
    )
    master = e.master(0, 0)
    noise = e.add("simplex", 0, 0, controls={"scale": 1.4, "contrast": 1.0}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    cut = e.add("smoothstep", 0, 0, controls={"edgeA": 0.55, "edgeB": 0.8})
    e.cable(noise, "value", cut, "input")
    grad = e.add("hsla", 0, 0, controls={"h": 0.08, "s": 0.85})
    e.cable(cut, "output", grad, "l")
    tap = e.add("tap", 0, 0)
    e.cable(grad, "output", tap, "input")
    e.cable(cut, "output", tap, "number")
    mark = e.add("polygon", 0, 0, controls={"sides": 4.0, "radius": 0.09, "rotation": 0.125, "softness": 0.01, "foreground": "#ffffff"})
    e.cable(tap, "output", mark, "background")
    e.cable(tap, "x", mark, "centerX")
    e.cable(tap, "y", mark, "centerY")
    e.cable(mark, "color", e.output(0, 0, resolution=SQUARE), "input")
    return e


def examples(new):
    return [brightness(new), sample_point(new), exposure(new), feeding_back(new), into_motion(new)]
