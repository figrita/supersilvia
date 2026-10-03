# Proposal: picture windows on macOS

**Status: built.** [docs/rendering.md](../docs/rendering.md#on-macos-and-windows),
[docs/ui.md](../docs/ui.md) and [docs/decisions.md](../docs/decisions.md) describe the system;
what is kept here is the argument, what the source settled, and what is still to be checked by
hand. Before it, every pop-out and fullscreen mark on the Mac was refused by
`src/render/picture/macos.rs`, with the toast "picture windows are not written for macOS". This is the windows lane of step 9 in [wgpu.md](wgpu.md#the-steps). It was approved with three answers: `winit` is declared for macOS,
fullscreen is instant and in place, and a picture window casts no shadow.

**The route: W1.** Each picture window is a winit window made inside eframe's
own event loop, on the main thread. Each one is drawn on a thread of its own, on the one
device. It needs no `unsafe` and one new line in `Cargo.toml`, for a crate that is already
compiled into the app. A build-only spike on this Mac compiled the whole shape under
`#![forbid(unsafe_code)]`.

## What you will see and do

Everything a picture window does on Linux, it does on the Mac.

- **Open.** An Output's pop-out mark opens a window with no title bar and no frame. It shows
  only the picture, letterboxed on black. It opens in front of the editor, at the picture's own
  size, capped to three quarters of the largest display. The mark lights while it is open, and
  clicking it again closes the window.
- **Move.** Press on the picture and drag. A click that does not travel four points moves
  nothing.
- **Resize.** A band twelve points wide runs along every edge and corner. Over it the cursor
  becomes the matching resize arrow. Dragging there resizes from that edge or corner.
- **Keep the shape.** Hold `Shift` or `Ctrl` while resizing and the picture's aspect is kept.
- **Fullscreen.** `F` or a double-click toggles it. The fullscreen mark opens a window already
  fullscreen. Fullscreen covers the screen at once, in place, with no animation (decision 1).
- **Leave.** `Escape` leaves fullscreen. `Escape` again closes the window. A window the
  fullscreen mark opened closes on the first `Escape`, as on Linux.
- **The editor can go away.** Minimize the editor and every picture window keeps moving. This
  is the reason picture windows exist.
- **A hidden window rests.** A picture window that is minimized, or fully covered, or on a Space
  you are not looking at, stops painting. Its thread sleeps until it can be seen again.
- **The pointer reads.** A `mouseinput` node set to *Picture* reads the hand over the picture,
  as it does on Linux.
- **The bookkeeping is the same.** Deleting a node closes its window. Opening another project
  closes every node's window and keeps the mix's.

## The route

Two routes were on the table.

- **W1: winit windows inside eframe's own event loop.** `eframe::create_native` hands back
  eframe as an `EframeWinitApplication`, and we run winit's loop ourselves around it. Our
  wrapper forwards every event to eframe except those for our own windows.
- **W2: AppKit directly, through `objc2`.** A borderless `NSWindow` with a view subclass of
  our own, a `CAMetalLayer` handed to wgpu as a raw layer, and a `CADisplayLink` per window.

### What each costs

| | W1: winit in eframe's loop | W2: AppKit through objc2 |
| --- | --- | --- |
| **`unsafe`** | none — the spike compiles under `forbid(unsafe_code)` | a view subclass (`define_class!` with `unsafe impl` blocks for the event overrides), a display-link target class, `create_surface_unsafe` over the raw layer |
| **new dependencies** | `winit`, macOS only, already in the lock at 0.30.13; declaring it adds no package | `objc2`, `objc2-app-kit`, `objc2-foundation`, `objc2-quartz-core`, all in the lock, but with AppKit and QuartzCore features the app does not compile today |
| **new code** | the main-thread half and a drawing thread, about the size of `thread.rs`'s window logic without the Wayland protocol | the same, plus the Objective-C classes, the display link on a run loop of our own, and the event plumbing winit already does |
| **`main.rs`** | changes: eframe is started through `create_native` on macOS | unchanged: `App::ui` runs on the main thread and could make an `NSWindow` there directly |
| **resize** | our own 12-point band and aspect lock, in safe Rust (below) | AppKit's native edge resize and `contentAspectRatio` |
| **fullscreen** | both kinds are one winit call each | both, and a window level above the menu bar |
| **pacing** | `Fifo`, one thread per window, parked on occlusion | `CADisplayLink`, macOS 14 or later |
| **risk** | winit's AppKit layer is well used; the new parts are ours and safe | every Objective-C override is ours to get right, and a mistake is undefined behaviour |
| **what Linux could share** | the whole shape: winit windows, a wgpu surface each, a thread each. Linux could later drop its Wayland code for it | nothing |

### Why W1

The first plan leaned to W1, and the source confirms it. W1 is the simpler path on every count
that matters. It needs no `unsafe`, so the invariant that `unsafe` lives only in
`render::picture` and `render::dmabuf` needs no widening. It adds no package. It is the shape
[platform.md](platform.md#what-renders-rewrite-needs-from-here) calls portable.

W2's real advantages are three, and none is needed. Native edge resize is replaced by a band of
our own that matches Linux exactly. The display link is replaced by `Fifo`, which needs no
macOS 14. A window level above the menu bar is the one thing W1 cannot do; decision 1 says what
that costs on screen.

## How it fits together

### `main.rs`, and Linux unchanged

`main.rs` stops calling `eframe::run_native` directly. It calls
`supersilvia::render::picture::run` with the same three arguments.

- **On Linux, `run` is `eframe::run_native("supersilvia", options, creator)`, and nothing
  else.** Linux starts eframe exactly as it does today. That is the guarantee, and it holds by
  reading one three-line function.
- **On macOS, `run`** builds `EventLoop::<eframe::UserEvent>::with_user_event()`, calls
  `eframe::create_native` on it, wraps the result in `picture::Loop`, and calls `run_app`.

This is the same loop the editor runs in today. On macOS winit's `run` is `run_on_demand`
(`winit-0.30.13/src/platform_impl/macos/event_loop.rs:271`). eframe's `run_native` with the
default `run_and_return: true` also ends in `run_app_on_demand`, with the same
`WinitAppWrapper` flag that `create_native` uses (`eframe-0.36.1/src/native/run.rs:374`, `:448`,
`:471`). So the editor's own life on the Mac, including Cmd-Q and the unsaved-changes question
on close, does not change.

`main.rs` names no winit type. It keeps `#![forbid(unsafe_code)]`.

### The main thread: `picture::Loop`

`Loop` owns eframe's `EframeWinitApplication` and every picture window's
`Arc<winit::window::Window>`. It implements winit's `ApplicationHandler` and forwards all nine
of its methods to eframe. A window event for a window id that is ours is handled and not
forwarded. eframe never sees our windows.

**An `Ask` needs no wake-up.** Every `Ask` comes from a mark, so it is sent from inside
`App::ui`. On macOS `App::ui` runs inside winit's `RedrawRequested`. winit then calls
`about_to_wait` in the same turn of the loop (`macos/app_state.rs:384–392`). `Loop` drains the
asks there, so a window opens on the same turn as the click.

An `EventLoopProxy` is not needed, and it would not help. `create_proxy` exists, but its event
type is eframe's `UserEvent`, a closed enum of two variants
(`eframe-0.36.1/src/native/winit_integration.rs:64`). It cannot carry an `Ask`. eframe's own
"current event loop" slot, which `App::ui` might have used to make a window directly, is
private (`native/mod.rs:3`).

**How the two halves meet.** `run` makes the two channels before eframe starts. It keeps the
loop's ends in `Loop` and parks the editor's ends in a main-thread slot. `Host::for_eframe`
takes them from the slot inside `App::new`, which eframe runs on the main thread. It sends the
loop one first message carrying a clone of the `Gpu`, the synth's `Arc<Live>` and the
`Arc<pointer::Feed>`. `App::new` keeps its signature, so `tests/ui.rs` and the examples do not
change. A harness has no `Loop` and no slot, and its `Host` is detached, as it is today.

**`Told` goes back on a plain channel.** `Host::told` drains it once a frame in
`poll_pictures`, as on Linux. A window closed by `Escape` while the editor is minimized is
reported when the editor paints again.

### Opening a window

On the main thread, in `about_to_wait`:

1. `event_loop.create_window` with no decorations, not resizable (the band is ours), a minimum
   of 160×90, and the size capped from `available_monitors()`.
2. `instance.create_surface(Arc::clone(&window))`. This is safe, and it must happen here. winit
   gives a window handle only on the main thread (`macos/window.rs:55–63`).
   `raw_window_metal::Layer::from_ns_view`, which wgpu-hal calls, panics anywhere else
   (`raw-window-metal-1.1.0/src/lib.rs:373`).
3. The surface is checked with `adapter.is_surface_supported`. A failure is
   `Told::Failed(picture, why)` and a toast.
4. A thread named `picture` is started for the window, and the surface moves to it.
   `wgpu::Surface<'static>` is `Send`; the spike asserts it.
5. A window opened by the fullscreen mark is made fullscreen now.

### Drawing: a thread per window

Each window's thread owns its surface and its configuration. It shares one `Viewer` per surface
format with the other window threads. The `Viewer`'s pipeline is made on a window thread,
never on the main thread, which is the editor's frame thread on a Mac. The surface format is
the non-sRGB `Bgra8Unorm`, as on Linux.

The loop is the Linux paint without the frame callback. It takes the newest `Published` from
`Live`, calls `get_current_texture`, clears to black and blits with `Fit::Letterbox`, submits
through `Gpu::submit`, presents, and then lets the `Published` go. Nothing waits on the synth.

Between frames the thread reads its own channel without blocking. The main thread sends it
three things:

- **`Resized`**, from winit's `Resized` and `ScaleFactorChanged`. The thread configures the
  surface again at the new physical size. The `CAMetalLayer` follows the view's bounds and scale
  by itself (raw-window-metal's observer layer); `configure` sets the drawable size.
- **`Occluded`**, from winit's `WindowEvent::Occluded`, which comes from AppKit's occlusion
  notification (`macos/window_delegate.rs:349–352`).
- **`Close`**.

### A hidden window stops painting

While its window is occluded, the thread blocks on its channel with no timeout. The main
thread's `Occluded(false)`, a resize or a close wakes it. A hidden window then costs no
wake-ups.

wgpu has its own occlusion check. On macOS, `acquire_texture` returns `Occluded` at once for an
occluded window rather than waiting for a drawable (`wgpu-hal-30.0.1/src/metal/surface.rs:347–372`).
The thread treats that answer the same way: it sleeps on its channel. That sleep has a
quarter-second timeout, in case wgpu saw the occlusion before AppKit sent winit its event.

### Pointer input

Pointer events arrive on the main thread, in `Loop`. `CursorMoved`, the two buttons and
`CursorLeft` go into the same `crate::pointer::Feed`, through the same `pointer::place` against
the picture's letterboxed rect, as `thread.rs::report` does. `Feed` is a mutex; the main thread
writes it as safely as the pictures thread does on Linux.

### Closing, and quitting

**The main thread holds every window until the thread that draws it has ended.** This rule
avoids a deadlock. A winit window dropped off the main thread closes itself by waiting on the
main thread (`macos/window.rs:19–23`). If a drawing thread dropped the last `Arc<Window>` while
the main thread waited for it, neither would move.

**Closing ends the thread first, and the window stays up until it has.** Closing sends the
window's thread `Close` and waits up to a tenth of a second for it to end, then drops the
window. A visible window's thread is at most a refresh from its next present, so it ends well
inside that. A thread that has not ended is waiting for a drawable, which a hidden window may
never be given. Its window is hidden at once and held in a retiring list, which `about_to_wait`
empties when the thread ends. Hiding the window first would have been the simpler order, and it
is the one that could strand every closing thread in its acquire.

**Quitting.** `App::on_exit` calls `Host::stop`, which only lets go of the loop's channel.
`Loop::exiting` then runs after eframe's own exit and closes every window the same way. A
window whose thread still has not ended is let go of without being dropped, so that thread never
holds the last of it, and the process ending takes both. On Linux, `on_exit` keeps its order,
which exists for the borrowed `wl_display`.

## Pacing, and the present mode

Metal offers two present modes, `Fifo` and `Immediate`. wgpu-hal treats any other as
unreachable (`metal/surface.rs:260–264`). `Mailbox`, which Linux picks first, does not exist
there.

**Each window presents in `Fifo`, on its own thread.** `Fifo` turns on the layer's display
sync, so presents follow the display the window is on. With `desired_maximum_frame_latency: 1`
the layer keeps two drawables (`surface.rs:330`). The thread blocks in `get_current_texture`
until one is free. So the window paints once per refresh of its own display, and a picture is
at most one refresh behind.

**Why a thread per window, and not one thread.** A `Fifo` acquire blocks. On one thread, a
window on a 60 Hz projector would hold back a window on the 120 Hz laptop screen. With a thread
each, every window keeps its own display's rate and cannot observe any other.

**Why not a display link.** `CADisplayLink` from a view needs macOS 14, an Objective-C target
class and a run loop on the drawing thread. That is W2's `unsafe`, for pacing `Fifo` already
gives. Choosing `Fifo` also means picture windows set no minimum macOS version. That takes one
feature off the list behind the 14.2 minimum.

## The hand: move, resize, keys

**Move is ours.** A press in the middle of the window is remembered. When the pointer has
travelled `DRAG_SLOP` (4 points), every motion sets the window's origin from where the press
landed. Waiting for the slop keeps the second press of a double-click, as on Linux.

*What lost:* winit's `drag_window`, which is `performWindowDragWithEvent:` with the current
event (`window_delegate.rs:1170`). Apple documents that call for a mouse-down. Called on the
press, it would take the second press of a double-click with it. Called on the first drag past
the slop, whether AppKit honours it is unverified, and a window that silently did not move
would be the failure. Our own move works either way, and is the same code as the resize.

**Resize is ours.** winit's `drag_resize_window` returns `NotSupported` on macOS
(`window_delegate.rs:1179`). A press in a band starts our own resize. Each `CursorMoved` then
sets the window's frame with `request_inner_size` and then `set_outer_position`: AppKit keeps a
window's bottom-left corner when its size changes, and the origin set after puts the top-left
where it belongs. AppKit keeps
sending `mouseDragged` to the window while a button is down, even outside it; winit passes those
on (`view.rs:1061–1076`). The arithmetic is one pure function beside `edge_at` in
`picture/mod.rs`, `dragged`: the starting frame, the pointer's travel and the edge give the new
frame, and no edge moves the whole window. With `Shift` or `Ctrl` held it goes through
`fit_aspect`, anchored on the dragged edge, and a locked size shrunk past the minimum grows back
to it at its aspect. The
modifiers come from winit's `ModifiersChanged`, by name, with no keymap bit convention.

The window is made not resizable. winit's borderless style mask includes `Resizable` by
default (`window_delegate.rs:535–555`). Turning it off keeps AppKit's own edge resize, if it
gives one, from taking presses meant for our band.

**The cursor.** Over a band, `set_cursor` shows AppKit's own two-headed resize cursor for that
edge or corner, which winit names (`macos/cursor.rs:203–217`).

**Keys by position.** winit's `KeyCode::KeyF` is macOS key code `0x03` and `KeyCode::Escape`
is `0x35` (`macos/event.rs:480`, `:530`). Both are physical positions, as the evdev codes are on
Linux. With no IME, winit sends one `KeyboardInput` per `keyDown:`; its `cancelOperation:`
(`view.rs:551`) is for Cmd-period. winit's window class can become the key window, so a
borderless window takes the keyboard (`macos/window.rs:108–119`).

**Double-click.** winit reports no click count. Two left presses within 300 ms on one window
are a double-click, as on Linux.

## `unsafe`

**There is none on the Mac.** `render/picture/macos/` stays under the crate root's
`deny(unsafe_code)`. The invariants row and CONTRIBUTING.md's hard rule already say `unsafe` is
allowed on the Linux half of `render::picture` only, and the rule's "why" cell says the
borrowed display is Linux's and the macOS half has none.

## New dependencies

One, approved:

- **`winit` 0.30, macOS only, with default features off and `rwh_06` on.** For naming the event
  loop, the window, the window events and `KeyCode`, which eframe does not re-export. It is
  already compiled into the app through eframe, at 0.30.13, with `rwh_06` already on. Declaring
  it adds no package to `Cargo.lock`; the spike's lock gained only the spike itself.

Nothing else. No `objc2` crate is declared. No crate is added on Linux.

## Decisions

**1. Fullscreen is instant, in place.** `F`, a double-click or the fullscreen mark covers the
screen the window is on at once, with no animation and no new Space. The editor stays one click
away in the same Space. It is `set_simple_fullscreen(true)`, on a window made with
`with_borderless_game(true)`, so the menu bar and the Dock are hidden outright rather than on
hover (`window_delegate.rs:1734–1781`). They are hidden while supersilvia is the app in front.
**(unverified)** When another app is in front, macOS brings them back over the top of the
picture. A projector wants a picture that appears where it is, at once.

*What lost:* **a Space of its own**, macOS's native fullscreen,
`set_fullscreen(Some(Fullscreen::Borderless(None)))`, which is `toggleFullScreen:`
(`window_delegate.rs:1418`). It slides into a new Space with the animation, keeps the menu bar
hidden whatever app is in front, and puts the editor a swipe away. With *Displays have separate
Spaces* off it blanks every other display. *Also lost:* keeping the menu bar off the picture
while another app is in front, which needs a window level above the menu bar and so W2.

**2. No shadow.** A picture window casts none, as on Linux, so the edge of the picture is the
edge of the window. It is made with `with_has_shadow(false)`.

*What lost:* the soft shadow a borderless Mac window casts by default.

### Other Mac behaviours you will notice

These need no decision. They are how macOS works.

- **Cmd-H hides every supersilvia window**, picture windows included. Bare `H` still hides
  only the editor's canvas.
- **All windows share supersilvia's one Dock icon.** Linux's separate pop-out and fullscreen
  icons come from Wayland app ids, which a Mac does not have.
- **The first click on a picture window both brings it forward and starts a drag**, because
  winit's windows accept the first mouse by default.
- **The double-click speed is fixed at 300 ms**, not taken from System Settings.
- **A picture window in a Space you are not looking at counts as hidden** and stops painting
  until you look at it.

## Tests

### What stays automated

`tests/picture_windows.rs` stays as it is and must pass: the `Wall`, `edge_at`, `fit_aspect`
and `escaped`. It gains a table for the new resize function: every edge and corner, with and
without the lock, and the minimum size. `tests/ui.rs`'s pop-out test runs detached, as today.
`./check.sh` builds and lints the macOS half on the Mac, and `cargo test` never opens a window.

### By hand

The egui MCP cannot drive a second window, so this list is checked by hand. Each step says
what should be seen.

1. The editor opens, works, and quits with Cmd-Q. Closing it with unsaved changes still asks.
2. An Output's pop-out mark opens a borderless window in front of the editor, moving. The mark
   is lit. Clicking the mark again closes the window.
3. Drag the picture: the window follows the pointer, including onto another display. A short
   click does not move it.
4. Move the pointer over each edge and corner: the cursor changes. Drag each one: the window
   resizes from that edge.
5. Resize with `Shift` held, then with `Ctrl`: the window keeps the shape it opened at, so the
   picture shows no black bars. Let go of the key mid-drag: the drag goes on unlocked.
6. `F`: the window covers its screen at once, with no animation, and the menu bar and Dock
   are gone. `F` again: back to the pop-out where it was. A double-click does the same.
7. From fullscreen, `Escape`: back to the pop-out. `Escape` again: it closes.
8. The fullscreen mark: the window opens fullscreen. One `Escape` closes it, and the menu bar
   and Dock come back.
9. Drag a pop-out to another display, especially one of a different scale: the picture stays
   sharp and fills the window.
10. Minimize the editor: the pop-out keeps moving.
11. Minimize or cover the pop-out, with the editor up: supersilvia's CPU in Activity Monitor
    drops. `top -pid $(pgrep supersilvia)` shows it too.
12. A `mouseinput` node set to *Picture* reads the pointer over the pop-out, and nothing over
    the black bars.
13. Delete the Output: its window closes. Open another project: node windows close, the mix's
    stays.
14. Quit with two picture windows open, one fullscreen: the app exits at once. No new report
    appears in `~/Library/Logs/DiagnosticReports/`.

## The build, in commits

Three commits, about fourteen files in all. Each is green on `./check.sh` on the Mac and on
Linux, and each carries its own doc change.

1. **The loop, and no windows yet.** `render::picture::run`, Linux's body `eframe::run_native`
   and macOS's `create_native` inside `Loop`, which forwards everything. `winit` declared for
   macOS. The marks are still refused. What you check: step 1 of the list.
   Files: `Cargo.toml`, `Cargo.lock` (one edge), `src/main.rs`, `src/render/picture/mod.rs`,
   `src/render/picture/macos.rs` becoming `macos/mod.rs` and `macos/event_loop.rs`,
   `docs/architecture.md` (the `render/` row names winit on macOS).
2. **Windows open, paint, go fullscreen and close.** The slot, the ask and told channels, window
   creation, the surface on the main thread, the drawing thread with `Fifo`, occlusion parking,
   the retiring list, `F`, `Escape`, instant fullscreen, quitting. Steps 2 and 6–11 and
   14.
   Files: `src/render/picture/macos/{mod,event_loop,draw}.rs`, `src/render/picture/mod.rs`
   (module doc), `src/app/frame.rs` (the `on_exit` comment says why the order is Linux's),
   `docs/rendering.md` ("Picture windows", "Every window is a viewer", "One device"),
   `docs/ui.md` (the pop-out section), `docs/architecture.md` ("The pictures, on a thread of
   their own"), `docs/decisions.md` (W2 lost, one line), `CONTRIBUTING.md` (the hard rule's two
   words).
3. **The hand.** The move behind the slop, the band and its cursor, our own resize with the
   lock, the double-click, the pointer into `Feed`. Steps 3–5, 12 and 13.
   Files: `src/render/picture/mod.rs` (the resize function), `src/render/picture/macos/event_loop.rs`,
   `tests/picture_windows.rs`, `src/pointer.rs` (its module doc names both sources),
   `docs/ui.md`, this proposal's status line, and the windows rows of
   [wgpu.md](wgpu.md#what-the-platform-layer-must-provide-per-os) and
   [platform.md](platform.md#what-renders-rewrite-needs-from-here).

## Risks and open questions

- **Our own move and resize may lag the pointer. (unverified)** Each motion is up to two
  AppKit calls, a new size and a new origin, on the main thread, which also runs the editor's
  frame. If it looks rough, the fallback for the resize is AppKit's native edge resize, by
  making the window resizable; that loses the 12-point band and the lock, and whether AppKit
  resizes a borderless window from its edges at all is also unverified. Our own move does not
  snap to screen edges as AppKit's drag does.
- **An acquire that never returns. (unverified)** wgpu sets `allowsNextDrawableTimeout` to
  false (`surface.rs:334`), so a `Fifo` acquire can wait with no limit. If a window becomes
  occluded between wgpu's check and the acquire, its thread may wait until it is visible again.
  It is contained: that window stays stuck, not the others; closing it hides it after a tenth
  of a second and does not wait for the thread; quitting does not wait for it either. If it
  shows up, the fallback is `Immediate` with a sleep of one refresh interval.
- **Two windows fullscreen at once. (unverified)** winit saves and restores the app's
  presentation options per window. Leaving fullscreen on one while another stays fullscreen may
  bring the menu bar and the Dock back over the second.
- **wgpu reads `NSWindow.occlusionState` from our drawing thread** (`surface.rs:366`). AppKit
  properties are main-thread-only on paper, and Xcode's Main Thread Checker would flag it. It
  is wgpu's own workaround and the editor's surface takes the same path. **(unverified)** that
  it does any harm.
- **`configure` sets layer properties off the main thread.** Core Animation allows it. It may
  log "uncommitted CATransaction" warnings on a thread with no run loop. **(unverified)** It is
  noise at worst.
- **Two drawables and full rate. (unverified)** Whether a latency of one keeps a 120 Hz display
  at 120. If not, a latency of two, as Linux uses.
- **The menu bar in fullscreen. (unverified)** Whether it returns over the picture when another
  app is in front, and whether it hides on a second display at all.
- **A new eframe or winit.** `Loop` forwards all nine `ApplicationHandler` methods. The trait
  gives each a default, so a method added upstream and not forwarded would still compile. An
  upgrade must re-read the trait.
- **Linux on the same shape later.** W1 is what Linux could adopt, deleting its Wayland code and
  its `unsafe`. Whether a hidden Wayland window's `Fifo` acquire blocks in Mesa is unknown. That
  is a separate proposal, not this one.

## What the source settled

The first plan marked these **(unverified)**. Each is now read in the crate the lock holds.

| question | answer | where |
| --- | --- | --- |
| `create_native` and `EframeWinitApplication` | exist, public; eframe runs as an `ApplicationHandler<UserEvent>` inside a loop we build | `eframe-0.36.1/src/lib.rs:376`, `native/run.rs:480` |
| `EventLoopProxy` user events through eframe | a proxy can be made, but `UserEvent` is eframe's closed enum; not needed, since `RedrawRequested` is followed by `about_to_wait` in the same turn | `winit_integration.rs:64`; `winit-0.30.13/…/macos/app_state.rs:384–392` |
| borderless and resizable | `with_decorations(false)` gives `Borderless \| Resizable \| Miniaturizable`; whether AppKit then resizes from the edges is still **(unverified)** and no longer matters | `macos/window_delegate.rs:535–555` |
| `drag_window` | `performWindowDragWithEvent:` with the current event | `window_delegate.rs:1170` |
| `drag_resize_window` | `NotSupported` on macOS | `window_delegate.rs:1179` |
| `Occluded` | sent from AppKit's occlusion notification | `window_delegate.rs:349–352` |
| `Fullscreen::Borderless` | `toggleFullScreen:`, a native Space with the animation | `window_delegate.rs:1301`, `:1418` |
| `set_simple_fullscreen` | the screen's frame, no animation; menu bar and Dock auto-hidden, or fully hidden with `borderless_game` | `window_delegate.rs:1734–1781` |
| `KeyCode::KeyF`, `Escape` | physical key codes `0x03` and `0x35` | `macos/event.rs:480`, `:530` |
| `create_surface(Arc<Window>)` | safe; main thread only | `wgpu-30.0.1/src/api/instance.rs:184`; `macos/window.rs:55–63`; `raw-window-metal-1.1.0/src/lib.rs:373` |
| `configure` and `present` off the main thread | no main-thread demand; `present` commits a command buffer with `presentDrawable` | `wgpu-hal-30.0.1/src/metal/surface.rs:248–341`; `metal/mod.rs:802–826` |
| present modes | `Fifo` and `Immediate`; anything else is unreachable | `surface.rs:260–264`; `metal/adapter.rs:464` |
| `CAMetalLayer` sizing | raw-window-metal's sublayer tracks the view's bounds and scale; wgpu reads them atomically; `configure` sets the drawable size | `raw-window-metal-1.1.0/src/lib.rs:360–370`; `surface.rs:193` |
| `objc2` crates in the lock | `objc2` 0.5.2 (winit, accesskit) and 0.6.4 (eframe, arboard, wgpu-hal, cpal); `objc2-app-kit`, `-foundation`, `-quartz-core`, `-metal` at 0.2.2 and 0.3.2; `objc2-core-foundation` 0.3.2; `block2` 0.5.1 and 0.6.2; `dispatch2` 0.3.1 | `Cargo.lock`, `cargo tree -i` |

**The spike** was `windows-spike`, a scratch crate outside the repository, built and never run. It
builds eframe through `create_native` around a forwarding `ApplicationHandler`, makes a
borderless window with `with_has_shadow(false)` and `with_borderless_game(true)`, makes its
surface from `Arc<Window>`, and moves the surface to a thread that configures it in `Fifo`,
acquires, submits and presents. It uses `drag_window`, `set_cursor`, `set_outer_position`,
`request_inner_size`, both fullscreens, `ModifiersChanged` and the two `KeyCode`s. It compiles
under `#![forbid(unsafe_code)]` against the lock's eframe 0.36.1, winit 0.30.13 and wgpu 30.0.1.
