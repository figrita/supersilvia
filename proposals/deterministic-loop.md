# Proposal: the synth keeps its own time

**Status: built.** All four steps of *The order* are in. What is left below is the argument
and the measurements, kept because the shape it chose is the shape the code has;
[docs/architecture.md](../docs/architecture.md#one-clock-on-a-thread-of-its-own) and
[docs/decisions.md](../docs/decisions.md) are what describe the system; everything below
describes what was true before it was built.

Scrapped one morning on the finding that the compositor-paced loop was enough, and reopened
the same afternoon on the case it is not: a **minimized** editor stops the world. Read
[pacing.md](pacing.md) for the measurements; this is what they leave.

The requirement: *a deterministic loop timing it, not Wayland's loop.* And the case that
decided it: oscillators that cannot smooth over zero frames for a whole second, so their
trace is broken, and feedback-based patches are broken.

## What was true before

`r.draw()` — the call that renders **every active Output** — runs inside a `PaintCallback` on
a `Ui` in the **main window**, and `Synth::tick` runs at the top of the same frame. The
projector's own callback does nothing but `blit_mixer`: it shows a picture that was drawn
somewhere else.

```
winit wakes us → App::ui → Clock::tick → Synth::tick → build_frame_job → PaintCallback → r.draw → swap
                                                                              ↓
                                                  projector's callback → blit_mixer (display only)
```

The swap waits for the compositor's frame callback, and KWin stops sending them for a
minimized window. So the swap blocks, and the tick and every render block with it. Occluded
is fine — a covered window still gets callbacks, measured at zero late frames in six hundred
— and every other pacing complaint turned out to be `pre_present_notify` and a trace plotted
by sample count, both fixed without a loop. **Minimized is the one case left, and it is not
a pacing problem: it is that the world stops.**

What comes back after a minimized second is worse than a pause. One tick with `dt` clamped
to 100 ms: every accumulator advances a tenth of what it should and never catches up, so an
oscillator is permanently 0.9 s behind whatever it was in step with; a feedback patch ran no
iterations at all; the trace shows the gap and then the wave from the wrong phase. No clock
fixes that. The world has to keep ticking while nothing is showing it.

Two more things fall out of the same change, and both are wanted:

- What the audience sees stops depending on whether the editor is visible, focused,
  occluded or minimized.
- A patch's timing stops depending on which display the editor is on. A 100 Hz editor and a
  60 Hz projector currently cannot both be right; under a loop of its own each window shows
  the newest frame and neither sets the rate.

**Half of this exists.** The offline render engine already drives time from a frame index
rather than the wall clock, `Clock::set_elapsed` is how that time reaches everything, and
`FrameJob` is already a value the renderer consumes rather than a walk over `App`. What is
proposed is the live engine being the same shape, running at wall-clock rate on a thread of
its own.

## The shape: the synth thread owns the running state

Chosen over the alternatives at the foot of this file. One thread — **the synth** — owns
the clock, every `Box<dyn CpuNode>`, the published uniform numbers, colors, frames and
events, the `Renderer`, and a GL context of its own shared with eframe's. It ticks and
renders on its own schedule. The editor keeps the **document** — the graph, the project, the
history — and talks to the synth through two channels:

```
editor ──commands──▶ synth        (a Command applied, a control scrubbed, a seek, a press)
editor ◀──snapshot── synth        (what the last tick published, for the canvas to draw)
```

Every window, editor and projector alike, becomes a **viewer**: its paint callback samples
the newest texture the synth has published and nothing else. When a window is minimized its
swap blocks, its viewer stops, and the synth does not notice.

### The line the split follows

`docs/cpu.md` already draws it: what a saved file holds (`Node.controls`, `Node.options`,
the graph) against what a node computes (`Box<dyn CpuNode>`, the published values). The
document stays with the editor; the running state is the synth's. **This is a refactor with a
principle behind it rather than a new seam**, and the tick was already on the right side of
it: it reads `&Graph` and never writes it. What it writes is `cpu`, `uniforms`,
`uniform_colors`, `frames`, `actions`, and it reads `held`, `seeks`, `readbacks`,
`measured_in` and the Main Input — all running state or input to it.

What the canvas reads back from the running state is the list `ui::show` is already handed
per frame: `uniforms`, `scopes`, `playheads`, `traces`, `notes` (a node's error, status and
debug line), `costs`, `readouts`, `sampling`, `live`. That list *is* the snapshot. Nothing in
`ui/` reaches into a `CpuNode` today, so the snapshot is a struct the synth fills once a tick
and hands over, and the UI is unchanged in what it reads.

### The graph crosses as a copy, on change

The synth ticks over a `Graph` of its own, replaced whenever the editor applies a command.
`Graph` is already `Clone` — undo snapshots it — and a command is the only mutation path, so
the editor sends the command's result rather than the command: `Arc<Graph>` after `apply`.
That is one clone per *edit*, not per frame, which is the cost the earlier draft feared and
the command bus already pays for undo. A control being scrubbed is a command a frame while
the hand moves, which is what it is today too.

### `CpuNode` is not `Send`, and does not need to be

The trait says so, and it is right: a cpal stream is not `Send`. That rules out *moving* a
node's state to the synth. It does not rule out the synth **creating** it. State is created
on the first tick after a node appears and dropped on the first tick after it is gone — that
is `Synth::tick` today — so under the synth every `CpuNode` is born, ticked and dropped on the
synth thread and never crosses. The Main Input's capture and analysis move with them, since
they are the running state of one source. What the editor needs from a device — the list of
cameras, the audio devices — is gathered and refreshed only on *Look for devices*, which is
already the rule.

### The GL context, and why this stopped being KMS-sized

The earlier draft said eframe exposes nothing a shared context needs — `CreationContext::gl`
is a `glow::Context` and no glutin handle — and concluded that owning the context meant
replacing eframe's creation, which is the KMS-sized change. **That was wrong about EGL.**
`render/dmabuf.rs` already loads libEGL through `khronos-egl`'s `DynamicInstance<EGL1_5>`,
and on Wayland glutin's context *is* an EGL context. From the frame thread, while eframe's
context is current:

```
display  = egl.get_current_display()
share    = egl.get_current_context()
config   = one matching the current context's EGL_CONFIG_ID
synth    = egl.create_context(display, config, Some(share), [CONTEXT_MAJOR 4, MINOR 3, ...])
```

and on the synth thread, `make_current(display, NO_SURFACE, NO_SURFACE, synth)` under
`EGL_KHR_surfaceless_context`, which this Mesa has (`eglinfo` lists the surfaceless
platform), then `glow::Context::from_loader_function(egl.get_proc_address)`. No window, no
swap, no fork. Textures, buffers and sync objects are shared between the two contexts by the
share group, so an Output's published texture is sampled by a viewer's blit as it is now.

What sharing does **not** give: container objects — framebuffers, VAOs, programs' bindings —
are per context. The `Renderer` moves whole to the synth, so its framebuffers and the quad's
VAO go with it, and the viewer keeps only what a blit needs: a program, a VAO and a sampler
of its own, made on the frame thread. `blit_mixer` is that already, minus owning its
objects.

**Synchronization is a fence, and the fence exists.** The synth signals a `glFenceSync` after
each Output's draw — the same fence `output.rs` polls for drops — and a viewer waits on the
newest published one with `glWaitSync` (server side, no stall) before sampling. A viewer that
paints twice between ticks shows the same frame twice, which is correct and is worth stating
so it is not later read as a drop.

### The synth's schedule

One tick per display interval of the display the editor is on, from a `timerfd`-backed
sleep (`std::thread::sleep` to a deadline carried forward, re-anchored when a tick lands more
than a frame late), and **not** from any window's frame callback. The rate is a preference
with that as its default; a projector at 60 beside an editor at 100 shows every other frame
twice, and the patch is the same patch.

`Clock::tick` moves to the synth with the clock, fed from `Instant` rather than egui's input
time. `MAX_DT` stays — a stall is still a stall — but under a loop that does not stop, a
stall is a bug to report rather than a fact of life, which is what the pacing readout is for.

### The viewers wait for the compositor, and the synth does not

The world running is only half of what a minimized editor has to leave alone. The projector
and the pop-outs are eframe viewports, painted one after another by the same event loop
thread as the editor; when the editor is minimized its swap blocks in Mesa waiting for a
frame callback KWin will never send, and the whole thread blocks with it. The synth keeps
rendering into textures nobody shows, and the projector holds its last frame. **A synth
thread alone does not fix the projector.**

The fix is the call that came out earlier the same day, put back on the right side. Once a
window is a viewer that owns nothing, `pre_present_notify` is exactly what it should do:
winit gates *each window's* redraw on *that window's* compositor callback, so a minimized
editor gets no callbacks, gets no redraws, never swaps and never blocks, while the projector's
callbacks keep coming and it keeps painting. The reason the call was wrong before is that the
window's paint *was* the tick, so gating the paint gated the world; after step 4 a late
editor frame delays a blit and nothing else, which is what a viewer is for.

Two things go with it. eframe polls the event loop while a redraw is gated — the core spent
spinning in [pacing.md](pacing.md)'s second finding — and the fix is to ask for the next
repaint one display interval later rather than at once, which turns the poll into a sleep.
And the editor window's own pacing becomes the compositor's again, which it may be: nothing
that matters is timed by it any more.

**The goal, stated so it can be measured:** the editor minimized for a full minute — past
KWin's few seconds of grace — with a projector on the other display, and the projector never
stalls, the oscillator comes back where wall time says, the feedback patch has run, and the
synth's own `late` stays at zero for the run. That is step 4's done-condition, and the
bench in [pacing.md](pacing.md) gains a minimized column to hold it.

### What blocks the synth, and what may not

Nothing on the frame thread. The file dialog, a long tessellation, the project tab decoding
posters: the synth does not see them. The one thing that is allowed to cost the synth a
frame is what already does — a program linking, a render capturing — and `DropCause` names
both.

The synth must not block on the editor either, so the snapshot is a swap of two buffers under
a mutex held for the length of a `mem::swap`, and commands are a channel the synth drains at
the top of a tick. The renderer's `Arc<Mutex<Renderer>>` goes: only the synth renders.

## The order

Each is a step, and each leaves the app working.

1. **The running state as a struct. Done.** `src/synth/` holds `Synth` — the clock, `cpu`,
   `uniforms`, `uniform_colors`, `frames`, `actions`, `held`, `seeks`, `readbacks`,
   `measured_in` and the Main Input's capture — ticked on the frame thread by
   `Synth::tick(&Inputs, dt)`, with `Synth::snapshot` handed to `ui::show`. The editor keeps
   what is left, and what the tick is still handed per frame is the set of nodes an open tab
   leaves awake. The renderer's collectors write the tap readings into the synth rather than
   holding them, which is what leaves them where the renderer can stop being the editor's in
   step 3.
2. **The graph crosses as `Arc<Graph>`. Done.** `Synth` holds the graph it ticks and
   `App::publish_graph` is the one place the copy is made, reached from `apply` and from
   `after_time_travel` — the tail of an undo, a redo, a cancelled scrub, and every project
   opened or imported. One clone per command a tick could *observe*, which
   `Command::affects_tick` answers: a node drag, an auto-arrange, a collapse and a workspace
   rename cross nothing, because the tick reads no position and no layout. A scrub crosses on
   every frame of the gesture, where undo coalesces it into one snapshot and pays once — and
   the clone carries the graph's built adjacency and tick order, so that frame costs the copy
   and not a rebuild in the tick, measured on a chain of 1024 at 0.50 ms to clone against
   0.28 ms plus 0.41 ms to rebuild. `tests/published_graph.rs` holds it from every side: an edit is ticked on the
   next frame, an undo puts the old graph back, a refused command changes nothing, a move
   crosses nothing, and a hundred frames with no edit leave the same `Arc`. The two things
   step 1 left standing came with it: `Inputs` is now `live`, an `&dyn Assets`, the Main
   Input's `&MainInput` choice and `report`, and the tick closes its own reading of the
   buttons — it returns the decks a press or an event claimed, so `claim_decks` only applies
   them to the mixer. What is deferred: `Graph` memoizes its ancestor walks in a `RefCell`, so the `Arc` is an `Arc` for the shape
   rather than for sharing — step 4 is where those memos answer for themselves.
   `Synth::main_input_mut` is still reached by the renderer to drive the capture, and moves
   with the renderer in step 3.
3. **The shared context. Done.** `render/egl.rs` is the one EGL loader — the DMA-BUF import
   keeps only its extension entry point — and `SharedContext::create` makes the synth's
   context beside eframe's from the frame thread: the display and the context to share with
   asked for, the config by `EGL_CONFIG_ID`, the version and profile off glow, and
   `EGL_NO_SURFACE` under `EGL_KHR_surfaceless_context`, with no pbuffer fallback. The
   `Renderer` moved into `Synth` with it, and `Synth::render` is the whole switch — enter,
   draw, publish, leave — called from `App::ui` straight after `build_frame_job`, so the
   frame is drawn before a panel is laid out. Every window is a viewer: `render::Viewer`
   owns the blit program, the VAO and the sampler on eframe's context, and each callback
   waits on the frame's fence with `glWaitSync` and blits a texture out of
   `render::Published`. `Renderer::draw` blits nothing: a surfaceless context has no
   default framebuffer to blit into, and `FrameJob::display` is now read by the preview's
   viewer. What stayed on `App` is the compiler's half — the shaders, `needs_recompile`,
   the probes, and the walk that routes a tap's words back to its node — because each reads
   the editor's graph or the shaders built from it; they reach the renderer's CPU side
   through `Synth::renderer`/`renderer_mut`, which is the pair a channel replaces in
   step 4. `tests/headless_gl.rs` makes a shared context beside the harness's, clears a
   texture on one and reads the pixel back on the other after a fence — skipped with a
   printed reason where the harness's context is not EGL. The switch costs **0.063 ms to
   enter and 0.003 ms to leave** on Linux with an Intel UHD 770, warm, against a frame ms
   unchanged at the display's 11.77 and a cpu ms of 0.61 → 1.00, which is the draw moving inside the measured
   window as much as it is the switch.

4. **The thread. Done.** `Synth` moves to a thread named `synth`, created there with only the
   EGL context handed over from the frame thread — so every `CpuNode`, the Main Input's
   capture, the renderer and the offline render's writer are all born on the thread that owns
   them and nothing that is not `Send` ever crosses. The context is made current once and left
   current; `enter`/`leave` survive for a caller that wants it briefly and the live path uses
   neither. One tick per display interval from a deadline carried forward, re-anchored when a
   tick lands more than an interval late, fed from a monotonic `Instant`; the rate is the
   editor's monitor or the new `tick rate` preference. Commands are an `mpsc` channel drained
   at the top of every tick, and the graph crosses as a **move** — the memos' answer is that
   nobody shares one — with `App::graph_generation` counting the copies where `Arc::ptr_eq`
   used to. What crosses back is one owned `Snapshot` a tick, swapped under a mutex held for
   a `mem::swap`; the editor's every reach into the renderer became a field on it. That swap
   can skip a tick, so the three one-shots in it — the deck claims, the probes' counts and a
   thumbnail — accumulate until the editor takes them, which is the one bug this step found on
   screen and not in a test. `request_repaint_after(one interval)` replaces the bare
   `request_repaint`, and a **quarter of a second** while the viewport reports itself
   minimized, which stops eframe running passes for a window that cannot be seen and so stops
   it blocking in `swap_buffers` on a frame callback that will never come. The projector
   became a **deferred** viewport.

   **The projector was the one thing this left open**, and it is closed by
   [picture-windows.md](picture-windows.md): every picture window is a Wayland surface of its
   own on a thread of its own, so none of them is eframe's event loop's to service and a
   minimized editor is not a thing any of them can observe. The projector itself is retired —
   the mix is a picture like any other.

   **`pre_present_notify` was measured again and rejected again.** It is the right shape for a
   window that owns nothing, and it still costs what pacing.md's second finding says: eframe
   polls the event loop for as long as a redraw is held, so asking for it every frame is
   97.9% of a core against 17.2% without, on the same machine, same window, same graph. What it buys
   is the projector: the polling is what services a deferred viewport while the parent is
   idle, so without it the projector stops with the editor. A spinning core all evening is not
   a trade a performance tool makes for that — the world keeps running either way, which is
   what this proposal was for. Named in docs/decisions.md. The synth
   runs *inline* with no GL — headless, every layer-1 test, egui_kittest — through the same
   channel, the same `Synth::step` and the same swap.

   **Measured**, editor minimized for a full minute on a 100 Hz display with the projector
   open: the synth woke 100.2–100.4 times a second for every one of twelve five-second
   samples, the frame thread sat at **0.0%** of a core (against 17–29% painting two windows),
   `late` stayed at zero, and after restoring, 22,571 ticks over 3 m 45 s is 100.3 a second —
   a `perlin` at 0.5× reading 112.9 against a wall time of 225.8 s. The snapshot costs 2.8 µs a
   tick on the Pumpkin project; the one term that scales is the traces, at about 2 µs each,
   and they are gathered only while something draws them.

## The alternatives, and why not

**A lock around the graph, one owner.** Simplest: the synth locks the editor's graph each
tick. The frame thread then blocks the synth on every long lock, which is the coupling being
removed pointed the other way. Rejected.

**A synth thread with GL left on the frame thread.** The synth does the CPU half and the
frame thread does the draws it asks for. The picture still stops when the window does, so it
does not deliver the requirement. Rejected.

**Our own cadence on the frame thread — vsync off and a timer.** Measured: with nothing
throttling the client, Mesa's Wayland EGL stalls waiting for the compositor to release a back
buffer, sixteen drops a second and a bimodal interval. And a minimized window never releases
one. Rejected by the numbers in [pacing.md](pacing.md).

**KMS.** Own the page flip, no compositor at all. Still the right shape for a box that only
ever goes to a gig, and still a second windowing path with input, viewports and dialogs to
answer for. Not this; the shared context gets the requirement without it.

## Open questions

- **What rate when nothing is watching?** The editor's display, as above, or a fixed 60. A
  preference, defaulting to the display, is the proposal.
- **The readbacks.** A tap's measurement is read back on the thread that rendered it, which
  becomes the synth, and it already feeds the next tick — so this simplifies. A save's
  thumbnail is read back for the editor; it asks the synth for one and gets it a frame later,
  which is what happens now.
- **MIDI and the gesture.** *Answered, and the other way round from what this said.* The
  reader thread talks to the **synth**, which drains its queue at the top of every tick and
  applies a bound CC or note there; the editor reconciles the writes into the document, so a
  knob's undo step still closes on silence — now on the editor's next run after it.
