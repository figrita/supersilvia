// SPDX-License-Identifier: AGPL-3.0-or-later

//! An offline render, from the editor's side: what a render is asked for, where it writes,
//! and how it reads back.
//!
//! **The render itself runs on the synth thread** — see [`crate::synth::offline`] — because
//! everything it drives is the synth's: the clock, every `CpuNode`, the renderer and the
//! context. What is here is the half that reads the document: the settings off the Output's
//! own controls, the folder under `renders/`, the `!` a live source earns, and the progress
//! the banner draws out of the snapshot.
//! See [docs/rendering.md](../../docs/rendering.md#the-render-job).

use super::App;
use crate::clock::Warmup;
use crate::graph::NodeId;
use crate::maininput::{AudioSource, VideoSource};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// What a render writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A folder of `00000.png`, `00001.png`, …: lossless, for an editor.
    PngSequence,
    /// One `.mp4` on the machine's hardware encoder: for sending.
    Video,
    /// One animated `.gif` that repeats forever: for a loop, where it is posted.
    Gif,
}

impl Format {
    /// The extension a single-file format's destination carries; none for a sequence, which
    /// is a folder.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::PngSequence => None,
            Self::Video => Some("mp4"),
            Self::Gif => Some("gif"),
        }
    }
}

/// What a render is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSettings {
    pub fps: f64,
    /// Kept frames.
    pub frames: u32,
    pub warmup: Warmup,
    /// How much larger than the Output every frame is drawn before it comes back down to
    /// the Output's own size: 1, 2 or 4. The film is the Output's resolution either way.
    pub supersample: u32,
    pub format: Format,
    /// The folder a sequence is written into, or the video file's own path.
    pub destination: PathBuf,
}

/// Where a render is, for whatever draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub output: NodeId,
    /// Kept frames written so far, and how many there will be.
    pub written: u32,
    pub frames: u32,
    pub destination: PathBuf,
}

/// How a render ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done { frames: u32, destination: PathBuf },
    Canceled,
    Failed(String),
}

/// What an Output's Writer select asks for.
fn writer_of(node: &crate::graph::Node) -> Format {
    match node.options.get("writer").map(String::as_str) {
        Some("video") => Format::Video,
        Some("gif") => Format::Gif,
        _ => Format::PngSequence,
    }
}

