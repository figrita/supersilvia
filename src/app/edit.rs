// SPDX-License-Identifier: AGPL-3.0-or-later

//! An edit, from the app's side: the document applies it, and this tells every other part
//! what it did.
//!
//! [`Document::apply`](super::document::Document::apply) writes the graph and says what the
//! write means — an Output made stale, a node gone, a selection planted — and nothing more.
//! [`App::apply`] is the one place that hears it and hands each piece to the part that owns
//! it: the shaders, the canvas, the tabs, the synth and the MIDI barrier.

use super::App;
use super::document::{Clipboard, Env, Origin};
use crate::command::{Command, CommandError};
use crate::graph::{ControlValue, NodeId};
use crate::ui::ClipAction;

impl App {
    /// Apply one command. The only way anything mutates.
    pub fn apply(&mut self, cmd: Command) -> Result<(), CommandError> {
        self.apply_from(cmd, Origin::Hand)
    }

    /// Apply one command written by `origin`, which decides what holds open the gesture it
    /// writes into. See `document::history`.
    pub(super) fn apply_from(&mut self, cmd: Command, origin: Origin) -> Result<(), CommandError> {
        // The document is what the render is reading, and one rule closes it for every edit
        // — a node, a cable, a control, an option, an undo — rather than a disable per widget.
        if self.rendering() {
            return Err(CommandError::Rendering);
        }
        let env = Env {
            measured: self.canvas.measured(),
            project: &self.project,
        };
        let applied = self.doc.apply(cmd, &env, origin)?;
        if applied.nothing {
            return Ok(());
        }
        self.link.mark_stale(applied.stale);
        self.link.forget(&applied.removed);
        self.canvas.deselect(&applied.removed);
        if !applied.planted.is_empty() {
            self.canvas.select_only(&applied.planted);
        }
        if let Some((id, report)) = applied.imported {
            self.media.set_status(report);
            self.imported = Some(id);
        }
        // A paste from another project copied its media into `assets/`; a file it could not
        // find is a failure, said as every failure is.
        if let Some((said, failed)) = applied.brought {
            self.media.refresh_assets(&self.project);
            if failed {
                self.fail(said);
            } else {
                self.media.set_status(said);
            }
        }
        // A workspace that is gone has no tab. Session state, so it is set here rather than
        // being part of the step the command pushed.
        if let Some(id) = applied.closed {
            self.close_workspace(id);
        }
        // The command succeeded, so the synth's graph is behind by exactly this edit — when
        // it is an edit a tick could see at all.
        if applied.command.affects_tick() {
            self.publish_graph();
            // After the graph it went out on, which is what a MIDI write is barred against.
            // A knob's own write does not bar the knob.
            if origin == Origin::Hand {
                self.bar_midi(&applied.command);
            }
        }
        Ok(())
    }

    /// The pointer let go, so the next drag opens an undo step of its own — once the knobs
    /// are silent too: see `App::settle_midi_at`.
    ///
    /// Without this a gesture would end only when some *other* command interrupted it, and
    /// two scrubs of one control with a release between them would collapse into a single
    /// step that undid both.
    pub fn end_gesture(&mut self) {
        self.doc.end_gesture();
    }

    pub fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    /// How many steps the undo ring is holding, which is what the two caps bound.
    pub fn undo_len(&self) -> usize {
        self.doc.undo_len()
    }

    /// Step back one edit.
    pub fn undo(&mut self) -> bool {
        if !self.doc.undo() {
            return false;
        }
        self.after_time_travel();
        true
    }

    /// Step forward one edit.
    pub fn redo(&mut self) -> bool {
        if !self.doc.redo() {
            return false;
        }
        self.after_time_travel();
        true
    }

    /// Abandon the control scrub in flight, putting `value` back where the drag found it.
    ///
    /// `Escape` mid-drag is not a second edit. A scrub coalesces into one undo step, so
    /// setting the value back would leave a step in the ring that restores nothing and an
    /// edit serial the title reads as unsaved work. The step is dropped and its serial
    /// restored instead, which is exactly undoing the drag without offering it as a redo.
    ///
    /// The step goes whole, so a knob turned or a value the tick wrote while it was open goes
    /// with the scrub. And a knob turning across a release holds one step open over two
    /// scrubs, so that step may have begun before this one: the value the drag actually
    /// began at is re-applied afterwards, as an edit of its own.
    pub fn cancel_control_drag(&mut self, node: NodeId, key: &'static str, value: f32) -> bool {
        if !self.doc.cancel_scrub(node, key) {
            return false;
        }
        self.after_time_travel();
        if self.doc.graph().get(node).and_then(|n| n.controls.get(key))
            != Some(&ControlValue::Float(value))
        {
            let _ = self.apply(Command::SetControl {
                node,
                key,
                value: ControlValue::Float(value),
            });
        }
        self.end_gesture();
        true
    }

    /// Put back the node drag in flight: `Escape` with nodes in hand.
    ///
    /// The drag is one coalesced `MoveNodes` step, so it is dropped rather than answered
    /// with a move back, as [`Self::cancel_control_drag`] drops a scrub's: a step that
    /// restores nothing would sit in the ring and the title would read unsaved work. The
    /// nodes are in the order the drag moved them, which is the selection's.
    pub fn cancel_node_drag(&mut self, nodes: &[NodeId]) -> bool {
        if !self.doc.cancel_move(nodes) {
            return false;
        }
        self.after_time_travel();
        self.end_gesture();
        true
    }

