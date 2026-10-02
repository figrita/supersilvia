# Proposal: a delay that names its Output

**Status: declined, 21 September 2026.** The parity review decided against it: variable-time
feedback is being removed, and there will be no variable time delay and no implicit
(contextful) sources. What stands instead is one frame back and nothing deeper — a
cable from an Output's Frame Out, which `camcordercrt` and `stargate` take as an ordinary
picture input. Everything below stays as the record of what was considered, not as work.

**It is not a deeper feedback.** silvia's frame history is a *time axis a field can address*
— every fragment may read a different moment — and the one node that used it that way is
`variabletimefeedback`. Read [what it is](#what-it-is-which-is-not-a-deeper-feedback) before
anything else here; the rest of the proposal only makes sense once that is clear.

## What left

`feedback` read `u_frame_history` — a ten-layer array every Output kept, blitted into once a
frame — at the previous index. It was the only node whose picture depended on the Output it
was compiled into, and that dependence is what made a tap under two Outputs read two
pictures. The array went with the node: only `feedback` read it, and the frame port reads
the separately published `output_texture`, so nothing else needed the layers.

## What it is, which is not a deeper feedback

`u_frame_history` is a `sampler2DArray`, and silvia's `variabletimefeedback` reads it at a
layer index **computed per fragment**:

```glsl
float delay_norm = <delayAmount>;                              // a float *input*: a field
float absolute_delay = mix(1.0, float(u_frame_buffer_size - 1), delay_norm);
float target_frame_index = mod(
    float(u_current_frame_index) - absolute_delay + float(u_frame_buffer_size),
    float(u_frame_buffer_size)
);
return texture(u_frame_history, vec3(screenUV, target_frame_index));
```

So every pixel may be reading a **different moment**. Noise into `delayAmount` is not a
trail, it is a time axis addressed by a field — which is the effect, and which one-frame
feedback cannot approach at any depth. `feedback`, `feedbackmix`, `stargate`, `geissflow`
and `camcordercrt` all read the same array at a *constant* index; this is the one that
reads it at a varying one.

## The two numbers are different numbers, and only one of them allocates

This is the whole design, and getting it wrong thrashes the GPU.

| | what it is | where it lives | cost of changing it |
| --- | --- | --- | --- |
| **How many frames are kept** | `u_frame_buffer_size`, silvia's `frameHistorySize` | a **value on the Output**, an `s-number` 1–120 | `_initFramebuffers()`: the whole array is reallocated |
| **How far back this fragment reads** | `delayAmount`, normalized **0 to 1** | a **`VaryingNumber` input** on the reading node | nothing: an index into what is already there |

**The depth read is normalized, not a frame count.** `mix(1.0, size - 1, delay_norm)` maps
0..1 onto whatever is allocated, so a patch means the same thing at any buffer size and a
cable into it can never index past the end. A `frames` count would couple the two numbers
and make every patch's delay change meaning when the buffer did.

**The allocation is not modulatable, and silvia says so in the markup.** The Frame History
`s-number` is the one control in the node carrying `midi-disabled`. A value a hand can drag
deliberately, and nothing else can reach: dragging it reallocates, and a control that
reallocates is not one to put a sequencer on.

## The shape, if it is wanted

A **`delay`** node, `Category::Source`, that **names its Output** the way an asset option
names a file: an option listing the Outputs in the project, and a **`VaryingNumber`**
`depth` input, normalized 0 to 1, defaulting to 0.5 as silvia's does. Its output is a
`VaryingColor` texture of that Output as it was, per fragment, that far back. It is
context-free — the same picture in every program it is compiled into — because the Output is
chosen on the node and not by the consumer.

The array comes back on the Output, sized by a **value** in `Node::values` and allocated only
while some `delay` names that Output. Four costs to face before building it:

- **Zero flash.** Allocation happens on the frame the value changes, and the layers that have
  not been filled yet read the last completed frame rather than black.
- **The VRAM figure is the control's price tag, and the two arrive together.** silvia's own
  `w*h*4*(historySize + 2)` at 720p is 42.2 MB at ten layers and 429 MB at its maximum of
  120. supersilvia's chain is `RGBA16F`, so matching its precision doubles that again: near two
  gigabytes at 1080p. A control that can allocate a gigabyte without saying so is not a
  control. It is counted from what is allocated rather than calculated from the resolution,
  since half of an Output's targets come and go.

  **It goes on the cost strip, not in the node body.** silvia centers its figure under the
  Frame History number because silvia has nowhere else to put it — no cost strip and no
  Status box. Copying that placement would be copying a constraint this editor does not have.
  A VRAM figure is a *measurement*, and `View ▸ Costs` is where measurements live and is off
  by default, which is the right default for one. It appends to the Output's own strip beside
  the GPU time; the bar and the `hot` flag stay the vsync fraction, because a standing
  allocation is a stock and the bar measures a rate.
- **Which Output's resolution.** The delay samples the named Output's layers through
  `textureSize`, as the frame port does, so an Output of one size can be delayed into
  another.
- **Precision against depth.** `RGBA16F` layers are twice the bytes of silvia's `RGBA8` ones
  and the ring is where the bytes are. Whether the history is the one place in the chain that
  drops to 8 bits is a question to decide, not an assumption — it is the difference
  between 120 layers and 60.

The frame port stays the one-frame case. A `delay` at depth zero is the frame port with a
second node in the way, and that is fine: the node exists for the varying index, not for
feedback.
