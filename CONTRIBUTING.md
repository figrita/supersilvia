# Contributing to supersilvia

supersilvia is a native modular video synthesizer: Rust, egui/eframe 0.36 and wgpu, on Vulkan
on Linux, Metal on macOS and Direct3D 12 on Windows. This page is the working reference for changing it:
how to build and test, the rules the code keeps, the traps the tests have, and how comments
and docs are written.

**Read [docs/](docs/) before changing anything structural.** It is the spec: what the system
is and why. Start at [docs/README.md](docs/README.md) and
[docs/architecture.md](docs/architecture.md).

Two sibling folders hold writing that is *not* the spec. [proposals/](proposals/) holds things
argued into a shape and not built. [meditations/](meditations/) holds thinking that has not
converged. Neither describes the system. Do not implement from a meditation without turning it
into a proposal first.

A **timeline and viewport** over one parameter address space is designed and not built, in
[docs/decisions.md](docs/decisions.md). Read it before touching `Node::controls`.

## Setting up

[DEVSETUP.md](DEVSETUP.md) has the whole of it. In short:

- `setup-supersilvia.sh` prepares a machine. On an immutable Fedora host (Aurora, Bluefin,
  Silverblue, Kinoite) it builds the `supersilviabox` distrobox from
  [distrobox.ini](distrobox.ini), and all work happens inside it
  (`distrobox enter supersilviabox`). The box shares `$HOME`, the display and the GPU with
  the host. Never `rpm-ostree install` a dependency: add it to `distrobox.ini` and rebuild
  with `distrobox assemble create --replace --file distrobox.ini`.
- A Mac builds in its own shell, with Homebrew's GStreamer. DEVSETUP.md has the steps.
- `scripts/doctor.sh` decides whether the environment is usable, and it is the first line of
  `check.sh`.

**Never accept a software adapter.** If the GPU you see is llvmpipe, lavapipe or SwiftShader,
the environment is broken: a green run on it says nothing about the render pipeline. The app,
every GPU test and `doctor.sh` take the adapter by one rule, `render::adapter::choose`: never
a software one unless `SUPERSILVIA_SOFTWARE_GPU=1`, and otherwise the strongest GPU, a
discrete one before an integrated one. `SUPERSILVIA_ADAPTER` names another: `integrated`, a
piece of a name such as `intel`, an index, or `vendor:device` in hex.

The tests and benches ask for the integrated GPU themselves (`adapter::Asked::integrated`),
because the image snapshots are an Intel iGPU's pixels, and `check.sh` exports
`SUPERSILVIA_ADAPTER=integrated` for the doctor's check. On a machine with no integrated GPU,
name the one the gate should use: `SUPERSILVIA_ADAPTER=radeon ./check.sh`.

## Commands

```sh
./check.sh                              # the gate before every commit; must be green
cargo run                               # native window, on the most recent project
cargo run friday/                       # a project folder, or any path inside one
cargo run tunnel.ssw                    # a loose workspace file, imported into a project of its own
cargo run -- --check                    # what this machine gives the app, and what it lacks
cargo run --release                     # before measuring anything about performance
cargo test                              # test layers 1-3
cargo insta review                      # accept changed WGSL/text snapshots
UPDATE_SNAPSHOTS=1 cargo test --test ui # accept changed UI image snapshots, on Linux only (docs/testing.md)
cargo test --test shader_targets -- --nocapture   # the WGSL gate, and how many nodes have WGSL
cargo test --test gpu_ring              # the renderer's ring: zero flash, feedback, throttle, churn
cargo test --test gpu_app               # the whole App on the GPU: Snap, the render job, a live recording, a loop closing to the byte
cargo test --test ndi                   # NDI®: a picture sent and received in one process; skips without the NDI runtime
cargo test --test syphon                # macOS: a picture published over Syphon and received by its own client
cargo xwin test --target x86_64-pc-windows-msvc --no-run --test gpu_d3d12   # Windows: the Direct3D 12 import, run under Wine or on Windows
cargo build --profile dist              # the shipping binary: lto, one codegen unit, function names kept
cargo run --release --example graph_bench   # what the graph costs per frame, by graph size
cargo run --release --example tick_bench -- <project> <tab>   # one tab ticked at 100 Hz, headless
cargo run --release --example editor_bench -- <project> <tab> [WxH]   # the editor's own painting on the GPU, headless
cargo run --release --example loop_gifs -- <project> [--only name,name] [--out dir]   # every tab's Output for one loop, as a GIF
cargo run --release --example queue_contention   # the editor's latency beside a synthetic synth
python3 scripts/demo-time.py            # the Time Gears demo project, into <documents>/supersilvia/Time Gears
packaging/appimage/build.sh             # Linux: target/supersilvia-<version>-linux-x86_64.AppImage, and its symbols beside it
packaging/appimage/symbolize.sh <log> <symbols>   # a panic in the AppImage's log, its frames named (packaging/README.md)
packaging/macos/build-app.sh            # macOS: dist/supersilvia.app, its .dmg and .zip
git tag v<version> && git push origin v<version>   # all three downloads, built on GitHub into a draft release (packaging/README.md#releases)
scripts/crate-licenses.py --target x86_64-unknown-linux-gnu > packaging/linux/rust-crates.txt   # after Cargo.lock moves
scripts/crate-licenses.py --target x86_64-pc-windows-msvc > packaging/windows/rust-crates.txt    # and Windows' too
```

