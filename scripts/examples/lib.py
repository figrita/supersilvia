# SPDX-License-Identifier: AGPL-3.0-or-later
"""The authoring library for the Example Library: one workspace per example, checked against
the node registry as it is built, and written as a strip the app bundles
(`assets/examples/<NN-Category>/<NN-Title>.ssw`).

Help ▸ Example Library arranges an example with the app's own auto-arrange as it adds it, so
the positions written here are not what anyone sees there. `Workspace.layout` lays the file out
by its dataflow anyway, for whatever opens the file as it is: an import, or the `--preview`
project.

An example is one small patch that does one thing, with a note saying what to look at and
which knob to turn. It names no file, opens no device — a camera node would open the camera
the moment the example is added, so an example that wants a picture from outside reads the
Main Input node — and every cable stays inside it. `make.py` runs every category module and
holds them to all of that; the app's own test holds the written files to loading cleanly.

The builder is `scripts/demo-solids.py`'s, which `Example` wraps: `add`, `note`, `cable`,
`shape`, `chain`, `mod`, `pair`, `render`, `show`, `master`, `sway`, `output`.
"""

import json
import shutil
import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
LOOP = 4.0
FPS = 25.0
RES = "1280x720"
HEAVY = "640x360"

# Nodes an example never holds: each opens a device or a file the moment it loads.
FORBIDDEN = {"camera", "screencapture", "audioin", "video", "syphon", "ndi", "imagegif"}

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

    def __init__(self, project, name, blurb, kind):
        self.project = project
        self.reg = project.reg
        self.name = name
        self.blurb = blurb
        # "video" or "solids": which library the workspace's menu leads with.
        self.kind = kind
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

    def loop_free(self):
        """The cables the layout reads: every one but those closing a loop through a delayed
        port — an Output's Frame read back upstream of it, a tap's reading — which are last
        frame's, so the loop reads left to right into its Output and the cable runs back.
        Counted, a loop has no first node and its columns land wherever the passes stop."""
        slug = {n["id"]: n["slug"] for n in self.nodes}

        def delayed(c):
            out = next(o for o in self.reg[slug[c["from"]["node"]]]["outputs"] if o["key"] == c["from"]["key"])
            return out["delayed"] or out["kind"] == "texture"

        def reaches(start, goal, cables):
            seen, stack = set(), [start]
            while stack:
                n = stack.pop()
                if n == goal:
                    return True
                if n not in seen:
                    seen.add(n)
                    stack.extend(c["to"]["node"] for c in cables if c["from"]["node"] == n)
            return False

        kept = list(self.connections)
        for c in self.connections:
            if delayed(c) and reaches(c["to"]["node"], c["from"]["node"], kept):
                kept.remove(c)
        return kept

    def layout(self):
        """The file's own positions, which Help ▸ Example Library replaces with the app's
        arrange: every node but the note in columns left to right by how far down the dataflow it
        is, so a chain reads as a row; a node that only drives numbers — a gear, an
        oscillator — goes in the column before the first node it drives, under the rest.
        Each column is stacked by the real height of its nodes."""
        nodes = [n for n in self.nodes if n["slug"] != "note"]
        ids = {n["id"] for n in nodes}
        preds = {n["id"]: [] for n in nodes}
        succs = {n["id"]: [] for n in nodes}
        for c in self.loop_free():
            a, b = c["from"]["node"], c["to"]["node"]
            if a in ids and b in ids:
                preds[b].append(a)
                succs[a].append(b)
        by_id = {n["id"]: n for n in nodes}
        # A node that only drives numbers — a gear, an oscillator, a Multiply of a gear — goes
        # under the rest: every output a uniform once its cables are known. A chain of varying
        # numbers is dataflow like any picture and reads left to right.
        uniform = ("uniform number", "uniform color", "action")
        control = {
            n["id"]
            for n in nodes
            if succs[n["id"]]
            and all(
                self.project.output_type(n["id"], o["key"]) in uniform
                for o in self.reg[n["slug"]]["outputs"]
            )
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
            "kind": self.kind,
            "blurb": self.blurb,
            "layout": "linear",
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

    def workspace(self, name, blurb, kind="video"):
        ws = Workspace(self, name, blurb, kind)
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
        assert ws_a is ws_b, "an example is one workspace: a cable cannot leave it"
        ws_a.connections.append(c)

    def adopt(self, example):
        """Take `example`'s workspace in as a tab of this project, renumbered past this one's
        ids, for a preview holding many examples."""
        ws = example.ws
        shift = self._node
        ws.id = self.next_workspace()
        for n in ws.nodes:
            n["id"] += shift
        for c in ws.connections:
            c["from"]["node"] += shift
            c["to"]["node"] += shift
        self._node += example.project._node
        ws.project = self
        for n in ws.nodes:
            self.owner[n["id"]] = (ws, n)
        self.workspaces.append(ws)

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
                {"id": ws.id, "name": ws.name, "kind": ws.kind, "file": name, "view": {"pan": [0.0, 0.0], "zoom": 0.8}}
            )
        (root / "project.ssp").write_text(json.dumps(manifest, indent=2) + "\n")



class Example:
    """One example: a workspace of its own, in a project of one, so every id is its own."""

    def __init__(self, reg, title, blurb, kind="video"):
        self.title = title
        self.project = Project(reg)
        self.ws = self.project.workspace(title, blurb, kind)

    def __getattr__(self, name):
        return getattr(self.ws, name)

    def check(self):
        slugs = {n["slug"] for n in self.ws.nodes}
        bad = slugs & FORBIDDEN
        assert not bad, f"{self.title}: holds {sorted(bad)}, which open a device or a file"
        assert "note" in slugs, f"{self.title}: no note saying what to look at"
        assert "output" in slugs, f"{self.title}: no Output"
        for n in self.ws.nodes:
            assert "file" not in n["options"], f"{self.title}: {n['slug']} names a file"

    def file(self):
        self.check()
        return self.ws.file()


def file_name(index, title):
    """`NN-Title.ssw`, the title kept as written but for what a file name cannot hold."""
    safe = re.sub(r'[\\/:*?"<>|]', "", title).strip()
    return f"{index:02d}-{safe}.ssw"
