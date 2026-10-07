#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Name the frames of a panic's backtrace in a log from the AppImage, whose binary carries no
# symbols, with the symbols file build.sh kept beside it:
#
#   packaging/appimage/symbolize.sh supersilvia.log supersilvia-<version>-linux-x86_64.debug
#
# The log is printed as it is, every frame in the binary followed by the functions at its
# address, the inlined ones first, each with its file and line. A frame is written as its
# address in the running process, and each panic ends with `symbols: backtrace_on_panic at
# <address>` (src/main.rs); the symbols file says where that function is in the binary, and the
# difference between the two is where the binary was loaded. A frame outside the binary — in
# glibc, or a GStreamer plugin — is left as it is. The file has to be the one built with the
# binary that wrote the log: build.sh names it after the version, and `readelf -n` prints the
# build ID both carry.
set -euo pipefail

die() { echo "symbolize: $*" >&2; exit 1; }

[[ $# -eq 2 ]] || die "usage: symbolize.sh <log> <symbols file>"
log=$1
symbols=$2
[[ -r $log ]] || die "no log at $log"
[[ -r $symbols ]] || die "no symbols file at $symbols"
for tool in addr2line nm readelf; do
  command -v "$tool" >/dev/null || die "$tool not found: it is binutils'"
done

loaded=$(sed -n 's/^symbols: backtrace_on_panic at 0x\([0-9a-f]*\)$/\1/p' "$log" | tail -n 1)
[[ -n $loaded ]] || die "$log has no 'symbols:' line, so no panic was written in it"
linked=$(nm -C "$symbols" | awk '{ name = $0; sub(/^[^ ]+ [^ ]+ /, "", name) }
  !found && name == "supersilvia::backtrace_on_panic" { print $1; found = 1 }')
[[ -n $linked ]] || die "$symbols has no supersilvia::backtrace_on_panic: is it supersilvia's?"
bias=$((0x$loaded - 0x$linked))
# The end of the binary's last loaded segment, past which an address is another file's.
end=0
while read -r vaddr size; do
  if ((vaddr + size > end)); then end=$((vaddr + size)); fi
done < <(readelf -lW "$symbols" 2>/dev/null | awk '$1 == "LOAD" { print $3, $6 }')

while IFS= read -r line; do
  printf '%s\n' "$line"
  [[ $line =~ ^\ *[0-9]+:\ +0x([0-9a-f]+)\ -\  ]] || continue
  address=$((0x${BASH_REMATCH[1]} - bias))
  ((address > 0 && address <= end)) || continue
  # A frame's address is where it returns to; one byte before is the call it made.
  addr2line -e "$symbols" -f -C -i -p "$(printf '0x%x' $((address - 1)))" | sed 's/^/          /'
done <"$log"
