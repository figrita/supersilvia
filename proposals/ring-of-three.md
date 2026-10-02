# Proposal: a ring of three and a queue of one tick

**Status: implemented and benched.** It answers a finding of a code audit, with a second
finding folded in.

## The decision this answers

The goal is **no flashes**, not "no Output ever skips a tick". An Output that skips a
tick while its viewers keep showing its last finished frame is acceptable. A black frame, a
frame drawn over while a viewer reads it, or a blank ring is not.

Earlier the same day the rule had been that nothing in the draw set skips a tick. That was
said after visible Outputs dropped about 45 frames a second while the tick held 74 Hz, which
looked like stutter. So **evenness still matters**: a design that skips must skip rarely,
and it must not stutter one visible Output while its neighbours run smooth.

## Baseline before this change

The ring (`src/render/ring.rs`):
- `RING` is 3, the steady state: latest, shown and the one being drawn.
- It grows when a frame has nowhere to go, up to `RING_MAX` 6. A frame has nowhere to go
  when a viewer holds an older `Published`, or when the GPU has not finished earlier frames.
- A target beyond three that sits free for `SPARE_SWEEPS` (120 ticks) is freed.
- A stale-size target is freed as soon as nothing reads it (`free_stale`, from lane A).
- At the limit, a frame drops as `DropCause::Held`.

The queue (`src/render/mod.rs`):
- `TICKS_IN_FLIGHT` is 3. `Renderer::draw` waits, bounded by `QUEUE_WAIT_NS`, when the draw
  three ticks back is unfinished.
- Sized from it: `READBACKS` = `TICKS_IN_FLIGHT + 1` in output.rs, `timing::DEPTH` = 4, and
  `CAPTURE_RING` = 3, which the audit found redundant.

What the audit measured:
- **The real queue is deeper than three ticks.** Mesa's threaded context holds up to ten
  batches, and the kernel's execbuffer throttle holds the rest. On a GPU-bound tab the live
  synth spent about 88% of its time blocked in `DRM_IOCTL_I915_GEM_EXECBUFFER2`, while the
  `TICKS_IN_FLIGHT` wait fired on 0% of ticks. That is why the Status box read "GPU waits 0%"
  while "waiting on GPU" was 16 ms.
- The shown frame's real age is therefore unknown, and it is probably more than three ticks.
- With the GPU behind, a ring sits at 5 to 6 targets (probe `p10`). Holding 3 or 4 old
  frames gave 16 to 28 held drops per 100 ticks.

## The proposal

1. **A fixed ring of three.** Delete the growth, `RING_MAX`, `SPARE_SWEEPS`, the spare
   sweep, and most of `free_stale`'s reason to exist.
   - A resize reallocates a slot once nothing reads it, carrying the old picture scaled as
     today. Until then it keeps the old size.
   - With no free target, the Output skips the tick (`DropCause::Held` or a renamed cause).
     Its viewers keep the last finished frame.
   - The latest target must stay: a loop reads its own previous frame from it.
2. **A queue of one tick.** Before a tick submits, it waits for the previous tick's fence, so
   at most one of an Output's frames is ever on the GPU. With three targets, a free one then
   always exists unless a viewer holds a frame older than the shown one.
   - The wait is placed where the kernel's throttle cannot get in first: a `glClientWaitSync`
     with a flush on the previous tick's fence at the top of `draw`. Measure that it, and not
     execbuffer, is where the thread blocks, using the `/proc/<pid>/task/<tid>/syscall`
     sampling below.
   - This lowers latency. It costs throughput where the GPU idles between one tick's last
     draw and the next tick's first. The bench decides whether that cost is real.
3. **What follows if it holds:** `READBACKS` 2, `timing::DEPTH` 2, `CAPTURE_RING` 1, and the
   "GPU waits" row counting the one wait that now throttles.

## The benching that decides it

**Machine.** The Intel iGPU only (renderD128), never a discrete GPU, and never with other GPU
work running: the audit's numbers were contaminated by parallel test suites. Close the live app,
or measure it alone.

**Build a latency column into `examples/tick_bench.rs` first.**
- The shown frame's age in ticks is the tick a `Published` names, subtracted from the tick
  being published. `paced_over_budget_nothing_drops_and_the_tick_slows_evenly` already
  measures "stalest" this way, with a tapped Output that writes the tick it drew.
- Report its median, p90 and max beside the rate and drops per second.

**Runs, each for main and for the branch, 30 s each:**
- The demo's "Effect kernels", "Feedback and Output" and "Games and simulations" tabs at
  100 Hz. The demo is the twelve-tab project `21 September`, not in the repository; `tick_bench`
  copies it itself.
