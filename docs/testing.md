# Testing

Four layers. Each catches a class of bug the others cannot, and all four existed before any
node code was written.

## 1. `cargo test` — pure logic

`graph/`, `compile/`, `nodes/`, `audio/` and `video/` have no GPU and no egui, so they are
ordinary unit and integration tests. The generated WGSL is snapshot-tested with `insta`, and
`tests/shader_targets.rs` takes every WGSL module the compiler can write — each workspace
pass among them, and one holding every node in the registry — and every stage and
kernel the wgpu renderer writes for itself, through naga to MSL, SPIR-V and HLSL, so a module
Metal, Vulkan or Direct3D 12 would refuse is a failure here. Seconds, not
minutes — nearly all of it `video/clip.rs` driving a real hardware encoder; everything else
is instant. **Reach for this first, always.**

Review a new WGSL snapshot by eye once, then accept it; from then on the snapshot is the
contract.

No camera and no microphone is opened here. `audio::Analyzer` is pure and is what the audio
tests drive; `video::Camera` is tested against `videotestsrc`, which is the real pipeline and
the real sink with only the source swapped. `video::clip` and `tests/video.rs` encode a
synthetic clip through the real hardware encoder, transcode it into a scratch cache and play
it in both directions, so they do need a hardware codec pair — which `doctor.sh` checks for
before `check.sh` gets as far as `cargo test`. The clip is encoded with `Codec::probe()`'s
pair, the one an import transcodes with: `vah264enc` on Linux with an Intel GPU and `vtenc_h264_hw`
on a Mac. Every available codec round-trips, and every buffer out of each one's intra chain is
a keyframe. Where the machine has no pair, each of those tests prints `no hardware codec pair
here; skipping` and passes; `--nocapture` shows the line. The cache is the project's own
`cache/` folder, handed to the player and the transcode as a path, so a test hands them one in
the system temp folder, or a headless `App`'s scratch project's, and a run leaves nothing in
`~/.cache` or in any real project.
`tests/uniform.rs` ticks a headless `App` and reads back what a CPU node published;
`tests/actions.rs` does the same for the event half, with `App::press` as the hand on a
button and `App::edges` as what a port fired. `tests/suspend.rs` is what a closed workspace
does and does not do.

