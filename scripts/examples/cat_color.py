# SPDX-License-Identifier: AGPL-3.0-or-later
"""Color: colors made from numbers, pictures recolored and adjusted, keyed, taken apart and
put back, shifted over time, and mixed."""

from looks import EMBER, OCEAN, SUNSET, turns

ORDER = 3
CATEGORY = "Color"


def hsl_numbers(new):
    e = new("HSL from numbers", "Hue across the screen and lightness up it, from World Coordinates")
    e.note(
        40,
        40,
        "HSLA builds a color from four numbers: Hue, Saturation, Lightness and Alpha, each 0 "
        "to 1. Here World Coordinates give every pixel its own: X, reframed to 0 to 1, is the "
        "Hue, and Y is the Lightness, so the screen is a map of every color. The Master Gear "
        "turns the hue round. Drag Saturation on the HSLA.",
    )
    master = e.master(0, 0)
    world = e.add("worldcoordinates", 0, 0)
    across = e.add("reframerange", 0, 0, controls={"inMin": -1.78, "inMax": 1.78, "outMin": 0, "outMax": 1})
    up = e.add("reframerange", 0, 0, controls={"inMin": -1, "inMax": 1, "outMin": 0.12, "outMax": 0.88})
    spin = e.add("add", 0, 0)
    wrap = e.add("modulo", 0, 0, controls={"b": 1})
    color = e.add("hsla", 0, 0, controls={"s": 0.85})
    e.cable(world, "x", across, "input")
    e.cable(world, "y", up, "input")
    e.cable(across, "output", spin, "a")
    e.cable(master, "wrapped", spin, "b")
    e.cable(spin, "output", wrap, "a")
    e.cable(wrap, "output", color, "h")
    e.cable(up, "output", color, "l")
    e.cable(color, "output", e.output(0, 0), "input")
    return e


def rgb_numbers(new):
    e = new("RGB from numbers", "Red across, green up, blue swinging: RGBA from three numbers")
    e.note(
        40,
        40,
        "RGBA builds a color from how much red, green and blue light it has, each 0 to 1. "
        "Red rises left to right and green bottom to top, from World Coordinates; an "
        "Oscillator swings the blue up and down once a loop. Pull a cable out and drag that "
        "channel by hand, or swap the X and Y cables.",
    )
    master = e.master(0, 0)
    world = e.add("worldcoordinates", 0, 0)
    across = e.add("reframerange", 0, 0, controls={"inMin": -1.78, "inMax": 1.78, "outMin": 0, "outMax": 1})
    up = e.add("reframerange", 0, 0, controls={"inMin": -1, "inMax": 1, "outMin": 0, "outMax": 1})
    blue = e.sway(0, 0, master, amplitude=0.5, offset=0.5)
    color = e.add("rgba", 0, 0)
    e.cable(world, "x", across, "input")
    e.cable(world, "y", up, "input")
    e.cable(across, "output", color, "r")
    e.cable(up, "output", color, "g")
    e.cable(blue, "output", color, "b")
    e.cable(color, "output", e.output(0, 0), "input")
    return e


def by_brightness(new):
    e = new("Color by brightness", "A gray noise graded into shadows, midtones and highlights")
    e.note(
        40,
        40,
        "Color Mapping grades a picture by its brightness: the dark parts take the Shadows "
        "color, the bright parts the Highlights, and the middle the Midtones. Its input here "
        "is a plain gray noise. Pick the three swatches; or put a Colorize in its place, "
        "which tints the whole picture with one color.",
    )
    master = e.master(0, 0)
    noise = e.add("simplex", 0, 0, controls={"scale": 2.5, "contrast": 1.6}, options={"repeat": "1"})
    grade = e.add("colormapping", 0, 0, controls={"shadows": "#140b34", "midtones": "#c2367a", "highlights": "#ffd27a"})
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "color", grade, "input")
    e.cable(grade, "color", e.output(0, 0), "input")
    return e


def palette(new):
    e = new("A palette from one color", "Palette fans one color out into eight that go with it, shown as rings")
    e.note(
        40,
        40,
        "Palette takes one color and gives eight that go with it, A to H, its hue fanned "
        "along an arc. Here the one color is an HSLA whose hue the Master Gear turns, and the "
        "eight are drawn as rings, A outside to H in the middle. Turn the Palette's Spread, "
        "L Spread and C Spread.",
    )
    master = e.master(0, 0, length=8.0)
    seed = e.add("hsla", 0, 0, controls={"s": 0.7, "l": 0.6})
    fan = e.add("palette", 0, 0, controls={"spread": 0.45, "lSpread": 0.7, "cSpread": 0.3})
    e.cable(master, "wrapped", seed, "h")
    e.cable(seed, "output", fan, "input")
    ground = None
    for k, key in enumerate("abcdefgh"):
        ring = e.add("circle", 0, 0, controls={"radius": 0.95 - k * 0.11, "softness": 0.004, "background": "#0d0b17"})
        e.cable(fan, key, ring, "foreground")
        if ground is not None:
            e.cable(ground, "color", ring, "background")
        ground = ring
    e.cable(ground, "color", e.output(0, 0), "input")
    return e