- The same with `SUPERSILVIA_BENCH_EDITOR=<quads>` standing an editor beside it at the display
  rate.
- A light tab ("Start here") to confirm nothing regresses when the GPU keeps up.

**Record for each run:**
- ticks per second;
- every drawn Output's draws and drops per second;
- the shown frame's age;
- the editor stand-in's frame time;
- where the synth thread blocks (below).

**Sampling where the synth blocks**, live or headless: take about 1000 samples of
`/proc/<pid>/task/<synth tid>/syscall`. Syscall 16 (ioctl) with request `0x40406469` is
execbuffer. Syscall 202 (futex) or 7 (poll) near a fence wait is the renderer's own wait.

**Deciding:**
- The branch is taken if it holds the same tick rate within a few percent, and skips no more
  than a handful of frames a second, spread evenly rather than on one Output.
- It is taken if the frame's age drops. It is also taken if everything else is even and the
  code is smaller.
- If the one-tick queue costs real throughput, try a queue of two with a ring of three and
  compare skips.

## Tests that change

- `paced_over_budget_nothing_drops_and_the_tick_slows_evenly` becomes "skips are rare and
  even, and the stalest frame is at most one tick old".
- `nothing_drops_on_a_saturated_gpu` and
  `a_consumer_and_its_producer_draw_every_tick_on_a_saturated_gpu`: under a queue of one,
  draws should still land every tick; check before rewriting them.
- Under a fixed ring, these lane A and ring tests must be reworded:
  - `held_frames_are_never_drawn_into_and_the_ring_shrinks_once_let_go`;
  - `an_output_resized_every_tick_with_the_gpu_behind_drops_nothing`;
  - `one_resize_under_load_drops_nothing`.

  What must survive is that a held frame is never drawn into, and a resize never shows black.
- `unplugging_an_output_whose_targets_are_all_held_blanks_none_of_them` stays as it is.

## Docs that change

rendering.md:
- "Per Output, per frame" (the ring paragraph);
- "Every Output draws every tick", which becomes "an Output skips only where nothing is
  free";
- "Zero flash";
- "The GPU timer".

Elsewhere:
- decisions.md: the ring and the queue entries.
- ui.md: the Status box's drop and "GPU waits" rows.

## Bench result

Thirty-second release runs on the Intel iGPU, after five seconds of warmup, used the
demo project at 100 Hz. The age is the median shown frame age in ticks. The editor
column ran with `SUPERSILVIA_BENCH_EDITOR=1`. All one-tick runs had zero held drops.

| Tab | Editor | Baseline ticks/s | One tick ticks/s | Baseline age | One tick age |
| --- | --- | ---: | ---: | ---: | ---: |
| Effect kernels | no | 54.5 | 53.2 | 3 | 1 |
| Effect kernels | yes | 50.4 | 48.4 | 3 | 1 |
| Feedback and Output | no | 99.9 | 100.0 | 1 | 1 |
| Feedback and Output | yes | 99.8 | 100.0 | 1 | 1 |
| Games and simulations | no | 57.1 | 55.5 | 3 | 1 |
| Games and simulations | yes | 52.6 | 50.0 | 3 | 1 |
| Start here | no | 100.0 | 100.0 | 1 | 1 |
| Start here | yes | 100.0 | 100.0 | 1 | 1 |

The two-tick fallback ran Effect kernels at 51.9 ticks/s without an editor
and Games and simulations at 80.7, but dropped 17 frames/s per
Output on Effect kernels and 3–27 frames/s unevenly across Games and simulations.
With an editor it dropped 282 and 130 frames/s across the respective tabs. The
one-tick queue is the accepted choice. On the heavy headless runs, syscall samples
most often found the synth in `DRM_IOCTL_SYNCOBJ_WAIT` (`0xc02864c3`), so the
observed blocking is in the driver's sync wait.

The live app was opened with egui inspection on the same GPU. Its Outputs displayed
valid pictures after tab switches and resize. A clean Chrome run accumulated four
held drops in 30 seconds; Effect kernels remained flat over 15 seconds. An earlier
live Chrome run accumulated hundreds of held drops over 15 seconds. That burst did
not recur in the later clean run; its cause remains unknown, so live skip rates need
a longer repeat measurement before treating them as stable.

## Live editor correction

The isolated editor stand-in measured paint cost but did not keep a `Published` frame
through its paint cycle. In the real editor at about 55 fps, that frame can stay held
for two 100 Hz synth ticks. A fixed three-target ring then recorded about 25 held
drops/s on each visible Output while the GPU had spare time. The cap is five targets,
allocated on demand: latest, shown, one to draw, and two for slower viewers. The
one-tick queue remains. This corrects the bench coverage gap without restoring spare
target sweeps.
