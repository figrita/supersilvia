// SPDX-License-Identifier: AGPL-3.0-or-later

//! A live recording, from the editor's side: what the Record button asks for, where the file
//! goes, and what the row and the status line say about it.
//!
//! **The recording itself is the synth's** — see [`crate::synth::record`] — because what it
//! reads is: the clock, and each frame the renderer draws. What is here is the half that reads
//! the document: the Record section's FPS, the name under `recordings/`, the refusals a hand
//! can be told about on the spot, and how an end is said. A recording is done to the
//! instrument, not to the document, so nothing here is a command and nothing enters the undo
//! history. See [docs/rendering.md](../../docs/rendering.md#live-recording).

use super::App;
use crate::graph::NodeId;
use crate::synth::{RecordEnd, Recorded};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

/// The editor's bookkeeping of recordings: those asked for that the synth has not shown yet,
/// and how each Output's last one ended.
#[derive(Default)]
pub(super) struct Records {
    /// Each Output whose recording was asked for and is not yet in a snapshot, by the number
    /// it was asked under: it counts as recording from the press, a tick before the synth says
    /// so.
    asked: HashMap<NodeId, u64>,
    /// How many have been asked for.
    seq: u64,
    /// Which Output each recording of this project was asked for, by its number: an end from
    /// another project's recording is said on the status line and kept by no Output.
    mine: HashMap<u64, NodeId>,
    /// How each Output's last recording ended, or why the last press could not start one,
    /// until the next press.
    outcomes: HashMap<NodeId, Result<Recorded, String>>,
}

impl Records {
    /// Another project: its Outputs' ids are about to be the new one's.
    pub(super) fn forget_project(&mut self) {
        self.asked.clear();
        self.mine.clear();
        self.outcomes.clear();
    }
}

impl App {
    /// Start recording `output` live: its picture to an `.mp4` in `recordings/`, at its Record
    /// section's FPS, until [`App::stop_recording`].
    ///
    /// # Errors
    /// A render is running, `output` is not an Output, is on no open workspace, has nothing
    /// cabled into it, or is already recording. A machine's refusal — no encoder, a folder
    /// that cannot be made — arrives later as its end, because the synth opens the file.
    pub fn start_recording(&mut self, output: NodeId) -> Result<(), String> {
        if self.rendering() {
            return Err("a render is running: stop it first".to_string());
        }
        let node = self
            .doc
            .graph()
            .get(output)
            .ok_or_else(|| "no such node".to_string())?;
        if !node.def.is_output {
            return Err("not an Output".to_string());
        }
        if !node
            .workspaces
            .iter()
            .any(|w| self.project.session().open.contains(w))
        {
            return Err("the Output is on no open workspace".to_string());
        }
        if self
            .doc
            .graph()
            .source_of(crate::graph::PortRef::new(output, "input"))
            .is_none()
        {
            return Err("the Output has nothing connected".to_string());
        }
        if self.recording_of(output).is_some() {
            return Err("already recording".to_string());
        }
        let fps = match node.controls.get(crate::nodes::output::RECORD_FPS) {
            Some(crate::graph::ControlValue::Float(v)) => f64::from(v.max(1.0)),
            _ => 30.0,
        };
        let destination = self.record_destination(output);
        let records = &mut self.media.records;
        records.seq += 1;
        let seq = records.seq;
        records.asked.insert(output, seq);
        records.mine.insert(seq, output);
        records.outcomes.remove(&output);
        self.link.send(crate::synth::Msg::StartRecord(Box::new(
            crate::synth::RecordRequest {
                output,
                fps,
                destination,
                seq,
            },
        )));
        Ok(())
    }

    /// Stop recording `output`. The file closes on the synth's thread a tick or two later,
    /// with every frame drawn up to now, and its end is said on the status line.
    pub fn stop_recording(&mut self, output: NodeId) {
        self.link.send(crate::synth::Msg::StopRecord(output));
    }

    /// The Record row's button: stop the recording running, or start one, and where a start is
    /// refused say why on the row and as a failure.
    pub fn toggle_recording(&mut self, output: NodeId) {
        if self.recording_of(output).is_some() {
            self.stop_recording(output);
            return;
        }
        if let Err(e) = self.start_recording(output) {
            self.media.records.outcomes.insert(output, Err(e.clone()));
            self.fail(format!("record: {e}"));
        }
    }

