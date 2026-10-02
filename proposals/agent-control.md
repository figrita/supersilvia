# Proposal: agent control

**Status: proposed, not yet decided.** The aim is for every build, not only a debug build, to
be drivable by agents, from any model and any host, over MCP or a protocol like it. This says
what exists today, the layer underneath it that an agent should be
talking to instead, the routes and transports with the one recommended, how requests reach the
editor's frame, what keeps it safe, what an agent would see, and the open questions.
Everything here was read in supersilvia's source, in `egui_inspection` 0.36.2 and
`rmcp` 1.8.0 as they sit in the local cargo registry, in `egui_mcp` at the commit installed,
and in the MCP specification and the clients' own documentation; nothing was built or run. A
claim that could not be checked that way says **(unverified)**.

OSC is the performers' layer on the same control surface — a knob on a tablet, a cue from
QLab — and is argued in `proposals/osc.md`. This is the agents' layer: whole edits, reads and
pictures, with replies.

## The words

- **MCP**, the Model Context Protocol, is JSON-RPC 2.0 between a **host** (the app a model
  runs in: Claude Code, Cursor, Gemini CLI) and a **server** (what offers **tools** the model
  may call, each with a name, a description and a JSON Schema for its arguments). The model
  reads the tool list and decides what to call; the host makes the call.
- A **transport** carries it. MCP defines two: **stdio**, where the host starts the server as
  a child process and speaks over its stdin and stdout, and **Streamable HTTP**, where the
  server is already running and listens on one URL.
- **Driving the UI** is sending synthesized clicks and reading the accessibility tree.
  **Driving the document** is sending the edits the UI would have produced, by name.

## What exists today

### eframe's inspection server

- **A Cargo feature, off in release.** `Cargo.toml`'s `inspection = ["eframe/inspection"]`
  says "Never on in release". With it compiled in, eframe calls
  `egui_inspection::attach_from_env` when the window is made: `EGUI_INSPECTION` unset, empty,
  `0` or `false` is off; `1` or `true` listens on `127.0.0.1:5719`; **anything else is taken
  as a bind address**, so `EGUI_INSPECTION=0.0.0.0:5719` puts it on the network, with a logged
  warning and nothing more.
- **The protocol** (`egui_inspection/src/protocol.rs`, version 1). TCP. On connect the app
  writes a handshake: the four bytes `eins` and a big-endian `u32` version. Then each message
  is a 4-byte big-endian length and a MessagePack body (`rmp-serde`), at most 256 MiB, one
  response per request. The requests are `GetInfo`, `GetTree` (the whole AccessKit tree, a
  full snapshot every time), `GetScreenshot { pixels_per_point }` (a PNG of the editor's
  framebuffer), `ApplyEvents { events: Vec<egui::Event> }` (raw input, answered after a frame
  has processed it), `Resize` and `Settle { max_steps }`. The answers are `Info`, `Tree`,
  `Screenshot`, `Done`, `Settled` and `Error`. **No authentication.**
- **How it is serviced.** An `egui::Plugin` holds the queue; a thread per connection submits
  a request through the plugin handle and calls `request_repaint`, and the reply is made on the
  UI thread in the plugin's input and output hooks. So the reply needs an egui pass, and **an
  occluded or minimized editor does not answer** (`docs/testing.md`, layer 4). The listener's
  threads are detached and live for the process: once on, it cannot be switched off.
