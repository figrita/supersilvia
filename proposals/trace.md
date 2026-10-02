# Proposal: what a trace band says

**Status: proposed, not agreed.** Written because the same complaint was filed against three
nodes separately — `adsr`, `animation` and `oscillator` — and answering it three times on
three nodes is the wrong shape. The band is one widget, `src/widgets/trace.rs`, so the
question is what that widget says, not what `animation`'s picture says.

The parity review's answer on the `animation` entry was that this is a matter for the trace
component, which another node had raised too, and that it wanted a detailed proposal on the
component after research into what both programs afford.

## What silvia draws

`js/realtimeGraph.js` is one class, `RealtimeGraph`, and five nodes hand it a `<canvas>`:
`adsrenvelope`, `animation`, `automation`, `oscillator` and the node template. It is 320 by
120 in the node's custom area, `#222` on a 1px `#555` border, 4px radius.

| | |
| --- | --- |
| **History** | 200 samples, pushed once per `requestAnimationFrame` and shifted off the front. Not dated: one sample is one drawn frame, so the same second of signal is twice as wide on a 120 Hz display as on a 60 Hz one, and a stalled tab leaves a straight line. |
| **Scale** | `autoScale` fits the visible history and pads it by 10%, or the node passes fixed `minValue`/`maxValue`. `animation` passes the Start and End values and re-passes them whenever either knob moves; `adsrenvelope` passes 0 and its Max Level; `oscillator` passes the amplitude either side of the offset. |
| **Grid** | 4 horizontal lines across the height, vertical lines every 40px. A dashed `#555` zero line when the range spans zero. |
| **Line** | 2px, in the number port's own theme hue, drawn across the whole width whatever the history holds. |
| **Now** | a 3px filled dot at the right edge, at the current value, in a lighter shade of the same hue. |
| **Numbers** | the current value to three decimals, 12px monospace, top right; the range's top at the top left and its bottom at the bottom left, to one decimal, in `#666`. |

So a silvia trace answers three questions at once: what shape is this making, how far does it
reach, and what is the number right now.

## What we draw

`widgets::trace::TRACE` is a region — under a Trace heading, claims no pointer, 48 points tall, and it
asks for `SCOPE_NODE_WIDTH` (300) so the band is the same width as an audio scope. Three nodes
declare it: `adsr` (3 s of history), `animation` (200/60 s, silvia's 200 samples at the 60 Hz
they were drawn at) and `oscillator` (5 s). `ui::scope::trace` draws it.

| | |
| --- | --- |
| **History** | `CpuNode::trace`'s own `TraceRing`, a span in **seconds** rather than a count. Every sample is dated with the one clock's `elapsed`, and x is the sample's age over the span, so the picture is the same picture at any frame rate and a stall leaves a gap rather than a compression. This is the one place we are already ahead. |
| **Scale** | always fitted to what the ring holds, with a 4 pt margin top and bottom and a flat line padded rather than collapsed. Never fixed, and nothing is drawn to say what the fit is. |
| **Grid** | none. One hairline along the bottom, shared with the spectrum under a scope so the two read as the same kind of band. |
| **Line** | 1.5 pt at zoom 1, in the hue of the node's **first output port**, so an envelope's line is the number hue and nothing has to be configured. |
| **Now** | nothing; the newest sample is simply where the line ends, at the right edge. |
| **Numbers** | none. The live value is on the output row instead, which is where every other uniform number on every other node prints. |

## What a person actually loses

Two things, and they are not the same size.

1. **The band has no scale.** A travel from 0 to 1 and a travel from 0 to 1000 draw the same
   picture, and an envelope peaking at 0.1 looks like one peaking at 1. Nothing on the band
   says so, so the shape is honest and the size is a lie of omission. This is the complaint,
   on all three nodes.
2. **The band has no numbers.** This one is mostly already answered: the live value prints on
   the output row, a hand's width above the band, and printing it twice would be the only
   place in the program where a uniform number is drawn in two places on one node.

## What to do

**Recommended: give the band a declared range, and draw that range's two ends on it. Nothing
else.**

- `CpuNode::trace` gains a range beside its ring — `Option<(f32, f32)>`, the node's own answer
  to *what is the full height of this picture*. `adsr` says `(0.0, 1.0)`, because its envelope
  is normalized and that is already ruled. `animation` says its Start and End values, the way
  silvia's does, re-read every tick so moving a knob rescales the band. `oscillator` says
  offset ± amplitude. A node that genuinely cannot know — none today — says `None` and keeps
  the fit it has now.
- The band draws the range's two ends in its own corners, top left and bottom left, in the
  muted text color at the small size, exactly as silvia does. That is the whole of the
  numbers: no live value, because the output row has it.
- A value outside the declared range is **clamped to the band and the band says so** by
  drawing the line along the edge it left through; silvia's would simply draw it out of the
  box, because a canvas clips and nobody notices.

**Not recommended, and why:**

- **The grid.** Four horizontal lines and a vertical every 40px is a lot of ink for a band 48
  points tall — silvia's is 120. With the two end numbers there, the grid is decoration.
- **The dot at the leading edge.** It says *now* on a picture whose right edge is always now.
  It earns its place in silvia because silvia's history is frame-counted and can stall; ours
  is dated and cannot.
- **A second live number on the band.** The output row already prints it.
- **Matching silvia's 320 by 120.** The band is 300 wide because a node is, and 48 tall
  because that is a band rather than a panel. Three of these on a canvas at 120 points tall
  each is a wall of graph.

## The open question

**Does the band want a heading?** Answered on 1 October, and built: yes, a
**Trace** heading, open by default, on every trace and on Automation's recorded curve.

Done when: the three nodes declare a range, the band draws two numbers and clamps to it,
`tests/ui.rs` has a snapshot of `animation` with Start and End a thousand apart, and
`docs/ui.md`'s trace paragraph says what the band's scale is.
