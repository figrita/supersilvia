# SPDX-License-Identifier: AGPL-3.0-or-later
"""Mixer and performance: two decks and the fade between them, cuts a sequencer makes, the
controls a hand plays live, MIDI, Blackout and Freeze, and a set kept on one tempo."""

from kit_perform import DUSK, EMBER, OCEAN, ranged

ORDER = 14
CATEGORY = "Mixer and performance"


def two_decks(new):
    e = new("Two decks and a crossfade", "One Output on deck A, another on deck B, and the fade between")
    e.note(
        40,
        40,
        "Press Show on A on the left Output and Show on B on the right one. In the Main Mixer "
        "panel the A / B balance runs from all A at −1 to all B at +1; set Crossfade to a "
        "wipe or a luminance fade and sweep it. The deck that is off air keeps playing, so "
        "change its patch while the other one is on — that is what two decks are for.",
    )
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"frequency": 4, "radius": 0.38, "input": "#ffb347", "background": "#3b0d2c", "stagger": 0.5},
    )
    roto = e.add("rotozoom", 0, 0, controls={"speed": 0.4, "turns": 1.0, "sinCoeff": 0.5, "cosCoeff": 0.7})
    a = e.output(0, 0)
    noise = e.add("perlin", 0, 0, controls={"scale": 2.5, "contrast": 1.8, "speed": 0.4})
    cool = e.add("cosinegradient", 0, 0, controls=OCEAN)
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6, "zoom": 1.2})
    b = e.output(0, 0)
    e.cable(dots, "color", roto, "input")
    e.cable(roto, "output", a, "input")
    e.cable(noise, "value", cool, "t")
    e.cable(cool, "output", kal, "input")
    e.cable(kal, "output", b, "input")
    return e


def sequenced_cuts(new):
    e = new("A sequencer cuts the decks", "Step Sequencer lanes pressing Show on A on three Outputs")
    e.note(
        40,
        40,
        "Each lane of the Step Sequencer is cabled into one Output's Show on A, so deck A cuts "
        "between three patches on the Master Gear's beat. Turn the Main Mixer's balance to A "
        "to watch the cuts, and click cells in the grid to move them. Cable a lane into a "
        "Show on B instead and the sequence cuts the other deck.",
    )
    master = e.master(0, 0)
    seq = e.add(
        "stepsequencer",
        0,
        0,
        values={
            "pattern": {
                "Cells": [
                    "x.......x.......",
                    "....x.......x...",
                    "......x.......x.",
                    "................",
                ]
            }
        },
    )
    e.cable(master, "cycles", seq, "clock")
    board = e.add("checkerboard", 0, 0, controls={"frequency": 5, "color1": "#ffe8c2", "color2": "#3a1c71"})
    roto = e.add("rotozoom", 0, 0, controls={"turns": 1.0, "sinCoeff": 0.6, "cosCoeff": 0.6})
    one = e.output(0, 0)
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"frequency": 6, "radius": 0.4, "input": "#ff6f59", "background": "#0b1d3a", "stagger": 0.5},
    )
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 10, "zoom": 1.5})
    two = e.output(0, 0)
    cells = e.add("worley", 0, 0, controls={"scale": 4.0, "contrast": 1.1})
    tint = e.add("cosinegradient", 0, 0, controls=EMBER)
    three = e.output(0, 0)
    e.cable(master, "cycles", roto, "clock")
    e.cable(master, "wrapped", kal, "offset")
    e.cable(master, "cycles", tint, "clock")
    e.cable(board, "output", roto, "input")
    e.cable(roto, "output", one, "input")
    e.cable(dots, "color", kal, "input")
    e.cable(kal, "output", two, "input")
    e.cable(cells, "value", tint, "t")
    e.cable(tint, "output", three, "input")
    e.cable(seq, "lane1", one, "show_a")
    e.cable(seq, "lane2", two, "show_a")
    e.cable(seq, "lane3", three, "show_a")
    return e


def button_and_pad(new):
    e = new("A button and an XY pad", "Throw the puck to steer the picture; hold the button to flash it")
    e.note(
        40,
        40,
        "Throw the puck in the XY Pad: its X twists the Kaleidoscope and its Y zooms it, and "
        "it bounces on by itself until Drag slows it. Hold the Button for a flash of white — "
        "the ADSR shapes it, so its Decay is how fast the flash fades. Alt+click the Button's "
        "Press, then hit a pad on a MIDI controller, to play it from there.",
    )
    noise = e.add("simplex", 0, 0, controls={"scale": 2.0, "contrast": 1.3, "speed": 0.3})
    grad = e.add("cosinegradient", 0, 0, controls=DUSK)
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 8})
    pad = e.add(
        "xypad",
        0,
        0,
        controls={"minX": -1.5, "maxX": 1.5, "minY": 0.6, "maxY": 2.4, "drag": 0.01, "padX": 0.3, "padY": 0.2},
    )
    button = e.add("button", 0, 0)
    env = e.add("adsr", 0, 0, controls={"attack": 0.01, "decay": 0.35, "sustain": 0.25, "release": 0.4})
    flash = e.add("layerblend", 0, 0, options={"blend_mode": "add"}, controls={"opacity": 0.0})
    e.cable(noise, "value", grad, "t")
    e.cable(grad, "output", kal, "input")
    e.cable(pad, "x", kal, "twist")
    e.cable(pad, "y", kal, "zoom")
    e.cable(kal, "output", flash, "background")
    e.cable(button, "trigger", env, "gate")
    e.cable(env, "value", flash, "opacity")
    e.cable(flash, "output", e.output(0, 0), "input")
    return e


