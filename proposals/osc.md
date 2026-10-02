# Proposal: OSC

**Status: proposed, not yet decided.** Users of other VJ tools will ask for OSC before long:
it is how a phone or a tablet becomes a control surface, and how one app drives another. This
says what OSC and its discovery layer, OSCQuery, are, how VJ software uses them, what supersilvia
already has to hang it on — the MIDI map above all — the address space, both directions, the
crates, the threads, the network and its risks, the look in the app, and the open questions.
Everything here was read in supersilvia's code and docs, OSC's specifications,
other projects' documentation, crates.io and Apple's technical note on local network privacy;
nothing was built or run. A claim that could not be checked that way says **(unverified)**.

## What OSC is

**OSC is MIDI's idea — a message saying "this control is now this value" — over the network,
with names instead of numbers.** Where a MIDI knob sends *CC 21 on channel 3, value 97*, an OSC
fader sends *`/oscillator4/frequency` 0.76*: an address that reads like a file path, and one or
more typed values. It was designed at CNMAT, UC Berkeley, by Matt Wright and Adrian Freed
(1997; the 1.0 specification is from 2002). It has no licence to speak of, no vendor and no
SDK: it is a byte format, and every language has a library.

**Why people want it.** Three reasons, in the order a VJ meets them:

1. **A phone or a tablet as a control surface.** TouchOSC or Open Stage Control draws faders,
   buttons and XY pads on an iPad, and each sends an OSC message over Wi-Fi. No hardware to
   buy, as many controls as fit on the screen, laid out for this set.
2. **Apps driving each other.** Resolume, TouchDesigner, VDMX, MadMapper, Millumin, QLab,
   Ableton Live (through Max for Live) and most lighting software speak it. A cue in one fires a
   change in another, on the same machine or across the room.
3. **Scripts.** Three lines of Python send an OSC message; a sensor, a game engine or a
   generative score can play the patch.

**The words.**

- A **message** is an **address** (`/mixer/balance`), a **type tag string** saying what the
  values are (`,f` for one float), and the values. Every part is padded to four bytes.
- **Types.** The four every implementation must read are `i` (32-bit integer), `f` (32-bit
  float), `s` (string) and `b` (blob of bytes). The common extras are `T`/`F` (true, false,
  with no bytes), `N` (nil), `I` (impulse, "bang"), `h` and `d` (64-bit integer and float),
  `r` (a 32-bit RGBA colour), `m` (four MIDI bytes) and `t` (a time tag). OSC 1.1 made `T`,
  `F`, `N`, `I` and `t` required, and renamed `I` from *Infinitum* to *Impulse*.
- An **address pattern** may hold wildcards, so one message can reach many controls: `?` one
  character, `*` any run, `[1-4]` a set, `{bass,mid}` alternatives. OSC 1.1 adds `//`, any
  number of levels. The *receiver* matches; the sender sends one pattern.
- A **bundle** is several messages (or bundles) sent as one packet, with a **time tag**: the
  moment they should all take effect, as an NTP timestamp. The tag `1` means *immediately*,
  which is what nearly every sender sends **(unverified as a survey)**; OSC 1.1 deliberately
  leaves the meaning of any other tag unspecified. A bundle's messages are meant to apply
  together.
- **Transport.** Almost always **UDP**: one packet per message or bundle, no connection, no
  delivery guarantee, nothing to set up. A packet is best kept under about 1,470 bytes, one
  Ethernet frame, since a fragment lost loses the whole packet. Over **TCP**, OSC 1.0 frames
  each packet with its size in front; OSC 1.1 requires **SLIP** framing (RFC 1055, double
  `END`) instead, and the two do not interoperate — TouchOSC offers both. VJ tools use UDP.
  There is no formal 1.1 document: the 2009 paper by Freed and Schmeder is the reference.
- **Sender and receiver**, sometimes *client* and *server*. A receiver listens on a **port**;
  a sender needs the receiver's IP address and port typed in. That typing is OSC's usability
  problem, and OSCQuery's reason to exist.

**OSC against MIDI**, since supersilvia has MIDI:

| | MIDI | OSC |
| --- | --- | --- |
| Address | channel and number, 16 × 128 | a path, any length, human-readable |
| Value | 0–127 (14-bit with care) | 32-bit float, int, string, colour, several at once |
| Wire | USB, a cable | the network: UDP, any machine, Wi-Fi |
| Discovery | the OS lists devices | none in OSC itself; OSCQuery adds it |
| Learn | the only way: a knob has no name | needed for controller apps; a script can type the address |
| Reliability | a cable does not drop a message | UDP may drop one, and does on busy Wi-Fi |
| Security | a cable is physical access | none: anyone on the network who knows the port |

### How VJ software uses it

