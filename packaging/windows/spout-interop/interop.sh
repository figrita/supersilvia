#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Spout between processes on one Windows machine: tests/syphon.rs's Windows half, then
# supersilvia's peer (examples/spout_peer.rs) and the Spout SDK's (spout_peer.cpp here) sending
# the peer contract's pattern to each other in every direction. Run by
# .github/workflows/spout.yml, and by hand in Git Bash:
#
#   packaging/windows/spout-interop/interop.sh <bin> <spout_peer_sdk.exe>
#
# <bin> is the Windows folder's bin/ with spout_peer.exe and syphon_test.exe copied in beside
# GStreamer's DLLs. Every case runs, each process under a time limit, and the script exits 1
# when any case failed: a receiver that saw no match, or a sender that did not send.
#
# A shared texture opens only on its sender's adapter, so the SDK's peer is handed the adapter
# supersilvia's peer prints on its `adapter-luid <hex>` line, through SPOUT_PEER_LUID: in a
# case with one of each, supersilvia's starts first, its receiver before the SDK's sender. The
# SDK's frame counting, a registry value every Spout app reads at start, is set by the SDK's
# peer in each case, off or on through SPOUT_PEER_FRAMECOUNT.
#
# Environment, all optional:
#   SPOUT_WIDTH, SPOUT_HEIGHT  the pattern's size; 301 by 167, so a row pitch taken for the
#                              width shows
#   SPOUT_SEND_SECONDS         how long a sender sends; 25
#   SPOUT_RECEIVE_SECONDS      how long a receiver waits for a match; 20
#   SPOUT_LOGS                 where each process's output is kept; a temporary folder
set -euo pipefail

die() { echo "spout-interop: $*" >&2; exit 1; }

(($# == 2)) || die "usage: interop.sh <bin> <spout_peer_sdk.exe>"
bin=$1
sdk=$2
ours=$bin/spout_peer.exe
test=$bin/syphon_test.exe
for file in "$ours" "$test" "$sdk"; do
  [[ -f $file ]] || die "no $file"
done
command -v timeout >/dev/null || die "timeout not found"

width=${SPOUT_WIDTH:-301}
height=${SPOUT_HEIGHT:-167}
send_seconds=${SPOUT_SEND_SECONDS:-25}
receive_seconds=${SPOUT_RECEIVE_SECONDS:-20}
logs=${SPOUT_LOGS:-$(mktemp -d)}
mkdir -p "$logs"

results=()
failed=0
cases=0
# The LUID supersilvia's sender printed in the first case, for the SDK's own pair.
luid=

# The LUID on a log's `adapter-luid` line, with or without 0x; with a `pid`, waiting up to 30 s
# while that process, which writes the log, runs. Nothing when there is none.
luid_of() {
  local log=$1 pid=${2:-} found tries=0
  while :; do
    found=$(sed -n 's/.*adapter-luid \(0[xX]\)\{0,1\}\([0-9a-fA-F]\{1,16\}\).*/\2/p' "$log" | head -n 1)
    if [[ -n $found ]]; then
      echo "$found"
      return 0
    fi
    [[ -n $pid ]] && ((tries < 60)) && kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.5
    tries=$((tries + 1))
  done
}

# How a process ended, in words.
ended() {
  case $1 in
    0) echo "exited 0" ;;
    124) echo "was stopped at its time limit" ;;
    *) echo "exited $1" ;;
  esac
}

# Starts one peer in the background, its output in `log`; sets `started`. Reads run_case's
# `name`, `case_luid` and `frame_count`.
start() {
  local role=$1 kind=$2 log=$3 seconds
  if [[ $role == send ]]; then seconds=$send_seconds; else seconds=$receive_seconds; fi
  if [[ $kind == ours ]]; then
    timeout "$((seconds + 30))" "$ours" "$role" "$name" "$width" "$height" "$seconds" \
      >"$log" 2>&1 &
  else
    SPOUT_PEER_LUID=$case_luid SPOUT_PEER_FRAMECOUNT=$frame_count \
      timeout "$((seconds + 30))" "$sdk" "$role" "$name" "$width" "$height" "$seconds" \
      >"$log" 2>&1 &
  fi
  started=$!
}