- **It turns AccessKit on for good.** The plugin's `setup` calls `enable_accesskit`, so every
  widget's accessible name is formatted every frame from then on — the cost
  `docs/ui.md` (*Accessibility is the agent's API*) is careful to pay only while something
  reads it.
- **`egui-mcp`** is the MCP server that speaks it: `egui_mcp` 0.2.0 from
  `rerun-io/kittest_inspector` at `3f196da`, installed with `cargo install`, a stdio server on
  `rmcp` 1.7 and tokio. a development setup declares it to its MCP host as a bare
  `egui-mcp`, builds with `--features inspection`, launches with `EGUI_INSPECTION=1` and
  attaches. Its tools are
  `attach`, `query_tree`, `get_node`, `click`, `drag`, `hover`, `scroll`, `type_text`,
  `press_key`, `screenshot`, `resize`, `wait_for` and `batch`. Being stdio MCP, it is already
  usable from any MCP host — but only against a build with the feature.
- **`egui_kittest`** is the same tree in process: `tests/ui.rs` finds widgets by the names
  `docs/ui.md` defines (`checkerboard1.frequency`, `cable audioin1.level to oscillator2.frequency`)
  and clicks them with no window at all. Layers 2 and 4 share the labeling discipline.

### What driving the UI costs an agent

- **Reads are big.** `GetTree` is every node of every widget on screen, every time. A real
  patch is hundreds of rows, ports and cables, and the model pays for each in tokens.
- **Everything is found by its label.** The traps met in practice: `"Output"`
  matches the status line, every port label and a menu item; the library's entries are
  `{icon} {label}` and the zoom is `🔎`, not `🔍`; a submenu opens 200 ms after the pointer
  enters its category, so one must hover, wait and then click.
- **Many round trips per edit.** Adding one node is open the Nodes menu, hover the category,
  wait, click the entry, then read the tree to learn its id. A cable is a drag between two
  port dots computed from their bounds. A value is a scrub or a typed entry into a field
  that must be focused first. Each is an observe → act → verify loop.
- **The layout is part of the API.** A node scrolled off the canvas is culled and draws no
  widget **(unverified that a culled node leaves nothing in the tree)**; a popup's first frame
  is a sizing pass with provisional geometry (`CONTRIBUTING.md`, *Testing*); a panel slides.
  Pictures in windows of their own are out of reach entirely: they are not egui's.
- **Failures are silent.** A refused cable is a cable that did not appear. The command bus
  knows it was `Refused(TypeMismatch)`; the tree does not say.

**What it is still right for: testing the UI itself.** Does the menu open, does a click land
where the eye says it should, does the cable draw above the node — the six bugs
`docs/testing.md` counts that no test saw. An agent building a patch wants the document; an
agent checking the editor wants the editor.

## The semantic layer that already exists

The UI never mutates. It emits `Command`s and `App::apply` is "the only way anything
mutates" (`src/command.rs`, `src/app/edit.rs`, `CONTRIBUTING.md`'s rule on `ui/`). That is an
agent API already, missing its wire.

- **`Command`**, thirty-one variants: `AddNode`, `RemoveNodes`, `Duplicate`, `Paste`, `Connect`,
  `Bridge`, `Disconnect`, `DisconnectEdge`, `DisconnectPort`, `DisconnectAll`,
  `ResetControls`, `MoveNodes`, `SetControl`, `SetControls`, `SetRange`, `ClearRange`,
  `SetOption`, `SetValue`, `SetNodeWidth`, `SetCollapsed`, `SetLayout`, `AddWorkspace`,
  `RemoveWorkspace`, `RenameWorkspace`, `SetBlurb`, `MoveWorkspace`, `ShowOn`, `HideFrom`,
  `MoveTo`, `ImportWorkspace`, `AutoArrange`. Each carries its own arguments, by the rule the
  module states, so each replays the same whatever else has changed.
- **`CommandError`** says why an edit was refused, with a `Display` a model can read as it
  is: `NoSuchNode`, `NoSuchNodeKind`, `Refused(ConnectError)`, `NotConnected`,
  `AlreadyConnected`, `Rendering`, `NoSuchKey`, `NoSuchChoice`, `NoSuchWorkspace`,
  `LastWorkspace`, `NoWorkspaceLeft`, `EmptyClip`, `Failed`.
- **`App::apply`** refuses every edit while an offline render runs, applies through the
  document, hands the result to the shaders, the canvas, the tabs and the synth, and bars the
  MIDI writes it would fight. An edit from an agent is exactly an edit from a hand.
- **Undo.** `App::undo`, `App::redo`, `can_undo`, `undo_len`. The ring's steps are opened
  and closed by an `Origin` — `Hand`, `Midi`, `Tick` in `document/history.rs` — which decides
  what holds a gesture open. An agent is a fourth origin, not a new mechanism.
- **Identity.** `NodeId` is "one stable integer per node per project, never reused", and
  undo restores a removed node under the id it had; `WorkspaceId` is the same. A port is a
  `PortRef { node, key }`, the key a definition's.
- **The registry.** `nodes::REGISTRY`, about a hundred and sixty `NodeDef`s, and
  `nodes::find(slug)`. A kind has a `slug`, `label`, `icon`, `tooltip`, `category`, inputs
  (`InputDef`: key, label, `PortType`, and a `Control` — `Number { default, min, max, step,
  unit, log, capped_by }`, `Color { default }`, a button, or none), outputs, `options`
  (`OptionDef`: key, label, default, choices, kind, placeholder) and `values`. That is a
  schema of every node an agent could ask for, and the tooltips are written for people, which
  is what a model reads best.
- **Reads.** `App::graph()` is the whole graph: nodes with their kind, position, workspaces,
  controls, options and values, and the connections. `App::snapshot()` is what the synth last
  published: a uniform output's value (`App::uniform`), a node's status line
  (`App::node_status`), traces. `App::file_status()` is the status line; the Status box builds
  a `StatusView` and has a `plain` rendering of it, every section unfolded, which is already a
  text report of how the app is running.
- **The file format is the read format.** `workspace.rs`'s `SavedNode` and
  `SavedConnection` are serde JSON with string keys, and `workspace::resolve` turns a saved
  key back into a definition's `&'static str` through the node's own port list. A graph read
  in that shape is a shape the project already commits to.
- **Addressing a control from outside has a precedent.** The MIDI map's `Target::Port(PortRef)`
  or `Target::Balance` rides in `project.ssp` with string keys and is resolved against the
  graph on load, dropping a row whose port has gone. The accessibility tree names the same
  things `checkerboard12.frequency`. And `docs/decisions.md` (*one parameter address space*)
  already says every value has an address with several writers; an agent is one more.
- **Pictures, without waiting.** An Output's `snap` action input asks the renderer for the
  frame at its own resolution; the read is collected a frame or two later from
  `Snapshot::events.snaps`, and `App::collect_snaps` writes it to `snaps/`
  (`docs/rendering.md`, *The Snap readback*). The thumbnail is the same read at 240×135. Neither
  ever waits on the GPU.
- **The rest of the rig** is `App` methods rather than commands, since none of it is
  document: `press(port, down)` for an action input, `show_on`, `set_balance`, `set_method`,
  `start_render`, `save_project`, `open_project`.

So an agent adding a checkerboard would, underneath, call
`apply(Command::AddNode { slug: "checkerboard", at, workspace })` and read the new id back;
connecting it is `apply(Command::Connect { from, to })` and gets either `Ok` or a sentence
such as *type mismatch: VaryingColor output into UniformNumber input*.

### What is missing

1. **`Command` has no serde, and should not get it.** Its keys are `&'static str` from a
   definition, a slug is `&'static str`, `Bridge` carries `&'static [&'static str]`, and
   `Paste`'s `Clip` holds whole `Node`s with a `&'static NodeDef`. None deserializes. A wire
   type with `String` keys, and one function that resolves it against the graph the way the
   file loader and the MIDI map do, keeps `Command` free to change and makes the wire the
   versioned contract.
2. **`AddNode` and `AddWorkspace` do not return the id.** Their callers read the highest id
   back afterwards (the comment on `AddWorkspace` says so). The dispatcher does that and
   returns it.
3. **A port's effective type.** A dual port's type is the instance's (`PortDef` on the node),
   not the kind's, so the graph read carries the instance's ports, not the registry's.
4. **An agent origin.** `Origin::Agent`: each call one undo step, holding no gesture open, so
   a model's edit never coalesces with the hand's scrub it interrupted.
5. **A Snap that answers its asker.** Today a Snap goes to a file and the status line. An
   agent wants the bytes back; the ask needs a tag so the reply finds its request.
6. **A revision to poll against.** Nothing tells a client the graph changed. The document's
   edit serial, which `dirty` compares, returned with every read, lets an agent ask
   *has anything moved since 41?* cheaply.
7. **Sentences with names.** `CommandError` and `ConnectError` print a node as its `Debug`,
   `NodeId(12)`, and a key bare. The dispatcher words a refusal with the node's name,
   `checkerboard12.output`, and adds what would have worked where something knows it.

## The routes, and the one recommended

### A. Inspection in every build, gated at run time

Compile the inspection plugin into release, but start it from a switch instead of the
environment: depend on `egui_inspection` with its `plugin` feature directly, drop eframe's
`inspection` feature (which is what reads `EGUI_INSPECTION`), and call `ctx.add_plugin` and
`serve` from our own code when Preferences or a launch flag says so, on loopback alone.
`egui_inspection` is MIT or Apache-2.0 and already in the lock under the feature.

- **For:** UI testing against the shipping binary; `egui-mcp` works on any build; a user's
  bug can be reproduced on their own build.
- **Against:** every cost in *What driving the UI costs* above; no authentication in the
  protocol; no off switch without a listener of our own; AccessKit on for the rest of the run;
  a visible window required.
- **Browsers cannot speak it.** The app writes its handshake first and then reads a length.
  An HTTP request's first bytes, `POST` or `GET `, read as a length of about 1.3 GB, over the
  256 MiB cap, and the connection is dropped. Any local process can connect.

**Recommended as a tester's tool, not the agent API**: in every build, off by default, behind
the same switch as B with a second tick of its own.

### B. supersilvia's own control server (recommended)

A small server inside the app that exposes the command bus and the reads above as named
tools, and answers in facts: ids, values, refusals, pictures. It is what makes an agent's work
a dozen calls instead of a hundred, and it has nothing to do with where anything is drawn.

#### Transports for B

- **B1. MCP over Streamable HTTP on `127.0.0.1` (recommended).** Every local host that
  speaks MCP can be pointed at a URL (the table below). The current specification is
  **2026-07-28**, which made the protocol stateless: no `initialize`, no `Mcp-Session-Id`, no
  GET stream; every POST carries `MCP-Protocol-Version`, `Mcp-Method` and, for a tool call,
  `Mcp-Name`, which must match the body or the answer is `400` with `HeaderMismatch`
  (−32020); a server must implement `server/discover`. Clients written against **2025-11-25**
  and earlier still open with `initialize` and have no way forward, so for now the server
  should be **dual-era**: answer `initialize` in the legacy shape and per-request `_meta` in
  the modern one, which the specification allows on one endpoint. A tools-only server needs
  neither sessions nor SSE in either era: JSON replies, `405` for GET and DELETE.
  The specification's security section is quoted under *Safety*.
- **B2. MCP over stdio, through a shim.** `supersilvia mcp` is the same binary started by the
  host as a child: it opens no window, reads the running app's port and token from a file,
  and forwards each message over B1. Claude Desktop's local config and any other stdio-only
  host then work without `npx mcp-remote`, the generic bridge (0.14.3, MIT), which needs
  Node and has had a critical CVE of its own (CVE-2025-6514).
- **B3. Plain HTTP/JSON or WebSocket JSON-RPC.** Any script or `curl` could call it, but no
  model host discovers tools from it: each would need an adapter written by hand. MCP is
  JSON-RPC with a tool list and schemas on top, so B1 is already this, with discovery.
- **B4. A Unix socket.** File permissions are the authentication and no browser can reach it.
  But no MCP host takes a socket path except VS Code **(unverified for the others)**, so it
  would need B2 in front of it for everything else. Not in the first version.

#### Which hosts speak MCP

| Host | stdio | HTTP to a local URL | Notes |
| --- | --- | --- | --- |
| Claude Code | yes | yes, with headers | `claude mcp add --transport http … -H "Authorization: Bearer …"` |
| Claude Desktop | yes | through `mcp-remote` or B2 | its custom connectors are called from Anthropic's cloud and cannot reach localhost |
| Claude API MCP connector | no | no | the server must be public |
| OpenAI Agents SDK | yes | yes | `MCPServerStdio`, `MCPServerStreamableHttp` |
| OpenAI Responses API, ChatGPT developer mode | no | no | called from OpenAI's servers; a public URL, a tunnel, or OpenAI's Secure MCP Tunnel |
| OpenAI Codex CLI | yes | yes | `config.toml`, `bearer_token_env_var` |
| Gemini CLI | yes | yes | `mcpServers`, `httpUrl` and `headers` |
| Gemini API SDKs | experimental, tools only | | **(unverified from Google's own page)** |
| Cursor | yes | yes, with headers | |
| VS Code (Copilot agent mode) | yes | yes | also Unix sockets and named pipes |
| LM Studio 0.3.17+ | yes | yes, with headers | local models driving the app |
| Zed, Windsurf, Continue, Goose | yes | yes | |
| Ollama | no | no | needs a host in front of it, such as Goose |

So **B1 plus B2 reaches every local host that exists**, Claude's, OpenAI's, Google's and the
open-model ones; the cloud-hosted clients cannot reach a laptop's loopback without a tunnel,
and a tunnel is not something this proposal offers.

#### Which implementation of MCP

- **`rmcp`**, the official Rust SDK: **3.5.0** on crates.io (the local registry has 1.8.0,
  under `egui-mcp`), Apache-2.0, a Tier 1 SDK on the specification's list, speaking
  2026-07-28 and negotiating down to 2024-11-05. It is built on tokio, `futures`, `schemars`,
  `tracing`, `thiserror` and `chrono`; its Streamable HTTP server is a tower `Service` over
  `http`, `http-body`, `bytes`, `sse-stream`, `uuid` and `rand`, and **brings no HTTP server**:
  axum or hyper is ours to add. It checks the `Host` header against loopback by default but
  **leaves `Origin` validation off** until given a list (`StreamableHttpServerConfig`).
- **What the project's rules say.** "Open an issue before adding a dependency." `tests/rules.rs` places
  tokio in `platform/linux/` alone, where it serves the portal; on macOS it is not in the
  tree at all. A control server on rmcp would put tokio, tower and hyper in both builds and
  in a module that is not a platform backend, which breaks a rule the tests hold.
- **Hand-rolled (recommended).** A tools-only, JSON-only, dual-era MCP server is a small
  surface: `initialize`, `notifications/initialized`, `ping`, `server/discover`, `tools/list`,
  `tools/call`, the header checks, and the HTTP/1.1 it rides on — one request per connection
  or keep-alive, a `Content-Length` body, no chunked uploads accepted — over
  `std::net::TcpListener` on a thread, with `serde_json`, which is already here. A few hundred
  lines with no new crate **(unverified: the size, until written)**. The cost is tracking the
  specification's revisions ourselves; the MCP layer is kept a thin adapter over the
  dispatcher, so moving to rmcp later touches one module. Conformance can be checked in tests
  with rmcp's client as a dev-dependency, which never reaches the binary.

### D. `supersilvia ctl`, for any agent with a shell

The same binary, as a client: `supersilvia ctl graph`, `supersilvia ctl add checkerboard`,
`supersilvia ctl connect 12.output 7.input`, `supersilvia ctl set 12.frequency 8`,
`supersilvia ctl snap 3 -o out.png`, `supersilvia ctl call <tool> '<json>'`. It reads the
port and token from the discovery file and calls the same tools over B1. An agent with a
shell and no MCP at all — a script, a cron job, a model in a terminal — drives the app with
it, and a person can too. It shares its client with B2, so the two are one piece of work.

`main.rs` takes its first argument as a project path, so `ctl` and `mcp` are matched first,
before any window, device or GPU is touched; a project folder named `ctl` opens as `./ctl`.

### Not recommended

- **Inspection as the agent API** (A alone): the costs above, for every agent, forever.
- **Commands over OSC for agents**: OSC is fire-and-forget, with no reply, no ids and no
  errors; it is right for a knob and wrong for *add a node and tell me its id*
  (`proposals/osc.md`).

## Threading

- **The server is a thread of its own**, named `control`, with a blocking accept and a
  thread per connection, capped at a few. It parses, checks `Host`, `Origin`, the token and
  the MCP headers, and turns a tool call into a `control::Request`. **It never touches `App`.**
- **Requests cross on a channel** with a one-shot reply sender each, and the thread calls
  `ctx.request_repaint()` to wake eframe.
- **The editor drains the channel in `eframe::App::logic`.** eframe 0.36 calls `logic` before
  every `ui`, and also while the window is hidden if a repaint was asked for, running no egui
  pass. So a minimized or occluded editor still answers an agent, which inspection cannot.
  supersilvia does not implement `logic` yet; the non-painting housekeeping a reply depends on
  — taking the synth's snapshot, collecting Snaps — moves or is repeated there **(unverified
  that all of it can run outside `ui`)**.
- **An edit is applied on the UI thread through `App::apply`**, with `Origin::Agent`, exactly
  as a click's command is: the same render refusal, the same republish to the synth, the same
  MIDI bar. The reply — the result, a new id, the edit serial — is sent as soon as `apply`
  returns. The UI thread never waits on the socket; a send on the channel does not block.
- **A read that needs a later frame is parked.** A Snap is asked for, its request kept with
  its reply sender, and answered on the frame the picture lands, or with an error after two
  seconds.
- **A budget per frame.** At most a few requests, or a millisecond, are applied a frame; the
  rest wait for the next. An agent in a loop cannot cost the editor a frame, and the editor
  cannot cost the synth one: the synth only ever hears `Msg::Graph`, as for any edit.
- **Latency is a frame.** The editor already repaints every display interval, since it is a
  synth, so a request waits at most one; minimized, it paints every quarter second and the
  wake shortens that.
- **Testable without a socket.** `control::handle(&mut App, Request) -> Reply` runs on
  `App::headless()` in `cargo test`; the socket layer is tested separately in process.

## Safety

- **Off by default.** Preferences ▸ *Agents* ▸ *Let agents control supersilvia*, remembered;
  a `--control` launch flag for one session. Never an environment variable alone: a variable
  set in a shell profile would turn it on for every launch without anyone looking.
- **Loopback only.** Bound to `127.0.0.1`, with no setting for another interface. A rig that
  wants another machine to drive it uses OSC, or a proposal of its own.
- **A token.** Made at random when the server starts, and written with the port to a
  discovery file readable by its owner alone (`0600`) in `platform::dirs::config()`. Every
  request carries `Authorization: Bearer <token>`; a missing or wrong one is `401`. The shim
  and `ctl` read it themselves; a URL-configured host has it pasted into its headers, which
  every host in the table above takes. The MCP authorization specification (OAuth 2.1) is
  optional and meant for remote servers; a bearer token on loopback is the local answer, and
  stdio "SHOULD" take credentials from the environment, which is what the shim does.
- **`Origin` and `Host`.** Any request whose `Origin` header is present and not ours is `403`,
  as the specification requires; a `Host` that is not `127.0.0.1:<port>` or
  `localhost:<port>` is `403` too. POST only, `Content-Type: application/json` only, and no
  CORS headers ever sent.
- **What a malicious web page could do, and what stops it.** A page can `fetch` to
  `http://127.0.0.1:<port>` blind: it cannot read the answer, but it can cause the edit. A page
  can also rebind its own domain to `127.0.0.1` and then read the answers as same-origin. This
  is not hypothetical: the MCP Inspector's proxy was taken over exactly so (CVE-2025-49596,
  fixed with a token and Origin checks), and the official TypeScript and Python SDKs shipped
  with rebinding protection off by default until CVE-2025-66414 and CVE-2025-66416. Here the
  blind request fails on `Content-Type` (a cross-origin `application/json` needs a preflight
  we never answer), on `Origin` and on the token; the rebinding fails on `Host`, on `Origin`
  and on the token. Each would stop it alone.
