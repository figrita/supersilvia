# SPDX-License-Identifier: AGPL-3.0-or-later
#
# What every Windows cross-compile builds against, sourced by build.sh and by anything that
# builds beside it: GStreamer's official MSVC release, installed into a Wine prefix of its own
# the first time, and the environment cargo xwin builds against it in. From the repository's
# root:
#
#   source packaging/windows/env.sh
#   cargo xwin build --target "$TARGET" --profile release --example spout_peer
#
# It sets GST_VERSION, GST_URL, TARGET, cache and gst, and exports WINEPREFIX, WINEDEBUG,
# pkg-config's variables, which name the release's .pc files and nothing of this machine's own
# GStreamer, and cargo-xwin's. SUPERSILVIA_PACKAGING_CACHE is read as build.sh says. A caller's
# own `die` and `say` are used where it has them.

declare -F die >/dev/null || die() { echo "windows/env: $*" >&2; exit 1; }
declare -F say >/dev/null || say() { echo "== $*"; }

# ------------------------------------------------------------------------------ inputs

GST_VERSION=1.28.7
GST_URL=https://gstreamer.freedesktop.org/data/pkg/windows/$GST_VERSION/msvc
GST_INSTALLER=gstreamer-1.0-msvc-x86_64-$GST_VERSION.exe
GST_INSTALLER_SHA256=032fc6062b8539838fc8da22589cb9b24c5d820baa7f8cc160af9ea08395badf
TARGET=x86_64-pc-windows-msvc

cache=${SUPERSILVIA_PACKAGING_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/supersilvia-windows}

for tool in cargo cargo-xwin curl sha256sum clang-cl lld-link llvm-rc; do
  command -v "$tool" >/dev/null || die "$tool not found"
done
rustup target list --installed 2>/dev/null | grep -qx "$TARGET" ||
  die "the $TARGET target is not installed: rustup target add $TARGET"

# ------------------------------------------------------------------------------ GStreamer

# Installed under Program Files, whose space no linker argument survives, so the build and
# everything after it read it through a link without one.
export WINEPREFIX=$cache/wine WINEDEBUG=${WINEDEBUG:--all}
gst=$cache/gst
if [[ ! -f $gst/lib/pkgconfig/gstreamer-1.0.pc ]]; then
  command -v wine >/dev/null || die "wine not found, and GStreamer's installer needs it"
  mkdir -p "$cache"
  if [[ ! -f $cache/$GST_INSTALLER ]]; then
    say "downloading GStreamer $GST_VERSION"
    curl -fL --progress-bar -o "$cache/$GST_INSTALLER.part" "$GST_URL/$GST_INSTALLER"
    mv "$cache/$GST_INSTALLER.part" "$cache/$GST_INSTALLER"
  fi
  echo "$GST_INSTALLER_SHA256  $cache/$GST_INSTALLER" | sha256sum -c --quiet ||
    die "$GST_INSTALLER does not match its checksum"
  say "installing GStreamer $GST_VERSION into $WINEPREFIX"
  # Mono and Gecko are never needed, and asking for them puts up a dialog.
  WINEDLLOVERRIDES="mscoree,mshtml=" wineboot -i >/dev/null 2>&1
  wine "$cache/$GST_INSTALLER" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TYPE=full
  wineserver -w
  installed="$WINEPREFIX/drive_c/Program Files/gstreamer/1.0/msvc_x86_64"
  [[ -f $installed/lib/pkgconfig/gstreamer-1.0.pc ]] || die "the installer left no $installed"
  ln -sfn "$installed" "$gst"
fi

# ------------------------------------------------------------------------------ cargo xwin

# The shader compiler wgpu links in needs Microsoft's ATL, which cargo-xwin leaves out unless
# asked, so its CRT and SDK are kept in a cache of their own here, with ATL.
export XWIN_ACCEPT_LICENSE=1 PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_LIBDIR=$gst/lib/pkgconfig
export PKG_CONFIG_PATH='' XWIN_INCLUDE_ATL=true XWIN_CACHE_DIR=$cache/xwin
