// SPDX-License-Identifier: AGPL-3.0-or-later

//! Audio input, analyzed on the audio thread.
//!
//! cpal calls `Capture`'s callback with each block the device delivers. The block is mixed to
//! mono, pushed through `Analyzer`, and the result is published through a triple buffer.
//! `tick` reads the newest one without waiting: there is no lock on either side, and the
//! audio callback never allocates after start-up.
//!
//! This is the shape silvia could not have. Its `AnalyzerNode` smoothed over several
//! frames by design and was polled from one animation loop for another to consume, so a
//! transient reached a shader 50–100 ms late. Here the only smoothing is what the node asks
//! for, and the age of what a shader sees is measured rather than assumed.
//!
//! **Thresholds are crossed here, not on the frame thread.** A block is a few milliseconds
//! and a frame is sixteen, so a crossing detected where the samples are gets the sample it
//! happened on; a crossing detected where the pictures are gets rounded to the frame that
//! noticed it. Since an event carries when it happened (`nodes::action`), the first is worth
//! having and the second is what silvia was stuck with.

pub mod bands;
pub mod device;
pub mod monitor;
pub mod track;

pub use bands::{BANDS, BandConfig};
pub use device::Device;
pub use monitor::Monitor;
pub use track::{Reader, Track};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use std::sync::Arc;
use std::time::Instant;

/// FFT length. 1024 samples is 21 ms at 48 kHz: 47 Hz per bin, enough to tell a kick from
/// a bass line, and short enough that the analysis is not itself a smoothing.
pub const FFT_SIZE: usize = 1024;

/// How often the analysis runs, in samples. `FFT_SIZE / 4` is 5.3 ms at 48 kHz — about three
/// analyzes per displayed frame, and about 187 a second.
///
/// A **fixed hop rather than one analysis per delivered block**, so the rate does not depend
/// on how a device happens to chunk its callbacks. That matters twice: the running median
/// then looks back over a fixed span of *time* rather than a fixed number of blocks, and a
/// microphone and a decoded track analyze identically, which is what makes the two sources
/// interchangeable at the port level.
pub const HOP: usize = FFT_SIZE / 4;

/// Buckets in the published spectrum — one per column a scope draws.
///
/// **Log-spaced, not the FFT's own bins.** An FFT bin is a fixed number of hertz wide, so on
/// the logarithmic axis a spectrum is drawn on, the bottom three decades — where all the
/// music is — collapse into the first bin or two and the top decade gets everything else.
/// Bucketing by the axis the plot uses instead means every column is a column of the picture,
/// and a bass line is as legible as a hi-hat.
pub const SCOPE_BINS: usize = 128;

/// The frequency range the buckets span. The same ends the scope's axis has, because they are
/// the same axis.
pub const SCOPE_LO_HZ: f32 = 20.0;
pub const SCOPE_HI_HZ: f32 = 20_000.0;

/// Samples in the published waveform, which becomes a 512x1 texture an oscilloscope
/// samples. silvia's number, and it is the right one: a line drawn across a node is not
/// worth more resolution than this.
pub const WAVEFORM_LEN: usize = 512;

/// The band index a volume crossing carries. Volume is not a band — it is the whole signal —
/// but it thresholds identically, so it rides in the same log.
pub const VOLUME_BAND: u8 = BANDS as u8;

/// How many crossings are kept for the frame thread to collect.
///
/// A block is a few milliseconds and a frame is sixteen, so four or five arrive per frame at
/// the very most. Sixteen is room to spare, and it is a fixed array because the audio
/// callback may not allocate.
pub const CROSSING_LOG: usize = 16;

/// One threshold crossing, and the sample it happened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    /// Which threshold: a band index, or `VOLUME_BAND`.
    pub band: u8,
    /// True when the level rose through the threshold, false when it fell back.
    pub down: bool,
    /// Samples since the stream started. What makes this better than a frame number.
    pub at: u64,
}

/// The recent crossings, and how many there have ever been.
///
/// A ring rather than a queue, because this rides inside `Analysis` through the triple buffer
/// and the frame thread only ever sees the newest one. The frame thread remembers `total` and
/// takes the difference, so a crossing is delivered exactly once — and if it ever fell far
/// enough behind to lose one, `since` says so rather than pretending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossings {
    log: [Crossing; CROSSING_LOG],
    pub total: u64,
}

impl Default for Crossings {
    fn default() -> Self {
        Self {
            log: [Crossing {
                band: 0,
                down: false,
                at: 0,
            }; CROSSING_LOG],
            total: 0,
        }
    }
}

impl Crossings {
    fn push(&mut self, crossing: Crossing) {
        self.log[(self.total as usize) % CROSSING_LOG] = crossing;
        self.total += 1;
    }

    /// Every crossing since the consumer last looked, oldest first.
    ///
    /// `dropped` is how many were overwritten before they could be collected, which is only
    /// possible if the frame thread stalled for longer than sixteen audio blocks.
    pub fn since(&self, seen: u64) -> (impl Iterator<Item = Crossing> + '_, u64) {
        let first = seen.max(self.total.saturating_sub(CROSSING_LOG as u64));
        let dropped = first.saturating_sub(seen);
        let range = first..self.total;
        (
            range.map(move |i| self.log[(i as usize) % CROSSING_LOG]),
            dropped,
        )
    }
}

