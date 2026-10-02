#!/usr/bin/env python3
"""Regenerate the vendored symbol font in `assets/fonts/`.

`NotoSansMath-Subset.ttf` is Noto Sans Math cut down to the blocks a UI draws icons from.
The full face is 967 KB and 2,919 codepoints, almost all of it mathematical typesetting
this app will never render; the subset is 81 KB and 667.

Run it when an icon needs a symbol outside the ranges below — widen `RANGES`, re-run, and
`every_node_icon_has_a_glyph` in `tests/ui.rs` says whether it worked.

    pip install fonttools && scripts/subset-fonts.py path/to/NotoSansMath-Regular.ttf
"""

import subprocess
import sys
from pathlib import Path

from fontTools import subset

# Arrows, Mathematical Operators, Miscellaneous Technical, Geometric Shapes,
# Miscellaneous Symbols, Dingbats, Miscellaneous Symbols and Arrows.
RANGES = "U+2190-21FF,U+2200-22FF,U+2300-23FF,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+2B00-2BFF"
DEST = Path(__file__).resolve().parent.parent / "assets/fonts/NotoSansMath-Subset.ttf"


def find_source() -> str:
    if len(sys.argv) > 1:
        return sys.argv[1]
    listed = subprocess.run(
        ["fc-list", "--format", "%{file}\n"], capture_output=True, text=True, check=True
    ).stdout.split()
    for path in listed:
        if "NotoSansMath" in path:
            return path
    sys.exit("no NotoSansMath found; pass the path as an argument")


if __name__ == "__main__":
    source = find_source()
    subset.main(
        [source, f"--unicodes={RANGES}", f"--output-file={DEST}", "--no-hinting", "--desubroutinize"]
    )
    print(f"{DEST}: {DEST.stat().st_size // 1024} KB from {source}")
