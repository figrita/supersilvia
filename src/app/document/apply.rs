// SPDX-License-Identifier: AGPL-3.0-or-later

//! Applying a command, and what it leaves for the rest of the app to do.
//!
//! `apply` is the one door into the graph: every change to it arrives as a `Command`, is
//! checked here, and comes back as an [`Applied`] naming whatever must be rebuilt before the
//! next frame.

use super::Document;
use super::history::{Origin, Step};
use crate::command::{Clip, ClipEdge, Command, CommandError};
use crate::graph::{ControlValue, Graph, NodeId, PortRef, WorkspaceId};
use crate::project::Project;
use emath::Pos2;

/// What an edit reads that is not the document's.
pub(in crate::app) struct Env<'a> {
    /// The project an imported workspace copies its media into.
    pub project: &'a Project,
    /// The width and height a new Output is made at, the preference's; `None` is the Output's
    /// own default.
    pub output_resolution: Option<(u32, u32)>,
}

/// Insert a node of the named kind as [`crate::nodes::add_to_graph_on`] does, and give a new
/// Output the size `env` asks for: every option of an Output drawn by the resolution picker,
/// where the size is one a file would open with.
fn add_node(
    graph: &mut Graph,
    slug: &str,
    at: Pos2,
    workspace: WorkspaceId,
    env: &Env<'_>,
) -> Option<NodeId> {
    let id = crate::nodes::add_to_graph_on(graph, slug, at, workspace)?;
    if let Some((w, h)) = env.output_resolution
        && let Some(node) = graph.get_mut(id)
        && node.def.is_output
    {
        let size = format!("{w}x{h}");
        if crate::nodes::output::holds_resolution(&size) {
            for option in node.def.options.iter().filter(|o| o.resolution) {
                node.options.insert(option.key, size.clone());
            }
        }
    }
    Some(id)
}

/// What a command did that some other part answers for.
#[derive(Debug)]
pub(in crate::app) struct Applied {
    /// The command as it was applied: fitted to its definition by `coerce`.
    pub command: Command,
    /// Outputs whose shader the edit changed.
    pub stale: Vec<NodeId>,
    /// Nodes the edit took out of the graph. Whatever was built for them, and the
    /// selection of them, goes too.
    pub removed: Vec<NodeId>,
    /// Nodes a paste or a duplicate made, which become the selection: the next drag moves
    /// what was just made.
    pub planted: Vec<NodeId>,
    /// A workspace the edit removed, whose tab closes with it.
    pub closed: Option<WorkspaceId>,
    /// A workspace an import brought in, and the report of what it brought.
    pub imported: Option<(WorkspaceId, String)>,
    /// What a paste brought into `assets/` from another project, or could not find, as the
    /// line the status says, and whether a file could not be found. `None` where it touched
    /// no file.
    pub brought: Option<(String, bool)>,
    /// True where the command named nothing to act on — an empty selection — and so
    /// changed nothing, opened no step and has nothing to publish.
    pub nothing: bool,
}

impl Applied {
    fn new(command: Command) -> Self {
        Self {
            command,
            stale: Vec::new(),
            removed: Vec::new(),
            planted: Vec::new(),
            closed: None,
            imported: None,
            brought: None,
            nothing: false,
        }
    }
}

