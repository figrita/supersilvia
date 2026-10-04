#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build dist/supersilvia.app, with GStreamer and Syphon inside it, and the .dmg and .zip that
# carry it to a Mac with nothing installed. Apple Silicon, macOS 14.2 or later. Beside them,
# sources.sh puts GStreamer's source and SOURCE-OFFER.txt in one archive,
# gstreamer-<version>-source.tar, which travels with the download.
#
#   packaging/macos/build-app.sh
#
# GStreamer is the official macOS release, downloaded once into a cache outside the
# repository and unpacked there with pkgutil — nothing is installed, and Homebrew's GStreamer
# is never read. The binary is built against that copy, and the bundle carries the plugins the
# app uses, the libraries they reach and the plugin scanner, in
# Contents/Frameworks/GStreamer/{lib,libexec}: the layout GStreamer's own relocation expects,
# so the app sets nothing at start-up. README.md beside this has the whole of it.
#
# Environment, all optional:
#   SUPERSILVIA_SIGN_IDENTITY   a "Developer ID Application: …" identity in the keychain. Unset,
#                               the bundle is signed ad hoc. Set, every library and the app
#                               are signed with it under the hardened runtime.
#   SUPERSILVIA_NOTARY_PROFILE  a notarytool keychain profile (xcrun notarytool
#                               store-credentials). With an identity, the app and the .dmg are
#                               notarized and stapled.
#   SUPERSILVIA_PACKAGING_CACHE where GStreamer is downloaded and unpacked;
#                               ~/Library/Caches/supersilvia-packaging by default.
#   SUPERSILVIA_FEATURES        Cargo features for the binary, e.g. `inspection`, so an
#                               inspection client can drive the bundled app when it is started
#                               with EGUI_INSPECTION=1. Never for a build that is handed out.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

die() { echo "build-app: $*" >&2; exit 1; }
say() { echo "== $*"; }

# ------------------------------------------------------------------------------ inputs

GST_VERSION=1.28.7
GST_URL=https://gstreamer.freedesktop.org/data/pkg/osx/$GST_VERSION
GST_RUNTIME=gstreamer-1.0-$GST_VERSION-universal.pkg
GST_RUNTIME_SHA256=529fdf4a4027d942e59b5b3564f6400adaa008f63ce5f3fed4ffe35d73911994
GST_DEVEL=gstreamer-1.0-devel-$GST_VERSION-universal.pkg
GST_DEVEL_SHA256=72a44870cf02472cbf6e9a84bcc25ee6807dd1c26659a112066373544d365e7a

# The elements supersilvia makes, by name in src/ or inside a pipeline string there, and the
# ones a pipeline it builds reaches through decodebin for the files nodes::Accepts lets in.
# Each one's plugin is carried; one missing from the release stops the build.
ELEMENTS=(
  # every pipeline's plumbing (video/, audio/)
  filesrc filesink queue multiqueue capsfilter fakesink typefind appsrc appsink
  # a file of any kind (video/clip.rs, audio/track.rs), and Discoverer's uridecodebin
  decodebin uridecodebin
  # conversion
  videoconvert videoscale audioconvert audioresample
  # the Test source and the Text node (video/mod.rs, video/text.rs)
  videotestsrc textoverlay
  # PNG in and out (video/png.rs), and JPEG from a camera or inside a clip
  pngenc pngdec jpegdec
  # the clip cache and the render writer: VideoToolbox and MP4 (platform/macos/video.rs,
  # video/clip.rs, video/encode.rs)
  vtenc_h264_hw vtenc_h265_hw vtdec_hw h264parse h265parse mp4mux qtdemux
  # a camera and a microphone (platform/macos/video.rs, platform/macos/audio.rs)
  avfvideosrc osxaudiosrc
  # the containers Accepts::VIDEO and Accepts::AUDIO name, and what they hold
  matroskademux avidemux tsdemux oggdemux asfdemux wavparse aiffparse id3demux
  aacparse mpegaudioparse flacparse av1parse
  vp8dec vp9dec dav1ddec avdec_mpeg4 avdec_mpeg2video
  avdec_aac avdec_ac3 avdec_wmav2 mpg123audiodec opusdec vorbisdec flacdec
)
# Plugins no element above names: decodebin's type finders, and the types the registry
# serializes for the base libraries.
EXTRA_PLUGINS=(typefindfunctions pbtypes)
# Device providers, as plugin:feature: the camera list (platform/macos/video.rs).
PROVIDERS=(applemedia:avfdeviceprovider)

