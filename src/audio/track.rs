// SPDX-License-Identifier: AGPL-3.0-or-later

//! A file's soundtrack, decoded to mono once and held in memory.
//!
//! **Why decoded once, and why not from the transcoded cache.** The cache is an all-intra
//! video written so any frame can be drawn in any order; it carries no audio, and giving it
//! one would mean re-encoding a soundtrack lossily on top of the source for a track only the
//! analyzer ever reads. Decoding the *original* once instead buys three things a playing
//! pipeline cannot:
//!
//! - **Seek-exact.** A `video` node's position may be driven by a cable, backwards, or
//!   scrubbed. Samples indexed by time answer all of that; a second playback pipeline chasing
//!   the picture answers none of it well.
//! - **Repeatable.** Frame *n* analyzed twice gives the same numbers, which is what a render
//!   against an audio track needs and what a live device can never promise.
//! - **Sub-frame.** The whole span a frame covers is available at once, so a threshold
//!   crossing inside it is found at the sample it happened on rather than at the frame that
//!   noticed it.
//!
//! This is silvia's shadow player — a second, hidden media element analyzed independently of
//! the visible one — with the playing part taken out, because we do not need it to play.
//!
//! **The samples live beside the transcode, not on the heap.** Mono `i16` at 48 kHz is about
//! six megabytes a minute; an hour-long file is four hundred, and a VHS transfer is an
//! hour-long file. Holding that in memory per node was the first shape of this and it does
//! not survive contact with a real library. Written into the cache directory instead, it is
//! read a few kilobytes at a time as the playhead needs them — the page cache does the rest —
//! and it is decoded once *ever* rather than once per launch, which is how the video cache
//! beside it already works.

use super::{Analysis, Analyzer, FFT_SIZE};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::io::{Read as _, Seek as _, Write as _};
use std::path::Path;
use std::sync::Arc;

/// What the track is resampled to on the way in. One rate everywhere means a microphone and
/// a file are analyzed by identical code with identical constants.
pub const RATE: u32 = 48_000;

/// Where a track's samples are.
#[derive(Debug)]
enum Store {
    /// In the cache, as little-endian `i16`. What a real file uses.
    File(std::sync::Mutex<std::fs::File>),
    /// On the heap. For tests, and for a track short enough that a file would be silly.
    Memory(Vec<f32>),
}

/// A decoded soundtrack: mono at `RATE`.
#[derive(Debug)]
pub struct Track {
    store: Store,
    /// Samples, however they are stored.
    len: usize,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            store: Store::Memory(Vec::new()),
            len: 0,
        }
    }
}

/// Decodes started in this process, which names each one's temporary.
static DECODES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bytes per stored sample. `i16` rather than `f32`: it halves a long file, and sixteen bits
/// is four more than the dynamic range any of this measures.
const SAMPLE_BYTES: usize = 2;