# One sender and one receiver, `ours` or `sdk` each, under a sender name of the case's own.
run_case() {
  local label=$1 from=$2 to=$3 frame_count=${4:-}
  cases=$((cases + 1))
  local name="supersilvia ci $cases"
  local send_log=$logs/$cases-send.log receive_log=$logs/$cases-receive.log
  local case_luid=$luid started send_pid receive_pid send_code receive_code found

  echo "::group::$cases. $label"
  echo "== \"$name\", ${width}x$height, sender $from, receiver $to${frame_count:+, SPOUT_PEER_FRAMECOUNT=$frame_count}"
  if [[ $from == sdk && $to == ours ]]; then
    start receive ours "$receive_log"
    receive_pid=$started
    found=$(luid_of "$receive_log" "$receive_pid")
    [[ -z $found ]] || case_luid=$found
    start send sdk "$send_log"
    send_pid=$started
  else
    start send "$from" "$send_log"
    send_pid=$started
    if [[ $from == ours && $to == sdk ]]; then
      found=$(luid_of "$send_log" "$send_pid")
      [[ -z $found ]] || case_luid=$found
    fi
    start receive "$to" "$receive_log"
    receive_pid=$started
  fi
  if [[ ($from == sdk || $to == sdk) && -n $case_luid ]]; then
    echo "== the SDK's peer on the adapter $case_luid, SPOUT_PEER_LUID"
  elif [[ $from == sdk || $to == sdk ]]; then
    echo "== the SDK's peer on its default adapter: no adapter-luid line from supersilvia's"
  fi
  if wait "$receive_pid"; then receive_code=0; else receive_code=$?; fi
  if wait "$send_pid"; then send_code=0; else send_code=$?; fi
  [[ -n $luid || $from != ours ]] || luid=$(luid_of "$send_log")

  echo "-- sender ($from)"
  sed 's/^/   /' "$send_log"
  echo "-- receiver ($to)"
  sed 's/^/   /' "$receive_log"
  echo "::endgroup::"

  local verdict="PASS"
  if ((receive_code != 0 || send_code != 0)); then
    verdict="FAIL"
    failed=1
  fi
  results+=("$verdict  $cases. $label: the receiver $(ended "$receive_code"), the sender $(ended "$send_code")")
  echo "${results[-1]}"
}

cases=$((cases + 1))
echo "::group::$cases. tests/syphon.rs"
if timeout 300 "$test"; then test_code=0; else test_code=$?; fi
echo "::endgroup::"
if ((test_code == 0)); then
  results+=("PASS  $cases. tests/syphon.rs $(ended 0)")
else
  results+=("FAIL  $cases. tests/syphon.rs $(ended "$test_code")")
  failed=1
fi
echo "${results[-1]}"

run_case "supersilvia -> supersilvia" ours ours
run_case "Spout SDK -> Spout SDK, the control, frame counting off" sdk sdk 0
run_case "supersilvia -> Spout SDK, frame counting off" ours sdk 0
run_case "supersilvia -> Spout SDK, frame counting on" ours sdk 1
run_case "Spout SDK -> supersilvia, frame counting off" sdk ours 0
run_case "Spout SDK -> supersilvia, frame counting on" sdk ours 1

echo
echo "Spout interop, ${width}x$height; logs in $logs"
printf '%s\n' "${results[@]}"
if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
  {
    echo "### Spout interop, ${width}x$height"
    echo
    printf -- '- %s\n' "${results[@]}"
  } >>"$GITHUB_STEP_SUMMARY"
fi
exit "$failed"