- **What a malicious local process could do.** Anything the user can: it can read the token
  file as it can read the project folder. The token keeps out browsers and other users on the
  machine; it is not a sandbox, and nothing here pretends to be.
- **An indicator.** While a client is connected, a mark in the editor says so — *Agent*, with
  the client's name from `clientInfo` — and a click lists the last calls and turns the server
  off. The Status box gains a line of the same.
- **Every agent edit is undoable**, in the one ring, one call one step. What is not an edit —
  a save, a Snap, the mixer — is named as such in the tool's description.
- **Three levels.** *Read only* (graph, kinds, status, pictures), *Edit* (the command bus,
  undo, redo) and *Edit and save*. Default *Edit*.
- **Refused during a render**, as every command already is, with `CommandError::Rendering`'s
  sentence.
- **Left out of the first version:** opening, closing or making projects; quitting; *Save
  as* to a path; `ImportWorkspace`, which reads a path; setting a file option to a path,
  which copies that file into `assets/` — an agent may choose among assets already in the
  project; starting a render. Each is a filesystem or a destructive act a person should take.
- **Content is data.** A note's text, a workspace's name and blurb come back verbatim; a
  patch shared by someone else can carry words meant for a model. The tool descriptions say
  that returned text is the user's content, not instructions.
- **Limits.** A request body of at most 1 MiB, a picture of at most the Output's own size,
  four connections.

