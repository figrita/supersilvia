# SPDX-License-Identifier: AGPL-3.0-or-later
"""Audio-reactive: sound moving pictures, every one of them through the Main Input node, which
reads the one sound the Main Input panel is analyzing. Each still draws something in silence,
since a band at zero is the low end of a range rather than nothing."""

from ast_common import CHOOSE_AUDIO, scaled

ORDER = 10
CATEGORY = "Audio-reactive"


def main_input(e, uniforms=True, events=False):
    """A Main Input showing only the rows the example reads."""
    return e.add(
        "maininput",
        0,
        0,
        options={"uniforms": "on" if uniforms else "off", "events": "on" if events else "off"},
    )


def bass_pulse(new):
    e = new("Bass pulses a shape", "The bass band swells a turning hexagon")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " The Main Input's Bass is a number from 0 to 1; the Reframe Range "
        "turns it into a Radius from 0.4 to 0.95, so silence is a small hexagon and a kick "
        "swells it. Try Out Max higher for a bigger punch, or cable Mid or High in place of Bass.",
    )
    master = e.master(0, 0, length=8.0)
    mi = main_input(e)
    size = scaled(e, mi, "bass", 0.4, 0.95)
    glow = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#ffe29a", "edgeColor": "#ff2f7a", "radius": 0.5},
    )
    shape = e.add(
        "polygon",
        0,
        0,
        controls={"sides": 6.0, "softness": 0.012, "background": "#140a24"},
    )
    e.cable(size, "output", shape, "radius")
    e.cable(master, "wrapped", shape, "rotation")
    e.cable(glow, "output", shape, "foreground")
    bloom = e.add("bloom", 0, 0, controls={"threshold": 0.55, "intensity": 0.8, "radius": 0.03})
    e.cable(shape, "color", bloom, "input")
    e.cable(bloom, "color", e.output(0, 0), "input")
    return e


def three_bands(new):
    e = new("Bass, mid and high as red, green and blue", "Three circles, one a band, added together")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " Bass grows the red circle, Mid the green and High the blue, and Layer "
        "Blend adds them, so where they overlap the colors mix as light does: yellow, cyan, "
        "magenta, white in the middle. A bass line turns the picture red; a cymbal, blue.",
    )
    mi = main_input(e)
    circles = []
    for band, color, x, y in (
        ("bass", "#ff1e3c", 0.0, 0.27),
        ("mid", "#1eff5a", -0.234, -0.135),
        ("high", "#2a5cff", 0.234, -0.135),
    ):
        r = scaled(e, mi, band, 0.42, 0.8)
        c = e.add(
            "circle",
            0,
            0,
            controls={"foreground": color, "background": "#000000", "centerX": x, "centerY": y, "softness": 0.015},
        )
        e.cable(r, "output", c, "radius")
        circles.append(c)
    first = e.add("layerblend", 0, 0, options={"blend_mode": "add"})
    e.cable(circles[0], "color", first, "background")
    e.cable(circles[1], "color", first, "foreground")
    second = e.add("layerblend", 0, 0, options={"blend_mode": "add"})
    e.cable(first, "output", second, "background")
    e.cable(circles[2], "color", second, "foreground")
    e.cable(second, "output", e.output(0, 0), "input")
    return e


def beat_events(new):
    e = new("A beat picks a new look", "Each bass hit rolls a new number of sides and a new hue")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " Each time the bass crosses its threshold — the square on the Bass bar, "
        "which you drag in the panel — the Main Input fires Bass Event. Each one rolls the "
        "turning polygon a new number of sides and its colors a new turn of the hue wheel. "
        "Drag the threshold up so only the big hits fire, or cable High Event in its place.",
    )
    master = e.master(0, 0, length=8.0)
    mi = main_input(e, uniforms=False, events=True)
    roll = e.add("triggeredrandom", 0, 0, controls={"min": 3.0, "max": 8.99})
    e.cable(mi, "bassEvent", roll, "trigger")
    whole = e.add("floor", 0, 0)
    e.cable(roll, "value", whole, "input")
    turn = e.add("triggeredrandom", 0, 0, controls={"min": 0.0, "max": 1.0})
    e.cable(mi, "bassEvent", turn, "trigger")
    fill = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#fff1b8", "edgeColor": "#ff3d7f", "radius": 0.75, "offset": 0.0},
        options={"loop_mode": "mirror"},
    )
    shape = e.add("polygon", 0, 0, controls={"radius": 0.7, "softness": 0.01, "background": "#120a24"})
    e.cable(fill, "output", shape, "foreground")
    e.cable(whole, "output", shape, "sides")
    e.cable(master, "wrapped", shape, "rotation")
    hue = e.add("colorshift", 0, 0)
    e.cable(shape, "color", hue, "input")
    e.cable(turn, "value", hue, "hue")
    e.cable(hue, "output", e.output(0, 0), "input")
    return e


