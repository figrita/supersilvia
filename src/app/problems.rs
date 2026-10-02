// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the problems badge counts and its list holds.
//!
//! Two kinds of thing. **What is wrong now** is read fresh every frame and never kept: an
//! Output whose shader failed, from the renderer's report, and a node whose own
//! [`CpuNode::error`](crate::nodes::CpuNode::error) says something, from the snapshot. It
//! leaves the list the frame it stops being true. **What was said** is kept until the next
//! of its kind or a hand's **Clear**: the failures this session has said, newest first and
//! at most [`FAILURES`], every warning the last Open dropped, and what the last export left
//! behind. None of it is saved; a problem is about this run.

use super::App;
use crate::graph::NodeId;
use crate::project::ExportReport;
use crate::ui::problems::{Problem, ProblemAction, Severity};
use crate::workspace::LoadWarning;
use eframe::egui;
use std::collections::VecDeque;

/// How many failures the list keeps. Past it the oldest goes: a list that grows without
/// bound under a stuck sequencer is not a list anyone reads.
pub(super) const FAILURES: usize = 8;

/// What was said, kept for the list.
#[derive(Debug, Default)]
pub(super) struct Problems {
    /// Failures said this session, oldest first.
    failures: VecDeque<String>,
    /// Every warning the last Open dropped, whole.
    opened: Vec<LoadWarning>,
    /// What the last export of this project left behind.
    exported: Option<ExportReport>,
    /// The list is showing.
    pub(super) open: bool,
}

impl Problems {
    /// A failure was said. One said again moves to the front rather than filling the list.
    pub(super) fn failed(&mut self, text: String) {
        self.failures.retain(|f| *f != text);
        self.failures.push_back(text);
        while self.failures.len() > FAILURES {
            self.failures.pop_front();
        }
    }

    /// A project was opened, with these warnings — none, for a project that opened clean.
    pub(super) fn opened(&mut self, warnings: Vec<LoadWarning>) {
        self.opened = warnings;
    }

    /// A workspace was exported, and this is what stayed behind.
    pub(super) fn exported(&mut self, report: ExportReport) {
        self.exported = Some(report);
    }

    /// Another project replaced this one: its warnings and its export are not the next one's.
    /// The failures stay, since they are this run's.
    pub(super) fn forget_project(&mut self) {
        self.opened.clear();
        self.exported = None;
    }

    /// A hand's **Clear**: everything said goes.
    fn clear(&mut self) {
        self.failures.clear();
        self.opened.clear();
        self.exported = None;
    }
}

/// The node a load warning is about, where it names one.
fn warned_node(warning: &LoadWarning) -> Option<NodeId> {
    match warning {
        LoadWarning::DuplicateId { fresh, .. } => Some(*fresh),
        LoadWarning::UnknownControl { id, .. }
        | LoadWarning::UnknownOption { id, .. }
        | LoadWarning::WrongValue { id, .. }
        | LoadWarning::UnreadPainting { id, .. } => Some(*id),
        LoadWarning::UnknownNodeKind { .. }
        | LoadWarning::DroppedConnection { .. }
        | LoadWarning::MissingWorkspace { .. }
        | LoadWarning::AdoptedWorkspace { .. }
        | LoadWarning::UnreadableWorkspace { .. } => None,
    }
}

/// Every line an export's report has about what stayed behind, with the node where a line
/// is about one.
fn left_behind(report: &ExportReport) -> Vec<(String, Option<NodeId>)> {
    let mut lines = Vec::new();
    for cable in &report.dropped_cables {
        lines.push((format!("export left behind the cable {cable}"), None));
    }
    for (node, workspaces) in &report.shared {
        lines.push((
            format!(
                "export: node {node} stays shown on {} as well",
                workspaces.join(", ")
            ),
            Some(*node),
        ));
    }
    for missing in &report.missing {
        lines.push((format!("export could not find {missing}"), None));
    }
    match report.midi {
        0 => {}
        1 => lines.push(("export left behind 1 MIDI binding".to_string(), None)),
        n => lines.push((format!("export left behind {n} MIDI bindings"), None)),
    }
    lines
}

/// How many lines [`left_behind`] writes, without writing them.
fn left_behind_count(report: &ExportReport) -> usize {
    report.dropped_cables.len()
        + report.shared.len()
        + report.missing.len()
        + usize::from(report.midi > 0)
}

impl App {
    /// Something failed: the status line says it, a toast says it where a person is looking,
    /// and the problems list keeps it. Every failure that reaches the status line comes
    /// through here or through [`Media::fail`](super::media::Media::fail).
    pub(super) fn fail(&mut self, text: impl Into<String>) {
        self.media.fail(text);
        self.hear_failures();
    }

    /// Toast and list every failure the status line has said since the last call. Called by
    /// [`App::fail`] and once a frame, for the failures `Media` says of its own accord — a
    /// render that failed.
    pub(super) fn hear_failures(&mut self) {
        for text in self.media.take_unheard() {
            self.show.problems.failed(text.clone());
            self.say_failed(text);
        }
    }

