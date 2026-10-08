# SPDX-License-Identifier: AGPL-3.0-or-later
"""Getting started: the first things a new hand does — a picture into an Output, a knob, a
cable, two pictures at once, a second Output, feedback, time, the mixer."""

from kit_perform import OCEAN, RAINBOW, ranged

ORDER = 1
CATEGORY = "Getting started"


def first_patch(new):
    e = new("Your first patch", "A pattern into an Output: the whole of a patch")
    e.note(
        40,
        40,
        "A patch is pictures flowing left to right into an Output. This checkerboard is "
        "cabled into the Output's Input, and the Output draws it. Drag Frequency on the "
        "checkerboard and watch the Output change; pick its colors with the swatches.",
    )
    board = e.add("checkerboard", 0, 0, controls={"frequency": 6})
    e.cable(board, "output", e.output(0, 0), "input")
    return e


def knobs(new):
    e = new("Knobs and ranges", "Drag, type and narrow a number: the s-number and its range editor")
    e.note(
        40,
        40,
        "Every number on a node is a knob: drag it to scrub (Shift for fine, Ctrl for coarse), "
        "click it to type a value, press D over it to put it back. Right-click one for its "
        "range editor: the Polka Dot's Radius has been narrowed to 0.10 – 0.50, which is why "
        "it wears an accent border, so the whole drag is the useful part. Hover any knob to "
        "read its range.",
    )
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={
            "frequency": 3.5,
            "radius": 0.32,
            "softness": 0.04,
            "stagger": 0.5,
            "input": "#ffd166",
            "background": "#1b1035",
        },
    )
    ranged(e, dots, "radius", 0.1, 0.5, 0.01)
    e.cable(dots, "color", e.output(0, 0), "input")
    return e


def conversion(new):
    e = new("Cables and conversion", "Fields into a color through an RGBA, and the conversion menu")
    e.note(
        40,
        40,
        "A port's color says what it carries: a picture, a field (a number at every pixel), or "
        "one number (a diamond). The Perlin's Value and the Spiral's Mask are fields, and the "
        "RGBA builds a color from them, one per channel. Drag a field onto a color port — try "
        "the Spiral's Mask onto the Output — and the port rings with dashes: let go, and a menu "
        "offers Grayscale, Red Only and the rest, putting the RGBA in for you.",
    )
    noise = e.add("perlin", 0, 0, controls={"scale": 2.0, "contrast": 1.3, "speed": 0.5})
    spiral = e.add("spiral", 0, 0, controls={"turns": 5.0, "thickness": 0.12, "outerRadius": 1.9})
    rgba = e.add("rgba", 0, 0, controls={"g": 0.18})
    e.cable(noise, "value", rgba, "r")
    e.cable(spiral, "mask", rgba, "b")
    e.cable(rgba, "output", e.output(0, 0), "input")
    return e


def layering(new):
    e = new("Layering two pictures", "Layer Blend stacks one picture over another")
    e.note(
        40,
        40,
        "Layer Blend draws its Foreground over its Background the way a paint program stacks "
        "layers. Try its Blend Mode — Screen, Multiply, Difference — and drag Opacity. To "
        "crossfade from one picture to another instead, put a Mix in its place: Amount 0 is "
        "all A, 1 is all B.",
    )
    stripes = e.add(
        "stripes",
        0,
        0,
        controls={"frequency": 9, "rotation": 0.125, "color1": "#0f3057", "color2": "#00b7c2", "smoothing": 0.3},
    )
    spiral = e.add(
        "spiral",
        0,
        0,
        controls={"turns": 5.0, "thickness": 0.22, "outerRadius": 1.9, "foreground": "#ff6f59"},
    )
    blend = e.add("layerblend", 0, 0, options={"blend_mode": "difference"})
    e.cable(stripes, "output", blend, "background")
    e.cable(spiral, "color", blend, "foreground")
    e.cable(blend, "output", e.output(0, 0), "input")
    return e


