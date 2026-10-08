# SPDX-License-Identifier: AGPL-3.0-or-later
"""Effects: nodes that read a picture around each pixel, or in cells of it, and give back
something new — a blur, a glow, a print, a painting, a broken signal."""

from tef_common import EMBER, NEON, RAINBOW, drifting_noise, times

ORDER = 5
CATEGORY = "Effects"


def julia(e, master, colors=RAINBOW, iterations=120):
    """A Julia Set whose constant circles a little once a loop of `master`, its escape count
    colored by a Cosine Gradient. The gradient is returned."""
    fractal = e.add("juliaset", 0, 0, controls={"zoom": 1.7, "iterations": iterations, "cReal": -0.75})
    e.cable(e.sway(0, 0, master, amplitude=0.06, offset=0.18), "output", fractal, "cImag")
    grad = e.add("cosinegradient", 0, 0, controls=colors)
    e.cable(fractal, "smooth", grad, "t")
    return grad


def blur(new):
    e = new("Blur", "Polka dots blurred, the blur swinging wide and back")
    e.note(
        40,
        40,
        "Blur averages the pixels around each one, so the dots go soft. Its Blur X and Blur Y "
        "are swung by the Oscillator together; set them apart, say Blur X at 20 and Blur Y at "
        "0, for a sideways smear. Size is how far each sample reaches: a bigger kernel is "
        "smoother and costs more.",
    )
    master = e.master(0, 0, length=4.0)
    dots = e.add(
        "polkadot",
        0,
        0,
        controls={"input": "#ff4d6d", "background": "#13093b", "frequency": 4, "radius": 0.3, "stagger": 0.5},
    )
    soft = e.add("blur", 0, 0, options={"size": "7x7"})
    swing = e.sway(0, 0, master, amplitude=6.0, offset=6.0)
    e.cable(swing, "output", soft, "blurX")
    e.cable(swing, "output", soft, "blurY")
    e.cable(dots, "color", soft, "input")
    e.cable(soft, "output", e.output(0, 0), "input")
    return e


def emboss(new):
    e = new("Emboss", "A drifting noise lit from a turning angle, so it stands up like hammered metal")
    e.note(
        40,
        40,
        "Emboss reads the picture's brightness as height and lights it from one side, so a flat "
        "noise stands up like beaten copper. The Master Gear turns Light Angle once a loop and "
        "the Colorize tints the gray. Turn Strength, or set Mode to Deboss to press it in.",
    )
    master = e.master(0, 0, length=6.0)
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 1.3, "octaves": 4.0, "contrast": 1.0},
        options={"type": "ridged", "repeat": "1"},
    )
    e.cable(master, "cycles", noise, "clock")
    relief = e.add("emboss", 0, 0, controls={"strength": 4.0})
    e.cable(master, "wrapped", relief, "angle")
    tint = e.add("colorize", 0, 0, controls={"tint": "#e8935a", "amount": 0.85, "preserveLuminance": 0.8})
    e.cable(noise, "color", relief, "input")
    e.cable(relief, "output", tint, "input")
    e.cable(tint, "output", e.output(0, 0), "input")
    return e


def bloom(new):
    e = new("Bloom", "A thin neon spiral made to glow")
    e.note(
        40,
        40,
        "Bloom gathers everything brighter than Threshold from around each pixel and adds it "
        "back, so the thin spiral glows like neon tubing. The Oscillator swings Intensity up and "
        "down. Turn Radius for a wider halo, or lower Threshold until the dark parts glow too.",
    )
    master = e.master(0, 0, length=4.0)
    arms = e.add(
        "spiral",
        0,
        0,
        controls={"turns": 3.0, "thickness": 0.03, "outerRadius": 0.95, "foreground": "#ff3cac", "background": "#07021a"},
        options={"type": "archimedean"},
    )
    e.cable(times(e, master, "wrapped", -1.0), "output", arms, "rotation")
    glow = e.add("bloom", 0, 0, controls={"threshold": 0.2, "radius": 0.12}, options={"quality": "high"})
    e.cable(e.sway(0, 0, master, amplitude=1.2, offset=1.8), "output", glow, "intensity")
    e.cable(arms, "color", glow, "input")
    e.cable(glow, "color", e.output(0, 0), "input")
    return e


