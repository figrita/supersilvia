// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hearing what is being analyzed.
//!
//! **A `video` node makes no sound without this.** The transcoded cache is a picture format,
//! the soundtrack is decoded for analysis and never played, and until now nothing opened an
//! output device. silvia is not better designed here — its analyzer explicitly refuses to
//! connect to the destination — it simply gets sound for free from the `<video>` element it
//! analyzes, and we gave that up when we replaced the element with a cache and a sample
//! buffer. So this restores something rather than inventing it.
//!
//! **The samples the monitor plays are the samples the analyzer consumed.** `Reader` already
//! reads exactly the span of audio a frame covers; handing the same slice here is the whole
//! implementation, and everything else falls out of it:
//!
//! - at 2x the frame reads two seconds of audio a second and plays them in one, so it pitches
//!   up like tape;
//! - a negative speed reads the span backwards and plays it backwards;
//! - dragging `position` reads scattered windows, which is the sound of scrubbing tape,
//!   because it is the same operation;
//! - a jump reads one window, so a cut sounds like a cut.
//!
//! None of that is an approximation of what a VJ wants. A second playback pipeline chasing the
//! picture would have to be argued into every one of them.
//!
//! One device, opened once, shared by every source that asks. A source that is silent never
//! opens it, so a graph with the monitor off costs nothing.
//!
//! **A channel lives exactly as long as its handles.** [`Monitor::open`] adds one to the mix
//! and the last [`Send`] onto it to be dropped — the node's, the capture's, or the clone its
//! audio thread holds, whichever goes last — takes it out again, so a node deleted or a Main
//! Input switched to another source leaves nothing behind for the output callback to walk.

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The most audio a channel will hold before it starts dropping the oldest.
///
/// A quarter of a second. A source pushes a frame's worth at a time and the device takes a
/// few milliseconds at a time, so a healthy channel holds almost nothing; this only bounds
/// what happens when a source runs ahead of the device, and the right thing then is to lose
/// the stale audio rather than the recent audio.
const CAPACITY: usize = 12_000;

/// One source's channel into the monitor.
///
/// Dropping the last handle onto a channel removes the channel from the mix.
pub struct Send {
    channel: Arc<Channel>,
    monitor: &'static Monitor,
}

struct Channel {
    /// Samples waiting to be played, at the device's own rate.
    ///
    /// A mutex rather than a lock-free ring, because a lock-free ring means `unsafe` and
    /// `unsafe` lives in `render/` and nowhere else. The output callback uses `try_lock` and
    /// plays silence rather than waiting, so a contended lock is a click and never a stall.
    /// The frame thread holds it for the length of a `memcpy` once a frame; if that ever
    /// measures as a problem, the answer is a real-time ring crate, not a longer lock.
    samples: Mutex<VecDeque<f32>>,
    /// Volume, as bits. Zero means the source is not monitoring and pushes nothing.
    volume: AtomicU32,
    /// Where the resampler has reached between two input samples.
    phase: Mutex<f32>,
    /// How many [`Send`]s are onto this channel.
    handles: AtomicUsize,
}

impl Channel {
    fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::with_capacity(CAPACITY)),
            volume: AtomicU32::new(0.0f32.to_bits()),
            phase: Mutex::new(0.0),
            handles: AtomicUsize::new(1),
        }
    }
}

impl Drop for Send {
    fn drop(&mut self) {
        if self.channel.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.monitor.close(&self.channel);
        }
    }
}

