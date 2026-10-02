# Proposal: frame pacing, and a clock that does not judder

**Status: the judder is found, measured and fixed.** `pre_present_notify` is out, the
traces plot by time and the Status box reads the display's own rate; what is left here is
[the bench](#the-plan-in-order). The one case pacing cannot reach — a minimized editor — is
[deterministic-loop.md](deterministic-loop.md), reopened with its shape chosen. Read [the
second finding](#the-second-finding-pre_present_notify-is-the-judder) before anything else
here.

The goal: *drop zero frames on simple patches, one hundred per cent of the time*, and have a
CPU oscillator be a perfect sine while doing it.

This records what the words mean once measured, what is already true, and what each remaining
step would cost — so the order is argued before any of it is built.

## What is already true

**There is no capacity problem.** On a 3440x1440 patch with an oscillator, two checkerboards,
a blur and two Outputs: GPU 0.4–0.9 ms per Output, CPU 0.6–2.0 ms, against a ~10 ms frame.
About six per cent of the frame is in use. Nothing here is an optimization problem.

**Nothing is dropping.** With `pre_present_notify` in place and nothing else running, no drops
of any cause at all — see [the first finding](#the-first-finding-they-are-not-the-same-problem)
for what *is* happening.

So the work is not about making anything faster. It is about **timing and scheduling**, and
about the two places the editor currently lies to itself.

A caution about the numbers in this file, learned the hard way while taking them: an early
reading said `late 0` of 600 and was quoted here as a good baseline. It was measured when
*late* still meant one and a half times a hardcoded 60 Hz — twenty-five milliseconds, on a
display whose frames are ten. Nothing was ever going to reach it. Every figure here is under
the definition in `clock.rs` **as it stands**, and a figure taken under an older one is worth
nothing at all.

## What a "dropped frame" actually is

`render/output.rs` polls the previous frame's fence with a zero timeout before drawing an
Output again. Not signaled, and the Output reuses its published frame and counts a drop.

That is **GPU back-pressure**, not a missed vsync and not a stutter anyone saw. On a screen
showing a slowly-changing patch, one reused frame is usually invisible. It is also a strict
test: a driver that has batched the work may not have signaled yet even though the frame was
cheap.

So "drop zero frames" is not quite the goal it sounds like. `DropCause` now separates them:

| cause | what it means |
| --- | --- |
| `capture` | a render is reading every frame back at full size. **Expected** — a render is allowed to cost frames |
| `linking` | a new program is still linking. The cost of an *edit*, not of the patch |
| `thumbnail` | a save or a card is reading a picture back |
| `tap` | a measurement buffer had not been read |
| `busy` | nothing in flight but the frame itself. **The only one worth chasing** |

A target worth holding to is *zero `busy` drops in steady state*, with the rest explained by
what caused them.

## What a "perfect sine" actually is

The accumulator is already exact as a function: `elapsed` is unclamped and monotonic, and a
`∫ speed·dt` whose `dt`s sum to elapsed time gives the right phase at every sample. **What
jitters is when it is read, not the wave.**

This is why a thread does not fix it. Put the accumulator on a 1 kHz thread and the sequence
is beautiful — and the frame still samples it at whatever irregular instant the frame happens,
which is the same judder with two clocks to reconcile.

The trace that showed the problem was an honest picture of it: one sample per tick, plotted
at even spacing. Unevenly-*timed* samples of a perfect wave, drawn evenly, look exactly like a
wave with the wrong shape. **The trace now plots by time** — each sample dated, drawn where
its date falls — so a late frame is a wider gap and the wave on the node is the wave. What is
left for the clock to decide is only whether the wave's own phase should be smooth or true,
below.

## Open question: smooth, or true?

Two clocks are possible and they disagree under exactly the conditions that produced the
screenshot — a compositor delaying frames while a window opens over the editor.

| | tracks wall time | smooth on screen |
| --- | --- | --- |
| **advance by measured `dt`** — today | exactly | judders when frame arrival jitters |
| **advance by the display's cadence** | drifts during a stall, corrected after | smooth |

During a stall the second slows the wave and then catches it up, rather than tearing it.
Smoother, but for a beat you are not where wall time says you are. That is nothing at all for
a shape breathing on a wall, and it is everything for an oscillator locked to a MIDI clock or
an audio track.

**The likely answer is both, with the node choosing** — but that is a decision, not a default,
and it wants taking before the clock is written.

### `MAX_DT` is the same decision, already taken once

`dt` is clamped to 100 ms "for the sake of things that integrate it". That clamp *is* the
smooth-over-true choice: it stops the oscillator jumping forward after a stall. What is
genuinely wrong is that the drift it causes is **permanent and uncorrected** — the accumulator
never catches up, ever. A slow correction gives both properties with no jump, and belongs in
the same change.

## The steps, in the order they are worth taking

### 1. `pre_present_notify` — built, measured, and to come out

One line at the end of `App::ui`, through the `eframe::Frame` that was already in the
signature and unused. On Wayland it schedules the surface's frame callback, which is how a
client asks to be woken when a frame is wanted rather than painting into the dark and letting
`swap_buffers` block. Nothing in eframe calls it.

`Frame::winit_window()` is public, so this needed no fork — which is worth recording, because
it was assumed to need one.

**It was the wrong arrangement, and the measurement says so** — see [the second
finding](#the-second-finding-pre_present_notify-is-the-judder). With the line in, winit holds
every redraw until the compositor's frame callback and eframe spins the event loop at 100 %
of a core while it waits; with the line out, Mesa's own swap wait paces the frame, the thread
sleeps, and a window flapping over the editor costs one late frame in six hundred instead of
two hundred and thirty-seven.

### 2. The two measurements — built

**`Clock::pacing`** keeps the last 600 frame intervals, unclamped, and reports p50, p99, worst
and a count of *late* ones. A mean hides a stall and a single worst hides how often; only a
distribution answers "how consistent is this".

**`DropCause`** gives every drop a reason, from what was in flight when the fence had not
signaled. A count with no cause cannot be acted on.

Both are on the Status box, which is off by default.

### 3. The clock — proposed, wants the decision above

Advance a presentation clock by one display interval per frame and correct it slowly toward
the wall clock, rather than by however long the last frame took. About thirty lines in
`clock.rs`. Fixes the judder whether or not step 1 helped, and settles `MAX_DT` with it.

**Do not write it before looking at a `pacing` reading taken while something is flapping over
the window.** The design depends on what that distribution looks like: a handful of long
intervals is a different problem from a sustained wobble.

That reading has now been taken, with the line from step 1 out: one late frame in six hundred
under a flapping window, p99 12.2 ms on a 10 ms display. The judder the oscillator's trace
showed was step 1's, not the clock's. This step may be unnecessary; look at the trace again
before writing it.

### 4. Read the display's real rate — proposed, small

`VSYNC_MS` is hardcoded to 1000/60. This machine's display is 100 Hz, so the Status box's
`of 16.67 ms at 60 Hz` is wrong, the cost strip's bar is scaled against the wrong budget, and
a frame that missed three refreshes would read as on time. The pacing line already sidesteps
it by measuring *late* against the median rather than a constant, which is a workaround and
not a fix.

### 5. `wp_presentation` — not needed

The one thing that gives smooth *and* true with no trade: the real time each frame hit the
screen, so the next is computed for `last_presentation + interval`.

winit has no presentation API — `pre_present_notify` is throttling, not a timestamp — but it
already depends on `wayland-protocols` and `smithay-client-toolkit`, so the protocol is
compiled into the binary. This is **a missing API surface, not a missing dependency**. Getting
the `wl_display` and `wl_surface` out through `raw-window-handle` and running a small second
Wayland queue is fiddly and is not a windowing backend.

### 6. KMS, and realtime scheduling with it — not this

**Read [deterministic-loop.md](deterministic-loop.md) with this one.** Owning the GL context
is what a render loop of our own needs, whether or not KMS follows — so this stopped being an
exotic last resort for frame timing and became the same piece of work as the requirement
itself.

Own the page flip, no compositor scheduling, nothing else on the GPU. With `SCHED_FIFO` on the
frame thread, no allocation in the frame path and locked pages, misses become a bug rather
than a fact of life. The eventual right shape for a box that goes to a gig.

It is a second windowing path: input, viewports, the projector and the file dialogs all need
an answer. **Not before 1–5, and not before a measurement says the compositor is the cause.**

### 7. A pacing bench — proposed, and the procedure exists

A fixed patch, N seconds, reporting the distribution. `examples/graph_bench` measures graph
cost by size; nothing measures pacing over time. Without it, every judgement about steps 3–6
is a comparison of screenshots taken while the compositor was doing different things.

Headless is the wrong shape for it, since the compositor is the thing under test. The
procedure that produced the second finding is the bench, and it wants scripting rather than
redoing by hand: an env var that prints `Clock::pacing` and the drop counters to stderr every
600 frames, so a reading costs no inspection traffic, and a script that launches the app,
lets it settle, and applies each load in turn — sixteen busy threads, a host `kdialog` held
over the window, the same dialog flapped every half second — printing one row per condition.

**It reads the Status box's TICK block, not FRAME.** The synth keeps its own time on a thread
of its own, so `Clock::pacing`, the `late` count and the `worst` are all facts about the
*tick*; the editor's frame has a small mean and worst of its own and is only how often the
canvas is painted. Under load the bench is asking whether the world kept its cadence, which
is TICK. FRAME is worth a column beside it for what the load cost the editor, and nothing
about the instrument depends on it.

**And it gains a minimized column**: the editor minimized for a full minute with a projector
on the other display, reporting the synth's wakeups a second (from
`/proc/<pid>/task/<tid>/status`, `comm` == `synth`), the frame thread's share of a core,
`late`, and the tick count against wall time. Measured by hand once at 100 Hz —
100.2–100.4 wakeups a second, **0.0%** of a core on the frame thread, `late` zero, 100.3 ticks
a second over 3 m 45 s — and not since.

**The second finding still holds, retested.** With the tick on a thread of its own,
`pre_present_notify` every frame costs 97.9% of a core against 17.2% with the call out: the
poll is eframe's, not the tick's, and moving the tick away did not move it. See
[docs/decisions.md](../docs/decisions.md).

A ten-minute soak belongs in it too: the instance that had been up eight hours read a median
of 11.0 ms where a fresh one reads 10.0, and nothing explains that yet.

## The first finding: they are not the same problem

**The reading below was taken with the editor's window *backgrounded*, behind an editor and a
browser.** It was written up here as an idle baseline, which it is not: an occluded surface is
not being composited on the same schedule as a visible one, and a good part of the lateness is
probably that. A foreground reading has not been taken. The numbers still separate the two
problems, which is what they are quoted for, but they are not the floor.

Measured with nothing driving the editor and the ring covering a period with no inspection
traffic at all:

```
dropped         0   0/s on the worst Output, last second
late           18   10.01     20.12   p50 / p99 ms of 600 frames
```

**Three per cent of frames miss a refresh while the machine is doing nothing else**, and the
ninety-ninth percentile is 20.12 ms against a 10.01 ms median — almost exactly *two* refreshes.
That is the signature of missing a deadline and waiting a whole refresh for the next one.

And in the same window, **zero drops of any cause**. The GPU never once failed to finish a
frame.

So the two were never one story. The **drops** are a GPU-back-pressure counter currently
reading zero, and the **judder** is frame-arrival timing missing about one refresh in
thirty-three at idle. Only the second is a live problem, and it is the one the oscillator's
trace was showing.

Two caveats on the number. The binary measured carries `--features inspection`, which builds
an accessibility tree every frame and is not what ships. And it was taken with
`pre_present_notify` already in, so that line's own effect is unmeasured — there is no
before-and-after for it.

## The hazard underneath it: the render belongs to the editor's window

`r.draw()` — which renders **every active Output** — runs inside a `PaintCallback` on a `Ui`
in the **main window**. The projector's own callback does nothing but `blit_mixer`: it shows a
picture that was already drawn somewhere else.

So the picture an audience sees is downstream of the editor's window being composited. If the
main window stops painting, every Output stops rendering and the projector holds its last
frame. Minimizing already does exactly this, which is known. Occluding it evidently does not
stop it — the reading above is six hundred frames at the display's own cadence — but it is
the same dependency, and it is the reason "consistent regardless of what is in front of the
editor" is a structural question rather than a scheduling one.

**And `pre_present_notify` may cut the wrong way here.** It asks the compositor to schedule
redraws through the surface's frame callback, which is the right arrangement for a visible
window and is exactly what a compositor may stop sending for an occluded one. It went in
before this was understood, and its behavior on an occluded surface is untested. If it makes
a backgrounded editor paint *less*, it takes the render with it.

That makes a fourth thing worth measuring, ahead of the clock: **the same pacing reading
foreground, backgrounded, and minimized, with the line in and out.**

**Still unmeasured:** the same reading with a window flapping over the editor. Neither
`glxgears` (X11, cannot reach this Wayland display) nor a host window through `flatpak-spawn`
could be made to apply load from inside the development container, so that one needs a hand
on the session. The
question it answers: does load make the three per cent worse, or does it stay at three and
only the *worst* grow?

## The second finding: `pre_present_notify` is the judder

Measured on the 3440x1440 display at 100 Hz, Intel UHD 770, KWin 6.7, on the `Pumpkin`
project — a Julia set, a looping clip and three Outputs, 3.5 ms of GPU per frame — with the
Status box read through the inspection tree after at least 650 frames of nothing else. Each
cell is `late` of 600 frames, then p50 / p99 in ms.

| condition | as built: the call in, vsync | **the call out**, vsync | vsync off, paced by a timer of our own |
| --- | --- | --- | --- |
| foreground, undisturbed | 0–1, 10.1 / 11.2, worst 16.4 | **0, 10.0 / 10.85, worst 11.0** | 58–60, 10.0 / 21, worst 50 |
| sixteen busy CPU threads | 5–8, 10.0 / 13.4–15.6 | not taken | not taken |
| a dialog held over the window | 36–43, 10.0 / 20.0 | **0, 10.0 / 10.7** | not taken |
| the dialog flapped every half second | 237, avg 14.3 / p99 21.5 | **0–1, 10.0 / 12.2** | not taken |
| main thread | 100 % of a core, spinning | 22 %, asleep in `ppoll` | 18 %, asleep in `epoll` |
| drops | 0 | 0/s steady; 97 at start, 25 more under flapping | 16/s |

**Why the call in is worse.** winit's Wayland backend gates `RedrawRequested` on the frame
callback's state: once `pre_present_notify` has requested one, no redraw is delivered until
it arrives (`event_loop/mod.rs`, `FrameCallbackState::Requested`). eframe, meanwhile, sets
`ControlFlow::Poll` whenever a repaint is due, and a synth's repaint is always due. So between
frames the event loop polls `epoll` with a zero timeout — eight backtraces from the host's
`gdb`, seven of them in `calloop::dispatch` under `pump_events`, none in `swap_buffers` — and
the frame is drawn whenever the compositor's callback happens to land, which under any
disturbance is a whole refresh late. The p99 of exactly two refreshes in the first finding is
this.

**Why the call out is better.** With no callback requested, winit delivers the redraw at once
and `swap_buffers` with a swap interval of one blocks inside Mesa's own frame-callback wait,
in `wl_display_poll`. Same pacer, but a blocking one: the thread sleeps, the frame is drawn the
moment the compositor is ready, and a window over the editor changes nothing the distribution
can see.

**Why a timer of our own is worse still.** With the swap interval at zero and a repaint
scheduled per display interval, the intervals go bimodal — 3 ms and 21 ms — and the Outputs
drop sixteen frames a second: Mesa's Wayland EGL has a small pool of back buffers, and with
nothing throttling the client it stalls in `get_back_bo` waiting for the compositor to
release one. That is the "own cadence" shape of
[deterministic-loop.md](deterministic-loop.md) — its Shape A — and it is ruled out **through
EGL**. Owning the cadence means owning the buffers, which is Shape C, which is KMS-sized.

Caveats: the binary carried `--features inspection` and the readings passed through its
tree, and the flapping dialog took focus each time, which is a harsher case than a window
merely moving. Both apply to every column equally.

## The plan, in order

Done, the same day as the finding: the `pre_present_notify` line is out, with the reason in
`docs/rendering.md`; the trace plots by time (`cpu::TraceRing`), and the oscillator's band
under a flapping dialog is a sine; the Status box's budget is the display's own interval,
read from the monitor the window is on. Steps 5 and 6 above are not the answer — the loop
that is, a synth thread on a shared EGL context, is [deterministic-loop.md](deterministic-loop.md)
— and the presentation clock of step 3 is gone with them: a trace by time made the judder
visible for what it was, and what is left of the clock question is only `MAX_DT`'s permanent
drift after a stall, which the loop makes a bug to report rather than a fact.

What is left:

1. **The bench** of step 7, so the line cannot silently come back with an eframe upgrade —
   the gate is winit's and a future eframe may call `pre_present_notify` itself.
2. **A soak on a workspace that drops.** The Status box's `because` line already gives the
   drops by cause; on the Pumpkin project's oscillator workspace it reads `busy 336 · tap 258`
   after a minute, with two Outputs dropping ten a second right after a workspace switch and
   none a minute later. Whether any of that is steady state is what a ten-minute run says.

## Is one hundred per cent feasible?

Not literally, on a desktop with a compositor sharing the GPU, a process with no realtime
priority, and a kernel that can preempt. Some frames will miss for reasons outside this
program.

What is feasible, and is the claim worth holding to:

- **Zero `busy` drops in steady state on a simple patch**, with every remaining drop explained
  by its cause.
- **A stated worst case** — "p99 under one frame and max under two, over a ten-minute run" is
  something to be held to. "Never drops" is not.
