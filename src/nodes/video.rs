// SPDX-License-Identifier: AGPL-3.0-or-later

//! A video file, as a texture, played by its Time — its own Speed, ambient time or a gear's —
//! with Offset added.
//!
//! The file is transcoded on first use into all-intra H.264 in the cache (`video/clip.rs`),
//! so every frame is reachable at the same cost. Position is the primitive: each tick the
//! node decides which frame it wants.
//!
//! **A clip is an oscillator whose shape is a frame lookup.** One cycle is one play of the clip.
//! **Time** counts plays (`TickContext::cycle_at`): at Speed 1, or in Loop mode unplugged, the
//! clip's native speed, one play every clip length, so Speed −1 plays it backwards and 2 twice as
//! fast; in Loop mode a gear cabled in replaces it, a Ratio Gear at `-×1` backwards and one at ×2
//! twice as fast. **Offset** is added, 0 to 1 across the clip, and the Loop option wraps the sum
//! or, at Hold, clamps it to one play. A slow wave on Offset scratches around the playing clip. The
//! frame is `round(position × frames)`, so the same position is the same frame however it was
//! reached, and the node keeps no position of its own: a hand on the scrubber moves Offset so the
//! sum lands where it was dropped.
//!
//! **A render waits for its frame.** Live, the picture is whatever the decoder has delivered,
//! a tick late after a jump; in a render the stepper holds the frame until the one asked for
//! has arrived ([`CpuNode::waiting`]), so a render is the same film twice.
//!
//! **It analyzes its own soundtrack.** The cache holds no audio — it is a picture format — so
//! the original file's audio is decoded once into memory (`audio::Track`) and read at
//! whatever time the picture is at. That is what makes a video file and a microphone the same
//! kind of thing to a graph: both publish three bands and three events, so repatching the
//! source leaves the show working. Scrubbing scrubs the analysis with it, because the samples
//! are indexed by time rather than played.

use crate::audio::{self, bands};
use crate::graph::NodeId;
use crate::graph::PortType::{UniformNumber, VaryingColor};
use crate::nodes::{
    Category, CpuDef, CpuNode, Edge, Frame, NodeDef, OptionDef, OutputDef, OutputKind, TickContext,
};
use crate::video::clip::{self, Codec, Player, Settings, Transcode};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

