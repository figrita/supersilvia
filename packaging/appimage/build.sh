#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build target/supersilvia-<version>-<arch>.AppImage. An AppDir is a prefix like any other, so
# this is packaging/linux/install.sh pointed at a staging directory, plus the three things an
# AppImage wants at the top level: AppRun, the .desktop file, and .DirIcon.
#
#   packaging/appimage/build.sh
#
# The binary is built in a container from Containerfile beside this — Ubuntu 24.04, whose
# glibc 2.39 is the oldest the AppImage runs on — into target/appimage/, apart from the
# development build's target/. SUPERSILVIA_APPIMAGE_BINARY names a binary built elsewhere
# instead; its glibc floor is still checked. The container runs under podman or docker, or
# the host's podman through flatpak-spawn from inside a distrobox. appimagetool
# (https://github.com/AppImage/appimagetool/releases) is found on PATH or at APPIMAGETOOL.
#
# GStreamer, the Vulkan driver, ALSA, udev, fontconfig and the windowing libraries are
# deliberately NOT bundled: they are the machine's, the way a video app on Linux has to take
# them, and packaging/linux/TESTERS.md says what a tester installs. So the binary may link
# nothing past LINKED below, which this script holds it to, and `supersilvia --check` tells a
# tester what their machine lacks.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

die() { echo "appimage: $*" >&2; exit 1; }
say() { echo "== $*"; }

# The newest glibc symbol version the binary may need: Ubuntu 24.04's.
GLIBC_FLOOR=2.39
# Every library the binary may link, which every tester's machine has to provide. A new
# entry here is a new line in TESTERS.md.
LINKED=(
  libc.so.6 libm.so.6 libgcc_s.so.1 ld-linux-x86-64.so.2 ld-linux-aarch64.so.1
  libglib-2.0.so.0 libgobject-2.0.so.0 libgio-2.0.so.0
  libgstreamer-1.0.so.0 libgstbase-1.0.so.0 libgstapp-1.0.so.0 libgstaudio-1.0.so.0
  libgstvideo-1.0.so.0 libgstpbutils-1.0.so.0 libgstallocators-1.0.so.0
  libasound.so.2 libudev.so.1 libfontconfig.so.1
)
IMAGE=localhost/supersilvia-appimage-build:24.04

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[[ -n $version ]] || die "no version in Cargo.toml"
arch=$(uname -m)
appdir=${APPDIR:-$root/target/AppDir}
out=$root/target/supersilvia-$version-$arch.AppImage

# ------------------------------------------------------------------------------ the binary

binary=${SUPERSILVIA_APPIMAGE_BINARY:-}
if [[ -z $binary ]]; then
  if command -v podman >/dev/null; then runtime=(podman)
  elif command -v docker >/dev/null; then runtime=(docker)
  elif command -v flatpak-spawn >/dev/null && flatpak-spawn --host podman --version >/dev/null 2>&1; then
    runtime=(flatpak-spawn --host podman)
  else
    die "no podman or docker to build the binary in; or set SUPERSILVIA_APPIMAGE_BINARY"
  fi
  say "building the build image $IMAGE with ${runtime[*]}"
  "${runtime[@]}" build -q -t "$IMAGE" -f "$here/Containerfile" "$here" >/dev/null
  # Rootless podman maps the container's root to the caller, so what cargo writes is the
  # caller's; docker is asked to run as the caller instead. The checkout is mounted without
  # relabelling, so its SELinux context stays as it is.
  user=()
  [[ ${runtime[0]} == docker ]] && user=(--user "$(id -u):$(id -g)")
  say "cargo build --profile dist on Ubuntu 24.04, into target/appimage"
  "${runtime[@]}" run --rm ${user[@]+"${user[@]}"} --security-opt label=disable \
    -v "$root:/src" -w /src \
    -e CARGO_HOME=/src/target/appimage/cargo-home -e CARGO_TARGET_DIR=/src/target/appimage \
    "$IMAGE" cargo build --locked --profile dist
  binary=$root/target/appimage/dist/supersilvia
fi
[[ -x $binary ]] || die "no binary at $binary"

say "checking what $binary links"
glibc=$(objdump -T "$binary" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -n 1)
[[ $(printf '%s\n%s\n' "$glibc" "$GLIBC_FLOOR" | sort -V | tail -n 1) == "$GLIBC_FLOOR" ]] ||
  die "the binary needs glibc $glibc, newer than $GLIBC_FLOOR: build it in the container"
for lib in $(readelf -d "$binary" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
  printf '%s\n' "${LINKED[@]}" | grep -x -F "$lib" >/dev/null ||
    die "the binary links $lib, which LINKED does not name; add it there and to TESTERS.md"
done
echo "glibc $glibc or newer, and nothing linked past LINKED"

# ------------------------------------------------------------------------------ the AppDir

say "assembling $appdir"
rm -rf "$appdir"
PREFIX=$appdir/usr SUPERSILVIA_BINARY=$binary "$root/packaging/linux/install.sh"
[[ -x $appdir/usr/bin/supersilvia ]] || die "install.sh laid down no binary"

install -Dm644 "$appdir/usr/share/applications/supersilvia.desktop" "$appdir/supersilvia.desktop"
install -Dm644 "$root/assets/icon/supersilvia-256.png" "$appdir/supersilvia.png"
cp "$appdir/supersilvia.png" "$appdir/.DirIcon"

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
export PATH="$here/usr/bin:$PATH"
export XDG_DATA_DIRS="$here/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "$here/usr/bin/supersilvia" "$@"
RUN
chmod +x "$appdir/AppRun"

# ------------------------------------------------------------------------------ the AppImage

tool=${APPIMAGETOOL:-appimagetool}
command -v "$tool" >/dev/null || die "appimagetool not found: put it on PATH or name it in APPIMAGETOOL"
say "writing $out"
rm -f "$out"
# appimagetool is an AppImage itself, and one run where FUSE is not unpacks itself instead.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=$arch "$tool" "$appdir" "$out"
say "built"
du -h "$out"
