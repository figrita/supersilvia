# Design system

## Where it comes from

The palette and the editor's geometry came from a design mockup of silvia's editor, an
HTML and CSS project kept outside this repository. The files in it that mattered:

| Path | Contents |
| --- | --- |
| `colors_and_type.css` | the four anchors, saturation ladder, type, radii, spacing |
| `ui_kits/editor/styles.css` | node, port, wire, s-number, s-color, menu, modal geometry |
| `ui_kits/editor/*.jsx` | component structure |
| `design_handoff_silvia/screenshots/**` | rendered reference images |

supersilvia is egui, so markup does not transfer. **The token layer does**, and
`src/ui/theme.rs` is that layer — the only place a color is defined. One triple there does
not derive from the anchors: the three audio band colors are literal red, green and blue,
because a convention is what makes a dot legible without a label —
[media.md](media.md#the-scope-on-the-node) says why.

## Four anchors

Everything derives from four HSL anchors plus a saturation ladder. Neutrals are the theme hue
at 5–25% of the theme saturation, never gray. Setting `main.s = 0.0` therefore yields a
genuinely grayscale theme rather than a tinted one; a test asserts `r == g == b`.

The default is the canonical **vapor** preset:

| Anchor | HSL | Role |
| --- | --- | --- |
| main | 186, 84%, 58% | UI, cyan |
| number | 277, 84%, 58% | number ports, violet |
| color | 330, 81%, 60% | color ports, magenta |
| event | 45, 91%, 57% | action ports, amber |

A whole re-theme is four numbers. That is the visual expression of the same discipline as the
compiler: one parameter, everything else derived.

**And they are four numbers you can actually reach**, which was not true until the
Preferences window existed.

`Edit ▸ Preferences…` puts the four colors up as four `s-color` swatches, with silvia's
sixteen presets under them. The editor re-tints as you drag a picker. See [the Preferences
window](ui.md#the-preferences-window).

Two image snapshots in `tests/ui.rs` hold the claim above: the same graph under two presets.
A color that did not come from one of the four shows up there as a pixel that refused to
move.

The four colors above are the default, and the default is a preset — silvia's `vapor`,
rounded when the palette was first ported. silvia's own default look is `vanilla`: green
number ports, orange color ports, violet event ports. It is one click away.

**`UniformNumber` did not get a fifth anchor.** A uniform number is a number the CPU holds
rather than a different kind of quantity, so `src/ui/theme.rs` derives it from the number hue,
the same color exactly, and the diamond is what tells the two apart. A
fifth themeable color would say they are unrelated, and would make a re-theme five numbers to
express a distinction the shape already carries.

**The tokens beat the comments.** The hexes in the design system's CSS comments are the
colors its HSL tokens were rounded *from* — `hsl(186,84%,58%)` is `#3adcee` where the
comment reads `#3adfed`. The tokens are what the theme system interpolates, so the tokens
win. A test pins all four.

## Component rules worth stating

These are things the handoff specified that a reasonable person would guess wrong:

- **Node layout is header, then all inputs, then all outputs, then all options**, each its
  own block with alternating row bands that restart at each one — `.node-inputs` and
  `.node-outputs` are stacked sections in a column-flex node, not a left column beside a
  right one, and the option rows are a third such section under them. **Inputs and outputs
  are not full width.** Each insets 6px (`ROW_BLOCK_INSET`, silvia's `0.5rem`) away from the
  port side — the near edge, where the port hangs off it, stays flush to the node's true edge
  — and rounds that far corner by 6px (`ROW_BLOCK_RADIUS`) on its own first and last row only,
  reading as a slab short of the edge rather than the full row. Options carry neither inset
  nor per-row rounding. `canvas::row_block` is the one function that answers where a row's own
  background sits, so the fill, a label's room and a control's slot cannot disagree about it.
- **Shape carries the type alongside color**, so the distinction survives a grayscale theme and
  a colorblind viewer. Three shapes: an action port is a rounded square, a uniform number port
  is a diamond, everything else is a circle. **An unconnected input is a ring, not a filled
  dot** — the one thing silvia's uniformly filled ports do not say. Its interior is not empty
  canvas: it is a hole, the port's own hue sunk to `Theme::port_hole` (`s * 0.5`, `l = 0.12`),
  filled edge to edge under the stroke rather than a second, smaller dot floating inside a
  transparent ring. **A filled port is flat, not a bead.** silvia's `linear-gradient(140deg,
  ...)` on `.port` was tried as one small offset circle standing in for the gradient's
  highlight — then removed outright, because the shine looked bad. See
  [decisions.md](decisions.md#a-filled-port-is-a-flat-shape-no-highlight).
- **Nodes are pillowy (12 px radius); everything else is sharp (2–6 px).** A tab is sharp,
  rounded on its top two corners only, so it reads as joined to the canvas below it — fill and
  border by the one radius, since a border rounded on a corner the fill left square puts the
  fill outside its own line. The active tab's lit top edge is held in at each end by that same
  radius rather than running the full width and squaring the two corners off again.
- **A node casts the editor's own window shadow.** `Visuals::window_shadow` — the one the
  Status box and the Preferences window cast — laid under the body as a real `epaint::Shadow`,
  scaled by the zoom, behind the `node_shadow` preference. silvia's `box-shadow: 0 0 8px` was
  first tried as three concentric strokes stepping outward with falling alpha, to dodge a true
  blur's per-node tessellation cost, and that approximation was rejected outright. See
  [decisions.md](decisions.md#a-nodes-shadow-is-a-real-one-or-none-never-an-approximation-of-one).
- **A groove marks where a node's sections meet.** Between two non-empty sections — inputs to
  outputs, or either to options — a two-hairline groove (`bg_sunken` above `border_subtle`)
  stands in for silvia's `<hr>` (`border-top: 2px groove`), which a browser renders as a line
  carved into the surface rather than a flat one. This one is kept: a groove border is two flat
  tones, not a blur, so the same shape a browser paints is the same shape drawn here.
- **A tab says which one it is with the primary anchor and nothing else**: the active tab is
  `bg_tertiary` with a two-pixel `primary` line along its top edge, an inactive one is
  `bg_sunken` and `bg_secondary` under the pointer. Every one of those is a neutral off the
  ladder in `ui/theme.rs`; no color is written anywhere else.
- **A cross-workspace tag is a chip in its cable's color, not a badge of its own.** 18 px
  tall, sharp at `RADIUS_SM`, 6 px of padding, 12 px clear of the port and of the next tag,
  elided past 132 px. Its ground, outline and label are the port's own hue at three
  lightnesses — `tag_fill`, `tag_border`, `tag_text` in `ui/theme.rs` — so it re-tints with
  the four anchors and reads as the cable it stands in for. A tag naming a closed workspace
  is the same chip multiplied by `TAG_CLOSED`; closed is still in the project, so it is
  quieter rather than absent.
- **A key is a cap.** Wherever a key the hand presses is named — the keyboard shortcuts window
  first — it is drawn as one, `ui/keycap.rs`: a rounded cap at `RADIUS_SM`, its legend in
  `text_primary` on a `bg_interactive` face with a 1 px `border_normal` outline, lifted 2 px
  over a `bg_sunken` base that shows only as the thicker, darker edge along its bottom. **Only
  a key with a legend printed on it is a cap**: a letter, a digit, a symbol, a modifier, Esc,
  Tab, Enter, an arrow, a function key. The space bar carries none, so `Space` stays a word,
  and so does every gesture of the pointer's — *drag* and *click* are not keys. A chord is its
  caps joined by a muted `+`, which a Mac leaves out as it writes `⌃⌥⇧⌘` chords; alternatives
  are joined by a `/` and a run by a `–`. The caps are paint, so a row keeps its keys' text as
  its accessible name.
- **A select hugs its value.** The handoff's `.node-option select` is a native `<select>`:
  `bg_interactive`, 1 px `border_normal` at `RADIUS_SM`, `padding: 4px 6px`, `primary_muted`
  on hover and `primary` plus a ring on focus. Two things follow from *native* that a
  reimplementation drops by default and this one does not. It is **the width of its content**,
  never a fixed box — so a short choice takes the room a short choice needs and a file path
  takes whatever is left of the row, elided at the end that says least. And it has a
  **chevron**, which is the whole of what separates a select from a button; ours is painted
  rather than typed, because a glyph is the one thing a font fallback is allowed to turn into
  `◻`. The row it sits in is `OPTION_ROW_PITCH`, which is **tighter than the handoff's**: an
  option is set once and then read, and a `video` node has four of them.
- **Text is Space Grotesk at a 12 px base**, headings Space Grotesk SemiBold. Monospace is kept for what is
  read column by column: a path, a MIDI message, the Status box. See
  [Fonts](#fonts-and-why-four-are-vendored).
- **The canvas is compact.** Every size is the least that holds its text and its target at
  zoom 1: a 180-wide node, a 22-point header, 20-point port and option rows, 26-point control
  rows, and an s-number of 84x20. A screen holds more of the patch, and there is less chrome
  to read past and to draw.
- **The on-node render is a defined component**: 216x122, flush to the node's bottom corners.
  That is why an Output node is 216 wide where others are 180. A handful of other nodes are
  216 too — `NodeDef::width`, for a node whose longest row does not fit 180; see
  [decisions.md](decisions.md#a-node-may-declare-a-wider-body).
- **The pointer says what a press does**: a hand over what clicks, a grab over what carries,
  a resize over what scrubs, a crosshair over what aims. The table is in
  [ui.md](ui.md#interaction-rules).
- Wires are 4 px data, 2 px dashed action, in a lighter and less saturated version of the
  port color.
- The editor background carries a **24 unit** dot grid. `GRID_PITCH` is 24 *world* units, so
  it is 24 px only at zoom 1; the world pitch quadruples whenever the on-screen pitch would
  fall below `MIN_GRID_SCREEN_PITCH` (18), which is below about 0.75 zoom. See
  [ui.md](ui.md) for why the dot count has to stay bounded. The dot is the main hue at low
  saturation with an alpha in the sixties — bright enough to read as a grid, because the grid
  is what says where the plane is and how far you have panned, and one you have to hunt for
  does none of that job.

## Fonts, and why four are vendored

`theme::fonts` builds the stack. **The text face is Space Grotesk**, Regular for text and
SemiBold as the family `theme::STRONG` for headings: `theme::ui_font` and `theme::strong_font`
name them.
Monospace (egui's Hack) is kept for what is read column by column — a path, a MIDI message,
the Status box — and nothing else.

Space Grotesk is cut by `scripts/text-fonts.py` from the variable face: an instance per weight,
its **digits pointed at the tabular figures its `tnum` feature names** so a live number never
shuffles sideways (egui has no shaper to turn on `tnum`), and the cut **text only** — Latin, punctuation and the figure
space. It goes first in the proportional family, and because it carries no arrows, shapes or
symbols, every icon still falls through to the face that drew it before.

egui supplies four faces — Ubuntu, Hack, a **subset** of Noto Emoji, and emoji-icon-font — and
that subset is the problem: 887 codepoints against the full face's 1,496. An icon outside it
draws as `◻`, silently, visible only to whoever opens that menu.

Four faces are vendored into `assets/fonts/`, all OFL:

| | | |
| --- | --- | --- |
| `SpaceGrotesk-Regular.ttf`, `SpaceGrotesk-SemiBold.ttf` | 29 KB each | the text face, cut and given tabular digits by `scripts/text-fonts.py` |
| `NotoEmoji-Regular.ttf` | 869 KB | the whole face. Icons in the library fall outside egui's subset |
| `NotoSansMath-Subset.ttf` | 87 KB | Noto Sans Math cut to the symbol blocks by `scripts/subset-fonts.py`. `∿` for Sine, and `⬓` — the half-height, worldspace's length unit, on every transform control |

Vendored rather than read from the system, so an icon that renders here renders everywhere.

**The two symbol faces are appended as the last fallbacks, deliberately.** epaint walks a family in order and
moves on when a face lacks a glyph, so every character that renders today still comes from
the face it comes from now, and these are reached only where the alternative is a box.
Inserting them earlier would silently restyle glyphs that already work.

**A variation selector is not free.** epaint's `invisible_char` list stops at U+206F, so
U+FE0F has no glyph and draws the replacement: `⚖️` renders as the scales *and* a box. Paste
the bare emoji.

`every_character_the_registry_draws_has_a_glyph` in `tests/ui.rs` is what keeps this true —
it walks every icon, label, tooltip, port key, unit and option choice in the registry. Its
oracle is `glyph_width`, not `has_glyph`: `has_glyph` is `resolve_face(c) != replacement_face`
and the replacement face is just the first one carrying `◻`, which is Hack — so it answers
"no" for most symbols Hack itself provides.

**An icon is drawn `theme::ICON_BUMP` — four points — larger than the text beside it.**
An emoji at the text's own size reads as small next to it: the glyph sits inside a square em
box with its own padding where a letter's fills the line, so the pair looks like a small
picture standing next to some words rather than one object. `theme::icon_font` is the size,
and it takes the zoom, so a node header keeps the ratio at every scale. It applies where an
icon is paired with text — a node's title bar, the Nodes menu's categories, the browser's
entries — and not where an icon stands alone at a size of its own, like the minimap's marks,
a node header's `?` or an asset card's placeholder.

**Color emoji are not available and are not worth chasing.** epaint has no COLR, CBDT or
sbix support; glyphs rasterize into one coverage atlas. Color would mean a second texture
and image draws, which break egui's batching every time an icon sits between two runs of
text. Monochrome also suits a UI that is one hue on near-black.
