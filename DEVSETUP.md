# Dev environment

Written for a fresh Aurora (immutable Fedora) install. The same steps work on Bluefin,
Silverblue and Kinoite. Non-immutable Fedora and Debian/Ubuntu can skip the container
entirely and run `setup-supersilvia.sh` directly. A Mac has no box at all; see
[macOS](#macos).

## Host vs box

Two machines, one filesystem.

| | Host (Aurora) | Box (`supersilviabox` distrobox) |
| --- | --- | --- |
| Package manager | `rpm-ostree` — **do not use it for this project** | `sudo dnf`, passwordless |
| What runs here | VS Code, podman, distrobox, the KDE desktop | cargo, the tests, the app |
| `$HOME` | `/var/home/<you>` | `/var/home/<you>` — the *same* directory, bind-mounted |
| GPU | AMD, Intel or NVIDIA | the same hardware: same kernel, `/dev/dri` shared |
| Capture devices | the machine's camera and microphone | the same ones: `/dev/video*` and the session's audio server |

The box is not a VM and not an isolated build environment. It shares `$HOME`, the
display, the network, the GPU and the capture devices. That is why `cargo run` inside the box
opens a normal window on the KDE desktop, why the `inspection` feature's server on
`127.0.0.1:5719` is reachable from either side, and why the `camera` and `audioin` nodes find real hardware from in there.

`doctor.sh` checks both devices, as warnings rather than failures: **video device** looks for
`/dev/video*`, and **audio server** looks for a PipeWire or PulseAudio socket in
`$XDG_RUNTIME_DIR`, falling back to an ALSA capture device under `/dev/snd`.
A warning here means the build is fine and one node will find nothing.

It also means anything installed into `$HOME` — `~/.cargo/bin`, `~/.rustup` — is shared
between host and box and survives destroying the box. Only `/usr` is per-box.

Never `rpm-ostree install` a build dependency. Add it to `distrobox.ini` instead.

## Building the box

```sh
git clone <this repo> ~/supersilvia && cd ~/supersilvia
bash setup-supersilvia.sh
```

On an ostree host the script runs `distrobox assemble create --file distrobox.ini`, then
re-runs itself inside the box to install the Rust toolchain and the cargo tools. It is
idempotent: `assemble create` is a no-op when the box already exists.

After editing `distrobox.ini`, rebuild rather than patching by hand:

```sh
distrobox assemble create --replace --file distrobox.ini   # destroys and recreates
distrobox enter supersilviabox
scripts/doctor.sh
```

`scripts/doctor.sh` is the arbiter. It is also the first line of `check.sh`. It picks the
Vulkan adapter the app would render on, by the app's own rule (`render::adapter`): never a
software rasterizer, and otherwise the strongest GPU — a discrete one, NVIDIA included, before
an integrated one — unless `SUPERSILVIA_ADAPTER` names another: `integrated`, a piece of a
name such as `intel`, an index into `vulkaninfo --summary`'s list, or `vendor:device` in hex.
`check.sh` runs it with `SUPERSILVIA_ADAPTER=integrated`, the GPU the tests ask for. The app
itself also reads Preferences ▸ Performance ▸ Use GPU and Allow a software GPU, which
`SUPERSILVIA_ADAPTER` and `SUPERSILVIA_SOFTWARE_GPU` each win over when set. If the only adapter left is `llvmpipe` or `lavapipe`, the GPU is not reaching the box
and nothing about the render pipeline can be trusted — stop and fix that first.

## Working in the box

```sh
distrobox enter supersilviabox
cd ~/supersilvia
./check.sh                # cargo, the tests and the app all run INSIDE the box
```

## VS Code

VS Code runs on the **host** and attaches into the box. It is not installed in the box.

### One-time host settings

Dev Containers shells out to `docker` by default; Aurora has podman. In VS Code's **user**
settings (`~/.config/Code/User/settings.json` — these keys are application-scoped and are
ignored in workspace settings):

```json
"dev.containers.dockerPath": "podman",
"dev.containers.dockerComposePath": "podman-compose"
```

### Attaching

1. Start the box so it appears as a running container: `distrobox enter supersilviabox`.
2. Command Palette → **Dev Containers: Attach to Running Container…** → `supersilviabox`.
3. A new window opens. **It will be running as root.** Fix that before doing anything
   else — see below — then reload.

### remoteUser and workspaceFolder — the part that is not optional

`podman exec` defaults to the container's `USER`, which for a distrobox image is `root`.
Dev Containers inherits that, and it breaks the shared `$HOME`:

- Every file the attached session writes to the shared `$HOME` is created by container
  root, which maps to a **subuid** on the host. The host user then cannot read or
  delete it. `~/.vscode-server` — about 100 MB of server that VS Code downloads on first
  attach — is the usual casualty.

The fix lives in VS Code's global storage, **outside this repo**, so the location cannot
be committed. The *content* can, as a template with three placeholders:
[.vscode/attached-container.json](.vscode/attached-container.json).

```json
{
  "remoteUser": "<you>",
  "workspaceFolder": "<repo>",
  "remoteEnv": {
    "HOME": "<home>"
  }
}
```

Filled in for a user `alice` with the repo at `~/supersilvia`, it reads `"remoteUser":
"alice"`, `"workspaceFolder": "/var/home/alice/supersilvia"` and `"HOME": "/var/home/alice"`.
An attached-container file substitutes no host variables, so the placeholders are filled in
when it is installed (below).

- `remoteUser` is the whole point: attach as your own user, which `--userns keep-id` in
  `distrobox.ini` maps to the same user on the host.
- `workspaceFolder` makes the attached window open the repo instead of `/`.
- `remoteEnv.HOME` keeps `$HOME` at `/var/home/<you>` rather than the container's default
  for that user, so cargo and rustup find their existing state.

VS Code keys this file by **image**, not by container name, and the filename is the image
reference with `/` as `%2f` and `:` as `%3a`:

```
~/.config/Code/User/globalStorage/ms-vscode-remote.remote-containers/imageConfigs/registry.fedoraproject.org%2ffedora-toolbox%3a44.json
```

So **changing `image=` in `distrobox.ini` orphans it.** VS Code does not migrate or warn:
it writes a fresh file containing only `workspaceFolder`, and that attach is root again.
This happens on the first attach after any Fedora bump. Install the committed template from
the host, with its placeholders filled in, instead of retyping it:

```sh
cd ~/supersilvia
img=$(sed -n 's/^image=//p' distrobox.ini)
dest=~/.config/Code/User/globalStorage/ms-vscode-remote.remote-containers/imageConfigs/$(printf '%s' "$img" | sed 's|/|%2f|g; s|:|%3a|g').json
mkdir -p "$(dirname "$dest")"
sed -e "s|<you>|$USER|" -e "s|<repo>|$PWD|" -e "s|<home>|$HOME|" .vscode/attached-container.json > "$dest"
```

Then reload the window. Confirm with `scripts/doctor.sh` — its first two checks are
exactly this failure. Old files for images you no longer use are inert and can be deleted.

### Recovering from a root attach

If an attach already happened as root, `~/.vscode-server` is owned by a subuid. Delete it
and let VS Code re-download it as your own user.

From inside the box (sudo is passwordless there):

```sh
sudo rm -rf ~/.vscode-server
```

Or from the host, without sudo:

```sh
podman unshare rm -rf ~/.vscode-server
```

Then attach again. Any other path root touched on that first attach has the same problem
and the same fix; `~/.vscode-server` is the usual only casualty, but if git or an
extension later reports permission denied on a path in `$HOME`, treat it the same way.

## macOS

An Apple Silicon Mac builds and runs supersilvia in its own shell: no distrobox and no VS Code
attach.

```sh
xcode-select --install                     # linker and clang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env                        # also in the shell profile
rustup show                                # installs rust-toolchain.toml's toolchain
brew install gstreamer pkgconf             # GStreamer 1.24 or newer, found through pkg-config
cargo install cargo-insta --version 1.48.0
scripts/doctor.sh
```

The versions are `setup-supersilvia.sh`'s `PIN_*` lines. **Full Xcode** is needed only
for the Metal compiler that checks the generated MSL ([below](#the-msl-check)): install it
from the App Store and select it with
`sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`.

**GStreamer: Homebrew's or the official framework, never both.** The floor is 1.24, because
`gstreamer-app` and `gstreamer-video` take the `v1_24` feature. Homebrew's `gstreamer` formula
carries every plugin the Mac uses and is on pkg-config's default path. The official
framework, from gstreamer.freedesktop.org (the runtime and the development `.pkg`), is what the
`.app` ships — [below](#making-the-app), from a copy unpacked into a cache and never installed.
Installed for development instead, it needs its own paths in the shell profile, or the build
fails in `glib-sys`:

```sh
export GST=/Library/Frameworks/GStreamer.framework/Versions/1.0
export PATH="$GST/bin:$PATH"
export PKG_CONFIG_PATH="$GST/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

A missing element is invisible at build time. These are the ones the Mac uses:

```sh
for el in videotestsrc decodebin videoconvert appsink mp4mux h264parse h265parse \
          avfvideosrc vtenc_h264_hw vtenc_h265_hw vtdec_hw osxaudiosrc textoverlay; do
  gst-inspect-1.0 "$el" >/dev/null 2>&1 && echo "ok   $el" || echo "MISS $el"
done
```

`avfvideosrc`, `vtenc_*` and `vtdec_hw` are `applemedia`'s, `osxaudiosrc` is `osxaudio`'s, and
`textoverlay` (pango) draws the Text node.

`doctor.sh` takes its macOS branch on Darwin and runs under the system's bash 3.2. It checks
rustc, cargo-insta and `cc` as on Linux, then `xcode-select -p`, the Metal compiler
(a warning), GStreamer 1.24 or newer through pkg-config, the elements
`videotestsrc decodebin videoconvert appsink mp4mux avfvideosrc`, a VideoToolbox pair —
`vtenc_h264_hw vtdec_hw h264parse` or its H.265 twin, a failure as on Linux, since video import
and layer 1's clip tests transcode through it — and a Metal GPU from `system_profiler`. The Vulkan, VA-API, capture-device and declared-package checks are Linux's and do
not run. Each failure names the command that fixes it.

### Making the `.app`

```sh
packaging/macos/build-app.sh
```

It builds the `dist` profile against GStreamer's official release and writes
`dist/supersilvia.app`, `dist/supersilvia-<version>-macos-arm64.dmg` and the same as a `.zip`, for
Apple Silicon Macs on 14.2 or later with nothing installed. The first run downloads the
release's two `.pkg`s (about 900 MB) into `~/Library/Caches/supersilvia-packaging/` and unpacks
them there with `pkgutil`; it installs nothing, needs no `sudo`, and leaves Homebrew's GStreamer
alone — the development build keeps using it. The build has a target folder of its own,
`target/app/`, so it and `cargo build` never rebuild each other's dependencies. Signed ad hoc
unless `SUPERSILVIA_SIGN_IDENTITY` names a Developer ID; `SUPERSILVIA_NOTARY_PROFILE` adds
notarization. [packaging/macos/README.md](packaging/macos/README.md) has what it carries, how
GStreamer finds its plugins inside the bundle, the entitlements and the signing, and
[packaging/macos/TESTERS.md](packaging/macos/TESTERS.md) is the page to send with a build.

### The MSL check

`tests/shader_targets.rs` writes every module the compiler can make as MSL through naga, and
the `gpu_*` suites have Metal compile what wgpu hands it. The offline pass with Apple's own
compiler names the file and line of a failure, which the driver does not. Run it after a change
to a node's WGSL; a failure is fixed in the WGSL, never in the generated text.

```sh
DUMP=$(mktemp -d)
SHADER_TARGETS_DUMP="$DUMP" cargo test --test shader_targets -- --nocapture
for f in "$DUMP"/*.metal; do
  xcrun -sdk macosx metal -c "$f" -o /dev/null 2>/dev/null || echo "FAIL $(basename "$f")"
done
```

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `doctor: FAIL not root` | attached without `remoteUser` — see above |
| `doctor: FAIL $HOME /root` | same |
| `doctor: FAIL Vulkan adapter` naming `llvmpipe` or `lavapipe`, or no device | the box lost `/dev/dri`, or Mesa's Vulkan driver (`mesa-vulkan-drivers`) is missing; rebuild with `--replace` |
| `doctor: WARN VA-API H264 + HEVC` | the VA-API driver for this GPU is missing. It is per vendor and all of them come from RPM Fusion, installed by `init_hooks`: `mesa-va-drivers-freeworld` on AMD, `intel-media-driver` on Intel Gen8+, `libva-nvidia-driver` on NVIDIA. The hook that installs it failed, so rebuild with `--replace` and read the hook output. NVENC/NVDEC work regardless, which is why this is a warning |
| `doctor: FAIL hardware video codec` | **no VA-API or NVENC encoder+decoder pair.** This blocks all of `check.sh`: video import transcodes through the hardware codec and layer 1 tests it, so a box without one cannot run `cargo test`. Fix the VA-API row above, or install NVENC's GStreamer elements. On a Mac the pair is VideoToolbox's, from the `applemedia` plugin Homebrew's `gstreamer` carries |
| `doctor: FAIL GStreamer` | `gst-inspect-1.0` missing; install `gstreamer1-devel` (it is declared in `distrobox.ini`, so rebuild with `--replace` rather than by hand) |
| `doctor: FAIL GStreamer elements` | a named element is absent — `v4l2src` lives in plugins-good, the VA-API encoders in plugins-bad-free. Missing at run time, invisible at build time; rebuild with `--replace` |
| `doctor: WARN video device` | no `/dev/video*`: the box cannot see a camera, or the machine has none. The `camera` node will find nothing; nothing else is affected |
| `doctor: WARN audio server` | no PipeWire or PulseAudio socket in `$XDG_RUNTIME_DIR` and no ALSA capture device under `/dev/snd`. The `audioin` node will find nothing; nothing else is affected |
| permission denied under `~/.vscode-server` | root attach; `podman unshare rm -rf ~/.vscode-server` |
| attached as root again after an image bump | the image-keyed attach config was orphaned; reinstall it (see above) |
| `doctor: FAIL Vulkan adapter` saying `SUPERSILVIA_ADAPTER=integrated names nothing` | `check.sh` asks for the integrated GPU, as the tests do, and this machine offers none: name the GPU the gate should use instead, `SUPERSILVIA_ADAPTER=radeon ./check.sh`, and add `nvidia=true` to `distrobox.ini` if it is NVIDIA, so the container can see its driver |
| `cargo` not found after entering the box | `~/.cargo/env` not sourced; re-run `setup-supersilvia.sh` |
