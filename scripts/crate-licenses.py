#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Write the licence of every Rust crate compiled into supersilvia for one target, to stdout.

    scripts/crate-licenses.py --target x86_64-unknown-linux-gnu > packaging/linux/rust-crates.txt
    scripts/crate-licenses.py --target x86_64-unknown-linux-gnu --check packaging/linux/rust-crates.txt
    scripts/crate-licenses.py --target x86_64-pc-windows-msvc > packaging/windows/rust-crates.txt
    scripts/crate-licenses.py --target aarch64-apple-darwin      # build-app.sh, into the .app

The crates are the ones `cargo metadata` resolves for the target, followed from supersilvia
through normal dependencies only: a build or dev dependency is not in the binary. Each crate
is named with its version and its declared licence, and every LICENSE, COPYING, NOTICE or
COPYRIGHT file its package carries follows it, with the few files deeper in a package that
license data compiled in beside its code (`EXTRA`); a text shared word for word by several
crates is printed once, under all of their names.

Linux's output is committed as packaging/linux/rust-crates.txt and Windows' as
packaging/windows/rust-crates.txt, each compiled into its binary for Help > Licences;
`tests/notices.rs` runs `--check` against both, so a Cargo.lock that moves fails `cargo test`
until they are written again. The Mac's is written into the bundle by
build-app.sh. Needs nothing but python3 and cargo, never the network (`--offline`: every
package it reads is one a build of the target has already fetched), and runs from anywhere.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys

PREFIXES = ("license", "licence", "copying", "notice", "copyright", "unlicense")

# Licences a package carries below its root for data compiled into the binary rather than its
# code: egui's four default fonts, and the controller database gilrs embeds.
EXTRA = {
    "epaint_default_fonts": [
        "fonts/OFL.txt",
        "fonts/UFL.txt",
        "fonts/Hack-Regular.txt",
        "fonts/emoji-icon-font-mit-license.txt",
    ],
    "gilrs": ["SDL_GameControllerDB/LICENSE"],
}


def notices(target: str) -> str:
    root_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    meta = json.loads(
        subprocess.check_output(
            [
                os.environ.get("CARGO", "cargo"),
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--offline",
                "--filter-platform",
                target,
                "--manifest-path",
                os.path.join(root_dir, "Cargo.toml"),
            ]
        )
    )
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]

    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    seen.discard(root)

    texts: dict[str, tuple[str, list[str]]] = {}
    order: list[str] = []
    bare: list[str] = []
    crates = sorted((packages[p] for p in seen), key=lambda p: (p["name"], p["version"]))
    for p in crates:
        label = f'{p["name"]} {p["version"]} ({p.get("license") or "no licence declared"})'
        folder = os.path.dirname(p["manifest_path"])
        found = sorted(
            f
            for f in os.listdir(folder)
            if f.lower().startswith(PREFIXES) and os.path.isfile(os.path.join(folder, f))
        )
        found += [f for f in EXTRA.get(p["name"], []) if os.path.isfile(os.path.join(folder, f))]
        if not found:
            bare.append(label)
            continue
        for f in found:
            with open(os.path.join(folder, f), encoding="utf-8", errors="replace") as fh:
                text = fh.read().replace("\r\n", "\n").strip()
            key = hashlib.sha256(text.encode()).hexdigest()
            if key not in texts:
                texts[key] = (text, [])
                order.append(key)
            texts[key][1].append(f"{label}, {f}")

    title = f"Rust crates compiled into supersilvia for {target}"
    out = [
        f"{title}\n{'=' * len(title)}\n\n"
        f"{len(crates)} crates, each under the licence it declares. Where a crate's package\n"
        "carries its licence text, the text follows under every crate that carries it.\n\n"
    ]
    for p in crates:
        out.append(f'  {p["name"]} {p["version"]}: {p.get("license") or "see its package"}\n')
    if bare:
        out.append(
            "\nThese carry no licence file in their package; their declared licence applies,\n"
            "and its standard text is at https://spdx.org/licenses/:\n\n"
        )
        for label in bare:
            out.append(f"  {label}\n")
    for key in order:
        text, owners = texts[key]
        out.append("\n" + "-" * 78 + "\n")
        for owner in owners:
            out.append(f"{owner}\n")
        out.append("\n" + text + "\n")
    return "".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", required=True, help="the target triple the binary is for")
    parser.add_argument(
        "--check",
        metavar="FILE",
        help="compare with FILE instead of printing, and fail where they differ",
    )
    args = parser.parse_args()
    text = notices(args.target)
    if args.check is None:
        sys.stdout.write(text)
        return 0
    with open(args.check, encoding="utf-8") as fh:
        if fh.read() == text:
            return 0
    sys.stderr.write(
        f"{args.check} is not what Cargo.lock compiles for {args.target}. Write it again:\n"
        f"  scripts/crate-licenses.py --target {args.target} > {args.check}\n"
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