/// One analysis of the most recent block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    /// Root mean square of the last block, 0 for silence and about 0.7 for a full-scale sine.
    pub rms: f32,
    /// Largest absolute sample in the last block.
    pub peak: f32,
    /// What a graph sees: each band's own level.
    pub bands: [f32; BANDS],
    /// The spectrum, 0 to 1 a bin. What the scope draws behind the band markers.
    pub spectrum: [f32; SCOPE_BINS],
    /// The newest samples, `128` at silence. Uploaded as a 512x1 texture for an
    /// oscilloscope, which is why it is bytes and not floats.
    pub waveform: [u8; WAVEFORM_LEN],
    /// When this was written, on the audio thread. `age` is what a consumer wants.
    pub published: Option<Instant>,
    /// Samples seen since the stream started.
    pub samples: u64,
    /// Thresholds crossed, with the sample each happened on.
    pub crossings: Crossings,
}

impl Default for Analysis {
    fn default() -> Self {
        Self {
            rms: 0.0,
            peak: 0.0,
            bands: [0.0; BANDS],
            spectrum: [0.0; SCOPE_BINS],
            waveform: [128; WAVEFORM_LEN],
            published: None,
            samples: 0,
            crossings: Crossings::default(),
        }
    }
}

impl Analysis {
    /// How long ago this was published, or `None` if nothing has been.
    pub fn age(&self) -> Option<std::time::Duration> {
        self.published.map(|t| t.elapsed())
    }
}

/// Windowed FFT over a ring of the most recent `FFT_SIZE` mono samples. Pure: no device,
/// no threads, so it is what the tests exercise.
pub struct Analyzer {
    sample_rate: f32,
    fft: Arc<dyn realfft::RealToComplex<f32>>,
    window: Vec<f32>,
    ring: Vec<f32>,
    /// Where the next sample lands in `ring`.
    head: usize,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    samples: u64,
    /// The DC blocker's state: the previous raw sample and the previous filtered one.
    dc_in: f32,
    dc_out: f32,
    /// Each bin as a fraction of the decibel window, smoothed between analyzes the way an
    /// `AnalyzerNode` smooths: without it a band flickers at the block rate.
    smoothed_bins: Vec<f32>,
    /// Where each band listens. Set from the node's controls.
    pub config: [BandConfig; BANDS],
    /// The level each band, and the volume, fires at. Above 1 is never.
    pub thresholds: [f32; BANDS + 1],
    gates: [bool; BANDS + 1],
    /// The sample each threshold last fired down on, for the hold-off.
    last_fired: [u64; BANDS + 1],
    /// How long a threshold ignores a second crossing after firing.
    pub debounce: std::time::Duration,
    crossings: Crossings,
    /// Samples since the last analysis step.
    since_step: usize,
    /// What the last step computed, republished until the next one.
    bands: [f32; BANDS],
    scope_bins: [f32; SCOPE_BINS],
}

