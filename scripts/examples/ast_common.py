# SPDX-License-Identifier: AGPL-3.0-or-later
"""Helpers the Audio-reactive, Simulations and Text and drawing categories share: a Cosine
Gradient palette, a number scaled onto a knob's range, and notes pinned under the nodes they
describe."""

# The sentence every Audio-reactive note starts from: the node reads the panel, not a device.
CHOOSE_AUDIO = (
    "First choose a sound in the Main Input panel on the left, under Audio: a microphone, a "
    "Sound file, or System audio (what you hear)."
)


def palette(bias, amp, freq, phase):
    """The Cosine Gradient's twelve hidden coefficients, bias + amp · cos(2π(freq · t + phase))."""
    keys = {}
    for row, values in (("bias", bias), ("amp", amp), ("freq", freq), ("phase", phase)):
        for channel, v in zip("RGB", values):
            keys[f"{row}{channel}"] = v
    return keys


SUNSET = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))


def scaled(e, node, key, lo, hi, clamp=True):
    """`node`'s `key`, 0 to 1, onto `lo` to `hi` through a Reframe Range: a band onto a knob,
    so silence sits at `lo` rather than at nothing."""
    r = e.add(
        "reframerange",
        0,
        0,
        controls={"inMin": 0.0, "inMax": 1.0, "outMin": lo, "outMax": hi},
        options={"clamp": "on" if clamp else "off", "named": "off"},
    )
    e.cable(node, key, r, "input")
    return r


def notes_under(e, notes, gap=40, width=240):
    """Lay `e` out now and put a note under each node `notes` names, {node: text}: in the
    node's column, under the lowest node there, so the words sit beside the thing they
    describe and cover nothing. The layout leaves notes where they are put."""
    ws = e.ws
    ws.layout()
    by_id = {n["id"]: n for n in ws.nodes}
    bottom = {}
    for n in ws.nodes:
        if n["slug"] != "note":
            x = n["pos"]["x"]
            bottom[x] = max(bottom.get(x, 0.0), n["pos"]["y"] + ws.height(n))
    for node, text in notes.items():
        x = by_id[node]["pos"]["x"]
        y = bottom[x] + gap
        made = e.note(x, y, text, width=width)
        bottom[x] = y + ws.height(next(n for n in ws.nodes if n["id"] == made))
