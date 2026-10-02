# Proposal: what ships, and how it gets built

**Status: the metadata is in, and two builds exist.** [`packaging/`](../packaging/) holds
the desktop entry, the MIME type, the AppStream metainfo, a Flatpak manifest, the AppImage
build (`packaging/appimage/`), the macOS `.app` build (`packaging/macos/`), and every icon
rendered from [`assets/icon/`](../assets/icon/) by `scripts/make-icons.py`; `main.rs` bakes
the 256 in and sets the app id a Wayland compositor matches the entry by. What is still not
built is CI and a release pipeline: there is no `.github/` and nothing is published to any
channel. This document was written before the macOS port, when `src/` assumed Linux
throughout; the OS-specific code now lives in `src/platform/` ([platform.md](platform.md)).

## The shape of the problem

supersilvia is not a self-contained binary. [distrobox.ini](../distrobox.ini) is its real
bill of materials, and it is long: GStreamer core plus `plugins-base`, `-good` and
`-bad-free`, `pipewire-gstreamer`, the VA-API vendor drivers, and a full ffmpeg swap because
Fedora strips H.264/HEVC. A user's machine has none of that guaranteed.

Worse, the dependency splits in two along a line that decides everything:

- **Userspace we can carry.** GStreamer, its plugins, ffmpeg, GLib. Version-pinned, ours to
  build, ours to ship. `Cargo.toml` asks for the `v1_24` feature on `gstreamer-app` and
  `gstreamer-video`, so the floor is GStreamer 1.24 — above what the Steam
  Runtime's own base carries, which settles the question of whether to vendor. There is no
  version of this that uses the host's GStreamer.

- **Driver-bound userspace we cannot.** `libva` dlopens the vendor driver, which is built
  against the host's Mesa and LLVM. [render/dmabuf.rs](../src/render/dmabuf.rs) dlopens the
  host `libEGL` deliberately, and says so. These follow the kernel driver and can never be
  carried in a depot.

Every packaging decision below is really a decision about how that second category reaches
the process. That seam is the only genuine risk in this document; the rest is labor.

## The four artifacts

| Artifact | Built how | Ships as |
| --- | --- | --- |
| **Flatpak** | `org.freedesktop.Platform` 24.08, GStreamer from the runtime | Flathub; Deck desktop mode |
| **Linux bundled tree** | old-glibc container, GStreamer vendored via Cerbero | Steam depot, itch, AppImage, direct download |
| **Windows bundled tree** | MSVC, official GStreamer MSVC binaries | signed installer, Steam depot, itch, portable zip |
| **Source tarball** | `cargo package` plus a spec file | COPR, AUR, Nix, build-from-source |

Four builds. Nine or so channels. The collapse matters: the Steam Linux depot and the
portable Linux download are byte-identical but for a launch script, and the Windows depot and
the Windows installer wrap the same tree.

### Flatpak is the best Linux answer and should be first

Not because it reaches the most people — it does not — but because it is the only option that
solves the seam instead of deferring it. The Freedesktop runtime supplies GStreamer at a
version above our floor, so nothing is vendored. `org.freedesktop.Platform.GL.*` supplies
Mesa and the VA drivers matched to the host, which is the hard problem, already solved by
people who do it full time. `Platform.ffmpeg-full` carries the codecs Fedora strips.

Three things we already did land here for free:

- `rfd` is built on `xdg-portal` with the GTK backend deliberately off
  ([Cargo.toml](../Cargo.toml)). Portals are how a sandboxed app opens a file. The choice was
  made for KDE theming and it pays again here.
- Every file that reaches a node is copied into `assets/`, so portal-mediated access — where
  the app gets a handle, not a path — is the access pattern the project model already wants.
- DMA-BUF import works under Flatpak unchanged.

The cost is the manifest and the sandbox holes: `--device=dri` for the GPU, `--socket=pulseaudio`
or PipeWire for `cpal`, and a decision about `/dev/video*` for the `camera` node, which portals
do not yet mediate well.

### The Linux bundled tree is the tedious one

Vendoring is a solved problem — GStreamer ships **Cerbero** to do exactly this, and it is what
produces the official binaries — but it is a week of fiddling and a permanent tax:

- Relocatable prefixes: `GST_PLUGIN_SYSTEM_PATH` and `GST_PLUGIN_SCANNER` set relative to the
  install directory by a launch script. `src/` has no `GST_*` handling at all today; this is
  new code, and small.