impl Track {
    /// Decode the audio of `path` into `cache`, or open what is already there.
    ///
    /// Blocks for as long as reading the file takes, so it runs on a worker. A file with no
    /// audio track is not an error — it is a silent track, because a video without sound is an
    /// ordinary thing and the node should still work.
    ///
    /// Two nodes opening the same file at once may both decode it; each writes a temporary
    /// named for itself and renames it over the same track, so the second wastes work and
    /// both get the whole track.
    pub fn decode(path: &Path, cache: &Path) -> Result<Self, String> {
        let cached = crate::video::clip::audio_cache_path(cache, path);
        if let Some(cached) = &cached
            && cached.is_file()
        {
            return Self::open(cached);
        }

        gst::init().map_err(|e| format!("gstreamer: {e}"))?;
        let description = format!(
            "filesrc location=\"{}\" ! decodebin name=dec \
             dec. ! queue ! audioconvert ! audioresample \
             ! audio/x-raw,format=S16LE,channels=1,rate={RATE},layout=interleaved \
             ! appsink name=sink sync=false max-buffers=0",
            escape(path),
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("{e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_string())?;
        let sink = pipeline
            .by_name("sink")
            .ok_or("no appsink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "sink is not an appsink".to_string())?;
        // The video pads have to go somewhere or `decodebin` errors with *not-linked*, which
        // is the same trap the transcode falls into — and the same one that lost the audio
        // there by sinking it and never putting it back.
        sink_other_streams(&pipeline, "dec", &sink);
        let bus = pipeline.bus().ok_or("no bus")?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("decode audio: {e}"))?;

        // Written straight through to a temporary, so an hour of audio never sits in memory
        // on the way to the disk it is going to anyway.
        let Some(cached) = cached else {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(format!("{}: cannot read", path.display()));
        };
        if let Some(dir) = cached.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let partial = cached.with_extension(format!(
            "pcm.{}-{}.part",
            std::process::id(),
            DECODES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut out = std::io::BufWriter::new(
            std::fs::File::create(&partial).map_err(|e| format!("{}: {e}", partial.display()))?,
        );

        // Ends at EOS, or at the first error, which leaves no track behind.
        let mut wrote = 0usize;
        loop {
            if let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
                let _ = pipeline.set_state(gst::State::Null);
                drop(out);
                let _ = std::fs::remove_file(&partial);
                return Err(match message.view() {
                    gst::MessageView::Error(e) => format!("decode audio: {}", e.error()),
                    _ => "decode audio: failed".to_string(),
                });
            }
            let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) else {
                if sink.is_eos() {
                    break;
                }
                continue;
            };
            let Some(buffer) = sample.buffer() else {
                continue;
            };
            let Ok(map) = buffer.map_readable() else {
                continue;
            };
            // S16LE, asked for explicitly in the caps rather than in native order, so this
            // does not quietly depend on the machine.
            let bytes = map.as_slice();
            let whole = bytes.len() - bytes.len() % SAMPLE_BYTES;
            if out.write_all(&bytes[..whole]).is_err() {
                break;
            }
            wrote += whole / SAMPLE_BYTES;
        }
        let _ = pipeline.set_state(gst::State::Null);
        out.flush()
            .map_err(|e| format!("{}: {e}", partial.display()))?;
        drop(out);

        // Renamed only once whole, so an interrupted decode never leaves a short track that
        // the next launch would trust.
        std::fs::rename(&partial, &cached).map_err(|e| format!("{}: {e}", cached.display()))?;
        let _ = wrote;
        Self::open(&cached)
    }

    /// Open an already-decoded track.
    fn open(cached: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(cached).map_err(|e| format!("{}: {e}", cached.display()))?;
        let len = file
            .metadata()
            .map_err(|e| format!("{}: {e}", cached.display()))?
            .len() as usize
            / SAMPLE_BYTES;
        Ok(Self {
            store: Store::File(std::sync::Mutex::new(file)),
            len,
        })
    }

    /// A track of known samples, for tests.
    pub fn from_samples(samples: Vec<f32>) -> Self {
        Self {
            len: samples.len(),
            store: Store::Memory(samples),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Seconds of audio.
    pub fn duration(&self) -> f64 {
        self.len as f64 / f64::from(RATE)
    }

    /// The sample index a time lands on, clamped into the track.
    fn index(&self, t: f64) -> usize {
        if t <= 0.0 {
            return 0;
        }
        ((t * f64::from(RATE)) as usize).min(self.len)
    }

    /// Read samples `a..b` into `out`, which is cleared first.
    ///
    /// A read failure is silence rather than an error: the file is in a cache directory
    /// somebody may empty at any moment, and a graph that stops drawing because of that would
    /// be worse than one that goes quiet.
    fn read(&self, a: usize, b: usize, out: &mut Vec<f32>) {
        out.clear();
        if a >= b {
            return;
        }
        match &self.store {
            Store::Memory(samples) => out.extend_from_slice(&samples[a..b]),
            Store::File(file) => {
                let Ok(mut file) = file.lock() else { return };
                if file
                    .seek(std::io::SeekFrom::Start((a * SAMPLE_BYTES) as u64))
                    .is_err()
                {
                    return;
                }
                let mut bytes = vec![0u8; (b - a) * SAMPLE_BYTES];
                if file.read_exact(&mut bytes).is_err() {
                    return;
                }
                out.extend(
                    bytes
                        .as_chunks::<SAMPLE_BYTES>()
                        .0
                        .iter()
                        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0),
                );
            }
        }
    }

    /// Samples from `from` to `to` into `out`, empty when the range is backwards or outside.
    pub fn read_between(&self, from: f64, to: f64, out: &mut Vec<f32>) {
        self.read(self.index(from), self.index(to), out);
    }

    /// The samples ending at `t`, zero-padded at the start of the track.
    ///
    /// What a seek needs: a window to analyze at the destination, with nothing carried over
    /// from where the playhead was before.
    pub fn window_ending_at(&self, t: f64, out: &mut Vec<f32>) {
        let end = self.index(t);
        let start = end.saturating_sub(FFT_SIZE);
        let mut have = Vec::new();
        self.read(start, end, &mut have);
        out.clear();
        out.resize(FFT_SIZE - have.len(), 0.0);
        out.extend_from_slice(&have);
    }
}

/// A crossing placed inside the read that found it.
///
/// `Crossing::at` counts samples since the analyzer started, which is the right thing for the
/// analyzer and the wrong thing for a consumer. This is what a tick wants: seconds after the
/// start of the read, which is exactly `nodes::Event::at`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub band: u8,
    pub down: bool,
    /// Seconds after the moment this read started.
    pub at: f32,
}

/// What one move of the playhead produced.
pub struct Read {
    pub analysis: Analysis,
    /// Empty in the ordinary frame, which is why it costs no allocation there.
    pub crossings: Vec<Placed>,
    /// Crossings lost because the consumer fell more than `CROSSING_LOG` behind.
    pub dropped: u64,
}

/// A playhead over a track, and the analyzer following it.
///
/// Owns its own `Analyzer` because the exciter's running median is a property of *this*
/// reading of *this* track: two nodes on the same file, at different positions, hear
/// different things.
pub struct Reader {
    track: Arc<Track>,
    analyzer: Analyzer,
    /// Where the analysis has reached, in seconds. `None` before the first read.
    at: Option<f64>,
    /// How many crossings the consumer has taken.
    seen: u64,
    /// The samples this read is about, kept between frames so a tick allocates nothing.
    scratch: Vec<f32>,
    /// What the last read produced, so the canvas can draw between ticks.
    last: Analysis,
}

/// The longest stretch fed sample by sample before a move counts as a jump.
///
/// A quarter of a second is fifteen frames at 60 Hz: long enough that an ordinary stutter
/// still analyzes every sample it skipped over, short enough that scrubbing across a
/// three-minute clip does not decide to analyze three minutes of audio inside one frame.
const CONTINUOUS: f64 = 0.25;

impl Reader {
    pub fn new(track: Arc<Track>) -> Self {
        Self {
            analyzer: Analyzer::new(RATE),
            track,
            at: None,
            seen: 0,
            scratch: Vec::with_capacity(FFT_SIZE * 2),
            last: Analysis::default(),
        }
    }

    /// The analyzer, for setting band configuration and thresholds.
    pub fn analyzer_mut(&mut self) -> &mut Analyzer {
        &mut self.analyzer
    }

    pub fn track(&self) -> &Arc<Track> {
        &self.track
    }

    /// The most recent analysis, for a scope drawn between reads.
    pub fn last(&self) -> &Analysis {
        &self.last
    }

    /// The samples the last read analyzed.
    ///
    /// What the monitor plays, and the reason speed, direction and scrubbing all come out
    /// right without being implemented: it is not a second reading of the track, it is the
    /// same one.
    pub fn last_samples(&self) -> &[f32] {
        &self.scratch
    }

    /// Move the playhead to `t` and analyze what was passed over.
    ///
    /// A short forward move feeds every sample between where it was and where it is, so the
    /// analysis is continuous and a crossing inside the move is dated by its own sample. A
    /// backwards move or a long one is a **jump**: the analyzer is reset and given one window
    /// at the destination, and no crossing is reported for it, because arriving somewhere is
    /// not the same as something happening there.
    pub fn advance_to(&mut self, t: f64) -> Read {
        let read = self.read_at(t);
        self.last = read.analysis;
        read
    }

    fn read_at(&mut self, t: f64) -> Read {
        if self.track.is_empty() {
            return Read {
                analysis: Analysis::default(),
                crossings: Vec::new(),
                dropped: 0,
            };
        }
        let previous = self.at;
        self.at = Some(t);
        let jumped = match previous {
            None => true,
            Some(was) => t < was || t - was > CONTINUOUS,
        };

        if jumped {
            self.analyzer.reset();
            self.track.window_ending_at(t, &mut self.scratch);
            let analysis = self.analyzer.push(&self.scratch);
            // Everything the jump's own window produced is dropped: a seek is not a beat. The
            // gates go with it, so a level that is already above its threshold at the
            // destination reports a proper `Down` on the next read rather than leaving an
            // `Up` to arrive later with nothing to close.
            self.seen = analysis.crossings.total;
            self.analyzer.clear_gates();
            return Read {
                analysis,
                crossings: Vec::new(),
                dropped: 0,
            };
        }

        let was = previous.expect("only a first read has no previous, and that jumped");
        // Where the analyzer's sample count stood before this read, so a crossing's absolute
        // position can be turned into an offset inside it.
        let base = self.analyzer.samples_seen();
        // Exclusive of where we were, so no sample is analyzed twice.
        self.track.read_between(was, t, &mut self.scratch);
        let analysis = self.analyzer.push(&self.scratch);

        let (edges, dropped) = analysis.crossings.since(self.seen);
        let crossings = edges
            .map(|c| Placed {
                band: c.band,
                down: c.down,
                at: c.at.saturating_sub(base) as f32 / RATE as f32,
            })
            .collect();
        self.seen = analysis.crossings.total;
        Read {
            analysis,
            crossings,
            dropped,
        }
    }
}

/// A path inside a quoted `gst-launch` string.
fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Everything that is not audio goes to a `fakesink`, or `decodebin` errors with *not-linked*.
///
/// A file whose streams are all out with no audio among them ends `sink` at once, since
/// nothing will ever be linked to it and it would otherwise wait for a first sample for ever.
fn sink_other_streams(pipeline: &gst::Pipeline, decodebin: &str, sink: &gst_app::AppSink) {
    let Some(dec) = pipeline.by_name(decodebin) else {
        return;
    };
    let heard = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sink = sink.clone();
    let any_audio = Arc::clone(&heard);
    dec.connect_no_more_pads(move |_| {
        if !any_audio.load(std::sync::atomic::Ordering::Relaxed)
            && let Some(pad) = sink.static_pad("sink")
        {
            pad.send_event(gst::event::Eos::new());
        }
    });
    let pipeline = pipeline.clone();
    dec.connect_pad_added(move |_, pad| {
        let is_audio = pad
            .current_caps()
            .and_then(|c| c.structure(0).map(|s| s.name().starts_with("audio/")))
            .unwrap_or(false);
        if is_audio {
            heard.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if pad.is_linked() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, seconds: f64) -> Track {
        let n = (seconds * f64::from(RATE)) as usize;
        Track::from_samples(
            (0..n)
                .map(|i| {
                    let t = i as f32 / RATE as f32;
                    (2.0 * std::f32::consts::PI * hz * t).sin()
                })
                .collect(),
        )
    }

    #[test]
    fn a_track_knows_how_long_it_is_and_what_is_where() {
        let t = tone(100.0, 2.0);
        assert!((t.duration() - 2.0).abs() < 1e-6);
        let mut out = Vec::new();
        t.read_between(0.5, 1.0, &mut out);
        assert_eq!(out.len(), RATE as usize / 2);
        t.read_between(1.0, 0.5, &mut out);
        assert!(out.is_empty(), "backwards is empty");
        t.read_between(5.0, 6.0, &mut out);
        assert!(out.is_empty(), "and so is past the end");
    }

    #[test]
    fn a_window_at_the_start_is_padded_rather_than_short() {
        let t = tone(100.0, 2.0);
        let mut w = Vec::new();
        t.window_ending_at(0.0, &mut w);
        assert_eq!(w.len(), FFT_SIZE);
        assert!(w.iter().all(|s| *s == 0.0), "before the start is silence");

        t.window_ending_at(1.0, &mut w);
        assert!(w.iter().any(|s| s.abs() > 0.5), "and inside it is the tone");
    }

    #[test]
    fn reading_forward_analyzes_every_sample_it_passed_over() {
        let mut r = Reader::new(Arc::new(tone(100.0, 2.0)));
        r.advance_to(0.5);
        let before = r.analyzer.samples_seen();
        // A sixtieth of a second later.
        r.advance_to(0.5 + 1.0 / 60.0);
        let moved = r.analyzer.samples_seen() - before;
        assert!(
            (moved as i64 - 800).abs() < 8,
            "a frame of audio is 800 samples at 48 kHz: {moved}"
        );
    }

    #[test]
    fn a_jump_resets_rather_than_analyzing_everything_between() {
        let mut r = Reader::new(Arc::new(tone(100.0, 120.0)));
        r.advance_to(1.0);
        let before = r.analyzer.samples_seen();
        // Sixty seconds forward: nearly three million samples, which a frame may not spend.
        r.advance_to(61.0);
        let moved = r.analyzer.samples_seen() - before;
        assert!(
            moved <= FFT_SIZE as u64,
            "a jump analyzes one window, not the minute it skipped: {moved}"
        );
    }

    #[test]
    fn a_jump_reports_no_crossing_but_leaves_the_gates_honest() {
        let mut r = Reader::new(Arc::new(tone(100.0, 10.0)));
        r.analyzer_mut().thresholds[0] = 0.05;
        r.analyzer_mut().debounce = std::time::Duration::ZERO;
        let first = r.advance_to(1.0);
        assert!(
            first.crossings.is_empty(),
            "the first read is a jump: {:?}",
            first.crossings
        );

        // Reading on from there does report them.
        let mut fired = Vec::new();
        let mut t = 1.0;
        for _ in 0..20 {
            t += 1.0 / 60.0;
            fired.extend(r.advance_to(t).crossings);
        }
        assert!(
            !fired.is_empty(),
            "the gate the jump closed has to open again on the next read, or a later fall \
             would deliver an up that nothing opened"
        );
        assert!(
            fired.iter().all(|c| (0.0..=1.0 / 60.0).contains(&c.at)),
            "and every one is placed inside the read that found it: {fired:?}"
        );
    }

    #[test]
    fn a_silent_track_is_not_an_error() {
        let mut r = Reader::new(Arc::new(Track::default()));
        let read = r.advance_to(1.0);
        assert_eq!(read.analysis.bands, [0.0; super::super::BANDS]);
        assert!(read.crossings.is_empty());
    }
}
