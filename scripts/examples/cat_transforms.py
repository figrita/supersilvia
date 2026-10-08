# SPDX-License-Identifier: AGPL-3.0-or-later
"""Transforms: nodes that move where a picture is read from — turn, fold, repeat, bend it —
without changing a single color."""

from tef_common import DUSK, RAINBOW, drifting_noise, gear, times

ORDER = 4
CATEGORY = "Transforms"


def zoom_rotate_slide(new):
    e = new("Zoom, rotate and slide", "Translate, Rotate and Zoom in a row, each on the Master Gear")
    e.note(
        40,
        40,
        "A transform moves where the picture is read from, never its colors. Here Translate "
        "slides the checkerboard, Rotate turns it half a turn and Zoom breathes in and out, all "
        "once a loop of the Master Gear. Pull the cable out of Rotate's Angle and drag the knob "
        "yourself, or swap the order of Rotate and Zoom.",
    )
    master = e.master(0, 0, length=8.0)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 8, "color1": "#ffd166", "color2": "#3a0ca3"})
    slide = e.add("translate", 0, 0)
    # One checker period is two cells, 4 / Frequency, so the slide comes back each loop.
    e.cable(times(e, master, "wrapped", 0.5), "output", slide, "y")
    turn = e.add("rotate", 0, 0)
    e.cable(times(e, master, "wrapped", 0.5), "output", turn, "angle")
    zoom = e.add("zoom", 0, 0)
    e.cable(e.sway(0, 0, master, amplitude=0.6, offset=1.4), "output", zoom, "zoom")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.7, "radius": 0.9})
    e.cable(board, "output", slide, "input")
    e.cable(slide, "output", turn, "input")
    e.cable(turn, "output", zoom, "input")
    e.cable(zoom, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def mirror(new):
    e = new("Mirror", "A drifting noise folded down the middle into an inkblot")
    e.note(
        40,
        40,
        "Mirror folds the picture about a line through its center, so a drifting noise becomes "
        "an inkblot with two matching halves. Set Mode to Mirror XY for four quarters, or to a "
        "Flip for a plain upside-down. Slide Mix down to watch the picture walk into its own "
        "reflection.",
    )
    master = e.master(0, 0, length=6.0)
    noise = drifting_noise(e, master, colors=DUSK, scale=1.8, kind="ridged", contrast=1.4)
    fold = e.add("mirror", 0, 0, options={"mode": "mirror_x"})
    vig = e.add("vignette", 0, 0, controls={"strength": 0.8, "radius": 0.85})
    e.cable(noise, "output", fold, "input")
    e.cable(fold, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def kaleidoscope(new):
    e = new("Kaleidoscope", "A turning noise folded into eight mirrored wedges")
    e.note(
        40,
        40,
        "Kaleidoscope folds the picture into wedges around the middle, every other one mirrored, "
        "like a tube of mirrors. The noise behind it turns once a loop and the Oscillator swings "
        "Twist, which curls the wedges into a spiral. Turn Segments, or try Style Spiral.",
    )
    master = e.master(0, 0, length=8.0)
    noise = drifting_noise(e, master, colors=RAINBOW, scale=2.2, kind="ridged", contrast=1.3)
    spin = e.add("rotate", 0, 0)
    e.cable(master, "wrapped", spin, "angle")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0}, width=240)
    e.cable(e.sway(0, 0, master, amplitude=0.8, offset=0.0), "output", kal, "twist")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.9, "radius": 0.9})
    e.cable(noise, "output", spin, "input")
    e.cable(spin, "output", kal, "input")
    e.cable(kal, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def tile(new):
    e = new("Tile", "One star repeated across the frame, every other row slid half a tile")
    e.note(
        40,
        40,
        "Tile wraps the whole frame onto one rectangle of its input, so a single star repeats "
        "forever. Slide at 0.5 shifts every other row half a tile, the way bricks are laid, and "
        "the Master Gear walks Offset X one tile a loop. Turn Span for more tiles, or put a "
        "Repeater in its place for a grid of copies with gaps between them.",
    )
    master = e.master(0, 0, length=6.0)
    star = e.add(
        "star",
        0,
        0,
        controls={"points": 6, "innerRadius": 0.2, "foreground": "#ffbe0b", "background": "#0b3d4f"},
    )
    e.cable(times(e, master, "wrapped", 1.0 / 6.0), "output", star, "rotation")
    tiles = e.add("tile", 0, 0, controls={"width": 1.2, "height": 1.2, "span": 2.4, "slide": 0.5})
    e.cable(times(e, master, "wrapped", 1.2), "output", tiles, "offsetX")
    e.cable(star, "color", tiles, "input")
    e.cable(tiles, "output", e.output(0, 0), "input")
    return e


def polar(new):
    e = new("Polar coordinates", "A checkerboard turned into a spiral that zooms forever")
    e.note(
        40,
        40,
        "Polar Coordinates in Log-Polar mode reads the picture's X as the angle around the "
        "middle and its Y as how far out, on a log scale. So the skewed checkerboard becomes a "
        "spiral, and sliding it up a little each frame zooms in forever. Try Mode Cartesian to "
        "Polar for plain rings, or turn the Stretch/Skew's Skew X.",
    )
    master = e.master(0, 0, length=4.0)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 8, "color1": "#ff5d8f", "color2": "#14102b"})
    slide = e.add("translate", 0, 0)
    e.cable(times(e, master, "wrapped", 0.5), "output", slide, "y")
    skew = e.add("stretchskew", 0, 0, controls={"skewX": 0.5})
    polar = e.add("polarcoords", 0, 0, controls={"scale": 1.0}, options={"mode": "to_log_polar"})
    smooth = e.add("supersampling", 0, 0, options={"quality": "2"})
    e.cable(board, "output", slide, "input")
    e.cable(slide, "output", skew, "input")
    e.cable(skew, "output", polar, "input")
    e.cable(polar, "output", smooth, "input")
    e.cable(smooth, "output", e.output(0, 0), "input")
    return e


