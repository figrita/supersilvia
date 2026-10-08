# SPDX-License-Identifier: AGPL-3.0-or-later
"""Generators: pictures made from nothing — shapes, patterns, gradients, noise, fractals,
spirals, words, a canvas to paint on, the wallpaper groups and a tunnel."""

from looks import EMBER, LAGOON, MIDNIGHT, NIGHT, OCEAN, RAINBOW, SUNSET, turns

ORDER = 2
CATEGORY = "Generators"


def shapes(new):
    e = new("Shapes", "A star on a hexagon on a circle, each one the next one's background")
    e.note(
        40,
        40,
        "Circle, Polygon and Star each draw one shape over a Background. Cable one shape's "
        "Color into the next one's Background and they stack: here a star on a hexagon on a "
        "circle, turned by a Master Gear. Try the Polygon's Sides, or the Star's Points and "
        "Inner Radius.",
    )
    master = e.master(0, 0)
    circle = e.add("circle", 0, 0, controls={"radius": 0.82, "softness": 0.004, "foreground": "#1d4e89", "background": "#0b0a1a"})
    hexagon = e.add("polygon", 0, 0, controls={"sides": 6, "radius": 0.66, "softness": 0.004, "foreground": "#f6c453"})
    star = e.add("star", 0, 0, controls={"points": 5, "innerRadius": 0.2, "foreground": "#e8505b"})
    e.cable(circle, "color", hexagon, "background")
    e.cable(hexagon, "color", star, "background")
    e.cable(turns(e, master, -1 / 6), "output", hexagon, "rotation")
    e.cable(turns(e, master, 0.4), "output", star, "rotation")
    e.cable(star, "color", e.output(0, 0), "input")
    return e


def dots_and_stripes(new):
    e = new("Stripes in polka dots", "Polka dots filled with moving stripes, over a grid")
    e.note(
        40,
        40,
        "A pattern's colors are inputs, so a pattern can be painted with another. The Polka "
        "Dot's Input is a Stripes, read at the middle of each dot like a sign made of bulbs, "
        "and its Background is a Grid. A Master Gear slides the stripes along. Turn the Polka "
        "Dot's Frequency, Stagger and Radius, or the Stripes' Rotation.",
    )
    master = e.master(0, 0)
    stripes = e.add(
        "stripes",
        0,
        0,
        controls={"frequency": 4, "rotation": 0.125, "color1": "#ff4d6d", "color2": "#4cc9f0", "smoothing": 0.5},
    )
    grid = e.add(
        "grid",
        0,
        0,
        controls={"cellsX": 32, "cellsY": 18, "thickness": 0.06, "foreground": "#3d5a99", "background": "#141a33"},
    )
    dots = e.add("polkadot", 0, 0, controls={"frequency": 5, "radius": 0.4, "softness": 0.02, "stagger": 0.5})
    e.cable(master, "wrapped", stripes, "phase")
    e.cable(stripes, "output", dots, "input")
    e.cable(grid, "color", dots, "background")
    e.cable(dots, "color", e.output(0, 0), "input")
    return e


def houndstooth(new):
    e = new("Houndstooth in rainbow", "A Pride Flag woven into a houndstooth")
    e.note(
        40,
        40,
        "Houndstooth weaves two colors, and either can be a picture. Its Color A here is a "
        "Pride Flag, so the flag's bands run through the weave; a Master Gear drifts the cloth "
        "along. Pick another Flag Type, or turn the Houndstooth's Frequency.",
    )
    master = e.master(0, 0)
    flag = e.add("prideflag", 0, 0, options={"flagType": "rainbow"})
    cloth = e.add("houndstooth", 0, 0, controls={"scale": 2.5, "colorB": "#15121f"})
    e.cable(flag, "output", cloth, "colorA")
    e.cable(turns(e, master, 0.4), "output", cloth, "offsetX")
    e.cable(cloth, "output", e.output(0, 0), "input")
    return e


