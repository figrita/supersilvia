#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the "Solids" demo project: one small patch per thing three-dimensional solids can do,
then one per thing they cannot do for free.

    scripts/demo-solids.py [project folder] [--registry registry.json]

The folder defaults to "Solids" in the projects folder the app makes Untitled in, found as
scripts/demo-time.py finds it, and is replaced whole, except for its
renders/ and assets/, which are kept. Each workspace is one idea into one Output, with a note
saying what to look at and which knob to turn, and a Master Gear whose loop the render takes.

**A scene is a chain.** Every shape node takes the solid built so far on its first row, Onto,
and lands its own shape on it by its Mode — Union, Subtract or Intersect — melted in by Blend;
it turns and scales its own shape about its own center under its folded Transform rows. So a
scene here is a row of shapes cabled one into the next, read left to right, ending in Render 3D.
A Union node appears only where two branches each changed by its own modifier (a Twist, a
Texture, a Repeat) meet, and a Place only where a whole group is moved: a modifier changes
everything cabled into it, so a shape it should not touch goes after it in the chain or on a
branch of its own.

The first half shows the abilities: the chain, the primitives and the newer shapes, the modes,
the transforms, repeats and scatters and folds, noise, seams, color fields, pictures on a
solid's skin and made into solids, a render going on into video nodes, motion from a gear, a
render with a transparent background, and Render 3D's options. The second half shows the
limits, each with the knob that reveals it: a copy wider than its cell is sliced; a stretch, a
twist or a displacement makes the march take more steps, which the Steps shading paints; a
texture far off sparkles, since no texture has mipmaps; a blur after a render marches the
render again for every tap; and fine detail to infinity runs out of steps.

The node definitions come from `cargo run --release --example registry`, which this runs
unless --registry names a dump, and every cable, control, option and value is checked against
them as in `scripts/demo-time.py`, whose Project and Workspace this borrows. A cable between
two workspaces — an Output's Frame into a Texture's Picture — goes in project.ssp's
`connections`, as the app saves one.

Then render the loops, one GIF a workspace into the project's renders/:

    cargo run --release --example loop_gifs -- ~/Documents/supersilvia/Solids --pre-roll 0

Nothing here feeds back, so no pre-roll is needed. loop_gifs opens one tab at a time, and an
Output on a closed tab is suspended, so the two tabs that wear the Video for solids Frame —
Video onto solids and Far textures sparkle — render without their picture moving there; in the
app, with Video for solids open as this project opens it, they wear it.
"""

import json
import math
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LOOP = 4.0
FPS = 25.0
RES = "960x540"
HEAVY = "640x360"

VARYING_OF = {"uniform number": "varying number", "uniform color": "varying color"}

# Options whose value is any of a kind rather than one of the listed choices.
FREE_OPTIONS = ("file", "resolution", "size", "texsize")


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


# The rows a shape's own Transform heading folds, by the names this script calls them.
TRANSFORM_KEYS = {"yaw": "tYaw", "pitch": "tPitch", "roll": "tRoll", "scale": "tScale"}


class Workspace:
    """One workspace file's nodes and cables, checked against the registry as they are added."""

    def __init__(self, project, name, blurb):
        self.project = project
        self.reg = project.reg
        self.name = name
        self.blurb = blurb
        self.nodes = []
        self.connections = []
        self.loop = LOOP
        self.id = project.next_workspace()

    def add(self, slug, x, y, controls=None, options=None, values=None, width=None):
        assert slug in self.reg, f"no node {slug} in the registry"
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
            assert not choices or value in choices or key in FREE_OPTIONS, f"{slug}.{key}={value} not in {choices}"
        saved["options"] = dict(options or {})
        if values:
            keys = {v["key"] for v in d["values"]}
            for key in values:
                assert key in keys, f"{slug} has no value {key}"
            saved["values"] = values
        if width:
            saved["width"] = float(width)
        self.nodes.append(saved)
        self.project.owner[node_id] = (self, saved)
        return node_id

    def note(self, x, y, text, width=380):
        return self.add("note", x, y, values={"text": {"Text": text}}, width=width)

    def cable(self, a, out, b, into):
        self.project.cable(a, out, b, into)

    def solid_input(self, slug):
        """The key of a modifier's one solid input: not a shape's Onto, nor a boolean's A."""
        keys = [i["key"] for i in self.reg[slug]["inputs"] if i["type"] == "solid" and i["key"] != "onto"]
        assert len(keys) == 1, f"{slug} has solid inputs {keys}"
        return keys[0]

    def shape(self, slug, at_x, at_y, onto=None, mode=None, blend=None, turn=None, options=None, **controls):
        """A shape node landing on `onto` by `mode`, melted in by `blend`; `turn` is its own
        Transform rows, {"yaw", "pitch", "roll", "scale"}, which are unfolded when set."""
        controls = dict(controls)
        options = dict(options or {})
        if mode:
            options["mode"] = mode
        if blend is not None:
            controls["blend"] = blend
        if turn:
            for k, v in turn.items():
                controls[TRANSFORM_KEYS[k]] = v
            options["transform"] = "on"
        n = self.add(slug, at_x, at_y, controls=controls, options=options)
        if onto is not None:
            self.cable(onto, "solid", n, "onto")
        return n

    def chain(self, x0, y, steps, onto=None, dx=280):
        """Shapes one after another, each landing on the one before; `steps` are
        (slug, keyword arguments of `shape`). The last is returned."""
        x = x0
        for slug, kw in steps:
            onto = self.shape(slug, x, y, onto=onto, **kw)
            x += dx
        return onto

    def mod(self, slug, x, y, solid, controls=None, options=None):
        """A modifier of `solid`: a transform, a repeat, a texture, a color field."""
        n = self.add(slug, x, y, controls=controls, options=options)
        self.cable(solid, "solid", n, self.solid_input(slug))
        return n

    def pair(self, slug, x, y, a, b, controls=None, options=None):
        """A node of two solids, A and B."""
        n = self.add(slug, x, y, controls=controls, options=options)
        self.cable(a, "solid", n, "a")
        self.cable(b, "solid", n, "b")
        return n

    def output(self, x, y, resolution=RES, duration=None, transparent=False):
        """An Output as long as the workspace's Master Gear, so a render where no gear can say
        how long a loop is takes one loop of it."""
        duration = self.loop if duration is None else duration
        options = {"resolution": resolution, "writer": "gif"}
        if transparent:
            options["transparent"] = "on"
        return self.add(
            "output",
            x,
            y,
            controls={"fps": FPS, "duration": duration, "warmup": 0.0},
            options=options,
        )

    def master(self, x, y, length=LOOP, display="rosette"):
        """A Master Gear `length` seconds long."""
        self.loop = length
        return self.add("mastergear", x, y, controls={"length": length}, options={"display": display})

    def sway(self, x, y, master, amplitude=0.05, offset=0.08):
        """An Oscillator once a loop of `master`, for a knob to swing on."""
        osc = self.add("oscillator", x, y, controls={"amplitude": amplitude, "offset": offset})
        self.cable(master, "cycles", osc, "clock")
        return osc

    def render(self, x, y, solid, controls=None, options=None):
        r = self.add("solidrender", x, y, controls=controls, options=options)
        self.cable(solid, "solid", r, "solid")
        return r

    def union(self, x, y, solids, smooth=0.0):
        """Every one of `solids` as one, through as many Unions as four inputs a piece take:
        only for branches each changed by a modifier of its own."""
        solids = list(solids)
        while len(solids) > 1:
            u = self.add("union", x, y, controls={"smooth": smooth})
            for key, s in zip("abcd", solids[:4]):
                self.cable(s, "solid", u, key)
            solids = [u] + solids[4:]
            y += 200
        return solids[0]

    def place(self, at_x, at_y, solid, **controls):
        """A Place moving a whole group."""
        return self.mod("place", at_x, at_y, solid, controls=controls)

    def show(self, x, y, solid, master=None, sway=None, resolution=RES, controls=None, options=None):
        """`solid` through a Render 3D into an Output, the camera's Orbit swung once a loop of
        `master` by `sway` (amplitude, offset) or turned whole by its Wrapped where `sway` is
        "turn". The Render 3D is returned."""
        r = self.render(x, y, solid, controls=controls, options=options)
        if master is not None and sway == "turn":
            self.cable(master, "wrapped", r, "orbit")
        elif master is not None and sway:
            osc = self.sway(x, y + 900, master, amplitude=sway[0], offset=sway[1])
            self.cable(osc, "output", r, "orbit")
        self.cable(r, "output", self.output(x + 280, y, resolution=resolution), "input")
        return r

    def height(self, node):
        """About how tall `node` is drawn on the canvas: its header, a row for each input and
        option shown, its declared bands — a trace, a thumbnail, a preview — and its foot."""
        d = self.reg[node["slug"]]
        folded = node["options"].get("transform", "off") != "on"
        rows = [i for i in d["inputs"] if not i["hidden"] and not (folded and i["key"] in TRANSFORM_KEYS.values())]
        h = 30 + 22 * len(rows)
        for o in d["options"]:
            h += 18 if o.get("heading") else 20
        for v in d["values"]:
            h += 16 * v.get("rows", 1) + 12
        for r in d["regions"]:
            h += 70 if r.get("heading") and r["heading"]["key"] == "trace" else 130
        return h + 20

    def width_of(self, node):
        return max(node.get("width") or 0.0, self.reg[node["slug"]]["width"] or 180.0, *(r.get("width") or 0.0 for r in self.reg[node["slug"]]["regions"]))

    def layout(self):
        """Every node but the note in columns left to right by how far down the dataflow it
        is, so a chain reads as a row; a node that only drives numbers — a gear, an
        oscillator — goes in the column before the first node it drives, under the rest.
        Each column is stacked by the real height of its nodes."""
        nodes = [n for n in self.nodes if n["slug"] != "note"]
        ids = {n["id"] for n in nodes}
        preds = {n["id"]: [] for n in nodes}
        succs = {n["id"]: [] for n in nodes}
        for c in self.connections:
            a, b = c["from"]["node"], c["to"]["node"]
            if a in ids and b in ids:
                preds[b].append(a)
                succs[a].append(b)
        by_id = {n["id"]: n for n in nodes}
        control = {
            n["id"]
            for n in nodes
            if all(o["type"] not in ("solid", "varying color") for o in self.reg[n["slug"]]["outputs"])
            and succs[n["id"]]
        }
        col = {}
        for _ in range(len(nodes) + 1):
            for n in nodes:
                i = n["id"]
                ps = [col.get(p, 0) for p in preds[i] if p not in control]
                col[i] = max(ps) + 1 if ps else 0
        for _ in range(len(nodes) + 1):
            for i in control:
                col[i] = max(0, min(col[s] for s in succs[i]) - 1)
        columns = {}
        for n in nodes:
            columns.setdefault(col[n["id"]], []).append(n)
        x = 520.0
        centre = {}
        top = {}
        order = {n["id"]: k for k, n in enumerate(nodes)}
        for c in range(max(columns) + 1 if columns else 0):
            members = columns.get(c, [])
            if not members:
                continue

            def key(n):
                ys = [centre[p] for p in preds[n["id"]] if p in centre]
                return (n["id"] in control, sum(ys) / len(ys) if ys else -1.0, order[n["id"]])

            members.sort(key=key)
            y = 40.0
            for n in members:
                # Level with the node it carries on from, so a branch reads as a row.
                first = next((p for p in preds[n["id"]] if p in top and p not in control), None)
                if first is not None:
                    y = max(y, top[first])
                n["pos"] = {"x": x, "y": y}
                top[n["id"]] = y
                h = self.height(n)
                centre[n["id"]] = y + h / 2
                y += h + 40
            x += max(self.width_of(n) for n in members) + 80

    def file(self):
        self.layout()
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
        self.owner = {}
        # Cables between two workspaces, which project.ssp holds.
        self.connections = []
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

    def slug_of(self, node_id):
        return self.owner[node_id][1]["slug"]

    def all_connections(self):
        return self.connections + [c for ws in self.workspaces for c in ws.connections]

    def output_type(self, node_id, key):
        d = self.reg[self.slug_of(node_id)]
        out = next((o for o in d["outputs"] if o["key"] == key), None)
        assert out, f"{self.slug_of(node_id)} has no output {key}"
        if out["dual"]:
            # A diamond where every cable into the node is a uniform number.
            into = [c for c in self.all_connections() if c["to"]["node"] == node_id]
            if all(self.output_type(c["from"]["node"], c["from"]["key"]) == "uniform number" for c in into):
                return "uniform number"
        return out["type"]

    def cable(self, a, out, b, into):
        src = self.output_type(a, out)
        d = self.reg[self.slug_of(b)]
        i = next((i for i in d["inputs"] if i["key"] == into), None)
        assert i, f"{self.slug_of(b)} has no input {into}"
        # A hidden input is a setting on the node, not a port: it takes a value, never a cable.
        assert not i["hidden"], f"{self.slug_of(b)}.{into} is a setting, not a port"
        dst = i["type"]
        assert src == dst or VARYING_OF.get(src) == dst, (
            f"{self.slug_of(a)}.{out} ({src}) cannot feed {self.slug_of(b)}.{into} ({dst})"
        )
        assert not any(c["to"] == {"node": b, "key": into} for c in self.all_connections()), f"{into} is cabled twice"
        # A cable into a Time is a clock: the node loops on it rather than running free.
        node = self.owner[b][1]
        if into in ("clock", "clockY") and any(o["key"] == "clockMode" for o in d["options"]):
            node["options"]["clockMode"] = "loop"
        c = {"from": {"node": a, "key": out}, "to": {"node": b, "key": into}}
        ws_a, ws_b = self.owner[a][0], self.owner[b][0]
        (ws_a.connections if ws_a is ws_b else self.connections).append(c)

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
            "connections": self.connections,
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