`check.sh` runs the doctor, `cargo fmt --check`, `cargo clippy --all-targets --all-features
-D warnings` and `cargo test`. A pull request is expected to pass it.

The `inspection` feature exposes egui's inspection protocol on `127.0.0.1:5719`, for a tool
that drives the running app (`EGUI_INSPECTION=1 cargo run --features inspection`). It is never
on by default and never in a release build.

## Hard rules

[docs/invariants.md](docs/invariants.md) says which of these a machine catches and which rest
on you noticing.

**`unsafe` lives in a fixed list of modules, and every block carries a `// SAFETY:` line.**
The crate root denies `unsafe_code`, and `clippy::undocumented_unsafe_blocks` requires the
comment. These modules are allowed it, each where its parent module declares it:

- `render::dmabuf` — the DMA-BUF, `IOSurface` and Direct3D 12 imports through wgpu-hal
- `render::picture`, its Linux half: `thread` and `wayland` — the picture windows' borrowed
  `wl_display` and the raw surface handle made on it
- `platform::linux::filedrop` — file drops on Wayland, on a queue of its own on eframe's
  borrowed `wl_display`
- `platform::linux::ndi` — the NDI® runtime opened by its path through `libloading`
- `platform::macos::audio` — Core Audio's property reads, and the loopback's process tap
- `platform::macos::screen` — ScreenCaptureKit, and the classes it calls back into
- `platform::macos::pixels` — a `CVPixelBuffer` locked and read as a frame
- `platform::macos::gpu` — the IORegistry's GPU statistics
- `platform::macos::menu` — the menu bar's items and actions
- `platform::macos::syphon` — Syphon's framework classes, declared by hand
- `platform::windows::check` — the kernel's version, read by `RtlGetVersion`
- `platform::windows::clock` — the thread's CPU times, read by `GetThreadTimes`
- `platform::windows::dirs` — the shell's Known Folders, and the strings it hands back
- `platform::windows::fonts` — DirectWrite's system font collection, through COM
- `platform::windows::d3d12` — GStreamer's Direct3D 12 library, declared by hand

`tests/rules.rs` holds every `#[allow(unsafe_code)]` to this list. A new one fails the test
until it is added there, here and in docs/invariants.md. Outside the crate,
`examples/queue_contention.rs`'s hal device, `tests/gpu_surface.rs`'s bare `CAMetalLayer` and
`tests/gpu_d3d12.rs`'s textures reached through wgpu-hal are the only others.

The rest:

