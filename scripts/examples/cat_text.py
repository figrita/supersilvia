# SPDX-License-Identifier: AGPL-3.0-or-later
"""Text and drawing: words as a picture, letters as a solid, a canvas to paint on, and the
note that says what a patch is for."""

from ast_common import SUNSET, notes_under

ORDER = 12
CATEGORY = "Text and drawing"


def words(e, text, size, font="Georgia", weight="bold", **controls):
    return e.add(
        "text",
        0,
        0,
        controls=controls,
        options={"font": font, "size": str(size), "weight": weight},
        values={"words": {"Text": text}},
    )


def basics(new):
    e = new("Words as a stencil", "The Text node's Ink as a mask: letters as windows onto moving color")
    e.note(
        40,
        40,
        "Type into the Text node's box; a new line is a new line on screen, and Font, Size and "
        "Align are its menus. Besides its colored picture, Text has Ink: white letters on "
        "black, the shape of the words alone. Here Ink, read as a number, mixes a dim copy of "
        "a drifting palette into a bright one, so the letters are windows. Try another word, "
        "or Ink into a Blur first for soft edges.",
    )
    master = e.master(0, 0, length=8.0)
    noise = e.add("fractal", 0, 0, controls={"scale": 1.6, "contrast": 1.3}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=SUNSET)
    e.cable(noise, "value", grad, "t")
    dim = e.add("colorshift", 0, 0, controls={"value": 0.22, "saturation": 0.7})
    e.cable(grad, "output", dim, "input")
    t = words(e, "look\nthrough", 250, font="Impact", weight="normal")
    ink = e.add("luminosity", 0, 0)
    e.cable(t, "ink", ink, "input")
    window = e.add("mix", 0, 0)
    e.cable(dim, "output", window, "a")
    e.cable(grad, "output", window, "b")
    e.cable(ink, "output", window, "amount")
    e.cable(window, "output", e.output(0, 0), "input")
    return e


def trails(new):
    e = new("Words leaving trails", "A turning word, and the Output's last frame zoomed and fading under it")
    e.note(
        40,
        40,
        "The word turns back and forth on an Oscillator through Rotate, over a clear "
        "background. Under it is the Output's own last frame, zoomed in a little and dimmed "
        "with a hue step, so every frame leaves an echo that grows outward and changes color. "
        "Try the Zoom below 1 to pull the echoes inward, or Value nearer 1 for longer trails.",
    )
    master = e.master(0, 0, length=4.0)
    t = words(e, "echo", 230, font="Impact", weight="normal", textColor="#ffffff", backgroundColor="#00000000")
    swing = e.sway(0, 0, master, amplitude=0.06, offset=0.0)
    turn = e.add("rotate", 0, 0)
    e.cable(t, "output", turn, "input")
    e.cable(swing, "output", turn, "angle")
    out = e.output(0, 0)
    grow = e.add("zoom", 0, 0, controls={"zoom": 1.03})
    e.cable(out, "frame", grow, "input")
    fade = e.add("colorshift", 0, 0, controls={"hue": 0.025, "value": 0.93})
    e.cable(grow, "output", fade, "input")
    seed = e.add("colorize", 0, 0, controls={"tint": "#ff6a3d", "amount": 1.0, "preserveLuminance": 0.0})
    e.cable(turn, "output", seed, "input")
    layer = e.add("layerblend", 0, 0, options={"blend_mode": "normal"})
    e.cable(fade, "output", layer, "background")
    e.cable(seed, "output", layer, "foreground")
    e.cable(layer, "output", out, "input")
    return e


