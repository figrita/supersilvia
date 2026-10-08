# SPDX-License-Identifier: AGPL-3.0-or-later
"""Sending and recording: an Output's own ways out — a rendered loop, a live recording, Syphon
and NDI, a picture with its alpha, and a Snap."""

from kit_perform import DUSK, EMBER, options

ORDER = 15
CATEGORY = "Sending and recording"


def render_loop(new):
    e = new("Render a loop", "The Output's Render section, as long as the Master Gear's loop")
    e.note(
        40,
        40,
        "The Output's Render section is open: a GIF at 25 FPS, four seconds long. That is the "
        "Master Gear's caption, loops in 1 cycle · 4.000 s, since everything moving is on that "
        "gear, so the film ends exactly where it began. Press Render and it "
        "is written to renders/ in the project. Change the gear's Length, then set Duration "
        "to what its caption says.",
    )
    master = e.master(0, 0)
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 1.4, "octaves": 3.0, "gain": 0.5, "contrast": 1.1},
        options={"type": "ridged", "repeat": "1"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=EMBER)
    warp = e.add(
        "domainwarp",
        0,
        0,
        controls={"amplitude": 0.3, "frequency": 1.2, "octaves": 2.0},
        options={"repeat": "1"},
    )
    out = e.output(0, 0)
    options(e, out, offline="on")
    e.cable(master, "cycles", noise, "clock")
    e.cable(master, "cycles", grad, "clock")
    e.cable(master, "cycles", warp, "clock")
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", warp, "input")
    e.cable(warp, "output", out, "input")
    return e


def live_recording(new):
    e = new("Record a performance", "The Output's Record row: what your hands do, written as a video")
    e.note(
        40,
        40,
        "The Output's Record section is open. Press Record, throw the XY Pad's puck around, "
        "and press Stop: the take is a video in recordings/ in the project, as long as you "
        "played, picture only. Render steps time a frame at a time, so it cannot follow your "
        "hands; Record keeps what really happened. Its FPS is its own, and its line counts any frame "
        "the show was too slow to draw.",
    )
    pad = e.add(
        "xypad",
        0,
        0,
        controls={"minX": -1.4, "maxX": 1.4, "minY": -0.8, "maxY": 0.8, "drag": 0.004, "temperature": 0.6,
                  "padX": 0.4, "padY": 0.3, "vx": 1.2, "vy": -0.9},
    )
    dot = e.add("circle", 0, 0, controls={"radius": 0.14, "softness": 0.05, "foreground": "#9be7ff", "background": "#00000000"})
    e.cable(pad, "x", dot, "centerX")
    e.cable(pad, "y", dot, "centerY")
    zoom = e.add("zoom", 0, 0, controls={"zoom": 0.995})
    fade = e.add("colorshift", 0, 0, controls={"hue": 0.01, "value": 0.975})
    blend = e.add("layerblend", 0, 0)
    out = e.output(0, 0)
    options(e, out, recording="on")
    e.cable(out, "frame", zoom, "input")
    e.cable(zoom, "output", fade, "input")
    e.cable(fade, "output", blend, "background")
    e.cable(dot, "color", blend, "foreground")
    e.cable(blend, "output", out, "input")
    return e


def sending(new):
    e = new("Send to another app", "The Output's Send rows: Syphon to an app on this Mac, NDI to the network")
    e.note(
        40,
        40,
        "The Output's Send section is open, and nothing is sent until you press a button. Name "
        "is what other apps list it as. Syphon hands it to another app on this Mac — a VJ app, "
        "a mapper, OBS — and NDI® sends it to any machine on the network, once the free NDI "
        "runtime is installed. A sent Output stays live with its tab closed; the Main Mixer "
        "sends the whole mix the same way.",
    )
    board = e.add("checkerboard", 0, 0, controls={"frequency": 16, "color1": "#9be7ff", "color2": "#0b1d3a"})
    tunnel = e.add("tunnel3d", 0, 0, controls={"speed": 0.5, "twist": 2.0}, options={"path": "lissajous", "shading": "heavy"})
    out = e.output(0, 0)
    options(e, out, send="on", sendName="supersilvia example")
    e.cable(board, "output", tunnel, "input")
    e.cable(tunnel, "output", out, "input")
    return e


def transparent(new):
    e = new("A transparent Output", "A picture with its alpha kept, to lay over someone else's")
    e.note(
        40,
        40,
        "Only the stars are drawn: their Background is fully transparent, and the Output "
        "keeps the alpha. Its Render section writes a PNG sequence, which keeps it, as a Snap "
        "does; its Send section's Alpha, shown once Syphon or NDI is on, is set to "
        "Transparent, so a receiver can lay the stars over its own video. A video render or a "
        "recording drops the alpha: the picture over black.",
    )
    master = e.master(0, 0)
    star = e.add(
        "star",
        0,
        0,
        controls={"points": 5, "innerRadius": 0.42, "foreground": "#ffd166", "background": "#00000000"},
    )
    small = e.add("zoom", 0, 0, controls={"zoom": 0.22})
    tile = e.add("tile", 0, 0, controls={"width": 0.75, "height": 0.75, "slide": 0.5})
    turn = e.add("rotate", 0, 0)
    out = e.output(0, 0)
    options(e, out, offline="on", send="on", transparent="on", writer="png")
    e.cable(master, "wrapped", star, "rotation")
    e.cable(master, "wrapped", turn, "angle")
    e.cable(star, "color", small, "input")
    e.cable(small, "output", tile, "input")
    e.cable(tile, "output", turn, "input")
    e.cable(turn, "output", out, "input")
    return e


def snapshots(new):
    e = new("Snapshots", "Snap writes the frame in front of you as a full-size PNG")
    e.note(
        40,
        40,
        "Press Snap on the Output, or the Button cabled into it, and the frame in front of you "
        "is written as a PNG at the Output's full resolution into snaps/ in the project, "
        "while the show plays on. Snap is an action like Show on A, so a sequencer lane or a "
        "MIDI pad can take pictures too: Alt+click the Button's Press and hit a pad.",
    )
    master = e.master(0, 0, length=8.0)
    sway = e.sway(0, 0, master, amplitude=0.006, offset=0.382)
    dots = e.add(
        "phyllotaxis",
        0,
        0,
        controls={"count": 420, "radius": 1.25, "dotSize": 0.045, "bg": "#140a24", "fg": "#ffd166"},
    )
    e.cable(sway, "output", dots, "angle")
    glow = e.add("bloom", 0, 0, controls={"threshold": 0.5, "intensity": 0.8, "radius": 0.02})
    button = e.add("button", 0, 0)
    out = e.output(0, 0)
    e.cable(dots, "output", glow, "input")
    e.cable(glow, "color", out, "input")
    e.cable(button, "trigger", out, "snap")
    return e


def examples(new):
    return [
        render_loop(new),
        live_recording(new),
        sending(new),
        transparent(new),
        snapshots(new),
    ]
