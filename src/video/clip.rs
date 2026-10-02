// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video files: transcoded once into all-intra H.264, then played by frame number.
//!
//! A delivery file has one keyframe every few seconds and every other frame is a diff, so
//! showing frame *n* means decoding from the last keyframe forward. That is why scrubbing,
//! reverse and speed changes fall apart in a browser. Here a clip is re-encoded on import
//! with a group-of-pictures size of one — every frame a keyframe — and from then on any
//! frame costs the same to reach, in any order. Reverse play is asking for *n − 1*.
//!
//! The transcoded file lives in the project's own `cache/`, named by a fingerprint of the
//! source and the settings, so the same source is never encoded twice and the directory can
//! be deleted at any time. The original is never touched. Where that directory is comes in
//! as an argument: this module holds no path of its own.
//!
//! `Player` owns a worker thread. The node says which frame it wants; the worker seeks or
//! steps to it and publishes through a triple buffer; `tick` reads the newest frame and
//! never waits.

use crate::nodes::{Frame, Layout, Mapped, Pixels, Planes, Yuv, YuvMatrix};
use crate::video::{Delivery, propose_video_meta, sink_chain};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_pbutils as gst_pbutils;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::VideoFrameExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a clip is, before any frame of it is decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipInfo {
    pub width: u32,
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    pub frames: u64,
    pub duration: Duration,
}

impl ClipInfo {
    /// The pipeline time of frame `index`.
    fn time_of(&self, index: u64) -> gst::ClockTime {
        gst::ClockTime::from_nseconds((index as f64 / self.fps * 1e9).round() as u64)
    }

    /// The frame that a buffer stamped `pts` belongs to.
    fn index_of(&self, pts: gst::ClockTime) -> u64 {
        (pts.nseconds() as f64 * self.fps / 1e9).round() as u64
    }
}

/// The frame a position shows, `position` in clips — 0 to 1 across one — of a clip `frames`
/// long: `round(position × frames)`. Past the last frame is the first again where the clip
/// wraps, and the last where it holds.
pub fn frame_at(position: f64, frames: u64, wrap: bool) -> u64 {
    let frames = frames.max(1);
    let index = (position * frames as f64).round();
    if !index.is_finite() || index < 0.0 {
        return 0;
    }
    let index = index as u64;
    match (index >= frames, wrap) {
        (false, _) => index,
        (true, true) => index % frames,
        (true, false) => frames - 1,
    }
}

/// Ask GStreamer what a file is. Blocks for up to a few seconds on a slow disk; call it
/// from the transcode thread, not from a tick.
pub fn discover(path: &Path) -> Result<ClipInfo, String> {
    gst::init().map_err(|e| e.to_string())?;
    let uri = gst::glib::filename_to_uri(path, None).map_err(|e| e.to_string())?;
    let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(10))
        .map_err(|e| e.to_string())?;
    let info = discoverer
        .discover_uri(&uri)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let video = info
        .video_streams()
        .into_iter()
        .next()
        .ok_or_else(|| format!("{}: no video stream", path.display()))?;
    let rate = video.framerate();
    let fps = if rate.denom() > 0 && rate.numer() > 0 {
        f64::from(rate.numer()) / f64::from(rate.denom())
    } else {
        30.0
    };
    let duration = info
        .duration()
        .map_or(Duration::ZERO, |d| Duration::from_nanos(d.nseconds()));
    let frames = (duration.as_secs_f64() * fps).round() as u64;
    Ok(ClipInfo {
        width: video.width(),
        height: video.height(),
        fps,
        frames,
        duration,
    })
}

// ------------------------------------------------------------------------------- codecs

/// Which hardware encoder writes the cache, and the decoder and parser that go with it.
///
/// The cache does not care which codec it holds, only that this machine encodes and decodes
/// it in hardware and every frame can be a keyframe. So the codec is probed, not fixed: an
/// Intel part without H.264 encode still has HEVC or AV1, and an NVIDIA card has NVENC
/// rather than VA-API. The choice goes into the cache fingerprint, so two machines that
/// chose differently never read each other's entries by mistake.
///
/// Which codecs there are to probe is the machine's: [`CODECS`], from
/// [`crate::platform::video`], in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Codec {
    /// A short tag for the fingerprint and the status line.
    pub name: &'static str,
    pub(crate) encoder: &'static str,
    pub(crate) decoder: &'static str,
    pub(crate) parser: &'static str,
    /// The encoder's properties that make every frame a keyframe at constant quality.
    pub(crate) intra: &'static str,
    /// The encoder's properties for a file to send someone: constant quality, and the
    /// encoder's own keyframe interval, since nobody scrubs a delivery.
    pub(crate) delivery: &'static str,
}

pub use crate::platform::video::CODECS;

impl Codec {
    /// Are the encoder, the decoder and the parser all installed?
    pub fn available(&self) -> bool {
        gst::init().is_ok()
            && [self.encoder, self.decoder, self.parser]
                .iter()
                .all(|e| gst::ElementFactory::find(e).is_some())
    }

    /// The first codec this machine encodes and decodes in hardware, or `None` — and then
    /// the node says so on its status line, because a rig without hardware encode is not
    /// one this is for.
    pub fn probe() -> Option<Codec> {
        CODECS.iter().copied().find(Codec::available)
    }

    /// The encode half of a transcode pipeline, which is also what the tests encode a
    /// synthetic clip with, so it imports as a file this machine made would.
    pub fn encode_chain(&self) -> String {
        format!("{} {} ! {}", self.encoder, self.intra, self.parser)
    }

    /// The encode half of a render's pipeline: the same hardware, a delivery's keyframes.
    pub(super) fn delivery_chain(&self) -> String {
        format!("{} {} ! {}", self.encoder, self.delivery, self.parser)
    }
}

// ------------------------------------------------------------------------------ the cache

/// The transcode's settings: everything that changes what the cached file contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Settings {
    /// Frames taller than this are scaled down, keeping aspect.
    pub max_height: u32,
    /// What the cache entry is encoded with.
    pub codec: Codec,
}

/// FNV-1a. Stable across Rust versions, which `DefaultHasher` is not, and a cache keyed by
/// an unstable hash would silently re-encode everything after a toolchain update.
fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// What a source is known by inside `cache`: its identity and, where it has one, the
/// modification time that says it is still the same file.
///
/// A source **inside the project** — an asset, which is where every imported file lands — is
/// known by its path relative to the project root and its size. That is what lets the folder
/// be copied to another machine and the transcodes it carries be found there rather than
/// made again. A source **outside** one is known by its absolute path, its size and its
/// modification time: nothing here owns it and it may be replaced underneath us, and a
/// replaced file has to get a new entry.
///
/// `None` if the source cannot be stat'ed.
fn source_key(cache: &Path, source: &Path) -> Option<u64> {
    let meta = std::fs::metadata(source).ok()?;
    // The project root is the folder holding the cache.
    let inside = cache
        .parent()
        .and_then(|root| source.strip_prefix(root).ok());
    let mtime = if inside.is_some() {
        0
    } else {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    };
    let name = inside.unwrap_or(source).to_string_lossy().into_owned();
    let mut h = fnv1a(name.as_bytes(), 0xcbf2_9ce4_8422_2325);
    h = fnv1a(&meta.len().to_le_bytes(), h);
    h = fnv1a(&mtime.to_le_bytes(), h);
    Some(h)
}

