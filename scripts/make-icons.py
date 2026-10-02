#!/usr/bin/env python3
"""Render every icon from `assets/icon/*.svg`.

Two SVGs are the artwork — `supersilvia.svg`, the mark white with a black outline, and
`supersilvia-mark.svg`, the mark alone on nothing. Everything else in `assets/icon/` is
output: PNGs at the sizes a desktop, a browser and a store ask for, a Windows `.ico` and a
macOS `.icns`.

    scripts/make-icons.py

It takes no arguments and needs nothing installed — no rsvg, no ImageMagick, no Pillow.
The renderer below is a scanline filler over the two paths the logo is made of, which is
less code than a dependency and does not care what is on the machine. It is not a general
SVG renderer: `M`, `L`, `H`, `V`, `C`, `Z`, a solid fill, no strokes and no transforms is
exactly what the logo uses, and anything else in the file raises rather than draws wrong.

Re-run it when the logo changes, and commit what it writes: a build never runs it, and
`src/main.rs` bakes `supersilvia-256.png` into the binary with `include_bytes!`.
"""

import math
import re
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICON = ROOT / "assets/icon"

# Rows per pixel. Coverage across a row is exact — the filler knows where the span ends
# between two pixel centers — so the only sampling is vertical, and eight rows is past the
# point where a diagonal edge on a 16-square shows a step.
ROWS = 8


# ---------------------------------------------------------------- reading the artwork

def parse_svg(path):
    """`(view_box, [(rings, rgba)])` — one file's shapes, filled and stroked, in draw order.

    A stroke becomes fill: every segment a quad and every joint a disc, unioned by the
    nonzero rule. There is no stroking in the rasterizer, and no reason for one — a logo is
    a handful of segments and this is a dozen lines."""
    text = path.read_text()
    box = [float(n) for n in re.search(r'viewBox="([^"]+)"', text).group(1).replace(",", " ").split()]
    shapes = []
    for element in re.finditer(r"<(rect|path)\b([^>]*?)/?>", text):
        kind, attributes = element.group(1), element.group(2)
        if kind == "rect":
            color = fill_of(attributes)
            if color is None:
                continue
            x, y, w, h = (number_of(attributes, name) for name in ("x", "y", "width", "height"))
            shapes.append(([[(x, y), (x + w, y), (x + w, y + h), (x, y + h)]], color))
            continue
        rings = subpaths(re.search(r'\bd="([^"]+)"', attributes).group(1))
        fill = fill_of(attributes)
        if fill is not None:
            shapes.append(([points for points, _ in rings], fill))
        stroke = paint_of(attributes, "stroke")
        if stroke is not None:
            shapes.append((stroke_outline(rings, stroke_width_of(attributes)), stroke))
    return box, shapes


def fill_of(attributes):
    """The `rgba` a shape is filled with, or `None` for `fill="none"` and unfilled shapes."""
    return paint_of(attributes, "fill")


def stroke_width_of(attributes, fallback=1.0):
    match = re.search(r'\bstroke-width\s*[=:]\s*"?\s*([\d.]+)', attributes)
    return float(match.group(1)) if match else fallback


def paint_of(attributes, property_name):
    """One paint property — `fill` or `stroke` — as `rgba`, from an attribute or a `style`."""
    match = re.search(rf'\b{property_name}\s*[=:]\s*"?\s*([^";]+)', attributes)
    if not match:
        return None
    value = match.group(1).strip().strip('"')
    if value == "none":
        return None
    if value == "black":
        return (0, 0, 0, 255)
    if value == "white":
        return (255, 255, 255, 255)
    if value.startswith("#"):
        digits = value[1:]
        if len(digits) == 3:
            digits = "".join(d * 2 for d in digits)
        return (*(int(digits[i:i + 2], 16) for i in (0, 2, 4)), 255)
    channels = re.match(r"rgb\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)", value)
    if channels:
        return (*(round(float(c)) for c in channels.groups()), 255)
    raise ValueError(f"unhandled {property_name} {value!r}")