# Columns on the canvas: the note at the left, then the dataflow left to right.
C = [480 + 280 * i for i in range(16)]


# ------------------------------------------------------------------------------------------
# Abilities


def chain_demo(p):
    ws = p.workspace("Chain", "A scene as a row of shapes, each landing on the ones before it")
    ws.note(
        40,
        40,
        "Read the row left to right. Every shape takes what was built so far on its first "
        "row, Onto, and lands its own shape on it by its Mode: a Box; a Sphere set to "
        "Intersect keeps only the box inside the ball, a rounded die; a Cylinder set to "
        "Subtract, turned a quarter on its own Pitch, bores a hole through it; a Torus is added "
        "round its middle with Blend melting the seam; a Cone sits on top; the Ground goes "
        "last, so the Intersect before it never cuts it. No Union nodes: the chain is the "
        "union.\n\n"
        "Try: drag the Ground to the start of the row and the Intersect cuts it away to a "
        "disc; set the Cylinder's Mode to Union; turn the Torus's Blend up to 0.6. A shape's "
        "own turn and scale are under its Transform heading.",
        width=420,
    )
    master = ws.master(C[0], 700)
    scene = ws.chain(
        C[0],
        40,
        [
            ("box", dict(width=1.8, height=1.8, depth=1.8, rounding=0.04, color="#c9c4d6")),
            ("sphere", dict(mode="intersect", radius=1.22, color="#e8925a")),
            ("cylinder", dict(mode="subtract", radius=0.42, height=3.0, color="#f2b632", turn={"pitch": 0.25})),
            ("torus", dict(ring=1.12, tube=0.16, blend=0.25, color="#5a8fe8")),
            ("cone", dict(y=1.35, bottom=0.45, top=0.0, height=0.9, blend=0.15, color="#8fd17a")),
            ("ground", dict(height=-0.9, color="#4a4652")),
        ],
    )
    ws.show(C[6], 40, scene, master, "turn", controls={"distance": 4.8, "tilt": 0.07, "targetY": 0.15, "fog": 0.02}, options={"floor": "none"})


def primitives(p):
    ws = p.workspace("Primitives", "Every primitive in one chain, each with its own place and color")
    ws.note(
        40,
        40,
        "Every primitive, one chain: each shape lands on the ones before it, so the row needs "
        "no Union. A primitive has its own X, Y, Z and Color, and under its Transform heading "
        "its own Yaw, Pitch, Roll and Scale: the Torus stands up by its own Pitch. The chain "
        "starts with a Gyroid, which fills all of space, Intersected with the Sphere before it "
        "— so it comes first, before there is anything else for the Intersect to cut.\n\n"
        "Try: drag Render 3D's Orbit and Tilt to walk round them, or set its Shading to Normals "
        "to see which way each surface faces.",
    )
    master = ws.master(C[0], 700)
    scene = ws.chain(
        C[0],
        40,
        [
            ("sphere", dict(x=5.2, z=-3.5, radius=1.0)),
            ("gyroid", dict(mode="intersect", scale=0.9, thickness=0.12, color="#e88a7a")),
            ("sphere", dict(x=-6.5, radius=1.0, color="#e8925a")),
            ("box", dict(x=-3.9, y=-0.2, width=1.6, height=1.6, depth=1.6, rounding=0.08, color="#5a8fe8")),
            ("cylinder", dict(x=-1.3, radius=0.75, height=2.0, color="#8fd17a")),
            ("capsule", dict(x=1.3, y=0.1, length=1.2, radius=0.5, color="#e8d35a")),
            ("octahedron", dict(x=3.9, y=0.2, size=1.2, color="#b07ae8")),
            ("torus", dict(x=6.5, y=0.15, ring=0.85, tube=0.3, color="#d67a9e", turn={"pitch": 0.25})),
            ("cone", dict(x=-5.2, z=-3.5, bottom=0.9, top=0.0, height=2.0, color="#e86a5a")),
            ("hexprism", dict(x=-2.6, z=-3.5, y=-0.2, radius=0.8, height=1.6, color="#5ad1c4")),
            ("pyramid", dict(x=0.0, z=-3.5, y=-0.2, base=1.8, height=1.6, color="#e8b45a")),
            ("link", dict(x=2.6, z=-3.5, y=0.2, color="#9ab0ff")),
        ],
        dx=200,
    )
    ws.show(C[9], 40, scene, master, (0.04, 0.0), controls={"distance": 10.0, "tilt": 0.08, "targetZ": -1.6, "fog": 0.03})