def second_output(new):
    e = new("A second Output", "An Output's Frame port carries its picture on to another")
    e.note(
        40,
        40,
        "An Output's Frame port is its finished picture, and it cables on like any other. "
        "The first Output draws a tunnel flying through a checkerboard; the second folds that "
        "frame through a Kaleidoscope. A second Output is how one patch feeds a second screen, a "
        "deck of the mixer, or a send of its own — drag the Kaleidoscope's Segments.",
    )
    board = e.add("checkerboard", 0, 0, controls={"frequency": 16, "color1": "#ffe8c2", "color2": "#3a1c71"})
    tunnel = e.add("tunnel3d", 0, 0, controls={"speed": 0.5, "twist": 1.2}, options={"shading": "light"})
    first = e.output(0, 0)
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8, "zoom": 1.4})
    second = e.output(0, 0)
    e.cable(board, "output", tunnel, "input")
    e.cable(tunnel, "output", first, "input")
    e.cable(first, "frame", kal, "input")
    e.cable(kal, "output", second, "input")
    return e


def feedback(new):
    e = new("Feedback in one step", "An Output reading its own frame: trails")
    e.note(
        40,
        40,
        "The Output's Frame is cabled back into its own patch: each frame is the last one, "
        "zoomed in a little, turned and dimmed, with the triangle drawn over it — so the "
        "triangle leaves trails. Feedback always reads the frame before, which is what makes "
        "the loop allowed. Turn Color Shift's Value toward 1 for longer trails, or the Zoom "
        "below 1 to pull them inward.",
    )
    master = e.master(0, 0)
    shape = e.add(
        "polygon",
        0,
        0,
        controls={"sides": 3, "radius": 0.32, "softness": 0.005, "foreground": "#ffd166", "background": "#00000000"},
    )
    e.cable(master, "wrapped", shape, "rotation")
    zoom = e.add("zoom", 0, 0, controls={"zoom": 1.04})
    turn = e.add("rotate", 0, 0, controls={"angle": 0.01})
    fade = e.add("colorshift", 0, 0, controls={"hue": 0.03, "value": 0.94})
    blend = e.add("layerblend", 0, 0)
    out = e.output(0, 0)
    e.cable(out, "frame", zoom, "input")
    e.cable(zoom, "output", turn, "input")
    e.cable(turn, "output", fade, "input")
    e.cable(fade, "output", blend, "background")
    e.cable(shape, "color", blend, "foreground")
    e.cable(blend, "output", out, "input")
    return e


def time_on_a_node(new):
    e = new("Time on a node", "Speed in Free mode, a gear's Time in Loop mode")
    e.note(
        40,
        40,
        "Every node that moves has a Timing heading, open on both of these. The Perlin runs "
        "Free: its Speed is how fast — 0 stands it still, a negative runs it backwards. The "
        "Cosine Gradient is in Loop mode, its Time cabled from the Master Gear, so its colors "
        "turn exactly once every four seconds and stay on the beat. Click Free | Loop on the "
        "Timing heading to switch a node between the two.",
    )
    master = e.master(0, 0)
    noise = e.add(
        "perlin",
        0,
        0,
        controls={"scale": 2.5, "contrast": 1.2, "speed": 0.6},
        options={"timing": "on"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=RAINBOW, options={"timing": "on"})
    e.cable(master, "cycles", grad, "clock")
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", e.output(0, 0), "input")
    return e


def mixer(new):
    e = new("The mixer", "Two Outputs on the Main Mixer's two decks")
    e.note(
        40,
        40,
        "The Main Mixer panel, on the right, crossfades two decks into what the audience sees. "
        "Press Show on A on the first Output and Show on B on the second, then drag the A / B "
        "balance in the panel from one to the other, and try its Crossfade methods. The mix "
        "goes to a projector from the same panel: Project to background, Pop out or "
        "Fullscreen.",
    )
    dots = e.add(
        "phyllotaxis",
        0,
        0,
        controls={"count": 360, "radius": 1.2, "dotSize": 0.05, "bg": "#140a24", "fg": "#ffd166"},
    )
    a = e.output(0, 0)
    cells = e.add("worley", 0, 0, controls={"scale": 6.0, "contrast": 1.2}, options={"mode": "cells"})
    tint = e.add("cosinegradient", 0, 0, controls=OCEAN)
    b = e.output(0, 0)
    e.cable(dots, "output", a, "input")
    e.cable(cells, "value", tint, "t")
    e.cable(tint, "output", b, "input")
    return e


def examples(new):
    return [
        first_patch(new),
        knobs(new),
        conversion(new),
        layering(new),
        second_output(new),
        feedback(new),
        time_on_a_node(new),
        mixer(new),
    ]
