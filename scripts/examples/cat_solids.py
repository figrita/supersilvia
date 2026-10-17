# SPDX-License-Identifier: AGPL-3.0-or-later
"""Solids: shapes in three dimensions, built as a chain and drawn by Render 3D. Each example
is one of `scripts/demo-solids.py`'s workspaces brought down to one idea and to everything it
needs inside itself — the picture a solid wears is made beside it, not on another tab."""

from lib import HEAVY

ORDER = 13
CATEGORY = "Solids"
KIND = "solids"


def first_solid(new):
    e = new("A first solid", "A sphere into Render 3D into an Output")
    e.note(
        40,
        40,
        "A solid is a shape in three dimensions, and Render 3D is what makes it a picture: it "
        "shoots a ray from a camera through every pixel and lights what the ray lands on. The "
        "Master Gear turns the camera's Orbit once a loop, so the checker floor and the shadow "
        "swing round the ball. Drag the Sphere's Radius and Color, or unplug the Orbit and drag "
        "Orbit, Tilt and Distance on Render 3D yourself.",
    )
    master = e.master(0, 0)
    ball = e.add("sphere", 0, 0, controls={"y": -0.1, "radius": 0.9, "color": "#e8925a"})
    e.show(0, 0, ball, master, "turn", controls={"distance": 4.2, "tilt": 0.07, "fog": 0.02})
    return e


def chain(new):
    e = new("Building with a chain", "Shapes in a row, each landing on the ones before by Union, Subtract or Intersect")
    e.note(
        40,
        40,
        "Read the row left to right: every shape takes what was built so far into its Onto and "
        "lands on it by its Mode. A Sphere set to Intersect trims the Box into a rounded die, a "
        "Cylinder set to Subtract bores a hole through it, a Torus is melted round the middle "
        "by its Blend, and the Ground comes last so the Intersect never cuts it. Try the "
        "Torus's Blend up to 0.6, or the Cylinder's Mode set to Union.",
        width=420,
    )
    master = e.master(0, 0)
    scene = e.chain(
        0,
        0,
        [
            ("box", dict(width=1.8, height=1.8, depth=1.8, rounding=0.04, color="#c9c4d6")),
            ("sphere", dict(mode="intersect", radius=1.22, color="#e8925a")),
            ("cylinder", dict(mode="subtract", radius=0.42, height=3.0, color="#f2b632", turn={"pitch": 0.25})),
            ("torus", dict(ring=1.12, tube=0.16, blend=0.25, color="#5a8fe8")),
            ("cone", dict(y=1.35, bottom=0.45, top=0.0, height=0.9, blend=0.15, color="#8fd17a")),
            ("ground", dict(height=-0.9, color="#4a4652")),
        ],
    )
    e.show(0, 0, scene, master, "turn", controls={"distance": 4.8, "tilt": 0.07, "targetY": 0.15, "fog": 0.02}, options={"floor": "none"})
    return e


def turning(new):
    e = new("Turning and scaling a shape", "A shape's own Yaw, Pitch, Roll and Scale under its Transform heading")
    e.note(
        40,
        40,
        "Every shape turns and scales about its own center under its Transform heading. The Box "
        "turns by its own Yaw, cabled from the Master Gear; the Torus stands up by its Pitch "
        "and leans by its Roll; an Oscillator breathes the Octahedron's Scale. Drag any of "
        "the Yaw, Pitch, Roll or Scale rows — angles are in turns, so 0.25 is a quarter turn.",
    )
    master = e.master(0, 0)
    scene = e.chain(
        0,
        0,
        [
            ("box", dict(x=-2.4, width=1.3, height=1.3, depth=1.3, rounding=0.06, color="#5a8fe8", turn={"pitch": 0.1})),
            ("torus", dict(ring=0.75, tube=0.26, color="#d67a9e", turn={"pitch": 0.25, "roll": 0.08})),
            ("octahedron", dict(x=2.4, y=0.1, size=1.0, color="#f2b632", turn={"yaw": 0.06})),
        ],
    )
    box = next(n["id"] for n in e.nodes if n["slug"] == "box")
    e.cable(master, "wrapped", box, "tYaw")
    breathe = e.sway(0, 0, master, amplitude=0.3, offset=1.0)
    e.cable(breathe, "output", scene, "tScale")
    e.show(0, 0, scene, controls={"distance": 6.5, "tilt": 0.06, "orbit": 0.0, "fog": 0.02})
    return e


