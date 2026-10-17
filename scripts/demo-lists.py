#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the "Lists" demo project: repeats whose copies differ, each by a list on the repeat.

    scripts/demo-lists.py [project folder] [--registry registry.json]

The folder defaults to "Lists" in the projects folder the app makes Untitled in, and is
replaced whole but for its renders/ and assets/. It borrows the Solids demo's Project and
Workspace (scripts/demo-solids.py), which check every cable, control and option against the
registry. Every Output is small, since every open tab's Output renders each frame.
"""

import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("demo_solids", HERE / "demo-solids.py")
solids = importlib.util.module_from_spec(spec)
spec.loader.exec_module(solids)

SMALL = "480x270"
C = solids.C


def show(ws, x, y, solid, master, controls=None):
    return ws.show(x, y, solid, master=master, sway=(0.06, 0.1), resolution=SMALL, controls=controls)


def bars(p):
    ws = p.workspace("Bars", "A row of boxes, each as tall as its item of a list")
    ws.note(
        40,
        40,
        "A Range of sixteen heights goes into the Repeat twice: into Count X, so there are as "
        "many boxes as items, and into Scale Y, so box i is as tall as item i. The box sits "
        "on the floor, so it grows upward. An Oscillator swings the Range's To, and every "
        "bar follows.\n\nTry: change the Range's Count; cable the Range into Turn as well.",
    )
    master = ws.master(C[0], 700)
    box = ws.shape("box", C[0], 40, width=0.32, height=1.0, depth=0.32, y=0.5, rounding=0.03, color="#e8925a")
    heights = ws.add("range", C[0], 400, controls={"count": 16.0, "from": 0.2, "to": 2.4})
    swing = ws.sway(C[0], 560, master, amplitude=0.8, offset=2.0)
    ws.cable(swing, "output", heights, "to")
    row = ws.mod("repeatgrid", C[1], 40, box, controls={"spacingX": 0.45, "spacingY": 0.0, "spacingZ": 0.0}, options={"extent": "counted", "perCopy": "on"})
    ws.cable(heights, "list", row, "countX")
    ws.cable(heights, "list", row, "copyScaleY")
    show(ws, C[2], 40, row, master, controls={"distance": 7.0, "tilt": 0.08})


def city(p):
    ws = p.workspace("City", "An eight by eight grid, its heights a ramp across all sixty-four")
    ws.note(
        40,
        40,
        "A Counted Repeat of 8 × 8 boxes, and one Range of sixty-four heights into Scale Y. "
        "A list stretches over the copies in order, X fastest, so the heights climb along each "
        "row and from row to row.\n\nTry: set the Range's Count to 8, and each row repeats the "
        "same eight heights; set it to 3.",
    )
    master = ws.master(C[0], 700)
    box = ws.shape("box", C[0], 40, width=0.6, height=1.0, depth=0.6, y=0.5, color="#9db4e8")
    heights = ws.add("range", C[0], 400, controls={"count": 64.0, "from": 0.2, "to": 3.0})
    grid = ws.mod(
        "repeatgrid",
        C[1],
        40,
        box,
        controls={"spacingX": 0.8, "spacingY": 0.0, "spacingZ": 0.8, "countX": 8.0, "countZ": 8.0},
        options={"extent": "counted", "perCopy": "on"},
    )
    ws.cable(heights, "list", grid, "copyScaleY")
    show(ws, C[2], 40, grid, master, controls={"distance": 11.0, "tilt": 0.12})


def stair(p):
    ws = p.workspace("Spiral stair", "Repeat Around with each step higher and turned a little more")
    ws.note(
        40,
        40,
        "Twenty-four slabs round a ring. One Range into Offset Y lifts each step higher than "
        "the last; another into Turn turns each one a little further about its own center.\n\n"
        "Try: cable the height Range into Scale X too.",
    )
    master = ws.master(C[0], 700)
    slab = ws.shape("box", C[0], 40, width=0.9, height=0.12, depth=0.35, color="#c9c4d6")
    rise = ws.add("range", C[0], 400, controls={"count": 24.0, "from": 0.0, "to": 3.5})
    twist = ws.add("range", C[0], 560, controls={"count": 24.0, "from": 0.0, "to": 0.5})
    ring = ws.mod("repeataround", C[1], 40, slab, controls={"copies": 24.0, "radius": 1.4}, options={"perCopy": "on"})
    ws.cable(rise, "list", ring, "copyY")
    ws.cable(twist, "list", ring, "copyTurn")
    show(ws, C[2], 40, ring, master, controls={"distance": 9.0, "tilt": 0.1})


def spikes(p):
    ws = p.workspace("Spikes", "Scatter on Sphere, each spike longer than the one above it")
    ws.note(
        40,
        40,
        "Two hundred cones on a ball, counted from the top down, and a Range into Scale Y: "
        "short spikes at the top, long at the bottom.",
    )
    master = ws.master(C[0], 700)
    cone = ws.shape("cone", C[0], 40, color="#e8c95a")
    lengths = ws.add("range", C[0], 400, controls={"count": 200.0, "from": 0.3, "to": 1.6})
    ball = ws.mod("scattersphere", C[1], 40, cone, controls={"copies": 200.0, "radius": 1.4}, options={"perCopy": "on"})
    ws.cable(lengths, "list", ball, "copyScaleY")
    show(ws, C[2], 40, ball, master, controls={"distance": 8.0})


def forever(p):
    ws = p.workspace("Forever", "An endless grid cycling a list of three, each row shifted by one")
    ws.note(
        40,
        40,
        "A Forever Repeat has no count to stretch a list over, so the list cycles: three "
        "heights, and each row starts one further along, which makes diagonals. A list of two "
        "makes a checkerboard.",
    )
    master = ws.master(C[0], 700)
    box = ws.shape("box", C[0], 40, width=0.7, height=1.0, depth=0.7, y=0.5, color="#8fd6a4")
    heights = ws.add("range", C[0], 400, controls={"count": 3.0, "from": 0.3, "to": 1.5})
    grid = ws.mod("repeatgrid", C[1], 40, box, controls={"spacingX": 1.0, "spacingY": 0.0, "spacingZ": 1.0}, options={"extent": "forever", "perCopy": "on"})
    ws.cable(heights, "list", grid, "copyScaleY")
    show(ws, C[2], 40, grid, master, controls={"distance": 9.0, "tilt": 0.15})


def main():
    args = sys.argv[1:]
    registry = None
    if "--registry" in args:
        at = args.index("--registry")
        registry = args[at + 1]
        del args[at : at + 2]
    root = Path(args[0]).expanduser() if args else solids.documents() / "supersilvia" / "Lists"
    p = solids.Project(solids.load_registry(registry))
    for build in (bars, city, stair, spikes, forever):
        build(p)
    p.write(root)
    print(f"wrote {root}: {len(p.workspaces)} workspaces, {p._node} nodes")


if __name__ == "__main__":
    main()
