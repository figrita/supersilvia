// SPDX-License-Identifier: AGPL-3.0-or-later

//! A still image or an animated GIF, as a texture.
//!
//! The file is an [`Asset` option](../../../docs/nodes.md#option-kinds), so it reaches the
//! node through the file button and the project's own picker, and the node is handed a path
//! it can open without ever learning where the project is.
//!
//! **The decode is off the frame thread**, the shape `video` uses: a worker reads the file and
//! sends the frames back through a channel, and `tick` polls and never waits. `status` says
//! how many frames have landed in the meantime, and it is a count rather than a fraction
//! because a GIF's header does not say how many are coming — so there is no `progress` to
//! draw a bar with. A still is one frame; a GIF is as many as it holds, each with the delay it
//! was authored with. **Time counts plays of it**, `video`'s rule: at Speed 1, or in Loop mode
//! unplugged, the animation's own pace, one play every length of its delays, and Speed 2 twice
//! as fast; in Loop mode a gear cabled in replaces it, a Ratio Gear at ×2 twice as fast and one
//! at `-×1` backwards. **Offset** is
//! added, 0 to 1 across the whole animation laid over the frames' own delays, the sum wrapping
//! at the end as a GIF does; a slow wave on it scratches around the playing animation. The
//! frame shown is a function of that sum, so the same sum is the same frame however it was
//! reached, and the node keeps no playhead of its own.
//!
//! **The GIF is why there is an image crate here at all.** PNG in and out is GStreamer's, in
//! `video/png.rs`, and `decodebin` decodes a PNG, a JPEG and a WebP too — but no machine this
//! has run on has a gif loader for it, so `decodebin` refuses `image/gif` outright. See
//! `docs/decisions.md`.

