# SPDX-License-Identifier: AGPL-3.0-or-later
"""Helpers the Getting started, Mixer and performance, and Sending and recording categories
share: a Cosine Gradient's palette, a node's saved options set after it is made, and a
control's range on one instance — each checked against the registry as `lib` checks the rest.
"""


def palette(bias, amp, freq, phase):
    """The Cosine Gradient's twelve hidden coefficients: bias, amplitude, frequency and phase,
    each R G B."""
    keys = {}
    for row, values in (("bias", bias), ("amp", amp), ("freq", freq), ("phase", phase)):
        for channel, v in zip("RGB", values):
            keys[f"{row}{channel}"] = v
    return keys


RAINBOW = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
EMBER = palette((0.5, 0.35, 0.35), (0.5, 0.45, 0.45), (1.0, 1.0, 1.0), (0.0, 0.10, 0.20))
OCEAN = palette((0.35, 0.5, 0.6), (0.35, 0.45, 0.4), (1.0, 1.0, 1.0), (0.3, 0.2, 0.2))
DUSK = palette((0.5, 0.4, 0.6), (0.5, 0.4, 0.4), (1.0, 1.0, 1.0), (0.0, 0.2, 0.45))
CANDY = palette((0.5, 0.5, 0.5), (0.5, 0.5, 0.5), (1.0, 1.0, 1.0), (0.8, 0.9, 0.3))


def saved(e, node_id):
    """The node's saved dictionary, to change after `add` made it."""
    return next(n for n in e.ws.nodes if n["id"] == node_id)


def options(e, node_id, **opts):
    """Set options on a node already made, each checked against its definition's choices."""
    n = saved(e, node_id)
    d = e.ws.reg[n["slug"]]
    by_key = {o["key"]: o for o in d["options"]}
    for key, value in opts.items():
        o = by_key.get(key)
        assert o, f"{n['slug']} has no option {key}"
        choices = [c[0] for c in o["choices"]]
        assert not choices or value in choices, f"{n['slug']}.{key}={value} not in {choices}"
        n["options"][key] = value


def ranged(e, node_id, key, lo, hi, step):
    """Give one number control a range of its own on this node — what the range editor
    writes, and what a MIDI knob sweeps — with its value pulled inside it."""
    n = saved(e, node_id)
    d = e.ws.reg[n["slug"]]
    i = next((i for i in d["inputs"] if i["key"] == key), None)
    assert i and i["control"]["kind"] == "number", f"{n['slug']}.{key} is not a number control"
    assert lo < hi and 0 < step <= hi - lo, f"{n['slug']}.{key}: range {lo}..{hi} step {step}"
    n.setdefault("values", {})[key] = {"Range": {"min": float(lo), "max": float(hi), "step": float(step)}}
    v = n["controls"].get(key, {}).get("Float", i["control"]["default"])
    n["controls"][key] = {"Float": float(min(max(v, lo), hi))}
