#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the Example Library: every category module in this folder, each example a workspace
file under `assets/examples/<NN-Category>/`, the folder replaced whole.

    scripts/examples/make.py [--only module,module] [--registry registry.json] [--preview folder]

A category module is `cat_<name>.py` holding `ORDER` (its place in the menu), `CATEGORY` (its
title there) and `examples(new)`, which returns its examples in menu order; `new(title, blurb)`
makes one (`lib.Example`). `--only` rewrites just those modules' folders. `--preview` also
writes a project holding every example written, one tab each, for the app or for

    cargo run --release --example loop_gifs -- <folder>

to render each tab's Output headless.
"""

import importlib
import json
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import lib  # noqa: E402

OUT = lib.REPO / "assets" / "examples"


def modules(only):
    names = sorted(p.stem for p in HERE.glob("cat_*.py"))
    if only:
        names = [n for n in names if n in only or n.removeprefix("cat_") in only]
    return [importlib.import_module(n) for n in names]


def main():
    args = sys.argv[1:]
    only, registry, preview = None, None, None
    while args:
        a = args.pop(0)
        if a == "--only":
            only = set(args.pop(0).split(","))
        elif a == "--registry":
            registry = args.pop(0)
        elif a == "--preview":
            preview = Path(args.pop(0)).expanduser()
        else:
            raise SystemExit(f"unknown argument {a}")
    reg = lib.load_registry(registry)
    if only is None:
        shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True, exist_ok=True)
    every = lib.Project(reg)
    total = 0
    for m in modules(only):
        folder = OUT / f"{m.ORDER:02d}-{m.CATEGORY}"
        shutil.rmtree(folder, ignore_errors=True)
        folder.mkdir()
        made = m.examples(lambda title, blurb: lib.Example(reg, title, blurb))
        titles = [e.title for e in made]
        assert len(set(titles)) == len(titles), f"{m.CATEGORY}: two examples share a title"
        for i, e in enumerate(made, 1):
            (folder / lib.file_name(i, e.title)).write_text(json.dumps(e.file(), indent=2) + "\n")
            if preview:
                every.adopt(e)
        total += len(made)
        print(f"{m.CATEGORY}: {len(made)}")
    if preview:
        every.write(preview)
    print(f"wrote {total} examples into {OUT.relative_to(lib.REPO)}")


if __name__ == "__main__":
    main()