def repeat_forever(new):
    e = new("Repeat forever", "A grid of copies to the horizon, each scattered, spun, sized and tinted")
    e.note(
        40,
        40,
        "One rounded box, repeated on a grid that goes on forever: the Repeat folds every point "
        "into one cell, so endless copies cost what one does. Each copy is moved, turned, shrunk "
        "and tinted by its cell and the Seed, and the Ground after the Repeat is not repeated, "
        "since a modifier changes only what is cabled into it. Try the Repeat's Seed, Scatter, "
        "Spin, Size and Tint, or Render 3D's Lens Type on Fisheye.",
    )
    master = e.master(0, 0, length=8.0)
    box = e.add("box", 0, 0, controls={"width": 1.1, "height": 1.1, "depth": 1.1, "rounding": 0.18, "color": "#e8e2d6"})
    grid = e.mod(
        "repeatgrid",
        0,
        0,
        box,
        controls={"spacingX": 2.6, "spacingZ": 2.6},
        options={"extent": "forever"},
    )
    floor = e.shape("ground", 0, 0, onto=grid, height=-0.6, color="#6a5f8a")
    e.show(
        0,
        0,
        floor,
        master,
        "turn",
        controls={"distance": 9.0, "tilt": 0.035, "targetY": 0.5, "fog": 0.05, "background": "#6a5f8a", "lens": 1.4, "step": 0.6},
        options={"floor": "none"},
    )
    return e


def repeat_around(new):
    e = new("Repeat around", "A ring, an arch and a spiral staircase from Repeat Around")
    e.note(
        40,
        40,
        "Three Repeat Arounds of one shape each: twelve boxes in a ring standing on Z, seven "
        "balls in an arch where Spread is half a turn, and a spiral staircase round Y where each "
        "step Rises a little above the last, its post chained on after the Repeat so there "
        "is one post, not sixteen. Each Place moves its group into the row and a Union "
        "meets the three. Try the ring's Copies, the arch's Spread, and the staircase's Rise and "
        "Grow — grow it too far and the top steps are sliced by their neighbors' cells.",
    )
    master = e.master(0, 0)
    cube = e.add("box", 0, 0, controls={"width": 0.5, "height": 0.5, "depth": 0.5, "rounding": 0.06, "color": "#5a8fe8"})
    ring = e.mod("repeataround", 0, 0, cube, controls={"copies": 12.0, "radius": 1.5}, options={"axis": "z"})
    left = e.place(0, 0, ring, x=-5.0, y=1.0)

    pill = e.add("sphere", 0, 0, controls={"radius": 0.32, "color": "#e8d35a"})
    arch = e.mod("repeataround", 0, 0, pill, controls={"copies": 7.0, "radius": 1.7, "spread": 0.5}, options={"axis": "z"})
    # Rolled half a turn, so the half the arc keeps is the upper one.
    middle = e.place(0, 0, arch, y=-0.75, roll=0.5)

    slab = e.add("box", 0, 0, controls={"width": 1.3, "height": 0.14, "depth": 0.5, "rounding": 0.03, "color": "#e8925a"})
    stairs = e.mod(
        "repeataround",
        0,
        0,
        slab,
        controls={"copies": 16.0, "radius": 1.0},
        options={"axis": "y"},
    )
    tower = e.shape("cylinder", 0, 0, onto=stairs, radius=0.14, height=3.6, y=1.4, color="#c9c4d6")
    right = e.place(0, 0, tower, x=5.0, y=-0.9)

    scene = e.union(0, 0, [left, middle, right])
    e.show(0, 0, scene, master, (0.06, 0.0), controls={"distance": 9.6, "tilt": 0.06, "targetY": 0.4, "fog": 0.03})
    return e


