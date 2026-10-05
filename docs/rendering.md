# Rendering

The app draws with `render/`, on wgpu: Vulkan on Linux, Metal on macOS, Direct3D 12 on
Windows, one device for everything ([one device](#one-device)). `render/` is the renderer and what it shares with the
rest of the crate — the job types, `render::adapter`, the picture windows. `unsafe` is allowed
in `render::picture`, for the borrowed `wl_display` and the raw surface handle made on it, and
in `render::dmabuf`, for the imports through wgpu-hal, and outside `render/` only in the platform services that call a system's own libraries. `Renderer::draw` is a **safe** function: `app/` and
`ui/` never write `unsafe`.

## Zero flash

**No edit may put a frame on the projector that the performer did not ask for.** Not a black
frame, not a blink, not a stutter. The output changes when the graph says so and at no other
time.

This is a rule about *transitions*, and every transition has to name what is on screen while
it happens:

| transition | what is shown during it |
| --- | --- |
| shader recompile | the previous program, drawn with its own uniforms, until the new one has linked |
| a link that fails | the previous program, and the error goes to the status line |
| a sampler whose texture is gone — its node deleted, nothing published yet | black where it samples |
| resolution change | the previous frame, scaled into the new target |
| a GPU that cannot keep up | the newest finished frame; the tick slows |
| a dropped frame — every target held | the previous published frame |
| a frame still on the GPU | the newest one that has finished |
| an Output with nothing connected | black — *deliberately*, it is genuinely off |
| an Output claimed by a mixer deck | its own targets, untouched: a claim allocates nothing |
| an Output going idle | the last frame it drew, for as long as it is idle |
| an idle Output drawing again — its tab shown, a deck, a window | the last frame it drew, then the next tick's |
| an Output idle since it was made, first seen | its first picture, drawn once on the tick its program landed |
| the mix changing size | the new target is drawn on the frame it is made |
| a drag, with the mix at *Match viewport* | the mix at the size it had, scaled, until the canvas holds still |
| a resize with the ring at its limit | the old size stays shown until a target is free for the carry |
| a simulation's world changing size | its old picture scaled into the new one, drawn over on the same tick |
| a simulation whose kernels are still linking | its picture as it stands; the ticks wait, queued |

`tests/gpu_ring.rs` asserts the property rather than the mechanism: read the published
texture at the moment an edit lands and check the picture is still there.

When adding anything that reallocates or rebuilds, the question to answer first is *what is on
screen while this happens* — and "briefly nothing" is not an answer.

**A slow GPU slows the tick.** A draw waits for the last submission of the tick two before
it, and every submission waits until at most one of the synth's is queued behind the one
running, so the GPU cannot accumulate a deep queue of old pictures. Each
Output draws into one of at most five targets. If all five are unavailable, that Output skips the tick and
its viewers keep the newest finished frame. A render's wait for a capture can also time out.
`output::DropCause` records these two cases. See [draws and skips](#draws-and-skips).

**The frame is paced by the present and nothing else.** eframe's wgpu surface presents in
`AutoVsync`, one image per display refresh, so the frame waits for the display inside the
driver, and `App::ui` asks winit for no frame callback of its own: `pre_present_notify` makes
winit hold every redraw until the compositor answers while eframe spins waiting — see
[decisions.md](decisions.md#the-compositor-paces-the-viewers-the-synth-paces-itself).

## Per Output, per frame

`Renderer` holds one `OutputRenderer` per Output node and drives every one that
[draws](#which-outputs-draw) each frame, in the order
[the tick submits them](#the-order-a-tick-submits-in). An Output whose
input is unconnected has no shader: it is skipped rather than drawn, after one `go_dark` draws
black into a target of its own.

**An Output draws where it publishes.** It owns a small **ring** of render targets
(`render/ring.rs`), and each frame is:

1. Take a free target from the ring.
2. Record one pass of its program — a fullscreen triangle — straight into it, at the Output's
   own resolution, in an encoder of the Output's own. It becomes the ring's **latest**.
3. Submit the encoder as a submission of its own. The target keeps that submission's
   **serial** ([one device](#one-device)).

There is no temp target and no copy of the frame. A copy per Output per tick is 7.4 MB read
and 7.4 MB written at 720p — about 620 MB a tick over a demo of 42 Outputs, on an iGPU that
shares its memory with everything else — and drawing into a target the viewers are not shown
is what makes it unnecessary.

**Feedback is a one-frame delay.** Everything the synth reads is the latest target, looked up
as each draw is made: an Output sampling its own `frame` reads the latest from *before* this
frame, because this frame goes into another target; an Output drawn earlier in the same frame
is read as it was just drawn; one drawn later, as it was last frame. That is the delay a temp
target copied into a published one gives, and `tests/gpu_ring.rs` holds a self-feeding
Output and two Outputs feeding each other equal to exactly that — a temp-and-copy drawn beside
them, to the last bit of every half float over a run of frames. All of it is on the one
queue, in submission order, and nothing waits. wgpu forbids one texture as both a sampled
binding and an attachment in one pass, and the ring never asks for it.

**Viewers are shown the newest frame that has finished.** Every target keeps the serial of
the submission that last drew into it, and the synth compares each unfinished one with the
completed serial when it publishes, which asks the GPU nothing and waits for nothing. The
**shown** target is the newest whose submission has finished, and it is the one a
`Published` names. A frame drawn this tick may still be on the GPU at that moment, and then a
viewer is shown the one before it: at most a tick of latency, in exchange for a viewer that
never waits. See [every window is a viewer](#every-window-is-a-viewer).

**A target is drawn into again only when nothing can still read it**: not the latest, which the
next frame's feedback reads; not the shown one, which the next publish names; not one a
`Published` a viewer holds still names; and not one whose frame the GPU has not finished, which
is the next shown once it does. The third is a **lease** (`render::Lease`): the target keeps
one, every `Published` naming it keeps a clone, and the ring reuses a target only when its own
is the last. The fourth matters to whatever draws while its last frame is still on the GPU —
[every Output](#draws-and-skips) when the GPU is behind, and the mix — which
would otherwise draw over the frame about to be shown on every tick and show one picture for as
long as the GPU stayed behind. **Five targets is the cap** — latest, shown and one to draw
into, and two more a viewer whose frame outlasts two ticks can hold. A ring allocates them as
it needs them. Where each of the five is the latest, the shown one, unfinished or held by a viewer, that
Output skips a tick rather than writing under a viewer's read. Nothing waits for a viewer's
reads: a viewer's blit is submitted before it lets go of the lease, so a draw into that
target comes after it on the one queue — see [every window is a
viewer](#every-window-is-a-viewer). The tick two back has finished before the next draw
starts, so an Output has at most two unfinished frames, which the five hold beside the shown
one and one a viewer keeps. A target of an old size is
let go as soon as neither feedback nor a viewer can read it, finished on the GPU or not: wgpu
frees its memory once the GPU is done with it. A resize carries the picture across with a pass
into the new target sampling the old one — there is no blit in wgpu — and if it has no room
for that, the old picture stays shown at its old size until a target becomes available. A
blanking Output also waits for a safe target or clears its unleased latest target in place.

[The GPU timer](#the-gpu-timer)'s two timestamps are the beginning and the end of step 2's
pass, so what it measures is the draw and nothing else.

## Which Outputs draw

**An Output nothing depends on is a pure function of the clock.** Drawn at time *t* it is the
same picture whether or not it drew the ticks before, so skipping it while nobody can see it
changes nothing — and on the demo's twelve tabs, every one open, most Outputs are ones nobody
is looking at. What does change something is memory, and what is seen. This is the rule, and
the one place it is written out; the plan applies it (`link::plan::Rule`) and marks each awake
Output with a shader as drawing, with the reason, or **idle** (`synth::Mode`, `synth::Why`):

- **Roots.** On the workspace being looked at; on a mixer deck, which is whatever the mix
  shows, on the preview, the background or a window; in a picture window of its own;
  **sent out** — [published over Syphon](#syphon) for another app, or [sent over NDI](#ndi)
  for another machine — with nothing on screen showing it (`Why::Sent`); being captured by
  [the render](#the-render-job). A deck and a sent Output are **on air**: each wakes itself and
  everything upstream of it whatever its tabs are doing (`live_nodes`), so a closed tab never
  takes it off the air.
- **Loops.** In a cycle of data edges, delayed ones included: its `frame` reaching its own
  upstream directly, through other Outputs' frames, through a tap's reading, or through any
  mix of them. A loop is a simulation with its own history and keeps its full temporal
  resolution — every tick, every frame of it — whether or not anyone looks. One pass of
  Tarjan's strongly connected components over the awake graph finds every node in one
  (`Graph::in_cycles`, beside the order it shares the walk with).

**Pause freezes feedback.** A loop's next frame is made of its last, so drawing it again is
motion even when the clock stands still: trails would go on fading and a Star Gate on
falling while the show is paused. So an Output in a loop (`OutputPlan::feeds_back`) draws
only on a tick the playhead moved — playing, rendered or sought — whatever else would have it
draw: the tab being looked at, a deck, a window. On a tick it stood still
(`Transport::stood_still`: paused, and no seek since the tick before) it publishes its last
frame, and a seek while paused draws it once, on the tick the seek lands. A picture asked of
it — its first, a Snap, a save's thumbnail — still draws it, for that tick. An Output outside
every loop draws on while paused: its picture is a function of the clock, which stands still.
- **Frames.** An Output whose `frame` a drawing Output samples draws, and so on up.
- **Taps.** A measured node whose reading is read is measured every tick, in [its
  workspace's pass](#the-workspace-pass), and every Output whose frame that measurement samples
  draws (`Why::Tap`: *tap5 measures its frame, and something live or stateful reads it*). A
  reading is read where it matters whatever draws — the node's own CPU half integrates
  (`autoexposure`'s slew); the node is in a cycle, the reading closing a loop; it or a dual node
  a tick evaluates from it is on the workspace being looked at, whose row shows the number, or
  is read by an awake CPU node — and where a drawing Output's shader reads it or such a dual
  node, or another measured node's chain that is measured does. **Any CPU node counts**,
  stateful or not: a stateless one can still cut a deck or feed one that is not, and the
  precise answer is not worth the walk. An Output a tap is merely cabled into draws nothing on
  the tap's account: its shader holds the tap's pass-through, not its measurement.

The last two add Outputs, and an added Output adds what it reads, so the two are closed
together until neither adds one. Everything else is idle: **not drawn at all**, rather than
drawn at a lower rate — see
[decisions.md](decisions.md#an-output-nobody-depends-on-is-not-drawn).

**What an Output needs.** The same closure run from one Output alone, over frames and the
readings its shader reads, is what must draw on any tick it draws; the plan carries it per
Output (`OutputPlan::needs`), and each pass measuring a reading that closure takes names the
Output (`PassPlan::wanted_by`), so a pass draws its measurements on any tick such an Output
draws.

**Idle is not suspended.** An idle Output's nodes tick, its program is linked and relinked as
edits land, its resize happens, and it publishes the last frame it drew, so a tab shown again
shows that frame for one tick and the next tick's after — never black. **An Output with no
frame of its own to show draws one**, idle or not: until its program has drawn since its ring
was made, blanked for a render or gone dark, the renderer asks for a picture of it
(`OutputRenderer::wants_picture`), so it draws once on the tick its program lands and a viewer
switching to it is shown that frame rather than the blank ring. Nothing else about it is read
back, drawn or probed; `OutputRenderer::rest` collects what its last frame left once that frame
has finished, so its last GPU time arrives on time rather than on the tick it next draws, when
it would describe a frame long gone.

**What asks for a picture draws it for the tick it asks.** A thumbnail a save wants, a Snap,
a first picture, the render, and a deck a press, an edge or MIDI claims inside a tick are all
the synth's before the editor has seen them, so `Synth::drawing` adds each to the plan's set
with what the plan says it needs — one closure, the plan's, so a picture asked for reads this
tick's frames and this tick's readings. The frame is the one continuous drawing would have
drawn on that tick, and the Output is idle again on the next. A deck claimed for an Output on
an open workspace nobody is looking at draws on the tick that claims it. **A deck claimed for
an Output on a closed workspace does nothing:** the Output stays suspended, and the synth draws
nothing of it until the editor's next plan wakes it. The Project tab's cards are the pictures a
save wrote, not live frames, so they ask for nothing.

The Status box's GPU section counts the Outputs a tick drew, and its Outputs list keeps each
idle one's figure from the last frame it drew, muted — see [ui.md](ui.md#the-status-box).

## The mixer

[`mixer.rs`](../src/mixer.rs) is what the mixer *is* and `render/mixer.rs` is what
draws it: **silvia's `MainMixer`, and not a node.** Two decks, each an Output that claimed it
with `Show on A` or `Show on B`; a balance from −1 to +1, curved through
`0.5 + 0.5·tan(balance·π/2)` so the ends are hard A and hard B; and eight crossfade methods
ported from silvia's `MIXING_FRAGMENT_SHADER` — simple mix, horizontal wipe, vertical wipe,
radial wipe, dark-first luminance, light-first luminance, checkerboard, horizontal lines.

**The five that sweep across a screen are the mixer's alone.** The `mix` node keeps the plain
mix and the two luminance fades, which are a function of the color under the fragment; a
wipe needs a frame to sweep over, and no node body may read the one it is compiled into —
[decisions.md](decisions.md#no-node-reads-the-resolution). The mixer has a frame of its own,
which is the screen.

**The mixer's pipeline is the one program that never recompiles.** It is made once, with the
renderer, and it is written against two textures rather than any graph. An edit to either
deck's graph rebuilds that deck's Output, and undo and redo rebuild every Output; none of that
reaches the mixer, because nothing about it depends on what is upstream. That is the whole
reason it is its own render target: a performer builds, undoes and relinks deck B while deck A
is on air, the way a DJ cues the next record on the deck that is not playing. The crossfade
method is an `i32` in its uniform block — all eight branches are in the one module, and
picking another changes a number and nothing else.

Per frame, in the tick's last submission after every Output and the probes: a bind group
naming the two decks' latest views — an empty deck, or one whose Output has no program, names
the shared black texel — and one fullscreen pass into a free target of the mix's own ring,
`Rgba8Unorm`. The decks are read the way feedback reads an Output, on the one queue behind
the submissions that drew them, so the mix is of the frames just drawn and never a frame
behind them. There is no back-pressure: the pass is trivially cheap, and a mix is skipped
only when every target of its ring is still held by a viewer or still on the GPU — or when
neither deck has a picture: two empty decks are black whatever the fade says, so once one black
mix is drawn nothing is drawn until a deck has something on it, and a resize meanwhile carries
the black across. A mix skipped for want of a target is a drop, counted as an Output's are
(`Renderer::mixer_drops`) and added to the `held` of `RenderReport::drops`;
one skipped because both decks are empty is not. What a viewer is shown of it follows the
Outputs' rule — the newest mix whose submission has finished — blitted wherever the mix is shown: the
preview panel, letterboxed.

**Each deck is scaled to the mix's height about its center, silvia's way.** A deck wider
than the mix is cropped at the sides; a narrower one is mirrored out to them, which the
Outputs' mirrored-repeat wrap gives for free. The mix fills the projector, whatever shape the
decks are; it is the *mix* that is letterboxed into a panel or a window, never a deck into
the mix. `tests/gpu_mixer.rs` reads a 1:2 deck across the full width of a square mix.

**A claim allocates nothing.** Going on air is the worst moment for a flash, so `Show on A`
makes no texture and no view: the Output's targets and the mix's are the same ones before and
after, and the mix samples the deck on the frame it was claimed. The test asserts both sets of
textures and the pixel. When the mix changes size, the frame that reallocates it is the
frame that draws it, and the previous mix is scaled into the new target first for the same
reason an Output's resize does it; its old size's targets go as an Output's do, as soon as
nothing reads them.

**A mix at *Match viewport* takes the canvas's size once the canvas has held it for a quarter
of a second** (`app::frame::MIX_SETTLE_S`). A window or panel drag changes the canvas's size in
pixels on every frame, and a mix that followed it would reallocate on every frame and replan
with it; this way a drag resizes the mix once, where it stops, and the mix meanwhile goes on at
the size it had, scaled into the preview as any mix is. The first size a canvas has is taken at
once, so a run does not start with a resize.

**The fade stops short of the pole.** `tan` at exactly ±π/2 in 32-bit float lands on
whichever side of the pole the rounding put it, and the sign flips: hard B came out as hard A.
Both the shader and `mixer::mix_amount` clamp the balance to ±0.999 first, which is already
hundreds of times past the unit range and never in doubt about its sign.

**Blackout and Freeze are the mix's, after it is drawn and before anything shows it**
(`MixerJob::blackout`, `MixerJob::freeze`), so the preview, the mix behind the canvas, a
picture window, NDI and Syphon all show the same held picture. Freeze draws no new mix: the
ring's newest finished target goes on being published, and every viewer goes on blitting it.
Blackout publishes, in the mix's place, one opaque black texel the mixer made with itself,
named at the ring's own size so a window keeps its shape and a sender its resolution, and
stamped with the tick Blackout began on so an outlet sends it once; the mix goes on being
drawn underneath, so letting go shows the mix as it stands. Both held is black. A mix
published after Blackout is let go is stamped no earlier than that tick, so the frozen frame
shown again is newer than the black to an outlet that sends only newer frames — NDI. Neither
allocates anything when it is pressed: the texel is the mixer's from the start, and the ring
is the ring. See [decisions.md](decisions.md#blackout-and-freeze-are-the-mixs-and-every-way-out-shows-them).

An Output on a deck with nothing plugged in publishes black, as any unplugged Output does,
so unplugging the live deck's input blacks the projector. That is the DJ pulling the record
off, and it is deliberate; the mixer holds nothing on an Output's behalf.

## Draws and skips

Every Output in the [draw set](#which-outputs-draw) is asked to draw each tick. Before any of
them starts, the renderer waits for the preceding tick's last submission — `Gpu::wait` on its
serial, bounded by `QUEUE_WAIT` (one second). Within the tick, each submission waits for room
in [the throttle](#one-device), so the synth never queues far ahead of the GPU. Under a GPU
load the tick rate falls with the GPU's throughput, for the whole graph. The synth's deadline
re-anchors on a late tick, and `Clock::tick` integrates the elapsed time. A loop keeps one
step per tick that its Output draws.

**A frame skips only where its ring has no free target.** The latest target is kept for
feedback, the shown target is kept for viewers, and a `Published` held by an older viewer
keeps its target leased. A frame still being drawn is also unavailable. At the fixed limit
of five (`ring::RING`), the Output reports `held` and leaves its previous finished picture on
screen. A capture waits for its own prior frame because a film cannot have a hole; a
two-second timeout reports `capture`. The mix follows the same target rule and contributes
its skips to `held`.

Each frame has a record of `readback::Readbacks` — the frame's count in the ring, the source
that was wanted and, for a program with tap slots, a staging buffer — and its GPU time a pair
in its tick's range of the timer's ring. Each is collected on the first tick that finds its
map landed.

**Two ticks may be on the GPU, and one submission queued.** `TICKS_IN_FLIGHT` is two, so a
tick's first submissions go in behind the end of the tick before rather than after the GPU has
run dry; what bounds the queue in practice is the throttle, which keeps at most one of the
synth's submissions behind the one running whichever tick it belongs to. So the age of an
Output's newest finished picture is at most two ticks while it keeps drawing, and measured it
stays where one tick in flight had it — median, p90 and maximum 0 or 1 on every Output of the
demo's tabs, with no drop. `tick_bench` prints the shown frame's median, p90 and maximum age
in ticks beside each Output's draw and drop rates. The second tick bought the heavy tabs 7 to
10% more ticks a second and the render engine's busy share from about 92% to 99%
([proposals/wgpu.md](../proposals/wgpu.md), the headless figures). Under glow, with a
three-target ring and no throttle, a queue of two dropped visible Outputs unevenly under load,
which is why the queue was one tick there.

## The order a tick submits in

**A tick submits in the plan's order** (`Synth::job`): the graph's dependency order, with what
`Synth::drawing` adds inside the tick in its own place in it. The GPU runs the synth's
submissions in the order they are made, and that decides which frame a read sees: an Output
sampling another's `frame` sees this tick's picture where the producer comes earlier and last
tick's where it comes later. The graph's order puts every producer before its consumers, and a
whole loop after everything feeding it and before everything it feeds (see
[architecture.md](architecture.md#delayed-ports-and-feedback)), so a read sees last tick's frame
only inside a loop, where the delay is the point. Nothing the tick draws is dropped, so the
order decides nothing about what is drawn.

**A measurement orders nothing.** A measurement samples whatever its chain reads, another
Output's frame among it, but it runs in its workspace's pass, and every pass is drawn [in the
coda, after every Output](#the-workspace-pass). So the plan orders its Outputs by the cables
alone — `Graph::topological_order` — and a measurement reads every
frame as this tick drew it, whichever Outputs its reading feeds. A loop through a frame and a
tap's reading is a loop of data edges like any other: the reading is a delayed port, so the
Output it feeds reads last tick's measurement, as anything in a loop does.

`a_consumer_reads_its_producers_frame_of_the_same_tick` holds a consumer to its producer's
frame of the same tick, to the bit; `every_frame_a_shader_samples_is_drawn_first_outside_a_loop_in_random_graphs`
holds the order to every frame a shader samples outside a loop, over 3000 graphs;
`a_tap_on_a_frame_orders_no_output` holds that a measurement adds no edge; and
`a_consumer_and_its_producer_draw_every_tick_on_a_saturated_gpu` holds that neither misses a
tick while the GPU is behind.

## Precision

Render targets are **`Rgba16Float`**.

It matters specifically for feedback: at 8 bits a small mix amount rounds to zero, so a trail
dies abruptly and leaves dead zones where a channel never moves at all. Half floats also carry
values above 1.0, so additive and glow chains do not clip until the final blit to the window.

What reaches the screen is still 8-bit — the window framebuffer is. The precision lives in
the intermediate and feedback chain, which is where it is needed.

`Rgba32Float` is available and not used: it doubles memory again for precision no one can see in a
decay.

## Alpha

**Every color in the graph is premultiplied** — a node's return value, a source texture, an
Output's frame — and straight colors are converted once at the edge they come in or go out
by. The decision and what it rejected is in
[decisions.md](decisions.md#colors-in-the-graph-are-premultiplied).

| Boundary | Where | Conversion |
| --- | --- | --- |
| Color controls and CPU-published colors | the synth's uniform resolution | premultiplied |
| Straight pictures: images, GIFs, the drawing canvas | upload | premultiplied |
| Simulation state, whose alpha is data | — | none; not a picture |
| A viewer: node bodies, panels, picture windows | the blit, over black | none; blends premultiplied |
| Snap, PNG sequences, GIFs, the project card | readback | unpremultiplied |

**Linear filtering is why.** A sampler averages texels before the shader sees them, so a
straight texture's transparent texels lend their color to every edge; premultiplied, they lend
nothing. The prelude's `unpremultiply` and `premultiply` are for the nonlinear operations
inside a node, and `unpremultiply` of a transparent color is transparent black.

## Texture wrapping

**Every sampled texture is read through `MirrorRepeat`, not `Repeat` and not
`ClampToEdge`.** A sample outside `[0,1]` reflects the picture back in rather than tiling it
or smearing its edge — silvia's wrap mode, and a principle supersilvia adopts wholesale: a camera or a
video frame of one aspect in an Output of another mirrors out to the sides instead of
letterboxing, [the mix's own deck-fitting](#the-mixer) gets the same for free, and a node that
moves a sampling coordinate past the edge of its input — a wipe, a distortion, a feedback
chain's own edge — reads a reflection rather than a seam. **In wgpu the wrap and the filter
belong to a sampler, not to a texture**, so the renderer makes four once
(`render/shared.rs`) — mirror or repeat, linear or nearest, in
`compile::wgsl::Sampler::ALL`'s order — and binds all four into every group that has a
texture. A module reads each texture through the one its output declares, an Output's frame
and every CPU node's published frame alike — a camera, a video file, the oscilloscope — and a
viewer blits a picture through the one its `Picture::sampler` names. `tests/gpu_upload.rs`
samples a texture outside `[0,1]` against known texels and asserts the reflection, not a
driver default trusted by inspection.

**A texture output may declare otherwise, and two do.** `OutputDef::wrap` and
`OutputDef::filter` are `Mirror` and `Linear` for every output in the library but
`cellularautomata`'s `cells` — `Repeat` and `Nearest` — and `slimemold`'s `trail` — `Repeat`.
Both are simulated worlds that wrap: a glider off the right edge of the grid comes back on the
left, an agent off the bottom comes back at the top, and the neighborhood counts and the
diffusion read across every edge. Their pictures are sampled at raw worldspace `uv`, so the
field tiles across the world — and a *mirrored* tiling would fold a seam down the middle of
every second copy, drawing a discontinuity the simulation does not have. The picture would lie
about the world. `Nearest` is the same argument one level down: a cell is a cell and not a
sample of a smooth field, so blending two of them draws something the automaton never
computed. A scent field *is* smooth, so `slimemold` keeps `Linear`.

Nothing in the renderer reads the registry ([the layering
table](architecture.md#module-layering)). The compiler names the sampler in the module it
writes, and the app copies both fields into the frame job per published texture —
`SourceJob::wrap` and `SourceJob::filter` — for the `Picture` a viewer is handed. **A change
of either picks another sampler, not a reallocation**: the texture is untouched, so a
declaration can never cost a frame — *zero flash*. `tests/gpu_upload.rs` reads a frame
through both declarations, against the mirrored order and the tiled one, and asserts the
texture and its pixels are the same across a change of wrap.

## Source textures

A CPU node's frame reaches the GPU through `FrameJob::sources`: a `SourceJob` per published
port, carrying the `Arc<Frame>` — rows top first — beside [how it is
sampled](#texture-wrapping). `Sources::sync` (`render/sources.rs`) puts each one into a
texture kept per port and lets the texture go when the node is gone. A frame is one of three things
(`Pixels`): a CPU node's own packed RGBA bytes, a source's buffer mapped in the layout the
source made it in, or a DMA-BUF.

**An `Arc` already uploaded is skipped, by pointer.** A camera at 30 Hz publishes the same
frame for two ticks out of three at 60 Hz, and those ticks cost nothing here. The texture
remembers an uploaded frame by a `Weak`, so the buffer behind it goes back to the source as
soon as the upload is queued, while the allocation the `Weak` keeps stops another frame
arriving at the same address. A texture that is new, or replaced — a first frame, a size or
format change, the first bytes after an import — is made and written in the same tick, before
the submission that samples it, so there is no frame on which the texture is empty, and bytes
never go into an import's memory. **Nothing changes until a frame is known to fit**: every
plane's length and stride is checked first, so a frame that does not fit leaves the texture
holding the last one that did.

**Bytes go up as they lie, through `queue.write_texture`**, which copies them once into
wgpu's staging and queues the copy ahead of the next submission. That copy is the only CPU
work on the synth thread. A padded row is `bytes_per_row` equal to the source's own stride,
never a repack — `write_texture` has no 256-byte row rule; BGR is a `Bgra8Unorm` texture,
which samples as RGBA. **An `x` byte goes through the conversion pass below**, which writes
alpha one: wgpu has no alpha swizzle. It costs one pass per new frame, for `x` layouts only.

**YUV is converted by a pass** (`sources::CONVERT`, recorded into the tick's first
submission). Each plane goes into a texture of its own — a byte, a pair or a packed 4:2:2 pair
of pixels a texel — kept per source and reallocated only when a plane changes shape, and one
fullscreen pass writes RGB with alpha one into the source's `Rgba8Unorm` texture with the
matrix and range the caps named (BT.601, 709 or 2020; studio or full). Chroma is sampled
linearly between texels, which is the upsampling. What a node samples is then an RGBA texture
like any other: nothing in `compile/` or in a node's shader knows a camera was YUV, and a YUV
source costs one extra pass per new frame rather than a conversion in every shader that reads
it.

**A surface its producer draws into again goes through the same pass**, BGRA included: a
Syphon server's, marked `nodes::Redrawn` ([media.md](media.md#syphon)). The pass is the copy
into the source's own texture that keeps a frame from being sampled half drawn; its fourth
byte is kept as alpha (`Layout::Bgra`, the pass's fifth arrangement) or written one
(`Layout::Bgrx`), and the pass reads the planes bottom row first where the surface is laid out
so (its `flip`), so the source's texture is top row first like every other.

A decoded video file and a screen cast pay no upload at all where the machine can export and
import; their frames arrive as descriptors and go through [DMA-BUF import](#dma-buf-import).

A camera node publishes a 2x2 black frame until its first real one, so the sampler always has
a texture. That black is the source being genuinely off, the same as an unplugged Output.

## Simulations

**A world too big to step on the CPU is stepped here, by kernels its node wrote.** `slimemold`
is the one there is: seven thousand agents over a scent field at its defaults, which on the CPU
cost the synth thread 3 to 6 ms a batch — see
[decisions.md](decisions.md#a-simulation-steps-on-the-gpu-in-kernels-its-node-writes). The
node's tick stays on the CPU and publishes a `nodes::Simulation` through
`TickContext::publish_sim`: the world's shape, the numbers its kernels read, and this tick's
**passes** — each a compute kernel of the node's own, a `static` holding its WGSL, with the
agents it visits, a seed and one argument. The synth keeps the newest per port and
`FrameJob::sims` carries it, the passes taken out so each runs once. Nothing in the renderer
knows what an agent is: `render/sims.rs` is a runner over a fixed set of resources, and
the rules are the node's.

**One `World` per published port**, made on the first job that names it and let go on the
first that does not; a `Published` naming its picture keeps that texture alive through its
view. It holds, in group 0:

| WGSL name, binding | what |
| --- | --- |
| `agents`, 0 | a `vec4f` an agent |
| `state`, 1 | sixteen words a kernel reduces into — a peak, a running maximum |
| `field`, `field_next`, 2 and 3 | `R32Float` storage textures: the field a pass reads, and the one a flipping pass writes |
| `arrivals`, `arrivals_next`, 4 and 5 | a counter per cell, for `atomicAdd` |
| `picture`, 6 | an `Rgba8Unorm` storage texture, what every consumer samples |

Every kernel's module is the node's WGSL after a prelude declaring them, with one uniform
struct `u` in group 1 — `u_size`, `u_agents`, the pass's `u_from`, `u_to`, `u_stride`,
`u_seed` and `u_arg`, then an `f32` per number the simulation carries, by name — packed for
every pass of the tick into one buffer and bound at a dynamic offset. A kernel runs over the
agents, over the cells, over the cells in 8x8 workgroups that may share memory — the entry
point calls it for every invocation, so it may `workgroupBarrier()` — or once; one that
**flips** takes the world's other bind group next, made once beside the first with each pair
swapped, which is how a diffusion reads one field and writes the other with no copy. **No
barrier is written by hand**: wgpu places one between two dispatches that touch the same
storage resource, and between the last dispatch and an Output pass sampling the picture.

**Where it sits in the draw.** After the uploads and before the first Output, in the tick's
first submission: `Sims::sync` makes, lets go of and reshapes the worlds and records their
passes. A world's picture is then bound to a consumer exactly as an upload is — the views of
both are one map a draw looks its textures up in — and published to viewers the same way,
under a view. It is drawn in place every tick, as an upload is written in place, and a
viewer's blit is ordered against it by the one queue, so a viewer sees the picture before or
after a tick's passes and never between. **Nothing is read back and nothing is uploaded**:
the picture never leaves the GPU, and the only CPU work is the tick and the submission.

**A kernel links off the thread.** Creating a compute pipeline compiles it before it returns,
so the first pass naming a kernel sends it to a thread of its own, `sim-linker`; one pipeline
per kernel and the numbers it is handed, for the life of the renderer. A tick whose kernels
have not linked is **queued** with the rest, in order, and run when they have — so a world
steps late on its first frames and never skips a step, which is what keeps the same clock
making the same world. A kernel that fails to link empties the queue and is the node's status
line.

**A change of shape reallocates before the passes that asked for it.** A new size is a new
field, resampled nearest-neighbor from the old one by a compute pass (an `R32Float` texture is
not filterable, so it is read with `textureLoad`), zeroed counts, and a new picture the old one
is scaled into by the resize carry's linear pass — *what is on screen while this happens* is
the old picture scaled, and then the new one drawn on the same tick. A new population keeps the first `min(old, new)`
agents and zeroes the rest; the node throws the new ones itself.

**Deterministic by construction.** Randomness in a kernel is a hash of the invocation and the
pass's seed, which the tick draws from the node's own seeded generator; arrivals are integer
counts, whose sum is the same in whatever order the GPU ran the agents; and how many steps a
tick runs is the tick's, off the one clock's `dt`. `tests/gpu_sims.rs` and `tests/gpu_app.rs`
run one world twice through an uneven `dt` sequence and hold the agents, the field and the
picture equal to the last bit, and a world reset mid-run to a fresh one.

**The counts are on a buffer, not an `r32uint` image.** Agents crowd into the veins they make,
so many land on one cell in one step, and Intel's typed image atomics serialize under that
contention: a step's dispatches cost 73 µs each with them and 7 µs with untyped buffer atomics,
measured on the UHD 770 through GL. And **the diffusion reads its neighborhood once**, into
`var<workgroup>` memory, rather than nine times per cell: 132 µs a pass down to 60, measured
the same way.

## DMA-BUF import

`render/dmabuf.rs` imports on Vulkan, through wgpu-hal: `texture_from_dmabuf_fd`,
wrapped by `Device::create_texture_from_hal`, behind `Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF`,
which the device asks for where the adapter offers it. **Vulkan takes ownership of the fd it
is handed, and the fd is the producer's**, so it is `dup`ed first; the producer's own stays
open and a frame can be imported again. This module's `unsafe` is the `dup`, the hal import,
the wrap and two Vulkan queries; beside `render::picture`, it is one of the two modules the
crate root's denial is lifted for.

**What is importable is asked of Vulkan**, per fourcc (`XR24`, `AR24`, `XB24`, `AB24`, the
byte orders of 8-bit RGB a compositor shares a screen in): its DRM format modifier properties,
then the image format properties of exactly the image the import makes, so a pair is offered
only if the import would take it — which is what a screen cast offers the compositor. **Only
a one-plane modifier is offered**, since the import passes one plane's layout; that keeps out
every compressed tiling, whose metadata is a plane of its own, and it is also what keeps the
producer's pixels through the first barrier on the image, which wgpu can only make from
`UNDEFINED`. Each frame is checked against the answer before the driver is called, since an
image made with a modifier the device does not support is invalid usage rather than an error.
`dmabuf::imports()` and `importable_here()` answer on the device the renderer serves, and are
false and empty before a renderer exists.

**An `x` byte is padding, and wgpu has no alpha swizzle**: `XR24` and `XB24` import as
`Bgra8Unorm` and `Rgba8Unorm` whose alpha is whatever the producer left there, so such a frame
goes through [the conversion pass](#source-textures) into a texture of the source's own, alpha
written one.

**A frame goes back to its producer once nothing on the GPU can still read it.** That waits
for two things: the renderer has let go of it — replaced by the next import, by bytes, or by
the node leaving — and no viewer still holds a `Published` naming its texture, since a
viewer's blit may be submitted after the synth's next submission. So a `Published` carries a
`dmabuf::Held` claim per imported frame a source samples; whichever claim goes last puts the
frame in an inbox, from any thread; the next prelude submission stamps what the inbox holds
with its serial, and is made even with nothing else in it; and the frame goes once that
submission has finished, which on the one
queue is after everything submitted before it, the viewer's blit included. However many ticks
are in flight, the producer never writes into memory a draw or a blit is still reading.
**An import that fails changes nothing**: it is an `Err` from the call, the texture keeps the
frame it had, which stays up while the source falls back, and the frame's `refused` flag is
raised so its source goes back to bytes.

**macOS has no DMA-BUF; its frames are `IOSurface`s, and the same file imports them on
Metal.** A `Pixels::IoSurface` is the surface as an integer beside the same memory mapped as
bytes; `Imports::import_surface` makes one Metal texture per plane over the surface's own
memory with `newTextureWithDescriptor:iosurface:plane:` — NV12's luma as `R8Unorm` and its
chroma as `RG8Unorm` at half size, BGR as `Bgra8Unorm`, shader-read and shared storage — and
hands each to wgpu through `wgpu_hal::metal::Device::texture_from_raw`, with no drop callback,
and `create_texture_from_hal`. The plane count, each plane's size and the surface's pixel
format are checked before Metal is called, a pixel format of zero read as BGRA, which a Syphon
server built before October 2025 leaves its surface in. BGRA is then the source's texture where it lies;
NV12, and BGR whose fourth byte is padding, are the planes of [the conversion
pass](#source-textures), drawn with the frame's own matrix into a texture of the source's own.
**The return rule is the same**: the frame holds the decoder's buffer, which holds the
`CVPixelBuffer`, which holds the surface, and a frame goes back under the claims and the serial
above. **An import that fails uploads the bytes**, which on a Mac are the same memory mapped,
so there is no flag to raise and no pipeline to reopen; the renderer says so once for that
source. The `unsafe` is the `IOSurfaceRef` rebuilt from its integer, the hal device, and the
texture made from raw and its wrap.

**Windows has neither; its frames are Direct3D 12 textures, and the same file imports them on
Direct3D 12.** A `Pixels::D3d12` names each texture a decoder's or an upload's sample is in, the
array slice the frame is in, and the fence its writes are signalled on.
`Imports::import_d3d12` wraps each texture whole with `wgpu_hal::dx12::Device::texture_from_raw`
and `create_texture_from_hal` and views its planes — NV12 as wgpu's `NV12`, viewed as `R8Unorm`
and `RG8Unorm` at its chroma's half size, as a Mac's planes are, or NV12 in a texture per plane
where the producer's device holds no NV12 texture, or RGBA or BGRA — each view at the frame's
array slice, since a decoder may hand out a slice of a texture array. **The renderer's queue
waits on the producer's fence**: wgpu-hal's `add_wait_fence` stages a `Wait` ahead of the next
submission on the one queue, so the conversion pass never samples a texture its producer's queue
has not written, and nothing waits on the CPU. The pass always draws the frame into a texture of
the source's own, and a transition after it hands each texture back in the common state
GStreamer's queues expect of it. **The texture is the renderer's device's**: GStreamer is
handed the renderer's adapter and Direct3D 12 hands a process one device per adapter, so its
device is the renderer's; a texture of another device is opened on the renderer's through the NT
handle it comes with, and one with none is refused before the driver is called. The checks
before wgpu is called are the texture's own description — two-dimensional, one level and one
sample, the DXGI format its layout names, the slice within its array and its size at least the
frame's. The return rule is the same, and an import that fails raises the frame's `refused`
flag, so a camera goes back to bytes. NV12 is `Features::TEXTURE_FORMAT_NV12`, which the device
asks for on Windows where the adapter offers it. The `unsafe` is the resource and the fence
rebuilt from their integers, their COM calls, the hal device and queue, and the textures made
from raw and their wrap.

## Tap buffers

A renderer whose program claims tap slots — a [workspace pass](#the-workspace-pass), which
measures, and a [probe](#the-cost-probe), which counts — gets one storage buffer of
`slots × TAP_WORDS` words, a template it is reset from, and a staging buffer for every frame
the GPU can hold — `READBACKS`, `TICKS_IN_FLIGHT` and the one being drawn — each in that
frame's **record** (`render/readback.rs`) beside the frame's count in the ring and the source
that was wanted when it was drawn. Per frame, all in the renderer's own encoder: copy the
template into the tap buffer, draw with it bound, copy the words into the oldest record's
staging buffer. On the one queue these run in order, so the next frame's reset cannot overtake
this frame's copy. **A pass with thumbnails resets only its tap slots** (`Program::reset`,
through `Readbacks::tap_buffer`): its thumbnail words lie past them and each is written whole
by its own fragment, so a frame that draws only the measurements' square leaves the thumbnails
the last full frame wrote rather than the template's zeros. Every other program resets the
whole buffer.

**The tap buffer is in the draw's own bind group**, at binding 1, and it is the buffer of the
program drawing. A program keeps the tap slots of its own source, as it keeps its uniforms
([linking](#linking-happens-off-the-synth-thread)), and the buffer is sized for the program
drawing. wgpu has no global binding state, so the program before, drawing while its
replacement links, writes its taps into its own buffer and can never write into another
renderer's or into a simulation's agents: there is nothing left bound from before. A program
that writes no taps binds none.

**A reading is delivered only from the program the job names.** The synth maps a reading's
words to nodes through its plan's layout (`PassPlan::tap_nodes`), which is the layout of the
source its job names. A frame the program before drew — in flight when
the source changed, or drawn while the new one linked — is in the old layout, so its record is
collected, its GPU time counted and its words dropped (`Readbacks::take_taps`). A link that fails leaves the old
program drawing and no reading delivered, so every tap in it keeps its last value until a
source that links arrives.

**What writes into a slot is a measurement, not the pass-through.** A pass's `fs_main`
computes `let cell = vec2i(frag_coord.xy)` from `@builtin(position)` and calls each measured
node's `{slug}{id}_measure(p: vec2f)` under `cell.x < N && cell.y < N`, at the cell's center in
the unit square; a `sample`'s is one call at cell `(0, 0)`. So **a tap's grid is a fixed `N²`
evaluations of its input per frame** — 16,384 at the default 128 — however large the frames it
feeds are and however many times their pictures evaluate the tap's pass-through, which writes
nothing.

**The measurements have the target's corner, sized to fit.** Every measurement in a pass runs
in the square of side `Shader::grid` at its corner — the largest tap's grid, 1 for a lone
`sample`, 0 with none (`Shader::tap_region`) — and the thumbnail tiles lie to its right,
from `x = grid`. `compile::pass_size` makes the target wide and tall enough for both, so every
cell of every grid has a fragment to run it, at any grid and in any test.

`TAP_WORDS` is fifteen. A `tap`'s slot spends them on a count, four sums — the measured
quantity, the two coordinate sums and the weight — and the two extremes. **A sum is 64 bits
across two words**: the low word's `atomicAdd` returns what it replaced, so a wrap is visible
and costs one further atomic on the high word, and nothing but a wrap costs anything. A
`sample`'s slot uses the first five words for a flag and a color. Three more words hold the
tap's **mean color**, one per channel: a channel needs neither the range nor the precision a
measured quantity does, so it is clamped to `tap::COLOR_MAX` and kept to
1/`tap::COLOR_SCALE`, and the two are chosen so the largest grid — 256×256 points at the top
of the range — is exactly 2^31 and cannot overflow one word.

The template is zeros but for the minimum's seed, which is all ones. The extremes are keys
rather than floats — a negative float's bits inverted, a non-negative float's sign bit set —
so all-ones is the largest key a minimum can be seeded with and zero is the smallest a
maximum can be. Neither seed is a key any finite sample produces, and a slot no fragment
wrote decodes to zeros off its count.

**A read is a staging buffer and a map, collected when its callback has fired, never waited
for.** wgpu has no persistent mapping of a buffer the GPU writes: a buffer the CPU maps may
only be a copy's destination, and it must be unmapped whenever a submission uses it. So once
the frame's submission is made the record's staging buffer is asked for a map
(`map_async`), whose callback only stores into an atomic; before each draw, and on each tick a
renderer does not draw, every record whose map has landed is read, oldest first and up to the first
still on the GPU, and the newest is the reading. The words are copied out, the buffer
unmapped, and only then is it a copy's destination again, which a later draw makes it. **A
record is read only once its own submission has finished**, whatever became of the target
its frame went into — a frame [blanked in place](#per-output-per-frame) at the ring's limit, or
a target of an old size let go while the GPU was still on it — so a frame is never taken for
finished because its target has gone.

**This is why each Output is a submission of its own.** The GPU knows completion per
submission, not at a point inside one. If a tick were one submission, every Output's frame, tap
words and GPU time would land when the heaviest had finished — measured, on Mesa's `iris` GL
driver, where a buffer is busy per submission too, at the length of one heavy frame behind a
finished one, every tick. With a submission per Output, a map completes when its own
Output does, and waits for nothing queued behind it.
`a_finished_frames_readbacks_are_taken_without_waiting_for_the_queue_behind_it` in
`tests/gpu_ring.rs` holds that a finished frame's words and thumbnail are taken while heavy
frames queued behind it are still running; how long the reads took is printed, and not held,
since the tests share the GPU. [An Output draws](#draws-and-skips)
behind frames still on the GPU, so its readings arrive as each of them finishes rather than on
a tick that finds the latest done. [The thumbnail](#the-thumbnail-readback) and [the GPU
timer](#the-gpu-timer) are the engine's other two readbacks and follow the same rule.

A dropped frame reads nothing, and the app keeps each tap's last reading, so a tap does not
blink to zero on one — nor while its pass draws nothing, whose last frame's buffer is read on
the tick after it and nothing after that. The buffers go with the renderer.

**One slot, in one pass.** A measured node is measured in [its workspace's
pass](cpu.md#taps-the-other-direction) and nowhere else, so `Synth::collect_readbacks` routes
each pass's tap words to its `PassPlan::tap_nodes` with nothing to rank or pick, and its
thumbnail words to their ports only from the pass on screen, which is the one that draws them.
Readings are kept only for nodes holding a slot in a pass the plan carries (retained on
`Msg::Plan`), so a pass that stopped drawing holds its last value and a tap no pass measures
loses its uniform number instead of freezing it.

**A storage buffer written from a fragment shader** needs
`DownlevelFlags::FRAGMENT_WRITABLE_STORAGE`, which Vulkan, Metal and Direct3D 12 all have;
`atomicAdd`, `atomicMin` and `atomicMax` on `u32` are core WGSL, and `tests/shader_targets.rs`
takes every tapping module to MSL, SPIR-V and HLSL.

## The thumbnail readback

A save writes a picture of every open workspace, and **it is read back the way a tap is**:
a map collected when its callback has fired, never waited for.

Per Output, on demand: a pass of `readback::PICTURE` draws the frame just drawn into a 240x135
`Rgba8Unorm` target, letterboxed on black as the on-node render is, then
`copy_texture_to_buffer` copies it into a staging buffer, its rows padded to the 256 bytes a
texture copy asks for, and the buffer is mapped once the Output's submission is made. That
shape — target, staging buffer, map — is the thumbnail's, and [the Snap](#the-snap-readback) is
the same one at another size. A later tick finds the map landed and copies the bytes out, the
padding dropped. The pass samples by `@builtin(position)`, so the rows lie bottom first, as an
Output's frame does, and are flipped once on the CPU.

**The bytes are straight.** The frame is premultiplied and a PNG is not, so the pass's
`fs_straight` divides by alpha after the sampler has filtered, on the frame's own precision
and before the 8-bit rounding: the letterbox's scaling averages premultiplied colors, and a
transparent pixel is written transparent black. The bars are opaque black. The project tab
loads the file back as the straight PNG it is and draws it over its black card.

**A read that waited would be a stalled tick.** Polling a map with a wait on the synth thread
is a synchronous download, the same trap as creating a pipeline there, one subsystem over.
Asked for and collected later, it is a queued copy, and nothing blocks.

The request is a flag rather than a call, so a frame dropped with every target held carries it
to the next one. The read is recorded **after** the pass, in the Output's own encoder, out of
the target the frame was drawn into, so the picture is the frame that was actually drawn.
Collecting is done once a frame for every Output whether or not it drew, so an Output that
stopped rendering between the request and the answer still hands the picture over.

The target is allocated the first time one is asked for and kept, so a save reallocates
nothing and never disturbs the Output's own chain — **zero flash**: a save is invisible on
the projector.

`App` marks the workspaces a save wants a picture of, collects them a frame or two later and
writes each as a PNG through `video/png.rs`. Only an Output that is awake and has a shader is
asked, because a suspended or unplugged one would never render and the request would never
land. An [idle](#which-outputs-draw) one is drawn for the tick the request lands on, with every
Output whose frame it reads, so the picture is the frame continuous drawing gives then. The
status line says *saved friday, thumbnails pending* until the last one is written, and then
*saved friday*.

## The Snap readback

**Snap is the frame in front of you, written to disk while the show goes on.** silvia's, and
an action input like the two deck claims beside it, so a sequencer or a MIDI pad takes a
picture as readily as a finger does. The synth reads the port the way it reads `Show on A` —
a button that went down since the last tick, or an edge arriving from something that did —
and asks the Output's renderer for one. An [idle](#which-outputs-draw) Output is drawn for
that tick, as for a thumbnail.

It is the thumbnail's read at the Output's **own resolution**: nothing letterboxed, nothing
scaled, a straight copy of the frame just drawn by the same pass, into a staging buffer that
is mapped. The read lands a frame or two later, on the tick that finds the map landed, so
**nothing waits on the GPU** — and so the answer to *what is on screen
while this happens* is: the frame after this one, drawn on time. A resize between the ask and
the issue frees the target and allocates the size that fits, because a Snap of the wrong size
is not the frame that was in front of you.

`App::collect_snaps` writes each one as a PNG through `video/png.rs`. **PNG, and lossless**:
the point of Snap over the renderer is that it is the exact frame, so the file is the exact
pixels, unpremultiplied by the same pass as a thumbnail, since a PNG is straight — `snap_writes_a_full_resolution_png_into_the_projects_snaps_folder` in
`tests/gpu_app.rs` reads one back and compares every byte.

**Into `snaps/` inside the project folder**, named `output3-20260921-134501.png`: the Output
that took it, then the date and the time. The project folder is the thing that travels, so a
picture taken out of a patch belongs beside the patch that made it — but not in `workspaces/`
or `assets/`, which are the project's own machinery and are read back on open. A snap is
nobody's input; it is a person's picture, so it gets a folder of its own that nothing loads.
The stamp is **UTC**: there is no timezone database in this binary, and what the name is for —
telling two snaps apart and sorting them — UTC does exactly as local time would. Two inside
one second get a counter rather than overwriting each other.

**There is no Rec.** silvia's fourth button records until it is pressed again; a realtime
recorder is an encoder running beside the show at the rate the show is going, dropping
nothing, while the graph keeps its own time. That is machinery rather than a button and is
left out until it is wanted on its own terms.

## The capture

An offline render reads back **every frame at the Output's full size, and may not drop
one.** It is the thumbnail's shape repeated into a queue: a pass draws the frame just drawn
into an `Rgba8Unorm` target the film's size, and a copy goes into a staging buffer of its own
that is mapped, the reads in flight kept oldest first and a collected one copied into again.
A read is copied into again only once it has been collected, and a new one is made while
every one is still in flight, so however many ticks are on the GPU no read is overwritten.

**Supersampling lives in that pass.** `set_capturing` takes the multiplier the render was
started with, and the film's size is the Output's drawn size divided by it: the synth sizes
the `OutputJob` of the Output being rendered at `resolution × scale` for the length of the
run, so the whole patch above it is *evaluated* at that size, and the capture brings each
frame back down before it is read. The way down is **halvings**, with a linear filter: one at
2x, and two at 4x through a half-float target twice the film's size — because a single
linear pass from four times the size reads four of the sixteen drawn pixels behind a written
one and throws the other twelve away, which is the aliasing the multiplier was for. At 1x the
two sizes are equal, the filter never runs, and the pass is a nearest copy. A
multiplier that does not divide the Output's resolution is refused back to 1x rather than
rounded, so the film is never a pixel off what the preview was.

**Alpha is the writer's.** `set_capturing` takes a `readback::Alpha` with the multiplier: a
PNG sequence and a GIF are written straight, divided by alpha in the pass that writes the film,
after the halvings — every halving averages premultiplied colors, so an edge against
transparency comes back its own color at partial alpha rather than darkened, and the
half-float target stays premultiplied. A video is captured premultiplied: the encoder drops
alpha, and premultiplied color with its alpha dropped is the picture over black, which is what
every viewer shows.

**A capture waits for its own preceding frame**, bounded by two seconds, so a hung driver
costs one frame and a log line. Live drawing is bounded by the tick two back and the
throttle, while a capture needs the Output's own previous frame to complete before its read can
be taken.

The switch is a flag, `set_capturing`, so the app never touches the GPU; the next draw makes
the targets and staging buffers and, once the flag is off, the next poll collects what is
still in flight and lets it go, so the last frames of a render all arrive. A resize frees the ring the same way, collecting
first. Frames are taken oldest first, rows top first, through `Renderer::take_captured`; the
queue they wait in belongs to the Output rather than to the ring, so the last frames of a
render are still owed once the ring is freed. **A capture that turns on starts from nothing**:
the count of reads issued goes back to zero and a frame the last capture left unclaimed is
dropped, so the second render of a session counts and writes its own frames rather than
inheriting the first render's. `tests/gpu_readback.rs` holds that every frame comes back at
full size and in order, that a supersampled one comes back at the film's size, averaged, and
averaged premultiplied, and that a capture is straight for a file and premultiplied for a
video.

## The render job

An offline render is `synth/offline.rs`, with the editor's half — the settings read off the
Output's controls, the folder under `renders/`, the banner — in `app/render.rs`. One Output to
a numbered PNG sequence, a video file or an animated GIF, time a function of the frame index,
nothing dropped.
**It runs on the synth thread**, because everything it drives is the synth's: the clock, the
transport, every `CpuNode` and the renderer. The editor asks for one with `Msg::StartRender`,
watches it through the snapshot, and stops it with `Msg::CancelRender`.

Per tick, in place of the live beat: **collect**, then **step**. `render_collect` takes what
[the capture](#the-capture) has read back and hands each kept frame to a writer thread, which
encodes it off the synth thread — a PNG each through `video/png.rs`, one video file through
`video/encode.rs`, or one GIF through `video/gif.rs`, as the Output's **Writer** says. `render_step` drives the clock and,
through `Transport::drive`, the transport's playhead to `Stepper::time_of(i)`, `(i − warm) / fps`,
so each frame's advance is exactly one frame: an unplugged Time reads `rate × T` and a gear
integrates exactly the frames' advances, at any frame rate. It ticks the
graph — the open workspaces, exactly what live play ticks, so
a render wakes nothing a closed tab holds — and the ordinary job draws it. No step is clamped:
a stateful node takes each frame's whole step, where only a live tick holds one to
`transport::MAX_DT`. A step is taken only
when the previous one has been **drawn**, which the count of captures the renderer has issued
says; that is what keeps a frame from being stepped twice. It is also taken only while the
writer has room: that is the back-pressure, and it slows the stepper rather than losing a
frame. A render gives up on an Output that holds a stepped frame undrawn for 600 ticks; a tick
spent waiting on the writer — a GIF quantizing, a slow disk, the last frames being closed into
a file — is the writer's time and counts towards nothing.

**Time in a shader is a number, so a render builds nothing of its own.** A node that moves
with time declares `NodeDef::timing`, its rate at rest, its pace and its period. With nothing
cabled into its Time (key `clock`, no knob), the compiler reads the input as a published count
under that key, `u_count_{slug}{id}_clock` (`compile::CompileContext::published_count`), and
on every tick the synth writes there `playhead × rate` from the `f64` playhead in Loop mode, or
in Free mode the node's own playhead, its Speed integrated, times its pace — which a render
starts again from its first frame, so the film is the same twice. Cabled, Time
reads what arrives the same way, usually a gear's Cycles. A count is a `vec2f`, its whole part
wrapped at 40320 and the `f32` of its fraction (`nodes::phasor::split`, resolved by the synth
from `UniformProvider::NodeCount`), and a body, through the prelude's time helpers, reduces the whole part by its period — one
cycle, `N` under Repeat, 64 for the Tunnel while its depth wraps — before it adds the
fraction. So a module holds no clock and no loop: there is no Loop bit in the compiler,
the link or the plan, one module has one form, and a pause, a seek or a render changes numbers
and rebuilds nothing. What does rebuild is a noise's or Static's **Repeat**, an ordinary
option.

**A GIF** is the `image` crate's encoder, already in the binary for `imagegif`: each frame
quantized to its own 256 colors (NeuQuant, at its default speed), repeating forever. A GIF
counts in hundredths of a second, so frame `i` is held from `round(100 i ÷ fps)` to
`round(100 (i + 1) ÷ fps)` hundredths and a loop lasts its length to the hundredth; a browser
holds any frame under two hundredths for ten, so a GIF plays as rendered at 50 fps and under.

**A render is shown as it is made.** Every rendered frame is drawn into the Output's own
ring and published like any other, so a picture window on the Output, the node's picture
and the mix of a deck it is on step through the film one frame at a time, a tick behind as
every viewer is, and hold the last frame drawn while the render waits on its writer or a
clip. Supersampled, what they are shown is the larger frame, fitted as any picture is.
`a_render_is_shown_frame_by_frame_as_it_is_made` in `tests/gpu_app.rs` holds both to the
film's own frames, in order.

**A frame that was not stepped draws nothing.** The job suspends every Output on it, so a
feedback ring advances once per frame of the film rather than once per frame of the screen,
and a recompile elsewhere is left in place for the next stepped frame the way it is for a
closed workspace; the rendered Output, or an Output it reads, with a program on its way is
handed it on that tick without drawing. **No frame is
stepped while the program it would be drawn with is on its way**: while the plan still holds
an unsent source for the Output or an Output it reads, or the renderer is linking one
(`Synth::program_ready`), the render waits — for up to a minute, then gives up — where a
link in flight would have drawn the program it replaces into the film. Once every frame is
drawn the capture is turned off and the last reads drain.

**A clip is waited for.** A `video` or `imagegif` whose Time asks for a frame its decoder has
not delivered holds the frame: only what waits ticks again, with the transport where it is,
so it sees no advance and asks for the same frame, and nothing else takes a second step.
The frame is drawn once every clip has the frame it asked for, or after five seconds
(`WAIT_LIMIT`), a decoder that has stopped. Live play never waits; it shows what the decoder
has. The Main Input's clip and sound file are driven to each frame's time with the clock, so
an audio-reactive patch renders against the file — [media.md](media.md#the-main-input).

The three warm-ups are `clock::Warmup`: **Black** blanks every Output before frame zero
(`OutputJob::clear`) and runs nothing before it; **Hold** runs the warm-up frames at `t = 0`
with zero advance; **Run** runs them at negative time. **A render's start re-births every
gear.** Every CPU node starts from scratch — its live instance set aside for a fresh one, or
a device's reset where it is (`CpuNode::reset`) — and the playhead jumps to the first frame
(`Transport::start_render`) as a seek, so every gear is born again where the playhead puts it
and fires nothing on the way: a Master Gear at `playhead ÷ length`, a Ratio Gear at its ratio
times its input's reading, firing the one beat it is born on where that is a whole cycle. So at playhead zero, the first kept frame whatever the warm-up, every Master
Gear is at its cycle's start, every Ratio Gear on one at its own whatever its ratio, and a
node on ambient time reads its Time at zero. No other node
keeps time, so nothing else is put in place. At the end the live clock is put back,
re-anchored to now, with the live transport whole (`Transport::resume`), every node's live
instance, every simulation's world on the GPU and every Output's latest frame
(`Renderer::park_live`, `restore_live`: `OutputRenderer::keep` copies the frame in the prelude
of the render's first draw, before a black warm-up blanks it, and `put_back` copies it into a
slot of the ring as its latest in the prelude of the first live draw, before anything samples
it), and where each last read the transport, so the live show carries on as the render found
it, feedback included, and nothing sees a jump — [cpu.md](cpu.md#the-transport).

What starts one is the Output's own Render section — its numbers, its warm-up mode, its
button — and what says one is running is the band across the editor; both are in
[ui.md](ui.md#the-render-section-and-the-band-across-the-editor). **The ordinary render is
the one way to render**, a loop included: a loop's length is a Master Gear's, which its
caption states (`nodes::chain`), and the render's frames are that length at the Output's
FPS. There is no loop export, no seam measured in the app and no loop badge —
[decisions.md](decisions.md#one-transport-over-the-one-clock-and-every-rate-integrates-it).

**`examples/loop_gifs` renders a loop through it.** For each workspace it takes the Output's
ordinary render for as long as its Master Gear says a loop is (`nodes::chain::master_length`:
the gear's length times the least common multiple of its chains' denominators), from the only
Master Gear upstream of the Output, else the first by id, else `--length`, 8 s; with a loop of
Run warm-up and one frame more. The example compares frame `F` with frame zero itself, its own
seam check, and writes frames `0` to `F − 1` as a GIF on one shared palette.

`tests/gpu_app.rs` renders a Cosine Gradient under ÷2 of a one-second Master Gear, which
`master_length` makes two seconds: frame 20 is frame zero to the byte, frame 10 is not, and
the same length through the Output's GIF writer is twenty frames. A Tunnel at ×32 of that
master closes in two seconds to the byte; a two-second clip on ambient time comes back to its
first frame, the render holding for its decoder; a Rotozoom over a Perlin draws the same frame
at one moment whatever warm-up and frame rate came before; and a render hands the playhead
back as a seek.

An Output whose render would read a source that can only answer now — a camera, a screen, a
microphone, the Main Input panel pointed at one — wears a `!` that names them; it warns and
the render runs. See [ui.md](ui.md#the-render-section-and-the-band-across-the-editor).

The document is closed for the run: `App::apply` refuses every command with
`CommandError::Rendering`, which blocks a node, a cable, a control, an option and an undo
with one rule. Starting and canceling are not commands — a render is a thing done to the
instrument, not an edit — so they are `start_render` and `cancel_render`. A render goes to a
numbered folder under `renders/` in the project, beside `assets/`, so it travels with the set
and never overwrites the last one. `tests/gpu_app.rs` renders a feedback patch under each
warm-up and reads the PNGs back.

## The cost probe

**View → Costs** puts a strip under every node. On an Output it is the GPU time of its frame
from [the timer](#the-gpu-timer), against one vsync interval and in the accent color past it,
and the frames it [dropped](#draws-and-skips) in the last second. On every other node
it is how many times that node's function ran: per pixel of the Outputs it is compiled into,
and per frame in all, the bar being its share of the busiest node's. That multiplier is what
makes a patch fall over — a blur samples its input nine times, so everything upstream of it
runs nine times per pixel, and a bloom downstream of an edge detection multiplies the two — and
it is invisible in the graph until it is drawn.

**Measured, not declared.** How many times a node runs depends on loops whose trip counts are
options *and controls* — a blur's size, a bloom's rings, a phyllotaxis's seed count, a
scatter's density — so no table of taps per node kind could stay right. Every awake Output with
a shader also has a **probe**, whether or not View ▸ Costs is on: `compile::wgsl::build_probe` emits
the same shader with one `atomicAdd` at the top of every node function, counting into that
node's tap slot — the slot's last word, which no kind of measurement uses. The renderer draws
each probe — on the ticks its Output draws, so an idle Output's counts stand as its last draw
left them — at [`PROBE`](../src/render/mod.rs) pixels, sixteen by nine, into a target of its
own in the tick's last submission, after every Output, and reads the counts back the way a
tap's words come back: once their map has landed, never by waiting. Each **call site**
— a consumer calling the function that feeds one of its inputs — is counted too, wrapped in the
sequence operator so the spliced expression stays an expression; a consumer's calls over its
own evaluations are its **taps**, which is how the strip names the cause (`×9 taps` on the edge
detection) beside the effect (`9.0/px` on the video it fell on). **A probe emits no
measurement**, as its Output's module does not: a measurement runs in [its workspace's
pass](#the-workspace-pass). A tap's grid is therefore outside the probe's figure — a fixed
`N²` evaluations of its input a frame, [stated above](#tap-buffers), on top of what the strip
says. `App::ingest_probe` divides each count by the probe's
pixels and multiplies by the Output's, so what the strip says is what the real frame does. A
probe is one more pipeline to link on an edit that changes its Output's source — it is built
from that source and rebuilt only when it changes — and one draw of 144 fragments a frame; a
node with no shader, or an Output that has gone to sleep, frees its probe on the next frame,
and an Output that was *deleted* — which is not in the frame job at all — has its probe
and its readings dropped alongside every other per-node thing the tick prunes, so a dead
Output's last count never keeps a warning alive.

**A node whose taps cross 8 wears the warning on its own header, view or no view.** The
reading the probe makes is what View ▸ Costs draws as a strip and what the header's amber `⚠`
reads from — the view only decides whether the strip is drawn, not whether the measurement is
made. See [decisions.md](decisions.md#a-header-warns-on-what-the-probe-measures-not-on-the-view).

The picture a probe draws is thrown away. Only the counts leave it.

## The workspace pass

**Every measurement runs in its workspace's pass, and so does every thumbnail.** A tap's,
a `sample`'s and an `autoexposure`'s reading ([cpu.md](cpu.md#taps-the-other-direction)), and
every varying output on the workspace being looked at drawn as a thumbnail beside its port,
connected or not — see [ui.md](ui.md#the-value-on-the-row) — are made in one program per
workspace, `compile::wgsl::build_pass(graph, workspace, measured, thumbs)`, that is nobody's
picture: each measured node's `measure_wgsl` first, its slot first, in the order given, and
then, where `thumbs`, every node function the workspace holds, each pulling its chain above it
into the one module. `compile::wgsl::build_measure(graph, id)` is one node's measurement
alone, which [the draw rule](#which-outputs-draw) reads to know what frames and readings a
measurement samples. `link::plan::measured_on` says which pass measures what: each awake
measured node once, in the pass of the first of its workspaces in project order that has a
tab, or, where it is awake only because a deck or a send reads it, of its first workspace. So
there is a pass for every open workspace, with thumbnails, and one for every closed workspace
holding such a node, with measurements alone. An Output's module and its probe measure
nothing.

**A pass is context-free.** A thumbnail is the port's own function over the frame's own
coordinates, `x` from `-16/9` to `16/9` and `y` from `-1` to `1`, and a measurement is the
node's input over the unit square, whoever reads either and at whatever coordinates. So a port
reaching no Output has a thumbnail, a node on a workspace with no Output has one and a tap there
reads, a tap cabled into two Outputs of different size and aspect reads the same as one cabled
into none, and a Zoom downstream does not zoom what is upstream of it: the thumbnail is what the
node makes. A consumer that samples its input many times — a blur — is a question about the
consumer, and nothing here answers it.

**One fragment per cell, in two regions.** The measurements have the square of side
`Shader::grid` at the target's corner, each called under its own grid guard
([tap buffers](#tap-buffers)); the thumbnails have tiles of 48x27 cells to its right, from
`x = grid`, eight across, one per port in node order; `compile::pass_size` fits the target to
both. A thumbnail's fragment finds its tile and its cell and a `switch` calls that tile's
function once, at the cell's center. It writes one word of the pass's buffer — the tap buffer,
bound and read back as [tap buffers](#tap-buffers) describes, past the measurements' slots —
at the cell's own index, so no two fragments write one word: a number's `f32` bits, or a color
clamped to `[0, 1]` and packed four bytes to a word. The picture is thrown away. A tile is one
port throughout, so a workgroup runs one case of the `switch` but where it straddles two
tiles. `u_resolution` reads 1280x720 in a pass, not the target's size, so a node that measures
in pixels draws its thumbnail and is measured as in an Output of the default size.

**Drawn in the coda, after every Output**, before the probes, into its own renderer
(`Renderer::passes`, keyed by workspace and batch), so what it samples of another Output's
frame is this tick's. A pass wakes nothing by being drawn: an Output that is idle is sampled as it last drew,
and what a measurement's reading wakes, [the draw rule](#which-outputs-draw) wakes in the plan.
**Its thumbnails draw only while its workspace is the one looked at** (`PassPlan::shown`);
every other open workspace keeps its program linked and its last thumbnails, so a tab looked at
again has its thumbnails on the first tick. **Its measurements draw on every tick one of them is
read** (`PassPlan::measures`, the draw rule's), and on every tick an Output reading one draws
(`PassPlan::wanted_by`), so a deck the synth claims before the editor replans draws the pass
beside it. A pass not shown draws only the measurements' square, by a scissor
(`OutputRenderer::set_region`, the job's `region`), and since a pass resets only its tap slots,
the thumbnail words it leaves are the last full frame's. Nothing draws while a render holds.

**Built on every change to the graph's shape, and sent only when it differs.** A pass holds
every node on its workspace and every chain above them and above its measured nodes, which may
run through any other, so the app builds every pass's module again whenever the document's
shape moves, and wherever a pass's measured list or whether it has thumbnails changes
(`SynthLink::plan_passes`) — string work — and sends only a source the renderer does not hold.
An edit therefore marks stale only the Outputs downstream of it: no Output carries a
measurement. A pass links off the synth thread like any program; until it lands the old one
draws, so a thumbnail stands still rather than blank and a reading holds its last. A link that
fails is logged once, and its thumbnails and readings stand where they were — none, for a pass
that never linked; the Outputs are untouched. The Status box times the passes, measurements
and thumbnails together, on their own line, `thumbnails`, from `OutputsTo` to `PassesTo`.

**What it costs**, measured on the UHD 770 with the machine not idle, so as a size and not a
figure: 0.07 ms a tick on the demo's Convert, Mix and Math and Games and simulations, 0.20 ms
on Effect manglers and 0.28 ms on Effect kernels, where the Outputs took 5 to 27 ms. A
thumbnail is 1,296 evaluations of its port against 921,600 for a 720p frame, so a chain's
thumbnails cost about its depth times 0.07 % of drawing it. A measurement costs its grid's `N²`
evaluations of its chain on the ticks its reading is read. `tick_bench` prints the phase, and
`SUPERSILVIA_THUMBS=0` builds no thumbnails (`compile::thumbs_enabled`), for the figure without
them; the passes still measure.

**A crowded workspace is drawn in batches.** A pass binds every texture any node on the
workspace samples, where an Output binds only its chain's, so a tab of images, clips and
simulations can bind more than a stage may sample. `compile::PASS_TEXTURES` is sixteen,
WebGPU's floor, which every adapter wgpu runs on offers whatever its own limit — Metal's
among them. A workspace whose pass would bind more is built as several (`wgsl::build_pass`
returns them in order): each measurement and each port, in the pass's own order, joins the
batch being filled while the textures they bind together stay within the floor, and each is in
exactly one. Only a crowded workspace pays for it: the whole is built first, and split only
where it binds too many. Each batch is a pass of its own to everything after the compiler —
keyed by `render::PassKey`, its workspace and its index, with its own renderer, program,
buffer and readback — and a batch that keeps its index and its source across a rebuild is not
sent again. A single port whose own chain binds more than the floor is a batch alone, and fails
to link as an Output binding as many would. One pass holding every node in the registry binds
15, so it stays one module; `tests/compile.rs` splits twenty cellular automata and holds every
batch to the floor and every port to one batch, and `tests/gpu_app.rs` draws them and has every
thumbnail back.

## The GPU timer

The tick's pacing is a CPU figure. It sees a GPU that is running out of headroom only once the
overshoot is large enough to push a frame past a vsync interval, so the Status box could say the
CPU is under a millisecond and say nothing at all about how close the picture is to falling
over. **Two timestamps per Output's pass** are what says it, and they are the engine's third
readback, under the same rule as the other two (`render/timing.rs`).

The Output's render pass carries `timestamp_writes` at its beginning and its end
(`Stamps::span`). **The span is the pass**, which is the whole of an Output's per-frame GPU
cost; the thumbnail's pass after it is outside it. A span is ticks of the
GPU's clock times `Queue::get_timestamp_period`.

**A renderer has one query set**, whatever it draws: a ring of `timing::DEPTH` ranges —
`TICKS_IN_FLIGHT` + 1, since a range is read two draws after its own at the earliest — each
holding that tick's [ten marks](#a-tick-by-phase) and then a pair per Output drawn, for the
first `timing::TIMED` (512) to draw in plan order. Plan order holds from tick to tick, so an
Output past them is untimed every tick they all draw: its cost is never known, and it goes in
a submission of its own, as an Output never timed does. One set, not one per Output, because
**Metal holds at most 32 counter sample buffers in a process**, across every device, and
wgpu-core loses the device that asks for a 33rd — the driver's "Cannot allocate sample
buffer" comes back as an unexpected error, which wgpu treats as fatal. So on Metal every query
set the process makes is also made under a claim on `timing::QUERY_SETS` (16), one per
renderer and one per editor paint timer: a renderer that finds every claim held draws untimed
rather than lose its device, with no line in the Status box, and asks again at the start of
each draw until a claim is given back.
Vulkan has no such limit, and there nothing is claimed.

A tick's range is resolved into a buffer and copied into a staging buffer in the recording of
the tick's last submission, after its last pass and last mark. Its map is asked for at the top
of the next draw, once that submission has gone in, and read at the top of the first draw
after that to find it landed — two draws after its own at the earliest — asked for, never
waited for. The spans are read oldest first and handed to each Output's `Timer`, whichever
program drew them; a tick that finds its range still in flight times nothing rather than
overwrite one, and a range still in flight holds back the ones after it. An Output the GPU
stays behind on still reports what its frames cost.

The figure reported is **the longest span in the last two seconds**, beside the latest, for
the reason [a worst is](decisions.md#a-worst-not-a-mean-frame-time). A dropped frame carries
no result and the window keeps what it had. An Output that has gone dark forgets its
readings, and a suspended one is not drawn, so neither shows a line: an Output with nothing
to draw has no cost to report, and a figure left standing would describe work nothing is
doing. An idle one keeps the figure of the last frame it drew — it is what drawing it again
would cost — and the Status box mutes it and leaves it out of the total.

A pass-boundary timestamp needs `Features::TIMESTAMP_QUERY` and nothing more, which the device
asks for where the adapter offers it. **A device without it has no timer at all**: no ring is
made, nothing is marked, and the Status box has no line for any Output — never a zero, which
would read as an idle GPU. **A timestamp the GPU did not write reads zero, and is no reading
either**: a span or a draw's marks holding one are dropped. A Vulkan counter reads zero only on
the tick it wraps, which loses that one reading rather than show a wrong one.

### A tick by phase

The per-Output spans say which Output costs what; they do not say where the rest of a tick
goes. **While the Status box is open** — the `status` flag `Msg::Inputs` carries, which already
gates its CPU lines — the synth times every tick by phase, on both processors, and closed it
times nothing: no lap reads the clock and the renderer places no mark.

**On the CPU, laps on the thread's own clock** (`synth/meter.rs`). The tick runs from the top
of one loop to the top of the next on the wall clock, and each lap inside it reads the synth
thread's CPU clock — `CLOCK_THREAD_CPUTIME_ID`, through `rustix` — and gives the CPU time since
the last lap to one `Work`: the editor's messages, the MIDI drain, the clock and every node's
tick, the frame job, the whole draw, `publish`, the tap readings and the snapshot, with
whatever no lap claims as `other`. The one span read on the wall is the sleep before the
deadline. What the wall saw and the CPU did not is the thread blocked inside a phase — in the
driver with the GPU's queue full, almost all of it — and is `waiting`: the tick less the sleep
less every work, so **the work, the waiting and the sleep add up to the tick** by
construction. A draw [waiting for the GPU's queue](#draws-and-skips) is in
`waiting`, and the draw's own CPU time is one figure, since what a person tells apart inside it
is the GPU's. Inside the nodes' lap each node is timed on its own, on the same CPU clock as the
lap, the Main Input's capture as one more, and the five costliest by their mean are named — so
a node that blocks never outweighs the total it is part of. Every figure is kept as this tick
and a one-second running mean, and every mean is blended with the same weight each tick, so
the means add up too.

**On the GPU, timestamps at phase boundaries** (`timing::Stamps`). The draw is marked ten
times — `Start`, `UploadsFrom` and `UploadsTo` either side of the uploads, `SimsTo` after the
simulations, `OutputsFrom` before the first Output, `OutputsTo` after the last, `PassesTo`,
`ProbesTo`, `MixTo`, and `End` — each an empty compute pass whose end writes one timestamp into the
recording of the phase it bounds, so a mark needs `TIMESTAMP_QUERY` alone and sits between any
two passes. The first five are in the tick's first submission, the rest in its last, and a
mark not placed asks nothing of a recording, so a phase with nothing in it still submits
nothing. The spans are differences of those marks: `uploads` (every texture write and every
conversion pass), `sims` (every simulation's dispatches and a reshape's passes), `outputs`
(first Output pass to last, so the thumbnail passes and capture reads between them are in
it, where the OUTPUTS table's total sums the passes alone), `thumbnails` (the
[workspace passes](#the-workspace-pass), their measurements included), `probes`, `mix`, and
`whole`, each a `GpuPhase`.
**`between` is the whole less the six parts**: the GPU work of the
draw's bookkeeping, resizes and clears.

**On Vulkan and Direct3D 12 a mark is written at the bottom of the pipe**, after every command recorded before
it, so the dispatches ahead of `SimsTo` are inside `sims`, and `slimemold`'s steps read there
rather than in the first Output that samples its picture. A timestamp is the GPU's clock, so a span is
everything between its two marks on the one queue — including the time the throttle held the
synth's next submission back, and any editor frame or picture-window blit the queue ran between
them.

The marks are read under the timer's rule, in the tick's range beside its Outputs' spans:
resolved and copied into a staging buffer in the recording of the last mark, and read once
that buffer's map has landed, which asks and never waits; the ranges are read oldest first. A
draw whose range is still in flight places no marks and has no reading, rather than overwrite
one or wait for it. Marks that do not run forward — a wrapped counter — are dropped.

**On Metal a draw has no breakdown.** Metal writes no timestamp at the end of a pass with no
work in it, so every mark reads zero and no draw gives a reading; the GPU rows stay dashes
while each Output's span, whose pass draws, is read as anywhere else. A mark that dispatched
work would be written, but Metal orders passes only by the resources they share: an Output's
pass is then found outside the marks around it, before `Start` at times, so the phases would
not be what they name.

**Beside them, the whole process.** The synth's marks see only the synth's submissions. The
kernel's DRM client counters see all of it: each DRM descriptor's `/proc/self/fdinfo` carries a
`drm-engine-render` line, the render engine's cumulative nanoseconds for that client — the
Vulkan driver's descriptor is a DRM client like any other. Summed over the process's clients,
each once, and differenced across a second, it is the share of the engine the synth, the
editor's painting and every picture window used together. The descriptors are found by their
links into `/dev/dri`, searched again every five seconds. **A Mac counts no engine time per
process**: what it has is the IORegistry's `IOAccelerator` service, whose `PerformanceStatistics`
carry `Device Utilization %`, the whole GPU's busy share and the figure Activity Monitor shows.
It is read once a second as it is, with nothing to difference, and shown as the whole GPU's.

### The editor's own painting

The synth's marks are in the synth's submissions; the editor paints in egui_wgpu's. **Two
timestamps bracket its frame** (`render::timing::PaintTimer`), each placed by a paint
callback `PaintTimer::mark_on_paint` builds and `App` asks for while the Status box is open:
`PaintMark::Start` on the background layer before any panel is laid out, which writes a
timestamp into egui's encoder ahead of its render pass from the callback's `prepare`, and
`PaintMark::End` on the debug layer, above every window and tooltip, which writes one inside
egui's render pass from the callback's `paint`. The span is every mesh and every picture blit
egui paints; the clear before it and the present after it are outside it. **The end mark is
a timestamp inside a pass**, which needs `TIMESTAMP_QUERY_INSIDE_PASSES`: Vulkan offers it,
Apple GPUs are not expected to, and a device without it gives the editor no figure — never a
zero. It is read under the timer's rule, on a ring of four pairs: nothing runs in egui's
encoder after its pass, so a frame's pair is resolved in the next frame's `prepare` and its map
asked for in the one after that, and a frame that finds its pair still in flight marks
nothing. The reading is folded into the Status box's *GPU per frame* row. Closing the box lets
every pair still unread go, so the first reading after it reopens is of a frame painted after
it did. Both marks are written into egui_wgpu's one submission, so the synth's submissions
queued around it are outside the span.

## Paint callbacks, and the viewport they are given

Everything a window *shows* is blitted from inside an egui paint callback, in egui_wgpu's own
render pass; the drawing itself happens in the synth's submissions before any panel lays out.
**Every callback the editor paints through is built by `render::viewer`** — a node's
picture by `Viewer::node_callback`, the mix by `Viewer::mixer_callback`, each an
`egui_wgpu::Callback::new_paint_callback(rect, ViewerCallback)` — and the timer's by
`PaintTimer::mark_on_paint`, so `app/` says what is shown where and names no backend
(`tests/rules.rs`). `Viewer::open_frame` and `Viewer::close_frame`, which `app/frame.rs` calls
either side of the frame's painting, owe the device nothing and are empty.

`ViewerCallback` implements `CallbackTrait`. **`prepare`** makes the blit's bind group — the
picture's view and the sampler its `Picture::sampler` names — and its uniform block, placing
the rect against the whole target in the vertex stage. **`paint`** sets the viewport to the
whole target and draws. The pipeline is the editor's `Viewer`, made once for egui_wgpu's
`target_format`, blending premultiplied as egui_wgpu's own does; eframe is configured with no
depth buffer and no multisampling (`depth_buffer: 0`, `multisampling: 0`), which the pipeline
assumes of the pass it draws in.

**The viewport is the one piece of host state that bites.** egui_wgpu sets
`viewport_in_pixels` before a callback's `paint`, and that rect is clamped to the window: a
blit fitted to it would shrink the picture of a node at the canvas's edge into the part still
on screen. So `paint` replaces it with the whole target, the rect keeps its size whichever
side it hangs off — or past `max_texture_dimension_2d` at a deep zoom, which `set_viewport`
would refuse — and egui_wgpu's scissor, set from the clip rect, is what cuts the picture off.
wgpu has no other global state for a caller to leave behind: every Output's pass names its own
target, viewport and bind group. `tests/gpu_mixer.rs` pins it: a blit fits its rect and the
clip cuts it rather than shrinking it, and a rect off the window is cut off rather than
squeezed.

## One device

**The whole app draws on one wgpu device and its one queue.** `main.rs` makes it before the
window, with `render::Gpu::headless`: an instance over the machine's one backend
(`render::adapter::BACKENDS`) — Vulkan on Linux, Metal on macOS, and Direct3D 12 on Windows,
whose HLSL the DirectX Shader Compiler linked into the binary compiles — the adapter
`render::adapter::choose` picks — the strongest GPU, never a software adapter unless told
([invariants.md](invariants.md#what-a-machine-catches)) — one device with the adapter's own
limits and whichever of the renderer's wanted features it offers, and its queue. A box with no
adapter the rule accepts refuses to start and says what it was offered.

**The strongest GPU is the kind, not a measurement.** Of the adapters left, a discrete GPU
of any vendor, NVIDIA included, comes before an integrated one, before any other hardware,
before a software adapter where `SUPERSILVIA_SOFTWARE_GPU=1` allows one; of two of a kind, the
first the instance lists, which on Vulkan is the loader's order and on Direct3D 12 DXGI's. **`SUPERSILVIA_ADAPTER`
names one instead and wins**: an index into the list a refusal prints,
`vendor:device` in hex, `integrated` for every integrated GPU, or a case-insensitive piece of
the adapter's name — `intel`, `radeon`, `nvidia`, `4070`. It leaves only what it names, so one
naming nothing refuses to start rather than fall back, and it does not lift the software rule.
A machine with a discrete GPU renders on its iGPU with
`SUPERSILVIA_ADAPTER=integrated`, and the tests, the benches and `check.sh` ask for the
integrated GPU themselves, `adapter::Asked::integrated`
([testing.md](testing.md#3-the-gpu--the-actual-pixels)). `scripts/doctor.sh` applies the same rule to
`vulkaninfo`'s list. eframe is handed the four through
`Renderer::Wgpu` and `egui_wgpu::WgpuSetup::Existing`, so the editor paints through the same
device; eframe is built with `wgpu_no_default_features`, so no GLES backend is in the binary
and each machine's has its own backend alone, and its surface presents in eframe's default
`AutoVsync`.

`App::new` takes its `Gpu` from `render::Gpu::for_eframe`, which wraps
`cc.wgpu_render_state`. Under egui_kittest there is none, and the app runs headless: pictures
are placeholders and the synth runs inline. **The synth's is `Gpu::for_synth`, a clone** —
the same device and the same one queue — carried to the synth thread, where `Renderer::new`
keeps it for the life of the run. Nothing is made current on any thread: `Device` and `Queue`
are `Send + Sync`, and wgpu serializes what needs it. **A surface configures while the synth
submits**, which wgpu-core 30.0.1 refused, with a panic from the default error handler: the
build takes wgpu-core from `vendor/wgpu-core`, which accepts it, since a surface is used by one
thread and nothing the synth submits touches its textures
([decisions.md](decisions.md#wgpu-core-carries-one-patch-so-a-surface-configures-beside-the-synth);
`tests/gpu_surface.rs`). The picture host is handed a clone too. `Synth::render` draws once per tick, on the synth's own
schedule, whatever the frame thread is doing. `Gpu::for_synth` is the one door a synth on a
device of its own would change.

**One counter of finished submissions** says what the GPU has done (`render/gpu.rs`). Vulkan,
Metal and Direct3D 12 know completion per submission, not at a point in the command stream, so every
submission made through `Gpu::submit` is numbered, and `Queue::on_submitted_work_done` stores
the number into one atomic once the GPU has finished it. "Has this finished?" is a comparison
with `Gpu::completed`, and a slot, a readback or a returned DMA-BUF carries the serial of the
submission that last touched it. The number is taken and the submission made under one lock,
so serials are handed out in the order the queue receives them, whichever thread submits. A
callback runs on whatever thread next polls or submits — egui_wgpu's submit inside the editor's
frame included — and it only stores into the atomic.

**One queue has no priority.** The editor's paint, every picture window's blit and the synth's
tick go into it in submission order, and a frame submitted behind a queued tick waits for it.
Measured headless on the UHD 770 with a stand-in editor of 2.9 and 4.8 ms beside a synthetic
synth, a synth that submitted a whole tick at once starved it: at nine tenths of an interval
of synth a third of the editor's frames overran 10 ms, and at saturation most did. So **the
synth submits about one Output's pass at a time and keeps little queued**: an Output whose
last pass cost more than `SUBMISSION_MS` (2 ms), or that has not been timed, is a submission of
its own, a run of cheaper ones shares one, and `render::queue::Throttle` submits the next only
once at most `QUEUED_AHEAD` — one — of its earlier submissions is still on the GPU, so an
editor frame lands behind the submission running and at most one more. Sharing is for the
synth's sake: a submission costs its thread about a tenth of a millisecond in wgpu and the
driver, and one per Output left the GPU idle after each cheap one while the thread recorded
the next. Against
`examples/queue_contention.rs`'s synthetic synth that took every overrun away at every load
measured, an editor frame costing its own pass and about one synth pass, the synth paying for
it as a device of its own at low priority makes it pay. Beside the demo's own tabs,
`tick_bench`'s stand-in editor kept every frame inside 10 ms beside Start here and Effect
kernels, and let 333 of eight seconds' over beside Games and simulations, whose one heavy
Output is a single 17 ms pass (the proposal's headless figures); the live window is measured at step 7. `Renderer::throttled` and
`Renderer::most_queued_ahead` count the throttle's waits. **A device of its own for the synth,
at low queue priority, is held in reserve** — Plan B of
[proposals/wgpu.md](../proposals/wgpu.md), with its cross-device sharing — for the case the
live window says the throttle costs frames; see
[decisions.md](decisions.md#the-synth-shares-the-one-queue-throttled).

**A lost device saves the work and says so; it is not recovered.** wgpu's own defaults panic
on an error no error scope captured and on a lost device, which ended the process with the
unsaved work in it. So `Gpu::open` puts `render::gpu::watch` on every device it makes: an
uncaptured error — a validation error included — is a line in the log and nothing more, and a
loss reaches a callback. The app's is set in `App::new`, over that one, and it cannot ask
anything on screen: **the editor paints on the same device**, so a lost device is a lost
editor as well as a lost synth and every picture window. It writes the newest unsaved
document into [`.autosave/`](architecture.md#saving-and-opening) at once, whatever the
autosave's timer says; says *The GPU stopped responding. Your work was saved; restart
supersilvia.* on stderr and in the log, with wgpu's reason after it, or says the work could
not be saved and why; writes that line into the log file as the reason the run went down; and
exits with status 1, from whichever thread wgpu reported the loss on. The next launch says
*supersilvia closed unexpectedly* with that line, then offers the autosave back
([ui.md](ui.md#what-a-tester-can-send)). Rebuilding the device, and every Output, probe,
upload and window on it, in place is not attempted
([decisions.md](decisions.md#the-frame-job-runs-in-synthrender-and-no-panel-carries-it)).

**Nothing is freed by hand.** Every GPU object is reference-counted and freed once nothing
holds it and the GPU is done with it. `App::on_exit` stops the pictures first — on Linux
their wgpu surfaces must go before the `wl_display` they were made on, which eframe tears down
when `on_exit` returns; on macOS it only lets go of the loop, which closes the windows once
eframe has exited — then the synth, whose renderer goes with its thread (`Synth::destroy_gpu`
drops it).

## Every window is a viewer

The main window's preview, the mix painted behind the canvas, an Output's picture on its own
body, every picture window: each one **blits a texture the synth has finished drawing**, and
draws nothing.

- The synth publishes, as `render::Published`, the newest target of each Output and of
  the mix whose submission has finished — views, shapes and samplers, which is all a
  `Send + Sync` callback can hold. **A viewer never waits on the synth's work**: what it blits
  has nothing left on the GPU, so a thumbnail of a cheap Output does not wait behind the
  heaviest Output in the tick, which is what one submission holding every Output would make it
  do.
- **A CPU node's frame is rewritten in place**, and a simulation's picture with it — uploaded
  again when a new frame arrives, or drawn again by the passes. On the one queue the write and
  a viewer's blit are ordered by submission, with wgpu's barrier between them, so a viewer
  sees the frame before the rewrite or the one after, whole, and never a torn one. Nothing
  waits for it on any thread; a wait with a timeout on the frame thread would stall every other
  window with it — see [the proposal](../proposals/deterministic-loop.md). An import's memory
  goes back to its producer only once nothing on the GPU can read it — see [DMA-BUF
  import](#dma-buf-import).
- **A window that cannot be seen stops painting.** The editor asks for its next repaint one
  display interval out, so eframe sleeps between frames rather than polling — and *a quarter of
  a second* out while the viewport reports itself minimized, which stops eframe running passes
  for it at all. That is what keeps a minimized window from blocking, in acquiring its next
  surface image, the thread every other window paints on. `pre_present_notify` is the call that
  would gate the redraw properly and it is not used: it holds eframe in `ControlFlow::Poll` and
  cost 97.9% of a core against 17.2% without, measured on the GL renderer. See
  [decisions.md](decisions.md#the-compositor-paces-the-viewers-the-synth-paces-itself).
- **A picture window is not a viewport at all.** It is a window of our own drawn off the
  editor's thread — on Linux a Wayland surface on the pictures thread, paced by its own
  `wl_surface.frame` callback, and on macOS a winit window with a thread of its own, paced by
  its display through `Fifo` — see below — so the editor's painting is not in its way and a
  minimized editor is not a thing it can observe.
- A viewer that paints twice between two ticks shows the same picture twice. That is
  correct, and is not a drop.
- **The `Published` a viewer holds may be several ticks old.** It took it at the top of an
  editor frame, and the synth has been ticking since. So nothing a `Published` names is freed
  while it is held — it holds views, a view keeps its texture alive, and wgpu keeps whatever a
  submitted command buffer names alive until the GPU is done — and no Output's or mix's target
  it names is drawn into: an unplugged Output's black waits for a target nothing holds. That
  second half is the **lease**, a plain claim a `Published` holds on each such target, which
  the ring asks before drawing into one. There is no count of ticks in it: an editor frame held
  across a resize holds what it names for exactly as long as it is held.
- **A viewer holds its `Published` until after the `queue.submit` that carries its reads.**
  The editor's lives on `App` until the next frame's snapshot replaces it, past egui_wgpu's
  submit at the end of the frame, and each `ViewerCallback` holds one too, which egui drops
  only after that submit; a picture window drops its own after the submission before its
  present. The synth draws into a target only once no `Published` names it, so its draw is
  submitted after the viewer's blit, and the one queue runs it after. **Nothing is waited for
  on either side.** `tests/gpu_ring.rs` holds it with a viewer on another thread sampling the frame it holds
  while the synth draws on, and `tests/crosstalk.rs` with the synth's renderer and a viewer on
  threads of their own. The snapshot buffers do not hold one past their use: the synth points
  a spent buffer at the newest `Published`, and so does the mailbox when the editor hands one
  back, so a stale snapshot never holds a target.
- **A window painted outside the editor's pass reads its own slot.** The `Published` it blits
  comes from `render::Live`, which the **synth** writes on every tick. A slot the frame thread
  filled would freeze at whatever it last held, which is precisely while the editor is
  minimized.

Folding the Main Mixer panel away hides a picture and nothing else: the draw is the synth's,
and no panel carries it. **Not drawing an Output is done in the job**, by suspending it or
leaving it [idle](#which-outputs-draw), never by leaving it out: it stays in the job, so the
renderer keeps the targets it has allocated and waking it is not a reallocation.

**While the editor is minimized nothing on the frame thread runs at all.** MIDI is not read,
no plan is republished, no probe counts or thumbnails are collected, and no command is applied.
Every picture window goes on painting, because none of that is on its path. The world advances
and the routing does not — with one exception: a deck claim is applied by the synth to the mix
it renders, so a sequencer lane or a Master Gear's Trigger on `Show on A` still cuts, and the cut is on
screen the moment a window paints again. See
[architecture.md](architecture.md#the-two-channels).

## Picture windows

A picture in a window of its own — a node's render, a source's frame, or the mix — is **not an
egui viewport**. eframe runs one winit `EventLoop` (winit permits exactly one per process:
`EVENT_LOOP_CREATED` is a static and a second is `EventLoopError::RecreationAttempt`), and that
loop services every viewport in turn, so a minimized editor would take every other window down
with it. A window paced by its own compositor cannot.

How a window is made is the one thing that differs by system. On macOS and Windows it is a
winit window inside eframe's own loop, [below](#on-macos-and-windows). On Linux, `render/picture/` opens **Wayland
surfaces of its own** on a thread named `pictures`: an
`xdg_toplevel` with no decorations per picture, a wgpu surface on each, and a `Viewer` per
surface format, all on a clone of the one device. `proposals/picture-windows.md` is the
argument; the shape is:

**It borrows eframe's `wl_display`.** The thread wraps eframe's display with
`Backend::from_foreign_display` and runs an event queue of its own on it, which is
libwayland's own multi-queue design rather than something being got away with. A Vulkan
surface needs *a* `wl_display`, and the textures are the device's whatever connection a window
is on, so a connection of its own would draw as well. The borrow is kept because a window from
a second client connection is a window from a client that does not hold focus, and the
compositor's focus-stealing prevention may open it behind the editor.

**The threading contract.**

| owns | borrows | never touches |
| --- | --- | --- |
| the connection's queue, every window, the wgpu surface on each, a `Viewer` per surface format | eframe's `wl_display`, a clone of the one `Gpu`, the synth's `Arc<Live>` | the graph, the snapshot, egui, the command bus |

The editor sends `picture::Ask` — open this picture at this size, close it, fullscreen it —
over a calloop channel, and reads `picture::Told` back once a frame: closed, fullscreen, or
failed with a sentence for the toast. The editor's own list of what *should* have a window is
`picture::Wall`, which is pure and tested with no compositor. The synth's `Published` reaches
the thread through the same `render::Live` the editor's viewers read, written by the synth on
every tick. What it names is held exactly as it is for any viewer.

**A window on a node goes with the node.** Deleting it closes its window, and so does opening
another project, or starting a new one: ids restart at 1 in every project, so the node holding
that id afterwards is another node. The mix's window stays, because the mix is the rig's.

**A window's clock is its frame callback**, and **exactly one is outstanding at a time**. It
is asked for immediately before the present whose commit carries it, and cleared when it
arrives; asking for a second while one is outstanding would make the window present once per
request per compositor frame, for ever. On each callback the thread takes the newest
`Published` and paints: `get_current_texture`, a pass that clears to black and blits with
`Fit::Letterbox` over `Viewport::whole`, the submission, the frame callback's request, the
present, and then the `Published` let go — waiting nothing for an Output, the mix or an
upload. **The present mode is `Mailbox`, else `Immediate`, else `Fifo`**: one that never waits
for the display, because the callback is already the pacing, and a present that waited as well
would hold one window while another's callback was due. A surface that reports itself outdated
or lost is configured again; one that times out or is occluded skips the paint, and the
watchdog below repaints it.

**A picture window nobody can see stops painting**, which is the editor's rule too, arrived
at differently: a compositor stops sending frame callbacks for a surface
it is not compositing, so a minimized picture window has one callback outstanding and sleeps
until that surface is composited again. The thread blocks with no timeout while every window
is in that state, so a hidden window costs no wakeups at all. The 50 ms watchdog is **not**
for that case and must not fire in it — it is the safety net for a window with *no* callback
outstanding that has somehow not painted, and it skips any window that is on no output.

**A window's surface is made on its first configure**, because a surface is configured at a
size: `instance.create_surface_unsafe` over the raw Wayland display and the window's
`wl_surface`, configured at the physical size — the logical size times the scale — and again on
every configure after. **The format is a non-sRGB `Bgra8Unorm` or `Rgba8Unorm`**, so the values
written are the ones the editor's surface gets, which egui_wgpu chooses the same way. **The
surface is opaque**: a Wayland surface with alpha is blended with the desktop behind it, so a
mix whose alpha is below one would show the desktop through the show. Composite alpha is
`Opaque` where the surface offers it, and `set_opaque_region` on each configure tells the
compositor the same thing, so it need not blend at all. **A surface is dropped before its
`wl_surface`**, which is why `App::on_exit` stops this thread before anything else.

**One display means shared wakeups.** Because both threads read one `wl_display` socket,
libwayland wakes every reader on it: each thread wakes on the other's traffic and goes back to
sleep with no work done. Measured on the GL renderer at a few hundred wakeups a second for a tenth of a
percent of a core each way, going to **zero on both** when neither window is visible. That is
the price of borrowing the display, and it buys a window that opens in front of the editor.

**Wayland only, on Linux.** Any other Linux session gets no picture windows and a line saying
so. There is no second path, and an X11 one is not wanted: it would be a whole windowing
backend for a case this instrument does not have.

**On Linux, `unsafe` is two things, both in `render/picture/`**: the borrowed display, wrapped
where the thread starts, and each window's wgpu surface, made over raw handles whose
`wl_surface` the window drops only after the surface. The protocol side is `wayland-client` and
`smithay-client-toolkit`, which are safe.

### On macOS and Windows

AppKit makes and drives windows from the main thread alone, and Win32 hands a window's
messages to the thread that made it, which winit allows to be the event loop's alone, so a
thread that owns windows is Linux's alone. On macOS and Windows each picture window is **a
winit window made inside eframe's own event loop**, in `render/picture/winit/`, one
implementation for both. `render::picture::run` builds that loop, starts eframe in it through
`eframe::create_native`, and runs `picture::Loop` around it on the main thread, passing eframe
every event that is not for a picture window. `main.rs` calls `run` on every system, and on
Linux `run` is `eframe::run_native` and nothing else. `proposals/macos-windows.md` is the
argument, written for the Mac.

**The threading contract, on macOS and Windows.**

| | owns | borrows | never touches |
| --- | --- | --- | --- |
| the main thread, in `Loop` | every picture window's winit `Window`, its keys, its pointer and its fullscreen | eframe, which it forwards to | the graph, the snapshot, egui, the command bus |
| a thread named `picture`, one per window | that window's wgpu surface | a clone of the one `Gpu`, the synth's `Arc<Live>`, a `Viewer` per surface format shared by every window | the window itself |

**An ask needs no wake-up.** The editor's `picture::Ask` is sent from `App::ui`, which here
runs inside winit's `RedrawRequested` on the main thread; winit calls `about_to_wait` in the
same turn of the loop, and `Loop` acts on the ask there. `picture::Told` comes back on a plain
channel the editor drains once a frame. The two ends meet through a slot on the main thread:
`run` fills it before eframe starts, and `Host::for_eframe` empties it inside `App::new`,
sending the loop the device and `Live`. A harness never runs `run`, finds no slot, and is
detached.

**A window's surface is made on the main thread and drawn on the window's own.** winit hands
out a window handle on the main thread alone, and on a Mac the `CAMetalLayer` under it is made
from the view, which AppKit allows nowhere else; the surface then moves to the window's thread, which
configures it, paints and presents. That thread's paint is Linux's: the newest `Published`,
a clear to black, a blit with `Fit::Letterbox` over `Viewport::whole`, the submission, the
present, and the `Published` let go. The surface is the non-sRGB `Bgra8Unorm` and opaque, as
on Linux.

**A window's clock is `Fifo`.** Metal has `Fifo` and `Immediate` and no `Mailbox`, and Direct3D
12 on Windows paces `Fifo` by the display's vertical blank as well. The surface presents in
`Fifo` with two drawables, so the thread blocks in `get_current_texture` until one
is free and paints once per refresh of the display the window is on, at most one refresh
behind. That is why there is a thread per window rather than one for all: the acquire blocks,
and on one thread a window on a 60 Hz projector would hold back one on a 120 Hz screen.

**A picture window nobody can see stops painting.** AppKit's occlusion, or a minimized
window on Windows, reaches the window's thread as winit's `Occluded`, sent on by `Loop`, and the thread blocks on its channel with no
timeout until the window can be seen again, is resized or is closed. wgpu answers `Occluded`
at once for such a window rather than waiting for a drawable; the thread sleeps on that too,
for a quarter of a second at a time, in case wgpu saw it before the window system said so.

**A window is dropped on the main thread, after its thread.** A winit window dropped anywhere
else closes itself by waiting on the main thread, so a drawing thread holding the last of one
while the main thread waited for it would wait for ever. Closing a window sends its thread
`Close` and waits up to a tenth of a second for it to end, which a visible window's thread does
within a refresh or two, and then drops the window. A thread that has not ended is waiting for
a drawable a hidden window may never be given: its window is hidden and held until it has. On
quit, `Loop` does the same after eframe's own exit, and lets go of any window whose thread is
still waiting without dropping it.

**Fullscreen covers the window's screen at once, in place.** On a Mac it is
`set_simple_fullscreen`, on a window made with `with_borderless_game`, so the menu bar and the
Dock are hidden outright while supersilvia is in front, with no animation and no Space of its
own, and a window casts no shadow. On Windows it is winit's borderless fullscreen on the
monitor the window is on, which covers the taskbar and changes no display mode. The move, the
resize band and the aspect lock are the same on both: the gesture `Loop` computes from
`picture::dragged`.

**No `unsafe`.** winit's safe API is the whole of `render/picture/winit/`, which stays under the
crate root's `deny(unsafe_code)`.

## Syphon

On a Mac a picture can be **published over Syphon**, so another app — Resolume, VDMX,
MadMapper, OBS — takes it as a source. `proposals/syphon.md` is the argument and the
decisions; the framework is `vendor/syphon/`, linked by `build.rs` on macOS alone. What is
published is every Output switched on in its **Syphon** row, under its one name — the one NDI
sends it under, `supersilvia Output <id>` until it is given another
([ui.md](ui.md#sending-an-output-out)) — and the mix while the Main Mixer's Syphon mark is lit,
as `Mix` ([ui.md](ui.md#syphon)); the app name other apps list beside it is the process's own,
`supersilvia`, which the framework fills in and a server cannot set. A renamed Output's server
is renamed in place (`setName:`).

**Our own drawing into the framework's surface.** A server is Syphon's base class,
`SyphonServerBase`, bound in `platform::macos::syphon` and reached through the
`platform::syphon` service, which Linux answers as a machine without Syphon. Its one shared
surface — 8-bit BGRA, made again only when the size changes — is made a texture on the one
device by `dmabuf::surface_target`, the way a clip's `IOSurface` is imported, but drawn into:
render-target usage, shared storage, nothing cleared. `render::syphon::Outlet::draw` clears it,
blits the picture into it with a picture window's `Viewer` at the picture's own size, and
submits; `Outlet::publish` tells the clients once that submission has finished. There is no
second queue, no raw command buffer, no copy, and neither the framework's Metal server nor its
shader library is used. The server's device is ours because it has none: the base class draws
nothing.

**The publisher is a thread of its own, named `publisher`** (`render::publish`), shaped like a
macOS picture window's, and it sends [over NDI](#ndi) as well.

| | owns | borrows | never touches |
| --- | --- | --- | --- |
| a thread named `publisher` | every Syphon server, its surface and the texture over it; every NDI sender, its target, its staging buffers and its pipeline; and a `Bgra8Unorm` `Viewer` made on it | a clone of the one `Gpu`, the synth's `Arc<Live>` | the synth, the editor's frame, the graph |

The editor hands it a list — `render::publish::Wanted`, each a picture, a way out (`Via::Syphon`
or `Via::Ndi`), a name and a `Look` — once a frame, and `Publisher::want` sends it only when it
changed; the thread starts the first time the list is not empty. It wakes when the synth sets a
new `Published` (a `Live` watcher, which only sends it a message) and when a submission of its
own finishes (a completion callback on the one queue, which runs on whatever thread next polls,
and a poll of its own every few milliseconds while a frame is on the GPU, which never waits).
On each wake it publishes every Syphon outlet whose submission has finished and hands every NDI
sender the frames whose maps have landed, then draws every outlet whose picture's `drawn_tick`
is newer than the one last drawn for it and that can take one — for Syphon, a server nothing is
being drawn into and that some client reads, so a server nobody reads costs nothing. **It never
waits on the synth or on the GPU.**

**Zero flash on a new size.** The new surface reaches clients with the publish after the frame
drawn into it has finished; until then they keep reading the old one, which their own reference
keeps. **A server goes only once its last frame is drawn**: one no longer wanted waits, as
does every one when the publisher stops — on quit, after the picture windows and before the
synth — for its last submission, at most a second, and dropping it stops it and retires its
announcement.

**Orientation and alpha, as the loopback shows them.** `tests/syphon.rs` publishes a
four-quadrant picture through an `Outlet` and reads the surface back through Syphon's own
client in the same process, found by the framework's own directory; then does the same through
the publisher's thread from a `Live` it sets, a new size included. With the default `Look`, the
surface's memory holds the picture's **bottom row first** — OpenGL's layout, which is how
Simple Client, OBS and ofxSyphon draw a surface — and it is **opaque over black**, alpha 255
everywhere and a half-covered pixel's color as it would be over black. With `Look::flip` and
`Look::transparent` (the Output's **Flip** and **Alpha**) it holds the top row first
and the picture's own alpha, **premultiplied**, as the graph holds it: Syphon defines no alpha
convention of its own, and Core Animation and Metal composite premultiplied, so a Mac app that
draws the surface over something gets the picture's edges right. The blit writes a picture's top row first, so the
default flips once more by drawing the picture as though its rows were the other way up. The
framework's own Metal server keeps the same choice with its `flipped:` argument, `NO` copying a
Metal texture's top row to row 0.

**Discovery needs the main thread's run loop**: the framework's directory is made when it
loads, on the main thread, and hears announcements as distributed notifications there. eframe's
event loop runs it; the loopback test is a harness-less `main` that pumps it. Frames need no
run loop: they cross Mach ports the framework services on queues of its own. **A client's
frame handler asks for nothing**: the framework's `stop` holds the client's lock while it waits
for the frame queue, and `newSurface` takes that lock, so the handler only wakes a thread of the
client's own, which asks.

## NDI

A picture can be **sent over NDI®** ([ndi.video](https://ndi.video)), so another machine on the
network — Resolume, OBS through DistroAV, TouchDesigner, vMix, or supersilvia on a second
laptop — takes it as a source, on Linux as on a Mac. `proposals/ndi.md` is the argument and the
decisions; the plugin and the runtime it needs are [media.md](media.md#ndi)'s. What is
sent is every Output switched on in its **NDI** row, under its one name — `supersilvia Output <id>`
until it is given another ([ui.md](ui.md#sending-an-output-out)) — and the mix while the Main
Mixer's NDI mark is lit, as `supersilvia Mix` ([ui.md](ui.md#ndi)); the network shows each as
`MACHINE (supersilvia Output 3)`. Picture only: the mix has no sound of its own to send.

**The Syphon outlet's blit, then a readback, on the same thread** — the publisher above,
`render::ndi::Outlet` beside `render::syphon::Outlet`. The picture is drawn with the thread's
`Bgra8Unorm` `Viewer` into an 8-bit BGRA target of its own at the picture's own size, **top row
first**, which is the one orientation NDI has, so the blit draws it as it is; then
`copy_texture_to_buffer` copies it into a staging buffer, rows padded to 256 bytes, and the map
is asked for once the submission is made — [the thumbnail's](#the-thumbnail-readback) shape,
every frame. **Nothing waits**: on each wake the thread takes every map that has landed, oldest
first, drops the padding while copying the rows into a buffer from a GStreamer pool, and pushes
it into that sender's own `appsrc ! ndisink` pipeline (`video::ndi::Sender`), whose `appsrc`
keeps two frames and drops the oldest past that, and whose `ndisink` hands the runtime the
frame on the pipeline's thread, where it is compressed. Two staging buffers are in flight at
most; **a picture that arrives while both are still out is not drawn**, so the newest frame
that can be read back wins and one that cannot is dropped, as a camera's is. A new size makes a
new target, and a read still out on the old one is never sent. A sender goes at once when it is
no longer wanted, and a renamed one is announced again, since NDI fixes a name when it is
announced: the outlet is kept by its picture and its way, so a rename makes the new sender,
drops the old — whose pipeline going to `Null` takes its stream off the network — and draws the
new one the picture it has rather than waiting for the next.

**Alpha follows the Output's Alpha**, the one Syphon reads too: opaque by default,
drawn over black and sent as BGRx, which NDI carries with no alpha plane; transparent, sent as
BGRA with the picture's own alpha **straight**, since the NDI SDK defines its BGRA and RGBA
frames as not premultiplied. The transparent picture is drawn by the viewer's second pipeline,
`Viewer::show_straight`, which divides by alpha after the sampler and writes rather than
blends, so a pixel the picture does not cover stays transparent black. `tests/ndi.rs` sends a
premultiplied half-transparent red through the publisher and receives full red at half alpha.
**Flip is Syphon's alone.**

**The rate, and the time.** An NDI stream declares a frame rate and each frame a timecode. The
caps declare the rate the synth ticks at — the display's, or the preference's — rounded to a
whole number and handed over with the list, so a stream is announced again only when the rate
itself changes. Each frame is stamped with the pipeline's clock as it is pushed (`appsrc`'s
`do-timestamp`, live), which `ndisink` turns into its NDI timecode, the clock's own time, and
sends as it arrives (`sync=false`). The synth's rate varies by a frame here and there; how every
receiver treats an uneven stream is not known.

**The cost is a readback and an encode per sent picture per frame**: 1080p BGRA is about 8 MB a
frame, copied off the GPU into staging and once more into the pipeline's buffer, and compressed
by `libndi` on the CPU. Whether the runtime skips the compression while nobody receives is
not known; the publisher draws a sent picture whether or not anything receives it, since the
plugin does not say.

**Where the runtime is missing**, a sender is refused and the thread says nothing: the Output's
NDI row and the mark's hover say it ([ui.md](ui.md#ndi)). **Any other refusal, and a frame that
fails, is said back**: the thread keeps a list of `render::publish::Failure`s — a picture, a way
and why — written only when it changes and read by the editor once a frame, and the row that
asked for the outlet shows it ([ui.md](ui.md#sending-an-output-out)). `tests/ndi.rs` sends an Output's picture
through the publisher's thread from a `Live` it sets and receives it with `ndisrc`, top row
first and with its alpha, where the runtime is installed, after the plugin's own loopback.

NDI® is a registered trademark of Vizrt NDI AB.

## Showing a frame

A picture is drawn by **blitting its texture in a paint callback** — the preview panel shows
[the mix](#the-mixer), and each Output node draws its own frame on its body.

egui_wgpu's `register_native_texture` is the other way, and is not used. A ring rotates the
texture an Output shows every tick, so the registration would be redone every frame, and the
fit, the corner and the flip would live somewhere other than the blit. A paint callback needs
none of that and is the same operation into a smaller rect. **Most apparent platform constraints have
a shared solution simpler than the platform-specific one; reach for the shared path first.**

Both blits fit rather than stretch, and the fit is the caller's — `Fit::Letterbox` for the mix
in a panel or a picture window, and for the mixer panel's channel previews, fixed 16:9 boxes an
Output of any resolution is shown whole in; `Fit::Cover` for a background and for an Output
node's own picture, whose slot `widgets::picture::RENDER` already sizes to the Output's exact
aspect, so nothing is cropped or barred and no black ground can show at an edge — and the fit
is computed against the **whole** rect the caller asked for, which may reach past the window on
any side. The scissor egui_wgpu set from the callback's clip rect is what cuts the picture off;
the renderer never fits a picture to it. The callbacks `render::viewer` builds convert the
callback's rect to pixels themselves (`viewport_of`), with nothing clamped, because
`PaintCallbackInfo::viewport_in_pixels` clamps to the window and a clamped rect makes an Output
at the edge of the canvas shrink its render to fit the part still on screen — see [paint
callbacks](#paint-callbacks-and-the-viewport-they-are-given).

**An Output node's picture is flush with its body**, sides and bottom, as silvia's
`.output-canvas` is, and the body's bottom corners are round. The blit rounds the picture's
own two bottom corners to match: its uniform block carries the rect and the radius in pixels,
and the fragment discards what lies inside a corner's square but outside its arc, measuring
from the rect's bottom edge, since a target's row 0 is its top. The alternative — holding the picture off the sides and bottom by
`r · (1 − 1/√2)` so its square corners landed inside the body's arc — left a visible margin
of body around the picture, which read as black around the video. A radius
of zero, a picture window's and the channel previews', rounds nothing.

## Linking happens off the synth thread

A driver shader compile costs tens to hundreds of milliseconds, and [the recompile
boundary](architecture.md#the-recompile-boundary) means every structural edit triggers one.
Paying that on the thread that ticks the world is a visible stutter at the moment somebody is
playing — and the one thing a tick is allowed to cost a frame, which is why `DropCause` names
it.

wgpu has no asynchronous pipeline creation on native: `create_shader_module` and
`create_render_pipeline` return once the driver has compiled to machine code. The device is
`Send + Sync`, so **links are made on threads of ours, named `linker`**
(`render/link.rs`): `Programs::set_shader` sends the module to them and returns, and
`Programs::poll`, once a tick, is a `try_recv` on the Output's own channel that swaps in a
program that has landed. A few linkers take from one queue, so a project opening links its
Outputs side by side rather than one after another. **The current program keeps rendering the
whole time**, so an edit never shows a blank frame.

- **No shader module and no pipeline is made on the synth or the frame thread** but at
  startup: `Program::create` blocks for as long as the driver compiles, and runs on a linker.
  The mixer's, the viewers', the conversion pass's and the readback passes' pipelines are made
  once, with their owners. A module that does not parse, or a pipeline the device refuses, is
  caught by a validation error scope around the creation — thread-local, so on a linker it
  holds that link's errors and nothing the synth is doing on the same device — and comes back
  as the message for the status line, naga's naming the function.
- **A newer source supersedes one still in flight**, which during a drag is every frame. Every
  request is numbered and the Output keeps the number it wants in an atomic the linkers read:
  a linker drops a request no longer wanted rather than build it, and `Programs::poll` drops
  any result that is not the one pending, so a superseded link is never shown.
- **A program is kept by its source.** One replaced by a newer link, or by the Output going
  dark, is put aside under a hash of the module it was linked from, `KEPT_PROGRAMS` — four —
  deep per Output and per probe, the oldest let go when a fifth joins them. A source that comes
  back — an undo restoring the graph before an edit, a cable pulled and put back — is swapped in
  on the tick it arrives and links nothing, and a source the Output is already drawing with or
  already linking sends nothing at all.
- **Every module is made with every runtime check on.** `create_shader_module` hands naga
  wgpu's defaults, so naga puts a counter into every loop it writes, which costs the
  loop-heavy per-pixel nodes a fifth to a quarter of their GPU time on the iGPU. What it is and
  what could change is [loop-bounding.md](loop-bounding.md).
- **No pipeline cache of our own.** Mesa's Vulkan drivers keep a disk cache of compiled
  shaders, so a second run of a project links from it, and wgpu's `create_pipeline_cache`
  takes bytes it cannot validate.
- Failure leaves the working program running, which `tests/gpu_link.rs` asserts directly.
- **A program draws with what its own source was compiled for.** A job names the source its
  uniform list and tap slots belong to (`OutputJob::source`), and the list moves on the tick
  an edit is compiled, while the program changes only once the new one has linked. So each
  program keeps what the last job naming its source carried (`Programs::adopt`) and
  draws with that until it is replaced: a sampler the new list dropped still samples its own
  input, the controls hold where they were for the frames of the link, and a link that fails
  leaves the old picture as it was. Every texture is looked up as the draw's bind group is
  made, and one that is not there binds the shared black texel. A program that lands from a
  link no job has named since draws nothing until one does, and its Output keeps its last
  frame.
- **A source waits for a job that draws its Output.** The renderer does nothing with a
  suspended Output's job — one on closed workspaces only, or every Output while [a
  render](#the-render-job) holds — so the synth sends a source only with a job that is not
  suspended, and it stays in the plan until then: an Output whose tab comes back during a
  render's hold draws its new program on the first frame the render steps.

Undo and redo rebuild every Output's shader on the editor, since a graph restored whole may
differ anywhere, but only a source that came out different is sent, and one the renderer drew
with before is swapped in without a link — so an undo of a move links nothing, and an undo of
an edit links nothing it had already linked. See
[architecture.md](architecture.md#the-two-channels).

## Discipline

- **A block layout is computed on link** (`compile::wgsl::uniform_layout`), so nothing is
  looked up in the per-frame path. Every draw's block goes into one uniform buffer a tick, one
  `queue.write_buffer`, bound at the draw's own offset; never a buffer per draw.
- **A bind group is made per draw**, because rings rotate what every consumer samples; a
  simulation's two are made once per world. Nothing else is allocated on every frame: query
  sets, staging buffers and the samplers are made with their owners and reused, and a picture
  ring allocates up to five targets on demand as viewers retain frames or its size changes.
- **Nothing waits on the GPU, on any thread**, but the bounded waits named above: a render's
  frame, the tick's bound and the throttle. The readbacks — a tap's buffer, a thumbnail's
  pixels, a Snap's, the timer's span, a draw's marks, a probe's counts — are all collected once
  their map's callback has fired, and each asks whether the answer is ready rather than
  demanding it.
- **A failed shader link keeps the old program** and reports to the node's status line, so a
  typo does not blank the screen mid-performance.
- **Nothing is freed by hand**: every GPU object is freed once nothing holds it and the GPU is
  done with it, and `App::on_exit` only stops the two threads that draw.