impl Document {
    /// Apply one command. The only way anything in the graph changes.
    ///
    /// A refused command leaves the graph, the history and the serial exactly as it found
    /// them. One that succeeds has opened an undo step or joined the gesture in progress —
    /// which `origin` decides, see `history` — and what it means for the rest of the app is
    /// in the [`Applied`] it returns.
    pub(in crate::app) fn apply(
        &mut self,
        cmd: Command,
        env: &Env<'_>,
        origin: Origin,
    ) -> Result<Applied, CommandError> {
        let cmd = self.coerce(cmd)?;
        let continues = self.continues(&cmd, origin);
        // Taken before the mutation and kept only if it succeeds, so a refused command
        // leaves no undo step behind. A pointer, not a copy: the edit below writes through
        // `graph_mut`, which copies what it writes and leaves this graph as it was.
        let before = (!continues).then(|| (std::sync::Arc::clone(&self.graph), self.edit));
        let mut out = Applied::new(cmd.clone());

        match cmd {
            Command::AddNode {
                slug,
                at,
                workspace,
            } => {
                if !self.graph.has_workspace(workspace) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
                let id = add_node(self.graph_mut(), slug, at, workspace, env)
                    .ok_or(CommandError::NoSuchNodeKind(slug))?;
                self.mark(&mut out, id);
            }
            Command::Connect { from, to } => {
                // A cable that is already there is not an edit: nothing to log, publish or
                // rebuild.
                if !self
                    .graph_mut()
                    .connect(from, to)
                    .map_err(CommandError::Refused)?
                {
                    return Err(CommandError::AlreadyConnected { from, to });
                }
                self.mark(&mut out, to.node);
            }
            Command::Bridge {
                from,
                to,
                slug,
                inputs,
                output,
                at,
                workspace,
            } => {
                if !self.graph.has_workspace(workspace) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
                // Built on a copy — pointer bumps — and put on screen only once every cable
                // has landed. The menu offers only what `nodes::can_bridge` allows, and a
                // refusal there is a refusal of the whole gesture: the copy is dropped with
                // the node and whatever cables did land, so a command that failed leaves
                // nothing behind, not even the id the node would have spent.
                let mut graph = Graph::clone(&self.graph);
                let id = add_node(&mut graph, slug, at, workspace, env)
                    .ok_or(CommandError::NoSuchNodeKind(slug))?;
                inputs
                    .iter()
                    .try_for_each(|key| graph.connect(from, PortRef::new(id, key)).map(drop))
                    .and_then(|()| graph.connect(PortRef::new(id, output), to).map(drop))
                    .map_err(CommandError::Refused)?;
                self.graph = std::sync::Arc::new(graph);
                self.mark(&mut out, to.node);
            }
            Command::AddConnected {
                slug,
                at,
                workspace,
                end,
                key,
            } => {
                if !self.graph.has_workspace(workspace) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
                // On a copy, for the reason a bridge is: a refused cable leaves no node behind.
                let mut graph = Graph::clone(&self.graph);
                let id = add_node(&mut graph, slug, at, workspace, env)
                    .ok_or(CommandError::NoSuchNodeKind(slug))?;
                let made = PortRef::new(id, key);
                let downstream = if graph
                    .get(end.node)
                    .is_some_and(|n| n.output(end.key).is_some())
                {
                    graph.connect(end, made).map(|_| id)
                } else {
                    graph.connect(made, end).map(|_| end.node)
                };
                let downstream = downstream.map_err(CommandError::Refused)?;
                self.graph = std::sync::Arc::new(graph);
                self.mark(&mut out, downstream);
            }
            Command::Splice {
                node,
                from,
                to,
                input,
                output,
            } => {
                if self.graph.get(node).is_none() {
                    return Err(CommandError::NoSuchNode(node));
                }
                // On a copy: the old cable goes and the two new ones land, or nothing changes.
                let mut graph = Graph::clone(&self.graph);
                if !graph.disconnect_edge(from, to) {
                    return Err(CommandError::NotConnected(to));
                }
                graph
                    .connect(from, PortRef::new(node, input))
                    .and_then(|_| graph.connect(PortRef::new(node, output), to))
                    .map_err(CommandError::Refused)?;
                self.graph = std::sync::Arc::new(graph);
                self.mark(&mut out, to.node);
            }
            Command::Disconnect { to } => {
                if self.graph_mut().disconnect(to).is_empty() {
                    return Err(CommandError::NotConnected(to));
                }
                self.mark(&mut out, to.node);
            }
            Command::DisconnectEdge { from, to } => {
                if !self.graph_mut().disconnect_edge(from, to) {
                    return Err(CommandError::NotConnected(to));
                }
                self.mark(&mut out, to.node);
            }
            Command::DisconnectPort(port) => {
                let removed = self.graph_mut().disconnect_port(port);
                if removed.is_empty() {
                    return Err(CommandError::NotConnected(port));
                }
                // An output's edges each feed a different input, on a different node; an
                // input's is one. Marking every `to` covers both.
                for c in &removed {
                    self.mark(&mut out, c.to.node);
                }
            }
            Command::DisconnectAll(ref ids) => {
                if ids.is_empty() {
                    out.nothing = true;
                    return Ok(out);
                }
                for id in ids {
                    if self.graph.get(*id).is_none() {
                        return Err(CommandError::NoSuchNode(*id));
                    }
                }
                // Collected before the edges go: afterwards there is nothing left to walk.
                for id in ids {
                    self.mark(&mut out, *id);
                }
                for id in ids {
                    self.graph_mut().disconnect_node(*id);
                }
            }
            Command::ResetControls(ref ids) => {
                if ids.is_empty() {
                    out.nothing = true;
                    return Ok(out);
                }
                for id in ids {
                    if self.graph.get(*id).is_none() {
                        return Err(CommandError::NoSuchNode(*id));
                    }
                }
                for id in ids {
                    crate::nodes::reset_controls(self.graph_mut(), *id);
                    // The range is the node's own, so a reset of the node hands it back.
                    // Only the ranges: a note's text is not a control and a reset of the
                    // controls is not a reason to lose what somebody wrote.
                    self.graph_mut()
                        .get_mut(*id)
                        .expect("checked above")
                        .values
                        .retain(|_, v| v.range().is_none());
                }
                // Deliberately no recompile: every control is a uniform, and the options
                // this leaves alone are the only part of a node that changes generated code.
            }
            Command::MoveNodes { ref moves } => {
                // Checked before any is applied, so a move naming a stale node leaves the
                // rest of the selection where it was rather than half-moving it.
                for (node, _) in moves {
                    if self.graph.get(*node).is_none() {
                        return Err(CommandError::NoSuchNode(*node));
                    }
                }
                for (node, to) in moves {
                    self.graph_mut().get_mut(*node).expect("checked above").pos = *to;
                }
                // Position is not compile-relevant, so nothing is marked stale.
            }
            Command::SetNodeSize {
                node,
                width,
                height,
            } => {
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                // Anything that is not a size is no size at all: a NaN through undo would
                // be a node of no size, and a negative one a node drawn inside out.
                let size = |v: Option<f32>| v.filter(|v| v.is_finite() && *v > 0.0);
                n.dragged_width = size(width);
                n.dragged_height = size(height);
                // Nothing is marked stale: a body's size is layout, exactly as a position
                // is, and no shader has ever read one.
            }
            Command::SetControl { node, key, value } => {
                // Already checked and fitted by `coerce`.
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                n.controls.insert(key, value);
                recap(n, key);
                crate::nodes::timing::fit_offsets(n);
                // Deliberately no recompile: a parameter edit sets a uniform. That
                // asymmetry is what makes the instrument playable during a performance.
            }
            Command::SetControls { node, ref values } => {
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                for (key, value) in values {
                    n.controls.insert(key, *value);
                }
                for (key, _) in values {
                    recap(n, key);
                }
                crate::nodes::timing::fit_offsets(n);
            }
            Command::SetRange { node, key, range } => {
                // Already fitted to the definition's range by `coerce`.
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                n.values.insert(key, crate::graph::Value::Range(range));
                refit(n, key, range);
                // No recompile, for the same reason `SetControl` does not: a range moves a
                // uniform's ends, and the uniform is already a uniform.
            }
            Command::ClearRange { node, key } => {
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                // Both refusals before either write, so a command that fails leaves the
                // graph exactly as it found it: what `apply` publishes to the synth is the
                // result of a command, and a half-applied one would cross as a graph no
                // command produced. Only a range — a key holding a note's text is not a
                // range to clear.
                if n.values
                    .get(key)
                    .and_then(crate::graph::Value::range)
                    .is_none()
                {
                    return Err(CommandError::NoSuchKey(node, key));
                }
                let Some(range) = crate::nodes::default_range(n, key) else {
                    return Err(CommandError::NoSuchKey(node, key));
                };
                // This is what hands the range back: a guard that only looked would leave
                // the node's own range exactly where it was.
                n.values.remove(key);
                // An Offset goes round its period rather than to an end of it.
                crate::nodes::timing::fit_offsets(n);
                refit(n, key, range);
            }
            Command::SetValue {
                node,
                key,
                ref value,
            } => {
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                // A value the definition does not declare is not a value. The range half of
                // the map is `SetRange`'s business and arrives through its own command, so
                // this refuses one rather than writing a range nothing fitted.
                if value.range().is_some() || n.def.value(key).is_none() {
                    return Err(CommandError::NoSuchKey(node, key));
                }
                n.values.insert(key, value.clone());
                // Deliberately no recompile: a value reaches no generator and no uniform.
            }
            Command::SetOption {
                node,
                key,
                ref value,
            } => {
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                if !n.options.contains_key(key) {
                    return Err(CommandError::NoSuchKey(node, key));
                }
                let def = n.def;
                n.options.insert(key, value.clone());
                // A period that shrank takes its Offset round it, in the same step.
                crate::nodes::timing::fit_offsets(n);
                // A time mode puts one row away, and the cable into it goes in the same step,
                // so the switch and the disconnect are one undo.
                if !self.graph_mut().drop_inactive(node).is_empty() {
                    self.mark(&mut out, node);
                }
                // Only a `Code` option is in the shader. A `Uniform` one is an `int` the
                // program already reads and an `Asset` names a texture the renderer binds,
                // so neither is a reason to rebuild — which is what lets a crossfade method
                // change during a set and a clip be swapped without a stutter. Resolution
                // only reallocates, but paying one rebuild is cheaper than tracking which of
                // the `Code` options is which.
                if def
                    .option(key)
                    .is_none_or(crate::nodes::OptionDef::rebuilds)
                {
                    self.mark(&mut out, node);
                }
            }
            Command::SetSettings {
                node,
                ref options,
                ref controls,
            } => {
                // Every key already checked and fitted by `coerce`, so this cannot stop halfway.
                let n = self
                    .graph_mut()
                    .get_mut(node)
                    .ok_or(CommandError::NoSuchNode(node))?;
                let def = n.def;
                for (key, value) in options {
                    n.options.insert(key, value.clone());
                }
                for (key, value) in controls {
                    n.controls.insert(key, *value);
                }
                for (key, _) in controls {
                    recap(n, key);
                }
                crate::nodes::timing::fit_offsets(n);
                // And its time mode's: the cable into a row put away goes with it.
                if !self.graph_mut().drop_inactive(node).is_empty() {
                    self.mark(&mut out, node);
                }
                // The rule `SetOption` keeps: only a `Code` option is in the shader.
                if options.iter().any(|(key, _)| {
                    def.option(key)
                        .is_none_or(crate::nodes::OptionDef::rebuilds)
                }) {
                    self.mark(&mut out, node);
                }
            }
            Command::SetCollapsed {
                ref nodes,
                collapsed,
            } => {
                // Presentation only: nothing is marked stale. The ports and the cables are
                // untouched, so the shader that was running still describes the graph.
                for node in nodes {
                    if self.graph.get(*node).is_none() {
                        return Err(CommandError::NoSuchNode(*node));
                    }
                }
                for node in nodes {
                    self.graph_mut()
                        .get_mut(*node)
                        .expect("checked above")
                        .collapsed = collapsed;
                }
            }
            Command::SetLayout { workspace, mode } => {
                if !self.graph_mut().set_layout(workspace, mode) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
            }
            Command::ImportWorkspace { ref file } => {
                let (id, report) = env
                    .project
                    .import_workspace(file, self.graph_mut())
                    .map_err(|e| CommandError::Failed(format!("import failed: {e}")))?;
                for w in &report.warnings {
                    log::warn!("importing {}: {w}", file.display());
                }
                for missing in &report.missing {
                    log::warn!("import could not find {missing}");
                }
                // Every Output that arrived has no shader yet, and neither has anything
                // downstream of a node the import re-pointed.
                let arrived: Vec<NodeId> = self.graph.on_workspace(id).map(|(n, _)| n).collect();
                for node in arrived {
                    self.mark(&mut out, node);
                }
                out.imported = Some((id, report.to_string()));
            }
            Command::AutoArrange { workspace, height } => {
                if !self.graph.has_workspace(workspace) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
                let placed = crate::ui::layout::arrange(&self.graph, workspace, height);
                for (id, pos) in placed {
                    if let Some(node) = self.graph_mut().get_mut(id) {
                        node.pos = pos;
                    }
                }
            }
            Command::Duplicate { ref nodes, offset } => {
                // Copy the node, not its definition's defaults: a duplicate that lost the
                // values someone had dialed in would be a new node with extra steps.
                let clip = self.lift(nodes)?;
                // No workspace named: a duplicate belongs where its original is, on all of
                // the workspaces the original is on.
                self.plant(&mut out, &clip, offset, None);
            }
            Command::Paste {
                ref clip,
                at,
                workspace,
            } => {
                if !self.graph.has_workspace(workspace) {
                    return Err(CommandError::NoSuchWorkspace(workspace));
                }
                let anchor = clip.anchor().ok_or(CommandError::EmptyClip)?;
                // Its media first: a clip names its files by where they are, and a file from
                // another project is copied into this one's `assets/` as an import copies it.
                let (brought, said) = crate::app::files::bring_media(clip, env.project);
                // The anchor lands on `at` and the rest keeps its shape around it, so a
                // patch of six nodes arrives as the patch it was rather than as a pile.
                self.plant(
                    &mut out,
                    brought.as_ref().unwrap_or(clip),
                    at - anchor,
                    Some(workspace),
                );
                out.brought = said;
            }
            Command::DuplicateWorkspace { id, ref name } => {
                self.duplicate_workspace(&mut out, id, name.clone())?;
            }
            Command::AddWorkspace {
                ref name,
                kind,
                layout,
                seed,
            } => {
                let id = self.graph_mut().add_workspace(name.clone(), kind);
                self.graph_mut().set_layout(id, layout);
                // A workspace is a view: no node moved, so no shader changed — unless it was
                // born holding something, and then the Output it was born holding has one.
                if let crate::command::Seed::SourceToOutput { width } = seed {
                    self.seed_workspace(&mut out, id, width, env);
                }
            }
            Command::RemoveWorkspace(id) => {
                if !self.graph.has_workspace(id) {
                    return Err(CommandError::NoSuchWorkspace(id));
                }
                if self.graph.workspaces().len() < 2 {
                    return Err(CommandError::LastWorkspace);
                }
                // Collected before the removal: afterwards the orphans and their edges are
                // gone, and the Outputs that fed from them cannot be found.
                let doomed = self.graph.nodes_only_on(id);
                for node in &doomed {
                    self.mark(&mut out, *node);
                }
                self.graph_mut()
                    .remove_workspace(id)
                    .expect("checked above");
                out.stale.retain(|o| !doomed.contains(o));
                out.removed = doomed;
                // A workspace that is gone has no tab. Session state, so the caller closes it
                // rather than it being part of the step this pushes.
                out.closed = Some(id);
            }
            Command::RenameWorkspace { id, ref name } => {
                if !self.graph_mut().rename_workspace(id, name.clone()) {
                    return Err(CommandError::NoSuchWorkspace(id));
                }
            }
            Command::SetBlurb { id, ref blurb } => {
                if !self.graph_mut().set_blurb(id, blurb.clone()) {
                    return Err(CommandError::NoSuchWorkspace(id));
                }
            }
            Command::MoveWorkspace { id, to } => {
                if !self.graph_mut().move_workspace(id, to) {
                    return Err(CommandError::NoSuchWorkspace(id));
                }
            }
            Command::ShowOn {
                ref nodes,
                workspace,
            } => {
                self.check_membership(nodes, workspace)?;
                for node in nodes {
                    self.graph_mut()
                        .get_mut(*node)
                        .expect("checked above")
                        .workspaces
                        .insert(workspace);
                }
            }
            Command::HideFrom {
                ref nodes,
                workspace,
            } => {
                // Every id checked before any is written, so a hide that would strand one
                // node leaves the whole selection where it was.
                self.check_membership(nodes, workspace)?;
                for node in nodes {
                    let on = &self.graph.get(*node).expect("checked above").workspaces;
                    if on.len() == 1 && on.contains(&workspace) {
                        return Err(CommandError::NoWorkspaceLeft(*node));
                    }
                }
                for node in nodes {
                    self.graph_mut()
                        .get_mut(*node)
                        .expect("checked above")
                        .workspaces
                        .remove(&workspace);
                }
            }
            Command::MoveTo {
                ref nodes,
                workspace,
            } => {
                self.check_membership(nodes, workspace)?;
                for node in nodes {
                    self.graph_mut()
                        .get_mut(*node)
                        .expect("checked above")
                        .workspaces = std::collections::BTreeSet::from([workspace]);
                }
            }
            Command::RemoveNodes(ref ids) => {
                // Each named once, by `coerce`, so a second removal of one cannot fail.
                for id in ids {
                    if self.graph.get(*id).is_none() {
                        return Err(CommandError::NoSuchNode(*id));
                    }
                }
                for id in ids {
                    // Collect before removing: afterwards the edges are gone.
                    self.mark(&mut out, *id);
                    self.graph_mut().remove_node(*id).expect("checked above");
                }
                out.stale.retain(|o| !ids.contains(o));
                out.removed.clone_from(ids);
            }
        }
        if cmd.reshapes() {
            self.shape += 1;
        }
        // A new edit becomes an undo step, and anything undone is no longer reachable.
        if let Some((graph, serial)) = before {
            let step = Step::new(graph, &self.graph, cmd.clone(), serial);
            self.push_undo(step);
        } else {
            self.continue_step(cmd.clone());
        }
        // A serial per state, taken whether or not the command opened a step: a gesture that
        // coalesces still changes the graph, and the title says so.
        self.next_edit += 1;
        self.edit = self.next_edit;
        self.gesture_after(cmd, origin);
        Ok(out)
    }