def more_shapes(p):
    ws = p.workspace("More shapes", "Box Frame, Superellipsoid, Ellipsoid, Helix, Dodecahedron and Icosahedron on a tipped Slab")
    ws.note(
        40,
        40,
        "The newer shapes in one chain, on a Slab tipped toward the camera by its Pitch: a Box "
        "Frame, a Superellipsoid whose Exponent an Oscillator swings from a star (under 1) "
        "through an octahedron (1) and a ball (2) to a rounded box, an Ellipsoid, a Helix, a "
        "Dodecahedron and an Icosahedron.\n\n"
        "Try: the Helix's Pitch and Tube, the Box Frame's Edge, the Slab's Yaw and Pitch, or "
        "stop the Oscillator and drag the Exponent yourself.",
    )
    master = ws.master(C[0], 700)
    scene = ws.chain(
        C[0],
        40,
        [
            ("slab", dict(y=-1.0, pitch=0.01, thickness=0.4, color="#4a4652")),
            ("boxframe", dict(x=-5.0, y=0.0, width=1.6, height=1.6, depth=1.6, edge=0.1, turn={"yaw": 0.1})),
            ("superellipsoid", dict(x=-3.0, y=0.0, rx=0.85, ry=0.85, rz=0.85)),
            ("ellipsoid", dict(x=-1.0, y=-0.1, rx=0.95, ry=0.6, rz=0.6, turn={"roll": 0.08})),
            ("helix", dict(x=1.0, y=0.0, radius=0.6, pitch=0.45, tube=0.1, height=2.0)),
            ("dodecahedron", dict(x=3.0, y=0.0, size=0.95, turn={"yaw": 0.05})),
            ("icosahedron", dict(x=5.0, y=0.0, size=0.95, turn={"yaw": 0.05})),
        ],
    )
    superellipsoid = next(n["id"] for n in ws.nodes if n["slug"] == "superellipsoid")
    swing = ws.sway(C[2], 400, master, amplitude=2.6, offset=3.2)
    ws.cable(swing, "output", superellipsoid, "exponent")
    ws.show(C[8], 40, scene, master, (0.03, 0.0), controls={"distance": 7.6, "tilt": 0.07, "fog": 0.02, "targetY": 0.1}, options={"floor": "none"})


def booleans(p):
    ws = p.workspace("Booleans", "Union with Blend, Subtract and Intersect as each shape's Mode")
    ws.note(
        40,
        40,
        "Three pairs, each a box and a ball, as one chain: a shape's Mode says how it lands on "
        "everything before it. Right, the ball Intersects the box: only where both are. Left, "
        "the ball is a Union with Blend, melting into its box, colors blending across the "
        "seam. Middle, the ball Subtracts and the cut takes its gold. The Intersect pair comes "
        "first, since an Intersect keeps only what is inside it of everything before.\n\n"
        "Try: each ball's Blend, which melts a union and rounds a cut. The Union, Subtract and "
        "Intersect nodes are still there for two branches that each had their own modifier.",
    )
    master = ws.master(C[0], 700, length=8.0)
    scene = ws.chain(
        C[0],
        40,
        [
            ("box", dict(x=4.0, width=1.7, height=1.7, depth=1.7, color="#8fd17a")),
            ("sphere", dict(x=4.0, mode="intersect", radius=1.1, color="#d67a9e")),
            ("box", dict(x=-4.0, y=-0.25, width=1.5, height=1.5, depth=1.5, color="#5a8fe8")),
            ("sphere", dict(x=-3.65, y=0.5, radius=0.7, blend=0.5, color="#e8925a")),
            ("box", dict(y=-0.1, width=1.8, height=1.8, depth=1.8, rounding=0.04, color="#c9c4d6")),
            ("sphere", dict(y=-0.1, mode="subtract", radius=1.18, color="#f2b632")),
        ],
    )
    ws.show(C[6], 40, scene, master, "turn", controls={"distance": 8.5, "tilt": 0.09, "fog": 0.03})


def transforms(p):
    ws = p.workspace("Transforms", "A shape's own turn, Stretch, Twist, Bend and Mirror Space")
    ws.note(
        40,
        40,
        "A shape turns itself: the octahedron at the left is turned by its own Yaw, Pitch and "
        "Roll, under its Transform heading. A modifier — Stretch, Twist, Bend, Mirror Space — "
        "changes everything cabled into it, so a shape it should not touch goes after it in "
        "the chain: the Twist winds the bar, and the octahedron after it is not wound. The "
        "egg, the arch and the four pearls each have a modifier of their own, so each is a "
        "branch, and this is where a Union belongs: it meets branches. Bend and Mirror Space "
        "work about the center, so a Place moves each bent group into the row afterwards.\n\n"
        "Try: Twist's Rate, Bend's Curvature, Stretch's Y, and Mirror Space's Across.",
        width=420,
    )
    master = ws.master(C[0], 1300, length=8.0)
    bar = ws.add("box", C[0], 40, controls={"width": 0.8, "height": 2.4, "depth": 0.8, "rounding": 0.04, "color": "#5a8fe8"})
    twist = ws.mod("twist", C[1], 40, bar, controls={"rate": 0.18})
    wound = ws.shape("octahedron", C[2], 40, onto=twist, x=-6.0, y=0.2, size=1.0, color="#b07ae8", turn={"yaw": 0.08, "pitch": 0.1, "roll": 0.06})

    ball = ws.add("sphere", C[0], 300, controls={"x": -3.0, "radius": 0.6, "color": "#e8925a"})
    egg = ws.mod("stretch", C[1], 300, ball, controls={"sx": 1.0, "sy": 1.6, "sz": 1.0})

    plank = ws.add("box", C[0], 560, controls={"width": 2.6, "height": 0.35, "depth": 0.7, "rounding": 0.04, "color": "#8fd17a"})
    bend = ws.mod("bend", C[1], 560, plank, controls={"curvature": 0.7})
    arch = ws.place(C[2], 560, bend, x=3.0, y=0.2)

    pearl = ws.add("sphere", C[0], 820, controls={"x": 0.5, "z": 0.5, "radius": 0.42, "color": "#e8d35a"})
    mirror = ws.mod("solidmirror", C[1], 820, pearl, options={"axes": "xz"})
    pearls = ws.place(C[2], 820, mirror, x=6.0, y=-0.58)

    scene = ws.union(C[3], 40, [wound, egg, arch, pearls])
    ws.show(C[4], 40, scene, master, (0.05, 0.0), controls={"distance": 8.6, "tilt": 0.06, "fog": 0.03})


def repeat_forever(p):
    ws = p.workspace("Repeat forever", "A grid of copies to the horizon, each scattered, spun, sized and tinted")
    ws.note(
        40,
        40,
        "One rounded box, repeated on a grid that goes on forever: the Repeat folds every point "
        "into one cell, so endless copies cost what one does. Each copy is moved, turned, "
        "shrunk and recolored by its cell and the Seed, and the fog fades them to the sky. The "
        "Ground after the Repeat is not repeated: a modifier changes only what is cabled into "
        "it.\n\n"
        "Try: change the Repeat's Seed for a different crowd, push Scatter, Spin, Size and Tint, "
        "or set Extent to Counted.",
    )
    master = ws.master(C[0], 700, length=8.0)
    box = ws.add("box", C[0], 40, controls={"width": 1.1, "height": 1.1, "depth": 1.1, "rounding": 0.18, "color": "#e8e2d6"})
    grid = ws.mod(
        "repeatgrid",
        C[1],
        40,
        box,
        controls={"spacingX": 2.6, "spacingZ": 2.6, "scatter": 0.2, "spin": 1.0, "size": 0.5, "tint": 0.85, "seed": 7.0},
        options={"extent": "forever"},
    )
    floor = ws.shape("ground", C[2], 40, onto=grid, height=-0.6, color="#6a5f8a")
    ws.show(
        C[3],
        40,
        floor,
        master,
        "turn",
        controls={"distance": 9.0, "tilt": 0.035, "targetY": 0.5, "fog": 0.05, "background": "#6a5f8a", "lens": 1.4, "step": 0.6},
        options={"floor": "none"},
    )


def repeat_around(p):
    ws = p.workspace("Repeat around", "A ring, an arc and a spiral staircase from Repeat Around")
    ws.note(
        40,
        40,
        "Three Repeat Arounds. Left, twelve boxes in a whole ring standing on Z. Middle, Spread "
        "at half a turn makes an arch of seven balls. Right, round Y, each step Rises, Turns "
        "and Grows a little more than the one before: a spiral staircase, its post a Cylinder "
        "chained on after the Repeat so there is one post, not eighteen. Each repeat turns "
        "about the center, so a Place moves each group into the row, and a Union meets the "
        "three.\n\n"
        "Try: the ring's Copies, the arch's Spread, and the staircase's Rise and Grow — Grow too "
        "far and the last steps are sliced by their neighbors' cells.",
    )
    master = ws.master(C[0], 1000)

    cube = ws.add("box", C[0], 40, controls={"width": 0.5, "height": 0.5, "depth": 0.5, "rounding": 0.06, "color": "#5a8fe8"})
    ring = ws.mod("repeataround", C[1], 40, cube, controls={"copies": 12.0, "radius": 1.5, "tint": 0.7, "seed": 3.0}, options={"axis": "z"})
    left = ws.place(C[2], 40, ring, x=-5.0, y=1.0)

    pill = ws.add("sphere", C[0], 340, controls={"radius": 0.32, "color": "#e8d35a"})
    arch = ws.mod("repeataround", C[1], 340, pill, controls={"copies": 7.0, "radius": 1.7, "spread": 0.5}, options={"axis": "z"})
    # Rolled half a turn, so the half the arc keeps is the upper one.
    middle = ws.place(C[2], 340, arch, y=-0.75, roll=0.5)

    slab = ws.add("box", C[0], 640, controls={"width": 1.3, "height": 0.14, "depth": 0.5, "rounding": 0.03, "color": "#e8925a"})
    stairs = ws.mod(
        "repeataround",
        C[1],
        640,
        slab,
        controls={"copies": 18.0, "radius": 1.0, "rise": 0.17, "turn": 0.01, "grow": 0.015, "tint": 0.3, "seed": 5.0},
        options={"axis": "y"},
    )
    tower = ws.shape("cylinder", C[2], 640, onto=stairs, radius=0.14, height=3.6, y=1.4, color="#c9c4d6")
    right = ws.place(C[3], 640, tower, x=5.0, y=-0.9)

    scene = ws.union(C[4], 40, [left, middle, right])
    ws.show(C[5], 40, scene, master, (0.06, 0.0), controls={"distance": 9.6, "tilt": 0.06, "targetY": 0.4, "fog": 0.03})


