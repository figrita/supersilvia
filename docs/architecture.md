# Architecture

## The one idea

**A graph is an expression, not a pipeline.**

Most node-based tools pass textures: each node owns a framebuffer, renders into it, hands the
result downstream. That design pays for itself in machinery — buffer pools, allocation
strategy, format negotiation, a scheduler deciding who renders when.

supersilvia compiles instead. Every node output is a WGSL function. Every connection is a
function call. Compiling a graph means printing one expression, and the whole compiler is a
recursive descent from an Output's input.

Everything below follows from that.

### Four kinds of value, and one asymmetry

| | lives | shape |
| --- | --- | --- |
| `VaryingNumber` | GPU | `fn f(uv: vec2f) -> f32` — a field, one value per pixel |
| `VaryingColor` | GPU | `fn f(uv: vec2f) -> vec4f` — a field |
| `UniformNumber` | CPU | one `f32` per frame |
| `UniformColor` | CPU | one `[f32; 4]` per frame |
| `Action` | CPU | an event, many-to-many, never compiled |

Four of the five sit on two axes, and `PortType::kind` and `PortType::rate` hand each back: a
**kind**, number or color, and a **rate**, varying or uniform. Every cell of the two-by-two
is filled. `Action` is an event, and an event has no rate.

A field is meaningful only inside a shader, where `uv` exists. A `UniformNumber` is exactly
the kind of value that fits in an `f32` on the CPU, and it is the same thing a number control
already is — one float, constant across the frame, uploaded as a uniform — promoted to
something a cable can carry. A meter, a MIDI knob, an audio band and a slew are all uniform
numbers, so the type system carries the control-rate/signal-rate boundary rather than a
person.

**A uniform into a varying input of its own kind is free and needs no node**: it is a
uniform, which is what an unconnected control already compiles to. **A varying into a uniform
input is a node**, because collapsing a million pixels to one value is a reduction, and a
reduction has to run somewhere. That one-way rule is physics, not policy, and
`Graph::can_connect` is where it lives. It reads the kind and the rate, so it says the same
thing about a color as about a number: a `color` node feeds any color input, and a picture
dropped on that node's own swatch is refused.

A uniform color reaches the shader exactly as a uniform number does, and differs in the two
things the shader has to be told: the name is `u_color_{slug}{id}_{key}` — silvia's own —
rather than `u_float_…`, and it is a `vec4f` member of `u` rather than an `f32`. `App` keeps the two in maps
of their own, `uniforms` and `uniform_colors`, because the readers differ: one becomes a
number on a row, the other a swatch.