/// Pole of the DC blocker. At 48 kHz this is a corner near 25 Hz: an offset a microphone
/// or its ADC adds reads as level otherwise, and never goes away.
const DC_POLE: f32 = 0.9967;

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let n = FFT_SIZE as f32;
        // Hann. Its coherent gain is 0.5, which `magnitude` divides back out.
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n).cos())
            .collect();
        Self {
            sample_rate: sample_rate as f32,
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            ring: vec![0.0; FFT_SIZE],
            head: 0,
            samples: 0,
            dc_in: 0.0,
            dc_out: 0.0,
            smoothed_bins: vec![0.0; FFT_SIZE / 2 + 1],
            config: bands::DEFAULT,
            // Above one, so nothing fires until a hand puts a threshold somewhere. A graph
            // that opened with three events already firing would be a graph nobody asked for.
            thresholds: [f32::INFINITY; BANDS + 1],
            gates: [false; BANDS + 1],
            last_fired: [0; BANDS + 1],
            debounce: std::time::Duration::from_millis(50),
            crossings: Crossings::default(),
            since_step: 0,
            bands: [0.0; BANDS],
            scope_bins: [0.0; SCOPE_BINS],
        }
    }

    /// How many samples have been pushed through since this analyzer was made.
    pub fn samples_seen(&self) -> u64 {
        self.samples
    }

    /// Close every threshold gate without reporting it.
    ///
    /// For the frame a seek lands on: the destination's level is not news, but the gates must
    /// not be left open by it either, or the next fall would deliver an `Up` that no `Down`
    /// ever opened. Closed here, the next analysis reports the crossing properly.
    pub fn clear_gates(&mut self) {
        self.gates = [false; BANDS + 1];
    }

    /// Forget the window and everything measured from it. For a seek: what came before a
    /// jump is not what the track was doing before the place it jumped to.
    pub fn reset(&mut self) {
        self.smoothed_bins.fill(0.0);
        self.ring.fill(0.0);
        self.gates = [false; BANDS + 1];
        self.bands = [0.0; BANDS];
        self.scope_bins = [0.0; SCOPE_BINS];
    }

    /// Push one block of mono samples. Allocates nothing.
    ///
    /// The analysis runs every `HOP` samples, however the device chunks its callbacks, so a
    /// block may produce several steps or none. What comes back is always the newest values —
    /// a block shorter than the hop republishes the last ones rather than a gap.
    pub fn push(&mut self, block: &[f32]) -> Analysis {
        let mut sum_sq = 0.0f32;
        let mut peak = 0.0f32;
        for &raw in block {
            // y[n] = x[n] - x[n-1] + p * y[n-1]
            let s = raw - self.dc_in + DC_POLE * self.dc_out;
            self.dc_in = raw;
            self.dc_out = s;
            sum_sq += s * s;
            peak = peak.max(s.abs());
            self.ring[self.head] = s;
            self.head = (self.head + 1) % FFT_SIZE;
            self.samples += 1;

            self.since_step += 1;
            if self.since_step >= HOP {
                self.since_step = 0;
                self.step();
            }
        }
        let rms = if block.is_empty() {
            0.0
        } else {
            (sum_sq / block.len() as f32).sqrt()
        };
        // Volume thresholds against the block's own loudness, which is what a hand on a
        // meter is looking at.
        self.cross(BANDS, rms, self.samples);

        let mut waveform = [128u8; WAVEFORM_LEN];
        for (i, slot) in waveform.iter_mut().enumerate() {
            // The newest `WAVEFORM_LEN` samples, oldest first.
            let from = (self.head + FFT_SIZE - WAVEFORM_LEN.min(FFT_SIZE) + i) % FFT_SIZE;
            *slot = (self.ring[from].mul_add(127.0, 128.0)).clamp(0.0, 255.0) as u8;
        }

        Analysis {
            rms,
            peak,
            bands: self.bands,
            spectrum: self.scope_bins,
            waveform,
            published: Some(Instant::now()),
            samples: self.samples,
            crossings: self.crossings,
        }
    }

    /// One analysis of the window ending at the current sample: the FFT, the three bands,
    /// and any threshold they crossed.
    fn step(&mut self) {
        // Oldest sample first, windowed.
        for (i, slot) in self.input.iter_mut().enumerate() {
            *slot = self.ring[(self.head + i) % FFT_SIZE] * self.window[i];
        }
        // The only failure is a length mismatch, and the vectors came from the planner.
        let _ =
            self.fft
                .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch);

        // Each bin as a fraction of the decibel window, smoothed. Measuring in decibels is
        // what makes the ported band constants mean what they meant: they were tuned against
        // a browser's byte spectrum, which is this domain.
        for k in 0..self.smoothed_bins.len() {
            let now = bands::to_db_fraction(self.magnitude(k));
            let s = &mut self.smoothed_bins[k];
            *s = *s * bands::BIN_SMOOTHING + now * (1.0 - bands::BIN_SMOOTHING);
        }

        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        let bin_count = self.smoothed_bins.len();
        for b in 0..BANDS {
            let cfg = self.config[b];
            let range = cfg.bins(bin_hz, bin_count);
            self.bands[b] = bands::level(&self.smoothed_bins, range, &cfg);
        }

        // The spectrum a scope draws, bucketed onto its own logarithmic axis. Each bucket
        // takes the loudest bin under it rather than the mean, because a bar chart that
        // averaged them would lose the peak a bar chart is for — and at the bottom of the
        // range a bucket is narrower than a bin, so it takes the one it falls inside.
        let (lo, hi) = (SCOPE_LO_HZ.ln(), SCOPE_HI_HZ.ln());
        for (i, slot) in self.scope_bins.iter_mut().enumerate() {
            let f0 = (lo + (hi - lo) * (i as f32 / SCOPE_BINS as f32)).exp();
            let f1 = (lo + (hi - lo) * ((i + 1) as f32 / SCOPE_BINS as f32)).exp();
            let from = ((f0 / bin_hz) as usize).clamp(1, bin_count - 1);
            let to = ((f1 / bin_hz).ceil() as usize).clamp(from + 1, bin_count);
            *slot = self.smoothed_bins[from..to]
                .iter()
                .copied()
                .fold(0.0, f32::max);
        }

        // Against what the graph sees, which is what the threshold on the meter is drawn
        // over.
        let at = self.samples;
        for b in 0..BANDS {
            self.cross(b, self.bands[b], at);
        }
    }

    /// Compare one level against its threshold and log the edge if it crossed.
    ///
    /// Rising is held off by `debounce`, so a level hovering on the threshold does not
    /// machine-gun; falling never is, because a gate that would not close is worse than one
    /// that closes early — an envelope downstream would be stuck open.
    fn cross(&mut self, index: usize, level: f32, at: u64) {
        let threshold = self.thresholds[index];
        let over = level >= threshold && threshold.is_finite();
        if over == self.gates[index] {
            return;
        }
        if over {
            let hold = (self.debounce.as_secs_f32() * self.sample_rate) as u64;
            if at.saturating_sub(self.last_fired[index]) < hold {
                return;
            }
            self.last_fired[index] = at;
        }
        self.gates[index] = over;
        self.crossings.push(Crossing {
            band: index as u8,
            down: over,
            at,
        });
    }

    /// Magnitude of bin `k`, scaled so a full-scale sine on that bin reads 1.
    fn magnitude(&self, k: usize) -> f32 {
        // Two-sided to one-sided, and the window's coherent gain.
        self.spectrum[k].norm() * 2.0 / (FFT_SIZE as f32 * 0.5)
    }
}