- **Resolume** (Arena and Avenue) has a **fixed address space for everything in the
  composition** — `/composition/layers/1/clips/1/connect` launches a clip,
  `/composition/layers/1/video/opacity` fades a layer, `/composition/master` the master — with
  floats **normalised to 0–1** across each parameter's range — opacity 0.25 is 25 %, rotation
  0.5 the middle of −180…180 — an int parameter taking a float the same way, and relative
  writes with a `+`, `-` or `*` prefix. It listens on port **7000** by default; its output
  port is set beside it (7001 by custom **(unverified as Resolume's own default)**) and sends
  parameter changes, so a TouchOSC fader follows. **Shortcuts ▸ Edit OSC** shows the address
  of whatever is clicked, and any incoming address can be **learned** onto a parameter, as MIDI
  is. Its monitor keeps the last two hundred messages. Resolume does **not** answer OSCQuery
  **(unverified absence; its manual never mentions it)**: its discovery is a REST API and a
  WebSocket of its own on port 8080.
- **TouchDesigner** has *OSC In* and *OSC Out* operators (CHOP for numbers, DAT for raw
  messages): the patch decides what an address means. No native OSCQuery; a community
  component serves one from its Webserver DAT.
- **VDMX** has OSC in and out and is both an OSCQuery **server** and **client** — its maker,
  Vidvox, wrote the proposal — so it can build controls from another app's address space.
- **MadMapper** listens for OSC, typically on port **8000**, with a fixed address space
  (`/master/master_level`, `/media/<name>/select`), a **Copy OSC Address** on a parameter's
  right-click, and an OSCQuery server on **8010**.
- **Millumin** listens on port **5000** by default, with an address space for its layers,
  columns and actions (`/action/launchColumn`), and says it supports OSCQuery.
- **Controller apps.** **TouchOSC** (hexler; proprietary, a few euros, every platform) is the
  one most VJs own: a layout editor, then faders and buttons that send an address the user
  types, 0–1 by default, and receive the same address back to follow. It can find OSC
  receivers by Bonjour but does **not** read OSCQuery. Its stock layouts send addresses like
  `/1/fader1` **(unverified for the current TouchOSC; true of the original)**, which is why
  learn matters: nobody retypes a layout. **Open Stage Control** is the open-source one
  (GPL-3.0, now on framagit, runs in a browser, so any phone); whether it reads OSCQuery
  **(unverified)**.
- **Ableton Live** has no OSC of its own; Ableton's free **Connection Kit** for Max for Live
  has *OSC Send* (any Live parameter out as OSC), *OSC TouchOSC* and *OSC Monitor*, over Max's
  `udpsend`/`udpreceive` — which is how a DJ's tempo and cues reach a VJ's machine.
- **Sequencers and bridges** — **Chataigne**, **ossia score**, **Vezér** — are OSCQuery
  clients: they list an app's parameters and animate or route them. They are where OSCQuery
  pays off.

**What users expect of an app that "supports OSC"**, from those apps' own documentation:

1. A port to listen on, a switch to turn it on, and nothing else to set up to start.
2. **Every performable parameter has an address** they can type or find, readable and stable.
3. **Learn**: move a fader in TouchOSC and the control under the mouse answers to it — exactly
   the gesture supersilvia already has for MIDI.
4. **Feedback**: what they drive sends its value back, so the fader on the tablet follows.
5. **Out**: the app's own signals — levels, beats, triggers — can be sent to another app or to
   the lighting desk.
6. **A monitor** showing what arrives, because the first question is always "is it getting
   anything?".
7. Increasingly, **OSCQuery**: the tablet or sequencer lists the parameters by itself.

## OSCQuery

**OSCQuery is the missing half of OSC: a way to ask an app what addresses it has.** It is a
proposal by Vidvox, not a ratified standard, last changed in 2018
(github.com/Vidvox/OSCQueryProposal), and widely implemented through the ossia project's
`libossia`. An app that "has OSCQuery" runs three small things beside its OSC port:

- **An HTTP server** whose answer to `GET /` is the whole address space as a JSON tree. Each
  node carries `FULL_PATH`, `CONTENTS` (its children), and for a parameter `TYPE` (an OSC type
  tag string, `"f"`), `VALUE`, `RANGE` (`MIN`/`MAX`, or `VALS` for a set of choices), `ACCESS`
  (0 none, 1 read, 2 write, 3 both), `DESCRIPTION`, and optionally `UNIT` and `CLIPMODE`.
  `GET /oscillator4/frequency` answers that subtree; `?VALUE` or `?RANGE` one attribute.
  `GET /?HOST_INFO` answers the app's name, its **OSC port and transport**, and which of the
  optional extensions it supports; a missing path is a 404.
- **Optionally, a WebSocket** on the same port: a client sends `{"COMMAND":"LISTEN","DATA":
  "/oscillator4/frequency"}` and is pushed that value, as raw OSC in binary frames, whenever it
  changes; the server pushes `PATH_ADDED`, `PATH_REMOVED`, `PATH_RENAMED` and `PATH_CHANGED`
  when the tree changes. That is how a sequencer's view stays true while nodes come and go.
- **Bonjour (mDNS) advertising** of the HTTP server as `_oscjson._tcp`. The proposal names only
  that; VRChat's library also advertises the OSC port as `_osc._udp`, the type the OSC 1.1
  paper suggested, and that is what TouchOSC's Bonjour browsing would find.

**Who speaks it**: as servers, VDMX, MadMapper, Millumin, ossia score and **VRChat**, which
adopted it in 2023 for its avatar parameters and brought it a crowd of hobbyist tools; as
clients, VDMX, Chataigne, ossia score and Vezér. **Not** Resolume, TouchDesigner (but for a
community component) or TouchOSC. So OSCQuery makes supersilvia a first-class citizen for
sequencers and bridges, and changes nothing for the phone in the VJ's hand — learn does that.
Chataigne once ignored `HOST_INFO`'s `OSC_PORT` (its issue 65, fixed 2020), which is the kind of
thing to try against it.

## What supersilvia already has

Almost the whole receiving side, because **MIDI already solved it**, and OSC only changes the
wire. What is there, read in the code:

- **A reader thread that never blocks the frame, into the synth's queue.** `midi/`'s reader —
  a thread blocked on the ALSA sequencer on Linux, CoreMIDI's callback through `midir` on macOS,
  with a `midi watch` thread wiring in late devices — posts each parsed `midi::Message` into a
  `std::sync::mpsc` channel. The receiving end is the **synth's**: `Msg::MidiIn(rx)` hands it
  over, and `Synth::take_midi` drains every queue at the top of each tick with `try_iter`.
- **One map from a trigger to a target.** `midi::Bindings` is a `BTreeMap<Trigger, Binding>`;
  a `Target` is a `PortRef` — a number control, keyed by its input port, or an action input,
  which *is* a port — or `Target::Balance`, the Main Mixer's fade. The editor publishes only the
  **live** half (`Bindings::live`, the bindings whose control the graph has) through
  `MidiDesk::publish` as `Msg::MidiMap`, so the tick reads a copy and never one mid-edit.
- **How a CC reaches a control.** In `take_midi`, a `Control` message bound to a port goes to
  `midi_control`, which scales 0–127 linearly across `nodes::control_range` — the instance's
  own range where it has one, the definition's otherwise — and calls `Synth::write_control`.
  That writes the value onto the synth's graph at once (the world moves on that tick, editor
  painting or not), and records it in `midi_owned` against what the editor last published. The
  write rides out on the snapshot as `midi_writes`; `MidiDesk::take` hands them to `App`, which
  applies each as the `Command::SetControl` a hand would have sent, so the file and the undo
  history say what the knob did. A **barrier** keeps a write made before the editor saw a hand's
  drag from undoing the drag; a knob's undo step closes on **silence** (`SETTLE`, 0.2 s),
  since a knob has no release. A note bound to an action input holds and releases the same
  button a hand would (`midi_note`, into `held_apart`), counted per trigger. A CC on the fade
  is −1 to +1, written to the synth's balance and landed by `Mixer::set_balance`, not a command.
- **Learn.** `Alt` + click on any number control, action button or the A / B balance
  (`MidiDesk::learn`) arms a wait: the control breathes in the accent (`ui::learning_ring`),
  loses its old binding at once, and the synth drives nothing (`Msg::MidiLearn(true)`) until
  the **first message read after the ask** (`midi_learn_from`) binds it. `Escape` cancels.
  **Bind MIDI…** in the range editor is the findable form of the same gesture. A bound control
  wears a dot and its accessible name says `bound to CC 21 ch 3`.
- **The map is project data**, rows in `project.ssp` as `{trigger, target}` with a port as
  `{node, key}` and the fade as `{"mixer": "balance"}`; a deleted node's bindings stay for an
  undo, and **node ids are never reused**, so a binding left behind never drives another node.
- **Project ▸ MIDI…**, a window with devices, mappings (node as a link, control, trigger, last
  value, `✕`) and a **monitor** that keeps two hundred messages and only while watching.
- **Stable names for every control.** A node's name is `{slug}{id}` (`ui::NodeName`,
  `checkerboard12`) and a control's accessible name `{slug}{id}.{key}` (`ControlName`) — the id
  never reused, the slug and key stable identifiers with alias tables for the few renamed ones.
  **That is already an address space**; OSC would write it with slashes.
