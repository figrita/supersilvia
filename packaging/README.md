# packaging

Everything that puts supersilvia somewhere other than `cargo run`, and everywhere the logo
has to appear on the way. Nothing in here is built by `check.sh` or by any test; it is here
so that the answer to "where does the icon go for X" is a file rather than a search.

## The logo

Two SVGs in [`../assets/icon/`](../assets/icon/) are the artwork:

| | |
| --- | --- |
| `supersilvia.svg` | the mark in white with a black outline, transparent, squared. **The app icon.** |
| `supersilvia-mark.svg` | the mark alone, transparent, for anywhere with its own background |

Everything else is rendered from those two by
[`../scripts/make-icons.py`](../scripts/make-icons.py) — PNGs at 16 through 1024, a Windows
`.ico` and a macOS `.icns`. The script needs nothing installed, no build runs it, and its
output is committed. Edit an SVG, re-run it, commit.

Two more SVGs beside them are window icons rather than app icons:
`supersilvia-popout.svg` and `supersilvia-fullscreen.svg`, the two marks a picture carries,
white on black. They are the geometry of `popout_mark` and `expand_mark` in
`src/ui/node_widget.rs`, copied into a 100-unit square — so the icon of a picture window is
the mark that was clicked to open it. Change a mark, change its SVG.

It reaches the running app in one place: `src/main.rs` bakes `supersilvia-256.png` into the
binary with `include_bytes!` and hands it to `ViewportBuilder::with_icon`. Everywhere else —
the dock, the task switcher, a Wayland titlebar, a file manager — reads it from the installed
`.desktop` file and the hicolor theme instead, matched to the window by the app id
`supersilvia`, which `src/main.rs` and `StartupWMClass` in the desktop entry both have to
say.

**A picture window is not the editor**, and carries one of two app ids of its own —
`supersilvia-popout` or `supersilvia-fullscreen`, in `src/render/picture/thread.rs`. Each has
a `NoDisplay` desktop entry in `linux/` whose only job is to hold an `Icon=` line, so a
pop-out and a fullscreen picture are their own entries in a taskbar with their own marks
instead of hiding behind the editor's. The id is set when the window is made and not changed
after: KWin re-resolves a changed app id badly and drops back to the editor's icon, so a
pop-out that `F` makes fullscreen keeps the pop-out mark. With nothing installed, all three
fall back to the desktop's unknown-window icon — which is what happened before any of this.

## What is here

| | | state |
| --- | --- | --- |
| [`linux/`](linux/) | desktop entries — the app, and one each for the two kinds of picture window — MIME type for `.ssw`, AppStream metadata, the crates' notices, `install.sh` into any prefix, and [the page for testers](linux/TESTERS.md) | works |
| [`appimage/`](appimage/) | `build.sh`: the binary built on Ubuntu 24.04 in a container (`Containerfile`), its glibc and linked libraries checked, the AppDir, `AppRun`, `.DirIcon`, appimagetool | built here; unpacked, resolved and `--check`ed in clean Fedora 44, Ubuntu 24.04 and Arch containers; never opened on a tester's desktop |
| [`flatpak/`](flatpak/) | the Flathub manifest, `io.github.figrita.supersilvia` | skeleton; needs `cargo-sources.json`, a tag and a current runtime |
| [`windows/`](windows/) | `build.sh`: cross-compiled from Linux with cargo-xwin against GStreamer's MSVC release, the relocatable folder and its `.zip`; `wine.sh` to run it; the icon and version from `build.rs` | built here; `--check`ed and opened under Wine 11; never run on Windows |
| [`macos/`](macos/) | `build-app.sh`: the `.app` with GStreamer and Syphon inside, its `.dmg` and `.zip`, signed ad hoc or with a Developer ID, and the page for testers | works; tested on one Apple Silicon Mac |

## The AppImage

```sh
packaging/appimage/build.sh     # target/supersilvia-<version>-x86_64.AppImage
```

