#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Cross-compile supersilvia's Spout peer (examples/spout_peer.rs) and tests/syphon.rs for
# Windows, in the environment packaging/windows/build.sh builds the app in, and put them in the
# folder it assembled: dist/windows/supersilvia/bin/spout_peer.exe and syphon_test.exe, beside
# GStreamer's DLLs, with any library they import that the folder lacks.
#
#   packaging/windows/build.sh
#   packaging/windows/spout-interop/build-peers.sh
#
# SUPERSILVIA_PROFILE is the Cargo profile, as for build.sh; `dist` by default.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
cd "$root"

die() { echo "spout-interop/build-peers: $*" >&2; exit 1; }
say() { echo "== $*"; }

profile=${SUPERSILVIA_PROFILE:-dist}
bin=$root/dist/windows/supersilvia/bin
[[ -f $bin/supersilvia.exe ]] || die "no $bin/supersilvia.exe; run packaging/windows/build.sh"
for tool in llvm-objdump python3; do
  command -v "$tool" >/dev/null || die "$tool not found"
done

# shellcheck source=packaging/windows/env.sh
source "$root/packaging/windows/env.sh"

say "cargo xwin build --profile $profile --example spout_peer --test syphon"
messages=$(mktemp)
trap 'rm -f "$messages"' EXIT
cargo xwin build --target "$TARGET" --profile "$profile" --example spout_peer --test syphon \
  --message-format=json-render-diagnostics >"$messages"

# Each executable by the target that made it, from Cargo's own messages: a test's file name
# carries a hash.
executable() {
  python3 - "$messages" "$1" "$2" <<'EOF'
import json, sys
path, name, kind = sys.argv[1:]
for line in open(path, encoding="utf-8"):
    if not line.startswith("{"):
        continue
    message = json.loads(line)
    target = message.get("target") or {}
    if (message.get("reason") == "compiler-artifact" and target.get("name") == name
            and kind in target.get("kind", []) and message.get("executable")):
        print(message["executable"])
EOF
}
peer=$(executable spout_peer example | tail -n 1)
test=$(executable syphon test | tail -n 1)
[[ -f $peer ]] || die "Cargo named no executable for the example spout_peer"
[[ -f $test ]] || die "Cargo named no executable for the test syphon"
cp "$peer" "$bin/spout_peer.exe"
cp "$test" "$bin/syphon_test.exe"
say "$bin/spout_peer.exe and $bin/syphon_test.exe"

# Every DLL the two import that the folder lacks and the release has, followed to the end, as
# build.sh follows the app's.
queue=("$bin/spout_peer.exe" "$bin/syphon_test.exe")
while ((${#queue[@]})); do
  file=${queue[0]}
  queue=("${queue[@]:1}")
  while read -r dll; do
    [[ -z $(find "$bin" -maxdepth 1 -iname "$dll" -print -quit) ]] || continue
    found=$(find "$gst/bin" -maxdepth 1 -iname "$dll" -print -quit)
    [[ -n $found ]] || continue
    say "carrying $(basename "$found"), which $(basename "$file") imports"
    cp "$found" "$bin/"
    queue+=("$bin/$(basename "$found")")
  done < <(llvm-objdump -p "$file" | sed -n 's/^ *DLL Name: //p')
done