def edges(new):
    e = new("Edge detection", "The outlines of a drifting noise, drawn as glowing contour lines")
    e.note(
        40,
        40,
        "Edge Detection keeps only where the picture changes and drops the flat parts, so the "
        "posterized noise becomes a map of glowing contour lines. Turn Sample Distance for "
        "thicker lines and Strength for brighter ones, or set Mode to Laplacian. Pull the "
        "Posterize out to see what soft edges give.",
    )
    master = e.master(0, 0, length=6.0)
    noise = drifting_noise(e, master, colors=NEON, scale=0.9, contrast=1.0)
    steps = e.add("posterize", 0, 0, controls={"levels": 6})
    lines = e.add("edgedetection", 0, 0, controls={"strength": 2.0, "sampleDistance": 1.5}, options={"mode": "sobel"})
    e.cable(noise, "output", steps, "input")
    e.cable(steps, "output", lines, "input")
    e.cable(lines, "output", e.output(0, 0), "input")
    return e


def halftone(new):
    e = new("Halftone", "A drifting rainbow noise printed in cyan, magenta, yellow and black dots")
    e.note(
        40,
        40,
        "Halftone prints the picture as dots, each one as big as the ink under it is strong. "
        "In CMYK mode every ink gets its own screen at its own angle, the way a press prints a "
        "magazine, so step back from the screen and the colors come back. Turn Dot Size, or set "
        "Mode to Mono for newsprint.",
    )
    master = e.master(0, 0, length=6.0)
    picture = drifting_noise(e, master, colors=RAINBOW, scale=1.0, contrast=1.2)
    dots = e.add("halftone", 0, 0, controls={"dotSize": 9.0, "smoothness": 0.15}, options={"mode": "cmyk"})
    e.cable(picture, "output", dots, "input")
    e.cable(dots, "color", e.output(0, 0), "input")
    return e


def mosaic(new):
    e = new("Mosaic", "A drifting noise set into stained glass")
    e.note(
        40,
        40,
        "Mosaic fills each cell with the color at its center and draws leading between them, so "
        "the drifting noise turns into stained glass. Voronoi cells scatter at random; try "
        "Shape Hexagon or Square. Turn Cell Freq for smaller pieces and Border Width for "
        "thicker leading.",
    )
    master = e.master(0, 0, length=6.0)
    noise = drifting_noise(e, master, colors=RAINBOW, scale=1.2, contrast=1.1)
    glass = e.add(
        "mosaic",
        0,
        0,
        controls={"cellSize": 14.0, "randomness": 0.9, "borderWidth": 0.12, "borderColor": "#120d0a"},
        options={"shape": "voronoi"},
    )
    e.cable(noise, "output", glass, "input")
    e.cable(glass, "color", e.output(0, 0), "input")
    return e


def dither(new):
    e = new("Dither", "A soft gradient broken into two-tone dots, like an old handheld screen")
    e.note(
        40,
        40,
        "Dither turns a smooth number into just two colors by checking it against a fixed "
        "pattern of thresholds, so gray becomes a pattern of dots. Here the number is the "
        "brightness of a drifting noise, and the two colors are an old handheld's greens. Turn "
        "Scale for bigger pixels, or try Mode Noise.",
    )
    master = e.master(0, 0, length=6.0)
    noise = e.add(
        "fractal",
        0,
        0,
        controls={"scale": 1.4, "octaves": 3.0, "contrast": 1.8},
        options={"repeat": "1"},
    )
    e.cable(master, "cycles", noise, "clock")
    dots = e.add("dither", 0, 0, controls={"light": "#c4f0c2", "dark": "#0f380f", "scale": 4.0}, options={"mode": "bayer4"})
    e.cable(noise, "value", dots, "value")
    e.cable(dots, "color", e.output(0, 0), "input")
    return e


def kuwahara(new):
    e = new("Kuwahara painting", "The Main Input painted in flat strokes that keep their edges")
    e.note(
        40,
        40,
        "Kuwahara averages whichever corner around a pixel is the smoothest, so flat areas melt "
        "into strokes of color while the edges stay sharp, and a camera picture looks painted. "
        "Choose a camera or a clip in the Main Input panel to feed it. Set Size larger for "
        "broader strokes, at more cost.",
    )
    source = e.add("maininput", 0, 0)
    paint = e.add("kuwahara", 0, 0, options={"size": "9x9"})
    rich = e.add("vibrance", 0, 0, controls={"amount": 0.4})
    e.cable(source, "frame", paint, "input")
    e.cable(paint, "color", rich, "input")
    e.cable(rich, "output", e.output(0, 0), "input")
    return e