/// What one node's audio looks like right now, for the scope the canvas draws.
///
/// Gathered from the node rather than read out of `audio/` directly, because a `video` node
/// reads a decoded file and `audioin` reads a device: the scope should not know which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scope {
    pub spectrum: [f32; SCOPE_BINS],
    /// What the graph sees, band by band. The meter's fill.
    pub levels: [f32; BANDS],
    /// Where each band's threshold sits, 0 to 1. Drawn on the meter as the port that fires it.
    pub thresholds: [f32; BANDS],
    /// Where each band listens, for the markers over the spectrum.
    pub config: [BandConfig; BANDS],
    /// The sample rate the spectrum was measured at.
    pub sample_rate: f32,
}

impl Scope {
    /// Build one from an analysis and the tuning that produced it.
    pub fn new(
        analysis: &Analysis,
        config: [BandConfig; BANDS],
        thresholds: [f32; BANDS],
        sample_rate: f32,
    ) -> Self {
        Self {
            spectrum: analysis.spectrum,
            levels: analysis.bands,
            thresholds,
            config,
            sample_rate,
        }
    }

    /// The frequency at the center of spectrum bucket `i`.
    ///
    /// The buckets are log-spaced across `SCOPE_LO_HZ..SCOPE_HI_HZ`, which is the axis the
    /// scope draws, so a bucket is a column and this is only needed to say which band owns it.
    pub fn bucket_hz(i: usize) -> f32 {
        let (lo, hi) = (SCOPE_LO_HZ.ln(), SCOPE_HI_HZ.ln());
        (lo + (hi - lo) * ((i as f32 + 0.5) / SCOPE_BINS as f32)).exp()
    }
}

/// The levels the audio thread compares against, written by the frame thread.
///
/// Atomics rather than a lock, because the callback may not block and a threshold that
/// arrives one block late is not worth a mutex. Stored as bits: `f32` has no atomic.
#[derive(Debug)]
pub struct Thresholds([std::sync::atomic::AtomicU32; BANDS + 1]);

/// Where each band listens, written by the frame thread and read by the audio one.
///
/// The same atomics-not-a-lock reasoning as `Thresholds`: the callback may not block, and a
/// band that starts listening one block late is a band nobody noticed was late.
#[derive(Debug)]
pub struct Tuning([[std::sync::atomic::AtomicU32; 2]; BANDS]);

impl Default for Tuning {
    fn default() -> Self {
        Self(std::array::from_fn(|b| {
            [
                std::sync::atomic::AtomicU32::new(bands::DEFAULT[b].freq.to_bits()),
                std::sync::atomic::AtomicU32::new(bands::DEFAULT[b].q.to_bits()),
            ]
        }))
    }
}