def scatter_sphere(p):
    ws = p.workspace("Scatter on Sphere", "Cones standing all over a ball, each turned, sized and tinted")
    ws.note(
        40,
        40,
        "A Cone, scattered by Scatter on Sphere: 140 copies spread evenly over a sphere, each "
        "standing on it with its top facing out, turned by Spin, shrunk by Size and tinted by "
        "Tint. The ball they stand on is a Sphere chained on after the scatter, so it is not "
        "scattered itself.\n\n"
        "Try: Copies from 20 to 400, Seed, and Size; a cone wider than the gap between copies "
        "is sliced, as a Repeat's copy is outside its cell.",
    )
    master = ws.master(C[0], 700)
    cone = ws.add("cone", C[0], 40, controls={"bottom": 0.2, "top": 0.0, "height": 0.9, "color": "#f2b632"})
    spread = ws.mod("scattersphere", C[1], 40, cone, controls={"copies": 140.0, "radius": 1.6, "spin": 1.0, "size": 0.45, "tint": 0.5, "seed": 3.0})
    ball = ws.shape("sphere", C[2], 40, onto=spread, radius=1.5, color="#3a5ae8", blend=0.08)
    ws.show(C[3], 40, ball, master, "turn", controls={"distance": 6.5, "tilt": 0.06, "fog": 0.0, "targetY": 0.6, "floorHeight": -1.6})


def fold(p):
    ws = p.workspace("Fold", "A box folded like a kaleidoscope into a fractal, in each Symmetry")
    ws.note(
        40,
        40,
        "Fold is the demoscene's kaleidoscopic fold: it mirrors space onto one wedge of a "
        "symmetry, turns it and scales it about Offset, Iterations times, then puts the box in "
        "the folded space — a fractal of its copies, for the price of one box. The three are "
        "the same box and the same Fold in each Symmetry: Cube, Tetrahedron, Octahedron. One "
        "Oscillator on the gear swings all three Turn Xs either side of 0, so the fractals "
        "twist and fold back each loop. Each Fold works about the center, so a Place moves each "
        "into the row, and a Union meets the three.\n\n"
        "Try: Scale between 1.5 and 3, Offset, Iterations, and Tint, which colors each part by "
        "the path it took through the folds.",
    )
    master = ws.master(C[0], 900)
    swing = ws.sway(C[1], 900, master, amplitude=0.03, offset=0.0)
    parts = []
    # Each Symmetry with the box, Scale and Iterations that show it: Cube at Scale 3 is a
    # sponge of clusters, Tetrahedron at 2 a Sierpinski pyramid, Octahedron a lattice.
    for n, (sym, size, scale, iterations, x) in enumerate(
        (("cube", 2.0, 3.0, "3", -3.4), ("tetrahedron", 2.0, 2.0, "4", 0.0), ("octahedron", 1.6, 2.0, "3", 3.4))
    ):
        y = 40 + 280 * n
        box = ws.add("box", C[0], y, controls={"width": size, "height": size, "depth": size, "rounding": 0.03, "color": "#e8e2d6"})
        f = ws.mod("fold", C[1], y, box, controls={"scale": scale, "tint": 0.35, "turnZ": 0.0}, options={"symmetry": sym, "iterations": iterations})
        ws.cable(swing, "output", f, "turnX")
        parts.append(ws.place(C[2], y, f, x=x, y=0.2, scale=0.8))
    scene = ws.union(C[3], 40, parts)
    ws.show(C[4], 40, scene, master, (0.04, 0.0), controls={"distance": 8.0, "tilt": 0.06, "fog": 0.02, "targetY": 0.2})


def fold_wrap(p):
    ws = p.workspace("Fold plane and wrap", "A mirror plane turning through two shapes, and a row of boxes bent into a seamless ring")
    ws.note(
        40,
        40,
        "Left, Fold Plane mirrors space across a plane, so the side its normal points to is "
        "drawn on both: a ball and a capsule beside it are mirrored, and the gear turns the "
        "plane's Yaw a whole turn a loop, so the twin swings round, and where the normal "
        "turns away from the shapes there is nothing on its side to mirror and they vanish. Right, Wrap bends the X "
        "axis into a ring of Radius 1.5, its circumference 2π × 1.5 = 9.42; the Repeat lays "
        "boxes along X every 0.785, a twelfth of that, so the ring closes without a seam.\n\n"
        "Try: the Repeat's Spacing X at 0.8 — no longer a whole number of copies round the "
        "ring, so where the ends meet one box is cut. Or Fold Plane's Pitch and Offset.",
        width=420,
    )
    master = ws.master(C[0], 900)
    twin = ws.chain(
        C[0],
        40,
        [
            ("sphere", dict(x=1.0, y=0.4, radius=0.45, color="#e8925a")),
            ("capsule", dict(x=0.7, y=-0.2, z=0.5, length=0.8, radius=0.22, color="#5ad1c4", turn={"roll": 0.15})),
        ],
    )
    mirror = ws.mod("foldplane", C[2], 40, twin, controls={"offset": 0.2})
    ws.cable(master, "wrapped", mirror, "yaw")
    left = ws.place(C[3], 40, mirror, x=-2.8)

    brick = ws.add("box", C[0], 400, controls={"width": 0.45, "height": 0.9, "depth": 0.35, "rounding": 0.04, "color": "#b07ae8"})
    row = ws.mod("repeatgrid", C[1], 400, brick, controls={"spacingX": 2 * math.pi * 1.5 / 12, "spacingY": 0.0, "spacingZ": 0.0, "tint": 0.6, "seed": 2.0}, options={"extent": "forever"})
    ring = ws.mod("wrap", C[2], 400, row, controls={"radius": 1.5})
    right = ws.place(C[3], 400, ring, x=2.8, y=-0.5)

    scene = ws.union(C[4], 40, [left, right])
    ws.show(C[5], 40, scene, master, (0.04, 0.0), controls={"distance": 8.0, "tilt": 0.1, "fog": 0.02})


def noise_warp(p):
    ws = p.workspace("Noise and warp", "Noise Displace roughens a ball into a rock; Warp pushes a torus about")
    ws.note(
        40,
        40,
        "Left, Noise Displace roughens a ball into a rock: Amount is about the tallest bump, "
        "Frequency how many there are, Roughness how much each finer Octave adds. Right, Warp "
        "pushes the space round a torus by a smooth noise, and the gear's Cycles in its Phase "
        "set it flowing, coming back each loop. Each has its own modifier, so each is a branch "
        "and a Union meets the two.\n\n"
        "Try: Noise Displace's Octaves and Roughness, Warp's Amount and Frequency — and set "
        "Render 3D's Shading to Steps to see the march slow down on both.",
    )
    master = ws.master(C[0], 900)
    ball = ws.add("sphere", C[0], 40, controls={"x": -2.2, "radius": 1.1, "color": "#a89a8a"})
    rock = ws.mod("noisedisplace", C[1], 40, ball, controls={"amount": 0.28, "frequency": 1.6, "roughness": 0.55}, options={"octaves": "5"})
    torus = ws.shape("torus", C[0], 400, x=2.2, ring=0.95, tube=0.35, color="#5ad1c4", turn={"pitch": 0.2})
    flow = ws.mod("warp", C[1], 400, torus, controls={"amount": 0.35, "frequency": 0.9})
    ws.cable(master, "cycles", flow, "phase")
    scene = ws.union(C[2], 40, [rock, flow])
    ws.show(C[3], 40, scene, master, (0.04, 0.0), controls={"distance": 7.5, "tilt": 0.07, "fog": 0.02, "targetY": 0.1})


def seams(p):
    ws = p.workspace("Seams", "Stairs Union, Columns Union and Exclusive, each joining two solids")
    ws.note(
        40,
        40,
        "Three ways to meet two solids other than a plain union. Left, Stairs Union joins a "
        "floor and a wall with a flight of Steps stairs, Radius wide. Middle, Columns Union "
        "fills the seam of a floor and a post, fluted with a row of columns. Right, Exclusive "
        "keeps where exactly one of two balls is, hollowing their overlap; a Box chained on "
        "after it with Mode Subtract cuts the front open so the hollow shows.\n\n"
        "Try: Stairs Union's Steps and Radius, Columns Union's Columns, and the cutting Box's "
        "Mode set back to Union.",
    )
    master = ws.master(C[0], 1000)
    floor_a = ws.add("box", C[0], 40, controls={"x": -4.0, "y": -0.8, "width": 2.6, "height": 0.4, "depth": 1.6, "rounding": 0.02, "color": "#c9c4d6"})
    wall = ws.add("box", C[0], 240, controls={"x": -4.9, "y": 0.2, "width": 0.5, "height": 2.4, "depth": 1.6, "rounding": 0.02, "color": "#5a8fe8"})
    stairs = ws.pair("stairsunion", C[1], 40, floor_a, wall, controls={"radius": 0.8, "steps": 5.0})

    floor_b = ws.add("box", C[0], 440, controls={"y": -0.8, "width": 2.6, "height": 0.4, "depth": 1.6, "rounding": 0.02, "color": "#c9c4d6"})
    post = ws.add("cylinder", C[0], 640, controls={"y": 0.3, "radius": 0.35, "height": 2.4, "color": "#e8925a"})
    columns = ws.pair("columnsunion", C[1], 440, floor_b, post, controls={"radius": 0.5, "columns": 5.0})

    one = ws.add("sphere", C[0], 840, controls={"x": 3.6, "radius": 0.9, "color": "#d67a9e"})
    two = ws.add("sphere", C[0], 1040, controls={"x": 4.6, "radius": 0.9, "color": "#8fd17a"})
    xor = ws.pair("solidxor", C[1], 840, one, two)
    opened = ws.shape("box", C[2], 840, onto=xor, mode="subtract", x=4.1, z=1.0, width=3.0, height=3.0, depth=2.0, color="#f2b632")

    scene = ws.union(C[3], 40, [stairs, columns, opened])
    ws.show(C[4], 40, scene, master, (0.05, 0.03), controls={"distance": 9.5, "tilt": 0.08, "fog": 0.02})