def auto_gain(new):
    e = new("Auto Gain keeps the level useful", "The loudness, stretched to fill 0 to 1 whatever the source")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " A quiet microphone's Level barely leaves zero and a loud song never "
        "comes down. Auto Gain follows the floor and the ceiling of what arrives and says where "
        "the level sits between them, 0 to 1, so the dots swell just as far for a whisper as "
        "for a stadium. Cable Level straight into the Reframe Range to see the difference.",
    )
    mi = main_input(e)
    gain = e.add("autogain", 0, 0, controls={"attack": 0.05, "release": 3.0})
    e.cable(mi, "level", gain, "input")
    size = scaled(e, gain, "output", 0.18, 0.48)
    ramp = e.add(
        "lineargradient",
        0,
        0,
        controls={"startColor": "#ffb347", "endColor": "#ff3d7f", "angle": 0.125, "frequency": 0.6},
    )
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"frequency": 4.0, "stagger": 0.5, "softness": 0.04, "background": "#170b26"},
    )
    e.cable(ramp, "output", dots, "input")
    e.cable(size, "output", dots, "radius")
    e.cable(dots, "color", e.output(0, 0), "input")
    return e


def slew(new):
    e = new("Slew smooths a jittery band", "The same High band twice: raw on the left, slewed on the right")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " High jumps about from one frame to the next. The left star reads it "
        "straight and jitters; the right one reads it through a Slew, which eases toward each "
        "new value, quickly up and slowly down. Turn Rise and Fall down for a lazier star, or "
        "switch Shape to Rate Limit.",
    )
    mi = main_input(e)
    spike = scaled(e, mi, "high", 0.18, 0.75)
    smooth = e.add("slew", 0, 0, controls={"rise": 12.0, "fall": 2.5}, options={"shape": "ease"})
    e.cable(spike, "output", smooth, "input")
    raw = e.add(
        "star",
        0,
        0,
        controls={"points": 7.0, "centerX": -0.85, "foreground": "#8f8aa6", "background": "#000000"},
    )
    e.cable(spike, "output", raw, "innerRadius")
    eased = e.add(
        "star",
        0,
        0,
        controls={"points": 7.0, "centerX": 0.85, "foreground": "#6ff2d2", "background": "#000000"},
    )
    e.cable(smooth, "output", eased, "innerRadius")
    both = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(raw, "color", both, "background")
    e.cable(eased, "color", both, "foreground")
    e.cable(both, "output", e.output(0, 0), "input")
    return e


def waveform(new):
    e = new("The waveform as a picture", "The oscilloscope bent into a ring that leaves rainbow echoes")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " The Main Input's Oscilloscope is the sound's own wave as a line, flat "
        "in silence; Polar Coordinates bends it into a ring that an Oscillator breathes. The "
        "Output's last frame, zoomed in and shifted in hue, comes back under it as echoes. Try "
        "the Zoom below 1 to send the echoes inward.",
    )
    master = e.master(0, 0, length=2.0)
    breathe = e.sway(0, 0, master, amplitude=0.08, offset=0.9)
    mi = main_input(e, uniforms=False)
    ring = e.add("polarcoords", 0, 0, options={"mode": "to_polar_smooth"})
    e.cable(mi, "oscilloscope", ring, "input")
    e.cable(breathe, "output", ring, "scale")
    color = e.add("colorize", 0, 0, controls={"tint": "#4af2ff", "amount": 1.0, "preserveLuminance": 0.0})
    e.cable(ring, "output", color, "input")
    out = e.output(0, 0)
    grow = e.add("zoom", 0, 0, controls={"zoom": 1.035})
    e.cable(out, "frame", grow, "input")
    fade = e.add("colorshift", 0, 0, controls={"hue": 0.03, "value": 0.94})
    e.cable(grow, "output", fade, "input")
    layer = e.add("layerblend", 0, 0, options={"blend_mode": "add"})
    e.cable(fade, "output", layer, "background")
    e.cable(color, "output", layer, "foreground")
    e.cable(layer, "output", out, "input")
    return e


def solid(new):
    e = new("Sound ripples a solid", "The bass band sets how far a ball's surface ripples", "solids")
    e.note(
        40,
        40,
        CHOOSE_AUDIO + " Bass, scaled from 0.04 to 0.3, is the Amount of a Displace on a ball: "
        "in silence it breathes gently, and a kick throws its surface out. The gear turns the "
        "ripples round. Try Mid into the Displace's Frequency as well.",
    )
    master = e.master(0, 0, length=8.0)
    mi = main_input(e)
    ball = e.add("sphere", 0, 0, controls={"radius": 1.25})
    amount = scaled(e, mi, "bass", 0.04, 0.3)
    ripple = e.mod("displace", 0, 0, ball, controls={"frequency": 5.0})
    e.cable(amount, "output", ripple, "amount")
    e.cable(master, "cycles", ripple, "phase")
    paint = e.mod(
        "heightgradient",
        0,
        0,
        ripple,
        controls={"colorBottom": "#3b2ae0", "colorTop": "#ff8ac8", "bottom": -1.4, "top": 1.4},
    )
    e.show(0, 0, paint, master, "turn", controls={"distance": 5.2, "tilt": 0.05})
    return e


def examples(new):
    return [
        bass_pulse(new),
        three_bands(new),
        beat_events(new),
        auto_gain(new),
        slew(new),
        waveform(new),
        solid(new),
    ]