def glitch(new):
    e = new("Glitch", "A word shoved into broken blocks, a new break eight times a loop")
    e.note(
        40,
        40,
        "Glitch cuts the picture into blocks, shoves some sideways and pulls red and blue apart "
        "along the shove, like a broken video signal. Its Step picks the pattern: the Master "
        "Gear's Wrapped, times 8 and rounded down, gives a new break eight times a loop. Turn "
        "Intensity and RGB Split, or type another word into the Text.",
    )
    master = e.master(0, 0, length=4.0)
    word = e.add(
        "text",
        0,
        0,
        controls={"textColor": "#ff4fd8", "backgroundColor": "#0b0f3a"},
        options={"font": "Impact", "size": "220"},
        values={"words": {"Text": "GLITCH"}},
    )
    beats = e.add("floor", 0, 0)
    e.cable(times(e, master, "wrapped", 8.0), "output", beats, "input")
    broken = e.add(
        "glitch",
        0,
        0,
        controls={"intensity": 0.35, "shift": 0.25, "blockSize": 0.08, "rgbSplit": 0.03},
        options={"direction": "horizontal"},
    )
    e.cable(beats, "output", broken, "step")
    e.cable(word, "output", broken, "input")
    e.cable(broken, "color", e.output(0, 0), "input")
    return e


def pixel_sort(new):
    e = new("Pixel sort", "A Julia set's columns sorted by brightness into a waterfall")
    e.note(
        40,
        40,
        "Pixel Sort cuts every column into chunks and sorts the pixels in each by brightness, "
        "so the fractal runs down the screen like wet paint. Chunks whose contrast is under "
        "Threshold are left alone. Turn Threshold, or set Sort By to Hue and Direction to "
        "Horizontal.",
    )
    master = e.master(0, 0, length=6.0)
    picture = julia(e, master, colors=EMBER, iterations=60)
    sorted_ = e.add("pixelsort", 0, 0, controls={"threshold": 0.2}, options={"direction": "vertical", "chunkSize": "32"})
    e.cable(picture, "output", sorted_, "input")
    e.cable(sorted_, "color", e.output(0, 0), "input")
    return e


def crt(new):
    e = new("Chromatic aberration and CRT", "Color bars through a cheap lens and onto an old television")
    e.note(
        40,
        40,
        "Chromatic Aberration pulls red, green and blue apart the way a cheap lens does, harder "
        "toward the edges. Camcorder CRT then bends the picture onto a curved tube with "
        "scanlines and phosphor glow. Turn Spread on the first, and Curvature, Scanlines and "
        "Glow on the second.",
    )
    master = e.master(0, 0, length=4.0)
    bars = e.add("prideflag", 0, 0, options={"flagType": "gilbert_baker"})
    turn = e.add("rotate", 0, 0, controls={"angle": 0.25})
    shake = e.add("shakycam", 0, 0, controls={"amplitude": 0.015})
    e.cable(master, "cycles", shake, "clock")
    e.cable(master, "cycles", shake, "clockY")
    lens = e.add("chromaticaberration", 0, 0, controls={"offset": 0.025}, options={"mode": "radial"})
    tube = e.add(
        "camcordercrt",
        0,
        0,
        controls={"curvature": 0.3, "aberration": 0.6, "scanlines": 0.6, "glow": 0.7, "brightness": 1.1, "vignette": 0.5},
    )
    e.cable(bars, "output", turn, "input")
    e.cable(turn, "output", shake, "input")
    e.cable(shake, "output", lens, "input")
    e.cable(lens, "output", tube, "input")
    e.cable(tube, "color", e.output(0, 0), "input")
    return e


def examples(new):
    return [
        blur(new),
        emboss(new),
        bloom(new),
        edges(new),
        halftone(new),
        mosaic(new),
        dither(new),
        kuwahara(new),
        glitch(new),
        pixel_sort(new),
        crt(new),
    ]