def color_fields(p):
    ws = p.workspace("Color fields", "Six ways to paint a solid by where its surface is, the shape untouched")
    ws.note(
        40,
        40,
        "Color fields paint a solid and leave its shape alone. Back row, left to right: "
        "Checker 3D, cubes of two colors through a ball; Facing, a dodecahedron painted by "
        "which way each face looks — up, side, down; Height Gradient, a helix from blue at the "
        "bottom to gold at the top. Front row: Proximity glows a floor gold under a bobbing "
        "ball (its B, which it reads but does not draw — the ball you see is a second Sphere "
        "chained on after, on the same Oscillator); "
        "Contours rings a box by its distance from a point at its corner; Stripes 3D bands a "
        "torus, the gear sliding the bands up through it.\n\n"
        "Try: Checker 3D's Size, Proximity's Reach and Falloff, Contours' Width, Stripes 3D's "
        "Axis.",
        width=420,
    )
    master = ws.master(C[0], 1500)
    y = 40

    ball = ws.add("sphere", C[0], y, controls={"x": -3.0, "y": 0.2, "z": -1.6, "radius": 0.9})
    checks = ws.mod("checker3d", C[1], y, ball, controls={"size": 0.4, "colorA": "#f2efe6", "colorB": "#3a5ae8"})
    y += 240

    gem = ws.shape("dodecahedron", C[0], y, y=0.15, z=-1.6, size=1.05, turn={"yaw": 0.07, "pitch": 0.04})
    faces = ws.mod("facing", C[1], y, gem, controls={"colorUp": "#f2efe6", "colorSide": "#e8925a", "colorDown": "#3a2a5a"})
    y += 240

    spring = ws.add("helix", C[0], y, controls={"x": 3.0, "y": 0.2, "z": -1.6, "radius": 0.6, "pitch": 0.4, "tube": 0.13, "height": 2.2})
    heights = ws.mod("heightgradient", C[1], y, spring, controls={"bottom": -0.9, "top": 1.3, "colorBottom": "#3a5ae8", "colorTop": "#f2b632"})
    y += 240

    pad = ws.add("box", C[0], y, controls={"x": -3.0, "y": -0.85, "z": 1.4, "width": 2.2, "height": 0.3, "depth": 1.8, "rounding": 0.04, "color": "#4a4652"})
    bob = ws.add("sphere", C[0], y + 200, controls={"x": -3.0, "z": 1.4, "radius": 0.35, "color": "#ffb84a"})
    bob_y = ws.sway(C[0], y + 420, master, amplitude=0.45, offset=0.15)
    ws.cable(bob_y, "output", bob, "y")
    glow = ws.pair("proximity", C[1], y, pad, bob, controls={"color": "#ffb84a", "reach": 0.25, "falloff": 0.6})
    # The ball B stands for, drawn: a second Sphere chained on, its Y on the same Oscillator.
    shown = ws.shape("sphere", C[2], y, onto=glow, x=-3.0, z=1.4, radius=0.35, color="#ffd27a")
    ws.cable(bob_y, "output", shown, "y")
    y += 640

    block = ws.add("box", C[0], y, controls={"y": -0.2, "z": 1.4, "width": 1.5, "height": 1.3, "depth": 1.5, "rounding": 0.05})
    corner = ws.add("sphere", C[0], y + 200, controls={"x": 0.75, "y": 0.45, "z": 2.15, "radius": 0.05})
    bands = ws.pair("distancebands", C[1], y, block, corner, controls={"width": 0.14, "colorA": "#e8e2d6", "colorB": "#3a5ae8"})
    y += 440

    ring = ws.shape("torus", C[0], y, x=3.0, y=-0.1, z=1.4, ring=0.8, tube=0.35, turn={"pitch": 0.15})
    # Two bands, Width 0.2 each, a loop: the offset climbs 0.4 a loop and comes back the same.
    climb = ws.add("multiply", C[1], y + 220, controls={"b": 0.4})
    ws.cable(master, "wrapped", climb, "a")
    stripes = ws.mod("stripes3d", C[1], y, ring, controls={"width": 0.2, "colorA": "#e8e2d6", "colorB": "#d67a9e"}, options={"axis": "y"})
    ws.cable(climb, "output", stripes, "offset")

    scene = ws.union(C[3], 40, [checks, faces, heights, shown, bands, stripes])
    ws.show(C[4], 40, scene, master, (0.04, 0.0), controls={"distance": 8.0, "tilt": 0.09, "fog": 0.02, "targetY": -0.1}, options={"floor": "none"})


def video_source(p):
    """A video workspace of generators only, its Output's Frame for the solids to wear."""
    ws = p.workspace("Video for solids", "A kaleidoscope of checkers whose Output's Frame the solids wear")
    ws.note(
        40,
        40,
        "An ordinary video patch: a colored checkerboard, turned and folded by a Kaleidoscope. "
        "Its Output's Frame Out is cabled into the Texture nodes on the Video onto solids and "
        "Far textures sparkle tabs, so whatever this shows, they wear.\n\n"
        "Try: change the checkerboard's colors or Frequency and look at the other tabs. Keep "
        "this tab open: an Output on a closed tab is suspended, so its Frame stands still and "
        "the solids wearing it stop moving.",
    )
    master = ws.master(C[0], 500)
    picture = video_chain(ws, 40, master)
    out = ws.output(C[4], 40, resolution="640x360")
    ws.cable(picture, "output", out, "input")
    return out, master


def video_chain(ws, y, master):
    """The video patch the solids wear: checkers turned once a loop of `master`, folded and
    tinted. Its last node is returned."""
    checks = ws.add("checkerboard", C[0], y, controls={"frequency": 5.0, "color1": "#fff1b8", "color2": "#4b2aa8"})
    turn = ws.add("rotate", C[1], y)
    ws.cable(master, "wrapped", turn, "angle")
    kal = ws.add("kaleidoscope", C[2], y, controls={"segments": 6.0}, width=240)
    rings = ws.add(
        "radialgradient",
        C[2],
        y + 360,
        controls={"centerColor": "#ff5e7e", "edgeColor": "#2ec4b6", "radius": 0.6},
        options={"loop_mode": "mirror"},
    )
    tint = ws.add("layerblend", C[3], y, options={"blend_mode": "multiply"})
    ws.cable(checks, "output", turn, "input")
    ws.cable(turn, "output", kal, "input")
    ws.cable(kal, "output", tint, "background")
    ws.cable(rings, "output", tint, "foreground")
    return tint


def video_onto_solids(p, frame):
    ws = p.workspace("Video onto solids", "Another workspace's Output on a box, a cylinder and a ball")
    ws.note(
        40,
        40,
        "The Video for solids tab's Frame, on three solids through three Textures, each with "
        "its own Mapping: Box wraps it on from all six sides, Around rolls it round the "
        "cylinder, Sphere maps it onto the ball, the ball's Scale 0.5 tiling it twice. A "
        "Texture paints everything cabled into it, so each shape is its own branch, textured "
        "at the center and moved after by a Place, so the picture moves with it; a Union "
        "meets the three.\n\n"
        "Try: each Texture's Mapping and Scale, or move a Place in front of a Texture to make "
        "it a projector the shape slides through. Keep the Video for solids tab open: an "
        "Output on a closed tab is suspended, and its Frame stands still.",
    )
    master = ws.master(C[0], 1000)
    shapes = [
        ("box", {"width": 1.8, "height": 1.8, "depth": 1.8, "rounding": 0.06}, "box", 0.9, -3.6, -0.1),
        ("cylinder", {"radius": 0.9, "height": 2.0}, "around", 1.0, 0.0, 0.0),
        ("sphere", {"radius": 1.0}, "sphere", 0.5, 3.6, 0.0),
    ]
    parts = []
    y = 40
    for slug, controls, projection, scale, x, lift in shapes:
        s = ws.add(slug, C[0], y, controls=controls)
        tex = ws.mod("surfacetexture", C[1], y, s, controls={"scale": scale}, options={"projection": projection})
        ws.cable(frame, "frame", tex, "picture")
        parts.append(ws.place(C[2], y, tex, x=x, y=lift))
        y += 300
    scene = ws.union(C[3], 40, parts)
    ws.show(C[4], 40, scene, master, (0.07, 0.0), controls={"distance": 8.0, "tilt": 0.07, "fog": 0.03, "light": 0.08})