- **Values to send.** Each tick, the synth holds every `UniformNumber` output in
  `uniforms: HashMap<PortRef, f32>` and every event fired in `actions`, keyed by the output
  port — the audio bands (`bass`, `mid`, `high`) and their events (`bassEvent`, `midEvent`,
  `highEvent`) on `audioin`, `video` and `maininput`, taps, oscillators, `bpmclock` edges. The
  audio thread already crosses thresholds with the sample they happened on.
- **Reasons to stay awake.** A node on a closed workspace is suspended and does not tick;
  a deck and a published Output are "on air" and keep what feeds them live
  (`link::live_nodes`, `Why::Syphon`). A sent output will need the same kind of reason.
- **The command bus** for everything else, and the one-parameter-address-space decision
  (`docs/decisions.md`, *A timeline and a viewport*): "automation, graph uniform numbers, MIDI
  and direct manipulation are all sources that write to addresses". OSC is a fifth writer.

## How a message would reach a control

The same road as a CC, with an address in place of a trigger:

1. **A thread named `osc`** blocks on `UdpSocket::recv_from` (with a read timeout so it can be
   told to stop), parses the packet, and posts an `osc::Message { address, args }` — each
   message of a bundle, together — into an `mpsc` channel handed to the synth as `Msg::OscIn`.