pub static DEF: NodeDef = NodeDef {
    slug: "video",
    category: Category::Source,
    icon: "🎞",
    label: "Video",
    tooltip: "A video file. Transcoded once so any frame is reachable; plays at its own speed \
              times its Speed, or as a gear cabled into Time plays it, and Offset is added to \
              where that is.",
    timing: Some(TIMING),
    inputs: crate::audio_inputs![
        crate::nodes::timing::time_row(TIMING, 0),
        crate::nodes::timing::speed_row(TIMING, 0),
        crate::nodes::timing::offset_row(TIMING, 0, UniformNumber),
    ],
    hidden: crate::nodes::audio_ports::TUNING,
    outputs: crate::audio_outputs![OutputDef {
        key: "frame",
        label: "Frame",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // The same mapping as the camera: worldspace into the frame's own [0,1] by its real
        // aspect, v flipped once because rows are uploaded top first, and mirrored outward
        // past its own edge by the texture's own wrap mode, set in render/mod.rs.
        wgsl: |node, ctx, _func| {
            let tex = ctx.texture_uniform(node, "frame");
            let sampler = ctx.sampler(node, "frame");
            format!(
                "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
            )
        },
        ..OutputDef::EMPTY
    },],
    options: crate::nodes::timing::options![
        OptionDef {
            key: "file",
            label: "File",
            default: "",
            // An asset reference: the canvas offers a file button — the project's own clips
            // first, and the file dialog under them — and swapping one rebuilds no shader,
            // because the clip reaches the fragment as a bound texture and not as WGSL.
            choices: &[],
            kind: crate::nodes::OptionKind::Asset,
            accepts: crate::nodes::Accepts::VIDEO,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "size",
            label: "Size",
            default: "1080",
            choices: &[("720", "Up to 720p"), ("1080", "Up to 1080p")],
            // Read by `tick` to pick the transcode's height cap. It re-transcodes; it
            // generates nothing.
            kind: crate::nodes::OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "loop",
            label: "Loop",
            default: "loop",
            choices: &[("loop", "Loop"), ("hold", "Hold last frame")],
            // Read by `tick`: what the sum of the playback and Position does at an end.
            kind: crate::nodes::OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        // The row of ticks, silvia's own, in silvia's order — with the picture's on the end,
        // which silvia has no tick for because its `<video>` element is always drawn.
        crate::nodes::audio_ports::SHOW_UNIFORMS,
        crate::nodes::audio_ports::SHOW_EVENTS,
        crate::nodes::audio_ports::SHOW_SCOPE,
        crate::nodes::SHOW_PREVIEW,
    ],
    row_headings: crate::nodes::timing::ROW_HEADINGS,
    // What the `preview` tick draws: the clip itself, the same texture the port hands
    // downstream, letterboxed into a 16:9 band because a clip's aspect is whatever was
    // imported and the band is laid out before a frame has been decoded.
    regions: &[
        crate::nodes::Region::Scope,
        crate::nodes::Region::Preview("frame"),
    ],
    cpu: Some(CpuDef {
        create: || Box::new(VideoNode::new()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// A clip's timing: a play a cycle, at the clip's native speed — one over its own length,
/// which the node reads off the clip and hands to `TickContext::cycle_at`, and which the graph
/// does not hold (`Timing::clip`). A looping clip comes back every play, and one holding its
/// last frame never does.
pub const TIMING: crate::nodes::Timing = crate::nodes::Timing::repeating(1.0, |node| {
    (node.options.get("loop").map(String::as_str) != Some("hold")).then_some(1.0)
})
.clip();

/// An elapsed time as a clock reads it: `0:07`, `1:42`, `13:05`.
///
/// Minutes and seconds with no hours, because an import that ran for an hour is a bug rather
/// than a wait, and `61:20` says that more plainly than `1:01:20` does.
fn clock(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The transcode's height cap for a `size` option value.
fn max_height(size: &str) -> u32 {
    match size {
        "720" => 720,
        _ => 1080,
    }
}

/// What the options asked for, so a tick can tell whether the clip still matches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wanted {
    source: PathBuf,
    settings: Settings,
}

enum Stage {
    /// No file chosen, or the last one failed; `error` says which.
    Idle,
    Transcoding(Transcode),
    Playing(Player),
}

/// The soundtrack's progress from opening the file to being readable.
enum Sound {
    /// No file, or a file whose audio could not be decoded. Bands read zero.
    Silent,
    /// A worker is decoding. `tick` polls and never waits.
    Decoding(mpsc::Receiver<audio::Track>),
    Ready(Box<audio::Reader>),
}

// The two `None`s mean different things and both are live states: not probed yet, and
// probed with no encoder on this machine. Flattening them would lose the difference.
#[allow(clippy::option_option)]
struct VideoNode {
    /// `None` until probed; `Some(None)` means this machine cannot encode.
    codec: Option<Option<Codec>>,
    wanted: Option<Wanted>,
    stage: Stage,
    error: Option<String>,
    /// Time and Offset, wrapped or held, as the scrubber reads it.
    shown: f64,
    /// The frame asked of the player this tick, and the one it last delivered: a render
    /// holds until they agree.
    asked: Option<u64>,
    delivered: Option<u64>,
    black: Arc<Frame>,
    /// Silence, as a waveform: what the oscilloscope draws before there is a soundtrack.
    flat: Arc<Frame>,
    /// The soundtrack's waveform, reused from tick to tick.
    scope: crate::nodes::audio_ports::Waveform,
    sound: Sound,
    /// This node's channel into the shared output device. Silent, and costing nothing, until
    /// `monitor` is turned up.
    monitor: audio::monitor::Send,
    /// What the tick last asked the analyzer for, so the scope draws the tuning the analysis
    /// actually used.
    config: [bands::BandConfig; bands::BANDS],
    levels: [f32; bands::BANDS],
}

impl VideoNode {
    fn new() -> Self {
        Self {
            codec: None,
            wanted: None,
            stage: Stage::Idle,
            error: None,
            shown: 0.0,
            asked: None,
            delivered: None,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            flat: Arc::new(crate::nodes::audio_ports::waveform_frame(
                &[128; crate::audio::WAVEFORM_LEN],
            )),
            scope: crate::nodes::audio_ports::Waveform::default(),
            sound: Sound::Silent,
            monitor: audio::Monitor::shared().open(),
            config: bands::DEFAULT,
            levels: [1.0; bands::BANDS],
        }
    }

    /// Point at a file: play it from the cache, or start making the cache entry.
    ///
    /// `cache` is the project's own: a folder that carries its transcodes opens on another
    /// machine without encoding them again.
    ///
    /// **A picture is refused by name.** A GIF, a PNG or a JPEG is the Image/GIF node's, and
    /// a project saved while a dropped GIF still made a video node holds one here. The
    /// transcode has no clip to make of it: GStreamer gives a picture no length, so it fails
    /// with `no frames`, and on Linux it cannot decode a GIF at all. The node says what the
    /// file is and which node shows it instead, and asks no encoder.
    fn open(&mut self, wanted: Wanted, cache: &Path) {
        self.error = None;
        self.asked = None;
        self.delivered = None;
        let name = wanted
            .source
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if crate::nodes::Accepts::IMAGE.matches(&name) {
            self.error = Some(format!("{name} is a picture: an Image/GIF node shows it"));
            self.stage = Stage::Idle;
            self.sound = Sound::Silent;
            self.wanted = Some(wanted);
            return;
        }
        self.stage = match clip::cache_path(cache, &wanted.source, wanted.settings) {
            None => {
                self.error = Some(format!("{}: cannot read", wanted.source.display()));
                Stage::Idle
            }
            Some(cached) if cached.is_file() => self.play(&cached),
            Some(cached) => Stage::Transcoding(Transcode::start(
                wanted.source.clone(),
                cached,
                wanted.settings,
            )),
        };
        // The soundtrack comes from the **source**, not the cache: the cache is a picture
        // format and carries none. Decoding blocks for as long as it takes to read the file,
        // so it happens on a worker and `tick` polls for it.
        let (tx, rx) = mpsc::channel();
        let source = wanted.source.clone();
        let cache = cache.to_path_buf();
        std::thread::spawn(move || {
            let track = audio::Track::decode(&source, &cache).unwrap_or_default();
            // The node may be gone by now, and a send to nobody is not an error.
            let _ = tx.send(track);
        });
        self.sound = Sound::Decoding(rx);

        self.wanted = Some(wanted);
    }

    /// Read the soundtrack at `seconds` and publish what it says.
    ///
    /// Every value a graph sees comes from here, including the events: a crossing found
    /// inside this read carries the moment it happened, so an envelope downstream gets the
    /// beat where it fell rather than where the frame noticed it.
    fn tick_audio(&mut self, id: NodeId, ctx: &mut TickContext<'_>, seconds: f64) {
        if let Sound::Decoding(rx) = &self.sound {
            match rx.try_recv() {
                Ok(track) => {
                    self.sound = Sound::Ready(Box::new(audio::Reader::new(Arc::new(track))));
                }
                // Still working, or the worker died: either way, nothing to wait for.
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.sound = Sound::Silent,
            }
        }
        // Read before the soundtrack is consulted, because the scope draws these whether or
        // not there is one: a node with no file yet still has its band handles under a hand.
        let config = crate::nodes::audio_ports::config(id, ctx);
        let levels: [f32; bands::BANDS] =
            std::array::from_fn(|b| ctx.input(id, crate::nodes::audio_ports::LEVELS[b]));
        self.config = config;
        self.levels = levels;

        let Sound::Ready(reader) = &mut self.sound else {
            for name in bands::NAMES {
                ctx.publish(id, name, 0.0);
            }
            // A flat line rather than nothing: a port that published no texture would sample
            // whatever was last uploaded under it.
            ctx.publish_frame(id, "oscilloscope", Arc::clone(&self.flat));
            return;
        };

        let analyzer = reader.analyzer_mut();
        analyzer.config = config;
        for (b, level) in levels.into_iter().enumerate() {
            analyzer.thresholds[b] = level;
        }

        let read = reader.advance_to(seconds);
        // The monitor plays exactly the samples the analysis just consumed, which is why a
        // scrub sounds like a scrub and a negative speed plays backwards: it is the same read,
        // not an imitation of one.
        self.monitor.set_volume(ctx.input(id, "monitor"));
        self.monitor.push(reader.last_samples(), audio::track::RATE);
        for (b, name) in bands::NAMES.into_iter().enumerate() {
            ctx.publish(id, name, read.analysis.bands[b]);
        }
        ctx.publish_frame(
            id,
            "oscilloscope",
            self.scope.frame(&read.analysis.waveform),
        );
        for crossing in &read.crossings {
            let Some(port) = crate::nodes::audio_ports::EVENTS.get(crossing.band as usize) else {
                continue;
            };
            let edge = if crossing.down { Edge::Down } else { Edge::Up };
            ctx.fire_at(id, port, edge, crossing.at);
        }
    }

    fn play(&mut self, cached: &Path) -> Stage {
        match Player::open(cached) {
            Ok(p) => Stage::Playing(p),
            Err(e) => {
                self.error = Some(e);
                Stage::Idle
            }
        }
    }
}

impl CpuNode for VideoNode {
    fn reset_in_place(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        self.shown = 0.0;
        self.asked = None;
    }

    fn waiting(&self) -> bool {
        self.asked.is_some() && self.asked != self.delivered
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // The option holds a reference; the project says where that is. This node never
        // learns the project root.
        let file = ctx.path(id, "file");
        // Probed once per instance: the element list does not change while running.
        let codec = *self.codec.get_or_insert_with(Codec::probe);
        let wanted = match (file, codec) {
            (None, _) => None,
            (Some(_), None) => {
                self.error = Some(format!(
                    "no hardware video encoder: {} is needed to import",
                    crate::platform::video::CODEC_HINT
                ));
                None
            }
            (Some(source), Some(codec)) => Some(Wanted {
                source,
                settings: Settings {
                    max_height: max_height(ctx.option(id, "size")),
                    codec,
                },
            }),
        };
        if self.wanted != wanted {
            if let Some(w) = wanted {
                self.open(w, &ctx.cache_dir());
            } else {
                self.stage = Stage::Idle;
                self.wanted = None;
                self.error = None;
            }
        }

        // A finished transcode becomes a player on the tick it lands.
        if let Stage::Transcoding(job) = &self.stage
            && let Some(result) = job.result()
        {
            self.stage = match result {
                Ok(path) => self.play(&path),
                Err(e) => {
                    self.error = Some(e);
                    Stage::Idle
                }
            };
        }

        let hold = ctx.option(id, "loop") == "hold";
        // A hand on the scrubber puts the clip where it was dropped: Offset is moved so the sum
        // lands there, and Time plays on from it.
        let dropped = ctx.seek(id);
        let offset = f64::from(ctx.input(id, crate::nodes::timing::OFFSET));

        // Where the picture is, in seconds, so the soundtrack can be read at the same place.
        let mut seconds = None;
        self.asked = None;
        let frame = match &mut self.stage {
            Stage::Playing(player) => {
                let info = player.info();
                let frames = info.frames.max(1);
                let length = frames as f64 / info.fps.max(0.001);
                // Plays of the clip: one every `length` seconds at its native speed, and where
                // its Time is without the Offset, for a scrub to move Offset against.
                let mut sum = ctx.cycle_at(id, 1.0 / length);
                if let Some(to) = dropped {
                    let time = sum - offset;
                    let to = f64::from(to).clamp(0.0, 1.0);
                    let offset = if hold {
                        // A clip on Hold never comes back: its Offset reaches a play either way.
                        let reach = crate::nodes::timing::offset_range(None);
                        (to - time).clamp(f64::from(reach.min), f64::from(reach.max))
                    } else {
                        crate::nodes::phasor::fraction(to - time, 1.0)
                    };
                    ctx.write_control(id, crate::nodes::timing::OFFSET, offset as f32);
                    sum = time + offset;
                }
                self.shown = if hold {
                    sum.clamp(0.0, 1.0)
                } else {
                    crate::nodes::phasor::fraction(sum, 1.0)
                };
                let index = clip::frame_at(self.shown, frames, !hold);
                seconds = Some(index as f64 / info.fps.max(0.001));
                player.request(index);
                self.asked = Some(index);
                if let Some(e) = player.error() {
                    self.error = Some(e);
                }
                let latest = player.latest();
                self.delivered = latest.as_ref().map(|(i, _)| *i);
                latest.map(|(_, f)| f)
            }
            _ => None,
        };
        ctx.publish_frame(
            id,
            "frame",
            frame.unwrap_or_else(|| Arc::clone(&self.black)),
        );
        // After the picture, and at the picture's own time: the soundtrack is read where the
        // frame is, not where the wall clock is, so a scrub scrubs the sound with it.
        self.tick_audio(id, ctx, seconds.unwrap_or(0.0));
    }

    /// Always a scope, even with no file open.
    ///
    /// The canvas reserves the scope region from the definition's `Region::Scope`, which knows
    /// nothing about whether a soundtrack has been decoded yet; publishing `None` there left a
    /// node that had just been spawned with an empty band inside its border. Silence is a
    /// picture of silence, and the band handles on it are live from the first frame.
    fn scope(&self) -> Option<audio::Scope> {
        let silence;
        let analysis = if let Sound::Ready(reader) = &self.sound {
            reader.last()
        } else {
            silence = audio::Analysis::default();
            &silence
        };
        Some(audio::Scope::new(
            analysis,
            self.config,
            self.levels,
            audio::track::RATE as f32,
        ))
    }

    /// Where the clip is, 0 to 1, for the scrubber on its picture. `None` until there is a
    /// clip to be anywhere in.
    fn playhead(&self) -> Option<f32> {
        matches!(self.stage, Stage::Playing(_)).then_some(self.shown as f32)
    }

    fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What a person standing over a slow import wants: that it is working, how far it has
    /// got, and **how long it has been**.
    ///
    /// The percentage is real — `Transcode::progress` is the encoder's own position in the
    /// file — but a percentage alone cannot say whether a wait is ten seconds or ten
    /// minutes, and a first import of a long clip is minutes. The clock beside it says so.
    /// The words are the ones a person would use for the wait rather than the name of the
    /// operation, and the line is drawn on the node's own preview band by
    /// `widgets::picture`, which is where a hand is already looking.
    ///
    /// **A node that is not playing says why there**, for the same reason: a file it refused
    /// or a transcode that failed leaves the band black, and the black band is where the eye
    /// is. `error` alone reaches only the Status box, which is closed by default.
    fn status(&self) -> Option<String> {
        match &self.stage {
            Stage::Transcoding(job) => Some(format!(
                "Preparing clip… {:.0}%  {}",
                job.progress() * 100.0,
                clock(job.elapsed()),
            )),
            Stage::Idle => self.error.clone(),
            Stage::Playing(_) => None,
        }
    }

    fn progress(&self) -> Option<f32> {
        match &self.stage {
            Stage::Transcoding(job) => Some(job.progress()),
            _ => None,
        }
    }

    fn debug(&self) -> Option<String> {
        match &self.stage {
            Stage::Playing(p) => {
                let i = p.info();
                Some(format!(
                    "video {}x{} @ {:.2} fps, {} frames, at {:.4}, cache {}, {}",
                    i.width,
                    i.height,
                    i.fps,
                    i.frames,
                    self.shown,
                    self.wanted.as_ref().map_or("?", |w| w.settings.codec.name),
                    p.delivery
                ))
            }
            Stage::Transcoding(_) => Some("video transcoding".to_string()),
            Stage::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clock;
    use std::time::Duration;

    /// The wait reads as a clock: padded seconds, and minutes that keep counting rather than
    /// rolling into an hours field nothing needs.
    #[test]
    fn the_elapsed_time_reads_as_a_clock() {
        for (seconds, want) in [
            (0, "0:00"),
            (7, "0:07"),
            (59, "0:59"),
            (60, "1:00"),
            (125, "2:05"),
            (3_600, "60:00"),
        ] {
            assert_eq!(clock(Duration::from_secs(seconds)), want);
        }
        // A fraction of a second is not a second: a clock at 1.9 s reads 0:01.
        assert_eq!(clock(Duration::from_millis(1900)), "0:01");
    }
}