def wave(new):
    e = new("Wave", "A flag waving, its bands pushed up and down by a travelling wave")
    e.note(
        40,
        40,
        "Wave pushes the picture back and forth along a wave, so the flag's straight bands "
        "ripple like cloth. Rotation is a quarter turn, so the wave runs across the flag, and "
        "the Master Gear moves its Phase one wave a loop. Turn Amplitude and Frequency, or try "
        "Mode Radial with Displacement Ripple.",
    )
    master = e.master(0, 0, length=2.0)
    flag = e.add("prideflag", 0, 0, options={"flagType": "progress"})
    shrink = e.add("zoom", 0, 0, controls={"zoom": 0.8})
    bend = e.add(
        "wave",
        0,
        0,
        controls={"amplitude": 0.1, "frequency": 2.5, "rotation": 0.25},
        options={"mode": "linear", "displacement": "wiggle"},
    )
    e.cable(times(e, master, "wrapped", -1.0), "output", bend, "phase")
    e.cable(flag, "output", shrink, "input")
    e.cable(shrink, "output", bend, "input")
    e.cable(bend, "output", e.output(0, 0), "input")
    return e


def whirl(new):
    e = new("Whirl, pinch and fisheye", "A grid twisted by Whirl & Pinch, then bulged through a Fisheye Lens")
    e.note(
        40,
        40,
        "Distortions bend the grid instead of moving it whole. Whirl & Pinch twists a disc in "
        "the middle, swung back and forth by the Oscillator, and pulls it in with a negative "
        "Pinch; the Fisheye Lens then bulges the whole frame like a doorway peephole. Set the "
        "Fisheye's Distortion below zero for a pincushion.",
    )
    master = e.master(0, 0, length=4.0)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 14, "color1": "#f1faee", "color2": "#1d3557"})
    twist = e.add("whirlandpinch", 0, 0, controls={"pinch": -0.25, "radius": 0.9})
    e.cable(e.sway(0, 0, master, amplitude=0.6, offset=0.0), "output", twist, "whirl")
    lens = e.add("fisheye", 0, 0, controls={"distortion": 0.5, "radius": 1.2})
    e.cable(board, "output", twist, "input")
    e.cable(twist, "output", lens, "input")
    e.cable(lens, "output", e.output(0, 0), "input")
    return e