def fold(new):
    e = new("Fold", "A box folded like a kaleidoscope into a fractal of its own copies")
    e.note(
        40,
        40,
        "Fold mirrors space onto one wedge of a symmetry, turns it and scales it, Iterations "
        "times, then puts the box in the folded space: a fractal of its copies for the price of "
        "one box. An Oscillator swings Turn X either side of 0, so the fractal twists and folds "
        "back each loop. Try Scale between 1.5 and 3, Iterations, and the Symmetry on Cube with "
        "Scale 3 for a sponge.",
    )
    master = e.master(0, 0)
    swing = e.sway(0, 0, master, amplitude=0.03, offset=0.0)
    box = e.add("box", 0, 0, controls={"width": 2.0, "height": 2.0, "depth": 2.0, "rounding": 0.03, "color": "#e8e2d6"})
    f = e.mod("fold", 0, 0, box, controls={"scale": 2.0, "tint": 0.35, "turnZ": 0.0}, options={"symmetry": "tetrahedron", "iterations": "4"})
    e.cable(swing, "output", f, "turnX")
    e.show(0, 0, f, master, (0.04, 0.0), controls={"distance": 4.4, "tilt": 0.06, "fog": 0.02, "targetY": 0.0, "floorHeight": -1.4})
    return e


def noise_warp(new):
    e = new("Noise, warp and displace", "A ball roughened into a rock, a torus pushed about, a ball rippling")
    e.note(
        40,
        40,
        "Three modifiers that move a surface, one branch each. Noise Displace roughens a ball "
        "into a rock; Warp pushes the space round a torus by a smooth noise, flowing as the gear "
        "turns its Phase; Displace ripples a ball by a wave the gear rolls round it. Try Noise "
        "Displace's Octaves and Roughness, Warp's Amount, and Displace's Frequency.",
    )
    master = e.master(0, 0)
    ball = e.add("sphere", 0, 0, controls={"x": -3.0, "radius": 1.0, "color": "#a89a8a"})
    rock = e.mod("noisedisplace", 0, 0, ball, controls={"amount": 0.28, "frequency": 1.6, "roughness": 0.55}, options={"octaves": "5"})
    torus = e.shape("torus", 0, 0, ring=0.8, tube=0.32, color="#5ad1c4", turn={"pitch": 0.2})
    flow = e.mod("warp", 0, 0, torus, controls={"amount": 0.35, "frequency": 0.9})
    e.cable(master, "cycles", flow, "phase")
    pearl = e.add("sphere", 0, 0, controls={"x": 3.0, "radius": 0.95, "color": "#e8925a"})
    ripple = e.mod("displace", 0, 0, pearl, controls={"amount": 0.1, "frequency": 3.0})
    e.cable(master, "cycles", ripple, "phase")
    scene = e.union(0, 0, [rock, flow, ripple])
    # 256 Steps: at 128 the rays that graze the rock run out of steps and leave a dark rim.
    e.show(0, 0, scene, master, (0.04, 0.0), resolution=HEAVY, controls={"distance": 7.8, "tilt": 0.07, "fog": 0.02, "targetY": 0.1}, options={"steps": "256"})
    return e