def adjust(new):
    e = new("Adjust a picture", "Levels, Contrast, Gamma, Saturate and Vibrance, one after another")
    e.note(
        40,
        40,
        "The adjusters each change one thing about a picture. Levels sets where black and "
        "white are, Contrast and Gamma bend the brightness, Saturate and Vibrance the "
        "colorfulness. An Oscillator swings Saturate from gray to vivid. Turn each one's "
        "knobs and watch the Output.",
    )
    master = e.master(0, 0)
    noise = e.add("fractal", 0, 0, controls={"scale": 1.6, "octaves": 5}, options={"repeat": "1"})
    picture = e.add("cosinegradient", 0, 0, controls=OCEAN)
    levels = e.add("levels", 0, 0, controls={"inBlack": 0.08, "inWhite": 0.92})
    contrast = e.add("contrast", 0, 0, controls={"contrast": 1.2})
    gamma = e.add("gamma", 0, 0, controls={"gamma": 1.1})
    saturate = e.add("saturate", 0, 0)
    vibrance = e.add("vibrance", 0, 0, controls={"amount": 0.3})
    swing = e.sway(0, 0, master, amplitude=1.0, offset=1.0)
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "value", picture, "t")
    chain = [picture, levels, contrast, gamma, saturate, vibrance]
    for a, b in zip(chain, chain[1:]):
        e.cable(a, "output", b, "input")
    e.cable(swing, "output", saturate, "amount")
    e.cable(vibrance, "output", e.output(0, 0), "input")
    return e


def posterize(new):
    e = new("Posterize", "A smooth palette cut into a few flat bands, dithered at the edges")
    e.note(
        40,
        40,
        "Posterize cuts a picture's smooth tones into a few flat Levels, like a screen print. "
        "Dither scatters the edges between bands in a Pattern, here a Bayer grid at Scale 4. "
        "Turn Levels from 2 to 12, raise Dither, and try the other Patterns.",
    )
    master = e.master(0, 0)
    noise = e.add("simplex", 0, 0, controls={"scale": 1.4}, options={"repeat": "1"})
    picture = e.add("cosinegradient", 0, 0, controls=SUNSET)
    poster = e.add("posterize", 0, 0, controls={"levels": 4, "dither": 0.1, "scale": 4}, options={"pattern": "bayer4"})
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "value", picture, "t")
    e.cable(picture, "output", poster, "input")
    e.cable(poster, "output", e.output(0, 0), "input")
    return e


def chroma_key(new):
    e = new("Chroma key", "A star shot on a green screen, the green swapped for a sky")
    e.note(
        40,
        40,
        "Chroma Key cuts one Key Color out of a picture and shows its Background through the "
        "hole. The green-screen shot here is a turning Star on pure green, and the Background "
        "is a gradient sky. Turn Threshold and Softness; to key a camera, cable a Main "
        "Input's Frame into Input and pick its Key Color.",
    )
    master = e.master(0, 0)
    shot = e.add("star", 0, 0, controls={"points": 6, "innerRadius": 0.3, "foreground": "#ff5fa2", "background": "#00ff00"})
    sky = e.add(
        "lineargradient",
        0,
        0,
        controls={"startColor": "#ffb36b", "endColor": "#2d1b69", "angle": 0.25, "frequency": 0.5, "center": -1.0},
    )
    key = e.add("chromakey", 0, 0, controls={"keyColor": "#00ff00", "threshold": 0.4, "softness": 0.1})
    e.cable(turns(e, master, 1 / 6), "output", shot, "rotation")
    e.cable(shot, "color", key, "input")
    e.cable(sky, "output", key, "background")
    e.cable(key, "color", e.output(0, 0), "input")
    return e


