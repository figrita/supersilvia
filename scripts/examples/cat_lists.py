# SPDX-License-Identifier: AGPL-3.0-or-later
"""Lists: many numbers a frame on one cable, and the repeats that read one number a copy from
them — Stamp, Kaleidoscope and Repeat Around here. A list is drawn by hand, spaced by a Range,
rolled by a Random or laid out by Circle and Grid Points; the maths runs on every item, HSLA
makes it colors, Lag eases it and Reduce brings it back to one number."""

from ast_common import CHOOSE_AUDIO
from timekit import INK

ORDER = 9
CATEGORY = "Lists"


def times(e, node, key, b):
    """`node`'s `key` times `b`, through a Multiply; the Multiply is returned."""
    m = e.add("multiply", 0, 0, controls={"b": b})
    e.cable(node, key, m, "a")
    return m


def plus(e, node, key, b):
    """`node`'s `key` plus `b`, through an Add; the Add is returned."""
    a = e.add("add", 0, 0, controls={"b": b})
    e.cable(node, key, a, "a")
    return a


def beat(e, master, per_loop):
    """A count of `per_loop` beats a loop of `master`, 0, 1, 2 … and round again: a Ratio Gear
    `per_loop` : 1, floored, wrapped by a Modulo. A seed that steps a beat at a time."""
    g = e.add("ratiogear", 0, 0, controls={"p": float(per_loop), "q": 1.0})
    e.cable(master, "cycles", g, "clock")
    whole = e.add("floor", 0, 0)
    e.cable(g, "cycles", whole, "input")
    count = e.add("modulo", 0, 0, controls={"b": float(per_loop)})
    e.cable(whole, "output", count, "a")
    return count


def bar(e, low, high):
    """A Linear Gradient up the frame, `low` at the bottom and `high` at the top: a stamped
    copy reads the whole of it, so every bar wears the gradient however tall it is."""
    return e.add("lineargradient", 0, 0, controls={"startColor": low, "endColor": high, "angle": 0.25})


def stamp(e, picture, layout, controls, key="color", overlap=False):
    """A Stamp of `picture` on `layout` with its Per copy rows unfolded, over the night ground;
    `overlap` lets a copy grown past its cell show whole."""
    controls = {"bgColor": INK, **controls}
    options = {"layout": layout, "perCopy": "on", "overlap": "on" if overlap else "off"}
    s = e.add("stamp", 0, 0, controls=controls, options=options)
    e.cable(picture, key, s, "input")
    return s


def drawn(new):
    e = new("A list you draw", "A List node's bars, one bar of a Stamp each")
    e.note(
        40,
        40,
        "A list is many numbers on one cable, the double diamond. The List node's are drawn by "
        "hand: drag up and down across its bars. Its List goes into the Stamp's Count, so there "
        "is a copy for each bar, and into Scale Y under Per copy, so copy i is as tall as bar i. "
        "The Oscillator swings the List's Max, and every bar follows. Redraw the bars, set "
        "Count to 24, or cable the List into Turn as well.",
    )
    master = e.master(0, 0, length=4.0)
    heights = e.add(
        "list",
        0,
        0,
        controls={"count": 12.0, "min": 0.3, "max": 3.2},
        values={"values": {"Numbers": [0.35, 0.6, 0.85, 1.0, 0.8, 0.5, 0.3, 0.45, 0.75, 0.95, 0.65, 0.4]}},
    )
    e.cable(e.sway(0, 0, master, amplitude=0.9, offset=2.6), "output", heights, "max")
    picture = bar(e, "#ff3d7f", "#ffd166")
    bars = stamp(e, picture, "line", {"size": 0.5, "spacing": 0.24, "copyScaleX": 0.36}, key="output")
    e.cable(heights, "list", bars, "count")
    e.cable(heights, "list", bars, "copyScaleY")
    e.cable(bars, "output", e.output(0, 0), "input")
    return e