def seams(new):
    e = new("Seams", "Stairs Union, Columns Union and Exclusive, each joining two solids")
    e.note(
        40,
        40,
        "Three ways to meet two solids other than a plain union. Stairs Union joins a floor and "
        "a wall with a flight of steps; Columns Union fills the seam of a floor and a post with "
        "fluted columns; Exclusive keeps only where exactly one of two balls is, and a Box set "
        "to Subtract cuts the front open so the hollow overlap shows. Try Stairs Union's Steps "
        "and Radius, and Columns Union's Columns.",
    )
    master = e.master(0, 0)
    floor_a = e.add("box", 0, 0, controls={"x": -4.0, "y": -0.8, "width": 2.6, "height": 0.4, "depth": 1.6, "rounding": 0.02, "color": "#c9c4d6"})
    wall = e.add("box", 0, 0, controls={"x": -4.9, "y": 0.2, "width": 0.5, "height": 2.4, "depth": 1.6, "rounding": 0.02, "color": "#5a8fe8"})
    stairs = e.pair("stairsunion", 0, 0, floor_a, wall, controls={"radius": 0.8, "steps": 5.0})

    floor_b = e.add("box", 0, 0, controls={"y": -0.8, "width": 2.6, "height": 0.4, "depth": 1.6, "rounding": 0.02, "color": "#c9c4d6"})
    post = e.add("cylinder", 0, 0, controls={"y": 0.3, "radius": 0.35, "height": 2.4, "color": "#e8925a"})
    columns = e.pair("columnsunion", 0, 0, floor_b, post, controls={"radius": 0.5, "columns": 5.0})

    one = e.add("sphere", 0, 0, controls={"x": 3.6, "radius": 0.9, "color": "#d67a9e"})
    two = e.add("sphere", 0, 0, controls={"x": 4.6, "radius": 0.9, "color": "#8fd17a"})
    xor = e.pair("solidxor", 0, 0, one, two)
    opened = e.shape("box", 0, 0, onto=xor, mode="subtract", x=4.1, z=1.0, width=3.0, height=3.0, depth=2.0, color="#f2b632")

    scene = e.union(0, 0, [stairs, columns, opened])
    e.show(0, 0, scene, master, (0.05, 0.03), controls={"distance": 9.5, "tilt": 0.08, "fog": 0.02})
    return e


def color_fields(new):
    e = new("Color fields on a solid", "Checker 3D, Facing, Height Gradient and Stripes 3D painting four shapes")
    e.note(
        40,
        40,
        "A color field paints a solid by where its surface is and leaves its shape alone. Left "
        "to right: Checker 3D cuts a ball into cubes of two colors, Facing paints a dodecahedron "
        "by which way each face looks, Height Gradient runs a helix from blue to gold, and "
        "Stripes 3D bands a torus, the gear sliding the bands up through it. Try Checker 3D's "
        "Size, Facing's three colors, and Stripes 3D's Axis.",
    )
    master = e.master(0, 0)
    ball = e.add("sphere", 0, 0, controls={"x": -4.2, "radius": 0.9})
    checks = e.mod("checker3d", 0, 0, ball, controls={"size": 0.4, "colorA": "#f2efe6", "colorB": "#3a5ae8"})

    gem = e.shape("dodecahedron", 0, 0, x=-1.4, y=0.05, size=1.0, turn={"yaw": 0.07, "pitch": 0.04})
    faces = e.mod("facing", 0, 0, gem, controls={"colorUp": "#f2efe6", "colorSide": "#e8925a", "colorDown": "#3a2a5a"})

    spring = e.add("helix", 0, 0, controls={"x": 1.4, "y": 0.1, "radius": 0.6, "pitch": 0.4, "tube": 0.13, "height": 2.2})
    heights = e.mod("heightgradient", 0, 0, spring, controls={"bottom": -1.0, "top": 1.2, "colorBottom": "#3a5ae8", "colorTop": "#f2b632"})

    ring = e.shape("torus", 0, 0, x=4.2, y=-0.1, ring=0.8, tube=0.35, turn={"pitch": 0.15})
    # Two bands, Width 0.2 each, a loop: the offset climbs 0.4 a loop and comes back the same.
    climb = e.add("multiply", 0, 0, controls={"b": 0.4})
    e.cable(master, "wrapped", climb, "a")
    stripes = e.mod("stripes3d", 0, 0, ring, controls={"width": 0.2, "colorA": "#e8e2d6", "colorB": "#d67a9e"}, options={"axis": "y"})
    e.cable(climb, "output", stripes, "offset")

    scene = e.union(0, 0, [checks, faces, heights, stripes])
    e.show(0, 0, scene, master, (0.04, 0.0), controls={"distance": 8.2, "tilt": 0.07, "fog": 0.02})
    return e


