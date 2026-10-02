# Proposal: every picture window is a surface of our own

**Status: built.** The shape below is the shape the code has;
[docs/rendering.md](../docs/rendering.md#picture-windows),
[docs/architecture.md](../docs/architecture.md) and [docs/decisions.md](../docs/decisions.md)
are what describe the system. What is kept here is the argument and the measurements.

[deterministic-loop.md](deterministic-loop.md) is what this continues: the synth keeps
its own time and never stops, and this is the other half — the *windows* never stopping
either.

The decision, which is the whole requirement: pop-out windows and projectors get their own
window and their own loop, and the projector is borderless like the pop-out. No special
projector is needed if the mixer's result can be popped out.

So there is one kind of thing — a **picture window** — and the projector is not a kind of
window, it is the mix's pop-out.

## The cost this is paying off

`deterministic-loop.md` closed with one open cost, recorded in
[docs/decisions.md](../docs/decisions.md) under the loop decision: **a minimized editor stops
the projector.** The synth thread fixed the world stopping; it did not fix the picture
stopping. eframe runs one winit `EventLoop`, that loop services every viewport in turn, and a
deferred viewport is only stepped while the parent is running passes. A minimized editor runs
none — `request_repaint_after(quarter second)` is what stops it blocking in `swap_buffers` on
a frame callback KWin will never send — so the projector holds its last frame.
`pre_present_notify` is the call that would service the deferred viewport while the parent
idles, and it was measured twice and rejected twice: 97.9% of a core against 17.2%, because
eframe polls the loop for as long as a redraw is held.

Both sides of that trade are eframe's one event loop. The way out is to stop asking eframe
for these windows at all.

## What a second winit loop cannot do

winit permits exactly one `EventLoop` per process, and it is not a convention — it is a
static:

```rust
// winit-0.30.13/src/event_loop.rs:69
static EVENT_LOOP_CREATED: AtomicBool = AtomicBool::new(false);
// :118
if EVENT_LOOP_CREATED.swap(true, Ordering::Relaxed) {
    return Err(EventLoopError::RecreationAttempt);
}
```

`EventLoopError::RecreationAttempt` prints "EventLoop can't be recreated". The flag is
cleared only when the loop is dropped, and eframe holds ours for the life of the process. A
second winit loop is out, on any thread. Patching eframe to paint viewports off its own
thread is the other door, and it is not a door: the glow backend keeps one GL context and
makes it current per viewport in turn, which is a per-thread resource, so "paint that
viewport over there" means giving each viewport a context of its own — which is this
proposal, done inside someone else's crate.

## The shape: a Wayland surface of our own, on a thread of its own

One thread, named `pictures`. It owns a `wayland-client` connection, an `xdg_toplevel` per
picture window with no decorations, an EGL **window surface** on each, one GL context sharing
with the synth's group, and one `Viewer` made on that context. Each surface's frame callback
is that window's clock: wait the published fence with `glWaitSync`, blit the picture, swap.
The compositor paces each window separately, so what the editor's window is doing — idle,
occluded, minimized, gone — is not a fact any picture window can observe.

### The one hard constraint: the share group is per `EGLDisplay`

This is the finding that decides the shape, and it is the one that would have been found late
and expensively. An EGL share group cannot cross two `EGLDisplay`s, and on Wayland an
`EGLDisplay` is `eglGetPlatformDisplay(EGL_PLATFORM_WAYLAND_KHR, wl_display)` — **keyed by
the `wl_display` pointer**. A fresh `Connection::connect_to_env()` is a fresh `wl_display`, so
a context made on its display could not sample one texture the synth drew. Everything would
compile; nothing would draw.

So the pictures thread does not open a connection. It **borrows eframe's**:

```
App::new(cc)  →  cc.display_handle()  →  RawDisplayHandle::Wayland { display }
                 Backend::from_foreign_display(display)   (unsafe; render/)
                 Connection::from_backend(backend)
                 conn.new_event_queue()                   (the pictures thread's own queue)
```

`Backend::from_foreign_display` exists for exactly this — wayland-backend names "interacting
with an existing connection" as its use — and one `wl_display` carrying several
`wl_event_queue`s, one per thread, is libwayland's own design rather than something being got
away with: a proxy created on our queue delivers its events to our queue, and
`prepare_read`/`read`/`dispatch_pending` is the reader protocol that keeps two threads off the
socket at once. winit's side of it goes through `calloop-wayland-source`, which is the same
protocol.

The EGL side then needs no guessing either. `SharedContext::create` today asks EGL what is
*current*, which only works on the frame thread; the pictures thread gets the display and the
context to share with **handed to it as values**, so a third context is made the same way the
synth's second one is and joins the same group.

### What is Wayland-only, and what happens elsewhere

All of it. On X11 — or any session whose display handle is not
`RawDisplayHandle::Wayland` — a picture window does not open, and the mark that asked for one
says so in a line the toast carries: *picture windows need Wayland*. **There is no second
path.** An X11 fallback would be a whole second windowing backend, with its own input,
its own decorations and its own EGL surface handling, to serve a case this instrument does not
have: it is developed on KWin under Wayland, `doctor.sh` already refuses a session it does not
recognise, and a second path nobody runs is a second path nobody fixes. The editor itself is
unaffected — eframe keeps both backends, and only the picture windows are Wayland's.

### The crates, and why none of them is new weight

Everything needed is already linked into the binary, transitively through winit, and the
versions are the ones the lock already holds — so declaring them directly moves no version
and adds no code to the image:

| crate | lock | who already pulls it |
| --- | --- | --- |
| `wayland-client` 0.31 | present | winit, smithay-clipboard |
| `wayland-backend` 0.3 | present | the same |
| `wayland-sys` 0.31 (`egl`) | present | the same |
| `smithay-client-toolkit` 0.19 | present | **winit's own**, pinned to 0.19.2 |
| `calloop` 0.13, `calloop-wayland-source` 0.3 | present | winit's own |
| `raw-window-handle` 0.6 | present | eframe, winit |

Declaring all seven moved **nothing** in `Cargo.lock`: the diff is seven edges from our own
package and no new entry.

`smithay-client-toolkit` is the one worth arguing for rather than dropping to bare
`wayland-client`. It gives xdg-shell as a `Window` with `move_`, `resize`,
`set_fullscreen`/`unset_fullscreen` and `request_decoration_mode` already written, plus the
seat and pointer handling. Bare `wayland-client` would mean writing the xdg-surface
configure/ack dance and the decoration negotiation by hand, for the two gestures this needs.
**0.19.2 is pinned deliberately**: it is the version winit resolves to, so the lock does not
gain a second copy of sctk, calloop, rustix or thiserror. (0.20.0 is also in the lock, under
smithay-clipboard; taking it would duplicate all four.)

**Its `xkbcommon` feature stays off**, which is how winit takes it too: turning it on would
add the `xkbcommon` crate and a second `memmap2` — the one thing in this change that would
have been new weight. Without it sctk has no keyboard, so `wl_keyboard` is bound directly and
the two keys are read as **evdev positions**: `Escape` is 1 everywhere, and `F` is the key
where `F` sits on a QWERTY board. An xkb keymap reader for one shortcut is not the trade.

`wayland-egl` is not a crate in the lock, but `wayland-sys`'s `egl` feature is exactly it:
`wl_egl_window_create/resize/destroy` through `dlib`, the same shape `khronos-egl`'s dynamic
instance already is, and a feature flag rather than a dependency.

### Where the `unsafe` is

`unsafe` is `render/`'s alone, and this keeps it there. The protocol side is safe: sctk and
`wayland-client` are safe APIs, so window state, input and the message loop are ordinary Rust
and can live wherever they read best. The three unsafe things are all EGL or libwayland
handles and all go in `render/egl.rs` beside the loader:

- `Backend::from_foreign_display(ptr)` — a `wl_display` we borrow and never free.
- `wl_egl_window_create/resize/destroy` — dlopened, and the handle is a pointer.
- `eglCreateWindowSurface` on that handle, and `eglSwapBuffers`.

## Behaviour: the pop-out's, kept

[docs/ui.md](../docs/ui.md)'s pop-out section is the contract, and none of it changes in what
a hand does — only in who implements it. What eframe's viewport commands did, xdg-shell does
directly, and rather more honestly, since `StartDrag` was always `xdg_toplevel.move` underneath:

| what | today | on our own surface |
| --- | --- | --- |
| no decorations | `with_decorations(false)` | `request_decoration_mode(Client)`, and none drawn |
| dragged by its picture | `ViewportCommand::StartDrag` | `xdg_toplevel.move(seat, serial)` |
| resized by its edges | `BeginResize(SouthEast)` | `xdg_toplevel.resize(seat, serial, edge)` |
| `F` / double-click | `ViewportCommand::Fullscreen` | `set_fullscreen` / `unset_fullscreen` |
| `Escape`, then `Escape` | the same two-step | the same two-step, read off our own keyboard |
| letterbox | `Viewer` with `Fit::Letterbox` | unchanged — it is the same `Viewer` |
| opens at the picture's size | `with_inner_size` | the first `xdg_surface` commit's size |

The serial is the one thing that has to be right and is easy to get wrong: a compositor
refuses a `move` or a `resize` carrying a serial that is not from a recent input event on
that seat, so the pointer handler keeps the **press** serial and the gesture uses it.

### The player strip does not come

This is the one behaviour that is lost, and it is stated here as the cost rather than
discovered later. The strip along a picture's foot — the scrubber, the speaker, the volume —
is `ui/player.rs`: an egui widget, drawn into an egui `Ui`, reporting a `Touched` back to the
command bus. A picture window has no egui in it. Drawing it there means either putting an
egui context and a tessellator on the pictures thread — a second renderer, a second font
atlas, and every frame of it competing with the blit this window exists to do — or writing a
second scrubber in raw GL that has to be kept in step with the first by hand.

Neither is worth it, and the decision said which way to fall: *the priority is the picture
staying live.* So **a picture window is a bare picture.** The strip stays where it has always
been drawn and is not going anywhere: on the picture in the node's own body, in the editor.
A clip is scrubbed on its node; the window shows what that did. `docs/ui.md` says so in the
pop-out section.

The two marks — close and fullscreen — go the same way and for the same reason, and are less
of a loss: `Escape` closes, `F` fullscreens, and the node's own marks still say what the
window is doing and still put it away, which is the affordance that mattered when the window
is on a screen the hand cannot see.

## The projector is retired

There is no projector. The Main Mixer panel's **Open projector** becomes the mix's pop-out
mark, drawn as the pair every other picture carries, and the mix joins the same list of
picture windows everything else is in. What goes with it:

- `Show::projector`, `App::projector_closed`, `show_projector`, and the `PROJECTOR_WIDTH`
  that was half a 1080p row.
- `MenuAction::SetProjector` and the View menu's **Projector** tick.
- `render::Live` — the slot that existed so a *deferred viewport's* `Fn + Send + Sync`
  callback could read a `Published` it could not borrow. A picture window is not a callback
  and does not have that problem: the pictures thread holds the `Arc<Live>` the synth writes
  and reads it on its own clock. The slot stays; what goes is its being the projector's.

`PopOut` gains a third case for the mix beside a node's render and a source's texture, which
is the one place the mix was never "a picture" and now is.

`H` to hide the editor and `F` to fullscreen the **editor** are untouched: they are the
editor's own keys and always were.

## The threading contract

Written properly in [docs/rendering.md](../docs/rendering.md); the shape:

```
editor ──Ask──▶ pictures        open a picture / close it / fullscreen it / the mix's size
editor ◀─Told── pictures        this window closed, this one went fullscreen
synth  ──Arc<Live>──▶ pictures  the newest Published, the same slot the editor's viewers read
```

The pictures thread owns its connection, its queue, its windows, its EGL context and its
`Viewer`, and nothing of that crosses. The editor owns the list of which pictures *should*
have windows, which is what the node marks read — so the marks answer instantly and the
thread confirms. Fences and textures retire on the synth's own ring exactly as they do now.

### `RETIRE_TICKS` has to answer for a window on another clock

`retire.rs` justifies four by the *editor's* frame: a `Published` is taken at the top of a
frame and blitted until the end of it, at a tick-per-frame ratio of one, plus two dropped
frames, plus one. A picture window breaks that argument's premise, because it is not the
editor's frame: a 60 Hz projector beside a 100 Hz tick holds a `Published` for 16.7 ms while
the synth places 1.67 fences in it — so the oldest fence it can wait on is two ticks old, plus
one for the tick it was taken on, which is three, and four still covers it with one to spare.
It stops covering it at a tick rate **more than three times** a window's refresh: a 240 Hz
tick beside a 60 Hz window is four, and the fifth is a skipped wait and a torn frame.

That is not a rate this runs at and not one the preference offers, but the argument in
`retire.rs` no longer holds on its own terms, so it is rewritten to the ratio rather than to
the editor's frame, and the number goes to **six** — the same "two dropped frames plus one"
slack, over a ratio the slowest window and the fastest tick can actually reach. Six fences and
six textures is a handful of words and a texture that lives two ticks longer; it costs
nothing worth counting against a tear.

## What this is not

**Not KMS.** Still the right shape for a box that only goes to a gig, and still a second
windowing path with input and dialogs to answer for. This gets the requirement without it.

**Not a second winit loop.** The static above.

**Not eframe patched.** One context per viewport inside someone else's crate is this
proposal, written where it cannot be tested.

**Not egui in the picture window.** The strip is the price and it is named above.

## What the build found that the argument did not

**The window was translucent.** The EGL config is eframe's own window config, which carries an
alpha channel because egui wants one — and a Wayland surface with alpha is *blended with the
desktop behind it*. The blit wrote the mix's own alpha, so the desktop showed through the show:
visible in the first capture of a picture window and in nothing any test could have asked. The
fix is `glColorMask(…, false)` over a clear that wrote 1.0, plus `set_opaque_region` so the
compositor need not blend at all. Not a config of our own: sharing needs one `EGLDisplay` and
compatible contexts rather than an identical config, and reusing the one the group already
agreed on is simply the simplest way to be sure they are compatible.

**A frame callback is one-shot, and a compositor stops sending them** for a surface it is not
compositing — which turns out to be the mechanism that makes a hidden picture window free
rather than a problem to solve: it has one callback outstanding and sleeps, and the callback
arrives when the surface is composited again. The rule that has to hold is *exactly one
outstanding per window*; a watchdog that asked for another on every hiccup would leave the
window swapping once per request per compositor frame for ever. So the request is tracked, the
thread blocks with no timeout while every window is paced or hidden, and the 50 ms watchdog is
only a net under a window with **no** callback outstanding that has somehow not painted.

## Measured

On Linux — KWin on Wayland, a 100 Hz editor display — with the Pumpkin project open and
the mix on deck A driven by a running oscillator.

**The editor alone minimized for a full minute, the mix popped out:**

| | |
| --- | --- |
| two captures of the picture window, 10 s apart | **45.7% of pixels differ** — it never stopped |
| `pictures` | 506–538 wakeups/s, **1.6–1.8% of a core** |
| `synth` | 103–110 wakeups/s (the 100 Hz tick), 2.6–2.9% |
| frame thread | ~610 wakeups/s, **0.3% of a core** — it paints nothing |
| `late`, after restoring | **0** |

**The picture window alone minimized, the editor up** — the window nobody can see must stop:

| | pictures thread |
| --- | --- |
| both windows visible | 607 wakeups/s, **2.0% of a core** |
| picture window minimized, editor up | 394–463 wakeups/s, **0.3–0.5%** |
| **both** minimized | **0 wakeups/s, 0.0%** |

The CPU is the figure that answers the question: painting stops the moment the compositor
stops sending that surface's frame callbacks, and the thread then blocks with no timeout.
The wakeups that remain in the middle row are **not** the picture window: one `wl_display`
means libwayland wakes every thread reading the socket, so each thread wakes on the other's
traffic and goes straight back to sleep. With both windows minimized there is no traffic and
both go to zero — which is what the third row proves, and what the frame thread's own ~610
wakeups at 0.3% in the first table are. That is the price of the borrowed display, and it is
a wakeup with no work in it.

**Quitting with two picture windows open and one of them fullscreen**: the process exits
cleanly, **zero bytes on stderr** — no Wayland protocol error, which libwayland aborts loudly
on — and no surface left behind. `App::on_exit` stops and joins the pictures thread first, so
its surfaces and contexts are gone while eframe's `wl_display` and the synth's context are
both still alive.

Fullscreen was driven end to end through the mark — `Ask::Fullscreen` → `set_fullscreen` →
1424x1012 becoming 3440x1440 and back — and so were open and close. **The drag, the resize
bands, the cursor and the two keys are implemented and were not driven**, because the egui
inspection server can only reach the editor's own window and Wayland has no synthetic input
for the rest; they are `xdg_toplevel.move`/`resize` behind a travel threshold, a
`cursor-shape-v1` request, and a raw `wl_keyboard`, and they want a hand on them before they
are believed.
