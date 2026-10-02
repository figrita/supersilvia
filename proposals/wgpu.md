# Proposal: wgpu in place of glow

**Status: steps 1–6 are built** — adapter selection, the seam on glow, the skeleton of
`src/render_wgpu/`, the compiler's WGSL with the node lanes 4a–4e, the renderer lanes 5a–5i, and
the flip, which deleted glow and GLSL and renamed `render_wgpu` to `render`. Step 7, a
measurement of the flipped app by hand, is next; steps 8 and 9 are open. The decision is to
convert everything to wgpu: a native macOS app on Apple Silicon (Metal),
and Linux on Vulkan, one renderer for both. This is the design and the staging plan. Two things are settled elsewhere and only referenced here:

- **What wgpu draws is WGSL.** Every node is rewritten in it, beside its GLSL until the flip,
  and naga takes it to MSL and SPIR-V with nothing between. See `proposals/shader-path.md` for
  the decision and [docs/nodes.md](../docs/nodes.md#wgsl) for the conventions. What `render/`
  gets from the compiler is in
  [what the shader path gives the renderer](#what-the-shader-path-gives-the-renderer).
- **Linux-only code outside `render/`** is moving into `src/platform/`. This proposal says what
  a platform layer must provide for picture windows, and leaves where the files live to that
  move.

## Why glow cannot come along

macOS's OpenGL stops at 4.1, deprecated. 4.1 has no shader storage buffers, no atomics in a
fragment shader, no compute shaders and no image load/store. Taps are atomics into a storage
buffer from inside the Output's own pass (GLSL 4.30, `compile::TAP_DECL`), the cost probe is the
same, and `slimemold` is compute kernels over storage buffers and `r32f` images
(`render/sim.rs`). None of it runs on a Mac through GL. Keeping glow on Linux and writing Metal
for the Mac would be two renderers, which is the shape this project has refused everywhere
else. So Linux moves too, onto Vulkan through the same wgpu code.

## The shape in one paragraph

One `render::Gpu` — a wgpu `Instance`, the `Adapter` chosen by
[the adapter rule](#4-adapter-selection), one `Device` and its one `Queue` — is made in
`main()` before eframe starts. eframe is handed it through
`egui_wgpu::WgpuSetup::Existing`, so the editor paints through the same device. The synth
thread and the pictures thread get clones; `Device` and `Queue` are `Send + Sync` and `Clone`.
**There is no second context, no share group, no current context and no `unsafe` handle
passing.** The ring, the leases, the plan, the draw rule, zero flash and the one tick in flight
all keep their meaning. What changes is how "has the GPU finished this?" is asked (one counter
of completed submissions replaces every fence), how a readback lands (a staging buffer and
`map_async` replace persistent mappings), and how a link happens off the synth thread (a worker
thread of ours replaces `GL_KHR_parallel_shader_compile`).

---

## 1. Every GL mechanism, and what replaces it

### 1.1 Three contexts become one device

Today (`render/egl.rs`): `SharedContext::create` makes the synth's context on the frame thread
while eframe's is current, surfaceless, at low priority through `EGL_IMG_context_priority`.
`SharedContext::handles` and `from_handles` make the pictures thread's third context. Every
context calls `release_pending` once a frame.

With wgpu all of that is gone:

| today | with wgpu |
| --- | --- |
| `SharedContext::create` on the frame thread, `make_current` on the synth thread | the synth thread is handed a `Gpu` clone; nothing is made current |
| `Handles` read on the frame thread before the synth's context leaves | the pictures thread is handed a `Gpu` clone |
| `EGL_KHR_surfaceless_context` required, "no pbuffer fallback" | nothing required: a device needs no surface |
| "the synth's context is current on exactly one thread" | no such invariant; wgpu serializes internally |
| `Viewer` owns its own program and VAO per context, since containers do not share | one `Viewer` pipeline per surface format, usable from any thread |
| `release_pending` per context, per frame | deleted: it drained Mesa's threaded-GL parking across a share group, and there is no share group |
| `SYNTH_PRIORITY`, `SUPERSILVIA_SYNTH_PRIORITY` | no equivalent on one queue; see [the queue risk](#the-one-real-risk-one-queue) |
| `App::on_exit` frees GL objects while the context is current | resources are reference-counted; dropping the renderer frees them whenever the GPU is done with them |

Step 2 already made the synth's door `Synth::attach_gpu(Gpu)`, the tests' included, with the
glow `Gpu` carrying the synth's own `SharedContext`; the flip changes what a `Gpu` holds and
not who takes one. The synth names no graphics crate, and `tests/rules.rs` holds it to that.

**One place wgpu did not serialize: a surface configured while another thread submits.**
wgpu-core 30.0.1's `Surface::configure` waits for the device to go idle and refuses the
configuration, which the default handler turns into a panic, if anything was submitted during
the wait — on the one device, whenever the synth is running. The editor's surface at launch
and on a resize and every picture window's raced it, and about four launches in ten crashed on
the Mac. The build takes wgpu-core from `vendor/wgpu-core`, 30.0.1 with upstream's fix
(gfx-rs/wgpu#10296) in its one match arm, until a release carries it
([decisions.md](../docs/decisions.md#wgpu-core-carries-one-patch-so-a-surface-configures-beside-the-synth),
[vendor/README.md](../vendor/README.md)); `tests/gpu_surface.rs` holds it.

### 1.2 Every fence becomes one counter of finished submissions

GL puts a fence anywhere in the command stream and polls it with a zero timeout. Vulkan and
Metal know completion **per submission**. So the renderer counts its submissions and keeps one
number: the newest submission known to have finished. `Queue::on_submitted_work_done` registers
a callback per submission that stores its serial into an `AtomicU64`; `Device::poll(PollType::Poll)`
once a tick (in `Renderer::publish`, where the zero-timeout polls are today) runs whatever
callbacks are due. A "fence" is then a serial, and "has it signaled" is a comparison.

The callbacks fire on whichever thread polls or submits — the editor's `queue.submit` inside
egui_wgpu runs them too. So a callback only stores into an atomic or a slot; it never touches
renderer state.

| GL fence today | where | with wgpu |
| --- | --- | --- |
| a slot's fence, placed by `Ring::commit` | `ring.rs` | the slot's `drawn_in` serial |
| `Ring::poll` / `Ring::finished(drawn, 0)` | `ring.rs` | `drawn_in <= completed` |
| the tick's fence, `Renderer::ticks`, waited with `QUEUE_WAIT_NS` | `mod.rs` | the previous tick's last `SubmissionIndex`, waited with `device.poll(PollType::Wait { submission_index, timeout: 1 s })` |
| a capture's own frame, waited 2 s (`CAPTURE_TIMEOUT_NS`) | `output.rs` | the same wait on that frame's submission |
| `source_fence`, a `glWaitSync` a viewer queues before sampling an upload | `mod.rs`, `viewer.rs` | deleted: see [1.5](#15-viewers-leases-and-retire) |
| reader fences a viewer hands back through a `Lease` | `retire.rs`, `viewer.rs` | deleted: see [1.5](#15-viewers-leases-and-retire) |
| the `unimported` queue's fence before a DMA-BUF goes back | `mod.rs` | a serial taken when the frame is let go; see [1.11](#111-dma-buf-import) |
| a readback's fence (`Readback::fence`, `CaptureSlot::fence`) | `output.rs` | the `map_async` callback's flag |
| `GL_QUERY_RESULT_AVAILABLE` | `output.rs`, `timing.rs` | the `map_async` callback on the resolved query buffer |

**Granularity is the one thing that could get worse.** A GL fence after a cheap Output signals
when the GPU passes it, even though heavier Outputs follow in the same submission. A Vulkan
submission finishes as a whole. If a tick were one submission, every Output's frame, tap words
and GPU time would land when the heaviest had finished, which is the pathology the persistent
mappings were written to escape on iris (`OutputRenderer::settle`'s module doc; 38 ms measured).
So **each Output is its own submission**, in plan order: uploads and simulations first, then one
per Output, then the probes and the mix. A cheap Output's frame is shown, and its readings
taken, as soon as it is done. The cost of 40-odd submits a tick is on the
[watch list](#costs-to-watch).

A side effect is a truer bound. On iris the real queue was deeper than `TICKS_IN_FLIGHT`
because Mesa batched and the kernel's execbuffer throttle held the rest (`ring-of-three.md`).
A wait on a submission index is a wait on that submission's own fence.

### 1.3 The ring, and every row of the zero-flash table

`Ring` (`ring.rs`) is CPU bookkeeping over five slots, and all of it stays: latest, shown,
never drawn into while leased or unfinished, stale sizes freed when nothing reads them. A slot
becomes a `wgpu::Texture` with a view (`RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`,
`Rgba16Float` for an Output, `Rgba8Unorm` for the mix), with no framebuffer object. Each row of
[rendering.md's table](../docs/rendering.md#zero-flash):

| transition | how it still holds |
| --- | --- |
| shader recompile | the old pipeline draws until the new one arrives from the link worker ([1.13](#113-linking)) |
| a link that fails | the worker's naga error or validation error goes to the status line; the old pipeline stays |
| a sampler whose texture is gone | the bind group takes the 1×1 black texture's view, as `bind_uniforms` takes `Renderer::black` |
| resolution change | the carry is a render pass into the new slot sampling the old latest with a linear sampler — there is no `glBlitFramebuffer` — and viewers keep the old size until the carry's submission has finished |
| a GPU that cannot keep up | `device.poll(Wait)` on the previous tick's submission slows the tick; viewers get the newest finished slot |
| a dropped frame — every target held | `Ring::free_slot` returns `None`, unchanged: leases are CPU-side |
| a frame still on the GPU | shown is the newest slot with `drawn_in <= completed` |
| an Output with nothing connected | `go_dark` is a pass with `LoadOp::Clear` into a free slot, or into the unleased latest at the limit |
| an Output claimed by a mixer deck | the mix's bind group names the deck's latest view; no texture is made. The test still asserts the texture set is unchanged |
| going idle, idle drawing again, idle since made | plan-level (`Synth::drawing`, `wants_picture`); untouched |
| the mix changing size | as an Output's resize: the carry pass, then the mix drawn on the same tick |
| a drag at *Match viewport* | app-level (`MIX_SETTLE_S`); untouched |
| a resize with the ring at its limit | `Ring::resize` returns false and asks again, unchanged |
| a simulation's world changing size | `SimRenderer::reshape`'s blits become a compute pass that resamples the field with `textureLoad` (an `R32Float` texture is not filterable) and a render pass that scales the picture |
| a simulation whose kernels are still linking | the tick queues, unchanged; the kernels link on the worker |

**Feedback stays a one-frame delay with no synchronization in it.** An Output sampling its own
`frame` reads the latest slot while drawing into another. wgpu forbids one texture as both a
sampled binding and an attachment in one pass, and the ring never does that.

### 1.4 The tick's bound on the GPU

`TICKS_IN_FLIGHT` and `QUEUE_WAIT_NS` (1 s) keep their meaning; `TICKS_IN_FLIGHT` went from
one to two once the throttle bounded the queue by submissions ([the synth's
submissions](#the-synths-submissions-after-the-flip)). `Renderer::draw` begins with
`device.poll(PollType::Wait { submission_index: Some(last of previous tick), timeout })`. A
`PollError::Timeout` is counted in `timeouts` and logged once, as today. `READBACKS`
(`TICKS_IN_FLIGHT + 1`), `timing::DEPTH` and `CAPTURE_RING` keep their sizes.

### 1.5 Viewers, leases and retire

What the lease protects has two halves today:

1. **Nothing a held `Published` names is freed.** With wgpu this is free: a `Published` holds
   `wgpu::TextureView` clones, a view keeps its texture alive, and wgpu keeps anything a
   submitted command buffer uses alive until the GPU has finished. **`Retire` is deleted.**
2. **No Output's or mix's slot a `Published` names is drawn into.** This is about what a viewer
   is *shown* (a finished frame, never the one being drawn), not about memory. It stays exactly
   as it is. `Lease` shrinks to a plain claim — the `Arc` whose count `is_free` asks — with no
   `Readers`, no `ReaderFence`, no `hand_back`, no `wait_readers` and no process-wide
   `READER_FENCES` count.

**Why the reader fences go.** Today the synth and a viewer are two GL contexts, and GL orders
work across contexts only through a sync object the writer waits on. With one device there is
one queue, and wgpu inserts the barriers between submissions on it in submission order. A viewer
submits its blit, then drops its `Published`; the synth draws into that slot only after its
lease is free, so its submission comes later and the GPU runs it later. The rule a person must
still read for becomes: **a viewer holds its `Published` until after the `queue.submit` that
carries its reads.** For the editor that is egui_wgpu's submit at the end of the frame (the
`Published` lives on `App` until the next frame's snapshot, as now). For a picture window it is
the submit before `present`.

**Why the source fence goes.** A CPU node's texture and a simulation's picture are rewritten in
place while a viewer may hold them. On one queue an upload and a blit are ordered by submission,
with wgpu's barrier between them. A viewer sees the frame before the rewrite or the one after,
whole, and never a torn one. `Picture::fence`, `FrameFence` and `source_fence` are deleted, and
`Viewer::show` loses its `glWaitSync`.

`Viewer::fence_reads` is deleted with them. What the editor's frame owes its context on either
side of its painting, `Viewer::open_frame` (`release_pending`) and `Viewer::close_frame`
(`fence_reads`), becomes nothing; `app/frame.rs` still calls both, and each can stay an empty
method or go with its two call sites.

All of this is Plan A, one device for everything, which is what the skeleton builds: the
synth shares the device and its one queue, and [its throttle](#the-one-real-risk-one-queue)
bounds what an editor frame waits behind. Should the synth ever move to a device of its own,
[Plan B](#plan-b-the-synth-on-a-device-of-its-own) brings a CPU-side version of the reader
fences back between the two.

### 1.6 Readbacks: taps, thumbnail, Snap, capture

`render::output::Mapped` is immutable storage mapped once, persistent and coherent, read with no
GL call once the fence after its write has signaled. wgpu has no persistent mapping of a buffer
the GPU writes. A buffer with `MAP_READ` may only be `COPY_DST` (without the native-only
`MAPPABLE_PRIMARY_BUFFERS`, which this should not lean on), and it must be unmapped whenever a
submission uses it.

So each readback becomes **a GPU-side buffer, a staging buffer, and a map per read**:

- **Taps.** Each in-flight record (`InFlight`) keeps a tap buffer (`STORAGE | COPY_SRC |
  COPY_DST`) and a staging buffer (`MAP_READ | COPY_DST`). Per frame, in the Output's own
  command encoder: `copy_buffer_to_buffer` from the template (the same queued-behind-the-last-write
  shape as `glCopyBufferSubData` today), the pass, then `copy_buffer_to_buffer` into staging.
  After the submit, `staging.slice(..).map_async(Read, …)`. `settle` reads the words out of
  `get_mapped_range` once the callback's flag is set, then unmaps. The record is reused only
  after that, which `READBACKS` already guarantees.
- **Thumbnail and Snap** (`Readback`). The letterboxed or whole-frame blit is a render pass into
  the `Rgba8Unorm` target, then `copy_texture_to_buffer` into staging and a map. Rows must be
  padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256 bytes). A 240-wide thumbnail is 960 bytes a row
  and pads to 1024; the padding is dropped as the rows are copied out.
- **Capture** (`Capture`). The halvings (`issue_capture`'s `hop`) become one or two render
  passes with a linear sampler, then a padded copy and a map. The two-second bound is a
  `device.poll(Wait)` with that timeout on the frame's submission.
- **`flip_rows` stays.** See [1.15](#115-orientation-nothing-moves-in-memory): row 0 in memory is
  still the bottom of the picture.

"A read waits for nothing queued behind its frame" still holds, because a map completes when its
own submission does, and each Output is a submission of its own ([1.2](#12-every-fence-becomes-one-counter-of-finished-submissions)).

The test-only reads (`read_output`, `read_latest`, `SimRenderer::read`, the tests' `rgba_of` and
`pixels_of`) become one helper: copy into a padded staging buffer, submit,
`poll(Wait)`, map. They wait, and only tests call them, as now.

### 1.7 Tap buffers and atomics in a fragment shader

A storage buffer bound `read_write` and visible to the fragment stage. It needs
`DownlevelFlags::FRAGMENT_WRITABLE_STORAGE`, which Vulkan and Metal both have. `atomicAdd`,
`atomicMin` and `atomicMax` on `uint` are core. Apple GPUs do device-memory atomics in fragment
shaders. In WGSL the buffer is `var<storage, read_write> tap: array<atomic<u32>>` at binding
1, and `tests/shader_targets.rs` takes every tapping module to MSL's
`atomic_fetch_*_explicit` and to SPIR-V.

**"Binding 0 holds the buffer of the program drawing" becomes structural.** wgpu has no global
binding state. Each Output's pass sets its own bind group, and the bind group names the tap
buffer of the kept program's own `Setup`. The bug class the invariant guards (another Output's
tap buffer, or a simulation's agents, still bound from before) cannot be written. The
`memory_barrier(BUFFER_UPDATE | CLIENT_MAPPED_BUFFER)` after the draw is deleted: wgpu orders
the copy into staging after the pass.

### 1.8 Uniforms, textures and samplers

`bind_uniforms` sets loose GL uniforms by cached location and binds textures to units from 1
up. With wgpu:

- **Uniform values go in one uniform buffer per tick.** The renderer writes each Output's block
  at its own offset with one `queue.write_buffer`, and each draw binds its slice with a dynamic
  offset. The block is the module's one uniform struct `u`, and
  `compile::wgsl::uniform_layout` gives its members' offsets by WGSL's uniform layout rules —
  `u_resolution` and `u_time` first, then the uniforms by name — so no reflection is needed. "Uniform locations are cached on link" becomes "a block layout is computed on
  link".
- **Textures and samplers are separate bindings.** Wrap and filter are properties of a
  **sampler** in wgpu, not of a texture. `output::set_sampling` becomes four cached samplers
  (mirror/repeat × linear/nearest), chosen per binding from `SourceJob::wrap` and `filter`.
  "Setting them on a texture that already exists allocates nothing" still holds: a change of
  wrap picks another sampler for the next bind group. `Picture` carries the pair too, so a viewer
  blits a `cellularautomata` grid with a nearest sampler. Today the viewer gets that from the
  texture's own parameters (the "no sampler object" note on `Viewer::new`).
- **A bind group per draw per tick.** Rings rotate, so the views an Output samples change every
  tick. Bind groups are made fresh per draw, and their cost is on the
  [watch list](#costs-to-watch).
- **Limits.** wgpu's default `max_sampled_textures_per_shader_stage` is 16. The Intel iGPU and
  Apple Silicon both offer more. The device asks for the adapter's own limits. A shader over the
  limit fails pipeline creation and says so on the status line, which is what an over-limit GL
  link does today.

### 1.9 Compute simulations

`render/sim.rs` maps almost one to one:

| today | with wgpu |
| --- | --- |
| SSBOs 0–3 (`agents`, `state`, `arrivals`, `arrivals_next`) | storage buffers in one bind group |
| `r32f` images 0 and 1, `READ_WRITE` | `R32Float` storage textures, `ReadWrite` (`STORAGE_READ_WRITE` is core for `R32Float`) |
| `rgba8` image 4, `WRITE_ONLY` | `Rgba8Unorm` storage texture, `WriteOnly` |
| `u_size`, `u_from`… and a node's own numbers by name | a small uniform struct, laid out as in [1.8](#18-uniforms-textures-and-samplers) |
| `memory_barrier` after every dispatch, and a texture-fetch barrier after the last | deleted: wgpu inserts barriers between dispatches that write and read the same storage resource, and before the Output pass that samples the picture |
| a flipping pass swaps `front` and `back` | two bind groups per world, made once — front-to-back and back-to-front — so flipping picks the other one |
| `Kernels`: one program per `&'static Kernel`, linked off the thread | one compute pipeline per kernel, created on the link worker |
| `reshape`'s `blit` over two temporary framebuffers | a compute pass (field, nearest, `textureLoad`) and a render pass (picture, linear) |

Arrivals stay integer atomics on a buffer. The reason is in rendering.md (Intel's typed image
atomics serialize) and holds under Vulkan too.

### 1.10 Texture upload from CPU nodes and video

`render/upload.rs` stages every frame through an orphaned pixel-unpack buffer so
`glTexSubImage2D` is queued like a draw. `queue.write_texture` does the same by construction: one
copy into wgpu's staging, queued for the next submit. Per layout:

- **Packed RGBA and RGBA with a padded stride.** `write_texture` with `bytes_per_row` equal to
  the source's stride. `write_texture` has no 256-byte row alignment rule; only
  `copy_buffer_to_texture` does. `UNPACK_ROW_LENGTH` and the `texels` helper go.
- **BGR byte orders.** A `Bgra8Unorm` texture samples as RGBA, so `GL_BGRA` becomes a format
  choice.
- **An `x` byte.** `TEXTURE_SWIZZLE_A` has no wgpu equivalent. `Rgbx` and `Bgrx` go through the
  conversion pass the YUV layouts already take, which writes alpha 1. That is one extra
  fullscreen pass per new frame, for `x` layouts only. It is not a CPU repack.
- **YUV.** The planes stay textures of their own (`R8Unorm`, `Rg8Unorm`, `Rgba8Unorm` for packed
  4:2:2), and the conversion stays one pass (`Convert`) into the source's `Rgba8Unorm` texture.
  wgpu's `NV12` format and its `EXTERNAL_TEXTURE` feature are not needed and not used.
- **"Storage allocated and filled in one call, so there is no frame on which the texture is
  empty."** A new or resized texture is created and written in the same tick, before the
  submit that samples it, so the same holds.

### 1.11 DMA-BUF import

Today (`render/dmabuf.rs`): `eglCreateImage` with the DMA-BUF attributes, then
`glEGLImageTargetTexture2DOES`. Failure is read from `glGetError`, drained first because the flag
is sticky. `importable()` asks EGL for modifiers through `eglQueryDmaBufModifiersEXT`.

**Linux, Vulkan.** wgpu-hal 30 already has the import:
`wgpu_hal::vulkan::Device::texture_from_dmabuf_fd(fd, desc, drm_modifier, stride, offset)`, one
plane, behind `Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF` (`VK_EXT_external_memory_dma_buf` and
`VK_EXT_image_drm_format_modifier`). It is wrapped with `Device::create_texture_from_hal`. Three
things to get right:

- **Vulkan takes ownership of the fd on success**, and the fd is GStreamer's. So it is `dup`ed
  first.
- **`importable()`** becomes `vkGetPhysicalDeviceFormatProperties2` with
  `VkDrmFormatModifierPropertiesListEXT`, reached through `Adapter::as_hal::<Vulkan>()`, keeping
  the same fourccs (`XR24`, `AR24`, `XB24`, `AB24`) and the same one-plane filter (`one_plane`).
  `video/mod.rs` keeps asking `render::dmabuf` and nothing else, so the layering exception in
  architecture.md stays one edge.
- **A frame goes back to its producer** once every submission that could have sampled it has
  finished. When a frame is replaced or its node leaves, the renderer registers an
  `on_submitted_work_done` then. That callback covers every submission made so far by any
  thread, viewers' blits included, and its serial replaces the `unimported` fence and the
  reader fences in one. Failure is an `Err` from the call. The `glGetError` drain, and its test
  `a_stale_gl_error_is_not_read_as_a_refused_import`, go.

Importing makes a `VkImage` and a `VkDeviceMemory` per new frame, as `eglCreateImage` does per
frame today. A decoder's pool recycles a handful of buffers, so caching imports by the
buffer's inode is an easy win if the import shows up in the uploads span.

**macOS, Metal.** No DMA-BUF. The equivalent is a `CVPixelBuffer` backed by an `IOSurface` out
of VideoToolbox, made an `MTLTexture` through `CVMetalTextureCache`, then
`wgpu_hal::metal::Device::texture_from_raw` and `create_texture_from_hal`. It belongs to the Mac
media work, after the flip. The byte path (`upload.rs`) works on the Mac from the first day.

### 1.12 The mixer

`MixerRenderer` is one pipeline made synchronously when the renderer is made. That is fine: it
happens before the first frame, as `shader::link` does today. It has its own ring, a bind group
per tick naming the two decks' latest views (or the black view), and an `int` in its uniform
block for the method. Nothing about "the one program that never recompiles" changes.

### 1.13 Linking

Today `shader::submit` hands the program to the driver and `poll_shader` asks
`GL_COMPLETION_STATUS` once a tick. The driver's threads do the work, and nothing waits for a
compile status.

wgpu has no asynchronous pipeline creation on native. `create_shader_module` and
`create_render_pipeline` do their work before they return, and on Vulkan that includes the
driver's compile to machine code. But `Device` is `Send + Sync`, so **the link moves to a worker
thread of ours**, named `linker`:

- `set_shader` sends `(output, source hash, full source)` to the worker and returns.
  `poll_shader` becomes a `try_recv`. The previous pipeline draws until the new one arrives,
  exactly as now. A newer source supersedes one still in flight by hash. The worker drops a
  stale request rather than build it, and a stale result is dropped on arrival.
- **Errors.** The front end's parse and validation errors come back as a string for the status
  line. A pipeline error is caught with `push_error_scope(Validation)` around the creation.
  Error scopes are thread-local, which is exactly right on the worker. Blocking on the scope's
  future is fine there.
- **Kept programs** (`KEPT_PROGRAMS`, four per Output and per probe) stay, keyed by source hash.
  `Kept::program` becomes a `wgpu::RenderPipeline` plus its block layout.
- **Simulation kernels** go through the same worker. A tick whose kernels are not ready stays
  queued.
- **No `PipelineCache` of our own.** Decided at step 6: Mesa's Vulkan drivers keep their own
  disk cache of compiled shaders, as its GL drivers did, so a second run of a project already
  links from cache; and wgpu 30's `create_pipeline_cache` is `unsafe`, taking bytes it cannot
  validate. Metal keeps its own.
- The mixer's, the viewer's and the upload converter's pipelines are made synchronously at
  startup, as `shader::link` does now.

"Shader links go to the driver's threads, not to one of ours" in decisions.md loses on its own
terms: its objections were GL's thread affinity and eframe never handing out a display. Neither
exists here. The worry left is whether wgpu-core holds a lock during a pipeline creation that
the synth's recording or submit needs. That is item 6 on the
[measurement list](#what-to-measure-before-and-after).

### 1.14 GPU timing

`render/timing.rs` and `OutputRenderer`'s timer:

| today | with wgpu |
| --- | --- |
| one `GL_TIME_ELAPSED` query around each Output's draw | the Output's render pass carries `timestamp_writes` at its beginning and end (`Features::TIMESTAMP_QUERY`), a pair in its tick's range of the renderer's one query set |
| nine `glQueryCounter(GL_TIMESTAMP)` marks a draw | the same nine marks, each an empty compute pass whose end writes a timestamp, at the head of the tick's range. Metal writes none for a pass with no work, so on a Mac they read zero and a draw has no breakdown; a mark doing work is written, but Metal orders passes only by shared resources and an Output's pass is found outside its phase's marks |
| a query object per Output and per mark | **one query set per renderer**: `timing::DEPTH` ranges of the nine marks and a pair per Output, up to `timing::TIMED`. Metal holds at most 32 counter sample buffers in a process and wgpu loses the device that asks for a 33rd, so every set is made under a claim on `timing::QUERY_SETS` (16), and a renderer past them draws untimed |
| results read when `QUERY_RESULT_AVAILABLE` says yes | `resolve_query_set` into a buffer, copied into staging, read by `map_async` under the same "a range still in flight places no marks" rule; a timestamp that reads zero was never written and is no reading |
| nanoseconds | ticks × `Queue::get_timestamp_period()` |
| `PaintTimer`: two marks from paint callbacks on eframe's context | the start mark goes in a callback's `prepare()`, into egui's encoder ahead of its pass. The end mark needs a timestamp inside egui's render pass (`TIMESTAMP_QUERY_INSIDE_PASSES`). Vulkan offers it. Apple GPUs are not expected to, since they sample counters only at stage boundaries. Where it is missing the editor's figure is absent, never a zero |
| the whole-process line from `/proc/self/fdinfo` `drm-engine-render` | unchanged on Linux: the Vulkan driver's fd is a DRM client like the GL one. Absent on a Mac |

`timing::Ring` holds the in-flight rule for both timers, the renderer's ranges and the paint
timer's pairs. The "a driver that refuses a query leaves no line, never a zero" rule is "an
adapter without `TIMESTAMP_QUERY`, a Metal process whose query sets are all held, or a
timestamp the GPU never wrote leaves no line".

rendering.md's note that `sims` reads near zero on iris because a GL timestamp does not wait
for compute ahead of it is iris-GL-specific. It is to be measured again on Vulkan, not carried
over.

### 1.15 Orientation: nothing moves in memory

This removes a whole class of worry, so it is spelled out. In GL a fragment written into texel
row 0 of a framebuffer texture has `gl_FragCoord.y = 0.5`. In wgpu the same holds: framebuffer
row 0 is the first row in memory, and `FragCoord.y` there is 0.5. Every Output computes
`screenUV` from `gl_FragCoord` (`compile/mod.rs`'s `main`), and so do the mixer
(`mixer::VARYING`), the YUV converter and a tap's measurement `cell`. So **every Output's texture
has the same bytes in the same rows under both backends.** Feedback, the ring's bit-for-bit
tests, `Picture::flip` for uploads, `flip_rows` for readbacks and every expected pixel value in
the tests carry over unchanged.

What does differ:

- **Vertex position.** In GL, `gl_Position.y = +1` lands in the *last* row of an offscreen
  target. In wgpu with naga, it lands in row 0. So every offscreen pass the renderer writes —
  the resize carry, the thumbnail letterbox, the capture halvings, the picture resample — takes
  its texture coordinate from `gl_FragCoord` (or `textureLoad`), never from an interpolated
  vertex position. Fullscreen passes then write identical bytes. The on-screen blit is the one
  vertex-derived picture. WebGPU's NDC is y-up like GL's, so its orientation on screen comes out
  the same under naga, which writes both backends' shaders. A pixel test holds it.
- **The window's own coordinates.** A surface's row 0 is at the top, not the bottom.
  `Viewport::from_bottom` becomes `top`, and the blit's `u_corner` rounding (`viewer.rs`'s
  `FRAGMENT`) measures its bottom band from the viewport's bottom edge. Written as it is, it
  would round the top corners.

### 1.16 What the three contexts leave behind

Deleted outright:

- `render/egl.rs`: `SharedContext`, `Handles`, `Saved`, `Priority`, `WindowSurface`, the EGL
  loader. The last of these goes when `dmabuf.rs` moves to wgpu-hal.
- `render::release_pending` and its three call sites: `Renderer::draw`, `Viewer::open_frame`'s
  callback, and `Pictures::paint`.
- `retire.rs`'s `Retire`, `ReaderFence` and `reader_fences()`. `Lease` becomes a claim.
- `ring::Fence`, `viewer::FrameFence`, `output::Mapped`, and the `unsafe impl Send/Sync` on each.
- `shader.rs` as it stands: `parallel_supported`, `enable_parallel`, `submit`,
  `submit_compute`, `is_ready`, `finish`, `abandon`.
- `quad.rs`. A fullscreen triangle from `vertex_index` needs no buffer.
- `Renderer::draw`'s save and restore of `SCISSOR_TEST` and `BLEND`. wgpu has no global state for
  a caller to leave behind, and `a_host_scissor_box_does_not_clip_the_offscreen_render` goes
  with it.
- Dependencies: `glow`, `khronos-egl`, `glutin` (dev), and `wayland-sys`'s `egl` feature.

Tests that lose their reason: [section 6](#6-the-test-suite). Docs: [section 8](#8-what-goes-stale-in-docs).

### What the shader path gives the renderer

All of it is `compile::wgsl`, and none of it needs reflection:

1. **One module per Output and per probe**, with `vs_main` (a triangle from `vertex_index`, no
   vertex buffer) and `fs_main` in it: `wgsl::build`, `build_hosting` and `build_probe`, the
   twins of `compile::build`, `build_hosting` and `build_probe`. `Shader::uniforms`,
   `Shader::taps` and the source hash mean what they mean for GLSL.
2. **Every binding in group 0, from the `Shader` alone**: the uniform struct at 0, the tap
   buffer at 1 where `taps` is not empty, four samplers at 2 to 5 — mirror or repeat, linear or
   nearest, in `wgsl::Sampler::ALL`'s order — where there is a texture, and one texture per
   `NodeTexture` uniform from 6 up in name order. `wgsl::bindings` lists them and
   `wgsl::uniform_layout` places the struct's members; the gate holds both against naga's own
   reading of every module. A texture reads through the sampler its output's `wrap` and
   `filter` pick, so the renderer makes the four once and binds them all.
3. **The vertex y convention** of [1.15](#115-orientation-nothing-moves-in-memory): `fs_main`
   builds `uv` from `@builtin(position)` unflipped.
4. **During the build-alongside**, the two renderers read two outputs of one walk:
   `compile::Target` picks the generator each output is asked for. The GLSL half is deleted at
   the flip ([step 6](#the-steps)).
5. **Errors**: naga's, rendered with `emit_to_string` against the module, naming the function,
   which is named `{slug}{id}_{port}`.

The compute kernels and the renderer's own stages are written in WGSL by their lanes (5d, 5e,
5a), to the same conventions, and join `tests/shader_targets.rs`'s corpus there.

---

## 2. Picture windows

Today (`render/picture/`): a thread named `pictures` runs calloop over eframe's own `wl_display`
(borrowed through `Backend::from_foreign_display`), an `xdg_toplevel` per picture, an EGL window
surface on each, a third context in the share group, and a `Viewer`. It is paced by each
surface's `wl_surface.frame` callback with exactly one outstanding, swaps at interval zero, and
masks alpha.

**What changes on Linux is only the drawing half.** Everything in `thread.rs` about windows,
input, pacing and the protocol — `Win`, `Press`, `edge_at`, `fit_aspect`, the one-outstanding
callback rule, the watchdog, `Wall`, `Ask` and `Told` — is untouched. The EGL half becomes:

- **A surface per window.** `instance.create_surface_unsafe(SurfaceTargetUnsafe::RawHandle {
  raw_display_handle: Wayland(display), raw_window_handle: Wayland(wl_surface) })`, configured on
  each `configure` with the physical size. This replaces `WindowSurface` and
  `wl_egl_window_resize`.
- **Opacity by configuration.** `CompositeAlphaMode::Opaque`, with `set_opaque_region` kept.
  The `color_mask` trick becomes unnecessary; writing alpha 1 anyway costs nothing.
- **Pacing stays the frame callback.** The present mode is `Mailbox` or `Immediate`, the
  equivalent of swap interval zero, so a present never blocks one window while another's callback
  is due. The callback request goes on the `wl_surface` just before `SurfaceTexture::present()`,
  whose commit carries it, exactly as `eglSwapBuffers`' commit does. Whether Mesa's Vulkan WSI on
  Wayland ever blocks in `get_current_texture` for a hidden surface is on the measurement list.
- **The format.** A non-sRGB surface format (`Bgra8Unorm` or `Rgba8Unorm`), so the values written
  are the values GL wrote today. egui_wgpu makes the same choice for the editor.
- **Drop order.** Surfaces drop before their `wl_surface`. `App::on_exit` still stops the pictures
  thread first.

**The borrowed display is no longer required, and is kept anyway.** It existed because an EGL
share group cannot cross two `EGLDisplay`s. A Vulkan surface needs *a* `wl_display`, and the
textures are the device's whatever connection the window is on. So the thread *could* open a
connection of its own. That would delete the one `unsafe` borrow and the shared wakeups
rendering.md prices at a few hundred a second. But a window from a second client connection is
a window from a client that does not hold focus, and KWin's focus-stealing prevention may open
it behind the editor. Fixing that means an `xdg_activation_v1` token from the editor's winit
window, handed across. Keep the borrow for the flip, where it changes nothing a person sees, and
try the own connection after.

### What the platform layer must provide, per OS

The pictures thread's drawing is one piece of code for both OSes: a `wgpu::Surface`, the
`Viewer` pass, present. What differs per OS is a window, a clock and input. The platform layer
provides:

| | Linux (Wayland) | macOS (winit, inside eframe's own loop — built, [macos-windows.md](macos-windows.md)) |
| --- | --- | --- |
| **make a window** | on the pictures thread: `xdg_toplevel`, no decorations, `set_app_id` (`supersilvia-popout` / `-fullscreen`) | **on the main thread**, in `picture::Loop` around eframe: a borderless, shadowless winit window. AppKit is main-thread-only |
| **hand the drawable to the drawing thread** | the raw `wl_display` and `wl_surface` pointers | the wgpu surface itself, made on the main thread from `Arc<Window>` — winit hands out a window handle nowhere else — and moved to the window's own thread. wgpu-hal's Metal `configure` and `present` make no main-thread demand, and it reads the layer's size atomically. Metal offers `Fifo` and `Immediate` only, no `Mailbox` |
| **a clock per window that stops when nobody can see it** | `wl_surface.frame`, one outstanding | `Fifo` with two drawables, on a thread per window, blocking in the acquire; winit's `Occluded`, sent across, parks the thread |
| **move** | `xdg_toplevel.move` behind `DRAG_SLOP`, with the press serial | our own, behind the same slop: the window's frame set from `picture::dragged` on each motion |
| **resize** | `xdg_toplevel.resize` from the 12-point bands (`BAND`, `edge_at`), aspect lock on Shift/Ctrl (`fit_aspect`) | our own, from the same bands, through `picture::dragged`, which takes the same `fit_aspect` lock; winit has no resize on macOS |
| **fullscreen** | `set_fullscreen` / `unset_fullscreen` | instant, in place: `set_simple_fullscreen` with `borderless_game`, the menu bar and the Dock hidden outright |
| **keys** | `F` and `Escape` as evdev positions on a raw `wl_keyboard` | winit's `KeyCode::KeyF` and `KeyCode::Escape`, physical positions |
| **pointer, for `crate::pointer`** | `wl_pointer` into `pointer::Feed` | winit's pointer events on the main thread, into the same `Feed` |
| **what the editor sends and hears** | `Ask` over a calloop channel; `Told` back | `Ask` read by `Loop` in the same turn of the loop `App::ui` sent it in; `Told` back on a plain channel |

**Why the main thread is not a problem on a Mac.** Every `Ask` comes from a mark in the editor,
so the editor is visible and `App::ui` is running on the main thread when the window has to be
made. Nothing afterwards needs the main thread per frame: painting is on each window's own
thread, paced by its display. So a minimized editor stops nothing, which is the whole point of
picture windows. Keys and the pointer arrive on the main thread through winit's loop, which
runs whatever eframe is painting.

The Mac side needs no `unsafe` and no `objc2` crate of its own: winit's safe API is all of it,
and the invariant row stays as it was. The AppKit-through-`objc2` route this table first
described lost; [macos-windows.md](macos-windows.md) says why.

## 3. eframe on its wgpu backend

- `Cargo.toml`: eframe's `glow` feature becomes `wgpu`, and `egui-wgpu` moves from
  `[dev-dependencies]` to `[dependencies]`. wgpu's features are trimmed to `vulkan` and `metal`,
  plus `wgsl`, which is all the shader path needs. The default GLES backend and DX12 go, both
  for size and because a GL adapter in the enumeration is how kittest met EGL's `BadAccess`
  (`tests/ui.rs`'s module doc).
- `main.rs`: `renderer: eframe::Renderer::Wgpu`, and `wgpu_options.wgpu_setup =
  WgpuSetup::Existing(WgpuSetupExisting { instance, adapter, device, queue })` from the `Gpu`
  made just before. `present_mode` stays `AutoVsync`, the analogue of swap interval one.
  `desired_maximum_frame_latency` is 1 if measurement says the default of 2 adds a frame the GL
  path did not have. The comment "Never wgpu" goes.
- `App::new` already takes its `Gpu` from `render::Gpu::for_eframe(cc)` and asks it for the
  synth's with `Gpu::for_synth`, which the picture host also reads. On wgpu `for_eframe` wraps
  `cc.wgpu_render_state` (instance, adapter, device and queue are all there) or the `Gpu`
  `main.rs` made, and `for_synth` is a clone, so `App::new` is untouched. `has_gl` can become
  `has_gpu`, which is a rename and nothing else.
- **Viewers inside egui** become `egui_wgpu::Callback::new_paint_callback(rect, ViewerCallback)`.
  `ViewerCallback` implements `CallbackTrait`:
  - `prepare()` writes the blit's uniforms (fit scale, flip, corner) into a per-frame buffer and
    makes the bind group (the picture's view and its sampler). The pipeline lives in
    `CallbackResources`, keyed by egui's `target_format`.
  - `paint()` sets the pipeline and bind group and draws. It first calls `set_viewport` with the
    **unclamped** rect. egui_wgpu sets `viewport_in_pixels()`, which clamps to the window, and
    that is the trap the invariant "a blit fits the whole rect, never the scissored part" guards
    (`viewport_of` in `render/viewer.rs`). wgpu accepts a viewport reaching past the target, up to
    twice the maximum texture size, so the override is legal. egui_wgpu's scissor still cuts the
    picture.
  
  Since step 2 the call sites (`blit_into`, `preview`, `mix_cover`) call `Viewer::node_callback`
  and `Viewer::mixer_callback`, and the timer's marks are `PaintTimer::mark_on_paint`, so the
  callbacks change inside `render/` and `app/` names no backend. The one exception is
  `App::on_exit`'s signature, which is eframe's and loses its glow argument at the flip.
- `register_native_texture` is available on egui_wgpu, and still not the right tool. A ring
  rotates the texture every tick, so the registration would be redone every frame. The paint
  callback keeps `Fit`, the corner and the flip in one place. "Blit in a paint callback, not a
  registered texture" stands, with its wording updated.
- **Minimized.** eframe's wgpu path must still run no pass while the editor is minimized
  (`request_repaint_after` a quarter second out), or `get_current_texture` can block on a surface
  the compositor will not release. rendering.md's figures (0.0% of a core, synth at 100.2 ticks
  a second, `late` 0 across a minute) are to be measured again.

## 4. Adapter selection

**The app renders on the strongest GPU the machine offers, and the tests on the integrated
GPU, never a discrete one.** The GL path did not guarantee either: the headless GL
harness skips a device that fails `eglInitialize`. And kittest's own selector prefers a software
adapter, then a discrete one, so it drew the snapshots on llvmpipe in a distrobox and would take an
NVIDIA card wherever the NVIDIA Vulkan driver loads. So the rule is written once, as a pure function,
and used by the app, every GPU test, every bench and kittest's shared device. **Built** (step 1)
as `render::adapter`, with kittest's shared device already on it:

```rust
/// The adapter supersilvia renders on, out of every adapter the instance offers.
pub fn choose(adapters: &[AdapterInfo], asked: &Asked) -> Result<usize, String>
```

`Asked` is the two variables below, read by `Asked::from_env`, which is what the app asks;
`Asked::integrated` is the same with `integrated` standing in for an unset
`SUPERSILVIA_ADAPTER`, which is what the tests and benches ask. `pick` applies the rule to live
adapters and logs the choice, and `headless` is an instance over Vulkan and Metal with the
chosen adapter.

1. **Backends.** Vulkan on Linux, Metal on macOS. Nothing else is enumerated.
2. **An override wins.** `SUPERSILVIA_ADAPTER` is an index into the list the log prints (a
   number past the end of the list is read as a piece of a name, so `4070` names an RTX 4070),
   or `vendor:device` in hex, or `integrated` for every integrated GPU, or a case-insensitive
   substring of the adapter's name (`intel`, `nvidia`). An override naming nothing is an error
   that says what was offered and what the variable takes. It never silently falls back, and it
   does not lift the software rule.
3. **Never a software adapter** (`DeviceType::Cpu`: llvmpipe, lavapipe, SwiftShader) unless
   `SUPERSILVIA_SOFTWARE_GPU=1`, the renamed `SUPERSILVIA_SOFTWARE_GL`.
4. **Then the strongest: `DiscreteGpu`, then `IntegratedGpu`, then other hardware, then a
   software adapter where one is allowed**, and of two of a kind the first the instance lists.
   NVIDIA is a discrete GPU like any other: a person with a card wants it used. A
   machine that wants its iGPU instead sets `SUPERSILVIA_ADAPTER=integrated`, and the tests ask
   with `Asked::integrated`. On Apple Silicon there is one adapter.
5. If nothing is left: refuse to start, naming every adapter offered and why each was passed
   over.

The log prints the choice with name, driver and backend, as the GL path logs its renderer
string. `scripts/doctor.sh` gets the same rule in shell over `vulkaninfo --summary`, and drops its
GL and EGL renderer checks; `check.sh` runs it with `SUPERSILVIA_ADAPTER=integrated`. The rule
is a table of unit tests over hand-built `AdapterInfo` lists: an Intel UHD 770 beside an NVIDIA
card in either order, an AMD card beside an iGPU, two discrete cards, an iGPU alone, NVIDIA
alone, software alone, `intel` and `integrated` on the first pair, `integrated` on an AMD APU
and on a machine with none, the NVIDIA card named each way it can be, an override naming
nothing. On a real machine, `the_test_adapter_is_the_igpu` in `tests/ui.rs` holds kittest's device to the iGPU, and
replaces `the_test_gl_context_is_the_real_gpu` at the flip.

## 5. Performance

### Where it stands

GPU-bound on the iGPU. Before render-on-demand the twelve-tab demo's 42 Outputs cost about
213 ms of GPU a tick at 7% of the synth thread's CPU. Render-on-demand, the ring, one tick in
flight and the mapped readbacks brought the live editor (3440×1440 at 100 Hz, maximized) to
96 of 100 Hz with nothing dropped. The editor's own paint is 1.7 ms of GPU at that size. The
per-Output spans in the Status box are the number that describes headroom.

### The one real risk: one queue

wgpu gives a device one queue. The editor's paint, every picture window's blit and the synth's
tick all go into it, in submission order. Today they are three GL contexts, and the synth's
asks for **low priority** so the GPU schedules a viewer's blit ahead of the synth's queued
frame. That was measured: editor frames over the 10 ms interval went from 0, 0 and 0 at low to 11,
286 and 62 at default priority, on the Color tab beside a 3440×1440 stand-in editor
(decisions.md, the low-priority entry). On one queue there is no priority at all. An editor
frame submitted while a heavy tick is queued waits for the tick. One tick in flight bounds that
wait to the rest of one tick's GPU work, which near saturation is most of an interval.

**Measured (step 0b): one queue starves the editor once the synth's tick costs most of an
interval, and Plan B does not.** `examples/queue_contention.rs`, release build, on the UHD 770
through `render::adapter::choose`. The synth is a thread ticking at 100 Hz from its own deadline
under the one-tick bound, eight passes a tick, one submission each, calibrated to 50%, 90% and
150% of the 10 ms interval (4.6–4.8, 9.0–9.2 and 14.9–15.2 ms of GPU a tick). The editor is one
3440×1440 pass calibrated to decisions.md's two GL paint costs, 2.9 and 4.8 ms. The figure is the
editor's **submit to done**; an overrun is a frame not done inside 10 ms. Headless, the editor
submits at 100 Hz deadlines; in the window it submits from an eframe `ui` on wgpu at the
100 Hz display's own vsync (9.93 ms) sharing the device, and a frame is **late** when eframe's frame
interval passes 1.5 of the display's. Three six-second runs a cell; a run during which another
process ran a GPU binary was thrown away and run again. Plan B is the synth on a second device
from the same adapter, at the driver's default queue priority and at
`VK_QUEUE_GLOBAL_PRIORITY_LOW` through `open_with_callback`, which anv grants without privilege.

| editor | synth tick | where the synth submits | submit to done, median / p99 / max ms | overruns per run, headless | late frames, window | synth ticks/s |
| --- | --- | --- | --- | --- | --- | --- |
| 2.9 ms | 50% | one queue | 3.5 / 6.3 / 8.0 | 0, 0, 0 of 601 | 0 of 1800 | 100 |
| | | second device, low | 3.4 / 4.0 / 6.7 | 0, 0, 0 | 0 of 1800 | 100 |
| | 90% | one queue | **8.5 / 25.1 / 68.3** | **145, 143, 135** of ~410 | **523 of 1278** | 66–90 |
| | | second device, default | 3.8 / 4.6 / 4.7 | 0, 0, 0 of 601 | 0 of 1800 | 80 |
| | | second device, low | 3.5 / 4.1 / 4.4 | 0, 0, 0 | 0 of 1800 | 80 |
| | 150% | one queue | **12.5 / 17.6 / 18.5** | **254, 257, 258** of 342 | **797 of 1004** | 57 |
| | | second device, default | 4.3 / 5.7 / 6.0 | 0, 0, 0 of 601 | 0 of 1800 | 47 |
| | | second device, low | 3.9 / 5.0 / 5.3 | 0, 0, 0 | 0 of 1800 | 47 |
| 4.8 ms | 50% | one queue | 5.9 / 8.7 / 9.6 | 0, 0, 0 of 601 | 328 of 1472 | 100 |
| | | second device, low | 5.0 / 5.6 / 5.8 | 0, 0, 0 | 0 of 1800 | 100 |
| | 90% | one queue | **8.8 / 13.0 / 13.9** | **160, 157, 159** of ~440 | **680 of 1121** | 76 |
| | | second device, default | 5.4 / 6.4 / 6.9 | 0, 0, 0 of 601 | 9 of 1791 | 58 |
| | | second device, low | 5.4 / 6.3 / 6.5 | 0, 0, 0 | 0 of 1800 | 59 |
| | 150% | one queue | **14.2 / 19.6 / 20.2** | **291, 286, 289** of 309 | **888 of 899** | 51 |
| | | second device, default | 6.5 / 7.9 / 8.4 | 0, 0, 0 of 601 | 143 of 1657 | 34 |
| | | second device, low | 5.7 / 6.8 / 7.2 | 0, 0, 0 | 0 of 1800 | 35 |

What it says:

- **On one queue the editor waits for the rest of the synth's tick**, as the mechanism predicts.
  At half an interval of synth there is room and nothing overruns. At nine tenths a third of the
  editor's frames overrun and the window loses 41% (2.9 ms) to 61% (4.8 ms) of its frames, which
  is the editor at roughly half the display's rate. At saturation the editor lives at 50 Hz:
  74–94% of its frames overrun. The worst single frame waited 68 ms. That is worse than anything
  GL showed even at *default* priority (0–26 and 11–286 overruns in six seconds), because GL's
  three contexts were three kernel contexts the scheduler timesliced, which one queue is not.
- **A second device fixes it, and low priority makes it clean.** At every load, a second device at
  low priority kept the editor within about a millisecond of its own cost, with no overrun and no
  late frame in any run. At the driver's default priority the headless figures are nearly as
  good, but the window still lost 143 frames at saturation with the 4.8 ms editor, so low is the
  setting.
- **The synth pays for the editor's smoothness, as it did under GL's low priority.** With the
  editor now served first at 100 Hz, a saturated synth ran 47 rather than 57 ticks a second
  (2.9 ms editor) and 35 rather than 51 (4.8 ms). On one queue those extra synth ticks were bought
  with the editor's frames.
- **The editor alone is not a lower bound.** With the synth off, the 2.9 ms pass took 6.5 ms and
  the 4.8 ms pass 10.9–11.2 ms, so every one of the latter overran and the window ran at 50 Hz:
  i915 drops the iGPU to 700 MHz under a load that light (1600 MHz under the synth), and one long
  pass a frame at 700 MHz is slow. A real editor frame is many small draws at 1.7 ms measured at
  3440×1440; this is a property of the synthetic load and of the governor, not of the queue,
  and is a line on the post-flip measurement list rather than a finding here.

**Throttled, one queue is nearly as good (step 3).** The starving above is the synth's tick
queued whole ahead of the editor. So the skeleton's synth submits one Output per submission and
keeps little queued: `render_wgpu::queue::Throttle` submits the next only once at most
`QUEUED_AHEAD` — one — of its earlier submissions is still on the GPU, so an editor frame lands
behind the pass running and at most one more. The same example's fourth mode runs the synth
through the renderer's own `Gpu` and `Throttle` on the editor's queue. Headless, release, the
UHD 770 (Mesa 26.1.8), one session with plain one queue and Plan B at low priority run beside it
for comparison; three six-second runs a cell, none thrown away (no other GPU binary ran). The
calibration landed at 5.0, 8.9 and 14.6 ms a tick and editor passes of 3.0 and 4.8 ms.

| editor | synth tick | where the synth submits | submit to done, median / p99 / max ms | overruns per run, headless | synth ticks/s |
| --- | --- | --- | --- | --- | --- |
| 2.9 ms | 50% | one queue | 4.9 / 6.0 / 7.4 | 0, 0, 0 of 601 | 100 |
| | | **one queue, throttled** | **3.5 / 4.4 / 5.3** | **0, 0, 0** | 100 |
| | | second device, low | 3.1 / 3.9 / 4.3 | 0, 0, 0 | 100 |
| | 90% | one queue | 8.9 / 11.2 / 12.6 | 165, 157, 147 of ~435 | 93 |
| | | **one queue, throttled** | **4.3 / 5.2 / 6.3** | **0, 0, 0 of 601** | 83 |
| | | second device, low | 3.5 / 4.3 / 4.6 | 0, 0, 0 | 82 |
| | 150% | one queue | 12.1 / 17.1 / 18.1 | 246, 247, 251 of 350 | 58 |
| | | **one queue, throttled** | **5.2 / 6.5 / 7.2** | **0, 0, 0 of 601** | 50 |
| | | second device, low | 3.7 / 4.9 / 5.0 | 0, 0, 0 | 50 |
| 4.8 ms | 50% | one queue | 5.3 / 7.8 / 8.3 | 0, 0, 0 of 601 | 100 |
| | | **one queue, throttled** | **5.1 / 6.2 / 6.9** | **0, 0, 0** | 100 |
| | | second device, low | 4.7 / 5.5 / 5.7 | 0, 0, 0 | 100 |
| | 90% | one queue | 8.6 / 13.1 / 14.2 | 167, 169, 166 of ~432 | 78 |
| | | **one queue, throttled** | **6.4 / 7.1 / 7.8** | **0, 0, 0 of 601** | 62 |
| | | second device, low | 5.2 / 5.9 / 6.5 | 0, 0, 0 | 61 |
| | 150% | one queue | 13.9 / 19.1 / 19.6 | 283, 285, 282 of 315 | 52 |
| | | **one queue, throttled** | **7.2 / 8.5 / 9.3** | **0, 0, 0 of 601** | 37 |
| | | second device, low | 5.7 / 7.1 / 7.5 | 0, 0, 0 | 36 |

The throttle takes every overrun away at every load: the editor's frame costs its own pass plus
about one synth pass (0.5 to 1.5 ms more than Plan B's low-priority device, which lets the
editor cut in rather than wait), and the synth pays for it exactly as it does under Plan B — 83
against 82 ticks a second, 50 against 50, 62 against 61, 37 against 36. The margin that is thin
is the heaviest cell: the 4.8 ms editor beside a saturated synth reaches 9.3 ms of its 10.
Only headless was run; the window's late frames, where the earlier run found Plan B at the
driver's default priority still dropping frames that headless did not, are measured again after
the flip (item 2 of [the list](#what-to-measure-before-and-after)).

**Verdict: the skeleton is one device and one queue, throttled.** It keeps the editor inside
its interval at every load measured, at the synth rate Plan B gives, for none of Plan B's
cross-device sharing or `unsafe`; Plan B stays written below as step 8, to be built only if the
window measurement after the flip says the extra millisecond or so costs frames.

### Plan B: the synth on a device of its own

The synth on **a device of its own**, from the same adapter, at low queue priority. Editor and
picture windows share the other device.

- **Priority on Linux.** wgpu-hal's `vulkan::Adapter::open_with_callback` lets the device's queue
  create info carry `VkDeviceQueueGlobalPriorityCreateInfoKHR` at `LOW`, which needs no
  privilege. The device is then made with `Adapter::create_device_from_hal`. On Metal there is no
  priority, but two devices are two `MTLCommandQueue`s, and Apple's GPU scheduler interleaves
  queues rather than running one behind the other.
- **Sharing across devices.** Two Vulkan devices do not share images. The synth's ring targets
  would be made exportable with `VK_KHR_external_memory_fd`, which wgpu enables as
  `Features::VULKAN_EXTERNAL_MEMORY_FD`. wgpu has no export call, so the export is `ash`
  through `Device::as_hal`. The targets are then imported into the viewers' device through
  `texture_from_raw`. On Metal, two wgpu devices on one adapter share one `MTLDevice`, so an
  `MTLTexture` from one can be wrapped by the other through `texture_from_raw` with no export.
- **Synchronization comes back, CPU-side.** The synth already publishes only finished frames,
  which covers "the viewer reads a finished frame". The other direction — "the synth does not
  rewrite what a viewer is still reading" — is today's reader fences. Here it becomes: a viewer
  lets go of its `Published` only once *its* submission has completed (`on_submitted_work_done`),
  not merely once it is submitted. In-place textures (uploads, a simulation's picture) would get
  a ring of two, so a rewrite never lands in the texture a viewer holds.

Plan B is several commits and most of the `unsafe` the conversion otherwise deletes. Step 0b's
measurement called for it; the throttled one queue measured after it answers the same starving
for none of that, so it is held back as step 8.

### What to measure, before and after

Before: on `main` just ahead of the flip, on glow. After: on the flip. Every number on the iGPU,
release builds, `cargo build --release` first.

| # | what | tool | pass if |
| --- | --- | --- | --- |
| 1 | the demo's heavy and light tabs: rate, draws a second, GPU by phase, the shown frame's median, p90 and max age, drops | `tick_bench <demo> <tab>` per tab | rate within 5% of glow on every tab; no drops |
| 2 | the editor's paint GPU ms and its frames over the interval, beside a saturated synth | `editor_bench` at 3440×1440, the priority experiment's setup | overruns no worse than glow at *default* priority; if worse than glow at *low*, that is Plan B's question |
| 3 | the live app at 3440×1440 and 100 Hz, maximized, demo open | Status box | ≥ 96 of 100 Hz, 0 dropped |
| 4 | editor minimized a full minute | `ps`, Status box after | synth 100 Hz, `late` 0, frame thread ~0% |
| 5 | a picture window minimized; both minimized | per-thread CPU and wakeups | 0 wakeups with both hidden, as rendering.md measured |
| 6 | a recompile storm: a cable dragged across nodes on a heavy visible tab | `late` count, tick p99 | the link worker never lengthens a tick |
| 7 | the synth thread's CPU per tick, and the draw's own CPU lap (`Work::Draw`) | Status box | within 1 ms of glow's; wgpu-core validation and tracking cost CPU per draw |
| 8 | GPU memory over four minutes of churn | `/proc/<pid>/fdinfo` `drm-total-system0` and GEM mappings | flat after warm-up. This is the soak that replaces `tests/release_pending.rs` |
| 9 | edit to new picture: time from a structural edit to the new pipeline on screen, first run and second run | log timestamps | no worse than GL's link on the first run; the second run as fast as GL's, off the driver's own cache |
| 10 | the shipping binary's size | `cargo build --profile dist` | recorded, not gated. wgpu and naga are several megabytes, and binary size was one of glow's reasons |

### The headless figures, before and after

Items 1, 2, 7 and 10 of the table can be taken without a window, and were: on glow at `macos`
3d8ecbe just ahead of step 6(i), and on wgpu once (i) was built, the same benches ported onto
`render_wgpu::Gpu`. Release builds, the UHD 770 (Mesa 26.1.8; iris under glow, anv under
wgpu), the demo project `21 September` with every tab open and the named one looked at, 8 s at
100 Hz after every program linked and five seconds of warm-up. Each run was made only while no
other process ran a GPU binary, and a run another one overlapped was thrown away and run again.
Items 3 to 6, 8 and 9 need the live app, and are measured by hand at step 7.

**Item 1 and item 7, `tick_bench <demo> <tab> 100 8`.** No drop of any cause on any tab under
either. GPU is the Outputs' span and the whole draw, per tick, from the phase marks; CPU is the
synth thread's wall time per tick and, of it, the time spent waiting (wall less the thread's
own CPU clock).

| tab | ticks/s glow → wgpu | GPU outputs / whole ms, glow → wgpu | CPU / waiting ms, glow → wgpu |
| --- | ---: | ---: | ---: |
| Start here | 100.0 → 100.0 | 6.26 / 6.85 → 6.30 / 6.74 | 1.64 / 0.03 → 7.81 / 4.57 |
| Transforms | 100.0 → 99.2 | 6.74 / 7.41 → 7.32 / 7.74 | 1.90 / 0.11 → 9.21 / 5.36 |
| Color | 100.0 → 100.0 | 7.03 / 7.70 → 7.10 / 7.54 | 1.73 / 0.06 → 8.14 / 4.00 |
| Effect kernels | 54.3 → **46.1** | 16.75 / 17.85 → 19.91 / 20.60 | 18.41 / 16.25 → 21.69 / 16.43 |
| Effect manglers | 84.3 → **69.1** | 10.63 / 11.35 → 12.73 / 13.15 | 11.85 / 10.00 → 14.46 / 10.04 |
| Convert, Mix and Math | 100.0 → 99.3 | 6.39 / 7.05 → 7.13 / 7.63 | 1.66 / 0.08 → 8.89 / 5.22 |
| Control | 100.0 → 99.9 | 6.97 / 7.79 → 6.41 / 6.85 | 1.80 / 0.10 → 8.16 / 4.88 |
| Time | 100.0 → 99.8 | 6.63 / 7.26 → 6.89 / 7.30 | 1.76 / 0.03 → 8.68 / 5.04 |
| Games and simulations | 55.9 → **40.1** | 16.69 / 17.38 → 23.05 / 23.47 | 17.88 / 16.06 → 24.91 / 20.64 |
| Feedback and Output | 100.0 → 100.0 | 6.08 / 6.51 → 6.76 / 7.22 | 1.20 / 0.02 → 8.25 / 5.16 |
| Sources and Input | 100.0 → 99.9 | 6.68 / 7.43 → 6.57 / 7.03 | 1.66 / 0.05 → 8.36 / 5.25 |
| Chrome | 100.0 → 100.0 | 6.21 / 6.77 → 7.05 / 7.46 | 1.72 / 0.06 → 7.77 / 4.46 |

What it says, for step 7 to weigh:

- **The light tabs hold 100 Hz on wgpu; the three heavy tabs fail item 1's 5%** — Effect kernels
  by 15%, Effect manglers by 18%, Games and simulations by 28%.
- **Most of the heavy tabs' loss is a few nodes' shaders costing more under anv than under
  iris**, per Output's own GPU ms, taken again on an idle machine (glow → wgpu, three
  alternating rounds of 8 s): `lyapunov` 11.85 → 17.33, `posterize` (over `pixelsort`) 2.89 →
  4.12, `kuwahara` 5.37 → 6.10, `bloom` 2.17 → 2.75, and every other Output within 0.25 ms,
  `geissflow` 0.64 → 0.36 and `camcordercrt` 3.15 → 2.83 the other way. The pictures are the
  same (`tests/node_pictures.rs`); the machine code is the drivers'. Most of it is naga's loop
  counter, [docs/loop-bounding.md](../docs/loop-bounding.md).
- **The throttle left the GPU idle between Outputs.** On the heavy tabs the Outputs' span
  was 2 ms longer than the sum of their own times (Effect kernels 19.91 against 17.80), where
  under glow the span was the shorter (16.75 against 17.20), and the process's share of the
  render engine fell from 93–95% to 89%: with one submission per Output, at most one queued,
  and one tick in flight, the GPU finished one Output while the synth was still recording the
  next, and ran dry at every tick's start. [The synth's
  submissions](#the-synths-submissions-after-the-flip) takes most of that back.
- **Item 7 failed as first read: the synth thread's wall per tick about quadrupled** on the
  light tabs, but most of it was the throttle waiting for each submission's predecessor, not
  work. Its own CPU (wall less waiting) was 3.2–3.9 ms under wgpu against 1.6–1.9 under glow;
  what the extra is, and what grouping submissions did to it, is in [the synth's
  submissions](#the-synths-submissions-after-the-flip).
- The shown frame's age is 0 ticks (median and p90) on nearly every Output under wgpu, against
  1 under glow: the throttle's waits mean a tick's frames have mostly finished by the time it
  publishes.

**Item 2, the editor beside the synth.** `editor_bench <demo> <tab> 3440x1440`: the editor's
own paint, whole frame, by the render engine's counter — egui_glow's painter into a framebuffer
under glow, egui_wgpu's renderer into a texture under wgpu.

| tab | paint ms glow → wgpu (counter, 20 paints a round) | once a frame, counter ms | the Status box's reading (now / avg / worst), glow → wgpu |
| --- | ---: | ---: | --- |
| Start here | 0.85 → 0.83 | 0.97 → 1.17 | 0.94 / 0.91 / 1.17 → 0.94 / 0.96 / 1.17 |
| Effect kernels | 0.50 → 0.51 | 0.62 → 0.93 | 0.59 / 0.54 / 0.72 → 0.62 / 0.62 / 0.75 |
| Games and simulations | 1.46 → 1.45 | 1.55 → 1.84 | 1.87 / 1.42 / 2.10 → 1.75 / 1.60 / 1.89 |

The painting itself costs the same; a frame painted once costs 0.2–0.3 ms more by the counter
under wgpu, which is the submission and its surrounding work rather than any shape (the
attribution by what is left out matches glow's to a few hundredths of a millisecond).

And `SUPERSILVIA_BENCH_EDITOR=1 tick_bench <demo> <tab> 100 8`: a stand-in editor of one
blended 3440×1440 quad sampling a 1080p half-float picture, at 100 Hz and waited for, beside
the synth. Under glow it is a context of its own at the default priority, the synth's at low;
under wgpu it is a thread submitting on the same device and its one queue, as eframe does.

| tab | synth ticks/s glow → wgpu | editor frames/s glow → wgpu | editor ms avg / worst, glow → wgpu | editor frames over 10 ms, glow → wgpu |
| --- | ---: | ---: | ---: | ---: |
| Start here | 100.0 → 100.0 | 100.0 → 100.0 | 1.94 / 4.48 → 2.91 / 5.40 | 0 → 0 |
| Effect kernels | 49.3 → 40.8 | 100.1 → 100.0 | 3.68 / 7.79 → 3.41 / 9.96 | 0 → 0 |
| Games and simulations | 50.8 → 38.7 | 96.4 → **77.6** | 5.68 / 16.81 → 8.78 / 19.92 | 81 → **311** |

Item 2's bar is "no worse than glow at default priority". On Games and simulations the editor
is worse than glow's at low priority and its frames over the interval are four times glow's:
the one Output there, `lyapunov`, is an 18 ms pass, and an editor frame queued behind it waits
for the whole of it whatever the throttle does, since the throttle bounds how many submissions
are ahead, not how long one is. That is Plan B's question (step 8), or one for
splitting a pass that long.

### The synth's submissions, after the flip

The first figures above left three things to chase: the GPU idle between Outputs, the synth
thread's time per tick, and a few nodes' shaders dearer on anv than on iris. Each was measured
headless with `tick_bench` as above, with a section timer inside `Renderer::draw` for the CPU
(taken out again; `perf` is not in the box). What the renderer does now: **an Output whose last
pass cost more than `SUBMISSION_MS` (2 ms), or that has not been timed, is a submission of its
own, and a run of cheaper ones shares one**, the prelude going with the first run and the coda
with the last; **`TICKS_IN_FLIGHT` is two**; `QUEUED_AHEAD` stays one.

**Where the synth thread's time went.** On Start here (four Outputs drawn a tick) the draw was
7.3 ms of wall a tick, of which 5.2 ms was the throttle waiting and 1.7 ms the tick-before wait
or recording. The work itself: finishing each command encoder about 115 µs (wgpu-core encodes
into the driver at `finish`, not as a pass is recorded), each `queue.submit` about 90 µs, each
bind group 8–10 µs, the uniform pack 0.06 ms, `publish` 0.03 ms. So bind groups are not worth
caching, and the cost is per submission: seven a tick on Start here, sixteen on Effect kernels.
Grouped, Start here makes three, and the draw's wall fell to 2.4 ms, the throttle's wait to
0.8. The thread's own CPU per tick (wall less waiting) went from 3.3 to 3.0 ms there — still
about 1.3 ms over glow's, which is wgpu-core's per-encoder and per-submission work and does not
go further without fewer submissions than the editor's latency allows.

**Throttle depth and grouping**, heavy tabs, ticks a second, release, the demo, runs made while
nothing else ran on the GPU (the session's other GPU binaries, compiles and the live app
checked before and during each run; a cell is one run or the range of several — the built
setting run twice agreed to 0.2%, the as-flipped one ranged 42–47 on Effect kernels over five):

| setting | Effect kernels | Effect manglers | Games and simulations | render engine busy |
| --- | ---: | ---: | ---: | ---: |
| glow, for reference | 54.3 | 84.3 | 55.9 | 93–95% |
| one submission per Output, `QUEUED_AHEAD` 1, one tick (as flipped) | 42.3–46.9 | 69.0–72.3 | 39.7–42.3 | 82–92% |
| the same, `QUEUED_AHEAD` 2 | 45.9 | 71.2 | 40.9 | 88–93% |
| the same, `QUEUED_AHEAD` 3 | 47.4 | 70.8 | 40.3 | 90–92% |
| a budget of 3 ms of estimated GPU queued, not a count | 47.3 | 66.4 | 40.0 | 82–88% |
| two ticks in flight alone | 47.1 | 71.9 | 42.1 | 87–92% |
| grouped to 2 ms alone | 50.0 | 73.3 | 42.6 | 90–93% |
| grouped to 1 ms, two ticks | 50.0 | 78.3 | 45.4 | 92–99% |
| **grouped to 2 ms, two ticks (built)** | **53.6** | **80.5** | **45.4** | **99%** |
| grouped to 2 ms, two ticks, and the 2 ms budget | 53.5 | 80.4 | 45.5 | 98–99% |

A deeper `QUEUED_AHEAD` buys almost nothing and a cost budget is worse: what idled the GPU was
the tick boundary, where one tick in flight made each tick wait for the last to finish and then
record its prelude onto an empty queue, and the gap after every cheap Output. Two ticks close
the first and grouping the second. Nothing dropped in any run, and the shown frame's age stayed
0 or 1 tick (median, p90 and maximum) on every Output, as it was with one tick. Light tabs held
100 Hz under every setting; their wall per tick fell from 6.5–9.5 ms to 3.0–4.9 ms, the waiting
in it from 4–5 ms to under 1 ms — Color the exception, 7.2 ms with 3 of it waiting. Against
glow's figures in the first row that is Effect kernels within 1% and Effect manglers within 5%;
glow and wgpu run alternately on an idle machine put wgpu further behind, below.

**On the Mac a cheap Output drawn after a heavy one is timed at about the heavy one's cost.**
Its span is its render pass's two timestamps, and on Apple's tile GPU the cheap pass after a
12–16 ms one reads 12–16 ms on an idle M2 where its siblings read 0.02–0.4 ms — the Metal
branch of `gpu_ring`'s `cheap_outputs_share_a_submission_and_a_costly_one_has_its_own` holds
it. So there that Output goes in a submission of its own: three cheap Outputs, a heavy one and
three cheap ones are four submissions rather than three, and the Status box's figure for the
one after the heavy one reads high, near the heavy one's. This is accepted as it stands:
it costs one submission, about 0.2 ms of the synth thread by the figures above. It is revisited
only if a heavy tab feels slow on the Mac, measured there on an idle machine.

**The editor beside it**, `SUPERSILVIA_BENCH_EDITOR=1`, the same conditions:

| tab | synth ticks/s before → after | editor frames/s | editor ms avg / worst | editor frames over 10 ms |
| --- | ---: | ---: | ---: | ---: |
| Start here | 100.0 → 100.0 | 100.0 → 100.0 | 3.24 / 6.21 → 1.68 / 2.77 | 0 → 0 |
| Effect kernels | 43.0 → 48.1 | 100.0 → 100.0 | 3.07 / 8.52 → 4.39 / 9.23 | 0 → 0 |
| Games and simulations | 38.7 → 41.6 | 77.5 → 83.2 | 8.75 / 31.69 → 8.36 / 15.18 | 310 → 333 |

No frame over the interval where there was none, a longer average on Effect kernels (a 2 ms
submission ahead of an editor frame rather than a sub-millisecond one), and on Games and
simulations the same `lyapunov` story as before: an editor frame behind its 17 ms pass waits
for all of it, whatever the grouping — grouping to 1 ms gave the same 333.

**Glow against wgpu on an idle machine**, the merged tree with the grouping built, release,
`tick_bench <demo> <tab> 100 8`, three alternating rounds of 8 s each; ticks a second, glow /
wgpu: Start here 100 / 100, Effect kernels 56.5 / 52.8 (7% short), Effect manglers 87.4 / 79.9
(9% short), Games and simulations 58.4 / 45.1 (23% short). The render engine was 98–99% busy
under wgpu against 93–96% under glow, so what is left is the shaders' own cost, not
scheduling: per Output, `lyapunov` 11.85 → 17.33 ms, `posterize` over `pixelsort` 2.89 → 4.12,
`kuwahara` 5.37 → 6.10, `bloom` 2.17 → 2.75, everything else within 0.25 ms (`geissflow`
0.64 → 0.36 and `camcordercrt` 3.15 → 2.83 faster). An earlier run on the merged tree, of wgpu
before and after the grouping, overlapped a browser and a video player on the iGPU and read
20–35% low; these replace its rates. The stand-in editor was measured only in that loaded
run: beside it, Effect kernels' frames over 10 ms went 73 → 119 with the grouping and Games and
simulations' 228 → 242, Start here's 7 → 0.

**Per-node cost on anv: naga's loop bounding.** wgpu hands anv SPIR-V that naga writes with
every runtime check on; `create_shader_module_trusted` turns them off one by one, and is
`unsafe`. Per Output's GPU ms, each check off alone and all off:

| Output (upstream) | checked | loop bounding off | int division checks off | bounds checks off | all off |
| --- | ---: | ---: | ---: | ---: | ---: |
| `lyapunov` | 17.33 | 13.08 | 17.33 | 17.16 | 12.46 |
| `kuwahara` | 6.07 | 5.27 | 6.09 | 6.08 | 5.26 |
| `bloom` | 2.75 | 2.15 | 2.75 | 2.75 | 2.15 |
| `posterize` over `pixelsort` | 4.13 | **10.10** | 4.13 | 4.11 | **10.28** |
| tab: Games and simulations, ticks/s | 42.3 | 51.9 | 42.2 | 42.4 | 53.0 |
| tab: Effect kernels | 46.9 | 50.3 | 47.1 | 46.7 | 51.3 |
| tab: Effect manglers | 72.3 | 50.1 | 71.8 | 71.6 | 49.9 |

Bounds checks and integer-division guards cost nothing measurable. **Loop bounding** — a
counter naga adds to every loop so a driver may not assume it ends — is `lyapunov`'s 4–5 ms
and `bloom`'s and `kuwahara`'s fifth, and without it anv compiles `pixelsort`'s nested
insertion sort two and a half times slower. (`lyapunov` ran 80 iterations in every figure
here; it runs a fixed ten.) So it is not a switch to throw for every module;
it is a decision below, and a node lane's to rewrite the loops that pay for it (a fixed
trip count the driver can unroll, or fewer iterations). Turning it off makes a shader that
never ends undefined rather than merely hung; every loop in the library ends within a constant
number of iterations, three of them by a clamp or a direction of travel rather than a literal
bound. What the counter is, why it costs, every loop in the library and the options are
[docs/loop-bounding.md](../docs/loop-bounding.md).

**Item 10, the shipping binary.** `cargo build --profile dist`: 23,751,888 bytes under glow,
28,130,464 under wgpu with glow's renderer still compiled in, unused, and 27,932,960 once
step 6(ii) deleted it and every GLSL generator (+4.2 MB over glow, 18%): glow and GLSL were
about 0.2 MB of it, and wgpu and naga are the rest.

### Costs to watch

- **Bind group churn.** One or more bind groups per Output per tick, because rings rotate what
  every consumer samples. Simulations use two premade groups. Measured at 8–10 µs each, not
  worth a cache.
- **Uniform upload.** One `write_buffer` per tick into a ring of uniform memory, dynamic offsets
  per draw. Never a buffer per draw.
- **Submits.** Each submission costs the synth about 0.2 ms: 115 µs to finish its encoder and
  90 µs to submit. Cheap Outputs are grouped to about `SUBMISSION_MS` of GPU a submission,
  which keeps an editor frame behind about one pass; fewer, larger submissions would save CPU
  and cost the editor ([the synth's submissions](#the-synths-submissions-after-the-flip)).
- **Polling and `map_async`.** One `device.poll(Poll)` a tick, plus the blocking one at the top of
  a draw. Callbacks only store to atomics. Maps are small (tap words), or rare (thumbnails,
  Snap), or a render's capture.
- **Pipeline creation.** On the worker thread. The worry is lock contention with the synth
  (item 6), not the compile itself.
- **Load and store ops.** An Output covers its whole target. Its pass can clear rather than
  load, which costs a full-frame read on Apple's tile-based GPUs if left as a load. It costs
  little on the Intel iGPU either way.
- **Validation.** Debug builds validate every call. Only release builds are measured.

## 6. The test suite

The GPU tests get simpler. A wgpu device needs no display server, no EGL device enumeration, no
`PBUFFER` config template, no current context and almost no `unsafe`.

**The harness,** `tests/common/gpu.rs`: one `Instance` and one `Adapter` per test process,
through `choose` asking for the integrated GPU, in a `OnceLock`. That is kittest's hard lesson: two threads in
`vkCreateInstance` at once segfaulted the Vulkan loader, per `tests/ui.rs`'s module doc. A
`Device` per test from that adapter, except where a test measures device-wide counts. Every test
device gets `on_uncaptured_error` set to panic. That is stronger than today's
`watch_gl_errors` and `gl_errors` helpers: any validation error anywhere fails the test that
caused it.

**`tests/headless_gl.rs` (114 tests) becomes several files**, one per area, so parallel lanes do
not collide in one 10,000-line file: `gpu_ring.rs`, `gpu_readback.rs`, `gpu_upload.rs`,
`gpu_mixer.rs`, `gpu_sims.rs`, `gpu_nodes.rs`, `gpu_render.rs`. By kind:

- **Property tests through the `Renderer` API** — zero flash, the ring and leases, feedback bit
  for bit, the saturated-GPU tests, readbacks never waiting, the capture and render, the mixer,
  sims, on-demand, taps, dual nodes, masks — **port with the same assertions and the same
  expected pixels**, because the bytes in memory are the same ([1.15](#115-orientation-nothing-moves-in-memory)).
  What changes is the helpers: `rgba_of`, `pixels_of`, `pixel_of` and `half_floats` become one
  `read_texture`. Every `gl` argument to the `Renderer`, a `Viewer` and the paint timer went
  at step 2; the GL building blocks' (`OutputRenderer`, `MixerRenderer`, `shader`, `quad`) go
  with glow.
- **The feedback reference**, a temp target blitted into a published one (`reference_target`),
  becomes a temp target copied with `copy_texture_to_texture`. The claim "equal to the last bit
  of every half float" is unchanged.
- **Tests of GL itself, deleted**: `clearing_an_fbo_to_red_reads_back_red`,
  `the_desktop_gl_can_do_half_float_render_targets` (`Rgba16Float` rendering is core),
  `a_host_scissor_box_does_not_clip_the_offscreen_render` (no global state),
  `a_stale_gl_error_is_not_read_as_a_refused_import` (no error flag),
  `a_texture_drawn_on_a_shared_context_is_sampled_on_the_first` and
  `a_viewer_on_a_third_context_from_handles_samples_a_published_texture` (replaced by one test: a
  viewer on another thread samples a published texture), and the GL-binding half of
  `a_tapped_output_relinking_keeps_its_own_buffer_bound_and_delivers_nothing_old`. That half is
  now structural ([1.7](#17-tap-buffers-and-atomics-in-a-fragment-shader)); the "delivers
  nothing old" half stays.
- **`gl_objects_do_not_grow_with_churn`** counts GL names. It becomes the same churn counted by
  wgpu's internal counters (the `counters` feature, `Device::get_internal_counters`) on a device
  of its own.
- **`every_node_compiles_on_the_gpu`**, the checkerboard and feedback shader tests create a
  pipeline from each node's WGSL module. They depend on the node lanes of step 4.
- **`the_test_gl_context_is_the_real_gpu`** becomes `the_test_adapter_is_the_igpu`.

**`tests/release_pending.rs` is deleted.** It proved a workaround for Mesa's threaded GL context
parking buffer storage across a share group. There is no GL, no threaded context and no share
group. Measurement 8 covers the only question left, whether memory stays flat.

**`tests/reader_fences.rs` shrinks to one two-thread test, folded into `gpu_ring.rs`.** Its four
tests hold the reader-fence order for a reused target, an upload in place and an import's
return, plus the process-wide fence count. The count and its reason for a binary of its own go
with the fences. What survives is a viewer thread holding a `Published` while the synth thread
keeps drawing and uploading: the viewer's readback is the frame it leased, and no slot it holds
is drawn into. Under Plan B this test grows back its completion-order half.

**`tests/crosstalk.rs` stays**, ported. "A picture only ever holds its own node's pixels" is
backend-independent, and the shapes it churns through are the live app's: the synth on its own
thread, a viewer on another, Outputs flipping, a project opened over another, a tab closed and
reopened.

**`tests/ui.rs`** keeps one shared device, now through `choose` asking for the integrated GPU,
so kittest cannot pick a discrete GPU either. After the flip, kittest's wgpu renderer *could* run the real viewer callbacks and put
real Output pictures in UI snapshots. It should not: snapshots would then depend on the GPU and
the clock. The harness stays `has_gpu: false`, drawing placeholders. `examples/node_shots` is the
place to try real pictures, since it is headless and deliberate.

**Examples.** `tick_bench` and `editor_bench` move onto `Gpu`. `editor_bench` paints with
`egui_wgpu::Renderer` into an offscreen texture instead of `egui_glow::Painter` into a
framebuffer.

**Invariants that change** (docs/invariants.md):

| row | change |
| --- | --- |
| `wgpu` never reaches the shipping binary | deleted. In its place: the app never renders on a software adapter unless told, and the tests never on a discrete GPU unless told, held by `choose`'s unit tests and `the_test_adapter_is_the_igpu` |
| no `unsafe` outside `render/` | stands, or widens to `platform/` if the Mac window code lands there |
| a blit fits the whole rect it was given | holds through `set_viewport` in `paint()`; the tests move |
| a thumbnail readback never waits: "a PBO and a fence" | "a staging buffer and `map_async`, collected when its callback has fired" |
| the GPU timer never waits: `GL_QUERY_RESULT` | pass timestamps resolved into a buffer read by `map_async` |
| a program draws with its source's uniforms and tap slots, "with its own tap buffer at binding 0" | "pipeline", and the tap buffer is in the draw's own bind group |
| feedback equals a temp target blitted into a published one | "copied", same test |
| one wait per draw, for the draw `TICKS_IN_FLIGHT` before it, bounded by `QUEUE_WAIT_NS` | the wait is `device.poll(Wait)` on that draw's submission |
| a finished frame's readbacks make no GL call and wait for nothing queued behind: mapped once, persistent, coherent | rewritten: read out of a staging buffer mapped after that Output's own submission finished, and so behind nothing queued after it |
| a viewer is never handed an unfinished frame: "zero-timeout poll found signaled" | "whose submission the completed serial has passed" |
| nothing a held `Published` names is freed or drawn into … reader fences … `glWaitSync` | the freeing half becomes wgpu's reference counting; the drawing half stays with `Lease`; the fence half is deleted with `tests/reader_fences.rs` |
| an imported DMA-BUF goes back after every draw and viewer read | the same claim, on a completion serial |
| buffer storage Mesa parks is freed | deleted |
| every node compiles on the real GPU; every simulation kernel links | "every node's pipeline, every kernel's compute pipeline, is created on the real GPU" |
| **person-caught:** a shader's compile or link status is never queried before the driver says it is ready | replaced by "no shader module or pipeline is created on the synth or frame thread". It can become machine-caught: `render`'s one creation function asserts in debug builds that it is on `linker` or during startup |
| **person-caught:** a viewer fences its reads and holds its `Published` until the swap | "a viewer holds its `Published` until after the submit that carries its reads" |

---

## 7. Staging

### Build alongside, then flip — recommended

**Converting in place cannot keep the app running.** A GL program cannot sample a wgpu texture
without an interop layer (`GL_EXT_memory_object_fd` over Vulkan exports), and building one to
throw away is a detour bigger than any stage here. Every half-converted state of `render/` is a
state where nothing draws. A long branch would work, but visual work is checked by eye
as it lands, `main` keeps moving, and a 9,000-line module rewritten on a branch is a merge nobody
wants.

**Alongside keeps `main` green and runnable.** The new renderer, `src/render_wgpu/`, grows next
to `render/`, exercised only by its own headless tests until it reaches parity. Then eframe,
the synth, the viewers and the pictures thread flip onto it in a short series. glow is
deleted after the flip, and `render_wgpu` is renamed `render`.

What makes the flip short is a seam put in first, on glow (step 2): after it, `app/` and `synth/`
talk to `render::` in terms that do not name a backend. The flip then changes `render/`, `main.rs`
and the tests, and leaves app code alone.

### The steps

Sizes are in files and commits. "Needs" names what must land first. Steps with no arrow between
them run in parallel, on separate branches.

| step | what | needs | runs | size |
| --- | --- | --- | --- | --- |
| **0a** | the shader path: **WGSL**, decided — `proposals/shader-path.md` | — | done | — |
| **0b** | **the queue spike — built.** `examples/queue_contention.rs`: a heavy synthetic fragment load submitted from one thread under the one-tick bound, a light "editor" submission at the display rate from another, on one wgpu device on the iGPU and then with the synth on a second device at default and at low queue priority, headless and in an eframe window. [The verdict](#the-one-real-risk-one-queue): one queue starves the editor; Plan B does not | — | parallel | 1 file, 1 commit |
| **1** | **adapter selection.** `choose` with its unit tests; wgpu promoted to `[dependencies]`; kittest's `shared_gpu` in `tests/ui.rs` goes through `choose`; the "never wgpu" rule leaves the contributor rules and invariants.md in the same commit | — | **done**: `render::adapter`'s `choose`, `pick` and `headless` | — |
| **2** | **the seam, on glow.** `Renderer` holds its own context handle, so no `pub fn` of `render::` takes a `&glow::Context`. `render::Gpu` is the one handle type, wrapping glow for now. `Picture` and `Published` hold opaque handles. Paint callbacks are built by `render::viewer` constructors, so `app/` stops naming `egui_glow`; `release_pending_on_paint`, `fence_reads_on_paint` and `PaintTimer` wiring move under `render::`. The synth names no graphics crate, so `tests/rules.rs` tightens. Tests and examples follow mechanically. **The app runs after every commit** | — | **done**: `render/handle.rs` holds `Gpu` and `Texture`; `tests/rules.rs` holds GL to `render/` and `main.rs` but for `App::on_exit`'s signature; the GL building blocks the GL tests drive still take a context | — |
| **3** | **the skeleton.** `src/render_wgpu/`: `Gpu` (instance, adapter through `choose`, device, queue), the completion serial, `Ring` and `Lease` (claim only), `OutputRenderer` drawing a fixed pipeline, `publish`, the `TICKS_IN_FLIGHT` wait, and `Renderer::draw`'s loop with every phase present as a stub (sources, sims, probes, mix, marks). `tests/common/gpu.rs` and `gpu_ring.rs` hold the ring, zero-flash and feedback tests on hand-written WGSL | 1 | **done**: one device and one queue with [the throttle](#the-one-real-risk-one-queue), and a pipeline made from a compiled WGSL module rather than a fixed one — a checkerboard graph draws through it and reads back pure black and white. What each renderer lane fills is [below](#what-each-renderer-lane-fills) | — |
| **4** | **`compile/` writes WGSL beside GLSL**, [what the shader path gives the renderer](#what-the-shader-path-gives-the-renderer), and the pilot's 32 nodes with it: the conventions, the gate, the macro's WGSL half | — | **done** | — |
| **4a–4e** | **the node lanes**: the rest of the library in WGSL, [five lanes of whole files](#the-node-lanes) | 4 | five in parallel, beside 3 and 5 | ~15–28 nodes and 1–2 commits each |
| **5a** | uploads: `upload.rs` ported, `x` layouts through the conversion pass; `gpu_upload.rs` | 3 | parallel lane | 2 files and tests, 1–2 commits |
| **5b** | readbacks: taps (template copy, staging, map), thumbnail, Snap, capture with its halvings; `gpu_readback.rs`. Tests with real tap nodes wait for 4 | 3 (4 for tap nodes) | parallel lane | 2 files and tests, 2 commits |
| **5c** | timing: pass timestamps, the nine marks, `PaintTimer`'s wgpu form | 3 | parallel lane | 1–2 files, 1 commit |
| **5d** | simulations: compute pipelines, premade bind groups, reshape; `slimemold`'s kernels and its picture in WGSL, since both live in `nodes/slimemold.rs`; `gpu_sims.rs` | 3, 4 | parallel lane | 2 files and tests, 1–2 commits |
| **5e** | the mixer and the viewer: the blit pipeline for egui_wgpu callbacks and for surfaces, `Fit`, corner, unclamped viewport, flip; `gpu_mixer.rs` | 3 | parallel lane | 2–3 files and tests, 1–2 commits |
| **5f** | DMA-BUF: `texture_from_dmabuf_fd` with a dup'd fd, `importable()` through Vulkan modifier properties, serial-based return; its tests | 3 | parallel lane | 2 files and tests, 1–2 commits |
| **5g** | linking: the `linker` worker, error scopes, kept pipelines, probes (no pipeline cache: see [1.13](#113-linking)); the link tests (keeps the old program, a failure keeps it, superseded links, a source drawn before) | 3, 4 | parallel lane | 2 files and tests, 1–2 commits |
| **5h** | the node-level tests: every node compiles, taps, dual nodes, masks, Perlin, tile, the render job, on-demand, the camcorder; `gpu_nodes.rs`, `gpu_render.rs` | 5b, 5g, 4 | parallel lane, splittable in two | 2 test files, 2–4 commits |
| **5i** | **node pictures, glow against wgpu**: every node with WGSL drawn by both renderers and compared, [below](#node-pictures-glow-against-wgpu); a node that differs goes back to its lane's file | 5a, 5b, 5g, and each lane of 4 for its nodes | parallel lane; rerun as lanes land | 1 test file, 1–2 commits |
| **6** | **the flip.** In order: (i) `main.rs` makes the `Gpu` and hands it to eframe with `Renderer::Wgpu`, `App::new`, the synth and the picture host move onto `render_wgpu`, and the pictures thread draws on wgpu surfaces, borrowed display kept — **the app runs on wgpu here**; (ii) glow's `render/`, `tests/headless_gl.rs`, `tests/release_pending.rs`, and `glow`, `khronos-egl`, `glutin` and `wayland-sys`'s `egl` feature are deleted, and GLSL with them: `OutputDef::glsl`, `NodeDef::shader_utils` and `measure`, the macro's GLSL half, `compile/glsl.rs`, `prelude.glsl` and `compile::Target`, with `wgsl`, `wgsl_utils`, `wgsl_common` and `measure_wgsl` keeping their names rather than being renamed across sixty files, `reader_fences.rs` folds into `gpu_ring.rs`, `crosstalk.rs` is ported; (iii) `render_wgpu` is renamed `render`; (iv) the examples, `doctor.sh`'s Vulkan check and `distrobox.ini`. Docs change in the commits that change behavior, per [section 8](#8-what-goes-stale-in-docs) | every 4 and 5, 0b's verdict | **done**: (i) `main.rs` makes the one `render_wgpu::Gpu` and hands it to eframe through `WgpuSetup::Existing`; the synth, the editor's viewer and paint timer, and the pictures thread's surfaces draw on it; `tests/gpu_app.rs` holds the App-level tests on it, `tests/crosstalk.rs` runs on it. (ii)–(iv) glow's renderer, its four GL test files and `node_pictures.rs` are deleted with `glow`, `khronos-egl`, `glutin` and `wayland-sys`, and GLSL with them; `render_wgpu` is renamed `render`, holding `render::adapter` and `render::picture`; `doctor.sh` checks the Vulkan adapter by `choose`'s rule and zero-copy by the device's DMA-BUF extensions | ~35 files plus the node files' GLSL, 5–6 commits, each green |
| **7** | **measure** the [table](#what-to-measure-before-and-after) against the glow figures taken just before (i), and **it is run by hand on the live app**. The flip is not done until it has been | 6 | sequential | numbers into rendering.md and decisions.md |
| **8** | Plan B's device of its own for the synth and its sharing across devices (export, import, the viewers' completion-side release), **only if** the throttled one queue the skeleton carries measures badly after the flip (item 2 of [the list](#what-to-measure-before-and-after)); `render::Gpu::for_synth` is the one door it changes | 3, 7 | sequential | ~6 files, 3–4 commits |
| **9** | macOS: the picture windows (**built**: winit windows inside eframe's own loop, [macos-windows.md](macos-windows.md)), the bytes-only media path with VideoToolbox's IOSurface import after it, the app bundle, signing, CI on Apple Silicon | 6 | three parallel lanes: windows, media, packaging | each its own proposal |

Step 0b was cheap and early on purpose, and it showed the editor starving behind the synth on
one queue. The skeleton answers it on the one queue instead — one Output per submission, at most
one of the synth's submissions queued ahead of the next — which measured within a millisecond or
so of Plan B with no overrun (the one-queue section). `render::Gpu::for_synth` is the one
door Plan B would change, so step 8 needs no caller to move.

### What each renderer lane fills

The skeleton (step 3) calls every phase already, so each of lanes 5a–5g fills **its own files
and nothing else**: the functions below exist as stubs with their final signatures, and
`src/render_wgpu/mod.rs` — `Renderer::draw`, `Renderer::publish` and every accessor that mirrors
glow's `Renderer` — already calls them in the right place. A tick is three kinds of
submission: the **prelude** (`sync`, then 5a's uploads, then 5d's dispatches), **one per
Output** (`OutputRenderer::encode`), and the **coda** (the probes, then 5e's mix); 5c's marks
bound each phase inside those recordings.

| lane | owns | fills |
| --- | --- | --- |
| **5a** uploads | `src/render_wgpu/sources.rs`, `tests/gpu_upload.rs` | `Sources::sync` (bytes through `queue.write_texture`, the conversion pass recorded into the prelude's `Recording`, skip by pointer, drop the gone), `views`, `publish`, `texture_of`, `forget`; a DMA-BUF frame goes to 5f's `Imports::import` and a replaced one to `Imports::let_go` |
| **5b** readbacks | `src/render_wgpu/readback.rs`, `tests/gpu_readback.rs` | `Readbacks::settle` (collect finished records; the capture's two-second wait), `tap_buffer` (the in-flight record's buffer; the template copy is there already), `after_pass` (tap copy into staging, thumbnail, Snap, capture halvings), `submitted` (`map_async` after the submit), `take_taps`, the thumbnail, Snap and capture accessors, `wants_picture`, `poll`. `read_texture` is the tests' blocking read and stays |
| **5c** timing | `src/render_wgpu/timing.rs` | `Stamps::{new, collect, mark, take}` — `mark` gets the phase's `Recording` and calls `encoder()` only when it writes, so an empty phase still submits nothing — and `Timer::{pass_timestamps, after_pass, collect, reading, forget}`; `PaintTimer`'s wgpu form in the same file |
| **5d** simulations | `src/render_wgpu/sims.rs`, `nodes/slimemold.rs`'s WGSL, `tests/gpu_sims.rs` | `Sims::{sync, views, publish, texture_of, read, forget}` |
| **5e** mixer and viewer | `src/render_wgpu/mixer.rs`, `src/render_wgpu/viewer.rs`, `tests/gpu_mixer.rs` | `Mixer::{sync, draw, submitted, poll_frames, publish, texture, targets, size, dropped}` on a `ring::Ring` of `Format::Byte`, resized with `Ring::resize`'s carry; the blit and its `CallbackTrait` in `viewer.rs`, which is empty |
| **5f** DMA-BUF | `src/render_wgpu/dmabuf.rs`, `tests/gpu_dmabuf.rs` | `Imports::import` (today a refusal), `importable()`; `let_go`, `submitted` and `sweep` already hold a frame until the serial after its last sampler has finished. `gpu::WANTED_FEATURES` already asks for `VULKAN_EXTERNAL_MEMORY_DMA_BUF` |
| **5g** linking | `src/render_wgpu/link.rs`, `src/render_wgpu/program.rs`, `tests/gpu_link.rs` | `Programs::set_shader` sends to a `linker` thread and `Programs::poll` becomes a `try_recv` — today the pipeline is made in `set_shader` and lands on the next `poll`, so callers already see the asynchronous shape; `Program::create` (its error scope is there) runs on the worker |
| **5h** node tests | `tests/gpu_nodes.rs`, `tests/gpu_render.rs` | tests only, through `Renderer` and `tests/common/gpu.rs`'s `one_node`, `compiled`, `job`, `link_all` and `rgba_of` |

**Nobody but the flip edits** `mod.rs`, `gpu.rs`, `queue.rs`, `ring.rs`, `lease.rs`,
`output.rs`, `shared.rs`, `uniforms.rs` or `tests/common/gpu.rs`. A lane that needs a helper the
harness lacks writes it in its own test file; a lane that finds a hook missing or misplaced
reports it back rather than moving it, so two lanes never meet in one file. The renderer's own
stages are WGSL constants in the file that draws them (`shared::CARRY` so far) and join
`tests/shader_targets.rs`'s corpus with the lane.

### The node lanes

After the pilot, 103 nodes have no WGSL; `cargo test --test shader_targets -- --nocapture`
prints which. Each lane owns **whole files**, so no two lanes touch one file:

| lane | files | nodes |
| --- | --- | --- |
| **4a** generators | `shapes.rs`, `patterns.rs`, `fractals.rs`, `worldcoordinates.rs`, `cosinegradient.rs` | 16 |
| **4b** color | `adjust.rs` (all but `vignette`), `saturate.rs`, `recolor.rs`, `palette.rs`, `hsla.rs`, `chromakey.rs`, `wavefold.rs` | 15 |
| **4c** space | `transform.rs`, `distort.rs`, `region.rs`, `wallpaper.rs`, `geissflow.rs` | 22 |
| **4d** effects | `convolve.rs`, `multisample.rs`, `screentone.rs`, `edgedetection.rs`, `chromaticaberration.rs`, `pixelsort.rs`, `glitch.rs`, `camcordercrt.rs`, `stargate.rs` | 21 |
| **4e** numbers and sources | `math.rs`, `reframerange.rs`, `sliderule.rs`, `muxevent.rs`, `muxnumber.rs`, `camera.rs`, `screencapture.rs`, `maininput.rs`, `imagegif.rs`, `text.rs`, `brickgame.rs` | 28 |

`slimemold.rs` is lane 5d's. What keeps five parallel lanes from colliding:

- **A lane edits only its own files**: a `wgsl` generator beside each `glsl` one, `wgsl_common`,
  `wgsl_utils`, `; wgsl =`, `measure_wgsl`, and `*_WGSL` constants. Never `nodes/mod.rs`,
  `macros.rs`, `compile/`, `tap.rs`, `decompose.rs`, `audio_ports.rs`, the two test files or the
  docs — a gap in the conventions or a compiler bug is reported back and fixed once, on `macos`,
  not patched in five places.
- **No count to bump.** How many nodes have WGSL is printed, never asserted, and no registry
  list names them. A lane's snapshots are new files, `wgsl_{group}_{slug}_{wiring}.snap`, one
  pair per node, so two lanes never write the same one.
- **Helpers shared across lanes are written already.** `noise::SIMPLEX3D_WGSL` and `FBM_WGSL`
  for `domainwarp`, `distort::HASH_RANDOM_WGSL` for `scatter`, `randomhurl` and `pixelsort`.
  A lane's own helper takes a prefix of its file or node family, since a helper name is unique
  across the registry: `every_wgsl_util_name_is_unique` fails the merge that would clash.
- **Each lane's gate is `./check.sh`** with its nodes gone from the printed list, and its new
  snapshots reviewed once by eye. The merges are then file-disjoint.

### Node pictures, glow against wgpu

A module that validates only type-checks. Before the flip deletes glow, every node with WGSL
is drawn by both renderers, in one test process on the iGPU: **`tests/node_pictures.rs`**,
built. Each node is placed in the graph `tests/compile.rs` snapshots, unconnected and
connected, then **apart** where it has two inputs of one kind — every input from a source of
its own (a checkerboard, a linear and a radial gradient; a vignette's mask, world x, world y),
so a blend of two pictures or a swapped pair of inputs does not compare a thing with itself —
and again for every choice of each `Code` and `Uniform` option. Each is drawn once at 128×72
and one `u_time`, every uniform a CPU node publishes at one value, one asymmetric RGBA8 frame
uploaded to both for every texture a CPU node publishes, and each Output read back as half
floats and compared as it lies: both store rows bottom first, which
`the_rows_lie_the_same_way_under_both` holds on a field of `y` and a camera frame. A case past
its tolerance fails. 134 nodes, 854 cases, under ten seconds; `NODE_PICTURES=slug,slug`
narrows it, `NODE_PICTURES_DUMP=1` prints the pixels that differ, `--nocapture` the table.

- **Tolerance per case, not one number.** The proposal expected Mesa's GL and Vulkan drivers to
  differ in a `sin`'s last bits, and hashed nodes to come out as different noise. On this iGPU
  they do not: iris and anv share Intel's compiler backend, and **98 nodes are bit for bit**,
  hashed ones included — `static`, `worley`, `glitch`, `scatter`, `randomhurl`, `halftone`,
  `dither`, `posterize` — and 34 more within one half-float step (2.4e-4 near 0.5, 4.9e-4
  near 1). So a case is held pixel for pixel, every channel within 4e-3, and none is compared
  by mean and spread; the table prints the mean-and-spread difference beside the rest.
- **Taps** are compared as decoded readings, within 1e-3: every `tap`, `sample` and
  `autoexposure` case reads identically. A **probe's** counts are reported, not failed, where
  a WGSL body binds a spliced input to a `let` or evaluates both sides of a `select`, counted
  over a probe's 144 pixels: `radialgradient` (GLSL splices `radius` twice, 236 or 272 calls
  against WGSL's 144) and `divide` (GLSL's guard skips `a` where `b` is zero, 92 against 144,
  and reads `b` twice where WGSL's `select` reads it twice everywhere, 236 against 288). The
  pictures agree.
- **No node's WGSL had a translation error.** Two nodes are held looser, and three have a
  known difference in some cases, measured and printed rather than failed:

| node | cases | worst | what |
| --- | --- | --- | --- |
| `kuwahara` | 12 | 5.4e-3 | held within 1e-2: each quadrant's variance is E[x²] − E[x]², which cancels, so of two nearly as flat quadrants either may win, a few thousandths apart |
| `pixelsort` | 18 | 0.5 on 0.17% of pixels | held within 4e-3 but for 0.5% of pixels: on a wired checker a chunk's samples land on an edge, and whether `a − b·c` is fused decides the side |
| `wallpaper` | 51, 9 known | 1.0 on 32% | `p4`, `p4m`, `p4g`: `S_nm` is real, so `f.y` is a zero each compiler rounds to its own sign, `angle` is `atan2` of it, and a wired checker sampled at `0.5 + f.y · texScale` sits on its edge. The GLSL is as ill-conditioned; the square groups' `angle` is noise under either renderer |
| `phyllotaxis` | 3, 1 known | 1.0 on 26% | apart only: a cabled `dotSize` of 0 meets the edge's two ends, and GLSL's reversed `smoothstep(dotSize, dotSize · 0.5, d)` paints the dot color where WGSL's ordered `1 − smoothstep` paints the background. Both undefined; the knob stops at 0.005. Reversing the WGSL's makes the case bit for bit |
| `tunnel3d` | 27, 1 known | 1.0 on 14% | apart, `helix`: a cabled radius clamped to 1e-3 is inside the march's 0.001, so it stops on the helix's axis and the wall angle is `atan2` of a rounding-signed zero; every differing pixel is where world x is negative |

- `slimemold` is not compared until lane 5d writes its WGSL; the test takes a node the day its
  module carries no `Untranslated`, and gives a simulated texture the same uploaded frame a
  CPU node's gets.
- The comparison goes with glow.

The glow tests keep running through steps 1 to 5, so `check.sh` runs both suites for a while.
Before step 6 it takes noticeably longer. After it, less than today.

## 8. What goes stale in docs

To be folded in by the commits of step 6, since a change in behavior and its doc belong in one
commit, and docs record what is true rather than what changed.

- **The contributor rules.** The first line ("OpenGL via `glow`"). The Done paragraph's "the `Renderer` on
  its shared EGL context", and the picture-window sentence about the borrowed display, the share
  group, EGL window surfaces and a third context. Hard rules: the `unsafe` row's "render/ exists
  to be the one place wrapping glow", and the "Never wgpu in the shipping binary" row (deleted in
  step 1). Testing traps: "Headless GL needs a `PBUFFER`" becomes "one wgpu `Instance` per test
  process"; "Never query a shader compile or link status on the frame thread" becomes "never
  create a pipeline on the synth or frame thread"; "GPU ms per Output from a `GL_TIME_ELAPSED`
  query" becomes pass timestamps. The Environment section's `llvmpipe` line gains `lavapipe`
  and names the adapter rule.
- **docs/rendering.md.** Nearly every section names a GL mechanism:
  - The intro's "glow's unsafe surface".
  - "The frame is paced by the swap": `swap_buffers` inside Mesa becomes eframe's surface present.
  - Per Output, per frame; draws and skips; the order a tick submits in: fences become the
    completion serial and per-Output submissions. Fix while folding: "at the fixed limit of
    three" contradicts `ring::RING`, which is five.
  - Texture wrapping: `glTexParameteri` becomes a sampler choice.
  - Source textures: the pixel-unpack buffer, `UNPACK_ROW_LENGTH`, `GL_BGRA`, the alpha swizzle.
  - Simulations: image units and barriers; the iris timestamp note.
  - DMA-BUF import: EGL, `glEGLImageTargetTexture2DOES`, the sticky error flag.
  - Tap buffers: binding 0, the memory barrier, persistent mapping, `glCopyBufferSubData`, the
    iris per-submission argument. The argument survives as the reason for per-Output submissions.
  - The thumbnail, Snap and capture readbacks: PBOs, `read_pixels`, blits.
  - The GPU timer and the phases: `GL_TIME_ELAPSED`, `glQueryCounter`. The editor's own painting.
  - "Paint callbacks, and the GL state they inherit": rewritten around `CallbackTrait` and the
    viewport override.
  - "The three contexts": **deleted**, replaced by "One device", with the queue and priority
    finding.
  - Every window is a viewer: `glWaitSync`, `fence_reads`, reader fences.
  - Picture windows: EGL surfaces, the alpha mask, and why the display is borrowed.
  - Showing a frame: `register_native_glow_texture`.
  - Linking: `GL_KHR_parallel_shader_compile`.
  - Discipline: "uniform locations are cached on link" and "no GL objects are allocated on every
    frame", now false of bind groups.
- **docs/architecture.md.** The module-layering rows for `render/` (`glow`, the Wayland crates)
  and `synth/` ("may name `glow` where it hands a context"). "One clock, on a thread of its own"
  ("what the frame thread makes and hands over is the EGL context alone, which has to be created
  while eframe's is current"). "The pictures, on a thread of their own" (borrowed `wl_display`,
  third context). "The frame, and the viewers" (the three contexts link).
- **docs/decisions.md.**
  - "glow, not wgpu" becomes "wgpu, not glow", with why glow lost: macOS GL 4.1 has no storage
    buffers, fragment atomics or compute, and one renderer for two OSes.
  - "Blit in a paint callback, not a registered texture": the egui_glow wording.
  - "The compositor paces the viewers": Mesa's swap and EGL.
  - "Every picture window is a Wayland surface of its own": the share-group argument, and
    "rejected: a Wayland connection of our own", whose reason is gone. Its remaining reason is
    focus.
  - "`GL_TIME_ELAPSED` per Output" becomes timestamps per pass; its rejection of timestamp pairs
    inverts.
  - "An Output draws into a ring" and "a bounded ring and one GPU tick in flight": fence wording.
  - "The synth's context runs at low GPU priority": deleted, or rewritten as Plan B if it is built.
  - "Every context frees what Mesa parks": deleted.
  - "Shader links go to the driver's threads, not to one of ours": reversed.
- **docs/invariants.md.** The rows in [section 6](#6-the-test-suite).
- **docs/nodes.md.** Everything that says GLSL: the generator contract's `float name(vec2
  uv)`, the macro's "a body is GLSL", `shader_utils`, and the [WGSL](../docs/nodes.md#wgsl)
  section's "until that renderer replaces glow", which becomes the contract itself. The
  contributor rules' "GLSL 4.30" for taps and "A control value is never baked into generated GLSL".
- **docs/testing.md.** Layer 3 (EGL on the device, `PBUFFER`, the software renderer strings and
  `SUPERSILVIA_SOFTWARE_GL`). The crosstalk paragraph's "a viewer on a shared context". The
  one-instance rule, moved up from `tests/ui.rs`'s module doc to where every GPU test sees it.
- **docs/media.md.** The DMA-BUF paragraphs (`EGLImage`, "one EGL call", the formats "EGL
  imports", "asked of EGL by a test").
- **docs/ui.md.** "`Ground::Cleared`, wherever eframe paints with glow"; "no GL context" in the
  Status box rows; "GL textures are y-up and egui images are y-down" (true of the bytes still,
  but named for GL); the picture-window paragraphs' "EGL surfaces".
- **DEVSETUP.md, scripts/doctor.sh, distrobox.ini.** GL and EGL renderer checks give way to the
  Vulkan adapter check. The zero-copy check asks Vulkan for `VK_EXT_external_memory_dma_buf` and
  `VK_EXT_image_drm_format_modifier` rather than EGL. The package list drops the EGL and GL
  headers and keeps `mesa-vulkan-drivers` and `vulkan-tools`.
- **Cargo.toml comments** on `khronos-egl`, `wayland-sys`'s `egl` feature, `glutin` and
  `egui-wgpu`.
- **Module docs in `src/`**: `render/mod.rs` ("All glow code"), `egl.rs`, `retire.rs`,
  `viewer.rs`, `ring.rs`, `output.rs`, `shader.rs`, `picture/mod.rs` and `thread.rs`,
  `synth/mod.rs` and `thread.rs`, `app/frame.rs`. These change with their code anyway.
- `proposals/picture-windows.md` and the other built proposals are records, and stay as written.

## Decisions

Only the ones visible in use:

1. **The one-queue measurement came back bad, and the throttle fixes it.** On one queue as
   first measured, a tab that keeps the iGPU busy for most of each frame drops the editor to
   about half the display's rate, and a tab heavier than the GPU can keep up with drops it to
   50 Hz. With the synth handing the GPU one Output at a time, the editor stays smooth at every
   load measured and the synth slows instead, as it does today — the same as Plan B, a
   millisecond or so later per editor frame, for none of Plan B's extra code. The plan now
   builds the throttle and keeps Plan B in reserve, to be judged on the live app after the
   flip.
2. **Fullscreen on the Mac — decided:** `F` makes a picture window cover its screen instantly,
   in place, with the menu bar and the Dock hidden while supersilvia is in front, rather than
   sliding into a Space of its own. [macos-windows.md](macos-windows.md), decision 1.
3. **Faster shaders for some nodes, slower for one.** Turning off a safety counter wgpu adds
   to every shader loop makes Lyapunov a quarter cheaper (Games and simulations from 42 to 53
   ticks a second) and Bloom and Kuwahara a fifth, but makes Pixel Sort two and a half times
   dearer. Either leave it on everywhere, or turn it off everywhere but for nodes that are
   measured to lose, or leave it on and have the node lanes rewrite the loops that pay for it.
   Lyapunov, the largest, runs a fixed ten iterations, which is built;
   those figures are at 80 iterations, and Bloom and Kuwahara are what is left. The
   explainer, the measurements and a recommendation (the third) are
   [docs/loop-bounding.md](../docs/loop-bounding.md).
4. **The editor behind one long Output.** Beside Games and simulations the editor still misses
   its 10 ms on about a third of its frames, because one Output there, Lyapunov at 80
   iterations, was a single 17 ms pass and an editor frame waits for all of it. Lyapunov runs a
   fixed ten, an eighth of that loop's work, and the tab has not been measured since; what
   follows holds for any Output that long. Short of a second GPU device for the
   synth at low priority (Plan B), the one thing that would help is drawing an Output that
   long in bands, a few milliseconds each, so the editor can go in between; that costs a
   little synth time and is unbuilt.
5. **No Vulkan, no app:** on Linux, a machine with no Vulkan driver (very old Intel graphics, or
   a box whose driver is missing) will refuse to start and say why. It does not fall back to GL.
   That is a direct consequence of converting everything. It is stated here so it is not a
   surprise.