    /// Has anything changed since the last save?
    pub fn dirty(&self) -> bool {
        self.doc.dirty()
    }

    /// The graph was replaced whole rather than edited, so nothing derived from it survives.
    ///
    /// Every Output recompiles rather than working out which changed: a restored graph may
    /// differ from the current one in any way at all, and one rebuild is cheap next to
    /// getting the answer wrong.
    pub(super) fn after_time_travel(&mut self) {
        // The tail every such path runs: an undo, a redo, a cancelled scrub, a project
        // opened or imported.
        self.publish_graph();
        // A project opened brings its own map, and an undo past a delete brings back the
        // node a binding names, which is what makes the binding live again: the map keeps a
        // binding whose node is gone and the synth is handed only the live ones. The tick
        // reads the map, so it crosses here with the graph it names.
        self.publish_midi();
        // A restored graph may have a different set of workspaces, so the tabs are checked
        // against it. Session state is not in the snapshot — undoing a workspace back into
        // existence brings it back closed, because opening one was never an edit.
        self.project.reconcile(self.doc.graph());
        self.link.rebuild_all(self.doc.graph());
        self.canvas.forget_graph_refs(self.doc.graph());
    }

    /// Whether a paste has anything to plant, which is what every Paste entry is enabled by.
    pub fn can_paste(&self) -> bool {
        !self.doc.clipboard().clip.is_empty()
    }

    /// What the clipboard holds, as a line of text, or `None` while it holds nothing.
    ///
    /// Put on the **system** clipboard by a copy or a cut. Two reasons, and the first is
    /// mechanical: `egui-winit` pushes an `Event::Paste` only when the system clipboard has
    /// text in it, so a node clip that lived purely in memory would leave `Ctrl+V`
    /// generating no event at all. The second is manners — pressing copy and finding the
    /// system clipboard untouched is a surprise, and what was taken is worth being able to
    /// paste into a message.
    pub fn clipboard_summary(&self) -> Option<String> {
        let nodes = &self.doc.clipboard().clip.nodes;
        let (first, rest) = nodes.split_first()?;
        let mut out = format!("{} node", nodes.len());
        if rest.is_empty() {
            out.push_str(": ");
        } else {
            out.push_str("s: ");
        }
        out.push_str(first.def.slug);
        for node in rest {
            out.push_str(", ");
            out.push_str(node.def.slug);
        }
        Some(out)
    }

    /// Answer a copy, a cut or a paste, from whichever door it was asked at.
    ///
    /// One implementation behind all three — the keyboard, the Edit menu and the node menu —
    /// so the three cannot drift apart the way two copies of the selection menu once did.
    ///
    /// A **copy** is no command at all: it changes nothing in the graph, so there is nothing
    /// about it to undo. A **cut** is that copy and a `RemoveNodes`, which is one step
    /// rather than a command of its own. A **paste** is a `Command::Paste` carrying what the
    /// copy took, so it replays against the graph rather than against whatever the clipboard
    /// holds by then.
    pub fn clipboard_action(&mut self, action: ClipAction) {
        match action {
            ClipAction::Copy(nodes) => {
                self.copy_nodes(&nodes);
            }
            ClipAction::Cut(nodes) => {
                if self.copy_nodes(&nodes) {
                    let _ = self.apply(Command::RemoveNodes(nodes));
                }
            }
            ClipAction::Paste { at } => {
                let Some(workspace) = self.active_workspace() else {
                    return;
                };
                if self.doc.clipboard().clip.is_empty() {
                    return;
                }
                let origin = self.canvas_origin();
                // Aimed, or not. A paste from the menu a right-click opened lands where that
                // click was. One from the keyboard or the Edit menu named no point, so it
                // lands where the copy was in the *window*, read back through whatever the
                // view is doing now.
                //
                // Pasting into the same view without moving it therefore lands the copy
                // exactly on top of its original. That is the promise of the rule rather
                // than a miss: the copies become the selection, so the next drag moves them
                // off, and a nudge invented here would put the clip somewhere nobody asked
                // for on every other paste.
                let at = at.unwrap_or_else(|| {
                    self.canvas_transform()
                        .to_world(origin, origin + self.doc.clipboard().anchor)
                });
                let clip = self.doc.clipboard().clip.clone();
                let _ = self.apply(Command::Paste {
                    clip,
                    at,
                    workspace,
                });
            }
        }
    }

    /// Lift these nodes onto the clipboard, with where they sat in the window. False where
    /// there was nothing to take, which is what stops a cut deleting on an empty selection.
    fn copy_nodes(&mut self, nodes: &[NodeId]) -> bool {
        if nodes.is_empty() {
            return false;
        }
        let Ok(mut clip) = self.doc.lift(nodes) else {
            return false;
        };
        // The folder its `assets/…` references name files in, so a paste into another project
        // brings the files with it.
        clip.from = Some(self.project.root().to_path_buf());
        let Some(anchor) = clip.anchor() else {
            return false;
        };
        let origin = self.canvas_origin();
        self.doc.set_clipboard(Clipboard {
            anchor: self.canvas_transform().to_screen(origin, anchor) - origin,
            clip,
        });
        true
    }
}