use crate::graph::NodeId;
use crate::graph::PortType::{UniformNumber, VaryingColor};
use crate::nodes::phasor;
use crate::nodes::{
    Accepts, Category, CpuDef, CpuNode, Frame, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, Pixels, TickContext,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

/// A play a cycle, at the animation's own pace — one over its length, which the node reads
/// off its delays and hands to `TickContext::cycle_at`.
const TIMING: crate::nodes::Timing = crate::nodes::Timing::periodic(1.0);

pub static DEF: NodeDef = NodeDef {
    slug: "imagegif",
    category: Category::Source,
    icon: "🖼",
    label: "Image/GIF",
    tooltip: "A still image or an animated GIF, as a texture. A GIF plays by its own frame \
              delays times its Speed, or as a gear cabled into Time plays it, and Offset is \
              added to where that is.",
    // A play a cycle, at the animation's own pace, which the node reads off its delays.
    timing: Some(TIMING),
    inputs: crate::nodes::timing::inputs![TIMING;],
    outputs: &[
        OutputDef {
            key: "output",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The same mapping as the camera and the clip: worldspace into the frame's own
            // [0,1] by its real aspect, v flipped because rows are uploaded top first, and
            // mirrored outward past its own edge by the texture's own wrap mode — which is
            // what silvia asks for here too.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "output");
                let sampler = ctx.sampler(node, "output");
                format!(
                    "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "frame",
            label: "Frame",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "frames",
            label: "Frames",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
    ],
    options: crate::nodes::timing::options![
        OptionDef {
            key: "file",
            label: "File",
            default: "",
            // An asset reference, like `video`'s: the canvas draws a file button, the project's
            // own pictures are offered first, and swapping one rebuilds no shader because the
            // picture reaches the fragment as a bound texture.
            choices: &[],
            kind: OptionKind::Asset,
            accepts: Accepts::IMAGE,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_PREVIEW,
    ],
    row_headings: crate::nodes::timing::ROW_HEADINGS,
    // The picture itself, on the body, which is the whole of what this node holds: silvia
    // draws the loaded image in the node up to about 320 across, so a row of them reads as a
    // contact sheet. It is the clip node's own region under the clip node's own heading —
    // the same texture the port hands downstream, letterboxed into a 16:9 band.
    regions: &[crate::nodes::Region::Preview("output")],
    cpu: Some(CpuDef {
        create: || Box::new(Picture::new()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The most frames one file contributes. A GIF's length is not in its header, so the only
/// bound available is a count, and a wall of frames is a wall of megabytes.
const MAX_FRAMES: usize = 1024;

/// And the bytes they may take, which is the bound that matters for a large picture: four
/// frames of 4K is already past it.
const MAX_BYTES: usize = 256 * 1024 * 1024;

/// The shortest delay a frame is held for. A GIF may say zero, which every player clamps,
/// and without a floor a large `speed` would spin the playhead for as long as the frame
/// budget let it.
const MIN_DELAY: f32 = 0.01;

/// One decoded frame and how long it is held.
struct Still {
    frame: Arc<Frame>,
    delay: f32,
}

/// How far the file has got from being named to being playable.
enum Load {
    /// No file, or a file that would not decode; `error` says which.
    Idle,
    /// A worker is reading. `tick` polls and never waits.
    Decoding {
        rx: mpsc::Receiver<Result<Vec<Still>, String>>,
        /// Frames decoded so far, written by the worker.
        done: Arc<AtomicU32>,
    },
    Ready(Vec<Still>),
}

struct Picture {
    /// The file the current load is for, so a tick can tell whether it still matches.
    wanted: Option<PathBuf>,
    load: Load,
    error: Option<String>,
    /// Which frame is showing.
    at: usize,
    /// Where Time and Offset put it, in the GIF's own seconds, for the Debug line.
    seconds: f64,
    /// Published until a real frame arrives, so a sampler always has a texture.
    black: Arc<Frame>,
}

impl Picture {
    fn new() -> Self {
        Self {
            wanted: None,
            load: Load::Idle,
            error: None,
            at: 0,
            seconds: 0.0,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
        }
    }

    /// Start reading a file on a worker.
    fn open(&mut self, source: PathBuf) {
        self.error = None;
        self.at = 0;
        let (tx, rx) = mpsc::channel();
        let done = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&done);
        let path = source.clone();
        std::thread::spawn(move || {
            // The node may be gone by now, and a send to nobody is not an error.
            let _ = tx.send(decode(&path, &counter));
        });
        self.load = Load::Decoding { rx, done };
        self.wanted = Some(source);
    }

    /// Take a finished decode, if one finished. Never waits.
    fn collect(&mut self) {
        let Load::Decoding { rx, .. } = &self.load else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(frames)) => self.load = Load::Ready(frames),
            Ok(Err(e)) => {
                self.error = Some(e);
                self.load = Load::Idle;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            // The worker died without answering, which is a failure like any other.
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("no answer from the decoder".to_string());
                self.load = Load::Idle;
            }
        }
    }

    /// The animation's length in its own seconds: every delay, each held for at least
    /// [`MIN_DELAY`]. `None` until it has decoded, and for a still, which has nowhere to go.
    fn length(&self) -> Option<f64> {
        let Load::Ready(frames) = &self.load else {
            return None;
        };
        (frames.len() > 1).then(|| {
            frames
                .iter()
                .map(|f| f64::from(f.delay.max(MIN_DELAY)))
                .sum()
        })
    }

    /// The frame showing `t` seconds into the animation, wrapped at its end: the one whose
    /// delay `t` falls inside.
    fn frame_at(&self, t: f64) -> usize {
        let Load::Ready(frames) = &self.load else {
            return 0;
        };
        let Some(length) = self.length() else {
            return 0;
        };
        let mut t = phasor::fraction(t, length);
        for (i, frame) in frames.iter().enumerate() {
            let delay = f64::from(frame.delay.max(MIN_DELAY));
            if t < delay {
                return i;
            }
            t -= delay;
        }
        // Only a rounding at the very end lands past the last delay.
        frames.len() - 1
    }
}

/// Read a file into frames. Runs on a worker; `done` is how far it has got.
fn decode(path: &Path, done: &AtomicU32) -> Result<Vec<Still>, String> {
    use image::AnimationDecoder as _;

    let named = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let reader = image::ImageReader::open(path)
        .map_err(|e| named(&e))?
        .with_guessed_format()
        .map_err(|e| named(&e))?;

    // A still is one frame held for as long as anybody looks at it.
    if reader.format() != Some(image::ImageFormat::Gif) {
        let picture = reader.decode().map_err(|e| named(&e))?.to_rgba8();
        done.store(1, Ordering::Relaxed);
        return Ok(vec![Still {
            frame: Arc::new(rgba(&picture)),
            delay: 0.0,
        }]);
    }

    let decoder =
        image::codecs::gif::GifDecoder::new(reader.into_inner()).map_err(|e| named(&e))?;
    let mut out: Vec<Still> = Vec::new();
    let mut bytes = 0;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|e| named(&e))?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = if denominator == 0 {
            0.0
        } else {
            numerator as f32 / denominator as f32 / 1000.0
        };
        let picture = frame.into_buffer();
        bytes += (picture.width() as usize) * (picture.height() as usize) * 4;
        out.push(Still {
            frame: Arc::new(rgba(&picture)),
            delay,
        });
        done.store(out.len() as u32, Ordering::Relaxed);
        if out.len() >= MAX_FRAMES || bytes >= MAX_BYTES {
            break;
        }
    }
    if out.is_empty() {
        return Err(format!("{}: no frames in it", path.display()));
    }
    Ok(out)
}

/// An `image` buffer as a [`Frame`]: the same bytes, rows top first, which both agree on.
fn rgba(picture: &image::RgbaImage) -> Frame {
    Frame {
        width: picture.width(),
        height: picture.height(),
        pixels: Pixels::Bytes(picture.as_raw().clone()),
    }
}

impl CpuNode for Picture {
    fn reset_in_place(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        self.at = 0;
        self.seconds = 0.0;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // The option holds a reference; the project says where that is. This node never
        // learns the project root.
        let file = ctx.path(id, "file");
        if self.wanted != file {
            if let Some(source) = file {
                self.open(source);
            } else {
                self.load = Load::Idle;
                self.wanted = None;
                self.error = None;
            }
        }
        self.collect();

        // Only an animation that has decoded plays, and a still has nowhere to go: plays of
        // it, at its own pace, with Offset added.
        self.seconds = match self.length() {
            Some(length) => ctx.cycle_at(id, 1.0 / length) * length,
            None => 0.0,
        };
        self.at = self.frame_at(self.seconds);

        let (frame, count) = match &self.load {
            Load::Ready(frames) => (
                frames.get(self.at).map(|s| Arc::clone(&s.frame)),
                frames.len(),
            ),
            _ => (None, 0),
        };
        ctx.publish(id, "frame", self.at as f32);
        ctx.publish(id, "frames", count as f32);
        ctx.publish_frame(
            id,
            "output",
            frame.unwrap_or_else(|| Arc::clone(&self.black)),
        );
    }

    fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What it is doing, for the bar on the file button.
    ///
    /// There is no fraction to report: a GIF says nowhere in its header how many frames it
    /// holds, so the only honest figure while decoding is how many have arrived.
    fn status(&self) -> Option<String> {
        match &self.load {
            Load::Decoding { done, .. } => {
                Some(format!("decoding, {} frames", done.load(Ordering::Relaxed)))
            }
            _ => None,
        }
    }

    fn debug(&self) -> Option<String> {
        let Load::Ready(frames) = &self.load else {
            return None;
        };
        let first = frames.first()?;
        Some(format!(
            "imagegif {}x{}, {} frames, at {}, {:.3}s in",
            first.frame.width,
            first.frame.height,
            frames.len(),
            self.at,
            self.seconds,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still(delay: f32) -> Still {
        Still {
            frame: Arc::new(Frame::solid(1, 1, [0, 0, 0, 255])),
            delay,
        }
    }

    fn animation(frames: usize, delay: f32) -> Picture {
        let mut picture = Picture::new();
        picture.load = Load::Ready((0..frames).map(|_| still(delay)).collect());
        picture
    }

    /// The frame is the one whose delay the time falls inside, wrapped at the end, and a
    /// time before the start is the end played backwards.
    #[test]
    fn the_frame_follows_the_delays() {
        let picture = animation(4, 0.25);
        assert_eq!(picture.frame_at(0.125), 0, "half a delay is the same frame");
        assert_eq!(picture.frame_at(0.25), 1, "and a whole one is the next");
        assert_eq!(
            picture.frame_at(0.875),
            3,
            "three and a half are the fourth"
        );
        assert_eq!(picture.frame_at(1.0), 0, "and the end wraps to the start");
        assert_eq!(picture.frame_at(-0.125), 3, "before the start is the end");
    }

    /// A still has one frame whatever the time, and a delay of zero — which a GIF may carry
    /// and every player clamps — is held for the shortest delay rather than skipped.
    #[test]
    fn a_still_has_nowhere_to_go_and_a_zero_delay_is_held() {
        let mut single = Picture::new();
        single.load = Load::Ready(vec![still(0.0)]);
        assert_eq!(single.frame_at(10.0), 0, "a still cannot advance");
        assert_eq!(single.length(), None);

        let picture = animation(8, 0.0);
        assert!((picture.length().unwrap() - 8.0 * f64::from(MIN_DELAY)).abs() < 1e-9);
        assert_eq!(picture.frame_at(f64::from(MIN_DELAY) * 2.5), 2);
    }

    /// The option says what it takes, and it is what `image` decodes by default.
    #[test]
    fn the_file_option_accepts_pictures() {
        let option = DEF.option("file").expect("the asset option");
        assert!(option.is_asset());
        for name in ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.webp"] {
            assert!(option.accepts.matches(name), "{name}");
        }
        for name in ["clip.mp4", "friday.ssw", "notes.txt"] {
            assert!(!option.accepts.matches(name), "{name}");
        }
    }
}