def number_of(attributes, name):
    return float(re.search(rf'\b{name}="([^"]+)"', attributes).group(1))


def subpaths(data):
    """A `d` attribute flattened to polylines, in the SVG's own units.

    Each is `(points, closed)`: a fill closes every ring regardless, but a stroke has to
    know, because an open line gets two caps where a loop gets none."""
    tokens = re.findall(r"[MmLlHhVvCcZz]|-?\d*\.?\d+(?:[eE]-?\d+)?", data)
    paths, points = [], []
    at = (0.0, 0.0)
    start = (0.0, 0.0)
    index = 0
    command = None
    while index < len(tokens):
        if re.match(r"[A-Za-z]", tokens[index]):
            command = tokens[index]
            index += 1
            if command in "Zz":
                if points:
                    paths.append((points, True))
                points, at = [], start
                continue
        # A repeated coordinate list continues the last command, except that a repeated
        # moveto is an implicit lineto — the one place where the command changes itself.
        elif command in "Mm":
            command = "L" if command == "M" else "l"

        def take(count):
            nonlocal index
            values = [float(v) for v in tokens[index:index + count]]
            index += count
            return values

        if command in "Mm":
            x, y = take(2)
            at = (x, y) if command == "M" else (at[0] + x, at[1] + y)
            if points:
                paths.append((points, False))
            points, start = [at], at
        elif command in "Ll":
            x, y = take(2)
            at = (x, y) if command == "L" else (at[0] + x, at[1] + y)
            points.append(at)
        elif command in "Hh":
            (x,) = take(1)
            at = (x if command == "H" else at[0] + x, at[1])
            points.append(at)
        elif command in "Vv":
            (y,) = take(1)
            at = (at[0], y if command == "V" else at[1] + y)
            points.append(at)
        elif command in "Cc":
            values = take(6)
            if command == "c":
                values = [v + at[i % 2] for i, v in enumerate(values)]
            control = [at, (values[0], values[1]), (values[2], values[3]), (values[4], values[5])]
            points.extend(flatten_cubic(control))
            at = control[3]
        else:
            raise ValueError(f"unhandled path command {command!r}")
    if points:
        paths.append((points, False))
    return paths


def flatten_cubic(control, per_unit=3.0):
    """A cubic as line ends, subdivided by the length of its control polygon."""
    span = sum(math.dist(control[i], control[i + 1]) for i in range(3))
    steps = max(8, int(span * per_unit / 4))
    (x0, y0), (x1, y1), (x2, y2), (x3, y3) = control
    out = []
    for step in range(1, steps + 1):
        t = step / steps
        u = 1 - t
        a, b, c, d = u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t
        out.append((a * x0 + b * x1 + c * x2 + d * x3, a * y0 + b * y1 + c * y2 + d * y3))
    return out


def stroke_outline(rings, width):
    """A stroked path as filled polygons: a quad per segment, a disc per joint and cap.

    Round joins and round caps, because every stroke the logo and the marks use is one
    weight and looks right with them. All polygons are wound the same way, so the nonzero
    rule unions them instead of punching the overlaps out.
    """
    half = width / 2
    polygons = []
    for points, closed in rings:
        walk = points + [points[0]] if closed else points
        for start, end in zip(walk, walk[1:]):
            run = (end[0] - start[0], end[1] - start[1])
            length = math.hypot(*run)
            if length == 0:
                continue
            across = (-run[1] / length * half, run[0] / length * half)
            polygons.append([(start[0] + across[0], start[1] + across[1]),
                             (end[0] + across[0], end[1] + across[1]),
                             (end[0] - across[0], end[1] - across[1]),
                             (start[0] - across[0], start[1] - across[1])])
        for point in walk:
            polygons.append([(point[0] + half * math.cos(math.tau * i / 16),
                              point[1] + half * math.sin(math.tau * i / 16)) for i in range(16)])
    return [wound(polygon) for polygon in polygons]


