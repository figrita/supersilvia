#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the "Time Gears" demo project: one small patch per thing the time model gives —
ambient time with a readout, Time and Offset on every moving node, and gears.

    scripts/demo-time.py [project folder] [--registry registry.json]

The folder defaults to "Time Gears" in the projects folder the app makes Untitled in when no
preference chooses another — supersilvia in the documents folder, which on Linux is
XDG_DOCUMENTS_DIR as user-dirs.dirs names it, else ~/Documents, and on macOS ~/Documents
(`src/platform/*/dirs.rs`) — and is replaced whole,
except for its renders/ and assets/, which are kept. Each workspace is one patch into one
Output with a note saying what to try, and a Master Gear whose caption says how long a loop
of it is. Every Output renders at 25 fps as a GIF for the length its loop takes.

Each patch is the first time system's (`git show c62a43d:scripts/demo-time.py`) on gears, so
its GIF is the old one frame for frame where the model allows: a speed Loop mode ran at n
whole cycles of its node's period a loop is a Ratio Gear at Teeth n : 1 under a four-second
Master Gear, a speed of zero is a node running free at Speed 0, a field cabled into Time is
Offset divided by the node's period, and every other knob is the same.

The node definitions come from `cargo run --release --example registry`, which this runs
unless --registry names a dump. Every cable is checked against them: the same port type, or
a uniform into the varying of the same kind, with a dual-mode math output a uniform where
every cable into it is. Every control, option and value key is checked too, so the loader
has nothing to warn about.

Then render the loops, one GIF a workspace into the project's renders/, each as long as its
Master Gear says, or its Output's Duration where no gear can:

    cargo run --release --example loop_gifs -- ~/Documents/supersilvia/"Time Gears"

"One clock" plays assets/seamless-noise.gif, the GIF the "Seamless noise" workspace exports;
loop_gifs copies it in from renders/ where it is missing, as an import would store it, so
delete it from assets/ to play a fresh render.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LOOP = 4.0
FPS = 25.0
PI = 3.14159265358979

VARYING_OF = {"uniform number": "varying number", "uniform color": "varying color"}