| Rule | Why |
| --- | --- |
| Nothing outside `nodes/` matches on a node slug | `NodeDef::is_output`, read through `Node::def`, and `PortDef::delayed` carry the two bits that escape |
| A `VaryingNumber` port is a *field* — `fn f(uv: vec2f) -> f32`, one value per pixel. A number the CPU holds is a `UniformNumber` | a CPU node's inputs are `UniformNumber`, never `VaryingNumber`, because `tick` has no `uv`; a `UniformNumber` feeds a `VaryingNumber` for free and the reverse is a node |
| A tap measures its input over the unit square, in its workspace's pass, and reads back a frame late | the measurement is a property of the input rather than of the coordinate a consumer asked at; anything that waits on the GPU on the frame thread stalls the picture |
| A `tick` never waits | device threads publish through a triple buffer, or a one-frame slot whose lock `tick` only tries; `tick` reads the newest and moves on |
| `graph/`, `compile/`, `nodes/`, `audio/`, `video/` take no graphical dependency, named or reached through any path the crate offers (`tests/rules.rs`) | they are tested with plain `cargo test`; `emath` is fine, it is math not graphics; GStreamer core is bytes, not GL |
| `ui/` never mutates: every surface draws and returns its actions — the canvas `Command`s, the menu `MenuAction`s, the tab bar `TabAction`s, the project tab `ProjectAction`s (`tests/rules.rs`) | the command bus is the only mutation path, and what the UI can ask for stays enumerable |
| Anything that is not the same on Linux, macOS and Windows, outside `render/`, is a service of `platform/`, and an operating system's crates are named in its own backend alone (`tests/rules.rs`) | each backend answers the same names, so no machine's build is surprised by another's dependencies; `Cargo.toml` declares each crate for its own machine only |
| wgpu renders on the adapter `render::adapter::choose` picks, and nowhere else | the app, every GPU test, every bench and kittest's shared device go through it, so the software-adapter refusal and `SUPERSILVIA_ADAPTER` hold everywhere |
| `vendor/wgpu-core` is wgpu-core 30.0.1 with one match arm changed, and nothing else in it is edited | a surface configures while the synth submits, which 30.0.1 refuses with a panic; the copy goes on the first release carrying gfx-rs/wgpu#10296 ([vendor/README.md](vendor/README.md)) |
| A control value is never baked into generated WGSL | it is a uniform; see the recompile boundary in [docs/architecture.md](docs/architecture.md) |
| Nothing in `nodes/` names `u_resolution`, `frag_coord` or the position builtin | a node whose field depends on the Output's size is a different field in each Output. A size in pixels divides by `nodes::REFERENCE_HEIGHT`; a registry test holds every generator to it |
| Every color in the graph is premultiplied: a node's return value, a texture it samples, an Output's frame. Straight colors — pickers, PNGs, GIFs, the drawing canvas, files written out — are converted once at the boundary. A linear operation runs on the color as it is; anything nonlinear in rgb goes through the prelude's `unpremultiply` and `premultiply`; a composite is `fg + bg · (1 − fg.a)` | linear filtering, a lerp and a blur are exact on premultiplied colors and fringe on straight ones; see [docs/decisions.md](docs/decisions.md#colors-in-the-graph-are-premultiplied) |
| **Zero flash**: no edit may show a frame nobody asked for | a rebuild keeps rendering the old program, and a resize carries the last frame across. When you add something that reallocates, answer *what is on screen while this happens* first; "briefly nothing" is not an answer. See [docs/rendering.md](docs/rendering.md) |

## Open an issue first

Open an issue and agree the change before you:

- add a dependency that is not already in `Cargo.toml`;
- restructure the crate into a workspace;
- change the worldspace convention or the shader prelude's standard uniforms, since every
  node depends on both.

## Testing

Four layers, described in [docs/testing.md](docs/testing.md). The traps, in short:

- **Never `Harness::run()`** in kittest. The UI never settles, because it is a synth. Use
  `h.step()` or `h.run_steps(n)`.
- **One wgpu `Instance` per test process**, through `tests/common/gpu.rs`: two threads in
  `vkCreateInstance` at once segfaulted the Vulkan loader. It also hands every test the
  integrated GPU and a device that panics on any validation error.
- **Never create a shader module or a pipeline on the synth or the frame thread.** Links go to
  the `linker` threads (`render::link`). A creation returns once the driver has compiled,
  which is the wait.
- **The first frame an `Area` appears is a sizing pass.** egui lays its content out once to
  measure it, then discards that pass and runs the frame again. kittest runs exactly one pass
  per `step()`, so the accessibility tree after that step is the *provisional* geometry and a
  click made from it lands nowhere. Give a popup another `step()` before reading the tree, as
  `add_node` does.
- **A double-click cannot be simulated with two `click()`s in kittest.** Its clock advances
  0.75 s per interaction and egui's `max_double_click_delay` is 0.3 s. Put both press/release
  pairs in one frame, as `double_click` in `tests/ui.rs` does.
- **kittest's `click()` does not reach every clickable thing.** The panel spines, which are an
  `allocate_exact_size` with `Sense::click()`, never see it: push the pointer events by hand.
  And egui hands a click to the *last* `interact` that asked for it, so a mark on a node
  registered before the header's or the body's own response loses the click to the drag
  handle. Register it after, where the `?` and `✕` are.
