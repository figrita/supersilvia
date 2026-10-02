#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Rebuilds vendor/syphon/Syphon.framework from Syphon-Framework's source: arm64 alone, macOS
# 14.2 and later, the Xcode project's Release settings reproduced with clang, metal and metallib
# directly. See vendor/README.md.
#
#   vendor/syphon/build-framework.sh <a Syphon-Framework checkout at the pinned commit>
set -euo pipefail
SRC=$(cd "${1:?the Syphon-Framework checkout}" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
OBJ=$WORK/obj
OUT=$WORK/Syphon.framework
SDK=$(xcrun --sdk macosx --show-sdk-path)
MIN=14.2
mkdir -p "$OBJ/include"
ln -s "$SRC" "$OBJ/include/Syphon"
CFLAGS=(-arch arm64 -isysroot "$SDK" -mmacosx-version-min=$MIN -Os -g -fvisibility=default
  -fobjc-arc -fobjc-weak -fmodules -fno-common -std=gnu99 -DSYPHON_CORE_SHARE=1
  -include "$SRC/Syphon_Prefix.pch" -I"$SRC" -I"$OBJ/include" -Wno-deprecated-declarations)
# The files the project builds with GL_SILENCE_DEPRECATION.
GLSILENCE=" SyphonCGL.c SyphonGLShader.m SyphonGLVertices.m SyphonIOSurfaceImageCore.m SyphonIOSurfaceImageLegacy.m SyphonOpenGLServer.m SyphonServerRendererCoreGL.m "
cd "$SRC"
for f in *.m *.c; do
  extra=()
  [[ "$GLSILENCE" == *" $f "* ]] && extra=(-DGL_SILENCE_DEPRECATION)
  xcrun clang "${CFLAGS[@]}" ${extra[@]+"${extra[@]}"} -c "$f" -o "$OBJ/${f%.*}.o"
done
xcrun -sdk macosx metal -c -mmacosx-version-min=$MIN -O2 SyphonMetalShaders.metal -o "$OBJ/SyphonMetalShaders.air"
xcrun -sdk macosx metallib "$OBJ/SyphonMetalShaders.air" -o "$OBJ/default.metallib"
V=$OUT/Versions/A
mkdir -p "$V/Headers" "$V/Modules" "$V/Resources/en.lproj"
xcrun clang -arch arm64 -isysroot "$SDK" -mmacosx-version-min=$MIN -dynamiclib -fobjc-arc -fobjc-link-runtime \
  -install_name @rpath/Syphon.framework/Versions/A/Syphon \
  -compatibility_version 1 -current_version 1 -Wl,-dead_strip \
  -Wl,-exported_symbols_list,"$SRC/Exported_Symbols.exp" \
  -framework Cocoa -framework Foundation -framework IOSurface -framework Metal -framework OpenGL -framework QuartzCore \
  "$OBJ"/*.o -o "$V/Syphon"
for h in Syphon.h SyphonClient.h SyphonClientBase.h SyphonImage.h SyphonImageBase.h SyphonMetalClient.h SyphonMetalServer.h \
         SyphonOpenGLClient.h SyphonOpenGLImage.h SyphonOpenGLServer.h SyphonServer.h SyphonServerBase.h SyphonServerDirectory.h SyphonSubclassing.h; do
  cp "$h" "$V/Headers/"
done
cp Syphon.modulemap "$V/Modules/module.modulemap"
sed -e 's/\${EXECUTABLE_NAME}/Syphon/' -e 's/\$(PRODUCT_BUNDLE_IDENTIFIER)/info.v002.Syphon/' -e 's/\${PRODUCT_NAME}/Syphon/' Info.plist > "$V/Resources/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string MacOSX" -c "Add :LSMinimumSystemVersion string $MIN" "$V/Resources/Info.plist"
plutil -lint "$V/Resources/Info.plist"
cp en.lproj/InfoPlist.strings "$V/Resources/en.lproj/"
cp "$OBJ/default.metallib" "$V/Resources/"
ln -s A "$OUT/Versions/Current"
for l in Syphon Headers Modules Resources; do ln -s "Versions/Current/$l" "$OUT/$l"; done
xcrun strip -x "$V/Syphon"
codesign --force --sign - --timestamp=none "$OUT"
rm -rf "$HERE/Syphon.framework"
ditto "$OUT" "$HERE/Syphon.framework"
cp "$SRC/License.txt" "$HERE/LICENSE"
rm -rf "$WORK"
echo "vendor/syphon/Syphon.framework rebuilt from $(git -C "$SRC" rev-parse HEAD)"