def load_registry(path):
    if path:
        return json.loads(Path(path).read_text())
    out = subprocess.run(
        ["cargo", "run", "--release", "--quiet", "--example", "registry"],
        cwd=REPO,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(out.stdout)


def hex_color(h):
    h = h.lstrip("#")
    if len(h) == 6:
        h += "ff"
    return [round(int(h[i : i + 2], 16) / 255, 7) for i in (0, 2, 4, 6)]


class Workspace:
    """One workspace file's nodes and cables, checked against the registry as they are added."""

    def __init__(self, project, name, blurb):
        self.project = project
        self.reg = project.reg
        self.name = name
        self.blurb = blurb
        self.nodes = []
        self.connections = []
        self.id = project.next_workspace()

    def add(self, slug, x, y, controls=None, options=None, values=None, width=None):
        d = self.reg[slug]
        node_id = self.project.next_node()
        saved = {"id": node_id, "slug": slug, "pos": {"x": float(x), "y": float(y)}}
        inputs = {i["key"]: i for i in d["inputs"]}
        saved_controls = {}
        for key, value in (controls or {}).items():
            i = inputs.get(key)
            assert i, f"{slug} has no input {key}"
            c = i["control"]
            if c["kind"] == "number":
                v = float(value)
                assert c["min"] - 1e-6 <= v <= c["max"] + 1e-6, f"{slug}.{key}={v} outside [{c['min']}, {c['max']}]"
                saved_controls[key] = {"Float": v}
            elif c["kind"] == "color":
                saved_controls[key] = {"Color": hex_color(value)}
            else:
                raise AssertionError(f"{slug}.{key} is not a control")
        saved["controls"] = saved_controls
        opts = {o["key"]: o for o in d["options"]}
        for key, value in (options or {}).items():
            o = opts.get(key)
            assert o, f"{slug} has no option {key}"
            choices = [c[0] for c in o["choices"]]
            assert not choices or value in choices or key == "file", f"{slug}.{key}={value} not in {choices}"
        saved["options"] = dict(options or {})
        if values:
            keys = {v["key"] for v in d["values"]}
            for key in values:
                assert key in keys, f"{slug} has no value {key}"
            saved["values"] = values
        if width:
            saved["width"] = float(width)
        self.nodes.append(saved)
        return node_id

    def note(self, x, y, text, width=380):
        return self.add("note", x, y, values={"text": {"Text": text}}, width=width)

    def slug_of(self, node_id):
        return next(n["slug"] for n in self.nodes if n["id"] == node_id)

    def output_type(self, node_id, key):
        d = self.reg[self.slug_of(node_id)]
        out = next((o for o in d["outputs"] if o["key"] == key), None)
        assert out, f"{self.slug_of(node_id)} has no output {key}"
        if out["dual"]:
            # A diamond where every cable into the node is a uniform number.
            into = [c for c in self.connections if c["to"]["node"] == node_id]
            if all(self.output_type(c["from"]["node"], c["from"]["key"]) == "uniform number" for c in into):
                return "uniform number"
        return out["type"]

    def cable(self, a, out, b, into):
        src = self.output_type(a, out)
        d = self.reg[self.slug_of(b)]
        i = next((i for i in d["inputs"] if i["key"] == into), None)
        assert i, f"{self.slug_of(b)} has no input {into}"
        dst = i["type"]
        assert src == dst or VARYING_OF.get(src) == dst, (
            f"{self.slug_of(a)}.{out} ({src}) cannot feed {self.slug_of(b)}.{into} ({dst})"
        )
        assert not any(c["to"] == {"node": b, "key": into} for c in self.connections), f"{into} is cabled twice"
        # A cable into a Time is a clock: the node loops on it rather than running free.
        if into in ("clock", "clockY") and any(o["key"] == "clockMode" for o in d["options"]):
            node = next(n for n in self.nodes if n["id"] == b)
            node["options"]["clockMode"] = "loop"
        self.connections.append({"from": {"node": a, "key": out}, "to": {"node": b, "key": into}})

    def output(self, x, y, resolution="1280x720", duration=LOOP):
        return self.add(
            "output",
            x,
            y,
            controls={"fps": FPS, "duration": duration, "warmup": 0.0},
            options={"resolution": resolution, "writer": "gif"},
        )

    def master(self, x, y, length=LOOP, display="rosette"):
        """A Master Gear `length` seconds long."""
        return self.add(
            "mastergear",
            x,
            y,
            controls={"length": length},
            options={"display": display},
        )

    def gear(self, x, y, p, q=1, clock=None, display="rosette"):
        """A Ratio Gear at Teeth `p : q`, its Clock In from `clock`'s Cycles where one is
        named."""
        g = self.add("ratiogear", x, y, controls={"p": p, "q": q}, options={"display": display})
        if clock is not None:
            self.cable(clock, "cycles", g, "clock")
        return g

    def file(self):
        return {
            "format": "supersilvia-workspace",
            "version": 1,
            "name": self.name,
            "kind": "video",
            "blurb": self.blurb,
            "layout": "canvas",
            "nodes": self.nodes,
            "connections": self.connections,
        }


class Project:
    def __init__(self, reg):
        self.reg = reg
        self.workspaces = []
        self._node = 0
        self._workspace = 0

    def next_node(self):
        self._node += 1
        return self._node

    def next_workspace(self):
        self._workspace += 1
        return self._workspace

    def workspace(self, name, blurb):
        ws = Workspace(self, name, blurb)
        self.workspaces.append(ws)
        return ws

    def write(self, root):
        root.mkdir(parents=True, exist_ok=True)
        for keep in ("renders", "assets"):
            (root / keep).mkdir(exist_ok=True)
        for leftover in ("workspaces", ".autosave", "cache"):
            shutil.rmtree(root / leftover, ignore_errors=True)
        (root / "workspaces").mkdir()
        manifest = {
            "format": "supersilvia-project",
            "version": 1,
            "workspaces": [],
            "open": [ws.id for ws in self.workspaces],
            "active": {"workspace": self.workspaces[0].id},
            "next_node_id": self._node + 1,
            "next_workspace_id": self._workspace + 1,
            "connections": [],
            "shared": [],
            "midi": [],
            "removed": [],
        }
        for ws in self.workspaces:
            name = f"{ws.name}.ssw"
            (root / "workspaces" / name).write_text(json.dumps(ws.file(), indent=2) + "\n")
            manifest["workspaces"].append(
                {"id": ws.id, "name": ws.name, "kind": "video", "file": name, "view": {"pan": [0.0, 0.0], "zoom": 0.8}}
            )
        (root / "project.ssp").write_text(json.dumps(manifest, indent=2) + "\n")


# The cosine gradient's twelve coefficients: bias, amplitude, frequency and phase, R G B.
def palette(bias, amp, freq, phase):
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


def start_here(p):
    ws = p.workspace("Start here", "The time readout, Time and Offset, and one Master Gear")
    ws.note(
        40,
        40,
        "There is one clock: ambient time, the playhead. It is the readout at the right of the "
        "menu bar, beside the frame rate: pause (or Space), the time, and back to zero. That is "
        "all the transport there is. View ▸ Time hides it.\n\n"
        "Every node that moves has two inputs for it. Time is a diamond with no knob: left "
        "empty, the node reads ambient time at its own pace; cabled, it reads whatever is "
        "plugged in, usually a gear. Offset is a knob, 0 to 1 of the node's own cycle, added "
        "every frame.\n\n"
        "Here a Master Gear four seconds long drives everything: its Phase turns the spiral and "
        "its Cycles turn the Cosine Gradient's palette once a loop. It is drawn as two meshing "
        "gears; its caption says the loop closes in one cycle, 4 s.\n\n"
        "Try:\n"
        "- Turn the Cosine Gradient's Offset. The colours slide through one cycle and come back "
        "at 1.\n"
        "- Pause, then press back to zero. Every gear starts its cycle again.\n"
        "- Set the Master Gear's Length to 2: everything goes twice as fast, and bends there "
        "rather than jumping.\n"
        "- Set its Display to Rosette.",
        width=420,
    )
    master = ws.master(520, 420, display="gears")
    spiral = ws.add(
        "spiral",
        520,
        60,
        controls={"turns": 4.0, "thickness": 0.18, "outerRadius": 1.6},
        options={"type": "logarithmic"},
    )
    twice = ws.add("multiply", 800, 420, controls={"b": 1.0})
    world = ws.add("worldcoordinates", 520, 700)
    radius = ws.add("pythagorean", 800, 700)
    arms = ws.add("add", 1060, 420)
    grad = ws.add("cosinegradient", 1060, 60, controls=RAINBOW)
    vig = ws.add("vignette", 1320, 60, controls={"strength": 0.9, "radius": 1.0})
    out = ws.output(1580, 60)
    ws.cable(master, "wrapped", twice, "a")
    ws.cable(twice, "output", spiral, "rotation")
    ws.cable(master, "cycles", grad, "clock")
    ws.cable(world, "x", radius, "a")
    ws.cable(world, "y", radius, "b")
    ws.cable(spiral, "angle", arms, "a")
    ws.cable(radius, "output", arms, "b")
    ws.cable(arms, "output", grad, "t")
    ws.cable(grad, "output", vig, "input")
    ws.cable(vig, "color", out, "input")


def seamless_noise(p):
    ws = p.workspace("Seamless noise", "A noise with Repeat on walks a circle, so the loop closes")
    ws.note(
        40,
        40,
        "Noise never repeats, unless you ask it to. Repeat is an option on every noise: Never, "
        "or every 1, 2, 4, 8 or 16 cells. With Repeat on, the noise walks a circle through its "
        "noise instead of a line, and comes back to where it began.\n\n"
        "Here a four-second Master Gear moves the Fractal and the Domain Warp one cell a loop, "
        "and both repeat every cell, so the loop closes. The same gear turns the palette once "
        "a loop.\n\n"
        "Try:\n"
        "- Set the Fractal's Repeat to Never. It drifts forever and the loop no longer closes.\n"
        "- Put a Ratio Gear at 2 : 1 into the Fractal's Time and set its Repeat to 2: a wider "
        "circle, twice as fast, and still seamless.\n"
        "- Turn the Fractal's Offset: it moves along the circle, and comes back at 1.\n\n"
        "The One clock tab plays this loop as a GIF.",
        width=400,
    )
    master = ws.master(480, 420)
    frac = ws.add(
        "fractal",
        480,
        40,
        controls={"scale": 1.4, "octaves": 3.0, "gain": 0.5, "contrast": 1.1},
        options={"type": "ridged", "repeat": "1"},
    )
    grad = ws.add("cosinegradient", 740, 40, controls=EMBER)
    warp = ws.add(
        "domainwarp",
        1000,
        40,
        controls={"amplitude": 0.3, "frequency": 1.2, "octaves": 2.0},
        options={"repeat": "1"},
    )
    vig = ws.add("vignette", 1260, 40, controls={"strength": 0.7, "radius": 0.95})
    out = ws.output(1520, 40)
    ws.cable(master, "cycles", frac, "clock")
    ws.cable(master, "cycles", warp, "clock")
    ws.cable(master, "cycles", grad, "clock")
    ws.cable(frac, "value", grad, "t")
    ws.cable(grad, "output", warp, "input")
    ws.cable(warp, "output", vig, "input")
    ws.cable(vig, "color", out, "input")


def one_beat(p):
    ws = p.workspace("Everything on one beat", "A Master Gear a beat long, and Ratio Gears for the bar and the phrase")
    ws.note(
        40,
        40,
        "One Master Gear half a second long counts beats at 120 BPM, drawn as gears. Ratio Gears "
        "under it count the rest: Teeth of 1 : 4 are a bar, 1 : 8 a two-bar phrase.\n\n"
        "The bar drives the Step Sequencer's Time — sixteen steps a bar — turns the picture "
        "once and moves the palette. A 3 : 1 gear on the phrase sways the Kaleidoscope through "
        "the Oscillator, three times a phrase, and the phrase moves the Simplex a cell. The "
        "sequencer's lanes kick the zoom and flash the colour.\n\n"
        "The Master Gear's caption says the loop closes in 8 cycles, 4 s: the 1 : 8 asks for "
        "eight beats.\n\n"
        "Try:\n"
        "- Turn the Master Gear's Length, slower or faster. Everything bends to the new "
        "tempo together, with no jump.\n"
        "- Press the Master Gear's Hold: the sequencer stops where it is. Press it again.\n"
        "- Set the phrase gear's Teeth to 1 : 16: the caption says 16 cycles.\n"
        "- Click cells in the sequencer's grid.",
        width=420,
    )
    beat = ws.master(500, 40, length=0.5, display="gears")
    bar = ws.gear(500, 380, 1, 4, clock=beat)
    phrase = ws.gear(500, 640, 1, 8, clock=beat)
    sway = ws.gear(500, 900, 3, clock=phrase)
    seq = ws.add(
        "stepsequencer",
        780,
        640,
        # A whole step: the first time system's sequencer, stepped by a clock's trigger, held
        # a lit lane for the whole step whatever its Gate said.
        controls={"gateLength": 1.0},
        values={
            "pattern": {
                "Cells": [
                    "x...x...x...x..x",
                    "....x.......x...",
                    "..x...x...x...x.",
                    "x.....x.....x...",
                ]
            }
        },
    )
    kick = ws.add("adsr", 1060, 640, controls={"attack": 0.005, "decay": 0.22, "sustain": 0.0, "release": 0.05})
    snare = ws.add("adsr", 1060, 900, controls={"attack": 0.005, "decay": 0.3, "sustain": 0.0, "release": 0.05})
    kick_zoom = ws.add("multiply", 1320, 640, controls={"b": 0.6})
    zoom = ws.add("add", 1580, 640, controls={"b": 1.0})
    snare_hue = ws.add("multiply", 1320, 900, controls={"b": 0.2})
    lfo = ws.add("oscillator", 780, 380, controls={"amplitude": 1.2})
    turn = ws.add("multiply", 780, 200, controls={"b": 1.0})

    noise = ws.add("simplex", 1060, 40, controls={"scale": 3.0, "contrast": 1.4}, options={"repeat": "1"})
    grad = ws.add("cosinegradient", 1320, 40, controls=DUSK)
    spin = ws.add("rotate", 1580, 40)
    kal = ws.add("kaleidoscope", 1840, 40, controls={"segments": 8.0}, width=240)
    shift = ws.add("colorshift", 2120, 40)
    out = ws.output(2380, 40, "1080x1080")

    ws.cable(bar, "cycles", seq, "clock")
    ws.cable(bar, "wrapped", turn, "a")
    ws.cable(turn, "output", spin, "angle")
    ws.cable(bar, "cycles", grad, "clock")
    ws.cable(sway, "cycles", lfo, "clock")
    ws.cable(phrase, "cycles", noise, "clock")
    ws.cable(lfo, "output", kal, "twist")
    ws.cable(seq, "lane1", kick, "gate")
    ws.cable(seq, "lane2", snare, "gate")
    ws.cable(kick, "value", kick_zoom, "a")
    ws.cable(kick_zoom, "output", zoom, "a")
    ws.cable(zoom, "output", kal, "zoom")
    ws.cable(snare, "value", snare_hue, "a")
    ws.cable(snare_hue, "output", shift, "hue")
    ws.cable(noise, "value", grad, "t")
    ws.cable(grad, "output", spin, "input")
    ws.cable(spin, "output", kal, "input")
    ws.cable(kal, "output", shift, "input")
    ws.cable(shift, "output", out, "input")


def bend(p):
    ws = p.workspace("Bend on Speed, lock on Teeth", "A gear locked to the master beside an Oscillator bending a Speed")
    ws.note(
        40,
        40,
        "A Ratio Gear is its parent times its Teeth, every frame: it never drifts and never "
        "bends. Change its Teeth and the layer jumps to where it would be had it always run at "
        "the new ratio. To bend a pace instead, run the node free and cable into its Speed.\n\n"
        "Here the dots turn on the Spin gear, Teeth 2 : 1 under the four-second Master Gear: "
        "twice a loop, locked to it. Rotozoom runs free, and an Oscillator once a loop swings "
        "its Speed between a half and one and a half, so its waves speed up and slow down and "
        "never skip. Shaky Cam shakes once a loop on the Master Gear.\n\n"
        "A cable in a Speed is one the caption cannot read ahead, so it says it can't tell; the "
        "Output's Duration, four seconds, is the render.\n\n"
        "Try:\n"
        "- Set the Spin gear's Teeth to 3 : 1. The dots jump to where three turns a loop puts "
        "them, and turn three times a loop from there.\n"
        "- Pull the cable out of Rotozoom's Speed and drag the Speed yourself: the waves bend, "
        "and never jump.",
        width=420,
    )
    master = ws.master(500, 700)
    swing = ws.add("oscillator", 760, 700, controls={"amplitude": 0.5, "offset": 1.0})
    spin = ws.gear(1020, 960, 2, clock=master, display="gears")
    twice = ws.add("multiply", 1540, 960, controls={"b": 1.0})
    dots = ws.add(
        "phyllotaxis",
        500,
        40,
        controls={"count": 420.0, "radius": 1.25, "dotSize": 0.045, "bg": "#140a24", "fg": "#ffd166"},
    )
    world = ws.add("worldcoordinates", 500, 440)
    radius = ws.add("pythagorean", 760, 440)
    tint = ws.add("cosinegradient", 1020, 440, controls=CANDY)
    colored = ws.add("layerblend", 1020, 40, options={"blend_mode": "multiply"})
    turn = ws.add("rotate", 1280, 440)
    roto = ws.add(
        "rotozoom",
        1280,
        40,
        controls={"turns": 5.0, "baseZoom": 1.1, "sinCoeff": 0.6, "cosCoeff": 0.4},
    )
    shaky = ws.add("shakycam", 1560, 40, controls={"amplitude": 0.035})
    ab = ws.add("chromaticaberration", 1840, 40, controls={"offset": 0.012})
    out = ws.output(2100, 40)
    ws.cable(master, "cycles", swing, "clock")
    ws.cable(swing, "output", roto, "speed")
    ws.cable(spin, "wrapped", twice, "a")
    ws.cable(twice, "output", turn, "angle")
    ws.cable(master, "cycles", shaky, "clock")
    ws.cable(master, "cycles", shaky, "clockY")
    ws.cable(master, "cycles", tint, "clock")
    ws.cable(world, "x", radius, "a")
    ws.cable(world, "y", radius, "b")
    ws.cable(radius, "output", tint, "t")
    ws.cable(dots, "output", colored, "background")
    ws.cable(tint, "output", colored, "foreground")
    ws.cable(colored, "output", turn, "input")
    ws.cable(turn, "output", roto, "input")
    ws.cable(roto, "output", shaky, "input")
    ws.cable(shaky, "output", ab, "input")
    ws.cable(ab, "output", out, "input")


def field(p):
    ws = p.workspace("Time as a field", "The distance from the middle cabled into Offset: one turn becomes a spiral")
    ws.note(
        40,
        40,
        "Offset is added to a node's Time, in its own cycles, and it can be different at every "
        "pixel.\n\n"
        "Here the distance from the middle, scaled by a Multiply, goes into Rotozoom's Offset: "
        "near the middle the picture is a little behind, at the edge a little ahead, so the "
        "stripes twist into a spiral, and the Rotate before it turns the spiral once a loop. The "
        "height goes into Shaky Cam's Offset X and Offset Y, so the shake travels down the picture like a flag. "
        "Everything runs on one Master Gear, so the field is the only difference between one "
        "pixel and the next.\n\n"
        "Rotozoom runs free at Speed 0: its waves hold still and only the field moves them, "
        "since Turns ties its turn to its waves. The steady turn is the Rotate's.\n\n"
        "Try:\n"
        "- Change the Multiply numbers: more lag, a tighter spiral. At 1 the top edge is a "
        "whole cycle ahead of the middle.\n"
        "- Cable the Master Gear's Cycles into Rotozoom's Time: the waves breathe, and "
        "Rotozoom turns five more times a loop.\n"
        "- Cable the distance into Shaky Cam's Offset X and Offset Y instead of the height: a ripple from the "
        "middle.\n"
        "- Pause: the twist stays, since the field is in Offset and not in the time.\n\n"
        "The field is added, never multiplied, so it moves the picture by what it says, however "
        "long the show has run, and the loop still closes.",
        width=420,
    )
    master = ws.master(500, 700)
    stripes = ws.add(
        "stripes",
        500,
        40,
        controls={"frequency": 4.5, "rotation": 0.125, "color1": "#0b1d3a", "color2": "#ff8c42", "smoothing": 0.6},
    )
    turn = ws.add("multiply", 760, 700, controls={"b": 1.0})
    spin = ws.add("rotate", 760, 40)
    world = ws.add("worldcoordinates", 500, 420)
    radial = ws.add("pythagorean", 760, 420)
    # The first time system's Time was in seconds of silvia's motion; Offset is in the node's
    # own cycles, 20π of them, so its lag of 3 and its sheet of 2.5 are divided by 20π.
    lag = ws.add("multiply", 1020, 560, controls={"b": 3.0 / (20.0 * PI)})
    sheet = ws.add("multiply", 1020, 760, controls={"b": 2.5 / (20.0 * PI)})
    roto = ws.add(
        "rotozoom",
        1020,
        40,
        controls={"turns": 5.0, "baseZoom": 0.7, "sinCoeff": 0.5, "cosCoeff": 0.5, "speed": 0.0},
    )
    shaky = ws.add("shakycam", 1300, 40, controls={"amplitude": 0.06})
    grad = ws.add("cosinegradient", 1580, 420, controls=OCEAN)
    luma = ws.add("luminosity", 1580, 40)
    out = ws.output(1840, 40)
    ws.cable(master, "wrapped", turn, "a")
    ws.cable(turn, "output", spin, "angle")
    ws.cable(master, "cycles", shaky, "clock")
    ws.cable(master, "cycles", shaky, "clockY")
    ws.cable(master, "cycles", grad, "clock")
    ws.cable(world, "x", radial, "a")
    ws.cable(world, "y", radial, "b")
    ws.cable(radial, "output", lag, "a")
    ws.cable(lag, "output", roto, "phaseOffset")
    ws.cable(world, "y", sheet, "a")
    ws.cable(sheet, "output", shaky, "phaseOffset")
    ws.cable(sheet, "output", shaky, "phaseOffsetY")
    ws.cable(stripes, "output", spin, "input")
    ws.cable(spin, "output", roto, "input")
    ws.cable(roto, "output", shaky, "input")
    ws.cable(shaky, "output", luma, "input")
    ws.cable(luma, "output", grad, "t")
    ws.cable(grad, "output", out, "input")


def one_clock(p):
    ws = p.workspace("One clock", "One gear drives a clip and an oscillator, so they cannot drift apart")
    ws.note(
        40,
        40,
        "One Ratio Gear drives a clip and an oscillator, so the two stay in step however the "
        "show is paused, sought or retimed.\n\n"
        "The Image/GIF plays the loop the Seamless noise tab renders. Its Time is the gear's "
        "Ping-pong, which plays it forward and back once a loop, kept a hair inside 0 to 1 by "
        "the Multiply and the Add, since a Time of exactly 1 is the first frame again. A 3 : 2 "
        "gear on the same gear runs the Oscillator three waves a loop, bending the ripple, and "
        "the gear's Phase turns the hue twice.\n\n"
        "Try:\n"
        "- Set the gear's Teeth to 1 : 1: clip, ripple and hue all go half speed, together.\n"
        "- Set them to 4 : 1: all three twice as fast, still in step.\n"
        "- Turn the GIF's Offset: it scrubs through the clip, on top of the gear.\n"
        "- Pull the cable out of the GIF's Time: it plays at its own speed.",
        width=420,
    )
    master = ws.master(500, 620)
    clock = ws.gear(500, 360, 2, clock=master, display="gears")
    waves = ws.gear(780, 620, 3, 2, clock=clock)
    gif = ws.add("imagegif", 500, 40, options={"file": "assets/seamless-noise.gif"})
    lfo = ws.add("oscillator", 780, 360, controls={"amplitude": 0.06}, options={"waveform": "triangle"})
    wave = ws.add(
        "wave",
        780,
        40,
        controls={"frequency": 6.0},
        options={"mode": "radial", "displacement": "ripple"},
    )
    kal = ws.add("kaleidoscope", 1060, 40, controls={"segments": 6.0}, options={"style": "classic"}, width=240)
    shift = ws.add("colorshift", 1340, 40)
    out = ws.output(1600, 40)
    # Kept a hair inside 0 to 1: a Time of exactly 1 is the GIF's first frame again.
    inside = ws.add("multiply", 1040, 620, controls={"b": 0.996})
    nudge = ws.add("add", 1300, 620, controls={"b": 0.002})
    # Hue is in turns: the gear's Phase, halved, shifts it half a turn a cycle.
    half = ws.add("multiply", 1300, 360, controls={"b": 0.5})
    ws.cable(clock, "pingpong", inside, "a")
    ws.cable(inside, "output", nudge, "a")
    ws.cable(nudge, "output", gif, "clock")
    ws.cable(waves, "cycles", lfo, "clock")
    ws.cable(lfo, "output", wave, "amplitude")
    ws.cable(clock, "wrapped", half, "a")
    ws.cable(half, "output", shift, "hue")
    ws.cable(gif, "output", wave, "input")
    ws.cable(wave, "output", kal, "input")
    ws.cable(kal, "output", shift, "input")
    ws.cable(shift, "output", out, "input")


def reverse(p):
    ws = p.workspace("Reverse the show", "Feedback trails that settle, and a Speed to run the breathing backwards")
    ws.note(
        40,
        40,
        "A triangle turns and breathes, and leaves echoes: the Output's own last frame, turned, "
        "zoomed, shifted in hue and dimmed, under the new one.\n\n"
        "The triangle runs on the Show gear, 1 : 1 of a four-second Master Gear: a 2 : 1 gear on "
        "it breathes the Oscillator twice a loop, and its Phase turns the triangle once.\n\n"
        "Try:\n"
        "- Switch the Oscillator to Free and set its Speed to -2. The triangle breathes "
        "backwards, and the echoes still grow and fade forwards: feedback runs once a frame, "
        "whatever the clocks say.\n"
        "- Set its Speed to 0: the triangle holds its size and the echoes stream out of it.\n"
        "- Set the Show gear's Teeth to 1 : 4: a slow-motion replay. The caption says four "
        "cycles.\n"
        "- Pause (Space): everything holds, the echoes too.\n\n"
        "The trails fade to nothing within a loop, so after a loop of warm-up the render comes "
        "back to its first frame.",
        width=420,
    )
    master = ws.master(500, 700)
    show = ws.gear(760, 700, 1, clock=master, display="gears")
    breathe = ws.gear(1020, 700, 2, clock=show)
    pulse = ws.add("oscillator", 500, 400, controls={"amplitude": 0.07, "offset": 0.17})
    spin2 = ws.add("multiply", 760, 520, controls={"b": 1.0})
    shape = ws.add(
        "polygon",
        780,
        40,
        controls={"sides": 3.0, "softness": 0.01, "foreground": "#ffb347", "background": "#000000"},
    )
    turn = ws.add("rotate", 1040, 400, controls={"angle": 0.02})
    trail = ws.add("zoom", 1300, 400, controls={"zoom": 1.07})
    fade = ws.add("colorshift", 1560, 400, controls={"hue": -0.0125, "value": 0.94})
    layer = ws.add("mix", 1300, 40)
    out = ws.output(1560, 40)
    ws.cable(breathe, "cycles", pulse, "clock")
    ws.cable(pulse, "output", shape, "radius")
    ws.cable(show, "wrapped", spin2, "a")
    ws.cable(spin2, "output", shape, "rotation")
    ws.cable(out, "frame", turn, "input")
    ws.cable(turn, "output", trail, "input")
    ws.cable(trail, "output", fade, "input")
    ws.cable(fade, "output", layer, "a")
    ws.cable(shape, "color", layer, "b")
    ws.cable(shape, "mask", layer, "amount")
    ws.cable(layer, "output", out, "input")


def wont_loop(p):
    ws = p.workspace("What won't loop", "A slime mold, a tunnel on ambient time and an XY Pad")
    ws.note(
        40,
        40,
        "Some things cannot close a loop, whatever the gears say.\n\n"
        "A slime mold grows, and a porthole drifts over it into a tunnel.\n\n"
        "- The Slime Mold is a simulation: each frame is the last one moved on, so its state "
        "at the end of a loop is not its state at the start. Its Rate is how many steps it "
        "takes, thirty times a second; it keeps no Time.\n"
        "- The XY Pad steers the porthole. It throws its puck with physics, and its Temperature "
        "keeps it wandering.\n"
        "- The Tunnel flies on ambient time, Loop mode with nothing in its Time: a 128th of its "
        "cycle a second, half a unit. Its cycle is a flight of 64 units, which comes back only "
        "every two minutes.\n\n"
        "No Master Gear here at all, so the render is the Output's Duration, four seconds, and "
        "it does not close.\n\n"
        "Try:\n"
        "- Cable a four-second Master Gear through a Ratio Gear at 1 : 4 into the Tunnel's "
        "Time: its flight now closes every sixteen seconds. The mold and the pad still do not.",
        width=420,
    )
    mold = ws.add(
        "slimemold",
        500,
        40,
        controls={
            "trailColor": "#ffc857",
            "agentColor": "#ff4f8b",
            "bgColor": "#0b0724",
            "sensorAngle": 45.0,
            "rotationAngle": 22.5,
            "sensorOffset": 15.0,
            "decay": 0.05,
            "jitter": 0.05,
            "stepsPerFrame": 20.0,
        },
        options={"population": "15", "gridScale": "10"},
    )
    pad = ws.add(
        "xypad",
        500,
        560,
        controls={"temperature": 1.5, "drag": 0.01, "minX": -0.5, "maxX": 0.5, "minY": -0.25, "maxY": 0.25},
    )
    tunnel = ws.add(
        "tunnel3d",
        800,
        40,
        controls={"twist": 0.8, "radius": 1.0, "zoom": 1.0},
        options={"mapping": "cartesian_mirror", "shading": "light", "clockMode": "loop"},
    )
    hole = ws.add("circle", 800, 560, controls={"radius": 0.42, "softness": 0.04})
    porthole = ws.add("mix", 1100, 40)
    out = ws.output(1360, 40)
    ws.cable(mold, "color", tunnel, "input")
    ws.cable(pad, "x", hole, "centerX")
    ws.cable(pad, "y", hole, "centerY")
    ws.cable(pad, "x", tunnel, "centerX")
    ws.cable(pad, "y", tunnel, "centerY")
    ws.cable(mold, "color", porthole, "a")
    ws.cable(tunnel, "output", porthole, "b")
    ws.cable(hole, "mask", porthole, "amount")
    ws.cable(porthole, "output", out, "input")


def documents():
    """The documents folder as the app resolves it: on macOS ~/Documents; elsewhere the last
    XDG_DOCUMENTS_DIR in $XDG_CONFIG_HOME/user-dirs.dirs (or ~/.config's) that is "$HOME",
    under "$HOME/" or absolute, else ~/Documents."""
    home = Path(os.environ.get("HOME") or Path.home())
    if sys.platform == "darwin":
        return home / "Documents"
    config = os.environ.get("XDG_CONFIG_HOME", "")
    config = Path(config) if os.path.isabs(config) else home / ".config"
    try:
        lines = (config / "user-dirs.dirs").read_text().splitlines()
    except OSError:
        lines = []
    for line in reversed(lines):
        key, eq, value = line.strip().partition("=")
        if not eq or key.strip() != "XDG_DOCUMENTS_DIR":
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] == '"':
            value = value[1:-1]
        value = value.replace('\\"', '"')
        if value == "$HOME":
            return home
        if value.startswith("$HOME/"):
            return home / value[len("$HOME/"):]
        if os.path.isabs(value):
            return Path(value)
    return home / "Documents"


def main():
    args = sys.argv[1:]
    registry = None
    if "--registry" in args:
        at = args.index("--registry")
        registry = args[at + 1]
        del args[at : at + 2]
    root = Path(args[0]).expanduser() if args else documents() / "supersilvia" / "Time Gears"
    p = Project(load_registry(registry))
    for build in (start_here, seamless_noise, one_beat, bend, field, one_clock, reverse, wont_loop):
        build(p)
    p.write(root)
    print(f"wrote {root}: {len(p.workspaces)} workspaces, {p._node} nodes")


if __name__ == "__main__":
    main()
