# Proposal: Syphon

**Status: approved and built** — the framework, publishing and receiving are in (`docs/rendering.md` and `docs/media.md`, *Syphon*), and `packaging/macos/build-app.sh` embeds and signs the framework in the `.app` with its licence in the About panel. Users have asked for Syphon. This says what it is, how other
software does it, the ways supersilvia could, and which one is built, with the decisions. Everything here was read in Syphon's source, other projects'
source and their documentation; nothing was built or run. A claim that could not be checked that
way says **(unverified)**.

## What Syphon is

**Syphon lets apps on one Mac hand each other live pictures, with no copy and no delay.** One app
*publishes* a picture; any other app on the same Mac can *receive* it and use it as a source,
every frame, as if it were a camera. The picture never leaves the GPU. It is Mac only and local
only: no network. It is open source (BSD licence), by Tom Butterworth and Anton Marini, with
Metal support contributed by Millumin (https://syphon.info/).

**Why people want it.** Live-visuals people chain specialised tools. A typical rig: supersilvia
makes the picture, **Resolume** or **VDMX** mixes it with clips, **MadMapper** maps it onto the
building, **OBS** streams it. Each hop is a Syphon link. Without Syphon, supersilvia is the end of
the chain; with it, it is one instrument in a rig. About seventy apps support it, among them
Resolume, VDMX, MadMapper, Millumin, Modul8, TouchDesigner, Max/Jitter, Isadora, QLab, CoGe,
Processing, openFrameworks, Unity (KlakSyphon), Blender (an add-on) and OBS (receiving is built
in; sending needs a plugin).

**The words.**

- A **server** is one published picture. An app with several outputs publishes one server per
  output, each with a **name** ("Composition", "Screen 1").
- A **client** receives one server's picture.
- The **server directory** is the live list of every server on the Mac. Receiving apps show it as
  a menu of "App – Server" entries, such as "Arena – Composition".

**What users expect of an app that "supports Syphon"**, from the apps' own documentation:

1. Its output is published by one toggle, under a clear name. Resolume's *Composition* is the
   model: switch it on and it appears in every other app's source list.
2. An app with several outputs publishes each as its own named server.
3. It lists the servers on the Mac and takes any of them as an input.
4. Alpha passes through, or at least can.
5. Nothing needs installing: support is built into the app.

## How it works underneath

- **One shared picture per server.** The server owns a single `IOSurface` — GPU memory the system
  can share between processes — always 8-bit BGRA, recreated only when the size changes. Clients
  look it up by its ID. There is no double buffering: a client reads the same memory the server
  draws into next.
- **Discovery by system-wide notifications.** A server announces itself (`info.v002.Syphon.
  ServerAnnounce`) with a description: its unique ID, its name, the app's name. A starting app asks
  everyone to announce again; a server that does not answer within six seconds is dropped.
- **Frames by message ports.** Each server and client opens a Mach message port named after its
  ID. A client subscribes; on each new frame the server sends "new frame", and a new surface ID
  when the size changed.
- **8 bits, no colour tags, alpha unspecified.** The surface is plain `BGRA8Unorm`; nothing says
  whether it is sRGB or linear, or whether alpha is premultiplied. In practice the bytes are
  display-encoded, like a window's.
- **Upside down.** Surfaces are laid out the OpenGL way, bottom row first. A Metal or wgpu app
  flips on the way out and on the way in. Getting it wrong is the most common Syphon bug report.

Two facts shape everything below:

- **No release has Metal.** The last binary release, "SDK 5", is from 2019 and is OpenGL only.
  Metal support has been in the source since January 2023 and is maintained, but every app that
  uses it — OBS, fosfora, the Rust crates — builds the framework from source at a pinned commit.
- **It is being modernised.** A `feature/1.0` branch (May 2026) makes Metal primary, deprecates
  OpenGL and adds optional 16- and 32-bit float surfaces with colour hints. It is not merged. And
  the shared surfaces rely on "global" IOSurfaces, which Apple deprecated in 2015 but has not
  removed; the maintainers plan new plumbing before Apple does.

## How other software does it

- **Most apps link Syphon.framework** and call its server and client classes (OBS, ofxSyphon,
  Processing, Max). OBS builds the framework from a pinned commit, embeds it in its `.app`, and
  receives by subclassing the framework's documented `SyphonClientBase` to get the raw `IOSurface`,
  which it turns into a texture on either renderer.
- **KlakSyphon (Unity) links nothing.** It compiles thirteen of Syphon's own source files into its
  plugin, makes its own shared `IOSurface`, wraps it as a Metal texture Unity draws into, and
  announces it through Syphon's own classes. That is the framework's documented "subclassing"
  recipe, done without the framework's Metal server.
- **In Rust** there are several attempts, all from 2026:
  - `syphon-core`/`syphon-wgpu` (BlueJayLouche) targets wgpu 30 and ships a prebuilt framework,
    but binds it through the old `objc`/`cocoa` crates, waits on the GPU on the CPU before each
    publish, and copies each frame twice.
  - `syphon-rs` (raycaster-io) is objc2 0.6 and can build the framework, but uses a device and
    queue of its own and waits for each frame to finish.
  - `fosfora`, a Rust VJ app, loads the framework at run time so the app runs without it, and its
    release workflow builds, embeds and signs it — the closest thing to a recipe for our `.app`.
  - `sp2-syphon` reimplements the protocol in pure Rust and has never been run on a Mac.

  None fits supersilvia as it stands: each either waits on the GPU, brings a second set of
  Objective-C crates, or is untested.

## What supersilvia already has

More than most apps start with:

- **The objc2 crates** the Mac media work uses, at the versions wgpu 30 uses, so binding Syphon's
  classes needs no new Rust crate.
- **IOSurface ↔ wgpu, both ways in principle.** `render/dmabuf.rs` already wraps an `IOSurface` as
  a wgpu texture (`newTextureWithDescriptor:iosurface:plane:` → `texture_from_raw` →
  `create_texture_from_hal`) for clips and screens. Pointing that at Syphon's surface with render
  usage is how we would draw into it.
- **The blit Syphon needs.** A picture window's `Viewer` already turns an Output's 16-bit float
  frame, stored bottom row first, into 8-bit BGRA, top row first, fitted to a size, with the same
  bytes the editor shows. Syphon wants exactly that frame, flipped once more.
- **A thread shape that never touches the synth.** A macOS picture window reads the newest
  published frame, blits it, submits, and presents — all off the synth. A Syphon server is the
  same thing with an `IOSurface` where the window's surface would be.
- **A way in for received frames.** Screen capture writes frames from a system thread into slots a
  `Camera` adopts without a pipeline, as `Pixels::IoSurface`. A Syphon client would do the same.

## The routes, and the one recommended

### Publishing

- **A. Our shared surface, the framework's announcement (recommended).** Make a server from
  Syphon's base class, ask it for its shared surface, wrap that surface as a wgpu render target
  the way clips and screens are wrapped, draw each frame into it with the picture windows' blit
  (flipped), and tell the server "published" once the GPU has finished. This is the framework's
  own subclassing recipe and KlakSyphon's design. It needs no raw Metal command buffer, no second
  queue, no extra copy, and not the framework's Metal shader library, which is the part that
  breaks in source builds. It runs on a thread of its own, like a picture window, so the synth
  never waits.
- **B. The framework's Metal server.** Hand it our texture on a wgpu command buffer and let it
  copy. Simpler to call, but it is a second pass over the frame (we must convert to 8-bit BGRA
  first anyway), it needs wgpu's raw command buffer on an encoder of its own, and it needs the
  shader library.
- **C. An existing crate** (`syphon-core`/`syphon-wgpu`). The least of our own code, but a new
  dependency with its own Objective-C crates, a CPU wait before each publish, and two copies.
- **D. Reimplement the protocol** in Rust with no framework. Nothing to embed or sign, but the
  protocol is not a public contract, and a change on Syphon's side would break us silently.

### Receiving

**One route stands out:** the framework's client base class hands over the server's `IOSurface`
on each new frame (OBS's approach). Wrap it as `Pixels::IoSurface` and adopt it the way a screen
capture's frames are. Two details:

- **Old servers report pixel format 0.** Frameworks built before October 2025 leave the surface's
  format unset. The importer accepts only BGRA and NV12 today, so it must read 0 as BGRA, as OBS
  does.
- **The server keeps drawing into the same surface.** Sampling it where it lies can show a torn
  frame. Copying it into a texture of ours each frame costs one GPU copy and never tears.
  Recommend the copy.

### Linking the framework

The framework must be built from Syphon's source; there is no Metal release to download.

- **Vendored prebuilt (chosen).** Build `Syphon.framework` once, for arm64 alone, from a pinned
  commit, and keep the result under `vendor/` beside `vendor/wgpu-core`, with a README saying how
  to rebuild it. A small `build.rs` (supersilvia has none yet) links it and sets the search path,
  so `cargo run` and `cargo test` find it; the `.app` carries it in `Contents/Frameworks`.
- **Built by `build.rs`** with `xcodebuild`, as `syphon-rs` does. No binary in the repo, but every
  clean build needs Xcode and adds a minute.
- **Syphon's sources compiled into our binary**, as KlakSyphon does. Nothing to embed or sign, but
  it compiles Objective-C in our build — the second language `proposals/macos-media.md` rejected
  for the media work.
- **Loaded at run time**, as fosfora does. The app runs without the framework, but a missing
  framework is then a silent absence.

## How it would look in the app

- **Publishing an Output**: a *Syphon* tick on the Output node. Its server is named after the
  Output (its title, e.g. "Output 1"); the app name is "supersilvia". A published Output is drawn
  even when nothing on screen shows it, so the render plan gains a reason to draw beside "a window
  shows it".
- **Publishing the mix**: a *Syphon* mark in the Main Mixer's **Window** row, beside the pop-out
  and fullscreen marks, server name "Mix".
- **Receiving**: *Syphon* in the Main Input's video sources, with a menu of servers by "App –
  Server", kept live from the directory; later perhaps a Syphon node like Screen Capture.
- **Receiving, more than one**: a **Syphon** node, like Screen Capture, with the server menu as its
  option, so a patch can take several at once.
- **What another app sees**: the same bytes the editor shows, 8-bit, at the Output's resolution,
  the right way up by default, opaque over black as a picture window is.
- **Two toggles on each side.** Sending and receiving each carry a *Flip* and a *Transparent*
  tick. The defaults are the popular ones: the Syphon convention's orientation, which OBS, fosfora
  and ofxSyphon follow, and opaque, which KlakSyphon and OBS default to. A rig that disagrees ticks
  its way out.

## Costs and risks

- **One more blit per published picture per frame**, on the GPU, off the synth. The same cost as a
  pop-out window of the same size.
- **8 bits only.** Values above 1 clip, as they do in a window. Syphon's unmerged 1.0 adds float
  surfaces; adopt them when it ships.
- **Global IOSurfaces are deprecated.** If Apple removes them before Syphon replaces them, every
  Syphon app breaks together, and rebuilding the framework is the fix.
- **Not in a sandbox.** Syphon cannot work in a sandboxed app, so a Mac App Store build could not
  have it. Outside the store, nothing changes.
- **Signing.** The embedded framework is signed with our identity first, then the app; with our
  Team ID on both, library validation passes **(unverified whether the
  `disable-library-validation` entitlement is still needed)**.
- **Discovery needs the main run loop.** eframe runs one, so announcements are answered; a test
  that wants to see a server must pump it.
- **Teardown.** A server must not be released while a publish is still on the GPU; one Rust
  project crashed that way. Stop it, wait for the last frame, then let go.

## Linux, and the other platforms

- **Linux has no Syphon.** The modern equivalent is a **PipeWire video source**: an app publishes a
  node of class `Video/Source`, other apps (OBS, GStreamer, browsers) see it like a camera, and
  frames can travel as DMA-BUFs with no copy. ossia score shipped exactly this in September 2026.
  supersilvia already speaks PipeWire for screen capture. The cheap first version is the capture
  readback into `appsrc ! pipewiresink`, one GPU-to-CPU copy per frame; the zero-copy version
  needs Vulkan memory export, which is new `unsafe` in `render::dmabuf`.
- **v4l2loopback** (a kernel module making a fake webcam) is what people use today; it is CPU
  memory and has no alpha.
- **Windows** has Spout, Syphon's twin; supersilvia has no Windows build.
- **NDI** sends pictures over the network between machines. It needs a proprietary SDK with
  licence obligations, encodes every frame, and is a different feature: `proposals/ndi.md`.

Recommend: build Syphon behind a platform service that answers on both systems, with Linux
refusing — as the Mac once did for everything — and bring PipeWire to Linux as a proposal of its
own.

## Decisions

1. **The framework is vendored**: built once for arm64 from a pinned commit, kept under `vendor/`,
   linked by a small `build.rs`. Apple Silicon only; no older macOS than the app's 14.2.
2. **Each Output publishes**, by a tick on the node, and so does the mix, by a mark in the Main
   Mixer.
3. **The Output's tick is saved** with the project; the mix's mark is not.
4. **Orientation and transparency are toggles** on both sides, defaulting to the popular choice.
5. **Receiving is both** the Main Input and a Syphon node, since rigs want several inputs.
6. **Linux is its own work**, later. Windows and NDI are out of scope.

## The build, in commits

1. **The framework and one server.** The vendored framework, `build.rs`, the objc2 bindings in one
   module with the `unsafe` allowance, and a test that publishes a test pattern and receives it in
   the same process through Syphon's own client, checking the bytes and that it is the right way
   up. That test settles the orientation and colour questions without opening anything.
2. **Publishing Outputs and the mix.** The publisher thread, the Output tick and its two toggles,
   the mixer mark, the render plan's new reason to draw, docs.
3. **Receiving.** The directory listing, the Main Input source and the Syphon node, the copy into
   our texture, pixel format 0, the two toggles.
4. **In the `.app`.** Embedding, signing and the licence notice, with the rest of packaging.

**What the loopback test showed** (`tests/syphon.rs`, commit 1). A picture drawn into the
server's surface with the default look reaches Syphon's own client in the same process with its
bottom row first in the surface's memory and alpha 255, a half-covered pixel as it would be over
black; flipped and transparent, top row first with the picture's own premultiplied alpha. The
surface's pixel format is `BGRA`, which this framework commit sets. The directory lists the
server as "<process name> – <server name>" — the app name is the process's own, so the binary's
`supersilvia` — and drops it when the server stops. The loopback proves the bytes and the
layout; which layout another app draws upright is its own reading, and Simple Client draws row 0
at the bottom (`Common/SimpleImageView.m`), so the default is the one it shows upright. The
framework could not be built with `xcodebuild` on the Mac it was built on (its first-launch resources
were stale), so `vendor/syphon/build-framework.sh` builds it with clang, as `vendor/README.md`
says.

## Sources

- Syphon: https://syphon.info/ · https://github.com/Syphon/Syphon-Framework (`SyphonServerBase.m`,
  `SyphonMetalServer.h/.m`, `SyphonSubclassing.h`, `SyphonPrivate.h`, `SyphonServerDirectory.m`,
  `Syphon.docc/ExtendingSyphon.md`) · issues #10 (sandbox), #24 and branch `feature/1.0` (float
  surfaces), #47 (global IOSurfaces), #64 and #97 (no Metal release), #76 (alpha), #105 (the
  shader library) · https://developer.apple.com/documentation/iosurface/kiosurfaceisglobal
- Apps: https://resolume.com/support/en/syphonspout ·
  https://docs.madmapper.com/madmapper/6/11.-live-performance-and-control ·
  https://docs.derivative.ca/Syphon_Spout_Out_TOP · https://github.com/obsproject/obs-studio
  (`plugins/mac-syphon`) · https://github.com/keijiro/KlakSyphon
- Rust: https://github.com/BlueJayLouche/syphon-rs · https://crates.io/crates/syphon-rs ·
  https://github.com/kevinraymond/fosfora · https://github.com/MikanseiLaboratory/sp2-rs ·
  https://github.com/naporin0624/linguine
- Linux: https://github.com/ossia/score/pull/2270 ·
  https://docs.pipewire.org/page_dma_buf.html · https://wiki.archlinux.org/title/V4l2loopback
- Spout and NDI: https://github.com/leadedge/Spout2 ·
  https://docs.ndi.video/all/developing-with-ndi/sdk/licensing