def evenly(new):
    e = new("A Range, evenly spaced", "Two Ranges turn and grow sixteen stars round a ring")
    e.note(
        40,
        40,
        "A Range is Count numbers evenly spaced From one To the other. The first goes from 0 to "
        "1 turn with Ends on From Only, so the last turn is left out and copy i of the ring is "
        "turned i sixteenths; the second grows the stars from small to big round the ring, its "
        "To swung by the Oscillator. The Rotate turns the whole ring once a loop. Try the "
        "turns Range To 3, or its Ends on Both and watch the first and last star meet.",
    )
    master = e.master(0, 0, length=8.0)
    fill = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#fff6d6", "edgeColor": "#ff8a3d", "radius": 0.7},
    )
    star = e.add("star", 0, 0, controls={"points": 5.0, "innerRadius": 0.42, "background": "#00000000"})
    e.cable(fill, "output", star, "foreground")
    turns = e.add("range", 0, 0, controls={"count": 16.0, "from": 0.0, "to": 1.0}, options={"ends": "from"})
    sizes = e.add("range", 0, 0, controls={"count": 16.0, "from": 0.35, "to": 1.4})
    e.cable(e.sway(0, 0, master, amplitude=0.35, offset=1.25), "output", sizes, "to")
    ring = stamp(e, star, "ring", {"size": 0.3, "radius": 0.68}, key="color", overlap=True)
    e.cable(turns, "list", ring, "count")
    e.cable(turns, "list", ring, "copyTurn")
    e.cable(sizes, "list", ring, "copyScaleX")
    e.cable(sizes, "list", ring, "copyScaleY")
    spin = e.add("rotate", 0, 0)
    e.cable(master, "wrapped", spin, "angle")
    e.cable(ring, "output", spin, "input")
    e.cable(spin, "output", e.output(0, 0), "input")
    return e


def rolled(new):
    e = new("Random, a new sky each beat", "Random lists place, size and turn forty stars, rerolled once a second")
    e.note(
        40,
        40,
        "Random is Count numbers between Min and Max, each chosen by the Seed and its own place "
        "in the list, so the same seed always gives the same sky. Four of them place, size and "
        "turn the stars of a Stamp on Points; the beat count steps every Seed once a second, "
        "each from its own start, and the sky is drawn again. Try a bigger Count on the X and "
        "Y Randoms — the stars already there stay put.",
    )
    master = e.master(0, 0, length=4.0)
    seed = beat(e, master, 4)
    fill = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#ffffff", "edgeColor": "#8fd3ff", "radius": 0.5},
    )
    star = e.add("star", 0, 0, controls={"points": 5.0, "innerRadius": 0.4, "background": "#00000000"})
    e.cable(fill, "output", star, "foreground")
    sky = stamp(e, star, "points", {"size": 0.16}, key="color")
    rows = (
        ("copyX", -1.65, 1.65, 0.0),
        ("copyY", -0.88, 0.88, 31.0),
        ("copyTurn", 0.0, 1.0, 62.0),
        ("copyScaleX", 0.25, 1.4, 93.0),
    )
    lists = {}
    for row, lo, hi, start in rows:
        r = e.add("randomlist", 0, 0, controls={"count": 40.0, "min": lo, "max": hi})
        e.cable(plus(e, seed, "output", start), "output", r, "seed")
        lists[row] = r
    e.cable(lists["copyX"], "list", sky, "count")
    for row, r in lists.items():
        e.cable(r, "list", sky, row)
    e.cable(lists["copyScaleX"], "list", sky, "copyScaleY")
    e.cable(sky, "output", e.output(0, 0), "input")
    return e


def joined(new):
    e = new("Points round a circle, joined", "Two rings of Circle Points, joined end to end, turning opposite ways")
    e.note(
        40,
        40,
        "Circle Points gives Count points round a circle as an X list, a Y list and the Angle "
        "of each. Join puts list B after list A, so three Joins make one ring of eight inside one "
        "of sixteen, and a Stamp on Points draws a triangle at each, turned by its Angle. The "
        "Master Gear turns the inner ring's Start forward and the outer's back. Try each "
        "Circle Points' Count and Spread, or its Radius.",
    )
    master = e.master(0, 0, length=6.0)
    inner = e.add("circlepoints", 0, 0, controls={"count": 8.0, "radius": 0.4})
    e.cable(master, "wrapped", inner, "start")
    outer = e.add("circlepoints", 0, 0, controls={"count": 16.0, "radius": 0.86})
    e.cable(times(e, master, "wrapped", -0.5), "output", outer, "start")
    fill = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#fff1b8", "edgeColor": "#ff5e7e", "radius": 0.8},
    )
    tri = e.add(
        "polygon",
        0,
        0,
        controls={"sides": 3.0, "radius": 0.85, "softness": 0.02, "rotation": 0.0, "background": "#00000000"},
    )
    e.cable(fill, "output", tri, "foreground")
    rings = stamp(e, tri, "points", {"size": 0.34}, key="color")
    for key, row in (("x", "copyX"), ("y", "copyY"), ("angle", "copyTurn")):
        j = e.add("join", 0, 0)
        e.cable(inner, key, j, "a")
        e.cable(outer, key, j, "b")
        e.cable(j, "output", rings, row)
        if key == "x":
            e.cable(j, "output", rings, "count")
    e.cable(rings, "output", e.output(0, 0), "input")
    return e


