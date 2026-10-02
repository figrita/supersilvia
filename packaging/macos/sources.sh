#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The source offer that travels beside the Mac download: GStreamer's own source for the
# release the .app carries, cerbero's tree at that release — the recipes naming the source of
# every other library in it, with the patches cerbero applies — and SOURCE-OFFER.txt saying
# so. The .app carries GStreamer's LGPL libraries, whose source the LGPL has accompany it.
#
#   packaging/macos/sources.sh <GStreamer version> <folder>
#
# build-app.sh calls it with its own GST_VERSION and dist/. Every file is checked before it is
# copied: each tarball against the SHA-256 written below, cerbero against the commit its tag
# names. Downloads are kept in SUPERSILVIA_PACKAGING_CACHE (as build-app.sh's are) and reused.
# Nothing here is Mac-only: bash, curl, git and shasum or sha256sum.
set -euo pipefail

die() { echo "sources: $*" >&2; exit 1; }
say() { echo "== $*"; }

[[ $# -eq 2 ]] || die "usage: sources.sh <GStreamer version> <folder>"
version=$1
out=$2

# The release these are for. A GStreamer bump in build-app.sh changes these with it.
GST_VERSION=1.28.7
[[ $version == "$GST_VERSION" ]] ||
  die "these checksums are for GStreamer $GST_VERSION, and build-app.sh carries $version"
SRC_URL=https://gstreamer.freedesktop.org/src
# The GStreamer modules whose plugins or libraries the .app carries, with the SHA-256 each
# release tarball is published with (<module>-<version>.tar.xz.sha256sum beside it).
MODULES=(
  "gstreamer 787329b2c5758e228a71d926a6dcf960bceaacca3cadd63874ba665dfcda013e"
  "gst-plugins-base ed6e5410f496d171818763af2265e7977154bc7f9b827e98acf8c5bed21dd5a7"
  "gst-plugins-good 87256969c82cf3bc8574301f3e7044a90de0ac500a5a27d8ba38c4dde894dd8b"
  "gst-plugins-bad dc525383c18b2c265bbe6a43d498656cd918aaa130aa4e3abeabcdaa741c3ffe"
  "gst-plugins-ugly 2b681170ddc22b6b283cafeed48f427c30a17056974a6a9ed137c354e0f7730c"
  "gst-libav 58da51dd39ecf1cf6faade34cc6412001be2e2e145bca8ae0f45336f60a36ab2"
)
CERBERO_URL=https://gitlab.freedesktop.org/gstreamer/cerbero.git
# The commit cerbero's tag $GST_VERSION points at.
CERBERO_COMMIT=e0e7007e210dab3e2f3e898939e2cfe1fd02f123

for tool in curl git; do command -v "$tool" >/dev/null || die "$tool not found"; done
sha256() {
  if command -v shasum >/dev/null; then shasum -a 256 "$1"; else sha256sum "$1"; fi | awk '{print $1}'
}

cache=${SUPERSILVIA_PACKAGING_CACHE:-$HOME/Library/Caches/supersilvia-packaging}/sources-$GST_VERSION
mkdir -p "$cache" "$out"

tarballs=()
for entry in "${MODULES[@]}"; do
  module=${entry%% *}
  want=${entry#* }
  file=$module-$GST_VERSION.tar.xz
  if [[ ! -f $cache/$file || $(sha256 "$cache/$file") != "$want" ]]; then
    say "downloading $file"
    curl -sSfL --retry 3 -o "$cache/$file.part" "$SRC_URL/$module/$file"
    mv "$cache/$file.part" "$cache/$file"
  fi
  [[ $(sha256 "$cache/$file") == "$want" ]] || die "$file does not match its checksum"
  cp "$cache/$file" "$out/$file"
  tarballs+=("$file")
done

cerbero=cerbero-$GST_VERSION.tar.gz
if [[ ! -f $cache/$cerbero || ! -f $cache/$cerbero.commit ||
      $(cat "$cache/$cerbero.commit") != "$CERBERO_COMMIT" ]]; then
  say "fetching cerbero $GST_VERSION"
  rm -rf "$cache/cerbero"
  git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$GST_VERSION" "$CERBERO_URL" "$cache/cerbero"
  got=$(git -C "$cache/cerbero" rev-parse HEAD)
  [[ $got == "$CERBERO_COMMIT" ]] ||
    die "cerbero's tag $GST_VERSION is $got, not the $CERBERO_COMMIT written here"
  git -C "$cache/cerbero" archive --format=tar.gz --prefix="cerbero-$GST_VERSION/" \
    -o "$cache/$cerbero.part" HEAD
  mv "$cache/$cerbero.part" "$cache/$cerbero"
  echo "$CERBERO_COMMIT" >"$cache/$cerbero.commit"
  rm -rf "$cache/cerbero"
fi
cp "$cache/$cerbero" "$out/$cerbero"

say "writing SOURCE-OFFER.txt"
{
  echo "Source for the GStreamer libraries inside supersilvia for macOS"
  echo "=============================================================="
  echo
  echo "supersilvia.app carries libraries and plugins from GStreamer's official macOS"
  echo "release $GST_VERSION, in supersilvia.app/Contents/Frameworks/GStreamer. GStreamer is"
  echo "under the GNU Lesser General Public License 2.1 or later, as are some of the"
  echo "libraries it is built with; the bundle's Contents/Resources/licenses/ holds the"
  echo "licence texts and GStreamer-libraries.txt, the list of every file carried."
  echo
  echo "GStreamer's source, and the build description of everything else, accompany this"
  echo "download in these files beside it:"
  echo
  for file in "${tarballs[@]}"; do
    printf '  %-36s %s\n' "$file" "$(sha256 "$out/$file")"
  done
  printf '  %-36s %s\n' "$cerbero" "$(sha256 "$out/$cerbero")"
  echo
  echo "The six .tar.xz files are GStreamer's own release tarballs, exactly as published"
  echo "at $SRC_URL/<module>/. $cerbero is cerbero, GStreamer's"
  echo "build system, at its tag $GST_VERSION (commit $CERBERO_COMMIT,"
  echo "$CERBERO_URL): the recipes that built the release."
  echo "Its recipes for the six modules name these very tarballs, by the same checksums,"
  echo "and apply no patch to them. Every other library in the release is named by its own"
  echo "recipe there, with the exact source, its checksum and any patch cerbero applies to"
  echo "it; GStreamer mirrors those tarballs at $SRC_URL/mirror/."
  echo
  echo "No patches: supersilvia uses the release's files as they are. Its build keeps each"
  echo "file's arm64 half (lipo -thin) and signs it with the app's signature, which changes"
  echo "no code. The app links them dynamically, so any of them can be replaced by your own"
  echo "build of the same version."
  echo
  echo "supersilvia's own source, under the GNU AGPL 3.0 or later, is at"
  echo "https://github.com/figrita/supersilvia."
} >"$out/SOURCE-OFFER.txt"
