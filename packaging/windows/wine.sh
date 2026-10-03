#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Run the folder build.sh assembled, under Wine, in the prefix GStreamer was installed into.
# Arguments go to supersilvia.exe: `--check` is the quickest proof the carried GStreamer loads.
#
#   packaging/windows/wine.sh [ARGS]
#
# Wine draws Direct3D 12 with vkd3d over the machine's own Vulkan driver, so the environment
# that picks a GPU on Linux picks it here too: SUPERSILVIA_ADAPTER is the app's own, and
# VK_DRIVER_FILES limits which drivers Vulkan loads at all. WINEDEBUG is quiet unless it is set.
#
# vkd3d takes its adapters from Wine's own DXGI and from no other, and a Wine that puts DXVK's
# DXGI in its place, as Fedora's does, leaves Direct3D 12 with no adapter at all. Then Wine's own
# is put in the prefix as a native library — the same file without the mark that makes Wine
# load DXVK's by its name — and preferred, and wined3d under it lists the GPU through Vulkan,
# which VK_DRIVER_FILES holds to the driver it names and which needs no window.
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
wine_dxgi=$(find /usr/lib64 /usr/lib -path '*/x86_64-windows/wine-dxgi.dll' -print -quit \
  2>/dev/null || true)
if [[ -n $wine_dxgi ]]; then
  native=$cache/dxgi-native.dll
  cp "$wine_dxgi" "$native"
  # The mark is "Wine builtin DLL" at byte 64, in the DOS stub.
  printf 'Wine native DLL\0' | dd of="$native" bs=1 seek=64 conv=notrunc status=none
  system=$WINEPREFIX/drive_c/windows/system32/dxgi.dll
  cmp -s "$native" "$system" || cp "$native" "$system"
  export WINEDLLOVERRIDES="dxgi=n${WINEDLLOVERRIDES:+;$WINEDLLOVERRIDES}"
  export WINE_D3D_CONFIG=${WINE_D3D_CONFIG:-renderer=vulkan}
fi
exec wine "$exe" "$@"