## What it would look like

Twelve tools, each small, each answering in JSON (with a text rendering beside it, since
some hosts show only text):

| Tool | Arguments | Answer |
| --- | --- | --- |
| `list_node_kinds` | `category?`, `query?` | slug, label, category, one line of tooltip each |
| `describe_node_kind` | `slug` | inputs with type, control, range, default; outputs with type; options with choices |
| `get_graph` | `workspace?`, `since?` | workspaces; nodes with id, name, kind, controls, options, values, ports; connections; `serial` |
| `add_node` | `kind`, `workspace?`, `at?` | `id`, `name` |
| `remove_nodes` | `ids` | — |
| `connect` / `disconnect` | `from`, `to` | — or the refusal |
| `set` | `target`, `value` | a control (a number, or `#rrggbbaa`), or an option (a choice) |
| `press` | `target` | an action input: `Show on A`, `Snap`, a `button` |
| `undo` / `redo` | — | whether it did, and the depth left |
| `workspace` | `add`, `rename`, `remove`, `arrange` | the id |
| `snapshot` | `output`, `max_width?` (default 512) | a PNG, as MCP image content |
| `status` | — | the status line, each Output's state and GPU time, node status lines, the last refusals |
| `save` | — | the project's name and path, when *Edit and save* allows |

An address is `node.key`, the node by its id — `12.frequency` — which is what the MIDI map
stores; the tree's `checkerboard12.frequency` is accepted too, and the name is returned beside
every id so a transcript reads. A key is resolved against that node's own ports and options,
so a wrong one is `NoSuchKey`, not a guess.