def extruded(new):
    e = new("Letters in three dimensions", "A blurred word pushed out into a solid and turned under a light")
    e.note(
        40,
        40,
        "Extrude pushes a picture out into a slab: solid where its brightness is over "
        "Threshold, Depth deep. The Blur softens the letters' edges first, which is what lets a "
        "low Steepness draw them smooth and fast — unplug it and the edges tear. Threshold is "
        "0.25 so the letters come out full. Try Depth and Bevel, or type another word.",
    )
    master = e.master(0, 0, length=8.0)
    t = words(e, "SOLID", 420, font="Impact", weight="normal", textColor="#ffffff", backgroundColor="#000000")
    soft = e.add("blur", 0, 0, controls={"blurX": 5.0, "blurY": 5.0}, options={"size": "7x7"})
    e.cable(t, "output", soft, "input")
    letters = e.shape(
        "extrude",
        0,
        0,
        scale=2.2,
        depth=0.6,
        threshold=0.25,
        steepness=8.0,
        bevel=0.04,
        color="#f0a35e",
    )
    e.cable(soft, "output", letters, "profile")
    e.show(0, 0, letters, master, (0.06, 0.03), controls={"distance": 6.0, "tilt": 0.06, "light": 0.2})
    return e


def drawing(new):
    e = new("Draw with symmetry", "A drawing canvas that mirrors every stroke six ways, over its guide lines")
    e.note(
        40,
        40,
        "Paint on the Drawing Canvas node with the mouse. Symmetry is on 6-Fold, so every "
        "stroke comes back six times round the middle; the faint spokes behind are where the "
        "copies meet, drawn by Stripes bent round by Polar Coordinates. B, E, L, R, C and F "
        "pick pen, eraser, line, rectangle, circle and fill, and [ ] the size. Try Symmetry "
        "on HV Mirror for a plain four-way mirror, and press Clear to start again.",
    )
    canvas = e.add(
        "drawingcanvas",
        0,
        0,
        controls={"brushSize": 6.0, "brushColor": "#7af7ff", "backgroundColor": "#00000000"},
        options={"symmetry": "r6", "canvas_res": "1024x1024", "wrap": "clamp"},
    )
    lines = e.add(
        "stripes",
        0,
        0,
        controls={"frequency": 3.0, "phase": 0.03, "rotation": 0.0, "color1": "#5a4690", "color2": "#0c0817", "smoothing": 0.01},
    )
    spokes = e.add("polarcoords", 0, 0, options={"mode": "to_polar"})
    e.cable(lines, "output", spokes, "input")
    layer = e.add("layerblend", 0, 0, options={"blend_mode": "normal"})
    e.cable(spokes, "output", layer, "background")
    e.cable(canvas, "output", layer, "foreground")
    glow = e.add("bloom", 0, 0, controls={"threshold": 0.5, "intensity": 0.9, "radius": 0.02})
    e.cable(layer, "output", glow, "input")
    e.cable(glow, "color", e.output(0, 0), "input")
    return e


def notes(new):
    e = new("Notes explain a patch", "A small patch with a note for every node saying what it does")
    e.note(
        40,
        40,
        "A Note connects to nothing and changes nothing: it is words on the canvas, saved with "
        "the patch, for whoever opens it next — you included. Add one from the library like any "
        "node, type into it, and drag its edge to widen it. Arranging puts notes first, so here "
        "the one for each node follows this one, in the order the picture flows.",
    )
    master = e.master(0, 0, length=8.0)
    noise = e.add("perlin", 0, 0, controls={"scale": 2.5, "contrast": 1.4}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=SUNSET)
    e.cable(noise, "value", grad, "t")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0})
    e.cable(grad, "output", kal, "input")
    out = e.output(0, 0)
    e.cable(kal, "output", out, "input")
    notes_under(
        e,
        {
            master: "Master Gear: the clock, one turn every eight seconds, so the noise comes back where it started.",
            noise: "Perlin Noise: walks one circle a turn of the gear. Its Value is a number per pixel.",
            grad: "Cosine Gradient: turns that number into a color, one cosine wave per channel.",
            kal: "Kaleidoscope: folds the picture into eight mirrored wedges.",
            out: "Output: draws whatever reaches its Input, and renders it to a file.",
        },
    )
    return e


def examples(new):
    return [
        basics(new),
        trails(new),
        extruded(new),
        drawing(new),
        notes(new),
    ]
