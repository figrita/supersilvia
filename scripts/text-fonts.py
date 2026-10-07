#!/usr/bin/env python3
"""Regenerate the vendored text face in `assets/fonts/`: Space Grotesk Regular and SemiBold.

Space Grotesk ships as one variable face. egui draws static faces only, so each weight is an
instance of it at a fixed `wght`.

Two things are done to each instance besides the cut:

- **Its digits are the tabular ones.** The default figures are proportional — a `1` is two
  thirds the width of a `0` — so a live number would shuffle sideways every time it changed.
  The cmap is pointed at the glyphs the face's own `tnum` feature substitutes, which is what
  turning `tnum` on would do in a shaper, and egui has no shaper.
- **It is cut to text.** Basic Latin, Latin-1, Latin Extended-A and the punctuation a sentence
  uses, and the figure space a padded number is padded with. No arrows, shapes or symbols:
  those are icons, and they keep coming from the faces that draw them today, since this face
  goes first in the stack and would otherwise restyle every one it has.

    pip install fonttools && scripts/text-fonts.py path/to/SpaceGrotesk[wght].ttf

The variable face is Google Fonts' `ofl/spacegrotesk/SpaceGrotesk[wght].ttf`.
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

RANGES = "U+0020-007E,U+00A0-017F,U+2007,U+2009,U+2010-2027,U+2030-203A,U+20AC,U+2122,U+2212"
FONTS = Path(__file__).resolve().parent.parent / "assets/fonts"
WEIGHTS = {"SpaceGrotesk-Regular.ttf": 400, "SpaceGrotesk-SemiBold.ttf": 600}


def tabular_digits(font: TTFont) -> None:
    """Point each digit at the glyph the face's `tnum` feature puts in its place."""
    gsub = font["GSUB"].table
    swap = {}
    for record in gsub.FeatureList.FeatureRecord:
        if record.FeatureTag != "tnum":
            continue
        for index in record.Feature.LookupListIndex:
            for table in gsub.LookupList.Lookup[index].SubTable:
                swap.update(getattr(table, "mapping", {}) or {})
    for table in font["cmap"].tables:
        for code in range(0x30, 0x3A):
            glyph = table.cmap.get(code)
            if glyph in swap:
                table.cmap[code] = swap[glyph]


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    source = sys.argv[1]
    for dest, weight in WEIGHTS.items():
        font = instancer.instantiateVariableFont(TTFont(source), {"wght": weight})
        tabular_digits(font)
        options = subset.Options()
        options.hinting = False
        options.layout_features = ["kern"]
        options.name_IDs = ["*"]
        sub = subset.Subsetter(options)
        sub.populate(unicodes=subset.parse_unicodes(RANGES))
        sub.subset(font)
        font.save(FONTS / dest)
        print(f"{dest}: {(FONTS / dest).stat().st_size // 1024} KB from {source}")
