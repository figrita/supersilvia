#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Run the folder build.sh assembled, under Wine, in the prefix GStreamer was installed into.
# Arguments go to supersilvia.exe: `--check` is the quickest proof the carried GStreamer loads.
#
#   packaging/windows/wine.sh [ARGS]
#
# Wine draws through the machine's own Vulkan driver, so the environment that picks a GPU on
# Linux picks it here too: SUPERSILVIA_ADAPTER is the app's own, and VK_DRIVER_FILES limits
# which drivers Vulkan loads at all. WINEDEBUG is quiet unless it is set.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cache=${SUPERSILVIA_PACKAGING_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/supersilvia-windows}
exe=$root/dist/windows/supersilvia/bin/supersilvia.exe
[[ -f $exe ]] || { echo "windows/wine: no $exe; run packaging/windows/build.sh" >&2; exit 1; }

export WINEPREFIX=$cache/wine WINEDEBUG=${WINEDEBUG:--all}
# Nothing from this machine's own GStreamer reaches the carried one.
unset GST_PLUGIN_PATH GST_PLUGIN_PATH_1_0 GST_PLUGIN_SYSTEM_PATH GST_PLUGIN_SYSTEM_PATH_1_0 \
      GST_PLUGIN_SCANNER GST_PLUGIN_SCANNER_1_0 GST_REGISTRY GST_REGISTRY_1_0
exec wine "$exe" "$@"