def solids_into_video(p):
    ws = p.workspace("Solids into video", "A Render 3D picture folded by a Kaleidoscope and shifted in hue")
    ws.note(
        40,
        40,
        "The other way round: Render 3D's picture is an ordinary picture, so any video node can "
        "take it. Here a ring of balls with a torus chained on, melted in by its Blend, turned "
        "by a Place — the whole group, so a Place — is folded by a Kaleidoscope and tinted by "
        "a Color Shift.\n\n"
        "Try: the Kaleidoscope's Segments and Zoom, or cable the render straight into the Output "
        "to see what is being folded.",
    )
    master = ws.master(C[0], 800)
    ball = ws.add("sphere", C[0], 40, controls={"radius": 0.3, "color": "#5ae8c8"})
    ring = ws.mod("repeataround", C[1], 40, ball, controls={"copies": 10.0, "radius": 2.1, "tint": 0.8, "seed": 2.0})
    both = ws.shape("torus", C[2], 40, onto=ring, ring=1.2, tube=0.32, blend=0.2, color="#e8925a")
    spin = ws.place(C[3], 40, both, pitch=0.1)
    ws.cable(master, "wrapped", spin, "yaw")
    r = ws.render(C[4], 40, spin, controls={"distance": 5.0, "tilt": 0.1, "fog": 0.0, "background": "#0d0a14"}, options={"floor": "none"})
    kal = ws.add("kaleidoscope", C[5], 40, controls={"segments": 8.0, "zoom": 1.2}, width=240)
    shift = ws.add("colorshift", C[6], 40)
    ws.cable(master, "wrapped", shift, "hue")
    ws.cable(r, "output", kal, "input")
    ws.cable(kal, "output", shift, "input")
    ws.cable(shift, "output", ws.output(C[7], 40), "input")


def extrude_revolve(p):
    ws = p.workspace("Extrude and revolve", "A word pushed out into letters and a pentagon turned on a lathe")
    ws.note(
        40,
        40,
        "Pictures into solids. A Text, blurred, into Extrude: solid where the picture's "
        "Brightness is over Threshold, pushed Depth deep, colored by the picture. A Polygon "
        "into Revolve: its right half is the outline, spun round the upright axis like a "
        "lathe — a spinning top. The Revolve goes first in the chain and a Place moves it "
        "aside, since a Place moves everything before it; the letters come after it, turned "
        "a little by their own Yaw.\n\n"
        "Two traps. Threshold: with Read on Brightness a mid-bright color such as this orange "
        "only just clears 0.5, so the letters would come out thin and torn — this one is at "
        "0.25. Steepness: too low and the edges tear, too high and the march crawls. The Blur "
        "before the Text softens its edge, which is what makes a low Steepness true; try "
        "unplugging it. A drawn shape such as the Polygon softens by its own Softness "
        "instead: a Blur of a drawn shape is a staircase of its taps, which speckles at any "
        "Steepness.",
        width=440,
    )
    master = ws.master(C[0], 900)
    outline = ws.add("polygon", C[0], 40, controls={"sides": 5.0, "radius": 0.7, "rotation": 0.1, "softness": 0.06})
    lathe = ws.shape("revolve", C[1], 40, scale=1.2, steepness=10.0, color="#d6a35a")
    ws.cable(outline, "color", lathe, "profile")
    aside = ws.place(C[2], 40, lathe, x=3.3, y=-0.1)

    words = ws.add(
        "text",
        C[0],
        400,
        controls={"textColor": "#e8925a", "backgroundColor": "#000000"},
        options={"size": "420", "font": "Impact", "weight": "bold"},
        values={"words": {"Text": "SOLID"}},
    )
    soft = ws.add("blur", C[1], 400, controls={"blurX": 5.0, "blurY": 5.0}, options={"size": "7x7"})
    ws.cable(words, "output", soft, "input")
    letters = ws.shape(
        "extrude",
        C[3],
        40,
        onto=aside,
        scale=1.2,
        depth=0.5,
        threshold=0.25,
        steepness=8.0,
        bevel=0.03,
        turn={"yaw": 0.04},
        options={"paint": "picture"},
    )
    ws.cable(soft, "output", letters, "profile")
    ws.show(C[4], 40, letters, master, (0.05, 0.02), controls={"distance": 7.0, "tilt": 0.06, "targetX": 0.9, "targetY": 0.0, "fog": 0.02})


def heightfield(p):
    ws = p.workspace("Heightfield", "Perlin noise raised into a landscape colored by itself, with a lake")
    ws.note(
        40,
        40,
        "Heightfield raises a picture into terrain: the ground stands at Base plus Height times "
        "the picture's Brightness, colored by the picture itself. Here a Perlin Noise, light "
        "sand over dark green, walking round its loop as the gear turns, so hills rise and "
        "sink. A Ground chained on after it at a little above the lowest is a lake.\n\n"
        "Try: any video chain in place of the noise — a camera, an Output's Frame. Height and "
        "Steepness: too low a Steepness and the slopes tear, too high and the march crawls; "
        "set Render 3D's Shading to Steps to see it.",
    )
    master = ws.master(C[0], 700)
    noise = ws.add("perlin", C[0], 40, controls={"scale": 3.0, "foreground": "#fff4d6", "background": "#123a1a", "contrast": 2.0}, options={"repeat": "1"})
    ws.cable(master, "cycles", noise, "clock")
    land = ws.add("heightfield", C[1], 40, controls={"scale": 4.0, "base": -1.0, "height": 1.3, "steepness": 4.0}, options={"paint": "picture"})
    ws.cable(noise, "color", land, "picture")
    lake = ws.shape("ground", C[2], 40, onto=land, height=-0.6, color="#2e5a9a", blend=0.05)
    ws.show(C[3], 40, lake, master, (0.04, 0.08), controls={"distance": 10.0, "tilt": 0.16, "fog": 0.02, "background": "#9fb4d8"}, options={"floor": "none"})


def animated(p):
    ws = p.workspace("Animated", "A gear ripples a ball, swings a twist, turns a shape and a ring")
    ws.note(
        40,
        40,
        "Every number on a solid takes a cable from a gear, an oscillator or a MIDI knob. Here "
        "one Master Gear ripples the ball through Displace's Phase, which loops in one turn, "
        "swings the bar's Twist through an Oscillator, turns the octahedron through its own "
        "Yaw, and turns the ring of capsules through a Place's Yaw — the ring is a group, so "
        "it is a Place. All of it comes back each loop.\n\n"
        "Try: Displace's Amount and Frequency, or the Oscillator's Amplitude.",
    )
    master = ws.master(C[0], 1000)
    ball = ws.add("sphere", C[0], 40, controls={"x": -3.6, "y": 0.1, "radius": 0.95, "color": "#e8925a"})
    ripple = ws.mod("displace", C[1], 40, ball, controls={"amount": 0.12, "frequency": 3.0})
    ws.cable(master, "cycles", ripple, "phase")

    bar = ws.add("box", C[0], 340, controls={"width": 0.8, "height": 2.2, "depth": 0.8, "rounding": 0.05, "color": "#5a8fe8"})
    twist = ws.mod("twist", C[1], 340, bar)
    swing = ws.sway(C[1], 600, master, amplitude=0.22, offset=0.0)
    ws.cable(swing, "output", twist, "rate")
    aside = ws.place(C[2], 340, twist, x=-1.2, y=0.1)
    spun = ws.shape("octahedron", C[3], 340, onto=aside, x=1.2, y=0.15, size=1.0, color="#b07ae8", turn={"pitch": 0.1})
    ws.cable(master, "wrapped", spun, "tYaw")

    pill = ws.add("capsule", C[0], 760, controls={"length": 0.5, "radius": 0.22, "color": "#e8d35a"})
    ring = ws.mod("repeataround", C[1], 760, pill, controls={"copies": 7.0, "radius": 0.9, "tint": 0.6, "seed": 9.0})
    turn = ws.place(C[2], 760, ring, x=3.7, y=-0.35)
    ws.cable(master, "wrapped", turn, "yaw")

    scene = ws.union(C[5], 40, [ripple, spun, turn])
    ws.show(C[6], 40, scene, controls={"distance": 8.0, "tilt": 0.07, "orbit": 0.0, "fog": 0.03})


def composite(p):
    ws = p.workspace("Composite", "A solid on a transparent background, laid over a video pattern")
    ws.note(
        40,
        40,
        "Render 3D's Background is a color with alpha: at alpha 0, with Floor None and no fog, "
        "every pixel the rays miss is transparent, so a Layer Blend lays the solid over any "
        "picture — here turning stripes. The torus turns by its own Yaw from the gear and "
        "ripples by a Displace after it.\n\n"
        "Try: set the Background's alpha back up, or the Floor to Checker, and the stripes "
        "vanish behind it; or set the Layer Blend's mode to Difference.",
    )
    master = ws.master(C[0], 800)
    stripes = ws.add("stripes", C[0], 40, controls={"frequency": 2.5, "color1": "#1b998b", "color2": "#2d3047", "smoothing": 0.3})
    turn = ws.add("rotate", C[1], 40)
    ws.cable(stripes, "output", turn, "input")
    ws.cable(master, "wrapped", turn, "angle")
    knot = ws.shape("torus", C[0], 360, ring=1.1, tube=0.38, color="#f2b632", turn={"pitch": 0.15})
    ws.cable(master, "wrapped", knot, "tYaw")
    wobble = ws.mod("displace", C[1], 360, knot, controls={"amount": 0.08, "frequency": 4.0})
    ws.cable(master, "cycles", wobble, "phase")
    r = ws.render(
        C[2],
        360,
        wobble,
        controls={"distance": 5.5, "tilt": 0.05, "fog": 0.0, "background": "#00000000"},
        options={"floor": "none"},
    )
    over = ws.add("layerblend", C[3], 40)
    ws.cable(turn, "output", over, "background")
    ws.cable(r, "output", over, "foreground")
    ws.cable(over, "output", ws.output(C[4], 40), "input")


