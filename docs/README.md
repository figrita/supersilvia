# supersilvia design docs

The source of truth for how supersilvia works and why. If this and the code disagree, one of
them is a bug — say which.

## What these are

Each document describes **what is true now**. Not what was tried, not what is planned, not
how it got here. A spec, not a changelog.

## Keeping them true

1. **A change in behavior and a change to the doc belong in the same commit.** A doc that
   drifts is worse than no doc, because it is believed.
2. **Purge superseded ideas.** Delete them from the document; do not annotate them as
   obsolete, and do not keep a "previously" section. Half-true documents are how a spec
   dies.
3. **The archeology lives in git.** Every commit here explains why a thing changed. `git
   log -p docs/` is the history; the file is the present.
4. **One exception to purging:** an idea that a reasonable person would propose again gets
   one line in [decisions.md](decisions.md) saying it was considered and why it lost. That
   file exists so a rejected path is rejected once.
5. **A decision is recorded when it is made, not when it is built.** `decisions.md` may hold
   a choice that nothing implements yet; such an entry says **Agreed, not built** in its first
   line. Nowhere else in `docs/` describes anything that does not exist — if you are reading
   about behavior outside that file, it is behavior you have.
6. **Write the reason, not just the rule.** A rule without its reason gets "cleaned up" by
   the next person who does not know what it was protecting.

## Index

| Document | What it covers |
| --- | --- |
| [architecture.md](architecture.md) | The model: a graph is an expression. Module layering, the four value kinds, the compiler, Output-as-memory, feedback, CPU nodes, workspaces and suspension, the clock, the command bus, and the project on disk — the three tiers of saved state, assets, export and import. |
| [rendering.md](rendering.md) | The GPU side, on wgpu: the per-Output ring, every Output drawn every tick and the one wait that slows the tick, precision, the paint callbacks and the viewport they are given, and the one device the synth, the editor and every picture window share. |
| [loop-bounding.md](loop-bounding.md) | The counter wgpu puts into every shader loop: what naga writes, why it is there, what it costs the loop-heavy nodes on the iGPU, every loop in the library, and the options with a recommendation. |
| [nodes.md](nodes.md) | The node definition model, the recipe for adding one, CPU nodes, and the catalogue of the library. |
| [cpu.md](cpu.md) | What a node computes outside a shader: the `UniformNumber` port type, the tick, and the two kinds of state — which one a saved file holds. |
| [media.md](media.md) | The real-world sources: the microphone, cameras, screen capture through the desktop portal, the loopback, and video files transcoded on import. The Main Input the rig shares, the analyzer, the scope, and monitoring. |
| [ui.md](ui.md) | The canvas and its interaction rules. What is ours and what is egui's. The tab bar and the project tab, the menu bar, the three ways to the library, the two side panels, the confirm and the recovery question, what a tester can send — the log file, the notice after a run that went down and Report a problem… — and the preferences kept between runs. |
| [design-system.md](design-system.md) | The visual token layer, where it comes from, and how to read it. |
| [invariants.md](invariants.md) | Every claim the rest of these docs make, and what enforces it: the tests and lints that catch a violation, and the four rules that only a reader will. |
| [testing.md](testing.md) | The four test layers, what each one catches, and their traps. |
| [decisions.md](decisions.md) | Consequential choices, and the alternatives that lost. |

## Where writing goes

Three folders, and the difference between them is how settled the thing is.

| Folder | Holds | Test |
| --- | --- | --- |
| `docs/` | what is true now | if the code changed, this changes with it |
| `proposals/` | a thing argued into a shape, not built | it has an order of work and open questions |
| `meditations/` | thinking that has not converged | it makes a question sharper and settles nothing |

A meditation that reaches a shape becomes a proposal. A proposal that gets built moves its
behavior here and its reasoning to [decisions.md](decisions.md). Nothing skips a step by
being written confidently.

Environment and editor setup live in [../DEVSETUP.md](../DEVSETUP.md).
[../CONTRIBUTING.md](../CONTRIBUTING.md) is the working reference for a contributor: the
commands, the hard rules, the testing traps and the comment style, with pointers here.
