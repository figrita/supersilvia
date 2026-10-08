# SPDX-License-Identifier: AGPL-3.0-or-later
"""Pictures and helpers the Transforms, Effects and Feedback categories share: a few cosine
palettes, a Ratio Gear, a Multiply on a number, and a drifting colored noise that loops."""


def palette(bias, amp, freq, phase):
    """The Cosine Gradient's twelve hidden coefficients, bias + amp · cos(2π(freq · t + phase))."""
    keys = {}
    for row, values in (("bias", bias), ("amp", amp), ("freq", freq), ("phase", phase)):
        for channel, v in zip("RGB", values):
            keys[f"{row}{channel}"] = v
    return keys


EMBER = palette((0.5, 0.35, 0.35), (0.5, 0.45, 0.45), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
OCEAN = palette((0.35, 0.5, 0.6), (0.35, 0.45, 0.4), (1.0, 1.0, 1.0), (0.3, 0.2, 0.2))
CANDY = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.8, 0.9, 0.3))
RAINBOW = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
DUSK = palette((0.5, 0.4, 0.6), (0.5, 0.4, 0.4), (1.0, 1.0, 1.0), (0.0, 0.2, 0.45))
NEON = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (2.0, 1.0, 1.0), (0.5, 0.2, 0.25))
GOLD = palette((0.55, 0.4, 0.3), (0.45, 0.4, 0.35), (1.0, 1.0, 1.0), (0.0, 0.05, 0.12))


def gear(e, master, p, q=1, direction="forward"):
    """A Ratio Gear at Teeth p : q on `master`'s Cycles."""
    g = e.add("ratiogear", 0, 0, controls={"p": p, "q": q}, options={"direction": direction})
    e.cable(master, "cycles", g, "clock")
    return g


def times(e, node, key, b):
    """`node`'s `key` times `b`, through a Multiply; the Multiply is returned."""
    m = e.add("multiply", 0, 0, controls={"b": b})
    e.cable(node, key, m, "a")
    return m


def drifting_noise(e, master, colors=RAINBOW, scale=1.6, kind="standard", octaves=4.0, contrast=1.2, turn_palette=True):
    """A Fractal Noise walking one cell a loop of `master`, Repeat 1 so it closes, colored by a
    Cosine Gradient whose palette turns once a loop. The gradient is returned."""
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": scale, "octaves": octaves, "contrast": contrast},
        options={"type": kind, "repeat": "1"},
    )
    e.cable(master, "cycles", noise, "clock")
    grad = e.add("cosinegradient", 0, 0, controls=colors)
    if turn_palette:
        e.cable(master, "cycles", grad, "clock")
    e.cable(noise, "value", grad, "t")
    return grad