A call, in the 2026-07-28 shape:

```http
POST /mcp HTTP/1.1
Host: 127.0.0.1:5720
Authorization: Bearer 7f3c…
Content-Type: application/json
MCP-Protocol-Version: 2026-07-28
Mcp-Method: tools/call
Mcp-Name: add_node

{"jsonrpc": "2.0", "id": 4, "method": "tools/call",
 "params": {"name": "add_node", "arguments": {"kind": "checkerboard", "workspace": 1},
            "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                      "io.modelcontextprotocol/clientInfo": {"name": "codex", "version": "…"},
                      "io.modelcontextprotocol/clientCapabilities": {}}}}
```

```json
{"jsonrpc": "2.0", "id": 4, "result": {
  "content": [{"type": "text", "text": "added checkerboard12 on workspace 1"}],
  "structuredContent": {"id": 12, "name": "checkerboard12", "serial": 42}}}
```

A refusal is a tool result, not a protocol error, so the model reads it and tries again (the
wording is illustrative; the middle is `ConnectError`'s own):

```json
{"jsonrpc": "2.0", "id": 5, "result": {
  "isError": true,
  "content": [{"type": "text",
    "text": "checkerboard12.output cannot feed phase7.rate: type mismatch: VaryingColor output into UniformNumber input. The conversion menu has a node for this: call bridge."}]}}
```

A picture:

```json
{"jsonrpc": "2.0", "id": 6, "result": {
  "content": [{"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgo…"},
              {"type": "text", "text": "output3 at 1920x1080, scaled to 512x288, tick 88412"}]}}
```

The result shapes are the ones 2025-11-25 defines **(unverified that 2026-07-28 changed none
of `content`, `structuredContent` or `isError`)**. The hint reads `nodes::bridge`, the table
the conversion menu reads, and a `bridge` tool (`Command::Bridge`) is a thirteenth tool if
conversions are to be offered to agents.

## Open questions

Each has a recommendation.

1. **The agent API is the command bus, not the UI.** Build B, with A kept as a tester's
   tool. Recommend yes.
2. **Inspection in every build.** Compiled in, off by default, started from the same switch
   by a tick of its own, loopback only, `EGUI_INSPECTION` no longer read. Recommend yes; it
   is cheap and it is how the shipping build gets tested.
3. **The transports.** MCP over Streamable HTTP on loopback, a stdio shim (`supersilvia mcp`)
   and a shell client (`supersilvia ctl`), all one server. Recommend all three, the shim and
   the client as one piece of work.
4. **Hand-rolled MCP, or `rmcp`.** Recommend hand-rolled on `std` and `serde_json`, dual-era,
   with rmcp's client as a test's dev-dependency; rmcp brings tokio, tower and an HTTP server
   to macOS and breaks the rule that places tokio.
5. **A token on loopback.** Recommend yes, always: it costs the shim and `ctl` nothing and a
   URL-configured host one header.
6. **How it is turned on.** A remembered preference and a one-session `--control` flag,
   never an environment variable alone. Recommend yes.
7. **What the first version may do.** Read, edit, undo, pictures and status; save behind a
   level; no open, import, file paths, render or quit. Recommend yes.
8. **One call, one undo step.** A `batch` that is one step needs a group in the history, which
   does not exist. Recommend one step per call first, a group later if agents ask.
9. **The port.** A fixed default beside inspection's 5719 — 5720 is proposed **(unverified
   that nothing common claims it)** — with the next free one taken if it is busy, the
   discovery file saying which. Recommend yes.

