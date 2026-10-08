# SPDX-License-Identifier: AGPL-3.0-or-later
"""Looks the Generators and Color examples share: Cosine Gradient palettes, and a turn of a
Master Gear scaled into a knob."""


def palette(bias, amp, freq, phase):
    """The Cosine Gradient's twelve coefficients: bias, amplitude, frequency and phase, R G B."""
    keys = {}
    for row, values in (("bias", bias), ("amp", amp), ("freq", freq), ("phase", phase)):
        for channel, v in zip("RGB", values):
            keys[f"{row}{channel}"] = v
    return keys


EMBER = palette((0.5, 0.35, 0.35), (0.5, 0.45, 0.45), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
OCEAN = palette((0.35, 0.5, 0.6), (0.35, 0.45, 0.4), (1.0, 1.0, 1.0), (0.3, 0.2, 0.2))
CANDY = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.8, 0.9, 0.3))
RAINBOW = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.33, 0.67))
DUSK = palette((0.5, 0.4, 0.6), (0.5, 0.4, 0.4), (1.0, 1.0, 1.0), (0.0, 0.2, 0.45))
# Inigo Quilez's, the warm-to-teal one.
SUNSET = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
# Dark at 0, rising through blue and violet to a warm white at a half.
NIGHT = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.5, 0.6, 0.7))
# Dim, two rounds of it from 0 to 1: a ground for something brighter on top.
MIDNIGHT = palette((0.2, 0.15, 0.3), (0.2, 0.15, 0.25), (2.0, 2.0, 2.0), (0.0, 0.2, 0.45))
LAGOON = palette((0.45, 0.55, 0.6), (0.45, 0.4, 0.35), (1.0, 1.0, 1.0), (0.55, 0.35, 0.15))


def turns(e, master, times):
    """A Multiply of `master`'s Wrapped by `times`: a knob in turns going round `times` a loop."""
    m = e.add("multiply", 0, 0, controls={"b": times})
    e.cable(master, "wrapped", m, "a")
    return m


# The kinds of value a picture is made of: a node putting out any of these is part of the
# row, never a control under it.