**One output's type is not its own.** A **dual** output — `OutputDef::eval`, the same formula
in Rust as its WGSL — declares `VaryingNumber` and is *effectively* `UniformNumber` on any
instance where every input resolves to a uniform number: a connected input whose source is
effectively `UniformNumber`, or an unconnected one holding a number control. There is nothing
per-pixel about `a + b` when `a` and `b` are uniform numbers, so the tick evaluates it and
publishes a uniform number; feed either input a field and the same node is a field again. The
effective type is written into the instance's own `PortDef::ty`, recomputed by `Graph` after
every structural change, so nothing that reads a port's type has to know the concept exists.
A **bulk** change — a file loaded, a workspace imported, a paste — is one change rather than a
sequence of gestures: `Graph::begin_bulk` holds the recompute off, and
`Graph::settle_effective_types` works every type out once, from the whole graph, and drops any
cable that leaves reading a field as a number.
Because a field does not feed a `UniformNumber` input, the one thing that cannot be allowed is
a flip that leaves a uniform number somebody reads — so a node in that position is **pinned**
and its own inputs are `UniformNumber` too, which makes the cable an ordinary type mismatch
and the port a diamond that says as much. The whole family and the rule are in
[nodes.md](nodes.md#dual-outputs).

### Going down: side effects in the expression

The reduction runs *inside the expression* — the node's own input chain, compiled as any
picture's is — but not at a picture's coordinate, and not in any Output's program. A node that
measures declares a **measurement**: `NodeDef::measure_wgsl`, a function of a point, which the
compiler emits as `{slug}{id}_measure(p: vec2f)` into its workspace's
[pass](rendering.md#the-workspace-pass), whose `fs_main` calls it at every cell of an `N`x`N`
grid over the unit square, `-1` to `1` on both axes. A `tap`'s measurement evaluates its own
input at the point and does atomic adds into a storage buffer: a count, a sum of the measured
quantity, the extremes, weighted coordinate sums. A `sample`'s is one call at the corner
fragment, evaluating its input at `vec2(x, y)` exactly. The compiler hands each such node a
slot in the pass's tap buffer; the renderer resets the slot before the draw and reads it back
once the frame's submission has finished; the node's CPU half decodes its slot on the next tick
and publishes uniform numbers. The pass-through, in whatever Output's module the node's port is
cabled into, is then nothing but `return {input};`.

Three things follow, and they are the reasons this shape won over an analysis node with a
render target of its own:

- **It measures its input.** Not the picture a consumer asked for, and not a re-evaluation in
  another context: the same expression the picture is drawn from, over a domain that is the
  node's own. The reading is therefore a property of the input, the same whatever Outputs
  draw it, or none.
- **It sits anywhere.** A tap after a zoom reads the zoomed picture, because the zoom is then
  its input; before it, the source. What moves the reading is the graph above the tap, never
  the graph below it.
- **It is one frame late**, because the readback is collected once the GPU has finished the
  frame rather than any thread waiting for the GPU. A tap's uniform number describes the previous frame.

**A measured node has one slot, in one pass**: the pass of the first of its workspaces that
has a tab (`link::plan::measured_on`), so there is no second reading to choose between and
nothing moves the measurement but the node's own workspaces. This is what needs atomics on a
storage buffer in the fragment stage — `tap: array<atomic<u32>>`, at binding 1 of the pass's
module — which macOS's OpenGL does not have and wgpu gives every backend
([decisions.md](decisions.md#wgpu-not-glow)).

`Action` is the one of the four that never reaches a shader. The type, the many-to-many rule
and the square port are modeled in `graph/`; an action output is an event, not a function to
generate, so the compiler resolves it to nothing and it is delivered inside `tick` instead.
`button`, `mastergear`, `clockdivider`, `counter` and `adsr` inhabit it, and [cpu.md](cpu.md#the-event-half)
has the two rules that decide everything about it: an action is a gate, and an event carries
when it happened.

### CPU nodes

A definition may carry a `cpu` half: a constructor for per-instance state with a `tick`. That
state is created the first time the node ticks and dropped the first tick after the node is
gone, so a microphone or a camera lives exactly as long as its node. `Synth::tick` runs every one
in topological order, so a producer's uniform number is published before its consumer reads it,
and what a tick publishes — a uniform number per output port, a frame for a texture output, or
this tick of a world the renderer steps — is what `resolve_uniforms` and the renderer read that
same frame.

**A world too big for the tick is stepped on the GPU, and the tick still owns it.** A node
like `slimemold` keeps its agents and its field in the renderer and its rules in compute
kernels it writes, the way a generator writes WGSL; its tick reads the clock, the knobs, the
options and the presses, decides how many steps this `dt` is worth and what a press or a new
grid asks for, and publishes that as a `nodes::Simulation` of passes. The renderer runs them
before any Output draws and binds the picture they leave to a consumer as it would an upload.
What is on the CPU is what has to be — the one clock, the seed, the status line — and nothing
comes back. See [rendering.md](rendering.md#simulations).

A CPU node whose behavior depends on whether an input is driven asks the context: a Ratio
Gear counts ambient seconds until something is plugged into its Clock In, then counts that
instead, and a node that moves with time reads the playhead at its own rate until something
is plugged into its Time. Silence, in a port, is a mode.

`TickContext::readback(id)` is how a tap's CPU half gets its slot; a headless app has no
renderer, so it is `None` and the tap withdraws its uniform numbers with
`TickContext::withdraw` rather than publishing zeros.

`nodes/` still takes no graphical dependency — a kernel is WGSL in a string, as a
generator's output is. `audio/` (cpal, realfft) and `video/` (gstreamer, -app, -video and
-pbutils) are not graphical either: they produce numbers and bytes, and `render/` is the only
thing that turns a frame into a texture or a simulation into a world on the GPU.

### Workspaces

A **workspace is a view**, not a unit. There is one `Graph`, one `NodeId` space, and a
workspace is a `WorkspaceId`, a name, a one-line blurb, a `WorkspaceKind` — `Video`, the only
kind there is — and a `LayoutMode`. `Graph.workspaces` is every one of them in project order, and a node
carries `workspaces: BTreeSet<WorkspaceId>` beside its `pos`.

**The layout mode belongs to the workspace**, not to the graph: one canvas is a plane while
the one next to it is a strip, and each one's mode rides in its own file. `SetLayout` and
`AutoArrange` name the workspace they act on, and an arrange moves nothing on any other.

A node on two workspaces is **one node**, with one position and one set of controls. Nothing
nests and nothing is instanced, so `compile/`, `render/` and `nodes/` know nothing about any
of this: a workspace is invisible to the shader, which is the point.

Two invariants, and `insert_node` is where both are kept:

- **Every node's set is non-empty.** A node has to be somewhere.
- **Every id in it names a live workspace.** A set is filtered to live workspaces on
  insertion and falls back to the first one, so a hand-edited file cannot describe a node on
  a workspace that is not there.

The last workspace therefore cannot be removed, and removing any other one takes it out of
every node's set, removes the nodes left on none, and removes every cable touching those —
which is what makes visibility a real property rather than a filter.

Nine commands, all undoable by the snapshot mechanism with no undo-specific code:
`AddWorkspace`, `RemoveWorkspace`, `RenameWorkspace`, `SetBlurb`, `MoveWorkspace`,
`DuplicateWorkspace`, `ShowOn`, `HideFrom` and `MoveTo`. `DuplicateWorkspace` lifts every node
on the workspace into a clip and plants it on a new workspace beside the original, then cables
each copy to whatever fed its original from outside the set; what it copies, and why, is
[in decisions.md](decisions.md#a-duplicate-workspace-is-a-variation-not-a-second-view). `HideFrom` is refused where it would empty a node's set, and it checks every id
in a multi-node command before it writes any, as the other multi-node commands do. `AddNode`
names the workspace it lands on rather than implying one, so redo puts a node back where it
was.

Visibility is graph data — saved with the project and undoable — because it is layout in the
same sense `pos` is. It is not a control and not an option, so it stays outside
`Node::controls` and the address space.

**A new graph has exactly one workspace and every node is on it.** The canvas draws the
nodes whose set holds the active workspace, and a cable is drawn when both its ends are. A
cable with one end elsewhere becomes a [tag](ui.md#the-tag-a-cable-whose-far-end-is-elsewhere)
on the port that is here, naming the node and the workspace at the other end and going there
when it is clicked.

### Open and closed, and what suspension means

A workspace is **open** or **closed**. Open means it has a tab; closed means it does not, and
that is all it means about the project — a closed workspace is still in it, is still saved,
and is not on disk in any different way.

**Closed means suspended, not unloaded.** The nodes stay in the graph with their state:
`link::live_nodes` is every node shown on at least one open workspace, and a node outside it
is suspended.

- Its `tick` is not called. Its `CpuNode` state is kept, so a camera's pipeline and a
  microphone's stream are not torn down and rebuilt by a tab being closed. **Its time is
  not suspended**: on the tick it wakes it is handed the whole distance the transport moved
  while it slept, as a jump, so every gear in it is born again where a tab left open has it,
  a node on ambient time reads the playhead as it is, and a stateful node carries across — [cpu.md](cpu.md#the-transport).
- Its Output is not built and not drawn. It stays in the `FrameJob` as
  `OutputMode::Suspended`, so the renderer keeps its targets, its program and its last frame —
  **zero flash**: closing a workspace and reopening it reallocates nothing.
- It is not in the shader build set. An edit made while it sleeps stays in `needs_recompile`
  and is built on the frame the tab comes back.

A node shown on a closed workspace *and* an open one does all of it, which is what one node
means. What a suspended producer publishes is covered in [cpu.md](cpu.md#the-tick).

**Awake is not drawn.** An awake Output draws only while something visible or with memory
consumes it; any other is **idle**: its nodes tick and its program is kept linked, and it
publishes the last frame it drew until something wants it again. The plan says which and why
(`OutputPlan::mode`); the rule is [rendering.md](rendering.md#which-outputs-draw).

**On air outranks closed.** An Output on a mixer deck, and every node upstream of it, is
live whatever its workspaces' tabs are doing: closing the live deck's workspace to tidy the
editor must not freeze the show. `link::live_nodes` walks the connections back from each
claimed Output and adds what it finds. See
[rendering.md](rendering.md#the-mixer).

**A clock read outranks closed** the same way. A gear — a Master Gear, a Ratio Gear, the Time
node — cabled straight into a node shown on an open workspace is live, and every node upstream
of it, so a Master Gear on a closed tab keeps counting for the noise it drives on an open one,
and the chain of gears above it with it. `link::live_nodes` adds each such gear and walks
back from it as from a deck. Any other producer on a closed tab is suspended and holds its
last value for the open node reading it.

### Session state: which are open, which is showing, and each view

Which workspaces are open, which tab is showing (`Active::Project` or
`Active::Workspace(id)`) and where each canvas was left are one `Session`, held by `Project`
and saved in `project.ssp` — `open`, `active`, and a `view` per workspace. A view is that
canvas's pan and zoom, and nothing else: how long a strip is, [the strip works out for
itself](ui.md#the-strips-control) every frame, so it is not a thing a file can be left
holding a stale answer to.

**None of it is a `Command`.** Opening, closing and switching are session state exactly as a
pan is: they never enter the undo history, so undoing a workspace back into existence brings
it back *closed*, because opening one was never an edit. `Session::reconcile` runs on both
sides of the file and after any graph replacement, so neither a hand-edited manifest nor an
undo can leave the app with nothing open or a tab showing a workspace that is gone.

It is in the project file rather than in preferences because it is about *this* project: the
set of tabs a show is mixed with travels with the show. That is [the tier
test](#three-tiers-of-saved-state), and it is the same one that keeps window geometry out of
the folder.

## Module layering

| Module | Depends on | Rule |
| --- | --- | --- |
| `graph/` | `nodes/`, nothing graphical | topology only. No graphics, no egui, no code generation. A node names its `NodeDef`, which is the whole of what `graph/` takes from `nodes/`. |
| `compile/` | `graph/`, `nodes/` | pure string generation. |
| `nodes/` | `graph/`, `compile/`, `audio/`, `video/` | static data plus one generator per output, and a CPU half where a node has state. What a node draws below its rows is named here, as a `nodes::Region`, and drawn in `widgets/`. |
| `audio/` | `cpal`, `realfft`, `triple_buffer`, GStreamer, `platform/` | the audio thread and its analysis. Publishes uniform numbers; owns no node. Two capture backends — cpal for the default input, a GStreamer pipeline for a named source, whose element and names are `platform::audio`'s: PulseAudio's on Linux, which is the only way there to reach a monitor. |
| `video/` | GStreamer, `platform/` | capture and decode pipelines. Publishes frames as the source's own buffer, mapped in its own layout, or as a DMA-BUF descriptor; owns no GPU object. The elements at a pipeline's ends — a camera's source, a screen's, the hardware codecs, the DMA-BUF export — are `platform::video`'s and `platform::screen`'s; everything between them is here. |
| `midi/` | `graph/`, `platform/` | the wire's vocabulary — a message, a source — and the map from a message to a control. Owns no node and no window. The device and its reader thread are `platform::midi`'s; the reader's queue belongs to the synth, which drains it inside the tick. |
| `platform/` | `linux/`: `alsa`, `ashpd`, `tokio`, `rfd`, `fontconfig`, `libloading`, GStreamer, `render/dmabuf.rs`; `macos/`: `rfd`, `midir`, GStreamer, `objc2` and Apple's bindings on it (ScreenCaptureKit, CoreVideo, CoreMedia, `objc2-core-audio`, `objc2-core-foundation`, `objc2-app-kit`), `dispatch2`; `windows/`: `rfd`, `midir`, `windows`, GStreamer and its Direct3D 12 library, `render/dmabuf.rs` | everything the app asks of the operating system that is not the same on each one, outside `render/`: one module per service in `platform/mod.rs` — `midi`, `screen`, `files`, `dirs`, `fonts`, `gpu`, `audio`, `video`, `menu`, `check` — whose names are re-exported from `linux/`, `macos/` or `windows/` by the build's target. `linux/` holds the ALSA sequencer, the xdg-desktop-portal conversations and the one tokio runtime they happen on (`linux/portal.rs`, because `ashpd` caches one D-Bus connection for the process and its socket has to keep being pumped), PipeWire, PulseAudio, V4L2, VA-API and NVENC, DMA-BUF, fontconfig, the XDG base directories, the kernel's DRM counters and the NDI® runtime opened by its path ahead of the plugin (`linux/ndi.rs`, where `ld.so` would not find it by name). `macos/` answers every service natively, apart from the GPU's per-process counters, which it has none of: the folders and the file manager need nothing but the standard library, the file dialogs are `NSOpenPanel` through `rfd`, the cameras and hardware codecs are AVFoundation's and VideoToolbox's, the named audio inputs and the loopback are Core Audio's, screen capture is ScreenCaptureKit's picker and a stream whose frames skip GStreamer, MIDI is CoreMIDI through `midir`, the Font menu is AppKit's font collection, and the menu bar is AppKit's own, built from menus `App` makes out of `ui::menu`'s model — Linux has none, and draws the egui bar. `windows/` answers every service through GStreamer's official MSVC release and the Win32 API: the file dialogs and the error box are `rfd`'s, the file manager is Explorer, the folders are the shell's Known Folders, MIDI is WinMM through `midir`, the Font menu is DirectWrite's system collection, the cameras are Media Foundation's, saved by device path, the named inputs and the loopbacks are WASAPI's through `wasapi2src`, screen capture is the primary monitor through `d3d11screencapturesrc`, answered with no picker, and the hardware codecs are NVENC's, Quick Sync's, AMF's and Direct3D 12's, and a clip's frame reaches the renderer as GStreamer's Direct3D 12 texture on the renderer's adapter; like Linux it draws the egui bar and has no Syphon, and it reads no GPU counter. A service hands back the app's own plain data and owns no node, graph or window; each machine's crates are declared for that machine alone and named in its own backend alone, `rfd` in all three, `midir` in `macos/` and `windows/`, Apple's bindings in `macos/` and `render/`, and `windows` in `windows/` and `render/` (`tests/rules.rs`). `linux/` allows `unsafe` in `ndi.rs` and `filedrop.rs`, `macos/` in six, `audio.rs`, `screen.rs`, `pixels.rs`, `gpu.rs`, `menu.rs` and `syphon.rs`, and `windows/` in five, `check.rs`, `clock.rs`, `d3d12.rs`, `dirs.rs` and `fonts.rs`, each where its `mod.rs` declares it. |
| `maininput.rs` | `audio/` | which source the rig is pointed at. State only; the devices it costs are `synth/maininput.rs`, exactly as `mixer.rs` is to `render/mixer.rs`. |
| `render/` | `wgpu`, `wgpu_hal` (for the DMA-BUF, `IOSurface` and Direct3D 12 imports), `eframe`, `egui` and `egui_wgpu` (for eframe's device, its display handle and the editor's paint callbacks), `compile/` (for the WGSL `Shader` and `compile::wgsl`'s bindings and uniform layout), `nodes/` (for `nodes::Frame` and `nodes::Simulation`), `wayland-client`, `smithay-client-toolkit`, `calloop` on Linux, `winit` on macOS and Windows, `objc2`, `objc2-metal` and `objc2-io-surface` on macOS, `windows` on Windows | the renderer the app draws with — [rendering.md](rendering.md) — and what it shares with the rest of the crate: the job types, `render::adapter` — the one rule every wgpu device is made through — and `render/picture/`, the pictures thread, which is the Wayland surfaces every picture window is and why the Wayland crates are here rather than in `app/`. `render::picture::run` is how `main.rs` starts eframe: `eframe::run_native` on Linux, and on macOS and Windows eframe inside a winit event loop of our own, which is why winit is named here. What the rest of the crate holds of it is `render::Gpu`, a `Renderer`, a `Viewer`, a `Published` naming `render::Texture`s, and the paint callbacks `render::viewer` builds for the editor. One device and one queue behind `Gpu`, a completion serial for what the GPU has finished, and the synth submitting one Output at a time with at most `QUEUED_AHEAD` of its submissions queued ahead. Allowed `unsafe`, every block with a `// SAFETY:` line, in two modules: `render::picture`, for the borrowed `wl_display` and each window's wgpu surface over raw handles, and `render::dmabuf`, for the imports through wgpu-hal, a DMA-BUF on Vulkan, an `IOSurface` on Metal and a texture on Direct3D 12. |
| `ui/` | `egui` | draws, and returns `Command`s. Never mutates the graph. The canvas is one pass, `ui::show`, handed a `CanvasFrame` — everything it reads this frame, borrowed — and handing back `Effects`, the commands and every request beside them, which `App::apply_canvas` performs in one place. Each node on screen is drawn from a `NodeCtx` and the `canvas::NodeLayout` the pass made of it once. |
| `widgets/` | `egui`, `ui/`, `nodes/` | what a node draws below its rows: `widgets::def` is the one table from a `nodes::Region` to the widget that sizes and draws it, so the registry names a region without naming the toolkit. |
| `workspace.rs` | `graph/`, `nodes/` | one workspace as a file: the `.ssw` DTO, its warnings, and the id remap an import needs. |
| `project.rs` | `workspace.rs`, `graph/`, `nodes/` | the folder: `project.ssp`, load and save of the set, adopting strays, `assets/` and the cache beside it. |
| `synth/` | `clock.rs`, `graph/`, `compile/`, `nodes/`, `audio/`, `video/`, `maininput.rs`, `mixer.rs`, `render/` | the **running state**, the tick over it, and the thread it runs on: the clock, every `CpuNode`, what they published, the Main Input's capture, the offline render — and the `Renderer`, on the `render::Gpu` it was made on, which the renderer holds: a clone of the one device and its one queue. Holds the editor's `Graph` through a shared `Arc`, handed across on an edit, and writes only a copy of its own — a MIDI knob or a node's `write_control`, through `Arc::make_mut`. It names no graphics crate — `tests/rules.rs` keeps `wgpu`, `egui` and `eframe` out — because what a node computes may not depend on what draws it, and because it hands the renderer nothing to draw with: the renderer keeps the handle it was made on. |
| `app/` | all of them | owns the **document**, applies commands, drives the frame. `App` keeps the view and the frame — the canvas, the tabs, the start menu, the theme, the preferences window, the confirm and the frame meters — and owns the rest as four parts, a struct each whose fields only its own module can reach, so one part reaches another only by being handed it: `document/` is `Document` — the graph, the undo ring, the edit serials and the clipboard, with `apply` its one door; `link/` is `SynthLink` — the synth's host, the snapshot, the plan and its counters, what it keeps per Output and per pass, handed the session by `App::with_link`; `midi` is `MidiDesk` — learning, the monitor, the map as last sent and the barriers; `media` is `Media` — the assets and their pictures, the file dialog, the status line and the offline render's bookkeeping. Where two parts move together it is `App` that says so, at one call site: `edit` is `App::apply`, which hands what an applied command did to whoever owns it. `mod.rs` holds the struct, the workspaces on screen and the frame's housekeeping; `files`, `frame`, `maininput`, `render`, `show` and `undo` each `impl App` over the parts' methods. |
| `check.rs` | GStreamer, `render/`, `platform/`, `video/`, `preferences.rs`, `project.rs` | `supersilvia --check`, `--version` and `--help`, answered before `main` makes anything else: the report of what the machine gives the app — GStreamer and every element a pipeline makes, grouped by what each serves and named by the plugin set a distribution packages it in, a hardware codec pair, the GPU `render::adapter` picks, `platform::check::machine`'s lines, and the NDI® runtime — each line a PASS, a WARN for a feature it turns off, or a FAIL for what the app cannot run without, which makes the exit code 1. The binary ships without the machine's libraries, and this is how a tester finds out what theirs lacks ([packaging/linux/TESTERS.md](../packaging/linux/TESTERS.md)). |

`emath` supplies `Pos2` to `graph/` and `compile/`. It is egui's own math crate with no
graphics in it, which is what lets those modules hold a node position without depending on a
UI toolkit.

The crate root denies `unsafe_code`; the modules [invariants.md](invariants.md) lists alone re-allow it — `render::picture`, `render::dmabuf`, and a few files in each of `platform/`'s backends — each where its parent module declares it.

"No graphical dependency" is walked, not grepped: `tests/rules.rs` follows every path a
pure module names — through the modules it reaches and the re-exports on the way — and fails
on the first file that names a graphics crate. The two edges it steps over are named there,
one per machine: `platform/linux/video.rs`, which `video/` and `audio/` reach through
`platform/`, asks `render/dmabuf.rs` whether the renderer's device can import a DMA-BUF, and
in which formats, before a screen cast or a clip asks its source for one; and
`platform/macos/video.rs` asks it whether the device can import an `IOSurface` before a clip
asks for its decoder's own memory.

**Nothing outside `nodes/` matches on a node slug.** Two bits of node semantics escape into
`graph/`, both carried as data rather than inferred: `NodeDef::is_output`, which a node
reaches through `Node::def`, and `PortDef::delayed`.

**A node names its definition.** `Node::def` is the kind's `&'static NodeDef`, set where a
slug becomes a kind — a file loading, a node added from the library or the bus — and read
everywhere after: the tick, the compiler, the command bus's coercion and the canvas all have
the kind's ports, options, regions and width through it without a search, and
`nodes::find` is for those two edges alone, which `tests/rules.rs` holds it to. What a node
carries beside the pointer is its own document data: its ports' effective types, its
position, controls, values and options, and the width a hand dragged it to. What the canvas
measures while it paints — how tall a note's text came out — is the canvas's, in
`ui::canvas::Measured` keyed by `NodeId`: it is not written to the graph, so it is not an
edit, not in an undo step or the file, and never crosses to the synth.

## The compiler

`compile::wgsl::build(graph, output) -> Option<Shader>`, and `build_probe`, `build_pass` and
`build_measure` beside it. `None` means the Output's input is
unconnected — not an error. That Output is inactive and renders black, which is what silvia
does too.

`CompileContext::input` is the whole of it, and has exactly four cases:

| Case | Yields |
| --- | --- |
| the port is connected to a field | a call: `checkerboard1_output(uv)` |
| the port is connected to a uniform number | a member of the uniform struct: `u.u_float_audioin3_level`, and no function |
| unconnected, has a number or color control | a member of the uniform struct: `u.u_control_checkerboard1_frequency` |
| unconnected, nothing else | a type fallback: `defaultUvMap(uv)` for a varying color, `0.0` for a varying number |

The generative `defaultUvMap` fallback is why an unplugged color input shows a hue wheel
rather than black. It is inherited from silvia deliberately: an empty input that shows
*something* reads as unfinished rather than broken. silvia's wheel turns; this one stands
still, a function of the point alone, so it never keeps a loop from closing.

**A dual output adds no sixth case.** The second row already says what happens to one: its
instance port *is* `UniformNumber` in diamond mode, so `input` takes the uniform number
branch it has always taken and never reaches `emit`. In circle mode the port is
`VaryingNumber` and it is the first row.
The concept lives entirely in `graph/` and `nodes/`; the compiler reads a port type.

Functions are emitted callee-before-caller, and a function is marked emitted *before*
recursing, so a diamond in the graph emits it once. Uniforms live in a `BTreeMap`, so
declaration order is deterministic and snapshots are stable.

### Degrade loudly, never silently

An unconnected color input compiles to `defaultUvMap(uv)` — the hue wheel. That is not an
error path, it is **the** behavior: delete a cable you did not mean to delete mid-set and you
get something to play against instead of a dead projector. silvia worked this way and it is
worth keeping.

What was wrong was that the *bug* path borrowed it. A node, definition or port that the graph
said existed but did not produced the same expression, so a broken invariant looked exactly
like an unplugged input.

`Shader::diagnostics` separates them. The emitted WGSL is unchanged — the show still runs —
but every unresolvable lookup is recorded, the status line shows the first one, and
`tests/compile.rs` asserts a healthy graph produces none.

An Output with nothing connected stays **black**, deliberately: it is genuinely off, and a
hue wheel there would read as working.

## Where the memory is

Three kinds, and only one of them is addressable.

**Outputs own the render targets**, and that is the memory a *graph* can address: an Output's
`frame` port hands its published texture to any other graph, and feedback reads it. **CPU nodes
own devices and per-instance runtime state** — a pipeline, a stream, a slew's last value — and
nothing can address that: a CPU node publishes through a port like everything else.
**A simulated world is the renderer's**, held by the port its picture is published on — a
`SimRenderer`, made from the frame job the way an `OutputRenderer` is — and it is as private
as a CPU node's state: what a graph reads of it is its picture, through that port, and its
agents and field are reachable from no shader but its own kernels. It is runtime state that
happens to live in GPU memory, and a saved file holds none of it.
[cpu.md](cpu.md) has the full split, including why there is no saved-in-the-file kind.

Everything else is stateless WGSL. An Output owns a small ring of render targets, draws each
frame straight into a free one, and publishes from them. Because its `frame` port hands the
latest of them to any other graph as a `texture_2d<f32>`, an Output is three things at once:

- **the screen** — what the performer sees;
- **an intermediate buffer** — Output A renders a heavy pattern once, and B and C sample A's
  `frame` rather than recompiling A's chain into their own shaders. This is the only way a
  *rendered* pass feeds another; the other path into a `texture_2d<f32>` is a CPU node publishing a
  frame it did not render — a camera, a video file;
- **a feedback source** — a graph sampling its own Output's `frame` reads the previous
  frame, because the frame being drawn goes into another target of the ring.

`render/` is therefore a map of `OutputRenderer`s from the start — and, beside it, a map of
the worlds it steps. There is no "the" output.

## Delayed ports and feedback

A port is **delayed** when consuming it reads a value from a previous frame rather than the
current one. `PortDef` carries one bit for it, set two ways: from `OutputKind::Texture` — an
Output's `frame`, and the captured frame `camera` and `video` publish under the same kind —
and from `OutputDef::delayed` on a `UniformNumber` output, which is how a `tap`'s, a
`sample`'s and an `autoexposure`'s are: the CPU half publishes each one a frame after the
pass measured it, so the reading is last frame's the same way a frame port's is. Three
consequences:

- **The compiler never descends through a delayed `Texture` port.** It samples a `texture_2d<f32>`
  binding, so a loop through one terminates. This is why a feedback shader is finite, and why
  a consumer Output costs one texture read rather than an inlined chain. A `UniformNumber`
  port never descends either way — the compiler resolves it to a uniform before a
  generator could run — so a delayed one costs nothing extra here.
- **The cycle check ignores delayed edges.** An immediate loop would make the compiler
  recurse forever, or ask a CPU tick to read a value it has not produced yet this frame, and
  is rejected. A loop through a `frame` port, or through a measurement's, is feedback, and is
  legal.
- **The topological order includes delayed edges**, so a consumer still runs after its
  producer and sees the current frame where that is possible. Where they close a loop, the
  order is over the loops rather than the nodes (`graph/order.rs`): Tarjan's strongly
  connected components, placed producer first with the lowest id first on a tie, so
  everything downstream of a loop still follows the whole of it. Inside one, Kahn runs over
  the loop's own edges with its delayed ones left out — a delayed read inside a loop is last
  tick's whichever member goes first — so a member follows what feeds it this frame and
  otherwise goes in id order. Only a loop's own members read one another a frame late. The
  plan orders its Outputs by the cables alone: a measurement samples frames too, but in a
  pass drawn after every Output ([rendering.md](rendering.md#the-order-a-tick-submits-in)).
- **The dual ports' types settle over immediate edges alone.** A delayed port is never dual,
  so a type still being decided never crosses one, and the immediate edges never close a
  loop: their order puts every dual producer before its consumers, a loop through a
  measurement included, and one pass settles a file.

## One clock, on a thread of its own

`clock.rs` measures one interval per tick, `transport.rs` is the playhead over it, and
`Synth::tick` is the single place per-frame CPU work happens: every CPU node, once, in
topological order, each handed how far the transport moved since it last ticked. **Nothing
else keeps a timer.** The
audio thread and GStreamer's threads run on the device's schedule and publish through a
triple buffer, or a camera's one-frame slot; `tick` reads the newest value and never waits
for one.

**What a tick is given.** The graph, shared across on an edit; the plan the compiler built; a
press, a seek, the Main Input's choice, the rate; and **MIDI** — the reader thread's queue and
the map, both the synth's, drained and read inside the tick.

**The synth keeps its own time.** `Synth::step` runs on a thread named `synth`, once per
display interval, from a deadline carried forward and re-anchored when a tick lands more than
a whole interval late; the rate comes from the monitor the editor is on, or from the `tick
rate` preference. Nothing about the schedule comes from a window's frame callback, which is
what lets the world go on while the editor is minimized. The clock is fed from a monotonic
`Instant` taken when the thread started, never from egui's input time, because egui's belongs
to a loop that stops with the window. `proposals/deterministic-loop.md` has the argument and
the measurements; [decisions.md](decisions.md) has what was rejected.

Every `CpuNode` is **born on that thread**. The trait is not `Send` — a cpal stream is not —
and it does not need to be: state is created on the first tick after a node appears and
dropped on the first tick after it is gone, so a node is created, ticked and dropped there and
never crosses. The Main Input's capture, the renderer, and the offline render's writer are the
synth's for the same reason. What the frame thread hands over is the synth's
`render::Gpu` alone — `Gpu::for_synth`, asked of the editor's: a clone of the one device
and its one queue, on which the synth thread makes its `Renderer`. Nothing is made current and
nothing else crosses.

## The pictures, on a thread of their own

Beside the synth runs a thread named `pictures`, and it owns every **picture window** — a
node's render, a source's frame, or the mix, each in a borderless window of its own. It is not
egui's: eframe runs one winit event loop and that loop services every viewport in turn, so a
minimized editor would stop every other window with it, and winit permits exactly one
`EventLoop` per process. The thread opens Wayland surfaces of its own instead — an
`xdg_toplevel` each, a wgpu surface on each, and a `Viewer` per surface format, on a clone of
the one device — and each window is paced by its own compositor's frame callback.

It borrows eframe's `wl_display` rather than opening a connection: a window from a second
client connection is from a client that does not hold focus, and the compositor may open it
behind the editor. It reads the synth's `render::Live` on its own clock, the same slot every other viewer reads. The
editor sends it `picture::Ask` and reads `picture::Told` back once a frame; what it never
touches is the graph, the snapshot, egui or the command bus.
[rendering.md](rendering.md#picture-windows) is the contract, `proposals/picture-windows.md`
the argument.

**That is Linux.** AppKit makes windows on the main thread alone, and Win32 hands a window's
messages to the thread that made it, which winit allows to be the event loop's alone, so on
macOS and Windows no thread owns them: each picture window is a winit window made on the main
thread, inside the event loop `render::picture::run` starts around eframe, and drawn on a
thread of its own named `picture`, on the same device and from the same `Live`
(`render/picture/winit/`). The editor's side — `Ask`, `Told`, `Wall` — is the same on all
three. [rendering.md](rendering.md#on-macos-and-windows) has the contract,
`proposals/macos-windows.md` the argument.

**The mix is a picture like any other**, popped out by the same pair of marks; there is no
projector window of another kind.

### The two channels

```
editor ──Msg────────▶ synth      the graph, shared; a plan, a press, a seek, the rate
editor ◀──Snapshot─── synth      everything one tick published, swapped under a mutex
```

**In** is a `std::sync::mpsc` channel the synth drains at the top of every tick.
**`Msg::Graph` carries the graph itself, shared**: an `Arc<Graph>` the editor and its undo
ring hold too. A `Graph` holds each node as an `Arc<Node>`, and its cables and its workspace
list behind an `Arc` of their own, and it is `Sync` — its two cached orders are `OnceLock` —
so both threads read one graph, and **neither writes a graph the other can see**.
An edit goes through `Arc::make_mut`: where the synth still holds the graph on screen, the
first write copies it, which is a map of node pointers and a few `Arc` bumps, and the write
then copies the one node it lands on. The synth's own writes, a MIDI knob and a node's `write_control`, go
the same way onto a copy of its own, and `tick` lets go of its handle before them, so only
the first write after a graph arrives copies anything.

`SynthLink::publish_graph` is the one place the graph crosses, reached from every path that
changes the editor's graph — a command applied, an undo, a redo, a cancelled scrub, a project
opened or imported — so the cost is one `Arc` bump per command a tick could *observe*.
`Command::affects_tick` is the question, and a node drag, an auto-arrange, a collapse and a
workspace rename answer no. A scrub crosses on every frame of the gesture, and a frame of it
costs the one node it wrote and the map beside it; the graph carries its built tick order with
it, shared, so that frame is not a rebuild inside the tick either. A frame that edits nothing
crosses nothing, and `App::graph_generation` counts the crossings so a test can say so.

**The cables are indexed by the node at each end.** Beside the list the file writes and the
compiler walks, a graph keeps the cables into each node and the cables out of each, so
`source_of`, `sources_of` and `targets_of` — asked per input of every CPU node every tick,
and per port of every drawn node every frame — look at a handful of cables rather than scan
them all. The index is **kept, not derived**: `link` and every unlink write the list and both
maps together, behind the one `Arc`, so no query rebuilds it, a load that seats a thousand
cables into a graph nothing else holds writes each in place, and a clone shares it as it
shares the list. An edit that makes or breaks a cable on a graph an undo step shares copies
list and index once, together, and an edit that touches no cable copies neither.

**Every walk is a walk of that index.** Each entry carries how its value travels — an action,
this frame's value, or last frame's through a delayed port — fixed from its source port when
the cable is made, so the orders, the cycle check and the recompile marking filter
edges without looking a port up and nothing derived is rebuilt per cable: a load seats each
cable with a walk up from its producer and no more. What a walk answers is not remembered.
The one caller that asks the same question many times, a cable drag asking the cycle question
of every candidate port on every frame, walks once from the end it holds and keeps the
`graph::Reach` in the canvas's state for the drag; `Graph::can_connect_within` believes it only
while the graph holds the cables it was walked over.

`Msg::Plan` carries what the compiler decided — the shaders, where each uniform's value comes
from, the decks and the mix's size, the tap routing, which Outputs draw and why — and the
synth turns it into a `FrameJob` on **every** tick, because it ticks when the editor is not
painting. The uniforms cross as *providers* rather than values: resolving one reads the graph
and what the last tick published, both of which are the synth's, so resolving them on the
frame thread would make every uniform number driving a shader a tick stale.

**A plan is built only when something it reads has changed**, and crosses only then.
`SynthLink::publish_plan` compares a `PlanKey` with the one the last plan was built on: the
document's `shape` — a count bumped by every command that `Command::reshapes` the graph and by
every graph restored or replaced whole, which a scrub, a reset, a drag and a rename do not —
the open tabs and the one on screen, the decks and the mix's size, and the two besides that
make an Output draw: the pictures with a window of their own, and the Output a render is
capturing. A frame whose key matches builds nothing, unless an awake Output has a rebuild
waiting or the renderer asked for a source again; so a control scrub, which the plan reads
nothing of, rebuilds no plan, and neither does a frame nobody touches. **The fade, the
crossfade method, Blackout and Freeze are not in the plan**: a fade moves them every frame and
nothing a plan works out reads them, so they cross on their own as `Msg::Fade`, where any
changed — and on a hand on the fade or a press whatever it says, because its MIDI barrier
waits on the fade that carries the move, with `mixer::Hands` saying which a hand moved.
What the editor is reading — whether anything reads the per-node report, whether the Status box
is open, whether the Main Input panel shows its picture — is the tick's business and crosses
in `Msg::Inputs`, so opening the box or folding the panel builds no plan either. The **live
set** is worked out once per change to the shape, the tabs or the decks and read by the plan,
the passes' measurements and `Msg::Inputs` alike, and `Msg::Inputs` crosses only where it
differs from the last one sent.

**A source crosses once.** The link hashes each Output's shader and each probe as it builds
them and keeps the hash the renderer was last sent; a rebuild that comes out as what the
renderer holds sends nothing. An undo replaces the graph whole, so every Output is rebuilt —
and an undo of a move, which compiles every Output to what it was, sends no source at all. A
source that comes out as an *earlier* one is sent, and the renderer, which keeps the programs
it drew with by their source (see [rendering.md](rendering.md#linking-happens-off-the-synth-thread)),
draws with the one it had rather than linking it again. A probe is rebuilt only when its
Output's source changed, since a probe is a function of that source and nothing else.

**A shader rides on one plan and must survive the next.** The editor publishes a plan on every
frame that changes one and the synth drains every one waiting at the top of a tick, keeping the
last; an Output's source is on the plan built the frame it was compiled and on no other. So a
source the job has not yet taken out of the synth's plan is carried into the plan that replaces
it, rather than dropped with it — on an editor that paints several frames per tick, a plan
carrying a fresh source is superseded whenever the next change lands before the next tick, and
a new Output would stay black until the next recompile. The renderer's own "still no shader"
report is the backstop for a target it could not allocate, and the editor believes it per
Output only from a snapshot that has drawn from the plan that carried that source, whose number
the link keeps beside the source in its one entry per Output: a single gate against the plan
before the newest is never satisfied while the synth runs slower than the editor. That entry —
the shader, the probe, the hashes of what the renderer holds, the plans they went out on, the
probe's counts and the drop rate — is let go in one place, when the plan stops carrying the
Output.

**Out** is a double buffer under a mutex held for the length of a `mem::swap`, plus one slot of
its own: `render::Live`, which the synth writes each tick for the picture windows, because a
picture window is painted outside the editor's pass and must keep showing new frames while no
frame runs at all. The synth fills one `Snapshot` per tick — the notes, the scopes, the
playheads, the traces, what every port published, the held buttons, the tap readings, what the
renderer reported, and the handles a viewer blits — and the frame thread swaps its spent buffer
for it at the top of each frame. Neither side ever waits on the other's work. It costs 2.8 µs a
tick on the Pumpkin project; the one term that scales is the traces, at about 2 µs each, and
they are gathered only while something is drawing them.

**The editor only moves forward.** The slot is one buffer, so a frame painted between two ticks
would find the spent buffer it left on the previous frame and take it back as news: every
reading a tick older, then the newer one again — a phase running backwards every other frame
whenever the synth falls behind the display. `Mailbox::take` compares `Snapshot::seq`, counted
per publish, and keeps the editor's own buffer when the slot holds nothing newer.

**A snapshot may be skipped**, because the synth can tick twice between two frames and only
the newest buffer survives the swap. Everything in a snapshot that is a *fact about this tick*
is fine to overwrite — a uniform, a picture, a drop count. An **event** is not: a deck claimed
on a tick the editor never read would simply never happen. `Snapshot::actions` is the one
that is overwritten anyway, and says so: nothing on the frame thread acts on an edge, and the
day something does it has to.

**No event rides in a buffer**, because the mailbox hands the synth back whichever buffer the
slot held, taken or not. An event accumulated in the buffers would go out again under a newer
`seq`, behind one made after it: an older deck claim landing after a newer one, a recording
landing after the Clear that emptied it. So the synth keeps each kind in an ordered log of its
own, `synth::Events` — the deck claims, each tick's firings, the values a tick wrote, each
tick's probe counts, the pictures a save asked for, the Snaps and the MIDI messages — each
beside a count of every entry it has ever held. A snapshot carries the entries the editor has
not acknowledged and the count; the editor applies only the entries past the count it last
saw, in order, and nothing twice. **Handing a buffer back is the acknowledgment**: the mailbox
marks everything in it seen, the synth lets those entries go when the buffer reaches it, and a
publish copies into a buffer only what the buffer does not already hold, so a tick costs the
events it made and not the length of a log.

**A log is bounded**, which bounds how long the editor can be away: `synth::events::KEPT`
entries, a tick's firings and a tick's probe counts being one entry each, and
`synth::events::SNAPS` for the Snaps, each a whole frame. An editor that misses more than that
— minimized for a long set, or stalled — is told how many it missed, and logs the loss rather
than applying the part that is left: a run with a hole at its front is not the run that
happened.

The MIDI **messages** are one of those logs. They are the monitor's and learning's, so an
overrun costs the monitor that stretch of knob and leaves a control being learned waiting for
the next message; the world and the document heard every one of them by other paths.

The MIDI **writes** are not a log, because losing one of those is losing a value. They are one
entry per control — the newest value each has — republished whole every tick and retired where
the editor's own graph shows it has landed. A log would have let a long sweep of one knob
push another knob's last value out of it, and `MidiWrite::published` would then have gone on
restoring that value against every graph the editor sent: the world and the file disagreeing
for the rest of the session. A map keyed by the control cannot lose one, however long the
editor is away.

### The decks are the synth's, and the project's mixer catches up

`Show on A` is an action input, so a sequencer lane or a Master Gear's Trigger can cut decks inside
a tick with nobody watching the editor — and the project's mixer is the *editor's*. If the
claim only landed where the editor applies it, a minimized editor would hold every cut until
it came back and then land them all at once. So the synth keeps **which Output is on which
deck** and applies each claim to the mix it renders, on the tick it happened; the editor
reconciles the project's mixer from `Events::decks` and republishes a plan that agrees, one
tick out and one frame back. A plan's decks are adopted only where they *changed* since the
last plan, so one built before the editor had seen a claim does not undo it.

Only the deck assignment is the synth's. The mix resolution stays with the plan, and the
crossfade method with the editor's last `Msg::Fade`: no action input touches it, so nothing
inside a tick can move it and there is nothing for the two sides to disagree about. The fade,
Blackout and Freeze are the editor's last `Msg::Fade` too, except where a bound MIDI message
has moved one inside a tick — the same rules a control write keeps, in
[media.md](media.md#the-map).

### MIDI is read by the tick, and the document catches up

Same shape, same reason. The reader thread's queue is the **synth's**: it is drained at the
top of every tick, before anything reads the graph or the buttons. A CC bound to a number
control is scaled across that control's own `nodes::control_range` and written to the graph
this thread owns; a note bound to an action input holds and releases the same button a hand
would. The world answers on the tick the message arrived, whether or not the editor is
painting — which is what a person minimizing the editor and reaching for a knob expects.

The editor keeps the two halves that need it. **Learning** is a control under the pointer, so
`Alt` + click stays here; while a control is waiting the synth drives nothing, so the knob
being taught does not also move what it moved a moment ago. The **monitor** is a window, and
it sees every message because every message rides out in the snapshot's log in the order it
arrived.
The map crosses the other way on every change, through `MidiDesk::publish`, exactly as the
graph crosses through `publish_graph`.

**The document catches up through the bus.** Each control write rides out on the snapshot and
the editor applies it as the `SetControl` it would have been, so the undo history and the file
say what the knob did. A press does not: a press was never an edit. And a graph the editor
built *before* it saw a write must not undo it, so the synth adopts an arriving graph's
control only where it differs from the one the editor last sent — the plan's rule for decks,
on controls.

**The hand is later, and wins.** The same staleness points the other way: a write the editor
has not read yet is sitting in a snapshot it is about to take, and a drag of that same control
in between would be undone by it a frame after it happened. So a control the editor moved
itself carries a **barrier** — the graph generation the move went out on — and a write is held
until the tick has run over a graph at least that new, by which point the synth has retired its
own entry and anything still there was written after the hand.

### The frame, and the viewers

`App::ui` takes the snapshot at the top, does the editor's housekeeping around it — the shaders
rebuilt, the probe counts ingested, the drop rates, the claims applied to the mixer — draws,
and hands the synth a plan where the frame changed what one is built from. It renders nothing:
what each window paints is a **viewer**, a blit of a texture the synth has already finished
drawing, so a window nobody is looking at costs a blit and a window that is looking waits on no
Output. [rendering.md](rendering.md#one-device) has the one device every window and the synth
draw on, and [every window is a viewer](rendering.md#every-window-is-a-viewer) what each window
does about its own compositor.

The line between the two halves is
[cpu.md](cpu.md#two-kinds-of-state-and-which-one-a-saved-file-holds)'s: what a saved file holds
against what a node computes.

**The synth runs inline where there is no GPU** — `App::headless`, every layer-1 test, and
egui_kittest — on the caller's thread, through the same channel, the same `Synth::step` and the
same buffer swap. There is one implementation and not two: a test executes the path a
performance executes.

silvia has 45 `requestAnimationFrame` call sites across 24 files, each with its own
`performance.now()`, and no defined order within a frame. Audio results reach the shader one
or two frames late and jitter. That, not pixel throughput, is what this rewrite exists to
fix: the GPU work is the same fragment shader either way. The frame-pacing overlay shows the
**audio age** — how long ago the analysis a shader is about to use was written — and on this
machine it reads 1–10 ms against silvia's 50–100.

**Nothing on the clock is clamped.** `elapsed` and the interval between two ticks are
both what happened. The clamp is a stateful node's: a simulation, a slew or an envelope
takes the transport's advance as its step, bounded at `transport::MAX_DT` (100 ms) live and
zero on a jump, where a gear integrates the whole of it. Clamping at the clock would make
every gear quietly disagree with the world after every stall, and everything downstream — a
sequencer, a musical grid, two workspaces meant to stay in phase — would drift apart with
nothing to notice it. A stall is a thing to find and fix, not to hide inside the clock; the
Status box's pacing is how you find it.

There is one clock. **The transport is a coordinate system over it**, not a timer of its own:
a playhead `T` in seconds and a play bit, read as
`T = T_anchor + playing × (elapsed − elapsed_anchor)` and re-anchored on every play, pause and
seek. It has no speed and no loop. The playhead is **ambient time**: a node that moves with
time reads it at its own rate, `Time + Offset` with nothing kept, and a gear — the one place a
rate is set — is a coordinate system over *it*, integrating how far the playhead moved. When audio is running, the authority for the
clock becomes the device's sample counter rather than the system clock — *who holds it*
changes, the number of clocks does not. [cpu.md](cpu.md#the-transport) has the per-node
reading.

**The transport can be driven.** An offline render makes time a function of the frame index
and of nothing else: frame `i` is at `T = (i − warmup) / fps`, the playhead is driven there
with the distance between two frames as each one's advance, and `Clock::set_elapsed` puts the
render's own clock at the same `t` for `ctx.elapsed`. A stateful node's step is the frame,
unclamped, because the clamp bounds a stall and virtual time has none. The render's first
frame is a seek, so every node starts the render from scratch and every gear is born again on
it, where the playhead puts it — every Master Gear at the start of its cycle on the first kept
frame — and a node on ambient time reads each frame's own moment.
`clock::Stepper` owns a clock for the length of a run and the synth puts its live one aside,
with the live transport, every CPU node's live instance, every simulation's world on the GPU
and every Output's latest frame; when the render ends all of them come back, the clock re-anchored, so **the live show carries on as the render found it** and
no node sees a jump ([cpu.md](cpu.md#the-transport)).
Same `t`, same `u_time`, however many times it is asked — which is the property a render
rests on, and what `tests/offline_clock.rs` and `tests/transport.rs` hold.

## The command bus

Every user-visible mutation is a `Command` applied by `App::apply`. The UI only emits them.

- Tests build graphs with three `apply` calls instead of thirty simulated drags.
- **A command that fails never reaches the history**, and leaves no undo step. Neither does
  one that would change nothing: a cable that is already there is refused, so it is not
  logged, not handed to the synth and rebuilds no Output.
- Consecutive `MoveNodes` for the same **set** of nodes, and consecutive writes to the same
  **control**, coalesce. A drag is one undo step, not sixty — and one snapshot, not sixty.
  Anything a gesture writes together goes in one command for that reason: `MoveNodes` takes a
  list because a selection is dragged as one, and `SetControls` takes a list because the audio
  scope's band handle carries a frequency and a Q. Two commands a frame would be two steps a
  frame, which is a full undo ring in a second of dragging.
- **Letting go ends the gesture.** The pointer release is the boundary, so two scrubs of one
  control are two steps; without it a gesture ended only when some other command interrupted
  it, and one undo walked back both.

## Undo is a snapshot, not an inverse

`Document::apply` keeps the `Arc<Graph>` it held before each new edit. Undo restores one.

The step is the graph itself, not a copy: the edit that follows writes through
`Arc::make_mut`, so the graph after it shares every node it did not write with the one the
step holds, and taking a step copies nothing. Undo puts the very graph back, and a redo the
very one it replaced. **The id counters are not rewound**: a restored graph takes the higher
of its own `next_id` and `next_workspace_id` and the ones it replaces, so Add, Undo, Add
hands out a new `NodeId` rather than one a command in the log already names, and a new
`WorkspaceId` rather than one another tab's file and picture carry.

**The history is the ring.** `App::history` lists the command each step carries — the one
that opened it, or the last one its gesture wrote — oldest first, and is trimmed with the
ring because it is the ring. It labels steps; it is not a log that replays, since a gesture
that wrote a value and then cleared its range shows only the clearing. What a person reads is
`Command::name` of that command against the two graphs either side of it — the step's own
snapshot and its neighbour's, or the graph on screen — so a deleted node is named from the
graph it was deleted out of: `App::step_names` for the Undo History, `App::undo_name` for the
Edit menu. `App::travel_to` walks the ring to any step and rebuilds once, at the end, however
far it went ([ui.md](ui.md#undo-by-name)).

A command therefore has no inverse to get wrong, and **every command that ever gets added is
undoable the day it is written** — including ones whose inverse would be real work.
Restoring a deleted node together with every edge that touched it and every control value it
held is what `tests/undo.rs` covers, and it needed no undo-specific code — and neither did
`Duplicate` or `ImportWorkspace`, which bring nodes in by the dozen, when they came.

The cost is memory, and the ring is capped **by bytes first**: `UNDO_BYTES` is 64 MB, with
`UNDO_DEPTH` at 256 steps as a second bound. A step is counted by what it holds that its
neighbour does not — `Graph::footprint_beside`: its own map of node pointers, every node the
edit wrote, and the cable or workspace list where the edit made a new one. A knob turn is one
node, so an ordinary session is bounded by the depth; an edit that writes every node of a
large graph, an auto-arrange of it, holds a whole graph of its own, and 256 of those are
bounded by the bytes instead, which is the point — depth alone scales the ceiling with
whatever someone opens. `Graph::footprint` is the estimate beneath it, since a `Graph`'s heap
is not measurable without an allocator hook: a per-node constant plus the entries that vary
between graphs, calibrated so a whole 200-node graph reads the measured 150 KB. Both caps
trim oldest first, and the newest step is never dropped. Coalescing still matters — a control
scrub emits a command every frame, and without it a knob turn would be sixty steps a second.

Why not a list of inverses is in
[decisions.md](decisions.md#undo-by-snapshot-not-by-inverse-commands).

### A gesture ends on a release, or on silence

A gesture is what decides where one undo step ends and the next begins. **One is open at a
time**, and its step is the one on top of the ring.

**A hardware knob has no release.** A MIDI CC bound to a control writes the same
`SetControl` a scrub does, because a control's value is document data: it is saved, and a
knob moved during a set changes what the file says. But a gesture that waited for a knob's
release would run until some *other* command interrupted it, and then swallow that one too.
So a knob holds a gesture open until **silence** instead: a burst that stops for
`app::midi::SETTLE` — a fifth of a second, longer than the gap between two messages from a
turning knob and shorter than the pause between two deliberate moves. The tick applied the
value when the message arrived; the knob lets go when the **editor** next runs after the
silence, which is the same moment for anybody watching a screen.

So the rule is: **the gesture closes when the pointer has been released and the knobs have
been silent** — or on any command that is not a continuous write (an add, a delete, a cable,
a paste), an undo, a redo or opening a project. While it is open, **every continuous write
lands in its step, whatever it targets**: the hand's drag, scrub or grip, a knob's
`SetControl`, and a value the tick writes onto its own node (`App::take_value_writes`, an
automation's recording stopping). `document::Origin` says which of the three wrote a command,
since that decides what holds the gesture open: the hand until a release, a knob until
silence, and the tick nothing — a recording that lands with nothing open is a step of its
own.

Two knobs turned together are therefore one step per stretch of movement, however long the
stretch — a step per knob per frame emptied the 256-step ring in two seconds. A knob falling
silent mid-scrub does not split the scrub, because the pointer is still down, and a
recording that lands mid-drag joins the drag. A node dragged while a knob turns is **one
step**, and one undo takes back the drag and the knob's movement together. That is the
trade for there being one step to reason about rather than two that interleave.

The hand alone keeps the boundary it always had: with the knobs silent, a write of the
hand's that is not the same thing it was writing — a wheel on another control, a drag after
typing into a note — opens a step of its own (`document::history::joins`). Two scrubs of one
control with a release between them are two steps rather than one that walks back both.

`Escape` on a scrub drops the open step whole and restores the graph and edit serial it began
on, offering no redo. **Whatever a knob or the tick wrote into that step goes back with the
scrub**, exactly as an undo of the step would take it; the graph then disagrees with the
physical knob until it next moves, as after any undo. A knob turning across a release holds
one step open over two scrubs, so the step may have begun before this scrub did: the value
the drag began at is re-applied afterwards, as an edit of its own.

Learning a binding is **not** a command and never enters the history, exactly as claiming a
deck is not: binding a knob is setting the rig up, even though the map it writes rides in the
project file.

## The project

**The project is the only thing that is saved.** It is a folder, and everything else is a
part of it. There is no "save this workspace" beside "save the project": `Ctrl+S` writes the
folder, and a workspace *leaves* a project by export and *enters* one by import.

```text
friday/
  project.ssp          the workspaces in project order, the two counters, the glue
  workspaces/
    tunnel.ssw         one workspace: kind, name, blurb, its nodes and the cables between them
    tunnel.png         a picture of it, written a frame or two after each save
    gumby.ssw
  assets/
    gumbasia.webm      the media, referenced as assets/gumbasia.webm
  cache/
    3f9a…c1.mp4        what was derived from it: a transcode, a decoded soundtrack — made
                       on demand, deletable, and left behind by Save as
  renders/
    output3-001.mp4    an Output's render: a film, a GIF, or a folder of numbered PNGs
  snaps/
    output3-20261001-201500.png   a Snap, named by its Output and the UTC time it was taken
  .autosave/           the unsaved edits: a project folder of its own, while there are any
```

`renders/` and `snaps/` are the person's own pictures, written beside the patch that made them
and read back by nothing; Save as copies them with the rest.

**The two extensions say which file is which.** A `.ssw` is always one workspace and opens on
its own; a `.ssp` is always a project, and means nothing without the folder around it. Nothing
has to look inside a file to know what it is, and a file dialog can filter on it.

**A workspace file is portable; a project file is glue.** `workspace.rs` writes a
`WorkspaceFile`: a name, a kind, its own layout mode, the nodes that workspace writes and
every connection with both ends among them. It means the same thing in any project.
`project.rs` writes everything that is about *this* set of workspaces together — their order,
the node id counter and the workspace id counter, a cable whose two ends are written by
different files, the workspaces a shared node is on beyond the one that writes it, and the
[session](#session-state-which-are-open-which-is-showing-and-each-view): which workspaces are
open, which tab is showing and each one's view.

**A workspace's name, kind, layout mode and blurb are all in its own file**, because all four
describe the workspace rather than the set: they travel with an export and mean the same thing
in any project. Its picture is a `.png` of the same name beside it, for the same reason.

**There are exactly two kinds of glue**, and they are the two things a set of workspaces
knows that no one of them does:

- **`connections`** — a cable whose two ends are written by different workspace files. It is
  in neither of them, because neither may name a node it does not hold, and it comes back
  through `Graph::connect` like every other saved cable: one the graph refuses is a warning
  and the project still opens.
- **`shared`** — the workspaces a node is on *beyond* the one that writes it. Membership is a
  fact about the project rather than about any one workspace, so a file lists the nodes it
  writes and says nothing about where else they are seen.

**A node is written once, by the first workspace in project order among its set.** Nothing
stores that choice: it follows from the order, which is silvia's rule — whichever tab
serialized a node first owned it. So reordering the workspaces moves a shared node between
files at the next save, and Save rewrites every file anyway. Storing the owner would give the
project a second opinion about a question the order already answers, and the two would drift.
The consequence worth naming is that **no workspace file refers to a node it does not hold**,
which is what makes one portable at all.

A workspace's file name is derived from its name, sanitized and deduplicated, and the mapping
is kept in `project.ssp` — so renaming a workspace does not lose the file its nodes are in.
Deduplicated against every name the last save wrote, a removed workspace's included: that
file is deleted by the save, so a new Tunnel where a removed one stood is `Tunnel-2.ssw`.

The file is a DTO, not a serialized `Graph`, for reasons that hold for both kinds:

- `Graph` carries derived views that have no business in a file.
- **Ports are not saved.** They are rebuilt from the registry on load, so a node whose
  definition gained a port since the file was written opens with that port present. Saving
  them would freeze every node's shape at whatever it was when somebody hit Save.
- **Controls are defaulted first, then overlaid** with the file's values, so a control added
  since the file was written arrives at its default rather than absent.
- **Connections go through `Graph::connect`**, so a hand-edited file cannot describe a graph
  the editor could not have produced. A cable that would close a cycle is dropped, reported,
  and the rest of the project still opens.

Node ids and workspace ids are saved and restored exactly, which is what `insert_node` exists
for, and **the counters are saved with them**: they are the project's, so every node in every
file of a project has an id no other node will be given. Ids appear in generated WGSL
function names, so a reopened project has to reuse them or the names describe different nodes.

**There is no backwards compatibility and no migration.** Every field is required except the
ones a later step will add. A loader that reads yesterday's shape is code kept forever for
nobody, and this is the moment in a program's life when that is true.

### Three tiers of saved state

Everything remembered lives in one of three places, and one question decides which:

| tier | file | test for membership |
| --- | --- | --- |
| **workspace** | `workspaces/*.ssw` | changes what the workspace *is*. Export it to another project and this must come with it: its nodes, its cables, its name, kind, blurb and layout mode. |
| **project** | `project.ssp` | about the whole patch rather than any one workspace: the order, the glue, the session, the assets, the MIDI map, what the Main Input plays and how it is tuned. |
| **preferences** | `preferences.json` | about the person. Two people opening the same project may reasonably disagree, and the project still means the same thing: the window, the Status box, the recent list, whether each side panel is folded. |

There is a fourth kind, and it is the one that is written **nowhere**: the **rig**. Which
Output is on a deck, where the fade is, which crossfade and resolution, whether the mix is
painted behind the canvas, and which camera, capture device or screen the Main Input has open.
All of it starts at its default every run. The Main Input's clip, sound file and tuning are
the project's and ride in the manifest; Open brings those back and switches no device on
(`MainInput::restored`).

That is a decision and not an omission: a project is the patch, and the rig is what is in the
room tonight. A project opens the same on any machine, on a laptop with no camera, with
nothing already on air and the canvas not already covered by a mix. See
[decisions.md](decisions.md#the-mixer-does-not-persist-and-the-main-input-does).

The test settles arguments that otherwise recur. A layout mode is workspace data, because a
workspace laid out as a strip *is* laid out as a strip for whoever opens it; what would be a
preference is the default mode for a new workspace. Which tabs are open is project data,
because the set of tabs a show is mixed with travels with the show. Window geometry is a
preference, because a laptop and a rig machine differ in it and the show does not. Which
camera is plugged in is none of the three, because it is a fact about tonight. The
preferences file itself is described in [ui.md](ui.md#preferences); there is exactly one
store per tier, which is the whole of what this table is for.

### Assets

**`assets/` is the project's media, and every file that reaches a node is copied into it.**
A drop on the window, the file button's dialog and a paste of nodes copied in another project
all call `Project::import_asset`, and what lands in the node's option is the reference it
returns: `assets/<name>`, relative to the root,
with forward slashes always and joined natively on load. So a project folder can be moved,
zipped and sent, and every reference inside it resolves at the other end. A name collision on
the way in gets a numeric suffix — `clip.webm` and `clip-2.webm` — and the same file imported
twice is one asset, by a fingerprint of its bytes. A file already in this project's `assets/`
is not copied again.

**Nodes never see the root.** `TickContext::option` stays a string and `TickContext::path`
resolves it through the project, which is what keeps `nodes/` from knowing where a project is:
`cpu.rs` declares an `Assets` trait, `project.rs` implements it, and `Synth::tick` is handed
an `&dyn Assets` rather than the project, which it passes to the context. A reference that is
not under `assets/` — an absolute path somebody typed into the option by hand — resolves to
itself, so nothing is forced into the folder.

**`cache/` is what was derived from the media**: a clip's all-intra transcode and its decoded
soundtrack, named by a fingerprint the way [media.md](media.md#video-files) describes. It is
in the project rather than in `$XDG_CACHE_HOME` so that a folder carries its own transcodes
and plays on another machine at once — the argument is in
[decisions.md](decisions.md#assets-and-the-cache-live-inside-the-project). It is still a
cache — made on demand, never listed as an asset, never written by Save, and deletable at any
time for the cost of a re-import.

**Which nodes use which asset is computed by walking every node's `Asset` options** — the
[option kind](nodes.md#option-kinds) that names a file, so the walk asks what a node declares
rather than inferring it from an option that happens to hold a path. That walk is what the
project tab's asset cards show — which nodes on which workspaces reference each file — and each user
goes there when it is clicked.

**Removing an asset is a gesture on the project tab, not something a save does: Save deletes
nothing under `assets/` or `cache/`** but a painting file it wrote and no longer names — see
below. `Project::remove_asset` is **refused while anything
references it**, and says by what, because a node pointing at a file the project no longer
holds is a silent break and the usage list on the card is the answer to what to do about it.
An unreferenced one goes, and **what was derived from it goes with it**: a transcode of
something that is gone is bytes nobody will ever ask for. Everything a source produces is
named after that source — `clip::cache_path` carries the source key as well as the settings
hash — so the entries can be found without knowing which codec somebody transcoded it with
two months ago.

### Export and import

**A workspace leaves a project by export and enters one by import**, and those are the only
two gestures that touch a file outside the folder. One rule places them both: *a thing enters
a project at the list and leaves from the thing.* Import is at the top of the project tab's
Workspaces section, and dropping a `.ssw` on the window is the same import; Export is on the
Workspace menu and on the card.

**Export writes every node on the workspace as a plain node**, shared or not, with every cable
whose two ends are both here, its picture, and every asset those nodes reference — a painting's
picture file among them — copied under `assets/` beside the file — with the references rewritten, so the exported `assets/<name>`
resolves wherever the folder ends up. That is the point of an export: the file works on its
own, and being shown on three other workspaces is a fact about *this* rig.

**If the folder is already a project the files go into its `workspaces/` and `assets/`**,
name-deduplicated, so *export into that project* is the same gesture as *export to a folder*
and the other project [adopts the stray](#saving-and-opening) next time it is opened.

**The report says exactly what stayed behind**, to the status line and the log: cables to
nodes not on this workspace, the other workspaces each shared node is on, assets that could
not be found, and MIDI bindings — nothing binds anything yet, and the line is there so the
report is already the whole answer when `midi/` lands. Silence about the glue would be the
wrong answer, not a shorter one.

**A kind that means nothing on its own is refused by name.** `WorkspaceKind::is_portable` is
where that lives: a `Video` workspace is nodes and cables and travels whole, and a timeline
will be every reference into other workspaces, so it travels with the project instead. No such
kind exists yet; this is the branch it lands in.

**Import remaps.** Every node arrives through `insert_node` under an id from *this* project's
counter and every cable inside the file is re-pointed to match, because a file's ids are
stable once it is in a project and mean nothing outside one. The workspace gets a fresh
`WorkspaceId` and is appended to project order, opened and shown. Its media is looked for in
the `assets/` beside the file — or `../assets/` when the file is inside a `workspaces/`
folder, which is where an export into a project put it — and copied in through
`import_asset`, so the references land pointing at this project's own copies; a painting is
read out of the same folder into its node, and written into this project's `assets/` by the next
save. One that cannot
be found is reported and the option is left pointing at it: a `video` node with a missing file
already shows its error.

**Importing is a `Command`.** Nodes appear, which is an edit like any other, so it is one undo
step and the snapshot covers it; and the command log stays honest about where those nodes came
from. It carries the file rather than the nodes, because the remap *is* the operation — which
ids they land under is this project's counter's answer, not the file's. Opening the tab
afterwards is not a command, because showing a workspace never is. The assets it copied stay
in the folder through an undo, for the same reason Save deletes nothing under `assets/`.

**A loose `.ssw` outside any project** goes through the same import: `supersilvia tunnel.ssw`
makes a project named after the file in the projects directory and imports it into that.

### Saving and opening

- **Save writes every file** — small, deterministic, pretty JSON with `BTreeMap`s, so a
  project diffs cleanly under git. A workspace deleted since the last save has its file and
  its picture removed; **Save deletes nothing else** but its own temporary files and the
  painting files it wrote and no longer names, so a file somebody put in the folder is safe
  and `cache/` and every other file in `assets/` are untouched.
- **A `drawingcanvas`'s painting is a PNG in `assets/`, named by its content** —
  `painting-<print>.png` — and the workspace file holds the reference, not the pixels. Save
  writes each painting not on disk yet first, synced and renamed into place, so the manifest
  commits a save whose pictures are already there; open reads them back into the node's value.
  See [decisions.md](decisions.md#a-painting-is-saved-with-the-project).
- **A save lands whole or not at all.** Each workspace file is written beside its own as
  `<file>.<save>.tmp` and synced; then the manifest, which carries the save's number, is
  written to a temporary name, synced and renamed over `project.ssp`. That rename is the
  commit. After it the workspace files are renamed into place and the removed ones deleted,
  and Open does the same first, before it reads anything, for a save that was cut off after
  its commit. So a save cut off at any file opens as the last save whole or as this one
  whole. Any other temporary workspace file is what a save that never committed left, and
  both delete it. Save as… copies the folder — the media, the pictures, `renders/` and
  `snaps/`, which are the person's own work — but not `cache/`, which the copy makes again on
  demand the first time a clip plays, nor `.autosave/`.
- **Save also asks for a picture of every open workspace** — the workspace's first Output in id
  order, or the selected preview Output when it is on that workspace. It is *asked for*: the
  readback lands a frame or two later, because [nothing waits on the GPU for a picture of
  itself](rendering.md#the-thumbnail-readback), and the status line says *saved friday,
  thumbnails pending* until the last one is written. An [idle](rendering.md#which-outputs-draw)
  Output is drawn for the tick the request lands on. A closed workspace keeps the last picture
  it had, and one whose Output is unplugged or suspended has none — the card draws its
  placeholder.
- **Unsaved edits are autosaved into `.autosave/`**, a project folder of its own inside this
  one — its own `project.ssp` and `workspaces/`, written by the same save — at most once
  every thirty seconds (`app::autosave::INTERVAL_S`) while the document is dirty and has
  changed since the last write, and never over the project's own files. The frame only
  records the newest document, an `Arc` of the graph and a copy of the project's few
  fields; a thread named `autosave` serializes, writes, syncs and renames, one write at a
  time. A document undone back to the save has its autosave deleted, and so does every
  successful Save — Save as deletes the old folder's too and copies none — and a Discard in
  the confirm. The autosave's own file names are its own, since only its graph, session,
  MIDI map and Main Input are ever read back, and it copies no media: it refers to
  `assets/` as the project does.
  **Open offers it back** when it is newer than `project.ssp` and differs from what that
  opened as (`Graph::same_document`) — [the recovery question](ui.md#the-menu-bar)
  — and deletes one that is neither; one that cannot be read is left where it is and not
  offered. A project has a folder from the moment it exists (below), so there is no document
  the autosave cannot reach.
- **A stray file in `workspaces/` is adopted on open**: a `.ssw` the project file does not
  list is imported with an id remap, every cable inside it re-pointed, and reported. Someone
  who drops a workspace into the folder from a file manager gets what they meant.
- **Open replaces everything and clears the undo history.** It is not an undo step: undoing
  your way out of one project into the previous one is not a thing anyone wants at a show,
  and a snapshot from before the switch describes nodes the project on screen never had. New
  project is the same. They are the only two things that clear it.
- **Open replaces the running state too, and undo never does.** Node ids restart at 1 in every
  project, so what the synth keeps by id would carry the old project onto whichever node holds
  that id now: a playing transport, a count, a world, an Output's last frame for feedback to
  sample. So Open and New raise a **project** count that crosses with every graph, and a graph
  carrying a new one makes the synth drop every node's state, everything published, the plan
  and the decks, and the renderer every Output, probe, upload and world — the project starts
  as a launch opening it would, playing from zero, since nothing of the transport is saved. A snapshot of the old project that reaches the editor after
  the switch is read for nothing it says about a node, so no deck claim, recording, knob
  write, thumbnail, Snap or probe of the old project lands in the new one. An undo restores
  the same patch and raises nothing. **What is the run's is kept**: the clock, whose elapsed
  time is the uptime the Status box reads, MIDI's devices and log, and a picture window on the
  mix; a window on a node closes. The Main Input is not dropped either, but it follows the new
  project's choice, which the project opens with as it was saved, less any device. Open and
  New are refused while a render runs, since the render is reading the document.
- **A project always has a folder.** There is no unsaved-anywhere state, so Save never asks
  where. The first launch on a machine, with nothing recent, makes `Untitled/` in **the
  projects folder** — `supersilvia` in the person's documents folder unless the Preferences
  window chose another: on Linux `XDG_DOCUMENTS_DIR` as `user-dirs.dirs` names it, else
  `~/Documents`, `~/Documents` on macOS, and the Documents Known Folder on Windows, through
  `platform::dirs` — with one empty video
  workspace, and the status line names its whole path; Save as… copies the folder wherever it
  should live and switches to it. Open project… starts in the projects folder. **A projects
  folder that cannot be made or read is said, never swapped for another**
  (`project::projects_dir_problem`): the launch comes up on the empty document it started
  with — a scratch project in the system temp folder, whose Save is Save as…, so a Save never
  writes into the temp folder — and the status line says what failed and, on a Mac, what to
  allow in System Settings ▸ Privacy & Security ▸ Files and Folders; New project says it in its window, Open project…
  on the status line before its dialog opens wherever the desktop opens it, and the
  Preferences window under the folder's path.
- **`supersilvia friday/`** opens a project, as does `supersilvia friday/project.ssp` or any
  path inside the folder. **`supersilvia tunnel.ssw`** outside a project imports the file
  into a new project named after it, in the projects folder — the one case where "there is
  no file that is not in a project" has to be said by the program.

Failures split the way [the compiler's do](#degrade-loudly-never-silently). `LoadError` means
the project could not be read at all and what is on screen is untouched; `LoadWarning` means
it opened with something dropped or answered another way — a listed file that is gone, a stray
adopted, a cable refused, a node kind rebuilt out of cables — and each one is reported rather
than swallowed.

**Unsaved edits get one confirm**: Quit, Open and New
ask Save / Discard / Cancel, and a Save that fails leaves the question up, saying why, with
nothing quit, opened or replaced. Whether there is anything to confirm is two integers — every
edit takes a serial, undo and redo restore the one their step carried, and the last save
records the one it wrote, so undoing back to what is on disk answers *no* without anything
diffing a graph. The window title carries the folder name and a marker while it differs.

## The recompile boundary

The property that makes the instrument playable, and the one most worth protecting:

> **Changing what the graph *is* costs a shader rebuild. Changing what it *does* costs a
> float.**

`AddNode`, `RemoveNodes`, `Duplicate`, `Connect`, `Bridge` — which adds a node and its cables
— `Disconnect`, `DisconnectEdge`,
`DisconnectAll`, `ImportWorkspace` — which brings nodes in — and `RemoveWorkspace`, the one
workspace command that can take a node with it, mark downstream Outputs for rebuild.
`SetControl`, `ResetControls`, `MoveNodes`, `SetCollapsed`, `SetNodeWidth` and the other seven workspace commands never do: a workspace is a view, so moving a node between views
changes no shader. `ResetControls` is on the cheap side because it writes controls and
nothing else — the options it leaves alone are the part of a node that changes generated
code. Control values live on the node and the
renderer reads them each frame to set a uniform; a control value is never baked into the
WGSL.

**`SetOption` asks the option.** Only an option whose [kind](nodes.md#option-kinds) is `Code`
is in the shader: a `Uniform` option is an `int` the running program already reads, an `Asset`
names a texture the renderer binds, a `Runtime` option is read by a `tick` or by the renderer
on the next frame, and a `Presentation` option reaches no shader at all. So four of the five
kinds sit on the cheap side of the boundary with `SetControl`, and which side an option is on
is not a judgement call — `tests/compile.rs` compiles every option's choices and asserts that
a `Code` option changes the WGSL and that nothing else does.

**A measurement adds no clause.** No Output compiles a measurement — a tap cabled into one
is a pass-through there, see [cpu.md](cpu.md#taps-the-other-direction) — so an edit marks
stale only the Outputs downstream of it (`Graph::downstream_outputs`), and a cable upstream of
a tap that reaches no Output marks none. The measurement lives in its workspace's pass, which
the app builds again on every change to the graph's shape and sends only where its source
differs ([rendering.md](rendering.md#the-workspace-pass)); cabling a tap into an Output or out
of one leaves it there.

`tests/controls.rs` asserts all of this rather than trusting it, including that the generated
shader does not contain the number, that swapping an asset or changing what is drawn
rebuilds nothing, that a cable upstream of a loose tap marks no Output, and that cabling a
loose tap into an Output leaves its measurement in the pass. The `Uniform` half is in `app/mod.rs`,
because the only node carrying an option of every kind is the `cfg(test)` fixture.