- `GST_REGISTRY` must point somewhere per-user and writable, or every launch rescans every
  plugin, visibly.
- Build against an old glibc or the binary refuses to start on anything but a rolling distro.
  This is the same container the Steam depot wants, which is why these are one artifact.
- ffmpeg is ours to security-patch forever. This is the cost people forget: the distro was
  doing that work for free and now it is a standing obligation on a shipped product.

The seam is unhelped here. Nothing supplies host drivers; we get whatever `libva` finds.

### Windows is a port, not a research project

Covered in full by the platform analysis, but the short list: `rfd`'s `xdg-portal` feature
moves under `[target.'cfg(unix)'.dependencies]`; the XDG paths in
[preferences.rs](../src/preferences.rs) and [project.rs](../src/project.rs) get a known-folder
branch; [render/dmabuf.rs](../src/render/dmabuf.rs) is `cfg`-ed out and video falls to the
bytes path, which already exists as a first-class `Delivery` and is exercised; the `CODECS`
table gains `nvh264enc`, `qsvh264enc` and the `d3d11` decoders. GL 4.3 and fragment atomics
are fine on WGL.

The `camera` node needs real work rather than a translation: [camera.rs](../src/nodes/camera.rs)
hardcodes `/dev/video0` through `/dev/video3` as literal option strings. It should enumerate
through `gst::DeviceMonitor`, which is the correct fix on Linux too and stops the node
offering three devices that are not there.

### The source tarball is somebody else's channel

AUR and Nix happen without us given clean versioned tarballs. COPR is ours and nearly free:
a spec built by `rust2rpm`, Fedora's own build infrastructure, `dnf copr enable`. An Ubuntu
PPA is the same shape of work if it ever seems worth it.

Official Fedora is a real possibility later — Fedora permits bundled Rust deps for leaf
applications and ships GStreamer 1.24+ — and needs only a packager, which could be us. The
Debian archive is not worth pursuing: its Rust policy wants every crate packaged separately,
which this dependency tree makes absurd.

## The codec table should prefer AV1

[clip.rs](../src/video/clip.rs) orders `CODECS` by vendor — all the VA elements, then all the
NVIDIA ones — so `probe()` takes `vah264enc` first. Two reasons to reorder by codec instead:

**Patents.** AV1 is royalty-free by AOMedia's non-assert. H.264's foundational patents have
largely expired. HEVC is fragmented across Via LA, Access Advance and unaffiliated holders,
and its patents run into the 2030s. The order should walk from least encumbered to most.

**The cache is all-intra.** Every entry sets `key-int-max=1` or `gop-size=1`, which switches
off exactly the inter-frame tooling HEVC's advantage over H.264 comes from. On a regenerable
local cache, the remaining gap buys nothing worth the licensing surface.

Proposed order: `av1` (va), `nv-av1`, `h264` (va), `nv-h264`, `h265` (va), `nv-h265`.

HEVC stays, last, because the comment at `clip.rs:96` is right that some Intel parts have
HEVC encode without H.264, and dropping it strands them. It becomes the fallback it should
always have been rather than the second choice. On RDNA3, Ada and Arc we land on AV1 and the
question evaporates; on a Steam Deck (RDNA2, no AV1 encode) we land on H.264, the expired one.

A vendor-grouped order also has a bug in it: on an NVIDIA card with `libva-nvidia-driver`
installed, `vah264enc` can win over `nvav1enc` on a card that does AV1. Ordering by codec
fixes that as a side effect.

**Also worth re-testing:** Fedora has been re-enabling H.264 in stock Mesa as the patents
expire. The RPM Fusion `mesa-va-drivers-freeworld` swap in [distrobox.ini](../distrobox.ini)
may be partly obsolete. If it is, the development setup gets simpler and the "Fedora will not
package this" objection gets weaker.

## CI is the actual project

Four artifacts across two platforms is not something anyone does by hand twice. There is no
`.github/` today, and it should exist before the second channel rather than after the fifth.

Two workflows:

**On every push** — the gate. `./check.sh` on Linux, which is doctor, `fmt --check`, `clippy
--all-targets --all-features -D warnings` and `cargo test`. The trap is that
[scripts/doctor.sh](../scripts/doctor.sh) refuses `llvmpipe` and CI has no GPU. The container
already declares `xorg-x11-server-Xvfb` for this, and the headless GL tests need a `PBUFFER`
config; doctor needs a documented CI mode that accepts a software renderer for the layers that
do not touch the GPU while still failing locally, or the GPU-dependent layers get their own
job on a runner that has one. This is the first thing to work out and it is not obvious.

**On a tag** — the release. Build the four artifacts, sign the Windows installer, push the
Steam depot with `steamcmd`, attach the tarballs to a GitHub release, submit the Flatpak.
One pipeline, or releases stop happening.

The Windows build needs a code-signing certificate. Without one SmartScreen throws a wall at
every user, and reputation accrues slowly even with one, so it wants buying early.

## What changes in `src/`

Small, and none of it structural:

| Change | Where | Why |
| --- | --- | --- |
| `GST_PLUGIN_SYSTEM_PATH`, `GST_PLUGIN_SCANNER`, `GST_REGISTRY` set before `gst::init` | a new `platform` module | a vendored GStreamer is not found otherwise |
| Known-folder paths on Windows | [preferences.rs](../src/preferences.rs), [project.rs](../src/project.rs) | two functions, both already isolated |
| `rfd` feature split by target | [Cargo.toml](../Cargo.toml) | `xdg-portal` pulls `ashpd`, which is Linux-only |
| `dmabuf` behind `cfg(target_os = "linux")` | [render/mod.rs](../src/render/mod.rs) | the bytes path already exists |
| `DeviceMonitor` camera enumeration | [camera.rs](../src/nodes/camera.rs) | hardcoded `/dev/videoN` is wrong on every platform including this one |
| `CODECS` reordered by codec | [clip.rs](../src/video/clip.rs) | above |

The `platform` module is the only new file, and the rule it should carry is the one the rest
of the codebase already follows: nothing outside it asks what OS it is on.

## `doctor.sh` should ship

It is already the best diagnostic in the project, and a Linux app with a GPU dependency
will generate support mail whose first question is always the same. Shipping it as a
user-runnable `--doctor` flag, printing a paste-able report, turns "it does not work" into a
transcript. The `llvmpipe` check alone will answer a fair share of tickets before they are
sent.

## Sequencing

All of this happens during the alpha, where a broken artifact costs an afternoon rather than
a release. That is the right order regardless: none of it can be verified without other
people's machines.

1. **CI on every push**, with the headless-GL question answered. Nothing else is safe without
   it, and it is what makes alpha builds exist at all.
2. **The Deck test.** Borrow a Deck, run the existing Linux binary under the Steam Runtime
   with a vendored GStreamer, and find out whether VA-API survives the container's driver
   passthrough. This gates any Steam build and costs a day.
3. **Flatpak, served from a repo of our own.** The best Linux artifact and the one needing no
   vendoring. Our own repo rather than Flathub proves every mechanic — sandbox, GL extensions,
   updates — before anything is submitted to Flathub.
4. **The Windows port**, into the same pre-release stream. A second OS of packaging risk that
   wants months of alpha rather than a release-week discovery.
5. **The bundled Linux tree.** Shared by Steam, itch, AppImage and direct download.
6. **`--doctor`**, once alpha reports show what gets asked repeatedly.
7. **COPR and the source tarball.** Cheap, and it lets other people carry the rest.
8. **Code signing and the installer.** Buy the certificate early — SmartScreen reputation
   accrues with use, so a certificate that has been signing alpha builds for a year is worth
   more on release day than one bought the week before.

## Open questions

- **Does VA-API survive the Steam Runtime's driver passthrough?** A Steam build rests
  on this and it cannot be answered from here. Step 2 exists to answer it.
- **What does CI do without a GPU?** A software renderer contradicts doctor's most important
  check. Either doctor gains a CI mode, or the GPU layers need a runner with a GPU, and that
  costs money.
- **Does the `camera` node work under a Flatpak sandbox?** Portals do not mediate V4L2 well.
  The likely answer is `--device=all`, which is a wide hole to open for one node.
- **Is the RPM Fusion swap still needed on Fedora 44?** If not, both the dev setup and the
  packaging story get simpler.
- **Windows zero-copy.** `WGL_NV_DX_interop2` over D3D11 shared textures is the equivalent of
  the DMA-BUF path. Worth it eventually; not worth blocking a first Windows build on.