def wound(polygon):
    """The same polygon, always in the same direction. See `stroke_outline`."""
    area = sum(polygon[i][0] * polygon[i - 1][1] - polygon[i - 1][0] * polygon[i][1]
               for i in range(len(polygon)))
    return polygon if area >= 0 else polygon[::-1]


# ------------------------------------------------------------------------- rasterizing

class Image:
    """A straight-alpha RGBA canvas, composited into and then handed to `png`."""

    def __init__(self, width, height):
        self.width, self.height = width, height
        self.pixels = [0.0] * (width * height * 4)

    def fill(self, shapes, box, rect):
        """Draw `shapes` with their `box` fitted, aspect kept, inside the pixel `rect`."""
        x, y, width, height = rect
        scale = min(width / box[2], height / box[3])
        offset = (
            x + (width - box[2] * scale) / 2 - box[0] * scale,
            y + (height - box[3] * scale) / 2 - box[1] * scale,
        )
        for polylines, color in shapes:
            placed = [[(px * scale + offset[0], py * scale + offset[1]) for px, py in line]
                      for line in polylines]
            self.compose(coverage(placed, self.width, self.height), color)

    def compose(self, coverage_of, color):
        source = [c / 255 for c in color[:3]]
        alpha_of = color[3] / 255
        for index, covered in enumerate(coverage_of):
            if covered <= 0.0:
                continue
            alpha = min(covered, 1.0) * alpha_of
            at = index * 4
            under = self.pixels[at + 3]
            over = alpha + under * (1 - alpha)
            for channel in range(3):
                mixed = source[channel] * alpha + self.pixels[at + channel] * under * (1 - alpha)
                self.pixels[at + channel] = mixed / over if over else 0.0
            self.pixels[at + 3] = over

    def rgba(self):
        return bytes(min(255, max(0, round(v * 255))) for v in self.pixels)


