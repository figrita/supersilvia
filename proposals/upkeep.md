# Proposal: upkeep

**Status: 2 taken 2026-09-12; 1 and 3 still found, not agreed.** Three findings from a read of
the tree on 2026-09-12. Nothing here is a decision. Each item is a finding with its evidence,
a shape for the fix, and what would make it done, to be accepted, reshaped or dropped. None blocks any stage of
[context-free.md](context-free.md), and none is a bug anyone can see on screen.

## What was read, and what held

The audit covered `compile/`, `graph/`, `app.rs`, `nodes/tap.rs`, and a slice of
`tests/headless_gl.rs` and `tests/ui.rs`. Recording what held matters as much as what did
not, because the next reader needs to know where not to look — and because one of these
nearly acquired a fix it did not need.

- **The tap's fixed point is tight, and deliberately so.** `SCALE` is 65536 and `BIAS` is
  32768 because `BIAS * SCALE` is exactly `2^31`: `stats_glsl` clamps the measured quantity
  into `±BIAS` on the first line of the emitted body, so every `int()` conversion below it
  lands inside `i32` and every `uint()` inside `u32`, with the weight sum — the one
  conversion written without a visible `clamp` — bounded by the same clamp through
  `w = max(l, 0.0)`. Changing either constant breaks all four conversions at once. Anyone
  reading `add64(base + WSUM, ...)` and seeing an unguarded `uint()` should read line one
  again before reaching for a clamp.
- **The panic surface is clean.** Of the `unwrap`/`expect` sites in `src/`, the ones in
  `video/clip.rs` and `ui/canvas.rs` — the two heaviest files by that count — are inside
  `#[cfg(test)]` modules. What remains in a live path is `expect("checked above")` beside
  the check, a `write!` into a `String`, and one thread spawn. Nothing in `tick` or
  `build_frame_job` can panic on a graph a hand can build.
- **No `TODO`, `FIXME`, `HACK` or `XXX` anywhere in `src/`.**
- **155 of 157 documentation anchors resolve.** Item 3 is the other two.

## 1 — `live_nodes` rescans every connection, three times a frame

**The evidence.** `App::live_nodes` walks upstream from the mixer's claimed Outputs, and
for every node it pops it scans the whole connection list to find that node's producers:

```rust
stack.extend(
    self.graph.connections().iter().filter(|c| c.to.node == id).map(|c| c.from.node),
);
```

That is the claimed subgraph times every connection in the project. `Graph` already keeps
reversed adjacency behind a `OnceCell` — but only `immediate_rev`, which drops delayed
edges, and liveness wants every edge including the delayed and the action ones, so the walk
cannot use it and scans instead.

It is called three times in a frame with no edit between them: `build_frame_job` calls
`reassign_hosts`, which calls `assign_hosts`, which calls it; `build_frame_job` calls it
again for the suspend decision; and `tick` calls it a third time. `mark_thumbnails` is a
fourth when it runs. Each call also collects a fresh `HashSet` over every node in the
project.

The rule this breaks is stated in this codebase, beside the memo that exists because a
cable drag hit the same wall: *"without the memo the sweep is quadratic however cheap the
adjacency is."*