impl Tuning {
    pub fn set(&self, config: [BandConfig; BANDS]) {
        for (slot, cfg) in self.0.iter().zip(config) {
            slot[0].store(cfg.freq.to_bits(), std::sync::atomic::Ordering::Relaxed);
            slot[1].store(cfg.q.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn read(&self, into: &mut [BandConfig; BANDS]) {
        for (slot, cfg) in self.0.iter().zip(into.iter_mut()) {
            cfg.freq = f32::from_bits(slot[0].load(std::sync::atomic::Ordering::Relaxed));
            cfg.q = f32::from_bits(slot[1].load(std::sync::atomic::Ordering::Relaxed));
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        // Infinity: nothing fires until a hand puts a level somewhere.
        Self(std::array::from_fn(|_| {
            std::sync::atomic::AtomicU32::new(f32::INFINITY.to_bits())
        }))
    }
}

impl Thresholds {
    pub fn set(&self, levels: [f32; BANDS + 1]) {
        for (slot, level) in self.0.iter().zip(levels) {
            slot.store(level.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn read(&self) -> [f32; BANDS + 1] {
        std::array::from_fn(|i| {
            f32::from_bits(self.0[i].load(std::sync::atomic::Ordering::Relaxed))
        })
    }
}

/// What is holding the device open. Dropping any one stops it.
///
/// More than one because a monitor — what is already going to the speakers — is not an ALSA
/// device and so is not a cpal device, and on a Mac is not a device at all but a process tap.
/// See [`device`].
enum Stream {
    /// Held for its lifetime only: cpal stops the stream on drop, so nothing reads it.
    Cpal(#[allow(dead_code)] cpal::Stream),
    Gst(gstreamer::Pipeline),
    /// Held for its lifetime only: the tap stops on drop.
    Tap(#[allow(dead_code)] crate::platform::audio::Tap),
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Self::Gst(pipeline) = self {
            use gstreamer::prelude::ElementExt as _;
            let _ = pipeline.set_state(gstreamer::State::Null);
        }
    }
}

/// An open input stream. Dropping it stops the device.
pub struct Capture {
    _stream: Stream,
    output: triple_buffer::Output<Analysis>,
    /// Shared with the audio thread, which reads them at the top of every callback.
    thresholds: Arc<Thresholds>,
    config: Arc<Tuning>,
    /// This device's channel into the output. Silent until something turns it up, which
    /// matters more here than anywhere else: a microphone monitored through the speakers it
    /// is sitting next to is a howl, and the first person to meet that will be on stage.
    monitor: monitor::Send,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// The half of a capture that does not depend on who delivers the samples.
struct Feed {
    output: triple_buffer::Output<Analysis>,
    thresholds: Arc<Thresholds>,
    config: Arc<Tuning>,
    monitor: monitor::Send,
}

/// Build the analysis half, and the closure a backend hands blocks of interleaved samples to.
///
/// The closure runs on somebody else's thread — cpal's callback or GStreamer's streaming
/// thread — so everything it needs is moved into it, and after the first few blocks it
/// allocates nothing.
fn feed(sample_rate: u32, channels: u16) -> (Feed, impl FnMut(&[f32]) + Send + 'static) {
    let (input, output) = triple_buffer::TripleBuffer::new(&Analysis::default()).split();
    let thresholds = Arc::new(Thresholds::default());
    let levels = Arc::clone(&thresholds);
    let config = Arc::new(Tuning::default());
    let tuning = Arc::clone(&config);
    let monitor = Monitor::shared().open();
    let send = monitor.clone_handle();
    let mut analyzer = Analyzer::new(sample_rate);
    // Sized for the largest block a device is likely to hand over, so the callback does
    // not allocate; a bigger one grows it once.
    let mut mono: Vec<f32> = Vec::with_capacity(4096);
    let mut input = input;
    let analyze = move |samples: &[f32]| {
        analyzer.thresholds = levels.read();
        tuning.read(&mut analyzer.config);
        mono.clear();
        let ch = usize::from(channels).max(1);
        for frame in samples.chunks(ch) {
            mono.push(frame.iter().sum::<f32>() / ch as f32);
        }
        // The same mono mix the analyzer sees, so what you hear is what is measured.
        send.push(&mono, sample_rate);
        input.write(analyzer.push(&mono));
    };
    (
        Feed {
            output,
            thresholds,
            config,
            monitor,
        },
        analyze,
    )
}

impl Capture {
    /// Open whichever device this names and start analyzing.
    pub fn open_device(device: &device::Device) -> Result<Self, String> {
        match device {
            device::Device::Default => Self::open(),
            device::Device::Pulse { name } => Self::open_pulse(name),
        }
    }

    /// Open a named source through GStreamer, by the element [`crate::platform::audio`]
    /// names — `pulsesrc` on Linux, `osxaudiosrc` on a Mac — or, for a name the machine reads
    /// without GStreamer, the Mac's loopback, by its tap.
    ///
    /// The caps are pinned to mono float at [`track::RATE`] so the analyzer sees exactly what
    /// it sees from a file and from a microphone: one rate everywhere is what makes the three
    /// sources interchangeable at the port level, and it means the sample rate is known here
    /// rather than after caps negotiation.
    fn open_pulse(name: &str) -> Result<Self, String> {
        if let Some(tap) = crate::platform::audio::Tap::open(name) {
            return Self::open_tap(name, tap.map_err(|e| format!("{name}: {e}"))?);
        }
        let source = crate::platform::audio::element(name).map_err(|e| format!("{name}: {e}"))?;
        Self::open_gst(name, &source)
    }

    /// Open the sound of the NDI source NDI names `name`, from a receiver of its own that asks
    /// for no picture — what the Main Input's *Video source* sound is when its video is one.
    pub fn open_ndi(name: &str) -> Result<Self, String> {
        let source = crate::video::ndi::audio_head(name)?;
        Self::open_gst(name, &source)
    }

    /// Analyze what `source`, the head of a GStreamer pipeline, hands over, as `name`.
    fn open_gst(name: &str, source: &str) -> Result<Self, String> {
        use gstreamer::prelude::*;
        let rate = track::RATE;
        gstreamer::init().map_err(|e| format!("{name}: {e}"))?;
        let (feed, mut analyze) = feed(rate, 1);
        let pipeline = gstreamer::parse::launch(&format!(
            "{source} \
             ! audioconvert ! audioresample \
             ! audio/x-raw,format=F32LE,channels=1,rate={rate},layout=interleaved \
             ! appsink name=sink sync=false max-buffers=4 drop=true"
        ))
        .map_err(|e| format!("{name}: {e}"))?
        .downcast::<gstreamer::Pipeline>()
        .map_err(|_| format!("{name}: not a pipeline"))?;
        let sink = pipeline
            .by_name("sink")
            .ok_or_else(|| format!("{name}: no sink"))?
            .downcast::<gstreamer_app::AppSink>()
            .map_err(|_| format!("{name}: sink is not an appsink"))?;
        // Grown once and reused: this runs on GStreamer's streaming thread for every block,
        // and the rule there is cpal's rule — do not allocate per callback.
        let mut samples: Vec<f32> = Vec::with_capacity(4096);
        sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gstreamer::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;
                    let map = buffer
                        .map_readable()
                        .map_err(|_| gstreamer::FlowError::Error)?;
                    // Decoded a word at a time rather than cast: the caps say F32LE, and
                    // saying so here is both exact and free of an `unsafe` the crate forbids.
                    samples.clear();
                    samples.extend(
                        map.as_slice()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|w| f32::from_le_bytes(*w)),
                    );
                    analyze(&samples);
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );
        pipeline
            .set_state(gstreamer::State::Playing)
            .map_err(|e| format!("{name}: {e}"))?;
        Ok(Self {
            _stream: Stream::Gst(pipeline),
            output: feed.output,
            thresholds: feed.thresholds,
            config: feed.config,
            monitor: feed.monitor,
            device: name.to_string(),
            sample_rate: rate,
            channels: 1,
        })
    }

    /// Start a tap the machine opened by name, which hands the analyzer mono blocks at its own
    /// rate from its own thread, as cpal's callback does.
    fn open_tap(name: &str, mut tap: crate::platform::audio::Tap) -> Result<Self, String> {
        let rate = tap.rate();
        let (feed, analyze) = feed(rate, 1);
        tap.start(analyze).map_err(|e| format!("{name}: {e}"))?;
        Ok(Self {
            _stream: Stream::Tap(tap),
            output: feed.output,
            thresholds: feed.thresholds,
            config: feed.config,
            monitor: feed.monitor,
            device: name.to_string(),
            sample_rate: rate,
            channels: 1,
        })
    }

    /// Open the default input device and start analyzing.
    pub fn open() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no audio input device".to_string())?;
        let name = device
            .description()
            .map_or_else(|_| "unnamed".to_string(), |d| d.to_string());
        let config = device
            .default_input_config()
            .map_err(|e| format!("{name}: {e}"))?;
        let sample_rate = config.sample_rate();
        let channels = config.channels();
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();

        let (feed, mut analyze) = feed(sample_rate, channels);
        let err = move |e| log::error!("audio input: {e}");

        let stream = match format {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(config, move |data: &[f32], _| analyze(data), err, None)
            }
            cpal::SampleFormat::I16 => {
                let mut buf = Vec::with_capacity(8192);
                device.build_input_stream(
                    config,
                    move |data: &[i16], _| {
                        buf.clear();
                        buf.extend(data.iter().map(|s| f32::from(*s) / 32768.0));
                        analyze(&buf);
                    },
                    err,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut buf = Vec::with_capacity(8192);
                device.build_input_stream(
                    config,
                    move |data: &[u16], _| {
                        buf.clear();
                        buf.extend(data.iter().map(|s| (f32::from(*s) - 32768.0) / 32768.0));
                        analyze(&buf);
                    },
                    err,
                    None,
                )
            }
            other => return Err(format!("{name}: unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        stream.play().map_err(|e| format!("{name}: {e}"))?;

        Ok(Self {
            _stream: Stream::Cpal(stream),
            output: feed.output,
            thresholds: feed.thresholds,
            config: feed.config,
            monitor: feed.monitor,
            device: name,
            sample_rate,
            channels,
        })
    }

    /// The newest analysis. Never waits.
    pub fn latest(&mut self) -> Analysis {
        *self.output.read()
    }

    /// Set the level each threshold fires at. Read by the audio thread on its next block.
    pub fn set_thresholds(&self, levels: [f32; BANDS + 1]) {
        self.thresholds.set(levels);
    }

    /// Set where each band listens. Read by the audio thread on its next block.
    pub fn set_config(&self, config: [BandConfig; BANDS]) {
        self.config.set(config);
    }

    /// How loudly this device plays back through the monitor. Zero is off.
    pub fn set_monitor(&self, volume: f32) {
        self.monitor.set_volume(volume);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32, n: usize, amplitude: f32) -> Vec<f32> {
        sine_from(hz, rate, n, amplitude, 0)
    }

    /// A tone continuing from sample `start`.
    ///
    /// Phase continuity across pushes is load-bearing in any test that pushes more than once:
    /// restarting at zero puts a step discontinuity on every block boundary, and a step is
    /// broadband — it lands in every band and swamps what the test meant to measure.
    fn sine_from(hz: f32, rate: u32, n: usize, amplitude: f32, start: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = (start + i) as f32 / rate as f32;
                amplitude * (2.0 * std::f32::consts::PI * hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn silence_analyzes_to_nothing() {
        let mut a = Analyzer::new(48_000);
        let r = a.push(&vec![0.0; FFT_SIZE]);
        assert_eq!(r.rms, 0.0);
        assert_eq!(r.peak, 0.0);
        assert_eq!(r.bands, [0.0; 3]);
        assert!(r.published.is_some());
    }

    /// A constant is not sound. Without this a microphone's offset reads as a steady level
    /// that never goes away.
    #[test]
    fn a_dc_offset_settles_to_silence() {
        let mut a = Analyzer::new(48_000);
        let mut r = Analysis::default();
        for _ in 0..40 {
            r = a.push(&vec![0.3; 1024]);
        }
        assert!(r.rms < 0.01, "rms {}", r.rms);
        assert!(r.bands[0] < 0.02, "low band {}", r.bands[0]);
    }

    #[test]
    fn a_full_scale_sine_has_the_expected_rms_and_peak() {
        let mut a = Analyzer::new(48_000);
        let r = a.push(&sine(1000.0, 48_000, FFT_SIZE, 1.0));
        // The DC blocker ripples a little at the start of a signal; a control value does
        // not care about two percent.
        assert!((r.rms - 0.707).abs() < 0.03, "rms {}", r.rms);
        assert!((r.peak - 1.0).abs() < 0.05, "peak {}", r.peak);
    }

    /// A band's value is a shaped measure rather than a calibrated magnitude, so what is
    /// asserted is *where the energy went*, not what number came out.
    ///
    /// Bands are not isolated and cannot be: a decibel window generous enough to hear quiet
    /// music is generous enough to hear a full-scale tone's spectral leakage two decades
    /// away. What a band owes is dominance in its own range.
    #[test]
    fn a_tone_is_loudest_in_its_own_band() {
        for (hz, band) in [(100.0, 0), (1000.0, 1), (8000.0, 2)] {
            let mut a = Analyzer::new(48_000);
            let r = a.push(&sine(hz, 48_000, FFT_SIZE * 4, 1.0));
            for other in (0..BANDS).filter(|b| *b != band) {
                assert!(
                    r.bands[band] > r.bands[other] * 1.2,
                    "{hz} Hz: band {band} reads {} against band {other}'s {}",
                    r.bands[band],
                    r.bands[other],
                );
            }
        }
    }

    /// The property a threshold rests on: a change is heard in the band it happened in, and
    /// not in the ones it only leaked into.
    #[test]
    fn a_change_is_heard_in_the_band_it_happened_in() {
        let mut a = Analyzer::new(48_000);
        let mut at = 0;
        for _ in 0..40 {
            a.push(&sine_from(1000.0, 48_000, FFT_SIZE, 0.2, at));
            at += FFT_SIZE;
        }
        let settled = a.push(&sine_from(1000.0, 48_000, FFT_SIZE, 0.2, at)).bands;
        at += FFT_SIZE;

        // The same tone, swelling. **Smoothly**: an amplitude step is a step, and a step is
        // broadband — it would land in every band and the test would be measuring a click
        // rather than the tone that made it.
        let ramp: Vec<f32> = (0..FFT_SIZE * 4)
            .map(|i| {
                let t = i as f32 / (FFT_SIZE * 4) as f32;
                let amp = 0.2 + 0.8 * 0.5 * (1.0 - (std::f32::consts::PI * t).cos());
                let s = (at + i) as f32 / 48_000.0;
                amp * (2.0 * std::f32::consts::PI * 1000.0 * s).sin()
            })
            .collect();
        let hit = a.push(&ramp).bands;
        let moved: [f32; BANDS] = std::array::from_fn(|b| hit[b] - settled[b]);
        assert!(
            moved[1] > 0.02,
            "the band the change happened in has to move: {moved:?}"
        );
        for other in [0, 2] {
            assert!(
                moved[1] > moved[other],
                "and to move further than the ones it leaked into: {moved:?}"
            );
        }
    }

    /// The window spans the last `FFT_SIZE` samples across blocks: a step happens every
    /// `HOP` samples however the device chunks its callbacks, so the same tone delivered in
    /// small blocks reads the same as one delivered whole.
    #[test]
    fn small_blocks_accumulate_into_the_same_window() {
        let tone = sine(1000.0, 48_000, FFT_SIZE * 2, 1.0);
        let mut whole = Analyzer::new(48_000);
        let expect = whole.push(&tone).bands;

        let mut blocks = Analyzer::new(48_000);
        let mut last = Analysis::default();
        for chunk in tone.chunks(128) {
            last = blocks.push(chunk);
        }
        assert!(
            (last.bands[1] - expect[1]).abs() < 0.001,
            "{} in chunks against {} whole",
            last.bands[1],
            expect[1]
        );
        assert_eq!(last.samples, (FFT_SIZE * 2) as u64);
    }

    // ---------------------------------------------------------------- thresholds

    #[test]
    fn nothing_fires_until_a_threshold_is_placed() {
        let mut a = Analyzer::new(48_000);
        a.push(&sine(100.0, 48_000, FFT_SIZE * 4, 1.0));
        assert_eq!(
            a.crossings.total, 0,
            "a patch that opened with three events already firing is not one anybody asked for"
        );
    }

    #[test]
    fn a_threshold_fires_down_when_crossed_and_up_when_it_falls_back() {
        let mut a = Analyzer::new(48_000);
        a.thresholds[0] = 0.2;
        a.debounce = std::time::Duration::ZERO;

        // Loud bass, then silence.
        a.push(&sine(100.0, 48_000, FFT_SIZE * 4, 1.0));
        let after_tone = a.crossings.total;
        assert!(
            after_tone > 0,
            "a loud band above its threshold has to fire"
        );
        let (first, dropped) = {
            let (mut edges, dropped) = a.crossings.since(0);
            (edges.next().expect("one crossing"), dropped)
        };
        assert_eq!(dropped, 0);
        assert_eq!((first.band, first.down), (0, true));
        assert!(
            first.at > 0,
            "a crossing is dated by the sample it happened on"
        );

        for _ in 0..40 {
            a.push(&vec![0.0; FFT_SIZE]);
        }
        let closed = a
            .crossings
            .since(after_tone)
            .0
            .any(|c| c.band == 0 && !c.down);
        assert!(closed, "and letting go of it has to close the gate");
    }

    #[test]
    fn a_crossing_is_delivered_exactly_once() {
        let mut a = Analyzer::new(48_000);
        a.thresholds[0] = 0.2;
        a.debounce = std::time::Duration::ZERO;
        a.push(&sine(100.0, 48_000, FFT_SIZE * 4, 1.0));

        let total = a.crossings.total;
        let (seen, _) = a.crossings.since(0);
        let count = seen.count() as u64;
        assert_eq!(count, total);
        let (again, _) = a.crossings.since(total);
        assert_eq!(again.count(), 0, "nothing is delivered twice");
    }

    #[test]
    fn the_hold_off_stops_a_level_on_the_line_from_machine_gunning() {
        let mut a = Analyzer::new(48_000);
        a.thresholds[0] = 0.2;
        a.debounce = std::time::Duration::from_millis(200);
        // Alternating loud and silent blocks: without a hold-off this fires on every one.
        for i in 0..20 {
            if i % 2 == 0 {
                a.push(&sine(100.0, 48_000, FFT_SIZE, 1.0));
            } else {
                a.push(&vec![0.0; FFT_SIZE]);
            }
        }
        let downs = a
            .crossings
            .since(0)
            .0
            .filter(|c| c.band == 0 && c.down)
            .count();
        // Twenty blocks of 1024 samples is 0.43 s, so a 200 ms hold-off allows about three.
        assert!(
            (1..=4).contains(&downs),
            "a 200 ms hold-off over 0.43 s should pass a handful, not ten: {downs}"
        );
    }

    #[test]
    fn a_reader_that_fell_far_behind_is_told_what_it_lost() {
        let mut c = Crossings::default();
        for i in 0..(CROSSING_LOG as u64 + 5) {
            c.push(Crossing {
                band: 0,
                down: i % 2 == 0,
                at: i,
            });
        }
        let (edges, dropped) = c.since(0);
        assert_eq!(dropped, 5, "five fell out of the ring before anyone looked");
        assert_eq!(edges.count(), CROSSING_LOG);
    }

    #[test]
    fn the_waveform_is_silence_at_the_middle_of_its_range() {
        let mut a = Analyzer::new(48_000);
        let r = a.push(&vec![0.0; FFT_SIZE]);
        assert!(
            r.waveform.iter().all(|&s| s == 128),
            "silence is the center"
        );
        let r = a.push(&sine(440.0, 48_000, FFT_SIZE, 1.0));
        assert!(r.waveform.iter().any(|&s| s > 200), "and a tone swings");
        assert!(r.waveform.iter().any(|&s| s < 55));
    }

    /// The producer-to-consumer handoff is what the whole design rests on: a value written
    /// on one thread is readable on another without either waiting, and stamped with when it
    /// was computed, so its age is how stale it really is.
    ///
    /// The stamp is held between two clocks read on the writing thread rather than under a
    /// bound on its age when read. The age at the read also counts the worker's exit and the
    /// join waking this thread, which is the scheduler's to stretch: with the suite running
    /// three times at once on macOS it measured 11 to 34 ms, against the 10 ms this once asked.
    #[test]
    fn a_published_analysis_reaches_the_reader_without_waiting() {
        let (mut input, mut output) =
            triple_buffer::TripleBuffer::new(&Analysis::default()).split();
        let worker = std::thread::spawn(move || {
            let mut a = Analyzer::new(48_000);
            let block = sine(100.0, 48_000, FFT_SIZE, 1.0);
            let started = Instant::now();
            input.write(a.push(&block));
            (started, Instant::now())
        });
        let (started, written) = worker.join().unwrap();
        let seen = *output.read();
        assert!(seen.bands[0] > 0.1, "the reader saw the writer's analysis");
        let published = seen.published.expect("published");
        assert!(
            started <= published && published <= written,
            "stamped while it was computed, not before and not when it was read"
        );
        let age = seen.age().expect("published");
        assert!(age <= started.elapsed(), "the age counts from the stamp");
    }
}