def video_chain(e, master):
    """A small video patch for a solid to wear: checkers turned once a loop of `master`,
    folded by a Kaleidoscope and tinted by rings. Its last node is returned."""
    checks = e.add("checkerboard", 0, 0, controls={"frequency": 5.0, "color1": "#fff1b8", "color2": "#4b2aa8"})
    turn = e.add("rotate", 0, 0)
    e.cable(master, "wrapped", turn, "angle")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6.0}, width=240)
    rings = e.add(
        "radialgradient",
        0,
        0,
        controls={"centerColor": "#ff5e7e", "edgeColor": "#2ec4b6", "radius": 0.6},
        options={"loop_mode": "mirror"},
    )
    tint = e.add("layerblend", 0, 0, options={"blend_mode": "multiply"})
    e.cable(checks, "output", turn, "input")
    e.cable(turn, "output", kal, "input")
    e.cable(kal, "output", tint, "background")
    e.cable(rings, "output", tint, "foreground")
    return tint


def video_onto(new):
    e = new("Video onto a solid", "A video patch worn by a box, a cylinder and a ball through three Textures")
    e.note(
        40,
        40,
        "A Texture puts any picture on a solid's skin — here a turning kaleidoscope of checkers "
        "built in the same patch. Each Texture's Mapping makes a picture out of the surface its "
        "own way: Box wraps it from six sides, Around rolls it round the cylinder, Sphere maps "
        "it onto the ball. Try each Texture's Mapping and Scale, or move a Place before its "
        "Texture and the shape slides through the picture like a projector.",
    )
    master = e.master(0, 0)
    picture = video_chain(e, master)
    shapes = [
        ("box", {"width": 1.8, "height": 1.8, "depth": 1.8, "rounding": 0.06}, "box", 0.9, -3.6, -0.1),
        ("cylinder", {"radius": 0.9, "height": 2.0}, "around", 1.0, 0.0, 0.0),
        ("sphere", {"radius": 1.0}, "sphere", 0.5, 3.6, 0.0),
    ]
    parts = []
    for slug, controls, projection, scale, x, lift in shapes:
        s = e.add(slug, 0, 0, controls=controls)
        tex = e.mod("surfacetexture", 0, 0, s, controls={"scale": scale}, options={"projection": projection})
        e.cable(picture, "output", tex, "picture")
        parts.append(e.place(0, 0, tex, x=x, y=lift))
    scene = e.union(0, 0, parts)
    e.show(0, 0, scene, master, (0.07, 0.0), controls={"distance": 8.0, "tilt": 0.07, "fog": 0.03, "light": 0.08})
    return e


def into_video(new):
    e = new("A solid into video effects", "A Render 3D picture folded by a Kaleidoscope and shifted in hue")
    e.note(
        40,
        40,
        "Render 3D's picture is an ordinary picture, so any video node can take it: a turning "
        "ring of balls and a torus is folded by a Kaleidoscope and tinted by a Color Shift. "
        "Try the Kaleidoscope's Segments and Zoom, or cable Render 3D straight into the Output "
        "to see what is being folded. A blur after a render is costly — it marches the scene "
        "again for every tap.",
    )
    master = e.master(0, 0)
    ball = e.add("sphere", 0, 0, controls={"radius": 0.3, "color": "#5ae8c8"})
    ring = e.mod("repeataround", 0, 0, ball, controls={"copies": 10.0, "radius": 2.1})
    both = e.shape("torus", 0, 0, onto=ring, ring=1.2, tube=0.32, blend=0.2, color="#e8925a")
    spin = e.place(0, 0, both, pitch=0.1)
    e.cable(master, "wrapped", spin, "yaw")
    r = e.render(0, 0, spin, controls={"distance": 5.0, "tilt": 0.1, "fog": 0.0, "background": "#0d0a14"}, options={"floor": "none"})
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8.0, "zoom": 1.2}, width=240)
    shift = e.add("colorshift", 0, 0)
    e.cable(master, "wrapped", shift, "hue")
    e.cable(r, "output", kal, "input")
    e.cable(kal, "output", shift, "input")
    e.cable(shift, "output", e.output(0, 0), "input")
    return e