**The time model** has three files of its own, each headless. `tests/transport.rs` is the
playhead: a pause holds everything, a seek lands every gear where the playhead puts it and
fires nothing on the way but the beat it lands on, a tab reopened after a minute is in phase with one left open, a
stall is advance and a clamped step, a render is the same twice and exact at any frame rate,
and a project opens playing at zero whatever its file says — nothing of the transport is
saved. `tests/clocks.rs` is the gears: a Ratio Gear its parent times its Teeth to the bit,
played, sought, sought away and back or rendered, forwards, in Reverse and with its Offset
added, a Teeth or direction change landing at once on the new product, a reversed gear's
beats going down, an Offset swaying a gear, 3 : 1 of 1 : 3 being the input, a Ratio Gear on a Master Gear bending on a length
change, a seek re-birthing every gear and a reset starting a cycle, a cabled clock's wrap and a
0 to 1 Phase into Clock In, the Master Gear's Trigger, the Time node, and an oscillator on a
gear with its Offset added. `tests/generators.rs` is ambient time: an unplugged Time is the playhead at
the node's rate, the same moment reads the same whatever the path, a pause holds it, a gear in
Time replaces it, Repeat and the tunnel's wrap bring it round, and Repeat rebuilds. Beside them,
`tests/compile.rs` holds a time-driven node to reading Time and adding Offset and a field in
Time to a type mismatch (`a_field_cannot_drive_time`); `tests/actions.rs` a sequencer driven
by a gear, its jump closing an open gate; `tests/video.rs` and `tests/pictures.rs` a clip and a
GIF played by Time with Offset added, forwards and back under a gear, holding on pause; and
`nodes::chain`'s own test a loop of a Master Gear as every chain's denominator.
`tests/ui.rs` holds the time readout's pause, reset and fixed width, View ▸ Time, and a gear's
region in both displays; `tests/gpu_nodes.rs` and `tests/gpu_app.rs` hold the pixels
([below](#3-the-gpu--the-actual-pixels)).

`tests/workspace.rs`, `tests/project.rs`, `tests/assets.rs` and `tests/autosave.rs` are the
files on disk: one workspace, the folder holding them, the media copied into it, and the
unsaved edits kept beside them, with the autosave's clock handed in by hand and each write
waited for; `tests/painting.rs` is a `drawingcanvas`'s picture across all four, from what its
tick publishes to the PNG a save writes. Every test there writes
under its own directory in the system temp and hands the project an explicit root, and
`App::headless` gets a scratch project and in-memory preferences, so nothing in the suite can
reach a real project or a real `preferences.json`. `tests/preferences.rs` is the one file
that reads and writes a preferences file, under `SUPERSILVIA_PREFERENCES`.
`tests/crashlog.rs` is the one that starts the log as `main` does, in a process of its own
since the logger is the process's one, and in a folder of its own under the system temp
(`crashlog::start_in`), so no test reaches the real log.

`tests/notices.rs` holds Help ▸ Licences to the build: on Linux it runs
`scripts/crate-licenses.py --check`, which writes the crate notices again from
`cargo metadata --offline` and fails where Cargo.lock has moved away from
`packaging/linux/rust-crates.txt`, and on every machine it holds the `licenses/` folder to
what `ui::about::ASSETS` compiles in. It needs `python3`.

```sh
cargo insta review
```

## 2. `egui_kittest` — UI behavior, headless

`tests/ui.rs` drives the app through the AccessKit tree — find a widget by label, click it,
run frames, assert on state — with no window and no GPU of the app's own. `tests/canvas_keys.rs`
does the same for the canvas's keys and the two cable gestures, and `tests/menus.rs` for the
menu bar's own help — undo by name, the Undo History, the shortcuts window, a greyed entry's
reason. Each writes out again the few helpers it needs and takes no snapshot, so none of their
harnesses reaches a GPU and they need none of the sharing below.

Two rules this imposes:

- **Every interactive thing must be a real egui `Response` with `widget_info`.**
- **The app must run with `cc.wgpu_render_state == None`.** `App::headless()` is that state;
  the renderer is simply absent, the synth runs inline and the preview draws a placeholder.

**Never use `Harness::run()`.** It repaints until the UI settles and supersilvia's never does —
`App::ui` calls `request_repaint` unconditionally, because it is a synth. Use `h.step()` or
`h.run_steps(n)`.

```sh
UPDATE_SNAPSHOTS=1 cargo test --test ui
```

Image snapshots render through wgpu on this GPU, so they may differ on other hardware. The
PNGs in `tests/snapshots/` are an Intel iGPU's pixels (Mesa, on Linux), and they are the only set:
Linux compares against them at kittest's defaults, a per-pixel threshold of 0.6 with no
failing pixel. A Mac compares against the same PNGs at a threshold of 2.0, which
`kittest.toml`'s `[mac]` section sets and which touches no other OS. Apple's GPU draws the
editor with scattered single pixels on the edges of text and curves off by up to 1.5 on that
scale, and nothing else. A moved or recoloured widget is far above 2.0, so a Mac still fails
on a real change. Never write a Mac's pixels over the PNGs (`UPDATE_SNAPSHOTS=1` on a Mac):
the Linux gate would then fail on every edge a Mac drew differently. A snapshot is accepted
on Linux; a Mac only compares. **What the machine offers is pinned, so the PNGs hold
no machine's answers**: the harness's app (`app` in `tests/ui.rs`) reads NDI® as not installed
and Syphon as absent, whatever the machine has — `video::ndi::pretend_missing` and
`platform::syphon::pretend_unavailable`, test hooks that hold for the calling thread alone,
which is where kittest runs the editor and its inline synth. An Output's NDI row says *runtime
not installed* and no Output, Main Mixer or Nodes menu carries Syphon, on a box with the NDI
runtime and on a Mac alike, so a Mac draws the snapshots Linux wrote. The tests about sending
out and about a Syphon source — `ndi_sends_an_output_by_its_row_and_the_mix_by_its_mark`,
`syphon_publishes_an_output_by_its_row_and_the_mix_by_its_mark` and
`a_macs_syphon_source_reads_as_macos_only_where_there_is_none` — take `machine_app`, which
answers as the machine does, waiting for the NDI probe first, and take no snapshot: on a Mac
they cover the Syphon row, the mark and the source list, and on a box with the runtime, NDI's
**Send** and **Stop**. The Preferences window names the file manager, Files or Finder, which
nothing pins, so `the_preferences_window_says_where_things_are_kept_and_what_it_draws_on`
takes its two snapshots on Linux alone and a Mac checks its labels.

**One wgpu device serves the whole file.** kittest builds a renderer per `Harness`, and
`WgpuTestRenderer::new` creates a wgpu instance, adapter and device of its own every time it
runs. libtest runs tests on as many threads as the box has cores, so two of the snapshotting
tests reach their first snapshot together: one thread sits inside `vkCreateInstance` while the
other, already past it, walks the Vulkan loader's handle tables from
`vkSetDebugUtilsObjectNameEXT` — `wgpu_hal::vulkan`'s `create_bind_group_layout` naming a
descriptor set layout — and reads the tables the creation is writing.
`loader_get_icd_and_device` in `libvulkan.so.1` is frame 0, and the whole process dies with
`SIGSEGV` after a different passing test each time, passing on a rerun. `shared_gpu()` in
`tests/ui.rs` holds one instance, adapter and device in a `OnceLock` and hands every harness a
`WgpuSetup::Existing` over it, so `vkCreateInstance` happens once, with no other thread in the
loader, and nothing destroys it either. `cargo test --test ui -- --test-threads=1` is the
experiment that names the mechanism: serial runs are green where parallel ones are not.

**The shared device is on the integrated GPU, which the harness asks for by name**:
`render::adapter::choose` with `Asked::integrated`, which is `SUPERSILVIA_ADAPTER=integrated`
unless the variable already names an adapter. It is a test's choice and not the app's, whose
default is the strongest GPU, a discrete one where there is one. So on Linux an Intel iGPU
(Mesa) and on a Mac its Apple GPU, which wgpu also reports as integrated, and never a software
rasterizer. kittest's own selector prefers a software adapter first and a discrete GPU second,
so left alone it drew on llvmpipe and would draw on an NVIDIA card wherever the NVIDIA Vulkan
driver loads. `the_test_adapter_is_the_igpu` holds the choice.
The snapshots are the iGPU's pixels.

That is a fault in the test process, not an exit. **A run that ended because a window was
closed is not this.** Closing a window ends `cargo run`, leaves a zero status and writes no
core; this is `signal: 11, SIGSEGV` under `cargo test` with a core dump behind it. There is no
`gdb` in the development container, so read the core through systemd: `coredumpctl list` names the `ui-<hash>`
binary and `coredumpctl info <pid>` prints a stack trace per thread, the faulting one first and
every other parked in a wait.

Sharing the instance costs two conditions of its own, and both are in `tests/ui.rs`. It offers
**the app's own backends and never GL**, `render::adapter::BACKENDS` — Vulkan on Linux, Metal
on macOS, Direct3D 12 on Windows — because every harness's renderer enumerates the instance's adapters and a GL
adapter's `AdapterContext` clones share one EGL context — made current on two threads at once
that is a `BadAccess` unwrapped in `wgpu_hal::gles::egl`, a panic in one test rather than a
fault in the process. Which also means the snapshots need a Vulkan or Metal device on the
box. And a `SETUP` mutex holds that enumeration to one
thread at a time, since it is the only work left that reaches into the shared instance.

## 3. The GPU — the actual pixels

`tests/gpu_*.rs` drive the renderer on a real wgpu device with no window and no display
server, on the integrated GPU, as the snapshots are — `render::adapter::choose` with
`Asked::integrated`, never a discrete GPU and never a software rasterizer unless told, and
`the_harness_renders_on_the_igpu` in `gpu_ring.rs` holds it. The benches in `examples/` ask the
same, `tests/check.rs` runs `--check` with `SUPERSILVIA_ADAPTER=integrated`, and `check.sh`
exports it for the doctor. By area: `gpu_ring.rs` (the ring, zero
flash, feedback, the throttle, a saturated GPU, churn read off wgpu's own counters, a viewer on
another thread reading the frame it holds),
`gpu_readback.rs`, `gpu_upload.rs`, `gpu_mixer.rs` (the mix and the viewer's blit),
`gpu_sims.rs`, `gpu_link.rs`, `gpu_timing.rs`, `gpu_dmabuf.rs`, `gpu_d3d12.rs` (Windows: the
Direct3D 12 import, built with `cargo xwin` and run under Wine, where vkd3d reports an iGPU as
discrete, so `SUPERSILVIA_ADAPTER=intel` names it), `gpu_nodes.rs`,
`gpu_render.rs`, `gpu_surface.rs` (a surface configured while the synth submits, on a Mac,
where a bare `CAMetalLayer` needs no window) and `gpu_lost.rs` (an uncaptured error logged
rather than fatal, and a loss forced with `Device::destroy` reaching the callback); `gpu_nodes.rs` holds Offset
adding to Time, a field into Offset making a ripple, a gear in Time being the node at the
gear's reading and the tunnel coming back after its period; `gpu_app.rs` is the whole `App` on
the GPU — a Snap into `snaps/`, the render job, a time-driven picture the same at one moment
whatever came before, a loop as long as its Master Gear says closing to the byte, the tunnel's
flight closing every cycle, a clip rendered for its whole play coming back to its first
frame, the slime mold's app half, idle Outputs drawn on demand, egui_wgpu painting the editor's
own frame to the pixel, the editor timing its own painting, and a workspace pass
thumbnailing ports that reach no Output, each cell its own coordinate to the bit, while it
measures a tap on the same workspace with no Output.

`examples/loop_gifs` is the loop check on a real project, headless: each tab's Output through
its ordinary render for as long as its Master Gear says a loop is — the only Master Gear
upstream of the Output, else the first by id, else `--length`, else the Output's own Duration —
after `--pre-roll` loops of warm-up, three by default, and with one frame more, frame `F` compared with frame zero for the seam it prints, and
frames `0..F` written as a GIF on one shared palette into the project's `renders/`.

- **One `Instance` and one `Adapter` per test process**, in a `OnceLock` in
  `tests/common/gpu.rs`, which every GPU test file takes with `#[path = "common/gpu.rs"] mod
  gpu;`. Two threads in `vkCreateInstance` at once segfaulted the Vulkan loader — the same
  fault as layer 2's, above. A device per test from that adapter.
- **Every test device panics on an uncaptured error**, so a validation error anywhere fails
  the test that caused it.
- **Software rasterization is rejected.** A green run on llvmpipe or lavapipe says nothing
  about the pipeline this project is building. `SUPERSILVIA_SOFTWARE_GPU=1` with
  `SUPERSILVIA_ADAPTER=llvmpipe` opts CI without a GPU in; never by default.
- **Prefer point assertions over image diffs.** "Every pixel is exactly one of the two
  control colors" catches blended, premultiplied or half-transparent output, which squinting
  at a screenshot does not.

This layer is also where a generated module is **made into a pipeline on the real driver**. A
snapshot proves the text is what you meant to write; only the driver proves it runs. It is
also where a published frame is uploaded and read back through a consumer's shader, which is
the camera path minus the camera, and where a tap's slot is read back after a real draw: a
solid gray measures its level to the bit, with every fragment counted once.

`tests/crosstalk.rs` holds that a picture only ever holds its own node's pixels. Two CPU
sources whose colors cannot be mistaken for each other — the brick game's grey and the
automaton's, with no blue — are read back wherever they land: the source texture, the
Output drawing it, and a viewer on another thread showing both beside every Output. The
shapes are the live app's: the synth's renderer on a thread of its own on the one device with
a viewer churning textures and buffers as egui does, Outputs flipping between inputs, a project
opened over another whose ids name other kinds, and a tab closed and opened again.

## 4. Driving the running app

Attach to the app and drive the real window — query the widget tree, click, drag, screenshot.
Screenshots include the pictures the synth draws, so this is how you see what it actually
renders.

Needs a **visible window**: an occluded or minimized window does not paint, and the request
times out.

Layers 2 and 4 both go through AccessKit, so the labeling discipline of layer 2 is what
makes layer 4 pleasant. **Treat the accessibility tree as the agent's API.**

## Habits that have paid

- **Run `cargo test`, not `cargo check`, while iterating.** A hex-parsing bug shipped because
  the test that caught it had been written but never run.
- **Measure before optimizing.** View → Costs puts each Output's GPU time and each node's
  evaluations per pixel on the canvas, which is where the multiplier that sinks a patch is
  found. The Debug overlay showing CPU milliseconds is what turned
  "layout feels wasteful" into "1.07 ms, and here is where it goes". The GPU line beside it
  is the other half: `gpu output3: 1.8 ms (worst 2.4)` per Output, from the timestamps at the
  beginning and end of its pass, because the CPU figure stays flat while the GPU runs out of headroom and the first
  thing you hear about it is a dropped frame.
- **Look at the real app.** Six things so far were invisible to every test and obvious within
  seconds of using it: a missing `#version` line, a scissor box clipping an off-screen render,
  a color picker that opened nothing, a zoom that collapsed the framerate, `Ctrl+Shift+Z`
  undoing twice instead of redoing, and a backward cable bowed by the wrong quantity.

  The last two say the most. The undo *model* had eight passing tests and the *binding* was
  wrong — testing the unit does not test the wiring. And the cable's first fix passed the
  tests written for it, because those tests asserted the shape of the curve rather than
  whether it cleared a node that is tall for reasons the curve knew nothing about. **A test
  can only check the thing you thought of.**
- **When a screenshot contradicts your reasoning, check you are looking at the current
  binary.** A replaced binary's `/proc/*/exe` reads `... (deleted)`, so a kill loop matching
  the plain path silently misses it and leaves a stale instance holding the port.