/// Where the transcode of `source` under `settings` lives in `cache`, whether or not it
/// exists yet.
///
/// The name carries the codec, so two machines that chose different encoders keep separate
/// entries in one project folder and each re-encodes only what it cannot decode. `None` if
/// the source cannot be stat'ed.
pub fn cache_path(cache: &Path, source: &Path, settings: Settings) -> Option<PathBuf> {
    let key = source_key(cache, source)?;
    let mut h = fnv1a(&settings.max_height.to_le_bytes(), key);
    h = fnv1a(settings.codec.name.as_bytes(), h);
    Some(cache.join(format!("{key:016x}-{h:016x}.mp4")))
}

/// Every file in `cache` derived from this source, whatever settings made it.
///
/// **Everything a source produces is named after that source**, which is why the transcode
/// carries the source key as well as its own: removing an asset has to remove what was
/// derived from it, and nothing else knows the codec somebody transcoded it with two months
/// ago. Empty where the source cannot be stat'ed or the folder does not exist.
pub fn cache_entries(cache: &Path, source: &Path) -> Vec<PathBuf> {
    let Some(key) = source_key(cache, source) else {
        return Vec::new();
    };
    let prefix = format!("{key:016x}");
    let Ok(entries) = std::fs::read_dir(cache) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    out.sort();
    out
}

/// Where the decoded soundtrack of `source` lives in `cache`, whether or not it exists yet.
///
/// Keyed on the file alone, not on the transcode settings: a resolution change does not
/// change what the track sounds like, and re-decoding an hour of audio to find that out would
/// be an hour of audio decoded for nothing.
pub fn audio_cache_path(cache: &Path, source: &Path) -> Option<PathBuf> {
    let h = source_key(cache, source)?;
    Some(cache.join(format!("{h:016x}.pcm")))
}

/// Where the poster frame of `source` lives in `cache`, whether or not it exists yet.
///
/// Keyed on the file alone, like the soundtrack and for the same reason: a resolution
/// change does not change what the first second looks like.
pub fn poster_path(cache: &Path, source: &Path) -> Option<PathBuf> {
    let h = source_key(cache, source)?;
    Some(cache.join(format!("{h:016x}.poster.png")))
}

/// How wide a poster is written. Enough for a card at any zoom the project tab draws one at,
/// small enough that a folder of them is not a second copy of the media.
pub const POSTER_WIDTH: u32 = 320;

/// Where in a clip the poster is taken from. Not frame zero: a cut usually opens on black,
/// and a black card says nothing about which clip it is.
const POSTER_SECONDS: f64 = 1.0;