## The build, in commits

1. **The wire and the dispatcher, no socket.** `src/control/`: the wire types with string
   keys, their resolution against the graph, `Origin::Agent`, `handle(&mut App, Request) ->
   Reply` for the tools above but `snapshot`, the id returned from `add_node`, the edit serial
   on every reply. Tests on `App::headless()`: every tool, every refusal's sentence, undo of
   an agent's edit, the render refusal. An exhaustive match holds that every `Command` has a
   wire form or is listed as left out.
2. **The server.** The `control` thread, HTTP/1.1 on loopback, `Host`, `Origin`, token and
   MCP header checks, dual-era JSON-RPC, the discovery file, the drain in `logic` with its
   budget; Preferences ▸ *Agents* with the three levels, `--control`, the indicator and its
   Status box line. Tests over a real socket in process: a forged `Origin`, a bad `Host`, no
   token, a header mismatch, a legacy `initialize`, a modern call; and rmcp's client, as a
   dev-dependency, listing and calling the tools.
3. **Pictures and status.** The Snap readback answered to its asker, scaled on the control
   thread with the `image` crate already here; the thumbnail as the cheap form; the `status`
   tool from `StatusView`. In `tests/gpu_app.rs`.
4. **`supersilvia mcp` and `supersilvia ctl`.** One client module, dispatched in `main.rs`
   before anything else starts.
