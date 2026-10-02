# Proposal: `text`

**Status: built.** `src/nodes/text.rs` is the node, `src/video/text.rs` the rasterizer, and
`tests/video.rs` renders a known string and finds the ink where the glyphs are; the reasoning
is in [docs/decisions.md](../docs/decisions.md) under *The words are a node's value and the
screen is a node's own session*, and what it does is in
[docs/media.md](../docs/media.md#words-as-a-picture). One thing below changed in the building:
the pipeline is `textoverlay` rather than `textrender`, which are one plugin and one
rasterizer — `textrender` sizes its output to the text, which would make Size a resolution
rather than a size.

**Agreed on 21 September 2026**, building on the multi-line text the note already has.
silvia's `text` renders a string into a texture with a
font, a size and a color. Left out of stage 10 of [context-free.md](context-free.md)
because `nodes/` may take no graphical dependency and a font rasterizer is one.

## The shape, and why it needs no new crate

GStreamer already rasterizes text: **`textrender`** (pango) turns a text buffer into video
frames, and it is installed in the box. The node is a pipeline —
`appsrc ! textrender font-desc=... ! videoconvert ! appsink` — fed the string whenever it
changes, publishing one frame through the same triple buffer `camera` uses, republished
untouched while nothing changes. The text is a **value**, the kind `note` already keeps (the
font and size are options, the color a `VaryingColor` input mixed in GLSL over the rendered
coverage), so the picture stays context-free and the color cable-able.

## What it costs

Pango's fonts are the system's; a project opened on another machine may render with a
different face, which is what silvia's did too.

**The multi-line box already exists, so there is no one-line fallback.** `note` keeps its
text in `Node::values`, declared by a `ValueDef { key: "text", label: "", kind:
ValueKind::Text { rows, placeholder } }` in `src/nodes/note.rs` and drawn by `src/ui/text.rs`
over egui's `TextEdit::multiline`. See [docs/decisions.md](../docs/decisions.md) under *A
node's own values are not options* for why that store exists and why the node draws it
itself. This node declares the same value, four rows by default, and passes the string
straight to the pipeline — the only new thing is that a keystroke here pushes a buffer
instead of costing nothing.

Done when: a string appears in an Output, changing it costs no shader rebuild, and
`tests/video.rs` renders a known string and finds ink where the glyphs are.