def extrude_revolve(new):
    e = new("Extrude and revolve", "A word pushed out into letters and a pentagon turned on a lathe")
    e.note(
        40,
        40,
        "Pictures into solids: Extrude pushes out a blurred Text wherever it is brighter than "
        "Threshold, Depth deep, and Revolve spins the right half of a Polygon round the upright "
        "axis like a lathe into a spinning top. The Blur softens the letters' edge so the march "
        "can trust it; unplug it and the edges tear. Try Extrude's Depth and Bevel, the "
        "Polygon's Sides, and the Text's words.",
        width=420,
    )
    master = e.master(0, 0)
    outline = e.add("polygon", 0, 0, controls={"sides": 5.0, "radius": 0.7, "rotation": 0.1, "softness": 0.06})
    lathe = e.shape("revolve", 0, 0, scale=1.2, steepness=10.0, color="#d6a35a")
    e.cable(outline, "color", lathe, "profile")
    aside = e.place(0, 0, lathe, x=3.3, y=-0.1)

    words = e.add(
        "text",
        0,
        0,
        controls={"textColor": "#e8925a", "backgroundColor": "#000000"},
        options={"size": "420", "font": "Impact", "weight": "bold"},
        values={"words": {"Text": "SOLID"}},
    )
    soft = e.add("blur", 0, 0, controls={"blurX": 5.0, "blurY": 5.0}, options={"size": "7x7"})
    e.cable(words, "output", soft, "input")
    letters = e.shape(
        "extrude",
        0,
        0,
        onto=aside,
        scale=1.2,
        depth=0.5,
        threshold=0.25,
        steepness=8.0,
        bevel=0.03,
        turn={"yaw": 0.04},
        options={"paint": "picture"},
    )
    e.cable(soft, "output", letters, "profile")
    e.show(
        0,
        0,
        letters,
        master,
        (0.05, 0.02),
        resolution=HEAVY,
        controls={"distance": 5.6, "tilt": 0.06, "targetX": 1.0, "targetY": 0.0, "fog": 0.02},
    )
    return e


def heightfield(new):
    e = new("Heightfield terrain", "Perlin noise raised into a landscape colored by itself, with a lake")
    e.note(
        40,
        40,
        "Heightfield raises a picture into ground: each point stands at Base plus Height times "
        "the picture's brightness, painted by the picture itself. The picture is a Perlin Noise "
        "walking round its loop with the gear, so hills rise and sink, and a Ground chained on "
        "after it is a lake. Try Height and the noise's Scale; too low a Steepness tears the "
        "slopes, too high makes the march crawl.",
    )
    master = e.master(0, 0)
    noise = e.add("perlin", 0, 0, controls={"scale": 3.0, "foreground": "#eadcae", "background": "#1d4f25", "contrast": 2.0}, options={"repeat": "1"})
    e.cable(master, "cycles", noise, "clock")
    land = e.add("heightfield", 0, 0, controls={"scale": 4.0, "base": -1.0, "height": 1.3, "steepness": 4.0}, options={"paint": "picture"})
    e.cable(noise, "color", land, "picture")
    lake = e.shape("ground", 0, 0, onto=land, height=-0.55, color="#2e5a9a", blend=0.05)
    e.show(
        0,
        0,
        lake,
        master,
        (0.04, 0.08),
        resolution=HEAVY,
        controls={"distance": 9.0, "tilt": 0.1, "fog": 0.025, "background": "#9fb4d8"},
        options={"floor": "none"},
    )
    return e