def domain_warp(new):
    e = new("Domain warp", "Straight stripes melted into marble by a fractal noise")
    e.note(
        40,
        40,
        "Domain Warp moves every pixel by a fractal noise, and each iteration reads the noise "
        "where the last one moved it, so straight stripes melt into marble. The Master Gear "
        "walks the noise one cell a loop, and Repeat 1 brings it back. Turn Amplitude for more "
        "melt, or set Iterations to 3.",
    )
    master = e.master(0, 0, length=6.0)
    bands = e.add(
        "stripes",
        0,
        0,
        controls={"frequency": 3.0, "rotation": 0.1, "color1": "#f4e9d8", "color2": "#7a2e1d", "smoothing": 0.8},
    )
    warp = e.add(
        "domainwarp",
        0,
        0,
        controls={"amplitude": 0.5, "frequency": 0.9, "octaves": 3.0},
        options={"iterations": "2", "repeat": "1"},
    )
    e.cable(master, "cycles", warp, "clock")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.6, "radius": 0.95})
    e.cable(bands, "output", warp, "input")
    e.cable(warp, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def perspective(new):
    e = new("Perspective", "A checkerboard laid back into a floor that runs to the horizon")
    e.note(
        40,
        40,
        "Perspective divides the picture by a leaning plane, so the checkerboard lies down into "
        "a floor whose far edge runs off to a horizon. The Translate before it slides the floor "
        "toward you one row a loop. Turn Tilt Y, or give it some Tilt X to bank the floor.",
    )
    master = e.master(0, 0, length=2.0)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 4, "color1": "#ffffff", "color2": "#ff006e"})
    slide = e.add("translate", 0, 0)
    e.cable(times(e, master, "wrapped", -1.0), "output", slide, "y")
    floor = e.add("perspective", 0, 0, controls={"tiltY": -0.8})
    smooth = e.add("supersampling", 0, 0, options={"quality": "2"})
    vig = e.add("vignette", 0, 0, controls={"strength": 0.8, "radius": 0.8})
    e.cable(board, "output", slide, "input")
    e.cable(slide, "output", floor, "input")
    e.cable(floor, "output", smooth, "input")
    e.cable(smooth, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def tunnel(new):
    e = new("Tunnel", "A flight down a curving tube with a checkerboard on its walls")
    e.note(
        40,
        40,
        "Tunnel wraps its Texture around the inside of a tube and flies a camera down it. The "
        "Ratio Gear at 1 : 4 flies a quarter of a flight a loop, which is how often the Helix "
        "path comes back. Turn Twist for a wilder path and Zoom for a wider lens, or cable a "
        "noise into Texture.",
    )
    master = e.master(0, 0, length=8.0)
    flight = gear(e, master, 1, 4)
    board = e.add("checkerboard", 0, 0, controls={"frequency": 8, "color1": "#80ffdb", "color2": "#5e2bff"})
    tube = e.add(
        "tunnel3d",
        0,
        0,
        controls={"twist": 1.2, "zoom": 1.2},
        options={"path": "helix", "shading": "light"},
    )
    e.cable(flight, "cycles", tube, "clock")
    e.cable(board, "output", tube, "input")
    e.cable(tube, "output", e.output(0, 0), "input")
    return e


def rotozoom_shake(new):
    e = new("Rotozoom and shaky cam", "A houndstooth turned and zoomed on interlocking waves, then shaken by hand")
    e.note(
        40,
        40,
        "Rotozoom turns and scales the picture on one pair of waves, so turning and zooming "
        "breathe together, and Shaky Cam slides it on four waves that never line up, like a "
        "hand-held camera. Both run on the Master Gear, so the loop closes. Turn Rotozoom's Sin "
        "and Cos Coefficients, or Shaky Cam's Amplitude.",
    )
    master = e.master(0, 0, length=10.0)
    cloth = e.add("houndstooth", 0, 0, controls={"colorA": "#f2e8cf", "colorB": "#bc4749", "scale": 1.2})
    roto = e.add(
        "rotozoom",
        0,
        0,
        controls={"turns": 1.0, "baseZoom": 1.0, "sinCoeff": 0.5, "cosCoeff": 0.4},
    )
    e.cable(master, "cycles", roto, "clock")
    shaky = e.add("shakycam", 0, 0, controls={"amplitude": 0.03})
    e.cable(master, "cycles", shaky, "clock")
    e.cable(master, "cycles", shaky, "clockY")
    e.cable(cloth, "output", roto, "input")
    vig = e.add("vignette", 0, 0, controls={"strength": 0.7, "radius": 0.9})
    e.cable(roto, "output", shaky, "input")
    e.cable(shaky, "output", vig, "input")
    e.cable(vig, "color", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        zoom_rotate_slide(new),
        mirror(new),
        kaleidoscope(new),
        tile(new),
        polar(new),
        wave(new),
        whirl(new),
        domain_warp(new),
        perspective(new),
        tunnel(new),
        rotozoom_shake(new),
    ]