def ripple(new):
    e = new("Maths on every item", "A grid of dots rippling out from the middle, worked out a dot at a time")
    e.note(
        40,
        40,
        "A list into a maths node runs the maths once per item and gives a list back. Grid Points "
        "lays out 16 × 9 dots; Pythagorean takes each dot's X and Y and gives its distance from "
        "the middle, and the Sine of that distance, its Phase walked back by the Master Gear, "
        "is a wave rolling outward. Add 0.55 and every dot has a size, small in the troughs and "
        "big on the crests. Try the Sine's Frequency, or Grid Points' Columns and Rows.",
    )
    master = e.master(0, 0, length=3.0)
    grid = e.add(
        "gridpoints",
        0,
        0,
        controls={"columns": 16.0, "rows": 9.0, "width": 3.4, "height": 1.9},
    )
    far = e.add("pythagorean", 0, 0)
    e.cable(grid, "x", far, "a")
    e.cable(grid, "y", far, "b")
    wave = e.add("sine", 0, 0, controls={"frequency": 0.9, "amplitude": 0.5})
    e.cable(far, "output", wave, "input")
    e.cable(times(e, master, "wrapped", -1.0), "output", wave, "phase")
    size = plus(e, wave, "output", 0.55)
    fill = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#b8fff1", "edgeColor": "#2a6cff", "radius": 0.9},
    )
    dot = e.add("circle", 0, 0, controls={"radius": 0.9, "softness": 0.06, "background": "#00000000"})
    e.cable(fill, "output", dot, "foreground")
    dots = stamp(e, dot, "points", {"size": 0.21}, key="color")
    e.cable(grid, "x", dots, "count")
    e.cable(grid, "x", dots, "copyX")
    e.cable(grid, "y", dots, "copyY")
    e.cable(size, "output", dots, "copyScaleX")
    e.cable(size, "output", dots, "copyScaleY")
    e.cable(dots, "output", e.output(0, 0), "input")
    return e


def eased(new):
    e = new("Lag eases every item", "Bars that leap to new random heights each beat and drift back down")
    e.note(
        40,
        40,
        "Lag eases each item of a list toward where it is going: Rise is how slowly an item "
        "climbs and Fall how slowly it sinks, in seconds. A Random list of sixteen heights is "
        "rerolled once a second, and through the Lag the bars leap up at once and settle "
        "slowly, the way a meter's do. Take the Lag out and cable the Random straight into the "
        "Stamp to see them jump, or set Rise and Fall the same.",
    )
    master = e.master(0, 0, length=4.0)
    seed = beat(e, master, 4)
    heights = e.add("randomlist", 0, 0, controls={"count": 16.0, "min": 0.15, "max": 3.4})
    e.cable(seed, "output", heights, "seed")
    lag = e.add("lag", 0, 0, controls={"rise": 0.04, "fall": 0.35})
    e.cable(heights, "list", lag, "list")
    picture = bar(e, "#2a6cff", "#6ff2d2")
    bars = stamp(e, picture, "line", {"size": 0.5, "spacing": 0.2, "copyScaleX": 0.34}, key="output")
    e.cable(lag, "output", bars, "count")
    e.cable(lag, "output", bars, "copyScaleY")
    e.cable(bars, "output", e.output(0, 0), "input")
    return e


