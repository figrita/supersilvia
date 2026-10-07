# The canvas

## What is ours

There is no node-editor dependency. Nothing matching `snarl` or `node_graph` is in the
lockfile. The editor is some seven thousand lines in `src/ui/`, and the egui surface it uses
is small:

```
ui.painter()               rect_filled, rect_stroke, circle_filled, circle_stroke, text, add
ui.interact(rect, id, sense)   -> Response
ui::accessible(&response, ..)  its response.widget_info: what puts it in the accessibility tree
```

The built-in egui widgets on the canvas are few, and all but one live in its popups:
`selectable_label` rows in the select and asset lists, `DragValue` for the color picker's
channels, a `Button` for their worded actions, and a single-line `TextEdit` wherever text is
typed — a number's exact value, a color's hex, a tab's name, a card's blurb, the browser's
query, a one-line typed option, and a **plain number field** (`ui::text::number`). The last
is a number typed as text, with none of the s-number's hands — no scrub, no steppers, no
fill: it admits only what a number is written with, wears the invalid border while the draft
is not a number it would commit, commits once, on Enter or when it is left, clamped into its
bounds, and puts back what was there on `Escape`. The range editor's fields are four of them,
and an option that declares `OptionDef::number` is one — the Text node's size — as are the
resolution picker's width and height, in the s-number's bevel. The s-number and s-color are painted rectangles with
`ui.interact` over them, not egui widgets.

That trio — **painter, interact, widget_info** — is why the editor can look nothing like
default egui and still be fully in the accessibility tree.

## One pass

`ui::show` takes a `CanvasFrame` and returns `Effects`. The frame is everything the canvas
reads this frame, borrowed and never written: the graph and the workspace to draw, what the
last tick published — the notes, the scopes, the traces, a read-only view of the uniform
numbers — the assets and their posters, the costs, the MIDI bindings, the render in progress
the preferences and whether the Nodes menu is open. `App` builds it once a frame. The effects are everything the pass asks
the app to do: the `commands`, and every *request* beside them — the pictures to blit into
the slots the canvas reserved, the action buttons under a finger, a seek, a touch on a pad, a
pop-out, a render
button, a file dialog, a tag's workspace, the node drag in flight, a scrub or a node drag
abandoned with `Escape`, a control to learn or forget, a clip, and a note's measured height. `ui/` holds no GPU
handle, never opens a file dialog, cannot see the tab bar and does not hold the undo ring, so
all of them are things it describes and `App` performs — in one place, `App::apply_canvas`,
in the frame's order.

The pass runs in phases, each a function of its own:

- **the hand** — `Escape` puts back a node drag or drops a cable before anything moves;
- **layout** — every node on the workspace, once, in world units (see [Layout](#layout));
- **view** — the canvas's rect, the hand on the background, the creep at an edge, the glide,
  the strip's clamp, the ground and the grid, all settled before anything is drawn;
- **hit test** — every port placed on screen for that view, and what the pointer is on: the
  port under it, the cable nearest it and the ports that lights, and the cable a node in the
  hand would go into;
- **cables** — painted behind the nodes, their click handles held back;
- **nodes** — each node on screen drawn from a `NodeCtx`: the frame, the node, its layout and
  the view. Its body, its controls and its ports each take that one context and the one
  `Effects`, rather than the frame's maps an argument at a time and an out-parameter per
  answer;
- **drag** — the node drag in flight, then every cable's handle, the rubber band and the
  background's menu;
- **popups** — a control's popup, the browser and the conversion menu, above every node;
- **keys** — [the canvas's own](#the-canvass-keys), and a refused port's words;
- **drag end** — a cable let go, the wheel, the cost strips, the minimap and the selection
  count.

**Which widget wins a click is the order of the phases.** egui hands a tied hit test to the
widget registered last, so the order is the design: the background first, then each node's
ground, its regions, its header and its marks, its controls and its ports — and every cable's
handle after all of them, so a port's larger target cannot swallow the cable beside it.

## The tab bar

A row above the canvas, `ui/tabs.rs`, drawn the way `ui/menu.rs` draws the menu: it returns
`Vec<TabAction>` and never mutates. Leftmost and **pinned, the project tab**; then one tab per
**open** workspace in project order. A closed workspace has no tab and is still in the
project, which is why the project tab exists. Where the menu bar is the operating system's,
the row's right end holds [the problems badge](#problems), [the time
readout](#the-time-readout) and the frame-rate meter, which otherwise stand at the end of the
menu bar.

**Every tab is a `Response` with `widget_info`** — `tab project`, `tab Workspace 1` — so
`Ctrl+2` and a click on a tab are the same gesture to a test and to the agent-driven layer.
The name in the tree is the workspace's name, not whatever text fits in the tab.

**A tab's name is one line.** A tab is at most `TAB_MAX` wide, and a name longer than that
ends in `…` rather than wrapping: a wrapped name makes its tab two lines tall in a bar one line
tall. The whole name is under the pointer. The bar has no margin under it: a tab sits on the
canvas it shows.

| gesture | what it does |
| --- | --- |
| click | show that tab |
| double-click | rename inline |
| right-click | Rename · Duplicate · Close |
| `+` (*add workspace*) | one entry per `WorkspaceKind` with a UI — Video, today |
| the chevron (*all workspaces*), only once some open tab does not fit | exactly the tabs that do not fit, never one already on the bar; a click shows it |
| `Ctrl`+T | a video workspace, opened and shown |
| `Ctrl`+1..9 | the tab in that position, the project tab first |
| `Ctrl`+Tab, `Ctrl`+Shift+Tab | the next tab and the one before, wrapping at the ends |
| a node dropped on it | shows that node there too — `Command::ShowOn` |

**The bar never runs off the window.** The tabs that fit are drawn in project order and the
rest leave the bar, the one showing always among those drawn: where it is further along than
the room reaches, it takes the last place, because the lit tab is what says which canvas this
is. **The chevron is a real overflow, not a second way to the whole project**: it is absent
outright while every open tab fits, and once one does not, it stands after the `+` and lists
only the tabs that left the bar — never one already drawn — so a click there is reaching for
something the strip itself could not show. Whether it is needed, and which tabs it lists, is
worked out fresh from the available width every frame rather than carried from the last one,
so it never flickers in and out at the boundary: the room the tabs are measured against only
loses the chevron's own width once they do not fit without it (`tabs::fitting`). The `+` is
always drawn; the chevron, when it is, stands after it. Both are painted
([design-system.md](design-system.md#component-rules-worth-stating)).

**`Ctrl`+Tab walks the tabs `Ctrl`+1..9 counts**, the project tab first, and `Ctrl`+Shift+Tab
walks back; both wrap. `Ctrl` on a Mac as well as on Linux, as every browser binds it, rather
than the command key. A text field does not take the chord, so it is read whatever has the
keyboard, but not while a picture window does. Both are `menu::keys::NEXT_TAB` and
`PREVIOUS_TAB`, rows of the shortcuts window's Global group.

**A closed workspace comes back from the Project tab**, whose card for it offers Open. The
chevron does not list closed workspaces — it is the bar's overflow, not a directory of the
project — so this is the one way back. There is no history of closed tabs to walk back
through.

**Duplicate copies the workspace whole**, beside it in project order, as `<name> copy` —
`<name> copy 2` and up where that is taken — and opens the copy and shows it, looking where
the original was left. One `Command::DuplicateWorkspace`, so one undo step; what it copies and
which cables come is [in decisions.md](decisions.md#a-duplicate-workspace-is-a-variation-not-a-second-view).

**A new tab counts up until its name is free.** `Workspace n` where `n` starts at how many
workspaces there are plus one and rises past every name already taken — because the count is
where to start and not the answer. Close Workspace 2 of three and the count says three,
which is a tab already on the bar; rename one to `Workspace 7` and the trap is set further
out. Two tabs under one name is a name that names neither, and saying which tab you mean is
the one thing anyone does with a tab. It is not a promise that the number beats every other
tab's — with a `Workspace 9` on the bar the next is still `Workspace 4` if that is free —
because the rule is about collisions and nothing else. The name is chosen where the tab is
asked for and travels inside `Command::AddWorkspace`, so redo puts back the name that was
given rather than recomputing one against a graph that has moved on.

Both shortcuts are consumed at the top of the frame beside undo, with the same
most-specific-first ordering. `Ctrl`+1..9 indexes `TabBar::order`, which is the list the bar
draws, so the shortcut cannot drift from what is on screen.

**Dragging a node onto a tab shows it there** — silvia's gesture, and the one people
use. The tab under the pointer lights up while a node drag is in flight, and dropping on the
project tab does nothing, because a node cannot be shown on it. Neither half of that can see
the other: the tab bar is drawn before the canvas and has no idea a node is being dragged,
and the canvas cannot see the bar. So the canvas reports the drag in `Effects::node_drag`,
the bar reports the tab under the pointer as a `TabAction::DropTarget`, and **`App` joins
them** — which is what keeps `ui/` a thing that draws and returns rather than a thing that
reaches sideways. The bar tests `contains_pointer` rather than `hovered`, because egui takes
hover away from every widget while something else is being dragged, which is exactly this case.

**Opening, closing and switching are not commands** — neither is reopening. They are the
project's session state, saved in `project.ssp` and never in the undo history — see
[architecture.md](architecture.md#session-state-which-are-open-which-is-showing-and-each-view).
Renaming is an edit and goes through the bus. Close needs no confirm, because nothing is
lost: a closed workspace is still saved, its nodes are still in the graph, and they are
[suspended](architecture.md#open-and-closed-and-what-suspension-means) rather than unloaded.

## The time readout

The show's one clock, `ui/timecode.rs`: the playhead every node reads as its ambient time,
with the only two controls the transport has. It stands at the right end of the menu bar,
immediately left of the frame-rate meter, whichever tab is showing, the project tab included;
where the menu bar is the operating system's, it stands at the end of the tab row beside the
meter, since the bar it would stand in draws nothing of ours. [The problems badge](#problems)
stands left of it. **View ▸ Time** shows and hides
it, beside View ▸ Costs — the preference `show_time`, **on by default**, since pause
lives here. It draws and returns a `transport::Command` that `App::transport` sends — a hand
on the instrument, **never an edit**, so none of it reaches the undo history — and **nothing
of it is saved**: a project opens playing, at zero ([cpu.md](cpu.md#the-transport)). From the
left:

| piece | name in the tree | what it does |
| --- | --- | --- |
| pause | `time.pause` | `Pause` while playing, `Play` while paused; the glyph is what a press does, two bars while playing and a play triangle while paused, the triangle in the accent, since a show that is not moving is what somebody needs to see from across a room |
| the playhead | `time.playhead 00:12.34` | `mm:ss.ff`, hundredths of a second, the minutes growing to three digits past 99. A readout: a click does nothing |
| back to zero | `time.reset` | `Seek(0)`: a bar and a triangle pointing at it |

| key | what it does |
| --- | --- |
| `F8` | pause or play, whatever is showing — the key a Mac's keyboard prints ⏯ on, read only while no field has the keyboard and the editor window has the focus, as `H` and `F` are; View ▸ Pause, which reads Play while paused, is the same press. Not `Space`: a hand brushes it, and it would stop the whole show |

**Nothing on it moves.** The time is in tabular figures and laid out for `000:00.00`, so the playhead
crossing ten or a hundred minutes changes what it says and never where anything is: live data
never reflows the layout. Pause and back to zero are squares the height of the bar.

**Back to zero is a seek.** Every gear is born again at the start of its cycle, so the whole
show starts over together, and a stateful node — a simulation, a slew, a counter — carries
across it as it carries across any seek, and a free-running node moves by its Speed times the
jump. There is no speed, no loop and no typed seek on the transport: a node that should run
slower or faster has its own Speed, or a gear in its Time in Loop mode, and a loop is what a
Master Gear's caption says ([the gear region](#the-gear-region)).

**While a render runs the readout is disabled**: the render owns the playhead, the readout
shows the render's time, neither button answers and `F8` does nothing. When the render
ends the playhead goes back to where the live show left it, and every gear, pad, slime mold,
animation and feedback trail carries on as the render found it.

## The project tab

`ui/project.rs`, returning `Vec<ProjectAction>`, and it replaces the canvas while it is
showing — the preview panel stays, so an Output goes on rendering while the project is being
looked at.

**A tab, not a modal and not a pane** — a pinned first tab costs nothing while it is not
showing, is reachable by `Ctrl`+1, and is what an empty project shows. Why the other two
lost is in [decisions.md](decisions.md#the-project-tab-is-a-pinned-first-tab).

A **Workspaces** section: one card per workspace in project order, **open or not**, with its
picture, its name, kind, open-or-closed state and node count, and its blurb. Each card is a
`Response` with `widget_info` named `workspace card Workspace 1`; `UiBuilder::sense` registers
the card's own sense *below* the buttons on it, so a button takes its own click and the rest
of the card takes the rest.

| on a card | |
| --- | --- |
| click | open it and show it |
| Open / Close | give it a tab, or take it away |
| Rename | inline, on the card |
| Duplicate | the workspace whole, beside it, opened and shown — the tab's Duplicate |
| the blurb | a single-line editor in place — the line *is* the editor, and Enter or clicking away commits it as `SetBlurb` |
| Export | write it out on its own, wherever the folder dialog says |
| Delete | with an inline confirm, because it removes the nodes shown only there |
| drag | reorder, which is tab order |

**The picture is the last one a save read back**, loaded from `workspaces/<name>.png` and
re-loaded when a save rewrites it; a workspace with none — never saved, or its Output
unplugged — draws the placeholder rectangle instead, which is the same size, so the layout
does not move when the picture arrives. **One picture is read per frame at most**: reading a
PNG is a GStreamer pipeline, and doing every card's on the frame the page first appears is
exactly the thing the UI is not allowed to do.

An **Assets** section below it: one card per file in `assets/`, used or not, with a picture
where there is a reader for the format — a `.png` is read through `video/png.rs`, and a clip
shows its icon, since its first frame would be a decode pipeline of its own — its name, its size,
and **which nodes on which workspaces reference it**, each one going there when it is clicked,
the way a cross-workspace tag does. Each card offers Export (copy it out to a folder), Reveal
(the desktop's file manager on its folder, spawned and never waited on by the frame thread)
and Remove, which is [refused while anything references
it](architecture.md#assets) with the reason on the status line.

**A thing enters a project at the list and leaves from the thing.** Import… is at the top of
each section — a `.ssw` for workspaces, any file for assets — and dropping a file on the
window is the same import: a `.ssw` is a workspace and anything else is media. Export is on
the card, and on the Workspace menu, because it acts on one thing and that is where you are.

**A dropped file lands where it is pointed.** With a workspace showing, media makes a node on
that workspace — a `video`, or an `imagegif` for a picture — with its corner under the pointer,
and a second file a step down and across from the first. Where the pointer is over a panel or
the bar rather than the canvas, it lands at the centre of the view. **On Wayland winit hears no
drag at all**, so `platform::filedrop` does: a `wl_data_device` of its own on eframe's display,
on a thread, which takes only a `text/uri-list` over the editor's own surface and reads it as
the drag enters. `App::feed_file_drags` puts what it hears into egui's input before each frame,
as hovered and dropped files and, since a drag holds the pointer, as the pointer's position — so
a drop from Dolphin lands under the hand as one on X11, Windows or the Mac does. With the project tab showing there is no
canvas, so media is an asset and nothing else, and the Assets list is what it joins. A `.ssw` is
imported as a workspace on either. **While a file is held over the window** an outline in `primary`,
as a tab offered a node is lit, rings where it would land — the canvas, or the page — and one line at its top says what
the drop will make: `new Video node on Tunnel`, `tunnel.ssw: import as workspace`,
`gumbasia.webm: add to assets`, several files counted on the same line. The line is one line
of fixed height however many files are held, truncated rather than wrapped, and it goes the
frame the files leave or land (`App::show_drop_hint`, from egui's `hovered_files`).

**Every card and button is a `Response` with `widget_info`**, and where the text on one is not
unique the widget carries a name that is: `asset card gumbasia.webm`, `remove gumbasia.webm`,
`export Workspace 1`. Four cards each carrying a Remove would otherwise be four widgets called
Remove, and neither a test nor an agent could reach any of them by name. **The widget carries
the name; the geometry carries the click** — the same split cables and tags are drawn under.

## The menu bar

`Project · Workspace · Edit · View · Help`, described once as data by `menu::model` in
`ui/menu.rs` and drawn by whichever bar the machine has: `egui::MenuBar`, by `menu::show`, on
Linux and under every test; AppKit's own on the Mac, where `main` asks for it and
`platform::menu` builds it, with Settings… (`⌘,`) and Quit (`⌘Q`) in the application menu
beside About, Services and Hide, and a Window menu after View. **Help** holds Keyboard
shortcuts… (`F1`), see [the keyboard shortcuts window](#the-keyboard-shortcuts-window), then
About supersilvia and Licences…, see [About and Licences](#about-and-licences), then under a
rule Report a problem…, see [What a tester can send](#what-a-tester-can-send). On the Mac,
AppKit's standard About panel in the application menu answers About and Licences, so its Help
menu holds Keyboard shortcuts… and Report a problem…. There the
frame-rate meter, [the time readout](#the-time-readout) and [the problems badge](#problems)
stand at the end of the tab row instead, since the bar they stand in elsewhere is not the
window's. The
library is not among them: it is on a start button in the canvas's own bottom-left corner —
see [Three ways to the library](#three-ways-to-the-library).

**`menu::show` draws and returns `Vec<MenuAction>`; it never mutates**, the same shape as
`ui::show` for the canvas. What it needs to draw itself — what to enable, what to tick, what
the node verbs would act on — arrives as a read-only `MenuState`. `App::handle_menu` is the
one place that answers.

**Edit carries the node context menu's verbs** — Copy, Cut, Paste, Duplicate,
Collapse/Expand, Reset controls, Disconnect all, Delete — for the hand that looks in a menu
bar before it thinks to right-click. Same commands and the same undo steps: a second door, not a second
implementation. They act on **the selection on the workspace being looked at**, because the
canvas holds one selection across the whole project and a menu naming one node must not
delete one nobody can see. With nothing selected every one of them is disabled: a destructive
verb with no object is greyed out, not given a guessed one. Collapse/Expand reads the
selection the way the context menu does — one expanded node means Collapse, so a mixed
selection closes rather than scrambling. **Paste is the one entry there that does not read
the selection**: it wants something on the clipboard and a workspace to put it on, so it is
enabled by the clipboard alone and disabled when that is empty.

That is worth more than tidiness. Inline in `App::ui`, the menu closure reached into fourteen
different fields of `App`, and there was no way to see what a menu could do without reading
all of it. As an enum, the set is the type.

Graph edits among those actions go through the command bus like everything else and are
undoable; the rest — Open, Save, Reset view, **View ▸ Time** (see [the time
readout](#the-time-readout)), View ▸ Costs — are app state and deliberately are not commands.
Undoing a View toggle is not a thing anyone wants.


It is native on the Mac and not on Linux, where it cannot be — see [decisions.md](decisions.md)
for why, for why a menu bar rather than the design system's floating hamburger, and for how
the clipboard keys survive AppKit taking them first.

Edit carries Undo and Redo, on `Ctrl`/`Cmd`+Z and `Ctrl`/`Cmd`+Shift+Z (`Ctrl`+Y too), each
naming the step it would take — *Undo Delete 3 nodes*, *Redo Move Checkerboard* — and Undo
History… under them; see [Undo by name](#undo-by-name). The shortcuts are consumed at the top
of the frame, before any widget reads input, by `menu::shortcuts`, which answers each as the
`MenuAction` its entry sends — so a key and the entry it stands beside are one path through
`App::handle_menu`. `F1` and `Ctrl`/`Cmd`+`/` are consumed there too, for the shortcuts
window; `?` is not theirs, because it opens the node browser.

**Every key that has an entry is printed beside it**, in the bar's own shortcut
column: Project's Open, Save and Save as; Workspace ▸ New, `Ctrl`+T; Edit's Undo, Redo, the
clipboard three, Duplicate, `Ctrl`+D, and Delete node, `Delete`, which send the command the
canvas's own key sends ([The canvas's keys](#the-canvass-keys)); View's Pause or Play, `F8`, Hide editor, `H`, Fullscreen, `F`, and Zoom
in, Zoom out and Actual size, `Ctrl` with `+`, `−` and `0`; Help's Keyboard shortcuts…, `F1`.
`Ctrl`+1 to `Ctrl`+9 have no entry, since the tabs are theirs, and the shortcuts window lists
them. A key is said by its symbol where it has one — `Ctrl++`, `Ctrl+−`, `/` — where egui
would spell out `Plus`, `Minus` and `Slash` (`menu::said`). The **bare keys** — `F8`, `H`
and `F` — are `menu::bare_keys`, answered as the View entries they stand beside, and read
only under the guard the clipboard three have below. On the Mac only a chord with `⌘` held
becomes an AppKit key equivalent: AppKit takes a menu's key before the window sees it, so a
bare `H` there would stop being a letter in every field. So `⌘D` on the Mac is Edit ▸
Duplicate's before the canvas sees it, and does what the canvas's key does; Select all has no
entry, so `⌘A` stays the window's, where a field selects its text with it.

**A greyed-out entry says why on its hover**, in place of its hint while it is greyed: *A file
dialog is open.* on Project's and Workspace's file entries while a dialog is up, *Select a node
on this workspace first.* on the node verbs, *Copy or cut a node first.* on Paste, *Nothing to
undo.*, *A render owns the playhead until it is done.* on Pause, and the zoom at either end.
The node menu's Workspaces ▸ box that would leave a node on none says *A node has to stay on at
least one workspace.* The reasons are `menu::why`, and the Mac's bar shows the same text as
the entry's tooltip.

**View ▸ Zoom in, Zoom out and Actual size zoom the canvas**, about its middle, a quarter
in and out at a time between `canvas::MIN_ZOOM` and `MAX_ZOOM`, and Actual size is a world unit
to a point; View ▸ Reset view puts the pan back too. The keys are `Ctrl` with `+`, `-` and `0`,
as in every node editor. They are egui's own, but egui's `zoom_with_keyboard` is off and
`menu::shortcuts` consumes them as the entries: egui's would scale the whole editor, and how
large the whole editor is drawn is [the interface size's](#the-preferences-window) alone.

**`Ctrl`+C, `Ctrl`+X and `Ctrl`+V are consumed under a guard the others are not.** Undo is
taken above whatever holds the keyboard, because `Ctrl`+Z in a text field is an undo of the
graph either way. The clipboard keys are not: every `TextEdit` in the app — a hex field, a
typed number, a note, a workspace rename, the node browser's search — answers them itself,
and none of them guards itself, so taking them at the top of the frame would take the
clipboard away from every field there is. They are read only while nothing wants the
keyboard and the editor window has the focus, which is the rule the bare `H` and `F` follow
for the same second reason: a picture window holding the keyboard is not the editor.

**Redo is consumed before undo, and the order is load-bearing.** egui's
`Modifiers::matches_logically` rejects a shortcut only when the *pattern* requires a modifier
that is not held — it does not reject extra modifiers that are. The pattern `Ctrl+Z`
therefore matches a `Ctrl+Shift+Z` press, and consuming undo first made redo step backwards.
`tests/ui.rs` pins it.

**The Workspace menu exists only while a workspace is active.** On the project tab there is
none, because there is nothing for it to act on — `MenuState::workspace` is `None` and the
menu is not drawn. It holds New (`Ctrl`+T, the tab bar's `+`) · Rename… · Close · Export… ·
Layout ▸ Canvas / Linear · Auto-arrange. Rename… starts the tab bar's inline editor rather than
putting up a dialog, so there is one rename gesture and it is on the tab. Export… writes the
active workspace out on its own — see [architecture.md](architecture.md#export-and-import) for
what goes and what the report says stayed behind.

**Layout and Auto-arrange live here** because both are about one workspace — the mode is
its data and an arrange is an edit of its canvas
([decisions.md](decisions.md#project-and-workspace-menus-not-file)). Both edit the graph
rather than the app, so both go through the bus, and both name the active workspace, which
`App` fills in, as it fills in the arrange's height.

**Project** holds New project…, Open project… (`Ctrl`+O), Recent ▸, Save (`Ctrl`+S), Save
as… (`Ctrl`+Shift+S) and Show project folder; then MIDI…, whose map is saved in the project;
then Quit — eight entries, because opening, saving and finding the project is what it is for.
The project is [the only thing that is saved](architecture.md#the-project), so there is
nothing else for this menu to offer. *Show project folder* opens the project's folder in the
desktop's file manager. Open project… asks for a *folder*, starting in the projects folder: a
project is one. Its entries are shown **disabled** while a folder dialog is up rather than
hidden — the shape of the menu is part of what it communicates.

**New project… and Save as… ask for a name**, in one small modal (`ui/project_name.rs`): a
*Project name* field, selected, so Enter takes it and typing replaces it; the projects folder
it goes in, in monospace and cut in the middle; and the button that takes the name, **Choose
location…** and **Cancel**. Under *New project* the field holds the next free *Untitled N* in
the projects folder — `Untitled`, then `Untitled 2`, `Untitled 3` — and **Create** makes
`<projects folder>/<name>` itself, one empty video workspace in it. Under *Save project as* it
holds the project's own name where the projects folder has no folder of that name, and
otherwise the next free count after it — `Friday 2` for `Friday`, `Friday 3` for `Friday 2` —
and **Save** copies the project to `<projects folder>/<name>` and carries on there. Choose
location… puts the folder dialog up instead, starting in the projects folder for a new project
and beside this one for a copy, for anywhere else — and there a folder that already has
anything in it is refused rather than written into, as is a copy's folder inside the project's
own. The name is checked every frame against the folder, and a name that cannot
be had says why on a line of its own under the folder, in the error color, with Create off:
*Give the project a name.*, *A name cannot hold /* — and `:` on the Mac, which the Finder
shows as `/` — or *There is already a folder called Friday here.* The line is there whether
or not it says anything, so typing never moves the buttons. Spaces at either end are not
part of the name; Escape cancels. A projects folder that cannot be made or read is that line
instead, and Choose location… is the way on.

**Two questions and one notice, and they are the only other dialogs.** Quit, Open project… and New project… with
unsaved edits put up a small window — Save, Discard, Cancel — and the window's own close
button does the same, held with `ViewportCommand::CancelClose` until it is answered.

**The same question stands in front of a show going out**, edits or none: a picture window
open, the mix sent over NDI or published over Syphon, or a render running. With no unsaved
edits it says only what is going out and asks — *The mix is on a screen. Stop the show?*, *A
render is running (frame 120 / 300). Quit and cancel it?* — over **Quit anyway**, **Open
anyway** or **New anyway**, named for what it goes on to, and **Cancel**. With unsaved edits
the same lines stand under them, with *Save or Discard stops the show*. Going on with a render
running cancels it and waits for it to end before quitting, opening or starting, so what was
written stays whole (`app/onair.rs`). `Escape` on a picture window asks nothing: one `Escape`
closes one, which is [the key for it](#pictures-in-windows-of-their-own). A Save
there that fails does none of what the question stood in front of: nothing quits, opens or
starts, the edits stay on screen, and the question stays up with the reason under it to be
answered again. Discard throws the [autosave](architecture.md#saving-and-opening) away with
the edits and writes no more of them, so the next launch asks nothing: a recovery is only
ever offered for edits a crash, a kill or a lost GPU took. The window title is the project's folder name with a marker while there are
unsaved edits, so the question is never a surprise.

The other is **the recovery question**. Opening a project — Open, Recent, the command line,
or the launch that reopens the last one — whose `.autosave/` is newer than its last save and
holds a different document asks *Recover unsaved changes from 4 minutes ago?*, over the
project as it was saved, with **Recover** and **Discard** and no Cancel. Recover puts the
autosaved document on screen, tabs and views and MIDI map and Main Input with it, **still
unsaved**: the title carries the marker and the person saves to keep it, and until then the
autosave stays on disk. Discard deletes it and leaves the project as saved. The time is
relative — *a moment ago*, *4 minutes ago*, *2 hours ago*, *3 days ago* — because the binary
carries no timezone database to say a clock time in.

The notice is ***supersilvia closed unexpectedly***, on the launch after a run that went down,
and it comes first: the recovery question waits behind it until **OK**, since the notice says
why there is anything to recover. It says why the last run went down, as that run's log says
— *The last time it ran, it stopped with:* and the reason, cut at 300 characters — or that it
wrote no reason, which is the system stopping it or a driver failing underneath it. **Show
log** opens the folder holding that log with the file selected. A line under the reason says
Help ▸ Report a problem… puts the log in a bug report. See [What a tester can
send](#what-a-tester-can-send). Nothing else asks anything.

**It is modal, and it closes the menus.** An `egui::Modal`, not an `egui::Window`: a window
draws at `Order::Middle` and both ways to the library are `Order::Foreground` areas, so the
one question in the app could be covered by a list of nodes. The modal draws above them and
its backdrop takes the clicks the canvas underneath would answer — and because `n` and `/`
are read inside the canvas rather than through a widget, a list that opens under the question
is closed on the frame it opened. The arrange offer is the same kind of thing and is the same
kind of window.

### Undo by name

Every step in the undo ring already carries the command that made it, so a step's name is
read off that command: `Command::name`, against the graphs on either side of it — a deleted
node is named from the graph it was deleted out of, an added one from the graph it was added
to. A node is its kind's label and a row the label it wears, so the names are what a person
did: *Add Checkerboard*, *Delete 3 nodes*, *Move 2 nodes*, *Connect*, *Change Checkerboard
Frequency*, *Reset Zoom range*, *Show 2 nodes on Second*, *Auto-arrange*. Nothing new is kept;
a gesture that wrote several things is named by the last, as its step is.

**After an undo or a redo, the toast says what changed** — *Undid Delete 3 nodes*, *Redid Move
Checkerboard* — and where the change is not in view it carries a **▸ go** and stays eight
seconds, as a Show does. The go is `App::navigate_to` on the first node the command names that
is still in the graph, which opens its workspace, centres the view on it and throbs it; where
the command named no node or its nodes are gone — an Add undone — it opens the workspace the
command named. A change on the workspace showing, with its node inside the canvas, says what
it was and offers nothing. The toast comes from `App::handle_menu`, so the Edit menu, `Ctrl`+Z
and the Mac's bar say it alike; `App::undo` itself says nothing.

**Edit ▸ Undo History…** is a window of the ring's steps by name (`ui/history.rs`), of one
fixed size whatever the ring holds: row 0, *Start*, as far back as undo goes; every step that
can be undone, oldest first, numbered, the one the graph on screen stands at chosen; then
every step that can be redone, dimmed, the next one first. Rows are one height, truncate
rather than wrap and are drawn only while in view, so a full ring costs what a short one does.
A click on a row undoes or redoes to it in one walk (`App::travel_to`): the graph goes on
screen once at the end however many steps it crossed, and the toast says *Undid 3 steps,
through Add Output*. It opens scrolled to the step on screen, and its place is kept like
About's. A render running refuses the walk, as it refuses an edit.

## Three ways to the library

How you reach a node is three gestures rather than one — silvia's three, because they are
the ones that were used — and none of them is on the menu bar, which is for what the app
does rather than what the instrument is made of
([decisions.md](decisions.md#three-ways-to-the-library-and-the-background-right-click-is-one-of-them)).

| gesture | where the list is | where the node lands |
| --- | --- | --- |
| **the ▲ Nodes button**, bottom-left, or **`n`** | standing on the button, by category | staggered from the top-left of the view |
| **right-click the canvas** | at the pointer, one searchable list | under the pointer |
| **`/`, `~` or `` ` ``** | at the top of the canvas | in the middle of the view |

The last two are one thing shown twice: `ui/browse.rs` draws a search field over every node
in the registry, and the only difference between them is where the panel sits and where the
node lands. That pairing is silvia's — its quick menu and its quake bar are the same DOM
node — and it is why the browser holds both positions itself rather than being told them.

**The rows are the nodes' shapes, not their names.** Each carries the icon, the label, and
the port glyphs the node would arrive with, in port colors and shapes, a circle, a diamond or a
square as the node draws them, in the node's own left-to-right order. Choosing by shape is how anyone patching at speed reads a menu, and the same row
function draws the Nodes menu's submenus, so the two lists cannot drift apart.

**The scoring is silvia's, kept whole because it is tuned.** A query matches when its
characters appear in order in `"{label} {slug}"`; each matched character scores the length of
the run it lands in, so `chkr` beats the same letters scattered; and then three bonuses
decide the order anyone actually notices — what *starts* with the query, then what has a word
starting with it, then what merely starts with its first letter. Ties keep label order,
because the list starts sorted and the sort is stable. `Enter` takes the top of the list,
which is what makes typing three letters and pressing `Enter` the fastest way to place a
node.

**A cable let go over empty canvas opens the same browser** at the pointer, narrowed to the
kinds that can take the cable, and the kind chosen lands wired: see
[Cables](#cables). It is the library asked a narrower question rather than a fourth way to it.

**The browser carries a Paste row when the clipboard holds something**, above the search
field and outside the list. A right-click on the background means *something here*, and what
was copied is one of the things it can be; the clip lands exactly where a node chosen from
the same panel would. Above the field rather than in the list because the list is the
registry — a row that is not a node would be caught by a search for its letters and taken by
`Enter`.

**A right-click on the canvas asks for a node, unless several nodes are selected.** The
background is where the pointer is when the answer is *something new here*, and one node's
menu is on that node. With more than one selected, the pointer has no single node to be
over, and a right-click anywhere in the editor opens the selection's menu instead: see
[Acting on a selection](#acting-on-a-selection).

### The Nodes menu: a start button, two timers, and a mouse that speaks in crossings

`ui/start.rs`. The button floats in the canvas's bottom-left corner — silvia's
`#nodes-menu-btn`, 22 points tall, 8 in from the corner — and the menu stands *on* it, so the
button squares off its top corners while it is open and the two read as one object. Its
hover, while the menu is shut, names the two keys that reach the library without it, `N` and
`/` (`menu::keys::NODES` and `BROWSER`). `n`
toggles it, which is silvia's Windows key. It is drawn after the canvas and only where there
is one: on the project tab there is nothing for a node to land on.

**The categories are `Category::ALL`, in roughly signal order**: Source, Generate, Color,
Transform, Effect, Convert, Tap, Math, Control, **Gears** and Output. Gears, under `⚙`, holds
the Master Gear, the Ratio Gear and Time — the clocks, the one place a rate is set, changed
or divided — between the values that change over time and what a picture ends in. **A
category of one node is that node**: Output's row has no ▶ and no submenu, and a click or
`Enter` adds the Output, since a submenu holding one entry of the same name chooses nothing.

**The button is canvas furniture and a window is not**, so a window covers it. The button is
an `Area` — it has to be, for the menu to stand on it — at `Order::Background`, which is
above the central panel the canvas paints into, because a later layer at the same order draws
over an earlier one. So it still floats over the nodes and still falls under the Status box
or the MIDI window when one is dragged there. It was `Order::Foreground`, which paints over
an `egui::Window` — those are `Order::Middle` — and `Middle` is not the fix, since same-order
layers fall back to insertion order and the button still won. The test drags the *Preferences*
window over it rather than the Status box, whose every line is a live timing: a snapshot of
one would differ from itself between runs. **The menu it opens stays
`Foreground`**: it is open only while a hand is in it, and a window over the list of nodes is
worse than one over the thing that opened it.

**The mouse speaks in transitions, not in positions.** A browser fires `mouseenter`,
`mouseleave` and `mouseover` when the pointer *crosses* something; a pointer sitting still
says nothing at all. Every timer and every selection change hangs off a crossing — `Over`
and `StartMenu::over` are last frame's answer, and `crossings` is the whole mouse model: the
menu acts only when this frame's answer differs from last frame's. The two faults a menu
built on `hovered()` gets instead are in
[decisions.md](decisions.md#three-ways-to-the-library-and-the-background-right-click-is-one-of-them).

**The two timers, and they are asymmetric on purpose:**

- **A category the pointer *enters* opens its submenu after 200 ms.** Crossing three
  categories on the way to a fourth therefore flashes nothing open. Clicking one does not
  wait. Entering another cancels that one's timer, not this one's.
- **A submenu survives the pointer *leaving* it — or leaving its category — by 300 ms.** That
  is what lets the pointer cut the diagonal to the far corner of a submenu instead of
  tracking along the narrow valley a menu without that delay demands. Reaching the submenu
  cancels the close.

Neither timer ever runs while the pointer is still, which is why the keyboard can drive the
menu with the mouse sitting in the middle of it.

**The deadlines are checked before the menu is drawn, not after.** A submenu whose delay
expired during this frame belongs on screen in this frame; checked afterwards it opens a
frame late, which is exactly the sixteen milliseconds these delays exist to spend well.

**The categories and the open submenu are one `Area`, not two**, and the panel's height is
computed from its rows rather than measured. An `Area` reports the rect it had when it was
last laid out, so anything placed from another's rect lands a frame behind — and the panel
stands on the button, so a height that is last frame's puts this frame's menu in the wrong
place. Rows are laid out with **no item spacing** for the same reason: the computed height
has to be the drawn height, or the panel hangs over its own button and every click on the
button lands on the last category. **The `Area` is unconstrained**, because that geometry is
the geometry: a submenu with more entries than there are categories is pushed up to fit the
screen and so reaches above the area's own origin, and egui's constraint answers that by
sliding the whole area down — which moves the categories out from under the pointer on the
frame their submenu opens.

**A sizing pass is not geometry.** The first time an `Area` is shown, egui lays its content
out once at a position it has not worked out yet, to measure it, then discards that pass and
runs the frame again. A person never sees it. The mouse model must not read it either — the
provisional layout puts *some* category under the pointer, and reacting to that selects a
category nobody pointed at. `ui.is_sizing_pass()` is the guard.

**The keys are read once per frame, not once per pass.** egui may run a frame's UI more than
once — a discarded pass and then the real one — replaying the same input each time, and a
menu that steps its selection on every pass moves two entries for one press.
`Context::cumulative_frame_nr` is what makes that exactly once.

**The keyboard is Windows's**, and every clause of that is deliberate: the menu opens with
*nothing* selected, so `Down` is the first entry and `Up` is the last; both wrap; `Right` and
`Enter` go into a submenu and `Left` and `Escape` come back out of it; `Escape` outside one
closes the menu. Opening a submenu with the **pointer** takes the keyboard into it with
nothing selected — silvia's `onSubmenuShown` — so `Down` then walks its entries and `Enter`
on nothing does nothing; opening one with **`Right`** selects its first entry, because a key
that goes in has said which way it is going. And the pointer moves the selection only in the
half the keyboard is in: crossing a category while the keyboard is inside a submenu opens
that category's submenu on its timer but does not move the highlight, which is silvia's
`handleMouseOver` exactly.

**One menu at a time.** Opening the browser closes the Nodes menu and opening the Nodes menu
closes the browser: two lists of the same library on screen at once is one too many. Each
records that it opened and `App` closes the other, which keeps the canvas's state and the
start menu's from having to know about each other.

## Preferences

What is remembered between runs lives in one file, `preferences.json`, in the machine's config
directory (`platform::dirs`): on Linux `$XDG_CONFIG_HOME/supersilvia/preferences.json`, or
`$HOME/.config/…` when the first is unset, on macOS
`~/Library/Application Support/supersilvia/preferences.json`, and on Windows
`%APPDATA%\supersilvia\preferences.json`, the roaming `AppData` Known Folder. `SUPERSILVIA_PREFERENCES`
overrides the path, which is how a test keeps its hands off the real one. The Preferences
window's Files section shows where it is.

What is in it is what the tier test lets in: a preference is about the *person*, so two
people opening the same project may reasonably disagree about it and the project still means
the same thing.

| field | what it holds |
| --- | --- |
| `version` | the file's own version, not the app's: moved only for a change this reader could not cope with |
| `show_status_box` | the Status box, Preferences ▸ Performance, so it survives the evening you turned it on |
| `show_fps` | the synth's frames a second at the right-hand end of the menu bar, in the accent once it is five per cent short of the tick rate, Preferences ▸ Performance. Off by default |
| `show_costs` | the cost strip under every node, View ▸ Costs, the same way |
| `show_time` | [the time readout](#the-time-readout) beside the frame-rate meter, View ▸ Time. On by default, since pause lives there |
| `lock_cursor_while_scrubbing` | grab and hide the pointer while a number control is dragged, so the scrub is relative motion with no screen edge. Off by default — see [below](#the-number-control) |
| `theme` | the four anchors every color in the editor derives from — see [the Preferences window](#the-preferences-window) |
| `port_hover_highlight` | hovering a port lights every cable it carries and the port at the far end of each, and hovering a cable the ports at both its ends. On |
| `cable_droop` | let a cable sag between its ports. On, as in silvia |
| `phi_cables` | color every cable by a golden-angle walk of the hue circle rather than by what it carries, and outline each connected port to match. Off, where silvia has it on |
| `node_shadow` | cast the window shadow under every node body. On |
| `scroll_x_inverted` | which way the wheel moves a Linear workspace along its strip |
| `default_layout` | the mode a new workspace opens in. A workspace's own mode is saved with it |
| `main_input_collapsed`, `mixer_collapsed` | the Main Input panel folded to the left edge and the Main Mixer to the right, each to its spine. The Main Input starts folded and the Mixer open |
| `main_input_width`, `mixer_width` | the width each panel's edge was last dragged to. Absent is the panel's own, 300 and 380 — see [the two side panels](#the-two-side-panels) |
| `tick_rate` | how often the synth ticks: the editor's display, 60 or 30 — see [below](#the-preferences-window) |
| `status_folds` | which of the Status box's sections are folded, and whether it lists every Output — see [the Status box](#the-status-box) |
| `window` | inner size, outer position and maximized, so the second launch opens where the first closed |
| `interface_size` | `Percent90` to `Percent150`: how large the whole editor is drawn — see [the Preferences window](#the-preferences-window) |
| `windows` | where each of the editor's own windows was left, by title — the Status box, Preferences, MIDI, About, Licences, Keyboard shortcuts and Undo History: the outer top-left corner, and the outer size of the two that resize |
| `projects_dir` | the projects folder, where New project makes a project, Untitled is made and Open project… starts — chosen in [the Preferences window](#the-preferences-window). Absent is the default, `supersilvia` in the documents folder |
| `recent` | project folders opened or saved, most recent first, deduplicated, capped at ten |

Which things belong here rather than in the project or a workspace is [the tier
test](architecture.md#three-tiers-of-saved-state). Four rules hold the file together:

1. **`#[serde(default)]` on the struct, and `Default` is exactly what the app did before
   there was a file.** A missing file, a truncated one and one from a newer build all load as
   the defaults with a `log::warn`; a field this build does not know is ignored. Adding a
   field never needs the version to move. **A file that is there and cannot be read is kept**:
   nothing is written over it until something changes, and the first write renames it
   `preferences.json.bad` beside the new one, so a hand edit gone wrong is not lost.
2. **One store, ours, geometry included.** eframe's `persistence` feature is off, so no
   `app.ron` is written or read: the window, the UI zoom and where each of our own
   `egui::Window`s stands are all in this file, and nothing else overrides them at start-up
   ([decisions.md](decisions.md#one-preferences-store-ours-geometry-included)).
3. **`App` owns it and nothing else imports it.** It is passed by reference where it is
   needed — the menu takes `recent` as a read-only slice — and there is no global. `graph/`,
   `compile/`, `nodes/`, `render/`, `audio/` and `video/` may not name it at all: **no
   preference reaches the shader**, which `tests/rules.rs` checks.
4. **A preference is never a `Command`.** It does not go through the bus and never enters the
   undo history, the same as Open, Save and Reset view. It is written to disk on change,
   debounced to the end of the frame: a dirty flag checked once, at most one write per frame.
   An unwritable path logs and the run continues with the preferences it has in memory.

The window's geometry is read back from the viewport every frame and rounded to whole points,
so a fractional wobble is not a change worth saving. A maximized window records only that it
is maximized, keeping the size and position that unmaximizing restores. Wayland reports no
window position, so `position` stays `None` there and the compositor places the window.

The interface size is handed to egui as its zoom factor before the first frame and again at
the end of every frame; it is never read back from egui. An editor window's place is read out of egui's own memory of it once a frame, by its
title, and handed back as that window's default position — and default size, for the Status
box and the MIDI window, which resize — the first time it is built in the next run
(`ui::placed`). A window that has never been opened is not in the file and opens where it
always has; egui keeps it on the screen if the screen has shrunk since.

**`Project ▸ Recent`** lists the folder names, with the whole path as hover text. An entry
whose folder is gone is shown **disabled** rather than dropped: the list is what was opened,
and a missing entry says more than an absence does. It is also what the next launch reads:
**the most recent project that is still there is the one that opens**, and a machine with
none gets a new `Untitled` in the projects folder, with the status line naming its whole path. `App::save_project`, `App::open_project` and `App::new_project`
are what push to it, so every route in — the dialog, the command line, the launch — is
covered by the three that already exist.

## The Preferences window

`Edit ▸ Preferences…` opens an `egui::Window`, not a modal overlay.

The canvas is painted by hand because it is an instrument. This is a settings screen, so it is
ordinary egui widgets. That means it lands in the accessibility tree on its own, with no
hand-written `widget_info` anywhere in it.

**Four tabs: Appearance, Editing, Performance and Files.** The strip along the top is the
Licences window's, a row of egui's selectable labels, the tab showing lit. Appearance holds the
interface size, the four colors, the presets and how nodes and cables are drawn; Editing, how
the editor answers a hand and a controller; Performance, the tick rate, the two frame-pacing
readouts and the GPU; Files, where things are kept.

**Every tab is as tall as the tallest**, so a click on the strip never moves the window's foot.
The heights are measured when the window opens, and again when the interface size changes:
each tab is laid out once in an invisible child that takes no room in the window and answers
no pointer, and the area under the strip is fixed at the greatest. A shorter tab leaves room at
its foot. Where the screen is shorter than that height, the tab scrolls inside the window under
the strip.

**The window opens on the tab it was closed on**, for the rest of the run; a new run opens on
Appearance. The tab is not a preference.

**The four colors are four `s-color` swatches.**

They are the real control, the same one a color port carries: same popup, same hex field,
same bevel. Learn to pick a color once and you have learned it everywhere.

Clicking a swatch opens the picker over the window. On the frame it opens, the picker ignores
its own dismissal. The click that opened it is also a click outside a popup that had not been
drawn yet. The canvas carries the same guard for a node's picker.

A theme color is stored as HSL. The picker works in RGB. So a color picked there comes back
through `Hsl::from_rgb`, which keeps the hue you started with when the result is a gray.
Without that, dragging saturation down to nothing and back up again would return a different
color.

**Sixteen presets**, silvia's own, ported from `js/settings.js`.

silvia gives each look six colors. Two of them are for port types this editor does not have,
so they are dropped rather than given a use.

The look currently on screen is marked. It stays marked only while the four colors are still
that preset's. Drag one and nothing is marked, because none of the sixteen is what the editor
is wearing any more.

**Live preview, and no OK button.**

Drag a color and three things happen in that same frame. The editor draws with the new
colors. egui's own `Visuals` are rebuilt from them, so windows and menus match the canvas.
And the preference is written to disk.

There is nothing to confirm and nothing to roll back. The way back to where you started is the
`vapor` preset, which is exactly what the editor ships with.

**Appearance also holds the interface size and, under *Nodes and cables*, three answers about
how the canvas is drawn.**

*Interface size* is **90%**, **100%**, **110%**, **125%** or **150%**: egui's zoom factor, the
one control over how large the whole editor is drawn, for a display whose own scale is not the
one wanted and for eyes that want everything larger. It is the zoom factor and not the fonts
because a node's rows, a number field and a panel's width are fixed sizes in points, each sized
for the text it holds: a larger font alone would clip in every one of them, where a larger point
grows text, rows, controls and panels together. It sits on the display's own scale,
`native_pixels_per_point`, and the canvas's own zoom works inside the points as it always does.
Everything that turns points into pixels reads egui's `pixels_per_point`, so a drag, a dropped
file and the inspection protocol's coordinates — logical points — line up at any size. A
picture window is not egui and is untouched. **There is no key for it**: `Ctrl` with `+` and
`-` zoom the canvas, and a size that is set once for a screen and a pair of eyes does not need
one. It is applied at start-up and is not project data and not undoable.

*Nodes cast a shadow* puts the editor's own `window_shadow` under every node body — the
shadow the Status box and the Preferences window already cast, so the canvas and the windows
over it agree about where the light is. It is one rounded rect with a wide feather per drawn
node, scaled by the zoom so it stays fixed to the body, painted immediately before that
body's fill: under its own node, over the cables, and over any node drawn before it, so an
overlap reads as a stack. On by default. Off is the flat canvas, where the graph sits in the
mix rather than floating over it.

*Droopy cables* lets a cable sag between its ports. silvia's `droopyCables`, and its sag:
fifteen points plus a seventh of the span, stopping at eighty. Only a forward cable sags. One
that runs backwards already bows downward to clear the node bodies, and a second reason for
the same curve would fight the first. An action cable stays straight, because a dashed line
means *event* and a sagging one would read as the cable a data port uses.

*Phi-spaced cable colors* gives every cable a hue of its own, walked around the circle by the
golden angle so that neighbours are as unlike as they can be, and outlines each connected
port to match. silvia's `phiSpacedWires`, and **off** where silvia has it on: our port colors
are this editor's type system drawn, and a cable in its port's hue is that system continued
along the wire. What phi spacing buys instead is telling two cables in a bundle apart, which
is worth more the denser a patch gets and nothing at all on a small one — so it is offered
rather than assumed. See [cables](#cables) for what it does to a port.

**Five answers on the Editing tab.**

*Lock the cursor while scrubbing* is on the Editing tab and nowhere else. It is taste rather
than something toggled for an evening, and taste is what somebody opens this window for.

*Light cables and ports on hover* works from either end: a hovered port brightens every cable
it carries and the port at the far end of each, and a hovered cable brightens the ports at
both of its own ends. On by default. It does not touch the highlight on the cable nearest the
pointer, which is the click-to-delete affordance and stays either way.

*Invert scrolling along a strip* flips which way the wheel moves a Linear workspace. It is a
preference because Linear mode is the one place the wheel means *x*, and which way that should
go is a property of the hand rather than of the workspace.

*New workspaces open as* picks Canvas or Linear for the next workspace made. It changes
nothing about the ones that exist: a workspace's mode is its own document data, saved in its
own file, and opens the way whoever made it left it.

*MIDI soft takeover*, **off by default**: on, a bound fader or knob whose position disagrees
with its control moves nothing until it passes the control's value, then takes over, so the
picture never jumps when a knob is first touched after an undo, a reload or a hand on the
control. Meanwhile the control wears [a ghost mark](#alt--click-to-bind) where the fader is.
Off, the first message writes. See [media.md](media.md#the-map).

Changing the theme is not a `Command` and never enters the undo history, like every other
preference.

`tests/ui.rs` holds two things here. That a theme change adds no undo step. And the design
system's headline claim, as two pictures: the same graph under two presets. Any pixel that
differs, differs because of those four colors — so a hardcoded color anywhere in the editor
shows up as a pixel that refused to move.

**Three answers on the Performance tab, over the GPU.**

*Tick rate* is how often the graph advances, in ticks a second: **Display**, 60 or 30.
Display — the default — follows the monitor the editor is on. The synth keeps its own time on
a thread of its own, so this is the rate the *world* runs at and not the rate anything is
drawn at: every window shows the newest frame whenever its own compositor asks it to, and a
picture window at 60 beside an editor at 100 shows every other frame twice. The two fixed rates are
for a box whose display is not the rate the set wants to run at — a 144 Hz panel with the
show going to 60. See [architecture.md](architecture.md#one-clock-on-a-thread-of-its-own).

*Frame rate in the menu bar* puts the synth's own frames a second at the right-hand end of the
menu bar, one figure always in view, for the evening the Status box is too much. It is in the
accent once the synth is five per cent short of the rate it is asked to tick at, the line
[the Status box](#the-status-box)'s verdict turns the accent at (`status::short_of`), and its
hover says so. *Show the
Status box* opens [the Status box](#the-status-box), and its own ✕ unticks it: the two are one
preference. It is here rather than under View because it is about the machine, as the tick
rate is, and the View menu keeps what changes what the canvas shows — the time readout and
the cost strip.

**Where things are kept, on the Files tab.** Two rows, each a caption with its buttons on the
right and its path on the line under it:

- **Projects folder**, with **Show in Files** — **Show in Finder** on the Mac — which makes
  the folder if nobody has yet and opens it, and **Change…**, a folder dialog starting there.
  The folder chosen is the `projects_dir` preference: where New project makes a project,
  Untitled is made and Open project… starts. Choosing the default's own path stores no choice.
  A folder that cannot be read says so on a line under its path, in the error color — *could
  not read the projects folder … : Permission denied*, and on the Mac what to allow in System
  Settings ▸ Privacy & Security ▸ Files and Folders, which is where a *Don't Allow* on the
  Documents question leaves it.
- **Preferences file**, with **Show**, which opens the folder holding `preferences.json` with
  the file selected where the file manager can, and **Open**, which opens it in the desktop's
  editor for it — `open -t` on the Mac, so a text editor rather than whatever claims `.json`.
  Both write the file first if nothing has yet. One muted line under it says an edit takes
  effect the next time supersilvia starts, since it writes its own copy while it runs; an
  edit that does not parse is [kept aside](#preferences) rather than lost.

**A path is one line whatever it is**: monospace, cut in the middle with `…` where it is
longer than the window is wide — the start says which disk and the end which folder — and
whole on its hover, so no path moves a row.

**What it draws on, under GPU** at the foot of the Performance tab, read-only: a grid of a
caption and its value. *In use* is the adapter the editor and the synth draw on — its name, then its kind (*integrated*, *discrete*,
*software* or *other*) and backend, then its driver and the driver's own version string, as
wgpu reports them. *Picked by* is one line: `SUPERSILVIA_ADAPTER=…` when the variable named it,
or *the strongest: a discrete GPU before an integrated one*. *Offered* is every adapter the
machine reported, in its order, the same three lines each behind a painted dot from
`ui::icon`, as every status dot is: the one in use filled, named *in use* for the tree, with
*(in use)* after its name, and the rest hollow, named *not in use*. Every value is one line, cut at its end and whole on its hover.
What can later be chosen here — the colour precision, the GPU itself — is a row under the
last. The list is `render::adapter::Choice`, the enumeration `main` already made to pick the
adapter, handed to `App` beside the device: listing the adapters opens no device on any of
them. A host that hands the app a device of its own choosing, as egui_kittest does, has
none, and the section says *Not reported by this host*.

`tests/ui.rs` draws both tabs whole with a preferences file and a projects folder of the
test's own and a made-up machine of three adapters, so the snapshots do not depend on the
GPUs of the machine they run on (`preferences_files`, `preferences_performance`); Appearance is
`preferences_window` and Editing `preferences_editing`, taken by the test that holds every tab
to the one window rect and the reopened window to the tab it closed on.

## Layout

`canvas::rows` is the single source of node layout: one list giving order and height for
inputs, then outputs, then options. Node height, port centers and row hit rects all derive
from it, which is what stops them disagreeing when heights vary (a row carrying a control is
taller). It returns an **iterator, not a `Vec`**: an allocation per node per frame would be the
largest single cost in the canvas.

**A node is laid out once a frame.** `canvas::Layouts` walks every node on the workspace at
the top of the pass and keeps a `NodeLayout` for each, in world units: the body, every row
with its top and height, every band below the rows with its rect, and every port's dot.
Everything that draws or hit-tests a node reads that and nothing else — the cull, the shadow,
the body and its controls, the ports and the cables, the marquee, the cost strips, the strip's
bounds and the minimap — so nothing walks a node's rows twice, and a region's size is asked
once: an Output parses its `resolution` once a frame rather than every time something wanted
its height. World units, so the layout does not wait on the pan it is used to clamp; each
port is moved to the screen once the view has settled. The buffers are the canvas's and are
reused from frame to frame, so laying a workspace out allocates nothing once it has been done.

Whatever needs one node outside a frame — an arrange, a clamp to the strip, following a tag,
a test — asks `Layouts::one`, which lays that node out alone by the same walk. There is one
layout path, so there is nothing for a second one to drift from.

`Node::options` is a `BTreeMap`, so the file and the command bus stay deterministic; row
*count* comes from it — `canvas::rows` only needs `node.options.len()`, agnostic to which
option sits at which index. Row *order* is `node_widget::controls`' to answer, and it answers
from the definition's own declaration order rather than the map's alphabetical one: an
option's position in `NodeDef::options` is an authoring decision — `waveform` before `axis`
before `mode` is how the node explains itself — and the two need not agree on how they are
stored versus how they are drawn.

## The canvas shows one workspace

The canvas draws the nodes whose set holds the active workspace: a second filter where the
culling already filters, applied before the off-screen test, so a node on another workspace
is not drawn, not hit and not laid out. It has no port geometry either, which is what makes a
cable with one end elsewhere simply absent rather than drawn to nowhere — **a cable is drawn
when both its ends are visible.**

The selection follows: a node not on the workspace on screen is never selected, never hit and
never moved by a drag, and switching tabs clears it. Which workspace is showing, which are
open and where each canvas was left are the project's session state — see
[architecture.md](architecture.md#session-state-which-are-open-which-is-showing-and-each-view).

### The tag: a cable whose far end is elsewhere

A cable with one end on another workspace is not drawn, and the ports it touches say so
rather than looking unplugged.

- **The input** it feeds keeps its connected border — it *is* connected — and grows a **pill
  beside the port**: the source node's `NodeDef::icon` and the workspace it is on, in the
  cable's own port color, **dimmed while that workspace is closed**, because closed is still
  in the project. An action input takes many sources, so a source elsewhere gets a pill each,
  stacked leftwards from the port. The icon is drawn `theme::ICON_BUMP` larger than the name,
  like every other icon paired with text, and the pill takes its **height from what is in
  it** — floored at `TAG_HEIGHT`, since a fixed height would clip the larger glyph and a pill
  taller than `PORT_PITCH` would run into the row above. `tests/ui.rs` pins that bound.
- **The output** a cable leaves toward a node that is not here keeps its connected border and
  says where it goes **on hover**: `to output2.input on Cameras`. There is no room for a pill
  on that side — a cable leaves an output rightwards into whatever is there.

**Which workspace the tag names** is the first **open** one in project order among the source
node's set, because that is where clicking can go without opening anything; a node on none
that are open names the first closed one, and clicking opens it. That is `Graph::home_of`,
and every link that shows a node goes by it — a tag, an asset card's user, a name in the
Status box, a MIDI mapping, a deck's workspace and an Output's `!` — preferring the tab being
looked at where the node is on it, so no link opens a tab where one already open would do.

**Clicking a tag navigates**: activate that workspace, opening it if it is closed, and pan so
the source node is centerd. It is a *request* — `Effects::navigate` — that `App`
performs, and it is session state, not a command: opening a tab, switching to it and panning
never enter the undo history.

**The node that was asked for then throbs**, a ring in the accent just outside its border,
brightest on arrival and gone in about a second. Centring alone lands the view on a node that
looks like every other node around it, so the throb is the answer to the click. It is
brightest first rather than swelling in, because it is catching an eye that has just been
moved somewhere else; and it is a detached halo where a selected node's border is a stroke
*inside* its own, so the two do not say the same thing. Every route in shares it, because
every route in is `App::navigate_to` — a tag, the status box, the MIDI panel, the mixer's
*Go to*, the project panel's *Reveal*. The clock is egui's and the start is stamped on the
first frame that draws, so a link followed while the window is not painting still throbs in
full when it comes back.

Each tag is a `Response` with `widget_info` named
`tag from {slug}{id}.{key} on {workspace name}`. **The widget carries the name; the geometry
carries the click**, the same split cables are drawn under.

## Two layout modes

A workspace is navigated as a **plane** or as a **strip**, and which one is saved with it. It
is document data rather than a preference: a workspace laid out as a strip is *laid out* as a
strip, and opens that way for whoever opens it. `LayoutMode` lives on `Workspace`, so one
canvas can be a plane while the one next to it is a strip; it is undoable by the same
snapshot as everything else and round-trips through that workspace's own file.

| | `Canvas` | `Linear` |
| --- | --- | --- |
| zoom | 0.25–3.0, about the pointer | pinned at 1.0 |
| the wheel | zooms | scrolls along the strip |
| background drag | pans freely | pans, clamped to the content |
| node y | anywhere | clamped to the viewport |
| overview | zoom out | [the minimap](#the-minimap) |

**Why a mode with no zoom exists at all.** A node looks right at one scale — the node widget
drops its icon, title, `?` and `✕` below zoom 0.4, and its controls, port labels and
readouts together below 0.5 — one threshold, `canvas::DETAIL_ZOOM`, because a row that draws
its control and not the label naming it reads as text that failed to paint rather than as a
zoom level. So zoom does
not shrink a node, it takes it apart — and a left-to-right dataflow is a one-dimensional
document that a plane presents as two. Mapping the wheel to x puts navigation on the axis the
data already runs along; what is lost is overview, and that is the minimap's job. The
alternatives that lost are in
[decisions.md](decisions.md#a-workspace-chooses-between-a-plane-and-a-strip).

**Linear constrains navigation, not authorship.** Nodes are freely draggable, after an
auto-arrange exactly as before one. The only constraint is the y clamp, which keeps a node
inside the strip's height; a node taller than the viewport is pinned to the top margin rather
than pushed off it. Making position derived data would be a cleaner model and a worse
instrument — the person who moves a node two rows up because *that* is the pair worth reading
together is doing something the ranker cannot.

**A wheel and a trackpad are different devices and are treated as such.** A mouse wheel
reports in lines on one axis, so its notches move the view *along* the strip — that is the
whole reason the mode exists. A trackpad reports in points and has two axes of its own, so it
pans in both, and the vertical goes as far as there is content to reach.

**Scrolling has momentum.** A flick keeps going and decays, rather than stopping dead with
the fingers. This reads the frame's `MouseWheel` events directly rather than
`smooth_scroll_delta`, because egui spreads one flick across many frames: taken as live
input, that would keep re-arming the fling and it would never decay. The momentum replaces
egui's smoothing here instead of compounding with it. Putting a hand on the canvas — dragging
the background — stops a glide, and a glide that reaches the end of the strip ends there
rather than pushing against the clamp and springing back when the view can move again.

Panning is clamped to the content's bounds plus a margin, so the strip has ends and a node
cannot be lost off the side of one — out to a viewport's width at the least, because a strip
shorter than the window is a strip with a gap at the end that nothing can scroll to. That is
the whole of `ui::strip_bounds`: **nothing about how long a strip is, is stored.** There is
no extent, no target, and no field in the project file; a node moved or deleted changes the
answer in the frame it happens.

**Nothing yanks the view, whatever changed the strip's length.** The pan is clamped to the
strip, so a strip that shortens in one frame takes the camera with it in one frame — and
there are many ways for it to shorten: a node deleted, an undo, a node collapsing, an
arrange, a hand dragging the outermost node inward, a strip trimming itself behind a node
that moved in. Each of those was its own jump.

So the rectangle the clamp holds the view to is not the strip: it is `CanvasState::bounds`, a
rect that **eases** toward `ui::strip_bounds` every frame, at the one `EASE_TWEEN` everything
else on the canvas eases at, and stops asking for frames once it is within `EASE_SETTLED` of
it. One mechanism rather than a case per cause. The pan clamp and the minimap both read that
one rect, so the map is always a map of the strip you can reach and the two cannot disagree.

**While a node drag is in flight the bounds grow and never shrink.** That is the one rule on
top of the ease, and the one the lag alone cannot give: the node under the hand is part of
the content, so dragging the outermost one inward would shorten the strip under the gesture —
the clamp narrows, the pan is pulled along and the thing being placed moves while it is being
placed. Dragging a node further *out* is making room to drop it into, which is the point; it
is the shrinking that yanks. Letting go needs no rule of its own: the hand opens, the growth
rule stops applying and the ease that was always running carries the strip home.

**And the strip trims itself.** Nothing asks it to and nothing can: room past the last node
that nothing is reaching for is room the ease takes back, the same way it takes back every
other change of length. A button to crop the strip would be asking for a decision the strip
has already made — which is why there is no longer one.

The eased rect is kept only in Linear, where something reads it. A plane is not clamped and
has no map under it, so the walk over every node that measures the strip is not paid for
there at all.

**Across the strip the view is pinned to the band, because a strip has no up and down.**
`clamp_to_strip` holds every node inside the viewport's own height, so there is nothing above
or below the view to reach for, and `strip_bounds` is that band whatever the content happens
to occupy. Normally the clamp's range across the strip is a single point and the pan sits on
it: dragging the background up or down moves nothing. It used to be left wherever the hand
put it, which meant a drag could push the whole strip off the top of the window with nothing
on screen to bring it back.

**The band being the viewport's and not the content's is what makes that safe.** Clamped to
the content, the pan would recentre the whole strip every time a node grew, collapsed or was
dragged — motion nobody asked for, and the reason this correction used to be skipped wherever
the content fit. A node taller than the viewport is the one thing that overflows the band, and
then the range is real and there is somewhere to scroll to.

**Dragging against an edge creeps the view.** While a node is held within
`EDGE_SCROLL_MARGIN` of the edge of the canvas, the view moves that way at up to
`EDGE_SCROLL_RATE`, falling off linearly to nothing at the inner lip of the margin, so how
fast it goes is chosen by how far in the hand pushes. **That rate is how fast carrying a
node somewhere far away goes, and nothing else is** — see the far end, below. It was three
hundred points a second and is nine hundred. The rate is a function of the pointer
and the frame and of **nothing the creep itself moves** — not the pan and not the bounds —
because a rate read from anything it changes accelerates into itself. A pointer
dragged clean out of the window creeps at the edge's own rate rather than running away with
how far outside it went, and the creep asks for the frames it needs, since a still canvas
asks for none and a hand holding a node still sends no events.

Along the strip only in Linear: a node there is clamped into the viewport's own height, so
there is nothing above or below the view to reach. It is both axes on a plane, where panning
is unclamped and the gesture is the same one.

**The far end of the strip yields as the drag pushes at it**, a little ahead of the view:
`CanvasState::bounds` grows by `EDGE_GROWTH_AHEAD` times the creep, read straight back out of
the creep vector so there is one falloff and not two and so the growth depends on the pointer
and the frame exactly as the creep does. It is written into the clamp's own rectangle rather
than into a number of its own, and that rectangle only grows while a hand is closed, so the
room is still there on the next frame. What is made and not used costs nothing, because the
ease gives it back the moment the hand opens.

**Leading is worth a little and no more.** Room arriving exactly as fast as it is used leaves
the pan against its own clamp for the whole push, which is worth avoiding. But growing the
strip faster than that changes nothing anyone can feel: the node is held under the cursor and
the cursor is at the edge, so the node travels at the rate the *view* does. Making room ahead
of a view that cannot get there sooner is a longer strip and an unchanged gesture — which is
how the first attempt at this was wrong, and what raising `EDGE_SCROLL_RATE` fixed.

The near end has no such give, because a strip's start is its first node and there is nothing behind it; what room
there is comes from the dragged node itself travelling that way, which it does, since it is
held under the cursor. The creep happens before anything is painted, so the pan every node,
cable and port is drawn at is the one the frame ends on rather than one that changed halfway
down it.

### The strip's control

silvia's `#workspace-controls`, in the bottom-right corner of a linear canvas: one button, an
icon that opens to give its name while the pointer is on it, which is silvia's
`.ws-ctrl-label` width transition. **Auto-arrange**, the Workspace menu's arrange under the
pointer. Only in Linear — a plane has no ranks to stack into columns.

**Extend and Crop were the other two and are gone.** Extend put five hundred points of room
past the far end; Crop gave every bit of it back. Both were buttons for a length the strip
now settles for itself: a drag at the far edge makes room [faster than the view
follows](#two-layout-modes), and the eased bounds take back whatever is not being reached for. A
button to make room and a button to give it back are both asking for a decision that has
already been made, and the state behind them — `extent`, `extent_target`, and
`project::View::extent` — is gone with them. **A project file saved with an extent still
opens**; the number is ignored, because the strip measures itself on the first frame.

Arranging **is** an edit, and goes to the bus as `Command::AutoArrange` like every other one.

**The button is on the canvas's own layer rather than in an `Area`.** A layer above the
canvas is not the canvas: `contains_pointer` goes false under anything on one, and the wheel
that scrolls the strip is gated on it. silvia blurs the control on click for the same
reason — the pointer being over a control does not stop it being over the workspace.

The icon is lucide's `columns-3`, drawn as its own vector geometry in the 24-unit space the
SVG is in, the way a node's `?` and `✕` are.

It is named *Auto-arrange* here and not *Arrange*, because the offer a switch to
Linear puts up answers with a button called *Arrange* and two buttons on screen under one
name is a name that names neither.

### Auto-arrange

`ui/layout.rs`, a port of silvia's `autoLayout.js`: rank each node one past its deepest
source, make each rank a column, split a column taller than the strip, and stack each column
with the leftover space spread up to a limit. Available in both modes — it only sets
positions — but the strip is what keeps a result tidy afterwards.

Two rules carry meaning rather than tidiness. **Action edges do not contribute to a rank**,
because an action cable is a CPU event and not a value; counting one would drag a sequencer
into the column after the thing it triggers. **A cycle ranks zero** rather than failing:
feedback graphs are the ordinary case here.

**The height travels inside `Command::AutoArrange`.** Redo replays the command, so an arrange
that re-read the viewport would place nodes somewhere else if the window had been resized in
between. It is one command and therefore one undo step, and it runs when asked and never
otherwise — not on add, not on delete, not on resize. An arrange fired by dragging a window
edge would be an undoable edit with no gesture behind it.

Switching to Linear on a graph laid out by hand **offers** an arrange rather than performing
one, and only when a node is displaced by more than a row's height. The arrange is its own
undo step, so it can be taken back without also undoing the switch.

### The minimap

A bar under a linear canvas showing the whole **strip** — the graph and whatever room a drag at
the edge has made past the end of it, which is the same `CanvasState::bounds` the view is
clamped to, so the map is always a map of the strip you can actually reach, and a drag that
holds the strip still holds the map's scale still with it. Push at the far edge and the map
rescales; let go and it comes back as the strip trims itself. What is on it is **the canvas
drawn at a smaller transform, and nothing else.** The same `NodeLayout` the canvas made this
frame, its body and its ports, and the same `cable::Curve`. A node is its own rectangle at its
own position with its `NodeDef::icon` in it where there is room, and a cable is the cable — so
a feedback loop bows downward on the map because it bows downward on the canvas, with nothing
written to make that happen. Anything the map drew its own way would be a second description of
the graph that can disagree with the first; the version that did is in
[decisions.md](decisions.md#a-workspace-chooses-between-a-plane-and-a-strip).

**Across the strip it frames a whole viewport, always** — not the nodes' own top and bottom.
That is `strip_bounds` again rather than anything the map does for itself, so the map and the
pan clamp cannot disagree about either axis. Framed instead to whatever part of the band
happens to be occupied, a node's place on the map means something different from one frame to
the next: one node moving down it slides every other node on a map nobody touched.

**One scale for both axes**, the smaller of the two fits, centerd in what is left. The map is
therefore the graph's actual shape rather than a squashed version of it. Fitting a whole
viewport's height as well as the strip's length is what pays for that: on a short strip the
scale is set by the height, and the map sits smaller than the bar with a real gutter at each
end. That is the right way round — the alternative fills the bar by lying about the shape of
the thing it maps — and once the strip is long enough, the length is what binds and the map
fills the bar again, so how much of it is filled still tells you how long the graph is
against the window. A node's rectangle is banded with the color of every kind of
port it produces, stacked in declaration order as the output rows themselves are — a node commonly publishes more than one, a tap
being a picture *and* the numbers measured from it, so a single tint taken from the first
output would say something untrue about a good part of the library. The stripes are the port
colors, so the map needs no palette of its own.

**Everything off the ends of the window sinks back into the ground.** The map is veiled
outside the window rectangle with the rail's own `bg_sunken` at `rail::SHADE`, so what the
canvas is showing reads at a glance rather than having to be found by its outline — the
outline and its tint stay, over the part that is not veiled.

**Click or drag anywhere on the map and the view goes there, centerd on the pointer.** Not a
relative pan: at this scale a point is a long way, and "show me that" is the only thing
anyone means by clicking a minimap. Dragging is the same gesture continued, so the window
follows the cursor. Scrolling is
view state and never reaches the command bus. Hovering a node brightens the cables that touch
it and dims the rest. Each node's rectangle is a `Response` with `widget_info` named
`rail {slug}{id}`, expanded to a hittable size when the scale makes it smaller than a pointer
can reach.

## Cost

The UI must never make the render miss a frame. Three rules keep it cheap:

- **Off-screen nodes are culled**, and a node on another workspace never reaches the cull at
  all. Port geometry is still computed for every node on this workspace, because a cable can
  run from off-screen to on-screen, but drawing and hit-testing are skipped. UI cost is
  proportional to what is *visible*, not to graph size — which is what a thirty-workspace
  project needs. **The rule has no exception**, a node in hand included: a drag is run from
  the pointer rather than from the dragged node's widget, so a node carried out over a side
  panel is culled like any other and comes back with the cursor all the same.
- **Canvas font sizes are quantized to whole pixels** by `theme::font_size`. egui caches
  glyph rasterization per size, and `base * zoom` is a new size every frame during a smooth
  zoom, so every glyph on screen was being re-laid-out and re-rasterized continuously.
  Measured: a zoom sweep cost 5x idle CPU before, and nothing after the first pass once
  sizes are bounded. The cost is that text steps rather than glides while zooming.
- **The grid's world pitch steps up as you zoom out**, so the dot count stays bounded. A
  fixed pitch produced ~25,000 circles at zoom 0.35 and grew without limit.
- **Nothing the canvas asks per port rescans the whole graph.** Port slots are laid out once
  a frame into one buffer and indexed; which inputs are fed, and which ports have a cable to
  a node that is not on this workspace, are built in one pass over the connections rather
  than by asking `sources_of` per port — and where a port is asked about alone, `Graph`
  answers from the cables it keeps indexed by node rather than from a scan. `Graph` caches its
  adjacency and memoizes ancestor sets, so `can_connect` for every candidate port during a
  cable drag is one traversal rather than one each.
- **Nothing the canvas asks about a node's kind searches for it.** A node holds its
  definition as `Node::def`, so a label, an icon, a port's declared range and the rows its
  options make are a pointer away; `nodes::find` is for a slug that arrived as a string.

`cargo run --release --example graph_bench` measures the drag path, and the cost is linear in
graph size rather than quadratic.

**The editor's painting is a GPU cost too**, and it grows with the window: every fill is a
read and a write of every pixel it covers, and the Intel iGPU shares its memory bandwidth with
everything else on the machine. Two rules keep it down without changing a pixel:

- **The window is cleared to the ground.** eframe clears it to `theme::clear_color` — the
  panels' and the canvas's own `bg_primary`, opaque — before each frame, and on a window
  cleared that way (`Ground::Cleared`, wherever eframe hands the app a GPU) no panel fills
  `panel_fill` and the canvas leaves its ground out: `theme::apply_cleared` makes the panels'
  fill transparent, and `CanvasFrame::cleared` skips the ground. A clear is nearly free where a
  fill is not, and the grounds are a screenful of fill between them. A window keeps its fill,
  since it floats over something else. egui_kittest
  clears to a color of its own, so there every ground is painted as it always was, and the
  UI snapshots are unchanged.
- **A node's shadow is hollow where its body covers it.** The body's fill is opaque, so the
  shadow's solid core under the body's interior is never seen. `node_widget::hollow`
  tessellates the shadow as egui would, keeps its fading ring, and replaces the core with the
  parts of it outside the body's interior, cut along axis-aligned lines so every pixel outside
  is covered once in the core's one color. `a_hollow_shadow_paints_what_the_whole_one_did` in
  `tests/gpu_app.rs` holds the two to the byte, painted by egui_wgpu.

What is left is mostly the pictures: a blit samples a whole half-float frame minified into a
small slot, which costs per pixel of the slot rather than per blit, and several times what a
fill of the same size does. `cargo run --release --example editor_bench -- <project> <tab>`
paints the real `App` headless at 3440x1440 and attributes the frame by leaving one kind of
shape out at a time, timed by this process's own render-engine counter rather than a
timestamp, which would count another process's GPU work in between.

**The editor repaints once a display interval, always.** Every picture on the canvas changes
on every tick, so a frame that repaints only on input would show still pictures; the frame asks
for its next repaint one interval out, and a quarter of a second out while minimized, which is
[rendering.md](rendering.md#every-window-is-a-viewer)'s rule. Where the synth ticks slower than
the display, some of those frames paint the same pictures again.

The Status box's Editor worst — the longest frame in the last two seconds, unclamped — is the
number to watch while changing any of this; [decisions.md](decisions.md#a-worst-not-a-mean-frame-time)
says why not a mean.

**And a GPU line per Output, beside the drop count:** `gpu output3: 1.8 ms (worst 2.4)`, with
a total summing the latest across Outputs, since the GPU runs them in sequence. The CPU
figures above it say nothing about GPU headroom — a graph can sit at 0.4 ms of CPU and be one
effect away from missing vsync — so this is the number the mixer, a larger resolution and
every extra Output are judged by. It is two timestamps around each Output's pass, read
back without ever waiting; [rendering.md](rendering.md#the-gpu-timer) is how. A line is drawn
only where there is a reading: an Output that is suspended or has nothing connected shows
none, and so does a build with no renderer, because a zero would read as an idle GPU rather
than as an absent measurement.

## Interaction rules

- **An illegal connection is unofferable, not rejected.** While a cable is dragged, every
  port that cannot legally receive it is dimmed, via `Graph::can_connect`. Nothing is dropped
  onto it, so there is no error state: a cable let go there lands on nothing. This is
  silvia's trick and it is worth keeping. A port that cannot take the cable but could take
  it *through a node* is neither of those two states: see
  [The conversion menu](#the-conversion-menu).
- **A dimmed port under the pointer says why.** The dim says no; a line beside the pointer,
  where a tooltip goes, says which no: *would make a loop*, *a node can't feed itself*, or
  what each side carries — *a number can't take a picture*, *a picture can't take an
  event*. It is `ConnectError` in the words a hand reads, a port named for what it carries:
  a picture, a field, a number, a color or an event (`ui::refusal_words`). The line passes the
  pointer through, so the canvas under it is still where the cable lands. A legal port needs
  no words and a convertible one has its ring.
- **A port answers the pointer in one shape.** Its square — three times the dot's radius on a
  side, the dot inside it, and short of the rows above and below — is where its widget takes
  a click or starts a drag, where it lights under the pointer, and where a cable let go lands
  on it. The canvas makes one hit test of its own a frame, ahead of the cables, and what it
  finds is read by the paint and by the input alike: the port that lights is the port the
  cable lands on. Where two squares hold the pointer the nearer port has it; a collapsed
  node's ports share one point, as do the wired outputs a tick hides, and there the first a
  cable can land on takes it.
- **A port answers only where it can be seen.** The pointer is on the canvas where it is
  inside the canvas's rect and egui's `layer_id_at` names the canvas's own layer — the gate
  `wheel` has from `contains_pointer` — so over the tab bar, a side panel, the Status box,
  Preferences, the MIDI window or a popup no port or cable lights, and a cable let go there
  lands on nothing. And a port under a body painted after its own node's is under that body:
  nodes are painted in the order they are laid out, and a cable let go on a body that covers a
  port lands on nothing, as it would anywhere else on that body.
- **Double-click a cable to delete it.** silvia deletes on a single click, and it is the one
  place worth diverging: a cable is targeted by *proximity*, cables leaving one output run
  side by side, and crossings are ordinary, so a single click near a bundle destroys a
  connection on the way to doing something else. Removal is the only gesture on the canvas
  with nothing to fall back on but undo. Hit-testing samples the curve at 24 points; a
  cubic's true closest point needs a quintic root and 24 samples are well inside click
  tolerance at any zoom a person can see. Only the **nearest** cable within the hit radius is
  hovered and deleted, and it goes as `Command::DisconnectEdge` for that one edge — testing
  each independently meant one click deleted every cable it happened to be near, each as its
  own undo step.

- **The cursor says what a press will do**, from one rule: `ui::cursor` shows an icon while a
  widget is under the pointer or held, `ui::refused` shows not-allowed over a control drawn
  disabled, and `ui::pointing` is the pointing hand for an egui checkbox, radio button or
  select, which egui leaves bare. egui's own buttons and menu entries take the hand from
  `Visuals::interact_cursor`, and its text fields set the I-beam. egui counts a widget being
  dragged as hovered wherever the pointer goes and nothing else as hovered meanwhile, so a
  drag keeps its cursor to the end and nothing it passes over takes it; the canvas's own
  gestures are set after everything on it, from its state rather than a widget's, so they
  hold while the node or port they began on is culled.

  | Over | Cursor |
  | --- | --- |
  | a button, a tab, a select, a tick, a link, a menu entry, a heading that folds, an s-number's `−` and `+`, a press button, a picture mark, the header's `⊗`, a cable, the Nodes button, a minimap, a workspace card | pointing hand |
  | an s-number's track, and the whole of a scrub wherever the pointer goes | east-west resize; none while *Lock the cursor while scrubbing* hides it |
  | a text field, an s-number being typed into | I-beam |
  | a node's header, the body around its rows, a port | open hand |
  | a node carried, the view panned, a cable in flight, a minimap or a workspace card dragged | closed hand |
  | a port a cable in flight can neither land on nor be carried to | not allowed |
  | the drawing canvas, the XY Pad's square, the viewfinder, the color picker's square | crosshair; a closed hand while the puck is held or the viewfinder is dragged |
  | the audio scope's band handle, which moves both ways | move |
  | a threshold on a meter | east-west resize |
  | the color picker's two bars | north-south resize |
  | the header's `?`, the amber `⚠`, a fault flag, a MIDI mark | help |
  | a control a cable answers for, a disabled button or entry | not allowed |
  | a size grip, a side panel's edge | the resize shape egui gives it |

## Selection

Session state, not document data: it is not in the saved file and not undoable, which is why
`ui/` writes it directly where it may not write the graph. `CanvasState::selected` is a
`BTreeSet<NodeId>`, so a command built from it has a deterministic order.

| gesture | effect |
| --- | --- |
| click a node | select only that node |
| `Shift` + click a node | add it, or take it back out if it was already in |
| `Shift` + drag the background | a rubber band; a plain drag still pans |
| drag a node in the selection | move the whole selection, each node keeping its place under the cursor |
| drag a node outside it | that node becomes the selection, then moves |
| click the background | clear, unless `Shift` is held — an additive gesture that added nothing is not a clear |
| `Ctrl`+A | every node on this workspace, and none on another |
| `Escape`, with nothing in hand | clear |
| switch tabs | clear, because a node not on the workspace on screen is never selected |

**A selection is shown by the nodes it lights, and nothing else.** No count stands on the
canvas; the selection's own right-click menu is headed with how many it holds.

**The band selects while it is dragged, not on release.** Each frame recomputes the answer
from the selection the band started with, unioned with what it now touches, so shrinking it
lets a node go again and the highlight tracks the pointer. That is also why the anchor is
kept with a copy of the base selection.

The anchor comes from `press_origin`, not `interact_pointer_pos`: a drag starts on the frame
the pointer has already passed the threshold, so the interact position is the far end of the
band by the time anyone asks, and the band would be empty. It is stored in **world** units,
so a band survives the view moving under it.

Touching, not containing, decides membership: a band has to be dragged around a node either
way, and requiring the whole of a tall Output inside it makes the gesture fussy.

**A drag holds the node where the hand closed on it.** On the first frame of a node drag the
canvas takes a **grip** per selected node — the node's position less the pointer's, in world
units, from `press_origin` for the reason the band gives — and every frame after places the
node at the pointer plus its grip. `CanvasState::drag_offsets` is that, and it is empty
exactly when no node is being dragged, which is also how the rest of the canvas knows one is.

It is an offset rather than an accumulated delta because a delta only tracks the cursor while
the view is perfectly still. The pointer's *world* position already carries whatever happened
to the view underneath it, so a node follows the cursor over a view that pans, creeps at an
edge or zooms, with nothing extra to do. And the strip's y clamp stops being destructive: the
clamp decides where a node is **placed** and never touches the grip, so a node pushed into
the floor and lifted off it again is back at the offset it was taken with rather than at one
the clamp quietly ate.

**Only the start of the gesture belongs to the node's widget.** Which node the hand closed on
is a widget event and is where the grips are taken; after that frame the whole gesture is
`drag_offsets` and the pointer, and the canvas runs it from those — once a frame, for however
many nodes are in the hand, outside the per-node loop. So a node culled off the edge of the
view or deleted under the hand cannot take the gesture with it, and the cull needs no
exemption for the thing being carried.

The gesture ends with the button rather than with the node's own response, for the same
reason: a drag must not end because the thing it is moving stopped being drawn. The offsets
are cleared on the frame the button comes up and nowhere else, which is also what marks that
frame as the release — the one the tab bar answers a drop on.

**A node let go of where you cannot see it is fetched back into view.** The cull has no
exemption for the thing in hand, so a drag can carry a node out over a side panel or clean
off the edge, and the drop can put it down with nothing on screen to show for it — the hand
comes back empty and the node is somewhere the view is not. On release, `ui::reveal_pan`
measures where the drop actually lands, from the `moves` the release itself computed rather
than from the graph, which is still a frame behind; if that is outside the canvas it sets
`CanvasState::pan_target` and the view **glides** there at the one `EASE_TWEEN`.

**The smallest move that does it, not a centring.** Following a tag centres, because there
the person asked to be taken somewhere and the node is the answer to a question. A drop is
not a question — the hand knows where it put the thing — so the view nudges the node inside
its near edge and keeps everything already on screen on screen. Inside by
`EDGE_SCROLL_MARGIN`, because that band is where a drag creeps: a node revealed into it
would sit exactly where the view runs away from it the moment it is picked up again.

Any hand on the view cancels the glide — a background drag, the wheel, a rail click, taking
hold of another node — because a glide the person is steering against is a fight neither
wins. And **a glide that cannot move is over**, exactly as a fling against the end of the
strip is: if a frame's easing survives the clamp unchanged, the target is dropped rather
than asking for frames forever against an end the pan is never going to pass.

**The border is painted last.** The row bands and the header are full-width fills over the
body, so a border drawn before them is covered on three sides and a selected node shows a
couple of slivers instead of an outline.

## Acting on a selection

Every one of these is a single command over a list of nodes rather than one command per node,
because undo is a snapshot per `apply`: a group delete has to be one step, not one per node.
`Command::MoveNodes` coalesces while the **set** is unchanged, so a group drag is one step and
grabbing something else opens the next.

**There is one menu, not a node menu and a selection menu.** Every entry is already a command
over a list, so the only difference between one node and twenty is the length of that list —
and `ui::selection_menu` is the single place it is built. Two copies of it drifted within a
day: the header's kept acting on its own node while a selection was live, which is not what
right-clicking one of several selected nodes means.

- **Right-click a header inside the selection** and the menu acts on the whole selection,
  headed by its count.
- **Right-click a header outside it** and that node becomes the selection first, the same
  rule dragging one follows: a menu acting on nodes the pointer is nowhere near is a surprise.
- **Right-click the background with several selected** and you get this same menu, for the
  whole selection: the pointer has no single node to be over then, and a right-click
  anywhere in the editor means *these*.
- **Right-click the background with one or none selected** and you get the node browser
  instead: see [Three ways to the library](#three-ways-to-the-library). One node's menu is
  on the node, and the browser carries the Paste row, so the clipboard is reachable from an
  empty canvas without taking that gesture away from the library.

The entries are Copy, Cut, Paste, Duplicate, Collapse/Expand, Reset controls, Disconnect
all, Workspaces ▸, Move to ▸ and Delete; Duplicate and Delete are keys as well, on the
selection on this workspace ([The canvas's keys](#the-canvass-keys)). An Output renders from
[its own Render section](#the-render-section-and-the-band-across-the-editor), not from this
menu. **Collapse/Expand reads as one verb
for the group**: if any node in it is still open the entry closes them all, rather than
toggling each into the opposite of what it was and scrambling them.
- **Duplicate copies the node, not its definition's defaults** — control values, options and
  collapsed state come with it, or the copy would be a new node with extra steps. A cable is
  copied when **both** ends are in the set; one arriving from outside is not, because there is
  no second source to duplicate and silently sharing the original's would make the duplicate a
  fan-out rather than a copy. The copies become the selection, since moving what was just made
  is the next thing anyone does.
- **Copy, Cut and Paste are the same three things everywhere** — see
  [The clipboard](#the-clipboard). Copy and Cut act on the selection this menu is displaying
  and are disabled without one; Paste is offered whenever the clipboard holds something and
  plants it at the click that opened the menu.
- **Reset controls is the number control's `R` for a whole node** — every control back to
  its definition's default, and the node's own ranges cleared with them, since a range is the
  node's. Options are untouched: an option changes generated code, and a reset that rebuilt a
  shader would be a different thing from putting the parameters back. Controls are uniforms,
  so this recompiles nothing.
- **Disconnect all takes every cable touching the selection**, whichever end of it is in
  there, and leaves the nodes where they are. It is structural, so the Outputs downstream of
  any of them rebuild.
- **Workspaces ▸ is a checkbox per workspace in project order**, ticked where *every* node in
  the selection is on it — so a mixed selection reads as unticked and one click puts all of
  it there. Toggling emits `ShowOn` or `HideFrom` for the whole selection. **The last
  workspace a node is on is drawn disabled**: `HideFrom` refuses to strand a node, and a box
  that is offered and then refused is worse than one that is visibly not on offer.
  **Move to ▸** is the same list, emitting `MoveTo` — show there, hide everywhere else — for
  the case where relocating was what was meant.
- **A multi-node command checks every id before it writes any of them**, so one stale node
  leaves the rest of the selection where it was rather than half-applying.

## The canvas's keys

| key | what it does |
| --- | --- |
| `Delete` or `Backspace` | delete the selection on this workspace, one undo step |
| `Ctrl`+D | duplicate it beside itself, as the menu's Duplicate does; the copies become the selection |
| `Ctrl`+A | select every node on this workspace |
| `Escape` | put back a node drag, or drop a held cable; with nothing in hand, clear the selection |
| `Home` | Frame All: glide the view to fit every node on this workspace |
| `.` | Frame Selected: glide it to fit the selection |

`Cmd` stands for `Ctrl` on a Mac, as it does for every shortcut here. Each key is a constant
in `menu::keys`, the one table of keys there is, and a row of the shortcuts window's Canvas
group; Edit ▸ Duplicate and Delete node print theirs. `F` and `H` are taken by fullscreen and the
editor, which is why the frames are Home and `.`.

**They are read after everything else on the canvas has had its turn.** `ui::show` reads
them once a pass, after every node, control, popup, the browser and the conversion menu, so a
key a control consumed never arrives. Then they are read only when nothing else holds the
keyboard (`ui::keys_free`):

- **not while a field has it, or had it when the last pass ended.** egui takes focus away on
  `Escape` before anything is drawn, so without the second half the key that closed a typed
  number would also clear the selection;
- **not while a popup, a control's popup, the browser, the conversion menu, the Nodes menu or
  a modal question is up**, each of which answers keys of its own;
- **not while a number control is under the pointer.** Its keys are read on hover rather
  than on focus ([The number control](#the-number-control)), and a `Delete` aimed at a
  parameter is not aimed at its node. `number::hovered_this_pass` is what a hovered control
  leaves for the canvas to read;
- **not with anything in hand**, and not while another of our windows has the focus.

**`Escape` puts back what is in the hand, before anything else reads it.** With a node drag
in flight, the drag's own step — one coalesced `MoveNodes` — is dropped and the graph and
edit serial it holds are put back (`App::cancel_node_drag`), exactly as `Escape` mid-scrub
drops a scrub's: a move back would leave a step that restores nothing, and the title would
read unsaved work. With a cable in flight, the cable is dropped where it is and lands on
nothing. It is read at the top of the pass, so the frame it lands on creeps and places
nothing, and egui itself abandons a widget drag on `Escape`, so the rest of the press moves
nothing either. A number control mid-scrub takes its own `Escape` first.

**A frame glides, as a drop's reveal does, and never past actual size.** Frame All fits the
nodes and the strip's margin around them; Frame Selected fits the selection with the same
margin. Both hand `CanvasState::pan_target` a whole view, pan and zoom, which eases at the one
`EASE_TWEEN`; since a world point's place on screen is affine in the two, every point travels
in a straight line to where it is going. The zoom fits both axes, held between the canvas's
least zoom and 1, since a frame is for seeing where things are rather than for reading one
node up close. A strip has one zoom and one height, so there a frame is the pan along it,
held to its ends, and where the nodes are longer than the view it shows their start. The
wheel, a drag on the background and every other hand on the view cancel the glide.

## The clipboard

**Copy puts the selection on an in-memory clipboard, Cut copies and then deletes, and Paste
plants what is on it into the workspace being looked at.** Three doors, one implementation:
`Ctrl`+C / `Ctrl`+X / `Ctrl`+V, the Edit menu, and the node context menu all end in
`App::clipboard_action`, so the three cannot drift apart the way two copies of the selection
menu once did.

**The clipboard is session state.** What is on it is a piece of graph, which nothing outside
this process could read, so it is not written to the project and does not survive the run.
Copying changes nothing in the graph, so **a copy is not a command at all** and there is
nothing about it to undo. A **cut** is that copy and a
`RemoveNodes`, which is one command and therefore one undo step. Only the **paste** is a
command.

**The payload travels inside `Command::Paste`.** It carries owned nodes and the cables
between them rather than a reference to the clipboard, because a command that read session
state at apply time would replay against whatever the clipboard held by then. A clip planted
after its originals were deleted plants exactly what was copied. Undo covers it with no
paste-specific code, as it covers everything else: the snapshot is taken before the match.

**Only cables with both ends inside the selection travel**, for the reason `Duplicate`'s do:
one arriving from outside has no second source to copy, and silently sharing the original's
would make the paste a fan-out rather than a copy. `Duplicate` *is* this code now — one
`lift` that takes nodes out of the graph and one `plant` that puts a clip back — with a
duplicate lifting and planting in one breath where a copy keeps what it lifted until a paste
asks for it.

**A pasted node belongs to the workspace it was pasted into**, not the one it was copied
from: `plant` replaces the node's whole workspace set. A duplicate names no workspace and so
keeps the original's, which is every workspace the original is shown on.

**A paste into another project brings its media.** The clipboard outlives Open and New, and
a node's file is an `assets/…` reference into the project it was copied in — which in another
project names a file that is not there, or a different file under the same name. So a copy
records the folder it was taken in on the clip itself, `Clip::from`, and travels with it inside
`Command::Paste`; a paste into another folder resolves each of the nodes' `Asset` options there
and copies the file in through `Project::import_asset`, as an import does, and the node takes
the reference that returns — `gumbasia-2.webm` where a different `gumbasia.webm` was already
here, and the asset already here where the same bytes are. The status line says what came. A
file gone from where it was is said too, and the node is left pointing at the path it was
looked for at rather than at this project's file of the same name. A paste in the project the
copy was taken in touches no file, and a painting needs nothing, since its pixels travel in
the node. Undo takes the nodes back and leaves the copied file, as an import's undo does.

**Where a paste lands depends on how it was asked for**, and the payload keeps its own shape
around that point either way — the clip's anchor is the top-left of the box its node
positions make, and every node keeps its offset from it.

- **From a context menu**, at the click that opened it, in world units. You right-clicked
  somewhere, so that somewhere is the answer.
- **From `Ctrl`+V or the Edit menu**, back where the copy was *in the window*. The copy
  records its anchor as an offset in **screen points** from the canvas origin, and the paste
  reads that offset back through whatever the view is doing now. Screen points rather than
  world units is the whole point: the clip lands in the same spot on screen however the zoom
  and the pan have moved, and pasting into another workspace puts the nodes where they
  visually were rather than at whatever coordinate that workspace happens to have there.

Pasting into the same view without moving it therefore lands the copy **exactly on top of
its original**. That is the promise of the rule rather than a miss: the copies become the
selection, so the next drag moves them off, and a nudge invented here would put the clip
somewhere nobody asked for on every other paste.

### The keys are events, not a shortcut

**`Ctrl`+C, `Ctrl`+X and `Ctrl`+V never arrive as key presses.** `egui-winit` recognizes the
clipboard chords itself and pushes `Event::Copy`, `Event::Cut` or `Event::Paste`, returning
*before* it pushes the `Event::Key` that `consume_shortcut` matches against. So these three
are taken off `InputState::events` by hand where every other shortcut is a `KeyboardShortcut`;
`keys::COPY` and its pair survive only as the text the Edit menu prints beside the entry.

This is worth knowing because the wrong version passes its test. A kittest that synthesizes
`Ctrl`+V as a key press exercises a path no window takes: the app is broken and the suite is
green. The tests here push the same events winit does.

**They are guarded where `Ctrl`+Z is not.** Undo is consumed above whatever holds the
keyboard, because `Ctrl`+Z in a text field is an undo of the graph either way. The clipboard
three are not: every `TextEdit` in the app — a hex field, a typed number, a note, a rename,
the browser's search — answers them itself and **none of them guards itself**, so consuming
them here would take the clipboard away from every field there is. `egui_wants_keyboard_input`
is the guard, and the test asserts both halves: nothing is planted while a field has focus,
*and* the text lands in the field. Only the first would also pass if the guard swallowed the
event instead of leaving it, having silently broken every field in the app.

**A copy or a cut also writes a line to the system clipboard** — `3 nodes: juliaset,
checkerboard, output`. Half mechanics, half manners. The mechanics: winit pushes
`Event::Paste` **only when the system clipboard holds text**, so a clip that lived purely in
memory would leave the next `Ctrl`+V generating no event at all and looking broken. The
manners: pressing copy and finding the system clipboard untouched is a surprise, and what was
taken is worth being able to paste into a message. The consequences are that copying nodes
replaces whatever text was on the system clipboard, and that a `Ctrl`+V on the canvas plants
the node clip whatever some other application last copied.

**The whole node is the handle, not only its header.** `node_widget::body` registers the body
rect as a `click_and_drag` ground before the header, the close button, the controls and the
ports, and returns the two as one response: egui's hit test breaks a tie by taking the widget
registered last, so anything with a gesture of its own keeps it and the ground answers only
where nothing else does — the whitespace beside a port's label, an option row's margin, an
Output's picture. Select, `Shift`-select, drag and the context menu are therefore written
once and read the same wherever on the node the pointer landed.

- **The header carries a context menu** (above), a **`?`** and a **close `✕`** on its right,
  each in its own circle — silvia's `circle-help` and `close.svg`, 20px marks inset 3px from
  the header's right edge with 1px between them (`MARK_SIZE`, `MARK_MARGIN`, `MARK_GAP` in
  `canvas.rs`), drawn always, not only on hover: the ring is part of the icon, the way it is
  in silvia's own SVGs, rather than a `bg_hover` ground that used to appear under a bare glyph
  when the pointer arrived. **Both are the icon's own vector geometry, not a font glyph in a
  ring.** `node_widget::help_mark` draws Feather's `help-circle` — a circle, the hook of the
  question mark (`M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3`, an arc into a cubic bezier, sampled
  into a 15-point polyline with a filled circle at each vertex standing in for the path's own
  round joins and caps) and its dot (`M12 17h.01`, round-capped — a stroke too short to be
  anything but its own cap). `node_widget::close_mark` draws `close.svg`: a filled ring — the
  same shape as a 2-wide circle stroke at its own radius — and a cross of two 2-wide bars with
  square ends. **The two are not the same weight**, matching silvia's own asymmetry: `?` is
  `text_muted` at rest and stays that color on hover too, since silvia's `.node-tooltip` has
  no hover style at all; `✕` is `text_primary` — already brighter — and moves to
  `theme.accent()` on hover, standing in for silvia's `--accent-light`. The drag handle stops
  short of both rather than overlapping them, so clicking the `✕` cannot also be a
  zero-distance drag and an undo step for a node that no longer exists.
- **The `?` is where `NodeDef::tooltip` reaches the canvas.** Every node in the registry has
  one written and the browser's rows were the only place it was read. Hovering the mark
  shows it as an ordinary egui tooltip, so the text is in the accessibility tree beside
  everything else. It sits `MARK_GAP` left of the `✕`, shares that mark's size, and leaves at
  the same zoom, because a mark with no title beside it names nothing. It senses drags as well
  as clicks where the `✕` senses only clicks: the body ground under the header is a drag
  handle and egui gives a drag to the topmost widget that wants one, so a mark whose whole job
  is to be pointed at has to take the drag and do nothing with it. Clicking it does nothing at
  all — its `widget_info` is a `Button` regardless, so the accessibility tree still says it can
  be pointed at. **It stays on a collapsed header**, which is the node most likely to be asked
  what it does.
- **The header itself is 26 points tall** — the taller of silvia's 20px icon padded 0.25rem
  top and bottom, and its title padded 0.5rem — with a 1px `border_subtle` rule where it meets
  the first row, standing in for the seam silvia's own header reads as against a lighter row
  fill.
- **A node whose measured taps of its own input cross 8 wears an amber `⚠`**, one square left
  of the `?`, drawn only for that node so an ordinary header keeps the `?`'s own room.
  Hovering it shows silvia's own sentence with the probe's measured count in place of its
  declared one, and it draws whether or not View ▸ Costs is on — see
  [decisions.md](decisions.md#a-header-warns-on-what-the-probe-measures-not-on-the-view).
- **A node at fault wears a flag in the same square**: a disc in the accent with a `!` cut out
  of it, and on its hover why — an Output whose shader failed, a Camera that would not open
  or has delivered nothing for ten seconds (*not responding*), an Audio In whose device or
  monitor failed, a Text that could not be drawn, a clip that could not be read: whatever the
  node's own `CpuNode::error` says, gathered every tick the canvas is drawn, and the
  renderer's error for an Output. It outranks the sampling warning, whose sentence then
  follows the reason on the hover. Named `{slug}{id} fault {why}` for a test and the
  agent-driven layer, and a row of [the problems list](#problems), whose `▸ go` comes here.
- **The body below the rows is an ordered list of regions, and each one is the node's own.**
  A node declares `NodeDef::regions` — a `nodes::Region` per band, in draw order — and
  layout reads the list through `Node::def`, a pointer rather than a search. The name is the
  registry's and the drawing is `ui/`'s: `nodes::Region` says which region it is, the heading
  it wears — built from the option that keeps its state — and the picture it shows, with no
  toolkit in it, and `widgets::def` is the one table from that name to the
  `widgets::RegionDef` that sizes and draws it. Nothing above a region knows what kind it is:
  `ui/` walks the list, hands each band to `widgets::show`, and turns what comes back into
  commands. The eighteen there are live under `src/widgets/` — `Render` for an Output's own
  render, `Preview(port)` for a source showing what it publishes, `Scope` and `Meters`,
  `Trace` with `Caption` under it, `Status`, one line of what the node's own
  `CpuNode::status` says, which is where `clockdivider` counts down to its next pass,
  `Curve`, the recording `automation` holds, `Palette` and `Steps`, the grids
  `cosinegradient` and `euclideanrhythm` are dialled in, `Grid`, the cells `stepsequencer`'s
  pattern is clicked into, `Viewfinder`, the square a
  camcorder's feedback loop is aimed in, `Ranges`, the two rows of named bounds Reframe
  Range's knobs are written from, `Buttons`, a row of buttons that write their node's own
  settings, [below](#a-nodes-own-buttons), `XyPad`, the square
  [an XY Pad's puck](#the-xy-pad) is thrown around, `Paint(port)` with `Brush` under it,
  the surface a `drawingcanvas` is painted on and the tools, size and colors it is painted
  with, and `Gear`, [a gear's picture](#the-gear-region) turning at its real rate — and a
  node has **every** region it declares rather than
  the one a ladder picked, which is why `video` keeps its meters *and* its clip.

  **A region reports its height without drawing**, which is the one function silvia's
  `customArea` does not need: `RegionDef::size` is pure and cheap, because a frame lays every
  node on the workspace out before anything is painted, and asks each region once.
  `Layouts` folds it into the body's height and stacks the bands up from the body's bottom
  edge, so a band cannot be drawn at a height the body was not laid out with. `canvas::Region` is `Declared(i)` — the i-th of the node's own list — or `Pad`, the
  six points of nothing a node with no open region has at its foot.

  **The foot of a node is one rule.** The last band either paints its own ground edge to edge
  — an Output's render, an open preview, a scope, a trace — and is handed the body's own radius
  so it rounds its two bottom corners inside the arc, or it does not, and then
  `Layouts` puts `canvas::FOOT_PAD` of plain body under it, the `RADIUS_LG` the body is
  drawn with, so no square corner crosses the curve. A closed region is a heading bar and
  nothing else, which is the second case; a node with no band at all keeps its `BODY_PAD`,
  which is the same distance. **The body's bottom corners are the slabs' own radius**, not the
  pillowy `RADIUS_LG` its top wears: `node_widget::foot_radius` is `ROW_BLOCK_RADIUS`, so the
  arc under an output slab, a heading bar or a picture is the same arc they round by, and both
  pads are that radius — a 12 point curve under a 6 point one read as two feet.
  silvia gets the same shape from `overflow: hidden` on `.node`, which egui has no equivalent
  of — a band there is clipped by the rounding rather than asked to respect it.

  **The Output's render is last in its node's order**, so it stays flush with the body's
  bottom corners whatever else the node grows, and it declares no heading at all: an Output's
  render is the node, and a triangle that closed it would turn an Output into a header with
  ports.

  **A region that can close carries its own heading** — twenty-four points
  (`canvas::HEADING_HEIGHT`) holding a bar with a disclosure triangle and a tiny label, drawn by `widgets::heading_row`.
  It is silvia's `.section-toggle` — a header with `gap: 6px` between a rotating arrow and its
  label, on a section of its own ground — in supersilvia's own tokens: the bar pulls in by
  `ROW_BLOCK_INSET` on each side and by `widgets::HEADING_PAD` above and below, so two stacked
  headings are two strips with air between them, and it is `bg_secondary`, silvia's own
  `.mixer-section` ground and a step *above* the body's `bg_sunken` rather than below it,
  brightening toward `bg_hover` under the pointer, under a dimmed `border_subtle` hairline with
  a brighter one along its bottom edge — the one-hairline version of the s-number's own bevel,
  so a heading reads as a slab sitting in the body rather than as a hole cut in it. The triangle is an equilateral one with
  rounded corners in a fixed box, `widgets::TRIANGLE` wide, `widgets::TRIANGLE_GAP` clear of
  the label; its box's left edge lines up with a port row's label inset. It **turns** rather
  than swapping — `animate_bool_with_time` over `widgets::TURN_TIME`, silvia's own 0.15 s —
  from pointing right to pointing down, and the bar, its hairline and both tints brighten under
  the pointer over `widgets::HOVER_TIME`. **No tooltip**: a triangle beside a label needs no
  words, and the one it had opened over the heading below it. `Heading` names the option the
  state lives in and the default a node nobody has asked opens with, so hiding needs no
  mechanism beyond the fold: a closed region is its heading and nothing more. The affordance
  is a triangle rather than a tick in a row elsewhere on the node because **a closed region
  still says it is there** — which is what every properties pane does, and what a tick cannot
  do. The storage is unchanged: an option in `Node::options`, `Presentation`, undoable, and
  read off the `Node` by layout. `scope`, `preview`, `named` and `trace` are the four.

  **A region says whether it claims the pointer.** The node's hit rect is not one size.
  `claims_pointer: false` — most of the regions there are — lets a press through to the
  body under it, so a hand carries the node by its trace or its render exactly as by the
  whitespace beside a port; `claims_pointer: true`, which the camcorder's viewfinder,
  Reframe Range's named ranges, a row of buttons, the XY Pad's square and a Drawing Canvas's
  paint surface are, owns
  every press inside the rect and never starts a node drag — a drag there aims the loop or
  presses the button, and the node stays where it was put. It is the region's own declaration and `canvas` has no
  rule about it: the body's ground is registered before the regions and egui hands a drag to
  whatever asked last, so a claiming region takes it and `ground` sees nothing. The same
  ordering is why the scope's band handles work inside a region that claims nothing.

  **The widest thing in the body sets the width.** `NodeDef::width` is the rows' answer and
  each region declares the narrowest body it can be drawn in; `canvas::node_width` is the
  larger. An Output is 240 because `picture::RENDER` says so, an audio node 300 because the
  scope does. A region asks whether it is open or closed, so folding a scope away does not
  reflow the node under the hand that closed it. See
  [decisions.md](decisions.md#a-region-declares-its-own-heading-its-own-width-and-its-own-hit-rect).

  **And a hand may size a text box.** A node holding a text box of more than one line — the
  note and the Text node — wears a **grip** in the bottom-right corner of the box: three
  hairlines across the corner, in the accent under the pointer, the mark a browser draws on a
  resizable textarea. Dragging it writes `Command::SetNodeSize`: document data like a
  position, so it is undoable, the whole drag coalesces into one step, and it rides in the
  workspace file. **The box's height on both**, as `Node::dragged_height`; **the body's width
  too on the note**, the one kind with `NodeDef::resizable`, as `Node::dragged_width`.
  `canvas::node_width` is the *larger* of `canvas::natural_width` — the rows' and the regions'
  answer above — and what was dragged, clamped below `canvas::MAX_NODE_WIDTH`, and
  `canvas::value_height` holds a dragged height between `MIN_TEXT_HEIGHT` and
  `MAX_TEXT_HEIGHT`, so a size from a file, from undo or from a grip can never put a row
  outside the body holding it.

  **A text box is the height it was given, never the height of its text.** Its definition's
  lines until a hand drags it, and what is typed past the bottom scrolls inside it. A box that
  grew with its text grew differently at every zoom — the text wraps at a whole-pixel font size
  while the box scales smoothly — so zooming made the node jump a line at a time. A height that
  is the document's own does not move.

- **Collapsed, a node is its header and nothing else.** `canvas::rows` yields nothing, so the
  body and the port bands follow from that one early return. The height has exactly one term
  that is not a row — the region bands, of which a collapsed node has none either — and it is
  the reason collapsing is two early returns rather than the one the row list promises.

  So an Output's render takes its height from the region list and has no band when collapsed.
  Reading `THUMB_HEIGHT` directly is what made a collapsed Output paint its frame 135 points
  up the canvas over whatever was there.

  **A trace is a region too**, on `adsr`, `oscillator` and `animation`: a fixed 48 pt band,
  flush to the body's bottom corners as an Output's render and the audio scope already are,
  holding a line of the node's own shape rather than a row of numbers — `widgets::trace`. Its
  samples are `CpuNode::trace`'s own ring, what the node published over the last few seconds,
  each dated by the one clock and drawn where its time puts it rather than one step per
  sample, gathered by `App` the way an audio scope is; a node that has not ticked yet draws
  an empty band. It is read-only, so it claims no pointer, and it sits under a **Trace**
  heading, open on a new node, since the shape is the first thing a person reads there and the
  most skippable once they know it; `automation`'s recorded curve is the same kind of band and
  wears the same heading, on the same `trace` option (`nodes::SHOW_TRACE`). It asks for
  `SCOPE_NODE_WIDTH` — the same 300 the audio scope uses — rather than a width of its own. See
  [decisions.md](decisions.md#a-trace-is-a-picture-of-the-shape-a-node-is-editing).

  **A region may hold the node's own numbers**, which is the second thing a region draws
  beside a picture: `widgets::palette` lays `cosinegradient`'s twelve coefficients out three
  across under R, G and B over a strip of the palette and a plot of its three channel curves,
  and `widgets::steps` draws `euclideanrhythm`'s four lanes of cells with a slab per lane
  under them carrying that lane's Steps, Pulses and Rotate. Both are silvia's own custom
  areas, geometry for geometry. The cell is not a new control: `RegionUi::number` draws the
  same inset s-number a port row carries, under the same accessible name `{slug}{id}.{key}`,
  and what comes back is `RegionEvent::Number` — the port row's own `NumberAction`, turned
  into edits by the one `node_widget::number_action` the row goes through. So a coefficient
  resets, opens its range editor, learns a MIDI binding — breathing while it waits and wearing
  [the bound dot](#alt--click-to-bind) on its corner once bound — and collapses an escaped
  drag exactly as a knob on a row does, and every edit is one step back.

  What those numbers are is [a hidden control](nodes.md#a-nodes-own-values): a value with no
  port. Neither region claims the pointer — the cells register their own interacts and so win
  the presses that land on them, which is the same ordering the scope's band handles rely on —
  so a drag on the strip, on the plot or on the gaps between cells still carries the node.

  **A region may edit one of the node's own values**, and the grid is the one that does:
  `widgets::steps::GRID` draws `stepsequencer`'s pattern, a [`Cells`
  value](nodes.md#a-nodes-own-values), as four lanes of sixteen cells a hand clicks on and
  off, with silvia's Clear under them. **It is Euclidean Rhythm's grid**, one grid for both
  nodes — `Steps` reads it and `Grid` edits it: the same cell painted by the same function,
  16 points square with a 4 point gap, the brighter downbeat on every fourth, a lit cell in the
  action port's own hue since it is the gate that lane will fire, and the playhead ringed with
  silvia's glow on the column the sequencer is on, the absolute step modulo the lane. silvia's
  two nodes space their lanes differently, `.euc-lane` with a 4 px margin and `.seq-lane` with
  none; the grid takes the Euclidean spacing for both, so a cell is its own target rather than
  a band touching its neighbours. A cell is a checkbox in the accessibility tree, named
  `{slug}{id}.lane{L}.step{S}` from one, and senses a click and nothing more: a click sends
  `RegionEvent::Value` with that one cell turned over, which is one `SetValue` and one step
  back, and Clear sends the whole grid dark in one more. So the region claims no pointer — a
  drag that starts on a cell or between two carries the node. Its height is the lanes the
  value declares and never what is lit, so lighting a cell moves nothing.

  **A region may be painted on.** `widgets::paint` is `drawingcanvas`'s custom area, silvia's
  geometry for geometry: `Paint(port)`, a 300 point canvas in a 1 point `border_normal` frame
  rounded `RADIUS_SM`, six points in from the body, as tall as Canvas Size's aspect makes it;
  and `Brush` under it, a centered row of six 28 point tool buttons four apart — Lucide's
  pencil, eraser, a line, square, circle and paint bucket, drawn as their own geometry, the one
  in the hand on `bg_active` — then Size, Color and Background, each a label at the left and
  its control at the right, and silvia's keys in two dim lines. **The surface claims the
  pointer**: a press on the picture begins a stroke, and the node stays where it is. It is the
  node's own published texture, blitted into the slot a preview's picture takes and wearing the
  same pop-out and fullscreen marks, so what the surface shows is what the patch gets, a tick
  after the edit; the crosshair is the cursor over it. A pen or an eraser writes the whole
  picture back as a `RegionEvent::Value` — a `SetValue` — on the press and on every frame it
  moves, all into the step the press opened, so a stroke is one undo step; a line, a rectangle
  or a circle is drawn by the region at 0.7 of the brush's alpha while it is dragged and written
  once when the pointer lets go; a fill is written where it lands. **A press gives the surface
  the keys**, and the ring around its frame, silvia's `0 0 0 2px` in `primary` at half
  strength, says so: B E L R C F pick a tool, `[` and `]` move the size a pixel — five with
  Shift — and so does the wheel over it. They are consumed, so `F` there is the fill and not
  fullscreen. **The brush is the node's own**: Size is `RegionUi::number`'s s-number and the
  two colors are `RegionUi::swatch`, the row's swatch under the name `{slug}{id}.{key}`,
  whose click comes back as `RegionEvent::Color` and opens the row's picker; the tool buttons
  write the `tool` option, which `OptionDef::in_region` keeps off the rows. The brush claims
  nothing, so the air between its buttons carries the node. See
  [decisions.md](decisions.md#the-paint-surface-claims-the-pointer-and-the-brush-is-the-nodes-own).

  **The ports stay.** Nothing about the graph changes: the cables are still connected and the
  shader is untouched, so a collapsed node keeps a slot per port, gathered on its header's
  left and right edges. A cable whose endpoint has no slot is dropped by the canvas silently
  and only on screen, so this is not decoration — without it, collapsing a wired node makes
  its cables disappear. `tests/ui.rs` asserts the cable survives.

  **An output row a tick hides gathers the same way, once it carries a cable.** Uniforms and
  Events off take their output rows, and a wired port with no row is exactly a collapsed
  node's port: `Layouts::push` hangs every port that has no row, collapsed or hidden and
  wired, off the header's middle, on the body's edge, in one loop. So a cable from a hidden
  uniform number leaves the header's right edge, and the point there lights, takes a cable
  let go on it and clears on a right-click as a collapsed header's does. A hidden output with
  nothing on it has no slot, so a header whose hidden rows carry nothing wears no dot — the
  tick hides the port as well as its row — while a collapsed node keeps a slot for every port.

  `collapsed` is document data, like a position: it rides in the saved file and through undo,
  and it never recompiles.
- **Clicking a connected input port clears everything feeding it**, as `Command::Disconnect`.
  A single click, because the port is an exact target rather than a nearby one. It is also a
  different intention from cutting a wire: an action input takes many sources, and "unplug
  this port" is not "cut this one".
- **Right-clicking a port, input or output, clears every connection on it**, as
  `Command::DisconnectPort` — one command, so however many edges it carried is one undo step.
  It is where silvia disconnects and supersilvia previously had no substitute: `Disconnect` above only
  ever reaches a port from its input side, and a cable leaving an output had nothing but
  `Disconnect all` on the whole node. A right-click on a bare port is refused by the command
  bus rather than answered by the canvas. Hovering a port paints every cable touching it at
  the same emphasis a hovered cable gets, so the gesture is shown before it is made; only the
  *nearest* cable to the pointer is still what a double-click on the cable itself deletes, so
  a port lighting up several wires at once does not make all of them one double-click from
  gone. Ports gathered on one point of a header share a square, and egui hands the click to
  whichever registered last, so there the right-click clears the first gathered port that
  carries a cable, and another right-click the next. See
  [decisions.md](decisions.md#a-port-right-click-clears-its-cables-not-ctrl-hover).
- **Releasing a dragged cable lands on the port under the pointer** — by the port's own
  square, the one it lights in — and of the ports there, the nearest it can legally land on,
  not the first in slot order: ports gathered on a collapsed header share a square, and two
  nodes laid over each other overlap theirs. Only where no legal port holds the pointer is a
  convertible one offered the [conversion menu](#the-conversion-menu).
- **A control whose input is connected is drawn disabled.** The connection overrides it, so
  an editable-looking control that silently does nothing would be a lie.
- **A drag is one undo step.** Node drags and control scrubs emit one command per frame;
  `App::apply` coalesces consecutive commands touching the same node or the same control.
- **Popups are drawn after every node.** A popup opened while its own node is being drawn is
  painted over by every node drawn after it. `ui::show` records which control is open and
  renders it at the end of the pass, so paint order is deterministic rather than a question
  about egui layer ordering.
- **A popup does not dismiss on the click that opened it.** The click-away check sees that
  same click; dismissal only applies to a popup that was already up before this frame.
- **Every popup is `ui::popup`**: a foreground `Area` that is not movable, a
  `Frame::popup` on `bg_secondary`, and one click-away rule — a click that lands outside the
  area's `contains_pointer`. Not `hovered`, which a click on a field or a row inside the popup
  takes from it; the range editor closed on every such click while it read `hovered`. A
  movable `Area` senses a drag rather than a click, so a click on its padding falls through
  to the node underneath.
- **A popup's anchor is stored in world coordinates**, and converted to screen when it is
  drawn. A screen point captured on the click it opened with would leave the picker behind as
  soon as the canvas panned or zoomed under it. There is a third `OpenControl` variant,
  `File`, which is not a popup at all: it is consumed at that same point in the pass and
  becomes a `file_requests` entry, leaving nothing open.
- **Zoom is applied at the end of the frame**, after every control has had its chance at the
  wheel. `src/ui/number.rs` takes the scroll with `input_mut` and clears it, so a number
  control under the pointer consumes the notch. Read at the top of the frame, one notch over a
  control both scrubbed the parameter and zoomed the canvas out from under it.
- **The canvas takes the wheel only where the pointer is on the canvas itself**, so a surface
  that takes the wheel takes all of it. The gate is one expression, the canvas background
  response's `contains_pointer`, and every reading of the wheel is behind it: the zoom in
  `Canvas`, the strip and its fling in `Linear`, and the `MouseWheel` events the momentum
  reads. That is egui's own hit test — a widget on a higher layer covering the point drops the
  canvas out of it, and the response's rect is the canvas area rather than the window — so the
  Nodes menu, the browser, a control popup, a modal and the preview panel each keep the whole
  notch. **At the end of a list included**, which is what a `ScrollArea` alone does not give:
  egui's consumes the delta only while it can move, so a list scrolled to its end leaves a full
  notch behind, and it never touches the `MouseWheel` events at all. Wheel input arriving this
  frame is what the gate is on: a glide already in flight was launched on the canvas and goes
  on gliding wherever the pointer wanders. Clearing the delta per surface, as the number
  control does, is not this rule
  ([decisions.md](decisions.md#the-canvas-asks-what-is-under-the-pointer-rather-than-each-surface-clearing-the-wheel)).

## Cables

Bezier for data, straight and dashed for action, as in the design system. A base width of
4 px for data and 2 px for action, scaled by zoom; emphasized — 1.5 times the base and
brightened, so 6 px for a data cable and 3 px for an action cable at zoom 1 — while it is the
cable nearest the pointer, which is also the click-to-delete affordance, **or** while the
pointer is on either of its own ports, which is not: a port can carry several cables at once,
and lighting all of them up is what tells a hand which wires a right-click there is about to
clear, before the gesture is made. Only the nearest-cable case gates the double-click
deletion. Drawn in a lighter, less saturated version of the port color so a dense graph
reads as cables over nodes rather than a second set of dots.

**And the ports at both ends light with it — from either end of the gesture.**

Put the pointer on a port and every cable it carries brightens, along with the port at the far
end of each. Put it on a *cable* and the ports at both of its ends brighten. The two halves
answer the same question from whichever end the hand happens to be on, and that is how you
trace where something goes without dragging it.

silvia does the port half through `glowOnHover`, with an actual glow. The glow is dropped: a
blur in egui is several more strokes per cable per frame, and the UI must never make the
render miss one. The color lightens instead, which is what hover already does everywhere else.

Both halves are the preference `port_hover_highlight`, on by default. The cable nearest the
pointer brightens regardless — that one is the click-to-delete affordance, not a decoration.

**Every cable can have a color of its own instead.** `phi_cables`, off by default, walks the
hue circle by the golden angle — `GOLDEN_ANGLE`, 137.508°, a turn divided by phi — handing the
next step to each cable the first time it is seen. Saturation cycles every step and lightness
every third, so two cables that land near each other still differ in more than hue; ours sit
lighter than silvia's 30/40/50%, because a cable here is drawn over a dark canvas rather than
a light editor.

**The cable being dragged already wears the color it will keep.** The step is reserved in
`CanvasState::drag_hue` when the drag arms, not picked when it lands — silvia's `CursorWire`
takes a color and hands that same one to the `Connection` it becomes, and a wire that chose a
fresh color on the frame it connected would have one frame where the color said nothing. A
drag let go over nothing spends its step anyway and the walk moves on; a drag released on a
convertible port makes *two* cables through the bridge, neither of which is the one the step
was reserved for, so both take their own.

The step is kept in `CanvasState::cable_hues` for the life of the session, so a cable holds
its color and deleting one does not recolor the rest. It is not in the document — a display
preference has no business there — and it is not derived from a cable's position in the graph,
which would reshuffle every color each time a cable went.

**A connected port is then outlined in its cable's color**, a ring just outside the dot. The
dot goes on saying what the port *is*; the ring says what it is wired to. The ring follows the
port's shape — round, square for an action, diamond for a uniform — because a round ring
around a square would undo half of why the square is a square. An output with several cables
wears the first in graph order, so the outline on a fanned-out port does not change for
reasons that have nothing to do with it.

**A backward cable sags, if you want it to.** `cable_droop` drops both control points of a
forward cable by silvia's own sag — fifteen points plus a seventh of the span, stopping at
eighty. It is on by default, as in silvia. A backward cable ignores it, and so does an action
cable; see [the Preferences window](#the-preferences-window) for why.

**A backward cable routes around, not through.** A feedback loop runs right to left, and a
forward cable's control points pull *inward* — so the curve cut back through exactly the space
the two node bodies occupy, and cables are drawn before nodes, so it vanished behind them.
Such a cable now reaches further sideways and bows downward instead.

The bow is sized from **the bottom edge of the nodes at both ends**, not from the horizontal
distance. That distinction is the whole fix: sizing it from the gap between the ports looks
right on small nodes and dives straight into an Output, which is tall because it carries its
own render. `ui::show` measures both bodies and hands the clearance to `cable::bezier_points`;
click-to-delete samples the same curve, so hit-testing follows the loop.

Downward always, rather than picking a side per cable: one predictable shape is easier to
follow across a dense graph than a field of individually-optimal ones.

**A cable let go over empty canvas asks what it lands on.** Not on a port and not on a
node's body, it opens [the node browser](#three-ways-to-the-library) at the pointer, holding
only the kinds that can take the cable, with the cable still drawn to where it was let go.
The kind chosen lands wired, as one `Command::AddConnected` and so one undo step: out of an
output, the cable lands on the new node's first input that takes it and the node stands to
the right of the pointer; dragged back out of an input, it leaves the new node's first output
that feeds it and the node stands to the left. The time rows are asked last, since a number
let go in the open means a parameter more often than a clock. `Escape` or a click away
dismisses the browser and the cable with it, and the browser carries no Paste row here, a
paste being no answer to where a cable goes. Where no kind can take the cable, nothing opens.

The list is the graph's own answer (`nodes::attach::takers`): a fresh node of every kind this
machine offers is put on a copy of the graph and asked `Graph::can_connect`, so a dual node's
effective types, a pin and an action against data are judged as the cable itself would be.
The copy shares every node it does not write, so asking costs a node per kind.

**A node carried across a cable goes into it.** While one node is in the hand, a cable within
twelve points of the pointer that the node could carry is lit in the selection's color, a
step wider; let go there, and the cable is replaced by two through the node,
`from` into its input and its output into `to`, as one `Command::Splice` — the shape of
`Command::Bridge` for a node that is already on the canvas. Twelve rather than a click's six
because the node's own body is over the cable at that moment. The move that put it there is
its own step before the splice, so one undo takes the node back out of the cable and leaves
it where it was dropped.

**Only one node, and one with no cables of its own**, is offered a cable
(`nodes::attach::splice_ports`): a selection has no one input and one output to put into a
cable, and a node already patched would be rewired by a drop meant only to move it. Which of
its ports carry the cable is asked of a copy of the graph with the old cable out and both new
ones in, inputs and outputs in declaration order with the time rows last; the answer is
kept for the node and the cable under the hand and forgotten when the hand opens, since a
drag writes positions and nothing else.

## The conversion menu

silvia's `js/typeConversions.js`, whole: a cable dragged over a port it cannot land on but a
node could carry it to, a menu on release of the few castings that make sense for the pair,
and the one chosen inserted between the two and wired both sides.

**A port has three states while a cable is in flight**, not two:

| state | drawn | what a release does |
| --- | --- | --- |
| legal | as it always is | the cable lands |
| **convertible** | a ring of dashes outside the dot, in the port's own hue | the conversion menu opens at the port |
| illegal | dimmed | nothing |

The ring is twelve short segments on one radius — egui has no dashed circle and a dozen
two-point dashes read as dotted at every zoom a port is legible at — drawn only on the frames
a drag is in flight, so it costs nothing the rest of the time. A convertible port's hover text
and its accessibility name both gain `(convertible)`, which is how a test and an agent see the
state a picture shows.

**A convertible port is one that `nodes::can_bridge` accepts**, and that is one question with
three clauses: the graph refuses the cable **only** on type — an action, a cycle, a
self-connection or a demotion is a no and stays a no — the table holds a casting for the pair,
and a node between the two would close no loop.

**The rows are a curated table**, `nodes::bridge`, and the curation is the feature. Each row
is a **casting** — a way of reading this value as that type, named for what it does rather
than for the node that does it, with silvia's own label and icon: *Luminosity*, *Red Channel*,
*Grayscale*. Five pairs, and the twelve rows of the first two are silvia's, row for row:

| released on | rows |
| --- | --- |
| a color onto a **field** | *Luminosity*, *Lightness*, *Hue*, *Saturation*, and the four channels through a `channelsplitter` |
| a field or a uniform number onto a **color** | *Grayscale*, *Red Only*, *Green Only*, *Blue Only*, all of them an `rgba` |
| a color onto a **uniform number** | *Average*, *Brightest*, *Darkest* — the `tap`, reading the frame |
| a field onto a **uniform number** | *Average*, *Highest*, *Lowest* — the same `tap`, through its `number` sidechain |
| a picture onto a **uniform color** | *Mean Color* — the same `tap`, and only the one row |

*Mean Color* is one row where the others are three because the extremes are extremes of a
*number*: there is no pixel a tap keeps to hand back as the darkest color, and a color
assembled from three separate channel minima is one that was never in the picture.

**A uniform color reads the color tables**, all of them: the promotion into a varying color
input is free, so every casting that takes a picture takes one of these too, and the rows do
not change. A uniform number onto a field, and a uniform color onto a color, are not
conversions at all — they promote with nothing in between.

**A row may fan out.** `Bridge::inputs` is a list, so *Grayscale* lands one cable on an
`rgba`'s red, green and blue at once — silvia's `float-to-color` closure, and the reason
`Command::Bridge` carries a list of keys rather than one.

What the table is *not* is a registry query. It was one for a day: every node with an input
the source feeds and an output of the target's own type, one row per pairing. That menu is an
inline patchbay — fifteen rows for a color onto a circle, every measurement there is for a
color onto a diamond, and a new row the day any node lands — and it answers a different
question than the one a hand asks with a dropped cable. A node that is not in the table is
still a node: it is added from the Nodes menu and wired by hand, which is what *Value*,
*Chroma*, `hsla`, `sample` and `autoexposure` want anyway, each having something to set. A
hand-written table can name a port that is not there, so a test resolves every row against the
registry: the node exists, the source feeds each input, and the output is the target's own
type.

**The menu is the node browser's shape** — an `Area` of icon-and-label rows with the arrows
and `Enter` on them, `Escape` or a click elsewhere to dismiss — without the search field,
since the list is already the answer to a question. It hangs off the port's own world position,
so a pan or a zoom under it does not detach it from what it is about, and the first frame it
appears is a sizing pass like any other popup's.

**Choosing a row is one `Command::Bridge`**, so the node and its cables are one undo step:
a hand that changes its mind presses undo once. The node lands at the midpoint of the two
nodes' own positions, which is silvia's, and `App::apply` takes it back out again if either
cable is refused — which the menu's own rule makes unreachable, and the bus does not rely on
that.

**One thing differs from silvia.** It has no `UniformNumber`, so every conversion it offers is
free; here a conversion at the **diamond** boundary costs a readback and a frame, which is
more than a luminosity node and is why the third state exists at all — a port whose conversion
is expensive should not light up exactly like one the cable simply fits. That is also what the
two extra pairs in the table are: a `tap`, which is the one node whose job is reading a
picture back to the CPU, and three summaries of what it measures.

## The number control

`s-number` is 84x20: a decrement stepper, a draggable value with a value-proportional fill
behind it, and an increment stepper. Dragging is the primary gesture — it is a slider you can
also type into, rather than one or the other — with `Shift` for a tenth of a step and `Ctrl`
for ten. A control whose input is connected is drawn disabled and shows the **arriving**
value — the number the producer published this frame, fill included — rather than the one it
stores: a connection overrides the control, so the stored number is not what the node sees.
Its number is drawn at a readout's weight, and the fill and the steppers are what go quiet.

**The chrome matches silvia's.** The two caps are rounded on their outer corner only — square
where each meets the trough — so they read as one pill-shaped control rather than two buttons
floating beside a bar. The fill is square-cornered on every side and confined to the trough
between the caps, exactly as silvia's `.s-number-slider` sits inside a wrapper inset by the
caps' own width; a fill rounded to match the control's outer corners read as a second, smaller
pill floating inside the first one. The border is a two-tone bevel — darker along the top and
left, brighter along the bottom and right — standing in for silvia's `2px inset border-normal`,
which a browser renders as a sunken edge; the dark side is a darkened `border_normal`, the
bright side `text_muted` (brighter than any border token on the ladder, which is why it is
the one that reads at 2x), both existing tokens rather than new color. `number::bevel_border`
draws it, and the range editor's own fields — [below](#a-range-belongs-to-the-instance) —
share the same call, so a value reads as the same recessed thing wherever it is edited. See
[decisions.md](decisions.md#the-s-numbers-chrome-matches-silvias-in-supersilvias-own-color-tokens).

| gesture | what it does |
| --- | --- |
| drag the value | scrub. `Shift` fine, `Ctrl` coarse; log controls move by ratio |
| drag the track | jump to that position in the range, and scrub on from there |
| wheel while hovered | one step, with the same multipliers |
| `↑` / `↓` while hovered | one step, with the same multipliers |
| `−` / `+` | one step |
| `Ctrl` + `−` / `+` | the *step* itself, a tenth or ten times |
| click the value | type an exact one. The text is selected, so typing replaces it; `Enter` or clicking away commits, `Escape` cancels, and anything unparseable leaves the value alone |
| `↑` / `↓` while typing | still one step, with the same multipliers — not a cursor move |
| `Escape` mid-drag | the value the drag began at |
| `[` / `]` | to this control's own ends |
| `D` | back to the definition's default |
| `R` | back to the default **and** the definition's range |
| right-click | the range editor |

Keys are read while the pointer is **over** the control, not while it is focused: the canvas
has no focus to give, because a node is not a widget tree. They are consumed rather than read,
so a key that moved a control does not also reach the canvas, and while a control is under
the pointer [the canvas's own keys](#the-canvass-keys) wait, `Delete` and `Ctrl`+D included.

**`Alt`+click learns a MIDI binding**, on an s-number and on a press button alike — silvia's
gesture, and never also a scrub, a typed entry or a press. See
[Alt + click to bind](#alt--click-to-bind).

**The text is the value and the rest of the bar is the track.** The number's own box, padded
four points either side so a one-digit value is still something to grab, is where a scrub
starts; anywhere else on the bar is a position in the range, and a press there puts the value
where the fill's edge would be. The split is read from where the press landed, not from where
the pointer stood when egui decided the gesture was a drag, so a scrub that began on the
number stays a scrub once it leaves it. It is a *drag's* split only: clicking anywhere but a
stepper still opens the typed entry, because there is nowhere else to put it.

**Typing keeps the control's shape.** Clicking the value does not swap the whole control for a
bare text box: the caps and the trough are the same `chrome` drawing call the idle control
uses, and only the value — in the trough between the caps — becomes an editable field. `↑` and
`↓` step the value while the field has focus, exactly as they do while merely hovered, rather
than moving a cursor there is nowhere to move it to — a single line has no second line to move
between. `←` and `→` are untouched and still move the cursor through the digits.

**`lock_cursor_while_scrubbing`, a preference, locks and hides the pointer for the length of a
drag** — *Lock the cursor while scrubbing* under Preferences ▸ Editing, off by default. On `drag_started` it sends
`ViewportCommand::CursorGrab(CursorGrab::Locked)` and `CursorVisible(false)`, and the drag's
cursor is `CursorIcon::None`, since egui-winit shows a hidden pointer again whenever the icon
changes; on `drag_stopped` or an `Escape` cancel it sends them back to `None` and `true`. Locked, the pointer's *absolute*
position stops changing — the OS pins it at the point the grab engaged — so `Response::drag_delta`
would read zero for the rest of the gesture; the control reads `ui.ctx().input(|i|
i.pointer.motion())` instead while the preference is on, which is the raw relative motion
`DeviceEvent::MouseMotion` reports and eframe forwards as `Event::MouseMoved` regardless of
grab state. That is what makes the drag keep moving past a screen edge a hand physically ran
out of: pick the mouse up and set it down again, and the scrub carries on from the raw motion,
never from where the (invisible) pointer nominally is. See
[decisions.md](decisions.md#the-lock-cursor-while-scrubbing-preference-reads-raw-motion-not-position).

**`Escape` mid-drag drops the undo step rather than adding one.** A scrub coalesces into a
single step, so setting the value back would leave a step in the ring that restores nothing
and an edit serial the title reads as unsaved work. `App::cancel_control_drag` pops the step,
restores its graph and restores its serial — undoing the drag without offering it as a redo.
That step may cover an earlier scrub of the same control, since a gesture ends when another
command interrupts it rather than when the pointer lets go, so the value the drag actually
began at is re-applied afterwards as an edit of its own.

**`Ctrl` on a stepper changes the quantum, not the multiplier.** A coarser step and a coarser
step *size* are the same wish, so they live on the same button. It is a `SetRange`, so it is
document data: undoable in one, in the file, and shown by the range editor. The step stays
between what the three-place readout can show — 0.001, or the definition's own step where
that is finer — and the width of the range, which is where a control stops having more than
one position. **The editor's step field is bounded by that same pair**, where its min and max
fields are bounded by the definition's ends: a quantum of 0.1 on a control running 1 to 64 is
an ordinary thing to want, and the ends' bounds would round it away the moment the editor
opened on it.

**The drag anchor resets when `Shift` changes.** A scrub carries the sub-step remainder across
frames, or a fine drag — a tenth of a step per four pixels — would round away to nothing. That
remainder is dropped when `Shift` is taken or released mid-drag, because otherwise it is spent
at ten times its size the moment the modifier comes off, and the value jumps exactly when the
point of fine mode was that it should not.

### A range belongs to the instance

`min`, `max` and `step` are properties of a *node*, not of a node kind. A gain that only ever
wants 0.9 to 1.1 is unscrubbable across the range its definition declares, and a fader or a
lane mapped onto a control needs to know where that control's ends are *here*. `Node::ranges`
holds the ones that differ — absent for almost every control in almost every graph — and
`nodes::control_range` is the one place the question is answered, so the widget, the command
bus and the file loader cannot disagree.

It is document data, like a position: `Command::SetRange` and `Command::ClearRange` go through
the bus, ride in the file, and are undoable, and neither recompiles, because a range moves a
uniform's ends and the uniform is already a uniform. Narrowing a range pulls the stored value
in with it — a control cannot sit outside the track it is drawn on — and a control whose range
is not its definition's wears an accent border, because nothing else on the node would show it.

**A range goes where it is put.** The definition's ends are printed in the popup's header as
advice and bound nothing: a control whose ends could only ever narrow cannot be pushed, and
that is half of what an editor for the ends is for. A non-finite end is still refused. That,
and why right-click opens the editor where silvia uses `Ctrl` + hover, are in
[decisions.md](decisions.md#a-controls-range-belongs-to-the-node-not-to-the-node-kind). The
editor is drawn in `ui::popup`, as the color picker and the select are, so it lands above
every node however late that node was painted and a click inside it keeps it open.

**The editor is a two-column table**, a row per number — Min, Step, Max, Value, and Unit where
the control has one — an editable field in the left column and that row's default in the
right, dimmer; clicking a default copies it into the field beside it, which is the reset for
that one row. **Min and Max are bounded by nothing**; only the value is held inside the ends,
and the step inside what a step can be. Each field is a plain number field, typed rather than
dragged, committed on Enter or on a click anywhere else, in the control's own `bevel_border`,
so the popup reads as the same recessed instrument the control itself is.
The header names the control and states the definition's own declared ends, so
the answer to "how far can this go" is the first thing read rather than something fished out
of a field. A `Reset` button under a rule puts every field back to its default in one gesture.
**It is also where MIDI is bound.** Under its own rule: **Bind MIDI…** on an unbound control,
and on a bound one the trigger that drives it beside an **Unbind** — and no rule at all on a
control MIDI does not bind. `Alt` + click is the
quick gesture and this is the findable one — a modifier nobody has been told about is not a
feature. See [the MIDI window](#the-midi-window).

The control itself answers the range question without opening the popup at all: hovering it
adds the range to the tooltip — `frequency 8/⬓ (0.1-64, step 0.1)` — which is what silvia's
`Ctrl` + hover actually bought. See
[decisions.md](decisions.md#the-s-numbers-range-editor-is-a-two-column-table-not-a-list-of-dragvalues).

### The value on the row

**A port row paints `InputDef::label` or `OutputDef::label`**, not the port's key — `Sample
Distance`, not `sampleDistance` — falling back to the key for a port whose definition cannot
be found. The key stays what a cable, a saved file and the accessibility name use; it is on
the row too, as a second line of the port dot's hover text, since it is what a patch file and
an export report name. A label too long for its row is elided at the end with `…`, the same
`fit` a select's value is cut with.

Every `UniformNumber` output draws what it published this frame, right-aligned against its own
dot with the port's label pushed left of it, in `theme::readout` — the uniform number port's
hue at a label's weight, so a number reads as belonging to the dot it is measured at without
competing with it. A port that has published nothing draws nothing: that is a node which has
not run, not a node reading zero. It is laid out at the same quantized size as the port
labels beside it and leaves at the same zoom they do.

**A `UniformColor` output fills the same slot with a swatch.** The color itself is the
reading, and a hex string beside a dot would be a worse one — the hex is on the port's hover
and in its accessible name, where a test and an agent read it. It is a rounded square the
**height of its row**, stopping two points short top and bottom, rather than a chip the size
of the text a number would print there: a color read off a swatch is read by area, and a
reading you have to squint at is worse than a number.

**A color input fed by a uniform color draws the color arriving at it**, exactly as a number
knob fed by a uniform number draws the arriving number: the swatch goes inert — silvia's own
dashed border rather than the inset bevel — and its picker does not open, because a connection
overrides the control. Where the producer has published nothing there is nothing to meter, and
the stored color is what the swatch has to say.

**A `VaryingColor` or `VaryingNumber` output fills the same slot with its thumbnail**: the
port's own function over the frame, 48 by 27, from the [workspace
pass](rendering.md#the-workspace-pass), as wide as 16:9 makes it at the swatch's height. Every
varying output on the workspace has one, connected or not, and it is **what the node makes,
not what a consumer asks of it** — a Zoom downstream does not zoom it. A color is drawn over
black, as the picture on an Output is. A number is shaded black to white between the ends of
its declared `OutputDef::range` where it has one, so a gray means the same number on every
frame, and otherwise between the lowest and the highest it reached on the grid that frame; a
number that is not finite is magenta. With the pointer over it the thumbnail is drawn five
times larger beside the pointer, with the two ends a number was shaded between under it.

**A varying output with no thumbnail yet prints its declared range there instead** — a
workspace whose pass has not linked, or one with `SUPERSILVIA_THUMBS=0` — where
`OutputDef::range` has one — `[0, 1]` on a `mask`, a `luminosity`, a
channel — in `theme.text_muted()`, the same slot the uniform readout owns. It is a static fact
standing in for a measurement there is none of: a `UniformNumber` output keeps its own live
readout in that slot and never carries a declared range, since a measurement beats a
declaration where there is one. An input's declared min and max stay off the row — the fill
bar is that annotation, and
[decisions.md](decisions.md#a-controls-range-belongs-to-the-node-not-to-the-node-kind) is
explicit that an instance's range can narrow the definition's, which a definition's `[0, 1]`
printed on an input row could not honestly promise.

**A dual output's port and readout follow its mode**, and neither draws anything special to
do it. The row reads the *instance's* port type, so a `math` node fed by knobs and diamonds
paints a diamond with the number its tick published, and the same node with a noise cabled
into it paints a circle with nothing beside it — the rule two paragraphs up, unchanged. A
constant is therefore two knobs and a diamond, and the flip is visible at the port the moment
the cable lands. The node browser still draws the definition's circle, because a node kind is
not an instance and has nothing feeding it yet; what a hand drops on the canvas is a diamond
on the first frame it is drawn. And a **pinned** dual node's inputs are diamonds by the same
rule read on the other side of the row — with a `slew` reading the `add`'s number a field
cannot land on either input, so both draw as any unconnected `UniformNumber` input does, and a
hand sees what may go there before it drags anything at it. See
[nodes.md](nodes.md#dual-outputs).

**Time is a diamond with no knob, and Speed is a knob.** Every node that moves with time
carries its time rows ([nodes.md](nodes.md#timing)): **Time**, **Speed** and **Offset**, one
of the first two at a time by the node's mode. In **Free** mode, the default, the row is
**Speed**, an ordinary s-number from −4 to 4 with a port: 1 is the node's own pace, 0 stands
it still and a negative number runs it backwards, and a cable — an LFO, an envelope — changes
how fast it runs. In **Loop** mode the row is **Time**, its label, its diamond and, where Speed's knob
stands in Free mode, [the loop meter](#the-loop-meter), which says where the node is in its own
loop: unplugged, the node reads ambient time — the playhead [the time readout](#the-time-readout)
shows — at a rate of its own, and a cable, usually a gear's Cycles, replaces it. A field cannot
land on either, since each is one number per node. **Offset** is an ordinary s-number in the
node's own cycles, added every frame in both modes, whose ends are one whole period either way
and follow the node's options as they change — −1 to 1 on a periodic node, −4 to 4 on a Perlin
at Repeat 4 and −16 to 16 at 16, a quarter either way on the tunnel's Helix, −1 to 1 where the picture never comes
back ([nodes.md](nodes.md#timing)) — with no accent ring, since that range is the node's
default rather than one a hand chose; on a node that draws it takes a field too, so a radial
cabled into it makes a ripple. A stateful node — a simulation, a
counter, an autoexposure — steps on the transport and keeps its pace in a row of its own,
**Rate** on the Slime Mold, the Cellular Automata and the Smooth Counter, **Response** on Auto
Exposure, **Drift** on Star Gate.

**Offset means that input alone.** Where another node's row said Offset for something else,
it says what it is: the Oscillator's DC level is **Level**, the Clock's hours from UTC
**Zone**, a gradient's or the Kaleidoscope's slide **Shift**, Chromatic Aberration's parting
**Spread**, Emboss's flat gray **Gray** and Star Gate's slit **Position**. A compound label
cannot be mistaken for it and keeps its word: Translate's X Offset and Y Offset, Tile's
Offset X and Offset Y, the Slime Mold's Sensor Offset.

**Two fixed decimal places, not three significant figures.** Every digit on the canvas is
one width (the text face's figures are tabular), so a fixed decimal count changes width only
when the integer part gains a digit —
once a decade — where significant figures re-flow on every crossing of a power of ten, and a
value crossing 1.0 sixty times a second would jitter its own label. A negative that rounds to
zero drops its sign, for the same reason.

A connected input's control is the other half of the same readout: disabled, and drawn from
the arriving value rather than the stored one, so it is a meter of what the node sees. **A
value outside the control's range keeps its digits and clamps its fill** — a position past
the end of a track is the end of the track, and rounding the number to the range would be the
control saying something untrue a second time. Where the producer has published nothing there
is nothing to meter, and the stored value is what the control has to say.

**A meter says it is a meter with its ground, not by being hard to read.** The number on a
connected control is the arriving value and the only place that value is written, so it is
drawn in `theme::readout` at full weight, exactly as the number on an output row is. What
carries the inertness instead is everything a hand would reach for: the fill behind it is the
uniform number port's hue and quieter, where a live control's is `primary`, and the steppers
are sunk with their glyphs at `text_disabled`. So the ground says which of the two things a
bar is before the number is read — **cyan is a knob, violet is a meter** — and the difference
survives at a glance across a node full of rows. Dimming the text is what a *stale* number
deserved, and the number stopped being stale when it started arriving down a cable.

**The cost rule for text that changes every frame.** A readout misses egui's galley cache by
construction, because its text is different on most frames. What bounds it is that there are
few of them and each is short: they are formatted into one buffer per node rather than a
`String` per port per frame, they are laid out at the quantized font size so the glyph atlas
is untouched, and they are drawn from the same row walk the labels are — so a culled node, a
collapsed node and a row a tick hides cost nothing, because none of them reach that
walk.

### The loop meter

**A Time row in Loop mode carries a loop meter where the Speed knob stands in Free mode**
(`ui::loop_meter`), in the knob's own place and size — `control_slot`, `number::WIDTH` by
`number::HEIGHT` — so switching the mode swaps the knob for the meter without either moving.
Its look is its meaning: the box is the node's own loop, its period `P` in its own cycles as
the graph gives it (`timing::period_in`: one on a periodic node, a noise's Repeat, a sequencer's
bars, a fraction of a cycle where a setting brings the picture back sooner), cut by a hairline
at each whole cycle — four segments on a Perlin at Repeat 4, one on a periodic node, and no
hairlines at all past sixteen (`timing::MAX_DIVIDED`) — and filled left to right by how far
through the loop the node is, `fraction(Time, P) ÷ P`, wrapping as the picture comes back. A
period that is a small fraction `p/q` is read as one (`chain::Fraction::near`), Time taken round
it as `q × Time` round `p`, so no rounding of it gathers over a long show; a loop shorter than a
cycle — four on the floor's quarter of a bar — is one box filled that many times a cycle, and
one that is not whole, two and a half, counts its last short cycle. In its middle, in the number
control's font, is the cycle it is in counted from one and the loop's length: `3/4`, `1/1` on
a periodic node, `11/16` on a Perlin at Repeat 16; a hairline parts around the caption rather than
crossing it. **A picture that never comes back** on its own — a noise at Repeat Never, the
tunnel at Depth Wrap None, a clip on Hold — has no loop, so the box spans the one cycle it is in, says
the whole cycles behind it, `12`, and leaves its right end open: the bevel stops short of the
right-hand corners and the end is dashed. A solid end means it loops. A hover says `cycle 3 of
4`, `back every 1/4 of a cycle`, `cycle 3 of a loop 5/2 cycles long`, or `12 whole cycles; never
comes back`.

It wears the number control's chrome — its ground, its inset bevel, its font, and the meter's
violet ground and number, since it reads rather than sets — but it is a readout: no caps, no
drag, no typing, nothing on a hover but the tooltip. It reads the node's Time as the node
does (`synth::Uniforms::time`): the `f64` the output cabled in published, and with nothing
cabled in, the reading published under the Time's own key — by the synth for a
node that draws, by `TickContext::cycle` for a CPU node, at the clip's own rate on a clip. It
never reads Offset, which on a node that draws is one value per pixel. Before the Time has a
reading the box is empty. It is drawn where the row is, so only in Loop mode under an open
Timing heading, and only at `DETAIL_ZOOM` and closer, as every control is.

## The color control

`s-color` is an 84x20 swatch drawn over a 5px checkerboard, so a transparent color does not
read as the node body. Its border is the s-number's own `bevel_border` — silvia's `2px inset
border-normal` is the same rule on both controls — so a color and a number read as the same
family of recessed field; a disabled swatch (its input is connected) wears a dashed
`border_subtle` outline instead, silvia's `s-color[disabled] { border: 2px dashed
border-normal }`, which is disabled's own signal rather than "recessed but usable." Clicking
it opens a popup — `ui::popup`, as the range editor and the select are, above every node
however late that node was painted — holding a picker of its own: a saturation/value square,
an upright hue bar and alpha bar beside it as tall as it is, and a column of the four channel
numbers under an `INT`/`FLOAT` toggle for the space they are in. A hex field and a `COPY`
button run underneath.

**The hex field is what silvia's four HSLA sliders bought and a picker alone does not**: a
color written down, pasted, matched to a brand, carried out of a design tool. It takes `#rgb`,
`#rrggbb` and `#rrggbbaa`, with or without the leading `#`, and commits on `Enter` or on
losing focus. **It defers to the picker**: it does not take focus when the popup opens, since
a picker is opened to pick, and a blur commits only a string somebody changed, never over a
color picked the same frame — otherwise a click into the square blurs the field and commits
the string a frame behind it, reverting the pick. Anything unparseable leaves the stored
color alone and the field snaps back to it; while the field does not have focus it mirrors
the square and the bars live, so dragging and reading the hex never disagree.

The picker's state is named by the control it belongs to (`ui::color::picker`'s `owner`: a
node and key on the canvas, an anchor in Preferences), not by one shared id: two different
swatches opened one after another must not inherit each other's typed text or hue. Both are
dropped the moment the popup is dismissed, so the next open starts on its own color.

## The press button

An action input whose definition declares `Control::Press` draws the button **as the row
itself** — silvia's shape, kept: the whole row from the port's label to the row's right edge
is the pressable target, filled in the action port's own color and captioned with the
port's label, rather than a small square hugging the right edge with a hand's width of empty
row before it. The port dot at the row's left edge, drawn outside the body as every port's
is, is still what a cable lands on; the button is the control that fires it. It reports a
**level** in `Effects::held`, not a click, because an action is a gate: the app hands the
level to the next `tick`, which turns held-and-then-not into a down and an up. It is not a
`Command`; pressing a button is playing the instrument, not editing the graph, so it never
enters undo.

Its accessibility name is unchanged by the row growing to fill the button —
`{slug}{id}.{key}`, same as the small square carried — so a test or an agent script that
found `output1.show_a` before still finds it.

Unlike a number control it does **not** go inert when something is connected. An action input
takes many sources and the hand is one more of them, which is what makes a button beside a
sequencer lane an override rather than a conflict.

**Snap is one of these.** An Output has three press rows — `Show on A`, `Show on B` and
`Snap` — and all three are action inputs, so a sequencer lane and a finger are the same thing
to each. What Snap does is [in rendering.md](rendering.md#the-snap-readback).

**A click reports one frame of hold.** `is_pointer_button_down_on` alone loses a fast tap
entirely: press and release inside one 16 ms frame leave it false at the end of that frame, so
the level never rises and the gate never opens. `held` is therefore that *or* `clicked()`,
which turns a tap into a down on the next tick and an up on the one after. This is invisible
below the UI — what `tick` sees is a level, and what a hand does is a click — so
`clicking_an_action_button_fires_a_gate` in `tests/ui.rs` is where it is held true.

## A node's own buttons

A **`Region::Buttons`** row is buttons that write settings of their own node: `lyapunov`'s
Random Seq, one button across the body, and `slimemold`'s nine presets, silvia's preset bar
numbered from one, sharing the width. The row is a `nodes::Buttons`, declared in the node's
own file — each button's key and caption, a `press` function from the button's index and a
seed to the `nodes::Settings` it writes, and an optional `pulse` — and `widgets::buttons`
draws it in the press button's chrome, [`press::click`](#the-press-button), at Reframe
Range's insets. Its names are `{slug}{id}.{key}`: `lyapunov1.randomize`, `slimemold1.preset4`.

**A press is an edit, not a gate.** It comes back as one `Command::SetSettings` — options and
number controls of one node together, each checked as `SetOption` and fitted as
`SetControls` would, all before any is written — so it is **one undo step** whatever it
writes, it is saved, and a `Code` option among it rebuilds the Outputs the node reaches while
anything else rebuilds nothing. It never coalesces: two presses are two steps. A button that
rolls dice rolls them in the widget, from the moment of the press mixed with the node's id,
and the command carries what was rolled, so redo replays the roll rather than rolling again.

**The part of a press that is the running world's goes by a pulse.** `pulse` names a key the
node's `tick` reads through `TickContext::downs`, and a press holds it for a frame in
`Effects::held` exactly as a press button holds its action input — so the slime mold's preset
nudges the world it lands on, as silvia's `_applyPreset` does, without the nudge being an
edit. It is not a port: nothing can cable into it.

**Not an action input**, where silvia's Random Seq is one. An option is a recompile
boundary, and a cable that rewrote one would rebuild the shader every time it fired. What a
tick *may* write is a node's own number controls, which is how `slimemold`'s `Randomize` stays
the action input silvia's is — see
[decisions.md](decisions.md#a-button-that-writes-its-nodes-settings).

## The throb on a firing

**An action is a moment, and a moment with nothing drawn is invisible.** So when an action
fires, its port says so: the dot brightens on the frame the firing lands and decays to
nothing over `ui::FIRE_SECONDS`, a sixth of a second. Where the port is an input with a press
button, the button throbs with it — the whole button fading toward the action color, which is
the chrome a finger on it already wears, so a firing that arrived down a cable or off a
controller looks like the press it is.

**One rule in the port drawing, not a readout per node.** Every action port in the library
gets it, on every node, because the fact is the same fact wherever it happens. A node that
also wants to say something about *itself* says it in its own status line: Random Fire's body
carries whether it is running, because Start/Stop leaves a state behind and no throb can show
a state.

Which ports fired is the **synth's** answer, not the editor's: `Events::fired` carries, a tick
at a time, every action port that saw a down — an output that emitted one, an input something
fired into, and a button a finger went down on. It is an event like a deck claim and for the
same reason: a firing on a tick the editor never read would otherwise never have happened
(see [architecture.md](architecture.md)). The editor applies the ticks it has not seen, and
`CanvasState` stamps each one on the first frame that *draws* it, so a throb is never spent
on a frame nobody saw. It is the same shape as the node throb a followed link leaves.

**silvia has no throb on a port.** The only geometry it has for a firing is Random Fire's own
`_showFireIndicator`, which steps a pill's background up one token and puts it back on a 50 ms
timer — a jump and a snap back, which reads as a glitch at a port's size. What is taken from
silvia is the duration of the one animation it gives an action control,
`.action-control-button { transition: transform 0.15s ease }`, with the brightening decaying
across it rather than being dropped at the end of it. The lift is one token's worth, which is
the size of step silvia's own indicator makes.

The throb is over the hover brightening rather than instead of it: the two answer different
questions — *this is the port you are pointing at* and *this just happened* — and a firing on
the port under the pointer must not be the one that cannot be seen.

## Options and the file button

Option rows are the third stacked section of a node, below the outputs, and every one of them
draws a **select**: `bg_interactive` at `RADIUS_SM`, a chevron, and a border that is the whole
of its state — `border_normal` at rest, `primary_muted` under the pointer, `primary` with a
ring around it while its list is up. A ring rather than a thicker stroke, so nothing moves
when it opens. See [design-system.md](design-system.md#component-rules-worth-stating) for
where those come from.

**The row's label is `OptionDef::label`, not the storage key**, falling back to the key for
an option not in the registry. `frequency` reads `Frequency`; the key is still what a cable
bus command and a saved file name, and what the select's own accessibility name carries.

**The select shows the choice's display name, closed as well as open.** `Node::options`
stores the machine value — `bayer4`, `cartesian` — and the select resolves it through
`OptionDef::choices` to paint the name a person chose, `Bayer 4x4`, `Cartesian`, falling back
to the stored value where it names no choice, which is what a hand-edited file can hold. The accessibility name carries whatever is shown, so `dither1.pattern Bayer 4x4` is
what a test and an agent read, not the value underneath it.

**It is the width of its value, not a fixed box.** It grows right-to-left from the row's right
edge, up to whatever the label leaves it, and what does not fit is elided — at the *end* for a
choice, where a menu's words differ, and at the *start* for a file, where the tail is what
tells files apart: the episode number, the extension. So `loop` takes the room `loop` needs,
and a path takes more of itself on a wide node than on a narrow one. The whole value is the
hover text either way. An option with none set reads `choose…`.

The row is `canvas::OPTION_ROW_PITCH`, 20 points, against `CONTROL_ROW_PITCH`'s 26 for an
input carrying an s-number. An s-number is taller than a select because it holds two
steppers and a track a hand scrubs along; a select holds one line and hugs it.

**An option that is only ever yes or no is a tick, and every tick a node has shares one
row.** `OptionDef::checkbox` says so, the value is `on` or `off`, and `canvas::Row::Checks` is
the single row they sit in at the foot of the option block — silvia's
`.audio-visibility-toggles`, where each one is centerd in an equal share of the width, the
way `justify-content: space-around` lays them out — unless a caption would not fit its equal
share, when each share is its own tick's width and what is left is spread evenly
(`check::shares`). The box is silvia's
`input[type="checkbox"]`: 13 points of `bg_secondary` at `RADIUS_SM` with a `border_normal`
hairline that brightens to `primary_muted` under the pointer, holding a checkmark in
`primary` when it is on, and the caption beside it is part of the target, because silvia
wraps both in a `<label>`. Nothing else about a tick is special: it is stored in
`Node::options`, it goes through `Command::SetOption`, and one click is one undo step.

**The ticks row is where a yes-or-no lives, and it never folds anything but port rows.**
`uniforms` and `events` hide runs of port *rows*; an option that folds anything else is a
heading instead —
`OptionDef::heading`, the same `on`/`off` value worn as a disclosure triangle. A node carries
both idioms and they are about two different things.

A heading usually sits on the region it folds, which is where the region declares it. An
Output's Render, Record and Send sections and a moving node's Time fold a run of **rows**, which is not
a band and has nowhere to hang a heading of its own, so the node names each option in
`NodeDef::row_headings` and `ui/node_widget` draws the same bar over the rows — one
`widgets::heading_row`, so they cannot drift apart by a point. A registry test holds every
heading option to having a region or a row heading to sit on.

**The time rows fold under a Timing heading that starts closed**, on every node that moves
with time (`nodes::timing::HEADING`, option `timing`): a new node runs at its own pace until a
hand opens it, and its Offset is set once and left, so they are the least read rows on the
node. The bar is `canvas::Row::TimingHeading`, in the input block where the first time row
is, over the rows under it, which a registry test holds every such node to; it takes no turn
in the banding, and the slabs either side round their corners against it as against a seam.
**The mode is on the bar**: two segments, **Free** and **Loop**, at its right end
(`widgets::heading_segments`, for the option `OptionDef::on_heading` names, `clockMode`),
drawn whether the heading is open or closed and registered after the bar, so a click on a
segment switches the mode and never folds the rows. The lit segment is the mode the node is
in. Switching shows Speed where Time stood, or Time where Speed stood, at the same height —
a Time row is as tall as the Speed knob it stands in for — so the node keeps its height, and
drops the cable in the row that goes away in the same undo step; a cable can never land on the
row the mode puts away. **A folded port keeps its cable**: a time row with a cable on it
gathers on the bar's left edge, so the cable is drawn into the heading that says the rows are
there, and one with nothing on it has no dot, as an output a tick hides has none — to cable a
folded row, open its heading. Every node that declares `timing` gets the heading and its mode
from `nodes::timing`: `node!` writes them, and a hand-written moving node takes them from
`timing::options!`.

Which options are ticks and which are headings is **the definition's** —
`NodeDef::checks` and `NodeDef::headings`, counted off its options and read through
`Node::def` the way `regions` is — so `canvas::rows` can tell that several options share a
row, and which are left to draw as selects. `Row::Option(i)` counts only the selects.

**A fixed width and height is the resolution picker, not a select** (`ui/resolution.rs`): an
Output's **Resolution**, the Text node's **Texture Size** and the Main Mixer's resolution.
`OptionDef::resolution` says so, and the value is any `WIDTHxHEIGHT` with both sides from 16
to 16384 — stored as its text, `option_is_valid` holding a file to that range rather than to
the choices, which are only the default's home. Closed, it is a select's box and chevron
holding a painted rectangle of the picture's shape, the ratio's name and the short side,
`16:9 · 1080` — `9:16 · 1080` stood on end, the size itself where it is no ratio on the strip.
Open, it is a popover, drawn after every node as a select's list is:

- **Shape**: *Wide* and *Tall* at the title's end, and a strip of thirteen cells, each a
  painted rectangle of its ratio over its name — 1:1, 5:4, 4:3, 3:2, 16:10, 16:9, 1.85, 2:1,
  21:9, 2.39, 32:9, 4:1 — and **Free**, a dashed square. *Tall* mirrors every glyph.
- **Size**, one row of short sides — 480, 720, 1080, 1200, 1440, 2160 — so 1080 is 1920x1080
  wide and 1080x1920 tall. Each one's size and memory is its hover text, and under the row is
  the readout: the size, its megapixels and the memory an Output of it commits, the figure the
  Output's own row prints.
- A width and a height to type, in the s-number's bevel, committed on Enter or a click away.

**Two axes, each kept while the other moves.** A shape keeps the short side, a short side keeps
the shape, and *Tall* keeps both. The long side is the short one times the ratio, rounded to
the nearest even number — an encoder wants even sizes, and a typed size is rounded the same
way — except 21:9, a marketing name rather than one ratio, which takes the sizes ultrawide
panels are sold at where there is one: 2560x1080, 3440x1440, 5120x2160. A size is a cell on the
strip only where that cell's arithmetic gives it exactly, so a typed size off the strip lights
**Free**, and a short side picked while Free keeps the size's own proportion. Clicking Free puts
the keyboard in the width. The popover stays up through any number of picks and closes on a
click away or Escape. Every glyph is paint, as the chevron is; every control is named under
the row's own name — `output1.resolution 16:9`, `output1.resolution 1080`,
`output1.resolution Tall`, `output1.resolution width`.

That same select **doubles as a progress bar**. While the node's CPU half is working, its
`NodeNote` supplies the label and a fraction, and the fraction is painted as a fill behind
the text. A video import transcodes for tens of seconds, and the place a person is already
looking is the button they just pressed — see [media.md](media.md).

Clicking it opens **the project's own media first**: a picker of every file in `assets/` that
this option accepts, with `Import a file…` under them. A rig is built out of clips that are
already imported, and reaching one of those through a file dialog means navigating into a
folder the project owns to find a copy it made itself. Choosing one is a `SetOption` carrying
the reference — nothing is copied, nothing is read, and no dialog opens.

Each entry is a **card**, not a line: a picture, the name and the size, the same vocabulary the
project tab's asset list uses, because a clip is recognized by what it looks like and a folder
of episodes named by their numbering is exactly what a list of names cannot answer. The picture
is the poster `App` decodes out of the file — see [media.md](media.md#posters). Any popup that
hangs off a select is at least as wide as it: a list narrower than the button that opened it
reads as a different object.

**Every row in one of these lists is the width of the list**, not the width of its own text.
egui's default is to size a `selectable_label` to its words, which leaves the space beside
them — space that reads as part of the same row — doing nothing. `top_down_justified` is what
fixes it, and it applies to the choices in a select's list and to `Import a file…` alike.

What an option accepts is `OptionDef::accepts`, one value naming both the dialog's filter and
what the picker offers, so the two routes to a file cannot disagree about what the node can
play. **A picker with nothing in it is not a choice**: where the project holds no file of the
kind, the button skips the menu and asks for the dialog, which is the gesture it always was.

The dialog itself opens no dialog here either. It sets `OpenControl::File`, which leaves the
canvas as a `file_requests` entry for `App` to answer, because `ui/` never blocks the frame.

**A select a cable answers is drawn disabled.** Where the option names an input in
`OptionDef::overridden_by` and something is connected to it, the choice is not what the node
reads, so the button shows that input's **label** instead of the chosen value, in the readout
color a connected number control uses, inside a subtle border, and senses hover only — a
click opens nothing. It stays in the accessibility tree in both states carrying what it
shows, so `tap1.measure Number` and `tap1.measure luminosity` tell a test and an agent which
state it is in. Nothing here names a node: the field on the definition is the whole of what
the canvas consults.

## Accessibility is the agent's API

Every node body, port dot and cable is a real `Response` carrying `widget_info`, so the tree
names them:

```
checkerboard1                                    the drag handle
help checkerboard1                               the header's `?`; hovering it adds its tooltip
checkerboard1.frequency (varying number input)
checkerboard1.output (varying color output)
audioin1.level (uniform number output) 0.42      a uniform output carries what it published
checkerboard1.frequency 8/⬓                      the s-number, value in the name
cable audioin1.level to oscillator2.frequency
```

This is not decoration. It is what lets a headless test click a port by name, and what lets
an agent drive the running window without guessing coordinates. **A hand-painted rect a user
can click but the accessibility tree cannot see is a bug.**

**A name is written only when something reads it.** Every control on a node is named by its
parts — `ControlName`, the slug, the id and the key; `NodeName` for the node itself — which
are what its widget is keyed by, and the string is formatted inside `widget_info`'s closure,
which egui calls only while an accessibility client is listening, or on the frame a tooltip
shows. A frame with nobody reading formats no names.

**A name has to be unique on screen.** Two widgets under one name is a name that names
neither: `get_by_label` fails with "found two or more", and an agent asked to press one has no
way to say which. The strip's arrange is *Auto-arrange* for exactly this reason — the offer a
switch to Linear puts up already answers with a button called *Arrange* — and a test is where
that collision surfaced.

Cables were the one thing that broke that rule, and no longer do. Each carries a `Response`
on a small square at the curve's midpoint, named
`cable {slug}{id}.{key} to {slug}{id}.{key}`, while the pointer still deletes by distance to
the whole curve — a 12-point square would be a far worse target than the cable itself. **The
widget carries the name; the geometry carries the click.** That split is what let the
agent-driven layer reach every control and no cable, and
`a_cable_is_in_the_accessibility_tree_and_deletable_by_name` in `tests/ui.rs` pins it.

## The Render section, and the band across the editor

An Output carries its render on itself, in silvia's offline output's shape merged onto the one
node, under a **Render** heading that folds it, closed on a new Output: **Supersampling**,
**Warm-up Mode** and **Writer** selects — the writer a PNG sequence, a video file or an
animated GIF — three numbers — **FPS**, **Duration** in seconds,
**Warm-up** in frames — and one button. (The option's key stays `offline`, silvia's own name
for the section and what every saved file holds; the heading reads what is under it.) The
numbers are hidden controls of the node, so they are document data, undoable and saved; the
selects are options drawn here rather than in the option block, so the whole section folds
together. Its rows are `canvas::Row::Render` under `canvas::Row::RenderHeading`, laid out from
the kind's `is_output` and that option the way its picture is.

**A heading and not a tick.** A tick in the shared row at the foot of a node is the
affordance for hiding a run of ports, and a ticked-off one says nothing at all about what it
hid: the render simply was not there. The bar with the disclosure triangle is on the node
whether it is open or closed, which is what says *this part of the node is here, and folded* —
the same thing the scope and the preview already say. The bar is drawn on the body's own
ground rather than on a banded row, and it takes no turn in the row alternation, so the rows
under it keep the banding they had. The button reads **Render**, and while that Output's render runs it reads
**Cancel n / N** with the progress filling it from the left: the button is the progress bar,
which is where silvia puts its own. A click is a `RenderRequest` — not a command, since a
render is done to the instrument — and `App` starts or cancels from it.

**Supersampling is the render's alone.** silvia's three — **1x (off)**, **2x**, **4x** — over
the Output's own `resolution`, which stays what it says and stays what is on screen. For the
length of a render the Output is *drawn* at that resolution times the multiplier and every
frame is brought back down to it before it is read, so the film an editor is handed is the
size the preview was and the hard edges in it are averaged rather than stepped. It is the
first select in the section because it says what the render is *of*. What the multiplier costs
and how a frame comes back down are in
[rendering.md](rendering.md#the-capture).

**An Output's Render section is the one way to render**, and a loop is rendered through it:
a Master Gear's caption says how long its loop is ([the gear region](#the-gear-region)), and
a render of that Duration loops — the frame after its last is its first, for everything on
those gears — since a render starts at the playhead's zero, where every gear is at the start
of its cycle. While it runs the render owns the playhead, and [the time
readout](#the-time-readout) shows its time and takes no hand.

**The `!`.** An Output with a camera, a screen or a microphone somewhere upstream — or a
`maininput` node while the panel is pointed at one — wears a `!` at the right end of its
Render button: a render of it reads whatever those give at whatever moment the frame is
drawn. Clicking it lists each such source, and a row goes there — the node on its
workspace by the tag's own navigation, or the Main Input panel unfolded. It warns and nothing
more: the render runs, which is the decision in
[decisions.md](decisions.md#a-render-warns-about-a-live-source-and-does-not-refuse). The
walk is `App::live_sources`, back through the cables from each Output, across workspaces as
the cables go, on the registry's `CpuDef::live`.

**A render is modal, and the editor says so.** A band in the action color runs under the
tabs and over every workspace for as long as one runs — *RENDERING Output 3, frame 120 / 300,
renders/output3-001* — with the progress darkening it from the left and the one button that
cancels at its right. Behind it the document is closed: `App::apply` refuses everything, and
every Render number goes inert. See [rendering.md](rendering.md#the-render-job) for what the
band is over.

## The Record section

**An Output records itself live under a heading of its own**, **Record**, between Render and
Send: the same bar and triangle, closed on a new Output as Render is: a set that never
records should not carry its rows. (Option `recording`, `on`/`off`, saved and undone like the
other two; not `record`, which is the Record button's name.) Under it, two rows:

- **FPS**, the recording's own frame rate, 1 to 120 and 30 on a new Output: a hidden control,
  so document data, undoable and saved, and unbindable as the Render numbers are. It is apart
  from the Render section's FPS, so a film rendered at one rate and a set recorded at another
  are each set once. It is read once, at Record, so it goes inert while this Output records,
  and while any render runs, as every Render number does.
- **The Record row**, which records the Output live, to a video in `recordings/`, while the
  show plays — the [live recording](rendering.md#live-recording). It is a way out's row in
  shape and size: its label, a dot and one line, and one button at its right.

| the line | the dot | the button |
| --- | --- | --- |
| *off* | `○` | **Record** |
| *0:12 · 0 dropped* — how long on the show's clock, and the frames that repeat the one before for lack of a picture | `●`, in the accent | **Stop** |
| why the last press or recording failed — *the Output has nothing connected*, *no hardware video encoder…* — until the next press | `○` | **Record** |
| *not while rendering*, while any render runs | `○` | **Record**, inert |

Its rows are `canvas::Row::Record` under `canvas::Row::RecordHeading`. The render's other
settings — supersampling, warm-up, writer, duration — do not apply to a recording; its size is
the Output's **Resolution**. A click is a request, not a command, as Render's is: a recording
is done to the instrument, so nothing enters the undo history and nothing is saved. The line
is cut short where the row runs out and never wrapped, so the row is the one height whatever
it says; the pointer on it shows the whole of a failure, or what recording does. A recording
that ends says so on the status line with the file and a Show — and why, where it was not the
button. While anything records, the Render button reads **Stop recording to render** and does
nothing.

## An Output's status line

**One line directly above an Output's own picture**, which is where silvia puts its own — the
node's last row, after its [Send rows](#sending-an-output-out) and before the region its
render sits in.

| | |
| --- | --- |
| `● Input` / `○ No Input` | something is cabled into it |
| `● On A` · `● On B` · `● A & B` / `○ Hidden` | which deck of the mixer claimed it |
| `● Rendering` / `● Recording` / `○ Ready` | a render, or a live recording, of **this** Output is running |

**Color says the same thing twice and never says it alone.** silvia's cells are green, gray,
amber and red; the palette here derives from four anchors and carries no green and no red, so
the hue cannot be ported. The filled and hollow dots carry the state — `●` against `○`, which
reads with the hue turned off — and the ink under them is the neutral ladder. The one
exception is the capture cell, which takes the accent while a render or a recording runs: that
is the only state here somebody needs to see from across a room.

**It is not behind View ▸ Costs.** A cost strip is a measurement and is off by default; a
status line is what the node *is*, and silvia shows its own always.

**A memory figure sits beside the resolution.** silvia prints one under its own resolution
and moves it as the resolution moves, so a second Output at 4K can be thought about before it
is made. Here it is on the resolution row itself, between the label and the picker, in
`text_muted`: whole megabytes, which is the grain the choice is made at, and the picker prints
the same figure for the size it holds and for each short side it offers. It is the two
half-float targets an Output commits (`nodes::output::memory_bytes`, eight bytes a pixel,
twice) and nothing else; a capture ring
exists only while a render runs, and the figure is there to be read while one is not. It is
not behind View ▸ Costs for the same reason the status line is not: it is what the node *is*.

silvia's **frame-history count** has no counterpart. It was the size of a `sampler2DArray` a
fragment indexes at a layer of its own choosing; the array went with the `feedback` node, and
[history-delay.md](../proposals/history-delay.md), which argued it back, was declined. One frame
back, through Frame Out, is the depth there is.

## The MIDI window

**Project ▸ MIDI…**, `ui/midi.rs`. Under Project rather than Edit because a binding names a
control on a node and is saved in the project manifest — it belongs to the project the way
its nodes do, and opening another project brings other bindings. A window of its own and not
a page of Preferences, for the same reason from the other side: a preference is about the
person and outlives every project, so one surface holding both would be the one place in the
editor mixing two tiers of saved state. It is not a panel either — the
two panel slots are the rig's.

silvia's three sections, in silvia's order, with the gesture in the header because somebody
opening this window is looking for how to bind something.

| | |
| --- | --- |
| **Devices** | every source the sequencer offers, `●` where it is wired in. **Rescan** re-scans and connects anything new; on macOS a device plugged in later is wired in within a second without it. There is nothing to pick: every source is wired in at once, because a binding names a channel and a number rather than a box. **Release all**, beside Rescan, lets go of every action input a held note is pressing — the way out of a note-off that never came. A device that goes away lets go of its own notes without it ([media.md](media.md#the-map)) |
| **Mappings** | one row per live binding — the node as a link, the control, the trigger, what the knob last said, and `✕` to forget it. **Clear all** forgets the lot. A deleted node's bindings are kept for an undo and listed again when it comes back |
| **Monitor** | off by default. On, every arriving message is listed, newest at the foot, whether or not anything is bound to it — which is how you find out what a knob actually sends |

**The window is the editor's, and the knob is not.** A message is read and acted on inside the
synth's tick, so a bound control moves with the editor minimized; what reaches this window is
the copy each tick leaves on its snapshot. Everything here — the device list, the mappings, the
monitor, learning — is therefore a thing that happens while somebody is looking at it, which
is the only time it means anything. See
[architecture.md](architecture.md#midi-is-read-by-the-tick-and-the-document-catches-up).

**The node's name is a link**, as every name in the Status box is, and clicking it shows that
node where [every link to a node](#the-tag-a-cable-whose-far-end-is-elsewhere) goes.

**The monitor keeps two hundred messages and only while it is watching.** A knob sends a
hundred a second; a log that kept them all would be the largest thing in the process by the
end of a set, and one kept while nobody is looking is a leak with a lid on it.

### Alt + click to bind

silvia's own gesture: `Alt` and a click on any **number control** or **action button** binds
it to the next message that arrives. **Bind MIDI…** in the range editor arms the same wait. The
MIDI window stays as it was: it opens from its menu entry and nowhere else.

**The control that is waiting breathes in the accent** until a message binds it. A ring inside
its own edge, at the radius and on the line where its hairline and a narrowed range's accent
border already sit, `ui::learning_ring`, rises from a third of the accent to all of it and
back once every one and a half seconds — the period of silvia's `midi-learning-pulse`. It is
the only sign that learning is armed, so the number control, the press button, a region's own
number and the mixer's fade all wear the same one. Accent rather than silvia's primary for the
reason the node throb is: primary is a selected node's border and a control's own fill.
Inside rather than silvia's ten-point halo, because a halo would spill onto the rows either
side and over the bound dot. It is a named state too: while it breathes the control's
accessible name ends in `(learning MIDI)`, which is what a test and the egui MCP read.

**Learning replaces a binding.** A control that is already bound loses its binding, and its
dot, the moment the learn is armed, so what it waits for is the new one.

**`Escape` stops the wait** and binds nothing, as silvia's does, so a control that was bound
is left unbound. The editor takes the key
before any popup, field or scrub is offered it, so on that frame it means only this; with
nothing waiting, `Escape` is theirs again. The window's **Cancel** does the same.

Three things it deliberately does not do:

- **It does not move the control.** An `Alt` drag would otherwise scrub the value on the way
  to binding it, and the number would end somewhere nobody asked for.
- **It does not open the typed editor**, which lives on the same pixels. Held, `Alt` takes
  the click before `number::typed_entry` is offered it at all.
- **It does not fire the button.** Binding a `Show on A` that also claimed the deck would cut
  to it while teaching a note to cut to it.

**A bound control wears a dot** in the accent, just outside the control — silvia's
`midi-mapped` indicator. Outside rather than on it, because a control is drawn to its own
edges and a dot inside would sit on the trough, or on the caption of the one control that
fills its row. **Every bindable control wears the same one**, drawn by `ui::midi_mark`
wherever the control is: a row's number, swatch or press button has it just left of its slot;
a number a region draws — `cosinegradient`'s twelve, `euclideanrhythm`'s lane numbers — has it
on the cell's top-left corner, in the gutter, since a grid's cells stand too close for a dot
beside one; and the Main Mixer's fade has it after its caption. It is a named widget rather
than paint alone: what drives a control is a fact about it, and `checkerboard1 frequency bound
to CC 21 ch 3`, `cosinegradient2 freqG bound to CC 21 ch 3` or `A / B balance bound to CC 1
ch 1` is what a test and the agent-driven layer read.

**The window's unbind is a drawn cross, not a typed one.** `✕` is not in the fonts this ships
with, so a `"✕"` button is a tofu box — every other cross in the editor is its own geometry
for the same reason.

**Not everything is bindable, and what is not says so by doing nothing.** A number a region
draws is bindable — it has no port, but it has the same `{node, key}` address — and the
Render section's own numbers are not: the Output lists them as `NodeDef::unbindable`, so
`Alt` + click does nothing there and their range editor has no MIDI row. silvia says the same
about its Frame History, which is `midi-disabled` for the reason a control nobody turns
mid-set does not want a knob. The Main Input's controls and the
Main Mixer's method and resolution are the rig's rather than a node's ports and have no
address either. **The fade, Blackout and Freeze have one**: the crossfade is the one control
a VJ would most want on a fader, and the two presses are what a hand reaches for when the set
goes wrong, so `Alt` + click on the **A / B balance**, **Blackout** or **Freeze** binds it
exactly as a node's control does. Each wears the same dot — after the fade's caption, `A / B
balance bound to CC 1 ch 1`, and on a press's top-left corner, `Blackout bound to Note C2
ch 10` — and its row in the window names *Main Mixer*, a link that unfolds the panel, and the
control, with the `✕` that forgets it. The binding is saved with the project as the others
are; the fade's position and whether a press holds are not, since [the mixer does not
persist](decisions.md#the-mixer-does-not-persist-and-the-main-input-does). See
[media.md](media.md#the-map).

**A fader out of pick-up wears a ghost.** With [soft takeover](#the-preferences-window) on, a
CC whose value disagrees with its control moves nothing until the fader passes the control's
value. Meanwhile the control wears a hairline in the accent across its trough, short of its
top and bottom, at the place the fader is — on the fill's own scale, so the gap between the
fill's edge and the hairline is the way to steer. It is drawn inside the `s-number`'s own
size, so nothing on a row or the panel moves, by `ui::number` for every number a MIDI fader
can drive: a row's, a region's and the fade. It is a named state too: the control's
accessible name ends in `(MIDI fader at 0.42)`, the fader's place at the control's own
precision. It goes when the fader picks up, and with the preference.

## About and Licences

**Help ▸ About supersilvia** and **Help ▸ Licences…**, `ui/about.rs`: two `egui::Window`s of
ordinary widgets, like Preferences, each closed by its own ✕ and neither changing size with
what it holds.

**About** is the mark, the name, the version from `Cargo.toml`, its one-line description, the
licence — AGPL-3.0-or-later, in LICENSE's own opening sentence, with the section 7 permission
for the NDI® runtime read out of LICENSE's head so the two cannot differ — the source link,
`Cargo.toml`'s `repository`, and a **Licences…** button that opens the other window beside it,
with a line of its own under it saying what that window holds — wrapped to the window rather
than beside the button, so no font runs it past the window's edge.

**Licences** is a row of five tabs over one fixed frame of monospace text that scrolls both
ways:

| tab | what it holds |
| --- | --- |
| **supersilvia** | LICENSE whole: the notice, the section 7 permission, the GNU AGPL |
| **Rust crates** | every crate compiled into the binary, its declared licence, and every licence text its package carries — `platform::notices::RUST_CRATES` |
| **Assets** | the repository's `licenses/` folder, file by file: the notices carried over from silvia |
| **GStreamer** | its LGPL, and that Linux does not carry it: a source build or an install links the distribution's, the AppImage the host's, the Flatpak the `org.freedesktop.Platform` runtime's — `platform::notices::GSTREAMER` |
| **NDI®** | the trademark line, and that the runtime is Vizrt's, installed by the user and never shipped |

**Static height, and cheap at any length.** The crates' tab is twelve thousand lines. Every
tab is hard-wrapped once, the first time it opens, to the ninety-six columns the window is wide,
and drawn with `ScrollArea::show_rows` — rows of one height, of which only those in view are
laid out — so a long line never reflows the frame and the longest tab costs a frame what the
shortest does.

**On Linux the crates' notices are a committed file**, `packaging/linux/rust-crates.txt`,
which `scripts/crate-licenses.py --target x86_64-unknown-linux-gnu` writes and the binary
compiles in: a source build, the AppImage and the Flatpak show the same text with no file to
find, and no build needs the network or Python. `tests/notices.rs` runs the script's
`--check` against Cargo.lock, so a lock that moves fails `cargo test` with the line that writes
the file again, and holds `ui::about::ASSETS` to the `licenses/` folder. Why a committed file
rather than one written at build time is in
[decisions.md](decisions.md#third-party-notices-a-committed-file-held-to-the-lock).

## What a tester can send

Four things, in `app/crashlog.rs` and `main.rs`, so a person who never opened a terminal has
something to send when it goes wrong
([decisions.md](decisions.md#a-tester-sends-a-log-a-notice-and-one-block-to-paste)).

**A log file, beside stderr.** `supersilvia.log` in `logs/` under the app's own folder in the
data directory: `~/.local/share/supersilvia/logs/` on Linux (`$XDG_DATA_HOME` where it is
set), `~/Library/Application Support/supersilvia/logs/` on macOS, `%LOCALAPPDATA%\supersilvia\logs\`
on Windows. `--check` names it at its
foot. stderr keeps env_logger's default, errors alone; the file takes `RUST_LOG` where it is
set and `warn,supersilvia=info` where it is not, each record with its time to the millisecond
in UTC. A panic is written there with its backtrace, on whichever thread it happens: every
frame with its address, and after them where the binary was loaded, so a backtrace from the
AppImage, whose binary carries no symbols, is named afterwards
([packaging/README.md](../packaging/README.md#the-appimage)). A lost GPU is written with the
line stderr is given. The file stops growing at 16 MB. A `--check`,
`--version` or `--help` never touches it.

**A run says how it ended.** The log's last line is `== closed` once the window has gone, or
once a box has said why the app could not start; a line starting `!! ` is a reason the app
went down — a panic, a lost GPU, a start that cannot go on. The next launch reads the file
before writing a line of its own, keeps it as `previous.log`, and a log with no closing line
is a run that closed unexpectedly, for the reason it wrote last. While one supersilvia runs,
its log is locked; a second one started beside it writes `supersilvia-<pid>.log` and judges
nothing, and the next launch to hold the main log deletes those whose process has gone.

**An exit before the window is up says so in a box on the desktop**, `platform::alert`: no GPU
to render on, the window not opening, a panic on the main thread while starting. The box is
titled *supersilvia cannot start*, says why, says that `supersilvia --check` in a terminal
names what is missing, and gives the log's path. It is `rfd`'s message dialog — on the Mac a
`CFUserNotification` alert, on Linux `zenity` where it is installed and `kdialog`, KDE's,
where that is, since there is no portal for a message. A desktop with neither gets stderr and
the log alone.

**The launch after a run that went down** puts up *supersilvia closed unexpectedly* before
anything else is asked; see [the menu bar](#the-menu-bar).

**Help ▸ Report a problem…** opens a window of that name, `ui/report.rs`, as plain as About
and of one size whatever is typed in it. Two fields: **What's up?**, which holds the keyboard
when the window opens, a band six rows tall that scrolls inside itself, and **Your name or
Discord handle**, one line, optional.
Under them, one row each, cut rather than wrapped: the version, the OS, the GPU, and what
comes with them — `--check` and the end of this run's log, and the last run's when it closed
unexpectedly. **The full report**, a disclosure that starts closed, shows the whole block in a
fixed frame that scrolls; opening it is the one thing that changes the window's height.

What the rows say is gathered when the window opens, afresh each time, on a thread named
`report`, and they read *gathering…* until it lands: the operating system — the
distribution, kernel, desktop and session on Linux, the macOS version and processor on the
Mac — the GPU the editor renders on and how it was picked, whether the last run closed
unexpectedly and why, `--check`'s report with that GPU handed in rather than a second device
opened, and the last 60 lines of this run's log, and of the last run's where it went down.

**Copy report**, one wide button, is the one way to the clipboard, and it is off until the
gathering lands. It copies one block: the person's words under a bold **What's up?** — *Not
filled in.* where it is empty — and their name after it where they gave one, then the
version, OS and GPU, the last run, and `--check` and each log fenced with three backticks, so
it pastes as code into Discord and into a GitHub issue alike. Once it has copied, it reads
*Copied. Now paste it in either place below*, until the next keystroke in a field. Under it
are two buttons that open a page and copy nothing: **Open the Discord bug channel**, the
Discord's bug report channel, and **Open a GitHub issue**, a new issue on the repository
titled with the first line of What's up?, cut to 80 characters — the title alone, since a
body with the logs in it would outrun what a URL holds. The GitHub link answers once the
repository is public. The window closes by its ✕ and keeps no place between runs.

## The toast

**The one line of news a person sees with the Status box closed**, `App::show_toast` in
`app/show.rs`: a line at the foot of the window, floating over the canvas so it moves nothing,
each one line whatever it says. Three kinds:

- **News of the editor itself** — *Editor hidden — press H to show*, *Undid Delete 3 nodes* —
  gone in four seconds. What `H` says replaces what it said last, since `H` twice is one state
  and not two; other news stacks, so three undos show three lines. One after an undo or a redo
  carries a **▸ go** where the change is out of view, and stays as long as a file written
  does — see [Undo by name](#undo-by-name).
- **A file written** — *snapped 1280x720 to snaps/…*, *rendered 120 frames to …* — with a
  **Show** after its text, which opens the folder holding the file with the file selected.
- **A failure**, every one that reaches the Status box's file line: a failed save, open, new
  project, Save as, import, export, Snap or render, a render or a project refused — *a render
  is running: cancel it first*, *Friday is not empty* — a projects folder that cannot be
  made or read, and a paste from another project that could not find a file it names. It wears a `⚠` before its text and an edge in the accent, since the palette has
  no red, and joins [the problems list](#problems). One path carries them, `App::fail`, which
  writes the status line, the toast and the list together; a render's outcome, which `Media`
  reads itself, goes through `Media::fail`, which the toast hears before it draws.

A file written, a failure and a toast with a ▸ go stay eight seconds. Every toast stays as long
as the pointer is on it.

**A short queue.** Up to three stand at once, stacked, the newest at the foot; a fourth pushes
the oldest off. Two things said in one second both stand — a Snap's line is not written over by
the failure after it — and the same line said again moves to the foot rather than standing
twice. `App::toasts` reads them, oldest first, for a test.

## Problems

**A badge, `⚠ 3`, at the right of the menu bar, left of [the time readout](#the-time-readout),**
whatever the preferences show: `ui/problems.rs`, counted by `App::problems`. It is a slot of
fixed width — a square for the glyph and three characters, the count reading `99+` past 99 —
drawn empty while there is nothing to count, so a count arriving or leaving moves nothing
beside it. It is in the accent while anything is an error and in the ordinary ink while
everything is a warning, and its hover counts each. Named `problems 3` in the tree.

**A click opens the list under it**, errors first, a line each, cut to its width with `…` and
whole on its hover:

| what | kind | `▸ go` | kept |
| --- | --- | --- | --- |
| a node at fault — what the flag on its header says | error | the node | while it is true |
| an Output whose shader failed | error | the Output | while it is true |
| a failure said this session, newest first, at most eight | error | — | until **Clear** |
| every warning of the last Open, not only the first the status line has room for | warn | the node it names, where it names one still there | until the next Open, or **Clear** |
| what the last export left behind: each cable, each node shown on other workspaces, each missing file, the MIDI bindings | warn | the shared node | until the next export or Open, or **Clear** |

A row's `▸ go` shows the node, on its workspace, centred, as every link to one does, and
closes the list; it is named `problem 2 go`. **Clear**, at the foot, forgets what was said —
the failures, the last Open's warnings, the last export's report — and what is still wrong
stays, since it is still wrong. A click away from the list closes it; a click on the badge
opens and closes it. Nothing of it is saved: a problem is about this run. The list draws and
returns a `ProblemAction`, and `App` answers.

## The keyboard shortcuts window

**Help ▸ Keyboard shortcuts…**, `F1` or `Ctrl`/`Cmd`+`/`, `ui/shortcuts.rs`: a fixed-size
`egui::Window` like About, its rows scrolling inside it, in five groups by where a key is
answered — **Global**, **Canvas**, **Number control**, **Nodes menu and browser** and
**Picture windows** — each row the keys in monospace and what they do. A gesture of the
pointer's is a row as well, with what is held through it: `Shift+click`, `Alt+click`,
*double-click a cable*.

**A key is drawn as a key**: a cap per key with a legend printed on it, the space bar and a
pointer's gesture staying words ([design-system.md](design-system.md), "A key is a cap"). The
arrows and Enter are rows of keys like any other, not text. **A group's heading is a size above
its rows**, 15 px in the primary ink against the rows' 12 in the secondary, with a rule under it.
**The keys are one column at a fixed 230 points in every group**, so the descriptions start on one line down the whole window, and a description too long
for the window wraps inside it rather than widening it past its frame.

**On a Mac nothing in it says Ctrl or Alt.** Every key and every gesture is spelled by egui for
the machine it runs on: `⌘` and `⌥` where the font has them, `Cmd` and `Option` where it does not.
Only a key held with Control itself says so, as `Ctrl+Tab` does on a Mac too. The number
control's fine and coarse scrubs are rows of their own, `Shift+drag` and `Ctrl+drag` (`⌘+drag`),
rather than modifiers named in a sentence. The MIDI window's hint and the Blackout and Freeze
hovers name Alt the same way, so a Mac reads `⌥ + click`.

**It is one table, `shortcuts::TABLE`, and the window draws nothing else.** A key the app
consumes from the top of the frame or the menu prints is a constant in `menu::keys`, and its
row names that constant rather than spelling the key again; a key a widget reads for itself —
the number control's `D`, a picture window's `K` — is written once in the table as the key it
is. Two tests hold it: one reads `menu::keys` out of the source and fails, by name, on any
constant no row names, and one fails on any key a menu entry prints that the table does not
list. So a key constant added to `menu::keys` reaches the window as one row in the group it
is answered in, and its entry as one `.shortcut(...)`.

## The Status box

**Show the Status box**, under Preferences ▸ Performance, opens a window that answers the
question a person opens it with — *how fast is it running, and what is holding it back?* — on
its first three lines, and then says where every millisecond went. `ui/status.rs`, drawing from a `StatusView` that `App` fills.

```
┌ STATUS ───────────────────────────────────────  17 Hz of 100 ┐
│ held back by   GPU  ██████████████████████    56 ms / 10     │
│                CPU  ██████▏░░░░░░░░░░░░░░░    15 ms / 10     │
│ pacing         even   · p99   68 ms · 0 dropped              │
│                GPU waits 100% of ticks · by cause: none      │
├ ▾ GPU per tick ──────────────────────────────────── 55.5 ms ─┤
│   uploads                            0.2                     │
│   sims                               0.0                     │
│   outputs         ██████████████▉   51.5  (14 drawn of 43)   │
│   probes          ▉                  3.0                     │
│   mix             ▎                  0.8                     │
│   between                            0.0                     │
│   whole process    97 % of the render engine                 │
├ ▾ CPU per tick ──────────────────────────────────── 60.5 ms ─┤
│   waiting on GPU  ████████████      45.2                     │
│   editor messages ▏                  0.3                     │
│   MIDI                               0.1                     │
│   CPU nodes       ███▍              12.8                     │
│                   lfo4               5.1                     │
│                   slimemold12        2.6                     │
│                   —                                          │
│                   —                                          │
│                   —                                          │
│   frame job                          0.1                     │
│   draw submission ▍                  1.6                     │
│   publishing                         0.1                     │
│   tap readbacks                      0.1                     │
│   snapshot                           0.1                     │
│   everything else                    0.1                     │
│   idle                               0.0                     │
├ ▸ Editor  11 ms · 90 fps ────────────────────────────────────┤
├ ▾ Costliest Outputs ─────────────────────────── top 8 of 43 ─┤
│   Kuwahara        Effect kernels    12.8 ms  ▸ go            │
│   Wavefold        Effect manglers    6.4 ms  ▸ go  shader e… │
│   Mandelbrot      Sources            4.3 ms  ▸ go            │
│   Slime Mold      Effect kernels     3.2 ms  ▸ go            │
│   Kuwahara        Effect manglers    2.6 ms  ▸ go            │
│   Wavefold        Sources            2.1 ms  ▸ go            │
│   Mandelbrot      Effect kernels     1.8 ms  ▸ go            │
│   Slime Mold      Effect manglers    1.6 ms  ▸ go  idle      │
│   …35 more under 2 ms                               show all │
├ ▸ Project  212 nodes · up 4:07 ──────────────────────────────┤
├ ▸ Nodes  (2 reporting) ──────────────────────────────────────┤
└───────────────────────────────────────────────── ◰ copy ─────┘
```

**Terminal-like on purpose.** Every character is one cell of the monospace face — the frame
is box-drawing, the bars are `█` with an eighth block at their end and `░` for the track — so
a line's width is a count of its cells, the right edge meets itself, and the box pastes into a
message or an issue as the same picture. A name is counted as a terminal counts it: a CJK
character, a full-width letter or an emoji is two cells and a combining mark none, so a
workspace named in any of them keeps the frame closed where it is pasted. `the_status_box_draws_in_whole_cells` in
`tests/ui.rs` holds every glyph to one cell. The colors are the theme's: the frame in
`border_strong`, headings in `text_secondary`, what is secondary in `text_muted`, what is
clickable in `primary`, and **anything over budget, dropping or failing in `accent`**. The box
is laid out at the window's inner width in whole characters: the window opens at sixty-four
and cannot be dragged narrower than sixty-two, so every line, a content row as much as a
heading, is padded to the box's width and closes on `│` in the column the corners and the
headings' `┤` stand in. Dragged wider, the box gains columns and a cut line shows more of
itself. The top edge and the verdict stay put above the sections and the bottom edge stays put
below them, and the sections scroll between, inside a window that stops at two thirds of the
screen's height.

**No reading moves a row.** A live figure changes what a line says and never how many lines
there are or where they sit, so nothing beneath it flashes or reflows:

- **Every section has a fixed number of rows.** The verdict is four whatever is measured —
  `measuring` and an empty row before the first tick, a GPU row with a dash until the GPU's
  first reading lands. The GPU and CPU sections are there from the first frame with dashes
  for figures nothing has measured yet; the GPU section is absent only with no GPU at all,
  which is a fact of the run. A list has slots and fills them, padded where it is short:
  `CPU nodes` is its total and then five slots, the costliest nodes by name, `—` where fewer
  tick; Costliest Outputs is eight rows and the line that counts them, blank rows where there
  are fewer Outputs.
- **Nothing wraps.** Every line is one line: text longer than its column is cut with `…` and
  is whole on that line's hover — an Output's state after its `▸ go`, a node's status, the
  file line, the drops by cause.
- **Numbers keep columns of their own width**, right-aligned, and a figure that only grows is
  last on its line, so a value gaining a digit moves nothing after it.
- **Only a person changes a height**: folding a section, and *show all*.
- **Nodes is last**, because how many nodes report a line is the one count that truly varies,
  and a row it gains or loses moves nothing but the bottom edge, which is pinned.

**One model, two renderings.** `status::build` turns the view into lines of styled spans;
`status::show` draws them, a run of spans with nothing to click as one label in as many colors
as it has, and **`◰ copy`** puts `status::copy_text` on the clipboard — the same lines at the
same width with every section unfolded and every Output listed. What is pasted is what was on
screen, opened out.

- **The verdict.** The top edge carries the synth's rate against the rate it is asked to tick
  at — ticks a second over the last second against the *Tick rate* preference, the display's
  own by default, `17 Hz of 100` — in the accent once it is five per cent short. The rate is
  the timed tick's own mean interval, top of one to top of the next, which nothing clamps, so
  a synth at 4 Hz reads 4; it is a dash until the box has timed a whole tick. So a synth
  held to 60 Hz on a 144 Hz panel reads `60 Hz of 60` and is keeping up. Under it the **GPU**
  — the synth's draw, first command to last — and the **CPU** — the synth thread's *work* per
  tick, the waiting and the sleep left out — each a bar and a figure against the budget, which
  is one interval of that tick rate. The bars share a scale, and a bar's cells past the budget
  are in the accent. The larger is named first: *held back by* while the synth is short of its
  rate, *busiest* while it keeps up. With no GPU reading there is no GPU bar, and the CPU is
  what it names. A Mac has no reading of the whole draw — Metal times no empty pass, and a
  phase's marks are empty passes — so there the GPU's figure is what the Outputs drawing cost,
  each its own timed pass, and the row's hover says so. Then **pacing**: *even* while the ninety-ninth percentile of the last six
  hundred ticks stays within half again the median, *uneven* in the accent past it — the
  spread and not the mean, because a mean hides a stall — with the p99 and the frames dropped
  since the run began. The ticks are those timed since the box opened, so a stall before it
  does not stand in the spread after, and a dash stands for both until one has been. Under
  it, **GPU waits**: the share of the last second's ticks whose draw found the tick two before it
  still on the GPU and waited for its last submission — the `TICKS_IN_FLIGHT` bound on the
  GPU's queue. It is not the whole of the wait: [the throttle](rendering.md#one-device) holds
  each of the synth's submissions until at most one of its earlier ones is still on the GPU,
  inside the draw, so the row can read 0% while the tick waits, and `waiting on GPU` in the CPU
  section is the honest figure. A ring with no free
  target can still skip one Output, and its drop appears by cause below. The hover says so and
  counts the GPU waits since the run began. Then every drop **by cause**, of which there are two
  and both are rare: `held`, a ring with no free target, and `capture`, a
  render's wait for its frame that ran out. See
  [rendering.md](rendering.md#draws-and-skips) and
  [proposals/pacing.md](../proposals/pacing.md).
- **GPU per tick** — the draw by part, each a bar against the section's total: `uploads` (the
  YUV passes with them), `sims` (every simulation's compute passes), `outputs` (the whole
  Output loop, with how many Outputs the tick drew of how many there are — the rest are
  [idle](rendering.md#which-outputs-draw)), `probes`, `mix`, and `between`, the draw less
  those five. A mark is written after the work recorded before it, so a simulation's
  dispatches read in `sims` — [rendering.md](rendering.md#a-tick-by-phase) has the mechanism.
  Then `whole process`, the render engine
  the whole process used in the last second — the editor's painting and every picture window
  included, which the synth's own marks cannot see — in the accent from ninety per cent. A Mac
  counts only the whole GPU, so there the row is `whole GPU`, its busy share `· every app`,
  this one's with the rest.
- **CPU per tick** — the tick from the top of one to the top of the next, split so that
  **waiting is not work**. Every phase is timed on the synth thread's own CPU clock
  (`CLOCK_THREAD_CPUTIME_ID`), and only the tick and its sleep on the wall, so what the wall
  saw and the CPU did not — the thread blocked in the driver with the GPU's queue full, almost
  all of it — is one row, `waiting on GPU`, at the top. Every other row is work: `editor
  messages`, `MIDI`, `CPU nodes` with its five slots under it — the costliest nodes by name,
  each on the same CPU clock as the row above them and its figure in the figure's column —
  `frame job`, `draw submission` (the draw's own recording and submitting, as one row), `publishing`, `tap
  readbacks`, `snapshot`, `everything else`, and then `idle`, asleep to the deadline. A work
  row past the budget is in the accent. **They add up to the tick** — the waiting is the tick
  less the sleep less every work, not a clock of its own — so a GPU-bound synth reads as a long
  `waiting on GPU` over a short CPU, and a CPU-bound one the other way round. See
  [rendering.md](rendering.md#a-tick-by-phase).
- **Editor** — the editor's own redraw off egui's clock, this frame, a one-second mean and the
  worst of two, and the CPU time its pass took. Only how often the canvas is painted: a window
  that is occluded or minimized moves these and nothing above, and that is not a fault. Then
  **GPU per frame**, what the editor's painting cost the GPU — every panel, window and picture
  blit egui paints, this frame, the mean and the worst of two seconds — from two timestamps
  in egui_wgpu's own submission, one from the frame's first paint callback and one from its
  last, read frames later without waiting (see
  [rendering.md](rendering.md#the-editors-own-painting)). Dashes until the first reading lands;
  with no GPU, or a device that cannot write a timestamp inside a render pass, the row is not
  there, as the GPU section is not.
- **Costliest Outputs** — every Output by the GPU's own time for the last frame it drew: the
  ones drawing this tick costliest first, then the idle ones by their last figure, then those
  nothing measured. The top eight, and then *…35 more under 1 ms* — under the costliest of the
  rest that still draws — with **show all** beside it. An Output is named by **what it shows** — the label of the node on the cable into its
  picture, since every Output's header says `Output` — and by its workspace, and a state
  follows its `▸ go` on the same line where there is one: *linking a new shader…*, *nothing
  connected*, *suspended*, *idle*, *shader error: …*, *diagnostic: …*, the frames it drops a
  second. An [idle](rendering.md#which-outputs-draw) Output keeps its figure from the last
  frame it drew, muted and left out of the heading's total, which is what the tick draws; a
  drawing one's hover says why it draws — on the tab being looked at, on a deck, in a window,
  captured, feeding back, sampled by another Output, or sampled by a tap whose reading
  something live or stateful reads — and an idle one's that nothing does. An Output nothing measured shows a
  dash, never a zero that would read as an idle GPU. Fewer than eight Outputs leave blank
  rows, and the last line counts them.
- **Project** — the nodes in the project and how many the canvas drew, shaders and uniforms,
  the undo depth, the ticks, the uptime, and what the last save or load did: the `file` row,
  the status line, in the accent where it says something failed. Where that line is about a
  file just written — a Snap's picture, a finished render's film or folder — it ends in
  **▸ show**, which opens the folder holding it
  with the file selected where the file manager can (`platform::files::reveal_file`), as
  the toast that said it first does with its **Show**.
- **Nodes**, last — each CPU node's own line, an error, a status or what it reports, grouped
  under its kind, one line each and cut to it. A clip's line ends with how its frames
  arrive: `bytes`, or `Zero copy` where the renderer samples the decoder's own buffer — a
  DMA-BUF on Linux and an `IOSurface` on a Mac, under the one name on both.

**A section folds** on its heading, `▾` open and `▸` folded, and folded it is one line with its
summary in the heading rule — `▸ Editor  11 ms · 90 fps`. The verdict, the GPU, the CPU and
the Outputs start open; the Editor, the Nodes and the Project start folded. What is folded, and
whether every Output is listed, is the preference `status_folds`, so it is how the box opens
next time. Each heading's fold is a button named for the tree as `Nodes section`.

**A hover carries what the line cannot.** A section's heading says what the section measures
and against what. A line cut to its column is whole on its hover, a CPU node's name cut to its
slot too. An Output's row has its detail — its resolution, the worst of two seconds, why it
draws — and a node's line its whole text; the pacing row has its window, its median and the
drops a second, the waits row what the tick does when the GPU is short and the waits since the
run began, and the editor's frame the
display's own rate. No other line has a hover.

**A node's name is a link.** An Output's name and its `▸ go`, and a CPU node's name, show that
node — its workspace if that is elsewhere, centerd on it — the jump the mixer's workspace link
makes. The `▸ go` is named `go to output22` for a test and the agent-driven layer, a CPU
node's name `go to ratiogear3`.

**All of it is measured only while the box is open**, and forgotten when it closes: no lap
reads a clock, the renderer places no mark and the editor's frame carries no timing callback. Closing the window is the same as unticking
the entry; both are the one preference. It is a window rather than a panel so the mixer panel
is the mixer and the canvas is the canvas: a thing someone opens, not a strip everyone pays
for. There is no status line elsewhere.

## The cost view

View → Costs draws a slim strip under every node on the canvas.

**Two anchors, not one line.** The strip is the node's width, and what it says is written
from both ends: a left half against the left edge and a right half against the right, with
the space between them opening as the node widens. Neither half needs a separator to keep
clear of the other, and the right half is **empty where there is nothing to say**.

```
×9 taps                          9/px 8.3M     the node that samples its input nine times
                                   1/px 922k   the node that is sampled
8.3 / 10.8 ms                                  an Output
14.2 / 18.1 ms                      3 drop/s   an Output that is dropping
```

**An Output says its last frame and its worst, and nothing about the budget.** The budget is
the strip: the bar behind the text is that Output's share of a vsync interval, so its full
width *is* the target and writing the figure out would draw the same fact twice. The bar turns the accent color once it is over, or once it is dropping frames at
all — and the drop count, which appears only when there are drops, takes the accent with it.
The drops are the last whole second's, from the renderer's running count, so a stall shows
for the second after it.

**Evaluations per pixel is a count and is written as one**, `1/px` rather than `1.0/px`. It
is a ratio underneath — the evaluations over the pixels of every Output the node reaches — so
a node feeding two Outputs of different sizes at different tap counts lands between two
integers and is rounded to the nearer. The strip is a badge, and a badge that reads `6.5/px`
is answering a question nobody asked.

The strip hangs off the body rather than being a row of it, painted after every node so
nothing covers it, so turning it on moves nothing. Each strip is a named widget joining both
halves with the one space they are read with — `checkerboard1 cost 9/px 8.3M` — so a test
reads it as text. Where the numbers come from is
[rendering.md](rendering.md#the-cost-probe); the toggle is a preference, like the
frame-pacing readout, because it is about how the person looks at a graph and not about the
graph.

## The two side panels

The editor is three columns: the **Main Input** on the left, the canvas in the middle, the
**Main Mixer** on the right. Both side panels **fold to a spine** — a strip
`ui::panel::SPINE` wide with the panel's name written up it, the whole strip being the button
that unfolds it, so a folded panel is never a mystery and never needs a menu item to find
again. Unfolded, the **whole header bar** is the button that folds it back, which is the same
gesture at the same size in both directions; it lights under the pointer to say so. The arrow
on its outer edge points at the edge the panel goes to rather than being the one place the
click lands.

**Three levels of type, and one label column.** The panel's name is in the strong face (Space
Grotesk SemiBold) a step up, on a band of its own; a section's heading is the strong face at the text
size; everything under it is the text face, a row's label in the secondary ink.
`panel::row` puts every label in a column `LABEL_WIDTH` wide and every row at an `s-number`'s
height, so the controls of a panel start at one x and a select and a number keep one rhythm.

**An empty picture says so inside its box**, in the muted ink — *No video source*, *No Output
assigned* — rather than on a line of its own under the black, which would be a second row
saying what the first one shows. The box keeps its size either way, so nothing moves when a
source comes or goes.

Each fold is a preference, not project data: whether you can see a panel is about the editor
in front of you, and a project carried to a smaller screen should not unfold two panels on it.
The Main Input starts **folded** where the Mixer starts open, because a new project has no
source chosen and an empty panel is not worth three hundred points of canvas.

**A folded panel is not resizable.** `exact_size` pins the width a panel may take but leaves
it resizable, which is egui's default — so a spine's edge lights under the pointer and offers
a resize cursor for a drag that can move nothing. Both spines pass `resizable(false)`.

**Each open panel keeps its width**, `main_input_width` and `mixer_width` in
`preferences.json`, and opens at it next run — 300 and 380 until one is dragged. Only a drag
of its edge writes it, on the release: egui keeps the width within a run and nothing across
runs, since eframe's own persistence is off, and a window too narrow for the panel squeezes it
without losing the width it is kept at.

Each state has **its own panel id** — `main-input` and `main-input-spine`. egui remembers a
panel's width by id and consults `default_size` only the first time it sees one, so a single
id used both ways comes back from its folded turn remembering the spine's width.

## The Main Mixer panel

The right panel is silvia's Main Mixer, drawn by `ui/mixer.rs` over
[the mixer](rendering.md#the-mixer): **Channel A** and **Channel B**, each a live picture of
the Output on it — a slot the panel reserves and `App` fills with a paint callback, exactly
as a node's on-body render is filled — with the name of the workspace it lives on at the far
end of the channel's heading, which is a link to that workspace centerd on that Output; or,
in the box, *No Output assigned*, named `Channel A: no Output assigned` for the tree. Then
**Mix**: the **Crossfade** method first, above the fade it governs, then the balance as an
`s-number` from −1 to +1, `A` and `B` at its two ends and no caption of its own — the tree
names it `A / B balance`, and a binding's dot stands after the *Mix* heading — because the
fade is a control and gets what every control gets — `Alt` + click binds it to a MIDI knob,
see [the MIDI window](#the-midi-window) — then **Blackout** and **Freeze**.

**Blackout and Freeze are two presses side by side under the fade**, each half the panel and
an `s-number`'s height. **Blackout** takes the mix to black until it is pressed again;
**Freeze** shows the mix's last frame again and again until it is pressed again, while the
decks go on underneath. Both held is black, and letting Blackout go shows the frozen frame.
Each holds **wherever the mix is shown** — this panel's preview, the mix behind the canvas, a
picture window of the mix, NDI and Syphon — because the audience sees the show through any of
them ([rendering.md](rendering.md#the-mixer)). A held press is lit: the accent fills it and a
`●` stands before its name, the glyph and the accent the palette says state with, at a fixed
width so lighting moves nothing. Each is a named `Response`, *Blackout* and *Freeze*, selected
while it holds. They are rig controls, as the fade is: not saved, not undo steps, off at every
launch and on every Open and New, and `Alt` + click binds one to a MIDI note — it learns, wears
the learning ring and then the dot on its top-left corner, and does not press on the way. Then **Projection**: the mix's
resolution, the [resolution picker](#options-and-the-file-button) with two entries above its
strip — *Match display*, the default, which names the smallest display's size beside it
([rendering.md](rendering.md#the-mixer)), and *Match viewport* — *Project to background*,
**Window** — *Pop out* and *Fullscreen*, the two marks every picture carries, worded here since
the panel has the room — and **Send** — *NDI®*, and *Syphon* on a Mac — each lit while what it
asks for holds, and the mix itself, the picture those rows project, with no rule over it. The status line is not here: it is in the Status box, and it still
describes the selected Output.

**`mixer::show` draws and returns `MixerOutput`; it never mutates**, the same shape as the
menu and the tab bar. Each gesture is a `MixerAction`, and `App` answers them by writing the
mixer directly — none is a command, because [a claim is not an
edit](decisions.md#the-mixer-is-a-render-target-with-two-decks-not-a-node) and neither is
the fade. `Show on A` and `Show on B` are not here: they are on the Output node, as action
inputs that are also buttons, so the panel shows what is on the decks and the node is where
something is put on one.

## The Main Input panel

The left panel is silvia's Main Input, drawn by `ui/maininput.rs` over
[the Main Input](media.md#the-main-input): one **video source** and one **audio source** for
the whole rig, read by any number of `maininput` nodes.

**Video** is *None*, *Camera*, *Screen or window*, *Syphon* on a Mac, *NDI®* or *Video file*, with a 16:9 picture
under it — a slot the panel reserves and `App` fills, exactly as a deck's is, reading *No video
source* while the source is *None* — and, while there is a source, a line saying what is
actually happening: `camera, 640x480`, `preparing clip… 40%`, `waiting for the
screen picker…`, `camera, no frame yet`. A source that has delivered nothing for ten seconds
since it opened, a clip's transcode and the picker's wait not counted, reads `not responding`
instead, and so does a camera that stops for ten seconds after its first frame
(`video::silence`); a screen with nothing moving on it and a clip on a paused playhead deliver
nothing new and are not stopped. A camera gets a device list with a **Test pattern** on the end, which is
GStreamer's color bars and the one source that is always there, so a black picture means the
patch rather than the hardware. *Syphon* gets a menu of the servers running on the Mac by
"App – Server", the first one chosen when the source is, and **Flip** and **Transparent**
ticks under it ([media.md](media.md#syphon)); its line names the server, or says none is
running. On a machine without Syphon the list does not offer it, and a project that chose it
on a Mac reads *Syphon (macOS only)* with nothing under it. *NDI* gets a menu of the sources on
the network by the name NDI gives each,
`MACHINE (Stream)` — nothing is chosen with the source, since the network is listed only from
then on — and a **Transparent** tick ([media.md](media.md#ndi)); where the NDI runtime is
missing, the panel says so under the menu.

**Audio** is *None*, **System audio (what you hear)**, *Microphone or line in*, *Video
source*, any named capture device the machine has, or a sound file. *Video source* is the
soundtrack of whatever the video source is, so it follows a clip changed under it and is
silent over a camera or a screen. Monitors in that list are marked `↺`. Under it, while there
is a source, its line, and then **Gain** and **Monitor** as `s-number`s.

**A file is picked the way a node's file button picks one.** Choosing *Video file* or *Sound
file…*, or clicking the file's own button, puts up the same picker a `video` node's button
opens — the project's clips or sounds as cards with their posters, and *Import a file…* as the
last entry — so a clip already in `assets/` is a click and not a walk through a dialog to a
folder the project owns. Where the project has nothing of that kind yet the dialog opens
directly. Which picker is up is `App`'s (`main_input_picker`), like the popup open on the
canvas, so the panel stays a draw-and-return; a choice is `SetVideo` or `SetAudio` as before.

**Analyzer** is the scope every audio node has — the spectrum, a two-axis handle per band
saying where it listens and how narrowly, and the threshold that fires its event sitting on
the meter it is compared against. It is here rather than on the node because **the tuning and
the thresholds are the input's**: there is one capture, the bands are measured and the
thresholds crossed on the audio thread inside it, so a per-node copy would mean whichever node
ticked last decided what every reader saw. `ui::scope` takes an `Owner` for this — a salt for
its egui ids and a name for its handles — so the identical picture serves a node and a panel.

**The meters are drawn on the `maininput` node as well**, `widgets::scope::METERS` over
`ui::scope::meters` — the same three bars at the same pitch, with the same threshold square in
the action port's own color, because silvia draws them on its node and a level is set while
the band it measures is being watched. There they are `Hands::Off`: the square says where the
panel has the threshold and does not offer to move it, since one capture has one answer, and
having no interact of its own it lets the drag through to the node's body. No spectrum and no
band handles go with them — those are about tuning, and this node's tuning is the panel's.

silvia's **demo video** is not offered; it was a file bundled with a web page that had nothing
else to show. Nor is its **Gain / Expand / Smooth** grid: shaping a band is the graph's job
here. See [decisions.md](decisions.md).

**`maininput::show` draws and returns**, the same shape as the mixer panel. Each gesture is a
`MainInputAction` and `App` answers by writing the Main Input directly; none is a command,
because pointing the rig at a camera is not an edit to the patch. **It never enumerates
devices while drawing** — that is once a frame, and a `DeviceMonitor` per frame is sixty a
second — so both device lists are handed in already gathered, empty until the machine has
answered on a thread of its own ([media.md](media.md#the-main-input)), and refreshed only when
*Look for devices again* throws them away. It is at the foot of every menu that lists the
machine's devices: the panel's audio menu, its camera menu, and the Camera node's Device
select, whose `OptionDef::devices` says so and whose list sends the same request back as
`Effects::look_for_devices`. The NDI and Syphon menus have none: their sources are listed
continuously, and nothing asked again would come sooner. The Syphon servers are handed in too, from a listing read at
most twice a second, and the NDI sources, from the provider's listing, while the source is *NDI*.

## Project to background, `H` and `F`

**Project to background**, a checkbox on the panel, paints the mix behind the
canvas — *covering* it, cropped rather than barred, because a background is a background —
with the nodes and cables floating on the show. silvia's single most striking thing, and
here it is the canvas reserving one slot under everything instead of painting its ground
and its dot grid, and `App` filling the slot with a paint callback at `Fit::Cover`.

It is **off at every launch and written to no file.** It is a way of looking at the patch you
are editing right now, not a property of the show: opening a project to find the canvas
already covered by the mix hides the very graph the project was opened to work on. `H` is the
same idea at full strength and is not remembered either.

**`H` hides the editor**: the central panel shows nothing but the mix, and a toast says
*Editor hidden — press H to show* for four seconds, in [the toast](#the-toast). The menu, the tabs and the Main Mixer
stay, as silvia's side panels do, so the fade and the sources are still to hand. **`F` is fullscreen** for
the editor window; in a picture window it is that window's own. View ▸ Hide editor and View ▸
Fullscreen are the same two, printed with their keys, and read *Show editor* and *Leave
fullscreen* while they hold.

Both are bare keys, and a bare key is read only while **no text field has the keyboard** —
`H` in the quake bar's search is a letter — and consumed with `Modifiers::NONE` as the
pattern, which is what rejects a press with `Ctrl` held. They are consumed at the top of
the frame with the other shortcuts, before any widget reads input. A hidden canvas reports
no held buttons, so a press button under a finger when `H` was pressed is let go.

## Preview and on-node render

An Output draws its own frame on the node, 216x122, flush to the bottom corners. The preview
panel shows **the mix** — what the audience sees, whatever is selected; see
[rendering.md](rendering.md#the-mixer). Both are paint callbacks blitting a texture,
letterboxed — see [rendering.md](rendering.md) for why not a registered texture. The status
line above the preview still describes the selected Output, because a shader error or a
compiler diagnostic belongs to a graph and the mix has none.

An Output's frame lies with its bottom row first and egui images are y-down. The flip
happens once, in the blit.

**The picture sits inside the border.** The node's screen is its own region at the foot of
the body, black before the first frame and rounded at the bottom with the body; the picture goes into a slot `ui/`
reserves *between* that ground and the border, so the border — and any node drawn later — is
painted over it rather than under it. The picture *is* the slot — flush with the body's sides
and bottom, as silvia's `.output-canvas` is — because the blit rounds its own two bottom
corners rather than the picture being held off the arc: `node_widget::corner_radius` is the
radius, and it travels to `render/` on `Thumbnail::corner`. See
[rendering.md](rendering.md) for how the blit rounds them. `App` fills the slot; the geometry
is `ui/`'s.

**A picture at the edge of the view is clipped, never shrunk.** The rect handed back is the
node's own, whether or not the node is on screen, and the clip rect is what cuts it — an
Output half off the canvas loses half its picture exactly as it loses half its body. What has
to be guarded is that the letterbox is computed against the whole rect: egui's
`PaintCallbackInfo::viewport_in_pixels` clamps to the window, and a blit given the clamped
rect fits the whole picture into the part still visible.

**A source can draw its own picture too.** A node kind that declares a
`nodes::Region::Preview(port)` region — `video`'s `frame`, `imagegif`'s `output`, and the
two CPU simulations whose state is the thing worth watching, `cellularautomata`'s `cells` and
`brickgame`'s `field` — draws it under a **Preview** heading, and while that heading is open the region
is a fixed 16:9 box of the body's width, black before the first frame, letterboxed because a
clip's aspect is whatever was imported and the band is laid out long before a frame has been
decoded. **The picture takes the foot of the body** — flush and rounded, exactly as an
Output's is — because the preview region comes after the scope in the node's own order, so a
node with both draws the meters directly above the picture: the picture is the thing being
looked at, and the meters are instrumentation over it.
**A node with something to say says it on its picture.** Where `CpuNode::status` is filled —
a clip being transcoded or failing to, a GIF being counted — the region paints that line across the middle
of the band, after the picture's slot is reserved so it stays legible, and registers hover
only so a hand still carries the node by its picture. It is the black band the eye is already
on, rather than a row under the rows or the bar behind a file button at the other end of the
node. `maininput` is **not** in the list: the Main Input panel is holding the rig's picture a
few inches to the left, so the node draws [the rig's meters](#the-main-input-panel) in that
band instead.

The texture is the same one the port hands downstream, which the renderer keys by **port**
rather than by node, so a `Thumbnail` names both (`Thumbnail::port`, `None` for an Output's
own render) and `Renderer::blit_source` is `blit_node`'s other half. It blits flipped: a
decoded frame is uploaded top row first where a framebuffer is filled bottom row first, which
is the same flip `video`'s own generator does in WGSL.

## The strip on a picture

A picture carries the controls every video player has put along its bottom edge, on the same
hover rule as the marks in the corner above them: a **scrubber**, a **speaker** that mutes,
and a short **volume** slider, on a scrim dark enough to read over a white frame. The shape
is conventional on purpose — a hand that has used a player knows what a filled bar and a
speaker do, and the new ideas on these nodes are the ports.

**Neither control is a value of its own.** Volume *is* the `monitor` input's control, so the
strip is a second editor for the address the number row on the node already has — the same
relationship a meter's threshold square has to its level — and it goes through
`Command::SetControl`, into the undo history, with the row moving as it does. A cable into
`monitor` takes the speaker away entirely, because a control something else is answering is
not one a hand may drag.

**Where a clip is playing from is not a control of its own.** It is Time plus Offset, in
plays of the clip, and the node keeps no position beside them, so a drag on the scrubber
goes back the other way as a *seek*: `Effects::seeks` → `App::seeks` → `TickContext::seek`,
read by the next tick and cleared, one-shot where `held` is a level. `video` moves its
Offset so the sum lands where the hand let go, and Time plays on from there,
which is what makes it a scrubber rather than a transport. Where the bar gets its reading is
the other half: `CpuNode::playhead`, gathered per frame beside the scopes, so `ui/` never
reaches into a node's state. A cable into Offset makes the bar a **readout** — dimmer, no
head, no drag — because the node would put the clip back on the next tick, and a control
that cannot win should not invite a hand.

The strip is drawn in one place — the picture on the node — and reads
`node_widget::playing` for what it shows. A picture window carries none; see below for why.

## The gear region

A Master Gear and a Ratio Gear each carry one region under their rows, `Region::Gear`
(`widgets/gear.rs`): a picture of what the gear is set to, turning at its real rate, with
the words beside it and, on a Master Gear, a caption under it. It is **92 points tall
whichever picture it draws**, so changing the picture, the Teeth or a caption arriving moves
nothing. It declares no heading and claims no pointer: a hand carries the node by it.

**The Display option picks the picture**, on both gears, and **Rosette** is the default.

| Display | a Ratio Gear at Teeth `p : q`, in lowest terms | a Master Gear |
| --- | --- | --- |
| **Rosette** | a still spirograph that turns once round an input cycle, so it winds `q` loops, the input cycles it takes to close, and waves in and out `p` times across them, the output's cycles, its petals — ×3 three petals in one loop, ÷4 one petal wound over four loops; a tick at the top where every loop begins; and a dot riding the curve at the output's phase | a ring of a clock face's twelve ticks, the first one long, and the dot |
| **Gears** | the input's gear of `k·p` teeth driving the output's of `k·q`, `k` the smallest that puts both at six or more; where that puts one past forty-eight, two gears of twelve with the ratio printed on the output's hub | one gear of twelve teeth, turning once a cycle |

**Both pictures turn by the gear's own phases**, never an animation clock, so a paused show
is still and a seek moves them at once.

**Beside the picture, the words.** A Master Gear's: its cycle in seconds, `2.000 s`, over
`a cycle`; and `held` while Hold is on. A Ratio Gear's: its ratio in lowest terms, `×3`,
`÷4` or `3/2`; what it closes in, `every cycle in` or `every 3 cycles in`; and the picture's
own count, `2 petals · 3 loops`, or under Gears the Teeth as the row has them, typed and
unreduced, `2 : 3` or `4 : 6` — not the teeth the wheels are drawn with, which are scaled up
to look like gears. The words do not say the direction, which the switch over them does; in
Reverse the rosette's dot rides its curve the other way and the output's gear turns against
the input's.

**Under a Master Gear, its caption**: what a loop of it needs, read from every node downstream
of it (`nodes::chain::caption`). The chains' ratios multiply down each chain, each node comes
back after its own period on what drives it — a noise's Repeat, a tunnel's Helix, a sequencer's lanes' figures, a
Trigger's beats, a node on its own clock beside it — and a loop is as many of the master's
cycles as the least common multiple of what they ask for — `loops in 1 cycle · 2.000 s`, or
`loops in 4 cycles · 8.000 s (÷4 on ratiogear12)` or `(÷4 on perlin5)`, naming the gear or node
that asks for the most — or `2 nodes will not close (perlin5)` where a node never repeats or
a count runs into a Speed, or `can't tell when 1 node closes (multiply3)` where a Speed is
cabled, a Math node stands between a gear and a Time,
or a CPU node keeps state of its own, naming the first such node
([nodes.md](nodes.md#when-a-loop-closes)). The node has no room for the reason, so the line is drawn up to
its ` (` and **the whole of it is on the hover**. It is what a loop of the patch is: an
Output rendered [from its Render section](#the-render-section-and-the-band-across-the-editor)
for that long, from zero, comes back to its first frame.

**Painted text is not in the accessibility tree, so the region says what it shows**: a label
named `{slug}{id}.gear` and its words joined with ` · ` — `ratiogear4.gear ×3 · every cycle
in · 3 petals · 1 loop` — and on a Master Gear a second, `{slug}{id}.loop` and the whole
caption, `mastergear1.loop loops in 4 cycles · 8.000 s (÷4 on ratiogear12)`.

### The Teeth row

A Ratio Gear's **Teeth** are one region above its picture, `Region::Teeth`
(`widgets::gear::TEETH`), on a lone input row's slab, short of the far edge with its outer
corners round, and two lines tall: the Teeth on a line the height of a port row's, and under
them the gear's direction. On the first, the word `Teeth` where a row's label stands, then
`p`, a colon and `q`, each the
inset s-number at 60 points rather than 100, so two and the colon fit where one control and
its label do and each keeps its steppers. They are the node's own hidden controls, with no
port, drawn by `RegionUi::number`, so each answers every gesture a number on a row does —
drag, steppers, arrows, wheel, a typed value, `D`, `R`, the range editor and a MIDI binding —
and steps by one from 1 to 64, so a drag and a typed `2.5` land on whole numbers and a typed
`0` on 1. They show the Teeth as typed: 2 : 4 stays 2 : 4, and the picture beside draws ÷2.
**Under `p : q`, its direction**: Forward | Reverse, as wide as the two numbers and the colon,
four points below them and as far above the slab's foot as the numbers are below its top. It
is the segmented switch the Timing heading's Free | Loop is (`widgets::segments`), the press
button's chrome split by a hairline, the lit segment `bg_active` and each name in the
heading's tiny type, and it writes the `direction` option, an option the region draws
(`OptionDef::in_region`); a click on the lit segment is nothing. The row is on the Teeth's slab
rather than a row of its own because it is the Teeth's: which way the gear turns its `p` turns.
It does not fit on the Teeth's line, which two numbers, a colon and the word already fill at
the node's width. Both lines are always drawn, so switching moves nothing.
The word and the colon carry a hover, `Turns 3 times for every 2 turns of its parent.`, and
in Reverse `…of its parent, backwards.`. The row's accessible name is
`ratiogear4.teeth 3 : 2`, with ` in reverse` after it in Reverse; each number's is its own,
`ratiogear4.p 3`, and each segment a radio button, `ratiogear4.direction.forward` and
`ratiogear4.direction.reverse`.

## The XY Pad

`xypad`'s body ends in silvia's own custom area, `widgets::xypad`: a square at the body's
width inside silvia's half-rem padding, its X and Y under it, a bar of nine numbered presets
and a line of help. The node asks for the audio scope's 300, so the square is as large as any
picture on a node. On the square, in the editor's tokens: silvia's ground, dashed center lines
and border; each gravity well as a ring as wide as it is strong with a dot and a cross, in the
event hue silvia's amber stands for, and each tether as its dashed circle, a dot and the string
to the puck, in the main hue; the trail of the last 500 steps fading in, in the uniform
number's hue at silvia's 60%; a line for the velocity; while a slingshot is drawn back, the
band, a dashed line where it will go and a dot where it will snap back to, in the color port's
hue standing in for silvia's red; and the puck, with a halo while a hand has it. Over the
square the pointer is silvia's crosshair, and a closed hand while it holds the puck.

**The square claims the pointer**, so a press there never carries the node. The primary
button puts the puck under the hand and holds it there; in **Slingshot**, silvia's default, a
drag pulls it back and the release snaps it to where it was pressed and launches it the other
way at five times the pull, and in **Cursor** a drag carries it, clamped to the square, and the
release lets it fly on at the speed of the last move. Each of those is a `SetControls` of the
node's four hand controls — Pad X, Pad Y and the velocity — the same four keys in the same
order every frame, so a whole throw is one step back. While the button is down the region
reports the hand as `RegionEvent::Held` under the key `puck`, a level in `Effects::held` as a
finger on an action input's button is, and the tick keeps the puck still. The secondary button
on a well takes it away; anywhere else it pulls a new one out of where it was pressed, a
gravity well three times as strong as the drag is long or a tether as long as the drag, by
**Place Mode**, and a click is silvia's default well. A well is the node's runtime state, so it
is not an edit: it goes as `RegionEvent::Touch`, `Effects::touches`, `TickContext::touches`,
one-shot as a seek is. What the square draws comes back the other way as `CpuNode::puck`,
gathered beside the curves; held, the puck is drawn where the hand has it this frame rather
than where the last tick left it.

**X and Y under the square are Pad X and Pad Y**, the inset s-number every number on a node
is, so each is typed, reset, ranged, learned by `Alt` + click and marked once bound, as a knob
on a row is. silvia's readouts show where the puck is; the X and Y rows do that here, and these
say where the hand put it. **A preset** writes its first edge, its ten numbers and its second
edge through the bus — one step back, since `history::joins` holds a node's `SetControls` and
its `SetOption`s together in one press — and sends its flight and its wells as a touch.

## Pictures in windows of their own

Every picture on a node carries two marks in its top-right corner, in a video player's order
— **pop out**, then **fullscreen** at the end. Pop out opens a window showing that picture
and nothing else; fullscreen opens the same window straight onto a screen. Both are lit while
what they ask for is true, so a picture on a display the hand cannot see still says here that
it is up, and the pop-out mark is a toggle: the click that opened the window puts it away.

**They are drawn only while the pointer is on the picture** — a player's rule, and the reason
is the picture: furniture that is always there is furniture on the render. The widget is
registered and *named* every frame regardless, before the drawing is decided, so `pop out
output1` and `fullscreen video1` are in the accessibility tree whether or not they are
painted; nothing invisible is in the way of a click either, since the corner cannot be
reached without being over the picture, which is what reveals them.

One function draws every one of them — `node_widget::picture_mark`, which owns the hover
plate, the hover-reveal rule, the pointing-hand cursor and the accessible name, over
`mark_rect`'s geometry. The Main Mixer panel's **Window** row calls it too, for the mix's own
pair: the two sets are meant to read as one family of mark, and a family is only one if there
is a single place that draws it.

**The window is a window of our own**, drawn on a thread of its own — on Linux a Wayland
surface, on macOS and Windows a winit window made inside eframe's event loop; see
[rendering.md](rendering.md#picture-windows), `proposals/picture-windows.md` and
`proposals/macos-windows.md`. It is not an egui viewport: eframe runs one winit event loop,
that loop services every viewport in turn, and a minimized editor runs no passes, so an
egui-hosted window stopped painting with the editor. A window paced by its own display cannot
observe what the editor is doing at all.

**There is no projector.** The mix is a picture like any other and wears the same pair of
marks, drawn in the Main Mixer panel's **Window** row because the panel has no picture of its
own to hang them on. *Open projector*, View ▸ Projector and the projector's own kind of
window are gone; what they did is the mix's pop-out. **A window of the mix shows Blackout and
Freeze** as every other way the mix is shown does — see [the Main Mixer
panel](#the-main-mixer-panel). While any picture window is open, Quit, Open and New ask
first ([the confirm](#the-menu-bar)); `Escape` closing one does not.

**No decorations**, so what is on screen is the picture: dragging the picture moves the window
(`xdg_toplevel.move`, with the press's own serial — a compositor refuses any other), and `F`
or a double-click is fullscreen. **On macOS, fullscreen covers the window's screen at once, in
place** — no animation and no Space of its own, the menu bar and the Dock hidden outright while
supersilvia is in front — and the window casts no shadow, as on Linux. On Windows it is a
borderless window the size of its monitor, over the taskbar.

**`K` keeps a window on top, on macOS and Windows**: AppKit's floating level or Win32's
topmost band, above every other app's windows whichever app is in front, kept through fullscreen, and let go by a second `K`. Linux
has no key for it. Wayland gives a client no way to raise its own window above others, and
KDE's own window menu, from the task bar entry's right-click, already has *Keep Above*.

**`Escape` undoes the last thing the hand did**, one step at a time, and closes the window
when there is nothing left to undo. A pop-out that was made fullscreen goes back to being
that pop-out, and a second `Escape` closes it. A window the **fullscreen mark** opened was
never a pop-out, so there is nothing behind it and one `Escape` closes it. The rule is
`render::picture::escaped`, a function of the two bits the window carries — whether it is
fullscreen now and whether it opened that way.

**Every edge and corner resizes it.** A band twelve logical points wide runs along each of the
four edges, and a corner is where two of them meet: a press in one that travels past the drag
slop is an `xdg_toplevel.resize` from that edge, and a press anywhere else is the move. Which
band a position is in is `render::picture::edge_at`, a pure function of the position, the
window's size and the band — so the nine regions are a table test with no compositor in it.
A **fullscreen** window has no bands, which it says by passing a band of zero: there is
nothing left to resize it to. The window keeps its minimum size, and a window narrower than
two bands still has two sides rather than one, because the band is never more than half of
it.

**On macOS and Windows the gesture is ours**, because winit on macOS has no resize at all and
Windows shares the Mac's windows. A press is
remembered, and once it has travelled the same four-point slop every motion sets the window's
frame from `render::picture::dragged`, recomputed from where the press landed: the middle moves
the window whole, and a band moves that edge or corner while the opposite side stays put, down
to the window's minimum. The move is ours too rather than AppKit's
`performWindowDragWithEvent:`, which Apple documents for a mouse-down, where calling it would
take the second press of a double-click with it. `dragged` is pure and tabled beside `edge_at`.

**Hold `Shift` or `Ctrl` and the resize keeps the picture's aspect.** A compositor's
`configure` is a *proposal* and a client may answer with a size of its own, which is the only
reason an aspect lock is possible: while a resize is running with either modifier down, the
answer is the largest size at the picture's own aspect that fits inside what was proposed,
anchored on the edge being dragged — a vertical edge is a width and the height follows it, a
horizontal edge is a height and the width follows, a corner is both and takes whichever fits.
That is `render::picture::fit_aspect`, pure and tabled, and the buffer committed at that size
is the answer. The modifier is read straight off `wl_keyboard.modifiers`' depressed mask,
whose bits are indexed by the *keymap's* own modifier order — `Shift` first and `Control`
third in every xkb keymap, by inheritance from the core X protocol, which is a convention and
not a guarantee. It is the same trade the two keys make: the alternative is an xkb keymap on
the pictures thread and the `xkbcommon` crate with it. The mask is cleared when keyboard
focus leaves, so a modifier held into another window does not lock a resize here. On macOS
and Windows there is no proposal to answer: `dragged` takes the same `fit_aspect` rule, anchored on the same
edge, and a locked size shrunk past the minimum grows back to it at its aspect. The modifiers
there are winit's, by name.

**The cursor says so before the press does**, which is the whole of the discoverability: over
a band it is the matching resize shape and everywhere else the ordinary arrow. It is set
through `cursor-shape-v1`, where the compositor names the shape and draws it — KWin does —
and falls back to loading the `wl_cursor` theme and attaching the image by hand where the
protocol is absent. Both halves are sctk's `ThemedPointer`, which is why `wl_shm` is bound on
that thread and used for nothing else. The shape is set on every pointer enter, as a Wayland
client must, and thereafter only when it changes. On macOS the shapes are AppKit's own
two-headed resize cursors, which winit names, set only when they change.

**And the window is still a bare picture**: nothing is painted over it to mark the corner. A
grip glyph was tried and taken out — the picture is the whole surface, and furniture on a
show is furniture on the show. The cursor is what says the window resizes, and it costs the
picture nothing. The window opens at the picture's own
size, capped to the output it lands on, and letterboxes on black. Which pictures are open is
app state, not saved: which display a rig is plugged into is decided on the night.

**A picture window is a bare picture, and that is the cost.** It carries no player strip and
no marks of its own. The strip — the scrubber, the speaker, the volume — is `ui/player.rs`, an
egui widget reporting to the command bus, and drawing it there would mean an egui context and
a font atlas on the pictures thread competing with the blit the window exists to do, or a
second scrubber drawn by hand on the GPU and kept in step with the first. The priority is the picture
staying live, so the strip stays where it is drawn in the editor: **a clip is scrubbed on its
node, and the window shows what that did.** The two marks go the same way and are less of a
loss, because `Escape` closes, `F` fullscreens, and the node's own marks still say what the
window is doing and still put it away — which is the affordance that mattered for a window on
a screen the hand cannot see.

**Wayland only.** On X11, or any session whose display handle is not Wayland's, no window
opens and the mark that asked says so in the toast. There is no second path: an X11 one would
be a whole windowing backend — input, decorations, surfaces — for a case this instrument
does not have. The editor itself is unaffected.

**What the marks show is what the window is doing**, not what was asked for. A mark sends an
ask to the pictures thread and the thread answers with the window's actual state, so a
fullscreen the hand pressed inside the window lights the node's mark too, and an ask a
compositor refused never lights anything. `render::picture::Wall` is that rule, and it is
tested with no compositor at all.

**The editor's bare keys stop at the editor's window.** `H` and `F` are read only while the
root viewport reports `focused`, because `F` in a picture window is that picture's fullscreen
and the editor taking it as well would fullscreen the very thing the picture was popped out
of. A picture window's keys never reach the editor at all now — it has a keyboard of its own —
but the gate stays, because it is also what stops `H` firing while another application has the
focus.

**Its two keys are read as evdev positions**, not as letters: reading them as letters would
mean an xkb keymap on the pictures thread, and the `xkbcommon` crate with it, for one
shortcut. `Escape` is the same key everywhere; `F` is the key where `F` sits on a QWERTY
board.

## Sending an Output out

**Under its Send heading, an Output has its name, then one row per way it can leave**:
**NDI®**, to other machines on the network, and **Syphon**, to other apps on the same Mac — on
a Mac alone; a machine without Syphon draws no Syphon row at all.

**The Name row** is the whole name the Output goes out under, every way: a label and a
one-line field from the status column to the row's end, holding `supersilvia Output <id>`
until the Output is given a name of its own, and then that — *warpzone*. The field is
`ui::text::line`, framed as a row's number field is, the text scrolling inside it rather than
wrapping. It commits on Enter or when the field is left, never a keystroke at a time, because
a commit restarts whatever is on air; Escape puts back what was there, and leaving it unchanged
is no edit. A commit is one `SetOption` of `sendName` and one undo step. Empty goes back to the
default, which is stored as nothing, so a copy of an Output that never had a name takes its own
number. The pointer on the field says so, and that renaming on air means receivers see the old
source go and a new one appear.

**A name is the Output's own across the project**, every workspace included, and the mix's
`supersilvia Mix` with them, told apart without regard to case, as the network tells them.
`nodes::output::unique_name` is the rule: trimmed, cut to 48 bytes (`NAME_BYTES` — NDI announces
`MACHINE (name)` as one DNS label of 63 bytes at most, and 48 leaves the machine room), then
` copy` appended until no other Output has it — *warpzone copy*, *warpzone copy copy* — and
where the next ` copy` would pass 48 bytes, the Output's default instead. It is applied where a
name arrives: a commit, fitted by the bus before it is stored (`nodes::fit_option`), so the undo
step holds the name it went out under; a paste, a duplicate or an import, where the newcomer
yields to the Output already there (`nodes::output::settle_names`); and a project opened with
two Outputs under one name, where the later by id takes the ` copy` in the graph alone, so the
project opens clean and the file changes only when it is saved.

Each way's row is its name, a dot and one line saying what it is doing, and one button at its
right:

| the line | the dot | the button |
| --- | --- | --- |
| *off* | `○` | **Send** |
| *on air · warpzone* — the name it goes out under | `●` | **Stop** |
| what the sender said went wrong, while it is on and failing | `○` | **Stop** |
| *runtime not installed*, NDI only, where there is no NDI® runtime | `○` | **Get it**, or **Stop** where a file switched it on |
| *runtime won't load*, NDI only, where an NDI® runtime is there and does not load | `○` | **Get it**, or **Stop** where a file switched it on |

**Send** and **Stop** set the option the way does — `ndi` or `syphon`, `on` or `off` — through
`Command::SetOption`, so a click is one undo step and the choice is saved with the project.
**Get it** opens the page the missing-runtime message names (`video::ndi::RUNTIME_URL`) through
egui's own link opening. The line is cut short where the row runs out, never wrapped, and the
pointer on it shows the whole: the error, the runtime message with where it looked — or, for a
runtime that will not load, its path and the loader's reason — or what the way does and the
name it goes out under. What the sender says comes from the publisher's thread
as a `render::publish::Failure`, read once a frame into the Output's `OutputReadout`.

**Flip** sits under the Syphon row, its label in the status column, while Syphon publishes:
*Bottom first* | *Top first*. **Alpha** comes once, after the rows, while either way sends:
*Opaque* | *Transparent*, which both ways read. Each is `press::choice`, the press button's
field cut into a segment per value, the chosen one raised; a click on the other is one
`SetOption`. The keys are the ticks' own, `syphonFlip` and `transparent`.

**Static height.** Which rows there are is the options' answer and the machine's, never the
sender's: a row appears when a hand switches something on, as a fold opens, and what the
publisher reports changes only what a line says. The rows are `canvas::Row::Send` under
`canvas::Row::SendHeading`, laid out from `nodes::output::send_rows`, and each is an option
row's height. The heading is closed on a new Output, as Render's is; its option is `send`.

A sent Output is on air — drawn, and everything upstream of it ticking, with nothing on screen
showing it and its tab closed — and the Status box says why: *it is sent out over Syphon or
NDI*, or *over NDI* on a machine without Syphon. A Mac's project opened on Linux keeps its
`syphon` and `syphonFlip` values, reads them as off (`nodes::output::sent_of`), and publishes
nothing.

## Syphon

On a Mac, a picture can be handed to another app through Syphon
([rendering.md](rendering.md#syphon)). Two places publish one.

**An Output's Syphon row** ([above](#sending-an-output-out)) publishes the Output with its
name as the server's name — `supersilvia Output <id>` until it has one of its own — and Syphon
puts the app's name beside it, which the framework takes from the process and does not let a
server set, so another app lists it as "supersilvia – supersilvia Output 3" or
"supersilvia – warpzone". A rename renames the running server in place. Flip writes it top row first rather than Syphon's own
bottom row first, for an app that reads it upside down; Alpha carries the picture's own alpha
rather than opaque over black, over Syphon and [over NDI](#ndi) alike.

**The mix's mark.** The Main Mixer panel's **Window** row carries a third mark after the
pop-out and fullscreen pair, drawn by the same `node_widget::picture_mark` — a dot with two arcs
spreading from it — and named *publish the mix over Syphon*. Clicking it publishes the mix as
`Mix`, and it is lit while it does; clicking it again stops. It is session state, as the mixer
is: **not saved**, so every run starts with the mix unpublished. The mix is published with the
defaults, bottom row first and opaque.

**Receiving** is the Main Input's *Syphon* source and the **Syphon** node, each with a menu of
the servers running and the same two ticks, Flip and Transparent, read the other way round:
how the surface another app publishes is laid out. See [media.md](media.md#syphon).

**A machine without Syphon shows none of it**: no Syphon row on an Output, no mark on the
Main Mixer, no *Syphon* in the Main Input's list and no `syphon` node in the Nodes menu or the
browser (`NodeDef::offered`). Each asks `platform::syphon::available`. A `syphon` node that
arrives in a Mac's project still loads and says there is no Syphon here.

## NDI

A picture can be sent to another machine on the network through **NDI®**
([rendering.md](rendering.md#ndi); [ndi.video](https://ndi.video)), on Linux as on a Mac. Two
places send one.

**An Output's NDI row** ([above](#sending-an-output-out)) sends the Output under its name —
`supersilvia Output <id>` until it has one of its own — which the network lists as
`MACHINE (supersilvia Output 3)` or `MACHINE (warpzone)`. A sender's name is fixed when it is
announced, so a rename on air announces a new sender and drops the old one, which leaves the
network. It goes with the
alpha its Alpha says — the one Syphon reads too; Flip is Syphon's alone, since NDI has one
orientation. Where the NDI runtime is missing the row says *runtime not installed*, its hover
says *The NDI® runtime is not installed — get the NDI 6 Runtime at ndi.link/NDIRedistV6Apple*
(on Linux, *… get it at ndi.video*) with where it looked, and **Get it** opens that page. Where
a runtime is there and will not load the row says *runtime won't load*, and its hover names the
file and why — *The NDI® runtime at /usr/local/lib/libndi.so.6 would not load:
libavahi-client.so.3: cannot open shared object file — reinstall it from ndi.video*. An NDI
option switched on in a file stays on and is saved, since the project may open on a machine
that has the runtime, and it sends nothing here.

**The mix's NDI mark**, last in the Main Mixer's **Window** row — three linked dots,
`node_widget::ndi_mark`, named *send the mix over NDI* — sends the mix as `supersilvia Mix`,
opaque, lit while it does. Session state, **not saved**, as the Syphon mark is. Where the
runtime is missing, its hover says so and a click does nothing. What it sends is the mix as
the audience sees it, Blackout and Freeze included, and the Syphon mark's is the same; while
either sends, Quit, Open and New ask first.

**Receiving** is the Main Input's *NDI* source and the **NDI** node, each with a menu of the
network's sources and a Transparent tick, read the other way round: whether a source's alpha is
kept. See [media.md](media.md#ndi).

NDI® is a registered trademark of Vizrt NDI AB.