def glow(new):
    e = new("Glow", "Render 3D's Glow: a halo where rays pass near a surface")
    e.note(
        40,
        40,
        "With Render 3D's Glow above zero, every ray that passes close to a surface without "
        "hitting it picks up Glow Color, more the closer and longer it grazes. Thin shapes on a "
        "dark ground show it best: a Box Frame turning round a Helix. Try the Glow port on "
        "Render 3D, and Glow Reach and Glow Color in its Light section.",
    )
    master = e.master(0, 0)
    frame = e.shape("boxframe", 0, 0, width=2.0, height=2.0, depth=2.0, edge=0.05, color="#2a2a3a", turn={"pitch": 0.1})
    spring = e.shape("helix", 0, 0, onto=frame, radius=0.45, pitch=0.35, tube=0.06, height=1.6, color="#f2efe6")
    e.cable(master, "wrapped", frame, "tYaw")
    e.show(
        0,
        0,
        spring,
        controls={"distance": 5.0, "tilt": 0.05, "orbit": 0.08, "fog": 0.0, "background": "#07060c", "glowAmount": 0.5, "glowReach": 0.12, "glowColor": "#59d8ff"},
        options={"floor": "none"},
    )
    return e


def toon(new):
    e = new("Toon shading", "Render 3D's Shading on Toon: flat bands of light and dark outlines")
    e.note(
        40,
        40,
        "Render 3D's Shading on Toon: the light falls in a few flat Bands instead of smoothly, "
        "and an Outline is drawn round every silhouette. Try Bands from 2 to 8 and the Outline "
        "width in its Look section; set Shading to Normals or Depth to see what the render "
        "knows, or Reflections to One under Quality for mirror-like surfaces.",
    )
    master = e.master(0, 0)
    scene = e.chain(
        0,
        0,
        [
            ("sphere", dict(x=-1.6, y=-0.15, radius=0.85, color="#e8925a")),
            ("box", dict(x=0.4, y=-0.35, z=-0.6, width=1.3, height=1.3, depth=1.3, rounding=0.08, color="#5a8fe8", turn={"yaw": 0.08})),
            ("torus", dict(x=1.9, y=-0.2, z=0.7, ring=0.6, tube=0.2, color="#d67a9e", turn={"pitch": 0.2})),
            ("capsule", dict(x=0.0, y=-0.3, z=1.3, length=0.5, radius=0.35, color="#8fd17a", turn={"roll": 0.25})),
        ],
    )
    e.show(
        0,
        0,
        scene,
        master,
        (0.05, 0.05),
        controls={"distance": 6.5, "tilt": 0.08, "fog": 0.0, "bands": 3.0, "outline": 0.02, "background": "#f2e8c8"},
        options={"shading": "toon"},
    )
    return e


def side_by_side(e, left, right, shift=0.89):
    """`left`'s picture on the left half and `right`'s on the right, each moved over so the
    middle of its picture is the middle of its half."""
    a = e.add("translate", 0, 0, controls={"x": -shift})
    b = e.add("translate", 0, 0, controls={"x": shift})
    e.cable(left, "output", a, "input")
    e.cable(right, "output", b, "input")
    world = e.add("worldcoordinates", 0, 0)
    split = e.add("threshold", 0, 0, controls={"threshold": 0.0, "smooth": 0.004})
    e.cable(world, "x", split, "input")
    both = e.add("mix", 0, 0)
    e.cable(a, "output", both, "a")
    e.cable(b, "output", both, "b")
    e.cable(split, "output", both, "amount")
    return both


