#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the "Timing" demo project: one small patch per thing a node's Timing heading gives —
Speed, a cable into Speed, a field into Offset, and Free beside Loop.

    scripts/demo-timing.py [project folder] [--registry registry.json]

The folder defaults to ~/.local/share/supersilvia/projects/Timing and is replaced whole,
except for its renders/ and assets/, which are kept. Each workspace is a few nodes into an
Output, laid out left to right, with a note saying what it shows and what to try. Every
time-driven node has its Timing heading open, so its Speed or Time and its Offset show.

The file shape, the registry checks and the cable rule are `scripts/demo-time.py`'s, loaded
from beside this file: the node definitions come from `cargo run --release --example
registry` unless --registry names a dump, and every cable is the same port type or a uniform
into the varying of the same kind. A cable into Speed is checked here to land on a node
running Free, since Loop mode puts Speed away and the loader would drop it.
"""

import importlib.util
import sys
from pathlib import Path

# Loaded from its path, with no __pycache__ left beside it.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("demo_time", Path(__file__).with_name("demo-time.py"))
demo = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(demo)

DEFAULT_ROOT = Path.home() / ".local" / "share" / "supersilvia" / "projects" / "Timing"

# Cosine Gradient palettes: bias, amplitude, frequency and phase, R G B.
RAINBOW = demo.RAINBOW
EMBER = demo.EMBER
OCEAN = demo.OCEAN
DUSK = demo.DUSK


class Workspace(demo.Workspace):
    def timed(self, slug, x, y, controls=None, options=None, mode="free"):
        """A time-driven node with its Timing heading open, in `mode`."""
        opts = {"timing": "on", "clockMode": mode}
        opts.update(options or {})
        return self.add(slug, x, y, controls=controls, options=opts)

    def cable(self, a, out, b, into):
        if into in ("speed", "speedY"):
            node = next(n for n in self.nodes if n["id"] == b)
            assert node["options"].get("clockMode", "free") == "free", (
                f"{node['slug']}.{into} is put away in Loop mode"
            )
        super().cable(a, out, b, into)

    def output(self, x, y):
        return self.add("output", x, y)


class Project(demo.Project):
    def workspace(self, name, blurb):
        ws = Workspace(self, name, blurb)
        self.workspaces.append(ws)
        return ws


def speed(p):
    ws = p.workspace("Speed", "The same tunnel at three Speeds, and a Cosine Gradient turned up")
    ws.note(
        40,
        40,
        "Every node that moves has a Timing heading.\n\n"
        "Free is the mode a new node starts in. Its row is Speed.\n\n"
        "Speed is how fast the node runs. 1 is its normal pace. 0 is still. Below 0 it runs "
        "backwards.\n\n"
        "These three tunnels are the same node. Only Speed differs: 0.25, 1 and −1.\n\n"
        "Some nodes start still, at Speed 0. The Cosine Gradient is one. Here its Speed is "
        "turned up to 2, so its colours slide.\n\n"
        "Try:\n"
        "- Drag a Speed knob through 0. The tunnel slows, stops, and flies back.\n"
        "- Pause with F8. Everything stops. Play again and nothing jumps.\n"
        "- Turn the Cosine Gradient back to 0. Its colours stop.",
        width=400,
    )
    for i, (s, x, y) in enumerate(((0.25, 520, 40), (1.0, 1120, 40), (-1.0, 520, 600))):
        tunnel = ws.timed("tunnel3d", x, y, controls={"speed": s, "twist": 1.0})
        out = ws.output(x + 280, y)
        ws.cable(tunnel, "output", out, "input")
    world = ws.add("worldcoordinates", 1120, 600)
    grad = ws.timed("cosinegradient", 1120, 760, controls={"speed": 2.0, **RAINBOW})
    out = ws.output(1520, 600)
    ws.cable(world, "x", grad, "t")
    ws.cable(grad, "output", out, "input")


def lfo_into_speed(p):
    ws = p.workspace("LFO into Speed", "An Oscillator swings a tunnel's Speed: it swells, slows and reverses, and never jumps")
    ws.note(
        40,
        40,
        "Speed has a port. A cable can turn the knob for you.\n\n"
        "Here a slow Oscillator turns the tunnel's Speed. It swings from −1 to 2 and back, once "
        "every ten seconds.\n\n"
        "The flight speeds up, slows down, stops, and flies backwards. It never jumps.\n\n"
        "That is because Speed changes how fast the node runs, not where it is. The node keeps "
        "its own place and moves it on a little each frame.\n\n"
        "Try:\n"
        "- Raise the Oscillator's Level. The tunnel spends less time going backwards.\n"
        "- Set the Oscillator's Waveform to Square. The flight snaps between two speeds, and "
        "still does not jump.\n"
        "- Pull the cable out. The Speed knob comes back.",
        width=400,
    )
    lfo = ws.timed("oscillator", 520, 40, controls={"speed": 0.1, "amplitude": 1.5, "offset": 0.5})
    tunnel = ws.timed(
        "tunnel3d",
        900,
        40,
        controls={"twist": 1.2},
        options={"path": "helix", "shading": "light"},
    )
    out = ws.output(1180, 40)
    ws.cable(lfo, "output", tunnel, "speed")
    ws.cable(tunnel, "output", out, "input")


def linear_offset(p):
    ws = p.workspace("Linear gradient into Offset", "A ramp into Offset: each column a little behind the last")
    ws.note(
        40,
        40,
        "Offset is the last row under Timing. It moves the node ahead of where it is. 1 is one "
        "whole cycle.\n\n"
        "On a node that draws, Offset can be different at every pixel.\n\n"
        "Here a Linear Gradient goes into the Cosine Gradient's Offset. The ramp runs from 1 at "
        "the left to 0 at the right.\n\n"
        "So each column is a little behind the one to its left. The colour sweeps across the "
        "picture.\n\n"
        "The Luminosity turns the ramp's colour into a number Offset can take.\n\n"
        "Try:\n"
        "- Pull the cable out of Offset. The whole picture changes colour at once.\n"
        "- Turn the Linear Gradient's Angle. The sweep turns with it.\n"
        "- Set its Frequency to 2. Two sweeps fit across instead of one.",
        width=400,
    )
    ramp = ws.add(
        "lineargradient",
        520,
        40,
        # Angle 0 runs along X: (x + 2) / 4, near 0 at the left edge and near 1 at the right.
        controls={"angle": 0.0, "center": -2.0, "frequency": 0.25},
    )
    luma = ws.add("luminosity", 800, 40)
    grad = ws.timed("cosinegradient", 1060, 40, controls={"speed": 2.0, **EMBER})
    out = ws.output(1480, 40)
    ws.cable(ramp, "output", luma, "input")
    ws.cable(luma, "output", grad, "phaseOffset")
    ws.cable(grad, "output", out, "input")


def radial_offset(p):
    ws = p.workspace("Radial gradient into Offset", "A radial ramp into Offset: the motion spreads outward as rings")
    ws.note(
        40,
        40,
        "The same idea, from the middle out.\n\n"
        "A Radial Gradient goes into Offset. It is 1 in the middle and 0 at the edge. So the "
        "middle is ahead, and the colour spreads outward as rings.\n\n"
        "Its Loop Mode is Repeat, so the ramp starts again every ring. Each ring is one whole "
        "cycle of Offset, so the seams do not show.\n\n"
        "Try:\n"
        "- Drag the Radial Gradient's Radius. Wider rings, or tighter ones.\n"
        "- Move its Center X. The rings come from somewhere else.\n"
        "- Turn the Cosine Gradient's Speed below 0. The rings flow inward.",
        width=400,
    )
    rings = ws.add(
        "radialgradient",
        520,
        40,
        controls={"radius": 0.6},
        options={"loop_mode": "repeat"},
    )
    luma = ws.add("luminosity", 840, 40)
    grad = ws.timed("cosinegradient", 1100, 40, controls={"speed": 2.0, **OCEAN})
    out = ws.output(1520, 40)
    ws.cable(rings, "output", luma, "input")
    ws.cable(luma, "output", grad, "phaseOffset")
    ws.cable(grad, "output", out, "input")


def without_drift(p):
    ws = p.workspace("Different paces, without drift", "A field times a slow Oscillator into Offset: parts move ahead and fall back")
    ws.note(
        40,
        40,
        "Can parts of a picture run at different paces? Yes, through Offset.\n\n"
        "Here the radial field is multiplied by a slow Oscillator. That goes into Offset.\n\n"
        "When the Oscillator is 0, every pixel is in step and the picture is one colour. As it "
        "grows, the middle runs ahead and rings open. Then it falls back, and the picture comes "
        "together again.\n\n"
        "It never drifts apart. Offset says where each pixel is, so it always comes back.\n\n"
        "Speed is one number for the whole node. A different Speed at each pixel would add up "
        "differently every frame. After a minute the pixels would be far apart, and the picture "
        "would turn to noise.\n\n"
        "Try:\n"
        "- Raise the Oscillator's Amplitude. More rings open.\n"
        "- Set its Speed to 0. The rings hold where they are.",
        width=420,
    )
    # Once, and as wide as the frame's corner, so the field is 1 to 0 with no seam to scale.
    rings = ws.add("radialgradient", 540, 40, controls={"radius": 2.05})
    luma = ws.add("luminosity", 860, 40)
    lfo = ws.timed("oscillator", 540, 420, controls={"speed": 0.1, "amplitude": 3.0})
    times = ws.add("multiply", 1120, 40)
    grad = ws.timed("cosinegradient", 1380, 40, controls={"speed": 1.0, **DUSK})
    out = ws.output(1800, 40)
    ws.cable(rings, "output", luma, "input")
    ws.cable(luma, "output", times, "a")
    ws.cable(lfo, "output", times, "b")
    ws.cable(times, "output", grad, "phaseOffset")
    ws.cable(grad, "output", out, "input")


def free_vs_loop(p):
    ws = p.workspace("Free vs Loop", "The same noise on a Master Gear in Loop mode, and on its own in Free mode")
    ws.note(
        40,
        40,
        "The other mode is Loop. Click Loop on the Timing heading.\n\n"
        "In Loop the row is Time, a diamond with no knob. Cable a gear's Cycles into it, and "
        "the gear drives the node exactly.\n\n"
        "Both noises here repeat every 4 cells. The top one is in Loop, on a two-second Master "
        "Gear. The bottom one is in Free, at Speed 1. They start in step.\n\n"
        "The gear's caption says it loops in 4 cycles, 8 seconds. That counts the Loop "
        "noise. The Free one is on its own clock.\n\n"
        "Pick Loop for a seamless render, or to lock to a beat. Pick Free to play a node by "
        "hand, or to push its Speed with a cable.\n\n"
        "Try:\n"
        "- Press Hold on the Master Gear. Only the top noise stops.\n"
        "- Turn the bottom noise's Speed. Only it changes pace.\n"
        "- Set the Master Gear's Length to 1. The top noise doubles its pace.",
        width=420,
    )
    master = ws.master(540, 40, length=2.0, display="gears")
    looped = ws.timed(
        "perlin",
        860,
        40,
        controls={"scale": 4.0, "foreground": "#ffb347", "background": "#1a0f2e"},
        options={"repeat": "4"},
        mode="loop",
    )
    out_loop = ws.output(1140, 40)
    free = ws.timed(
        "perlin",
        860,
        560,
        controls={"scale": 4.0, "foreground": "#7fd1ff", "background": "#0b1a2e"},
        options={"repeat": "4"},
    )
    out_free = ws.output(1140, 560)
    ws.cable(master, "cycles", looped, "clock")
    ws.cable(looped, "color", out_loop, "input")
    ws.cable(free, "color", out_free, "input")


def main():
    args = sys.argv[1:]
    registry = None
    if "--registry" in args:
        at = args.index("--registry")
        registry = args[at + 1]
        del args[at : at + 2]
    root = Path(args[0]).expanduser() if args else DEFAULT_ROOT
    p = Project(demo.load_registry(registry))
    for build in (speed, lfo_into_speed, linear_offset, radial_offset, without_drift, free_vs_loop):
        build(p)
    p.write(root)
    print(f"wrote {root}: {len(p.workspaces)} workspaces, {p._node} nodes")


if __name__ == "__main__":
    main()
