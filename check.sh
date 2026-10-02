#!/usr/bin/env bash
# The gate before every commit. All of these must pass.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
# The gate runs on the integrated GPU: the tests ask for it themselves, and this has the
# doctor check the adapter they use rather than the strongest one the app would take.
export SUPERSILVIA_ADAPTER="${SUPERSILVIA_ADAPTER:-integrated}"
scripts/doctor.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
echo "check.sh: all green"
