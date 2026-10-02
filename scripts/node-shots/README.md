# node-shots

One PNG per node, for both synthesizers, with no window and nobody driving a mouse.

| | how | scale |
| --- | --- | --- |
| supersilvia | `examples/node_shots.rs`: a fresh `App` in egui_kittest per node, cropped to the node's body | 2 px per point (`PPP`) |
| silvia | `silvia_shots.py`: headless Chromium over the checkout served by `http.server`, one `SNode` per page | 2x device scale |

`shots.sh both OUT_DIR` runs the two and writes `OUT_DIR/{silvia,supersilvia}/<slug>.png`
plus an `index.jsonl` each: per node, every port, control (with its default and unit),
option and custom element, as read from the running thing rather than from source. Naming
slugs after `OUT_DIR` shoots only those: `shots.sh silvia OUT audioanalyzer`.

`SILVIA_PORT` is worth setting if 8080 is busy: the checkout is served there for the run, and
a server already on that port is what the page loads instead, with nothing saying so. silvia's
video nodes moved from `js/nodes/*.js` to `js/nodes/video/*.js`; both are read, and
`js/nodes/audio/` is left out of the default list because this comparison is the video graph.

For the numbers rather than the picture, `cargo run --release --example registry` prints the
whole supersilvia registry as JSON — every port's type, every control's default, range, step
and unit, every option's kind and choices, the regions and the CPU half.
`proposals/affordances/registry.json` is silvia's answer to the same question.

`shots.sh setup` once, for the Python side (a venv under this directory, playwright, and
playwright's chromium-headless-shell). The Rust side needs nothing beyond `cargo`.

kittest renders through wgpu with no glow paint callback, so an Output's preview is blank and
a device node shows what it shows with nothing open; everything else is exactly what the
window draws.