def costs(new):
    e = new("What the march costs", "A bar twisting harder each loop, Lit on the left and in Steps shading on the right")
    e.note(
        40,
        40,
        "The same twisted bar through two Render 3Ds: Lit on the left, and on the right Shading "
        "on Steps, which paints how hard the march worked for each pixel, blue for few steps to "
        "red for many. An Oscillator winds the Twist's Rate up and back; a twist has to shrink "
        "its distance to stay safe, so the harder it winds the hotter it reads, and at 64 Steps "
        "rays give up and pink gaps open along the lit bar's edges. Try both renders' Steps "
        "at 256 under Quality, or a Displace in place of the Twist.",
    )
    master = e.master(0, 0)
    bar = e.add("box", 0, 0, controls={"y": 0.1, "width": 1.4, "height": 2.4, "depth": 0.5, "rounding": 0.03, "color": "#5a8fe8"})
    twist = e.mod("twist", 0, 0, bar)
    wind = e.sway(0, 0, master, amplitude=0.35, offset=0.35)
    e.cable(wind, "output", twist, "rate")
    camera = {"distance": 5.0, "tilt": 0.03, "orbit": 0.0, "fog": 0.0, "lens": 2.0, "background": "#ff3d8b"}
    lit = e.render(0, 0, twist, controls=camera, options={"floor": "none", "steps": "64"})
    steps = e.render(0, 0, twist, controls=camera, options={"floor": "none", "steps": "64", "shading": "steps"})
    e.cable(side_by_side(e, lit, steps), "output", e.output(0, 0, resolution=HEAVY), "input")
    return e


def skydome(new):
    e = new("Skydome", "A glowing green grid wrapped round the whole scene through Render 3D's Sky, turning")
    e.note(
        40,
        40,
        "A picture cabled into Render 3D's Sky is wrapped round the whole scene, above and below: "
        "its width goes once round the horizon and its height from pole to pole, so a Grid "
        "becomes lines of latitude and longitude. Bloom makes the lines glow, and a Translate "
        "driven by the Master Gear's Wrapped turns the sphere, since moving the picture along X "
        "turns the sky. The shiny ball and ring show it in their reflections. Try Sky Tilt in "
        "Render 3D's Look section to tip the pole, or the Grid's Cells.",
        width=420,
    )
    master = e.master(0, 0, length=8.0)
    grid = e.add(
        "grid",
        0,
        0,
        controls={"foreground": "#3dff6e", "background": "#020a04", "cellsX": 18.0, "cellsY": 18.0, "thickness": 0.025, "smoothing": 0.015},
    )
    glow = e.add("bloom", 0, 0, controls={"threshold": 0.3, "intensity": 1.6, "radius": 0.003}, options={"quality": "high"})
    spin = e.add("translate", 0, 0)
    e.cable(grid, "color", glow, "input")
    e.cable(glow, "color", spin, "input")
    e.cable(master, "wrapped", spin, "x")
    scene = e.chain(
        0,
        0,
        [
            ("sphere", dict(radius=0.9, color="#0c1a10")),
            ("torus", dict(ring=1.45, tube=0.12, color="#1d3b26", turn={"pitch": 0.22, "roll": 0.06})),
        ],
    )
    render = e.show(
        0,
        0,
        scene,
        controls={"distance": 4.6, "tilt": 0.04, "orbit": 0.08, "fog": 0.0, "skyTilt": 0.06, "reflectivity": 0.85, "lightHeight": 0.18},
        options={"floor": "none", "reflections": "one"},
    )
    e.cable(spin, "output", render, "sky")
    return e


def examples(new):
    return [
        first_solid(new),
        chain(new),
        turning(new),
        repeat_forever(new),
        repeat_around(new),
        fold(new),
        noise_warp(new),
        seams(new),
        color_fields(new),
        video_onto(new),
        into_video(new),
        extrude_revolve(new),
        heightfield(new),
        glow(new),
        toon(new),
        costs(new),
        skydome(new),
    ]