def highest(new):
    e = new("Reduce a list to one number", "A ball that always sits on the tallest bar, found by two Reduces")
    e.note(
        40,
        40,
        "Reduce makes one number from a whole list: its Sum, its Mean, its lowest, its highest, "
        "or Argmax, the place of the highest. The bars are a Random list eased by a Lag. One "
        "Reduce on Argmax says which bar is tallest, and the maths turns that place into the "
        "ball's X; another on Max says how tall, half of which is the top of the bar, for the "
        "ball's Y. Try the second Reduce on Mean, and the ball floats over the tallest bar at "
        "the bars' average height.",
    )
    master = e.master(0, 0, length=4.0)
    seed = beat(e, master, 4)
    heights = e.add("randomlist", 0, 0, controls={"count": 12.0, "min": 0.2, "max": 3.0, "seed": 7.0})
    e.cable(seed, "output", heights, "seed")
    lag = e.add("lag", 0, 0, controls={"rise": 0.12, "fall": 0.25})
    e.cable(heights, "list", lag, "list")
    picture = bar(e, "#5b2a86", "#ff7ab6")
    bars = stamp(e, picture, "line", {"size": 0.5, "spacing": 0.26, "copyScaleX": 0.4}, key="output")
    e.cable(lag, "output", bars, "count")
    e.cable(lag, "output", bars, "copyScaleY")
    # Copy i of twelve on a Line stands at (i − 5.5) × Spacing.
    which = e.add("reduce", 0, 0, options={"mode": "argmax"})
    e.cable(lag, "output", which, "list")
    x = times(e, plus(e, which, "value", -5.5), "output", 0.26)
    tallest = e.add("reduce", 0, 0, options={"mode": "max"})
    e.cable(lag, "output", tallest, "list")
    # A bar is Size × Scale Y tall about the middle, so its top is a quarter of its Scale Y up.
    y = plus(e, times(e, tallest, "value", 0.25), "output", 0.09)
    ball = e.add(
        "circle",
        0,
        0,
        controls={"radius": 0.08, "softness": 0.01, "foreground": "#ffd166"},
    )
    e.cable(bars, "output", ball, "background")
    e.cable(x, "output", ball, "centerX")
    e.cable(y, "output", ball, "centerY")
    e.cable(ball, "color", e.output(0, 0), "input")
    return e


def rainbow(new):
    e = new("A rainbow from a Range", "A Range into an HSLA's Hue colors each wedge of a kaleidoscope")
    e.note(
        40,
        40,
        "HSLA fed a list gives a color list, one color an item. A Range of twelve from 0 to "
        "1 is twelve hues round the wheel; the Add moves them on by the Master Gear and the "
        "Modulo brings each back under 1, so the colors run round once a loop. The color list "
        "goes into the Kaleidoscope's Color under Per copy, and the Range into Segments makes "
        "as many wedges as colors, over stripes turning half a turn a loop. Try the Range's Count, or the HSLA's Saturation and Lightness.",
    )
    master = e.master(0, 0, length=8.0)
    hues = e.add("range", 0, 0, controls={"count": 12.0, "from": 0.0, "to": 1.0}, options={"ends": "from"})
    moved = e.add("add", 0, 0)
    e.cable(hues, "list", moved, "a")
    e.cable(master, "wrapped", moved, "b")
    wrapped = e.add("modulo", 0, 0, controls={"b": 1.0})
    e.cable(moved, "output", wrapped, "a")
    colors = e.add("hsla", 0, 0, controls={"s": 0.85, "l": 0.6})
    e.cable(wrapped, "output", colors, "h")
    board = e.add(
        "stripes",
        0,
        0,
        controls={"frequency": 5.0, "color1": "#ffffff", "color2": INK, "smoothing": 0.5},
    )
    turn = e.add("rotate", 0, 0)
    e.cable(times(e, master, "wrapped", 0.5), "output", turn, "angle")
    e.cable(board, "output", turn, "input")
    kal = e.add("kaleidoscope", 0, 0, controls={"zoom": 1.2}, options={"perCopy": "on"}, width=240)
    e.cable(turn, "output", kal, "input")
    e.cable(hues, "list", kal, "segments")
    e.cable(colors, "output", kal, "copyColor")
    e.cable(kal, "output", e.output(0, 0), "input")
    return e


