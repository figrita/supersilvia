# Proposal: foundations — a shared graph, an App in parts, and less memory traffic

**Status: agreed 2026-09-22; stages 1–6 built, and 1–3 checked by hand in the running app —
MIDI, a webcam and a screen share still work.** Written from a read-only audit of the tree
(six reviewers, one per area; every finding cited here was checked against the code). The audit's GPU scheduling findings are **not** taken now: rendering every
awake Output every tick stays, for simplicity, while the structure under it is fixed. What
those findings must respect when they are taken is recorded in
[what rendering everything is waiting for](#what-rendering-everything-is-waiting-for), so
nothing done here forecloses them.

Three problems, six stages:

- **The graph is copied, not shared.** Every edit deep-clones the whole graph at least once,
  and every frame of a knob scrub clones it again to move it to the synth.
- **`App` is one object doing everything.** 84 fields, thirteen `impl App` blocks across nine
  files, and a canvas function with 28 parameters.
- **Memory traffic.** Every Output copies its whole frame once more than it needs to, every
  viewer waits on the whole tick, and camera and screen frames are converted and copied on
  the CPU before an upload that blocks the synth thread.

## 1. A graph that is shared, not copied

**What is wrong.**

- `publish_graph` sends `Box::new(self.graph.clone())` after every command that affects the
  tick ([app/mod.rs](../src/app/mod.rs), `publish_graph`). `SetControl` is such a command, so a
  scrub clones the whole graph every editor frame; at 60 frames against a 5 Hz tick, about
  twelve whole graphs are dropped on the synth thread per tick.
- `apply` takes a second whole clone for undo on any command that opens a step
  ([app/edit.rs](../src/app/edit.rs), `let before = ...`).
- The reason it must clone: `Graph` holds `OnceCell` views and `RefCell` memos
  (`ancestors`, `descendants`), so it is not `Sync` and cannot sit in an `Arc` both threads read.
- On the synth, `write_control` (MIDI, tap tempo) calls `Arc::make_mut` while `tick` holds a
  second handle to the same `Arc`, so a tap-tempo tick deep-clones the graph there too.
- Undo restores `next_id` and `next_workspace_id` with the old graph, so Add → Undo → Add
  hands out the same `NodeId` twice, against [invariants.md](../docs/invariants.md)
  ("never reused"). A re-issued `WorkspaceId` names another tab's `.ssw` and thumbnail.
- Loading is quadratic: every `insert_node` and `link` recomputes effective types across the
  graph, then `settle_effective_types` does the whole thing once more at the end.

**The shape.**

- `Graph` becomes `Sync`: its views are `OnceLock`, and the two memos either move behind a
  `Mutex` or out to the cable drag that is their only reason to exist.
- Nodes are held as `Arc<Node>` (`BTreeMap<NodeId, Arc<Node>>`); `connections` and
  `workspaces` behind `Arc` too. A clone is pointer bumps; an edit to one node clones that
  node, through `Arc::make_mut`.
- The editor holds `Arc<Graph>`; `publish_graph` sends an `Arc` clone; undo steps hold
  `Arc<Graph>`. A scrub frame then costs one node's clone and one pointer across the channel.
  No delta message is needed.
- The synth drops its local handle before a control write, so `make_mut` finds it unique.
- The id counters live outside the snapshot: undo and redo keep the higher of the two.
- The undo ring's byte budget counts what a step holds that its neighbour does not, since
  steps now share almost everything.
- A loading flag skips the per-insert type recompute; `settle` does it once.

**Done when.** A test that Add → Undo → Add yields a fresh id and a fresh workspace id; a
test that a scrub step shares every untouched node with the previous graph (`Arc::ptr_eq`);
load of the twelve-tab demo measured before and after; `./check.sh` green;
[architecture.md](../docs/architecture.md) and [invariants.md](../docs/invariants.md) say
what crosses to the synth and why it is cheap.

## 2. The Output draws where it publishes, and no viewer waits

**What is wrong.**

- Every Output draws into a temp target and then blits the whole frame into its published
  texture ([render/output.rs](../src/render/output.rs), `render`). At 1280x720 RGBA16F that
  is 7.4 MB read and 7.4 MB written per Output per tick on top of the draw: about 620 MB of
  traffic per tick over the demo's 42 Outputs, on an iGPU that shares system memory.
- One fence is placed after all Outputs and the mix
  ([render/mod.rs](../src/render/mod.rs), `published_fence`), and every viewer does
  `wait_sync` on it before it blits ([render/viewer.rs](../src/render/viewer.rs)). A
  thumbnail of a cheap Output waits for the heaviest one in the tick. This is the measured
  editor frame of 84–122 ms against 5 ms of CPU.
- A retired texture is freed after six ticks, not after its last reader. An editor frame held
  longer than that across a resize or delete can blit a freed name.

**The shape.**

- A small ring of targets per Output (three, unless the design shows two suffices). The
  Output draws straight into the next free slot; there is no temp target and no blit.
- **Feedback does not change.** Everything the synth reads — an Output sampling its own
  `frame`, and one Output sampling another's — reads the slot drawn last tick, exactly the
  one-frame delay the blit gave. The same context orders it, so no fence is involved.
- Viewers are shown only a slot whose fence has already signaled, found with a zero-timeout
  poll on the synth. A viewer never waits.
- A slot is not drawn into again while a viewer's queued read may still touch it: freed by
  the last `Arc<Published>` that names it, not by a tick count. The same rule replaces the
  six-tick retire.
- Thumbnails, Snap, taps and the offline capture read the slot that was drawn, not a fixed
  `output_fbo`.
- Per-Output back-pressure stays as it is.
- An experiment in the same stage: the synth's context at `EGL_IMG_context_priority` LOW
  (needs no privilege), measured by the editor's frame time on the demo, kept only if it
  helps.

**Done when.** Headless tests that feedback through an Output's own `frame` and across two
Outputs are pixel-identical to today's; the demo's summed `GL_TIME_ELAPSED` and the editor's
frame time measured before and after on the Intel iGPU; [rendering.md](../docs/rendering.md)
rewritten where it describes the temp target, the blit and the one fence.

## 3. Frames from cameras and screens without the CPU

**What is wrong.** `Camera::open` always asks for bytes, and screen capture goes through the
same path. Each frame is converted by `videoconvert` on one CPU thread (an MJPEG webcam also
decodes on the CPU), copied row by row into a new `Vec` by `frame_from` even when the stride
is already tight, and uploaded by a synchronous `glTexSubImage2D` from client memory on the
synth thread ([render/mod.rs](../src/render/mod.rs), `sync_sources`). A 4K screen at 60 fps
is about 2 GB/s of conversion and copying before the GPU sees it.

**The shape.**

- Screen capture asks `pipewiresrc` for DMA-BUF and goes through the existing
  `dmabuf::import`, as a decoded clip already does.
- Where bytes are unavoidable: no `videoconvert` — upload BGRx as `GL_BGRA`, or YUV with the
  conversion in the shader; a `Pixels` variant that holds the mapped GStreamer buffer rather
  than a copy; uploads through a small ring of pixel-unpack buffers.
- MJPEG webcams decode through VA-API where it exists.
- The audio scope's 512×1 texture reuses its allocation instead of a new `Arc<Frame>` each tick.

**Done when.** A camera and a screen capture play on the demo machine with the synth
thread's CPU share measured before and after; the byte path covered by a headless test with
a synthetic source; [media.md](../docs/media.md) updated.

## 4. A node names its definition, and view state leaves the graph

**What is wrong.**

- `nodes::find` is a linear string scan over about 160 definitions, called for every live
  node every tick and several times for every drawn node every frame.
- To avoid that lookup, `Node` carries copies of layout data from its definition
  (`regions`, `width`, `checks`, `headings`) and view data measured while painting
  (`value_heights`, written in `app/frame.rs` outside `apply`). All of it rides in every undo
  step and across to the synth.
- Connections are a flat `Vec` scanned whole by `source_of`, `sources_of` and `targets_of`,
  called per input per CPU node per tick and per port per frame.
- The "no egui in `graph/`" rule is a grep that `crate::widgets` walks around.

**The shape.**

- `Node` holds `def: &'static NodeDef`; `nodes::find` survives only for loading.
- Layout and measured heights live in canvas state keyed by `NodeId`.
- `Graph` keeps an incoming index and outgoing lists, maintained by `link` and unlink.
- `graph/` and `nodes/` stop naming `crate::ui` and `crate::widgets`; the rule becomes a test
  that sees through the re-export.

**Done when.** No `nodes::find` in any tick or paint path; `value_heights` gone from `Node`;
`./check.sh` green; [architecture.md](../docs/architecture.md) updated.

## 5. `App` in parts

**What is wrong.** `App` holds the document, undo, the compile cache, the plan, the synth
link and its generation counters, MIDI reconciliation, file dialogs, posters, thumbnails,
probes and costs, picture windows and the preferences UI. Every piece can reach every other.
Two costs sit inside it:

- `build_plan` rebuilds the whole plan every editor frame, clones it and sends it, whether or
  not anything changed; `live_nodes` is walked three or more times per frame.
- `after_time_travel` clears every compiled shader, so one undo relinks every Output and its
  probe — 84 links on the demo, for Outputs the undo did not touch.

**The shape.** `App` keeps the view and the frame; the rest becomes owned parts with narrow
methods:

- **`Document`** — graph, history, undo, redo, gesture, the edit serials; `apply` is its one door.
- **`SynthLink`** — the host, the snapshot, the plan and its generations, the shader and probe
  bookkeeping. The plan is rebuilt only when the graph, the session or the mixer changed.
  Programs are cached by a hash of their source, so an undo that restores an old graph finds
  its old programs; probes are built when View ▸ Costs or the header warning needs them.
- **`MidiDesk`** — every `midi_*` field and the reconciliation.
- **`Media`** — assets, posters, thumbnails, the file status, the offline render's bookkeeping.

**Done when.** No `impl App` outside `app/mod.rs` and `app/frame.rs` that touches another
part's fields; a test that an undo of a move links nothing; a test that an unchanged frame
sends no plan; `./check.sh` green.

## 6. The canvas frame gets a context

**What is wrong.** `ui::show` is about 1,530 lines with 28 parameters; `node_widget::body`
is 630 and `node_widget::controls` 760. Results come back through about ten `&mut Option`
and `&mut Vec` out-parameters, and which widget wins a click depends on call order spread
across all three. Each visible node's layout is recomputed fifteen to twenty times a frame.

**The shape.** A read-only `CanvasFrame` in, an `Effects` sink out, a `NodeCtx` per node; one
`NodeLayout` per node per frame, computed once and passed down; `show` split into phases —
view, layout, hit test, cables, nodes, popups, drag end — with one hit-test pass returning a
`Hover` that both painting and input read.

**Done when.** No function in `ui/` over 300 lines or 8 parameters; the node-layout cost in
the Status box measured before and after; the kittest suite green unchanged.

## Order

Stages 1, 2 and 3 touch different code and run in parallel, each on its own branch. Stage
4 follows 1, since both reshape `Node`; 5 follows 4; 6 follows 5. Nothing merges to `main`
until it has been seen running.

## What rendering everything is waiting for

Not taken now. Recorded so that stages 1–6 leave room for it, and so the next proposal of it
starts from these decisions.

**Feedback is first-class.** silvia is for generative art, and a feedback loop is a
simulation with its own history. A loop must keep its full temporal resolution — every
tick, every frame of it — whether or not anyone is looking at it. So:

- **Render tiers** (full rate for decks and picture windows, reduced rate or size for
  everything else) may only lower what is not part of a loop. They need **feedback
  detection**: an Output whose `frame` reaches its own upstream, directly or through other
  Outputs; a tap whose delayed uniform number closes a loop; a CPU simulation feeding a
  picture. Anything in or upstream of one of those keeps its rate and its size.
- **Automatic passes** (a texture boundary under a many-tap kernel, so its upstream is
  evaluated once per pixel instead of once per tap) need the same detection. A pass is also a
  place the picture changes: past the frame's edge a kernel would read the mirror-wrapped
  texture, not the procedural value.
- **Hand-placed boundaries already exist.** An Output between two nodes is a render to a
  texture today, and it is the general, explicit tool. The open question is whether
  automatic passes are better. The lean: automatic where the result is exactly the same (a
  kernel sampling `uv + offset` over an identity domain), with the boundary marked on the
  cable so it can be seen; the Output stays the tool for a deliberate texture — feedback,
  mirror edges, a size of its own.
- **Half-float stays** for anything in a loop: at 8 bits a slow trail quantizes to nothing
  ([render/output.rs](../src/render/output.rs), `new_target`). An Output whose `frame` no graph
  reads could be narrower — the same detection decides it, and it is a question to
  decide, not a default.
- **The clock** still caps `dt` at 0.1 s, so while a tick takes 213 ms every node that
  integrates `dt` runs at about 47% of its knob's speed and drifts against `u_time`. Capping
  only true outliers is small and independent; a fixed-rate control tick apart from the
  render is the whole fix, and it keeps one clock.

## Also found, not part of this

Small, each a commit or two, listed so they are not lost:

- A press and release that arrive between two ticks cancel before the tick sees them
  (`Msg::Press` sets a level); at 5 Hz any tap under ~200 ms is lost. The gamepad thread
  samples levels after a drain in the same way.
- Saves are a plain `fs::write` with the manifest first; a crash between files can leave a
  shared node in two files, and load silently keeps the second.
- Two MIDI knobs moving together alternate targets and open an undo step per switch.
- A clip `Player` never reads its GStreamer bus: messages pile up on every seek and errors
  are never shown.
- `Player::open` runs a discover and waits up to 10 s for preroll inside a tick; opening and
  closing every device happens on the synth thread because `CpuNode` is not `Send`.
- `automation` can adopt its old take if an editor graph lands before its `SetValue` does
  (`value_writes` has no generation guard).
