# Proposal: `mouseinput`

**Status: built.** `src/nodes/mouseinput.rs` is the node, `src/pointer.rs` the one slot its
two sources write into — the pictures thread's own `wl_pointer` in
`render/picture/thread.rs`, and egui's in `app/frame.rs` for the preview and the canvas —
and `TickContext::pointer` the way it reaches a tick. [docs/media.md](../docs/media.md#the-pointer)
and [docs/decisions.md](../docs/decisions.md) carry it. Agreed on 21 September 2026, with
options for the position framing.

silvia's `mouseinput` publishes the pointer's position over the editor page,
normalized to the page, and the two buttons. Left out of stage 10 of
[context-free.md](context-free.md) because it needs pointer plumbing and a decision about
what surface the position is relative to.

## The decision

silvia's surface was the whole editor, because the editor was the whole app. Here there
are three candidates, and a performer means one of them:

- **A picture in a window of its own**, when one is open: the mix popped out, or an
  Output's own picture popped out, in that picture's world units, so `x` and `y` can drive
  a `sample`'s point or a shape's center and land where the hand is. There is no projector
  any more — see [picture-windows.md](picture-windows.md) — so this is *a* window rather
  than *the* window, and which one is now part of the question.
- **The mixer's preview**, when no picture is popped out.
- **The canvas**, in world units of the canvas: what silvia did, and the least useful,
  since the hand is on a node most of the time.

**The node chooses, and the picture is the default.** "Framing" is read here as the
surface the position is framed in, so the three candidates are an option on the node rather
than a rule in the code: *Picture* — a popped-out one, else the mixer's preview — *Preview*,
and *Canvas*, with *Picture* as the default. A patch that wants the hand measured against
the canvas says so, and a patch that says nothing gets the one a performer means.

The default reads in that picture's world units, with the buttons as gates through
`ctx.pressed`-style levels, and the node reading *not present* while the pointer is off the
framed surface. A picture window is a Wayland surface on the pictures thread rather than an
egui widget, so its pointer arrives through that thread's own `wl_pointer` and not through
egui; the preview, the canvas and the on-node pictures are ordinary egui regions, which is
the channel [node-body-controls.md](node-body-controls.md) built and which now exists. Two
sources, one port.

Done when: `x`, `y` on the rows follow the pointer over a popped-out picture, switching
the framing option to *Canvas* moves them onto the canvas instead, a `sample` fed by them
reads the color under the cursor, and `tests/uniform.rs` feeds a synthetic pointer through
the context.
