#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build dist/windows/supersilvia, a folder that runs on 64-bit Windows with nothing installed
# but the Visual C++ runtime, and the .zip that carries it. Cross-compiled from Linux against
# Microsoft's own C runtime and SDK, which cargo-xwin downloads, and GStreamer's official
# MSVC release.
#
#   packaging/windows/build.sh
#   packaging/windows/wine.sh [ARGS]       the folder's supersilvia.exe, under Wine
#
# GStreamer's Windows release is an Inno Setup installer and nothing else, so it is downloaded
# once into a cache outside the repository and installed there, into a Wine prefix of its own:
# Wine is needed for that one step, and for wine.sh. The binary is built against that copy,
# and the folder carries its plugins, the libraries they and the binary reach and the plugin
# scanner, in bin/, lib/gstreamer-1.0/ and libexec/gstreamer-1.0/: the layout GStreamer's own
# relocation expects from the folder libgstreamer is in, so the app sets nothing at start-up.
#
# Needs: cargo-xwin (`cargo install cargo-xwin`), the x86_64-pc-windows-msvc target, clang-cl,
# lld, llvm-rc and llvm-objdump (Fedora: clang lld llvm), curl, zip, and Wine to install
# GStreamer the first time.
#
# Environment, all optional:
#   SUPERSILVIA_PACKAGING_CACHE where GStreamer is downloaded and installed, the Wine prefix
#                               it is installed in, and cargo-xwin's CRT and SDK with ATL;
#                               ~/.cache/supersilvia-windows by default.
#   SUPERSILVIA_PROFILE         the Cargo profile; `dist` by default. `release` builds in a
#                               fraction of the time, for trying the folder rather than
#                               handing it out.
#   SUPERSILVIA_FEATURES        Cargo features for the binary, e.g. `inspection`, so an
#                               inspection client can drive it when it is started with
#                               EGUI_INSPECTION=1. Never for a build that is handed out.
#
# Every plugin of the release is carried for now, rather than the ones the app's elements
# are in: the curated list the macOS bundle has is the next step here.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

die() { echo "windows/build: $*" >&2; exit 1; }
say() { echo "== $*"; }

# ------------------------------------------------------------------------------ inputs

GST_VERSION=1.28.7
GST_URL=https://gstreamer.freedesktop.org/data/pkg/windows/$GST_VERSION/msvc
GST_INSTALLER=gstreamer-1.0-msvc-x86_64-$GST_VERSION.exe
GST_INSTALLER_SHA256=032fc6062b8539838fc8da22589cb9b24c5d820baa7f8cc160af9ea08395badf
TARGET=x86_64-pc-windows-msvc

cache=${SUPERSILVIA_PACKAGING_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/supersilvia-windows}
profile=${SUPERSILVIA_PROFILE:-dist}
features=${SUPERSILVIA_FEATURES:-}

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[[ -n $version ]] || die "no version in Cargo.toml"
dist=$root/dist/windows
out=$dist/supersilvia
zip=$dist/supersilvia-$version-windows-x86_64.zip

for tool in cargo cargo-xwin curl sha256sum clang-cl lld-link llvm-rc llvm-objdump zip; do
  command -v "$tool" >/dev/null || die "$tool not found"
done
rustup target list --installed 2>/dev/null | grep -qx "$TARGET" ||
  die "the $TARGET target is not installed: rustup target add $TARGET"

# ------------------------------------------------------------------------------ GStreamer

# Installed under Program Files, whose space no linker argument survives, so the build and
# everything below read it through a link without one.
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

# ------------------------------------------------------------------------------ build

say "cargo xwin build --profile $profile, against GStreamer $GST_VERSION"
# pkg-config reads the release's .pc files and nothing of this machine's own GStreamer. The
# shader compiler wgpu links in needs Microsoft's ATL, which cargo-xwin leaves out unless asked,
# so its CRT and SDK are kept in a cache of their own here, with ATL.
export XWIN_ACCEPT_LICENSE=1 PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_LIBDIR=$gst/lib/pkgconfig
export PKG_CONFIG_PATH= XWIN_INCLUDE_ATL=true XWIN_CACHE_DIR=$cache/xwin
cargo xwin build --target "$TARGET" --profile "$profile" ${features:+--features "$features"}
exe=$root/target/$TARGET/$profile/supersilvia.exe
[[ -f $exe ]] || die "no $exe"

