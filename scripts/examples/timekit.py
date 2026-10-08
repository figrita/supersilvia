# SPDX-License-Identifier: AGPL-3.0-or-later
"""What the Time and gears, Math and control and Taps examples share: the Cosine Gradient's
palettes, a Ratio Gear, and the trail an Output's own last frame leaves."""


def palette(bias, amp, freq, phase):
    """The cosine gradient's twelve coefficients: bias, amplitude, frequency and phase, R G B."""
    keys = {}
    for row, values in (("bias", bias), ("amp", amp), ("freq", freq), ("phase", phase)):
        for channel, v in zip("RGB", values):
            keys[f"{row}{channel}"] = v
    return keys


EMBER = palette((0.5, 0.35, 0.35), (0.5, 0.45, 0.45), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
OCEAN = palette((0.35, 0.5, 0.6), (0.35, 0.45, 0.4), (1.0, 1.0, 1.0), (0.3, 0.2, 0.2))
CANDY = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.8, 0.9, 0.3))
RAINBOW = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
SPECTRUM = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.33, 0.67))
PASTEL = palette((0.62, 0.55, 0.65), (0.35, 0.35, 0.3), (1.0, 1.0, 1.0), (0.0, 0.33, 0.67))
DUSK = palette((0.5, 0.4, 0.6), (0.5, 0.4, 0.4), (1.0, 1.0, 1.0), (0.0, 0.2, 0.45))

# One night-sky ground every example draws on, so the library reads as one set.
INK = "#0b0820"


def gear(e, p, q=1, clock=None, display="rosette", direction="forward", offset=None):
    """A Ratio Gear at Teeth `p : q`, its Clock In from `clock`'s Cycles where one is named."""
    controls = {"p": p, "q": q}
    if offset is not None:
        controls["phaseOffset"] = offset
    g = e.add("ratiogear", 0, 0, controls=controls, options={"display": display, "direction": direction})
    if clock is not None:
        e.cable(clock, "cycles", g, "clock")
    return g


def trail(e, out, picture, fade=0.93, slide=None, zoom=None, turn=None):
    """`picture` laid over the Output's own last frame, slid, zoomed or turned and dimmed by
    `fade` a frame, so whatever moves in it leaves a trail; the Layer Blend is returned for
    the Output's Input. A frame read back is a frame late, which is what makes the loop legal."""
    last = out
    key = "frame"
    if slide is not None:
        n = e.add("translate", 0, 0, controls={"x": slide[0], "y": slide[1]})
        e.cable(last, key, n, "input")
        last, key = n, "output"
    if zoom is not None:
        n = e.add("zoom", 0, 0, controls={"zoom": zoom})
        e.cable(last, key, n, "input")
        last, key = n, "output"
    if turn is not None:
        n = e.add("rotate", 0, 0, controls={"angle": turn})
        e.cable(last, key, n, "input")
        last, key = n, "output"
    dim = e.add("colorshift", 0, 0, controls={"value": fade})
    e.cable(last, key, dim, "input")
    blend = e.add("layerblend", 0, 0, options={"blend_mode": "screen"})
    e.cable(dim, "output", blend, "background")
    e.cable(picture[0], picture[1], blend, "foreground")
    return blend


