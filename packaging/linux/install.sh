#!/usr/bin/env bash
# Install the icon, the desktop entry, the MIME type, the licences and (if it is built) the
# binary into a prefix. This is the plain-Linux path, and the AppImage's, whose AppDir is a
# prefix packaging/appimage/build.sh points this at; the Flatpak lays the same files down its
# own way, from the same sources, and does not run it.
#
#   packaging/linux/install.sh                 # ~/.local, no root
#   PREFIX=/usr/local sudo packaging/linux/install.sh
#   packaging/linux/install.sh --uninstall
#   SUPERSILVIA_BINARY=<path> packaging/linux/install.sh   # a binary built elsewhere
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
prefix=${PREFIX:-$HOME/.local}
icons=$prefix/share/icons/hicolor
sizes=(16 24 32 48 64 128 256 512)
# The picture windows' own entries. They launch nothing and show nowhere; they exist so a
# Wayland desktop has an icon to find for a window whose app id is one of these. Their icons
# stop at 256, which is where make-icons.py stops for them.
windows=(supersilvia-popout supersilvia-fullscreen)
window_sizes=(16 24 32 48 64 128 256)
# The licence and the third-party notices, as text beside the binary: the same files the
# binary shows under Help > Licences, which it carries compiled in.
licences=$prefix/share/licenses/supersilvia

files=("$prefix/bin/supersilvia" "$prefix/share/applications/supersilvia.desktop"
       "$prefix/share/mime/packages/supersilvia.xml"
       "$icons/scalable/apps/supersilvia.svg"
       "$prefix/share/metainfo/io.github.figrita.supersilvia.metainfo.xml")
for size in "${sizes[@]}"; do files+=("$icons/${size}x${size}/apps/supersilvia.png"); done
for name in "${windows[@]}"; do
    files+=("$prefix/share/applications/$name.desktop")
    for size in "${window_sizes[@]}"; do files+=("$icons/${size}x${size}/apps/$name.png"); done
done
files+=("$licences/LICENSE" "$licences/rust-crates.txt")
for notice in "$root"/licenses/*.txt; do files+=("$licences/assets/$(basename "$notice")"); done

if [[ ${1:-} == --uninstall ]]; then
    rm -fv "${files[@]}"
else
    for size in "${sizes[@]}"; do
        install -Dm644 "$root/assets/icon/supersilvia-$size.png" "$icons/${size}x${size}/apps/supersilvia.png"
    done
    install -Dm644 "$root/assets/icon/supersilvia.svg" "$icons/scalable/apps/supersilvia.svg"
    for name in "${windows[@]}"; do
        for size in "${window_sizes[@]}"; do
            install -Dm644 "$root/assets/icon/$name-$size.png" "$icons/${size}x${size}/apps/$name.png"
        done
        install -Dm644 "$here/$name.desktop" "$prefix/share/applications/$name.desktop"
    done
    install -Dm644 "$here/supersilvia.desktop" "$prefix/share/applications/supersilvia.desktop"
    install -Dm644 "$here/supersilvia.xml" "$prefix/share/mime/packages/supersilvia.xml"
    install -Dm644 "$here/io.github.figrita.supersilvia.metainfo.xml" \
        "$prefix/share/metainfo/io.github.figrita.supersilvia.metainfo.xml"
    install -Dm644 "$root/LICENSE" "$licences/LICENSE"
    install -Dm644 "$here/rust-crates.txt" "$licences/rust-crates.txt"
    for notice in "$root"/licenses/*.txt; do
        install -Dm644 "$notice" "$licences/assets/$(basename "$notice")"
    done
    binary=${SUPERSILVIA_BINARY:-$root/target/dist/supersilvia}
    [[ -x $binary || -n ${SUPERSILVIA_BINARY:-} ]] || binary=$root/target/release/supersilvia
    if [[ -x $binary ]]; then
        install -Dm755 "$binary" "$prefix/bin/supersilvia"
    else
        echo "no binary at ${SUPERSILVIA_BINARY:-target/dist or target/release}; icon and entry installed anyway" >&2
    fi
fi

# Caches, so the icon and the .ssw association show up without a logout.
update-desktop-database "$prefix/share/applications" 2>/dev/null || true
update-mime-database "$prefix/share/mime" 2>/dev/null || true
gtk-update-icon-cache -f -t "$icons" 2>/dev/null || true