def gradients(new):
    e = new("Gradients", "A linear gradient for the sky, a radial one for the sun, rising and setting")
    e.note(
        40,
        40,
        "Two gradients make a sky. The Linear Gradient runs from its Start Color at the bottom "
        "to its End Color at the top; the Radial Gradient is the sun, a warm Center Color "
        "fading out to the sky, which is cabled into its Edge Color. The Master Gear's "
        "Ping-pong raises the sun and sets it. Try the Radial's Radius, and both gradients' colors.",
    )
    master = e.master(0, 0)
    sky = e.add(
        "lineargradient",
        0,
        0,
        controls={"startColor": "#ff8c5a", "endColor": "#1b1446", "angle": 0.25, "frequency": 0.55, "center": -0.9},
    )
    height = e.add("reframerange", 0, 0, controls={"inMin": 0, "inMax": 1, "outMin": -0.75, "outMax": 0.35})
    sun = e.add("radialgradient", 0, 0, controls={"centerColor": "#fff4c2", "radius": 0.55})
    e.cable(master, "pingpong", height, "input")
    e.cable(height, "output", sun, "centerY")
    e.cable(sky, "output", sun, "edgeColor")
    e.cable(sun, "output", e.output(0, 0), "input")
    return e


def cosine_palette(new):
    e = new("Cosine palettes", "A number from 0 to 1 laid across the screen, colored by a Cosine Gradient")
    e.note(
        40,
        40,
        "A Cosine Gradient turns a number from 0 to 1 into a color, along a palette made of "
        "three cosine waves. Here the number is the Linear Gradient's brightness, black at the "
        "left and white at the right, so the whole palette is laid out across the screen; the "
        "Master Gear in its Time cycles it. Drag the Phase and Freq rows in the grid on the "
        "node and watch the strip.",
    )
    master = e.master(0, 0)
    ramp = e.add(
        "lineargradient",
        0,
        0,
        controls={"startColor": "#000000", "endColor": "#ffffff", "angle": 0.0, "frequency": 0.281, "center": -1.78},
    )
    luma = e.add("luminosity", 0, 0)
    grad = e.add("cosinegradient", 0, 0, controls=SUNSET)
    e.cable(ramp, "output", luma, "input")
    e.cable(luma, "output", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    e.cable(grad, "output", e.output(0, 0), "input")
    return e


def fractal_noise(new):
    e = new("Fractal noise", "Layers of noise, colored by a palette, looping seamlessly")
    e.note(
        40,
        40,
        "Fractal Noise stacks Octaves of noise, each finer and fainter than the last; its "
        "Value colors a Cosine Gradient. Repeat is on, so the Master Gear walks the noise "
        "round a circle and the loop closes. Set Octaves to 1 for plain noise, try each Type, "
        "and turn Scale and Gain.",
    )
    master = e.master(0, 0)
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 2.2, "octaves": 5, "gain": 0.5, "contrast": 1.2},
        options={"type": "ridged", "repeat": "1"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=EMBER)
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", e.output(0, 0), "input")
    return e