    /// Everything the problems list holds, errors first, the newest failure first among them.
    pub fn problems(&self) -> Vec<Problem> {
        let graph = self.doc.graph();
        let live = |id: NodeId| graph.get(id).is_some().then_some(id);
        let name = |id: NodeId| format!("{}{id}", graph.get(id).map_or("node", |n| n.def.slug));
        let mut problems = Vec::new();
        // What is wrong now, in node order so the list does not shuffle under the pointer.
        let mut faults = self.faults();
        faults.sort_by_key(|(id, _)| *id);
        for (id, why) in faults {
            problems.push(Problem {
                severity: Severity::Error,
                text: format!("{}: {why}", name(id)),
                go: live(id),
            });
        }
        for text in self.show.problems.failures.iter().rev() {
            problems.push(Problem {
                severity: Severity::Error,
                text: text.clone(),
                go: None,
            });
        }
        for warning in &self.show.problems.opened {
            problems.push(Problem {
                severity: Severity::Warning,
                text: format!("opening: {warning}"),
                go: warned_node(warning).and_then(live),
            });
        }
        if let Some(report) = &self.show.problems.exported {
            for (text, node) in left_behind(report) {
                problems.push(Problem {
                    severity: Severity::Warning,
                    text,
                    go: node.and_then(live),
                });
            }
        }
        problems
    }

    /// What the badge counts: [`App::problems`] counted without writing a line of it, since
    /// the badge asks every frame and the list only while it is open.
    pub(super) fn problem_count(&self) -> crate::ui::problems::Count {
        let kept = &self.show.problems;
        let exported = kept.exported.as_ref().map_or(0, left_behind_count);
        crate::ui::problems::Count {
            errors: self.faults().len() + kept.failures.len(),
            warnings: kept.opened.len() + exported,
        }
    }

    /// Every node at fault this frame, and why: an Output whose shader failed, and a CPU
    /// node whose own error says something. What a header's flag wears and its hover says.
    pub fn faults(&self) -> Vec<(NodeId, String)> {
        let snapshot = self.link.snapshot();
        let mut faults: Vec<(NodeId, String)> = snapshot
            .render
            .errors
            .iter()
            .map(|(id, e)| {
                (
                    *id,
                    format!("shader error: {}", e.lines().next().unwrap_or(e)),
                )
            })
            .collect();
        faults.extend(snapshot.faults.iter().map(|(id, e)| (*id, e.clone())));
        faults.retain(|(id, _)| self.doc.graph().get(*id).is_some());
        faults
    }

    /// The list under the badge, while it is open, and what a hand asked of it.
    pub(super) fn show_problems(&mut self, ctx: &egui::Context) {
        if !self.show.problems.open {
            return;
        }
        let problems = self.problems();
        let listed = crate::ui::problems::list(ctx, &problems, &self.theme);
        if listed.dismissed {
            self.show.problems.open = false;
        }
        for action in listed.actions {
            match action {
                ProblemAction::Clear => self.show.problems.clear(),
                ProblemAction::Go(node) => {
                    if let Some(workspace) = self.home_of(node) {
                        self.navigate_to(workspace, node);
                    }
                    self.show.problems.open = false;
                }
            }
        }
    }

    /// The badge was clicked: the list opens, or closes.
    pub(super) fn toggle_problems(&mut self) {
        self.show.problems.open = !self.show.problems.open;
    }

    /// Whether the problems list is open. For tests.
    pub fn problems_open(&self) -> bool {
        self.show.problems.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure said again moves to the front rather than filling the list, and the list
    /// keeps the newest [`FAILURES`].
    #[test]
    fn the_failures_kept_are_the_newest_and_each_once() {
        let mut p = Problems::default();
        for i in 0..FAILURES + 3 {
            p.failed(format!("save failed: {i}"));
        }
        assert_eq!(p.failures.len(), FAILURES);
        assert_eq!(
            p.failures.front().map(String::as_str),
            Some("save failed: 3")
        );
        p.failed("save failed: 5".to_string());
        assert_eq!(p.failures.len(), FAILURES);
        assert_eq!(
            p.failures.back().map(String::as_str),
            Some("save failed: 5")
        );
        assert_eq!(
            p.failures.iter().filter(|f| *f == "save failed: 5").count(),
            1
        );
    }

    /// Every part of an export's report that stayed behind is a line, and a shared node's
    /// line goes to it.
    #[test]
    fn an_export_report_is_a_line_per_thing_left_behind() {
        let report = ExportReport {
            dropped_cables: vec!["noise3.out to mix9.a".to_string()],
            shared: vec![(NodeId(4), vec!["Intro".to_string(), "Drop".to_string()])],
            missing: vec!["assets/gone.mp4".to_string()],
            midi: 2,
            ..ExportReport::default()
        };
        let lines = left_behind(&report);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            left_behind_count(&report),
            4,
            "the badge counts what the list writes"
        );
        assert_eq!(lines[1].1, Some(NodeId(4)));
        assert!(lines[1].0.contains("Intro, Drop"));
        assert_eq!(lines[3].0, "export left behind 2 MIDI bindings");
    }
}