/// How long to wait for that frame before giving up. A decode that has not produced one
/// picture by now is one that never will.
const POSTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Decode one frame out of `source` and write it to `dest` as a PNG.
///
/// **The source, not the transcode.** A poster is wanted for every file in `assets/`,
/// including the ones no node has played yet — and those have no cache entry, so waiting for
/// one would mean a poster arriving only after somebody had already found the clip by hand.
/// `Player` is a `filesrc ! decodebin`, so it opens anything the machine can decode.
///
/// Blocking, and slow enough to matter: **call this on a worker.**
///
/// # Errors
/// The file cannot be decoded, no frame arrived inside `POSTER_TIMEOUT`, or the PNG could
/// not be written.
pub fn write_poster(source: &Path, dest: &Path) -> Result<(), String> {
    // RGBA bytes, not DMA-BUF or the decoder's own layout: this wants pixels it can read,
    // and a poster is written once.
    let mut player = Player::open_with(source, Delivery::Rgba)?;
    let index = (player.info().fps * POSTER_SECONDS).round() as u64;
    player.request(index.min(player.info().frames.saturating_sub(1)));
    let deadline = std::time::Instant::now() + POSTER_TIMEOUT;
    loop {
        if let Some(e) = player.error() {
            return Err(e);
        }
        if let Some((_, frame)) = player.latest() {
            let image = downsample(&frame, POSTER_WIDTH)
                .ok_or_else(|| format!("{}: frame is not readable bytes", source.display()))?;
            return crate::video::png::write(dest, &image);
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("{}: no frame in time", source.display()));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A frame, box-filtered down to at most `width` across, keeping its aspect.
///
/// Box rather than nearest, because a poster is a whole picture shrunk by a factor of six or
/// more and nearest at that ratio samples one pixel in thirty-six — which turns a face into
/// noise. `None` for a frame that is not packed RGBA in system memory, which a poster's own
/// player never produces.
fn downsample(frame: &Frame, width: u32) -> Option<crate::video::png::Image> {
    let src = frame.bytes()?;
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let w = width.min(frame.width).max(1);
    let h = ((u64::from(w) * u64::from(frame.height)) / u64::from(frame.width)).max(1) as u32;
    let mut out = crate::video::png::Image::new(w, h);
    for y in 0..h {
        // The source rows and columns this output pixel averages, as a half-open range.
        let y0 = (u64::from(y) * u64::from(frame.height) / u64::from(h)) as u32;
        let y1 = (((u64::from(y) + 1) * u64::from(frame.height) / u64::from(h)) as u32).max(y0 + 1);
        for x in 0..w {
            let x0 = (u64::from(x) * u64::from(frame.width) / u64::from(w)) as u32;
            let x1 =
                (((u64::from(x) + 1) * u64::from(frame.width) / u64::from(w)) as u32).max(x0 + 1);
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1.min(frame.height) {
                for sx in x0..x1.min(frame.width) {
                    let i = ((sy as usize) * (frame.width as usize) + sx as usize) * 4;
                    let Some(px) = src.get(i..i + 4) else {
                        continue;
                    };
                    for (channel, byte) in sum.iter_mut().zip(px) {
                        *channel += u32::from(*byte);
                    }
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let i = ((y as usize) * (w as usize) + x as usize) * 4;
            for (slot, channel) in out.rgba[i..i + 4].iter_mut().zip(sum) {
                *slot = (channel / n) as u8;
            }
        }
    }
    Some(out)
}

// -------------------------------------------------------------------------- transcoding

/// What a running encode shares with everyone waiting on it.
struct Job {
    progress: Mutex<f32>,
    result: Mutex<Option<Result<PathBuf, String>>>,
    pipeline: Mutex<Option<gst::Pipeline>>,
    /// How many `Transcode` handles point here. The thread and the in-flight list hold
    /// `Arc`s too, so the strong count cannot tell whether anyone is still waiting.
    handles: AtomicUsize,
    /// Set when the last handle is dropped: the thread stops and removes its partial.
    canceled: AtomicBool,
    /// The file being written, removed on cancel from whichever side gets there first —
    /// the process may exit before the thread does.
    partial: PathBuf,
    /// When the encode started. **The job's, not the handle's**: a second node that joins an
    /// encode already running is waiting on the wait that is actually happening, so its
    /// timer reads what the first one's reads.
    started: Instant,
}

/// Encodes in flight, by destination. Two nodes importing the same file share one job
/// rather than racing to write one cache entry.
static IN_FLIGHT: Mutex<Vec<(PathBuf, Arc<Job>)>> = Mutex::new(Vec::new());

/// A handle on an all-intra encode running on its own thread.
pub struct Transcode {
    job: Arc<Job>,
}

impl Transcode {
    /// Start encoding `source` into `dest`, or join the encode already doing so. Returns at
    /// once; poll `progress` and `result`.
    pub fn start(source: PathBuf, dest: PathBuf, settings: Settings) -> Self {
        let mut in_flight = IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, job)) = in_flight.iter().find(|(d, _)| *d == dest) {
            job.handles.fetch_add(1, Ordering::AcqRel);
            return Self {
                job: Arc::clone(job),
            };
        }
        let job = Arc::new(Job {
            progress: Mutex::new(0.0),
            result: Mutex::new(None),
            pipeline: Mutex::new(None),
            handles: AtomicUsize::new(1),
            canceled: AtomicBool::new(false),
            partial: dest.with_extension("part"),
            started: Instant::now(),
        });
        in_flight.push((dest.clone(), Arc::clone(&job)));
        drop(in_flight);

        let worker = Arc::clone(&job);
        std::thread::Builder::new()
            .name("transcode".into())
            .spawn(move || {
                let outcome = run_transcode(&source, &dest, settings, &worker);
                if let Ok(mut slot) = worker.result.lock() {
                    *slot = Some(outcome);
                }
                if let Ok(mut in_flight) = IN_FLIGHT.lock() {
                    in_flight.retain(|(d, _)| *d != dest);
                }
            })
            .expect("spawn transcode thread");
        Self { job }
    }

    /// 0 to 1.
    pub fn progress(&self) -> f32 {
        self.job.progress.lock().map_or(0.0, |p| *p)
    }

    /// How long the encode has been running.
    ///
    /// A percentage says how far along a wait is; only a clock says how long it has been,
    /// which is the thing a person standing over a slow import wants to know.
    pub fn elapsed(&self) -> Duration {
        self.job.started.elapsed()
    }

    /// `Some` once the thread has finished, either way.
    pub fn result(&self) -> Option<Result<PathBuf, String>> {
        self.job.result.lock().ok().and_then(|r| r.clone())
    }
}

impl Drop for Transcode {
    /// The last handle stops the pipeline, so dropping the node stops an encode nobody
    /// else is waiting on rather than orphaning it. The `IN_FLIGHT` entry holds one count.
    fn drop(&mut self) {
        if self.job.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.job.canceled.store(true, Ordering::Release);
            if let Ok(slot) = self.job.pipeline.lock()
                && let Some(p) = slot.as_ref()
            {
                // Synchronous: the sink has closed the file by the time this returns.
                let _ = p.set_state(gst::State::Null);
            }
            let _ = std::fs::remove_file(&self.job.partial);
        }
    }
}

/// Encode `source` into `dest` and report progress. Writes to a temporary name beside the
/// destination and renames at the end, so a half-written file is never mistaken for a clip.
fn run_transcode(
    source: &Path,
    dest: &Path,
    settings: Settings,
    job: &Job,
) -> Result<PathBuf, String> {
    let (progress, slot) = (&job.progress, &job.pipeline);
    let info = discover(source)?;
    if info.frames == 0 {
        return Err(format!("{}: no frames", source.display()));
    }
    let (width, height) = fit(info.width, info.height, settings.max_height);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let partial = job.partial.clone();

    // The hardware encoder, with every frame a keyframe. Constant-quantizer rate control
    // is what an intra-only cache wants: the same quality on every frame, no bit budget
    // shifting between them. `queue`s put the decode and the encode on their own threads.
    let description = format!(
        "filesrc location=\"{}\" ! decodebin name=dec \
         dec. ! queue ! videoconvert ! videoscale ! video/x-raw,width={width},height={height} \
         ! queue ! {} ! mp4mux ! filesink location=\"{}\"",
        escape(source),
        settings.codec.encode_chain(),
        escape(&partial),
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| format!("{e}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline".to_string())?;
    sink_other_streams(&pipeline, "dec");
    crate::platform::video::settle_before_eos(&pipeline);

    if let Ok(mut s) = slot.lock() {
        *s = Some(pipeline.clone());
    }
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("transcode: {e}"))?;

    let bus = pipeline.bus().ok_or("no bus")?;
    let total = info.duration.as_secs_f64().max(1e-9);
    let outcome = loop {
        if job.canceled.load(Ordering::Acquire) {
            break Err("canceled".to_string());
        }
        match bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            Some(msg) => match msg.view() {
                gst::MessageView::Eos(_) => break Ok(()),
                gst::MessageView::Error(e) => {
                    break Err(format!(
                        "{}: {}",
                        e.src().map_or_else(
                            || "pipeline".into(),
                            gstreamer::prelude::GstObjectExt::path_string
                        ),
                        e.error()
                    ));
                }
                _ => {}
            },
            None => {
                if let Some(pos) = pipeline.query_position::<gst::ClockTime>()
                    && let Ok(mut p) = progress.lock()
                {
                    *p = (pos.nseconds() as f64 / 1e9 / total).clamp(0.0, 1.0) as f32;
                }
            }
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    if let Ok(mut s) = slot.lock() {
        *s = None;
    }

    match outcome {
        Ok(()) => {
            // Another process may have finished the same entry first; theirs is as good.
            if let Err(e) = std::fs::rename(&partial, dest)
                && !dest.is_file()
            {
                return Err(format!("{}: {e}", dest.display()));
            }
            if let Ok(mut p) = progress.lock() {
                *p = 1.0;
            }
            Ok(dest.to_path_buf())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}

/// The hardware encoder's smallest frame on either side.
const MIN_SIDE: u32 = 128;

/// Scale a size down to fit `max_height`, keeping aspect and even dimensions, which the
/// encoder's chroma subsampling needs. A frame smaller than the encoder's minimum is scaled
/// up to it instead.
fn fit(width: u32, height: u32, max_height: u32) -> (u32, u32) {
    let (width, height) = (width.max(1), height.max(1));
    let max_height = max_height.max(MIN_SIDE);
    let mut scale = if height > max_height {
        f64::from(max_height) / f64::from(height)
    } else {
        1.0
    };
    let short = f64::from(width.min(height)) * scale;
    if short < f64::from(MIN_SIDE) {
        scale *= f64::from(MIN_SIDE) / short;
    }
    let w = (f64::from(width) * scale).round() as u32;
    let h = (f64::from(height) * scale).round() as u32;
    (w.max(MIN_SIDE) & !1, h.max(MIN_SIDE) & !1)
}

/// A path inside a quoted `gst-launch` string.
fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// A `decodebin` exposes a pad per stream and errors with *not-linked* if any stays
/// unlinked. Video is wired by the pipeline string; everything else — audio, subtitles —
/// goes to a `fakesink` here.
///
/// **The cache is a picture format and holds no sound.** That is deliberate, and it was a bug
/// for as long as it was merely a side effect of this function: a clip's audio was dropped
/// here and nothing ever picked it up, so a video node had no soundtrack to analyze. It is
/// picked up now, from the *source* rather than from the cache — `audio::Track` decodes the
/// original once into memory, which is seek-exact, repeatable, and does not re-encode a
/// soundtrack lossily into a file that exists to be scrubbed. Anything added here would be a
/// second copy of the same audio that only the analyzer reads.
fn sink_other_streams(pipeline: &gst::Pipeline, decodebin: &str) {
    let Some(dec) = pipeline.by_name(decodebin) else {
        return;
    };
    let pipeline = pipeline.clone();
    dec.connect_pad_added(move |_, pad| {
        if pad.is_linked() {
            return;
        }
        let is_video = pad
            .current_caps()
            .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
            .unwrap_or(false);
        if is_video {
            return;
        }
        let Ok(sink) = gst::ElementFactory::make("fakesink")
            .property("async", false)
            .property("sync", false)
            .build()
        else {
            return;
        };
        if pipeline.add(&sink).is_ok()
            && let Some(sinkpad) = sink.static_pad("sink")
            && pad.link(&sinkpad).is_ok()
        {
            let _ = sink.sync_state_with_parent();
        }
    });
}

// ------------------------------------------------------------------------------- playing

/// Sentinel for "no frame asked for yet".
const NOTHING: u64 = u64::MAX;

/// Stepping forward by up to this many frames pulls and discards rather than seeking. A
/// seek flushes the pipeline and costs more than decoding a handful of intra frames.
const STEP_LIMIT: u64 = 4;

/// A clip open for random access.
pub struct Player {
    pub delivery: Delivery,
    info: ClipInfo,
    wanted: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    pipeline: gst::Pipeline,
    output: triple_buffer::Output<Option<(u64, Arc<Frame>)>>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    current: Option<(u64, Arc<Frame>)>,
}

impl Player {
    /// Open a transcoded clip and start the worker, delivering DMA-BUFs where the machine
    /// can and bytes where it cannot.
    pub fn open(path: &Path) -> Result<Self, String> {
        if crate::platform::video::clip_dmabuf() {
            match Self::open_with(path, Delivery::DmaBuf) {
                Ok(p) => return Ok(p),
                Err(e) => log::warn!("{}: no DMA-BUF delivery ({e}); using bytes", path.display()),
            }
        }
        Self::open_with(path, Delivery::Bytes)
    }

    /// Open a transcoded clip with one delivery, and fail rather than fall back.
    pub fn open_with(path: &Path, delivery: Delivery) -> Result<Self, String> {
        let info = discover(path)?;
        if info.frames == 0 {
            return Err(format!("{}: no frames", path.display()));
        }
        // `sync=false`: frames come when asked, not when their timestamp says. One buffer
        // and no dropping: the pipeline decodes exactly one frame ahead and then waits for
        // the worker to take it, so a flush seek never has a queue of stale frames to drain.
        let description = format!(
            "filesrc location=\"{}\" ! decodebin name=dec dec. ! queue ! {}",
            escape(path),
            sink_chain(delivery)
                .ok_or("no DMA-BUF export on this machine")?
                .replace("drop=true", "drop=false")
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("{e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_string())?;
        sink_other_streams(&pipeline, "dec");
        crate::platform::video::settle_before_eos(&pipeline);
        stamp_decoder_input(&pipeline);
        let sink = pipeline
            .by_name("sink")
            .ok_or("no appsink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "sink is not an appsink".to_string())?;
        // Samples are pulled by the worker; the only callback is the allocation answer.
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(propose_video_meta)
                .build(),
        );

        start(&pipeline).map_err(|e| format!("{}: {e}", path.display()))?;

        let (input, output) = triple_buffer::TripleBuffer::new(&None).split();
        let wanted = Arc::new(AtomicU64::new(NOTHING));
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let worker = {
            let (wanted, stop, error, pipeline) = (
                Arc::clone(&wanted),
                Arc::clone(&stop),
                Arc::clone(&error),
                pipeline.clone(),
            );
            std::thread::Builder::new()
                .name("clip".into())
                .spawn(move || {
                    serve(
                        &pipeline, &sink, delivery, info, &wanted, &stop, input, &error,
                    );
                })
                .map_err(|e| e.to_string())?
        };

        Ok(Self {
            delivery,
            info,
            wanted,
            stop,
            pipeline,
            output,
            error,
            worker: Some(worker),
            current: None,
        })
    }

    pub fn info(&self) -> ClipInfo {
        self.info
    }

    /// Ask for a frame. The worker gets to it as soon as it can; `latest` says what arrived.
    pub fn request(&self, index: u64) {
        let index = index.min(self.info.frames.saturating_sub(1));
        if self.wanted.swap(index, Ordering::Release) != index
            && let Some(w) = &self.worker
        {
            w.thread().unpark();
        }
    }

    /// The newest decoded frame and its index. Never waits.
    pub fn latest(&mut self) -> Option<(u64, Arc<Frame>)> {
        if let Some((index, frame)) = self.output.read() {
            self.current = Some((*index, Arc::clone(frame)));
        }
        self.current.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Null unblocks a worker waiting in `pull_sample`.
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(w) = self.worker.take() {
            w.thread().unpark();
            let _ = w.join();
        }
    }
}

/// What a decoder's input is stamped with: the time of the frame each compressed buffer holds.
const STAMP: &str = "timestamp/x-supersilvia-source";

/// Stamp every buffer that goes into a video decoder in `pipeline`, now or added later by a
/// `decodebin`, with its own presentation time, so each decoded frame says which frame of the
/// file it is.
///
/// **The time on a decoder's output is not always the frame's own.** `GstVideoDecoder` hands
/// out timestamps from its list of pending frames and not from the frame being finished once
/// it has seen one go backwards, and then keeps doing so for thirty frames, flushes and all.
/// `vtdec` can finish a frame from before a seek after the seek's flush — the output task was
/// mid-push when the flush came and never went back for the rest of its queue — and that one
/// frame both goes backwards and is not on the list: a seek to frame 40 came back as frame 10's
/// picture stamped as 40. The stamp rides on the compressed buffer, is copied onto the picture
/// decoded from it, and names that picture whatever the decoder does to its timestamps.
fn stamp_decoder_input(pipeline: &gst::Pipeline) {
    let reference = gst::Caps::new_empty_simple(STAMP);
    pipeline.connect_deep_element_added(move |_, _, element| {
        if !element.is::<gst_video::VideoDecoder>() {
            return;
        }
        let Some(pad) = element.static_pad("sink") else {
            return;
        };
        let reference = reference.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
            if let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data
                && let Some(pts) = buffer.pts()
            {
                gst::ReferenceTimestampMeta::add(buffer.make_mut(), &reference, pts, None);
            }
            gst::PadProbeReturn::Ok
        });
    });
}

/// The time in the file of the frame `sample` holds: its decoder input's stamp where it has
/// one, and its own timestamp where it has not.
fn source_time(sample: &gst::Sample) -> gst::ClockTime {
    let Some(buffer) = sample.buffer() else {
        return gst::ClockTime::ZERO;
    };
    buffer
        .iter_meta::<gst::ReferenceTimestampMeta>()
        .find(|m| {
            m.reference()
                .structure(0)
                .is_some_and(|s| s.name() == STAMP)
        })
        .map(|m| m.timestamp())
        .or_else(|| buffer.pts())
        .unwrap_or(gst::ClockTime::ZERO)
}

/// Play a player's pipeline and wait until it is playing: a seek issued while the state
/// change is still in flight fails, so the worker may not ask for a frame before then.
///
/// **A start can fail by a race**, and is then tried again from nothing. A clip short enough
/// that the decoder reaches the end of the stream while it prerolls can lose every frame in
/// `vtdec`'s drain (see [`recover_from_short_end`]), and the decoder's *No valid frames* error
/// then fails the state change. A file that truly will not play fails every time.
fn start(pipeline: &gst::Pipeline) -> Result<(), String> {
    let mut tries = 0;
    loop {
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| e.to_string())?;
        let (change, _, _) = pipeline.state(gst::ClockTime::from_seconds(10));
        if change.is_ok() {
            return Ok(());
        }
        tries += 1;
        if tries >= END_RETRIES {
            return Err("could not start".to_string());
        }
        let _ = pipeline.set_state(gst::State::Null);
        if let Some(bus) = pipeline.bus() {
            bus.set_flushing(true);
            bus.set_flushing(false);
        }
    }
}

/// The worker: deliver whatever frame is wanted, by the cheapest route, as `delivery` makes
/// frames of the samples.
#[allow(clippy::too_many_arguments)]
fn serve(
    pipeline: &gst::Pipeline,
    sink: &gst_app::AppSink,
    delivery: Delivery,
    info: ClipInfo,
    wanted: &AtomicU64,
    stop: &AtomicBool,
    mut publish: triple_buffer::Input<Option<(u64, Arc<Frame>)>>,
    error: &Mutex<Option<String>>,
) {
    let mut delivered = NOTHING;
    // How many seeks in a row have ended short of the frame `missing`, and a frame given up
    // on, which is not asked for again until another has arrived.
    let (mut missed, mut missing) = (0, NOTHING);
    let mut abandoned = NOTHING;
    // The stream has ended since the last seek, so stepping forward has nothing to step to.
    let mut at_end = false;
    while !stop.load(Ordering::Acquire) {
        let want = wanted.load(Ordering::Acquire);
        if want == NOTHING || want == delivered || want == abandoned {
            std::thread::park_timeout(Duration::from_millis(2));
            continue;
        }

        // Forward by a few: decode through. Anything else: seek. Every frame is a keyframe,
        // so an accurate seek lands exactly and costs one frame's decode. A seek that has
        // already come back short lands earlier and decodes through, for the reason at
        // `REWIND`.
        let steps = want.wrapping_sub(delivered);
        let sequential = delivered != NOTHING && !at_end && (1..=STEP_LIMIT).contains(&steps);
        let from = if missing == want && missed > 0 {
            want.saturating_sub(REWIND)
        } else {
            want
        };
        let stepping = sequential || from < want;
        if !sequential
            && pipeline
                .seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    info.time_of(from),
                )
                .is_err()
        {
            set_error(error, format!("seek to frame {want} failed"));
            delivered = want;
            continue;
        }
        at_end = false;

        let mut got = None;
        let mut ended = false;
        loop {
            let Ok(sample) = sink.pull_sample() else {
                // Stopped, or the end of the stream. Either way there is nothing to wait for.
                if stop.load(Ordering::Acquire) {
                    return;
                }
                // Past the end: seek explicitly next time.
                delivered = NOTHING;
                ended = true;
                break;
            };
            let index = info.index_of(source_time(&sample));
            match frame_from(&sample, delivery) {
                Ok(frame) => got = Some((index, frame)),
                Err(e) => {
                    set_error(error, e);
                    break;
                }
            }
            if index >= want || !stepping {
                break;
            }
        }
        if let Some((index, frame)) = got {
            publish.write(Some((index, Arc::new(frame))));
            delivered = index;
        }
        if delivered == want {
            (missed, missing) = (0, NOTHING);
            abandoned = NOTHING;
        } else if ended {
            at_end = true;
            recover_from_short_end(pipeline);
            if missing != want {
                (missed, missing) = (0, want);
            }
            // Only a seek counts: a step forward that ran into the end asked for nothing new.
            missed += u32::from(!sequential);
            if missed >= END_RETRIES {
                set_error(error, format!("frame {want} never decoded"));
                abandoned = want;
                (missed, missing) = (0, NOTHING);
            }
        }
    }
}

/// How many seeks in a row may end short of one frame before that frame is given up on. Each
/// one loses the frame by a race rather than for a reason, and every one after the first
/// lands [`REWIND`] frames early, so three misses in a row is a frame that is not in the file.
const END_RETRIES: u32 = 3;

/// How far before a frame that came back short the next seek lands: more than twice the
/// deepest H.264 or HEVC picture buffer, sixteen frames, and one over.
///
/// A seek to one of a clip's last frames hands `vtdec` a frame or two and then the end of the
/// stream at once, and its output task — paused by the seek's flush and only asked to resume
/// by the first new frame — can still be waiting for the scheduler when the drain pauses it
/// again, so it never runs and every frame it held is lost. That is most of the losses under
/// load, not the rarer late hand-back. `vtdec` makes its input wait whenever more than twice
/// its picture buffer is queued, so a seek this far back cannot reach the end of the stream
/// until the output task has actually run.
const REWIND: u64 = 2 * 16 + 2;

/// Put the pipeline back to playing after the stream ended without the frame it was sent
/// for, so that the next seek can play.
///
/// **A decoder can lose the frames it still holds when the stream ends.** `vtdec` (GStreamer
/// 1.28's `applemedia`, unchanged on its main branch) keeps up to its picture buffer's depth
/// of frames back for reordering, so the last few frames of a clip leave it only in the
/// drain at the end of the stream — and that drain pauses the output task from outside, so a
/// frame VideoToolbox hands back during it can land after the task's last pass and be thrown
/// away. A seek to a clip's last frame came back empty about one time in twenty-five under
/// load. Asking again works, because it is a race; asking again *safely* is what this is for.
///
/// When a decoder ends a stream having output nothing since the seek, it posts *No valid
/// frames decoded* as an error, and a bin that sees an error marks its pending state change
/// failed and then ignores the sinks' preroll: the flushing seek left the pipeline paused,
/// and it stays paused, so every later seek's frame waits in the sink forever and the
/// worker blocks with it. Setting the state again clears the failure. A pipeline that is
/// already playing is not changed by it.
fn recover_from_short_end(pipeline: &gst::Pipeline) {
    if let Some(bus) = pipeline.bus() {
        while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
            if let gst::MessageView::Error(e) = message.view() {
                log::debug!("clip: the stream ended short of its frame: {}", e.error());
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Playing);
}

fn set_error(error: &Mutex<Option<String>>, e: String) {
    if let Ok(mut slot) = error.lock() {
        *slot = Some(e);
    }
}

/// Turn one sample of a pipeline delivering `delivery` into a `Frame`: for `DmaBuf`, the
/// frame the machine's zero-copy path makes of it where it makes one — a DMA-BUF's
/// descriptor, or an `IOSurface` — and otherwise the buffer in system memory mapped as it
/// lies. Either way the buffer is held rather than copied, so the memory stays valid for as
/// long as the frame does.
pub(crate) fn frame_from(sample: &gst::Sample, delivery: Delivery) -> Result<Frame, String> {
    let caps = sample.caps().ok_or("sample without caps")?;
    let buffer = sample.buffer().ok_or("sample without buffer")?;

    if delivery == Delivery::DmaBuf
        && let Some(described) = crate::platform::video::dmabuf_frame(caps, buffer)
    {
        return described;
    }
    mapped(caps, buffer)
}

/// `buffer`, whose caps are `caps`, mapped as it lies and held: the planes, their strides and
/// the color matrix the caps name.
pub(crate) fn mapped(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Result<Frame, String> {
    let info = gst_video::VideoInfo::from_caps(caps).map_err(|e| e.to_string())?;
    let (layout, swapped) = layout_of(info.format())
        .ok_or_else(|| format!("no upload for {:?} frames", info.format()))?;
    // Mapped through the video meta where there is one, so a device's or a decoder's own
    // strides and plane offsets are the ones read.
    let frame = gst_video::VideoFrame::from_buffer_readable(buffer.to_owned(), &info)
        .map_err(|_| "the buffer could not be mapped".to_string())?;
    let mut strides = [0; 3];
    for (i, s) in frame.plane_stride().iter().take(3).enumerate() {
        strides[i] = u32::try_from(*s).map_err(|_| "a negative stride".to_string())?;
    }
    if swapped {
        strides.swap(1, 2);
    }
    Ok(Frame {
        width: info.width(),
        height: info.height(),
        pixels: Pixels::Mapped(Mapped {
            layout,
            strides,
            yuv: yuv_of(&info),
            data: Arc::new(Held { frame, swapped }),
        }),
    })
}

/// The layout a GStreamer format uploads as, and whether its chroma planes are stored V
/// before U. `None` for a format the renderer has no upload for, which the sink chains never
/// let through.
fn layout_of(format: gst_video::VideoFormat) -> Option<(Layout, bool)> {
    use gst_video::VideoFormat as F;
    Some(match format {
        F::Rgba => (Layout::Rgba, false),
        F::Rgbx => (Layout::Rgbx, false),
        F::Bgra => (Layout::Bgra, false),
        F::Bgrx => (Layout::Bgrx, false),
        F::I420 => (Layout::I420, false),
        F::Yv12 => (Layout::I420, true),
        F::Y42b => (Layout::Y42b, false),
        F::Y444 => (Layout::Y444, false),
        F::Nv12 => (Layout::Nv12, false),
        F::Yuy2 => (Layout::Yuy2, false),
        F::Uyvy => (Layout::Uyvy, false),
        _ => return None,
    })
}

/// The matrix and range the caps name. GStreamer fills in its own default where the caps
/// say nothing — BT.601 below HD and BT.709 above, studio range — so this reads what it
/// decided.
fn yuv_of(info: &gst_video::VideoInfo) -> Yuv {
    let colorimetry = info.colorimetry();
    Yuv {
        matrix: match colorimetry.matrix() {
            gst_video::VideoColorMatrix::Bt709 => YuvMatrix::Bt709,
            gst_video::VideoColorMatrix::Bt2020 => YuvMatrix::Bt2020,
            _ => YuvMatrix::Bt601,
        },
        full_range: colorimetry.range() == gst_video::VideoColorRange::Range0_255,
    }
}

/// A mapped GStreamer frame, as the planes `render/` reads.
struct Held {
    frame: gst_video::VideoFrame<gst_video::video_frame::Readable>,
    swapped: bool,
}

impl Planes for Held {
    fn plane(&self, i: usize) -> &[u8] {
        let i = if self.swapped && i > 0 { 3 - i } else { i };
        self.frame.plane_data(i as u32).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of flat color, for the scaler.
    fn flat(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        Frame {
            width,
            height,
            pixels: Pixels::Bytes(
                std::iter::repeat_n(rgba, (width * height) as usize)
                    .flatten()
                    .collect(),
            ),
        }
    }

    /// The aspect is the frame's, the color survives, and a frame already small enough is
    /// not scaled up into a blur.
    #[test]
    fn a_poster_keeps_its_shape_and_its_color() {
        let out = downsample(&flat(1920, 1080, [10, 200, 30, 255]), POSTER_WIDTH).unwrap();
        assert_eq!(out.width, POSTER_WIDTH);
        assert_eq!(out.height, POSTER_WIDTH * 1080 / 1920);
        assert_eq!(&out.rgba[..4], &[10, 200, 30, 255], "flat in, flat out");

        let small = downsample(&flat(64, 64, [1, 2, 3, 255]), POSTER_WIDTH).unwrap();
        assert_eq!((small.width, small.height), (64, 64), "never enlarged");
    }

    /// Every output pixel averages the block under it, so nothing in the frame is dropped —
    /// which is the whole reason this is not nearest-neighbor.
    #[test]
    fn a_poster_averages_rather_than_samples() {
        // Two columns, black and white, scaled to one column: the answer is gray.
        let mut frame = flat(2, 1, [0, 0, 0, 255]);
        let Pixels::Bytes(bytes) = &mut frame.pixels else {
            unreachable!()
        };
        bytes[4..8].copy_from_slice(&[255, 255, 255, 255]);
        let out = downsample(&frame, 1).unwrap();
        assert_eq!(out.rgba[0], 127, "a sample would have given 0 or 255");
    }

    /// A frame with no pixels, and one that is already on the GPU: neither is a poster, and
    /// neither is a panic.
    #[test]
    fn a_frame_a_poster_cannot_be_made_from_is_none() {
        assert!(downsample(&flat(0, 0, [0; 4]), POSTER_WIDTH).is_none());
    }

    /// The machine's hardware codec pair, which the clips here are encoded with because it is
    /// what an import transcodes with — or `None`, having said the test is skipped.
    fn codec() -> Option<Codec> {
        let codec = Codec::probe();
        if codec.is_none() {
            eprintln!("no hardware codec pair here; skipping");
        }
        codec
    }

    /// A short synthetic clip on disk, encoded the way an import is: every frame a keyframe.
    fn intra_clip(dir: &Path, codec: Codec, frames: u32) -> PathBuf {
        intra_clip_sized(dir, codec, frames, 320, 240)
    }

    fn intra_clip_sized(dir: &Path, codec: Codec, frames: u32, width: u32, height: u32) -> PathBuf {
        // One file a call: two tests asking for the same length share a scratch folder, and
        // one encoding over the file the other is reading truncates it.
        static MADE: AtomicUsize = AtomicUsize::new(0);
        gst::init().unwrap();
        let n = MADE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("clip{frames}x{width}-{n}.mp4"));
        // `ball` moves every frame and `is-live=false` timestamps from zero, so frame n is
        // distinguishable from frame n+1 and the same n decodes to the same picture.
        let description = format!(
            "videotestsrc pattern=ball num-buffers={frames} \
             ! video/x-raw,width={width},height={height},framerate=30/1 ! videoconvert \
             ! {} ! mp4mux ! filesink location=\"{}\"",
            codec.encode_chain(),
            escape(&path)
        );
        let p = gst::parse::launch(&description)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        crate::platform::video::settle_before_eos(&p);
        p.set_state(gst::State::Playing).unwrap();
        let bus = p.bus().unwrap();
        // The pattern is drawn and converted on one CPU thread, about ten seconds for 900
        // frames of 720p on an M2 alone, and several times that beside the rest of the suite.
        let msg = bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(180),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
        // Stopped before any assertion, so a failure never disposes of a running encoder.
        p.set_state(gst::State::Null).unwrap();
        let msg = msg.expect("encode finished");
        assert!(
            !matches!(msg.view(), gst::MessageView::Error(_)),
            "encode failed: {msg:?}"
        );
        path
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("supersilvia-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The whole path, against a real file: decode, scale, write, and read it back.
    ///
    /// **From the source, not from a cache entry** — which is the point of the poster path.
    /// The test clip is 320 wide, so the poster is not scaled and what comes back is a
    /// picture of the frame rather than of the scaler.
    #[test]
    fn a_poster_is_one_frame_of_the_file_itself() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let source = intra_clip(&dir, codec, 60);
        let dest = dir.join("poster.png");
        write_poster(&source, &dest).expect("a clip the machine just encoded");

        let image = crate::video::png::read(&dest).expect("a PNG was written");
        assert_eq!((image.width, image.height), (320, 240));
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[..3].iter().any(|c| *c > 16)),
            "a poster taken a second in is a picture, not the black a clip opens on"
        );
    }

    /// A file that is not media at all fails rather than hanging or panicking: that is the
    /// answer `App` turns into "this one keeps its icon".
    #[test]
    fn a_file_with_no_picture_in_it_has_no_poster() {
        let dir = scratch();
        let source = dir.join("notes.txt");
        std::fs::write(&source, b"not media").unwrap();
        assert!(write_poster(&source, &dir.join("nope.png")).is_err());
    }

    fn wait_for(player: &mut Player, index: u64) -> Arc<Frame> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((got, frame)) = player.latest()
                && got == index
            {
                return frame;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "frame {index} never arrived: {:?}",
                player.error()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn discover_reads_size_rate_and_length() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let clip = intra_clip(&dir, codec, 45);
        let info = discover(&clip).unwrap();
        assert_eq!((info.width, info.height), (320, 240));
        assert!((info.fps - 30.0).abs() < 0.01);
        assert_eq!(info.frames, 45);
    }

    /// Any frame in any order. Reverse is the case a delivery codec cannot do.
    #[test]
    fn frames_arrive_in_any_order_and_the_same_index_decodes_the_same() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let clip = intra_clip(&dir, codec, 60);
        let mut player = Player::open_with(&clip, Delivery::Rgba).unwrap();

        player.request(40);
        let f40 = wait_for(&mut player, 40);
        player.request(41);
        let f41 = wait_for(&mut player, 41);
        player.request(39);
        let f39 = wait_for(&mut player, 39);
        player.request(5);
        let f5 = wait_for(&mut player, 5);
        player.request(40);
        let f40_again = wait_for(&mut player, 40);

        assert_ne!(f40.bytes(), f41.bytes(), "the ball moved between frames");
        assert_ne!(f40.bytes(), f39.bytes());
        assert_ne!(f40.bytes(), f5.bytes());
        assert_eq!(
            f40.bytes(),
            f40_again.bytes(),
            "intra frames decode deterministically"
        );
        assert_eq!(player.error(), None);
    }

    #[test]
    fn a_request_past_the_end_is_clamped() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let clip = intra_clip(&dir, codec, 30);
        let mut player = Player::open(&clip).unwrap();
        player.request(10_000);
        wait_for(&mut player, 29);
    }

    /// The import path end to end: discover, encode all-intra into the cache name, and the
    /// result opens and plays.
    #[test]
    fn a_transcode_lands_in_the_cache_and_plays() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let source = intra_clip(&dir, codec, 30);
        let settings = Settings {
            max_height: 144,
            codec,
        };
        let dest = dir.join("out.mp4");
        let job = Transcode::start(source.clone(), dest.clone(), settings);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let result = loop {
            if let Some(r) = job.result() {
                break r;
            }
            assert!(std::time::Instant::now() < deadline, "transcode hung");
            std::thread::sleep(Duration::from_millis(10));
        };
        let out = result.expect("transcode");
        assert_eq!(out, dest);
        assert!((job.progress() - 1.0).abs() < 1e-6);
        let info = discover(&out).unwrap();
        assert_eq!((info.width, info.height), (192, 144), "scaled to the cap");
        assert_eq!(info.frames, 30);
        let mut player = Player::open(&out).unwrap();
        player.request(12);
        wait_for(&mut player, 12);
    }

    #[test]
    fn the_cache_name_follows_the_source_the_settings_and_the_codec() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        let a = intra_clip(&dir, codec, 10);
        let s1 = Settings {
            max_height: 1080,
            codec: CODECS[0],
        };
        let s2 = Settings {
            max_height: 720,
            codec: CODECS[0],
        };
        let s3 = Settings {
            max_height: 1080,
            codec: CODECS[1],
        };
        let cache = dir.join("cache");
        assert_eq!(cache_path(&cache, &a, s1), cache_path(&cache, &a, s1));
        assert_ne!(cache_path(&cache, &a, s1), cache_path(&cache, &a, s2));
        assert_ne!(cache_path(&cache, &a, s1), cache_path(&cache, &a, s3));
        assert!(cache_path(&cache, &a, s1).unwrap().starts_with(&cache));
        assert_eq!(cache_path(&cache, Path::new("/nope/none.mp4"), s1), None);
    }

    /// A project folder carries its transcodes. Copied somewhere else — another disk,
    /// another machine — its assets ask for the same entry names, which is the whole reason
    /// the cache is in the folder rather than in `$XDG_CACHE_HOME`.
    #[test]
    fn a_project_that_moves_asks_for_the_same_cache_entries() {
        // Only the codec's name reaches the entry, so any listed codec will do, available or not.
        let Some(&codec) = CODECS.first() else {
            eprintln!("no codec listed here; skipping");
            return;
        };
        let dir = scratch().join("moved");
        let settings = Settings {
            max_height: 1080,
            codec,
        };
        let mut names = Vec::new();
        for project in ["one", "two"] {
            let assets = dir.join(project).join("assets");
            std::fs::create_dir_all(&assets).unwrap();
            let source = assets.join("clip.mp4");
            std::fs::write(&source, b"not really a clip, and never opened").unwrap();
            let cache = dir.join(project).join("cache");
            names.push((
                file_name(&cache_path(&cache, &source, settings).unwrap()),
                file_name(&audio_cache_path(&cache, &source).unwrap()),
            ));
        }
        assert_eq!(names[0], names[1]);
    }

    fn file_name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    /// Every codec this machine has must round-trip: encode all-intra, then reach a frame
    /// out of order. The list is what a machine without H.264 encode falls through.
    #[test]
    fn every_available_codec_encodes_intra_and_seeks() {
        let Some(first) = codec() else { return };
        let dir = scratch();
        let source = intra_clip(&dir, first, 20);
        let mut tried = 0;
        for codec in CODECS.iter().copied().filter(Codec::available) {
            let settings = Settings {
                max_height: 144,
                codec,
            };
            let dest = dir.join(format!("{}.mp4", codec.name));
            let job = Transcode::start(source.clone(), dest.clone(), settings);
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            let out = loop {
                if let Some(r) = job.result() {
                    break r.unwrap_or_else(|e| panic!("{}: {e}", codec.name));
                }
                assert!(std::time::Instant::now() < deadline, "{}: hung", codec.name);
                std::thread::sleep(Duration::from_millis(10));
            };
            let mut player = Player::open(&out).unwrap_or_else(|e| panic!("{}: {e}", codec.name));
            player.request(15);
            let a = wait_for(&mut player, 15);
            player.request(3);
            let b = wait_for(&mut player, 3);
            assert!(a != b, "{}: two frames, two pictures", codec.name);
            assert_eq!(player.error(), None, "{}", codec.name);
            println!("{}: delivered as {:?}", codec.name, player.delivery);
            tried += 1;
        }
        println!("round-tripped {tried} codecs");
        assert!(tried >= 1, "this machine has no hardware codec at all");
    }

    /// How many buffers come out of `chain` over a moving `videotestsrc`, and how many of those
    /// the parser flags as depending on another frame.
    fn frames_and_deltas(chain: &str, frames: u32) -> (u32, u32) {
        gst::init().unwrap();
        let description = format!(
            "videotestsrc pattern=ball num-buffers={frames} \
             ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert \
             ! {chain} ! appsink name=sink sync=false"
        );
        let pipeline = gst::parse::launch(&description)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let sink = pipeline
            .by_name("sink")
            .unwrap()
            .downcast::<gst_app::AppSink>()
            .unwrap();
        crate::platform::video::settle_before_eos(&pipeline);
        pipeline.set_state(gst::State::Playing).unwrap();
        let (mut out, mut deltas) = (0, 0);
        while let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_seconds(30)) {
            let flags = sample.buffer().unwrap().flags();
            out += 1;
            if flags.contains(gst::BufferFlags::DELTA_UNIT) {
                deltas += 1;
            }
        }
        assert!(
            sink.is_eos(),
            "{chain}: the encode stopped short of its end"
        );
        pipeline.set_state(gst::State::Null).unwrap();
        (out, deltas)
    }

    /// Every buffer out of each available codec's `encode_chain` is a keyframe, which is the
    /// property the cache exists for. The same encoder's delivery chain keeps its own keyframe
    /// interval and so has frames that depend on others, which says the flag is being read.
    #[test]
    fn every_frame_out_of_the_encode_chain_is_a_keyframe() {
        if codec().is_none() {
            return;
        }
        for codec in CODECS.iter().copied().filter(Codec::available) {
            let (frames, deltas) = frames_and_deltas(&codec.encode_chain(), 60);
            assert_eq!(frames, 60, "{}: one buffer a frame", codec.name);
            assert_eq!(deltas, 0, "{}: every frame a keyframe", codec.name);
            let (_, deltas) = frames_and_deltas(&codec.delivery_chain(), 60);
            assert!(
                deltas > 0,
                "{}: a delivery has frames between keyframes",
                codec.name
            );
        }
    }

    /// Dropping the only handle mid-encode stops the thread and leaves no partial behind.
    #[test]
    fn dropping_a_transcode_cancels_it_and_removes_the_partial() {
        let Some(codec) = codec() else { return };
        let dir = scratch();
        // Big enough that the encode outlives the drop below by a wide margin.
        let source = intra_clip_sized(&dir, codec, 900, 1280, 720);
        let dest = dir.join("canceled.mp4");
        let job = Transcode::start(
            source,
            dest.clone(),
            Settings {
                max_height: 144,
                codec,
            },
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(job.result().is_none(), "still running");
        drop(job);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while dest.with_extension("part").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!dest.with_extension("part").exists(), "partial removed");
        assert!(!dest.exists(), "it was canceled, not finished");
    }

    #[test]
    fn the_probe_prefers_the_first_available() {
        let Some(chosen) = codec() else { return };
        let first = CODECS.iter().copied().find(Codec::available).unwrap();
        assert_eq!(chosen, first);
    }

    #[test]
    fn fit_keeps_aspect_evenness_and_the_encoders_floor() {
        assert_eq!(fit(3840, 2160, 1080), (1920, 1080));
        assert_eq!(fit(1280, 720, 1080), (1280, 720));
        assert_eq!(fit(1001, 1001, 1080), (1000, 1000));
        assert_eq!(fit(4096, 2160, 720), (1364, 720));
        // Too small for the encoder: scaled up to its minimum side.
        assert_eq!(fit(64, 48, 1080), (170, 128));
        assert_eq!(fit(320, 240, 120), (170, 128));
    }
}