def coverage(polylines, width, height):
    """Per-pixel coverage of the filled polygons, nonzero winding rule."""
    edges = []
    for line in polylines:
        for index in range(len(line)):
            (x0, y0), (x1, y1) = line[index], line[(index + 1) % len(line)]
            if y0 == y1:
                continue
            winding = 1 if y1 > y0 else -1
            if winding < 0:
                (x0, y0), (x1, y1) = (x1, y1), (x0, y0)
            edges.append((y0, y1, x0, (x1 - x0) / (y1 - y0), winding))
    covered = [0.0] * (width * height)
    share = 1.0 / ROWS
    for row in range(height * ROWS):
        y = (row + 0.5) / ROWS
        crossings = sorted((x + slope * (y - y0), winding)
                           for y0, y1, x, slope, winding in edges if y0 <= y < y1)
        if not crossings:
            continue
        base = (row // ROWS) * width
        wind = 0
        span_start = 0.0
        for x, winding in crossings:
            if wind == 0:
                span_start = x
            wind += winding
            if wind != 0:
                continue
            left, right = max(span_start, 0.0), min(x, float(width))
            if right <= left:
                continue
            first, last = int(left), min(int(math.ceil(right)) - 1, width - 1)
            if first == last:
                covered[base + first] += (right - left) * share
                continue
            covered[base + first] += (first + 1 - left) * share
            for pixel in range(first + 1, last):
                covered[base + pixel] += share
            covered[base + last] += (right - last) * share
    return covered


# ------------------------------------------------------------------- container formats

def png(image):
    raw = bytearray()
    pixels = image.rgba()
    stride = image.width * 4
    for row in range(image.height):
        raw.append(0)  # filter: none. The logo is flat colour; filtering buys nothing.
        raw += pixels[row * stride:(row + 1) * stride]

    def chunk(kind, payload):
        return (struct.pack(">I", len(payload)) + kind + payload
                + struct.pack(">I", zlib.crc32(kind + payload)))

    header = struct.pack(">IIBBBBB", image.width, image.height, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def ico(images):
    """A Windows icon. BGRA bottom-up below 256, where the oldest readers want a DIB, and
    PNG at 256, which is the size the DIB form was never meant to carry."""
    entries, blobs, offset = [], [], 6 + 16 * len(images)
    for image in images:
        if image.width >= 256:
            blob = png(image)
        else:
            head = struct.pack("<IiiHHIIiiII", 40, image.width, image.height * 2, 1, 32,
                               0, image.width * image.height * 4, 0, 0, 0, 0)
            pixels = image.rgba()
            rows = []
            for row in reversed(range(image.height)):
                line = pixels[row * image.width * 4:(row + 1) * image.width * 4]
                rows.append(bytes(b for pixel in range(image.width)
                                  for b in (line[pixel * 4 + 2], line[pixel * 4 + 1],
                                            line[pixel * 4], line[pixel * 4 + 3])))
            mask_stride = ((image.width + 31) // 32) * 4
            blob = head + b"".join(rows) + bytes(mask_stride * image.height)
        entries.append(struct.pack("<BBBBHHII", image.width % 256, image.height % 256, 0, 0,
                                   1, 32, len(blob), offset))
        blobs.append(blob)
        offset += len(blob)
    return struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries) + b"".join(blobs)


# The macOS types, by the pixel size each one means. `ic11`-`ic14` are the @2x members of
# the 16, 32, 128 and 256 slots; a reader picks whichever it needs by name.
ICNS_TYPES = [(b"ic11", 32), (b"ic12", 64), (b"ic07", 128), (b"ic13", 256),
              (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)]


def icns(by_size):
    body = b"".join(kind + struct.pack(">I", len(by_size[size]) + 8) + by_size[size]
                    for kind, size in ICNS_TYPES)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


# ------------------------------------------------------------------------------ outputs

def square(shapes, box, size):
    image = Image(size, size)
    image.fill(shapes, box, (0, 0, size, size))
    return image


def plate(shapes, box, width, height, scale=0.72, field=(0, 0, 0, 255)):
    """The mark centred on a field — a capsule, a cover, a banner. `field=None` leaves it
    transparent, for whatever composites the logo over its own art."""
    image = Image(width, height)
    if field:
        image.compose([1.0] * (width * height), field)
    side = min(width, height) * scale
    image.fill(shapes, box, ((width - side) / 2, (height - side) / 2, side, side))
    return image


ICON_SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]

# A picture window is not the app, and a desktop that can only tell windows apart by their
# icon should be able to tell these apart: a pop-out carries the pop-out mark and a
# fullscreen picture the fullscreen mark, the same two marks that opened them. They are
# window icons only — no store ever sees one — so they stop at 256.
WINDOW_ICONS = ["supersilvia-popout", "supersilvia-fullscreen"]
WINDOW_ICON_SIZES = [16, 24, 32, 48, 64, 128, 256]


def main():
    box, shapes = parse_svg(ICON / "supersilvia.svg")
    mark_box, mark = parse_svg(ICON / "supersilvia-mark.svg")

    written = []

    def write(path, payload):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        written.append((path.relative_to(ROOT), len(payload)))

    squares = {size: square(shapes, box, size) for size in ICON_SIZES}
    for size, image in squares.items():
        write(ICON / f"supersilvia-{size}.png", png(image))
    write(ICON / "supersilvia.ico", ico([squares[size] for size in ICO_SIZES]))
    write(ICON / "supersilvia.icns", icns({size: png(squares[size]) for size in (32, 64, 128, 256, 512, 1024)}))
    write(ICON / "supersilvia-mark-1024.png", png(plate(mark, mark_box, 1024, 1024, 1.0, None)))

    for name in WINDOW_ICONS:
        window_box, window_shapes = parse_svg(ICON / f"{name}.svg")
        for size in WINDOW_ICON_SIZES:
            write(ICON / f"{name}-{size}.png", png(square(window_shapes, window_box, size)))

    for path, size in written:
        print(f"{path}  {size / 1024:.1f} KB")


if __name__ == "__main__":
    main()