5. **Inspection in every build**, if question 2 is yes: `egui_inspection` as a direct
   dependency with `plugin`, started from the switch; the development notes lose the build
   flag and the environment variable.
6. **Docs, in each commit above**, per `CONTRIBUTING.md`: `docs/architecture.md` (a second emitter
   of commands), `docs/ui.md` (the preference, the indicator), `docs/decisions.md` (why the UI
   is not the agent API), `docs/testing.md` (layer 4 against release), `CONTRIBUTING.md`, and
   an MCP host entry for the new server beside `egui`.

## Costs and risks

- **A listening port in a shipping app.** Off by default, loopback only, a token, `Origin` and
  `Host` checked — each of the four is what the specification or an advisory asks for — and
  still the largest new surface this app would have.
- **The specification moves.** Four revisions in two years, the last one breaking. A
  hand-rolled server tracks them; the dual-era shape is what the specification offers for the
  crossing, and legacy support can go once the hosts above have moved **(unverified which
  already have)**.
- **The wire is a contract.** Every new `Command` wants a wire form or a line saying it has
  none; the exhaustive match makes forgetting a compile error, not a surprise.
- **Descriptions are prompts.** A tool described badly is used badly. The registry's
  tooltips and `CommandError`'s sentences carry most of it; they will be read by models from
  now on, and worth writing with that in mind.
