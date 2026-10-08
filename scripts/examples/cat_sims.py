# SPDX-License-Identifier: AGPL-3.0-or-later
"""Simulations: pictures that are a world stepped on from the last frame — Life and its
rulesets, the slime mold, the brick game — rather than a function of time. None of them closes
a loop; each keeps going for as long as it is left to."""

from ast_common import SUNSET

ORDER = 11
CATEGORY = "Simulations"


def life(new):
    e = new("Game of Life", "Conway's Life on a wrapping grid, with fading trails")
    e.note(
        40,
        40,
        "Conway's Game of Life: a cell is born with three live neighbors and lives on with two "
        "or three. Auto-Run steps it thirty times a second, and Trail Decay leaves a fading "
        "ghost where cells died. When it settles into blinkers, press Randomize; Init "
        "Threshold, read on that press, is how empty the new grid starts. The world wraps round "
        "at its edges, so its picture tiles; the Zoom brings one tile up to the frame's height.",
    )
    cells = e.add(
        "cellularautomata",
        0,
        0,
        controls={
            "autoRun": 1.0,
            "stepsPerFrame": 1.0,
            "initThreshold": 0.7,
            "trailDecay": 0.06,
            "aliveInput": "#ffe08a",
            "deadInput": "#0b1a2e",
        },
        options={"algorithm": "life", "gridScale": "8"},
    )
    near = e.add("zoom", 0, 0, controls={"zoom": 2.0})
    e.cable(cells, "output", near, "input")
    e.cable(near, "output", e.output(0, 0), "input")
    return e


def brians_brain(new):
    e = new("Life's other rules", "Brian's Brain: cells fire, rest a step, and travel in sparks")
    e.note(
        40,
        40,
        "The same node under another ruleset. In Brian's Brain a cell fires, spends a step "
        "resting and only then can fire again, so nothing ever settles and sparks travel in "
        "streams. Switch the Algorithm to HighLife, whose replicators copy themselves, or Day "
        "and Night, where live and dead are mirror images; press Randomize after each.",
    )
    cells = e.add(
        "cellularautomata",
        0,
        0,
        controls={
            "autoRun": 1.0,
            "stepsPerFrame": 1.0,
            "initThreshold": 0.8,
            "trailDecay": 0.12,
            "aliveInput": "#7af7ff",
            "deadInput": "#16052a",
        },
        options={"algorithm": "brians_brain", "gridScale": "10"},
    )
    near = e.add("zoom", 0, 0, controls={"zoom": 2.0})
    e.cable(cells, "output", near, "input")
    e.cable(near, "output", e.output(0, 0), "input")
    return e


def slime_networks(new):
    e = new("Slime mold networks", "Thousands of agents following each other's scent into veins")
    e.note(
        40,
        40,
        "Thousands of agents wander the grid, each sniffing three ways ahead and turning toward "
        "the strongest scent, then dropping their own, so trails become veins and veins a "
        "network. This is preset 7 of the nine numbered buttons on the node; try the others, "
        "or Randomize for sensing angles nobody chose.",
    )
    mold = e.add(
        "slimemold",
        0,
        0,
        controls={
            "sensorAngle": 12.9,
            "rotationAngle": 156.4,
            "sensorOffset": 15.0,
            "trailColor": "#ffcf6e",
            "bgColor": "#0d0716",
        },
        options={"mode": "attract", "gridScale": "16"},
    )
    near = e.add("zoom", 0, 0, controls={"zoom": 1.5})
    e.cable(mold, "color", near, "input")
    e.cable(near, "output", e.output(0, 0), "input")
    return e


def slime_strands(new):
    e = new("Slime mold strands", "Preset 9: agents that sense close by, drawn into long wandering threads")
    e.note(
        40,
        40,
        "The same agents at preset 9: they sense only 4 cells ahead, so instead of a network "
        "they pull each other into long threads that wander and loop. Sense Angle, Turn Angle "
        "and Sense Dist are the whole of a preset, with Mode; set Mode to Repel and the "
        "threads shatter into cells. Decay is how long the scent lingers.",
    )
    mold = e.add(
        "slimemold",
        0,
        0,
        controls={
            "sensorAngle": 35.0,
            "rotationAngle": 70.0,
            "sensorOffset": 4.0,
            "trailColor": "#8ef0c8",
            "bgColor": "#06121a",
        },
        options={"mode": "attract", "gridScale": "16"},
    )
    near = e.add("zoom", 0, 0, controls={"zoom": 1.5})
    e.cable(mold, "color", near, "input")
    e.cable(near, "output", e.output(0, 0), "input")
    return e


def brick_game(new):
    e = new("A sequencer plays the brick game", "Two sequencer lanes hold the paddle left and right")
    e.note(
        40,
        40,
        "Brick Game is played with events. Lane 1 of the Step Sequencer holds Paddle Left for "
        "half a bar and Lane 2 holds Paddle Right for the other half, so the paddle sweeps "
        "back and forth, catching the ball now and then, and Auto Reset starts a new game when "
        "it misses. Each broken brick fires Brick Broken, which picks a new color; click cells "
        "to play better, or turn on Auto Play.",
    )
    master = e.master(0, 0, length=4.0)
    seq = e.add(
        "stepsequencer",
        0,
        0,
        controls={"gateLength": 1.0},
        values={"pattern": {"Cells": ["xxxxxxxx........", "........xxxxxxxx", "................", "................"]}},
    )
    e.cable(master, "cycles", seq, "clock")
    game = e.add(
        "brickgame",
        0,
        0,
        controls={"autoReset": 1.0, "paddleWidth": 0.4, "paddleSpeed": 0.02, "backgroundColor": "#000000"},
    )
    e.cable(seq, "lane1", game, "leftPaddle")
    e.cable(seq, "lane2", game, "rightPaddle")
    flash = e.add("triggeredcolor", 0, 0)
    e.cable(game, "brickBroken", flash, "trigger")
    tint = e.add("colorize", 0, 0, controls={"amount": 1.0, "preserveLuminance": 0.0})
    e.cable(game, "color", tint, "input")
    e.cable(flash, "color", tint, "tint")
    e.cable(tint, "output", e.output(0, 0), "input")
    return e


def recolored(new):
    e = new("A simulation recolored and mirrored", "A slime mold's density through a turning palette, folded six ways")
    e.note(
        40,
        40,
        "A simulation's picture is a picture like any other. The slime mold's Density is a "
        "number per pixel, so a Cosine Gradient colors it, its palette turned once a loop by "
        "the gear, and a Kaleidoscope folds the whole thing into six. Try Mirror in place of "
        "the Kaleidoscope, or another preset on the mold.",
    )
    master = e.master(0, 0, length=8.0)
    mold = e.add(
        "slimemold",
        0,
        0,
        controls={"sensorAngle": 35.0, "rotationAngle": 70.0, "sensorOffset": 4.0},
        options={"mode": "attract", "gridScale": "16"},
    )
    grad = e.add("cosinegradient", 0, 0, controls=SUNSET)
    e.cable(mold, "value", grad, "t")
    e.cable(master, "cycles", grad, "clock")
    kal = e.add("kaleidoscope", 0, 0, controls={"segments": 6.0, "zoom": 1.2})
    e.cable(grad, "output", kal, "input")
    e.cable(kal, "output", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        life(new),
        brians_brain(new),
        slime_networks(new),
        slime_strands(new),
        brick_game(new),
        recolored(new),
    ]
