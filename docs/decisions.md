# Decisions

Consequential choices and the alternatives that lost. This file exists so a rejected path is
rejected once: if an idea here looks appealing, the reason it lost is written down.

Everything else in `docs/` describes only what is true now. This is the one place a superseded
idea is named — and the one place a decision may run ahead of the code, marked **Agreed, not
built**.

---

### Native only; no browser build

**Chosen.** Video decode, audio threading and asset handling each needed a
second implementation, which is the wrong shape for a one-person performance tool. Everything
cheap about the web target — the shaders, the canvas — was already shared; everything
expensive was still ahead. silvia is the web thing and already exists.

Removed: the wasm entry point, Trunk, `web-sys`, `web-time`, the wasm32 target,
`compile::Target`.

Gained: RGBA16F render targets, threads for audio and video decode, `std::time::Instant`.

**Kept open cheaply:** shader bodies are WGSL, which is WebGPU's own language, so a browser
build later would be re-adding a shell rather than rewriting every node.

### No cross-implementation shader compatibility with silvia

**Chosen.** There is no diff test against silvia's output and no compatibility obligation.
Port node bodies freely — rewrite them, split them, change what is a parameter. The
correctness check is `insta` snapshots of *our* generated WGSL.

The naming scheme (`{slug}{id}_{key}`) is kept anyway, on its own merits: unique without a
symbol table, greppable, derived from node identity.

### `NodeDef` as a `const` struct, not a trait

**Chosen** over the trait in the kickoff's §5. Measured on a spike emitting identical shader text
both ways: 49 lines against 55 for `checkerboard`, 39 against 44 for `channelsplitter`.

The structural reason outweighs the line count. A trait attaches the generator to the *node*
and forces a `match port` with an unreachable arm; the data form attaches it to the *port*.
36 of silvia's 146 nodes have two or more generators and `math.js` has 14, so that dispatch
is not an edge case. It also turns a compile error into a runtime panic: an `OutputDef` added
without a matching arm compiles fine.

**Rejected because:** the trait's advantages — a default `tick` method, editor scaffolding,
and letting another crate add nodes — are worth little here. The last is moot in a single
crate.

### A node holds its definition, not copies of it

**Chosen.** `Node::def` is a `&'static NodeDef`, so everything about a node's kind is one
pointer away from every reader — the tick, the compiler, the bus, the canvas — and
`nodes::find` is for a slug arriving as a string.

**Rejected: stamping the layout bits onto the `Node`** — regions, width, the tick and heading
counts — so layout would not search the registry. It traded a search for copies that rode in
every undo step and across to the synth, and a construction route that skipped the stamp drew
the node as a different one; the pointer is cheaper than either the search or the copies.

**Rejected: a measured height on the `Node`.** How tall a note's text came out is what this
canvas measured, not what the document says, so it lives in `canvas::Measured`; written to
the graph, a note drawing itself copied the node out from under the undo step sharing it.

**Rejected: the cable index as a view rebuilt after each change**, beside the adjacency. The
tick asks what feeds each input of every CPU node every tick, so a rebuild would be paid on
the first tick after every cable edit, and a load that seats cables one by one would rebuild
between each; kept beside the list, the index costs a push per cable.

**Rejected: the region's drawing function on `NodeDef`.** It made `nodes/` — and through
`Node`, `graph/` — name the toolkit. The registry names a `nodes::Region` and `widgets::def`
keys the drawing on it.

### Time is an input port, not an ambient global

**Chosen.** Every node that moves with time has a **Time** input, key `clock`: a diamond with
no knob, in the node's own cycles. Unplugged, it reads ambient time — the transport's
playhead — at a rate the node declares (`NodeDef::ambient`), which the synth writes under the
input's own key every tick and the compiler reads as a published uniform. Plugged, what
arrives **replaces** it, usually a gear's Cycles. The body reads its Time and never `u_time`,
so time-warping, a clock of another rate and scrubbing are ordinary graph operations.
[proposals/time.md](../proposals/time.md#the-rules) is the argument.

Only 12 of silvia's 146 nodes touch `u_time`, and 11 of those 16 references are
`u_time * speed` — precisely the discontinuity `phaseAccumulator.js` spends 286 lines
smoothing. Here there is no speed to multiply: a node computes `Time + Offset` each frame and
integrates nothing, and a rate is set, turned or divided only in a gear
([Gears](#gears-the-one-place-a-rate-lives)).

**Why a diamond.** Time says which moment the node is at, one number per node, and the CPU
needs that number: for a sequencer's crossing, a clip's frame, the palette region's drift. A
lag per pixel is Offset's job, the circle beside it, so a field cabled into Time is an
ordinary type mismatch.

**Rejected: a speed on every node, integrated against the transport**, with the cable added
on top — the design this one replaced, and why it lost is under [Nodes keep no
time](#nodes-keep-no-time-and-a-loop-is-read-from-the-gears). **Rejected: a cabled time
multiplied by the speed**, `time × speed`, which is the jump
the accumulator removed, one level down. **Rejected: an unconnected Time bound to `u_time`**
(`Control::Global`): `u_time` has no rate and no period, so each node would multiply it again
in its body, and a node's reading would wrap rather than come round its own cycle. The
per-node reading is `playhead × rate` from the `f64` playhead, published as a count, which
the body takes round the node's period. **Rejected: Time added
rather than replaced.** Replacing is what makes a gear drive a node exactly; the added input
is Offset.

**Rejected: a Clock input on every node**, with a rate on the node read against whatever is
cabled there: the same choice offered on every moving node, and three time rows on each. A
gear offers it once. **Rejected: a Speed and an added Phase on each node**, multiplying to set
the rate and adding to set the offset on the node itself: every moving node kept a speed and
the state it integrated, and a loop needed a ratio beside each speed to close. The rate went to
the gears and the added input is Offset. **Rejected: a port type for time**, a phase carrying a
count and a fraction with periods the compiler follows. It would keep the count exact and
check a loop at compile time, at the cost of a new wire, its conversions and a second way to
say time; time is plain floats in cycles. **Rejected: a rate field**, a
speed per pixel integrated into a state texture per Output: two pixels whose rates differ drift
apart without bound, so the picture shreds into noise within minutes, and an added Offset
expresses every per-pixel rate that does not.

### Offset, added, in the node's own cycles

**Chosen: a gear sets the rate, Offset sets where it is.** Beside Time every moving node has
**Offset**, key `phaseOffset` (`nodes::phasor::OFFSET`), with a small knob: 0 to 1 is exactly
one of the node's cycles, it is **added** every frame, and it wraps. It is a varying circle
wherever the node draws, so the distance from the middle into it is a ripple with one cable,
and a uniform number on the CPU nodes, whose tick has no pixel. It is the generators' and
transforms' old Time, `video`'s and `imagegif`'s Position and the oscillator's Phase under one
name and one unit. A node's cycle is its longest period: Rotozoom and Shaky Cam take the 20π
every one of their periods divides, so an Offset of one brings both motions back together. A
node with no period takes its natural unit — a lattice cell on a noise, a roll on Static, a
unit of depth on the Tunnel — and there Offset does not wrap unless the node's Repeat is on.

**The name is Offset**, because it is what the input does: it is added. The gears publish
**Phase**, the 0 to 1 reading of a cycle, so a Phase cabled into an Offset places the node at
that phase. **What it cost**: eight ports already said Offset for a level or a distance, and
each is relabelled, the key kept so no file changes — the oscillator's Level, the Clock's
Zone, the gradients' and Kaleidoscope's Shift, Chromatic Aberration's Spread, Convolve's Gray
and the Star Gate's Position. Translate's X and Y Offset, Tile's Offset X and Y and the Slime
Mold's Sensor Offset keep theirs, since a compound label cannot be mistaken for the time
input. **Rejected: Phase** as the input's name, which it carried while the rate was a speed
on the node: it gave Cosine Gradient three things called Phase and four generators an input
and an output of one name, and beside a Time input it no longer says what it adds.

**Rejected: Time in seconds of the node's motion**, as first built. It matched cycles only
where a node's period was one, so the same cable meant a sixty-third of a cycle on Rotozoom
and a whole one on Mandelbrot. **Rejected: Position, the oscillator's Phase and Time as three
inputs** for one thing. **Rejected: an Offset per motion on Rotozoom**, four
time rows on a node already nine inputs tall. Shaky Cam has a Time and an Offset per axis, as
decided on 1 October, so its Y can run on a gear of its own; the four rows fold
under its Time heading.

### wgpu, not glow

**Chosen**, reversing glow, for one renderer on every operating system: a native macOS app on
Apple Silicon through Metal, Linux through Vulkan and Windows through Direct3D 12, from the
same code. **Why glow lost:**
macOS's OpenGL stops at 4.1, deprecated, and 4.1 has no shader storage buffers, no atomics in a
fragment shader and no compute shaders. Taps are atomics into a storage buffer from inside a
fragment shader, the cost probe is the same, and `slimemold` is compute kernels over storage
buffers — none of it runs on a Mac through GL. Keeping glow on Linux and writing Metal for the
Mac would be two renderers, which is the shape this project refuses everywhere else. The
editor paints through eframe's wgpu backend on the same device. See `proposals/wgpu.md`.

**What glow had going for it, and what it costs to give up:** silvia's node library was GLSL
that ported almost verbatim, where wgpu takes WGSL (below, decided in
`proposals/shader-path.md`); and binary size, since wgpu and
naga are several megabytes, recorded rather than gated. **No Vulkan, no app**: a Linux box with
no Vulkan driver refuses to start and says why, with no fallback to GL.

### Direct3D 12 on Windows, its shaders compiled by a DXC linked in

**Chosen**: wgpu renders on Direct3D 12 alone on Windows, and compiles naga's HLSL with the
DirectX Shader Compiler, linked into the binary by `mach-dxcompiler-rs`. Why: every Windows GPU
driver carries Direct3D 12, GStreamer's Direct3D 12 decoders hand their frames out as Direct3D
12 textures, which a device of the same API can sample with no copy, and DXC is the compiler
Microsoft maintains. **Not taken: Vulkan on Windows**, which Windows' drivers carry less evenly and which
would need the decoder's textures shared across APIs; nor Vulkan as a fallback beside it, which
would be a second backend to test for a machine that has none. **Not taken: FXC**, the compiler
that needs no library, which is slow and miscompiles the larger modules naga writes. **Not
taken: DXC as `dxcompiler.dll` beside the binary**, two more files to carry and find for the
same compiler; the cost of linking it is about 22 MB of binary. **A decoded frame is the
decoder's texture on the renderer's device**, since Direct3D 12 hands a process one device per
adapter and GStreamer is handed the renderer's; **shared NT handles are the fallback** for a
device that is another — vkd3d's, under Wine — **not the path**, since opening a handle per
frame and waiting for its writes on the CPU buys nothing where the device is already one.

### The strongest GPU, with the variable choosing another

**Chosen**: the app renders on the strongest GPU the machine offers — a discrete GPU of any
vendor, NVIDIA included, before an integrated one — and `SUPERSILVIA_ADAPTER` chooses another:
`integrated`, or a piece of a name such as `intel`. Why: an NVIDIA card is what a person with
one wants used, and a machine that wants its iGPU instead sets the variable. The tests,
benches and `check.sh` ask for the integrated GPU themselves (`adapter::Asked::integrated`),
so their pixels match the snapshots, rather than the app refusing anything. **Not taken:
refusing NVIDIA beside another GPU**, which made every NVIDIA owner with an iGPU set a
variable to use the card they bought. **Not taken: judging NVIDIA from the hardware** (sysfs's
display-class PCI devices, so a failed iGPU driver could not make the NVIDIA card look
alone), which existed only for
that rule and was machinery with no gain once the rule went. See
[rendering.md](rendering.md#one-device).

### WGSL for wgpu, not GLSL through a translator

**Chosen** for the move to wgpu: every node is written in WGSL, and naga — already inside
wgpu — takes it to MSL, SPIR-V and HLSL with nothing between; `tests/shader_targets.rs` puts
every module through all three.
**Rejected:** keeping GLSL and translating it through glslang to SPIR-V, which took 420 of the
421 shaders the program wrote but costs a C++ build, 3 MB of binary, a permanent dialect
transform in the compiler and a second reader of every program; and naga's GLSL frontend,
which has no atomics and so could take no tap and no probe. Drifting from silvia's GLSL is
fine (above). See `proposals/shader-path.md`.

### No node-editor crate

**Chosen.** `egui-snarl` and `egui_node_graph2` exist and are not used. The canvas is the
product; owning it means no upstream to fight when the design system says something specific,
and the design system specifies the look anyway.

### Blit in a paint callback, not a registered texture

**Chosen.** egui_wgpu's `register_native_texture` would have to be redone every frame, since a
ring rotates the texture an Output shows every tick, and the fit, the corner and the flip would
live apart from the blit. A paint callback is the same operation into a smaller rect, needs
none of that, and deletes more than it adds.

**Generalization worth remembering:** most apparent platform constraints have a shared
solution simpler than the platform-specific one. Reach for the shared path first.

### A menu bar, not a floating hamburger

**Chosen**, reversing an earlier recommendation. The argument against a menu bar was that
permanent chrome across the top costs vertical space in a fullscreen performance surface.
That argument does not apply: **what an audience sees is a separate window** — a picture
window, on its own Wayland surface — so the editor window's chrome costs nothing during a
show. A menu bar is more
usable and more discoverable than a hamburger.

The design system's `AppMenu.jsx` specifies the hamburger; we diverge from it here
deliberately.

**Native on the Mac, not on Linux — one model under both.** A true global menu bar on KDE
needs `com.canonical.dbusmenu` over DBus plus either an X11 property or the
`org_kde_kwin_appmenu` Wayland protocol, and winit exposes neither, so Linux draws
`egui::MenuBar`: in-window, keyboard-navigable, in the accessibility tree. A Mac app without
the screen's own menu bar reads as a port, and there it is AppKit's. What both draw is
`ui::menu::model`, the menus as plain data — so an entry, its enable, its shortcut and its
action are decided once, and the egui bar stays the one any future build without a native
bar (a browser) draws. The Mac moves Settings… and Quit into the application menu and adds
About, Services, Hide and a Window menu, which is where a Mac keeps them, and answers Help's
two entries with that About.

**`objc2-app-kit` rather than `muda`.** `muda` would be a new package and its own event
channel for about a hundred lines of `NSMenu`; AppKit's bindings were already in the lock.
The cost is one more file allowed `unsafe`, `platform/macos/menu.rs`.

**The clipboard keys go through AppKit first.** A key equivalent on a menu entry is the
menu's before the window sees it, so `⌘C` on Copy never reaches egui-winit and a focused
text field would lose its copy to the nodes. A clipboard entry chosen from the native bar is
replayed into egui as the `Event::Copy`, `Event::Cut` or `Event::Paste` the key would have
made, and meets the same guard. A disabled entry does not take its key, so `⌘C` with no node
selected goes to the window as before.

### Undo says what it did, and the keys are one table

**A step's name is read off the command it already holds**, against the graphs either side of
it, rather than a label written beside the step when it is made. There is nothing to keep in
step with the command, and a name follows the labels the registry gives nodes and rows.

**The toast after an undo offers ▸ go only where the change is out of view.** A go on every
undo would be a button beside every *Undid Move 2 nodes* whose node is under the pointer.

**The Undo History is a window, not a submenu of Edit.** The ring holds up to 256 steps, and a
menu that long is a list no one can scroll; a window keeps one size and draws the rows in view.

**The shortcuts window is `F1` and `Ctrl`+`/`, not `?`.** `?`, with `/` and `` ` ``, opens the
node browser at the top of the canvas — silvia's quake bar — and it stays that.

**The window is one table that tests hold to the bindings**, not a page written by hand. When
the review counted, about thirty bindings existed and nine were printed anywhere: a list kept
apart from the keys is a list that falls behind them.

### Third-party notices: a committed file held to the lock

**Chosen.** Linux's crate notices are `packaging/linux/rust-crates.txt`, written by
`scripts/crate-licenses.py` — the script the Mac bundle's own are written by — committed, and
compiled into the binary for Help ▸ Licences. `tests/notices.rs` writes them again from
`cargo metadata --offline` and fails where Cargo.lock has moved away from the file, so they
cannot go stale in silence, and no build needs the network, Python or a tool of its own.

**Rejected: writing them in `build.rs`.** It would run `cargo metadata` from inside a build,
which contends for the package cache the outer build holds, and Python or a JSON parser as a
build dependency in every build — the Flatpak's offline one included — to save one command
after a lock change that a failing test already names.

**Rejected: reading an installed file at run time.** A source build has none, and the three
Linux layouts put it in three places; the file is shipped beside the binary as well, under
`share/licenses/`, for anyone reading the package rather than the app.

**Rejected: `cargo-about` or `cargo-deny`.** A tool every contributor installs, for what one
script already does for the Mac.

**The Mac's Help menu has no About or Licences.** Its application menu already has AppKit's
standard About panel, whose `Credits.rtf` is the licences in brief and names the folder in
the bundle that holds all of them, so the model's two entries are answered by that panel, and
Help keeps the rest: Report a problem….

### RGBA16F render targets

**Chosen** once the browser target was dropped. At 8 bits a small feedback mix amount rounds
to zero, so a trail dies abruptly and leaves dead zones where a channel never moves.

**RGBA32F rejected:** doubles memory again for precision no one can see in a decay.

### The UI defers to the render, not the other way around

**Chosen** as the rule that settles any argument about canvas cost. The output is the
product; the editor is how you steer it. An editor interaction that costs the render a frame
is a bug even when the editor itself feels fine.

Three mechanisms follow from it, all described in [ui.md](ui.md): off-screen culling,
quantized font sizes, and a grid pitch that steps with zoom. Each was found the same way —
zoom out on a real graph and watch the editor's worst frame.

**Rejected: running the UI at a lower rate than the output.** Decoupling them means a second
frame loop, and silvia's 45 independent ones are what this rewrite exists to fix. It also
does not address the actual failure, which was one UI frame costing several milliseconds, not
UI frames being too frequent.

### Canvas fonts are quantized to whole pixels

**Chosen.** egui caches glyph rasterization per font size, so `base * zoom` misses that cache
for every glyph on screen on every frame of a smooth zoom. `theme::font_size` rounds to whole
pixels, which bounds the number of distinct sizes. Measured on an 8-node graph: idle 0.94 ms,
first zoom sweep 2.61 ms, a second sweep over the same range 0.53 ms.

**The cost is real and accepted:** text steps rather than glides while zooming, and only
while the zoom is moving.

**Rejected: caching scaled `FontDefinitions` ourselves.** That is the cache egui already has,
one layer up.

egui coalesces scroll events within a frame, so input rate was never the problem — but it
*smooths* them across frames, so one trackpad flick yields distinct zoom values for many
frames after the fingers stop. A continuously varying quantity has to be cheap to render, not
merely rare.

### Culling drawing, not geometry

**Chosen.** An off-screen node skips drawing and hit-testing; its port positions are still
computed, because a cable may run from off-screen to on-screen and still has to be drawn.

**Rejected: a spatial index.** Node counts are in the hundreds, and `rows()` over every node
is cheap once it does not allocate. The expensive part was painting and interacting, which is
exactly what the cull removes.

### A worst, not a mean frame time

**Chosen** for the Status box's Editor row: the longest *unclamped* frame in the last two
seconds. A
mean hides the failure that matters — sixty good frames and one 200 ms frame average to
something respectable and feel terrible. Unclamped because a stateful node's 100 ms clamp
exists to protect simulations, and would otherwise hide the stall it is protecting them from.

### The compositor paces the viewers; the synth paces itself

**Chosen.** The synth runs on a thread of its own, ticking once per display interval from a
deadline it carries forward, and every window — the editor, and every picture window — is a
**viewer** that waits for its own compositor and blits the newest texture the synth published.
The two are not coupled in either direction: a window that is slow, occluded or minimized
delays its own blit and nothing else, and a synth that misses its deadline is reported rather
than absorbed.

**What that buys is the case a compositor-paced loop cannot do**: the editor minimized stops
its own presents, and the world goes on. Measured on a 100 Hz display with the editor minimized
for a full minute — the synth ticked at 100.2 wakeups a second throughout, `late` stayed at
zero, and the accumulators came back exactly where wall time
said (a `perlin` at 0.5× reading 288.8 after 577 seconds). What the old loop returned from a
minimized second was one `dt` clamped to 100 ms, which leaves every oscillator permanently
behind and a feedback patch with no iterations at all.

**`pre_present_notify` stays rejected, and the measurement was repeated to be sure.** The call
asks winit for the surface's frame callback and gates the window's next redraw on it, which is
the right *shape* for a window that owns nothing — a minimized editor would get no callbacks,
never redraw, never present and never block the thread its other windows paint on. It was tried
again with the tick on its own thread and it still costs what
[proposals/pacing.md](../proposals/pacing.md)'s second finding says: eframe polls the event
loop for as long as a redraw is held, so asking for it every frame holds eframe in
`ControlFlow::Poll` for the life of the run. **97.9% of a core with the call, 17.2% without**,
same box, same window, same graph, measured under glow. Asking for it only while minimized does nothing at all,
because the pass that would ask is the pass that has stopped running.

**A minimized window stops painting instead.** It asks for its next repaint a quarter of a
second out rather than one display interval, so eframe runs no pass for it, nothing presents and
nothing blocks — **0.0% of a core, measured under glow over a full minute**, against 17–29%
painting. The synth never sees any of it: 100.2 ticks a second throughout, `late` at zero, and
100.3 ticks a second over the whole 3 m 45 s run. On wgpu the same minute is to be measured at
step 7 of `proposals/wgpu.md`.

**What that cost, and how it was paid off.** A minimized editor also stopped the projector.
It was a deferred viewport — an immediate one is painted and swapped *inside its parent's
pass*, so it would stop either way — and a deferred viewport's redraw is still serviced by the
parent's event loop, which is idle. The only way measured to keep it painting was
`pre_present_notify` every frame, and that is the spinning core above: the polling is what
services the viewport. A core spinning all evening was not a trade worth a window nobody is
looking at. The line that closed it was already written here — *giving the projector a window
and an event loop of its own is what would buy both* — and that is what
**[picture windows](#every-picture-window-is-a-wayland-surface-of-its-own)** below does.

**Rejected: our own cadence on the frame thread.** Vsync off and a timer, the cheap shape of
a loop of our own, was measured under glow and is worse through Mesa's EGL, which stalls
waiting for the compositor to release a back buffer with nothing throttling the client — and a
minimized window never releases one. The loop that works is a thread of its own.

**Accepted, with the gaps named.** While the editor is minimized nothing on the frame thread
runs: no plan is republished, no probe count or thumbnail is collected, no command is applied.
**The world advances and the routing does not.** Two things had to cross that line, and both
cross it the same way — applied inside the tick, on the state the synth owns, and reconciled
into the document a frame later. A **deck claim**, because `Show on A` is an action input and
a sequencer can cut with nobody watching. And **MIDI**: the reader's queue is drained at the
top of every tick, a bound CC writes the control on the graph the synth holds and a bound note
is the same press a button sends, so a controller plays the world with the editor away.

What is left of MIDI on the editor's side is **learning** — it needs the control under the
pointer — and the **monitor**, which is a window nobody is reading while it is minimized. The
map itself crosses on every change, like the graph. The rest is what a minimized editor means
— and no longer includes any picture: every picture window paints on the pictures thread and
is not eframe's at all.

**Rejected: a lock around one graph.** The synth locking the editor's graph each tick is the
coupling being removed, pointed the other way: every long lock on the frame thread would block
the synth. The graph crosses as a **move** instead — `Graph` memoizes its ancestor walks
behind a `&self`, so two threads holding one would be a data race and not merely a `!Sync`
inconvenience.

**Rejected: an event accumulated in the snapshot's buffers.** The mailbox hands the synth back
a buffer the editor never took, so what it carried went out again behind what came after it:
an older deck claim beat a newer one and a recording landed after the Clear that emptied it.
Each kind is an ordered log the synth keeps, counted, and applied past the editor's own count
([architecture.md](architecture.md)). **Rejected with it: republishing a log's whole tail
every tick**, which MIDI's messages did. It cost the log's length on every tick and would hold a
Snap's frame for as long as the frame stayed in the tail; the editor acknowledges by handing a
buffer back, and the synth keeps only what it has not acknowledged.

### Every picture window is a Wayland surface of its own

**Chosen.** A node's pop-out, a fullscreen picture and the mix are one kind of thing — a
**picture window** — and none of them is an egui viewport. They live on a thread named
`pictures`: a `wayland-client` queue on eframe's own `wl_display`, an `xdg_toplevel` with no
decorations each, a wgpu surface on each, and a `Viewer` per surface format, on the one
device the synth draws on. Each window is paced by its own compositor's frame callback, so a minimized editor is not something
any of them can observe. **The projector is retired**: it was a window of its own kind, and the
mix is now a picture like any other.

This pays off the cost the decision above named. Measured on Linux (KWin, Wayland) under glow, editor
minimized for a full minute with the mix popped out: two captures of the picture window ten seconds apart
differed in **57% of their pixels**, the synth held `late 0` across the whole run, the pictures
thread cost **1.6–1.7% of a core** and the frame thread 0.3%.

**Rejected: a second winit event loop.** Not a matter of taste — winit permits one per
process, as a static: `EVENT_LOOP_CREATED` in `event_loop.rs`, and a second is
`EventLoopError::RecreationAttempt`. The flag clears only when the loop is dropped, and eframe
holds ours for the life of the process.

**Rejected: patching eframe to paint viewports off its own thread.** eframe paints every
viewport from its one event loop, in turn — so "paint that viewport over there" means a
windowing loop per viewport, which is this decision, written inside someone else's crate where
it cannot be tested.

**Not taken: a Wayland connection of our own.** A Vulkan surface needs *a* `wl_display`, and
the textures are the device's whatever connection a window is on, so a fresh
`Connection::connect_to_env()` would draw. But a window from a second client connection is a
window from a client that does not hold focus, and KWin's focus-stealing prevention may open it
behind the editor; fixing that means an `xdg_activation_v1` token from the editor's window,
handed across. So the thread borrows eframe's display through `Backend::from_foreign_display`
and runs an event queue of its own on it, which is libwayland's own multi-queue design.

**Rejected: an X11 path beside it.** Picture windows are Wayland only; anywhere else the mark
says so and no window opens. A second windowing backend — input, decorations, surfaces —
for a case this instrument does not have is a path nobody runs and so a path nobody fixes.

**Accepted, with the cost named: a picture window is a bare picture.** It carries no player
strip and no marks of its own, because the strip is an egui widget and drawing it there would
mean an egui context and a font atlas on the pictures thread, competing with the blit the
window exists to do. The strip stays on the picture in the node's body: a clip is scrubbed on
its node, and the window shows what that did. One priority decided it — *the picture
staying live* — and the node's own marks still say what the window is doing and still put it
away. Its two keys are read as evdev positions rather than as letters, for the same kind of
reason: an xkb keymap, and the `xkbcommon` crate with it, for one shortcut.

### On macOS and Windows, a picture window is a winit window inside eframe's own loop

**Chosen.** AppKit makes and drives windows from the main thread alone, so the Linux shape — a
thread that owns its windows — cannot exist on a Mac; nor on Windows, where Win32 hands a
window's messages to the thread that made it and winit allows that to be the event loop's
alone, so Windows takes the Mac's implementation, with winit's borderless fullscreen in place
of `set_simple_fullscreen`. `render::picture::run` builds winit's
event loop, starts eframe inside it through `eframe::create_native`, and wraps it in
`picture::Loop`, which makes each picture window as a borderless winit window on the main thread
and passes eframe every other event. Each window is drawn on a thread of its own in `Fifo`, on
the one device. It needs no `unsafe` and one declared crate, `winit`, which was already in the
binary under eframe. On Linux `run` is `eframe::run_native`, unchanged.
`proposals/macos-windows.md` is the argument.

**Rejected: AppKit directly through `objc2`.** A borderless `NSWindow` with a view subclass of
our own, a `CAMetalLayer` handed to wgpu raw, and a `CADisplayLink` per window. It gives
AppKit's own edge resize, `contentAspectRatio` and a window level above the menu bar, and it
costs `unsafe` in every Objective-C override, four declared `objc2` crates with features the app
did not compile, and a display link that needs macOS 14. The band, the lock and the pacing are
had in safe Rust without it.

**Rejected: macOS's native fullscreen, a Space of its own.** It slides in with an animation and
puts the editor a swipe away. The choice is a picture that covers its screen at once, in
place, as a projector wants: `set_simple_fullscreen`, with the menu bar and the Dock hidden
outright while supersilvia is in front.

**Rejected: the shadow.** A borderless Mac window casts one by default; a picture window casts
none, as on Linux, so the edge of the picture is the edge of the window.

### Timestamps around each Output's pass, longest over two seconds

**Chosen** for the GPU figure in the Status box: the Output's render pass carries timestamp
writes at its beginning and its end, resolved into a buffer and read once its map has landed,
never waited for, reported as the longest span in the last two seconds beside the latest. The
window and the reason for it are the editor's worst frame's. A pair of timestamps is what
wgpu offers — there is no elapsed-time query — and it needs `TIMESTAMP_QUERY` alone; a device
without it shows no line rather than a zero.

**Rejected: waiting for the draw around it.** That is the wait, and the wait is what the whole
render path is built to avoid. It would also change what it measures: a pipeline drained
every frame is not the pipeline being timed.

**Rejected: one span around the whole frame.** Cheaper by one pair and it answers the wrong
question. "The GPU is at 12 ms" is not actionable; "output3 is 9 ms of it" is, and deciding
which Output to drop in resolution is exactly what the number is for.

### An Output draws into a ring, and a viewer is shown what has finished

**Chosen.** Each Output, and the mix, draws straight into a free target of a small ring;
feedback and every other read on the synth take the latest, and viewers are shown the newest
target whose submission the completed serial has passed. A target is drawn into again only
when no held `Published` names it, by a lease each `Published` carries. See
[rendering.md](rendering.md#per-output-per-frame).

**Rejected: a temp target blitted into one published texture, with one fence after the tick
that every viewer waits on.** The blit read and wrote a whole frame per Output per tick, and
the wait put every viewer behind the heaviest Output in the tick: on the twelve-tab demo, under
glow, the editor's frame took 84–122 ms against 5 ms of its own CPU. The GPU tests hold the
ring's feedback to a temp target's copy, bit for bit.

**Rejected: freeing a replaced texture a fixed six ticks later.** The count was set by the
slowest viewer anyone had thought of, and an editor frame held longer — across a resize or a
delete — could blit a freed name. A lease is freed when the last holder lets go, however long
that is.

**Rejected: two targets.** Feedback reads the latest and the frame being drawn needs another,
and a viewer is usually shown the one before the latest, since this tick's is still on the GPU
when the synth publishes. Two would have the synth drawing under the viewer's read on every
tick the GPU is behind.

**Not taken: the viewer choosing between the newest and the one before it**, asking whether
the newest has finished itself as it paints. It would show a frame up to a tick sooner, at the
cost of every `Published` holding two targets per Output. The synth's choice at publish is one
place and one rule; this is the next step if the tick of latency is ever felt.

### An Output nobody depends on is not drawn

**Chosen.** An Output nothing depends on is a pure function of the
clock: drawn at time *t* it is the same picture whether or not it drew the ticks before, so
skipping it while nobody can see it changes nothing. Anything with memory, or visible, that
consumes its result keeps it drawing, and a feedback loop keeps its full temporal resolution.
The rule is [rendering.md](rendering.md#which-outputs-draw)'s.

**Rejected: rendering everything.** It is the simplest rule, and it put the twelve-tab demo
at 13 Hz against a 100 Hz target with every tab open — the GPU's time spent almost entirely on
Outputs nobody could see.

**Rejected: a lower rate or a smaller size for what is not seen.** It still pays for pictures
nobody sees, and for no gain: a picture that is a function of the clock is exactly as right
drawn once when it is next wanted as drawn every few ticks meanwhile. What a lower rate would
keep — a history — is what only a loop has, and a loop keeps its full rate by the rule
anyway. A reduced tier would also make an Output's picture a function of which tier it was
in, which a feedback loop's must never be.

**Rejected: finding a loop by following frames.** A loop can close through a tap's reading —
one Output's frame measured in a workspace's pass, the reading driving that Output — which a
walk over frames, or over the immediate edges the compiler descends, does not see. A loop is
any cycle of data edges, delayed ones included, and one strongly-connected-components pass
finds them all.

**Rejected, for now: a CPU consumer counting only where it integrates.** Every awake CPU node
reading a tap keeps the measurement drawing. A stateless one can still cut a deck or feed one
that does integrate, and following that is a second walk for an Output or two; the
conservative rule is one line.

### A bounded ring and two GPU ticks in flight

**Chosen.** Each Output and the mix allocate up to five targets on demand. Three cover
latest, shown and drawing; two cover pictures held across synth ticks by slower viewers.
The renderer waits for the last submission of the tick two before before submitting the
next, and the one-queue throttle keeps at most one of the synth's submissions queued behind the
one running, so a slow GPU reduces the rate of the whole graph and a shown frame stays within
two ticks of the draw — measured, within one. A target is never reused while a
viewer holds it. If no target is free, that Output skips the tick and leaves its last
finished frame on screen. This is the no-flash boundary: an occasional
skip is acceptable where the picture stays valid; a black or overwritten frame is not.
See [rendering.md](rendering.md#draws-and-skips).

**Measured on the Intel iGPU, under glow.** Thirty-second demo runs at 100 Hz found no skips with the
one-tick queue. Heavy tabs showed frames one tick old, down from about three. The light
tabs held 100 Hz; the heavy tabs ran a few percent slower. With two ticks in flight and
the same three targets, the heavy Outputs dropped unevenly, by 17 to 27 frames a
second, so the queue was one tick there. The five-target cap removes the spare-target
sweep and keeps allocation on demand.

**Measured on the Intel iGPU, under wgpu: two ticks.** With five targets and the throttle
bounding the queue by submissions, a second tick in flight dropped nothing and aged no shown
frame on any demo tab — median, p90 and maximum age 0 or 1 tick as with one — while letting a
tick's first submissions queue behind the last one's rather than after the GPU had run dry:
the heavy tabs gained 7 to 10% in ticks a second and the render engine's busy share rose from
about 92% to 99% (proposals/wgpu.md, the headless figures).
A live editor at 55 fps held published pictures across roughly two 100 Hz synth ticks;
the three-target cap exhausted its free slots and dropped about 25 frames/s per visible
Output despite GPU headroom. The isolated headless editor stand-in did not hold those
pictures, so its zero-drop result did not cover this case.

**Rejected: waiting for a viewer to let go.** A viewer can be occluded or minimized for
seconds, so one held frame must not stall the whole synth. The Output skips only while
its own targets are unavailable.

**Rejected: shedding stateless Outputs to hold a faster tick.** Visible Outputs dropped
about 45 frames a second each while loops kept drawing; the pictures stuttered and fell
out of step. A single queue wait slows the tick for the graph together.

**Rejected: submitting loops first.** A tick submits in dependency order, so a reader
can use its producer's frame from that tick.

### The synth shares the one queue, throttled

**Chosen.** wgpu gives a device one queue, with no priority on it, and the editor's paint,
every picture window's blit and the synth's tick all go into it in submission order. So the
synth submits **about one Output's pass per submission** — an Output costing more than
`SUBMISSION_MS` (2 ms) alone, a run of cheaper ones together — and before each
`render::queue::Throttle` waits until at most `QUEUED_AHEAD` — one — of its earlier submissions
is still on the GPU: an editor frame lands behind the submission running and at most one more,
rather than behind a tick.
Measured headless on the UHD 770 (`examples/queue_contention.rs`), a stand-in editor of 2.9 or
4.8 ms beside a synthetic synth at half, nine tenths and one and a half of the 10 ms interval,
three six-second runs a cell: the throttle took every overrun away at every load, the editor's
frame costing its own pass plus about one synth pass — 0.5 to 1.5 ms more than beside a synth
on a device of its own at low priority — and the synth paying for it exactly as it does
beside that device: 83 against 82 ticks a second, 50 against 50, 62 against 61, 37 against 36.
The thin margin is the heaviest cell: the 4.8 ms editor beside a saturated synth reaches
9.3 ms of its 10. Beside the demo's own heaviest tab, Games and simulations, `tick_bench`'s
stand-in editor let 333 frames of eight seconds over 10 ms where glow's low-priority context
let 81 — one Output there is a single 17 ms pass — and none beside Start here or Effect kernels
(the proposal's headless figures). The
live window's late frames are measured at step 7 of `proposals/wgpu.md`, and decide whether
Plan B is built.

**Rejected: the synth submitting a whole tick at once on the one queue.** An editor frame then
waits for the rest of the tick. At nine tenths of an interval a third of the editor's frames
overran and the window lost 41 to 61% of its frames; at saturation the editor lived at 50 Hz,
and the worst frame waited 68 ms — worse than GL's three contexts at the default priority,
because those were three kernel contexts the scheduler timesliced, which one queue is not.

**Rejected: a deeper `QUEUED_AHEAD`, or a budget of estimated GPU time queued in its place.**
Measured on the demo's heavy tabs, two or three queued bought at most 5% and a 3 ms budget
lost ticks: what idled the GPU was the tick boundary under one tick in flight and the gap
after each cheap Output's own submission, which two ticks in flight and grouping cheap Outputs
close, for 11 to 19% (proposals/wgpu.md, the synth's submissions).

**Held in reserve: the synth on a device of its own, at low queue priority** — Plan B of
`proposals/wgpu.md`, its step 8. It measured 0.5 to 1.5 ms better per editor frame headless,
and lost no frame in the window at any load, and it costs the textures shared across two devices
(export and import through the hal, most of the `unsafe` the move to wgpu otherwise deletes)
and a viewer's release waiting for its submission to finish rather than merely be made. It is
built only if the live measurement says the throttle's extra millisecond costs frames;
`render::Gpu::for_synth` is the one door it changes.

**The cost is a signal the synth may not read.** Its draws queue behind the editor's paint
and the pictures' blits, so an unfinished submission says that frame is not done and nothing
about whether the GPU is over budget — which is why nothing is [shed or
dropped](#a-bounded-ring-and-two-gpu-ticks-in-flight) on one, and the only things that wait are
the draw, for the tick two before it, and each submission, for room in the throttle.

### wgpu-core carries one patch, so a surface configures beside the synth

**Chosen.** `vendor/wgpu-core` is wgpu-core 30.0.1 as published with one match arm changed,
and `Cargo.toml`'s `[patch.crates-io]` builds against it. In 30.0.1 `Surface::configure` waits
for the device to go idle and, when the wait ends with more work queued — another thread
submitted during it — refuses the configuration with `GpuWaitTimeout`, an uncaptured error
that wgpu's default handler turns into a panic. On the one device the synth submits all the
time, so every configure races it: the editor's surface at launch and on a resize, and each
picture window's. About four launches in ten crashed on the Mac, and a harness configuring a
bare `CAMetalLayer` beside a synth submitting back to back was refused 400 times in 400. The
patch accepts that outcome as it accepts an empty queue: a surface is used by one thread only,
and nothing another thread submits references its textures, so what the wait is for has
finished. It is upstream's own fix, gfx-rs/wgpu#10296, merged and not yet released.
`tests/gpu_surface.rs` holds it on a Mac, where it fails every configure without the patch.
[vendor/README.md](../vendor/README.md) says when the copy goes: on the first wgpu-core release
that contains #10296.

**Rejected: an app-side lock**, every configure taken under the lock `Gpu::submit` holds. The
editor's surface is configured by egui_wgpu inside eframe, where the app cannot take a lock, so
it would cover the picture windows and leave the editor's launch and resize, the crash that was
seen. And each configure would hold the synth's submissions for the length of a device-idle
wait.

**Rejected: a launch-only stopgap**, the synth held back until the editor's surface is first
configured. It covers one configure and none after it: a resize of the editor, or any picture
window opening or resizing, still races the synth.

### `.ssw` and `.ssp`, supersilvia's own formats

**Chosen.** A workspace saves as `.ssw` and a project as `.ssp`, with supersilvia's own field
names, not as silvia's `.svs` under silvia's 0.8 field names.

**Rejected: reading and writing `.svs`.** It would open existing silvia patches, but it buys
that by freezing node slugs, port keys and control names against another program's — the
exact obligation already declined for shader bodies, and a heavier one, because a save format is
forever in a way a shader body is not. supersilvia's nodes are allowed to split, merge and
rename their ports; a shared file format takes that away.

An importer is not ruled out, and would be the right shape if it is ever wanted: one-way,
best-effort, reporting what it could not map. It carries no ongoing obligation.

**A file's slugs and keys are read as this build's own, and nothing else.** No table
redirects a renamed slug, input, output or option ([no backwards
compatibility](#no-backwards-compatibility-and-no-migration)). A table that redirected silvia's
output names (`mandelbrot.mask` to `smooth`, a noise's `mask` to `value`) fired on every load,
and since Mandelbrot and Julia have a `mask` of their own it moved this build's own saved
cables to another port on reopen. silvia's patches are not imported.

### Undo by snapshot, not by inverse commands

**Chosen.** `App` keeps a bounded ring of `Arc<Graph>` snapshots and undo restores one.

**Rejected: `Vec<Command>` with inverses**, which is what the docs promised for a while. It
is O(1) memory per step, but it makes undo a per-command obligation: every future command has
to answer "what undoes me", some answers are genuinely hard (a removal has to restore the
node, its controls, its options and every edge that touched it), and a wrong answer corrupts
a graph quietly. Snapshots make the question disappear.

It also wanted a removed node to return under the same `NodeId`, which slotmap keys cannot
do — the tombstone-in-the-slotmap workaround for that would have added an invariant every
iteration, length and traversal has to remember forever.

**Memory is the trade and it was measured, not assumed:** a whole 200-node graph is roughly
150 KB. Nodes are shared between snapshots, so a step holds what its edit wrote and a map of
pointers — a knob turn is one node — and only an edit that writes every node costs a whole
graph.

### One node identity, not two

**Chosen.** `NodeId(u32)` from a counter that never rewinds, over a `BTreeMap`.

Previously a node had a slotmap key *and* a `display_id`, both never reused, one internal and
one appearing in shader names and destined for save files. Collapsing them removed a concept, a
field, a dependency, and the paragraph in three documents explaining the difference. Iteration
order became id order rather than slotmap's free-list order, so determinism stopped resting
on an allocator detail.

### The file dialog runs on a thread

**Chosen.** `rfd` with the **xdg-portal** backend, not gtk3: on KDE/Wayland the portal uses
the desktop's own dialog, matches the user's theme, and keeps GTK out of the process.

The portal is a DBus round trip, so the dialog runs on its own thread and answers through a
channel that the frame loop polls. Blocking the frame on it would freeze every Output for as
long as the dialog is up — a picture window holding a still frame while someone picks a file is
exactly what [the UI-defers-to-the-render rule](#the-ui-defers-to-the-render-not-the-other-way-around)
exists to prevent.

**Rejected: rfd's blocking API.** One line shorter, and it stalls the show.

**On macOS, `NSOpenPanel` through rfd's native backend**, from the same thread, with the same
blocking call. AppKit puts a panel up from the main thread alone; rfd 0.15 hands it there with
GCD's `dispatch_sync` onto the main queue, which the main thread drains because it is in
`NSApp`'s `run` under the winit loop eframe runs inside (`render::picture::run`). The panel is
application-modal and floats above every window, fullscreen picture windows included, and the
picture windows go on drawing on their own threads.

**Rejected on macOS: `rfd::AsyncFileDialog`.** Its future asks `NSApplication` for the main
window and the window list from the calling thread, which AppKit does not promise to answer off
the main thread, and hangs the panel as a sheet on that window — which can be a borderless
picture window on the projector. It would also want an executor to block on, which rfd brings
only with the portal backend.

### A fourth port type: uniform numbers

**Chosen.** The model is in [architecture.md](architecture.md#four-kinds-of-value-and-one-asymmetry);
this records why it is a type and what lost.

A graph carries four kinds of value, not three:

| | lives | shape |
| --- | --- | --- |
| `Action` | CPU | an event, many-to-many, never compiled |
| `VaryingNumber` | GPU | `fn f(uv: vec2f) -> f32` — a field, one value per pixel |
| `VaryingColor` | GPU | `fn f(uv: vec2f) -> vec4f` — a field |
| **`UniformNumber`** | **CPU** | **one `f32` per frame** |

**A uniform number is not a new concept — `Control::Number` is already one.** A number
control is a single `f32`, constant across the frame, uploaded as a uniform. The only
thing it cannot do today is be *connected to*. `UniformNumber` is that value promoted to
first class, which is why a meter, a MIDI knob, an audio envelope and an LFO all end up being
the same kind of thing.

**Why it earns a type rather than being a convention.** `VaryingNumber` means a *field*, and
the CPU cannot hold a field: there is no `uv` outside a shader. Without a second kind nothing
could be evaluated outside a shader and the tick would stay empty forever. `UniformNumber`
is exactly "the kind of value that fits in an `f32` on the CPU", so the type system carries
the control-rate/signal-rate boundary instead of a person carrying it. The corollary is
enforced by a registry test: every input of a CPU node is a CPU type — `UniformNumber` or
`Action`, never `VaryingNumber` or `VaryingColor` — because `tick` cannot sample a field.

**Drawn as a diamond**, in the number hue exactly. Shape carries the rate the way an
action port's square does, so the difference survives a grayscale theme.

#### The asymmetry is physics, not policy

- **`UniformNumber` → a `VaryingNumber` input is free**, and needs no conversion node. An
  unconnected number control already compiles to a bare uniform, and a connected
  uniform number produces the same shape by a different branch: `CompileContext::input` sees
  the source port is a `UniformNumber` and returns `published_uniform`'s `u_float_…` before it
  ever reaches the control lookup that yields `u_control_…`. Either way it never reaches
  `emit()`, so it emits no function at all.
- **`VaryingNumber` → `UniformNumber` costs a render pass, a readback and one frame.** You
  can broadcast a number across every pixel for nothing; you cannot collapse a million pixels
  to a number without asking the GPU and waiting.

That one-way relationship is not an implicit conversion in the sense the type rules forbid —
`VaryingNumber` to `VaryingColor` would be a *reinterpretation*, while `UniformNumber` to
`VaryingNumber` is the same number, and every control already does it.

#### Going down is a side effect, not a render target

**Chosen.** `tap` and `sample` reduce inside a fragment shader evaluating their input's
expression, through atomics into a storage buffer the renderer reads back a frame later. The
measurement is **inside a pass**, its workspace's, which also draws that workspace's
thumbnails — no target of its own, no second context — and it is **not at the caller's
coordinate**: `fs_main` calls it at canonical points, which is
[the entry below](#a-tap-measures-its-input-over-the-unit-square), in the pass
[a later entry](#a-tap-is-measured-in-its-workspaces-pass) chose. See
[architecture.md](architecture.md#going-down-side-effects-in-the-expression).

silvia had the idea as its `debug` node: sample a float at a point inside the shader and
print it into the picture with a bitmap font, a pass-through whose only output was pixels.
This is that tap with the number reaching the CPU as a port, so it can be used and not only
read.

**Rejected: an analysis node as its own render target.** An Output nobody looks at, rendering
the expression into a 32x18 target and reading that back. It re-evaluates the expression in a
second context — its own resolution, its own frame — and it would need no atomics, which was
its only advantage. The objection that a canonical domain "is not the feedback loop on
screen" was the other half of it, and it is moot: `feedback` is retired, so nothing in the
graph has a picture that depends on the Output it is drawn in. What survives of the objection
is the aliasing, which the grid's jitter answers.

**Rejected: mip-chain means on Output textures.** Exact and cheap for the mean of a whole
frame, but it only measures a finished Output, needs a compile-time "is this a texture"
check to be safe, and grew uniform number outputs on the Output node. Three concepts to save
a readback that was never expensive.

**Rejected: analysis on the Output node itself.** The tap at the end of a chain is that,
with nothing special-cased.

**Cost accepted: atomics in the fragment stage, and integer fixed point.** The tap buffer is
`array<atomic<u32>>`, which WGSL gives a fragment shader on every wgpu backend and macOS's
OpenGL 4.1 does not — one of the reasons [the renderer is wgpu](#wgpu-not-glow). Atomic adds
are integer, so sums are fixed point — 1/65536 a sample,
biased by 32768 so an unsigned word carries sign, over a measured range of -32768 to 32767.
Each sum is 64 bits across two words: `atomicAdd` returns the word it replaced, so a wrap is
visible and pays for one more atomic only when it happens, which buys exactness at every
frame size instead of buying headroom by throwing fractional bits away. Extremes are ordered
by a monotonic map of the float's bits — invert a negative, set a non-negative's sign bit —
which orders the whole line rather than only the non-negative half.

**Rejected: weighting the centroid by magnitude.** A signed field's "where is it" is where it
is positive; weighting by `abs` would put the centroid in a strongly negative region and call
that where the quantity is.

**The conversion menu is a curated table, not a registry query.** It was built as a query
first — a node with an input the source can feed and an output of the target's own type *is* a
conversion, so `rgba`, the `Convert` family and the measurements qualify without declaring
anything, and a node added tomorrow is a row the day it lands. What that argument gets wrong is
what the gesture is for. A dropped cable asks *what should this become?*, and the query answers
*what could be put here*: fifteen rows for a color onto a circle, twelve for a color onto a
diamond, half of them nodes with a point or a response time to set that nobody would pick
blind. silvia's table is twelve rows across the two pairs it has — four readings of a color,
its four channels, and four ways of painting a number — each named for the casting rather than
the node, and that curation is the feature rather than the maintenance burden the query was
avoiding. A node outside the table is not lost: it is added from the Nodes menu and wired by
hand, which is what a node with something to set wants anyway. The table also does what the
query could not express at all — `Bridge::inputs` is a list, so *Grayscale* fans one number
into an `rgba`'s red, green and blue, which is silvia's `float-to-color` closure. `nodes::bridge`
is the table and `docs/ui.md` is the menu; a test resolves every row against the registry,
since a hand-written table can name a port that is not there and a query never could.

**A convertible port is a third state, not a second kind of legal.** Two states lost to
silvia's three. The argument for two was that both work, so both are offered — the existing
rule *an illegal connection is unofferable, not rejected* applied to a wider notion of legal.
What it misses is the price: at the **diamond** boundary a conversion costs a readback and a
frame, which is more than a luminosity node, and a cable that quietly inserts a frame of
latency is not the same offer as one that fits. So a port the cable can take is drawn as it
always is, one a node could carry it to wears a dotted ring, and one that is neither is
dimmed — which is silvia's `.port.convertible` and its menu on release, and it also means a
release on a convertible port has somewhere to go instead of dropping the cable. `Action` is
exempt from all of it, having nothing to convert to or from.

#### The other direction, and it is built

A CPU node that publishes a *texture* is the same idea upward, and `OutputKind::Texture` is
the slot for it. `camera` and `video` both ship on it: `tick` publishes an `Arc<Frame>`, the
renderer uploads it into a source texture, and a shader samples it as an ordinary
`texture_2d<f32>`. See [media.md](media.md) and [rendering.md](rendering.md#source-textures).

The consequence falls out of the kind: a `Texture` output is **delayed**, so a CPU node's
picture is read the way an Output's `frame` is — a consumer samples what was last published
rather than descending into it, and a loop through a camera is legal. A cellular automaton
whose birth chance is the brightness of a video is the same shape and needs nothing new.

**Not every CPU number is a `UniformNumber`.** A value that a node reads once a frame — a
birth chance, a gain — is a uniform number input. A value whose change would reallocate the
node's world — the size of an automaton's grid — stays an option, because an option is a
structural edit and a uniform number is a per-frame one, and nothing should reallocate at
frame rate because a cable wiggled.

**Rejected: a control that rebuilds the world when it changes.** silvia refills the cellular
automaton's grid on every `input` event of its Init Threshold s-number, which is fine for a
hand on a knob. Here that number is a `UniformNumber` input, so it may be a cable — and a
cable that refilled the grid whenever its value moved would refill it at frame rate. The
threshold is read where a fill is actually asked for: the `randomize` action, and the
reallocation a grid-size change forces. What the knob decides is the *next* fill.

### A button that writes its node's settings

**Chosen: a row of buttons on the body whose press is one command; a roll of uniforms stays
the tick's.** silvia's `lyapunov` has Random Seq, and its `slimemold` a `Randomize` action and
nine preset buttons; all eleven write the node's own settings. Random Seq writes `sequence`,
a `Code` option, and a preset writes three knobs and the `Runtime` Mode — and a tick can
write a node's number controls (`TickContext::write_control`) but not its options, since an
option is structural and the compiler is the editor's. So the machinery is on the editor's
side: `Region::Buttons`, a row a node declares with what each button writes, whose press is
one `Command::SetSettings` — options and controls of one node, checked together, one undo
step, saved, rebuilding exactly when a `Code` option is among them. See
[ui.md](ui.md#a-nodes-own-buttons).

- **Rejected: `TickContext::write_option`.** A door from the tick to options would let a cable
  fire Random Seq as silvia's action input does, and every firing would be a shader rebuild —
  what may not be a pipable port under the parity review's decision. It would also split a
  preset into four writes the editor had to stitch back into one step.
- **Rejected: a batch command of commands.** `SetSettings` is the one shape the two users
  need, one node's options and numbers, and a general batch would have to answer for what
  every command inside it does to the gesture and the plan.
- **`Randomize` stays silvia's action input.** It writes only the three sensing knobs, which
  are uniforms, so it is `write_control` from the tick, and a sequencer may
  fire it; a patch saved with a cable into it keeps the cable.
- **A preset's nudge is a pulse, not an edit.** silvia's `_applyPreset` ends in
  `_nudgeSimulation`, which is the running world's, so the button holds a key the tick reads
  as a press — `Buttons::pulse`, `slimemold::NUDGE` — for one frame, and it never enters the
  history.

**Chosen: a CPU simulation publishes its state, not its picture.** A node whose picture is
computed on the CPU declares two outputs: a `Texture` carrying the state its tick simulated —
`cellularautomata`'s `cells` — and a `Glsl` picture that samples that texture through
`ctx.texture_uniform(node, state)` and mixes the node's color inputs into it. Coloring on
the CPU instead would mean a color input that no cable could drive and a whole frame
re-uploaded to change a hue. It also means those state ports are named for the state and not
out of the picture vocabulary, which
`a_node_names_its_outputs_from_the_vocabulary` has a third list for.

**Rejected: a `CpuNode` trait for definitions.** Definitions stay data. The trait covers
per-instance *state* — a stream, a pipeline — which has a lifetime a `const` cannot.

**Chosen: `autogain` and `autoexposure` as nodes.** Both compose from what exists — a tap,
a divide, a slew, a multiply — and both are nodes anyway, because a performer wants exposure
in one drop rather than four nodes and three cables, and because the composed form measures
after the multiply unless wired carefully. Same argument as `oscillator` beside `sine`.

**Chosen: exposure is open loop on the input.** `gain = target / measured` has no dynamics
to tune and cannot oscillate; the slew is the only time constant. Closed loop on the output
was rejected for that reason. Inside a feedback loop the input is last frame's output, so
the loop is held anyway.

**Chosen: a node may read its own published uniform number.** `ctx.own_uniform` registers the
same `NodeUniform` provider a connected uniform number gets. A node whose WGSL reads what its
`tick` computed is a new shape, and the honest one: the alternative was a hidden uniform
that no port exposed, and then the gain could not be seen or patched.

**Rejected: `OutputKind::CpuFloat`.** It spelled a CPU float as an ordinary GPU output whose
generator returns a uniform read — so it compiled to `meter4_luma(uv)`, a function that
ignores its only argument, where a control in the same position compiles to the bare
`u_control_…`. It is a worse spelling of something the compiler already did, and it left the
value un-typed on the CPU side, which was the whole point. The variant is gone, and so are
`CompileContext::float_uniform` and `UniformProvider::NodeFloat`, which were its compiler
half.

What replaced them is not nothing: a per-node float uniform still exists, as
`CompileContext::published_uniform` and `UniformProvider::NodeUniform`, and there is an
`OutputKind::Uniform` whose generator is `no_wgsl` and is never called. The distinction that
survives the rename is the real one. `CpuFloat` was a GPU output kind with a generator that
had to return *something*, so the value arrived as a function call; `UniformNumber` is a port
type the compiler resolves to a uniform before a generator could run, so no function is
ever emitted and the value is typed as a CPU value on both sides.

### Varying and uniform, on two axes

**Chosen.** A data port is named for what it carries and how often it varies: a **varying
number** or a **varying color** is one value per pixel, computed inside the shader; a
**uniform number** is one value per frame, held on the CPU and uploaded as a uniform.
The words are the GPU's own — `varying` and `uniform` are the two qualifiers older GLSL uses
for exactly this distinction — and "uniform across every pixel" is a sentence a person with
no shader background reads correctly the first time. `PortType` has the two axes as `Kind` and
`Rate`, and everything that tells the rates apart — the diamond, the dual
logic in `graph/`, the readout on the row — reads the rate rather than a variant.

**What lost.** `Float`, `Color` and `Scalar`. `Float` and `Color` said what the shader function
returned and nothing about where the value lived, so the one type that did live somewhere
else, `Scalar`, had to be explained against them every time: a scalar was "a number the CPU
holds", which is a definition and not a name. "Number" alone was worse, because it was used
for both rates. `fragment` and `field` both lost the rate to `varying`: a fragment is a
pixel-candidate, which says nothing about why the two rates are opposed, and neither word
reads as *different in different places* to somebody who has never written a shader. "Field"
stays as the concept word for a per-pixel function, since a varying number and a varying
color are both one, and "number" stays for the kind, which is what the theme's hue anchor and
the number control are named for.

**The fourth cell, now built.** `PortType::UniformColor` — one `vec4` per frame, a live
swatch on its row, the same free promotion into a varying color input — closes the
two-by-two, and silvia had it first: its compiler broadcast a CPU color through
`colorUniformUpdate` beside the float path this editor made a type of. The `color` node is
the whole of the shape — a swatch its tick publishes — and every varying color input fed by
one draws the color arriving at it, the way a number knob fed by a uniform number draws the
live number. It reaches the shader as `u_color_{slug}{id}_{key}`, silvia's own name, declared
`vec4`. `sample` publishing five uniform numbers and no color at all is what it cost not to
have this.

**A reading publishes a color *and* the numbers in it.** `sample` publishes a `color` beside
`r`, `g`, `b`, `a` and `luma` rather than in place of them, and the channels stayed
once they were both on the node: a hue is not a number and a channel is, so the two are
answers to different questions and neither stands in for the other. `hue`, `saturation` and
`lightness` joined them for a reason of their own — the `Convert` nodes that compute those
are shaders, so the only existing route from a sampled pixel to a uniform hue ran a field
through a `tap`'s sixteen thousand grid points to collapse it back to one number. The CPU is
already holding that pixel. `tap` is the same shape for the same
reason — its mean color is the mean of the *channels*, where its `mean` is the mean of the
picked quantity. The cost is rows, which is why the two nodes label their pass-through row
*Pass-Through* rather than *Output*: with a color published under it, "Output" stopped saying
which of the two a cable should come from.

**The shape alone carries the rate.** A uniform port is its kind's color exactly, a diamond
where the fragment is a circle. A lighter shade for the uniform was tried and dropped: two
shades of one hue read as two kinds, and the kind is the one thing the two rates share.

### A number stays varying

**Chosen.** An input on a GPU node is a `VaryingNumber` unless the node itself has a reason
for it not to be, and "it says how many" is not one. A kaleidoscope whose segment count
follows a noise, a polygon whose sides follow a gradient, a posterize whose levels follow a
picture are pictures a person makes on purpose, and a count that varies per pixel is as much
the node's effect as a strength that does. The one reason to make a GPU node's input a
`UniformNumber` is that the node could then publish a uniform of its own from it, and even
then the picture per-pixel variation would have made is weighed against it. A CPU node's
inputs are uniforms by construction, since a tick has no `uv`, and a Time is one, since it
says which moment the node is at; nothing else is.

**What lost.** A rule that a count is a uniform number: iterations, octaves, seeds, rows
and columns as diamonds, on the argument that a per-pixel loop bound runs at the worst case
and a per-pixel seed is noise of noise. Driven by a noise and by a pattern, the pictures were
cooler than the live readout was useful, and the WGSL is the same either way, so the rule
bought nothing but a diamond.

**The exception: `lyapunov` has no count at all.** It runs ten steps of its map, a constant
in `fractals.rs`, and has no Iterations control, port or value. Ten are
enough for the picture, never eighty. A count that is a field is a loop no compiler can bound
ahead of time, and this one ran three times per pixel with a body of two sines and a log, so
at the demo's eighty it was a 17 ms pass the editor waited behind. What lost is the one thing
the rule above protects, a count driven per pixel, which on this node bought a longer pass for
detail ten steps already give. See [loop-bounding.md](loop-bounding.md#lyapunov).

### GStreamer for video, and RGBA through an appsink first

**Chosen.** Cameras and, later, files and hardware decode come through GStreamer: a pipeline
string ending in an `appsink`, frames published through a triple buffer. `v4l2src` for a
camera, `videotestsrc` for the tests, the VA-API decoders from the same plugin set for files.

**Rejected: `ffmpeg-next`**, the kickoff's plan. It would have meant writing the capture,
decode, color conversion and threading by hand, and the VA-API surface import in `unsafe`
inside `render/`. GStreamer has all of it, on its own threads, and the device probing besides.
The cost is a system dependency — declared in `distrobox.ini`, checked by `doctor.sh` — and a
runtime that reports errors on a bus this app polls once a tick rather than through a GLib
main loop.

**Chosen: DMA-BUF import for decoded frames.** `vapostproc` converts to RGBA on the video
engine and exports a descriptor; the renderer imports it on Vulkan, through wgpu-hal. See
[media.md](media.md#frames-without-a-copy). The sink changed and the node did not, as
predicted. `render/` still sees no GStreamer type: the keep-alive behind a descriptor is an
opaque `Arc`.

**Rejected: `gstreamer-gl`.** Its frames are textures in a GL context of GStreamer's, and the
renderer is wgpu on Vulkan, which cannot sample a GL texture without an interop layer.
Importing the descriptor ourselves is one module, `render::dmabuf`.

**Rejected: importing NV12 planes.** Per-plane import puts a YUV matrix into every video
node's shader and doubles the samplers. Letting the video engine convert keeps the node's WGSL
and the texture model exactly as they are, and is free.

**Chosen: a camera's bytes in the camera's own layout.** A webcam's YUY2 or NV12, or the
planes an MJPEG decoder writes, are held mapped and uploaded as they lie, and the renderer
converts YUV to RGB with one draw into the source's RGBA texture. See
[rendering.md](rendering.md#source-textures).

**Rejected: `videoconvert` to RGBA in the pipeline.** One CPU thread converting every frame,
then a copy of it into a `Vec`: at 4K and 60 frames that is about 2 GB/s before the GPU sees
anything, and the GPU does the same arithmetic for nothing.

**Rejected: YUV planes sampled by the node's own shader.** The same reasons importing NV12
planes lost: a YUV matrix in every consumer's shader and a sampler per plane. The conversion
pass costs one draw per new frame and leaves the texture model as it is.

**Rejected: a triple buffer between a camera's sink and `tick`.** Its spare slots hold stale
frames, and a stale frame is a buffer the source cannot reuse; a screen cast with three
buffers deadlocks. A one-frame slot whose lock `tick` only tries holds just the newest. See
[media.md](media.md#cameras).

**Rejected: raising `vajpegdec`'s rank.** It is the whole process's decodebin that would pick
it, posters of arbitrary JPEGs included, and the video engine refuses some (progressive,
CMYK). The camera's own decodebin prefers it through `autoplug-sort` instead.

**Chosen: `auto` probes.** The device monitor answers only where udev is reachable, which in
a container it is not, and `/dev/video0` is an output-only loopback on any desktop running
OBS. So `auto` asks `v4l2src` to reach `Ready` on each node in turn and takes the first that
does.

**Chosen: one pipeline per camera, joined by every later reader.** A Camera node and the Main
Input on one camera both show it, at the size it was opened at; a reader asking for another
size is refused by name. See [media.md](media.md#cameras).

**Rejected: a pipeline per reader.** V4L2 refuses the second as busy, and on a Mac the second
`AVCaptureSession` sets the device's format under the first, whose frames then no longer fit
its caps. **Rejected: refusing a second reader outright**, as V4L2 does: the Main Input
and a Camera node on the one built-in camera is the ordinary case on a laptop. **Rejected:
reopening at the newest reader's size**, which would change the size under a reader that asked
for the old one.

### Video files are transcoded on import into all-intra H.264

**Chosen.** A delivery file has a keyframe every few seconds and every other frame is a
diff, so showing frame *n* means decoding from the last keyframe forward — which is why
scrubbing, reverse and speed changes fall apart in a browser. On first use a file is
re-encoded with a group-of-pictures size of one by the hardware encoder (`vah264enc`, CQP, on
Linux; `vtenc_h264_hw` at constant quality on a Mac), capped at 720p or 1080p by the node's
`size` option, into the project's own `cache/<fingerprint>.mp4`. From then on any frame costs the same to reach in any order, and
reverse is asking for *n − 1*.

The cache lives in the project's own folder — see [assets and the
cache](#assets-and-the-cache-live-inside-the-project).

The fingerprint is FNV-1a over the source, its size and the settings — stable across
toolchains, unlike `DefaultHasher`, so a Rust update does not re-encode every clip. A source
**inside the project** is keyed by its path relative to the root, which is what lets the
folder move with its entries; one **outside** is keyed by its absolute path and its
modification time, because nothing here owns it and a replaced file has to get a new entry.
Two nodes importing the same file share one in-flight job; the original is never touched.
Encoding writes to a `.part` beside the destination and renames at the end, so a half-written
file is never mistaken for a clip.

**Rejected: a reverse-encoded copy.** It exists only to work around inter-frame
dependencies, and all-intra has none.

**Rejected: MJPEG, ProRes, DNxHR.** All intra-only too, but none has hardware decode here;
H.264 does in both directions, and HEVC is the same pipeline if size ever bites.

**Rejected: a decoded ring in VRAM.** At 8 MB per 1080p frame a five-second loop is 1.2 GB.
A pipeline seek is a few milliseconds and a frame's decode, which at 60 Hz is enough for
reverse and scrubbing; the ring is an option for short loops if a real graph hits the limit.

**Chosen: the codec is probed, not fixed.** The cache does not care what it holds, only
that this machine encodes and decodes it in hardware. `Codec::probe` takes the first
installed encoder–decoder–parser triple from the machine's list: on Linux H.264, HEVC and
AV1 over VA-API, then the same three over NVENC; on a Mac H.264, then HEVC, over VideoToolbox.
An Intel part without H.264 encode lands on HEVC; an NVIDIA card lands on its own plugin. The codec's name is part of the cache fingerprint, so two machines that
chose differently keep separate entries in one project folder and each re-encodes only what
it cannot decode. Every available codec is round-tripped
by a test.

**Rejected: software encode as a fallback.** `x264enc` is not in the box and the point of
the cache is that the hardware does the work. A machine with no hardware pair gets a status
line saying so; it is not a rig this is for.

### On a Mac, the cache is written by VideoToolbox under Linux's names

**Chosen.** `platform::macos::video::CODECS` is two rows, `h264` over `vtenc_h264_hw` and
`h265` over `vtenc_h265_hw`, both decoded by `vtdec_hw`, with H.264 first. The names are the
ones Linux's VA-API rows carry, and a cache entry's name carries its codec, so a project folder
last used on Linux plays its clips on the Mac at once and the reverse: the folder holds one copy
of each clip, which is what [keeping the cache in the
project](#assets-and-the-cache-live-inside-the-project) is for. It holds because both machines
write standard streams — the Mac's are Main profile, and an all-intra High-profile file of
Linux's kind decodes on `vtdec_hw` bit for bit and seeks to every frame. The two encoders differ
at the same quality, so a copy made on either machine looks slightly different; nothing reads
that difference. Every frame is an IDR through `max-keyframe-interval=1`, and `quality=0.7`,
with the bitrate left automatic, is constant quality at about x264's QP 26, since VideoToolbox
has no constant-QP mode. `proposals/macos-media.md` has the measurements.

**Rejected: names of the Mac's own (`vt-h264`, `vt-h265`).** Each machine would prepare every
clip once more, and a synced folder would carry and sync two copies. It would have let the Mac
prefer H.265, a third smaller at the same quality and faster to encode.

**Rejected: H.265 first on the Mac.** Under shared names it would make the Mac choose a codec
Linux does not, and the entries would stop being shared.

**Rejected: AV1.** No Apple chip encodes it.

### On a Mac, a decoded frame is sampled in its own `IOSurface`

**Chosen.** `vtdec_hw`'s plain output is its own `CVPixelBuffer` on a two-plane `IOSurface`,
reached through applemedia's `GstCoreVideoMeta`, whose struct is mirrored and checked by the
meta's name, its registered size and the pointer's CoreFoundation type. The renderer makes a
Metal texture of each plane with `newTextureWithDescriptor:iosurface:plane:` and wraps it with
`texture_from_raw` and `create_texture_from_hal`; the frame carries the same memory mapped as
bytes, which an import that fails uploads. See [media.md](media.md#frames-without-a-copy) and
[rendering.md](rendering.md#dma-buf-import).

**Rejected: `CVMetalTextureCache`.** It hands back a `CVMetalTexture` that must outlive every
draw of it and a cache that must be flushed: a second keep-alive beside the return rule the
frame already obeys, for a texture the surface makes directly.

**Rejected: decoding with VideoToolbox directly** (`objc2-video-toolbox`), with GStreamer only
demuxing. All-intra frames decode independently, so it is feasible, and it would need no
mirrored struct; but it is another binding, another session to manage and more `unsafe`, to
avoid one struct whose change would send frames as bytes.

### On a Mac, a saved Camera node names the camera itself

**Chosen.** A Camera node on a Mac saves AVFoundation's unique ID in `device`, a 36-character
UUID for the built-in camera, and its menu shows each camera by name. Linux saves the V4L2
path, `/dev/video0`, as it did. `auto`, the default most nodes hold, means the first camera
on both machines. A project that used the built-in camera reopens on the built-in camera
after a USB camera is plugged in, and after a reboot: a rig that changes cameras when a cable
goes in is the failure a performer notices on stage. A camera the machine does not have shows
black with a status line naming it, like a missing clip; choosing from the menu fixes it, and
the next save writes this machine's camera. `avfvideosrc` opens by index alone, so the ID is
found again by listing and matching (`platform::macos::video`). Whether a USB camera keeps its
ID on another port is unverified.

The menu reads the list the Main Input's listing made (`video::camera_menu`), since the menu is
drawn every frame and a listing starts a device monitor. *Look for devices again* lists again.

**Rejected: the position**, the index `avfvideosrc` takes. It reopens on whatever is in that
place now. It would let the Mac save Linux's words, `/dev/video0` meaning the first camera, so a
file moves between machines unchanged; but it is unstable, and the numbers do not match across
the two anyway, since a Linux webcam often takes `/dev/video0` and `/dev/video1`.

### On a Mac, audio inputs are listed by Core Audio's properties

**Chosen.** `platform::macos::audio::sources` reads `kAudioHardwarePropertyDevices` and each
device's input streams, name and UID through `objc2-core-audio`, which opens nothing, and a
named input is `osxaudiosrc unique-id=<UID>`. The UID persists across boots.

**Rejected: GStreamer's device monitor over `Audio/Source`**, which is what Linux asks. On a Mac
its `osxaudio` provider binds a HAL AudioUnit to each microphone to probe its formats, which
opens it, and from a session that could not show the permission prompt the probe never
returned. The listing runs in the synth's tick.

**Rejected: `osxaudiosrc device=<AudioDeviceID>`.** The ID can change between boots.

### On a Mac, the loopback is a Core Audio process tap, read by an IO proc

**Chosen.** *System audio* on a Mac is a global stereo tap over every process, supersilvia
included as a PulseAudio monitor includes it on Linux, in a private aggregate device whose IO
proc mixes to mono and feeds the analyzer's closure directly (`platform::macos::audio::Tap`,
[media.md](media.md#the-loopback)). It needs only the audio-capture permission.

**Rejected: ScreenCaptureKit's system audio.** It needs a stream over a display, so the
Screen Recording permission or the picker, for a feature that has nothing to do with the
screen.

**Rejected: the tap wrapped in an `appsrc`**, so `platform::audio::element` could name it. It
adds a pipeline, a caps negotiation and a resampler between an IO proc and a closure that
already takes blocks.

**Rejected: leaving supersilvia's own process out of the tap**, which is one line. The Mac would
then hear less than Linux does.

**Rejected: the default output as the aggregate device's main sub-device**, which published
tap examples use for a clock. The headers ask for none, the sub-device's own inputs would sit
beside the tap's — an audio interface's microphones — and the device would go when that output
does. **Rejected too: `tapautostart`**, under which `AudioDeviceStart` waits for the first
tapped sound, on the synth's thread.

### The oldest macOS supersilvia opens on is 14.2

**Chosen.** `LSMinimumSystemVersion` is 14.2 in `packaging/macos/Info.plist`, and the build's
deployment target is the same (`MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml`). 14.2 is the
first with process taps, and every Apple Silicon Mac runs 14. Finder refuses to open it on an
older system, with its own message. At an older target `AudioHardwareCreateProcessTap` links as
a lazy bind that is not weak, so the app would open and die at the first loopback.

**Rejected: opening on older Macs** without the loopback before 14.2 and without screen capture
before 14.0. It means checking the version before every such call.

### The `.app` carries GStreamer's official release, and GStreamer finds itself in it

**Chosen.** `packaging/macos/build-app.sh` unpacks the official macOS `.pkg`s into a cache
without installing them, builds the binary against them, and carries the plugins for the
elements the app makes, the libraries they link and the plugin scanner in
`Contents/Frameworks/GStreamer/{lib,libexec}`. The release names every library `@rpath/<file>`
and GStreamer locates its plugins and scanner from wherever `libgstreamer` was loaded, so
nothing is rewritten and start-up sets nothing ([packaging/macos/README.md](../packaging/macos/README.md)).

**Rejected: copying Homebrew's GStreamer and relinking it** with `install_name_tool`. Its
libraries name each other by absolute `/opt/homebrew` paths, every one would need rewriting,
and the result would follow whatever `brew upgrade` last installed. The release has every
element the Mac uses at the same version.

**Rejected: setting `GST_PLUGIN_SYSTEM_PATH` and `GST_PLUGIN_SCANNER` at start-up.** It needs
`std::env::set_var`, which is `unsafe`, for what the release's relocation already does.

**Rejected: embedding the whole framework**, 680 MB of both architectures and every plugin.

### cpal for the microphone, analyzed in the callback

**Chosen.** The device's own callback mixes to mono, runs a 1024-point Hann FFT with a DC
blocker in front of it, and publishes RMS, peak and three bands through a triple buffer. The
callback allocates nothing after start-up. Nothing here smooths but the per-bin EMA the FFT
needs to stop a band flickering at the block rate.

**Rejected: analyzing on the frame thread.** It would tie the analysis to the frame rate
rather than the device's block rate, and put the FFT in the budget the render needs.

**Rejected: a smoothing constant in the analyzer.** That is what silvia's `AnalyzerNode` had,
and it is why a transient arrived two frames late. Smoothing belongs to whoever wants it —
which is `slew`, a node, downstream. See [band shaping](#band-shaping-belongs-to-the-graph).

### One clock, and it does not lie about elapsed time

**Chosen.** `Clock::elapsed()` is true monotonic time, and so is the interval between two
ticks: nothing on the clock is clamped. The one clamp is a stateful node's step,
`transport::MAX_DT`, applied where that step is taken — see
[One transport](#one-transport-over-the-one-clock-and-gears-hold-the-only-time).

It used to accumulate the *clamped* `dt`, so a 200 ms stall advanced it by 100 ms. The reason
given was that feeding a true value to a phase accumulator makes everything jump — which is
right, but a phase accumulator integrates **`dt`**, and `elapsed` is what everything *samples*:
`u_time`, and through it every generator's `time` input and `defaultUvMap`. The clamp was
applied to the one quantity that wanted the truth, to protect callers of a quantity nobody was
reading.

**Rejected: a second, unclamped clock for transport.** It was the first answer to the drift and
it is the wrong one — `clock.rs` and `architecture.md` both say *"Nothing else keeps a timer"*,
and silvia's 45 independent `requestAnimationFrame` loops are what this rewrite exists to fix.
Answering a timing problem with a second timer is the shape of the original disease. Musical
time is a **coordinate system** over the one clock, like a clip's local time or a scrub.

When audio runs, the authority for that clock becomes the device's sample counter — the only
one that does not drift against what you can hear. Who holds the clock changes; there is still
one.

**What visibly changed:** after a stall, `u_time` advances by the whole gap instead of half of
it, so a generator reading it jumps further. That is what happened, and it is the price of
everything agreeing about what time it is. Hiding it was making stalls invisible, which is the opposite
of useful — a stall is a bug to fix.

A leftover fell out: `last_raw` was set to the same value as `last` on every tick and existed
only because the clamp had confused the two. It is gone.

**Rejected: clamping `dt` at the clock**, which is where the clamp lived until the transport.
A clamped interval is what every speed then integrated, so after a 300 ms stall every
generator sat 0.2 s behind `u_time` for good, and a minimized second came back as 100 ms. A
gear and an ambient reading take the whole of a stall, and only a stateful node — a
simulation, a slew, an envelope, a pad — is bounded, because only a stateful node is harmed by a long step.

### One transport over the one clock, and gears hold the only time

**Chosen.** `transport.rs` is a playhead in seconds over the one clock, and it plays, pauses
and seeks — nothing else. It reads `T = T_anchor + playing × (elapsed − elapsed_anchor)`,
re-anchored on every play, pause and seek, the way Link, Tidal and SuperCollider keep a
position. That playhead is **ambient time**, the one clock of the show: every moving node
reads it at its own rate unless a gear is cabled into its Time, and every gear integrates how
far it moved. So the show pauses and seeks as one, and a render drives the same playhead a
frame at a time. [proposals/time.md](../proposals/time.md) is the argument.

**The time readout is all a person sees of it**: the playhead as `mm:ss.ff` at a fixed width,
a pause button and a reset to zero, at the right end of the menu bar beside the frame-rate
meter, or at the end of the tab row where the menu bar is the operating system's. `F8`
pauses — the key a Mac's keyboard prints ⏯ on. **Rejected: `Space`**: a hand
brushes the space bar, and pause stops the whole synth, which is no thing for a stray key to
do. View ▸ Time shows and hides it, a preference that is on by default, since pause lives
there. A reset is a seek to zero, so every gear is born again at its cycle's start.

**Each node's advance is its own**: the distance since *it* last ticked, `travel` carrying a
seek's jump as a jump and a counted seek telling a jump from a motion. A tab closed for a
minute wakes with a minute's advance, taken as a jump, so its gears land where a tab left open
has them and nothing inside the gap fires. **A stateful node** — a simulation, a slew, an
envelope, a pad's physics — takes that advance as its `dt`, clamped to 100 ms live
(`transport::MAX_DT`), zero on a jump and zero while paused: it carries across a seek rather
than following it. It keeps a **Rate**, not a Time, and is not a gear's to drive: the Slime
Mold's and Cellular Automata's Rate in ×30 a second, Smooth Counter's Rate, Auto Exposure's
Response; the Star Gate's Drift is a step per drawn frame. The Cellular Automata owe their
generations to `dt` with the fraction carried, so the pace is the same at any tick rate.

**State is `f64`, and a count is published whole.** A gear's Cycles, the Time node's Seconds
and an unplugged Time's ambient reading are **counts** (`TickContext::publish_count`), which a
Time reads at the same precision at every count forever: a CPU node in `f64`, unbounded, and a
shader as `vec2f(whole, fraction)` — the whole part wrapped at **40320**, the least common
multiple of 2520 and 128, centered on zero, and the `f32` of the fraction (`phasor::split`) —
reducing the whole part by its period before it adds the fraction, which is exact, since an
`f32` holds every whole number to 2²⁴. Everything else reads one `f32`: a gear's wrapped at
2520, the least common multiple of one to ten, centered on zero, −1260 up to 1260
(`phasor::wrap_count`), and Seconds' the playhead unwrapped. `u_time` is the playhead's
fraction of a second, which is all a shader reads of it. **Rejected: a count as one `f32`
wrapped at 2520.** An `f32` rounds one moment differently by how large the count is, so −0.01
and 0.99 are a hair apart once stored: through feedback, "Reverse the show" closed with 32
pixels a level off after a warm-up at negative time, worse the longer the warm-up, and a
noise at Repeat 16, Static at 16 to 128, the tunnel's 64 and a clip on Hold met a seam once
every 2520 cycles, since none of their periods divides 2520. **Rejected: the wrap into 0 up to
2520**, before it, for the same reason at its worst: a count a hair below zero read 2519.99,
where an `f32` resolves only 2⁻¹² of a cycle. **Rejected: an `f64` uniform**, which WGSL has
none of on the GPUs supersilvia runs on. **Rejected: Seconds' one `f32` wrapped at 2520**,
which jumped 21 minutes into a show, where it has to count on; at one a second an
unwrapped `f32` resolves a frame for 36 hours.

**Nothing is saved, and nothing is an edit.** A project opens playing, at zero. A hand on the readout is playing, as a deck claim is,
and enters no undo history.

**Rejected: a transport per workspace or per timeline**, alone or under a global one: a node
shown on two tabs, or a cable across them, would have two times, and a CPU node has one state
to follow them with. Local time is a gear.

**Rejected: Loop mode**, a switch for the whole show (View ▸ Loop, `L`, a strip with a loop
length) under which every speed ran a whole number of its cycles a loop and every noise was
compiled walking a circle. It was built and it lost for three reasons. It changed the picture:
a noise in Loop mode was another noise, so the show a person tuned was not the one they
exported. It needed a ratio per speed, a hidden control beside every speed that the row showed
and edited in the speed's place, so every moving node had two numbers for one rate and a
mode deciding which one was true. And it made a loop a property of the transport, where it is
a property of what drives the nodes: here a loop is the gear chain's, which a Master Gear's
caption reads ([Nodes keep no time](#nodes-keep-no-time-and-a-loop-is-read-from-the-gears)).
**Rejected with it: the speed dial**, which only a show that should run slower or faster
used — a Ratio Gear on ambient time does that for what it drives — **the transport strip**,
whose pause and reset the readout keeps, and **typed seek**. **Rejected before it: silently
rounding each speed to the nearest whole number of cycles a loop** (`phasor::looped`), under
which a knob said 128 BPM while the clock ran 127.5, and **the `→` readout of the speed that
ran**, which said one thing beside a knob that said another.

**Rejected: a render that leaves live play at the render's last frame.** The playhead goes
back to where the live show was, as a seek, so every gear is born again where it would have
been. **Rejected: a node on a closed tab keeping frozen time**: a tab reopened resumed where it
froze, out of phase with everything that ran on. Suspension still stops the tick and keeps the
state; it does not stop time. **Rejected: firing the crossings a waking node slept through**, a
minute of beats in one frame. **Rejected: a gear's Cycles as one unbounded `f32`.** An `f32` at
131 072 stops moving at 144 Hz; a count is published whole instead, and its one `f32` wraps at
2520. **Rejected: saving the playhead.** Where the show stood is tonight's.

**A loop is the ordinary render, and its length is a Master Gear's.** The Output's own
Render, with its PNG, video and GIF writers, is the one way to render
([rendering.md](rendering.md#the-render-job)); a loop is a render as long as a Master Gear's
caption says. **Rejected: a loop export window** — Export ▸ Loop… on an
Output, a length from a Master Gear or typed, a pre-roll of whole loops, frame `F` rendered and
compared with frame zero and the seam reported, and a badge naming each node that would not
close. It was a second way to render beside the first, and what it knew about a loop is what
the gear chain already says. `examples/loop_gifs` keeps its own seam check for the project's
GIFs. **A GIF is the `image` crate's encoder**, already in the binary for decoding, where
GStreamer's `gifenc` is a Rust plugin that a distribution's GStreamer often lacks.

### Gears: the one place a rate lives

**Chosen.** **Gears** are a category of their own, `Category::Gear`, between Control and
Output. No node that moves with time has a speed: a gear is where a rate is set, changed or
divided, and gears hold the only time state in the model. Time on a wire is a float in cycles
of some clock — **Cycles** the count, published whole for a Time and as one `f32` wrapped at
2520 centered on zero for anything else, **Phase** the fraction — and no new port type
carries it.

- **The Master Gear** (`mastergear`) is the show's clock at a length in seconds. It
  integrates the playhead's advance over its length in `f64`, so a length turned bends from
  where it is and never jumps, and it is born at `playhead ÷ length`: two Master Gears of one
  length agree, and at playhead zero every one is at its cycle's start. Gate is its Trigger's
  length. **Rejected: a length in beats or bars at a BPM, and a tap tempo**: a beat is a length in seconds and a bar a Ratio Gear below it, so a second way to
  say the length was a BPM knob read in two units of three and a Tap that wrote it.
- **The Ratio Gear** (`ratiogear`) is a clock in and a clock out: `ratio × ΔClock In`, a
  count read whole and anything else unwrapped where its output declares its wrap
  (`OutputDef::wraps_at`), or the playhead's seconds with nothing cabled. **Rejected: unwrapping at 2520 only**, which reads
  every wrap of a 0..1 Phase as a cycle backwards, so a gear following one stands nearly
  still. **Rejected: guessing the wrap from the readings**, one while every reading so far is
  under one: a Seconds or a Cycles is under one for its first second, and a seek inside it
  would read as a step back. A drag walks a ladder, ÷16 ÷8 ÷6 ÷4 ÷3 ÷2 ×1 ×2 ×3 ×4 ×6 ×8 ×12 ×16, and
  through ×0 into reverse; typed, it takes `×5`, `÷7`, `3/2`, `0.3` or a leading minus.
- **The Time node** (`time`) is Seconds, the playhead published as a count, and nothing
  else.

**A ratio change lands on the input's next whole cycle.** Until then the old ratio runs and
the display shows the new one pending; it waits at most one input cycle. Every input cycle
after it adds a whole number of output cycles, so a chain of whole ratios closes whatever
phase the change landed at, and the output's downbeat stays on the input's — what a person
listening expects of a gear change. **Rejected: a ratio that bends**, as a speed does, or a
glide to it, which `phase` had: either leaves the downbeat wherever the change ended. **A
chain of fractions closes**: its ratios multiply, and a loop of the Master Gear is as many of
its cycles as the least common multiple of the chains' denominators, so a ÷4 below asks for
four. A ratio that is no fraction with a denominator up to 64, or has a cable in it, runs and
never closes, and the caption counts it.

**Hold, Reset, and a seek.** Hold is a toggle that freezes the gear where it stands and closes
any gate it left open; Reset puts it at the start of a cycle and is a beat. A seek — the
readout's reset among them — and a render's start are jumps: every gear is born again where
the playhead puts it, a Ratio Gear at its ratio times its input's reading, and fires nothing
on the way; a gear born on a whole cycle fires that beat, so a render's first frame and the
readout's reset are a downbeat.
**Rejected: a Ratio Gear born at its ratio times the fraction of its input's cycle**, which a
÷4 bar starting afresh on whatever beat a seek lands on would give: a gear born again after a
seek, a reopened tab or a render's warm-up is then not where playing puts it, and a render's
first frame depends on how long its warm-up was.
silvia's Start/Stop and Reset on the oscillator and the sequencers are the Hold and Reset of
the gear that drives them.

**One option draws it, Display: a still Rosette, the default, or meshing Gears**, both turning
at the real rate in one body region (`Region::Gear`) of one height. The Rosette is `p` petals
wound over `q` loops for a ratio `p/q`, a turn an input cycle, a dot at the output's phase; Gears are `k·p` teeth driving
`k·q`. Under a Master Gear a caption says what a loop of it needs, read from the chains below
it (`nodes::chain`). **Rejected: two sibling nodes** for the two pictures: they draw the same
state, a file should not change slug to change a picture, and switching a sibling means
deleting and rewiring.

**The nodes a gear drives.**
- **`oscillator`** is a wave at `Time + Offset`, a wave a second at rest. Frequency,
  Start/Stop, Reset, One Shot and the 50 ms glide are gone: they are a gear's, and a one-shot
  is `animation`'s.
- **`video` and `imagegif`** play `Time + Offset` in plays of the clip, at the clip's own
  speed at rest; a scrub writes Offset, reverse is a Ratio Gear at `-×1`, and Loop/Hold wraps
  or clamps the sum. **A render waits for a clip's frame**, holding the frame and ticking only
  what waits, where live play shows what the decoder has.
- **`stepsequencer` and `euclideanrhythm`** fire on crossings of `floor(16 × (Time + Offset))`,
  sixteen steps a bar, and stand still at rest until a gear drives them: a Master Gear a bar
  long is their tempo. A reading that runs backwards, more than a bar in a tick, across a seek
  or onto another clock's cable is a jump that fires nothing and closes the gates. **Step stays**: while cabled, a sequencer
  counts Step events and ignores Time, the one stateful path, because an event clock — a tap,
  a threshold — is not a gear.

**Rejected: a ratio knob on each speed**, the hidden control Loop mode kept beside every
speed. A gear is where a rate lives: one cable drives any number of nodes at one rate, one
knob turns them together, and a chain of gears says whether they close. A rate on each node is
a rate in twenty places, each to find and each to keep whole. **Rejected: `phase` and
`bpmclock` kept beside the gears.** With no BPM on the moving nodes, a
`bpmclock` is exactly a Master Gear that fires on its cycle and `phase` exactly a Ratio Gear,
and two nodes doing one job is what this removes. **Rejected: keeping the Time node's own
Speed, Start/Stop and Reset**, a second transport beside the real one; and its loop Cycles and
Phase, which a Master Gear publishes. **Rejected: a cable that replaces a clip's playback
through its Offset**, as main's Position did: Time is the input that replaces, Offset adds,
and a Ratio Gear ×0 into Time plays the cable alone. **Rejected: a live clip that waits.** A
tick never waits; only a render, which owns its clock, holds a frame. **Rejected: ticking the whole graph again
while a render holds**: a noise oscillator would draw a second random and a one-frame gate
would close early, so how often the decoder was slow would change the film.

### Nodes keep no time, and a loop is read from the gears

**Chosen.** A node that moves with time is a function of `Time + Offset` and keeps nothing:
`phasor::Generator`, the per-node accumulator and its `accumulates:` macro arm are gone, and no
moving node publishes a phase of its own. A node may remember last tick's reading to fire on a
crossing — an edge detector, not an accumulator. So a node draws one picture at one moment
however the show got there, a render needs no path to reach a frame, and whether a loop closes
is arithmetic on the gear chains rather than a question about history. **Rejected: a speed on
each node**, integrated into a phase the node kept. It bent rather than jumped, but it made
every moving node stateful: its picture at a moment depended on every speed it had run at to
get there, a closed tab had to catch up on it, and a loop closed only where each node's history
happened to land on a whole cycle. Moving the one integral into gears keeps the bend, since a
gear's length or tempo turned bends, and leaves the nodes stateless.

**The rate at rest is silvia's default speed**, in the node's own cycles an ambient second,
declared on the node (`NodeDef::ambient`). A rate that was irrational in seconds, a period of
20π, is rounded to nearby whole seconds — Rotozoom and Shaky Cam a cycle a minute, Geiss Flow
one in 160 s — under 5% off, and a whole number of seconds is a length a loop can have. **Where
silvia is still at rest the rate is zero**: Cosine Gradient, Simplex,
Fractal, Domain Warp, Static and the two sequencers stay still until a gear drives them, and
Offset places them. **Rejected: a slow drift at rest** on each of them, which would put motion
silvia does not have on a node a person drops in.

**A noise repeats by an option, Repeat**: Never, silvia's look and the default, or every 1, 2,
4, 8 or 16 cells, and on Static every 4 to 128 rolls. With `N` a noise walks a circle of
circumference `N` through its four-dimensional noise at `(Time + Offset) ÷ N`, and Static reads
its roll modulo `N`. It is an ordinary option that rebuilds. **Rejected: repeating
automatically**, which Loop mode did: a circle through four dimensions is not the line through
three, so repeating changes the picture and a person chooses it. **Rejected: both forms in
every module, chosen by a uniform**, which would compile a four-dimensional noise into every
shader holding a noise for the few that repeat.

**The Tunnel's path is retuned so its flight repeats.** Every path frequency is multiplied by
5π/16, so Sine and Lissajous repeat every 64 units of depth and Helix every 16 — whole numbers
of both depth wraps — and the camera's depth is taken modulo 64 while the wall wraps: Time 64
draws Time 0. The wiggle is 1.8% slower, which nobody sees. Depth Wrap None never repeats. A
gear's count reaches it whole, its whole part wrapped at 40320, which 64 divides, so a tunnel
on a gear never meets a seam.

**When a loop closes.** A node on a chain from Master Gear `M` advances `m × Πr ÷ P` of its own
periods over `m` cycles of `M` — `Πr` the product of the chain's ratios, `P` the node's period —
and closes when that is whole; a node on ambient time closes over a length `L` when
`rate × L ÷ P` is whole. A Master Gear's loop is its length times the least common multiple of
its chains' denominators (`nodes::chain::master_length`), which its caption says: "loops in 4
cycles · 8.000 s (÷4 on ratiogear12)", or "2 nodes will not close". An unconnected color
input's hue wheel stands still, so it never holds a loop open. **Rejected: silvia's turning
hue wheel**, once every 20 s of `u_time`: no loop that was not a multiple of 20 s closed on it.
**Rejected: a loop form declared on every node** (`Loops`: Period, Circle,
Settles, CarriesAcross, Live) and a **loop badge** reading them: only the loop export's badge
read them, and with the export gone the chain arithmetic and the caption are the whole of it.

### An action is a gate, not a pulse

**Chosen** before the first inhabitant of `PortType::Action`, and now built: `button`,
`bpmclock`, `clockdivider`, `counter` and `adsr`. The choice was made ahead of the code
because it is not reversible cheaply: an event system's shape is assumed by every node
downstream of it, and a pulse-only design has to be replaced the first time an envelope is
written.

An edge fires **down** when its condition becomes true and **up** when it stops being true.
A receiver gets both. That is what lets an ADSR be held rather than retriggered, a MIDI note
be released, and an audio band say when it *stopped* being loud — three things a bare pulse
cannot express, and all three are in the first handful of nodes anyone would want.

**Rejected:** a pulse, with duration recovered downstream by pairing two edges. It moves the
state into every consumer and gets the boundary wrong the first time two sources overlap.

Delivery is inside `tick`, in the order `Synth::tick` walks. An event is a CPU thing: it never
reaches a uniform, and — like everything else in a tick — it never waits. That order had to
grow one thing the compiler's does not have: **action edges order the tick too**
(`Graph::tick_order`). Without them a chain of events gains a frame of latency per hop, and
which hop depends on node ids. Action edges are not cycle-checked, so a loop among them is
ordered as a feedback loop is: as one block ahead of what it fires, broken inside at a
deterministic node.

### An event carries when it happened

**Chosen** with the first five nodes, and it is the half of the event design that is about
*time* rather than about shape.

`tick` runs once per frame. A frame is 16 ms, an envelope's attack is often shorter than that,
and a beat at any interesting subdivision does not land on a frame boundary. Quantizing every
event to the frame it was noticed in is audible as jitter on an envelope and visible as a beat
landing late, and it silently drops beats whenever the interval is shorter than the frame.

So an `Event` is an edge **plus `at`**, its offset in seconds inside the frame delivering it. A
source with a phase of its own says where the crossing actually fell; a source that only knows
the frame says zero. An integrator advances in segments between the events rather than in one
lump: `adsr` gives a gate that opened three quarters of the way through a frame a quarter of a
frame of attack, and `tests/actions.rs` asserts exactly that, which no frame-quantized envelope
can pass.

This also settles the audio-reactive case without changing anything here. `audio/` already
analyzes on the audio thread and its `Analysis` already carries `samples` and `published`, so a
threshold crossing detected *there* can arrive stamped. The frame thread never has to see the
crossing to know when it happened.

**Rejected: sub-stepping the tick** — running `tick` N times per frame at `dt/N`. It costs N
times the CPU for every node whether it needs it or not, and it is still quantized, just more
finely. Nothing about it is exact.

**Rejected: moving the event half to the audio thread.** The graph, every CPU node's state and
the whole `tick` contract live on the frame thread; a `CpuNode` is deliberately not `Send`.
This would be a second architecture beside the first for a problem sub-frame stamps solve
outright.

**Not claimed:** a picture faster than a frame. A `UniformNumber` is sampled into a
uniform once per frame however precisely it was computed. Sub-frame time removes jitter and
fixes phase, and does not make a strobe faster than the display visible.

### A node publishes the field it already computed

**Chosen**, with the `Generate` library. Where a node calculates a varying number on the way
to its picture — a shape's coverage, a noise value before it was mapped to color, a fractal's
escape count, a region's inside/outside test — that number is an output beside the color, not
something a consumer re-derives.

This is silvia's rule, read out of the registry rather than off the prose: 21 of its nodes
publish one. It is emphatically **not** "every generator publishes a mask" — `stripes`,
`phyllotaxis`, `sierpinski`, both gradients, `houndstooth`, `prideflag` and `checkerboard`
have none, because in those the varying number *is* the picture and there is no second field
to expose. The question to ask per node is only *did this compute a field it is throwing
away*.

Nothing about this codebase argues for widening the rule. `VaryingColor` into
`VaryingNumber` costs a node here (the `decompose` family) exactly as it does in silvia
(`channelsplitter`), so a published varying number saves the same one node in both.

**The vocabulary**, fixed before the library was ported, because seventy nodes written
without one is seventy names for one idea and that is the part retrofitting cannot fix
cheaply: `output` for the picture where it is the only one and `color` where a field sits
beside it; `map` for a second picture; and for the fields, `mask` — **coverage, 0 to 1, 1
inside, and nothing else ever** — `smooth` for a normalized escape count, `angle` for a polar
sweep, `value` for the raw 0 to 1 quantity a picture was made from. A registry test holds
every `Generate` node to that list, so adding a name is a deliberate edit to it.
[nodes.md](nodes.md#what-a-generator-publishes-beside-its-picture) is the table.

**`value` is the widening the noises forced**, and it is the shape the mechanism was built
for. A noise computes a varying number and *then* picks a color from it; that number is not
coverage, so it cannot be `mask` without emptying `mask` of its meaning, and it is not an
escape count or a sweep. silvia calls all five of them `mask` — and `domainwarp`'s
displacement length too — which is exactly the drift the closed list exists to catch. `value`
is deliberately the last resort: a field that is coverage still says `mask`, and only a field
that is nothing more specific says `value`.

**Where this departed from silvia.** silvia's `mandelbrot` and `juliaset` name their escape
*fraction* `mask` — 0 inside the set, the opposite of every other mask it has — and sample it
at `uv - 0.5` while their other three outputs sample at `uv`. Here both publish `mask` for
the set and `smooth` for the count, which is what made the vocabulary worth fixing first: the mistake
is invisible until two nodes disagree, and by then there are seventy.

### A distortion is a `Transform`, not a category of its own

**Chosen** with the eight ported distortions. silvia has a `Distort` category and this does
not: `wave`, `whirlandpinch`, `kaleidoscope`, `tile`, `repeater`, `domainwarp`, `scatter` and
`tunnel3d` are `Category::Transform`, beside `zoom`, `rotate` and `fisheye`.

**Rejected: a `Distort` category.** The categories here say what a node *does* — `Generate`
makes a picture from `uv`, `Effect` reads a picture somewhere other than under the fragment,
`Transform` moves the sampling coordinate. All eleven do the last of those and nothing else: each reads its one
input at a coordinate of its own through the macro's `at`, and none of them looks at a second
texel or invents any color. The line between `zoom` and `kaleidoscope` is how far the
coordinate travels, which is a matter of degree and of taste, and a category boundary drawn
on taste has to be re-argued for every node that lands near it. Two menu groups where one
describes the operation is also two places to look for the node that resamples.

The cost is a longer `Transform` menu, and that is the right cost: someone reaching for a
kaleidoscope is reaching for the group that resamples.

### The crop is one node with two sets of handles, and a knob may take its top end from another

**Chosen** with the parity pass over the transforms: silvia's `regionabsolute` and
`regionsized` ported, and five nodes here brought to what the review of the two libraries
accepted.

**One crop, two nodes.** silvia's two are the same rectangle reached by four edges or by a
center and a size, and both are here under both sets of handles, sharing everything below
them in `region.rs` — the four outside modes, the soft edge, the coverage. One node with a
handles option was the alternative and it loses twice: the choice is a recompile boundary,
so switching handles mid-set would rebuild, and a node whose rows change under an option
cannot say in the menu which shape it is. The size is the full one and halved inside, as
silvia has it, so the sized node's default box is half the absolute one's default rectangle.

**The picture is `output` and the coverage is `mask`.** A `Transform`'s picture is its input
passed through, so it keeps that name with a field beside it — the one exception to the
picture rule — and what this publishes is coverage, which is the one thing `mask` may mean.
The row reads *Crop Mask*, silvia's label.

**Outside is branchless, and a rectangle a cable turned inside out stays defined.** `mod` and
`clamp` are identities inside the rectangle, so silvia's `inside` test buys the wrap nothing;
the tile size has a floor and `clamp` gets its ends in order, because either is undefined in
the shader and a cabled edge reaches both. The soft edge has a floor for the same reason —
`smoothstep(0.0, 0.0, x)` divides by zero and zero is the default. Softness bites in
background mode alone, as in silvia, and the help text says so rather than leaving it to be
found.

**A knob may take its top end from another knob.** `Control::num_capped` names a second input
of the same node whose value is this control's maximum, and the bus writes it into the node's
own `ControlRange` inside the same command — so a kaleidoscope's Source Segment stops where
Segments does, comes down with the count, and puts both back on one undo. silvia does exactly
this, from a listener that writes the `max` attribute the control is serialized with. It
follows the *control* and not the port: a cable into Segments leaves the range where it was,
because a number arriving from upstream is not this node's to bound by, and the fold clamp in
the shader is what keeps a cabled Source Segment on a wedge that exists.

**Rejected: deriving the cap in `control_range` instead of storing it.** It would need no
command and no document data, and it cannot pull the value down with the count — a stored
number outside its own track is exactly what the range editor's refit exists to prevent.
Storing it also puts the cap where every other range already is, so nothing new reads it.

The other four are one line each. `rotate`'s help text says which way positive turns, which is
the one thing about an angle knob that cannot be worked out by looking at it. `repeater`'s
second output is *Grid Mask* again: plenty of nodes here publish a Mask and only one publishes
a grid's. `domainwarp` keeps the name *Value* — it measures how far the coordinate moved, and
`mask` means coverage everywhere else — with the help text naming it as the displacement
length, which is what a hand looking for silvia's Mask needs to read. And `tile` declares a
wider body so its Slide Direction can be read closed.

### An angle is in turns

**Chosen: 1 is one full turn on every angle knob and input**, decided on 1 October.
Time is in cycles and a gear's Phase runs 0 to 1, so a Rotation in turns takes a Phase and
turns once a cycle with no snap and no Math node between them. The unit is the glyph `↻`,
because the word does not fit beside three decimals in the number control.

**Rejected: silvia's half turns, in π.** A Phase into a Rotation turned half way and snapped
back, and every knob read "0.5π" for a quarter turn. **Rejected: the word "turns" as the
unit**: "−0.250 turns" runs under both steppers of silvia's 100-point control.

### An effect reads elsewhere; a color map reads the texel it was handed

**Chosen** with the fifteen ported effects. silvia has one `Effects` heading with
thirty-three nodes under it. That heading is not a description of anything, so it is not
imported: each of the fifteen is filed by what it does to a picture, and eleven of them land
in two different categories.

The test is **where the answer comes from**. A `Color` node's output at a fragment is a
function of the input at that fragment and of nothing else — `contrast`, `gamma`, `levels`,
`invert`, `posterize` and `layerblend`, and `vignette` too, whose falloff comes from `uv`
while its color comes from the one texel it was handed. An `Effect` reads the picture
somewhere else: `blur`, `sharpen`, `bloom`, `dilate`, `erode` and `emboss` read a
neighborhood of texels, and `halftone`, `mosaic` and `dither` read a *cell* — what a
fragment gets depends on where it sits inside a tile, which is not a color map by any
reading. A `Transform` moves the sampling coordinate and returns what it found.

That widened `Category::Effect`'s own words from "looks at more than one texel", which was
true of the six kernels and of `edgedetection` before them and false of the three cell
patterns. Widening it is the right direction: the alternative was filing a halftone under
`Color`, where a node whose picture is a grid of dots does not belong, or inventing a
fourth group for three nodes.

**How dramatic a node looks is not the test.** A posterize to two levels changes a picture
more than a one-pixel blur does, and it is still a color map. This is the same argument the
[distortion entry](#a-distortion-is-a-transform-not-a-category-of-its-own) makes one category
over, and for the same reason: a boundary drawn on how strong the result is has to be
re-argued for every node that lands near it.

**A sampling radius is a `VaryingNumber`; a kernel size is an option.** A radius scales an
offset and changes no generated code, so it is a control and a cable can drive it. A loop's
half-width *is* generated code, so it is an option, which is the recompile boundary — and the
consequence, which is the point of writing it down, is that every loop in the library has a
constant bound and can be checked by eye. Nothing here reallocates, so this is not the
"[not every CPU number is a uniform number](#a-fourth-port-type-uniform-numbers)" argument
about cost; it is only about what the compiler is handed.

**These are quadratic, and there is nowhere to hide it.** A graph is one expression compiled
into one pass, so a blur has no intermediate target to separate into two passes: a half-width
of `r` is `(2r+1)²` evaluations of everything upstream, and stacking two of them multiplies.
Measured at 1280x720 over a `checkerboard`, on an Intel UHD 770: a 3x3 blur is
0.3 ms and a 7x7 blur 0.8 ms, and a 24-sample bloom on its own is 0.9 ms — each of them
comfortably inside a frame. `blur → bloom` is 3.6 ms at 3x3 and **10.8 ms at 7x7**, because
the bloom evaluates the blur twenty-five times and the blur evaluates the checkerboard
forty-nine, which is 1,225 reads a fragment. The sample counts are in the tooltips because
the person choosing `7x7` is the person who needs them, and the multiplication is why the
number to watch is the overlay's GPU line rather than any one node's.

### Five more reads of one picture, and every one of their loops is an option

**Chosen** with `kuwahara`, `motionblur`, `radialblur`, `sincfilter` and `supersampling`, the
five of silvia's effects that read their one input at many coordinates and had no counterpart
here. They are one file, `multisample.rs`, beside `convolve.rs`: a kernel over a square
neighborhood is one shape and a line, a ray or a sub-pixel grid is another, but the rule
above them is the same one and is worth being in two places rather than nine.

**Two of silvia's knobs became options, because each is a loop's bound.** Kuwahara's Radius
is a row a cable can drive from nine samples to a hundred and sixty-nine, and the Sinc
Filter's Kernel Size from nine to eighty-one, with nothing on screen saying what either
costs. That is the [radius-against-kernel-size
rule](#an-effect-reads-elsewhere-a-color-map-reads-the-texel-it-was-handed) exactly: a
half-width *is* generated code. Both are selects whose labels carry the count, as `blur`'s
do, and the defaults are silvia's — a 7x7 window and a 7x7 kernel, forty-nine samples each.

**Three of the five stepped in texels of the real output, and now step in pixels of a
720-high frame.** silvia divides by `u_resolution`, which on a 16:9 frame makes a Motion
Blur's streak lean — a hundred pixels of Amount reach nearly twice as far across as down,
which is visibly wrong at any angle off the axes — and makes a Kuwahara's window and a Sinc
Filter's kernel change shape with the Output. `REFERENCE_HEIGHT` on both axes is [one field
rather than one field per Output](#no-node-reads-the-resolution). **Radial Blur is the
exception that needed no correction:** its smear is a fraction of the ray from the center,
already in world units, so the port is silvia's arithmetic unchanged.

**Super Sampling's grid is centered on the fragment.** silvia offsets cell `i` of `n` by
`(i - 0.5) / n`, which lands symmetrically only at 2x2; at 3x3 and 4x4 the samples lean
toward one corner, so turning the option up both smooths the picture and shifts it. The
offset here is `(i + 0.5) / n - 0.5`, the same grid about the fragment at every count. **1x
stays the default**, where the body is a passthrough and the node costs nothing, so it can
sit in a patch until it is needed.

**Radial Blur keeps sixteen taps and gains no Quality option.** The tap count is not a knob
in silvia and making it a choice would change the picture at the default, which is the one
thing a port may not do. The trailing cousin of the effect — an Output fed back through Zoom
into a Mix — stays a patch rather than a mode: [feedback is a
cable](#feedback-is-a-cable-not-a-node), it costs nothing extra, and it is a livelier,
different look rather than a cheaper version of this one.

**Only Kuwahara had a field to publish.** It compares the variance of four quadrants and
keeps the smallest, which is a measure of how rough the neighborhood it took its color from
was: that is the `value` beside the `color`, and it is [a field the body computed
anyway](#a-node-publishes-the-field-it-already-computed). The other four average or resample
and throw nothing away — a motion blur's sum *is* its picture — so each publishes one
`output` and nothing else.

### A sort along a run, a border back where silvia had it, and a pattern on the ladder that already exists

**Chosen** with the parity pass over silvia's Mosaic, Pixel Sort and Color Dither.

**`mosaic`'s Border Color follows Border Width.** The two rows that make a border were at
opposite ends of the node, Border Color hoisted to the second row and Border Width three
rows below it, so a border was dialed at two ends of a body. silvia's order puts them
together and nothing was bought by splitting them. **And the hexagon drops its border at a
width of zero**, which is silvia's own guard: the hexagon's `edge` is a hex distance the
smoothing straddles at zero, so the softened step painted a hairline along every cell wall
that only came off by taking Smoothing down too. The other three lattices already went to
nothing, so the guard is on the hexagon alone. Everything else about the node was already
silvia's, down to the jitter hash, so the picture is unchanged.

**`pixelsort` reads along a run, which is the third shape an `Effect` takes.** A kernel reads
a patch around the fragment and a screentone reads a cell; this reads the whole chunk the
fragment's row or column was cut into, sorts it, and returns whichever pixel sorts to the
fragment's place. That is still "somewhere other than under the fragment", so the category
holds; it is its own file because it shares no lattice with `screentone.rs` and no kernel
with `convolve.rs`. Chunk Size is the loop's bound, so by the rule above it is an option with
its sample count on each label, and at 64 a fragment costs sixty-four samples of everything
upstream plus an insertion sort — the most expensive node here, and the tooltip says so
rather than leaving it to be discovered mid-set.

**Its seed is a knob and silvia's New Seed button is not ported.** silvia hides a counter
behind a press and re-rolls it into a uniform nobody can see or share. A row can be pressed —
a Button or a Master Gear into a Triggered Random — beaten, driven or shared between two nodes,
and silvia's Animate switch, which stirred that hidden counter every frame, is then a seed
driven by anything that moves rather than an option of its own.

**The run is indexed in pixels of a 720-high frame, biased into the positives.** silvia
indexes its scanlines off `u_resolution`, which [no node here reads](#no-node-reads-the-resolution);
the reference grid is the same convention every kernel steps by. Worldspace runs either side
of zero and integer division truncates toward it, so an unbiased index folds two chunks into
one along the picture's middle — the bias is half the pixels a coordinate can reach, added
before the division.

**silvia's Color Dither is two rows on `posterize`, not a second node.** Both snap each
channel to a ladder of levels; all Color Dither adds is where the rounding threshold comes
from and how wide one cell of it is. So `posterize` grows `Pattern` — Hash Noise, which is
what it already did, the three Bayer grids, Hexagonal and Triangular — and `Scale`, in
pixels. The Bayer matrices are `dither`'s own, read out of `screentone.rs`, so the two nodes
cannot draw different grids under one name; the hexagonal and triangular lattices are
silvia's, which are `mosaic`'s written over the pixel grid.

**Rejected: porting `colordither` as its own node.** It would be a second node snapping to
the same ladder, with its own defaults for Levels and Gamma, and a person reaching for a
posterize would have to know which of the two had the pattern.

**`posterize` stays a `Color`**, though it now reads its input at the cell's center. The
default Scale is one pixel of a 720-high frame, which *is* the texel under the fragment, so
the node is a color map wherever nobody widens it — and reading the four inputs at the cell
rather than at the fragment is what stops a modulated Levels tearing inside a cell, which is
the trick `dither` already does. Its dither offset stays `posterize`'s, `(threshold - 0.5)`
times the amount rather than silvia's amount divided by the level height, because the Dither
row and its range are not what this change was about and every patch holding one would move.

### A glitch takes its beat from a cable, and Step is a knob

**Chosen** with the port of silvia's `glitch`. silvia's node carries its own tempo: a BPM row,
a 1/4-to-4 subdivision select, and a Step button incrementing a counter nothing else can see.
Its shader quantizes `u_time` into a beat count, adds that counter, and hashes the sum — which
is what makes it a glitch rather than noise, because the pattern re-rolls on the beat and
holds dead still in between.

Here the beat is already four nodes. A Master Gear's Trigger fires it, `clockdivider` thins it to a
subdivision, `counter` turns the fires into a number and `button` covers the hand, so the node
needs one `Step` input and nothing else. It floors what it reads, so the pattern is still dead
between whole numbers whatever drives it, and a gear's Cycles cabled in steps rather than slides.
Two glitches on one counter then glitch in lockstep, which silvia's hidden per-node counter
made impossible, and a step can come from anything that counts.

**Rejected: a BPM knob and a Step button on the node.** It is a clock of its own in a program
whose clock is [one clock](architecture.md#one-clock-on-a-thread-of-its-own), duplicated once per glitch, and reachable
only by hand.

**It is an `Effect`, not the `Distort` silvia files it under.** There is no such category here
— [a distortion is a `Transform`](#a-distortion-is-a-transform-not-a-category-of-its-own) — and
a transform this is not: it reads the picture at four coordinates, and which one a fragment
gets depends on the block it sits in. That is the cell reading of
[an effect reads elsewhere](#an-effect-reads-elsewhere-a-color-map-reads-the-texel-it-was-handed),
the same one that files `mosaic` there.

**The early return stays.** The rest of the library is branchless where silvia branched, because
a `step` and a `mix` cost less than a divergent warp. Here the branch skips *three samples of
everything upstream* on the blocks the hash did not pick, which at the default Intensity of 0.15
is most of the frame. `mask` — the picked blocks — is published beside the picture, so a graph
can read which blocks moved without re-deriving it.

### `debug` was a `Tap`, and `note` is a node

**Chosen** with the effects. Two of silvia's nodes exist to tell you something rather than to
change a picture, and they land in different places.

**`debug` is `Category::Tap`.** `tap` and `sample` carry numbers out of the picture; `debug`
carries one in, drawing it as digits over the frame. The category is the boundary between the
numbers and the picture rather than the direction across it, and its words say both. The
alternative was `Effect`, and by the test above it is not one: it reads exactly one texel, the
one under the fragment, and paints over it.

It is worth having even though [a uniform number's value is already on its
row](#uniform-numbers-are-always-on-the-row). That row is on the canvas. `debug` puts the
number on the **output**, which is the screen a performance is actually looking at — and it is
the only way to read one there.

**The digit painter was removed on 21 September 2026.** A number is read out on a port's row
and through `tap` and `sample`, so painting digits into the picture had no use left.

**`note` is built, and it brought a third store with it.** See [a node's own values are not
options](#a-nodes-own-values-are-not-options). The paragraph that stood here said a note
needed either a fifth option kind or a canvas object, and both halves of its reasoning
expired: the kinds it listed were four and are now five, and the two registry objections that
kept the node out — a free-text option drawing a file button, and `asset_users` walking every
free-text option — were dissolved when both started reading `OptionKind::Asset` instead of
"has no choices". What was actually left was a store, and the store is the interesting part.

### A pure-WGSL node is a macro, not a builder or a trait

**Chosen.** `math.rs` and `decompose.rs` each carried a macro over one node shape; `node!` is
the same idea widened until any port list, option list and set of bodies fits, because 99 of
silvia's 144 nodes are exactly that and nothing else. It expands to the `const NodeDef` the
[struct decision](#nodedef-as-a-const-struct-not-a-trait) settled on, so nothing downstream
can tell a macro node from a hand-written one and a node leaves the macro the day it grows a
`cpu` half.

**Rejected: a builder.** `NodeDef::new("circle").input(…).output(…)` cannot be `const`, so
the registry becomes a `OnceLock` or a `Vec` built at startup, and the whole reason for a
`const` struct — no allocation, no initialization order, a definition that is data the
compiler has already laid out — goes with it.

**Rejected: `format!` for the bodies.** The macro splices `{key}` holes at run time instead.
Two things fall out and both matter at this scale: WGSL braces need no doubling, so a body is
the shader text and not an escaped version of it; and a hole nobody wrote is never asked for,
so an output that ignores an input neither declares its uniform nor drags that input's
producer into the shader — which is what lets a `mask` and a `color` share one `wgsl_common` block
without the mask function pulling in the color's texture.

### A tap measures what a picker says, and a cable overrides the picker

**Chosen.** `tap` carries a `measure` select over the eleven `Convert` quantities —
defaulting to `luminosity`, the one it has always measured — and a `VaryingNumber` input,
`number`, which is measured instead while anything is connected to it. The expressions are the
one table `decompose` defines, so the eleven nodes and the picker cannot drift; the cable wins
over the select because a cable is there or it is not, which is a more specific statement than
a stored string.

**Rejected: eleven tap nodes.** A node per quantity is the `Convert` family a second time,
with one slot each and a readback apiece, and the graph reads worse: a chain that measures
its red says `tapred` where it should say what it taps.

**Rejected: the tap publishing every quantity at once.** Seven words per quantity per Output
in the buffer, eleven quantities, and a node with fifty-five uniform number outputs. Nobody
wants eleven means; they want one, of the thing they are looking at.

**Rejected: the sidechain as a node of its own.** A tap that measures a varying number is
still a tap — the picture still passes through it, the slot is still the same slot, and the
CPU half still publishes the same five uniform numbers. Splitting it would be two nodes with
one behavior.

**The picker's inertness is data, not a case.** `OptionDef::overridden_by` names the input,
and the canvas draws any such select disabled showing that input's label. `ui/` matching on
`tap` to do it would be a third bit of node semantics escaping `nodes/`, where the rule allows
two.

### A control's range belongs to the node, not to the node kind

**Chosen** with the number control's missing gestures. `NumberSpec` was rebuilt each frame
from the static `InputDef`, so `min`, `max` and `step` were properties of a *kind*. They are
properties of an instance: a speed that only ever wants 0.9 to 1.1 is unscrubbable across the
range its definition declares, and a fader or a lane mapped onto a control has to know where
that control's ends are *here*. It is also what makes `R` a different gesture from `D` —
`D` returns the value, `R` returns the value and the range.

`Node::values` holds only the controls that differ, which is almost none of them in almost any
graph, so the file skips it when empty. `nodes::control_range` is the one place the question
is answered. The range lives in `values` beside a note's text because it *is* one — see [a
node's own values are not options](#a-nodes-own-values-are-not-options).

**A range goes where it is put; the definition's is advice.** silvia lets the ends go
anywhere and so does this. It was clamped to the declared range at first, on the argument that
a definition's range is the node's statement about where its own maths is defined and a zoom
of zero is a NaN frame however deliberately it was typed. In use,
that defeats half the point of an editor for the ends. A control whose ends can only ever
narrow cannot be pushed, and pushing a parameter past where its author expected is most of
what a video synth is for.

What is still refused is a non-finite end, because NaN is not a decision. An inside-out pair
is straightened rather than rejected, since that is what a half-typed pair looks like. The
declared ends stay printed in the popup's header, as the advice they now are, and a control
whose range is not its definition's still wears an accent border.

**Right-click opens the range editor, where silvia uses `Ctrl` + hover.** `Ctrl` is already
the coarse scrub multiplier, so silvia's gesture would have the editor open under the pointer
throughout any coarse scrub. Right-click is the gesture for an object's own context, and it is
findable in a way a modifier held over a hover is not.

### Uniform numbers are always on the row

**Chosen.** A `UniformNumber` output draws the number it published this frame on its own row,
and a control whose input is connected draws the number that arrives rather than the one it
stores. Patching numbers is otherwise done blind: every cable in the CPU half carries a value
nothing on screen says, and the one place a person looks — the control on the receiving
input — was showing the value the connection had already overridden, which is worse than
showing nothing.

**Rejected: showing it on hover.** A meter you have to hover is not a meter. The whole use is
watching several numbers move at once while a hand is on a third control, and a hover shows
one of them and only while the pointer is parked.

**Rejected: a readout node.** silvia's `debug` draws its number into the picture, which makes
it a performance element — it is on the library list as that, and it is a different tool. A
node per number also costs a node and a cable per number watched, which is exactly the cost
that stops anyone watching.

**A fed control is dimmed on its ground, not on its number.** The disabled styling was a
uniform dim, which was right while the number was the stale stored one and wrong the moment
it became the arriving one: the only place that value is written cannot be the least legible
thing on the node. The inertness moved to the fill and the steppers — the parts a hand
reaches for — and the fill took the uniform number hue as well, so cyan is a knob and violet
is a meter without a label saying so.

**The cost is real and bounded.** A readout's text changes on most frames, so it misses
egui's galley cache by construction; what keeps it off the frame budget is that the strings
are short, formatted into a reused buffer, laid out at the quantized font size, and drawn
from the same row walk as the port labels — so a culled node, a collapsed node and a row a
tick hides never reach it. See [ui.md](ui.md#the-value-on-the-row).

### A timeline and a viewport, over one parameter address space

**Agreed, not built.** [proposals/timelines.md](../proposals/timelines.md) has the shape —
clips, tracks, the transport, the compositor. This records what is agreed whatever the
details become, because each of these is assumed by everything downstream of it.

> **Everything with a value has an address, and automation, graph uniform numbers, MIDI and
> direct manipulation are all sources that write to addresses.**

Not four systems — one address space with four writers. Half of it exists: `Node.controls`
is already "the single source of truth for a parameter", and `ui/` already emits commands
that `App::apply` handles without caring which view produced them. **A viewport is a second
editor, not a second model** — dragging a handle on a picture emits the same kind of
coalesced command a number scrub does.

**Keyframes are document; values are derived.** Automation must not go through the undo
history: editing a lane is an edit, *playing* one is not, and a lane emitting a `SetControl`
every frame during playback would make undo meaningless however well it coalesced. So a lane
is saved, undoable document data, and the value it produces each frame is derived state
written during `tick` — the same relationship the graph has with its compiled shader. The
transport is app state, like a pan.

**Precedence is connection > lane > manual, with whichever is winning visible**, the way a
connected input already draws its control disabled. A lane is the third case and gets the
same treatment: visible, not silent.

**A lane is Read, Off, or Write** — the mixing-console model rather than the
post-production one, because a live tool needs to take the knob back without destroying the
automation. Off is therefore distinct from delete.

**Automation is created per parameter and grouped per node.** The unit is the address, so
the affordance belongs on the parameter; the node is how lanes are grouped for display,
because a flat list across a real graph is unreadable. A folder, not a switch.

**The graph and the timeline are orthogonal:**

> The graph answers *what does a frame look like, given a time*. The timeline answers *what is
> the time, and which parameters change across it*.

Most nodes have no extent in time; a track per `multiply` means nothing. A node earns a
place on the timeline by having time extent or by having something animated.

**Retiming is a function feeding the Time input.** Trimming, reversing, freeze-frame and
speed ramps are all that, which is why time was made an input port in the first place; what
arrives there replaces ambient time, so a cable drives the node wholly.

### An option carries what it costs, and three of the four kinds cost nothing

**Chosen**, replacing an earlier entry that proposed asset references as a third *parameter*
kind beside controls and options. They are a kind of **option**, and so are two other things,
which is a smaller change that answers more: `OptionDef::kind` says what changing an option
costs, and `SetOption` asks it instead of always rebuilding. The kinds are in
[nodes.md](nodes.md#option-kinds).

The original argument stands and is what the kind is for. Putting a parameter in the wrong
slot is a performance bug:

| | changes | cost of a change |
| --- | --- | --- |
| a control | a uniform | free |
| a `Code` option | generated WGSL | **shader rebuild** |
| an `Asset` option | which texture is bound | free, but discrete and asynchronous |
| a `Uniform` option | one `int` the program reads | free |
| a `Presentation` option | what the canvas draws | free |

Swapping an image changes no WGSL, so a rebuild on every swap is exactly what the recompile
boundary exists to prevent. Discrete lanes over assets also want **preloading**: at t=4.9 you
already know t=5.0 needs the next one.

**`Uniform` is the one that is not obviously free**, and it is not: every branch is in the
compiled program whether it runs or not, so the driver allocates registers for the worst of
them and the cheapest branch pays for the most expensive one. That is why it is not the
default and why a `Code` option stays the ordinary case. It is right exactly where a rebuild
is the thing that must not happen. The mixer is that case — a crossfade method changed in the
middle of a set — and it is silvia's answer too: `mainMixer.js` uploads `u_crossfadeMethod` as
a uniform, and `sliderule.js` is the general form, a select registered as an `int` uniform
rather than read at codegen. `sliderule` here goes the other way: its two bases are `Code`,
because nobody changes which range a conversion is between while a set runs, and a pick that
is read at codegen leaves one multiply in the shader instead of silvia's branch over five.

**`Presentation` came out of the same question asked once more.** `audio_ports::SHOW` — how
much of an audio node to draw — reaches no shader at all and was rebuilding every Output
downstream of it. It gets the answer `SetCollapsed` already gave: the ports and the cables
are untouched, so the shader that was running still describes the graph.

**Rejected: inferring the kind from the shape.** An option with no choices *was* the asset
reference, by having nowhere else to be. That worked while there was one such option and
stopped working the moment a second kind wanted to say something about itself — and it made
`note` impossible to write, because free text meant file path everywhere it was touched. A
node declaring what its option costs is one field and no inference.

### A free-text option, not a new `Control`

**Chosen**, for Lyapunov's `sequence`: `OptionDef` gained `placeholder: Option<&'static str>`
and `validate: Option<fn(&str) -> bool>` rather than the node library gaining a new `Control`
variant for inputs. `Some(placeholder)` draws an always-open field instead of a closed list,
with `choices` offered beside it as presets a person can still type rather than the only
values `option_is_valid` accepts; `validate` is read by `ui/` for the field's border and
enforces nothing, because `parse_sequence` already answers for anything unparseable.

A `Control` was the first shape reached for — silvia's own field is exactly what an input
control is, a typed fallback with a row — and it does not fit. A `Control` lives on an
`InputDef`, which carries a `PortType`, and there is no `PortType` a string could be: nothing
downstream reads it as a field or a uniform, and the app has no wire color, no cable rule and
no compiled name for one. `NodeDef::hidden` is the one no-port `Control` list, and it draws no
row at all — a band's center frequency is edited on the scope, not the node body — where
`sequence` needs exactly the row it had. `canvas::Row` only ever comes from `inputs`,
`outputs` or **`options`**, so a value with a row and no port is an option wherever it lives;
the question was only whether a `Code` option could hold free text as well as choices, and it
already could in spirit — `Asset` does, with empty `choices` standing in for "any". Lyapunov's
`sequence` keeps its eight named choices as presets, so it needed the field on `OptionDef`
that says "and also anything `validate` is happy with," not a second option kind and not a
new port-side concept to parallel one.

### One thing is saved: the project

**Chosen.** A project is a folder — `project.ssp` and one `.ssw` per workspace — and it is
the only thing supersilvia saves. `Ctrl+S` writes the folder. A workspace leaves a project by
export and enters one by import, and those are the only two gestures that touch a file
outside it. [architecture.md](architecture.md#the-project) has the shape.

The reason is silvia's, from the other side. silvia has three half-versions of keeping a
show: a session that reloads every tab, a per-workspace quick-save that remembers where, and
a compound save — and they disagree about what is saved with what. A cross-workspace cable
survives one and not another; a MIDI map saved per node in each patch drifts between two. The
disagreement is not a bug in any one of them, it is what having three tiers *is*.

**Rejected: a per-workspace save as a second tier.** It is the obvious convenience and it is
the thing that produced the disagreement. The moment a workspace can be saved on its own,
every piece of glue — a cable to another workspace, the workspaces a shared node is on, the
id counter — has to answer "and what happens to me", and each answer is a place two files can
disagree about one show.

**Rejected: a File menu holding both.** The menu is where the tiers would become visible and
therefore permanent. Project holds opening, saving and finding the project, and the MIDI map
saved in it.

**Rejected: opening a project as an undo step**, which is what opening a file was. Undoing
your way out of one project into the previous one is not a thing anyone wants at a show, and
a graph snapshot from before the switch describes nodes the project on screen never had. Open
and New clear the history instead, and the unsaved-edits confirm is what stands where the
undo step was.

**Rejected: a project with no folder until the first save.** It puts a folder dialog in front
of someone who has not drawn anything yet, and it needs an "unsaved, nowhere" state that every
save path then has to branch on. The first launch makes `Untitled/` in the projects folder,
`supersilvia` in the documents folder, and names its whole path on the status line; Save as…
moves it wherever it should live.

**Chosen: projects in the documents folder**, `~/Documents/supersilvia` — on Linux the
documents folder `user-dirs.dirs` names, in the person's own language — with a `projects_dir`
preference for another. **Rejected: the machine's data folder**,
`~/.local/share/supersilvia/projects` on Linux and `~/Library/Application Support` on the Mac.
It is where an application keeps what it manages itself, and a project is the person's own
work: it is what they back up, zip, send and open from a file manager, and a hidden folder is
the one place a person does not look for it.

**Chosen: New project asks for a name** and makes the folder in the projects folder itself,
with **Choose location…** for anywhere else. **Rejected: a folder dialog for every new
project.** A new project is almost always one more folder in the same place, and a dialog
that asks for an empty folder makes the person make one first, in a dialog that was not
built for it.

**Chosen: the manifest's rename commits a save.** Each workspace file is written beside its
own as `<file>.<save>.tmp` and synced, the manifest is written, synced and renamed over
`project.ssp`, and only then are the workspace files renamed into place and the removed ones
deleted — by the same step Open runs first, so a save cut off anywhere opens as the last save
whole or as this one whole. **Rejected: renaming each workspace file into place before the
manifest.** Every file is then whole, but a save cut off between two of them opens as a mix
of both saves, and a shared node that moved files at that save loads twice.

### No backwards compatibility and no migration

**Chosen.** supersilvia is pre-alpha and its file format is not yet stable, so a loader that
reads yesterday's shape is code kept forever for files nobody should still depend on.
When a node, a key or a format changes, it changes: no alias, no migration, no load note and no
fixture for an older build's file. Every field is required except the ones a later step will
add. An old file opens with whatever still matches, and anything else takes the loader's plain
warning for an unknown node kind, control, option or port, as a typo in a hand-written file
does. **Rejected: tables of old names and a note per dropped speed**, which the time model's
first builds carried — each was a rule to keep true against every later change, for files
nobody else holds.

### A workspace is a view, and a node is on a set of them

**Chosen.** One `Graph`, one `NodeId` space, and a node carries `workspaces: BTreeSet<WorkspaceId>`
beside its `pos`. A node on two workspaces is *one node*: one position, one set of controls,
one place in the shader. Every tab shows every node on it, an export carries every node on
it, and deleting a workspace deletes a node only when that workspace was its last — all three
follow from the set rather than being written three times.

**Rejected: one home per node**, with a workspace owning what it holds. It makes "show this
analyzer on both tabs" a copy or an instance, and the moment there are two of anything the
question of which one the shader compiles has to be answered forever.

**Rejected: an explicit `home` field** beside the set, to say which file writes a shared node.
The project order already answers it — a node is written by the first workspace in project
order among its set — and storing the answer gives the project a second opinion about a
question it can derive, which is a thing that drifts. Reordering the workspaces moves a shared
node between files at the next save, and Save rewrites every file anyway.

**Rejected: a position per workspace.** It would make visibility a map rather than a set and
cost every node a second `pos`, to express something silvia does not do: drag a node on tab A
and it moves on tab B, because it is the same node.

### A duplicate workspace is a variation, not a second view

**Chosen.** Duplicate on a tab's menu or a workspace card copies every node on the workspace
onto a new one beside it, `<name> copy`, as one `Command::DuplicateWorkspace` and so one undo
step. **Every node is copied as a plain node on the copy alone, shared or not**, as an export
writes it: a duplicate is the thing you change without changing the original, and a node still
shared with the original would be the original. The cables between the copied nodes come with
them, as a paste's do.

**Chosen: a cable from a node elsewhere comes too; a cable to a node elsewhere does not.** A
cable arriving from outside — a Master Gear on another tab, a source shown somewhere else — is
a second cable out of the same output, so the copy reads the clock and the picture the
original reads and plays as the original plays. A cable leaving for an input elsewhere cannot
come: that input already has the original's cable, and taking it would change the original's
patch from a gesture that promised not to. MIDI bindings and deck claims name the original's
nodes and stay with them.

**Rejected: the paste's rule, only the cables inside.** It is the right rule for a paste, which
lands a piece of graph wherever it is put; a duplicated workspace is a whole patch, and one cut
off from the clock it ran on is a copy that does not play. **Rejected: showing the shared nodes
on the copy as well**, which would make a knob turned on the copy move the original.

### A drop lands where it is pointed, and a paste brings its files

**Chosen: a dropped file lands on the workspace showing, under the pointer.** **Rejected: the
first workspace in project order at a staggered corner**, which can be a closed workspace and is
never where the hand is. On Wayland a drag from another app reaches the window as a data
offer, not as pointer motion, and winit hears neither: `platform::filedrop` listens for the
offer itself and gives its position to egui as the pointer's, so it lands under the hand there
too. Off the canvas it lands at the centre of the view. **On the project tab it is an asset and nothing else**, since there is no canvas
there to put a node on. **A held file says what it will make**, in one line over an outline,
because a drop that lands somewhere unexpected is a node to find and delete.

**Chosen: a paste into another project copies its files in, and the clip carries the folder it
came from.** A node names its file by an `assets/…` reference relative to its project, and the
clipboard outlives Open. **Rejected: the reference copied verbatim**, which names a file the new
project does not have, or another file under the same name. `Clip::from` is the folder the copy was taken in,
recorded on the clip so that `Command::Paste` carries it and a paste reads nothing from the
session. **Rejected: writing every reference as an absolute path at the copy**, which cannot
be told apart afterwards from an absolute path somebody typed into the option on purpose, which
nothing forces into `assets/`. **Rejected: remembering the folder on the clipboard**, beside the
anchor, which the command does not carry.

### Closed means suspended, not unloaded

**Chosen.** A closed workspace has no tab and is otherwise unchanged: its nodes stay in the
graph with their state, and a node shown on no open workspace is
[suspended](architecture.md#open-and-closed-and-what-suspension-means) — it does not tick, its
Output is not drawn, and the renderer keeps everything it had. Its time runs on: the tick it
wakes on hands it the whole distance the transport moved, as a jump.

**Rejected: unloading a closed workspace to disk.** It is the obvious reading of "closed" and
it is wrong three ways. Its cross-workspace cables dangle instead of being drawable on demand;
either Close writes to disk with no Save gesture, or an unsaved edit is lost on Close, or
there is a prompt; and undo across a Close is unanswerable, because a snapshot from before it
holds nodes the project says are on disk. Keeping them in memory dissolves all three, and it
is why Close needs no confirm: nothing is lost, because nothing leaves the project.

The cost is memory in the undo ring, and it is why the ring is
[capped by bytes](#undo-by-snapshot-not-by-inverse-commands) rather than by depth alone.

### The project tab is a pinned first tab

**Chosen.** What the project holds — its workspaces and its assets — is a page, reachable by
`Ctrl`+1, and it is what an empty project shows.

**Rejected: a modal.** A modal is chrome that stops the instrument, and this is a tool for a
stage. **Rejected: a pane beside the canvas.** It costs canvas width every frame for a list
that is looked at between songs. A tab costs nothing while it is not showing, is big enough
for thumbnails, and makes "the page you land on is the page that says what the project holds"
true without a second mechanism.

It is the one tab that is not a workspace and cannot be closed, and `ui/project.rs` returns
`Vec<ProjectAction>` the way every other surface does.

### Project and Workspace menus, not File

**Chosen.** `Project` holds New, Open, Recent, Save, Save as… and Quit; `Workspace` holds
Rename, Close, Export…, Layout and Auto-arrange, and exists only while a workspace is active.

**Rejected: a File menu holding both.** The menu is where a tier becomes visible and therefore
permanent — see [one thing is saved](#one-thing-is-saved-the-project). Two menus about the two
things there are say what the model is every time somebody opens one.

**Layout and Auto-arrange are under Workspace, not View and Edit.** A layout mode is that
workspace's data, saved in its file, and an arrange is an edit of one canvas; View was only
ever holding them because there was one workspace.

### Import at the list, export from the thing

**Chosen.** One rule places every import and export: **a thing enters a project at the list it
joins, and leaves from the thing.** Import… is at the top of the project tab's Workspaces and
Assets sections, and dropping a file on the window is the same import — a `.ssw` is a
workspace, anything else is media. Export is on the workspace's card and on the Workspace
menu, and on each asset's card.

**Rejected: both on one menu.** An import has no single object to act on, so it would be a
menu entry that asks *where does this go* — and the answer is always "the list you can see".
An export does have one, and putting it on a menu away from the thing means naming the thing
twice.

The report an export leaves is part of the gesture, not a nicety: the glue is about *this*
rig, so a cable to another workspace and the other workspaces a shared node is on do not
travel, and saying which is the difference between a decision and a silence.

### Assets and the cache live inside the project

**Chosen.** `assets/` is the project's media and every file that reaches a node is copied into
it; `cache/` beside it holds what was derived — a transcode, a decoded soundtrack. Both are in
the folder, and [architecture.md](architecture.md#assets) has the shape.

**Rejected: referencing media where it sits.** A project folder that can be moved, zipped and
sent is worth more than the disk a copy costs, and the alternative is a folder of references
into somebody's home directory that resolve on exactly one machine.

**Rejected: `cache/` in `$XDG_CACHE_HOME`.** A transcode is minutes of hardware encoding; a
folder that carries its own plays on another machine at once, and the cost is disk in a folder
its owner already has. It is still a cache: made on demand, listed nowhere, never written by
Save, deletable for the price of a re-import.

**Chosen: Save as leaves `cache/` behind**, and copies `renders/` and `snaps/`, which are the
person's work. **Rejected: copying the cache with the fork.** The fork is on the same machine
as its original, so the copy is gigabytes of transcodes made again only if a clip in it plays.

**Rejected: Save deleting an unreferenced asset.** Save deletes nothing under `assets/` or
`cache/` but a painting file it wrote itself and no longer names — [A painting is saved with
the project](#a-painting-is-saved-with-the-project). Removing media is a gesture on the project
tab with a refusal attached, because a save that quietly binned a file somebody was about to
point a node at is unrecoverable.

### Not the undo history in the project folder, and `.autosave/` instead

**Chosen: not built, and not without asking what it is for.** The folder is where an undo
history would go if there were one, and `App::history` — the command log that already exists
beside the snapshot ring — is the shape it would take: a base snapshot plus the log since.

It is not built because undoing yesterday's edits is rare and a crash wants an autosave rather
than an undo. It is recorded so nobody rediscovers the folder as the place and builds it
without asking what it is for.

**Chosen: `.autosave/`, built.** A crash, a kill, a power cut or a lost GPU used to lose
everything since the last Save. Now, while there are unsaved edits, the project is saved
into `.autosave/` inside its folder — a project of its own, written by `Project::save`
itself, so an autosave lands whole or not at all for the same reason a save does — at most
once every **thirty seconds**, on a thread of its own; opening a project whose autosave is
newer and different asks whether to recover it
([architecture.md](architecture.md#saving-and-opening), [ui.md](ui.md#the-menu-bar)).

- **Thirty seconds** bounds a crash's cost at half a minute of patching, and costs a write of
  a few kilobytes of JSON, synced, twice a minute at most — and nothing at all while the
  document is clean or has not changed since the last write.
- **Rejected: writing on every edit.** A drag is an edit a frame, and a synced write a frame
  is disk traffic for nothing a crash would miss.
- **Rejected: autosaving over the project's own files.** Save is the person's word that this
  is the patch; a crash mid-experiment must not make the experiment the saved project.
- **Rejected: writing on the frame thread.** The frame records an `Arc` of the graph and
  hands the rest to a thread named `autosave`, because an `fsync` can take as long as the
  disk likes, and the frame and the synth thread are not allowed to find out how long.
- **Recover leaves the document unsaved**, and the autosave on disk until a Save: recovering
  is not the person's word that it is the patch either.

### "Workspace", not "tab"

**Chosen.** A tab is where an *open* workspace is shown. A closed workspace has no tab and is
still in the project, so "the project tab lists every workspace, open or not" is a sentence
that does not survive with "tab" in it. The menu, the commands, the file name and the prose
all say workspace.

The word **patch** went with the same reasoning one level up: it named a file that held one
graph, and there is no such file any more. What is wired up is a workspace; what holds
workspaces is a project.

### The image crate is for the GIF and nothing else

**Chosen: PNG in and out stays GStreamer's.** `video/png.rs` writes a thumbnail with
`appsrc ! videoconvert ! pngenc ! filesink` and reads one back with
`filesrc ! pngdec ! videoconvert ! appsink`. Two short pipelines, in the module that already
talks to GStreamer. It lives in `video/` because that is where GStreamer lives, and it hands
out RGBA bytes rather than a texture, so `video/` still takes no graphical dependency.

**Chosen: `image` for `imagegif`, because GStreamer cannot decode a GIF.** It decodes a PNG, a
JPEG and a WebP — `pngdec`, `jpegdec`, `webpdec` — and for the animated GIF there is nothing:
`gdkpixbufdec`'s caps list png, tiff, bmp, tga, pcx, svg and the portable formats and not
`image/gif`, and `avdec_gif` needs the libav plugin, which is not installed on any machine this
has run on. `decodebin` answers `not-linked` and the pipeline never prerolls. So the crate is
in the shipping binary for the one format that cannot be got any other way, and it is the
version `egui_kittest` already pinned for `examples/node_shots`, so the lock does not move.

**Rejected: a GIF decoder of our own.** LZW, the frame disposal rules and a canvas to
composite onto, for a format the crate in the lock already reads — and `nodes/` would then
hold a decoder rather than a node.

### Shader links go to threads of ours, not to the synth's

**Chosen.** Pipelines are created on `linker` threads (`render/link.rs`), and a
simulation's kernels on `sim-linker`: `Programs::set_shader` sends the module and returns,
`Programs::poll` is a `try_recv` once a tick, and the old program renders until the new one
lands. wgpu has no asynchronous pipeline creation on native — `create_render_pipeline`
returns once the driver has compiled to machine code, tens to hundreds of milliseconds — and a
`Device` is `Send + Sync`, so a thread of ours is the whole of what it takes. A link's errors
come back through a validation error scope around the creation, which is thread-local and so
holds that link's errors and nothing the synth is doing on the same device.

**The trap it is built around:** a creation on the synth thread is a stalled tick, and on the
frame thread a stalled window. So no shader module or pipeline is made on either but at
startup, where the mixer's, the viewers' and the renderer's own stages are made once.

**Rejected: a pipeline cache of our own.** Mesa's Vulkan drivers keep a disk cache of compiled
shaders, so a second run of a project already links from it; and wgpu's
`create_pipeline_cache` is `unsafe`, taking bytes it cannot validate. Metal keeps its own.

**Open, and measured at step 7 of `proposals/wgpu.md`:** whether wgpu-core holds a lock during
a pipeline creation that the synth's recording or submission needs, which a recompile storm on
a heavy visible tab would show as late ticks.

### A workspace chooses between a plane and a strip

**Chosen.** Two layout modes, saved with the workspace: `Canvas` is the pan-and-zoom plane and
`Linear` is silvia's fixed-scale strip, with the wheel scrolling along the dataflow. See
[ui.md](ui.md) for what each does.

**Why silvia had no zoom, which is the argument for keeping the strip.** A node looks right
at one scale — this codebase concedes it in the quantized font sizes and in the three
`t.zoom` thresholds at which the node widget drops its icon, its title and its controls.
Moving something on a plane costs zoom-out, pan, zoom-in. A mouse wheel is one axis, and a
left-to-right dataflow graph is a one-dimensional document that a plane presents as two.
And a bounded strip cannot lose a node: scroll to the end and you have seen everything.

**Rejected: making Linear the only mode.** The plane is better at one thing — seeing the
whole shape of a large graph at once — and a graph mid-experiment is not always a tidy chain.

**Rejected: a Linear workspace with no manual y at all.** If auto-arrange were the only thing
that placed nodes, position would be derived and a linear workspace would be a column list
rather than a strip. A cleaner model and a worse instrument: the mode constrains navigation,
and taking authorship with it is not a trade anyone asked for.

**Rejected: a preference rather than document data.** Two people opening the same workspace
should see the arrangement its author made. What is a preference is the default mode for a
new workspace, which is `default_layout` — see [the Preferences
window](ui.md#the-preferences-window).

**Rejected: a rail of packed cells rather than a minimap.** The first build laid one
fixed-size cell per node along a bar, spread ties apart so a column did not collapse, and
drew each edge as its own arc — forward above the cells, backward below. It was legible, and
it was a *second description of the graph*: a layout algorithm and a cable renderer that
could disagree with the real ones, some 220 lines of them. The minimap is the canvas at a
smaller transform, reusing the canvas's own layout and `cable::Curve`, and a feedback loop
bows downward on it because it bows downward on the canvas. Nothing was written to make that
true.

**The general rule it is worth keeping:** a view that redraws the graph its own way will
drift from the one people edit. Scale the transform, not the drawing.

**Kept from that version:** the node's own emoji, because it is already in `NodeDef::icon`,
already on the node header, and is the first thing zoom discards — the mark you scan for is
the mark you then see.

### Three ways to the library, and the background right-click is one of them

**Chosen.** One menu bar entry to reach any node is two clicks, both of them a long way from
where the graph is. silvia answers this with three gestures — a categorized start menu, a
fuzzy-searched list at the pointer, and the same list as a quake bar over the middle of the
canvas — and all three are here.

**The quick menu and the quake bar are one thing shown twice**, as they are in silvia, where
they are literally one DOM node. The only difference is where the panel sits and where the
node lands, so `browse::Browser` carries both as fields and nothing else knows there are two
gestures.

**The background's right-click is the selection's menu while several nodes are selected,
and the browser otherwise.** The menu was on the background first, because that is where
the pointer is when no single node is the target; then it was taken off entirely, because it
cost the gesture a node editor is asked for constantly — *put a node here* — and every node
in a selection carries the menu on its whole body anyway. That went too far: with a band of
nodes selected, a right-click anywhere in the editor means *these*, and reaching for one
particular node in the band to get their menu is a detour. So the count decides. One or no
node selected, the canvas offers a node, and one node's menu is on the node; more than one,
the canvas offers their menu.

**Rejected: `menu_button` for the Nodes menu.** egui opens a submenu on hover with no delay
and closes it with no grace period, which is the flashing-and-narrow-valleys behavior that
`ui/start.rs`'s two timers exist to prevent. What is lost by not using it is a hundred lines
we now own; what is gained is the only part of a menu anyone feels.

**Rejected: the library as a menu bar entry.** It was one for a while and it is a start
button now, in the canvas's bottom-left corner with the menu standing on it. The menu bar is
for what the *app* does — files, undo, the view — and the library is what the instrument is
made of; it belongs where the hand already is, at the corner of the canvas rather than at the
top of the window. It is also silvia's, and the gesture people have.

**Rejected: reading the pointer's position instead of its crossings.** egui hands you
`hovered()`, a state, and a menu built on it gets two faults that feel like one: a submenu
opened by the keyboard closes itself 300 ms later because the pointer is not over anything,
and a pointer parked over the menu drags the selection back under itself every frame, so the
arrows cannot move. A browser's `mouseenter`/`mouseleave`/`mouseover` are *transitions*, a
still pointer says nothing at all, and silvia's whole menu is built on that. `Over` and
`crossings` are that model, and every timer and selection change hangs off them.

**Rejected: leaving the keyboard on the category when a hover opens a submenu.** Windows does
that, and it was tried here on the argument that it is what someone alternating between mouse
and keys expects. silvia's `onSubmenuShown` moves the keyboard *into* the submenu with
nothing selected, which is better than the argument: the pointer has already said which
category, so `Down` should walk what it opened rather than walk away from it. Going in by
`Right` still selects the first entry, because a key that goes in has said which way.

### The canvas reads its own keys, last

**Chosen.** Delete, Backspace, `Ctrl`+D, `Ctrl`+A, `Escape`, Home and `.` are read inside
`ui::show`, after every control, popup and menu on the canvas has drawn, rather than in
`App` beside `H`, `F` and `F8`. What they must yield to is only known inside the pass: a
number control under the pointer reads its keys on hover, not focus, and a control's popup,
the browser and the conversion menu are the canvas's own state. Read in `App` before the
canvas, `Delete` over a hovered number would delete its node; read there after it, the guard
would need all of that handed back out. The keys are still constants in `menu::keys`, the one
table a shortcuts window can be built from.

**`Escape` mid-drag drops the drag's step**, as it drops a scrub's, rather than moving the
nodes back with a second `MoveNodes`. The move back would join the drag's coalesced step and
leave a step in the ring that restores nothing, with an edit serial the title reads as unsaved
work.

**Rejected: `Escape` meaning nothing with nothing in hand.** It clears the selection, which is
what a canvas's `Escape` does elsewhere, and the picture windows' `Escape` is untouched: a
picture window is a surface of its own, and its keys never reach the editor.

**A frame never zooms past actual size.** Fitting one small node to the window would draw it
three times over; a frame is for finding where things are. Home and `.` because `F` and `H`
are taken, and both are what other node editors put there.

### A loose cable lists what takes it; a node on a cable splices in

**Chosen: the browser, narrowed, for a cable let go in the open.** The gesture asks *what node
goes here?*, which is the library's question, so it gets the library — the browser, searched
the same way — narrowed to the kinds that can take the cable. That is a registry query, where
[the conversion menu](ui.md#the-conversion-menu) is a curated table, and the two are
not in tension: the conversion menu answers *what should this become?* on a port the cable
cannot reach, and a handful of castings is the right answer to that; a cable in the open has
no target, and every kind that fits is.

**The list is put to `Graph::can_connect`, not to a second rule.** A fresh node of each kind
on a copy of the graph is asked, so a dual node's effective types and an action against data
are judged as the cable will be. A copy shares every node it does not write, so it costs a
node per kind, once, on the release.

**Only one node with no cables splices into a cable.** A selection has no one input and one
output to put in a cable, and a node already patched would be rewired by a drop meant to move
it. The lit cable wears the selection's color rather than the accent, which the spec keeps for
Rendering. The splice is a step after the drag's own move, so one undo takes the node out of
the cable and leaves it where it was dropped.

**Rejected: the move and the splice as one step.** The move is a coalesced gesture that is
already on the ring by the frame the hand opens; folding the splice into it would make the
splice the only command that rewrites a step it did not open.

### A monitor is one number, and zero is off

**Chosen.** Every audio source has a `monitor` control: zero is off, anything above it is the
level, and at zero nothing is queued and no output device is opened. The monitor plays the
samples the analyzer consumed, so speed, direction, scrubbing and jumps all sound like what
they are without a second playback pipeline. [media.md](media.md#monitoring) has the shape.

**Rejected: an on/off option and a separate level.** A node that is already too tall does not
get two rows for one idea, and a level on a monitor is not mixing — it is not deafening
yourself while placing a threshold.

**Rejected: pan.** The monitor sends the same sample to every channel. A monitor that panned
would be the beginning of a second mixer, which is
[affordances.md](../proposals/affordances.md)'s F4 and not this.

**Rejected: the microphone not monitoring.** It is the case that howls, and it is also the
only way to check a microphone is live without patching a picture to it. Zero by default
everywhere is the answer to the howl.

**The output device is the system default.** By [the tier
test](architecture.md#three-tiers-of-saved-state) a chosen device is project data, and no
gesture chooses one; the row is in the table so nobody makes it a preference.

### Band shaping belongs to the graph

**Chosen.** A band publishes its own level and nothing else. Smoothing it is `slew`, and an
exciter — the departure from a running median, expanded and soft-clipped — is a node.

**Rejected: the exciter inside the analyzer**, which is where it started, with `react` and
`smooth` as hidden per-band controls. It made every audio source pay for shaping whether or
not it was wanted, it could not be metered or reordered, and the same reshaping could not be
reached for anywhere else in the graph. As a node it composes with everything, and the two
controls it needed stop being six rows of tuning on a node that is already tall.

**Rejected: a per-band gain.** A gain on a band scales the number and moves nothing else, so
the meter and the threshold had to divide it back out to stay under the finger that placed
them — a value that has to be undone everywhere it is read is not paying for itself. What a
band is for is *where* it listens and *how narrowly*; scaling is a `multiply`.

### The scope: a two-axis handle, and a row of ticks for the rest

**Chosen.** Each band's handle on the spectrum is a two-dimensional dragger — X is where the
band listens, Y is how narrowly — over columns tinted by the band that covers them. The
values are `NodeDef::hidden` controls, and **`uniforms`, `events` and `scope` are three
options** — silvia's own, in silvia's order — each deciding whether that part of the node is
drawn. The first two are ticks sharing one row, because what they hide is a run of port
*rows*; `scope` is the heading over the region it hides, because that region can carry one.
See [a region declares its own heading](#a-region-declares-its-own-heading-its-own-width-and-its-own-hit-rect).

**Rejected: the parametric EQ convention**, X for frequency, Y for gain and the wheel for Q,
with each band drawn as a bell as tall as its gain. It was built and removed. The convention
reads as an equalizer, which is a thing that *changes* a sound; this plot measures one, so a
handle that moves up and down without changing what is heard is the wrong picture — and it
cost the axis that Q, which the band really does have, was being read from.

**Rejected: silvia's three columns of nine `s-number`s.** Six number fields say what one
drag says, on a node that is already tall.

**Reversed: one `show` select instead of three ticks.** It was `all` / `compact` / `ports`
for a year, on the argument that an option is already document data and three checkboxes
would be three more rows. Half of that argument was right and is kept — a tick **is** an
option, stored in `Node::options`, undoable, and read off the `Node` by `canvas` without
consulting the registry. The other half was wrong twice over. The three answers are not a
ladder: `compact` could not say "the events and the scope, but not the uniforms", and no
fourth value would have helped, because what is being asked is three independent questions.
And three ticks are not three rows — `Row::Checks` is one row holding all of them, which is
what silvia does and what the select cost anyway. silvia's checkmarks won
after using both.

Hiding uniform number and event rows still needs no new bit on a port: a uniform output is a
`UniformNumber` and an event output is an `Action`, and layout can already see both. What is
new on the `Node` is `checks` — which of its options are ticks — and `headings` beside it, for
the same reason `regions` is there: layout has to shape a node without reaching into the
registry.

**Rejected: a bit on `Node` saying what someone hid.** It would be set from the definition and
never change; what someone hides is not a property of the node kind, and would need a writer,
a command and a place in the file that the option already is.

**Rejected, with conditions: per-band history lanes.** Three lanes over two seconds were
built and removed: a lane starts empty and fills from the left, so the first two seconds are a
wedge meaning only *this has not been running long*, and a threshold drawn across it has no
relationship to the level a hand is placing. If lanes come back, three things have to be true
and none was: the history starts at the current value rather than at zero; the threshold is
drawn across the lane, so near-misses are visible; and they are big enough to read — silvia
gives a lane 32 points in a node 320 wide, and below that they are texture. They would go
behind `show`.

**Rejected: a held peak on the meter**, falling from full scale over two seconds, which is
what stood in for those lanes. It measured a band's own recent loudness, which was worth
knowing while the band was the exciter's output and a threshold was placed against beat-sized
departures. Against a plain level it says only that the track has been loud, which the bar
already says a moment earlier, and it is a second mark on a meter whose whole argument is that
the control and the data it measures are one object.

### A gesture is what coalesces, and the pointer release ends it

**Chosen.** What identifies a run of commands as one undo step is the command's *shape* — the
same node set for a move, the same control for a write — and a gesture ends when the pointer
comes up. Anything one gesture writes at once is one command.

**Rejected: keying coalescing off the last command alone**, which is what it was. Two failures
came out of the same gap. A handle carrying two axes emitted a command per axis, so the keys
alternated, nothing ever matched the one before it, and a second of dragging a band on the
audio scope filled the whole undo ring. And with no release boundary, two scrubs of one
control collapsed into a single step, so one undo walked back an edit the hand had already
finished and let go of.

**Rejected: collecting every control a gesture touches into a set.** It fixes the flood, but
only while a pointer is what is driving: `App::apply` is also the test harness's and the
bus's, and there a run of writes to different controls would silently become one step. The
bus has to mean the same thing with no pointer in the room, so what belongs together is said
in the command instead.

### The select is a select, and it is the width of its value

**Chosen.** One element for every option row: `bg_interactive` at `RADIUS_SM`, a painted
chevron, a border carrying all three states, and a box the width of its content grown
right-to-left up to whatever the label leaves it. The row is 24 points.

**Rejected: the fixed 100x25 box it was.** It was the s-number's geometry borrowed for
something that is not an s-number, and it lost on both sides at once: `all` reserved a hundred
points to say three characters, while a file path was cut to a length chosen before anyone
knew how wide the node was. With no chevron and no hover or open state it also read as a
button — which is what it was, since nothing about it said a list would appear.

**Rejected: a chevron glyph.** `▾` is exactly the kind of codepoint a fallback face renders as
`◻`, and it would do it silently, on whichever machine had the wrong font. Chrome is drawn.

**Rejected: the wheel stepping through choices**, which the number control does and which
would be quick for `loop` and `size`. An option rebuilds the shader where a number sets a
uniform, so a notch caught while zooming the canvas across a node is a recompile mid-show. The
number control can afford that gesture because its mistake is a value that scrubs back.

**Departed from the handoff on density.** `.node-option` is `padding: 0.4rem 1rem` around a
~22 point control, which is the 34 points this used to be. An option is set once and then read
— a `video` node has four of them, and they were a fifth of its height.

### A poster is a frame out of the file

**Chosen.** Every asset gets one decoded frame written into `cache/` as a PNG, drawn by the
file picker and the project tab alike.

**Rejected: an icon per file type**, which is what the cards had. Three clips off the same
camera are three identical film icons, and *which clip is this* is the only question the
picker exists to answer.

**Rejected: decoding the frame on demand, in the widget.** A GStreamer pipeline inside a
paint is a stalled picture. It goes on a worker, one at a time, and the file it writes is
why it happens once per clip rather than once per session.

**Rejected: taking the poster from the transcode.** The cache entry is made when a node first
plays a file, so keying off it would mean the pictures showing up only for clips already in
use — and the picker is for the ones that are not.

### The file button offers the project before it offers the file system

**Chosen.** Clicking an `Asset` option lists the project's own `assets/` filtered to what
the option accepts, with `Import a file…` at the bottom. `OptionDef::accepts` is one value
naming both the dialog's filter and what the picker offers, so neither route can offer a file
the other would refuse. A project holding nothing of the kind skips the menu and opens the
dialog, because an empty list in front of the only gesture available is a click spent saying
nothing.

**Rejected: the file dialog as the only way in**, which is what it was. Everything a node can
play has already been copied into `assets/` — that is the point of the folder — so the second
time a clip is wanted the dialog means navigating into a folder the project owns to find a
copy it made itself, and `import_asset` then fingerprints it to conclude it already has it.

**Rejected: silvia's two buttons**, Replace… beside an Assets browser. Two buttons is two
rows of chrome on a node body 200 points wide, and the distinction they draw — *the file
system* against *the library* — is one the person does not have to care about: they want a
clip, and where it lives is the app's problem. One button whose first offer is the library
says the same thing in one place.

**Rejected: a global asset library**, which is where silvia's `asset://` paths point. Assets
here belong to the project, because [the folder is the whole
show](architecture.md#the-project) and a rig that plays only on the machine that imported its
clips is not one that can be handed to anybody.

### One preferences store, ours, geometry included

**Chosen.** `preferences.json` in the machine's config folder, `#[serde(default)]`, with the
window geometry, the UI zoom and the place of each of our own windows in it, and eframe's
`persistence` feature off. [ui.md](ui.md#preferences) has the file and its four rules.

**Rejected: eframe's `persistence` feature.** It gives the window, the zoom and every
`egui::Window`'s place nearly free, in opaque RON in a place nobody looks — `app.ron` — and
it is a second store: while it was on, its window geometry overrode ours at start-up and it
was the only place the zoom and the windows' places were kept. A file of our own beside it is
silvia's five stores at a smaller scale. One store per tier is the point of [the tier
test](architecture.md#three-tiers-of-saved-state).

**Rejected: a preference as a `Command`.** It would enter the undo history, and undoing a
window size is not a thing anyone wants.

### A `?` on the header, rather than the header itself as the tooltip

**Chosen.** `NodeDef::tooltip` is a sentence per node, written for all of them, and the
browser's rows were the only place it was read. A mark says the sentence exists; the header
alone would not. [ui.md](ui.md#acting-on-a-selection) has the geometry.

**Rejected: hanging the tooltip off the header's own hover text.** It costs no pixels, and it
is invisible: nobody rests a pointer on a title bar to see whether something appears, and a
drag handle that also explains itself makes every grab a race against a popup. The header's
hover text stays what it is — the node's name.

**Rejected: a click that pins the tooltip open.** It needs state of its own, and egui's
tooltip is a hover lifetime; the mark is a hover target and clicking it does nothing.

### The canvas asks what is under the pointer, rather than each surface clearing the wheel

**Chosen.** One gate in `ui::show`: the canvas reads the wheel only where the canvas
background response contains the pointer, which is egui's hit test and therefore already
knows about layers, popups, modals and the panels beside the canvas.
[ui.md](ui.md#interaction-rules) has the rule.

**Rejected: every scroll surface taking the delta with `input_mut` and clearing it**, the way
`ui/number.rs` and `ui/scope.rs` do for a control under the pointer. It is a patch per
surface: the Nodes menu, the browser, the color picker, the select popup and the range editor
each need their own, and whichever surface is drawn next needs one nobody remembers to write.
It also cannot be written where the fault is — a `ScrollArea` that has reached the end of its
list has nothing to clear, and the strip's momentum reads the frame's `MouseWheel` events,
which clearing the smoothed delta does not touch. The two clears that remain are a control
consuming a notch it acted on, which is a different question from which surface the wheel
belongs to.

### The on-node render is placed by `ui/` and clipped by egui

**Chosen.** An egui paint callback clips to a rectangle, and the node's bottom corners are
round, so the picture is held off them by the inset that puts its square corners inside the
arc, and it goes into a shape slot reserved inside the node's own paint order so the border
is painted over it. Both are geometry, and geometry is `ui/`'s.
[ui.md](ui.md#preview-and-on-node-render) has the numbers.

**Rejected: a stencil or a rounded mask inside `render/`.** It is the exact shape, and it
would mean `render/` knowing a node has a corner radius, which side of it the picture sits
on, and what the border is worth. `render/` is handed a rect and a texture and knows nothing
else about the canvas; making it the second place node geometry lives would be paid for on
every change to the node's shape.

**Rejected: masking the corners in the canvas's background color.** It keeps the picture
flush, and it paints over whatever is actually behind the node there — a cable running under
the corner, a node below it, a grid dot. A mask that has to guess what it is covering is a
lie about what is behind it, where an inset is only smaller.

### A source draws its own picture, and any picture can leave for a window

**Chosen.** A node kind may name one output as the picture it can draw on itself
(a `nodes::Region::Preview` region; `video`'s `frame`, `maininput`'s, and the two CPU
simulations below), behind a **Preview** heading.
silvia shows the clip playing in the node body, through a `<video controls>` element, and
what that element was really giving a person was *this clip, here, while the patch is being
built* — worth having whether or not an Output is wired up yet. The band is a fixed 16:9 box
letterboxed rather than the Output's exact-aspect slot, because a clip's shape is whatever
was imported and the node is laid out long before a frame has been decoded.

**And both kinds of picture carry two marks**, a pop-out and a fullscreen, which is the other
half of what `<video controls>` was doing. They open one **picture window** per picture — a
Wayland surface of our own on the pictures thread, which is the one kind of window the mix
opens into too.

**Rejected: one mark that pops out, and `F` inside the window for fullscreen.** It was built
that way first. Wanting a picture beside the graph and wanting it filling a screen are two
different things to want, and a single button has to guess which; the second is the one that
matters during a show, and it should not need a window to be opened and then keyed.

**The picture takes the foot of the body, and the scope stacks above it.** It was the other
way round for a day — the argument being that the scope's meters run to the edge and should
be the flush band. The picture is the thing being looked at and the meters are instrumentation
over it, which is the order a player puts them in, and a picture wedged between rows of ports
and a plot reads as neither.

**The strip on a picture is conventional on purpose.** A scrubber, a speaker, a short volume:
nothing on it is a new idea, because a hand that has used a video player already knows all
three, and what is genuinely new on these nodes is the ports. What the strip edits is not new
either — volume is the `monitor` control, so the strip is a second editor for an address the
node already has, the way a meter's threshold square is for its level.

**Rejected: a `position` control for the scrubber to write.** Where a clip is playing from is
the node's own state, not the document's: a graph reopened tomorrow should start where its
`speed` says, not where a hand last dropped a scrubber, and a value written every frame by a
drag would be an undo history of nothing but scrubbing. So a scrub is a *request* —
`TickContext::seek`, one-shot, read by the next tick and cleared — and the node moves its own
position so the sum with `position` lands there, and plays on. While a cable drives `position`
the bar is a readout of that sum, and goes inert and says so.

**Rejected: keeping the window's title bar.** Decorations are movable and resizable for free,
which is the whole of what they were giving. They are also a strip of desktop chrome in the
middle of a show. The window keeps both affordances by hand — drag the picture, take hold of any
edge, with the cursor saying which — at the cost of one real bug, which
[ui.md](ui.md#pictures-in-windows-of-their-own) names: a compositor-driven move eats the
release, so both the pointer state and the *drag* state have to be cleared on the way out or
every second drag is dead.

**Rejected: a grip glyph painted in a picture window's corner.** With no decorations there
is nothing on screen saying the corner resizes, and three hairlines in it were the obvious
answer. They are also the one thing a picture window was built not to have: paint on the
show. What discoverability the window needs it gets from the **cursor**, which changes over
every edge and costs the picture not one pixel.

**Rejected: marks that are always on the picture, dim at rest.** Built that way first, and
what it is is furniture on the render. A player shows nothing until a hand comes near, which
is also the answer to "how does a picture stay a picture on a second screen". They stay named
in the accessibility tree while unpainted, so nothing that reads this app by name loses
them.

---

### A simulation you cannot watch is a simulation you cannot play

**Chosen.** `cellularautomata` and `brickgame` draw their own state on the node, each through
the same `nodes::Region::Preview` region a clip uses: `cells` for the one, `field` for the
other, under the standard **Preview** heading. Both already publish that state as a `Texture`
output, and the renderer keys a picture by *port*, so the whole of the change is a `regions`
line and a tick in the options — nothing new in `render/`, and no second upload.

The reason to single these two out of the CPU library is that the state *is* the node. Every
other node answers a question you can read off a row: a counter counts, a slew ramps, and the
number on the row is the whole answer. A game and an automaton answer in a picture. Before
this, a press of `Step` or of `Paddle Left` changed nothing anybody could see until the node
was wired to an Output and that Output was put on a deck — which is three acts of faith before
the first press, and for the game it means playing blind. silvia draws a 300 square on both of
these nodes and that is where the whole of their appeal lives.

**Rejected: a square band of its own for a square field.** Both of these worlds are square and
the shared preview band is a fixed 16:9, so each sits letterboxed in it with air either side.
A second band shape would buy tighter framing and cost a region kind, a second size rule and a
node that no longer looks like the other nodes with pictures. The band is a place to watch the
thing, not a frame for hanging it.

**Not done: drawing into the grid.** silvia lets a hand drag across the automaton's square to
spray live cells into a running Life, and that is a different ask from watching — it needs a
node body that takes the pointer, which `RegionDef::claims_pointer` allows and nothing yet
uses. Watching is most of the value and none of the machinery; it is not held back waiting for
the rest.

**Chosen: the knob is `Launch Speed` and the readout is `Ball Speed`.** silvia calls both of
`brickgame`'s a Ball Speed — an input near the foot of the controls that sets how fast the
ball is launched, and an output a few rows below reporting how fast it is going now. One is a
thing you set and one is a thing you read, and a node cannot use one name for both. The knob
is what moved, because the readout is the one whose name is *about* the ball rather than about
the launch, and because silvia's own output carries that name downstream. The key under it is
still `ballSpeed`, so the label is the only thing that changed and no saved patch needs an
entry in `workspace.rs`'s rename tables.

**Chosen: `Init Threshold` says when it lands, in the node's help text.** It is read when
`Randomize` fires, not as the knob turns — [a control that rebuilds the world when it
changes](#a-fourth-port-type-uniform-numbers) is rejected, because that number may be a cable
and a cable would refill the grid at frame rate. The cost of being right is a knob that
appears to do nothing under the hand, which reads as broken, so the node's `?` names the knob
and says what it waits for. **Rejected: saying it on the label.** A port row on a 240 wide
node has room for a label and not for a sentence; *Init Threshold (on Randomize)* is twice the
width of the widest label in the library and would either be cut or collide with the control
beside it.

---

### A simulation steps on the GPU, in kernels its node writes

**The problem.** On the CPU, `slimemold` cost more than a 100 Hz tick has: 12 to 13 ms a tick
on the 21 September demo for 7,372 agents, spread over scattered reads and writes with no one
call to blame, and landing in lumps because a private timer ran a whole batch of steps on one
tick.

**Rejected: the CPU fix.** Paying the steps as the clock goes is still about 1 ms a tick on a
P-core and 1.7 on an E-core, plus the picture and a 147 KB upload every tick — the costliest
node in the library at its defaults, and a cost that grows with two knobs a performer turns up
because the picture is better for it.

**Rejected: a thread of its own.** A tick never waits, so a world on another thread would
reach the picture a tick late through a triple buffer, and its steps would be that thread's
time and not the one clock's.

**Rejected: points blended into a float target**, the classic GL Physarum. It needs a
transform-feedback pass to move the agents and a point draw to deposit them — two mechanisms
where compute is one — and a fragment per deposit.

**Chosen: compute kernels the node writes, run by the renderer.** The world — agents in a
storage buffer, the field in two `r32f` images, a counter per cell, the picture — is the
renderer's, one world per port; the rules are WGSL `Kernel` statics in the node's own file,
and the tick publishes a `Simulation` of passes. The renderer runs them after the uploads and
before the Outputs and binds the picture as an upload is bound, so nothing is read back and
nothing uploaded. The tick's own work is negligible and the GPU spends about 0.24 ms a tick
on it. `render/` still reads no registry and names no node: the runner knows the shape of the
resources and nothing of what an agent is. See [rendering.md](rendering.md#simulations).

**Chosen: the steps are owed to the one clock.** `Rate` × 30 a second, silvia's rate, with the
fraction carried from tick to tick: the world moves every tick, at the same speed whatever the
display does, and the same `dt`s make the same world, which is what an offline render needs.
Nothing else in the node keeps time.

**Chosen: arrivals counted with integer atomics on a buffer.** Many agents land on one cell in
one step, and a count is exact and the same whatever order the GPU ran them in, which is what
makes a run repeatable. On a buffer rather than an `r32ui` image, because agents crowd into the
veins they make and Intel's typed image atomics serialize under that contention: 73 µs a
dispatch against 7 on the UHD 770. **And the diffusion reads each neighborhood once**, into
`shared` memory, which took it from 132 µs a pass to 60.

**Attract turns an agent toward the stronger of its two side sensors, and Repel away**, as
silvia's do.

---

### Costs are measured, not declared

**Chosen.** The cost strip's evaluations per pixel come from a probe: the Output's own
shader with an `atomicAdd` at the top of each node function, drawn at 144 pixels and read
back a frame late. See [rendering.md](rendering.md#the-cost-probe).

**Rejected: a `taps` count on each node kind**, multiplied down the graph by the compiler.
It is cheaper and needs no second program, and it is wrong: a blur's trip count is an
option, a bloom's is an option, a phyllotaxis's and a scatter's are *controls*, so the table
would drift from the shader with every generator edited and lie the moment a knob moved.
The probe reads the program that is actually running.

**Rejected: counting in the real program.** An atomic per node evaluation at 720p is
hundreds of millions of contended atomics a frame, which would slow the very frame being
measured. The probe is the same program at a size where that is nothing.

**Not offered: GPU time per node.** A pair of timestamps measures a pass, and a node is not
a pass; it is a function inlined into one. The Output's time is exact and is shown; a node's
share of it would be an estimate dressed as a measurement. The evaluation count is the
multiplier that makes a patch fall over, and it is exact.

### A header warns on what the probe measures, not on the view

**Chosen.** silvia marks a sampling input with an amber `⚠` from a declared `samplingCost`,
before a cable is even made. supersilvia has no declared table — see the entry above — but measuring
only *behind a preference* traded that away for nothing: the person who most needs to know a
patch has multiplied is the one who has never opened View ▸ Costs. So the probe now runs for
every awake Output with a shader whether or not the view is on, and a node whose measured taps
of its own input cross 8 draws silvia's mark on its header — not a row, since the warning is
about the node's whole pull on its source, not one port — with silvia's own sentence and the
probe's measured count where silvia's had a declared one. `SynthLink::sampling_warnings` holds the
threshold; `node_widget::body` draws it one square left of the `?`, reserving that room only
on a node that has one, and its `widget_info` is `{slug}{id} sampling {taps:.1}/px` so a test
or the agent-driven layer reads it directly.

**The strip stays a preference; the reading under it does not.** The strips are not built
while the view is off — the full bar under every node is about how a person chooses to look
at a graph, which is [kept](#costs-are-measured-not-declared) — but `SynthLink::build_plan`
does not gate the probe itself on `show_costs`. `tests/costs.rs` pins
the split: the probe runs and its reading survives a toggle either way; only the strip's own
text depends on the preference.

**What the warning needs, and what it does not.** It reads the probe's counts at each call
site, and those move with controls — a blur's size, a phyllotaxis's seeds — so the probe runs
whatever the view. What does not move with a control is the probe's source,
which is a function of its Output's: so a probe is built, sent and linked only when that
source changes, not on every rebuild of the Output, of which an undo makes one per Output.
Drawing the probe less often while the view is off was weighed and left: a probe is drawn on
every tick its Output draws, and an [idle](rendering.md#which-outputs-draw) Output's counts
stand as its last draw left them.

**Rejected: leaving the probe behind the preference and declaring the header mark instead.**
That is the table the probe exists to avoid — a blur's trip count is an option, a phyllotaxis's
is a control, and a declared threshold would drift from the shader exactly as a declared taps
table would. The mark has to read the same measurement the strip does, or the two could
disagree about the same node.

### The mixer is a render target with two decks, not a node

**Chosen.** silvia has both a `mixer` node and a `MainMixer`, and the earlier plan here was to
build the node first and make the global mixer "that node with its inputs bound to the two
channels". That was the wrong way round, and the node is not built. A node lives inside an
Output's program, and an Output's program is rebuilt by every structural edit upstream of it
and by every undo and redo; the mixer exists precisely to be **the one program that never
recompiles**, so that a performer can build, undo and relink deck B while deck A is on air —
a DJ cueing the next record on the deck that is not playing. A crossfade inside a graph
cannot promise that, whatever its options cost. So the mixer is `mixer.rs` and
`render/mixer.rs`: two decks that are Outputs, one target, one program linked once, and its
eight branches selected by an `int` uniform. See [rendering.md](rendering.md#the-mixer).

**`Show on A` and `Show on B` are action inputs on Output that are also buttons.** silvia's
shape, kept because being an action input is what lets a sequencer cut between two
Outputs, which is the thing a VJ wants and which no separate mixer UI would give. An Output
has no `tick`, so `App` reads the two ports after every node has ticked, and an edge that
fired inside the tick lands inside it.

**A claim is not an edit.** Putting an Output on a deck, moving the fade and choosing a
method are playing the instrument, like a press on a button or a pan of the canvas, so none
of them is a `Command` and none enters the undo history. That is also what keeps undo — which
rebuilds every Output — from ever touching what is on air: an edit undone puts the graph
back and leaves the decks where they were, and an Output deleted and then undone comes back
*off* the deck, because putting it on was never an edit either. The decks and the settings
are the rig's, so they are in `project.ssp` beside the session and reconciled against the
graph the same way.

**On air outranks closed.** An Output on a deck, and everything feeding it, is live whatever
its workspaces' tabs are doing. The alternative — closing the live deck's tab freezes the
show — is a trap for exactly the moment nobody can afford one. **A gear read from an open tab
is live too**, with its own upstream: a clock that stops because the tab it sits on closed
stops the time of everything on the open tab that counts it. Other producers on a closed tab
still hold their last value; a gear is the one whose value is a time.

**A deck is cropped to the mix, not letterboxed into it.** silvia's rule: each deck is scaled
about its center to the mix's height, so a deck of another shape loses its sides or mirrors
them out, and the mix always fills the screen it is shown on. Letterboxing is for the *mix* into a panel
or a window, where the picture being whole matters more than the rect being full.

**Unplugging the live deck blacks the show.** An unplugged Output publishes black, and
the mixer holds nothing on its behalf. That is the DJ pulling the record off, and holding a
frame the graph no longer produces would be a picture nobody asked for — the same rule as
zero flash, from the other side.

**Rejected: the preview showing the selected Output.** It showed the mix's ingredient rather
than the mix. What the panel shows is what the audience sees; each Output still draws its
own frame on its body, and the status line still describes the selected one, because a
shader error belongs to a graph.

---

### Textures mirror-wrap outside their bounds

**Chosen.** Every sampled texture the renderer binds — an Output's own frame, a camera, a
video file, the oscilloscope — is read through `MirrorRepeat`, not `Repeat` and not
`ClampToEdge`. A sample past `[0,1]` reflects the picture back in rather than tiling it or
reading a smeared edge. This was already true of an Output's own frame, and [the mixer's
deck-fitting](rendering.md#the-mixer) already leans on it; the change is making it a rule for
every texture rather than a property one of them happened to have. silvia sets
`TEXTURE_WRAP_S/T = MIRRORED_REPEAT` on its webcam texture for the same reason, and the
principle generalises past the one node silvia applied it to: a video file, a captured
waveform, anything a shader samples outside where it was told to look gets the same answer.

`camera` and `video` had each grown their own guard for the one case this covers — `if (t.x <
0.0 || t.x > 1.0) return vec4(0.0, 0.0, 0.0, 1.0);` — painting the bars a 4:3 source leaves in
a 16:9 Output black by hand, in the shader, on every fragment outside the picture. Both guards
are gone; the sampler the renderer binds for the texture does the job for free, and does
it for every node that oversamples an input, not only the two that thought to check.

**Rejected: an `edge` option on `camera`, choosing `black` bars or `mirror`.** The proposal
that first raised this offered exactly that, defaulting to `black` so a ported patch looked
unchanged. Turned down: this is not a per-node choice, it is what a texture *is* outside its
own bounds, the same way an Output's frame already answers it without an option. A node that
wants black bars can still make them — a `mask` or a bounds check reading coordinates the
node itself computes — but the texture underneath no longer offers a second,
node-local answer to a question the renderer already settles once.

`tests/gpu_upload.rs` samples a texture built through the production upload path
(`Sources::sync`) outside `[0,1]` against distinct texels and asserts the reflected order,
rather than trusting a driver default by inspection of the sampler's descriptor alone.

**The one exception is declared on the output: a simulated world that wraps.** Mirror wrap is
usually what a picture wants at its bounds, but the cells of Cellular Automata should wrap in
the simulation and in the texture, with no mirror, and the same for the mold. Both grids are tori: a glider off the right edge comes back on the left, an
agent off the bottom at the top, and every neighborhood count and diffusion reads across
every edge. Both pictures are sampled at raw worldspace `uv`, silvia's own mapping, so the
field tiles across the world — and a *mirrored* tiling folds a seam down every second copy,
drawing a discontinuity the simulation does not have. The picture would lie about the world.
So `OutputDef::wrap` and `OutputDef::filter` are fields of a texture output, `Mirror` and
`Linear` everywhere but `cellularautomata`'s `cells` (`Repeat`, `Nearest`) and `slimemold`'s
`trail` (`Repeat`) — which is what silvia asks for those two textures and nothing else.
`Nearest` is the same argument one level down: a cell is a cell, not a sample of a smooth
field, and blending two of them draws something the automaton never computed. A scent field is
smooth, so the mold keeps `Linear`.

This is not the per-node `edge` option that was turned down above. That one offered a node its
own answer to what a texture is outside its bounds — black bars against mirroring, a choice
about *presentation*. This is a statement about what the picture *is*: the texture of a
periodic world is periodic, and the declaration is on the output that publishes it rather than
on a menu a hand picks from. A camera has no such fact to declare, which is why it declares
nothing and gets the rule.
---

### A port right-click clears its cables, not Ctrl-hover

**Chosen.** silvia clears a port by right-clicking it and highlights a port's wires on hover;
supersilvia had neither, only a double-click on the cable itself and `Disconnect all` on the whole
selection — a per-cable gesture and a per-node one, nothing in between. Clearing four sources
into one action input meant four double-clicks or losing every other connection on the node
too. `Command::DisconnectPort` closes that gap: right-click a port, input or output, and
every edge on it goes in one undo step.

Right-click rather than `Ctrl` + hover, the gesture the number control's range editor chose
for a related reason
([above](#a-controls-range-belongs-to-the-node-not-to-the-node-kind)): `Ctrl` is the coarse
scrub multiplier on a control and would fight it, but a port carries no such conflict, and
right-click is the gesture that already means "this object's own context" everywhere else on
the canvas — a header, a selection, the background. Using it here keeps one rule for the
whole canvas instead of a second one just for ports.

**The hover highlight is not gated on the gesture.** A cable already brightens when it is the
one nearest the pointer, which is what a double-click there deletes. Hovering a port now
brightens every cable touching *that port* the same way, whether or not it is the nearest —
the same information silvia's `Ctrl` + hover bought, shown on the wires themselves rather
than only implied by the cursor. It does not gate the double-click: a port can carry several
cables, and lighting all of them up must not make all of them one double-click from gone.

**A no-op right-click is left to the command bus.** The port does not check first whether it
has anything to clear; `DisconnectPort` is refused like any other command naming nothing to
do, and a refused command never reaches the undo history. Checking in the canvas would only
duplicate what `Graph::disconnect_port` already has to decide.
---

### A port row paints the declared label, and keeps the key as the machine name

**Chosen.** Every `InputDef` and `OutputDef` has carried a written label since the registry
existed, and none of them was ever drawn — the row painted `port.key`, so a cabled patch read
`inBlack`, `sampleDistance`, `colorA` where silvia reads `In Black`, `Sample Distance`,
`Stable Color`. The label is display only; the key stays what a cable, a saved file, the
command bus and the accessibility tree name a port by, so nothing that resolves a port by
identity changed.

**The key did not disappear — it moved to the hover.** A port's hover text carries a second
line, `key: frequency`, because it is what a patch file and an export report name, and the row
alone is no longer where it can be read. The accessibility name is untouched — `{slug}{id}.{key}
({ty} kind)` — so a locator built on it keeps working exactly as it did.

**The same argument reaches an option row.** `OptionDef::label` was equally unpainted, and
`Node::options` being a `BTreeMap` meant the rows drew in alphabetical order regardless of how
the node's author sequenced them. The row now draws the label, and in the *definition's*
declaration order — the map stays the storage, for a deterministic file and command bus, but
`node_widget::controls` walks `NodeDef::options` and looks each key up in the map rather than
walking the map itself. A closed select shows the choice's **display name** too, resolved
through `OptionDef::choices`, for the same reason: the value is for the machine, and the
closed state is the one a node is in almost all the time.

**Rejected: leaving the key on the row and adding the label as a tooltip.** That is the
inverse of what a person patching wants read at a glance — the label is what a port *means*,
the key is what a file calls it, and the row is for meaning.

### A node may declare a wider body

**Chosen.** `NodeDef::width` is `Option<f32>`, and `canvas::node_width` is the larger of it
and the widest thing the node's regions ask for — a node whose longest row's label does not fit
`NODE_WIDTH` at 200 declares 240, the same width an Output's own render asks for.
Twenty-seven do, found by
rendering every node in the registry and reading which rows elided, after [the row-label
change](#a-port-row-paints-the-declared-label-and-keeps-the-key-as-the-machine-name) started
painting labels instead of keys. A registry test,
`no_registry_label_clips_its_declared_width` in `ui/node_widget.rs`, holds every definition to
its own declared width so a label added later that does not fit is caught rather than found by
eye — measured against the row's own block, which `canvas::ROW_BLOCK_INSET` makes six points
narrower than the body, because that is the rect the label is painted inside. **240 rather
than the least width each label would fit in**: the list of node widths stays short and
testable, which is the whole of what the rejected content-derived width gave up. Two nodes
take a step past it for the same reason, and by the same rule — `tile` 250, for a Slide
Direction that reads `Horizont…` at 240, and `note` 260 for the box it writes in.

**A row is a label and a control; a select is a label and its longest choice.** The registry
test measures the first and not the second, so a closed select too narrow for
`Horizontal (Rows)` was found by eye rather than caught. Widening the test to option rows is
the obvious fix and it fails seven other nodes that nobody has complained about, which makes
it a change to those nodes and not to this one; it is worth doing as its own pass.

**Rejected: sizing every node to its content**, which is what silvia does. Node height, port
centers and row hit rects all derive from one `canvas::rows` walk *because* every node of a
kind is the same width — a content-derived width would put a text measurement on that walk
every frame for every node, and the strip mode's column ranks and clamping lean on a grid of
equal-width nodes too. A declared width keeps everything the constant bought: it is still one
of a short, tested list of numbers rather than a per-node computation, and the list only grew
by one entry to do it.

**The row pitch is untouched.** Widening the body answers *labels clipping sideways*; a row's
*height* is a different question — `CONTROL_ROW_PITCH` and `OPTION_ROW_PITCH` are already
sized to what a row holds, not to what it says, and nothing here changes them.

### A trace is a picture of the shape a node is editing

**Chosen.** `adsr`, `oscillator` and `animation` draw a line in a fixed 48 pt region,
`widgets::trace`, rather than leaving their state to the Status box: an envelope, a
waveform and a travel are the nodes in the library whose *shape* is the parameter, and four
digits in a column do not show a shape the way a line does. All three widen to
`SCOPE_NODE_WIDTH` — the audio scope's 300 — for the same reason
[the width rule](#a-node-may-declare-a-wider-body) keeps its list short: a sixth number here
would be one more thing to test for one region — and the region asks for that width itself,
rather than the node declaring it.

**One source: `CpuNode::trace`, a ring of what the node published.** Each of the three has a
`cpu` half whose `tick` pushes its own output into a ring — three seconds for `adsr`, silvia's
own history sizes for the other two — and the canvas asks for it every frame, the way it asks
an audio source for its scope. The node declares the region and nothing else: `RegionDef::size`
says how tall the band is, and `widgets::trace` draws whatever ring it is handed.

**What a line still cannot say goes in a caption under it.** `adsr` declares a second region,
`widgets::trace::CAPTION`, drawing the `(key, value)` cells `CpuNode::caption` publishes —
whether the gate is on, and which stage the envelope is in, which is silvia's own pair of
readouts under its envelope graph. Both were only in the Status box, away from the node and
away from the picture they describe. The region knows nothing about which node it draws for,
so a second node wanting a caption writes a `caption()` and declares the region.

**Rejected: a shape computed from the controls.** `oscillator` was a field node when the
band went in, with no `cpu` half to tick a history, so a second answer existed —
`NodeDef::trace_samples`, a `fn(&Node) -> Vec<f32>` over the controls, drawing two cycles of
the wave against its own axis rather than against time. It went with the field node
([`oscillator` is a CPU node](#oscillator-is-a-cpu-node)), and with it went the thing that
made it a maintenance cost: a `match` on the waveform option kept in step with the shader
body formula for formula, plus Rust twins of the shader's `fract`, `mod` and `sign`, because
Rust's `f32::fract` and `f32::signum` disagree with the shader's at a negative argument and at
zero. A ring
of what was actually published cannot disagree with anything.

**What that costs: the trace is empty until the node ticks**, and a node on a closed workspace
does not tick. That is the same rule every published uniform number already follows, and the
band is blank rather than drawn at zero, because a line at zero is a reading and *nothing yet*
is not.

**The x axis is time, not the sample count.** The ring is `cpu::TraceRing`: a span of seconds,
each sample dated with the one clock's `elapsed` at the tick that published it, and the band
puts the newest sample at its right edge and every other one where its date falls. The first
version drew one sample per step, and that made a correct wave look wrong: a frame the
compositor held back gives the next tick a `dt` two refreshes long, the accumulator correctly
moves twice as far, and a plot that gives every sample the same width draws that as a kink.
Plotted by time, a late frame is a wider gap on a line whose shape is the shape — which is the
honest picture, and it also stops the trace from being a frame-pacing display in disguise
([proposals/pacing.md](../proposals/pacing.md)). A node with less than a span of history draws
from partway across, since the left of the band is *nothing yet* by the rule above.

### A port's interior is a hole, not empty canvas

**Chosen.** An unconnected input keeps its ring — the one thing supersilvia's shaped ports say that
silvia's uniformly filled dots do not, which is *nothing is plugged in here* — but the ring's
interior is filled with the port's own hue sunk to a shadow (`Theme::port_hole`), edge to edge
under the stroke, rather than left transparent with a second, smaller dot floating inside it
at 45% of the radius. `tests/snapshots` pins the new paint.

**Rejected: the ring left empty, showing the grid through it.** That was the original
argument for the ring — cheap, and it says *nothing here* by showing the canvas under it — but
in practice it read as an unfinished shape rather than a deliberate one, and the small dot
inside it (`bg_sunken`, 45% radius) read as a mistake: a mark with no reason to be smaller
than the ring it sits in. A hole the size of the ring's own interior, in the ring's own hue
sunk dark, reads as one object — a port with nothing in it, not a ring drawn around a
dot that does not fill it.

**The diamond gets the same fill.** A `UniformNumber` input's unconnected diamond used
`bg_sunken` — a neutral, un-hued dark — where the circle now uses `port_hole`; both draw from
the same token so a `UniformNumber` and a `VaryingNumber` port read as the same kind of
absence in different shapes, rather than one port type having a different rule for what
"nothing here" looks like.

### An action input's button is the row, not a square at the end of it

**Chosen.** silvia's full-width momentary button, kept whole this time. The row — from the
port's label to its right edge — is the pressable target, filled in the action port's own
color and captioned with the port's label, rather than an 18×18 square hugging the row's
right edge with the row's own width sitting empty in front of it as margin. The port dot at
the row's left edge is unchanged: it is still what a cable lands on, and the button is the
control that fires it, the same split silvia's small port beside its full-width button keeps.

**Rejected: the small port-shaped square**, which is what this replaces. The
[argument for it](#an-action-is-a-gate-not-a-pulse) — that the button and the port should read
as one object — survives; what did not survive contact with an actual row is that a target the
size of a stepper, sitting at the end of a mostly-empty 200-point-wide row, reads as *decoration
near a control* rather than *the control*. Making the whole row the button is the same
identity argument the small square made, sized to the row it is drawn on rather than to the
port beside it.

**The accessible name is unchanged.** `{slug}{id}.{key}` is what a test or an agent script
already found `output1.show_a` by, and the row growing to fill the button did not touch it —
only the button's geometry and its caption did.

### The s-number's chrome matches silvia's, in supersilvia's own color tokens

**Chosen.** A parity pass on the s-number specifically: the two caps round on their outer
corner only, square where each meets the trough, so they read as one pill-shaped control
rather than two buttons beside a bar; the value-proportional fill is confined to the trough
between the caps and square-cornered on every side of its own, matching silvia's
`.s-number-slider` sitting inside a wrapper inset by the caps' width rather than the control's
whole span; the border is a two-tone bevel, a darkened `border_normal` along the top and left
and `text_muted` along the bottom and right, standing in for the sunken read a browser's `2px
inset` border gives silvia's own control. `text_muted` sits several rungs up the same ladder
`border_strong` is on — the first cut used `border_strong` itself, and in a render at 2x it
did not read as a bevel; the wider gap between the two sides is
what fixed that. Both are tokens `theme.rs` already has — no color was added for this. Node
chrome elsewhere — the header, the row
pills, ports — is not covered here; **this
entry covers the s-number only**, and the rest remains open work under the same "match silvia
where the two share an element" charge. Palette and port hues stay out of scope
regardless: they are the theme editor's, and the theme editor is built.

**Typing keeps the chrome.** Clicking the value used to swap the whole control for a bare
`TextEdit` filling the entire rect; it now draws the same `chrome` the idle control does and
opens the field only in the trough, so the caps and the border stay on screen while a number
is typed. `↑`/`↓` step the value while the field has focus — silvia's own `keydown` handler
does this too — rather than doing nothing, which is what a single-line text field's cursor
keys would otherwise do; `←`/`→` are untouched.

**Rejected: a permanent min · step · max caption under every control.** Two rounds were
tried — always visible, then visible only while hovered or dragged — and the verdict on
the second was that a caption with nothing to click is not the win it looks like on
a static screenshot; the range editor is where those numbers belong, because that is where
they can be changed. Neither shipped; the control reads exactly as it did before this item,
plus the chrome.

### The s-number's range editor is a two-column table, not a list of `DragValue`s

**Chosen.** Right-click opens Min, Step, Max and Value as rows in a small grid: an editable
field per row, and that row's default beside it, dimmer, where clicking the default copies it
into the field — a reset scoped to one row rather than the whole control. Unit is shown the
same way, read only, where the control has one: there is no per-instance unit to edit, only
the definition's. A `Reset` button below a rule clears the instance's range and puts the value
back to the default in one gesture, which is what `R` already does from the control itself.
Each field is a plain number field (`ui::text::number`), typed and committed on Enter or on a
click anywhere else, drawn in the same `bevel_border` the control's own value sits in — the
popup is the control's own instrument for these numbers, and reads as the same family because
it is drawing the same bevel, not a description of it.

**Rejected: a `DragValue` per field.** A drag on a field sixty-four points wide is a number
nobody meant, and the field wore a drag cursor over a number that is read and typed; an end is
a number somebody means.

**Rejected: the value permanently on the row.** A caption under every control — tried twice,
in [the s-number's own chrome
entry](#the-s-numbers-chrome-matches-silvias-in-supersilvias-own-color-tokens) — is not where a value
worth changing gets changed; the editor is, because that is where the ends it is measured
against are read and set too.

### The `lock cursor while scrubbing` preference reads raw motion, not position

**Chosen.** A preference, off by default, that grabs and hides the pointer for the length of
a number control's drag — `ViewportCommand::CursorGrab(CursorGrab::Locked)` and
`CursorVisible(false)` on `drag_started`, both put back on `drag_stopped` or an `Escape`
cancel — so a scrub keeps moving after a hand runs out of monitor, the same trick a 3D
viewport's orbit camera uses. `egui-winit` already turns a raw `DeviceEvent::MouseMotion` into
`egui::Event::MouseMoved`, which `InputState::pointer.motion()` accumulates every frame
regardless of whether the grab is what is holding the cursor still — `eframe::native`'s
`device_event` handler forwards it whenever the window has focus or a button is down, gated on
neither. The control reads `motion()` instead of `Response::drag_delta` while the preference is
on: `drag_delta` is a *position* delta, and a locked cursor's position stops changing the
moment the grab engages, which would otherwise read as the drag going dead on the first frame
it should feel most alive.

**Verified two ways.** `tests/ui.rs`'s
`locking_the_cursor_scrubs_from_raw_motion_with_no_pointer_moved` proves the widget-level
logic in kittest: after a normal press-and-nudge starts the drag (egui's own click-vs-drag
threshold, `has_moved_too_much_for_a_click`, is set only from `Event::PointerMoved` and never
re-arms mid-drag), a step with only an `Event::MouseMoved` and no further `PointerMoved` still
moves the value — proof that `motion()`, not position, is what the preference reads. Separately,
the live app was driven through the egui MCP with the preference on: no `CursorGrab` warning
appeared in the log (`log::warn!` is what `egui-winit` emits when `window.set_cursor_grab`
returns `Err`), and a Speed control's value moved end to end across repeated grab → drag →
release cycles with an ordinary click working normally between them — under Wayland,
`CursorGrab::Locked` is accepted. Neither check is a substitute for a physical mouse
crossing a real screen edge, which no automated tool here can drive; both are the closest
approximation available; and no problem was invented before proving it.

### The header's `?` and `✕` are circled always, not only on hover

**Chosen.** Node chrome parity: the header's mark pair now matches silvia's literally
rather than approximately. What changed the read was checking silvia's own SVGs rather than
guessing from the rendered look — `circle-help` and `close.svg` are not a bare glyph with a
hover ring; the ring **is** the icon, a path silvia draws whether or not the pointer is
anywhere near it. `docs/ui.md`'s previous line — "silvia's ring, drawn only on hover" — was
wrong about that, written from how a hover state looks rather than from the source. Fixed
here: `MARK_SIZE` (20px), `MARK_MARGIN` (3px inset from the header's right edge) and
`MARK_GAP` (1px between the two marks) in `canvas.rs`, replacing a close/help square sized to
the whole header height and flush to its edge.

**The two marks are not the same weight**, which is silvia's own asymmetry and not a bug to
smooth over: `.node-tooltip { color: var(--text-muted) }` has no hover rule at all, where
`.node-close { background: var(--text-primary) }` moves to `--accent-light` on hover. `?`
therefore stays `text_muted` always; `✕` is `text_primary` at rest and `theme.accent()` on
hover.

`HEADER_HEIGHT` moved from 28 to 26, measured off a 2x capture of silvia's own `blur.png`: the
header's fill band is 52 image pixels tall, 26 at 2x, matching the CSS independently — the
taller of a 20px icon padded 0.25rem top and bottom (26) and a title padded 0.5rem (about
26.4 with a 12px line). Every node grew two points shorter as a result; the image snapshots in
`tests/snapshots/` were regenerated and reviewed, not merely accepted.

**A 1px `border_subtle` rule** now sits where the header meets the first row, standing in for
the seam silvia's own darker header reads as against a lighter row fill — not a literal port
of its `margin-bottom: 0.5rem` gap (which would show the node's own base color through a
visible band, a bigger structural change than "the header's bottom rule" asked for) but the
same idea: a line, where there was none.

### The `?` and `✕` are the icon's own vector geometry, not a font glyph

**Chosen.** The first pass drew `?` as a monospace glyph inside
a `circle_stroke` and `✕` as two crossed line segments inside the same — closer than a
hover-only ring, but still not what silvia draws, and both looked bad. Both marks are now the icon's own path, read off the source rather than
approximated from the rendered look.

**`node_widget::help_mark`** draws Feather's `help-circle` (`circle-help`), viewBox `0 0 24
24`: a circle stroke (`cx 12 cy 12 r 10`), the hook of the question mark
(`M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3` — an elliptical arc into a cubic bezier), and its dot
(`M12 17h.01`, round-capped). The hook is not itself drawable with epaint's primitives, so it
is sampled once, offline, into `HELP_HOOK_24`: the endpoint-to-center arc conversion gives a
circle of radius 3 centerd near `(11.92, 10.0)` swept from about `-160.6°` to `0°`, sampled at
8 points; the cubic bezier that follows it, 6 more, sharing their joint. A filled circle the
stroke's own radius at every vertex fakes the path's round joins and the two ends' round caps,
since a plain polyline of straight segments carries neither on its own. The dot is a stroke too
short to be anything but its own cap — a filled circle, radius equal to half the stroke width,
at the path's one point.

**`node_widget::close_mark`** draws `close.svg`, viewBox `0 0 20 20`, read from a filled path
(`M2.93 17.07A10 10...`) as a ring — an outer circle radius 10
and an inner radius 8 wound the opposite way, which fills identically to a 2-wide circle
stroke at radius 9 — and a cross of two 2-wide bars with square ends,
`(5.76,7.17)–(12.83,14.24)` and `(7.17,12.83)–(14.24,5.76)`. That reading is what is drawn: one
`circle_stroke` and two `line_segment` calls, no path parser and no fill-rule needed for a
mark this simple once it is seen for what it actually is.

Both scale from the icon's own coordinate space at draw time — `rect.width() / 24.0` for the
help mark, `rect.width() / 20.0` for the close mark — rather than pre-scaling the constants, so
the same numbers read against the SVG source stay in the code.

### A node is hand-painted, not an `egui::Window`

**Chosen.** The Status box and the Preferences window are `egui::Window`s and read as more
native than the nodes beside them, which is a fair observation and a bad trade. What a
`Window` gives over a hand-painted node is a shadow, a frame, drag, resize, collapse, close
and z-order; the canvas already has all of those but the shadow, and [the shadow is three
lines](#a-nodes-shadow-is-a-real-one-or-none-never-an-approximation-of-one).

What it costs is not throughput — a `Window` is tens of microseconds, not milliseconds — but
three structural things:

- **The cull goes.** `ui::show` skips a node whose body does not intersect the canvas, which
  is what keeps UI cost proportional to what is visible rather than to graph size. A `Window`
  runs its closure whether or not it is on screen, and its position lives in egui's memory in
  screen space, so skipping it means reading that back out first.
- **Text stops being crisp.** An `Area` is screen-space, so pan and zoom over forty of them
  means `Context::set_transform_layer` per node, which transforms geometry that has already
  been tessellated — scaled bitmap text, blurry zoomed in and mushy zoomed out. The node
  widget picks its font size at the current zoom (`theme::icon_font(FONT_BASE, t.zoom)`), so
  a glyph is rasterized for the scale it is drawn at.
- **The drag becomes egui's.** A `Window` constrains itself into the viewport by default, so
  a node panned off the side is yanked back; multi-select drag, snapping and the undo of a
  move would all need re-plumbing through egui's own drag.

**The chrome is not customizable enough anyway.** `Window::new` takes `impl IntoAtoms`, so an
emoji icon before the title works — but the title bar is built internally as collapse-atom,
grow, title, grow, close-atom. An `Atom::custom` pushed into it reserves space and `show` does
not hand back the `AtomLayout`, so its rect never comes out: there is nowhere to put the `?`
and its tooltip. `title_bar(false)` plus a header of one's own is the supported route, which
is what `node_widget::body` already is.

### A node's shadow is a real one or none, never an approximation of one

**Chosen, after one rejection.** silvia's `box-shadow: 0 0 8px var(--shadow-primary)` is a
soft ambient glow around the whole body. A first pass approximated it with three concentric
`rect_stroke` calls, growing outward with falling alpha, to avoid `epaint::Shadow`'s real
blurred `RectShape` tessellating extra geometry per node per frame. Seen in use, the
glow was not needed, and an approximation of a blur standing in for one was not wanted — so
the approximation went outright rather than being tuned closer.

What stands is the real thing: `node_widget::shadow` adds one `epaint::Shadow` as a rounded
rect with a wide feather, immediately before the body's own fill, scaled by the zoom. It is
`Visuals::window_shadow` rather than a number of its own, so it is literally the shadow the
Status box and the Preferences window cast and the canvas agrees with the windows over it
about where the light is. One extra feathered rect per **drawn** node, against the twenty-odd
a node already paints, and the cull means an off-screen node pays nothing. It is behind the
`node_shadow` preference because whether the graph floats over the mix or sits in it is taste.

The cost objection that drove the approximation was never measured. The approximation is what
lost, not the shadow.

### The groove between a node's sections is two hairlines, not one

**Chosen.** silvia's `<hr>` between two non-empty sections — inputs to outputs, or either to
options — is `border-top: 2px groove var(--border-subtle)`, and a browser renders `groove` as
a line carved into the surface: darker facing the content above it, lighter facing the content
below. `node_widget::groove` draws two adjacent hairlines, `bg_sunken` (already the ladder's
darkest neutral) above `border_subtle`, standing in for that — existing tokens, not a new one
for this alone. Drawn wherever `canvas::rows_with_top` crosses from one `Row` variant to
another, which is exactly where silvia's own `<hr>` sits in the DOM, and nowhere a section is
empty, since there is no crossing to detect there.

### An input or output block insets away from its ports and rounds its own ends

**Chosen.** The "pill rows" the parity review asked for turned out not to be a per-row
shape — `.node-input`/`.node-output` in `node.css` are flat zebra bands, `border-radius: 0`.
What actually reads as an inset "slab" is the *block*: `.node-inputs` carries
`padding-right: 0.5rem` and rounds `border-top-right-radius`/`border-bottom-right-radius` on
its own `:first-child`/`:last-child`; `.node-outputs` mirrors it on the left. The near edge —
where a port hangs off it, `margin-left: -1.1rem` on an input's port, `margin-right: -1.1rem`
on an output's — stays flush to the node's true edge; only the far edge pulls in 6px and
rounds where the block starts and ends. `canvas::row_block` answers this once, in world units,
and everything that used to ask `canvas::rows_with_top` for a full-width band now asks this
instead: the fill in `node_widget::body`, a label's room, and a control's slot in
`node_widget::controls`. Getting only the paint right and not the label/slot geometry would
have floated a control's right edge over the newly-bare gap rather than inside its own row.

**The alternating fill restarts at each block**, matching `:nth-child(even)` being scoped to
`.node-inputs`/`.node-outputs`/`.node-options` independently in the CSS — an output section's
first row is `bg_secondary` even when the input section above it ended on `bg_tertiary`, where
one continuous ordinal across the whole node would have carried the parity across the seam.

**Options carry neither inset nor rounding of their own**, which is asymmetric and
deliberate: `.node-options` has no `padding-right`/`padding-left` and no per-row
`border-radius` in the CSS, only a `:has(+ .node-custom:empty)` rule rounding its whole
bottom when it is the last section before an empty custom area — a different mechanism, already
covered by the node's own bottom corners reading through the pad region that sits under the
last row where a node has no band of its own.

### The color swatch shares the s-number's bevel, not its own flat border

**Chosen.** silvia's `.s-color-swatch` and `.s-number` both write `border: 2px inset
var(--border-normal)` — the identical rule, not a coincidence of two controls that happen to
look similar. The swatch used a flat single-tone `rect_stroke`, which was the same gap the
s-number's own border had before the chrome-parity work fixed it there; `number::bevel_border`
is now `pub(crate)` and `color::swatch` calls the same function the control calls, rather than
keeping a second, flatter implementation of what the CSS says is one rule applied twice.

**Disabled is dashed, not merely dimmer.** `s-color[disabled] .s-color-swatch { border: 2px
dashed border-normal }` — a distinct signal from the recessed-and-usable bevel, matching a
connection overriding the control the same way a disabled s-number's own steppers go quiet
rather than just fading its border. Built from four `Shape::dashed_line` calls, since egui
has no dashed `rect_stroke` of its own.

### A filled port is a flat shape, no highlight

**Tried and removed.** `--port-float-bg`'s own `linear-gradient(140deg, hsla(hue,sat,100%,0.5),
hsl(...) 50%)` is a diagonal glossy highlight, not a flat fill, and a first pass stood in for
it with `node_widget::port_highlight`: one small circle per filled port, the port's own color
lightened toward white and offset toward the upper-left — cheaper than a `Mesh` with
per-vertex color, which is the honest way to draw a true gradient in epaint. Seen on an actual
node, the shine looked bad — so it is gone outright, the same
call as [the node's shadow](#a-nodes-shadow-is-a-real-one-or-none-never-an-approximation-of-one):
a cheap approximation of a highlight is not what was wanted in place of no highlight at all. A filled port is a flat disc, diamond or square again. If a true
gloss is wanted later, the honest way to draw one is a `Mesh` with per-vertex color, not
another stand-in circle.

**The dark-interior ring rule survives contact with silvia's own port size.** `PORT_RADIUS` at
1.2rem is small enough that a ring's hole is still legibly a hole, not a blur — checked in
renders, not assumed because the rule predates it.

---

### An Output's picture is flush, and the blit rounds its corners

**Chosen.** The picture fills the slot edge to edge — sides and bottom flush with the body,
as silvia's `.output-canvas` — and the renderer rounds the picture's two bottom corners by
the body's own radius in the blit shader (`u_corner`, a discard inside the corner's square
and outside its arc). `Fit::Cover` rather than `Letterbox`, since the slot already has the
Output's aspect and a clear-to-black could only ever show as a hairline of a color no frame
contains.

**Rejected: an inset picture with square corners.** The previous rule held the picture off
the sides and bottom by `r · (1 − 1/√2)` so its corners fell inside the body's arc, and let
the border paint over the overlap. Correct, and cheap, but it drew a margin of body color —
with `Letterbox`, of black — around every Output's picture, and that margin is the first thing
a person sees on the node whose whole job is the picture. A second rejected route, painting
corner masks over the picture in the canvas color, fails because the canvas is not one
color: the mix is painted behind it. The shader is the one place that knows both the
picture and its rect.

---

### `feedback` is a cable, not a node

**Chosen.** A loop is a cable from an Output's `frame` port, which is already legal, already
tested and already the mechanism `mix` was written for. `feedback` was the only node whose
*picture* depended on the program it was compiled into: it sampled the frame history of
whichever Output that was. So the same node was two different pictures under two Outputs, and
a tap behind it measured both — `App::collect_readbacks` pooled the two readings into one
number that described neither. The frame port names its Output outright, which is what makes
it context-free, and that is the principle the rest of
[proposals/context-free.md](../proposals/context-free.md) rests on.

**The frame-history array went with it.** Ten `RGBA16F` layers per Output — 74 MB at 720p,
396 MB at 3440x1440 — a full-frame blit per Output per frame, and three prelude uniforms, all
of it read by that one node. **What the array is for is not a deeper feedback**: it is a
texture array a fragment indexes at a layer of its own choosing, so a field into that index
gives every pixel a different moment — silvia's `variabletimefeedback`, which supersilvia never ported.
That is [proposals/history-delay.md](../proposals/history-delay.md), to be built when
something wants it, and it will name its Output the way the frame port does.

**Rejected: keeping the node and giving it an Output option.** It makes the dependency
explicit rather than removing it, and then two nodes name an Output — the option and the
cable — with nothing to say which is right when they disagree. The cable is the one that
draws.

### No node reads the resolution

**Chosen.** A node's output is a function of `uv`, the clock, its own uniforms and textures
named by node — nothing about the program it is compiled into. A body that divided by
`u_resolution` broke that: the same node was a different field in Outputs of different size,
so a tap behind it had a reading per Output and one number to publish, which is the same way
`feedback` broke the one-value-per-uniform-number rule. Seven bodies read it — blur, sharpen,
emboss, edge detection, the halftone and dither grids, and `mix`'s wipes — and `nodes/mod.rs`
carries one `REFERENCE_HEIGHT`, 720, that each of them divides by instead.
`no_node_function_reads_the_resolution` holds every generator in the registry to it, over
every choice of every option, with `sample`'s texel tolerance the one entry left on its
allowlist.

**The controls keep their numbers.** A blur of `2.00px` is still `2.00px`, over the same
range and the same step, and the glyph stays `px`: the divisor changed, not the dial. A
720-high Output is pixel-identical on the y axis as a result, and every other Output draws
the same field at its own pixel pitch — which is the point, since that field is what a tap
measures.

**Rejected: converting the controls to world units.** The arithmetic is the same and the
numbers are not: every ported default would have become a fraction, every tooltip would have
had to be rewritten, and a person reading `0.0056⬓` on a blur learns less than one reading
`2.00px`. The reference height buys the same context-freedom and keeps the dial silvia's.

**The step is isotropic, and that is a fix.** A body that divided an x offset by
`u_resolution.x` and a y offset by `u_resolution.y` stepped a fixed fraction of each axis of
the *screen*, which on a frame wider than it is tall is a shorter step across than down: a
3x3 blur on a 16:9 Output gathered a neighborhood 16/9 taller than it was wide. Both axes
divide by the height, so the kernel is square, as worldspace is. The cost is the other half
of the same sentence — the x axis of blur, sharpen, emboss and edge detection reaches 16/9
further across at that aspect than the screen fraction reached, so a patch saved with a blur
set by eye is a little wider across on reopening.

**The cost, accepted: `dither` moirés off 720p.** Its cell is a cell of a 720-high frame, so
an Output of another height lands the pattern between its own pixels and the interference
shows. A dither that stood still in screen pixels is the one thing a resolution-free node
cannot be — it would be the fragment's position, which is the consumer's frame again — and the trade
is worth it: the ordered pattern is a picture the graph can transform, and the moiré only
appears where the Output is not the reference height.

**The wipes went to the mixer, which already had all eight.** `mix` keeps the plain crossfade
and the two luminance fades, whose answer is a function of the color under the fragment; the
five that sweep a screen — two wipes, the radial, the checkerboard, the lines — need a frame
to sweep across, and [the Main Mixer](rendering.md#the-mixer) owns one (`mixer::Method`).

### A tap measures its input over the unit square

**Chosen.** A measurement is a property of the node, `NodeDef::measure_wgsl`: a function of a
point that its workspace pass's `fs_main` calls at every cell of an `N`x`N` grid over the unit
square, `-1` to `1` on both axes. A tap's pass-through is then `return {input};` and nothing
else. See
[architecture.md](architecture.md#going-down-side-effects-in-the-expression) and
[cpu.md](cpu.md#taps-the-other-direction).

**The coordinates used to come from downstream, and that was the bug.** A tap measured inside
its pass-through function, so it measured at whatever `uv` its caller passed. The emitted
shader for gradient → tap → zoom → Output was `zoom3_output(uv)` computing `zoomedUV` and
calling `tap2_output(zoomedUV)`: the tap evaluated *and measured* the gradient at the zoomed
point. A transform between a tap and its Output, or a difference in aspect between two
Outputs, changed the region measured — and two Outputs then had two readings of one node,
which `collect_readbacks` pooled into a number neither of them saw. A reading should be a
property of the input, and the unit square is the frame the worldspace convention already
fixes: exactly 2.0 tall, centerd at the origin, the same in every Output. So the domain is
the graph's own and not the consumer's, and the reading is the input's.

**Cost accepted: the extremes are the grid's.** A peak that falls between two points is a
peak the tap does not see, where a sum over every fragment would have found it. `max` and
`min` are the extremes of what was measured, which is what they always were — the measured
set is now 16,384 points rather than two million fragments.

**Cost accepted: a fine pattern can bias the mean,** and jitter is the answer. A grid on a
pattern whose period divides it reads the same phase every time — stripes at the grid's pitch
can read all white — so `jitter`, a `Uniform` option, moves each point inside its own cell by
a hash of the cell and `u_time`. Off by default, because a steady reading is what most patches
want and a jittered one dithers; on, the bias becomes noise that a `slew` removes. It is a
uniform rather than code, so turning it on in the middle of a set rebuilds nothing.

**The default is 128, and the choices are 32, 64, 128, 256.** 16,384 evaluations of the tap's
input a frame, against 921,600 for evaluating every fragment twice at 720p — the picture and
the measurement — and against 2,073,600 for one evaluation per fragment at 1080p. The grid is
a `Code` option: it is a number in the emitted guard, so changing it rebuilds.

**One slot, in one pass.** A measured node is measured once, in [its workspace's
pass](#a-tap-is-measured-in-its-workspaces-pass), so there is one reading and nothing to
rank, pick or pool: no case where one Output dropped a frame and a pool swung to one side of
itself, and no Output for the status to name. A reading holds across a dropped frame, since a
pass that did not report replaces nothing.

**A tap with no reading says so.** With no slot in any pass, or none reported yet, the tick
withdraws the node's uniform numbers — `TickContext::withdraw` — and its status reads *not
measuring*. `0.00` is a reading, and a tap nothing measures has not made one; the rows draw
nothing, under the rule an unpublished port already follows. A readback is kept while the node
holds a slot in a pass the plan carries, so a pass that stopped drawing holds its last value
and a tap no pass measures — suspended with its workspace — loses its number instead of
freezing it.

**The probe runs no measurement**, as the Output module it is built from runs none. A tap's
grid is therefore a fixed `N²` evaluations a frame, outside what View ▸ Costs says, and
[rendering.md](rendering.md#tap-buffers) states it.

**The pass is sized to the grid.** The measurements run in a square of the largest grid's side
at the corner of their pass's target, and `compile::pass_size` makes the target large enough
for it, so every cell has a fragment to run it at any grid. A GPU test still sets a tap's grid
to 64, to keep the pass small.

### A tap's uniform is a delayed port

**Chosen.** `tap`'s five uniform numbers, `sample`'s five and `autoexposure`'s `gain` and
`luma` all carry `OutputDef::delayed`, beside every `Texture` output. The value is last
frame's by construction — the CPU half reads the shader's measurement back a frame after it
ran — so the cycle rule that refuses an immediate uniform number loop does not apply to one of
these: `Graph::can_connect` already skips a delayed edge in the check it built for an Output's
`frame` port, and the same argument holds a measurement's: a cable from `tap.mean` to a node
upstream of the tap is feedback, not recursion. `tick_order` already includes delayed edges,
so the loop still ticks in a total order, and the compiler already resolves a uniform number
to a uniform without descending into it, so neither needed a change.

This is the cable `autoexposure` could not take before: measuring after the gain it computes
rather than before it needs a loop from the multiply back to the tap that fed it, and that
loop was refused as an immediate cycle. `autoexposure` keeps `own_uniform` regardless — a
performer wants exposure in one drop, not a tap, a divide and a multiply of their own — but
the composed version of the same idea can now be wired closed-loop with an ordinary cable.

### A tap is measured in its workspace's pass

**Chosen.** A node with a `measure_wgsl` is measured once, in the
[pass](rendering.md#the-workspace-pass) of the first of its workspaces in project order that
has a tab — the same program that draws that workspace's thumbnails — or, awake only because a
deck or a send reads it, in the pass of its first workspace, which then measures and draws no
thumbnails. `link::plan::measured_on` is the rule. No Output's module measures anything: a tap
cabled into one is a pass-through there. The pass draws the measurements' square on every tick
the reading is read, by the rule that decides which Outputs draw, and every Output whose frame
the measurement samples draws with it. A suspended node is measured nowhere, because nothing
would tick to read the slot back. See [cpu.md](cpu.md#taps-the-other-direction).

**A tap's liveness is not a property of what is downstream of it.** A tap whose number was
there while a cable ran from it to an Output and gone when it did not would be a rule about the
graph *below* the node deciding whether the node worked at all, which is the same confusion as
measuring at the coordinate a consumer asked for. Measuring is something a workspace does, so
the node reads because it is on the canvas in front of you, whether or not anything there
renders.

**What it buys.** One measurement per node, so no ranking of Outputs and no reading to pick.
No Output to choose, so no choice to move and relink on a cut, a tab switch or a cable. No
clause in [the recompile boundary](architecture.md#the-recompile-boundary): an edit marks the
Outputs downstream of it and nothing else, and the pass is rebuilt with the graph's shape, as
its thumbnails are. An Output that only carries a tap's pass-through stays idle when
nothing else wants it. And a measurement orders nothing: the passes draw after every Output,
so a tap on a frame reads this tick's and no Output waits for it.

**Rejected, again: an analysis node with a render target of its own.** It lost to the tap for
re-evaluating the expression in a second context — its own resolution, its own frame — and the
pass is not one: it is context-free, one program per workspace that already exists for the
thumbnails, running each measurement at the canonical points [the tap
entry](#a-tap-measures-its-input-over-the-unit-square) made its own, and nothing in the graph
names it.

**Not taken: measuring inside an Output.** A tap measured in every Output its chain reached,
with a slot in each, took one reading by ranking the Outputs — a deck first, then the
workspace looked at, then the lowest id — and a tap reaching none was **hosted** in the
lowest-id rendering Output of its workspace, or the lowest-id deck reaching it on a closed
one. That made an Output carry a chain its picture did not, so the recompile rule needed a
clause marking the host on any edit upstream of a hosted node and on any change to the
assignment; the plan ordered the host after every frame its hosted chain sampled, with an edge
no cable made; a tap whose reading mattered drew its host, which might otherwise have been
idle; and choosing the host by what draws would have moved measurements between Outputs on
every tab switch, each move a recompile. Every piece of that was bookkeeping for a place the
measurement did not need to be.

**Cost accepted: a measured chain is evaluated in the pass.** Every awake measured node's
chain is `N²` evaluations in its workspace's pass on the ticks its reading is read — 16,384 at
the default grid — beside whatever the Outputs spend drawing the same chain. It is what the
same tap would cost measured inside an Output, spent in a program of its own; a pass not
looked at draws only the measurements' square, by a scissor, and a tap on a closed workspace
nothing keeps awake costs nothing.

**What lost besides: the rule that told the performer to make an Output of it.** It was true
and it was an explanation nobody should have had to hear.

### `oscillator` is a CPU node

**Chosen.** silvia's `oscillator` is a `PhaseAccumulator` in JavaScript with a start/stop, a
reset, a one-shot mode, seven waveforms and a scope, publishing one float a frame. Here it was
the one silvia CPU node that went varying: `time`, `frequency`, `phase`, `amplitude` and
`offset` as `VaryingNumber` inputs and a waveform evaluated per pixel. It is now a CPU node
again — `UniformNumber` inputs Time and Offset in waves, Amplitude and Level, silvia's own
waveform phases, and a `UniformNumber` output, which feeds a `VaryingNumber` for free. It
keeps no accumulator: its value is the wave at `Time + Offset`, a wave a second at rest, and
silvia's frequency, start/stop, reset and 50 ms glide are the gear's that drives it
([Gears](#gears-the-one-place-a-rate-lives)); a one-shot is `animation`'s.

**Why the field version lost: one name for a circle and a diamond is the ambiguity the
context-free graph exists to remove.** The field oscillator could be fed a gradient, a noise
or a distance and publish ripples over a picture — something the CPU node genuinely cannot do,
because a `tick` has no `uv`. But it could also not do what silvia's does: an oscillator whose
output is a field cannot drive `adsr.attack`, a Master Gear's Length, `counter.step`,
`slew.input` or a Ratio Gear's Ratio, because a `VaryingNumber` does not feed a `UniformNumber` — and those are most
of what a performer reaches for an LFO for. Keeping both under one name would make the *type*
of a port depend on which one you had, which is exactly the ambiguity the rest of this
proposal removes.
The field node keeps its own name and its own page:
[proposals/field-oscillator.md](../proposals/field-oscillator.md), as `waveform`, to be built
when something wants it.

silvia has no phase offset at all — a reset is how you place the wave — and Offset here is in
waves, zero by default.

**Formulas are silvia's, not the shader's.** A triangle starts at −1, a sawtooth at 0 rising,
a square is `p < π` rather than `sign(sin p)` and so is never 0 at a crossing, `pulse` is a
quarter duty. The parity doc lists each difference; the node that matches silvia is worth
more than the node that matches what supersilvia emitted, because the patches being ported are
silvia's. **`noise` is the one departure**: silvia draws a fresh random every frame, and here
it is one value a wave, keyed by `floor(Time + Offset)` and the node's id, so it holds for a
wave, is the same at the same moment and loops when its Time does — a random a frame would be
the one piece of the node that remembered its path.

**Noise is a per-instance xorshift, not `rand`.** A dependency for one `f32` a wave is not
worth it, and a deterministic generator keyed by the node's id is better than an entropy
one: two noise oscillators in a patch differ, and a test and a rerun see the same numbers.

### A gear publishes four readings, not a mode

**Chosen.** Both gears publish `Cycles` (the count, published whole),
`Phase` (the fraction alone), `Ping-pong` (a triangle over two cycles) and `Trigger` (an event
on each whole cycle, placed where inside the frame it fell), all four every frame. A `mode` option would rebuild
nothing and cost nothing, but it would make a patch that wants two of them two nodes — and the
gear already holds the state all four are derived from. This is the shape `counter` set,
publishing a count and its normalized fraction from one count.

### The three shapers take silvia's shapes: curves, a second slide, and a hold

**Chosen.** `adsr`, `slew` and `phase`, now the Ratio Gear, are the three nodes that shape a
number over time, and each was missing one shape silvia can make. Each now makes it.

**`adsr` follows a curve over the stage's own time, not a rate.** silvia lets each moving
stage be Linear, Exponential or Logarithmic and ships exponential on the decay and the
release; every stage here was a straight ramp, so a fall silvia tapers away left at a constant
speed and the two nodes did not draw the same envelope out of the box. The three menus are
silvia's, the formulas are silvia's `_applyCurve` — `1 − exp(−5t)` and `ln(1 + 9t) / ln 10` —
and so are the four times, 0.01, 0.1, 0.7 and 0.2 nudged in thousandths, which is what puts a
one-millisecond attack a nudge away. Rewriting the stages as a curve over elapsed time is what
made the curves possible at all: a rate has nowhere to put a shape. The sub-frame integration
survives it unchanged, because a segment still hands its remainder to the next stage.

**silvia's exponential reaches 0.9933 at the end of its stage, and that is kept.** The next
stage starts from its own end of the span, so there is a jump of seven thousandths at the
corner. Normalizing it would make every exponential curve here a different shape from the one
silvia draws, which is the thing being matched.

**A fresh gate restarts from zero** — the question the parity review left open. silvia's
`_gateOn` stamps a new gate time unconditionally, so a press during a release snaps to zero
and attacks from there; ours carried on from what was left, which never clicks and reads as
smoother. silvia's is the harder, more percussive one and it is plainly different under a fast
clock, so it is silvia's, and the tooltip says so. What is *not* silvia's is who decides the
gate is down: the level is the sum of the hand and every cable, so letting go of a button while
a cable still holds the gate is not a retrigger, where silvia's last caller wins.

**No Max Level.** silvia scales the whole envelope by a knob that goes to 1000. The envelope
here is 0 to 1 and a gain is a `multiply` downstream, which is how every other normalized
output in the library works.

**`slew` gains a shape, and keeps its two knobs.** A rate limit leaves at one speed and
arrives with a corner; silvia's own smoothing, the one inside Smooth Counter, is an
exponential approach that slows as it lands. Both are worth having and a rate cannot make the
second, so `Shape` picks between them and Rise and Fall are read as a speed in one and as an
approach rate in the other. Two knobs rather than four: a second pair would say the same thing
about a shape only one of them is ever in.

**A gear has a Hold.** `phase` had a Reset and no pause, and the nearest thing — winding its
rate down to zero — lost the rate you were running at. Hold freezes the gear and lets it go
from where it stopped, which is the one thing silvia's hidden accumulator could do and this
could not; on a Master Gear it is silvia's BPM clock's Start/Stop. It is a toggle, each press
freezing or letting go, and a gear that stops closes the gate it was holding open. The tick
walks the frame moment by moment rather than integrating in one lump: a hold and a reset
inside one frame are two moments, and which came first is the answer. A hold is a pause in the
*motion*, not in the controls.

### Uniform math is dual-mode

**Chosen.** One `add`, whose output is a diamond where everything feeding it is a diamond or
a knob and a circle otherwise — and whose *inputs* are diamonds where a field could not land
on them. The rule, the family and the demotion refusal are in
[nodes.md](nodes.md#dual-outputs).

**Why there had to be an answer: a `UniformNumber` input can only be fed by a uniform
number.** That is physics — a field has a `uv` and the CPU does not — and it is what makes the
`UniformNumber` type worth having. But it left the whole `Math` family on the wrong side of
the boundary. An `oscillator` into an `add` into `phase.rate` is the most ordinary patch there
is, and until this landed there was nothing to put in the middle of it: `add` was a varying
shader, and a varying number does not feed a rate. silvia has no CPU math on the video side
at all — a float input becomes a GLSL uniform and its template says outright that the value
cannot be read from JavaScript — so there was nothing to port, only a hole to fill.

**What lost: a twin family of CPU math nodes.** `add` and `addNumber`, `multiply` and
`multiplyNumber`, sixteen nodes where there are eight. It doubles the `Math` menu with rows
that differ in no way a person can see, and then makes a hand pick the right one *before*
knowing which side of the boundary the patch ends up on — the wrong guess is delete, re-add
and re-cable rather than an edit. The whole point of the context-free graph is that a node's
behavior is a function of what feeds it; two names for one formula is the same ambiguity
that cut the field oscillator, and the answer is the same one.

**Rejected: inferring it from the consumer.** An `add` could have been a uniform number
*because something downstream wanted a uniform number*, which needs no demotion rule at all.
It also means an `add` with nothing plugged into its output has no type, that plugging a cable
in can change what a node upstream of it *is*, and that a graph has to be solved rather than
walked.
Inference from the producers runs the way the data runs.

**The price, and it is two things.** A dual node's formula is written twice — once as WGSL
text and once in Rust — and nothing but a test stops the two drifting, so
`a_dual_nodes_two_implementations_agree` holds every one of them equal on the GPU: the node in
circle mode with a constant field on one input, measured through a tap, against what the tick
published on the same knobs. A dual node with no case in that list is itself a failure. And
**demotion is unofferable**: with a `slew` hanging off an `add`, a noise cannot go into the
`add`'s A, because the alternative is an edit that silently cuts the cable the number was for.
A bulk change — a file load, an import — cannot be checked cable by cable, since which of a
file's edges demotes depends on which of the others are in yet, so those go in unchecked and
the types are settled once at the end, with the consumer cable dropped and reported.

**The refusal was right and the ports lied.** A dual node whose flip would demote what reads
its number is **pinned**, and its inputs were still drawn as circles: the whole graph said *a
field may go here* and the drag found out otherwise on release. The type an input draws is the
type a cable may bring, so a pinned node's inputs are `UniformNumber` — a diamond, outlined
while unconnected, the same port a `slew`'s input is — and a hand reads the answer before it
drags rather than after. It costs nothing to say: the mode is already on the instance's own
`PortDef`, and the pin goes there beside it. It also has to be said, because a port is the
only place the answer could live; the alternative was a fourth port state for *legal except
here*, which is the dimming that had already failed to explain itself.

Two consequences fall out rather than being arranged. `ConnectError::WouldDemote` is **gone**:
a pinned input *is* a `UniformNumber`, so a field onto one is an ordinary `TypeMismatch`, and
every case it caught is a case the type check catches first — the registry holds every input
of a dual node to being a `VaryingNumber` with a number control, so a pinned node has no
input left for such a rule to fire on. And the conversion menu reaches a pinned input, which
the demotion rule had to suppress: a field released there is offered the `tap`'s three
readings, the same three a `slew`'s input is offered, and the measurement it lands publishes a
uniform number the pinned input takes. The gesture that was a dead end is the gesture that
answers the question.

**What falls out for nothing: a constant.** Two knobs, no cables, and a diamond publishing
their sum. Nobody has to add a `number` node, and the one that would have been added is
`add` with its B at zero.

### The Euclidean pattern is Bjorklund's, not silvia's accumulator

**Chosen: the textbook construction, so rotation zero draws the figure the rhythm is named
for.**

`euclideanrhythm` builds each lane with Bjorklund's pairing. Take every pulse and give it a
rest; pair up whatever is left over; repeat until fewer than two groups remain.

That gives the published figures at rotation zero. E(3, 8) is the tresillo, `x..x..x.`.
E(5, 8) is the cinquillo. E(5, 16) is the bossa figure, E(7, 16) the samba. Each one starts on
a pulse.

**Rejected: silvia's `_generateEuclideanPattern`, which is what landed first.**

silvia builds the pattern by adding up. It accumulates `pulses / steps` and marks a pulse each
time the running total crosses one.

That draws the same rhythm for every count this node accepts. It only disagrees about where
the rhythm starts, and it starts one step late. Its (8, 3) comes out `..x..x.x`, and the
tresillo only appears once you rotate the lane by one.

A sequencer that needs its rotation set to 1 before it plays the rhythm it is named for is
explaining an implementation detail to a performer.

**Rejected: a `pattern` option holding both.**

No patch wants the additive version for its own sake. It is one rotation away, and rotating is
what the rotation control is for.

The choice would also have to be made, named and understood on every lane of every instance,
forever.

**The cost, taken deliberately.**

A lane imported from a silvia patch now reads one step early. silvia's figure at rotation `r`
is this one at rotation `r - 1`, so a rotation carried across wants one subtracted from it.

That is a departure from silvia's shape. It is taken because the shape in question is not
silvia's invention — it is a published rhythm that silvia draws one step off. The node's own
doc says so, next to the test that pins both figures.

### `muxevent` switches, and there is no crossfade

**Chosen: the option is gone. That is also silvia's shape** — `muxevent.js` only ever
switches.

The crossfade came from the sibling node `muxnumber.js`. There, `select` is a continuous
position, and crossfade mixes the two channels either side of it.

Here the index only moves in whole steps, because an action moves it. So the fraction was
always zero and crossfade did exactly what switch did.

An option that cannot change the picture is worse than no option. It is a control a performer
reaches for mid-set, and nothing happens.

**Rejected: adding a `blend` input so the fraction has something to be.**

A number added to the index before the fraction is taken would let a `phase` sweep between
neighbors. That is a real node and somebody would use it.

But that node is `muxnumber`, where the selector is a number to begin with. It arrives when
that node is ported, rather than growing out of the side of this one.

**One test had to move.** `mode` was the only `OptionKind::Uniform` option any test checked
end to end. That test now runs against `mix`, which has two `Uniform` options of its own. What
is being held is the option kind, not the node it happened to be demonstrated on.

### `smoothcounter` keeps `counter`'s three actions

**Chosen: increment, decrement, reset, and nothing else.**

silvia's node has two more. `set` drives the target to `max`. `jump` drops the value straight
onto the target.

Both are cheap to add. Neither needs a new input — `set` reads `max`, and `jump` reads the
target it already has.

Both are still declined, because each one defeats the smoothing this node exists to add. If
you want a counter that snaps, that is `counter`, unchanged, one menu entry away. This node is
`counter` plus an ease, and the ease is all it sells.

**Recorded because it is a departure from silvia**, alongside one smaller one in the node's
own doc: the ends option is called `ends`, which is `counter`'s name for it, not silvia's
`mode`. What `step`, `min` and `max` open at is silvia's, as the section below says.

### The four Control nodes open where silvia's open

**Chosen**, on the [parity review](../proposals/silvia-node-parity.md)'s reading of
`counter`, `smoothcounter`, `bpmclock` and `clockdivider`. Each of the four keeps what it
does better than silvia's and takes back what silvia's said about itself. `bpmclock` has since
become the Master Gear ([Gears](#gears-the-one-place-a-rate-lives)), and what is said here of
its tap, its triplet and its stop is the Master Gear's.

**A default range is what node you get.** silvia's Counter opens at 0 to 1 in hundredths and
its Smooth Counter at 0 to 1 in tenths: a fade, a mix, an opacity, ready to plug in. Ours
opened at 0 to 8 in whole numbers, which is a thing that indexes. Both are useful nodes and
they are *different nodes to reach for*, so a silvia patch stepping a fade arrived here as an
eight-step index. The defaults are silvia's now, with Min and Max nudged in whole numbers,
and the node's behavior — a true wrap, a clamped fraction, every trigger in a frame counted —
is untouched. Nothing in a saved file moves: a control's value is in the file, and a default
is only what a *new* node opens at.

**Set to Max is a button and Normalized is a port.** silvia's Counter has a fourth action
sending the count to the top of its range, which is the other end of Reset and the only way
there from a trigger; it is here. silvia's Smooth Counter publishes the value and the same
value as a fraction of the range, which is what a fade takes with no arithmetic in between;
it is here too, beside `target` rather than in place of it, so the node has three outputs and
loses nothing. Set to Max and Jump are still declined *on the smooth one* for the reason the
section above gives.

**The tap tempo went with the BPM.** It wrote a whole tempo onto the Master Gear's own `bpm`
knob through `TickContext::write_control`, where it could be seen, nudged and saved; the
Master Gear's length is in seconds now ([Gears](#gears-the-one-place-a-rate-lives)), and a
`tap` measures a picture.

**Triplet was half of silvia's, which silently changes what a patch plays.** It fired two
thirds of a beat where silvia fires a third, so a ported patch clocked in triplets came
across at half speed — the worst kind of difference, because nothing says so. It is three to
the beat, as silvia has it: a triplet is a Master Gear a third of a beat long.

**A clock has to be stoppable, and a stopped clock closes its gate.** silvia's Start/Stop is
the first thing a hand reaches for and ours had none: to stop a rhythm mid-set you pulled the
cable. It is the gear's Hold, an action input like every other here, so a sequencer can stop
the clock as readily as a finger — and on the way down it closes whatever gate it was holding
open, or an envelope downstream would be held by a clock that is no longer running. It starts
running, as silvia's does: a clock you have to start before it says anything is a clock you
cannot check.

**A node that says nothing about itself looks broken.** `clockdivider`'s four rows never
moved, so with a slow clock there was no way to tell it was working. It now carries silvia's
own status line — *Ready ÷4*, then *3 of 4*, with a mark on the beat that got through — as a
`widgets::status` region reading the node's own `CpuNode::status`. The mark is held for a few
ticks rather than one, because a beat is one tick of the synth and the editor paints from
whichever snapshot it took: a flash that lasted one tick is a flash the screen can miss. The
count is held in ticks and not in seconds on purpose, so the node still does not integrate
time. Its ports are Clock In and Divided Out again, silvia's names, which are the whole node
in four words; the keys are unchanged, so no cable and no file cares. And moving Division
starts the group over, as silvia's does — live, that is the difference between a clean change
and one you nudge back into place.

**What was kept, against silvia**, because each fixes something silvia's own version gets
wrong: the beat-accurate firing and the Phase output on the clock, the true wrap and the
clamped fraction on the counters, the first beat and the matching release on the divider, and
every trigger in a frame counted everywhere.

### silvia's other seven Transforms, and the two that keep time

**Chosen: `translate`, `mirror`, `stretchskew`, `perspective`, `polarcoords`, `rotozoom` and
`shakycam` are ported whole, in `transform.rs`, with their numbers unchanged but for the two
that keep time.** Every label,
default, range, step and unit is silvia's, including the ones that read backwards:
`stretchskew`'s stretch scales the *sampling* coordinate, so a Stretch X above 1 squashes the
picture, and `rotozoom`'s Base Zoom multiplies where the `zoom` node divides, so a larger one
pulls the picture away. Both are the opposite of what the name suggests and both are kept, so
a patch carried across reads the same; each node's tooltip says so instead.

**The two that keep time read Time and Offset in one cycle, and Rotozoom counts its turns.**
silvia multiplies wall time by two speeds inside each body — Rotozoom's rotation and zoom,
Shaky Cam's X and Y. Here neither has a speed. One cycle of each is the 20π over which
silvia's rates line up — one set of zoom waves; one X wave and four Y waves — and ambient time
runs it at one a minute, silvia's speeds of one rounded to whole seconds. **Turns**, a whole
number from −10 to 10, default 5, is how many turns Rotozoom makes a cycle — silvia's equal
speeds give 5 — so the node still comes back on every cycle; zero is zoom alone and a sign
reverses it. Shaky Cam reads a Time and an Offset per axis, so Y can shake alone or on a gear
of its own; unplugged both read the one ambient time and the shake is silvia's.

**Rejected: a phase per speed**, integrated on the node — Rotation Phase and Zoom Phase, X Phase
and Y Phase — which this node had while every node kept a speed. It bent each motion on its
own, but it was two accumulators of state on one node, and the two only met again when both
speeds happened to run whole cycles. **Rejected: the second speed as a rate against the first
one's phase**: the first at zero froze the second axis too, and turning the second jumped the
picture by `Δspeed × phase`. **Rejected: reading `u_time` in the body and leaving the speeds as
multipliers**, which is silvia's own answer: it is the `u_time * speed` discontinuity, and a
shake that leaps whenever its speed is touched is the thing this model removes.

**`mirror` drops silvia's ninth mode and gains a center.** Quadrant emits `abs(uv)`, which is
exactly Mirror X+Y — a choice that cannot change the picture, the same reason `muxevent`'s
crossfade is gone. The center pair is the one `zoom`, `rotate` and `fisheye` already share, and
it is what makes the node a placeable mirror line rather than a fold down the middle of the
frame; at its default of (0, 0) the fold is silvia's exactly. Mix stays a mix of the two
*coordinates*, not of two pictures, because that is what makes the picture walk into its own
reflection rather than dissolve into it.

**`polarcoords` guards the scale the way `zoom` guards its own.** The control's range starts
at 0.1, but a connected field does not, and the Cartesian modes divide by it; the
magnitude is floored at 1e-6 with the sign kept, so a field swinging through zero mirrors
rather than emitting NaN. `perspective` keeps silvia's `max(w, 0.001)` for the same reason one
step further on: past the horizon the divisor turns negative and the picture folds through
itself.

**`polarcoords` folds the seam with one sample, not a blend.** Into round coordinates the
input's width wraps once around the center, so its left and right edges meet on the line out
to the left and a picture whose edges differ shows a cut there. The two Smooth modes read the
angle as `cos θ` rather than `θ / π`: the width runs out along the top and back along the
bottom, turning at each end with no slope, so there is no cut and no crease. The cost is the
cosine's own pace, the input's edges lingering at the turns and its middle passing quicker.
Cross-fading two samples was the other way to hide the cut and was not taken, since it
double-exposes the picture; a knob narrowing the turn (`asin(k cos θ) / asin(k)`) was tried
and taken out.

**None of the seven publishes a field.** Each one builds a coordinate and hands it to its
input; there is no coverage, no escape count and no raw quantity on the way. `polarcoords` is
the near miss — it computes an angle in four of its six modes — and the angle *is* the
coordinate it is about to sample at, so publishing it would be the same number twice under a
name that promises a sweep the other two modes never compute.

### The four neighborhood Effects keep silvia's reach, and one of them keeps our threshold

**Chosen: `bloom`, `dilate` and `erode` open at silvia's radius, `edgedetection`'s sample
distance reaches 0, and the morphology threshold stays at 0.**

These four read the picture at more than one texel, so what their radius opens at is what the
node looks like the moment it lands. Three of them had drifted wide. `bloom` opened at 0.050
against silvia's 0.010, which is five times the reach over a disc of eight arms and three
rings: at that radius the arms show, and a fresh Bloom is eight spokes rather than a halo.
`dilate` and `erode` opened at 0.010 against silvia's 0.005. All three are back on silvia's
number, and all three keep the wider top end — 0.5 on `bloom`, 0.2 on the two morphologies —
which is a superset of silvia's, so nothing carried over from a silvia patch clamps.

`edgedetection`'s Sample Distance had a floor of 0.1 where silvia's is 0. A silvia patch
dialed under a tenth had nowhere to land here, and 0 is a real setting: every tap collapses
onto one texel and the edges go away, which is how the effect is faded out from the top of the
knob. Strength keeps its wider 0 to 10 instead of silvia's 0.1 to 5, because reaching 0 there
fades the edges out on the node.

**The morphology threshold is the one choice made against silvia.**

silvia opens Dilate and Erode at a threshold of 0.50, meaning the winning neighbor has to be
half a brightness step away before it is allowed to spread. On ordinary material that
essentially never happens, so a freshly dropped Dilate in silvia does nothing at all until the
knob is pulled down. Here it opens at 0.00, every pixel takes its winner, and the node shows
what it does when it lands. That is the same reason `edgedetection` runs on the default
picture where silvia's feeds itself flat gray, and it is worth more than matching a number a
patch carries explicitly anyway. Dilate and Erode share the value, since they are one node
with the comparison flipped and nobody expects them to open differently.

**Not adopted: an Invert row on `edgedetection`.**

silvia has a fourth knob that sits on 0 or 1 and flips the edges to dark on light. It is the
`invert` node, one cable later, and a knob that only has two positions is an option pretending
to be a control. The tooltip says so, so nobody hunts the row.

### The theme belongs to the person, not the project

**Chosen: `theme` is a preference, and a project cannot override it.**

The editor looks the way you like it, whichever project is open. Carry a project to another
machine and it does not re-tint that machine's editor.

That is the tier test read in the right direction. Two people can open the same project,
disagree about the four colors, and the project still means the same thing.

**Rejected: letting a project carry its own theme.**

The argument for it is real. A show has a look, and a VJ might want that look to come back
with the set.

The answer is that the look a performance has is the *picture*. It is not the chrome around
the picture.

There is also a practical reason. Once a project can override a preference, nobody can answer
two questions: which one is in force right now, and which one did I just change?

### The preferences window previews live and has no OK button

**Chosen.** Drag a color and the whole editor re-tints in the same frame.

The point of "a whole re-theme is four numbers" is watching what those four numbers do. A
dialog that hides the result until you press OK makes you guess instead.

**So there is nothing to cancel**, and that part is worth saying out loud. A preference saves
as it changes. There is no draft copy that can drift from what is on screen.

The way back to where you started is a named look, not an undo stack: pick `vapor`, which is
exactly what the editor ships with. A test holds those two the same, because getting back to
where you started is the job Cancel would have done.

### The presets are a table in the source, not a folder of files

**Chosen: sixteen presets compiled into `theme::PRESETS`.**

A table can be tested. Three tests hold it: the default is one of the sixteen, no two are the
same look, and each one survives being written to the preferences file and read back.

A look that ships with the app is also not a document. It is part of the app.

**Rejected for now: a folder of small preset files.**

Files would be shareable, which is what people actually do with looks. That is why this is
*for now* rather than settled.

If presets become importable, the table becomes the built-in ones and the folder holds the
rest. Nothing here blocks that. A preset is four colors and a name either way.

**The sixteen are silvia's own**, ported from `js/settings.js` rather than invented.

Sixteen is the right number because a look is chosen rather than read. A list that long gets
browsed.

silvia gives each preset six colors. Two of them, `audio` and `midi`, are port types there
and not here, so they are dropped rather than given a use they do not have.

**The default stays ours.**

silvia's own default look is `vanilla`: green number ports, orange color ports, violet event
ports. The parity review parked that question here instead of answering it node by node.

`vanilla` is one of the sixteen, so it is one click away. The editor opens in `vapor` because
that is the palette everything else in this design system was drawn against.

### A node's own values are not options

**Chosen: a third store, `Node::values`.**

A node keeps three kinds of state, and silvia keeps the same three:

- **controls** — the value an input port shows when nothing is plugged into it.
- **options** — a choice out of a list the node declares.
- **values** — everything else the node needs to remember.

silvia's own registry describes the third as *"serializable, user-controlled state that
doesn't fit the standard `input` or `options` model"*. This editor had the first two. It had
been stretching `options` to cover the third.

**What separates an option from a value is who draws it.**

An option's row is drawn by one piece of code that works for every node in the library. That
code knows four shapes: a select, a file button, a tick, a one-line field. Those four are all
there will ever be. That is what makes an option portable — a node says "I have a choice" and
gets a row for free, without anyone writing drawing code for it.

A value is drawn by the node's own code. So the list of shapes stays open. There are three: a
box of text, a curve a hand performed, and a picture a hand painted — the last two drawn in a
region rather than in a row. The rest of
[node-body-controls.md](../proposals/node-body-controls.md) will bring a pad and a step grid.

That proposal is why the store was worth building now. Three of its four nodes need to
remember something and draw it themselves. A fourth stretch of `options` would have covered
them and left the word meaning two different things.

**Rejected: keeping a note's text in `options`.**

It was built that way first, and it worked. The problem is not mechanical. It is that
"option" would then mean both "a portable choice" and "whatever this node keeps". The drawing
code can only be shared between nodes because the first meaning is the only one.

**Rejected: a `Control` holding a string.**

A `Control` belongs to an input port. Every port has a type. There is no port type a
paragraph could be: nothing downstream can read it, it has no wire color, and the compiler
has no name for it. Lyapunov's `sequence` got the same answer for the same reason.

**A node has to declare its values, which silvia's do not.**

silvia's `values` is an untyped bag. Any node can put anything in it at any time.

Here, a node declares each value up front through a `ValueDef`: what it is called, and which
`ValueKind` draws it. A note says "I have a text box, four lines tall."

Two reasons it has to work that way. `ui/` is not allowed to ask "is this a note?" — that rule
is what keeps node-specific drawing out of a module every node shares. And undo, addressing
and the save file all need to know what a node holds without running it.

**`ranges` moved into `values`.**

A control's range on one particular node was already a value in everything but name. It is
user-controlled, it is saved, it is not a port's value and not a choice, and it has an editor
of its own. It had a separate field and a separate map in the save file only because there was
nowhere else to put it.

It is now `Value::Range`, in the same map, stored under the name of the input it narrows. A
registry test stops a node declaring a value with the same name as one of its own ports, which
is the only way the two halves could collide.

**The save file changed and nothing reads the old one.**

A `.ssw` written before this has `ranges` where the loader now looks for `values`, so it opens
with its ranges gone and a warning. There is no version bump and no migration. Nothing outside
this repository has ever been saved by this program, so a reader for the old format is a
reader nobody would ever run.

### Four of silvia's seven cable and scrolling settings, and why three are not here

silvia has seven toggles beside its theme. Each was judged on whether it says something the
design system does not already say with shape.

**Kept: `droopyCables`.** A cable sags between its ports. One offset on each control point, no
cost, and people are fond of it. silvia's own sag came with it — fifteen points plus a seventh
of the span, stopping at eighty — and it is on by default as it is there.

Only a forward cable sags. One running backwards already bows downward to clear the node
bodies at each end, and sagging it too would be two reasons pulling one curve. An action cable
stays straight, because dashed *means* event and a sagging dashed line reads as the cable a
data port uses.

**Kept: `reverseScrolling`, as `scroll_x_inverted`.** Linear mode maps the wheel to x, and a
wheel that goes the wrong way there is unusable rather than merely annoying. Which way it
should go is a property of the hand, not of the workspace.

**Kept: the mode a new workspace opens in.** Not one of silvia's — it has no Linear mode — but
it belongs with these. It changes nothing about the workspaces that exist: a workspace's mode
is its own document data, saved in its own file, and it opens the way it was left.

**Kept: `glowOnHover`, without the glow.** Hover a port and every cable it carries
brightens, along with the port at the far end of each. That is how you trace where a port
goes without dragging anything.

The glow itself is dropped. A blur in egui is several more strokes per cable per frame, and
the UI must never make the render miss a frame. The color lightens instead, which is what
hover already does everywhere else.

It is a preference, on by default. This was going to be simply true with no toggle, on the
grounds that the behavior is worth having and a setting for it is a setting nobody needs —
which is a reasonable argument, and it lost: the option was wanted. What stays
out of it is the highlight on the cable *nearest the pointer*. That one is the
click-to-delete affordance, and a preference that hides what a double-click is about to
remove is a preference for making the editor lie.

**Kept as an option, off by default: `phiSpacedWires`.** silvia gives each cable a hue
spaced around the wheel by the golden angle, which tells individual cables in a bundle apart.
It was dropped here first, on the grounds that coloring by port type tells *kinds* apart and
that is what this design system is built on — a wire's color is its type, everywhere, and a
mode where it is not makes the hue mean two different things depending on a setting.

That argument is right about the default and wrong about the toggle, and the toggle was
wanted. The two colorings answer different questions. *Which kind is this* is answered by
the port at each end whatever the wire does, and the shapes — a square for an action, a
diamond for a uniform — answer it again without color at all. *Which of these six wires is
the one I am following* has no other answer, and gets harder the denser a patch gets, which
is exactly when it is asked. So the default stays ours and the option exists, and it is the
one preference in the window that changes what a color *means* rather than whether something
is drawn.

**The ports wear the cable's color as an outline**, a ring just outside the dot rather than
a fill. The dot keeps saying what the port is; the ring says what it is wired to. silvia sets
a 3px border on the element, which is the same ring — CSS grows a border outward too — and the
ring follows the port's shape rather than always being a circle, since a round ring around a
square would undo half of why the square is a square.

An output with several cables wears the **first in graph order**, not the last drawn. silvia
lets whichever connection updated most recently overwrite the border, so the outline on a
fanned-out port changes for reasons that have nothing to do with that port.

**The wire in flight wears the color it will keep**, reserved when the drag arms rather than
chosen when it lands, which is silvia's shape: a `CursorWire` takes a color and hands it to
the `Connection` it becomes. The alternative has one frame in which the color changes for no
reason the hand can see, and a color that changes for no reason is the thing this whole
setting is trying not to do.

**A cable keeps its step for the session, in a map beside the canvas rather than on the
connection.** A display preference has no business in the document, and a step derived from a
cable's position in the graph would recolor every cable each time one was deleted. silvia's
counter reshuffles a whole patch on reload; this one is only reshuffled by closing the app,
and undo puts a cable back under the color it had.

**Dropped: `stripedWires`.** A coil texture on data wires takes the one visual distinction
that carries meaning here: dashed *means* action.

**Dropped: `showPortBorders`.** A connected port's border is a state indicator, not
decoration. A toggle that hides it is a toggle for making the editor lie.

### The Main Input is a panel and a node, and the argument against it was wrong

**Chosen.** silvia's Main Input — one video source and one audio source, chosen in a left
panel, read by any number of `maininput` nodes — is here, in silvia's shape.

The proposal that preceded it argued the opposite — that there should be no node at all — on
the grounds that everything it does is already something else: one device many readers is a node shown on
several workspaces, the audio half is `audioin`, the video half is `camera`. That is all true
and it misses the point, which is the same point the mixer makes. **A rig has things that are
not part of the patch.** Which camera is plugged in tonight, which screen is being captured,
what is coming out of the speakers — those are facts about the room, and a performer sets them
once and then builds a patch against them. Reaching into a node's option to change what the
whole show is pointed at is the wrong gesture, in exactly the way reaching into a node to
crossfade would be.

So the panel is the rig's and the node is a reader. It follows that:

- **The tuning and the thresholds are global**, on the panel, not on the node. Not as taste:
  the bands are measured and the thresholds crossed on the audio thread inside the one capture
  every reader shares, so a per-node copy would mean whichever node ticked last decided what
  all of them saw. A node that wants its own tuning is an `audioin`, which owns its capture.
- **`camera`, `video` and `audioin` are untouched.** This is the other shape, not a
  replacement: a patch that wants four cameras still wants four `camera` nodes.
- **A new video workspace is born with a Main Input and an Output, cabled.** The shortest
  patch that shows a picture is a better first thing to meet than an empty plane, and it is
  two nodes to delete if it is not what you wanted.

**Dropped from silvia's panel: the demo video.** A file bundled with a web page that had
nothing else to show.

**Dropped: the Gain / Expand / Smooth grid.** Nine numbers behind the analyzer, shaping each
band before anything sees it. Shaping a band is the graph's job here — `slew` smooths and an
exciter expands, where they can be seen, metered and patched. What the panel has instead is
the scope this app already has: the spectrum, a two-axis handle per band, and the threshold
sitting on the meter it is compared against, which is the thing silvia's grid could not do.

### The loopback is a monitor source, not a pair of PulseAudio modules

**Chosen.** *System audio (what you hear)* in the Main Input panel opens the default output's
monitor through GStreamer's `pulsesrc`, and follows the default output because the name
`@DEFAULT_MONITOR@` is resolved by the server on connect.

silvia ships `loopback/create_loopback.sh`, which
asks a person to load a null sink and a virtual source, then re-point every application they
wanted to hear at the null sink. That script exists because a browser can only ask for a
microphone. This is not a browser.

cpal cannot do it: its Linux host is ALSA, and ALSA does not expose a PipeWire monitor. So
`Capture` has two backends — cpal for the default input, which is the shorter route to a
microphone's samples and what the audio thread was written against, and a GStreamer pipeline
for a named PulseAudio source. Both hand blocks to the same analyzer, so a microphone, a
decoded file and the loopback are analyzed by identical code.

### Screen capture is the portal, `ashpd` by name, and one runtime for the process

**Chosen.** On Wayland an application cannot read the screen; it asks
**xdg-desktop-portal**, the desktop shows its own picker, and back comes a PipeWire remote —
one file descriptor and one node id. Below that it is what `camera` already is: a GStreamer
pipeline ending in an `appsink`, frames through a triple buffer, the picture as a texture.
`pipewiresrc` is the source element, and a screen cast skips `decodebin` because a compositor
handing over frames has nothing to typefind.

**`ashpd` is a named dependency.** It and `zbus` were already in the lock underneath `rfd`'s
file dialogs, so declaring it directly compiles nothing new — but a crate depended on by name
is a decision, so it is written here.

**Every portal conversation shares one long-lived runtime** (`platform/linux/portal.rs`), and this is the
part worth reading before touching any of it. `ashpd` caches **one D-Bus connection for the
whole process** in a `OnceLock`. Under zbus's tokio feature that connection's socket is pumped
by a task on whichever runtime made it. So the runtime that makes it must outlive every later
call, and nothing may block it.

Getting that wrong does not fail at the call site. The first implementation gave each ask its
own `current_thread` runtime and then parked inside it on a blocking `recv`, which starved the
connection's task for ever. The symptoms were a screen capture that ran for a minute and then
died, a *Choose another screen…* that did nothing at all, and black everywhere downstream —
three bugs that were one bug. The negotiation is now a task on the shared runtime that
`await`s its stop signal, and `rfd`'s dialogs run under the same runtime's guard, since they
share the connection.

The portal session must also **outlive the pipeline**: it ends when its D-Bus object drops, so
`screen::Cast` holds it for exactly as long as anything is reading, and dropping the cast is
how a capture stops and how the desktop's sharing indicator goes down.

### On a Mac, a screen is ScreenCaptureKit's picker, and its frames skip GStreamer

**Chosen.** `SCContentSharingPicker`, the system's own picker, answers with a content filter;
an `SCStream` over it delivers `32BGRA` `CVPixelBuffer`s on a queue of its own, and each is
published into a `Camera`'s one-frame slot as its `IOSurface` beside the same buffer locked,
with no `videoconvert`, no caps and no pipeline. `screen::Stream::head` answers either a GStreamer element (Linux's
`pipewiresrc`) or the slots (the Mac's), and a `Camera` adopts the slots where it would have
built a pipeline, so `latest`, `error`, the Main Input and the Screen Capture node do not
change. `ask`, `Pending` and `Cast` keep Linux's shape. See [media.md](media.md#screen-capture).

**What lost: an `appsrc` bridge.** `Stream::element` would have named an `appsrc` the cast
pushes each `CVPixelBuffer` into, wrapped in a `gst::Buffer`. It keeps `Camera` exactly as it
is, but adds a pipeline whose only job is to hand a buffer from one thread to another, and
the `IOSurface` behind the frame is lost inside the wrapped buffer unless a custom meta carries
it back out.

**The picker allows as many streams as are running and asked for, and stays active while any
runs.** `maximumStreamCount` is set to the streams its answers started plus the asks waiting,
before each `present` and again as each stream stops, as Electron's `SCContentSharingPicker`
patch sets it. **What lost:** the default, which is one ("The default value is 1", Apple's
documentation of the property): the Main Input capturing a window left a Screen Capture node
waiting, with no picker, until the panel's capture stopped. A large fixed count, which lets
Control Center offer a picker with no stream behind it whenever fewer run, and its answer
reaches no ask. And the picker made inactive after each answer, which leaves the streams it
started running under a picker that is not "available for managing capture".

**A stop is the delegate's `stream:didStopWithError:` and a sample marked
`SCFrameStatusStopped`, both writing the error slot.** **What lost:** the observer's
`contentSharingPicker:didCancelForStream:` read as a stop when it names a stream. A named stream
is a change of content the person cancelled, and that stream is still running.

**The `unsafe` is per file**, allowed on `platform::macos::screen` and
`platform::macos::pixels` where `platform/macos/mod.rs` declares them: each sits beside the one
service that needs it. **What lost:** one `platform::macos::sys` module wrapping every Apple
call safely, which is one allowance but a layer of wrappers, and the Objective-C classes would
have to live in it too; third-party safe wrappers such as the `screencapturekit` crate, which
are not in the lock, cover only part of this and are `unsafe` inside; and a Swift or
Objective-C shim built by a build script, which is a second language.

### The mixer does not persist, and the Main Input does

**Chosen.** Which Output is on air, where the fade is, which crossfade, which resolution,
whether Blackout or Freeze holds and whether the mix is painted behind the canvas are **not
written to the project**; each starts at its default every run, and Open and New start each at
its default too. What the Main Input plays and how it is tuned **is**, as silvia's is:
the manifest carries it, and Open brings back a clip, a sound file, the gain, the band tuning
and the thresholds, and leaves a camera, a capture device, a screen and the monitor off.

The mixer is a hand on an instrument. A project opens the same on any machine with nothing
already on air and the canvas not already covered by a mix; reopening last week's set to find a
deck live is not a saved setting, it is a surprise in front of an audience.

The Main Input is the patch's source. A set built against a clip is that clip, and a tuning
set for a track is part of the set; opening the project to find the clip gone and the bands at
their defaults is the patch coming back broken. The line silvia draws, and the one kept here,
is at the device: a camera switched on, a microphone opened or a portal picker put up by Open
is something nobody asked for tonight, so those come back as *None* and a hand chooses them
again. Choosing a source is still not an edit, so it is not a command, is not undone and does
not mark the project unsaved.

This went round twice. Both rode in the manifest first on the argument that *the show travels
with the project*; then neither did, on the argument that the rig is the room and not the
patch. The mixer is the room. The Main Input's files and tuning are the patch.

It follows that **screen capture always asks**. The portal offers a restore token that would
resume the same window with no dialog; it is thrown away, and the portal is asked to remember
nothing (`PersistMode::DoNot`). Choosing *Screen or window* and being silently handed last
week's window, with no say in it, is exactly the surprise this decision is about.

### The fade, Blackout and Freeze are the rig controls the MIDI map can address

**Chosen.** A binding's target is `midi::Target`: a port on a node, or one of the Main
Mixer's three show controls — `Balance`, the A / B fade, and `Blackout` and `Freeze`, the two
presses under it. Nothing else of the rig — the Main Input's gain and thresholds, the
crossfade method, the resolution, the deck claims — has an address.

**Rejected: a `PortRef` target.** A number control is keyed by its input port and an action
input *is* a port, so one kind of reference covers every node — and misses the one control a
VJ reaches for first, because the crossfade lives on a panel.

**Why an enum arm and not a node.** The mixer is not a node
([above](#the-mixer-is-a-render-target-with-two-decks-not-a-node)) and making it one to get a
port would be the reframing this design refuses. Why not a second map keyed by a mixer
control's name: two maps drift, the window would have two tables, and learning would have two
paths. One enum with an arm per mixer control costs a `match` at each place a target is read
and buys one table, one gesture and one file row shape.

**Why only these three.** Which deck is on air is already a button on the Output, so a note
reaches it through `show_a` and `show_b`. The method and the resolution are selects nobody
turns mid-set. The Main Input's tuning is set once for the room. The fade is the one thing on
either panel a hand rides during a set, and Blackout and Freeze are what it reaches for when
the set goes wrong — under a pad, since a mouse is the slowest way to the panel. The enum can
grow an arm the day another proves it.

**A note flips a press, and a CC at 64 and above holds it.** Blackout and Freeze hold until
pressed again, so a pad's note-on is the press and its note-off nothing. A CC is read as a
level so that both kinds of controller button work: a toggling one sends 127 then 0, which is
on then off; a momentary one sends 127 while held and 0 on release, which is a hold-to-black
flash. Rejected: a CC flipping on any value above zero, which a momentary button would flip
twice per press.

**The binding persists and the position does not.** The map is project data — a binding names
this project's controls — so `{"mixer": "balance"}`, `"blackout"` and `"freeze"` ride in
`project.ssp` beside the port rows. Where the fade *is*, and whether a press holds, stays
session state, by [the rig
decision](#the-mixer-does-not-persist-and-the-main-input-does):
opening a project with the fader bound to CC 1, the fade at deck A and nothing held is what a
set should open to.

**It is written where a control write is written, without the bus.** The synth moves the plan's
balance on the tick the CC arrives, keeps it across a plan built before the editor saw it, and
lets a hand's later move win — the three rules every control write already keeps, with the
plan generation standing in for the graph generation. The editor lands it with
`Mixer::set_balance` rather than a `Command`, because moving the fade never was an edit.
Blackout and Freeze are written the same way. A hand's move is said to the synth as well as
sent, `mixer::Hands` on `Msg::Fade`, because "the editor's value differs from the one the
write found" is how the synth tells an answer from a stale fade, and a hand pressing a press
back puts it exactly where the write found it.

### Blackout and Freeze are the mix's, and every way out shows them

**Chosen.** Blackout and Freeze act on the mix itself, in `render/mixer.rs`, after the mix is
drawn and before anything is shown — so the Main Mixer's preview, the mix behind the canvas,
a picture window of the mix, NDI and Syphon all show the same held picture. Freeze draws no
new mix and the newest finished one goes on being published; Blackout publishes an opaque
black texel at the mix's own size in its place, so a window keeps its shape and a sender its
resolution. Blackout outranks Freeze: both held is black, and letting Blackout go shows the
frozen frame, stamped as newer than the black so an outlet that sends only newer frames
sends it.

**Why the mix and not the window.** The audience sees the show through whichever of those it
is, often more than one at a time — a projector window and an NDI feed to a stream. A press
that blacked the projector and left the stream showing the patch would be a press nobody
could trust. An Output's own picture, its window and its Send rows are not touched: they are
an Output, not the show, and a performer watching one while the room is black is the point.

**Blackout keeps drawing the mix underneath; Freeze does not.** Letting Blackout go shows the
mix as it stands on the next frame rather than the one from before the black. Freeze has
nothing to draw: the frame it holds is the one it shows.

**Not saved and not an edit**, by the rig decision above; both start off at launch and on
Open and New.

### Quit, Open and New ask while the show is going out

**Chosen.** The unsaved-edits confirm also stands in front of Quit, Open project…, a Recent
project, New project… and the window's own close while the show is going out: a picture
window open, the mix sent over NDI or published over Syphon, or a render running. With no
unsaved edits it asks only that — `The mix is on a screen. Stop the show?`, `A render is
running (frame 120 / 300). Quit and cancel it?` — with a button named for what it goes on to,
*Quit anyway*, *Open anyway*, *New anyway*. With unsaved edits it says the same lines under
them and that Save or Discard stops the show. A render is canceled, not abandoned: going on
asks the synth to stop it and does what was asked once the render has ended, so what was
written stays whole. See `app/onair.rs`.

**Why not Escape.** One `Escape` closes a picture window, fullscreen or not: it is the key for it rather than an accident to guard against.

**Why an Output's own Send rows do not ask.** They are document data, saved with the
project, and a project that always sends an Output would ask on every Quit. The mix's marks
are session state, set tonight, which is what the question is about.

### Soft takeover is a preference, off by default

**Chosen.** Preferences ▸ Performance ▸ **MIDI soft takeover**, off by default. On, a CC whose value disagrees with its control's moves nothing until the fader
passes the control's value, and the control wears a ghost mark where the fader is meanwhile;
see [media.md](media.md#the-map). Off, a CC writes straight onto its control.

**Why a preference and not a property of a binding.** It is about the hand and the hardware
in front of it — a motorized fader never needs it, a cheap knob always does — rather than
about the project, and a binding row carrying it would be a column nobody sets per knob.

**Why off.** A fader that moves its control at once is what a hand new to the rig expects,
and a fader that does nothing until it is swept across a hidden value looks broken until the
ghost has been learned.

**Why a hairline inside the control.** The ghost sits on the trough where the fader is, the
fill's own scale, so the hand reads the gap between the two at a glance. Inside the
`s-number`'s own size because nothing on a node row or the mixer panel may reflow for live
data.

### A device that goes away lets go of its notes

**Chosen.** Every message crosses as a `midi::Wire` carrying its `Device`, and a held note is
keyed by device and trigger; `Wire::Gone` lets go of every note the device held. The MIDI
window's **Release all** lets go of every note there is. See [media.md](media.md#the-map).

**Why per device.** A key held on a second controller is a hand still on a button. Releasing
every note whenever any device went away would let that one go too.

### A control waiting to learn says so itself

**Chosen.** `Alt` + click, the fade's own learn and **Bind MIDI…** arm the wait and the control
breathes in the accent until a message binds it or `Escape` stops it. See
[ui.md](ui.md#alt--click-to-bind).

**What lost: the MIDI window coming up on every learn.** It covered the canvas at the moment
the eye was on the control being taught, and a window that opens itself is one more thing to
shut; the window opens from its menu entry alone.

### Every bound control wears one mark, drawn in one place

**Chosen.** `ui::midi_mark` draws the dot, its tooltip and its accessible name for every
bindable control there is — a row's number, swatch and press button, a number a region draws,
the Main Mixer's fade — and `ui::MarkAt` says where it sits against what the control is drawn
in: beside a row's slot, on a grid cell's top-left corner, after the fade's caption. The mark
belongs to the control, not to the row, so a control drawn anywhere wears it.

**What lost: a copy per site.** The row and the fade each drew a dot of their own and a region
drew none, so `cosinegradient`'s twelve could be bound and look unbound. Worse, a region's
number could be *learned* and drive nothing: a binding was live only where the node had a port
under its key, and a hidden control has none, so the synth never heard of it and the file never
kept it. A hidden control has the same `{node, key}` address a port's does, and
`NodeDef::bindable` is now what the map asks.

**Why a corner in a grid.** A grid's cells stand four points apart, so a dot beside a cell's
slot would sit on its neighbour's bevel. The four-way gutter at a corner is the one gap where
four rounded corners leave room for it — silvia's own `midi-mapped` dot sits on a corner too.

**Why an Output's render numbers are listed rather than hidden.** They are hidden controls,
and being hidden used to be what left them out of the map. Now that a hidden control is
addressable, the Output names them in `NodeDef::unbindable` — silvia's `midi-disabled`, for
settings nobody turns mid-set — and the range editor offers no binding for them.

### The frame job runs in `Synth::render`, and no panel carries it

**Chosen.** The synth draws once per tick on its own thread; every panel that
shows a picture — the preview, a thumbnail, a picture window — is a viewer that blits what was
published. Folding a panel away hides a picture and nothing else.

**Rejected: the frame job in a panel's paint callback.** It was once `App::preview`'s, so an
early `return` added when the mixer panel became foldable switched the renderer off for the
whole application: every Output black, the taps stopped, a clip driven by a tap frozen at frame
zero — indistinguishable from a crashed driver, and no kittest test can see a paint callback
go missing.

**Rejected: an emergency stop/start pair.** It could release a wedged GStreamer pipeline or
portal session, which is real, but it reused the same device and the same `Renderer`, so
it could not recover the failure people actually reach for such a button for: a lost device
after a driver fault, which needs every GPU object and the device itself rebuilt. A button that cannot do the thing its name promises is worse than no button,
and quitting the application already does strictly more.

**Chosen instead: a lost device saves the work and says so.** It is still not recovered —
rebuilding the device and everything on it in place is the same job the button could not do.
But the process no longer vanishes with wgpu's default panic: the unsaved document is written
into `.autosave/` at once, the person is told on stderr and in the log to restart, and the
next launch says the GPU stopped responding and then offers the edits back
([rendering.md](rendering.md#one-device)). A modal was not an option, because the editor
paints on the lost device too.

### The strip looks after its own length, and Extend and Crop are gone

**Chosen**, after using the edge creep and the eased crop: dragging at an edge should extend
the workspace faster, the strip should always crop itself, and Extend and Crop can go.

How long a strip is used to be a number the person maintained. Extend added five hundred
points past the last node, Crop gave every bit back, and the answer was stored per workspace
in the project file. Three things retired it at once. A drag at the far edge already made
room by itself; the rectangle the view is clamped to already eased, so a strip that changed
length no longer yanked the camera; and the floor — the content, or a viewport's width,
whichever is further out — was already the only length anyone ever cropped to.

So there is no length to keep. `ui::strip_bounds` measures it from the graph and the
viewport every frame, `CanvasState::bounds` eases toward that, and room past the last node
that nothing is reaching for is taken back the way every other change of length is. `extent`,
`extent_target` and `project::View::extent` are all gone. **An older project file still
opens**: an `extent` in a saved view is ignored rather than reported, because the strip
measures itself on the first frame and there is nothing for the person to decide.

**Extending faster is the creep and not the growth.** `EDGE_SCROLL_RATE`
goes from three hundred points a second to nine hundred. A first attempt read it as
*make room faster than the view travels* and left the creep alone; in use the slow pan left
the effect unchanged, which is the whole of it. The
dragged node is held under the cursor and the cursor is at the edge, so the node travels at
exactly the rate the view does. Room made ahead of a view that cannot reach it sooner is a
longer strip and an unchanged gesture.

The growth still leads the creep a little — `EDGE_GROWTH_AHEAD`, twice it — because room
that arrives exactly as fast as it is used leaves the pan against its own clamp for the whole
push. A multiple rather than a rate of its own, so the margin holds whatever the creep is
tuned to. It is written into the clamp's own rectangle rather than into a number of its own,
and read straight back out of the creep vector, so there is one falloff and not two and the
growth depends on the pointer and the frame and on nothing it moves itself.

The falloff is what keeps nine hundred from costing precision: it ramps from nothing at the
inner lip of the margin, so the slow end is still there for placing a node exactly and the
fast end is for carrying one a long way.

Auto-arrange stays, alone in the corner. It is an edit and always was.

### A region declares its own heading, its own width and its own hit rect

**Chosen**, on the four questions [the node-body
proposal](../proposals/node-body-controls.md) left open. The body of a node is **rows, then an
ordered list of regions**, and everything a region needs to be laid out and touched is
declared on the region itself.

**Rows stay rows.** They do not become regions. Making them regions would finish the shape —
the body would be nothing but a list — and it would drag the port rows, and with them the
whole cable system, through a mechanism built for a band of pixels. So `NodeDef::width` stays
the rows' own answer to *my longest label does not fit*, each region declares the narrowest
body it can be drawn in, and `canvas::node_width` is the larger of the two. An Output is 240
because its render says so and an audio node is 300 because its scope does — not because
layout knows what kind of node it is. A region asks for its width open or closed, so folding a
scope away does not reflow the node under the hand that closed it.

**The ticks row is the port-visibility control and nothing else.** `uniforms` and `events`
hide runs of *rows*, which have no heading to sit on; `scope` and `preview` hide regions,
which do, so both leave the row and become the triangle on the thing they govern. A node
carries both idioms at once and they are honestly about two different things. Since rows are
not becoming regions, that is permanent rather than a stage on the way somewhere.

**Regions are interactive from the first cut.** `RegionDef::show` returns `RegionEvent`s in
the stage that ports the five existing regions, before the pad that needs them. A read-only
region returns an empty `Vec` and pays nothing; designing the event half twice, the first time
with no interactive region to design it against, would have cost more than carrying it early.

**The hit rect is the node's business, not one size.** In the Slime Mold a drag between the
options and the ticks carries the node, and should; a node with an interactive drag, like the
XY pad, does not. So `claims_pointer` is a
region's own declaration and `canvas` has no rule about it. A region that does not claim the
pointer lets a press through to the body under it, so a hand carries the node by its trace or
its render exactly as by the whitespace beside a port; a region that claims it owns every
press inside its rect and never starts a node drag. The mechanism is the one egui already
gives the ports: the body's ground is registered first and whatever asks after it wins, which
is also why a scope's band handles work inside a region that claims nothing.

**Rejected: one hit rect for every region**, either the node keeping the whole body or every
region taking its own. The first makes a pad impossible and the second makes a node you cannot
pick up by the part of it you are looking at.

### silvia's other ten math nodes are dual, and Random is a CPU node

**Chosen**, on 21 September 2026, from the parity review's Math family: `abs`, `ceil`,
`floor`, `atan2`, `lerp`, `modulo`, `power`, `pythagorean`, `smoothstep` and `threshold` are
one expression each with silvia's knobs, written with `node!` beside the six the `binary!`
macro makes, and every one carries an `eval`, so a chain of them fed by knobs stays on the
CPU and drives a `UniformNumber` input — which silvia's never could.

**Where a shader body has an undefined case, the two implementations decide it the same way.**
`smoothstep` with its edges the wrong way round is undefined in MSL; here the edges are
ordered first, so Edge A above Edge B is the same ramp and not a black frame. Two equal
edges — `threshold` with no smoothing — are a hard step at the lower one. Both go through
one `eased` function, in Rust and as a shader util, so there is one place the rule lives.
`modulo` is spelled `a - b * floor(a / b)` in Rust, which is what the prelude's `floor_mod` is, so a
negative A wraps onto the positive side on both. `power` keeps silvia's guard, a negative
base under a fractional exponent reading 0, and writes WGSL's `sign(0.0) == 0.0` out by hand
because Rust's `signum` says 1.

**`pythagorean`'s knobs are the family's -100 to 100**, not silvia's -10 to 10, so the
arithmetic nodes all scrub alike. `lerp` keeps silvia's -10 to 10 on A and B and 0 to 1 on T
because those are the numbers a blend is made of, and T is left unheld at its ends so a
driven factor overshoots on purpose. `ceil` and `floor` wear the ceiling and floor brackets,
`⌈` and `⌊`, because silvia's up and down arrows are `max`'s and `min`'s icons here.

**`random` is a CPU node, not a dual one.** silvia's is a GLSL hash of its seed that does
not vary across the picture — one number for the whole frame — and that is a uniform number,
so its three knobs are `UniformNumber` inputs a counter can walk and its answer is published
where a knob can read it. The hash is silvia's own, in single precision, so a seed picks the
same number here that it picked there to within the last bits of a `sin`.

**Rejected: a dual `random`.** The WGSL and the Rust of `fract(sin(x) * 43758.5453)` do not
agree on the GPU's `sin` past the first few bits at an argument in the tens of thousands, so
the equivalence test that holds every dual node could never hold this one.

### A normal map is a picture, and the lamp that reads it has an ambient

**Chosen**, porting silvia's `heighttonormal` and `simplelight` together, which is the only
way either is worth having: nobody draws a normal map by hand, and nothing else here writes
one.

**`heighttonormal` is an `Effect` and `simplelight` is a `Color`**, by the rule the two
files already answer to — the first reads a nine-texel neighborhood, the second reads one
texel and maps it. So the pair that is one idea sits in two files, and each module doc says
why. `simplelight` reads no picture to map, which is the odd thing about it: what arrives is
a direction, and the surface it shades is its own two swatches. That does not make it a
generator, because the field it draws is the one that came in.

**The map is the picture and the height is the field.** `heighttonormal` weighs nine
luminances to get its slope and throws the center one away, so the center is `value` beside
the `color` — the rule in [nodes.md](nodes.md#what-a-generator-publishes-beside-its-picture),
and the thing the review asked for when it said to hand out the height it read. `simplelight`
publishes its Lambertian term the same way: the shading is one number per texel and a patch
has every reason to want it without the color over it.

**The step is a pixel of the reference frame.** silvia steps one texel of the real output, so
its normal map is a different map in every window it is drawn in; every kernel here divides by
`REFERENCE_HEIGHT` instead, and this one does too. That is a departure from the source, and
it is the one the parity review asked for by name.

**Ambient is added, and it defaults to 0.10.** silvia's unlit side is pure black, because the
diffuse term is multiplied straight onto the surface. Here the light is
`ambient + (1 - ambient) * diffuse`, which is silvia's shading exactly at zero, never clips
at one, and at the default leaves the far side readable rather than a hole. A lamp with a
floor under it is what everybody adds to this shader on the second day.

**Both directions are normalized against a guarded length.** A mid-gray normal map and three
lamp coordinates at zero are each a vector of length zero, and `normalize` of one is
undefined. silvia has the same two holes and gets away with them because only a slider can
reach them; here a cable can drive any input anywhere, which is the same reason every divisor
in `adjust.rs` is guarded.

**The icon is 💡 and not silvia's ☀️.** The sun is `gamma`'s icon here already, and two nodes
a menu apart wearing the same mark is worse than a lamp that is exactly what the node is.

### Saturate, Vibrance, Wavefold and Palette are `Color`, and Palette's ports are a set

**Chosen: all four are `Category::Color`**, where silvia files every one of them under its
single `Effects` heading of thirty-three. Each reads the one texel under it and answers, which
is the test the categories here are drawn by — the same test that put `adjust.rs` under `Color`
and the neighborhood kernels under `Effect`. Wavefold is the one that looks like it belongs
elsewhere, since a folded gradient is the most dramatic picture in the group, and it still
never asks what the pixel beside it is.

**The two color conversions live beside the nodes that need them, `pub`.** `rgb2hsv` and
`hsv2rgb` are in `saturate.rs`, which Saturate and Vibrance share; `srgb2oklab` and
`oklab2srgb` are in `palette.rs`. That is `hsla`'s `HSL2RGB_WGSL` shape, and it is what makes them
reachable: a `wgsl_utils` entry is emitted once into a shader that touches a node asking for
it and never into one that does not, so a second node wanting OKLab names the const rather
than inlining a copy.

**Rejected: calling the prelude's `hsv2rgb2`**, which is the same arithmetic under another
name. It is there for `defaultUvMap`, the picture an unconnected color input falls back to,
and it is in every shader whether anything wants it or not. A node reaching into the prelude
for a helper couples every such node to a file the whole compiler shares, to save ten lines in
the shaders that use it.

**Wavefold's fold is inlined into the body, not a shader util.** silvia emits a `hd_fold`
function per node, which a browser can do because it builds one shader per node graph the same
way each time. Here a `wgsl_utils` entry is one global per shader, so two Wavefolds in different
modes in one Output would be two definitions of one name. The mode picks statements over
`driven`, a `vec3f`, instead, and the body is `varying` on both menus.

**Palette's eight outputs are `a` through `h`, and that widens the vocabulary on purpose.**
The names a generator publishes under are a closed list because a library this size written
without one is one idea under seventy names. A set is the case the list cannot name: the eight
differ only in where along the arc they sit, so there is no picture that is *the* picture and
no word for the sixth of eight but its index.
`a_node_names_its_outputs_from_the_vocabulary` now exempts a node whose every picture is a
letter in order, which is narrow enough that a node with two pictures still has to call them
`color` and `map`.

**Rejected: the first output as `output` and the rest as variations.** It reads as a picture
with seven afterthoughts, and it is false — slot A is one end of the fan, not the original.
The original is the input, which is already a port.

**No preset palettes.** The brief allowed for a table of fixed colors behind an option.
silvia has none: `palette.js` computes all eight from the input color and five knobs, so there
is nothing to bake at compile time and every one of the five takes a cable.

**None of the four publishes a field beside its picture**, which is the question every new node
answers. Saturate and Vibrance compute a hue, saturation and value and use all three; Palette
computes a chroma and a hue and uses both, per output; Wavefold's luminance mode computes a
luminance and its per-channel mode does not, so a field published from one and not the other
would be a port that appears and disappears with a menu. Where a body wants that number on its
own, `saturation` and `luminosity` are `Convert` nodes already.

**Palette keeps silvia's 🎨, which `color` already wears.** silvia's own `color.js` and
`palette.js` share it, and the icon is part of what a ported node is recognized by. The label
and the category tell them apart in the menu.

### Colorize, Color Mapping and Color Shift are `Color`, and the aberration is centered

**Chosen: the three recolorings are `Category::Color`**, where silvia files all three under
its one `Effects` heading, and Chromatic Aberration is `Category::Effect`. The test is the one
the categories here are drawn by: a node that reads the one texel under it maps a picture, and
a node that reads a neighborhood of it is an effect. The three read one texel; the aberration
reads three, at coordinates it works out itself.

**One weighting for brightness, and one space for hue.** Colorize and Color Mapping read
brightness at the `luminosity` conversion's weights rather than silvia's `0.299, 0.587,
0.114`, and a test in `recolor.rs` reads the bodies against the table, because a Colorize
holding a different brightness than the `luminosity` node one cable away is two answers to one
question. Color Shift takes the picture apart with `decompose::HELPERS_WGSL` and the `hue`
conversion's own expression. Saturation and value there are HSV's — `delta / maxc` and `maxc`
— and not the `saturation` and `lightness` nodes', which are HSL's: a shifter that read a
picture in one space and rebuilt it in another would move colors nobody asked it to. Color
Mapping's saturation hold is HSV's for the same reason, being silvia's `rgb2hsv().y`.

**Rejected: a shader util for the write back to RGB.** Color Shift is the only node in the
file converting that direction, and a `wgsl_utils` entry is a global name claimed in every
shader the node appears in. Three lines in the body cost less than a name two files could both
define.

**Color Mapping publishes the brightness it graded by, as `value`.** The grade is an index
into three swatches and the index is the quantity the picture was made from, which is what
`value` means. Colorize computes a brightness too and publishes nothing: its luminance is not
an index, it is the `luminosity` conversion of the input, and a port for it would be a second
way to spell a cable. The aberration publishes nothing either — what it computes is an offset
vector, a knob times a direction, and not a field of the picture.

**The aberration is centered where the picture is, and the center moves.** silvia fixes it at
`vec2(0.5)` in a uv whose middle is elsewhere, so its radial fringe is lopsided and its barrel
bulges off center. Here Center X and Center Y are ordinary inputs like every other lens node's,
defaulting to the middle, so the center can be driven per pixel. The offset keeps silvia's
numbers — 0.01 of 0 to 1 — read as world units, which makes the fringe twice the width
silvia's is at the same reading; the alternative was halving a number every ported patch
names.

**Mode is a `Code` option.** The three are not one expression with a term swapped: Radial
normalizes and rotates a ray, Linear needs no center, and Barrel needs no angle and scales
rather than translates. Barrel asks for no angle at all, so that shader neither declares the
uniform nor calls whatever drives it — and nobody changes which kind of lens they have mid-set.

### The keyer is a color map, and the Star Gate takes last frame as a cable

**Chosen**, porting silvia's `chromakey` and `stargate`, the two effects
[the parity review](../proposals/silvia-node-parity.md) names as having no expression here at
all.

**`chromakey` is `Category::Color`, not `Effect`**, although silvia files it under Effects.
The test here is where a node reads the picture, and the keyer reads one texel of its input
and one of its Background under the same fragment — which is `mix` and `layerblend`'s shape,
and they are the two nodes it belongs beside. Its distance, its `smoothstep` and its spill
pull all port as they are; what changes is that the key it worked out is published as `mask`,
1 where the picture stays, so Mix and Layer Blend do the stacking and the node does not grow a
compositor of its own. The spill pull stays on a green or a blue key only, as silvia's does:
suppressing whichever channel happens to be largest on a red or a white key is not a spill,
it is a color shift.

**`stargate` takes last frame as a second picture input.** silvia reads the frame history of
whichever output it was drawn into, which is exactly what
[`feedback` is a cable, not a node](#feedback-is-a-cable-not-a-node) removed: a node whose
picture depends on the program it was compiled into is two pictures under two Outputs. A
cable from an Output's Frame Out is the mechanism that replaced it, it is one frame back with
no offset, and one frame back is all this effect ever wanted — the offset it needs is in
space, not in time. The node is then better than silvia's rather than a compromise: it drags
whatever is cabled to it, not only the Output it happens to sit inside.

**Its Drift is a step per frame, and it reads no Time.** A node that moves with time is a
function of its Time and Offset, because its picture is a function of the moment. This one is
not: each frame shifts what it was handed by one step and hands it on, so the motion is the
feedback loop and a Time would be a number the body has no use for. It is a stateful node, and
its pace is labelled Drift, not Speed, for that reason; the state is the picture in the cable,
not hidden in the node. The step is in pixels of a 720-high frame rather than silvia's
texel of the real output, because [no node reads the resolution](#no-node-reads-the-resolution)
and because a drag that speeds up when the window grows is a different effect in every Output.

**Its Background defaults to silvia's magenta.** Magenta is what a hex that does not parse
reads as here — the visible mistake — so `every_control_default_is_representable` used to
refuse it as a default outright and the keyer shipped with a transparent Background instead.
The test now holds magenta to its own hex spelling, `#ff00ffff`, rather than refusing it, since
that text parses by being the color's own name — so the keyer can carry silvia's default after
all, and the mistake the test still catches is a hex that does not spell any color, magenta
included.

**Rejected: a Background option on the keyer instead of a picture input.** silvia's
Background is already a whole picture, and an option that took a color would make the common
case — a camera over a graph — the one thing the node could not do.

### `sliderule` is a node beside `reframerange`, not a menu on it

**Chosen.** silvia's Slide Rule is ported as its own `Convert` node: one number in, *From*
and *To* picked out of five named bases — 0 to 1, 0 to 360, -1 to 1, 0 to 255, 0 to 2π — and
an invert on each side, which cancel when both are on because the invert is applied once to
the normalized number between them. It is dual, like `reframerange` and the `math` family, so
a knob or a diamond on its input maps a uniform number onto a uniform number on the CPU. The
bases are `OptionKind::Code`: the picks choose the arithmetic at compile time, so the shader
carries the one multiply the pair names rather than silvia's branch over two `int` uniforms.

**Rejected: the named ranges as right-click entries on `reframerange`.** There is no per-node
right-click menu here, and a menu is the wrong place for it anyway: what the five bases are
worth is reach, and reach is a row you can see. They are [rows on the node
instead](#reframe-ranges-named-ranges-are-rows-on-the-node-and-it-is-filed-under-convert).
The two nodes still say different things on the canvas: `reframerange` shows four numbers,
`sliderule` shows *From 0 to 1, To 0 to 360*, which is a range an instance stays in rather
than a place it was started from.
### Three silvia-only nodes: Random Hurl, World Coordinates and Number

**Chosen.** `randomhurl` is a `Generate` node with silvia's one Seed knob — 0, 0 to 1000, a
step of 1 — and three channels hashed at three points, which is the only noise here with
color in it: every other one rolls a number per point and mixes two colors by it, so the
picture lands on the line between them however the controls are set. Its second output is
`value` and not silvia's `mask`, because `mask` means coverage everywhere else in the
library and what this carries is the snow's brightness; its picture is `color` for the same
reason every noise's is, a picture with a field beside it. There is no time row, as in
silvia: this one holds still until Seed is driven and `static` is the noise that moves.

**`worldcoordinates` is a `Generate` node, not a `Source`.** silvia files it under Source
and the parity entry calls it a source node, both meaning only that it has no inputs. Here
`Source` means a device, a file or a previous frame, and `Generate` means made out of
nothing but `uv`, which is exactly what this is. It publishes no picture at all — two
fields and nothing else — which is a shape no other `Generate` node has and which the
snapshot harness and `every_node_compiles_on_the_gpu` already handle, since both gather a
node's fields through an `rgba` whether or not it has a color port.

**`x` and `y` widen the output vocabulary, deliberately.** They are the first names in it
that are not a field the body computed on the way to a picture: the point's own place is
what every other field is a function *of*. Widening the list in
`a_node_names_its_outputs_from_the_vocabulary` is the mechanism the vocabulary is kept by,
and a coordinate has exactly these two names, so nothing else will ask for them.

**`number` is a CPU node publishing a `UniformNumber`, in `Control`.** silvia's is a field —
a `float` generator returning its control — but a constant is the same value across the
frame by construction, so the CPU holds it, every consumer reads one uniform, and a CPU
node can read it too, which silvia's could not be. silvia's range comes across whole: 0,
-100 to 100, a hundredth a step. It lands in `Control` because `Control` is where a value the
CPU produces per frame lives; silvia's Input menu has no counterpart here.

**Rejected: not porting `number`, because `add` with its second knob at zero already is
one.** It is, and it has been all along, and that is the argument's own answer: the thing
exists and only the name is missing, so a person looking for Number finds Add and a knob
that is there to be ignored — which is a knob that gets read. `add` stays exactly what it
is, and a knob on the one input that reads a number stays the answer wherever only one
input reads it.

**Rejected: a dual output on `number`**, the way `add`'s is. A dual output exists so that
one node is both the field arithmetic and the uniform arithmetic; a node with no arithmetic
in it has nothing to be dual about, and its value is a uniform in every instance there
could ever be.
### The camcorder and the flow take the older frame as a cable, and the camcorder is aimed in a region

**Chosen.** `camcordercrt` and `geissflow` are silvia's two feedback looks, ported as nodes
that take the returning picture on an ordinary `Last Frame` input. Both of silvia's read a
frame history behind their backs — whichever Output they were compiled into — which is the
context-free rule's own counterexample and the reason `feedback` is a cable. Here the loop is
the cable from an Output's Frame Out, so a patch says out loud where the older picture comes
from, and anything may sit in its path.

**One frame back is all there is, and silvia's FB Delay is gone with it.** An
instantaneous delay with an old-frame input is enough. So there is no
history to read a depth out of, no delay knob, and
[proposals/history-delay.md](../proposals/history-delay.md) stays declined rather than
becoming this node's prerequisite. What the delay bought — a ghost that arrives visibly late
— is not here; what it cost was a ring of frames per Output allocated for one knob.

**`camcordercrt` is one node rather than a patch of six**, which is the question its parity
entry left open. Two of the six do not exist — nothing here draws scanlines or fringes the
color — and the ones that do (Bloom, Vignette, Zoom, Rotate on the loop) stay where they are
for anyone who wants the chain. A whole look you switch on and a chain you can take apart are
different instruments, and silvia's is the one that was missing.

**Three of silvia's numbers are the frame's, and a node here does not know the frame.**
`no_node_reads_the_resolution` is why: the glow's radius and the scanline pitch are measured
against `REFERENCE_HEIGHT`, so both are the same distance they were at 720p and the same
field in every Output; the barrel curve and the vignette go round the worldspace circle
rather than silvia's aspect-normalized ellipse, exactly as the `vignette` node does. silvia's
bezel darkens all four edges of the frame, and only two of them are edges this node can name
— worldspace is exactly 2.0 tall and as wide as the Output made it — so the top and the
bottom fade and the sides do not. And a sample the camera pushes past the edge mirrors back
in rather than being clamped to it, which is the rule here and what a loop this tight needs.

**A read of `Input` that cannot show is not taken.** silvia reads `Input` seven times a
fragment — three for the fringing, a four-tap cross for the glow — and each read here
evaluates the chain upstream again: under the demo's Perlin that was 6.1 ms at 720p on the
iGPU, on an Output that draws every tick because it is a loop. The glow's four reads sit
behind an `if` on Glow and the fringe's outer two behind one on Aberration, so a read is taken
only where it shows, and `tests/gpu_render.rs` checks the picture is bit-identical to the
seven-read one. The branch costs about 0.1 ms at 720p when every read is a texture fetch,
against about 1.7 ms saved per read of a generator, so it stays.

**The six aiming numbers are ports with controls, and the viewfinder writes them through the
bus.** silvia saves them beside the node with no ports at all; here anything a hand can move
is a knob something else can drive, and the region is a second way to reach the same six.
`widgets::viewfinder` is the first region in the library to declare `claims_pointer` — a drag
drifts, shift-drag tilts, scroll zooms, shift-scroll rotates, a double-click resets — and
every one of those is a `Command::SetControls`, so the write is undoable, saved, and visible
on the rows as it happens. `history::joins` coalesces consecutive writes of the same keys,
which makes a drag one undo step; a wheel has no release to close a gesture on, so a run of
notches on one control is also one step rather than one per notch.

**Two pieces of silvia's viewfinder are not drawn.** The datestamp needs a wall clock and
there is no date crate in the tree, and the hint line under the canvas is the node's tooltip
instead, where every other node's instructions are.

**`geissflow` keeps the floor.** `max(result, live × (1 − feedback))` is what stops the loop
swallowing the live picture at full feedback, and it is most of why the node reads as flow
rather than as a smear. Its flow field reads Time and Offset in its 20π flow cycle, ambient
time at one cycle in 160 s, rather than multiplying the clock by a speed, so nothing about
the field can jump; the feedback half stays per drawn frame.

### Reframe Range's named ranges are rows on the node, and it is filed under Convert

**Chosen**, from the parity review's Convert package. silvia's Slide Rule is one line away
from Reframe Range in the same menu, which is what made degrees, bytes and turns two picks
there and four typed numbers here. `widgets::ranges` is that speed on the general node: two
rows of five buttons — 0 to 1, 0 to 360, -1 to 1, 0 to 255, 0 to 2π, `sliderule`'s own five
by `sliderule`'s own keys — over a Swap that exchanges Output Min and Output Max, which is
what Slide Rule's pair of Invert boxes did.

**A preset writes the two knobs and lets go.** It is a `Command::SetControls`, so it is one
undo step, it rides in the file as the two numbers it wrote, and the bounds stay knobs with
ports on them — a named range is a starting point something can then drive off, which
silvia's fixed bases never were. Nothing stores which one was pressed, so a hand that scrubs
a bound afterwards is not contradicting a mode: there is no mode. The region declares a
heading, so a patch that never reaches for the five closes them and gets the node back.

**It is filed under `Convert`.** silvia keeps it there, beside Slide Rule and the color
readers, because what it does is convert; here it sat at the end of `Math` with the
arithmetic, where a silvia patcher opening `Convert` first did not find it. `Convert` is now
the eleven color readings, the splitter, and the two nodes that convert a number's range —
the heading is what a node answers, not what it reads. It stays dual either way, so the
[dual family](nodes.md#dual-outputs) is every `Math` node but `random` plus those two.

**Multiply and Divide rest at one, and Subtract, Multiply and Divide name themselves on the
output row.** Both knobs of silvia's Multiply and Divide start at 1, so the node dropped into
a running patch passes A through untouched; here they started at 0, which answered 0, blacked
out everything downstream and made a fresh Divide trip its own zero-divisor guard. The output
row carries the operation — `A - B`, `A × B`, `A ÷ B` — where it read Output, which on
Subtract and Divide is the only thing on the node that says which way round it goes. The live
number the row prints in circle mode sits beside the name, which silvia had no way of
showing. The divisor guard stays at `1e-9` rather than silvia's `1e-5`: the width of it is
invisible in use, and the narrower one is the more honest answer.

### Release builds keep their function names, and a panic prints its backtrace

**Chosen.** `[profile.release]`, which `dist` inherits, is `debug = false` and
`strip = "debuginfo"`: the debug information goes and the symbol table stays, and `main`
installs a panic hook that prints a backtrace whether or not `RUST_BACKTRACE` is set. The one
real crash on record, a SIGABRT, came off a binary built `strip = true` and could not be read:
every frame was an offset. Now a panic names every function it went through on stderr, and a
core dump does the same in `coredumpctl`, without the person having set anything first.

**Rejected: line tables** (`debug = "line-tables-only"`, file:line in every frame). The release
binary was 228 MB where it had been 35 MB, and 83 MB with its debug sections compressed; function
names at 44 MB won over line numbers at twice to five times that.

**Rejected: full debug information.** Types and locals are what a debugger stepping through
a live process reads, and nobody reads them from a crash report.

**On macOS** the symbol table stays in the binary the same way, and `packaging/macos/build-app.sh`
strips nothing, so a backtrace names every function on any Mac.

### A tester sends a log, a notice and one block to paste

**Chosen** for the tester release, 2 October: a start with no GPU
exited without a window, `env_logger` with no `RUST_LOG` kept only errors and wrote no file,
and a crash left only stderr, which a launch from a desktop icon throws away. So the log is a
file as well as stderr, a run marks how it ended, the next launch says when it did not, an
exit before the window says so in a box, and Help ▸ Report a problem… is a short form whose
one Copy report button takes the person's words and everything a report needs about the
computer, with the Discord's bug channel and a GitHub issue a button away
([ui.md](ui.md#what-a-tester-can-send)).

**Rejected: copying at once and opening the channel.** The report wants
the person's own words beside the machine's, so it is a form — one open question, *What's
up?*, and who they are — and one big copy button, with the two places to paste it as links that
copy nothing, so there is one way to the clipboard and it is the one the person pressed.

**How a run ended is read out of its own log.** The log's last line is `== closed` and a
reason is a line starting `!! `, so one file answers both whether the run went down and why.
The file is held under an exclusive lock for the life of the process, which the operating
system lets go of however it ends, so a second supersilvia started beside the first is told
apart from a run that died: it finds the lock held and writes a log of its own.

**Rejected: a marker file beside the log**, written at start and deleted at exit. Two files
whose meaning depends on each other, and a second instance reads the first's marker as a
crash unless a process id is checked, which is a guess after a reboot.

**Rejected: `rfd`'s `gtk3` backend for the box.** Its message dialog would be GTK's own on
every Linux desktop, but GTK would enter the process, which the file dialogs keep out by using
the portal ([the file dialog](#the-file-dialog-runs-on-a-thread)). There is no portal
for a message, so the box is `zenity` through `rfd`'s portal backend, and `kdialog` where a
KDE desktop has that instead, since KDE does not ship `zenity`.

**Rejected: an egui window for the box.** The exit it reports is often the GPU it would paint
with.

**Rejected: a report sent from the app.** It needs a server, and a person's consent for each
send; a block on the clipboard and the channel open beside it asks the person to read what
they send, and the channel is where the conversation about the bug happens anyway.

**The report runs `--check` with the editor's own GPU line.** `--check` opens a device to show
one opens; the running app already has one, and a second device on a working GPU is a cost
and a risk the report does not need.

### A painting is saved with the project

**Chosen** (29 September): `drawingcanvas`'s picture is saved with the
project, written into the folder as a picture file, and reopening the project brings it back.
silvia did not keep it — it kept the brush, the color, the canvas size and the symmetry, and
the page reloaded on a blank canvas — so this is more than silvia ever did, and the question it
answers is the one the parity review left open: *is a painting part of the project or part of
the evening?* Part of the project.

**The picture is one of the node's own values**, `Value::Painting` under a `ValueKind::Painting`,
by the rule: a node's values are saved and its runtime state is not. So it is
document data like a note's prose, with everything that follows — a stroke goes through the
command bus, a copied or duplicated node carries its picture, and undo has it. In memory it is
the pixels behind an `Arc`, which the undo ring, a pasted copy and the synth's graph share until
one of them paints, and which the tick publishes as the node's texture without a copy.

**On disk it is a PNG in `assets/`, named by what is in it**: `painting-<print>.png`, sixteen hex
digits of an FNV-1a over the size and the bytes, and the `.ssw` holds that reference and no
pixels. Named by content, a file is never overwritten — a painting that changed is a new name —
which is what lets it ride the save that already lands whole or not at all: `Project::save`
writes each painting that is not on disk yet, synced and renamed into place, before the
workspace files, so the manifest's rename commits a save whose pictures are already there. A
painting saved twice unchanged is written once. The PNG is GStreamer's, through `video/png.rs`,
like every other PNG the program writes.

**Save deletes the painting files it wrote and no longer names**, once it has committed — the
painting a stroke replaced. It is the one file under `assets/` a save deletes, and it is the
save's own output in the way a workspace file is, not somebody's media: the pixels are in
memory, so an undo back to the old picture writes it again at the next save, and a file some
node has taken up as an `Asset` of its own — an `imagegif` pointed at a painting — is kept,
because `asset_users` counts it, and so is any file without a painting's own name, so a
hand-edited workspace file naming a clip as a painting cannot have a save bin the clip. Without
this every save of a canvas in progress would leave a picture behind in the asset cards.

**The autosave carries it**, because the autosave is `Project::save` into `.autosave/`: a painting
there is `.autosave/assets/painting-<print>.png`, read back into memory when the autosave is
recovered and written into the project's own `assets/` by the next Save. An export writes the
paintings into the `assets/` beside the file; an import reads them from there. A painting whose
file is missing opens blank with a warning and keeps its name.

**A stroke is one undo step.** The paint surface writes the whole picture as a `SetValue` on the
press and on every frame the pen moves after it, and they coalesce per node and key into the
step the press opened, which the release ends — so a stroke undoes whole, and a shape or a fill,
written once, is one step too. A step holds the picture it replaced, and the undo ring's byte
budget counts a painting's pixels wherever the step does not share them with the graph beside
it, so a long session of strokes on a large canvas trims the oldest steps rather than holding
gigabytes. **Clear** is the node's action input, as silvia's is, so a sequencer can wipe the
canvas on a beat: the tick writes the background back through `TickContext::write_value`, one
step per Clear, and a Clear of a canvas that is already blank writes nothing.

**Rejected: the painting as a list of strokes.** Replaying a list is what an undo of a single
stroke would like, and a picture file is what was wanted; a list also grows without
end and makes opening a project repaint the evening. **Rejected: one file per node, rewritten
in place.** A save cut off between that file and the manifest would open a mix of two saves.
**Rejected: a `paintings/` folder of its own.** The choice was a picture file like any asset,
and `assets/` is where the export, the import, the asset cards and the file button already look.

### The paint surface claims the pointer, and the brush is the node's own

**Chosen, porting `drawingcanvas`:** the node's body is two regions, `Region::Paint` and
`Region::Brush` (`widgets::paint`), in silvia's custom-area order — the canvas, the six tool
buttons, Size, Color, Background, and the line of keys — at silvia's geometry: a 300 point
canvas in a 1 point border inside 6 points of padding, 28 point buttons 4 apart. **The surface
claims the pointer** and the brush does not, so a drag on the picture paints and a drag on the
air between the buttons carries the node, which is the region contract's whole point.

**What the surface shows is the texture the node publishes**, blitted into a slot the way a
source's preview is, so what is painted is what the patch gets and the surface wears the
pop-out and fullscreen marks every picture wears. The cost is a tick: a stroke reaches the
picture on the tick after the edit crosses. A shape being dragged is drawn by the region itself,
at silvia's 0.7 alpha, because it is not in the picture until the pointer lets go.

**The brush's size and colors are hidden controls**, the palette's and the sequencer's answer
— a number a node draws for itself — so the size is an s-number with every gesture a knob on a
row has, MIDI included, and the swatches open the same picker. **The tool is an option the
region draws**: `OptionDef::in_region` takes it off the rows, and the six buttons write it
through `RegionEvent::Option`, so it is saved and undone as a select is. **Symmetry is an
ordinary option row** above the canvas rather than silvia's select under it: it is the same
widget, and a row is where the library already draws one.

**Kept from silvia:** the arithmetic — round caps, source-over, the eraser taking alpha away
rather than painting the background, `strokeRect`'s square corners, the fill's tolerance of
twenty, symmetry carrying a shape's corners and building the shape again from where they land —
the keys, B E L R C F and `[` `]` with Shift for five, and the wheel on a focused canvas. **Not
kept:** Canvas Size clears silvia's canvas and stretches this one, since a menu that throws a
painting away is hostile and one that stretches it is undone by moving it back; Wrap defaults
to Mirror, [the house rule](#textures-mirror-wrap-outside-their-bounds), where silvia's was
Clamp; and the icon is 🖌, since silvia's 👨🏻‍🎨 is four codepoints and a zero-width joiner the
editor's text draws as its parts.

## Still open

Nothing. Open questions are recorded here as they appear.

### A render warns about a live source and does not refuse

**Chosen.** An Output with a camera, a screen or a microphone upstream wears a `!` whose popup
names each one and goes there; the render runs, and those sources contribute whatever they
give. The result is deterministic given what they gave.

**Rejected: silvia's `offlineBlocked` taint.** It refuses the render outright and propagates
transitively, so one webcam in a corner of a large patch refuses an Output that barely
depends on it. A render with a camera in it is a choice someone made, and refusing a thing
somebody might well have meant is the wrong side to err on. The bit the `!` walks for,
`CpuDef::live`, is registry data beside `integrates`, and both default to the safe answer.

### The wallpaper group is a compile-time branch, and its hint is the choice's own label

**Chosen**, porting silvia's `wallpaper` — the seventeen plane symmetry groups, after Frank
A. Farris's *Creating Symmetry*. Three questions, and the answers are all the same shape: put
it in the data the node already has.

**The group is a `Code` option read by the generator, not a `Uniform` read by the shader.**
silvia emits `int sym_type` and a seventeen-arm `if` chain into every wallpaper shader, which
is five lattice maps and seventeen wave sums the driver allocates registers for so that one
of them can run. Here `varying(|node, ctx| …)` picks the arm while the shader is built, so a
`p6m` program carries the hexagonal map and three terms and nothing else. The cost is the one
[option kinds](nodes.md#option-kinds) names — changing the group rebuilds — and it is the
right side of that trade, because the group is what the pattern *is* rather than a term in it:
nobody sweeps through the seventeen mid-set the way they ride a crossfade.

It pays a second time in the uniforms. A hole nobody writes is never asked for, so a square or
hexagonal group declares no uniform for `Lattice Param`, which it does not read, and `Coeff 5`
and `Coeff 6` reach only the two generic groups that spend all six. The select's own hint says
which coefficients bite; the shader says it too, by not having them.

**The hint line is the choice's display name.** silvia draws a select, a read-only textarea
under it holding a line about the chosen group, and a credit under that. An `OptionDef` here
carries no per-choice tooltip and nothing under the row, so the two places a string can go are
the choice's display name and the node's tooltip. The name takes the hint —
`p6m (Hexagonal) — 6-fold rotation and mirrors` — which reads whole in the open list, where a
person is choosing and the hint is worth its width, and truncates from the right in the closed
row, where `Keep::Start` leaves the crystallographer's name that tells the seventeen apart.
The credit is the tooltip's last sentence.

**Rejected: a `?` on the row, or a hint region under the options.** Both are a mechanism built
for one node — a per-choice tooltip has no other caller in the library, and a region that is a
paragraph of static text is a worse textarea. The label was already a string nobody was using
for anything else.

**`angle` is the field, and the modulus is not published.** The wave sum is a complex number
and both its components become the coordinate the input is sampled at, so what the body
computes beside the picture is its argument: a polar sweep, 0 to 1, the pattern's own phase,
which is [the vocabulary's](nodes.md#what-a-generator-publishes-beside-its-picture) `angle`.
Its modulus is the other half and has no name there — six coefficients of ±2 put no bound on
it, so it is neither coverage nor a 0-to-1 quantity, and a port that would need its own
normalization to mean anything is a port that means nothing.

**Kept, not fixed: `pg`'s first term cancels.** It is `E(1,0) - E(1,-0)`, identically zero, so
`Coeff 1` does nothing on that one group. That is Farris's rule rather than silvia's slip — a
glide reflection flips the sign of a term whose second index is its own negation — and a
wallpaper that departed from the source here would be a different pattern under the same name.
### `muxevent` gets Random back, and counts its channel from one

**Chosen: Random is a fourth button, beside the Reset we added.**

silvia's third action is `rand`, which jumps to one of the four. It was dropped when this node
was ported and `reset` put in its place, on the grounds that a return to the first was the
more useful of the two.

Both are useful, and they are not the same button. Random is what makes this a node you play:
a clock into it is a four-way shuffle, and nothing else in the library will throw you at an
unpredictable input on a trigger. Reset is what makes it a node you can drive from a patch,
and silvia has no equivalent at all. A silvia patch carried across has somewhere to plug its
clock again, and nothing here is lost.

The generator is `nodes::rng`, seeded from the node's own id — `cellularautomata`'s and
`oscillator`'s, not the host's entropy — so the same patch replays as the same performance.
Within a tick Random is read before Reset, so a frame carrying both lands on the first input,
and before the steps, so a Next in the same frame walks on from where the jump landed.

**Chosen: the published channel counts from one, and is drawn with no decimals.**

The rows are labelled Input 1 through Input 4 and silvia's own display reads `Input 1`. The
index published underneath them read `0.00` for that same input, so a person reading the node
added one in their head, on a number that only ever moves in whole steps.

It now publishes 1 through 4, and the shader subtracts the one back off before it picks. The
number on the row is the number in the label above it.

The decimals are an `OutputDef::integral` flag rather than a special case in the canvas. Two
fixed places is the right default — it keeps a monospace readout from re-flowing sixty times a
second — and it is wrong for a count, where the two zeroes claim a precision the value does
not have. A node says which of the two it publishes, and `ui/node_widget.rs` draws it without
knowing whose output it is.

**The cost, named.** A saved patch reading this node's index into something downstream now
reads one higher, and nothing catches it: the port did not move, what it says did. It is taken because the wrong reading was wrong every
time anybody looked at it, and this node has been in the library for days rather than months.

### `muxnumber` wraps when it switches and clamps when it fades

**Chosen: silvia's asymmetry, kept exactly.**

Switch takes the whole number, wraps it round the four with a `mod`, and hands back that
input. Crossfade clamps the number into `[0, 3]` instead, and mixes channel *i* into channel
*i + 1* by the fraction — with the fourth mixing into itself, so it stops there rather than
fading back round to the first.

Read as one node the two disagree, and it would be tidy to make Crossfade wrap too. It is not
tidy in the hand. A switch driven by a counter wants to come back round; a fader driven past
its end wants to stay where it was put, not to slam back to the first input. The two modes are
two instruments, and each one is right.

**Chosen: `mode` is `OptionKind::Uniform`.**

Both branches are cheap, both are in the program either way, and turning a fade into a cut is
exactly the thing a person does in the middle of a set. That is the case
[nodes.md](nodes.md#option-kinds) names for the kind, and `mix`'s own `method` is the
precedent. silvia recompiles here because a browser gave it nothing cheaper.

**Rejected: folding this into `muxevent` as a `select` input.**

One node with both a number and three buttons would have to say what happens when a cable and
a press disagree, and the honest answer is that whichever moved last wins — a hidden mode. Two
nodes, one taking its choice from a number and one from a trigger, each say what they do in
their own name. That is silvia's split, and it is the reason the crossfade that was cut out of
`muxevent` belongs here: here the selector is continuous to begin with, so the fraction is
always something.

### Supersampling multiplies the render and never the live picture

**Chosen.** The Output's `resolution` is one number, shared by the preview and the film, and
the **Supersampling** select — silvia's 1x, 2x and 4x — multiplies it for the length of a
render only. The Output being rendered is drawn at `resolution × scale`, so the whole patch
above it is evaluated at that size, and the capture halves each frame back down before it is
read. What an editor is handed is the size that was on screen, with the hard edges averaged
instead of stepped, which is the whole reason a person renders offline rather than recording
the window.

**Rejected: a resolution of the render's own**, which is what silvia's offline node has. It is
the same knob in a different place and it costs the one property the merged node exists for:
that the picture on the body is the thing you are about to get. A render at 1920x1080 from an
Output you previewed at 1280x720 is a render of a frame nobody looked at.

**Rejected: supersampling the live picture too.** Sixteen times the pixels at 4x is sixteen
times the fill for a performance that has to hold a vsync, and the aliasing a render is worth
fixing is the aliasing nobody can fix in an edit afterwards. Live, a dropped frame is worse.

**Rejected: one linear blit from 4x straight down.** A blit that shrinks by four reads four of
the sixteen drawn pixels behind each written one and discards the rest, which leaves the stair
step it was asked to remove. Two halvings average all sixteen, and the half-way target that
takes the first of them is a texture the capture ring already knows how to free. A multiplier
the Output's resolution does not divide by is refused back to 1x rather than rounded, since a
film one pixel off the preview is the lie the shared resolution was chosen to prevent.

### A note is dragged wider, and never taller

**Chosen.** `NodeDef::resizable` is true on the note alone, and the grip in the corner of its
box writes a width through `Command::SetNodeWidth` — document data like a position, undoable,
one step for the whole drag, saved with the patch. A comment box is sized to the thing it is
commenting on, a label beside one node or a paragraph across the top of a canvas, and that is
the one thing only the person writing it knows.

**Rejected: silvia's both-axis corner.** silvia's note is a fixed box that scrolls inside
itself, so a height is a thing a hand has to set. Here the box grows to fit what is typed and
shrinks back, which is the better half of the two behaviors and the one worth keeping; a
height a hand set would be overruled by the next keystroke or would start hiding text again.
Width is what is left, and it is what the resizing was for.

**Rejected: a width that is session state.** It is not about this run of the app: reopening a
patch to a paragraph reflowed into a column is reopening a different canvas. So it goes in the
workspace file, and a node nobody dragged writes nothing at all.

**Rejected: letting the width go below the node's own.** `canvas::node_width` clamps every
width — from a grip, a file or an undo — to `canvas::natural_width` and to `MAX_NODE_WIDTH`,
rather than trusting the writer. There is one place that decides how wide a node is drawn, and
a body narrower than its rows is a node with its rows outside it. The ceiling is six default
bodies: silvia's textarea has none, but silvia's is a box inside a node where here it *is* the
node, and a body nobody can see the far side of is a header, and so a drag handle, off screen.

### A dropped picture lands on the Image/GIF node, and a dropped clip counts its own wait

**Chosen: the file decides which node the drop makes.** A png, a jpg, a jpeg, a gif or a webp
makes an `imagegif`; everything else makes a `video`. Dropping a file on the window is the
gesture a person arrives with — silvia's node is mostly a dashed panel saying *drop image
here* — and a picture landing on a node that cannot show one is a black node to delete before
placing the right one by hand and walking its File row back to the same file.

**Chosen: a GIF is a picture, and only the Image/GIF node takes one.** `app::files::node_for_drop`
asks `Accepts::IMAGE`, the Image/GIF node's own list, and `Accepts::VIDEO` holds no picture, so
the drop, the file button and the Main Input's clip picker agree on what a GIF is and no file
is in two lists. The Image/GIF node plays an animated GIF by its own delays, on Time and
Offset in plays of the GIF, as `video` plays a clip. A video node that still holds one, from a project saved
before this, refuses it by name — `loop.gif is a picture: an Image/GIF node shows it` —
across its own picture band.

**Rejected: a GIF goes to `video`.** It was in both lists, and the argument was that a dropped
GIF is for an animation playing and `video` is where a clip drop lands. It never played there:
the transcode has no clip to make of a GIF. GStreamer's discoverer gives one no length, so on
a Mac with `avdec_gif` the import failed with `no frames`, and on Linux there is no GIF decoder
behind `decodebin` at all. The failure reached only the Status box, so what the gesture made
was a black node with nothing on it saying why.

**Chosen: the Image/GIF node draws its picture, under the Preview heading the clip node
already has.** The node whose entire content is one still picture was the one node showing
nothing of it — the only sign of what was loaded was a file name elided from the front. It is
the same `nodes::Region::Preview` region the clip and the two simulations carry, so it
costs a region declaration and nothing in `render/`.

**Chosen: a transcoding clip says `Preparing clip… 40%  0:07` across its own preview band.**
Three changes to one line. The **words** are the wait rather than the name of the operation.
The **clock** is the part a percentage cannot supply: a first import of a long clip is
minutes, and 40% does not say whether the rest is ten seconds or ten minutes away. And the
**place** is the black band on the node's body, which is where the eye already is, rather than
the bar behind the file button at the other end of the node.

The percentage stays because it is real — `Transcode::progress` is the encoder's own position
in the file — and the two together say more than either does. `Transcode::elapsed` reads a
start recorded on the shared `Job`, not on the handle, so a second node that joins an encode
already running reports the wait that is actually happening rather than starting a stopwatch
of its own.

**Chosen: the caption is `widgets::picture`'s, drawn from `CpuNode::status`.** Nothing in the
region knows whether it is a clip being prepared or a GIF being counted, which is the same
arrangement the status line region has; a node with a preview and something to say now says it
there. It is painted after the picture's slot is reserved, so it stays legible whatever lands
behind it, and it registers hover only, so a hand still carries the node by its picture.

### The Main Input node wears the meters and gives up the picture

**Chosen: three read-only meters on the node, with the panel's threshold on each.** silvia
draws three level bars on its Main Input node with the trigger handle sitting on each one, and
the reason is the whole argument: a level is set while the band it measures is being watched.
Here the node showed three numbers reading `0.00` and the meters were on the panel, so arming
a trigger was a trip to the far left of the window and back — and with several of these around
a large patch the row being cabled and the level being set were nowhere near each other.

**Chosen: the square says where the threshold is and does not offer to move it.**
`ui::scope::Hands::Off`. The threshold is the rig's, one number for every reader, crossed on
the audio thread inside the one capture — that is why the tuning lives on the panel at all
(see *The Main Input is a panel and a node*). Eight nodes each dragging a copy of it would be
eight answers to a question that has one. Read-only also means no interact of its own, so the
drag reaches the body and a hand still carries the node by its meters.

**Chosen: no spectrum and no band handles.** They are about tuning, and this node has none of
its own. What is left is exactly the meters, which is why they are a region
(`widgets::scope::METERS`) beside the scope rather than a mode of it, sharing `ui::scope`'s
own `meters` so the two nodes' bars are one element and not two. The node widens to
`SCOPE_NODE_WIDTH` for the same reason an `audioin` does.

**Chosen: the picture comes off the node.** The panel is holding the rig's picture a few
inches to the left of the canvas and is the thing that chose it; the node repeating it was one
picture drawn twice, and it cost the node the band the meters now have. The `preview` option
goes with it — a saved file carrying one lands a `LoadWarning::UnknownOption` and loses a
presentation bit, which is what that warning is for.

**Chosen: the Uniforms and Events ticks `audioin` has.** The node was ten rows whether or not
the patch listened to the sound, and the only tick on it was Preview. silvia's has *Numbers*
and *Events*, so a Main Input used for its picture alone is a short node with one row. No
Scope tick beside them: the meters have no heading, because they are the node the way an
Output's render is, and silvia has no tick for them either.

### The words are a node's value and the screen is a node's own session

**Chosen: two Source nodes, `text` and `screencapture`, and neither takes a new dependency.**

They arrived together because they are the same question asked twice — *what does a node own
that the rig already owns?* — and the two answers are opposite.

**`text`: the rasterizer is GStreamer's, because it already is.** `nodes/` may take no
graphical dependency and a font rasterizer is one, which is what kept a text node out of the
library until now. It never needed a new crate: the **pango** plugin that draws subtitles is
in the box, so the letters come out of a pipeline like any other source's and reach the frame
as a texture through the triple buffer a camera's frames come through.

**`textoverlay`, not `textrender`.** One plugin, one rasterizer; what differs is the canvas.
`textrender` sizes its output to the text it was given, so Size would be a resolution rather
than a size and a word would fill the frame whatever it said. `textoverlay` draws into a frame
of a stated size, which is silvia's `<canvas>` with `fillText` on it — so Size, Align,
Baseline and Texture Size mean here what they mean there, and the size goes to pango in
pixels.

**The words are a value, and the colors are ports.** The string is the multi-line box `note`
already draws, declared as a `ValueKind::Text` — see *A node's own values are not options*,
which this is the second user of. It reaches no generator and no uniform, so a keystroke is a
new pipeline and never a new shader. The picture is white on black and only its coverage is
used: `ink` is the raster as a port and `output` is WGSL mixing Text Color into Background
Color by it, silvia's own `mix(bg, textColor, mask)` and the shape `cellularautomata` takes,
where the state is the port and the picture is the shader reading it. So both colors stay
cable-able and neither costs a rebuild.

**A `ValueKind::Text` gained a `default`.** A note starts empty because it says what its
author types; this node starts holding silvia's four lines, because a text node that draws
nothing until it is typed into looks broken. The default is stamped into `Node::values` by
`apply_defaults` beside the options, so it is ordinary document data from the first frame:
saved, undoable, and editable to nothing.

**Rejected: the string as an option.** An option's row is drawn by code shared by every node
and knows four shapes, none of them a paragraph; and a `Code` option changing the string would
rebuild every Output downstream on every keystroke. The same answer `note` got, for the same
two reasons.

**`screencapture`: a session per node, at a dialog per node.** The Main Input's screen source
stays exactly as it is — one capture for the rig, one permission dialog, one picture read by
as many `maininput` nodes as want it, which is the whole reason that panel exists. It cannot
answer silvia's question, because there is one panel and silvia's two Screen Capture nodes are
two windows. So the node is beside `camera`: each one picks its own window through the
desktop's own picker and holds its own `screen::Cast`, and a mix can carry one window in one
corner and another somewhere else.

**The cost, named.** A project holding two of these puts two pickers up every time it is
opened, and a capture runs whether or not anything downstream reads it. That is what a second
window costs, and it is why the panel's one capture stays for everything else.

**No Start button.** silvia has one; the panel has none, and neither does this. Opening the
node is the ask, which is the gesture the panel already teaches. *Choose Screen* asks again
and *Stop* gives the session up — both `Action` inputs with a button on them, so a sequencer
can cut a window in or out — and the status line says which of waiting, capturing and stopped
it is in, because a picker that is up is a minute of nothing happening and a node that says
nothing during it reads as broken.

**None of the portal code is duplicated.** `platform/linux/screen.rs` is a module rather than
a method on the panel, so the node calls the same `ask`, polls the same `Pending` and holds
the same `Cast`; one runtime for the process, the session outliving the pipeline and the
restore token thrown away all hold unchanged. **A node asks every run too**, which is *The rig
does not persist* applied to a node: being silently handed last week's window is the surprise
that decision is about, wherever the picker was opened from.

**`cargo test` never puts a picker on anybody's desktop.** The node asks on its first tick, so
`tests/reset.rs` names it beside the camera and the microphone as one it does not instantiate,
and nothing else in the suite ticks every node in the registry. `doctor.sh` asks the portal
nothing, so the environment check is usable on a machine with no portal at all.

### The pointer and the controller are devices, and the pointer's surface is an option

**Chosen: one slot, two sources, and the framing named on the node.** silvia's `mouseinput`
measured against the editor page, because the editor page was the whole program. Here a hand
can be over three surfaces that mean different things, so the node's **Measure against**
option names one — a picture in a window of its own, the mixer's preview, or the canvas —
with the picture the default and a fallback from it to the preview when nothing is popped
out. "Options for the position framing" is read as exactly that: framing is the surface the
position is framed in.

The plumbing follows from a picture window not being an egui widget. A pop-out is a Wayland
surface on the pictures thread, so its pointer arrives through that thread's own
`wl_pointer`; the preview and the canvas arrive through egui. Both write one
`pointer::Feed`, which `Synth::tick` copies once before the walk and hands to every node
through `TickContext::pointer` — the shape the Main Input's analysis already has, and for
the same reason: two nodes reading one hand must agree about it.

**The position is placed against the picture, not against the rect it is drawn in.** A blit
letterboxes, so a picture whose shape is not its window's has bars; measuring against the
rect would put the number a finger's width off wherever they appear, which is the whole
thing this node is for. A hand on a bar is off the picture and reads nothing.

**Off the surface it publishes nothing rather than zero.** `0.00` is the middle of the
picture, and *the hand is somewhere else* is not that. The rows draw blank, under the rule a
tap with no reading already follows, and a button held when the pointer leaves fires its up —
an envelope it opened has to close somewhere, and a `wl_pointer` leave means the same thing.

**Rejected: reading the pointer wherever it happens to be and letting the node sort it out.**
One number that means a different thing depending on which window the hand wandered into is
not a number. The surface is part of what the value *is*, so it is named.

**Chosen: `gilrs`, on a thread, with the mapping silvia had.** The ask in
`proposals/gamepad.md` settled the dependency; on Linux it is evdev with its own hotplug
watch, and reading evdev by hand through `libc` would be a driver rather than a node. The
node is the camera's shape — a device thread, one slot, a tick that never waits — with one
addition: the slot carries a queue of button edges beside the axes, each stamped with the
instant the thread read it, so the tick can place a press inside its own frame from the age.
silvia polled in an animation frame and could not say better than the frame that noticed.

**The buttons are gates rather than pulses**, which is the event half's rule and not a
choice made here; what it buys is that a held button opens an envelope directly, where
silvia needed a press and a release wired separately. A controller unplugging releases
whatever it held, for the same reason the pointer leaving does.

**Rescan is an action input with a button on its row, not an entry in the device menu.** The
parity entry asked for silvia's Rescan button, and a select whose choices include a verb
would have to decide what the option's value is afterwards — the menu would say *Rescan*
while the node held a controller. A press is what a rescan is, the port that carries it is
where silvia's own button sat, and a sequencer can reach it as well as a hand. The device
menu stays a list of devices.

**Rejected: a `values` field holding the last pointer position, so a patch does not go quiet.**
There is no `values` here, and there should not be one for this: a source that repeats its
last reading forever is indistinguishable from a source that is working, which is the failure
a performer cannot see.

### The wall clock and the recorded curve: two nodes and one new kind of value

**Chosen: `automation`'s recording is a `Value::Points` in `Node::values`, and its transport
is not saved.**

A recorded curve is exactly what the third store is for, and the node is what the store was
built against: it is user-controlled state that is neither a port's control nor a choice out
of a list, and it is drawn by the node's own code. So the recording is one of the node's own
values, declared like any other through a `ValueDef`, and the file carries it.

It needed a new kind on both halves: `ValueKind::Points { min, max }` for the declaration —
the ends the curve is drawn against — and `Value::Points(Vec<Point>)` for what an instance
holds, a `{ time, value }` each. That is the whole of the new machinery.

Whether it is recording or playing, and where the playhead is, live in the `CpuNode` and are
dropped on save, the way every other transient is. Reopening a patch gets the curve back with
the transport stopped, which is what a performer means by *the automation is still there*.

**Chosen: a tick writes the recording through the bus, and writes it once.**

`TickContext::write_value` is `write_control`'s sibling, and exists for the same reason: the
thing the tick made has to be somewhere a person can see, undo and save, and a private field
on the node would be none of those. It takes the same seam — the synth's own graph on the
tick it was asked for, the document through the command bus a frame later, as the `SetValue` a
hand would have sent.

The write happens when a recording **stops**, not while it runs. Every write is one step of
the undo history, and a curve written once a frame would be a recording that costs three
hundred undo steps and a command bus full of points. What is drawn while the performance is
happening comes the other way, on the snapshot, as `CpuNode::curve`.

**Rejected: the curve as a row of the node.** Every other value is a row. A curve wants the
width of the body and a band under the rows, which is what a region already is — so
`ValueKind::Points` is the first value that takes no row at all, and `widgets::curve` draws
it. The band reads the saved value straight off the `Node`, so a patch just opened draws its
performance before anything has ticked.

**Rejected: the curve as a timeline lane.** `proposals/timelines.md` would hold one, and
`automation` would be the name of its record button. That makes a node wait on a workspace
kind it does not need, and the value store is where silvia put it too.

**Chosen: `clock` reads UTC and adds an `offset` knob, rather than the system's own zone.**

`proposals/clock.md` preferred `libc::localtime_r`, and it is the honest clock: glibc reads
`TZ` and `/etc/localtime`, and the reading is right twice a year without anyone typing
anything. Two rules stand in front of it, and changing either is a maintainer's call. `libc`
is not in `Cargo.toml` — it is in the binary transitively, but *a dependency not already in
`Cargo.toml`* is on CONTRIBUTING.md's *Open an issue first* list — and `localtime_r` is an
`unsafe` call, which that same file puts in `render/` and nowhere else. Neither is a thing to decide on the
way past.

So the node reads the epoch and adds `offset` in hours, quarter-hour steps because Kathmandu
is +5:45. The cost is named: it is wrong twice a year on its own, and the performer types
their zone once. The gain is that the offset is a knob like every other one — cabled, saved,
automatable — so a piece can be driven an hour forward without touching a clock. If the two
rules move, the node changes behind its own ports and nothing else does.

**Chosen: both are `Category::Control`, and `clock` is `live`.**

The proposal suggested `Source` for the clock, on the grounds that it reads the world rather
than the graph. silvia files it with `animation`, `oscillator` and `bpmclock`, which is where
a hand goes looking for a thing that makes a number move, and that is the argument that wins:
the menu is for finding nodes, not for classifying them. What the clock's honesty costs is
paid on its `CpuDef` instead — `live: true`, the same bit a camera carries, which says an
offline render cannot make it deterministic. `tests/reset.rs` exempts it for that reason: two
runs of a wall clock are two different instants, which is the whole of what the node is.

**Kept: silvia's arithmetic, in both.** Seconds carry their fraction, the twelve-hour face
runs 0 to 11, the week starts on Sunday, and a recording stores a point only where the knob
moved. The menu that picks a reading is called Mode rather than silvia's Output, which on that
node is also what both of its ports are called.

### A number a node draws for itself is a hidden control, not a port and not a value

**Chosen**, porting silvia's two custom areas made of `<s-number>`s: `cosinegradient`'s twelve
coefficients and `euclideanrhythm`'s twelve lane numbers. The palette is done just
like silvia's, with no cables and only its own s-numbers, and the sequencer's input ports
are scrapped.

**The twelve stop being ports.** They were `VaryingNumber` inputs, which bought what silvia
never had — an envelope on Amp G, a mask on Freq R — at the price of the node being fifteen
rows with nothing drawn on it. These two nodes are the ones whose whole job is to be looked
at: a palette is a picture of a palette, and a rhythm is the one thing a hand reads while it
is playing. So the picture takes the body and the numbers go three across under it, which is
silvia's own layout, cell for cell — `widgets::palette` and `widgets::steps`, geometry taken
from silvia's CSS: the 16 px strip, the 72 px plot, the `28px 1fr 1fr 1fr` grid, the 16 px
step cell with its 4 px gap and its brighter downbeat.

**They become hidden controls, not `Node::values`.** `values` is the third store, for state
that is neither a port's value nor a choice, drawn by the node's own code. A coefficient looks
like it belongs there — it is drawn by the node's own code — but it fails the first half:
it *is* a value the shader reads, and `hidden` is already the field for "a control with no
port, edited somewhere other than a row". An audio source's band center and Q have been that
since the scope's handles were built. Taking the other road would have meant a second uniform
path from a `ValueKind` into the compiler, beside the one `Control::Number` already has, for
nodes whose WGSL is unchanged — and the emitted shader is in fact byte-identical to the one
the ports compiled to, which is the plainest evidence that nothing about the *value* changed,
only about who draws it.

**So there is no loader alias, and there is one dropped cable.** The keys did not move:
`NodeDef::input` searches the hidden list, so a file written when they were ports seats all
twelve exactly where it always did, with no warning. What such a file can no longer have is a
*cable* into one, and that takes the ordinary answer every illegal cable in a file takes —
dropped on the settle, with a `DroppedConnection` warning — while the number it was overriding
stays. `tests/workspace.rs` holds both halves.

**A cell is the port row's own control, through the port row's own code.** `RegionUi::number`
draws `ui::number::scrub` in the rect the region chose, under the same accessible name
`{slug}{id}.{key}`; what comes back leaves the region as `RegionEvent::Number`, carrying the
same `NumberAction` a row produces, and both go through one `node_widget::number_action`. That
is what makes a reset put the range back, a right-click open the range editor, `Alt` + click
learn a MIDI binding and an escaped drag collapse its own undo step — on a coefficient exactly
as on a knob. Writing a smaller control for the grid would have been a second number control
to keep in step with the first, and the [s-number's
rules](ui.md#the-number-control) are detailed enough that it would have drifted within a
session. The cost is four more arguments on `node_widget::body`, which already had the three
it needed for the rows a few lines later in the same loop.

**The grid is read-only and the strip is live.** Drawing on `euclideanrhythm`'s cells is an
editing surface of its own and is not here; silvia cannot do it either. What the region draws
is `LaneParams::of`, the same figure the tick plays, read from the same numbers. The playhead
is `CpuNode::playhead` — `None` until something has stepped, silvia's `currentStep = -1` —
reaching the region through `widgets::Live`, and the palette's drift is the same channel
filled with the palette's Time and Offset, which is what the shader reads. So neither
region keeps a clock, a cache or a copy of anything.

**Pulses follows its own lane.** `Control::num_capped` already existed for `kaleidoscope`;
`capped_ranges` now walks the hidden controls too, so a 32-step lane can be filled the whole
way instead of stopping half empty at silvia's sixteen, and the cap moves inside the same
command that moved Steps.

**Rejected: keeping the ports and adding the picture.** It is the larger node in every way —
the rows stay, the region is added under them — and it keeps the shape the parity review
called wrong: twelve rows with ports nobody cabled, under a picture that says what they mean.
The call was the one silvia makes, and a coefficient a cable can reach is worth less
than a palette you can see.

**Rejected: a `Clear` action input.** silvia's third body button zeroes the four lanes, and it
could have been a fourth `Action` row and so sequencer-firable. It is an edit to the document
rather than a gate — undoable, saved, exactly one step — and an action input that writes four
controls whenever a lane fires into it is a patch that eats its own pattern. So it is a button
in the region, `widgets::button`, and Start, Reset and Step stay the action rows they are.

### The step sequencer's pattern is a value, and the grid is Euclidean Rhythm's

**Chosen**, porting silvia's `stepsequencer`: the sixty-four cells are one `Value::Cells` on
the node — silvia's `values.stepStates`, persisted as silvia persists it, while the playhead
is runtime state as silvia's `runtimeState.currentStep` is, following the rule that
*values are saved and runtime state is not*. A lane is a string of `x` and `.`, so the file
reads as the pattern and a hand writing a `.ssw` can type one. A click is a `SetValue` of the
whole grid with one cell turned over, one gesture and so one step back; Clear is one more.

**The grid is drawn once.** `widgets::steps` paints the cells for both sequencers through one
function, and `stepsequencer` runs the transport `euclideanrhythm` ran, moved into
`nodes::sequencer` — the same Start/Stop, Reset and Step, BPM and Gate, the same gate lanes.
Sharing it moved one thing on Euclidean Rhythm: its first step is zero, as silvia's is, where
it had been one — a first Step lit the second column and Start skipped the first, which on a
grid a hand draws is the downbeat never heard.

**Rejected: four hidden controls holding a lane's sixteen bits as a number.** It would have
reused `Control::Number` and the MIDI and range machinery whole, and every one of those is
wrong for a pattern: a knob that scrubs a lane through 65,536 patterns, a range editor on a
bitmask, a file saying `4369.0`. **Rejected: a list of lists of booleans**, silvia's exact
shape, which the pretty-printed file spends sixty-four lines on.

### A Snap, a Render heading, and a throb on every action port

**Chosen.** Three changes with one thing in common: each is a fact the instrument already
knew and did not say.

**A Snap, lossless, at the Output's own resolution.** silvia's Output has four big buttons and
two of them take pictures; here there were two buttons and no way at all to get a frame out of
a running patch. The renderer is not a substitute — it is modal, its duration is set
beforehand, and it stops the performance to do its work — where grabbing the frame in front of
you is the ordinary thing to want from a node called Output. It is an action input beside the
two deck claims, so a sequencer or a MIDI pad fires it, and it is read back through the same
read a thumbnail uses: a pass, a staging buffer and a map, collected on a later
tick. **Nothing waits on the GPU**, so the answer to *what is on screen while this happens*
is the next frame, on time.

**Rejected: writing it beside the workspace file.** `workspaces/` and `assets/` are the
project's own machinery and are read back on open; a picture dropped into either would be a
file the loader has to learn to ignore. `snaps/` inside the project folder is beside the patch
that made it — the project folder is the thing that travels — and nothing loads it. The name
carries the Output and a UTC stamp, because what a name is for here is telling two apart and
sorting them, and there is no timezone database in this binary to do better.

**Rejected: Rec.** silvia's fourth button records until it is pressed again. A realtime
recorder is an encoder running beside the show at the rate the show is going, dropping
nothing, while the graph keeps its own time — machinery rather than a button, and worth having
one day on its own terms.

**The render section folds under a heading, not a tick.** A tick in the shared row at the foot
of a node is the port-visibility control: it hides runs of *ports*, and a ticked-off one says
nothing about what it hid. The bar with the disclosure triangle is on the node whether it is
open or closed, which is what says *this part of the node is here, and folded*. It is the same
change the scope and the preview already took, and it is the last of the old affordance on a
node that has headings. The option is the same one the tick was — `offline`, `on`/`off`,
undoable, saved — so a file written before the change lands where it always did.

**Rejected: making the render a region.** A heading belongs to a region, and the obvious move
was to turn the whole section into one. It is not a band: it is six rows of selects, s-numbers
and a button that each already draw through the row machinery and leave as `Command`s and a
`RenderRequest`, none of which a `RegionEvent` carries. So the node names the option in
`NodeDef::row_headings` instead and the bar is drawn over the rows by the same
`widgets::heading_row` a region uses — one function, so the two cannot drift apart by a point,
and a registry test holds every heading option to having a region or a row heading under it.

**A throb on every action port.** The firing is invisible everywhere, not only on Random Fire:
an action is a moment and a moment with nothing drawn cannot be seen. So the fix is one rule
in the port drawing — the dot brightens on the frame a firing lands and decays over a sixth of
a second, and a press button on an input fades toward the chrome a finger on it already wears.
Which ports fired is the synth's answer, one-shot on the snapshot like a deck claim, because
the edge is against the previous tick and the previous tick is that thread's.

**Rejected: a readout on Random Fire.** silvia flashes a pill in that node's body, and porting
it would have answered the question on exactly one node out of a hundred and fifty-four. What
the node keeps is the thing a throb cannot show: whether Start/Stop left it running, in
silvia's own two words and two icons, on a status line of its own.

**Rejected: silvia's own 50 ms step-and-snap-back.** `_showFireIndicator` sets a background
one token brighter and puts it back on a timer; at a port's size that reads as a glitch rather
than a pulse. What is taken from silvia is the duration of the one animation it gives an
action control — `transition: transform 0.15s ease` — with the brightening decaying across it,
and a lift the size of the one token silvia's own indicator steps.

**Rejected: throbbing only outputs.** An input fired down a cable is the same event, and it is
the end a person is usually looking at: the sequencer lane is off to the left and the node it
is driving is what you are watching.

### The Status box is a terminal pane with a verdict first, and waiting is what the CPU clock did not see

**Chosen.** One model — lines of styled spans — drawn by the window and written by the copy,
with the rate, the GPU against the CPU and the pacing on its first three lines, sections that
fold and remember it, and a hover on every line. A verdict first, because readings of the
rate, the target and the bottleneck in three places answered nothing, and Outputs listed in
id order pushed the rest off the screen. See [ui.md](ui.md#the-status-box).

**Waiting is the residue of the thread's own CPU clock.** Every lap reads
`CLOCK_THREAD_CPUTIME_ID` beside the monotonic clock, and the waiting is the tick less the
sleep less the CPU time of every phase — so the rows add up by construction, and a draw that
reads 29 ms on the wall because the driver held the thread with the GPU's queue full reads as
waiting rather than as work. The clock comes through `rustix`, already in the tree under winit
and zbus with `time` on; `libc` would be the same call as an `unsafe` block outside `render/`.

**Rejected: timing the driver's blocking calls themselves.** Which call blocks depends on the
driver and on how full its queue is — a flush one frame, a fence query or a map the next — and
a timer around each would name the call rather than the cause. The CPU clock sees every block
wherever it lands.

**Rejected: the draw's seven parts on the CPU clock.** The renderer times its parts on the wall
only, and the synth folds them whole; the draw's CPU time is one row, and where the GPU's own
time went is the GPU section's business. Splitting it would put the thread's clock into
`render/` for a figure nobody has yet needed.

**An Output is named by what it shows.** Every Output's header says `Output`, so a list of
forty-three of them by header is forty-three identical names. The row carries the label of the
node on the cable into its picture, and its workspace, and the accessibility tree still names
the link by slug and id.

**Rejected: `⧉` for the copy.** None of the four faces nor the two vendored ones has it, and a
missing glyph is a box one cell wide or none, which is exactly what the frame cannot afford.
`◰` is in Hack.

**Chosen: the box is opened from Preferences ▸ Performance**, *Show the Status box*, beside
the tick rate and the frame-rate meter. **Rejected: View ▸ Status box.**
The View menu keeps what changes what the canvas shows, the time readout and the cost strip;
the box is about the machine, as the tick rate is.

### The machine is one module per service, re-exported by target

**Chosen.** Everything outside `render/` that is not the same on Linux, macOS and Windows is a
service of `platform/`: an inline module in `platform/mod.rs` per service whose names are
re-exported from `platform/linux/`, `platform/macos/` or `platform/windows/` under
`#[cfg(target_os)]`, a line per backend side by side. A name one backend lacks is a build failure on that target; the call sites name one path
and never a `cfg`. The Linux backend is the implementation the app is built and tested on,
and the macOS one compiles to refusals that read as a machine without the device. See
[architecture.md](architecture.md#module-layering) and `proposals/platform.md`.

**Rejected: a `Platform` trait with an implementation per OS.** There is one backend in any
build, chosen at compile time, so a trait buys a vtable and a boxed handle for a choice nothing
makes at run time, and the services share no state that would want an object to hold it.

**Rejected: `#[cfg]` at each call site.** It is how the Linux-only code would have leaked into
twenty files, and a macOS build would then find each one by failing on it.

**The re-exports name `crate::platform::linux::…`, not `super::linux::…`.** `tests/rules.rs`
follows paths to files and does not model inline modules, so `super::` from inside one reads
as the crate root and the walk stops at `platform/mod.rs` — silently, which is the one way a
layering test must not fail. The absolute path walks through to the backend, and the two edges
out of it that break the graphics rule, each machine's video service asking `render/dmabuf.rs`,
are named there.

### Syphon publishes by drawing into the framework's own surface, off the synth

**Chosen**, in `proposals/syphon.md`: a server is Syphon's `SyphonServerBase`, and the renderer
draws each frame into its shared `IOSurface` itself with a picture window's blit, on a thread
of its own, publishing once the GPU has finished. The framework is vendored, arm64 only, built
from a pinned commit of `main` and linked by `build.rs`. An Output's Syphon option is saved
with the project; the mix's mark is not, as the mixer is not. See
[rendering.md](rendering.md#syphon) and [ui.md](ui.md#syphon).

**Rejected: the framework's `SyphonMetalServer`.** It copies a texture handed to it on a
command buffer, which is a second pass over a frame that needs converting to 8-bit BGRA first
anyway, a raw Metal command buffer out of wgpu, and the framework's shader library, which is
the part that breaks in source builds.

**Rejected: an existing crate** (`syphon-core`/`syphon-wgpu`, `syphon-rs`). Each brings a
second family of Objective-C crates or a device of its own, and waits on the GPU on the CPU
before each publish. **Rejected: the protocol reimplemented** with no framework; it is not a
public contract, and a change on Syphon's side would break it silently.

**Rejected: loading the framework at run time, compiling its sources into ours, or building it
in `build.rs`.** A missing framework would be a silent absence; its sources are a second
language in our build; and every clean build would need Xcode and a minute. The vendored copy
is built by `vendor/syphon/build-framework.sh` with clang rather than `xcodebuild`, because on
a Mac whose Xcode first-launch resources are stale `xcodebuild` does not start at all.

**Rejected: a frame handler that asks the client for its surface.** The framework's client
`stop` holds the client's lock while it waits for the frame queue the handler runs on, and
`newSurface` takes that lock, so the two deadlock; the loopback test hung on it. The handler
only wakes a thread of the client's own, which asks.

**Rejected: drawing a server nobody reads.** Each outlet is drawn only while
`hasClients` answers yes; a client that connects gets the next frame.

**Rejected: publishing on the synth thread.** A server is a viewer, as a picture window is, and
the synth never waits on a viewer; the publisher reads the newest `Published`, which is exactly
what a window does.

**Rejected: Syphon drawn on Linux and doing nothing.** It was: the Output's tick, the mix's mark,
the Main Input's source and the node were all there, and published nothing. None of it
should show. A machine without Syphon (`platform::syphon::available`) draws no Syphon
row, no mark, no source in the list and no node in the library, and keeps every saved value, so
a Mac's project opens whole and publishes again on a Mac; its Syphon options read as off here.

### NDI sends by reading the Syphon blit back, on the same thread, into GStreamer's plugin

**Chosen**, in `proposals/ndi.md` (route A): the publisher's thread, which sends over Syphon,
draws a sent picture into a BGRA target of its own with the same blit, copies it into a staging
buffer, and pushes the bytes into an `appsrc ! ndisink` pipeline of that sender's own once the
map lands — two reads in flight at most, a picture arriving while both are out not drawn. The
plugin is `gst-plugin-ndi`, a Cargo dependency registered statically, which opens the user's
NDI® runtime at run time; `LICENSE` carries the AGPL section 7 permission for it. Picture only.
See [rendering.md](rendering.md#ndi) and [media.md](media.md#ndi).

**Rejected: the Mac's `IOSurface` handed to GStreamer as the buffer** (route B). It saves the
staging copy on unified memory, but it is the Mac's alone and needs a pool of surfaces while
the encoder holds one; it is the optimisation to reach for if a measurement asks.

**Rejected: bindings of our own to `libndi`** (route C), which is the plugin written again —
`unsafe` FFI, discovery, timing and sound — for NDI's asynchronous send alone. **Rejected: a
crate that links the NDI SDK** (route D): the SDK on every build machine, and the licence
question the run-time loading avoids. **Rejected: the system's GStreamer plugin**: Homebrew has
it, Linux distributions may not, and the `.app` would have to carry it; the static one also
replaces a system's, so which plugin runs is never a question.

**Rejected: a Flip for NDI.** NDI has one orientation, top row first, which the blit writes;
**Transparent** means the same thing both ways, so the Output has one Alpha for it.

**Rejected: a steady declared rate with frames resampled to it.** The stream declares the rate
the synth ticks at and stamps each frame with the clock as it is pushed, so what is sent is
what was drawn, when it was drawn; an uneven stream is left to the receiver.

**Rejected: the NDI device provider at its own rank.** It answers a camera listing's
`Video/Source`, and starting it starts NDI's discovery on the network, so it is ranked out of
every device monitor's reach and started by name only where NDI sources are listed.

### An Output's ways out are rows with a status and a button

**Chosen.** Under a **Send** heading, the Output's name, then one row per way out — NDI®, and
Syphon on a Mac — each its name, a dot and one line saying what it is doing (*off*, *on air ·
warpzone*, the
sender's error, *runtime not installed*) and one button: **Send**, **Stop**, or **Get it** where
the NDI runtime is missing. Syphon's Flip sits under Syphon while it publishes, and the Alpha
both ways share once after the rows while either sends, each a choice between its two values by
name. The options are the ticks' own keys and values, so a file opens as it did. See
[ui.md](ui.md#sending-an-output-out).

**Rejected: four ticks in a row** — Syphon, Flip, NDI, Transparent, as it was. A tick says what
was asked and nothing about what happened: whether the picture is on air, under what name, or
why not. Rows with status and buttons say both.

**Rejected: the Send rows as a region.** Regions come after every row, so the status line would
stop sitting directly above the picture; the rows are drawn under a heading over rows, as the
Render section is (`NodeDef::row_headings`).

**Chosen: one name per Output, the whole of it, both ways.** A field under Send holds the name
the Output goes out under over NDI and as its Syphon server — `supersilvia Output <id>` until it
is given another — committed on Enter rather than per keystroke, since a commit restarts what is
on air. It is the Output's own across the project: a name another Output has takes ` copy`
until it is free, and past 48 bytes (room for the machine in NDI's 63-byte `MACHINE (name)`)
falls back to the default, which is the Output's number.

**Rejected: a name per way, or the node's title as the name.** Two names for one picture is two
things to keep apart for no gain, and the node's header is not the send name: renaming a node is
not what was asked.

### The XY Pad: the hand's place is the document, and the flight is the tick's

**Chosen: `padX`, `padY`, `vx` and `vy` are hidden controls the tick takes on change.** silvia
keeps the four in `values`, and a number there is a control here, so they save, undo and bind
to a knob. The tick takes each one on the tick the document changes it and flies the puck
from there in its own state, so a patch reopens with the puck at rest where the hand last put
it.

**Rejected: the tick writing the puck back to the controls.** Every write is a step of the
undo history, and a flight is sixty of them a second.

**Rejected: saving the wells.** They are silvia's `runtimeState`, and the rule is
that values are saved and runtime state is not. They reach the tick as a `Touch`, a seek's
sibling, and a preset sends its own.

**Rejected: a Clear Wells button.** silvia has one. A right-click on a well takes that well
away and a preset replaces them all, which covers it, and an action port to clear them is a
row on an already tall node. Decided 1 October.

**Chosen: the physics runs on the pad, -1 to 1, and the range ends map it onto X and Y.**
silvia integrates between Min and Max. At silvia's default range the numbers are the same;
anywhere else a preset plays the same motion across the square, the hand's controls keep one
range a knob can sweep, and moving a range end does not move the puck under the hand.

**Chosen: a region's one press may write a node's controls and its options as one step.**
`history::joins` holds a `SetControls` and a `SetOption` on the same node together, in either
order, so a preset's edges and numbers undo together. Two options, or two handles, are still
told apart.

### A failure toasts, and what is wrong is counted beside the time readout

**Chosen.** Every failure that reaches the Status box's file line also toasts, marked by a `⚠`
and an edge in the accent, and joins the problems list a fixed-width badge opens from the menu
bar. The box is off by default, so a failed save, open, import, export or render said only
there was said to nobody, and the window title's `•` was the only sign a save had not
happened. One path carries all of them — `App::fail`, or `Media::fail` for the render's
outcome that `Media` reads itself — so no failure can reach the line without reaching the
toast. See [ui.md](ui.md#the-toast) and [ui.md](ui.md#problems).

**The accent and a glyph, not red.** The palette has no red, by design, so a failure is the
accent, and the accent never says it alone: the toast wears the `⚠`, the list a word,
`error` or `warn`, and a node's flag a `!`.

**Chosen: a short queue of three, stacked, rather than one toast at a time.** A Snap's line
was written over by whatever came next, a save's or a failure's, before it was read. Stacked,
two things said in one second both stand; shown in turn, the second would wait out the first's
six seconds, and a failure is not something to say late. Three is as many lines as fit over
the canvas's foot without covering what is being worked on. The same line said again moves to
the foot rather than standing twice, and news of the editor's own — *Editor hidden* — replaces
the last such news, since `H` twice is one state and not two.

**Chosen: the badge counts what is wrong now and what was said, and only Clear forgets the
second.** A shader that failed or a camera that stopped is read fresh every frame and leaves
the list when it stops being true. A failure, the last Open's warnings and the last export's
report are events: they stay until the next of their kind, or **Clear**. That is what made
every warning of an Open reachable, where the status line had room for the first and the rest
reached only the log.

**Rejected: a status bar along the foot of the window.** The spec has none outside the Status
box, and a count needs one slot, not a row. The badge stands left of the time readout rather
than beside the meter, so its empty slot is the same width whatever the meter and the readout
show and nothing beside them moves when a count arrives.

**Chosen: a node at fault wears a flag on its header,** a disc in the accent with a `!`, from
the node's own `CpuNode::error` and an Output's shader error, gathered every tick the canvas is
drawn rather than only while the Status box is open. It takes the square the sampling warning
takes, and outranks it, because what does not work is said before what costs too much; its
hover says both. Camera, Audio In and Text needed no new method: each had an `error()` that
reached only the box.

**Chosen: ten seconds of nothing is not responding.** A camera warms up in a second or two,
and one that has said nothing for ten — a webcam wedged on its first stream, a device another
program holds — looked exactly like one warming up. Any Main Input source with no frame for
ten seconds after it opened reads *not responding*, and a camera that stops for ten seconds
after its first frame does too; a screen with nothing moving on it and a clip on a paused
playhead deliver nothing new and are not stopped, so the second rule is the camera's alone.
