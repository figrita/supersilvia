# silvia and supersilvia, node by node

What a person notices using each node in both programs, and what to do about it. The
entries were reviewed with both screenshots side by side; this file is the record that lives
with the code. An entry names knobs and
menus by their labels and says what differs for a person, not how the code differs.
Numbers that differ are folded into a table at the end of each entry, silvia first.
Captured with `scripts/node-shots/shots.sh both`, last on 21 September 2026; the numbers
on this side are checked against `cargo run --release --example registry`, and the rule
behind what is not an issue here is
[docs/decisions.md](../docs/decisions.md#a-number-stays-varying).

Every entry's proposal stands until it is decided, and a decision that has been made leaves
this file along with the node it was about.

2 entries: 1 paired, 0 silvia only, 0 supersilvia only, 1 about every node.

**One hundred and eleven entries left on 21 September 2026 because there was nothing to build under
them**: the ones whose answer was *leave it alone* (`autogain`, `cosine`, `fisheye`, `sine`,
`tap`, `time`, `triggeredrandom`, `wave`, `whirlandpinch`, `zoom`), the ones passed
over (`autoexposure`, `audioanalyzer`, `chrome-output-band`, `oscillator`, `posterize`,
`scatter`, `triggeredcolor`, `tunnel3d`), the eleven Math nodes built the same day (`abs`,
`atan2`, `ceil`, `floor`, `lerp`, `modulo`, `power`, `pythagorean`, `random`, `smoothstep`
and `threshold`, which landed as `src/nodes/math.rs` and `src/nodes/random.rs`), the two whose
one change had already landed (`sample` and `color` both publish a uniform color),
`variabletimefeedback`, which is being removed rather than built, `feedbackmix`,
which at one frame back is a cable from an Output's Frame Out into a Mix and both of those
exist, and `animation`, whose only issue was the trace band and is now [trace.md](trace.md).
The same day the whole Transform family left too, built: the accepted changes to `rotate`, `tile`, `repeater`, `kaleidoscope` and `domainwarp`, and ten new nodes (`translate`, `rotozoom`, `mirror`, `stretchskew`, `perspective`, `polarcoords`, `shakycam`, `regionabsolute`, `regionsized`, `wallpaper`). The Color package left the same day too, built: `saturate`, `vibrance`, `wavefold`, `palette`, and `simplelight` with `heighttonormal`. The Effect kernels left the same day too, built: the accepted changes to `bloom`, `dilate`, `erode` and `edgedetection`, and `kuwahara`, `motionblur`, `radialblur`, `sincfilter` and `supersampling` new. The Effect manglers left the same day too, built: the accepted changes to `mosaic`, `colordither` as a Pattern and a Scale on Posterize rather than a node, and `pixelsort`, `colorize`, `colormapping`, `colorshift`, `chromaticaberration`, `chromakey` and `stargate` new. The Distort and Convert packages left the same day too, built: `glitch` new, `hue` and `saturation` with silvia's icons, and `sliderule` new beside `reframerange`. Mix and three small ports left the same day too, built: `muxevent`'s Random and whole-number index, and `muxnumber`, `randomhurl`, `worldcoordinates` and `number` new. Control and Games left the same day too, built: `adsr`'s curves and caption, `slew`'s Ease, `phase`'s Hold, `counter` and `smoothcounter` at silvia's fade, `bpmclock`'s Start/Stop, triplet and tap, `clockdivider`'s status line, and the field drawn on `brickgame` and `cellularautomata`. Feedback, Output and Debug left the same day, built: `camcordercrt` as one node with its viewfinder and last frame on a cable, `geissflow`, Supersampling on the Output's render section in place of `offlineoutput`, and `note` dragged wider. Source and two Input entries left the same day, built: `text` on GStreamer's own rasterizer with the note's text box, `screencapture` as a node beside the camera node, a dropped still landing on `imagegif` with its picture on the node and a clip saying "Preparing clip…" with a clock, and `maininput`'s meters in place of its preview. The pointer and a controller, and the arithmetic, left the same day, built: `mouseinput` with its framing option, `gamepad` on gilrs, `subtract`, `multiply` and `divide` naming their operation on the row and resting at silvia's numbers, and `reframerange` carrying the named ranges on the node under Convert. Time and the two custom bodies left the same day, built: `clock` with an offset knob, `automation` keeping its recording in the node's own values, `cosinegradient`'s twelve numbers as silvia's grid on the node with no ports, and `euclideanrhythm`'s pattern drawn on the node with the per-lane ports gone. The Output and Random Fire left the same day, built: a lossless PNG Snap and the render section under a heading on the Output, and every action port throbbing when it fires, which was Random Fire's answer. The Step Sequencer left on 29 September 2026, built: silvia's four lanes of sixteen cells clicked on and off, with Clear under them, drawn in Euclidean Rhythm's grid and run by its transport. `xypad` left the same day, built: silvia's puck with its physics, its wells and its nine presets on a pad in the node's body that claims the pointer, the ten knobs on rows with ports, and X and Y under the pad as the node's own numbers. `drawingcanvas` left on 29 September, built: the paint surface on the region contract, silvia's six tools, keys and eight symmetries, and the painting saved with the project as a picture file, which silvia never kept.


## Node chrome


### ⏱️ Time is a row here and thin air in silvia (simplex, every node)

Every generator that moves carries a Time row that silvia's does not, and whether that row belongs on all of them is the open question.


- **Whether time should be a row on every node that moves** (borderline). With the row there, time is something the patch can hold rather than something in the air: run one generator at half the speed of another, freeze a branch, scrub the whole thing from one knob, or hand it a number that varies across the picture so the left of the frame is a second behind the right. It costs nothing when it is left alone, because an untouched Time row reads the clock, and it replaces silvia's habit of multiplying the clock by a speed, which jumps the picture the moment the speed changes.

Without it, a generator looks exactly like silvia's: a Time Speed knob and nothing else, one less row to read past on a node that already has six, and one less port on the hit-testing edge. Time stays the same for everything, which is what it is almost every night, and the price is that warping it stops being something a cable can do, so the one patch that wanted a branch running backwards has nowhere to put that. *Suggest:* Decide whether the row stays on every generator, or only on the few where a hand would reach for it.


What this one does that silvia's did not:

- A node with a speed of its own keeps its own running phase, so turning the speed knob slides the picture instead of jumping it.
- The row reads the clock while nothing is connected, so a node left alone behaves exactly as silvia's does.

At the defaults a generator looks the same and moves the same in both, and Time Speed keeps silvia's own default and range.


**Keep as is, effort S: Keep the Time row while the question is open.**

- Keep: The Time row on the generators, reading the clock while nothing is connected.

This one is not decided. The row buys time-warping, scrubbing and time that varies across the picture; it costs a row on every node that moves, on nodes that already run long. Leaving it as it is changes nothing and keeps both answers available.


| what | silvia | supersilvia |
| --- | --- | --- |
| Time | no row; the clock is read inside the node | a row, reading the clock while nothing is connected |
| Time Speed | 0, 0 to 5, step 0.01 | the same |
| Phase | not published | published on a row of its own |


## Generate


### 🧫 Slime Mold (slimemold, paired)

The same simulation with the same rules, the same nine presets and the same Randomize, but silvia puts it on the node where you can watch it and poke it.


- **You cannot see the simulation on the node** (matters). silvia draws a square of the scent field right on the node, big enough to see veins in, so you turn Sense Angle and watch the network reorganize under your hand. These are knobs you tune by looking, not by reading. Here nothing shows until you have wired Output into an Output node and put that on a deck, and if you are building the patch around the simulation you are tuning it blind. The node already publishes a picture of the field on its Trail row, which is the same thing the video node draws on its own body. *Suggest:* Draw the trail on the node as a picture region under its own heading, the way a source draws its picture.

What this one does that silvia's did not:

- Grid Scale and Population say what they actually are, 192x192 and 20%, instead of a 12 with a times-sixteen label beside it and a bare percent.
- The scent field comes out on its own Trail row, so you can color it yourself or feed it somewhere else instead of taking the three-color mix as given.
- Density says on its row that it is the field multiplied by Heatmap, so you know what the number means before you patch it.
- silvia's eleven node-local numbers are all knobs with ports here, so a clock can drive Speed and an envelope can drive Decay.

Same rules and the same paper behind them, same defaults on all eleven numbers, same Attract and Repel, same Trails and Agents ticks, silvia's nine presets numbered under the rows, and a Randomize that rolls Sense Angle, Turn Angle and Sense Dist to silvia's ranges and nudges the world as silvia's does. The world wraps in both, and the picture tiles across the frame rather than mirroring at its edges, which is what keeps a wrapping world from showing a seam it does not have.


**Partial, effort S: Show the simulation on the node.**

- Draw the trail on the node as a picture region under its own heading, so the knobs can be tuned by watching.
- Keep: Grid Scale and Population as menus reading real sizes.
- Keep: The Trail output, which silvia keeps to itself.
- Keep: The wrapping, tiled field.

Every rule, every default, the presets and Randomize already match. What silvia has over it is the thing that makes a simulation tunable by hand: a picture to watch.


| what | silvia | supersilvia |
| --- | --- | --- |
| Field on the node | a 300 square you can drag on to push agents | nothing drawn |
| Push | 3.0, 0 to 20 | none, and nothing to push with |
| Grid Scale | 12, labeled times sixteen | 192x192, a menu |
| Population | 20, a percent | 20%, a menu |
| Outputs | Output and Density | Trail, Output and Density |