def spectrum(new):
    e = new("A spectrum ring", "The sound in thirty-six bands, a rainbow bar each round a ring")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " The Main Input's Spectrum is a list of 128 levels from low to high. "
        "Bands gathers it into thirty-six, Lag lets each fall slowly, and Multiply and Add make "
        "a band a bar's Scale Y, so even silence leaves a ring of short dashes. A Range of turns "
        "stands each bar out from the middle, and a Range into an HSLA's Hue colors them red "
        "round to violet. Try Bands' Count, Low and High, or Lag's Fall at 1.",
    )
    master = e.master(0, 0, length=8.0)
    mi = e.add("maininput", 0, 0, options={"uniforms": "off", "events": "off"})
    bands = e.add("bands", 0, 0, controls={"count": 36.0, "low": 50.0, "high": 14000.0})
    e.cable(mi, "spectrum", bands, "spectrum")
    lag = e.add("lag", 0, 0, controls={"rise": 0.02, "fall": 0.3})
    e.cable(bands, "bands", lag, "list")
    level = plus(e, times(e, lag, "output", 4.0), "output", 0.3)
    # Copy i of a Ring stands i / Count of a turn round; a quarter turn back points it outward.
    turns = e.add("range", 0, 0, controls={"count": 36.0, "from": -0.25, "to": 0.75}, options={"ends": "from"})
    hues = e.add("range", 0, 0, controls={"count": 36.0, "from": 0.0, "to": 0.82})
    colors = e.add("hsla", 0, 0, controls={"s": 0.9, "l": 0.6})
    e.cable(hues, "list", colors, "h")
    picture = bar(e, "#ffffff", "#ffffff")
    ring = stamp(e, picture, "ring", {"size": 0.34, "radius": 0.58, "copyScaleX": 0.16}, key="output", overlap=True)
    e.cable(lag, "output", ring, "count")
    e.cable(turns, "list", ring, "copyTurn")
    e.cable(level, "output", ring, "copyScaleY")
    e.cable(colors, "output", ring, "copyColor")
    spin = e.add("rotate", 0, 0)
    e.cable(master, "wrapped", spin, "angle")
    e.cable(ring, "output", spin, "input")
    e.cable(spin, "output", e.output(0, 0), "input")
    return e


def stair(new):
    e = new("A spiral stair of colors", "Repeat Around lifting and coloring each step from two lists", "solids")
    e.note(
        40,
        40,
        "A solid repeat takes lists on its Per copy rows the way a picture repeat does. "
        "Twenty-eight slabs go round a Repeat Around: a Range into Offset Y lifts each step "
        "above the last, and a Range into an HSLA's Hue paints them red round to violet. The "
        "post is chained on after the Repeat, so there is one post, not twenty-eight. The "
        "Oscillator swings the height Range's To, so the stair stretches and settles. Try a "
        "Range from 0 to 0.5 into Turn, which twists each step about its own center, or one "
        "into Scale X.",
    )
    master = e.master(0, 0, length=8.0)
    slab = e.add(
        "box",
        0,
        0,
        controls={"x": 0.0, "width": 1.25, "height": 0.12, "depth": 0.34, "rounding": 0.04, "color": "#ffffff"},
    )
    rise = e.add("range", 0, 0, controls={"count": 28.0, "from": 0.0, "to": 4.0})
    e.cable(e.sway(0, 0, master, amplitude=0.5, offset=4.0), "output", rise, "to")
    hues = e.add("range", 0, 0, controls={"count": 28.0, "from": 0.0, "to": 0.82})
    colors = e.add("hsla", 0, 0, controls={"s": 0.8, "l": 0.6})
    e.cable(hues, "list", colors, "h")
    ring = e.mod(
        "repeataround",
        0,
        0,
        slab,
        controls={"copies": 28.0, "radius": 0.95},
        options={"axis": "y", "perCopy": "on"},
    )
    e.cable(rise, "list", ring, "copyY")
    e.cable(colors, "output", ring, "copyColor")
    post = e.shape("cylinder", 0, 0, onto=ring, radius=0.16, height=5.2, y=2.0, color="#e8e2d6")
    lift = e.place(0, 0, post, y=-0.94)
    e.show(
        0,
        0,
        lift,
        master,
        "turn",
        controls={"distance": 6.6, "tilt": 0.06, "targetY": 1.0, "fog": 0.02},
    )
    return e


def examples(new):
    return [
        drawn(new),
        evenly(new),
        rolled(new),
        joined(new),
        ripple(new),
        eased(new),
        highest(new),
        rainbow(new),
        spectrum(new),
        stair(new),
    ]