impl Send {
    /// How loud this source plays, 0 to 1. Zero costs nothing: nothing is pushed, and the
    /// device is not opened until something asks to be heard.
    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.channel
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
        if volume > 0.0 {
            self.monitor.start();
        }
    }

    /// A second handle onto the same channel, for a source whose samples are produced on a
    /// different thread from the one that sets its volume — a capture callback, say.
    #[must_use]
    pub fn clone_handle(&self) -> Self {
        self.channel.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            channel: Arc::clone(&self.channel),
            monitor: self.monitor,
        }
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.channel.volume.load(Ordering::Relaxed))
    }

    /// Queue mono samples recorded at `rate`, resampled to the device's rate.
    ///
    /// Does nothing while the volume is zero, so a graph that is not monitoring never touches
    /// a lock and never opens a device.
    pub fn push(&self, samples: &[f32], rate: u32) {
        if self.volume() <= 0.0 || samples.is_empty() {
            return;
        }
        let out_rate = self.monitor.rate();
        let step = f64::from(rate) / f64::from(out_rate.max(1));

        let (Ok(mut queue), Ok(mut phase)) =
            (self.channel.samples.lock(), self.channel.phase.lock())
        else {
            return;
        };
        // Linear interpolation. A monitor is for hearing what the analyzer heard, not for
        // mastering, and the alternative is a resampler nobody asked for.
        let mut at = f64::from(*phase);
        while at < samples.len() as f64 {
            let i = at as usize;
            let frac = (at - i as f64) as f32;
            let a = samples[i];
            let b = *samples.get(i + 1).unwrap_or(&a);
            queue.push_back(a + (b - a) * frac);
            at += step;
        }
        *phase = (at - samples.len() as f64) as f32;

        // Oldest first: a source that ran ahead should be caught up with, not waited for.
        while queue.len() > CAPACITY {
            queue.pop_front();
        }
    }
}

/// The output device, and everything monitoring through it.
pub struct Monitor {
    channels: Mutex<Vec<Arc<Channel>>>,
    rate: AtomicU32,
    /// Held for its lifetime only: cpal stops the stream on drop.
    stream: Mutex<Option<cpal::Stream>>,
    error: Mutex<Option<String>>,
}

// The stream is not `Send`, and the monitor is only ever touched from the frame thread and
// the audio callback it owns. `Monitor::shared` hands out `&'static Monitor`, so this is the
// claim that makes a static of it legal.
//
// SAFETY-style note without `unsafe`: rather than assert that, the static holds the monitor
// behind a `OnceLock` created on the frame thread and every method takes `&self`, so nothing
// moves it. The stream lives in a `Mutex` that only `open` and `shutdown` touch.
static MONITOR: OnceLock<Monitor> = OnceLock::new();

impl Monitor {
    /// The one monitor. Opening the device is deferred until something actually monitors.
    pub fn shared() -> &'static Monitor {
        MONITOR.get_or_init(Monitor::new)
    }

    /// A monitor with no channels and no device.
    fn new() -> Self {
        Monitor {
            channels: Mutex::new(Vec::new()),
            rate: AtomicU32::new(48_000),
            stream: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    /// The device's sample rate, which is what `Send::push` resamples to.
    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// What went wrong opening the device, for a node's status line.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// A new channel, opening the device on the first one.
    pub fn open(&'static self) -> Send {
        let channel = Arc::new(Channel::new());
        if let Ok(mut channels) = self.channels.lock() {
            channels.push(Arc::clone(&channel));
        }
        // Deliberately not opened here. A node that exists is not a node that is monitoring,
        // and taking the output device from whatever else is using it because a graph was
        // loaded would be rude.
        Send {
            channel,
            monitor: self,
        }
    }

    /// Take a channel out of the mix: its last handle has gone.
    fn close(&self, channel: &Arc<Channel>) {
        if let Ok(mut channels) = self.channels.lock() {
            channels.retain(|live| !Arc::ptr_eq(live, channel));
        }
    }

    /// Mix every live channel into an interleaved buffer of `channels` channels: what the
    /// output callback does. Returns how many channels it walked.
    fn mix(&self, out: &mut [f32], channels: usize) -> usize {
        let Ok(sources) = self.channels.lock() else {
            out.fill(0.0);
            return 0;
        };
        mix_into(out, channels, &sources);
        sources.len()
    }

    /// Open the device if it is not open. Idempotent, and a failure is remembered rather than
    /// retried every frame.
    fn start(&'static self) {
        let Ok(mut slot) = self.stream.lock() else {
            return;
        };
        if slot.is_some() || self.error().is_some() {
            return;
        }
        match self.build() {
            Ok((stream, rate)) => {
                self.rate.store(rate, Ordering::Relaxed);
                *slot = Some(stream);
            }
            Err(e) => {
                if let Ok(mut error) = self.error.lock() {
                    *error = Some(e);
                }
            }
        }
    }

    fn build(&'static self) -> Result<(cpal::Stream, u32), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no audio output device".to_string())?;
        let name = device
            .description()
            .map_or_else(|_| "unnamed".to_string(), |d| d.to_string());
        let supported = device
            .default_output_config()
            .map_err(|e| format!("{name}: {e}"))?;
        let rate = supported.sample_rate();
        let channels = usize::from(supported.channels()).max(1);
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let mix = move |out: &mut [f32]| {
            self.mix(out, channels);
        };

        let err = move |e| log::error!("audio output: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => {
                device.build_output_stream(config, move |data: &mut [f32], _| mix(data), err, None)
            }
            cpal::SampleFormat::I16 => {
                let mut buf: Vec<f32> = Vec::new();
                device.build_output_stream(
                    config,
                    move |data: &mut [i16], _| {
                        buf.resize(data.len(), 0.0);
                        mix(&mut buf);
                        for (out, v) in data.iter_mut().zip(&buf) {
                            *out = (v * 32767.0) as i16;
                        }
                    },
                    err,
                    None,
                )
            }
            other => return Err(format!("{name}: unsupported sample format {other:?}")),
        }
        .map_err(|e| format!("{name}: {e}"))?;
        stream.play().map_err(|e| format!("{name}: {e}"))?;
        Ok((stream, rate))
    }
}

