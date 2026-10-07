# supersilvia

supersilvia is a modular video synthesizer for live performance.
You wire nodes together on a canvas, and the result is drawn live as a picture.
It is for VJs, live visuals and anyone who wants to play with video synthesis.
It is the native successor to [silvia](https://github.com/figrita/silvia), a browser-based
video synth.

<!-- screenshot -->

## Status

supersilvia is pre-release software. The current version is 0.9.0-alpha.4.

- Things change and break without notice.
- The project file format can still change, and nothing converts old projects. Keep a render
  of anything you care about.
- Builds for Linux, macOS and Windows are on the
  [Releases](https://github.com/figrita/supersilvia/releases) page. You can also build it from
  source.

## What it does

- **A graph compiles to one shader.** Each node is a function, and each cable is a function
  call. A node's output is a field, not a frame, so the graph has no resolution until an
  Output draws it.
- **More than 160 nodes** in eleven families: sources, generators, color, transform, effects,
  conversions, taps, math, control, gears and outputs. The generators include shapes,
  patterns, gradients, fractals, noise, and simulations such as a slime mold, cellular automata
  and a playable brick game.
- **Control signals travel on cables too.** Oscillators, clocks, envelopes, sequencers,
  counters, slews and an XY pad feed any number on any node. A tap measures a picture and
  turns it back into a number.
- **Feedback.** An Output's last frame can be fed back into the graph, for trails and echoes.
- **Time you can play.** Every node that moves runs free at a Speed you turn or cable, or
  locks to a clock. A Master Gear is the show's clock; Ratio Gears turn it into others, 3 : 2
  or 4 : 1, forwards or backwards, locked for good. A seek or a render puts every clock exactly
  where the playhead says.
- **Loops you can count on.** A Master Gear's caption says how many of its cycles bring
  everything it drives back to the start, exactly, or says it can't tell.
- **Live inputs.** Cameras, video files, still images and GIFs, screen capture, a
  microphone with band analysis, MIDI controllers, game controllers and the mouse.
- **NDI®** to receive and send video over the local network, on Linux, macOS and Windows,
  with or without transparency.
- **No internet.** supersilvia never connects to the internet: no update checks, no
  analytics, no accounts. It uses the local network only for NDI, and only when you send,
  receive or list NDI sources.
- **Syphon** to receive from and send to other apps on the same Mac.
- **A two-deck mixer** with an A/B crossfade, Blackout and Freeze.
- **Picture windows.** Any picture can pop out into its own window or go fullscreen on a
  projector.
- **Workspaces.** A project holds several graphs as tabs, and cables can cross between them.
- **Rendering.** An Output can render to a video file through the GPU's hardware encoder, a
  PNG sequence or an animated GIF, a frame at a time so nothing is dropped. Snap saves one
  full-size picture.
- **Live recording.** An Output can record what it shows as you play, to a video file.
- **MIDI mapping.** Alt + click a number or a button to bind it to the next MIDI message.

## Platforms

| Platform | Download | State |
| --- | --- | --- |
| Linux, x86_64 | `supersilvia-<version>-linux-x86_64.AppImage` | Main platform. Needs a Vulkan GPU driver. Pop-out and fullscreen windows need a Wayland session. |
| macOS on Apple Silicon | `supersilvia-<version>-macos-arm64.dmg` | Runs on Metal. Needs macOS 14.2 or later. Signed and notarized. Intel Macs are not supported. |
| Windows, 64-bit | `supersilvia-<version>-windows-x86_64.zip` | Runs on Direct3D 12. Needs Microsoft's Visual C++ Redistributable (x64). Newer and less tested than Linux and macOS. Screen capture offers the main monitor only, and there is no Syphon. |

[packaging/linux/TESTERS.md](packaging/linux/TESTERS.md) and
[packaging/macos/TESTERS.md](packaging/macos/TESTERS.md) walk through installing a download.

A software renderer such as llvmpipe is refused. supersilvia needs a real GPU.

## Building from source

You need [Rust](https://rustup.rs) (stable; `rust-toolchain.toml` selects it) and
GStreamer 1.24 or newer with its development files.

**Fedora.** These are the build packages from [`distrobox.ini`](distrobox.ini):

```sh
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config clang-devel \
  libxkbcommon-devel libxkbcommon-x11-devel libxcb-devel wayland-devel openssl-devel \
  mesa-dri-drivers mesa-vulkan-drivers vulkan-tools libva-utils \
  gtk3-devel alsa-lib-devel systemd-devel fontconfig-devel freetype-devel libva-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good \
  gstreamer1-plugins-bad-free pipewire-gstreamer
```

Importing a video file needs a hardware H.264, HEVC or AV1 encoder and decoder. Fedora leaves
those codecs out; [RPM Fusion](https://rpmfusion.org/Howto/Multimedia) provides them.

**Other Linux distributions** need the equivalent packages: a C toolchain and clang,
xkbcommon and Wayland headers, ALSA, libudev, fontconfig, the Mesa Vulkan driver, and
GStreamer's development files with the base, good and bad plugins and PipeWire's plugin.

**macOS:**

```sh
xcode-select --install
brew install gstreamer pkgconf
```

Then build and run:

```sh
cargo run --release
```

`cargo run --release -- --check` reports what the machine is missing, such as GStreamer
plugins, a hardware codec, the GPU or the NDI runtime, and exits. Preferences ▸ Performance ▸
Use GPU picks a GPU when there is more than one, from the next start; `SUPERSILVIA_ADAPTER` wins
over it, for example `SUPERSILVIA_ADAPTER=integrated`.

NDI needs NDI's free runtime from [ndi.video](https://ndi.video). Nothing proprietary is needed
to build.

[DEVSETUP.md](DEVSETUP.md) has the full development setup, including a container for
immutable Fedora and the macOS details. [packaging/](packaging/) has the recipes for an
AppImage, a Flatpak, a macOS `.app` and the Windows folder. A tag `v<version>` pushed to GitHub
builds the downloads into a draft release.

## Running the tests

```sh
cargo test        # the tests
./check.sh        # the full gate: environment check, rustfmt, clippy, then the tests
```

The tests draw on a real GPU, the integrated one by default. On a machine without one, name
another, for example `SUPERSILVIA_ADAPTER=radeon cargo test`. The video tests need the hardware
codec described above. `check.sh` first runs `scripts/doctor.sh`, which checks the setup that
DEVSETUP.md describes. Shader snapshots are reviewed with [cargo-insta](https://insta.rs), and
UI snapshots are redrawn with `UPDATE_SNAPSHOTS=1 cargo test --test ui`.

[docs/testing.md](docs/testing.md) explains the four test layers.

## Design documents

[docs/](docs/) describes how supersilvia works now and why. Start with
[docs/README.md](docs/README.md) and [docs/architecture.md](docs/architecture.md).
[docs/nodes.md](docs/nodes.md) lists every node, and [docs/ui.md](docs/ui.md) covers the
editor. [proposals/](proposals/) holds plans that are not built yet.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report bugs on
[GitHub Issues](https://github.com/figrita/supersilvia/issues).

## Licence

supersilvia is licensed under the GNU Affero General Public License, version 3 or later. See
[LICENSE](LICENSE).

LICENSE begins with an additional permission under section 7: you may convey supersilvia
combined with the proprietary NDI® runtime library without including that library's source.

Third-party notices for code, fonts and icons used in supersilvia are in
[licenses/](licenses/). The Rust crates' notices are in
[packaging/linux/rust-crates.txt](packaging/linux/rust-crates.txt). The running app shows all
of them under **Help ▸ Licences…**, and on a Mac in the About window.

NDI® is a registered trademark of Vizrt NDI AB.

## Credits

supersilvia follows [silvia](https://github.com/figrita/silvia), figrita's browser-based
video synthesizer. Many of its nodes and much of its interface come from silvia.

The people who helped make silvia are thanked in [THANKS.md](THANKS.md). Shader code and
assets from others are credited in the source and in [`licenses/`](licenses/).