def channels(new):
    e = new("Channels apart and back", "A picture split into red, green and blue, put back in another order")
    e.note(
        40,
        40,
        "Channel Splitter takes a picture apart into four numbers, R, G, B and A, and RGBA "
        "puts four numbers back together. Cabled straight across, the waving flag would come "
        "back as it was; here every channel moves one along, so red becomes green, green blue and "
        "blue red. Re-cable them, or put a Red, Hue or Lightness node in one's place.",
    )
    master = e.master(0, 0)
    flag = e.add("prideflag", 0, 0, options={"flagType": "rainbow"})
    wave = e.add("wave", 0, 0, controls={"amplitude": 0.05, "frequency": 2.0, "rotation": 0.25})
    split = e.add("channelsplitter", 0, 0)
    join = e.add("rgba", 0, 0)
    e.cable(flag, "output", wave, "input")
    e.cable(turns(e, master, 1), "output", wave, "phase")
    e.cable(wave, "output", split, "input")
    e.cable(split, "r", join, "g")
    e.cable(split, "g", join, "b")
    e.cable(split, "b", join, "r")
    e.cable(join, "output", e.output(0, 0), "input")
    return e


def color_shift(new):
    e = new("Color shift over time", "A kaleidoscope whose hue turns round the wheel once a loop")
    e.note(
        40,
        40,
        "Color Shift turns a picture's hue round the color wheel: a Hue Shift of 1 is the "
        "whole way round and back where it began. The Master Gear's Wrapped turns it once a "
        "loop, so this ember-colored kaleidoscope passes through every color. Turn its "
        "Saturation and Value too.",
    )
    master = e.master(0, 0)
    noise = e.add("fractal", 0, 0, controls={"scale": 1.8, "octaves": 4, "contrast": 1.2}, options={"repeat": "1", "type": "ridged"})
    ember = e.add("cosinegradient", 0, 0, controls=EMBER)
    scope = e.add("kaleidoscope", 0, 0, controls={"segments": 8})
    shift = e.add("colorshift", 0, 0, controls={"saturation": 1.2})
    e.cable(master, "cycles", noise, "clock")
    e.cable(noise, "value", ember, "t")
    e.cable(ember, "output", scope, "input")
    e.cable(scope, "output", shift, "input")
    e.cable(master, "wrapped", shift, "hue")
    e.cable(shift, "output", e.output(0, 0), "input")
    return e


def mix(new):
    e = new("Mix two pictures", "A crossfade from dots to stripes and back")
    e.note(
        40,
        40,
        "Mix crossfades between two pictures: Amount 0 is all A, 1 all B. An Oscillator "
        "swings the Amount once a loop, from the dots to the stripes and back. Set Method to "
        "Dark fade first or Light fade first, and the crossfade passes through black or white.",
    )
    master = e.master(0, 0)
    dots = e.add("polkadot", 0, 0, controls={"frequency": 4, "radius": 0.4, "input": "#ffd166", "background": "#ef476f"})
    stripes = e.add("stripes", 0, 0, controls={"frequency": 12, "rotation": -0.125, "color1": "#118ab2", "color2": "#073b4c", "smoothing": 0.1})
    fade = e.sway(0, 0, master, amplitude=0.5, offset=0.5)
    both = e.add("mix", 0, 0)
    e.cable(dots, "color", both, "a")
    e.cable(stripes, "output", both, "b")
    e.cable(fade, "output", both, "amount")
    e.cable(both, "output", e.output(0, 0), "input")
    return e


def blend_modes(new):
    e = new("Blend modes", "A turning hexagon laid over stripes by Difference, so it inverts them")
    e.note(
        40,
        40,
        "Layer Blend lays a Foreground over a Background by a Blend Mode. In Difference, "
        "black leaves the background alone and white turns it inside out, so the stripes "
        "invert inside the white hexagon. Try Multiply, Screen, Overlay and Exclusion, and "
        "lower Opacity.",
    )
    master = e.master(0, 0)
    stripes = e.add("stripes", 0, 0, controls={"frequency": 9, "rotation": 0.125, "color1": "#ffd166", "color2": "#1d3557", "smoothing": 0.15})
    shape = e.add("polygon", 0, 0, controls={"sides": 6, "radius": 0.7, "softness": 0.004})
    over = e.add("layerblend", 0, 0, options={"blend_mode": "difference"})
    e.cable(turns(e, master, 1 / 6), "output", shape, "rotation")
    e.cable(stripes, "output", over, "background")
    e.cable(shape, "color", over, "foreground")
    e.cable(over, "output", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        hsl_numbers(new),
        rgb_numbers(new),
        by_brightness(new),
        palette(new),
        adjust(new),
        posterize(new),
        chroma_key(new),
        channels(new),
        color_shift(new),
        mix(new),
        blend_modes(new),
    ]