/// Sum every channel into an interleaved output buffer.
///
/// Separated from the callback so it can be tested: the callback needs a device, and what is
/// worth checking is the arithmetic.
fn mix_into(out: &mut [f32], channels: usize, sources: &[Arc<Channel>]) {
    out.fill(0.0);
    let channels = channels.max(1);
    for source in sources {
        let volume = f32::from_bits(source.volume.load(Ordering::Relaxed));
        if volume <= 0.0 {
            continue;
        }
        // Never waits: a contended channel is silent for one callback, which is a click,
        // where waiting would be a gap in everybody's audio.
        let Ok(mut queue) = source.samples.try_lock() else {
            continue;
        };
        for frame in out.chunks_mut(channels) {
            let Some(sample) = queue.pop_front() else {
                break;
            };
            // The same sample to every channel: this is a monitor, and one that panned would
            // be the beginning of a second mixer.
            for slot in frame.iter_mut() {
                *slot += sample * volume;
            }
        }
    }
    // Two loud sources sum past full scale. Clipping is ugly; wrapping is a bang.
    for slot in out.iter_mut() {
        *slot = slot.clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monitor of the test's own, so no other test's channels are counted in it. The
    /// device is never opened in a test: nothing here sets a volume above zero through
    /// `Send::set_volume`, which is the one thing that opens it.
    fn fresh() -> &'static Monitor {
        Box::leak(Box::new(Monitor::new()))
    }

    /// A channel of a monitor of its own, at full volume.
    fn channel() -> Send {
        let send = fresh().open();
        send.channel
            .volume
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        send
    }

    fn loaded(volume: f32, samples: &[f32]) -> Arc<Channel> {
        let channel = Channel::new();
        *channel.samples.lock().unwrap() = samples.iter().copied().collect();
        channel.volume.store(volume.to_bits(), Ordering::Relaxed);
        Arc::new(channel)
    }

    fn live(monitor: &Monitor) -> usize {
        monitor.channels.lock().unwrap().len()
    }

    /// Every way an owner comes and goes — a `video` node made and deleted, a capture whose
    /// audio thread holds a clone and lets it go on its own thread, a Main Input switching
    /// source — over and over, and the mix is back to what it was, walking only what is live.
    #[test]
    fn owners_that_come_and_go_leave_no_channel_behind() {
        let monitor = fresh();
        let keeper = monitor.open();
        let baseline = live(monitor);
        assert_eq!(baseline, 1);
        let mut main_input: Option<Send> = None;

        for round in 0..500 {
            let node = monitor.open();
            let capture = monitor.open();
            let audio_thread = capture.clone_handle();
            main_input = Some(monitor.open());
            assert_eq!(live(monitor), baseline + 3, "round {round}");

            drop(capture);
            assert_eq!(
                live(monitor),
                baseline + 3,
                "the audio thread's clone keeps it"
            );
            std::thread::spawn(move || drop(audio_thread))
                .join()
                .unwrap();
            assert_eq!(live(monitor), baseline + 2, "round {round}");

            node.channel
                .volume
                .store(1.0f32.to_bits(), Ordering::Relaxed);
            node.push(&[0.5; 64], monitor.rate());
            drop(node);
            assert_eq!(live(monitor), baseline + 1, "round {round}");
        }
        main_input = main_input.take().map(|_| monitor.open());
        assert_eq!(live(monitor), baseline + 1);
        drop(main_input);
        assert_eq!(live(monitor), baseline);

        keeper
            .channel
            .volume
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        keeper.push(&[0.25; 4], monitor.rate());
        let mut out = [0.0f32; 8];
        assert_eq!(
            monitor.mix(&mut out, 1),
            1,
            "the callback walks the keeper alone"
        );
        assert_eq!(
            out[..4],
            [0.25; 4],
            "the deleted nodes' queued audio is gone with them"
        );
        assert_eq!(out[4..], [0.0; 4]);

        drop(keeper);
        assert_eq!(live(monitor), 0);
        assert_eq!(monitor.mix(&mut out, 1), 0);
        assert_eq!(out, [0.0; 8]);
    }

    #[test]
    fn a_mix_scales_by_volume_and_writes_every_channel() {
        let source = loaded(0.5, &[1.0, 1.0, 1.0, 1.0]);
        let mut out = [9.0f32; 8];
        mix_into(&mut out, 2, &[source]);
        assert_eq!(out, [0.5; 8], "stereo gets the same sample twice, halved");
    }

    #[test]
    fn two_sources_sum_and_the_sum_is_bounded() {
        let a = loaded(1.0, &[0.8; 4]);
        let b = loaded(1.0, &[0.8; 4]);
        let mut out = [0.0f32; 4];
        mix_into(&mut out, 1, &[a, b]);
        assert_eq!(out, [1.0; 4], "1.6 clamps rather than wrapping to a bang");
    }

    #[test]
    fn a_channel_at_zero_contributes_nothing_and_keeps_its_audio() {
        let quiet = loaded(0.0, &[1.0; 4]);
        let mut out = [0.0f32; 4];
        mix_into(&mut out, 1, &[Arc::clone(&quiet)]);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(
            quiet.samples.lock().unwrap().len(),
            4,
            "a muted source is not a drained one"
        );
    }

    #[test]
    fn a_channel_that_runs_dry_leaves_the_rest_of_the_buffer_silent() {
        let short = loaded(1.0, &[1.0, 1.0]);
        let mut out = [0.0f32; 6];
        mix_into(&mut out, 1, &[short]);
        assert_eq!(out, [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_silent_channel_queues_nothing() {
        let send = channel();
        send.set_volume(0.0);
        send.push(&[1.0; 100], 48_000);
        assert_eq!(send.channel.samples.lock().unwrap().len(), 0);
    }

    #[test]
    fn samples_at_the_device_rate_pass_through_one_for_one() {
        let send = channel();
        send.push(&[0.5; 480], send.monitor.rate());
        let queued = send.channel.samples.lock().unwrap().len();
        assert!(
            (queued as i64 - 480).abs() <= 1,
            "expected about 480, got {queued}"
        );
    }

    #[test]
    fn a_lower_rate_stretches_and_a_higher_one_compresses() {
        let half = channel();
        let out = half.monitor.rate();
        half.push(&[0.5; 480], out / 2);
        let stretched = half.channel.samples.lock().unwrap().len();
        assert!(
            stretched > 900,
            "half rate should roughly double: {stretched}"
        );

        let double = channel();
        double.push(&[0.5; 480], out * 2);
        let squashed = double.channel.samples.lock().unwrap().len();
        assert!(
            squashed < 260,
            "double rate should roughly halve: {squashed}"
        );
    }

    #[test]
    fn a_source_that_runs_ahead_loses_the_stale_audio_not_the_recent() {
        let send = channel();
        for i in 0..40 {
            let block: Vec<f32> = vec![i as f32; 1000];
            send.push(&block, send.monitor.rate());
        }
        let queue = send.channel.samples.lock().unwrap();
        assert!(queue.len() <= CAPACITY + 1);
        assert_eq!(
            *queue.back().unwrap(),
            39.0,
            "the newest audio is what survives"
        );
    }

    /// Pushing across block boundaries has to keep the fractional position, or every block
    /// restarts the interpolation and the output ticks at the block rate.
    #[test]
    fn the_resampler_carries_its_phase_between_pushes() {
        let send = channel();
        let rate = send.monitor.rate() * 3 / 2;
        for _ in 0..10 {
            send.push(&[0.25; 300], rate);
        }
        let queued = send.channel.samples.lock().unwrap().len();
        let expected = 10.0 * 300.0 * 2.0 / 3.0;
        assert!(
            (queued as f32 - expected).abs() < 5.0,
            "expected about {expected}, got {queued}"
        );
    }
}