def worley_cells(new):
    e = new("Worley cells", "Cells of Worley noise in palette colors, with their borders drawn")
    e.note(
        40,
        40,
        "Worley Noise scatters points and asks, everywhere, which is nearest. Its Cells mode "
        "gives each cell a number, which the Cosine Gradient colors; a second Worley in Borders "
        "mode draws the lines between them, multiplied on top. Turn Frequency and Randomness "
        "on both, and the Distance Metric.",
    )
    master = e.master(0, 0)
    cells = e.add("worley", 0, 0, controls={"scale": 7, "randomness": 0.9}, options={"mode": "cells"})
    grad = e.add("cosinegradient", 0, 0, controls=LAGOON)
    borders = e.add(
        "worley",
        0,
        0,
        controls={"scale": 7, "randomness": 0.9},
        options={"mode": "borders"},
    )
    lead = e.add("layerblend", 0, 0, options={"blend_mode": "multiply"})
    e.cable(cells, "value", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    e.cable(grad, "output", lead, "background")
    e.cable(borders, "color", lead, "foreground")
    e.cable(lead, "output", e.output(0, 0), "input")
    return e


def julia(new):
    e = new("Julia set", "A Julia set whose constant goes round a circle, so it melts and reforms")
    e.note(
        40,
        40,
        "A Julia set is drawn from one complex number, C. Two Oscillators a quarter turn apart "
        "walk C round a circle once a loop, so the set melts and reforms; its Smooth escape "
        "count colors a Cosine Gradient. Pull the cables out and drag C Real and C Imaginary "
        "by hand, or turn Zoom and Iterations.",
    )
    master = e.master(0, 0)
    re_ = e.sway(0, 0, master, amplitude=0.7885, offset=0.0)
    im = e.sway(0, 0, master, amplitude=0.7885, offset=0.0)
    e.ws.nodes[-1]["options"]["waveform"] = "cosine"
    fractal = e.add("juliaset", 0, 0, controls={"zoom": 0.62, "iterations": 160})
    grad = e.add("cosinegradient", 0, 0, controls=NIGHT)
    e.cable(re_, "output", fractal, "cReal")
    e.cable(im, "output", fractal, "cImag")
    lift = e.add("power", 0, 0, controls={"exponent": 0.4})
    e.cable(fractal, "smooth", lift, "base")
    e.cable(lift, "output", grad, "t")
    e.cable(grad, "output", e.output(0, 0), "input")
    return e


def mandelbrot(new):
    e = new("Mandelbrot", "The Mandelbrot set, its palette flowing outward from the edge")
    e.note(
        40,
        40,
        "The Mandelbrot set, the black shape in the middle. Its Smooth output, how fast each point "
        "escapes, runs a Cosine Gradient, and the Master Gear cycles the palette so color "
        "streams out of the set. Drag Center X, Center Y and Zoom to explore; raise Iterations "
        "as you zoom in.",
    )
    master = e.master(0, 0)
    fractal = e.add(
        "mandelbrot",
        0,
        0,
        controls={"centerX": -0.62, "centerY": 0.0, "zoom": 0.78, "iterations": 120},
    )
    lift = e.add("power", 0, 0, controls={"exponent": 0.4})
    grad = e.add("cosinegradient", 0, 0, controls=OCEAN)
    inside = e.add("layerblend", 0, 0, controls={"foreground": "#05030c"})
    e.cable(fractal, "smooth", lift, "base")
    e.cable(lift, "output", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    e.cable(grad, "output", inside, "background")
    e.cable(fractal, "mask", inside, "opacity")
    e.cable(inside, "output", e.output(0, 0), "input")
    return e


def lyapunov(new):
    e = new("Lyapunov", "Where a population model is stable and where it is chaos")
    e.note(
        40,
        40,
        "A Lyapunov fractal plays a population model at two growth rates, A and B, in the "
        "order its Sequence says, and colors each point by whether the result settles "
        "(Stable Color) or goes to chaos (Chaos Color). Pick another Sequence, and drag "
        "Center A, Center B and Scale.",
    )
    fractal = e.add(
        "lyapunov",
        0,
        0,
        controls={"foreground": "#ffcf5c", "background": "#0d1b3e", "centerA": 3.4, "centerB": 3.4, "scale": 1.2, "contrast": 3.0},
        options={"sequence": "AABAB"},
    )
    master = e.master(0, 0, length=8.0)
    e.cable(e.sway(0, 0, master, amplitude=0.12, offset=3.4), "output", fractal, "centerA")
    e.cable(fractal, "color", e.output(0, 0), "input")
    return e


def sunflower(new):
    e = new("Spiral and sunflower", "Seeds set at the golden angle, over a turning spiral")
    e.note(
        40,
        40,
        "Phyllotaxis sets seed after seed a fixed Angle round from the last, as a sunflower "
        "does; at the golden angle, 0.382 of a turn, they pack without gaps. An Oscillator "
        "rocks the Angle a hair either side, and the seeds swirl into spiral arms. Its "
        "Background is a Spiral, turned by the Master Gear. Try Count and Dot Size.",
    )
    master = e.master(0, 0)
    spiral = e.add(
        "spiral",
        0,
        0,
        controls={"turns": 6, "thickness": 0.22, "outerRadius": 1.9, "foreground": "#3a2a72", "background": "#120d26"},
        options={"type": "logarithmic"},
    )
    rock = e.sway(0, 0, master, amplitude=0.0025, offset=0.382)
    seeds = e.add("phyllotaxis", 0, 0, controls={"count": 420, "radius": 0.95, "dotSize": 0.034, "fg": "#ffcc4d"})
    e.cable(turns(e, master, 1), "output", spiral, "rotation")
    e.cable(spiral, "color", seeds, "bg")
    e.cable(rock, "output", seeds, "angle")
    e.cable(seeds, "output", e.output(0, 0), "input")
    return e


def words(new):
    e = new("Words as a picture", "The Text node's letters, colored by a moving palette and waved")
    e.note(
        40,
        40,
        "The Text node draws its words as a picture: type into its box. Its Text Color here is "
        "a Cosine Gradient cycling across the letters, and a Wave ripples the result. Pick "
        "another Font or Weight, or cable a pattern into Background Color.",
    )
    master = e.master(0, 0)
    ramp = e.add("lineargradient", 0, 0, controls={"startColor": "#000000", "endColor": "#ffffff", "angle": 0.0, "frequency": 0.6})
    luma = e.add("luminosity", 0, 0)
    ink = e.add("cosinegradient", 0, 0, controls=RAINBOW)
    text = e.add(
        "text",
        0,
        0,
        controls={"backgroundColor": "#101024"},
        options={"font": "Georgia", "weight": "bold", "size": "220"},
        values={"words": {"Text": "super\nsilvia"}},
    )
    wave = e.add("wave", 0, 0, controls={"amplitude": 0.03, "frequency": 3.0, "rotation": 0.25})
    e.cable(ramp, "output", luma, "input")
    e.cable(luma, "output", ink, "t")
    e.cable(master, "cycles", ink, "clock")
    e.cable(ink, "output", text, "textColor")
    e.cable(turns(e, master, 1), "output", wave, "phase")
    e.cable(text, "output", wave, "input")
    e.cable(wave, "output", e.output(0, 0), "input")
    return e


def canvas(new):
    e = new("Paint a mandala", "A Drawing Canvas in eightfold symmetry, glowing over a slow palette")
    e.note(
        40,
        40,
        "The Drawing Canvas is a picture you paint. Drag on its canvas: with Symmetry on R8 "
        "every stroke is repeated eight times round the middle, a mandala. It is screened "
        "over a slowly cycling palette, so the dark canvas shows the colors through. Pick a "
        "Tool and a Color, or press Clear.",
    )
    master = e.master(0, 0, length=8.0)
    canvas = e.add("drawingcanvas", 0, 0, controls={"brushSize": 10, "brushColor": "#ffe8b0"}, options={"symmetry": "r8", "canvas_res": "1024x512"})
    world = e.add("worldcoordinates", 0, 0)
    radius = e.add("pythagorean", 0, 0)
    glow = e.add("cosinegradient", 0, 0, controls=MIDNIGHT)
    dim = e.add("vignette", 0, 0, controls={"strength": 1.0, "radius": 1.1, "softness": 1.2})
    blend = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(world, "x", radius, "a")
    e.cable(world, "y", radius, "b")
    e.cable(radius, "output", glow, "t")
    e.cable(master, "cycles", glow, "clock")
    e.cable(glow, "output", dim, "input")
    e.cable(dim, "color", blend, "background")
    e.cable(canvas, "output", blend, "foreground")
    e.cable(blend, "output", e.output(0, 0), "input")
    return e


def wallpaper(new):
    e = new("Wallpaper groups", "One picture repeated by one of the seventeen symmetries of the plane")
    e.note(
        40,
        40,
        "There are seventeen ways to repeat a pattern over a wall, and Wallpaper does any of "
        "them to the picture it is given: here a palette over noise, which the Master Gear "
        "moves. Pick another Symmetry, p4m or p31m, and turn Frequency, Texture Scale and the "
        "Coeff rows.",
    )
    master = e.master(0, 0)
    noise = e.add("simplex", 0, 0, controls={"scale": 2.0, "contrast": 1.3}, options={"repeat": "1"})
    grad = e.add("cosinegradient", 0, 0, controls=SUNSET)
    paper = e.add("wallpaper", 0, 0, controls={"scale": 1.4, "texScale": 0.6}, options={"symmetry": "p6m"})
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", paper, "input")
    e.cable(paper, "output", e.output(0, 0), "input")
    return e


def tunnel(new):
    e = new("Tunnel", "A flight down a winding tunnel lined with a checkerboard")
    e.note(
        40,
        40,
        "Tunnel wraps any picture round the inside of a tube and flies down it; here the "
        "lining is a Checkerboard. The Master Gear in its Time sets the pace, and Shading "
        "darkens the far end. Turn Twist and Radius, pick another Path, or cable a different "
        "picture into Texture.",
    )
    master = e.master(0, 0)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 6, "color1": "#ffd166", "color2": "#3a0ca3"})
    tube = e.add("tunnel3d", 0, 0, controls={"twist": 0.8, "radius": 1.0, "zoom": 1.5}, options={"shading": "heavy", "path": "sine"})
    e.cable(board, "output", tube, "input")
    e.cable(master, "cycles", tube, "clock")
    e.cable(tube, "output", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        shapes(new),
        dots_and_stripes(new),
        houndstooth(new),
        gradients(new),
        cosine_palette(new),
        fractal_noise(new),
        worley_cells(new),
        julia(new),
        mandelbrot(new),
        lyapunov(new),
        sunflower(new),
        words(new),
        canvas(new),
        wallpaper(new),
        tunnel(new),
    ]