def midi_knobs(new):
    e = new("Knobs ready for MIDI", "Alt+click a knob, turn a controller's: the ranges are set for it")
    e.note(
        40,
        40,
        "Alt+click any knob, then turn a knob on a MIDI controller: they are bound, and the "
        "knob wears a dot. A controller sweeps the knob's own range, so the ones worth playing "
        "here have been narrowed — Twist 0 to 3, Segments 3 to 12, Hue Shift 0 to 1 — and wear "
        "accent borders. Bindings are kept in the project rather than in a workspace: Project "
        "▸ MIDI… lists them and clears them.",
    )
    board = e.add("checkerboard", 0, 0, controls={"frequency": 16, "color1": "#ffd166", "color2": "#1b1035"})
    tunnel = e.add("tunnel3d", 0, 0, controls={"speed": 0.6, "twist": 1.5})
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6, "zoom": 1.2})
    hue = e.add("colorshift", 0, 0, controls={"hue": 0.0, "saturation": 1.3})
    ranged(e, tunnel, "twist", 0.0, 3.0, 0.01)
    ranged(e, kal, "segments", 3.0, 12.0, 1.0)
    ranged(e, hue, "hue", 0.0, 1.0, 0.01)
    e.cable(board, "output", tunnel, "input")
    e.cable(tunnel, "output", kal, "input")
    e.cable(kal, "output", hue, "input")
    e.cable(hue, "output", e.output(0, 0), "input")
    return e


def blackout_freeze(new):
    e = new("Blackout and Freeze", "The two presses that save a set, under the Main Mixer's fade")
    e.note(
        40,
        40,
        "Press Show on A, then press Freeze in the Main Mixer: the projector holds this frame "
        "while the patch races on underneath, and pressing it again cuts back to it live. "
        "Blackout takes the projector to black until it is pressed again. Both hold wherever "
        "the mix is shown — a window, Syphon, NDI — and Alt+click puts either on a MIDI pad.",
    )
    master = e.master(0, 0, length=2.0)
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"frequency": 4, "radius": 0.42, "input": "#ffffff", "background": "#000000", "stagger": 0.5},
    )
    roto = e.add("rotozoom", 0, 0, controls={"turns": 1.0, "baseZoom": 0.8, "sinCoeff": 0.6, "cosCoeff": 0.8})
    tint = e.add("colorize", 0, 0, controls={"tint": "#ff3d7f", "amount": 1.0})
    shift = e.add("colorshift", 0, 0)
    e.cable(master, "cycles", roto, "clock")
    e.cable(master, "wrapped", shift, "hue")
    e.cable(dots, "color", roto, "input")
    e.cable(roto, "output", tint, "input")
    e.cable(tint, "output", shift, "input")
    e.cable(shift, "output", e.output(0, 0), "input")
    return e


def set_of_gears(new):
    e = new("A set built of gears", "One Master Gear's beat under both decks, so every cut lands on it")
    e.note(
        40,
        40,
        "One Master Gear half a second long is the beat, and Ratio Gears divide it into a bar "
        "(1 : 4) and a four-bar phrase (1 : 16). Both decks run on them, so whatever is on "
        "air stays on the beat. Turn the Master Gear's Length to retime the whole set at once "
        "— it bends, never jumps — and use its Hold and Reset to stop it and start again on "
        "the downbeat.",
    )
    beat = e.master(0, 0, length=0.5, display="gears")
    bar = e.add("ratiogear", 0, 0, controls={"p": 1, "q": 4})
    phrase = e.add("ratiogear", 0, 0, controls={"p": 1, "q": 16})
    e.cable(beat, "cycles", bar, "clock")
    e.cable(beat, "cycles", phrase, "clock")
    # Deck A: a tunnel flown once a phrase.
    board = e.add("checkerboard", 0, 0, controls={"frequency": 16, "color1": "#ff6f59", "color2": "#0b1d3a"})
    tunnel = e.add("tunnel3d", 0, 0, controls={"twist": 1.0}, options={"shading": "light"})
    a = e.output(0, 0)
    e.cable(phrase, "cycles", tunnel, "clock")
    e.cable(board, "output", tunnel, "input")
    e.cable(tunnel, "output", a, "input")
    # Deck B: dots slid round a kaleidoscope once a bar and kicked on every beat.
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"frequency": 5, "radius": 0.4, "input": "#ffd166", "background": "#140a24", "stagger": 0.5},
    )
    kick = e.add("oscillator", 0, 0, controls={"amplitude": 0.4, "offset": 1.2}, options={"waveform": "sawtooth"})
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6})
    b = e.output(0, 0)
    e.cable(bar, "wrapped", kal, "offset")
    e.cable(beat, "cycles", kick, "clock")
    e.cable(kick, "output", kal, "zoom")
    e.cable(dots, "color", kal, "input")
    e.cable(kal, "output", b, "input")
    return e


def examples(new):
    return [
        two_decks(new),
        sequenced_cuts(new),
        button_and_pad(new),
        midi_knobs(new),
        blackout_freeze(new),
        set_of_gears(new),
    ]