    /// Take nodes out of the graph whole, with the cables between them.
    ///
    /// The one place a clip is made. A duplicate lifts and plants in the same breath; a copy
    /// keeps what it lifted on the clipboard until a paste asks for it.
    pub(in crate::app) fn lift(&self, ids: &[NodeId]) -> Result<Clip, CommandError> {
        let mut nodes = Vec::with_capacity(ids.len());
        for id in ids {
            let node = self.graph.get(*id).ok_or(CommandError::NoSuchNode(*id))?;
            nodes.push(node.clone());
        }
        // Only edges with both ends inside the set. One from outside has no second source to
        // copy, and silently sharing the original's would make the copy a fan-out rather
        // than a copy.
        let index = |id: NodeId| ids.iter().position(|other| *other == id);
        let edges = self
            .graph
            .connections()
            .iter()
            .filter_map(|c| {
                Some(ClipEdge {
                    from: (index(c.from.node)?, c.from.key),
                    to: (index(c.to.node)?, c.to.key),
                })
            })
            .collect();
        Ok(Clip {
            nodes,
            edges,
            from: None,
        })
    }

    /// Put a clip's nodes in the graph under fresh ids, and its cables between them.
    ///
    /// `offset` moves the whole clip and `workspace` replaces every node's own set, which is
    /// what a paste names and a duplicate leaves alone.
    ///
    /// A bulk change, like a file: the effective types are worked out once, after the last
    /// cable, rather than after every node and cable.
    fn plant(
        &mut self,
        out: &mut Applied,
        clip: &Clip,
        offset: emath::Vec2,
        workspace: Option<WorkspaceId>,
    ) {
        let mut made = Vec::with_capacity(clip.nodes.len());
        self.graph_mut().begin_bulk();
        for node in &clip.nodes {
            let workspaces = workspace.map_or_else(
                || node.workspaces.clone(),
                |id| std::iter::once(id).collect(),
            );
            // The kind comes with `def`, so a copy has everything its definition declares
            // without being stamped; what it holds of its own is the original's, below.
            let new = self.graph_mut().add_node(
                node.def,
                node.pos + offset,
                node.inputs.clone(),
                node.outputs.clone(),
                workspaces,
            );
            let placed = self.graph_mut().get_mut(new).expect("just added");
            placed.controls.clone_from(&node.controls);
            placed.values.clone_from(&node.values);
            placed.options.clone_from(&node.options);
            placed.collapsed = node.collapsed;
            made.push(new);
        }
        for edge in &clip.edges {
            let (Some(from), Some(to)) = (made.get(edge.from.0), made.get(edge.to.0)) else {
                continue;
            };
            // The set was a legal graph and the copy has the same shape, so this cannot
            // fail; if it ever does, the copy is still usable.
            let _ = self.graph_mut().connect_loading(
                PortRef::new(*from, edge.from.1),
                PortRef::new(*to, edge.to.1),
            );
        }
        // The copy's types from the copy's own cables and knobs, not the original's; a cable
        // that leaves reading a field as a number goes, as a refused one would have.
        self.graph_mut().settle_effective_types();
        // A copy of an Output with a name of its own goes out under a name of its own too.
        crate::nodes::output::settle_names(self.graph_mut(), &made);
        for id in &made {
            self.mark(out, *id);
        }
        out.planted = made;
    }