**Read:** `src/app.rs` (`live_nodes`, `assign_hosts`, `reassign_hosts`, `build_frame_job`,
`tick`, `mark_thumbnails`), `src/graph/mod.rs` (`Adjacency`, `adjacency`, `can_reach`),
[docs/architecture.md](../docs/architecture.md#the-recompile-boundary).

**Change:**

- **`Adjacency` gains `rev`:** consumers to producers over every connection `with_actions`
  already covers, built in the same pass as the other three and cleared with them.
  `live_nodes` follows it instead of scanning.
- **One behavior note, which is the only thing in this item that is not purely mechanical.**
  `adjacency` skips a connection whose source port does not resolve; `live_nodes` does not.
  A graph naming a port no definition has would therefore lose that edge from the liveness
  walk. The compiler already reports that graph as `Diagnostic::NoSuchOutput`, so the change
  is from one arbitrary answer to another about a graph that is already broken — but it is a
  change, and it belongs in the commit message rather than being discovered later.
- **Optionally, one answer a frame.** A `HashSet` field, filled on the first ask and
  invalidated where `reassign_hosts` is already called — after a command, when a tab opens
  or closes, and once a frame for a deck claim. Those four sites are exactly the places
  liveness can change, which is why `reassign_hosts` is called at them. Take this only if
  the adjacency fix alone does not flatten the cost; a cache with four invalidation points
  is worth more care than a memo with none.

**Docs:** none. No behavior a hand can see moves, so `docs/` does not change; the note
above goes in the commit message.

**Done when:** `live_nodes` names no `connections()`; a `tests/graph.rs` test holds `rev` to
the edges `with_actions` carries; `./check.sh` is green.

## 2 — `app.rs` has lost its shape — taken, 2026-09-12

**The evidence.** 4,443 lines and 140 methods in one `impl`. Its module doc reads *"The
application. Thin: it owns the pieces and drives one frame."* Two of its methods are about
425 lines each: `apply`, the command dispatcher, and `ui`, the eframe frame body. File
dialogs, thumbnail marking, asset import and export, the projector window, pop-outs, toast
timing, undo trimming, deck claiming, uniform resolution and the tick all live in it.

This is the only file in the tree that has lost its shape, and it has lost it in the way
that is hardest to reverse: everything reaches everything through `&mut self`, so the seams
get fainter with each thing added.

**Read:** `src/app.rs` in full before moving any of it; the module layout of `graph/`,
`compile/`, `nodes/`, `render/`, `ui/`, `video/` and `audio/` as the shape to match.

**Change:** `src/app/` with a `mod.rs` holding `App`, its state and `tick`, and the rest
split along seams that already exist. Approximate sizes, from the tree as read:

| module | what moves | lines |
| --- | --- | --- |
| `app/edit.rs` | `apply`, `coerce`, `check_membership`, `refit`, `mark_recompile`, `dependent_outputs` | ~530 |
| `app/files.rs` | `ask_for_file` through `open_last_or_untitled` — dialogs, projects, assets, import and export | ~545 |
| `app/frame.rs` | the `eframe::App` body, `preview`, `blit_into`, `show_status_box`, `status_view` | ~425 |
| `app/show.rs` | `Show`, the projector, pop-outs, toast, `mix_cover`, window geometry | ~390 |
| `app/measure.rs` | `readback` through `cost_strips` — readbacks, probes, drops, costs | ~265 |
| `app/history.rs` | `Step`, `control_target`, `continues_last` through `reset_history` | ~235 |

That leaves a core of roughly 1,900 lines that is what the module doc claims: the pieces,
the frame job, the tick. The doc comment is then true again, which is the point of the item
— it is not a line count.

**Two conditions, because this is a large mechanical change that alters no behavior.** It
goes in one commit with nothing else in it, and it is taken when nothing else is open on
`app.rs`, because it conflicts with everything. Both were met.

**What was actually done.** The table above, exactly: six modules, and `mod.rs` keeping the
struct, the accessors, the workspaces and tabs on screen, the frame job and the tick. That
was chosen over two shapes that would have hit the line target — this page contradicted
itself, asking for a ~1,900-line core in the prose and no file over 600 in the done-when, and
the core is what was kept. `mod.rs` is 1,633 lines; nothing else is over 700.

Each module is an `impl App` over the same fields, since the seams are in what the methods
are *for* rather than in the state they touch. What crosses a seam is marked `pub(super)` and
nothing else was made more visible: 32 methods, four types, four constants and two fields.
All 129 method bodies moved byte for byte — the one that differs at all is `cost_strips`,
whose signature `cargo fmt` rewrapped once `pub(super)` made it too long. No snapshot moved,
which is the whole test of a move.

**Docs:** [docs/architecture.md](../docs/architecture.md) names `app.rs` where it describes
the frame; it names the folder instead. Nothing else in `docs/` describes the file's
insides.

**Done when:** ~~no file in `src/app/` is over about 600 lines~~ — set aside deliberately,
see above; the module doc of `src/app/mod.rs` is true as written — rewritten, since it now
has six siblings to name and "thin" was never going to be true of the core; `./check.sh` is
green and every snapshot is unchanged, which is the whole test of a move. Done.

## 3 — Two dead anchors, and nothing checks them

**The evidence.** 157 `*.md#anchor` references across `src/`, `docs/`, `proposals/` and
the contributor guide. 155 resolve. The two that do not are both in `proposals/` and
both point into `docs/`:

- ~~A queue item sent the reader to
  `docs/decisions.md#a-third-parameter-kind-asset-references`.~~ **Repointed 2026-09-13** at
  *An option carries what it costs, and three of the four kinds cost nothing*, which is the
  heading that ended up covering it.
- ~~[context-free.md](context-free.md) stage 1 sent the reader to
  `docs/rendering.md#the-frame-history`.~~ **Repointed 2026-09-13** at *Per Output, per
  frame*. The section was gone, deleted by the stage that cited it — which is the rot
  exactly: a landed change removed a heading and the prose that pointed at it was not in the
  same commit.

**Both were fixed by hand, which is the point rather than the fix.** A reconciliation pass on
2026-09-13 ran the slug rules below over the whole tree and found them in seconds; nothing
ran them between the rot appearing and someone going looking. The two anchors quoted *in this
file* stay dead on purpose — they are the evidence, not links — so a checker needs to read
this page as prose or allow it, which is one more argument for `tests/docs.rs` over a script
somebody remembers to run.

Two in 157 after 258 commits is a good record, and the point of this item is that it is a
good record kept by hand. `check.sh` gates the code and nothing gates the prose, and
`docs/decisions.md` is 2,242 lines — longer than any source file — growing by contract with
every stage that lands.

Two further references are not links at all: `nodes/adsr.rs` and `nodes/oscillator.rs` each
carry a bare anchor wrapped across two `//` lines — `decisions.md#a-trace-is-a-picture-of-`
and the rest of it on the next line — so neither half resolves and neither is clickable.
Still true on 2026-09-13, and they are the only two left in the tree.

**Read:** [docs/testing.md](../docs/testing.md) for which layer a prose test belongs to,
`scripts/doctor.sh` for the shape of a check that reports rather than asserts.

**Change:**

- **`tests/docs.rs`.** Collect every `<file>.md#<anchor>` in `src/**/*.rs`, `docs/*.md`,
  `proposals/*.md` and `CONTRIBUTING.md`; resolve each against the headings of the
  named file; assert none is unresolved, naming the source line of each that is. A test and
  not a script, so it runs under `cargo test` where every other invariant here runs.
- **The slug rules, written down, because they are easy to get wrong:** lowercase; drop
  every character that is not alphanumeric, a space, a hyphen or an underscore; spaces to
  hyphens. Backticks and commas vanish; underscores stay, so
  `worst_dt-not-mean-frame-time` is correct; an em dash surrounded by spaces leaves two
  hyphens, so `f4--the-mixer-is-one-more-render-target` is correct. A checker that strips
  underscores or maps the em dash to a hyphen reports five false failures on this tree.
- Fix the two dead anchors and put the two wrapped ones on one line each.

**Docs:** [docs/testing.md](../docs/testing.md) gains a line for the layer-0 prose test;
`CONTRIBUTING.md`'s Documentation section gains *an anchor into `docs/` is checked*, beside the
rule that a behavior change and its doc share a commit.

**Done when:** `tests/docs.rs` passes with no allowlist; `./check.sh` is green.

## Where these go

None is a stage of anything and none blocks a stage. If they are taken, 3 is small enough to ride along with
whatever touches `docs/` next, and 1 belongs before the next thing that makes a frame more
expensive. 2 is done.

One thing 2 turned up on the way, left for whoever takes 3: two doc comments in the old file
had slid off their owners. `FIRST_DROP`'s and `PROJECTOR_WIDTH`'s docs were stacked on top of
`DropRate`'s, which the split had to undo because the three constants now live in three
files; and in the `App` struct, `thumbnails_pending`'s doc sits above `pop_outs` with
`pop_outs`' own doc beneath it, leaving `thumbnails_pending` undocumented. The second is
untouched, because this commit was to contain nothing but the move.