cache=${SUPERSILVIA_PACKAGING_CACHE:-$HOME/Library/Caches/supersilvia-packaging}
prefix=$cache/gstreamer-$GST_VERSION
identity=${SUPERSILVIA_SIGN_IDENTITY:-}
notary=${SUPERSILVIA_NOTARY_PROFILE:-}
[[ -z $notary || -n $identity ]] || die "SUPERSILVIA_NOTARY_PROFILE needs SUPERSILVIA_SIGN_IDENTITY"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[[ -n $version ]] || die "no version in Cargo.toml"
dist=$root/dist
app=$dist/supersilvia.app
name=supersilvia-$version-macos-arm64
dmg=$dist/$name.dmg
zip=$dist/$name.zip

[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || die "this builds on an Apple Silicon Mac"
for tool in cargo curl shasum pkgutil ditto lipo otool install_name_tool codesign hdiutil \
            python3 /usr/libexec/PlistBuddy; do
  command -v "$tool" >/dev/null || die "$tool not found"
done

# Nothing from the caller's GStreamer reaches a probe of the release's.
unset GST_PLUGIN_PATH GST_PLUGIN_PATH_1_0 GST_PLUGIN_SYSTEM_PATH GST_PLUGIN_SYSTEM_PATH_1_0 \
      GST_PLUGIN_SCANNER GST_PLUGIN_SCANNER_1_0 GST_REGISTRY GST_REGISTRY_1_0 GST_DEBUG
work=$(mktemp -d "${TMPDIR:-/tmp}/supersilvia-app.XXXXXX")
trap 'rm -rf "$work"' EXIT

# ------------------------------------------------------------------------------ GStreamer

fetch() { # file sha256
  local file=$cache/$1
  if ! echo "$2  $file" | shasum -a 256 -c --status 2>/dev/null; then
    say "downloading $1"
    curl -fL --retry 3 -o "$file.part" "$GST_URL/$1"
    mv "$file.part" "$file"
    echo "$2  $file" | shasum -a 256 -c --status || die "$1 does not match its checksum"
  fi
}

if [[ ! -f $prefix/.complete ]]; then
  mkdir -p "$cache"
  fetch "$GST_RUNTIME" "$GST_RUNTIME_SHA256"
  fetch "$GST_DEVEL" "$GST_DEVEL_SHA256"
  say "unpacking GStreamer $GST_VERSION into $prefix"
  rm -rf "$prefix" "$prefix.part" "$cache/expanded" "$prefix.registry.bin"
  mkdir -p "$cache/expanded"
  pkgutil --expand-full "$cache/$GST_RUNTIME" "$cache/expanded/runtime"
  pkgutil --expand-full "$cache/$GST_DEVEL" "$cache/expanded/devel"
  mkdir -p "$prefix.part"
  # Every runtime payload installs into GStreamer.framework/Versions/1.0, which is this
  # prefix; the one that is the framework's own skeleton is left out.
  for pkg in "$cache"/expanded/runtime/*.pkg; do
    case $pkg in *osx-framework*) continue ;; esac
    ditto "$pkg/Payload" "$prefix.part"
  done
  # Of the development payloads, the build reads the .pc files and pkg-config alone.
  for pkg in "$cache"/expanded/devel/*.pkg; do
    if [[ -d $pkg/Payload/lib/pkgconfig ]]; then
      ditto "$pkg/Payload/lib/pkgconfig" "$prefix.part/lib/pkgconfig"
    fi
    if [[ -f $pkg/Payload/bin/pkg-config ]]; then
      ditto "$pkg/Payload/bin/pkg-config" "$prefix.part/bin/pkg-config"
    fi
  done
  rm -rf "$cache/expanded"
  [[ -x $prefix.part/bin/pkg-config && -f $prefix.part/lib/pkgconfig/gstreamer-1.0.pc ]] ||
    die "the development package held no pkg-config or gstreamer-1.0.pc"
  mv "$prefix.part" "$prefix"
  touch "$prefix/.complete"
fi

# The release's own tools, on a registry of their own so the caller's is never rewritten.
release_inspect() { GST_REGISTRY=$prefix.registry.bin "$prefix/bin/gst-inspect-1.0" "$@"; }
# The file a plugin feature is in, or nothing. `inspect` is one of the two functions here.
filename_of() { # inspect feature
  "$1" "$2" 2>/dev/null | awk '/^  Filename/ {print $2}' || true
}
is_macho() { [[ $(file -b "$1") == Mach-O* ]]; }
# What a Mach-O links, without its own install name.
deps() {
  local id
  id=$(otool -D "$1" | tail -n +2)
  otool -L "$1" | tail -n +2 | awk '{print $1}' | { grep -v -x -F "${id:-/}" || true; }
}
rpaths() { otool -l "$1" | awk '/cmd LC_RPATH/ {getline; getline; print $2}'; }

# ------------------------------------------------------------------------------ the binary

profile=release
grep -q '^\[profile\.dist\]' Cargo.toml && profile=dist
say "cargo build --profile $profile, against GStreamer $GST_VERSION"
# pkg-config sees the release's .pc files and nothing else, so a library the release lacks
# fails the build instead of being found in Homebrew. A target folder of its own keeps these
# artifacts apart from the development build's.
PKG_CONFIG=$prefix/bin/pkg-config PKG_CONFIG_LIBDIR=$prefix/lib/pkgconfig PKG_CONFIG_PATH= \
  CARGO_TARGET_DIR=$root/target/app cargo build --locked --profile "$profile" \
  ${SUPERSILVIA_FEATURES:+--features "$SUPERSILVIA_FEATURES"}
binary=$root/target/app/$profile/supersilvia

# ------------------------------------------------------------------------------ the bundle

say "assembling $app"
rm -rf "$app"
contents=$app/Contents
frameworks=$contents/Frameworks
gst=$frameworks/GStreamer
mkdir -p "$contents/MacOS" "$contents/Resources/licenses" "$gst/lib/plugins" "$gst/libexec/scanner"
# GStreamer looks for gstreamer-1.0/ beside libgstreamer and in ../libexec, and codesign takes
# any folder in Frameworks whose name has a dot for a bundle it cannot sign. So the folders are
# plugins/ and scanner/, and gstreamer-1.0 is a symlink to each, which codesign does not enter
# and GStreamer follows.
ln -s plugins "$gst/lib/gstreamer-1.0"
ln -s scanner "$gst/libexec/gstreamer-1.0"

cp "$here/Info.plist" "$contents/Info.plist"
# macOS reads a bundle version as numbers only: 0.9.0 of 0.9.0-alpha.1.
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${version%%-*}" \
                        -c "Set :CFBundleVersion ${version%%-*}" "$contents/Info.plist"
plutil -lint "$contents/Info.plist" >/dev/null
cp "$root/assets/icon/supersilvia.icns" "$contents/Resources/supersilvia.icns"
cp "$here/Credits.rtf" "$contents/Resources/Credits.rtf"

cp "$binary" "$contents/MacOS/supersilvia"
chmod u+w "$contents/MacOS/supersilvia"
# The checkout's own rpaths — vendor/syphon, and the release's lib folder its .pc files add —
# mean nothing on another Mac; only the @executable_path ones stay.
for rpath in $(rpaths "$contents/MacOS/supersilvia" | sort -u); do
  case $rpath in @*) continue ;; esac
  while rpaths "$contents/MacOS/supersilvia" | grep -x -F "$rpath" >/dev/null; do
    install_name_tool -delete_rpath "$rpath" "$contents/MacOS/supersilvia"
  done
done

# Syphon, without the headers and module map only a compiler reads.
ditto "$root/vendor/syphon/Syphon.framework" "$frameworks/Syphon.framework"
rm -rf "$frameworks/Syphon.framework/Headers" "$frameworks/Syphon.framework/Modules" \
       "$frameworks/Syphon.framework/Versions/A/Headers" \
       "$frameworks/Syphon.framework/Versions/A/Modules" \
       "$frameworks/Syphon.framework/Versions/A/_CodeSignature"

# One architecture: the release is universal, and this app is arm64 alone.
thin() { # src dst
  if [[ $(lipo -archs "$1") == arm64 ]]; then
    cp -L "$1" "$2"
  else
    lipo "$1" -thin arm64 -output "$2"
  fi
  chmod u+w "$2"
}

say "choosing plugins"
plugins=()
add_plugin() { # file name
  local p
  for p in ${plugins[@]+"${plugins[@]}"}; do [[ $p == "$1" ]] && return 0; done
  plugins+=("$1")
}
for element in "${ELEMENTS[@]}"; do
  file=$(filename_of release_inspect "$element")
  [[ -n $file ]] || die "GStreamer $GST_VERSION's release has no element $element"
  case $file in "$prefix"/lib/gstreamer-1.0/*) ;; *) die "$element was found outside the release: $file" ;; esac
  add_plugin "$(basename "$file")"
done
for plugin in "${EXTRA_PLUGINS[@]}"; do
  [[ -f $prefix/lib/gstreamer-1.0/libgst$plugin.dylib ]] || die "no plugin $plugin in the release"
  add_plugin "libgst$plugin.dylib"
done
for plugin in "${plugins[@]}"; do
  thin "$prefix/lib/gstreamer-1.0/$plugin" "$gst/lib/gstreamer-1.0/$plugin"
done
thin "$prefix/libexec/gstreamer-1.0/gst-plugin-scanner" "$gst/libexec/gstreamer-1.0/gst-plugin-scanner"

# Every library something in the bundle links, followed until nothing new appears. The
# release names each as @rpath/<file>, and every one of them lives in its lib folder.
say "carrying the libraries they link"
queue=("$contents/MacOS/supersilvia" "$gst/libexec/gstreamer-1.0/gst-plugin-scanner")
for plugin in "${plugins[@]}"; do queue+=("$gst/lib/gstreamer-1.0/$plugin"); done
i=0
while (( i < ${#queue[@]} )); do
  file=${queue[$i]}
  i=$((i + 1))
  for dep in $(deps "$file"); do
    case $dep in
      /usr/lib/* | /System/*) ;;
      @rpath/Syphon.framework/*) ;;
      @rpath/*)
        lib=${dep#@rpath/}
        if [[ ! -f $gst/lib/$lib ]]; then
          [[ -f $prefix/lib/$lib ]] || die "$(basename "$file") links $dep, which the release lacks"
          thin "$prefix/lib/$lib" "$gst/lib/$lib"
          queue+=("$gst/lib/$lib")
        fi
        ;;
      *) die "$(basename "$file") links $dep, which is neither the system's nor carried" ;;
    esac
  done
done

# ------------------------------------------------------------------------------ licences

say "writing licences"
lic=$contents/Resources/licenses
cp "$root/LICENSE" "$lic/supersilvia-AGPL-3.0.txt"
cp "$root/vendor/syphon/LICENSE" "$lic/Syphon-BSD.txt"
cp "$here/LGPL-2.1.txt" "$lic/GStreamer-LGPL-2.1.txt"
mkdir -p "$lic/assets"
cp "$root"/licenses/*.txt "$lic/assets/"
# crate-licenses.py reads `cargo metadata --offline`, which needs every crate Cargo.lock names,
# every platform's, and the build above downloaded only this one's.
cargo fetch --locked
python3 "$root/scripts/crate-licenses.py" --target aarch64-apple-darwin >"$lic/rust-crates.txt"
{
  echo "GStreamer $GST_VERSION, as carried in Contents/Frameworks/GStreamer"
  echo
  echo "Every file below is from GStreamer's official macOS release $GST_VERSION"
  echo "($GST_URL/), built by cerbero, and is unmodified but for"
  echo "keeping its arm64 half. GStreamer itself is under the GNU LGPL 2.1 or later"
  echo "(GStreamer-LGPL-2.1.txt); the libraries beside it are each under their own licence,"
  echo "LGPL or more permissive. The source of every one, at the release this is, is named"
  echo "by cerbero's recipes at tag $GST_VERSION:"
  echo "  https://gitlab.freedesktop.org/gstreamer/cerbero/-/tree/$GST_VERSION/recipes"
  echo "and GStreamer's own at https://gstreamer.freedesktop.org/src/. Both accompany the"
  echo "download in gstreamer-$GST_VERSION-source.tar, as the SOURCE-OFFER.txt inside it says."
  echo "You may replace any of these libraries with your own build of the same version."
  echo
  echo "Plugins (lib/gstreamer-1.0):"
  for plugin in "${plugins[@]}"; do echo "  $plugin"; done
  echo
  echo "Libraries (lib):"
  (cd "$gst/lib" && ls -1 *.dylib | sed 's/^/  /')
  echo
  echo "The plugin scanner (libexec/gstreamer-1.0/gst-plugin-scanner)."
} >"$lic/GStreamer-libraries.txt"

# ------------------------------------------------------------------------------ checks

# The carried GStreamer, tried with the release's own gst-inspect and gst-launch copied into
# the bundle for the length of the check: plugins found by relocation alone, the scanner
# found beside them, every element loading, every library loaded from the bundle or the
# system, and the Text node's text drawn with a system font.
say "trying the carried GStreamer"
mkdir -p "$gst/bin"
thin "$prefix/bin/gst-inspect-1.0" "$gst/bin/gst-inspect-1.0"
thin "$prefix/bin/gst-launch-1.0" "$gst/bin/gst-launch-1.0"
bundle_inspect() { GST_REGISTRY=$work/bundle.bin "$gst/bin/gst-inspect-1.0" "$@"; }

GST_DEBUG=GST_PLUGIN_LOADING:5 GST_DEBUG_NO_COLOR=1 DYLD_PRINT_LIBRARIES=1 bundle_inspect coreelements >"$work/scan.log" 2>&1 ||
  { tail -n 40 "$work/scan.log" >&2; die "the carried gst-inspect-1.0 failed"; }
grep -E "using system plugin scanner at .*/Contents/Frameworks/GStreamer/(lib/\.\./)?libexec/gstreamer-1.0/gst-plugin-scanner" \
  "$work/scan.log" >/dev/null || { tail -n 40 "$work/scan.log" >&2; die "GStreamer did not find the carried plugin scanner"; }
grep "Plugin scanner child running" "$work/scan.log" >/dev/null || die "the carried plugin scanner did not start"
if grep -E "Failed starting plugin scanner|External plugin loader failed|Spawning gst-plugin-scanner helper failed" \
  "$work/scan.log" >&2; then
  die "the carried plugin scanner did not run"
fi
bundle_real=$(cd "$app" && pwd -P)
# dyld prints each image it loads as "dyld[<pid>]: <<uuid>> <path>", the scanner's included.
sed -n -E 's/^dyld\[[0-9]+\]: <[0-9A-F-]+> //p' "$work/scan.log" >"$work/loaded.txt"
grep -F "/Contents/Frameworks/GStreamer/lib/libgstreamer-1.0.0.dylib" "$work/loaded.txt" >/dev/null ||
  die "dyld did not report libgstreamer loaded from the bundle"
if grep -v -E "^(/usr/lib/|/System/|$bundle_real/|$app/)" "$work/loaded.txt" >&2; then
  die "a library above was loaded from outside the bundle and the system"
fi
for element in "${ELEMENTS[@]}"; do
  file=$(filename_of bundle_inspect "$element")
  case $file in
    "$gst"/lib/gstreamer-1.0/* | "$bundle_real"/Contents/Frameworks/GStreamer/lib/gstreamer-1.0/*) ;;
    *) die "element $element does not load from the bundle (found: ${file:-nothing})" ;;
  esac
done
for provider in "${PROVIDERS[@]}"; do
  features=$(bundle_inspect "${provider%%:*}" 2>/dev/null || true)
  [[ $features == *"${provider#*:}"* ]] ||
    die "device provider ${provider#*:} is not in the carried ${provider%%:*}"
done
draw() { # file [element…]
  local out=$1
  shift
  GST_REGISTRY=$work/bundle.bin "$gst/bin/gst-launch-1.0" -q videotestsrc pattern=black num-buffers=1 \
    ! video/x-raw,width=320,height=120 "$@" ! videoconvert ! pngenc ! filesink location="$out" \
    >/dev/null 2>&1
}
draw "$work/black.png" || die "the carried gst-launch-1.0 failed"
draw "$work/text.png" ! textoverlay text=supersilvia "font-desc=Helvetica 40" ||
  die "the carried textoverlay failed"
(( $(stat -f %z "$work/text.png") > $(stat -f %z "$work/black.png") + 200 )) ||
  die "textoverlay drew no text: no system font reached pango"
rm -rf "$gst/bin"

# Every Mach-O in the bundle: arm64 alone, linking only the system and what is carried, with
# no rpath and no library path into Homebrew, /usr/local, /Library/Frameworks or any other
# absolute place.
say "auditing every Mach-O in the bundle"
count=0
while IFS= read -r -d '' file; do
  is_macho "$file" || continue
  count=$((count + 1))
  rel=${file#"$app"/}
  [[ $(lipo -archs "$file") == arm64 ]] || die "$rel is not arm64 alone"
  for dep in $(deps "$file"); do
    case $dep in
      /usr/lib/* | /System/* | @rpath/* | @loader_path/* | @executable_path/*) ;;
      *) die "$rel links $dep" ;;
    esac
  done
  for rpath in $(rpaths "$file"); do
    case $rpath in @*) ;; *) die "$rel carries the rpath $rpath" ;; esac
  done
  if otool -l "$file" | grep -E '^ +name (/opt/homebrew|/usr/local|/Library/Frameworks)' >&2; then
    die "$rel names a library in a place a tester's Mac does not have"
  fi
  if grep -a -q /opt/homebrew "$file"; then die "$rel mentions /opt/homebrew"; fi
done < <(find "$app" -type f -print0)
echo "$count Mach-O files, all clean"

# ------------------------------------------------------------------------------ signing

sign() { # path [extra codesign arguments]
  local path=$1
  shift
  if [[ -n $identity ]]; then
    codesign --force --sign "$identity" --options runtime --timestamp "$@" "$path"
  else
    codesign --force --sign - "$@" "$path"
  fi
}
say "signing ${identity:-ad hoc}"
# Inside out: every library and the scanner, then Syphon's framework, then the app, whose
# signature seals everything under it.
while IFS= read -r -d '' file; do
  if is_macho "$file"; then sign "$file"; fi
done < <(find "$gst" -type f -print0)
sign "$frameworks/Syphon.framework"
sign "$app" --entitlements "$here/entitlements.plist"
codesign --verify --deep --strict --verbose=1 "$app"

# ------------------------------------------------------------------------------ the .dmg and .zip

make_dmg() {
  local stage=$work/dmg
  rm -rf "$stage" "$dmg"
  mkdir -p "$stage"
  ditto "$app" "$stage/supersilvia.app"
  ln -s /Applications "$stage/Applications"
  hdiutil create -quiet -volname "supersilvia $version" -srcfolder "$stage" -fs HFS+ \
    -format UDZO -ov "$dmg"
  [[ -z $identity ]] || codesign --force --sign "$identity" --timestamp "$dmg"
}
make_zip() {
  rm -f "$zip"
  ditto -c -k --keepParent "$app" "$zip"
}
notarize() { # file
  say "notarizing $(basename "$1")"
  xcrun notarytool submit "$1" --keychain-profile "$notary" --wait --output-format json \
    >"$work/notary.json" || true
  cat "$work/notary.json"
  grep '"status" *: *"Accepted"' "$work/notary.json" >/dev/null ||
    die "notarization of $(basename "$1") was not accepted; xcrun notarytool log <id> --keychain-profile $notary"
}

if [[ -n $notary ]]; then
  # The app first, through a zip, so its ticket can be stapled to it; then the .dmg made from
  # the stapled app, and the zip made again for the same reason.
  make_zip
  notarize "$zip"
  xcrun stapler staple "$app"
  make_dmg
  notarize "$dmg"
  xcrun stapler staple "$dmg"
  make_zip
  spctl --assess --type execute --verbose=2 "$app"
else
  make_dmg
  make_zip
fi

# GStreamer's source and cerbero's recipes beside the download, with SOURCE-OFFER.txt, in one
# archive: the LGPL has the source of the libraries the bundle carries accompany it.
"$here/sources.sh" "$GST_VERSION" "$dist"

say "built"
du -sh "$app" "$dmg" "$zip"
du -h "$dist/gstreamer-$GST_VERSION-source.tar"