    /// Copy workspace `id` whole under `name`, beside it in project order.
    ///
    /// Every node on it is copied as a plain node on the copy and nowhere else, shared or
    /// not, as an export writes it: a duplicate is a variation to change without changing the
    /// original, and a node still shared with it would be the original. The cables between
    /// them come with them, as a paste's do. **A cable arriving from a node on no copy comes
    /// too**, a second cable out of the same output, so the copy reads the clock or the source
    /// the original reads; **one leaving for a node elsewhere does not**, because that input
    /// already has the original's and taking it would change the original's patch.
    fn duplicate_workspace(
        &mut self,
        out: &mut Applied,
        id: WorkspaceId,
        name: String,
    ) -> Result<(), CommandError> {
        let Some(from) = self.graph.workspace(id).cloned() else {
            return Err(CommandError::NoSuchWorkspace(id));
        };
        let at = self
            .graph
            .workspaces()
            .iter()
            .position(|w| w.id == id)
            .expect("checked above");
        let ids: Vec<NodeId> = self.graph.on_workspace(id).map(|(n, _)| n).collect();
        let clip = self.lift(&ids)?;
        // Read before anything is planted: the cables into the copied set from outside it,
        // by which node of the set each lands on.
        let inbound: Vec<(PortRef, usize, &'static str)> = self
            .graph
            .connections()
            .iter()
            .filter(|c| !ids.contains(&c.from.node))
            .filter_map(|c| {
                let to = ids.iter().position(|n| *n == c.to.node)?;
                Some((c.from, to, c.to.key))
            })
            .collect();
        let copy = self.graph_mut().add_workspace(name, from.kind);
        self.graph_mut().set_layout(copy, from.layout);
        self.graph_mut().set_blurb(copy, from.blurb);
        self.graph_mut().move_workspace(copy, at + 1);
        // Where the original's nodes are, on a canvas of its own.
        self.plant(out, &clip, emath::Vec2::ZERO, Some(copy));
        for (source, to, key) in inbound {
            let Some(node) = out.planted.get(to).copied() else {
                continue;
            };
            // The original took this cable, and the copy has the original's shape, so a
            // refusal would mean the graph moved under it; the copy is usable without it.
            if let Err(e) = self.graph_mut().connect(source, PortRef::new(node, key)) {
                log::debug!("a duplicate could not take a cable from outside: {e}");
            }
        }
        Ok(())
    }

    /// Every node exists and the workspace exists, checked before a membership command
    /// writes anything.
    fn check_membership(
        &self,
        nodes: &[NodeId],
        workspace: WorkspaceId,
    ) -> Result<(), CommandError> {
        if !self.graph.has_workspace(workspace) {
            return Err(CommandError::NoSuchWorkspace(workspace));
        }
        for node in nodes {
            if self.graph.get(*node).is_none() {
                return Err(CommandError::NoSuchNode(*node));
            }
        }
        Ok(())
    }

    /// Fit a command's payload to the node definition before anything applies or logs it.
    ///
    /// A control's value is fitted to its declared type and its range, a range to the
    /// definition's, and an option's value is refused where the definition does not offer
    /// it; a removal names each node once. Doing it here rather than in the widget means the
    /// bus, a saved file and anything mapped to a controller all get the same treatment:
    /// nothing downstream re-checks the variant against the declared type, and nothing
    /// downstream clamps.
    fn coerce(&self, cmd: Command) -> Result<Command, CommandError> {
        match cmd {
            Command::SetControl { node, key, value } => {
                let n = self.graph.get(node).ok_or(CommandError::NoSuchNode(node))?;
                let def = n.def;
                let range = crate::nodes::control_range(def, n, key);
                let (key, value) = crate::nodes::coerce_control(def, key, value, range)
                    .ok_or(CommandError::NoSuchKey(node, key))?;
                Ok(Command::SetControl { node, key, value })
            }
            Command::SetControls { node, values } => {
                let n = self.graph.get(node).ok_or(CommandError::NoSuchNode(node))?;
                let def = n.def;
                // Every key fitted before any is written, so a handle naming one bad key
                // moves neither axis rather than half of one.
                let values = values
                    .into_iter()
                    .map(|(key, value)| {
                        let range = crate::nodes::control_range(def, n, key);
                        crate::nodes::coerce_control(def, key, value, range)
                            .ok_or(CommandError::NoSuchKey(node, key))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Command::SetControls { node, values })
            }
            Command::SetRange { node, key, range } => {
                let n = self.graph.get(node).ok_or(CommandError::NoSuchNode(node))?;
                let (key, range) = crate::nodes::coerce_range(n.def, key, range)
                    .ok_or(CommandError::NoSuchKey(node, key))?;
                Ok(Command::SetRange { node, key, range })
            }
            Command::SetOption { node, key, value } => {
                let n = self.graph.get(node).ok_or(CommandError::NoSuchNode(node))?;
                // The loader's rule, so the bus cannot store what a file would not open.
                if n.def.option(key).is_some() && !crate::nodes::option_is_valid(n.def, key, &value)
                {
                    return Err(CommandError::NoSuchChoice(node, key));
                }
                // An Output's send name is made its own here, so the step holds the name it
                // went out under rather than the one typed.
                let value = crate::nodes::fit_option(&self.graph, node, key, value);
                Ok(Command::SetOption { node, key, value })
            }
            Command::SetSettings {
                node,
                options,
                controls,
            } => {
                let n = self.graph.get(node).ok_or(CommandError::NoSuchNode(node))?;
                let def = n.def;
                // `SetOption`'s rules, key by key — except that a key the node does not
                // declare is refused rather than stored, since a button names only what its
                // own node has.
                let options = options
                    .into_iter()
                    .map(|(key, value)| {
                        let key = def
                            .option(key)
                            .ok_or(CommandError::NoSuchKey(node, key))?
                            .key;
                        if !crate::nodes::option_is_valid(def, key, &value) {
                            return Err(CommandError::NoSuchChoice(node, key));
                        }
                        Ok((key, crate::nodes::fit_option(&self.graph, node, key, value)))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let controls = controls
                    .into_iter()
                    .map(|(key, value)| {
                        let range = crate::nodes::control_range(def, n, key);
                        crate::nodes::coerce_control(def, key, value, range)
                            .ok_or(CommandError::NoSuchKey(node, key))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Command::SetSettings {
                    node,
                    options,
                    controls,
                })
            }
            Command::RemoveNodes(mut ids) => {
                let mut seen = std::collections::HashSet::with_capacity(ids.len());
                ids.retain(|id| seen.insert(*id));
                Ok(Command::RemoveNodes(ids))
            }
            other => Ok(other),
        }
    }

    /// Mark every Output whose shader depends on this node.
    ///
    /// Only structural change reaches here. A parameter edit sets a uniform and never
    /// recompiles, which is what makes the instrument playable.
    fn mark(&self, out: &mut Applied, node: NodeId) {
        for output in self.graph.downstream_outputs(node) {
            if !out.stale.contains(&output) {
                out.stale.push(output);
            }
        }
    }

    /// Fill a brand-new workspace with the shortest patch that shows a picture: the Main
    /// Input on the left, an Output on the right, one cable between them.
    ///
    /// Inside the `AddWorkspace` command rather than beside it, so the whole tab — nodes,
    /// cable and all — is one undo step.
    ///
    /// The Output is placed against `width`, the canvas as it was when the tab was asked for.
    /// A window narrower than the two nodes side by side falls back to a fixed gap rather
    /// than stacking them on top of each other.
    fn seed_workspace(
        &mut self,
        out: &mut Applied,
        workspace: WorkspaceId,
        width: f32,
        env: &Env<'_>,
    ) {
        use crate::ui::canvas;
        use crate::ui::layout::{HORIZONTAL_MARGIN, VERTICAL_MARGIN};
        let at = |x: f32| Pos2::new(x, VERTICAL_MARGIN);
        let Some(source) = crate::nodes::add_to_graph_on(
            self.graph_mut(),
            "maininput",
            at(HORIZONTAL_MARGIN),
            workspace,
        ) else {
            return;
        };
        let right = width - canvas::OUTPUT_NODE_WIDTH - HORIZONTAL_MARGIN;
        // Never to the left of the source, and never on top of it.
        let right = right.max(HORIZONTAL_MARGIN + canvas::NODE_WIDTH + HORIZONTAL_MARGIN);
        let Some(output) = add_node(self.graph_mut(), "output", at(right), workspace, env) else {
            return;
        };
        // A refusal here would mean the two definitions had drifted apart, which the registry
        // tests would have caught first; there is nothing useful to do about it at run time
        // beyond leaving the two nodes uncabled.
        if let Err(e) = self.graph_mut().connect(
            crate::graph::PortRef::new(source, "frame"),
            crate::graph::PortRef::new(output, "input"),
        ) {
            log::debug!("a new workspace could not be cabled up: {e}");
        }
        self.mark(out, output);
    }
}

/// Move the range of every control that follows the one just written, and refit its value.
///
/// Inside `SetControl` rather than beside it, so a kaleidoscope's Segments and the Source
/// Segment range it caps are one undo step and one published graph. Which controls follow
/// which is `crate::nodes::capped_ranges`, so nothing here knows what node it is editing.
fn recap(node: &mut crate::graph::Node, changed: &'static str) {
    for (key, range) in crate::nodes::capped_ranges(node.def, node, changed) {
        node.values.insert(key, crate::graph::Value::Range(range));
        refit(node, key, range);
    }
}

/// Pull a control's stored value back inside a range that just moved.
///
/// Without this a narrowed range leaves the value outside its own track, and the fill
/// behind the number would be pinned to an end that no longer means what it shows.
fn refit(node: &mut crate::graph::Node, key: &'static str, range: crate::graph::ControlRange) {
    if let Some(ControlValue::Float(v)) = node.controls.get(key) {
        let fitted = v.clamp(range.min, range.max);
        node.controls.insert(key, ControlValue::Float(fitted));
    }
}