impl App {
    /// Start rendering `output`. The synth resets every CPU node, puts the live clock
    /// aside and runs the loop on its own thread; the document is closed until an
    /// [`Outcome`] comes back in the snapshot.
    ///
    /// # Errors
    /// A render is already running, `output` is not an Output, or its input is unplugged.
    /// Everything a machine can refuse — a missing encoder, a folder that cannot be made —
    /// arrives later as `Outcome::Failed`, because it is the synth that opens the writer.
    pub fn start_render(
        &mut self,
        output: NodeId,
        settings: &RenderSettings,
    ) -> Result<(), String> {
        if self.rendering() {
            return Err("a render is already running".to_string());
        }
        // A render steps the show's clock a frame at a time, and a recording follows it as it
        // plays: never both.
        if self.recording() {
            return Err("a recording is running: stop it first".to_string());
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
        if settings.frames == 0 {
            return Err("a render of no frames".to_string());
        }
        let seq = self.media.begin_render(settings.destination.clone());
        self.link.send(crate::synth::Msg::StartRender(Box::new(
            crate::synth::thread::RenderRequest {
                output,
                settings: settings.clone(),
                seq,
            },
        )));
        Ok(())
    }

    /// What a render of `output` would be, read off the node: its three hidden numbers, its
    /// warm-up mode and its supersampling, into a folder of its own in the project.
    pub fn render_settings_of(&self, output: NodeId) -> Option<RenderSettings> {
        let node = self.doc.graph().get(output)?;
        let number = |key: &str| match node.controls.get(key) {
            Some(crate::graph::ControlValue::Float(v)) => Some(*v),
            _ => None,
        };
        let fps = f64::from(number("fps")?.max(1.0));
        let duration = f64::from(number("duration")?.max(0.0));
        let warm = number("warmup")?.max(0.0).round() as u32;
        let warmup = match node.options.get("warmupMode").map(String::as_str) {
            Some("black") => Warmup::Black,
            Some("hold") => Warmup::Hold(warm),
            _ => Warmup::Run(warm),
        };
        let format = writer_of(node);
        Some(RenderSettings {
            fps,
            frames: (fps * duration).ceil() as u32,
            warmup,
            supersample: crate::nodes::output::supersampling_of(node),
            format,
            destination: self.destination(output, format),
        })
    }

    /// Where a render in `format` goes: a numbered name under `renders/`, a folder for a
    /// sequence and a file with the format's extension otherwise.
    fn destination(&self, output: NodeId, format: Format) -> PathBuf {
        let folder = self.render_folder(output);
        match format.extension() {
            Some(ext) => folder.with_extension(ext),
            None => folder,
        }
    }

    /// Start a render of `output` as its node describes it. A refusal goes to the status
    /// line, since the button that asked has nowhere else to say so.
    pub fn start_render_on(&mut self, output: NodeId) {
        let Some(settings) = self.render_settings_of(output) else {
            self.fail("render: not an Output");
            return;
        };
        if let Err(e) = self.start_render(output, &settings) {
            self.fail(format!("render: {e}"));
        }
    }

    /// The render in progress as the canvas draws it.
    pub fn render_view(&self) -> Option<crate::ui::RenderView> {
        self.link
            .snapshot()
            .offline
            .running
            .as_ref()
            .map(|r| crate::ui::RenderView {
                node: r.output,
                written: r.written,
                frames: r.frames,
            })
    }

    /// The sources upstream of each Output that a render cannot step — every node whose
    /// `CpuDef::live` is set, and a `maininput` node only while the panel is pointed at a
    /// device rather than a file — for the `!` each Output wears. A walk back through the
    /// cables from each Output, across workspaces as the cables go; an Output with none has
    /// no entry.
    pub fn live_sources(&self) -> HashMap<NodeId, Vec<crate::ui::LiveSource>> {
        let panel = {
            let input = self.project.main_input();
            let video = match &input.video {
                VideoSource::Camera { .. } => Some("camera"),
                VideoSource::Screen => Some("screen"),
                VideoSource::Syphon { .. } => Some("Syphon server"),
                VideoSource::Ndi { .. } => Some("NDI source"),
                VideoSource::None | VideoSource::File { .. } => None,
            };
            let audio = match &input.audio {
                AudioSource::Live { device } if device.is_system() => Some("system audio"),
                AudioSource::Live { .. } => Some("microphone"),
                AudioSource::None | AudioSource::File { .. } | AudioSource::Video => None,
            };
            match (video, audio) {
                (None, None) => None,
                (Some(v), None) => Some(v.to_string()),
                (None, Some(a)) => Some(a.to_string()),
                (Some(v), Some(a)) => Some(format!("{v}, {a}")),
            }
        };
        let mut out = HashMap::new();
        for (output, node) in self.doc.graph().iter() {
            if !node.def.is_output {
                continue;
            }
            let mut sources = Vec::new();
            let mut visited = HashSet::new();
            let mut stack = vec![output];
            while let Some(id) = stack.pop() {
                if !visited.insert(id) {
                    continue;
                }
                let Some(n) = self.doc.graph().get(id) else {
                    continue;
                };
                if n.def.cpu.as_ref().is_some_and(|c| c.live) {
                    if n.def.slug == "maininput" {
                        if let Some(what) = &panel {
                            sources.push(crate::ui::LiveSource {
                                node: id,
                                label: format!("Main Input panel — {what}"),
                                workspace: None,
                                panel: true,
                            });
                        }
                    } else {
                        let workspace = self.home_of(id);
                        sources.push(crate::ui::LiveSource {
                            node: id,
                            label: format!("{}{} — {}", n.def.slug, id.0, n.def.label),
                            workspace,
                            panel: false,
                        });
                    }
                }
                stack.extend(self.doc.graph().cables_into(id).iter().map(|c| c.from.node));
            }
            // One line for the panel however many nodes read it.
            let mut seen_panel = false;
            sources.retain(|s| !s.panel || !std::mem::replace(&mut seen_panel, true));
            sources.sort_by_key(|s| s.node);
            if !sources.is_empty() {
                out.insert(output, sources);
            }
        }
        out
    }

    /// Ask the running render to stop. It ends on the synth's next tick, once the renderer
    /// has been told to stop capturing; what was written stays.
    pub fn cancel_render(&mut self) {
        self.link.send(crate::synth::Msg::CancelRender);
    }

    /// Whether the document is closed. True from the moment a render is asked for, so the
    /// command after it is already refused, and false again when the outcome lands.
    pub fn rendering(&self) -> bool {
        self.media.rendering(self.link.snapshot())
    }

    pub fn render_progress(&self) -> Option<Progress> {
        self.link
            .snapshot()
            .offline
            .running
            .as_ref()
            .map(|r| Progress {
                output: r.output,
                written: r.written,
                frames: r.frames,
                destination: r.destination.clone(),
            })
    }

    /// How the last render ended, until the next one starts.
    pub fn render_outcome(&self) -> Option<&Outcome> {
        self.media.render_outcome()
    }

    /// Where a render of `output` goes: a numbered name under `renders/` in the project,
    /// beside `assets/`, so it travels with the set and never overwrites the last one — a
    /// folder for a sequence, and the same name with `.mp4` on it for a video.
    pub fn render_folder(&self, output: NodeId) -> PathBuf {
        let renders = self.project.root().join("renders");
        let stem = format!("output{}", output.0);
        let mut n = 1;
        loop {
            let candidate = renders.join(format!("{stem}-{n:03}"));
            if !candidate.exists()
                && !candidate.with_extension("mp4").exists()
                && !candidate.with_extension("gif").exists()
            {
                return candidate;
            }
            n += 1;
        }
    }
}