- **`Context::output` is empty after a `step`.** A test reading `h.ctx.output(...)` reads the
  *next* frame's. `Harness::output()` holds the last frame's `FullOutput`; the cursor icon is
  `h.output().platform_output.cursor_icon`.
- **A panel slides open, and is not resizable while it slides.** A test that unfolds a panel
  and immediately probes its edge finds no handle: `run_steps` until it lands.
- **An accessible name must be unique on screen.** `get_by_label` fails with "found two or
  more". Rename one; do not reach for `query_all` and an index.
- **`InputState::modifiers` is only updated by `Event::ModifiersChanged`.** A test that pushes
  an Alt-flagged press alone looks right and does nothing. `alt_click_at` in `tests/ui.rs`
  sends the `ModifiersChanged` on either side of the press, which is what a window manager
  does.
- **Consume the most specific keyboard shortcut first.** egui rejects a shortcut only when the
  *pattern* needs a modifier that is not held, so `Ctrl+Z` matches a `Ctrl+Shift+Z` press.
  `Ctrl` is the exception the other way: a pattern of `Modifiers::NONE` rejects a press with
  `Ctrl` held, so a key that means one thing bare and another with `Ctrl` is consumed with the
  *held* modifiers as its pattern, as `number.rs` does for the arrows.
- **Run `cargo test`, not `cargo check`,** while iterating. A bug shipped once because the
  test that caught it had been written and never run.
- **Measure before optimizing**, on a release build. The Status box (View ▸ Status box) shows
  CPU ms, dropped frames, counts, each CPU node's line, and GPU ms per Output from the
  timestamps around its pass, which is the only figure there that describes GPU headroom.
- **Video pipelines are tested with `videotestsrc`**, a real pipeline through the real sink;
  whether a camera exists is a fact about the machine. Audio is tested through the pure
  `Analyzer`. No camera and no microphone is opened by `cargo test`, but the clip tests do
  drive a real hardware encoder and decoder, so layer 1 needs a codec pair: VA-API or NVENC on
  Linux, VideoToolbox on a Mac. `doctor.sh` fails without one, before `check.sh` reaches
  `cargo test`.
- **UI image snapshots are Linux's.** A Mac's pixels fail the Linux gate, so accept them with
  `UPDATE_SNAPSHOTS=1` on Linux only.
- **Look at the real app.** Several bugs so far were invisible to every test and obvious on
  screen within seconds. Undo had eight passing tests and its keyboard binding was still
  wrong; a cable fix passed the tests written for it and still vanished behind a node.

## Documentation

`docs/` is the source of truth and describes only what is true *now*.

- **A change in behavior and a change to the doc belong in the same commit.**
- **Purge superseded ideas** rather than annotating them. Git holds the history; commit
  messages carry the reasoning.
- An idea someone would reasonably propose again gets one line in
  [docs/decisions.md](docs/decisions.md) saying why it lost.

## Comments

Three kinds, and the rule differs by kind:

- **A module doc (`//!`) may carry the why for that module**: the rule it implements, the
  shape that lost, the silvia behavior it keeps or departs from. The person opening the file
  is the person who needs it. It does not repeat what `docs/` says at length; a sentence and
  the name of the doc is enough where the argument is there.
- **An item or inline comment is factual: what, not why.** Reasoning at that grain belongs in
  the module doc, in `docs/` or in the commit message.
- **No temporal references anywhere.** Not "now", not "used to", not "yet", not a milestone
  number. A comment describes the code beside it as it is; git says what it was.

Write the name lowercase everywhere: supersilvia.

## The icon, and shipping

The logo is two SVGs in `assets/icon/`. Every PNG, the `.ico` and the `.icns` beside them are
rendered from those by `scripts/make-icons.py`, which needs nothing installed. Edit an SVG,
re-run it and commit what it writes; no build step does it for you. `src/main.rs` bakes the
256 into the binary. Everything else is installed, and matched to a window by its app id:
`supersilvia` for the editor, and `supersilvia-popout` or `supersilvia-fullscreen` for a
picture window. Each id is a desktop entry, and the entry is the only place a Wayland desktop
looks. [packaging/](packaging/) holds the rest: the desktop entries and `install.sh`, the
AppImage, the Flathub manifest and the macOS `.app`
([packaging/macos/README.md](packaging/macos/README.md)). The licences travel inside the
binary, under Help ▸ Licences, and beside it as files in every package
([packaging/README.md](packaging/README.md#licences-and-notices)).