It needs podman or docker — or, inside a distrobox, the host's podman, which it reaches through
`flatpak-spawn` — and [appimagetool](https://github.com/AppImage/appimagetool/releases) on
`PATH` or in `APPIMAGETOOL`. The binary is built in a container from
[`appimage/Containerfile`](appimage/Containerfile), Ubuntu 24.04, into `target/appimage/`:
a binary links the glibc it was built against as its floor, and the development box's is
newer than any LTS distribution's, so a binary from `cargo build` here would refuse to start on
Ubuntu 24.04. Ubuntu 24.04 is the floor because it is the oldest mainstream distribution with
GStreamer 1.24, which the `gstreamer` crates' `v1_24` feature makes the binary need.

**GStreamer, the Vulkan driver and the windowing libraries are the machine's, not the
AppImage's**, as they are for every video app on Linux: GStreamer's plugins come with the
distribution's codecs, VA-API drivers and PipeWire, and a bundled copy would be a second
GStreamer beside the system's, missing whichever of those it did not carry. So the requirement
is made explicit twice. `build.sh` holds the binary to a glibc of 2.39 or older and to linking
nothing past `LINKED` — glibc, GLib, GStreamer's core and base libraries, ALSA, udev and
fontconfig — failing the build otherwise; and **`supersilvia --check`** (`src/check.rs`) asks a
running machine for everything the app opens after start-up — every GStreamer element by the
plugin set it ships in, a hardware codec pair, the GPU `render::adapter` would pick, the Vulkan
loader and the windowing libraries, the session, the audio server, the MIDI sequencer and the
NDI® runtime — with a PASS, a WARN or a FAIL for each and an exit code of 1 on a FAIL.
[`linux/TESTERS.md`](linux/TESTERS.md) is what to install on each distribution and what each
line means.

**What was tried**, on clean Fedora 44, Ubuntu 24.04 and Arch containers with the packages
TESTERS.md names and nothing else, no display, and an Intel iGPU handed in as
`/dev/dri`. On all three the AppImage unpacks with `--appimage-extract` and runs with
`--appimage-extract-and-run` — a container has no FUSE, and run bare it says so and stops —
`ldd` finds every library the binary links, `--version` answers, and `--check` exits 0 with
every GStreamer line, the GPU on the distribution's own Mesa and the Vulkan loader passing. The
hardware codec line passes on Ubuntu 24.04 (`intel-media-va-driver-non-free`) and Arch
(`intel-media-driver`), and on Fedora only with RPM Fusion's `intel-media-driver`, as
TESTERS.md says; Fedora's own `gst-libav` lacks the H.264 and HEVC decoders. The session, audio
server and MIDI lines warn, as a container has none, and pass the session with Wayland's
variable set. On Ubuntu 22.04 the binary refuses to start: its glibc is 2.35. Not tried: a
window, a picture, a camera, a microphone, an imported clip, NDI, FUSE, AMD or NVIDIA.

## Releases

**A tag `v<version>` pushed to GitHub builds all three** —
[`../.github/workflows/release.yml`](../.github/workflows/release.yml) — and nothing else does:

```sh
git tag v0.9.0-alpha.1 && git push origin v0.9.0-alpha.1
```

The tag has to be Cargo.toml's version with a `v` before it, or the run stops at its first job.
Then each download is built on GitHub's machines by the same script as by hand: the AppImage
and the Windows folder on Ubuntu 24.04, the Windows one with Wine there for GStreamer's
installer alone, and the `.app` on an Apple Silicon Mac. Each is started once on its own
operating system — the AppImage on the Ubuntu machine, the Windows `.zip` unpacked on Windows
Server 2025, the `.app` on the Mac — and has to answer `--version`. `--check` runs after it and
is printed, not judged: these machines have no GPU worth the name and no hardware codec pair,
so its FAILs there say nothing about a tester's. When all of that passes, the AppImage, the
Windows `.zip`, the `.dmg`, the Mac `.zip` and `gstreamer-<version>-source.tar` — GStreamer's
source and `SOURCE-OFFER.txt`, which the LGPL has go with the downloads that carry it — go on a
**draft** release, marked a pre-release where the version has a `-`, for a person to
look over and publish. A run again for the same tag replaces the draft's files.

`check.sh` is not run there. Its GPU tests need a real GPU and its clip tests a hardware codec
pair, so the gate stays on a developer's machine, and a tag is put on a commit that passed it.

**The Mac build signs and notarizes with five repository secrets**, set under Settings ▸
Secrets and variables ▸ Actions:

| secret | what it holds |
| --- | --- |
| `MACOS_CERTIFICATE_P12` | the *Developer ID Application* certificate with its private key, exported from Keychain Access as a `.p12`, base64: `base64 -i certificate.p12 \| pbcopy` |
| `MACOS_CERTIFICATE_PASSWORD` | the password the `.p12` was exported with |
| `MACOS_NOTARY_KEY` | an App Store Connect API key's `.p8` file, its text as it is: Users and Access ▸ Integrations ▸ App Store Connect API, a Team key with the Developer role |
| `MACOS_NOTARY_KEY_ID` | that key's Key ID |
| `MACOS_NOTARY_ISSUER` | the Issuer ID above the list of keys |

The run puts the certificate in a keychain of its own, finds the identity in it, stores the key
as a `notarytool` profile there and hands both to `build-app.sh` as
`SUPERSILVIA_SIGN_IDENTITY` and `SUPERSILVIA_NOTARY_PROFILE`; the keychain is deleted when the
job ends. Without the certificate the `.app` is signed ad hoc, as a build by hand is, and
without the key it is signed and not notarized — each said as a warning on the run. Nothing on
Windows or Linux is signed.

## Licences and notices

The binary carries its own: **Help ▸ About supersilvia** and **Help ▸ Licences…** show
LICENSE, every Rust crate compiled in with its licence text, the repository's
[`licenses/`](../licenses/), GStreamer's terms and the NDI® trademark line
([docs/ui.md](../docs/ui.md#about-and-licences)). Every Linux package also lays the same text
down as files, so it can be read without running anything:

| | where |
| --- | --- |
| `install.sh`, and the AppImage, which is `install.sh` into an AppDir | `<prefix>/share/licenses/supersilvia/` |
| the Flatpak | `/app/share/licenses/io.github.figrita.supersilvia/` |
| the `.app` | `Contents/Resources/licenses/`, with `Credits.rtf` for AppKit's About panel |
| the Windows folder | `licenses/` |

Each holds `LICENSE`, `rust-crates.txt` and `assets/` — the `licenses/` folder — and the
`.app` adds Syphon's and GStreamer's, which it carries; no Linux package carries GStreamer.
The Windows folder adds GStreamer's, and the DirectX Shader Compiler's, which a crate links into
the binary whole; Help ▸ Licences shows the compiler's after the crates, from
[`windows/dxc-LICENSE.txt`](windows/dxc-LICENSE.txt).

**The crates' notices are written by [`../scripts/crate-licenses.py`](../scripts/crate-licenses.py)**
from `cargo metadata --offline`: for Linux into `linux/rust-crates.txt` and for Windows into
`windows/rust-crates.txt`, each committed and compiled in, and for the Mac into the bundle by
`build-app.sh`. When Cargo.lock moves, `tests/notices.rs` fails with the line that writes the
file again:

```sh
scripts/crate-licenses.py --target x86_64-unknown-linux-gnu > packaging/linux/rust-crates.txt
scripts/crate-licenses.py --target x86_64-pc-windows-msvc > packaging/windows/rust-crates.txt
```

## The app id

`supersilvia` is the Wayland app id and the name of the desktop entry.
`io.github.figrita.supersilvia` is the AppStream component id, the Flatpak id and the macOS
bundle id, because those three want reverse-DNS. Flatpak needs the desktop entry and the
icons renamed to the long id, which the manifest does at install time from the same files
`linux/` holds — so there is still one desktop entry in the tree, not two.

## Adding a target

Make a directory next to these, put the recipe in it, and add a row to the table above. If it
needs an image the logo can make, render it in `make-icons.py` rather than exporting one by
hand, so the SVGs stay the only artwork.