def still_life(ws, x, y):
    """A small scene for the render options: a ball, a box, a torus and a capsule in a chain,
    on Render 3D's own floor."""
    steps = [
        ("sphere", dict(x=-1.6, y=-0.15, radius=0.85, color="#e8925a")),
        ("box", dict(x=0.4, y=-0.35, z=-0.6, width=1.3, height=1.3, depth=1.3, rounding=0.08, color="#5a8fe8", turn={"yaw": 0.08})),
        ("torus", dict(x=1.9, y=-0.2, z=0.7, ring=0.6, tube=0.2, color="#d67a9e", turn={"pitch": 0.2})),
        ("capsule", dict(x=0.0, y=-0.3, z=1.3, length=0.5, radius=0.35, color="#8fd17a", turn={"roll": 0.25})),
    ]
    return ws.chain(x, y, steps)


def render_glow(p):
    ws = p.workspace("Render glow", "Render 3D's Glow: a halo where rays pass near a surface")
    ws.note(
        40,
        40,
        "Render 3D's Glow, On: every ray that passes close to a surface without hitting it "
        "picks up Glow Color, more the closer and the longer it grazes, Glow Reach the width "
        "of the halo. Thin shapes on a dark ground show it best — a Box Frame round a Helix, "
        "turning by their own Yaw.\n\n"
        "Try: Glow Amount, Glow Reach, Glow Color, and Glow Off to compare.",
    )
    master = ws.master(C[0], 700)
    frame = ws.shape("boxframe", C[0], 40, width=2.0, height=2.0, depth=2.0, edge=0.05, color="#2a2a3a", turn={"pitch": 0.1})
    spring = ws.shape("helix", C[1], 40, onto=frame, radius=0.45, pitch=0.35, tube=0.06, height=1.6, color="#f2efe6")
    ws.cable(master, "wrapped", frame, "tYaw")
    ws.show(
        C[2],
        40,
        spring,
        controls={"distance": 5.0, "tilt": 0.05, "orbit": 0.08, "fog": 0.0, "background": "#07060c", "glowAmount": 0.5, "glowReach": 0.12, "glowColor": "#59d8ff"},
        options={"floor": "none", "glow": "on"},
    )


def render_toon(p):
    ws = p.workspace("Render toon", "Render 3D's Toon shading in flat bands with dark outlines")
    ws.note(
        40,
        40,
        "Render 3D's Shading on Toon: the light comes in Bands flat steps instead of a smooth "
        "fall, and an Outline Outline wide is drawn round every silhouette and crease.\n\n"
        "Try: Bands from 2 to 8, Outline and Outline Color, and Shading on Flat for no light at "
        "all.",
    )
    master = ws.master(C[0], 700)
    scene = still_life(ws, C[0], 40)
    ws.show(C[5], 40, scene, master, (0.05, 0.05), controls={"distance": 6.5, "tilt": 0.08, "fog": 0.0, "bands": 3.0, "outline": 0.02, "background": "#f2e8c8"}, options={"shading": "toon"})


def render_reflections(p):
    ws = p.workspace("Render reflections", "Render 3D's Reflections: one bounce off every surface")
    ws.note(
        40,
        40,
        "Render 3D's Reflections on One: where a ray lands, a second ray goes on the mirror way "
        "and its color is mixed in by Reflectivity, so the balls show each other and the "
        "checker floor. One bounce only: a reflection in a reflection is just the sky.\n\n"
        "Try: Reflectivity from 0 to 1, and Reflections None to compare; a bounce is a second "
        "march, so it costs about twice as much.",
    )
    master = ws.master(C[0], 700)
    scene = ws.chain(
        C[0],
        40,
        [
            ("sphere", dict(x=-1.3, y=-0.1, radius=0.9, color="#c9c4d6")),
            ("sphere", dict(x=1.0, y=-0.4, z=0.6, radius=0.6, color="#e8925a")),
            ("sphere", dict(x=0.6, y=-0.6, z=-1.2, radius=0.4, color="#5a8fe8")),
            ("torus", dict(x=-0.2, y=-0.82, z=1.2, ring=0.5, tube=0.18, color="#8fd17a")),
        ],
    )
    ws.show(C[4], 40, scene, master, "turn", controls={"distance": 5.5, "tilt": 0.07, "fog": 0.02, "reflectivity": 0.55}, options={"reflections": "one", "floor": "checker"})


def render_lenses(p):
    ws = p.workspace("Render lenses", "Render 3D's Lens Type: orthographic on the left, fisheye on the right")
    ws.note(
        40,
        40,
        "The same grid of boxes through two Render 3Ds. Left, Lens Type Orthographic: no "
        "perspective, so the far boxes are as big as the near ones and the rows run parallel; "
        "Lens is the width of the view. Right, Fisheye: a whole half sphere of view bent into "
        "the frame, straight rows curving round the edges.\n\n"
        "Try: Lens on each, and Perspective, the default, between the two.",
    )
    master = ws.master(C[0], 1000)
    box = ws.add("box", C[0], 40, controls={"width": 0.8, "height": 0.8, "depth": 0.8, "rounding": 0.08, "color": "#e8e2d6"})
    grid = ws.mod("repeatgrid", C[1], 40, box, controls={"spacingX": 2.0, "spacingZ": 2.0, "countX": 3.0, "countZ": 3.0, "tint": 0.8, "seed": 4.0}, options={"extent": "counted"})
    orbit = ws.sway(C[2], 1000, master, amplitude=0.05, offset=0.08)
    ortho = ws.render(C[2], 40, grid, controls={"distance": 12.0, "tilt": 0.09, "lens": 2.2, "fog": 0.0}, options={"projection": "orthographic"})
    fish = ws.render(C[2], 400, grid, controls={"distance": 4.0, "tilt": 0.05, "lens": 1.0, "fog": 0.0}, options={"projection": "fisheye"})
    ws.cable(orbit, "output", ortho, "orbit")
    ws.cable(orbit, "output", fish, "orbit")
    both = side_by_side(ws, C[3], 40, ortho, fish)
    ws.cable(both, "output", ws.output(C[5], 40), "input")


def render_look_at(p):
    ws = p.workspace("Render look at", "Render 3D's Camera on Look at: a worm's-eye view from a point near the floor")
    ws.note(
        40,
        40,
        "Render 3D's Camera on Look at: instead of orbiting the target, the camera stands at "
        "From X, Y, Z and looks at Target X, Y, Z — here from low by the floor at the front "
        "right, a worm's-eye view of the still life, the torus turning by its own Yaw from the "
        "gear. From and Target are settings on the node rather than ports: to fly a camera, "
        "set Camera back to Orbit and cable Orbit, Tilt and Distance.\n\n"
        "Try: From Y up to 4 for a view from above, and Lens for a wider or narrower view.",
    )
    master = ws.master(C[0], 1000)
    scene = still_life(ws, C[0], 40)
    torus = next(n["id"] for n in ws.nodes if n["slug"] == "torus")
    ws.cable(master, "wrapped", torus, "tYaw")
    ws.show(
        C[5],
        40,
        scene,
        controls={"fromX": 2.8, "fromY": -0.5, "fromZ": 3.6, "targetX": -0.2, "targetY": -0.2, "targetZ": 0.0, "fog": 0.02, "lens": 1.3},
        options={"camera": "lookat"},
    )


# ------------------------------------------------------------------------------------------
# Limits


def side_by_side(ws, x, y, left, right, shift=0.89):
    """`left`'s picture on the left half and `right`'s on the right, each moved over so the
    middle of its picture is the middle of its half."""
    a = ws.add("translate", x, y, controls={"x": -shift})
    b = ws.add("translate", x, y + 220, controls={"x": shift})
    ws.cable(left, "output", a, "input")
    ws.cable(right, "output", b, "input")
    world = ws.add("worldcoordinates", x, y + 440)
    split = ws.add("threshold", x + 280, y + 440, controls={"threshold": 0.0, "smooth": 0.004})
    ws.cable(world, "x", split, "input")
    both = ws.add("mix", x + 280, y)
    ws.cable(a, "output", both, "a")
    ws.cable(b, "output", both, "b")
    ws.cable(split, "output", both, "amount")
    return both


def must_fit(p):
    ws = p.workspace("Copies must fit their cell", "Copies wider than their spacing, and copies scattered out of it, are sliced")
    ws.note(
        40,
        40,
        "A Repeat only looks at the nearest cell, so a copy that does not fit inside its cell is "
        "cut flat at the cell's edge. Left, boxes turned at random by Spin: a turned box's "
        "corners reach past its cell and are cut off. Right, balls that fit, but Scatter 1 "
        "pushes them over the edge, so some are sliced, and rays slip into the neighbors. Each "
        "repeat works about the center, so a Place moves each grid aside.\n\n"
        "Try: raise the left Repeat's Spacing X and Z to 2.2, or lower the right one's Scatter "
        "to 0.3 — the slicing is gone.",
    )
    master = ws.master(C[0], 800, length=8.0)
    box = ws.add("box", C[0], 40, controls={"width": 1.5, "height": 1.0, "depth": 1.5, "rounding": 0.06, "color": "#e8925a"})
    tight = ws.mod(
        "repeatgrid",
        C[1],
        40,
        box,
        controls={"spacingX": 1.7, "spacingZ": 1.7, "countX": 1.0, "countZ": 1.0, "spin": 1.0, "seed": 3.0},
        options={"extent": "counted"},
    )
    left = ws.place(C[2], 40, tight, x=-3.2, y=-0.5)
    ball = ws.add("sphere", C[0], 400, controls={"radius": 0.65, "color": "#5a8fe8"})
    loose = ws.mod(
        "repeatgrid",
        C[1],
        400,
        ball,
        controls={"spacingX": 1.7, "spacingZ": 1.7, "countX": 1.0, "countZ": 1.0, "scatter": 1.0, "seed": 4.0},
        options={"extent": "counted"},
    )
    right = ws.place(C[2], 400, loose, x=3.2, y=-0.35)
    scene = ws.union(C[3], 40, [left, right])
    ws.show(C[4], 40, scene, master, (0.06, 0.0), controls={"distance": 10.5, "tilt": 0.12, "fog": 0.01})