2. **The synth drains it** at the top of the tick, beside `take_midi`, and resolves each
   address against a table the editor published (`Msg::OscMap`): the **automatic** addresses
   of the live graph, plus **learned** ones. An address with wildcards is matched against the
   table; one without is a hash lookup.
3. **It acts exactly as a CC or a note does**, through the functions that already exist: a
   number onto a number control goes to `write_control` (scaled — see *Values*), a value onto an
   action input to `midi_note`'s hold-and-release (renamed for both), a number onto the fade to
   the balance write. So the barrier, the silence that closes an undo step, the document
   catching up through `SetControl`, and "nothing moves while learning" are inherited, not
   rewritten.
4. **Every message rides out on the snapshot** in a log of its own, for the monitor and for
   learning, as MIDI's do.

**Learn works the same way**, and needs almost nothing new: `Alt` + click arms the wait, the
synth drives nothing, and the first OSC message read after the ask binds its **address** to the
control. One difference: OSC is chattier than a knob, and a sender that streams (TouchDesigner
sending a CHOP every frame) would bind itself to whatever was clicked. So an OSC learn takes the
first address that arrives **with a changed value** after the ask **(a guess at the right rule;
the monitor will show whether it is needed)**.

**One map or two.** `Trigger` is `Copy` and ordered; an OSC address is a string. Either
`Trigger` gains an `Osc(Arc<str>)` arm and loses `Copy`, keeping one map, one window table and
one learn path — the argument `docs/decisions.md` made for the fade — or OSC keeps a map of its
own with the same `Target`. Recommended: **one map**, with *learning moves rather than doubles*
applied **per family**, so a control can answer to a MIDI knob *and* a TouchOSC fader, which
is a rig people have.

## The address space

**Every control a MIDI binding can reach has an automatic address**, with no learn:

| address | what it is |
| --- | --- |
| `/<slug><id>/<key>` | a node's number control or action input: `/oscillator4/frequency`, `/output2/show_a`, `/output2/snap` |
| `/mixer/balance` | the Main Mixer's A / B fade, −1 to +1 |
| `/<slug><id>/<key>` (out) | a node's `UniformNumber` or action output, when sent: `/audioin3/bass`, `/bpmclock5/beat` |
| `/maininput/bass`, `/mid`, `/high`, `/volume`, `/bassEvent`… (out) | the Main Input's analysis, sent with no node needed |

**Why the node's name and not the workspace.** The obvious reading is
`/supersilvia/<workspace>/<node>/<control>`, and it is wrong here for three reasons in the code:
a workspace is **a view, not an owner** — "it owns no node" (`graph/workspace.rs`) — and one node
can be on several, so it would have several addresses; a workspace can be **renamed**, and its
name need not be unique; and names hold spaces, which OSC addresses may not
(OSC 1.0 forbids space, `#`, `*`, `,`, `/`, `?`, `[`, `]`, `{`, `}`). Nodes, meanwhile, have
**no user names at all**: `checkerboard12` is what the node is called in the accessibility
tree, in tests and in the egui MCP, and `checkerboard12.frequency` is already every control's
accessible name. The OSC address is that name with slashes. Recommend no `/supersilvia` prefix
on what is received — the port already says which app — and one on what is **sent**, where a
lighting desk hears several apps on one port (`/supersilvia/audioin3/bass`), as a setting.

**What renames and deletes do.**

- A node cannot be renamed, so its address never changes while it lives.
- **Delete** kills the address; a message to it is dropped and the monitor says *no such
  address*. **Undo** brings it back, since the id comes back and is never reused: the rule the
  MIDI map already leans on.
- **Duplicate and paste** make new ids, so new addresses: a copy does not answer to the
  original's fader, which is what a hand would want.
- **Import** remaps a workspace's ids onto this project's, so an imported patch's addresses are
  new ones. Learned bindings do not travel with a workspace (the map is project data), exactly
  as MIDI's do not.
- **A slug renamed** in a later release is read through the slug alias table on the way in, as
  a file is, so an old TouchOSC layout keeps working.

**Values.** The single decision users will feel:

- **Normalised 0–1 across the control's own range** at the automatic address — Resolume's
  convention, TouchOSC's default, and MIDI's own rule (0–127 across `control_range`, so narrowing
  a range narrows every controller). Recommended. A `log` control scaled linearly is MIDI's
  behaviour today; both should honour `log` together, later.