- **Pictures are expensive in a model's context.** A 1080p PNG is several megabytes of
  base64; the default is 512 wide, and the full frame is a Snap to disk.
- **Nothing per frame while off.** While on: a channel polled once a frame, and AccessKit only
  if inspection is ticked too.
- **Picture windows stay out of reach** of both routes; the `snapshot` tool is the answer for
  what an Output shows.
- **macOS's firewall.** A listener on loopback alone should not raise the incoming-connections
  prompt **(unverified for a signed, notarised `.app`)**.
- **Linux is the same code**: `std` sockets, no input synthesis, nothing that differs under
  Wayland.

## Sources

- The MCP specification, 2026-07-28:
  https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
  (the security section, the request headers, backward compatibility) ·
  https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning (dual-era servers,
  `server/discover`) ·
  https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization ·
  https://blog.modelcontextprotocol.io/posts/2026-07-28/ ·
  https://modelcontextprotocol.io/specification/2025-11-25/basic/transports
- rmcp: https://crates.io/crates/rmcp · https://github.com/modelcontextprotocol/rust-sdk
  (`src/transport/streamable_http_server/tower.rs`, `StreamableHttpServerConfig`) ·
  https://modelcontextprotocol.io/docs/sdk
- egui: `egui_inspection` 0.36.2 (`src/protocol.rs`, `src/plugin.rs`, `src/lib.rs`) and
  `eframe` 0.36.1 (`src/lib.rs`, `maybe_attach_inspection_plugin`; `src/epi.rs`, `App::logic`)
  in the cargo registry · https://github.com/rerun-io/kittest_inspector (`crates/egui_mcp`, at
  `3f196da`)
- Hosts: https://code.claude.com/docs/en/mcp ·
  https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp ·
  https://platform.claude.com/docs/en/agents-and-tools/mcp-connector ·
  https://openai.github.io/openai-agents-python/mcp/ ·
  https://developers.openai.com/api/docs/guides/tools-connectors-mcp ·
  https://developers.openai.com/apps-sdk/deploy ·
  https://learn.chatgpt.com/docs/extend/mcp?surface=cli ·
  https://geminicli.com/docs/tools/mcp-server/ · https://gofastmcp.com/integrations/gemini ·
  https://cursor.com/docs/context/mcp ·
  https://code.visualstudio.com/docs/copilot/reference/mcp-configuration ·
  https://lmstudio.ai/docs/app/mcp · https://zed.dev/docs/ai/mcp ·
  https://docs.devin.ai/desktop/cascade/mcp · https://docs.continue.dev/customize/deep-dives/mcp ·
  https://gofastmcp.com/integrations/goose · https://github.com/ollama/ollama/issues/7865
- The bridge: https://registry.npmjs.org/mcp-remote
- Advisories: https://www.oligo.security/blog/critical-rce-vulnerability-in-anthropic-mcp-inspector-cve-2025-49596 ·
  https://github.com/advisories/GHSA-w48q-cv73-mx4w ·
  https://advisories.gitlab.com/pkg/pypi/mcp/CVE-2025-66416/ ·
  https://github.com/advisories/GHSA-6xpm-ggf7-wc3p
- supersilvia: `src/command.rs`, `src/app/edit.rs`, `src/app/mod.rs`,
  `src/app/document/history.rs`, `src/app/files.rs` (`collect_snaps`), `src/app/frame.rs`,
  `src/graph/`, `src/nodes/mod.rs`, `src/workspace.rs`, `src/midi/bind.rs`, `src/ui/status.rs`,
  `tests/rules.rs`, `docs/ui.md` (*Accessibility is the agent's API*), `docs/rendering.md`
  (*The Snap readback*), `docs/testing.md`, `docs/decisions.md`
- OSC, the performers' layer: `proposals/osc.md`