def stretch_costs(p):
    ws = p.workspace("Stretch costs steps", "A ball stretched 8:1 beside a capsule of its shape, Lit and in Steps")
    ws.note(
        40,
        40,
        "Top, a ball Stretched to 8 : 1; below it a capsule of nearly the same shape, which "
        "needs no stretch — it is chained on after the Stretch, so the Stretch does not touch "
        "it, and it lies down by its own Roll. Left half Lit, right half the same scene with "
        "Shading on Steps, painting how many steps each pixel took, blue few to red many. "
        "Stretch has to divide its distance by the squash to stay safe, so rays creep up on "
        "the spindle and it glows hot while the capsule stays cool.\n\n"
        "Try: bring the Stretch's X down from 4 toward 1 and watch the right half cool.",
    )
    master = ws.master(C[0], 1000, length=8.0)
    ball = ws.add("sphere", C[0], 40, controls={"y": 2.0, "radius": 1.0, "color": "#e8925a"})
    spindle = ws.mod("stretch", C[1], 40, ball, controls={"sx": 4.0, "sy": 0.5, "sz": 0.5})
    scene = ws.shape("capsule", C[2], 40, onto=spindle, y=-0.5, length=7.0, radius=0.5, color="#5a8fe8", turn={"roll": 0.25})
    orbit = ws.sway(C[3], 1000, master, amplitude=0.04, offset=0.1)
    camera = {"distance": 15.0, "tilt": 0.03, "targetY": 0.25, "fog": 0.0, "lens": 2.2}
    lit = ws.render(C[3], 40, scene, controls=camera, options={"floor": "none"})
    steps = ws.render(C[3], 400, scene, controls=camera, options={"floor": "none", "shading": "steps"})
    ws.cable(orbit, "output", lit, "orbit")
    ws.cable(orbit, "output", steps, "orbit")
    both = side_by_side(ws, C[4], 40, lit, steps)
    ws.cable(both, "output", ws.output(C[6], 40), "input")


def twist_displace(p):
    ws = p.workspace("Twist and displace slow the march", "A hard twist and a deep ripple, Lit and in Steps, running out of steps")
    ws.note(
        40,
        40,
        "A bar twisted hard and a ball rippled deep. Both keep their distance a safe bound by "
        "shrinking it, everywhere, so the march crawls: the Steps half on the right is red even "
        "in empty space, and where a ray runs out of its 64 steps the Lit half shows the pink "
        "background through holes in the surface. Each has its own modifier, so a Union meets "
        "the two branches.\n\n"
        "Try: set both Render 3Ds' Steps to 256 and the holes close; or lower the Twist's Rate "
        "and the Displace's Amount instead.",
    )
    master = ws.master(C[0], 1000)
    bar = ws.add("box", C[0], 40, controls={"width": 1.4, "height": 2.6, "depth": 0.5, "rounding": 0.03, "color": "#5a8fe8"})
    twist = ws.mod("twist", C[1], 40, bar, controls={"rate": 0.7})
    left = ws.place(C[2], 40, twist, x=-1.7, y=0.3)
    ball = ws.add("sphere", C[0], 300, controls={"x": 1.7, "y": 0.2, "radius": 1.1, "color": "#e8925a"})
    ripple = ws.mod("displace", C[1], 300, ball, controls={"amount": 0.3, "frequency": 5.0})
    ws.cable(master, "cycles", ripple, "phase")
    scene = ws.union(C[3], 40, [left, ripple])
    camera = {"distance": 7.5, "tilt": 0.04, "orbit": 0.0, "fog": 0.0, "lens": 2.0, "targetY": 0.2, "background": "#ff3d8b"}
    lit = ws.render(C[4], 40, scene, controls=camera, options={"floor": "none", "steps": "64"})
    steps = ws.render(C[4], 400, scene, controls=camera, options={"floor": "none", "steps": "64", "shading": "steps"})
    both = side_by_side(ws, C[5], 40, lit, steps)
    ws.cable(both, "output", ws.output(C[7], 40), "input")


def far_textures(p, frame):
    ws = p.workspace("Far textures sparkle", "The video tab's Frame tiled on an endless Ground, sparkling toward the horizon")
    ws.note(
        40,
        40,
        "The Video for solids Frame tiled across an endless Ground with Mapping Top, fog off. "
        "Near the camera it is sharp; toward the horizon one pixel covers many copies of the "
        "picture, and since a texture has no mipmaps each pixel takes one sample of them, so "
        "the distance shimmers and sparkles as the camera swings on its Orbit.\n\n"
        "Try: turn Render 3D's Fog up to 0.15 to hide the horizon, or the Texture's Scale up so "
        "fewer copies crowd into a pixel. Keep the Video for solids tab open: an Output on a "
        "closed tab is suspended.",
    )
    master = ws.master(C[0], 700)
    ground = ws.add("ground", C[0], 40, controls={"height": -1.0})
    tex = ws.mod("surfacetexture", C[1], 40, ground, controls={"scale": 0.6}, options={"projection": "top"})
    ws.cable(frame, "frame", tex, "picture")
    ws.show(
        C[2],
        40,
        tex,
        master,
        (0.04, 0.0),
        controls={"distance": 4.0, "tilt": 0.035, "fog": 0.0, "background": "#9fb4d8", "lens": 1.4, "targetY": -0.2},
        options={"floor": "none", "shadows": "none"},
    )


def blur_cost(p):
    ws = p.workspace("Cost of a blur after a render", "Render 3D into a 7×7 Blur: the scene is marched 49 times a pixel")
    ws.note(
        40,
        40,
        "A Blur asks its input at 49 points round each pixel, and its input here is Render 3D, "
        "which marches the whole scene each time it is asked: this picture costs 49 renders. "
        "Open View ▸ Costs, or read the GPU ms in the Status box, then set the Blur's Size to "
        "3×3, then cable the render straight into the Output.\n\n"
        "The cheap way: render into an Output in its own tab and blur that Output's Frame, which "
        "is a picture already drawn.",
    )
    master = ws.master(C[0], 700)
    scene = ws.chain(
        C[0],
        40,
        [
            ("sphere", dict(x=-2.4, radius=0.9, y=-0.1, color="#e8925a")),
            ("box", dict(width=1.5, height=1.5, depth=1.5, y=-0.25, color="#5a8fe8")),
            ("octahedron", dict(x=2.4, size=1.1, y=0.1, color="#b07ae8")),
        ],
    )
    orbit = ws.sway(C[3], 700, master, amplitude=0.05, offset=0.0)
    r = ws.render(C[3], 40, scene, controls={"distance": 8.5, "tilt": 0.07, "fog": 0.01})
    ws.cable(orbit, "output", r, "orbit")
    blur = ws.add("blur", C[4], 40, controls={"blurX": 3.0, "blurY": 3.0}, options={"size": "7x7"})
    ws.cable(r, "output", blur, "input")
    ws.cable(blur, "output", ws.output(C[5], 40, resolution=HEAVY), "input")


def infinite_detail(p):
    ws = p.workspace("Infinite detail is not free", "A Mandelbulb at 64 steps and at 512")
    master = ws.master(C[0], 900, length=8.0)
    ws.note(
        40,
        40,
        "A Mandelbulb: detail inside detail, so rays graze edge after edge on the way in. Left "
        "half at 64 Steps, right half at 512: at 64 the rays give up before they reach the "
        "finest surfaces and the pink background shows through.\n\n"
        "Try: both renders' Steps, and set the right one's Shading to Steps to see where the "
        "work goes.",
    )
    scene = ws.add("mandelbulb", C[0], 40, controls={"size": 1.5}, options={"iterations": "8"})
    camera = {"distance": 3.6, "tilt": 0.06, "fog": 0.0, "background": "#ff3d8b"}
    orbit = ws.sway(C[1], 900, master, amplitude=0.04, offset=0.0)
    few = ws.render(C[2], 40, scene, controls=camera, options={"floor": "none", "steps": "64"})
    many = ws.render(C[2], 400, scene, controls=camera, options={"floor": "none", "steps": "512"})
    ws.cable(orbit, "output", few, "orbit")
    ws.cable(orbit, "output", many, "orbit")
    both = side_by_side(ws, C[3], 40, few, many)
    ws.cable(both, "output", ws.output(C[5], 40, resolution=HEAVY), "input")


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
    root = Path(args[0]).expanduser() if args else documents() / "supersilvia" / "Solids"
    p = Project(load_registry(registry))
    for build in (
        chain_demo,
        primitives,
        more_shapes,
        booleans,
        transforms,
        repeat_forever,
        repeat_around,
        scatter_sphere,
        fold,
        fold_wrap,
        noise_warp,
        seams,
        color_fields,
    ):
        build(p)
    frame, _ = video_source(p)
    video_onto_solids(p, frame)
    solids_into_video(p)
    extrude_revolve(p)
    heightfield(p)
    animated(p)
    composite(p)
    for build in (render_glow, render_toon, render_reflections, render_lenses, render_look_at):
        build(p)
    must_fit(p)
    stretch_costs(p)
    twist_displace(p)
    far_textures(p, frame)
    blur_cost(p)
    infinite_detail(p)
    p.write(root)
    print(f"wrote {root}: {len(p.workspaces)} workspaces, {p._node} nodes")


if __name__ == "__main__":
    main()