- **Control units** (the number the control shows) are what a script and an OSCQuery client
  want: recommended under a sibling, `/<slug><id>/<key>/value`, and advertised with its real
  `RANGE`. **(Owner's question 3.)**
- **Int or float** are both accepted and read as numbers; `T`/`F` as 1/0.
- **An action input** takes `1`/`0` (or `T`/`F`) as down and up — a TouchOSC button sends
  exactly that, so it holds as a note does — and a message with **no value or `I`** as a
  press and release a tick apart, the one-frame gate the event half already uses.
- **A colour control** (`ControlValue::Color`) could take `r` or four floats; MIDI never
  reached colours. Recommend later, with the agent layer.
- **Options** (selects) are structural, and some rebuild a shader. OSC does not reach them, as
  MIDI does not; the agent layer does.

## Receiving

What arrives, and from whom: a TouchOSC or Open Stage Control layout (learned or typed
addresses), Resolume or TouchDesigner sending cues, Ableton through Max for Live, a script.
All of it is the path above. Two cases need a rule:

- **A stream.** Another app may send a value every frame for an hour. Each still writes the
  control and goes through the bus as `SetControl`, as a knob's does, so the silence that closes
  an undo step never comes and every hand edit meanwhile joins the stream's one step. That is
  the MIDI rule applied to a sender MIDI never had; the alternative is to treat OSC writes as
  playing rather than editing — the world moves, the document does not — which is how
  `docs/decisions.md` treats an automation lane. **(Owner's question 4.)**
- **A flood.** A script can send ten thousand messages a second. Per tick, only the **last
  value per control** is applied (a level stale by one message is still a level), while
  presses and releases are kept in order (a release dropped is a gate that never closes — the
  reason MIDI uses a queue). The channel is bounded (`sync_channel`), and a full one drops and
  counts, and the count is shown in the monitor.

## Sending

The half MIDI does not have. What is worth sending:

- **The audio analysis.** The Main Input's `bass`, `mid`, `high` and `volume` levels and their
  `Down`/`Up` crossings are the natural first thing: a lighting desk or another visual app
  wants the same kick supersilvia sees. Sent with no node needed, by a tick in the OSC window.
- **Any `UniformNumber` or action output**, by a choice on the port — a tap's brightness, an
  oscillator, a `bpmclock`'s beat, a `counter`. A sent output keeps its node awake as a deck
  does (a new reason beside `Why::Syphon`'s), since a suspended node publishes nothing.
- **Feedback** to controllers: the value of a control that an OSC sender drives, sent back to
  it when it changes by any other hand, so the tablet's fader follows. Resolume does this by
  sending every change to its output port; OSCQuery clients do it with `LISTEN`.

**How.** A level is sent **when it changes**, at most once per tick, as floats in its own
units (a band is already 0–1); an event as a message with `1` on down and `0` on up (or `I`
where the receiver wants a bang). One **bundle per tick**, time tag *immediately*, so a
receiver takes one tick's values together. **The synth never sends**: it leaves what changed on
a latest-wins map for levels and a queue for events, and a thread named `osc out` drains them
and writes the socket, since a `send_to` on a full socket buffer can block. Targets are
`host:port` pairs typed in the window, several allowed, resolved on that thread, never on the
synth.

## Crates

- **`rosc`** 0.11.4 (March 2025, MIT or Apache-2.0, about half a million downloads) — the de
  facto Rust OSC crate, which `nannou_osc` wraps. It encodes and decodes messages and bundles
  with NTP time tags, every 1.0 extended type and the 1.1 required ones (`I` under its old name;
  `S` not decoded), UDP packets and TCP with 1.0's size prefix, and has a pattern matcher,
  `rosc::address::Matcher`, for `?`, `*`, `[…]` and `{…}`. Missing: 1.1's `//` wildcard and SLIP
  framing, both small. Quiet for eighteen months, not abandoned. It depends on `nom` 7 and
  `byteorder`; the lock holds `nom` 8 only, so it adds a second `nom`.
- **Hand-rolled** is the real alternative: the format is four-byte-padded strings and
  big-endian numbers, a few hundred lines with the matcher, no second `nom`. It costs the edge
  cases `rosc` has already met. Recommend `rosc`, and hand-rolling if a second `nom` is not
  wanted **(open question 8)**.
- **Sockets** are `std::net::UdpSocket` — no async runtime. The same on Linux and macOS, so
  `osc/` needs **no `platform/` backend**, unlike MIDI. It is a new top-level module beside
  `midi/`, and belongs with the pure modules' rule: no graphical dependency.
- **OSCQuery** has no crate that fits. `oscquery` 0.2 has no repository and no mDNS; `oscq_rs`
  is stale; `vrchat_osc` 2.2 is active but VRChat-shaped; every one of them brings `tokio`,
  `hyper` and `axum`, a runtime the Mac build does not have. What OSCQuery needs is small: an
  HTTP/1.1 `GET` server on a `TcpListener` answering a JSON tree — `serde_json` is already a
  dependency, and the tree is the automatic address table with ranges from `control_range` —
  written by hand, since `tiny_http` has not released since 2022; plus, for `LISTEN`,
  `tungstenite` 0.30, a synchronous WebSocket over any stream, which fits a plain thread.
- **mDNS.** supersilvia advertises nothing today: NDI's discovery is done inside NDI's own
  runtime. **`mdns-sd`** 0.21 (September 2026, MIT or Apache-2.0, maintained, calls itself beta)
  is pure safe Rust, advertises and browses on macOS and Linux, and runs its own thread with no
  async runtime — one code path, no `unsafe` — though it brings `mio`, `socket2`, `if-addrs` and
  `flume`, and whether it shares port 5353 cleanly with Avahi and mDNSResponder already on it
  is **(unverified)**. The alternatives are the systems' own responders: `zeroconf` (Bonjour
  on macOS, Avahi on Linux, needing Avahi's development libraries), or `dns_sd` by hand in
  `platform/macos/` and Avahi over D-Bus (`zbus` is already in the tree) in `platform/linux/`.
  Recommend `mdns-sd`, and a `platform::mdns` service only if it misbehaves beside the system's.

Every one of these is a crate not already declared, which `CONTRIBUTING.md` says to open an
issue for before adding: `rosc` for commit 1, `mdns-sd` and `tungstenite` for OSCQuery. None of them is an
operating system's crate, so `tests/rules.rs` has nothing to place.

## Threading and timing

- **Two threads**, `osc` (receive) and `osc out` (send), both started and stopped with the
  setting; the receive thread blocks on the socket with a timeout, and the synth only
  `try_iter`s — the rule every device here keeps: **a tick never waits**.
- **Latency** is the network's plus up to one tick: a message arriving just after a tick is
  applied on the next, 16 ms at 60 Hz. MIDI has the same bound.
- **Rates.** TouchOSC sends as fast as a finger moves, tens a second; apps stream at their
  frame rate; scripts, anything. Coalescing per control per tick makes the synth's cost the
  number of *controls* touched, not messages.
- **Bundles** are applied whole, on one tick — the ordering OSC promises.
- **Time tags.** Honouring a future tag means a clock shared with the sender (NTP), which a
  VJ's rig rarely has. Recommend treating every tag as *immediately* and showing it in the
  monitor; if a scheduled use appears, an event could carry its moment inside the frame, as an
  audio crossing does.
- **Smoothing** belongs to the graph, the ruling for the audio bands (`docs/decisions.md`,
  *Band shaping belongs to the graph*): a control bound to a stepped source can be driven by a
  `number` whose control is bound, through `slew`. `Binding` already has room for a curve, as
  for MIDI.

## Networking and safety

- **Off by default**, and turned on with a port. The port, the switch and the send targets are
  about **this machine and this room**, not the patch, so they are **preferences**, beside the
  window and the tick rate; the learned map and what is sent are the project's, beside the MIDI
  map. A suggested default port that nothing in a VJ rig takes by default — not Millumin's
  5000, Resolume's 7000 (and the 7001 its users send back on), MadMapper's 8000 and 8010,
  Resolume's web server's 8080, or VRChat's 9000 and 9001; TouchOSC's habitual 8000/9000
  **(unverified)** collide with the same.
- **Loopback or the network.** A choice between *this Mac only* (bind `127.0.0.1`: apps on
  this machine, no prompt, nobody else) and *the network* (bind `0.0.0.0`: a tablet can reach
  it). A tablet needs the second, so it is the one people will pick.
- **No authentication, anywhere in OSC.** Anyone on the same network who finds the port can
  move any control, cut the decks with `show_a`, fire `snap`, or flood the synth. On a shared
  venue Wi-Fi that is a real risk to a show, if not to the machine: OSC reaches controls,
  action inputs and the fade — **never files, commands or structure**, which keeps the worst
  case "the picture was hijacked". Mitigations: loopback when it will do; an **allow list of
  sender addresses** (the tablet's IP), off by default, with the monitor showing each sender so
  it can be filled in; a clear *on* indicator. Structural control is the agent layer's
  (below), and should never share this unauthenticated port.
- **macOS's Local Network permission.** Since macOS 15, an app reaching the local network is
  asked about once. Apple's TN3179 is precise about what counts, and it splits OSC in two:
  **receiving** unicast UDP and **accepting** TCP connections need no permission, so a tablet
  driving supersilvia, and a client reading the OSCQuery tree, never prompt; **sending** UDP to
  the LAN (feedback, a lighting desk), and **every Bonjour operation** (advertising
  `_oscjson._tcp`, browsing) do. Loopback is not a local network by the note's definition, so
  it should not prompt **(inferred, not stated)**. The `.app`'s `Info.plist` already declares
  `NSLocalNetworkUsageDescription` — its sentence names NDI alone — and `NSBonjourServices`
  with `_ndi._tcp`; sending widens the sentence ("…NDI video and OSC control…"), and OSCQuery
  adds `_oscjson._tcp` and `_osc._udp`. Two testing traps from the same note: a command-line
  tool run from Terminal is **exempt**, so `cargo run` never shows the prompt and only the
  `.app` from Finder does; and an ad hoc signed build is tracked unreliably. A refusal can
  arrive before the person has answered, and a send refused fails quietly, so sending retries
  and the window says "check System Settings ▸ Privacy & Security ▸ Local Network" when a
  send is refused.
- **Linux firewalls.** Fedora Workstation's default zone allows high ports in **(unverified)**;
  other distributions' firewalls, when on, drop incoming UDP until opened. The window says the
  port, so the instruction is one line in the docs.
- **Wi-Fi drops UDP.** A lost fader message is corrected by the next; a lost button *release*
  is a stuck button. TouchOSC and others resend nothing. Recommend a held action input driven
  by OSC be released if its sender goes quiet for a while **(a guess; needs a real tablet)**.

## How it would look in the app

- **Project ▸ OSC…**, a window beside **Project ▸ MIDI…** in its shape: **Receiving** (the
  switch and port, loopback or network, this machine's addresses to type into the tablet, the
  allow list), **Mappings** (learned addresses, as MIDI's rows), **Sending** (targets
  `host:port`, the Main Input's analysis tick, the list of sent outputs with a `✕`), and a
  **Monitor** (off by default, two hundred messages, sender, address, values, and *no such
  address* where nothing answers). The switch and port could equally live in Preferences; the
  window keeps OSC in one place, which the MIDI window's reasoning favours for the project half.
- **Learn** is the MIDI gesture: `Alt` + click, and whichever of MIDI or OSC speaks first binds.
  The range editor's **Bind MIDI…** becomes **Bind MIDI or OSC…**, and its bound line shows the
  address. A bound control's dot and accessible name say `bound to /1/fader1`.
- **Copy OSC address** on a control's right-click and in the range editor — MadMapper's own
  wording for the same thing, and Resolume's *Edit OSC* mode does it by click — so a script
  writer never has to guess `oscillator4`. The tooltip that already shows the range could show the
  address with it.
- **Sending an output**: *Send over OSC* on an output port's right-click, and a mark on the
  port while it is sent.
- **Status**: a small indicator in the Status box — listening, on which port, messages per
  second — since a network problem is otherwise invisible.

## One control surface, two doors

`proposals/agent-control.md`, written alongside this, proposes an agent (MCP) server over the
app's `Command` vocabulary. The two should share one thing and not the rest:

- **Shared: the address space.** `/oscillator4/frequency` and `oscillator4.frequency` are one
  name, the one the accessibility tree already carries, so an agent, a test, the egui MCP and a
  TouchOSC layout all say the same thing. The OSCQuery tree is a description an agent could
  read too.
- **Not shared: reach.** OSC is the **performers'** door — values, presses and the fade, fast,
  unauthenticated, over the room's network. The agent door is the **editor's** — add, connect,
  open, save — through the command bus, on loopback, with whatever authentication that proposal
  settles. Letting OSC carry `Command`s would put structure behind no password.

## Costs and risks

- **Small on the synth**: one hash lookup per message and one write per control per tick.
- **The undo history under a stream** (Owner's question 4) is the real design risk.
- **Security** on shared networks, above; the allow list and loopback are the answer, and the
  documentation must say it plainly.
- **Address stability** rests on ids never being reused and slugs being aliased on rename, both
  already rules; it breaks only across import, as MIDI bindings do.
- **UDP loss** on Wi-Fi: stuck buttons, and a fader that ends a hair off.
- **OSCQuery's WebSocket** is the largest single piece of new code and the first long-lived
  TCP server in the app; it is optional and can come last.
- **Testing** needs no network: a test sends a UDP packet to `127.0.0.1` and reads the
  snapshot, as the MIDI tests post through `App::apply_midi`.

## Linux

**The same code**: `std::net` and `rosc` are the same on both machines, so unlike Syphon and
MIDI, OSC needs no backend of its own — only mDNS might, if the pure crate does not coexist
with Avahi.

## Open questions

Each has a recommendation.

1. **Build it, and in what order?** Recommend yes, receiving first (the MIDI road, with learn),
   then sending, then OSCQuery.
2. **The address: node name or workspace path?** Recommend `/<slug><id>/<key>`, the accessible
   name with slashes, with no workspace and no app prefix on what is received; `/mixer/balance`
   for the fade.
3. **Values at the address: normalised or units?** Recommend normalised 0–1 across the
   control's own range at the address, as MIDI and Resolume do, and units at `…/value`.
4. **Is an OSC write an edit?** Recommend yes, as a knob's is — the document catches up through
   `SetControl` and the undo step closes on silence — and revisit if streaming senders make the
   history useless; the alternative is playing, not editing, as an automation lane is.
5. **One map with MIDI, or two?** Recommend one: `Trigger` gains an OSC arm, one window table
   per family, and learning moves a binding only within its family, so a control can have a
   knob and a fader.
6. **Network by default?** Recommend off by default; when on, the network rather than loopback
   (a tablet is the point), with the allow list offered and the monitor naming senders.
7. **Where the settings live.** Recommend the port, switch and send targets in preferences (the
   machine's), the learned map and the sent outputs in the project, and one **Project ▸ OSC…**
   window for both.
8. **The crates.** Recommend `rosc` for the wire (it adds `nom` 7 beside the lock's `nom` 8;
   hand-rolling is the alternative), and for OSCQuery, last, `mdns-sd` and `tungstenite` with
   the HTTP side written by hand — each asked for when its commit comes.

## The build, in commits

1. **Receiving.** `osc/` with the receive thread and the message type, `rosc`, the preference
   (switch, port, loopback or network), `Msg::OscIn` and `Msg::OscMap`, the automatic address
   table published by the editor, resolution and pattern matching in the synth, the MIDI write
   paths shared, coalescing and the bounded queue; a test sending real UDP to loopback and
   reading the control, the fade and a held action input off the snapshot; docs.
2. **Learn and the window.** The OSC arm of the map, saved in `project.ssp`; `Alt` + click and
   *Bind MIDI or OSC…*; **Project ▸ OSC…** with mappings, the allow list and the monitor;
   *Copy OSC address*; the Local Network sentence widened.
3. **Sending.** The `osc out` thread, targets, the Main Input's analysis, *Send over OSC* on a
   port, the new reason to stay awake, feedback of driven controls, one bundle per tick.
4. **OSCQuery.** The HTTP JSON tree and `HOST_INFO`, mDNS advertising of `_oscjson._tcp` and
   `_osc._udp`, the `Info.plist` services; then the WebSocket's `LISTEN` and `PATH_*` pushes,
   tried against Chataigne or ossia score.

**Testing by hand needs only a phone**: TouchOSC or Open Stage Control on the same Wi-Fi, a
layout with a fader and a button, learned onto a control and `show_a`; and `oscsend` or three
lines of Python for the rest.

## Sources

- OSC: https://opensoundcontrol.stanford.edu/spec-1_0.html (the 1.0 specification) ·
  Freed, Schmeder, *Features and Future of Open Sound Control version 1.1 for NIME*, NIME 2009
  (https://opensoundcontrol.stanford.edu/files/2009-NIME-OSC-1.1.pdf) ·
  https://opensoundcontrol.stanford.edu/spec-1_1.html · RFC 1055 (SLIP) · RFC 8085 §3.2
  (UDP and fragmentation)
- OSCQuery: https://github.com/Vidvox/OSCQueryProposal ·
  https://ossia.io/site-libossia/features/oscquery.html ·
  https://github.com/vrchat-community/osc/wiki/OSCQuery ·
  https://github.com/vrchat-community/vrc-oscquery-lib
- Apps: https://resolume.com/support/en/osc ·
  https://resolume.com/download/Manual/OSC/OSC%20list.txt ·
  https://resolume.com/support/en/restapi · https://derivative.ca/UserGuide/OSC ·
  https://forum.derivative.ca/t/oscquery-protocol/11360 ·
  https://docs.vidvox.net/vdmx/vdmx_oscquery ·
  https://docs.madmapper.com/madmapper/6/11.-live-performance-and-control/osc-commands-and-channels-list ·
  http://forum.garagecube.com/viewtopic.php?t=35412 (MadMapper's OSCQuery on 8010) ·
  https://help.millumin.com/docs/connect/devices/ ·
  https://github.com/anome/millumin-dev-kit/wiki/OSC-documentation ·
  https://hexler.net/touchosc/manual/connections-osc · https://hexler.net/touchosc/releases ·
  https://framagit.org/jean-emmanuel/open-stage-control ·
  https://github.com/benkuper/Chataigne (issue 65) ·
  https://ossia.io/score-docs/devices/oscquery-device.html ·
  https://imimot.com/help/vezer/changelog/ ·
  https://www.ableton.com/en/packs/connection-kit/ ·
  https://github.com/Ableton/m4l-connection-kit
- Rust: https://crates.io/crates/rosc (https://github.com/klingtnet/rosc) ·
  https://crates.io/crates/oscquery · https://crates.io/crates/vrchat_osc ·
  https://crates.io/crates/mdns-sd · https://crates.io/crates/zeroconf ·
  https://crates.io/crates/tiny_http · https://crates.io/crates/tungstenite
- macOS: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy ·
  https://developer.apple.com/documentation/bundleresources/information-property-list/nslocalnetworkusagedescription
- supersilvia: `docs/media.md` (*MIDI*), `docs/architecture.md` (*MIDI is read by the tick*,
  *A gesture ends on a release, or on silence*, *Three tiers of saved state*), `docs/ui.md`
  (*The MIDI window*, *Alt + click to bind*), `docs/decisions.md` (*A timeline and a viewport,
  over one parameter address space*; *The fade is the one rig control the MIDI map can
  address*), `src/midi/`, `src/app/midi.rs`, `src/synth/mod.rs` (`take_midi`,
  `write_control`), `src/ui/node_widget.rs` (`NodeName`, `ControlName`),
  `packaging/macos/Info.plist`; `proposals/ndi.md`, `proposals/syphon.md`,
  `proposals/agent-control.md`