# ------------------------------------------------------------------------------ assemble

say "assembling $out"
rm -rf "$out" "$zip"
mkdir -p "$out/bin" "$out/lib/gstreamer-1.0" "$out/libexec/gstreamer-1.0" "$out/licenses"
cp "$exe" "$out/bin/"
cp "$gst"/lib/gstreamer-1.0/*.dll "$out/lib/gstreamer-1.0/"
cp "$gst/libexec/gstreamer-1.0/gst-plugin-scanner.exe" "$out/libexec/gstreamer-1.0/"
# fontconfig's configuration, which pango reads through it for the Text node's letters.
cp -r "$gst/etc" "$out/"

say "carrying the libraries they link"
# Every DLL any carried file imports that the release has, followed to the end; what the
# release does not have is Windows' own, or the Visual C++ runtime.
queue=("$out/bin/supersilvia.exe" "$out/libexec/gstreamer-1.0/gst-plugin-scanner.exe" \
       "$out"/lib/gstreamer-1.0/*.dll)
declare -A seen=()
while ((${#queue[@]})); do
  file=${queue[0]}
  queue=("${queue[@]:1}")
  while read -r dll; do
    key=${dll,,}
    [[ -n ${seen[$key]:-} ]] && continue
    seen[$key]=1
    # The release names a library in whatever case its linker wrote; the file is lower case
    # or not, and Windows does not care which.
    found=$(find "$gst/bin" -maxdepth 1 -iname "$dll" -print -quit)
    [[ -n $found ]] || continue
    cp "$found" "$out/bin/"
    queue+=("$out/bin/$(basename "$found")")
  done < <(llvm-objdump -p "$file" | sed -n 's/^ *DLL Name: //p')
done

say "writing licences"
cp "$root/LICENSE" "$out/licenses/supersilvia-AGPL-3.0.txt"
cp "$root/packaging/macos/LGPL-2.1.txt" "$out/licenses/GStreamer-LGPL-2.1.txt"
mkdir -p "$out/licenses/assets"
cp "$root"/licenses/*.txt "$out/licenses/assets/"
cp "$here/rust-crates.txt" "$out/licenses/rust-crates.txt"
cp "$here/dxc-LICENSE.txt" "$out/licenses/DirectXShaderCompiler-LICENSE.txt"
{
  echo "GStreamer $GST_VERSION, as carried in bin/, lib/gstreamer-1.0/ and libexec/"
  echo
  echo "Every file below is from GStreamer's official Windows release $GST_VERSION"
  echo "($GST_URL/), built by cerbero with MSVC, and is unmodified. GStreamer itself is"
  echo "under the GNU LGPL 2.1 or later (GStreamer-LGPL-2.1.txt); the libraries beside it"
  echo "are each under their own licence, LGPL or more permissive. The source of every one,"
  echo "at the release this is, is named by cerbero's recipes at tag $GST_VERSION:"
  echo "  https://gitlab.freedesktop.org/gstreamer/cerbero/-/tree/$GST_VERSION/recipes"
  echo "and GStreamer's own is at https://gstreamer.freedesktop.org/src/."
  echo "You may replace any of these libraries with your own build of the same version."
  echo
  echo "Plugins (lib/gstreamer-1.0):"
  (cd "$out/lib/gstreamer-1.0" && ls -1 *.dll | sed 's/^/  /')
  echo
  echo "Libraries (bin):"
  (cd "$out/bin" && ls -1 *.dll | sed 's/^/  /')
  echo
  echo "The plugin scanner (libexec/gstreamer-1.0/gst-plugin-scanner.exe)."
} >"$out/licenses/GStreamer-libraries.txt"

say "zipping $zip"
(cd "$dist" && zip -qr "$zip" supersilvia)
echo "$(du -sh "$out" | cut -f1) in $out, $(du -h "$zip" | cut -f1) zipped"
