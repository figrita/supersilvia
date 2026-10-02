#!/usr/bin/env bash
# Screenshot every node in silvia and in supersilvia, headless, for side-by-side comparison.
#
#   scripts/node-shots/shots.sh setup            # one-time: python venv, playwright, chromium
#   scripts/node-shots/shots.sh silvia OUT_DIR [slug ...]   # every silvia video node → OUT_DIR/<slug>.png + index.jsonl
#   scripts/node-shots/shots.sh supersilvia OUT_DIR   # every supersilvia node → OUT_DIR/<slug>.png + index.jsonl
#   scripts/node-shots/shots.sh both OUT_DIR     # OUT_DIR/silvia/ and OUT_DIR/supersilvia/
#
# SILVIA (default ~/silvia) is the silvia checkout; it is served on SILVIA_PORT (default 8080)
# for the run and stopped after. PPP (default 2) is supersilvia's pixels per point.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
SILVIA=${SILVIA:-$HOME/silvia}
SILVIA_PORT=${SILVIA_PORT:-8080}
VENV=${NODE_SHOTS_VENV:-$HERE/.venv}

setup() {
    python3 -m venv "$VENV"
    "$VENV/bin/pip" install -q playwright
    "$VENV/bin/playwright" install chromium-headless-shell
}

silvia() {
    local out=$1
    shift
    [ -x "$VENV/bin/python" ] || setup
    (cd "$SILVIA" && exec python3 -m http.server "$SILVIA_PORT" >/dev/null 2>&1) &
    local server=$!
    trap 'kill $server 2>/dev/null' RETURN
    sleep 1
    SILVIA="$SILVIA" SILVIA_URL="http://127.0.0.1:$SILVIA_PORT" "$VENV/bin/python" "$HERE/silvia_shots.py" "$out" "$@"
}

supersilvia() {
    local out=$1
    shift
    (cd "$REPO" && cargo build --release --example node_shots)
    "$REPO/target/release/examples/node_shots" "$out" "$@" 2>&1 | grep -v 'libEGL\|pci id' || true
}

case ${1:-} in
    setup) setup ;;
    silvia) shift; silvia "${1:?OUT_DIR}" "${@:2}" ;;
    supersilvia) shift; supersilvia "${1:?OUT_DIR}" "${@:2}" ;;
    both) shift; silvia "${1:?OUT_DIR}/silvia" "${@:2}"; supersilvia "$1/supersilvia" "${@:2}" ;;
    *) awk 'NR > 1 && /^#/' "$0"; exit 2 ;;
esac