    /// `output`'s recording as its row draws it, from the press until its file is closing.
    pub fn recording_of(&self, output: NodeId) -> Option<crate::ui::RecordView> {
        self.link
            .snapshot()
            .recordings
            .iter()
            .find(|r| r.output == output)
            .map(|r| crate::ui::RecordView {
                seconds: r.seconds,
                dropped: r.dropped,
            })
            .or_else(|| {
                self.media
                    .records
                    .asked
                    .contains_key(&output)
                    .then_some(crate::ui::RecordView {
                        seconds: 0.0,
                        dropped: 0,
                    })
            })
    }

    /// Whether anything is recording, from the press: a render is refused beside it.
    pub fn recording(&self) -> bool {
        !self.media.records.asked.is_empty() || !self.link.snapshot().recordings.is_empty()
    }

    /// How `output`'s last recording ended, or why the last press could not start one.
    pub fn record_outcome(&self, output: NodeId) -> Option<&Result<Recorded, String>> {
        self.media.records.outcomes.get(&output)
    }

    /// Take what the snapshot says about recordings: a start the synth has shown is no longer
    /// only asked for, and an end is said — the file, with a Show, or the failure.
    pub(super) fn collect_recordings(&mut self) {
        let snapshot = self.link.snapshot();
        let ends: Vec<RecordEnd> = (snapshot.events.recorded)
            .unseen("recording ends")
            .map(|(_, end)| end.clone())
            .collect();
        let running: Vec<(NodeId, u64)> = snapshot
            .recordings
            .iter()
            .map(|r| (r.output, r.seq))
            .collect();
        let records = &mut self.media.records;
        records
            .asked
            .retain(|output, seq| !running.contains(&(*output, *seq)));
        for end in ends {
            let records = &mut self.media.records;
            if records.asked.get(&end.output) == Some(&end.seq) {
                records.asked.remove(&end.output);
            }
            if records.mine.remove(&end.seq) == Some(end.output) {
                records.outcomes.insert(end.output, end.result.clone());
            }
            match end.result {
                Ok(recorded) => {
                    let said = said(&recorded, &self.project.root().join(RECORDINGS));
                    self.say_written(said.clone(), recorded.destination.clone());
                    self.media.set_status_showing(said, recorded.destination);
                }
                Err(e) => self.fail(format!("recording failed: {e}")),
            }
        }
    }

    /// Where a recording of `output` started now goes: `recordings/output3-20261005-134501.mp4`,
    /// the Output and the time, with a counter where that name is taken.
    fn record_destination(&self, output: NodeId) -> PathBuf {
        let slug = self
            .doc
            .graph()
            .get(output)
            .map_or("output", |n| n.def.slug);
        let stem = format!(
            "{slug}{output}-{}",
            super::files::stamp(std::time::SystemTime::now())
        );
        let dir = self.project.root().join(RECORDINGS);
        let taken = |p: &PathBuf| p.exists() || p.with_extension("part").exists();
        let first = dir.join(format!("{stem}.mp4"));
        if !taken(&first) {
            return first;
        }
        // Two recordings of one Output in one second is a hand on the button twice.
        (2..=RECORDS_PER_SECOND)
            .map(|n| dir.join(format!("{stem}-{n}.mp4")))
            .find(|p| !taken(p))
            .unwrap_or(first)
    }
}

/// The folder in the project a recording goes into, beside `renders/`.
const RECORDINGS: &str = "recordings";

/// How many recordings of one Output one second's stamp may name before the counter stops.
const RECORDS_PER_SECOND: u32 = 999;

/// What the status line says of a finished recording: how long, where, what was dropped, and
/// why it stopped where it was not the button.
fn said(recorded: &Recorded, recordings: &std::path::Path) -> String {
    let name = recorded.destination.strip_prefix(recordings).map_or_else(
        |_| recorded.destination.display().to_string(),
        |p| format!("{RECORDINGS}/{}", p.display()),
    );
    let mut line = format!(
        "recorded {} to {name}",
        crate::nodes::output::clock(recorded.seconds)
    );
    if recorded.dropped > 0 {
        let _ = write!(
            line,
            ", {} of {} frames dropped",
            recorded.dropped, recorded.frames
        );
    }
    if let Some(why) = &recorded.why {
        let _ = write!(line, " — stopped: {why}");
    }
    line
}
