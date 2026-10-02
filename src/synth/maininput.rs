// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Input, running: the device that is open, the clip that is decoding, the portal
//! session that is held.
//!
//! [`crate::maininput::MainInput`] is the *choice*, and it lives in the project. This is what
//! that choice costs — one camera, one capture, one screen cast — and it is the synth's for
//! the reason the renderer is: a choice travels in a file, and an open device cannot. The
//! panel that draws it is the editor's, in `app/maininput.rs`.
//!
//! One rule runs the whole thing. **[`Live::reconcile`] is called once a frame with the
//! choice, and makes what is open match it.** Nothing else opens or closes anything, so there
//! is no path where the panel says one thing and a device is doing another, and switching
//! sources five times in a second opens the fifth and only the fifth.
//!
//! The frame thread never waits here. A clip transcodes on a worker, a soundtrack decodes on a
//! worker, the screen picker sits on a thread of its own, the machine's devices are listed on a
//! thread of their own, and every one of them is polled. The listing has no bound of ours: an
//! ALSA card that does not answer is waited for through the kernel's timeouts, and a webcam's
//! microphone held one for 10.4 s, which in the tick was the whole of the first one
//! (docs/media.md, the Main Input).

use crate::audio::{self, Analysis, Capture, bands};
use crate::maininput::{AudioSource, MainInput, VideoSource};
use crate::nodes::Frame;
use crate::platform::screen;
use crate::video::silence::Silence;
use crate::video::{Camera, Source, clip};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

/// A clip on its way to being played: transcoded if the cache has no entry, then opened.
#[derive(Default)]
enum Picture {
    /// Nothing to show. Black.
    #[default]
    Idle,
    Transcoding(clip::Transcode),
    Playing(Box<clip::Player>),
}

/// A soundtrack on its way to being readable.
#[derive(Default)]
enum Sound {
    #[default]
    Silent,
    Decoding(mpsc::Receiver<audio::Track>),
    Ready(Box<audio::Reader>),
}

/// A screen capture, from the ask to the pipeline.
enum Cast {
    /// The picker is up, or nobody has answered it yet.
    Asking(screen::Pending),
    /// Held for as long as the pipeline reads it: this is the portal session, and nothing
    /// reads the value. Dropping it is what ends the cast.
    Running(#[allow(dead_code)] Box<screen::Cast>),
}

/// What the machine answers when asked what it has: its audio sources and its cameras.
type Devices = (Vec<audio::device::Listed>, Vec<(String, String)>);

/// How the machine is asked: the platform's two listings, everywhere but a test that stands a
/// slow machine in.
struct Lister(Arc<dyn Fn() -> Devices + Send + Sync>);

impl Default for Lister {
    fn default() -> Self {
        Self(Arc::new(|| {
            (audio::device::list(), crate::video::capture_devices())
        }))
    }
}

/// Everything the Main Input has open.
#[derive(Default)]
pub struct Live {
    /// What the last reconcile was given, so a frame that changed nothing opens nothing.
    video_wanted: Option<VideoSource>,
    audio_wanted: Option<AudioSource>,

    camera: Option<Camera>,
    cast: Option<Cast>,
    /// A Syphon server's client, kept connected as the server comes and goes.
    syphon: Option<crate::video::syphon::Receiver>,
    /// An NDI source's camera, kept open as the source comes and goes.
    ndi: Option<crate::video::ndi::Receiver>,
    picture: Picture,
    capture: Option<Capture>,
    sound: Sound,
    /// Where a played file has got to, in seconds. Free-running and looped: the Main Input is
    /// a source that is *on*, and everything a hand does to a clip belongs to a `video` node.
    /// Both the clip's frame and the sound file's analysis are read at this one time, which is
    /// what makes a render's virtual time reach them: a render *drives* it instead.
    position: crate::nodes::phasor::Phasor,
    /// Virtual time, while a render owns the clock: `position` is set to this each frame
    /// rather than integrated, so a clip and its sound sit exactly at frame `i / fps`.
    driven: Option<f64>,
    /// The clip's frame asked for this tick, and the one its decoder last delivered: a
    /// render holds until they agree.
    clip_asked: Option<u64>,
    clip_delivered: Option<u64>,
    /// This input's channel into the shared output device, so a sound file can be heard.
    monitor: Option<audio::monitor::Send>,

    /// The newest picture, black until something arrives. Never `None` once ticked, so a
    /// sampler always has a texture.
    frame: Option<Arc<Frame>>,
    /// The newest analysis, with the gain already applied where a node reads it.
    analysis: Analysis,
    /// How many crossings have been handed out, so no node sees one twice.
    ///
    /// Counted here rather than per node because the crossings are the *input's*: every
    /// `maininput` node in the project fires on the same edge of the same signal.
    delivered: u64,

    error: Option<String>,
    /// Every audio source that could be listened to, read once and on demand after that.
    devices: Vec<audio::device::Listed>,
    /// Every capture device, cached for the same reason and much more urgently.
    ///
    /// Asking costs a GStreamer `DeviceMonitor` started, probed and stopped. The panel used to
    /// ask while *drawing the list*, which is once a frame: sixty device enumerations a
    /// second, eleven thousand GStreamer warnings in a minute, and the frame's CPU time from
    /// half a millisecond to eleven. It looked exactly like the GPU had died.
    cameras: Vec<(String, String)>,
    /// The listing in flight, on a thread of its own: see [`Self::ask_devices`].
    listing: Option<mpsc::Receiver<Devices>>,
    lister: Lister,
    /// The machine has been asked what it has, whatever it answered.
    ///
    /// **Not inferred from the lists being empty.** A box with no camera *and* no audio
    /// source — a container with neither, which is one of the ones this runs in — answers
    /// both questions with nothing, and a guard that reads that as "not asked yet" asks
    /// again on the very next frame. That is two GStreamer `DeviceMonitor`s started, probed
    /// and stopped every frame, for ever.
    asked: bool,
    /// How long the video source has delivered nothing new, from when it was opened.
    silence: Option<Silence>,
    /// The picture last delivered, so a new one is told from the same one kept.
    heard: Option<Arc<Frame>>,
    /// The source is a camera, which streams: one that stops for ten seconds has stopped,
    /// where a screen with nothing moving on it or a clip on a paused playhead has not.
    streams: bool,
    /// The video source has delivered nothing for ten seconds: before its first frame,
    /// whatever it is, and since its last, for a camera.
    silent: bool,
    /// `None` until probed; `Some(None)` means this machine cannot encode, so no clip.
    #[allow(clippy::option_option)]
    codec: Option<Option<clip::Codec>>,
}

impl Live {
    /// The newest picture. Black before anything has arrived, never absent.
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.frame.as_ref()
    }

    /// The newest analysis, as the audio thread wrote it. Gain is a node's to apply.
    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    /// The rate the analysis was made at, which the scope's frequency axis needs.
    pub fn sample_rate(&self) -> f32 {
        match (&self.capture, &self.sound) {
            (Some(capture), _) => capture.sample_rate as f32,
            _ => audio::track::RATE as f32,
        }
    }

    /// Whether anything is actually being heard, as opposed to chosen.
    pub fn hears(&self) -> bool {
        self.capture.is_some() || matches!(self.sound, Sound::Ready(_))
    }

    /// What went wrong, if anything did. One line, for the status bar and the panel.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Ask the machine what it has to capture with, audio and video, on a thread named
    /// `devices`; the answer lands in the lists on the first tick after it arrives
    /// ([`Self::take_devices`]), and they are empty until then. Both cost a round trip —
    /// PulseAudio for one, a GStreamer `DeviceMonitor` for the other — so this happens on the
    /// first frame and then only when [`Self::forget_devices`] has emptied the lists. **Never
    /// while drawing**: see [`Self::cameras`].
    fn ask_devices(&mut self) {
        let (tx, rx) = mpsc::channel();
        let list = Arc::clone(&self.lister.0);
        match std::thread::Builder::new()
            .name("devices".into())
            .spawn(move || {
                // A receiver gone is a listing superseded or a run ending; nothing waits.
                let _ = tx.send(list());
            }) {
            Ok(_) => self.listing = Some(rx),
            Err(e) => log::warn!("main input: the devices could not be listed: {e}"),
        }
        self.asked = true;
    }

    /// Land a listing that has finished. Never waits.
    fn take_devices(&mut self) {
        let Some(listing) = &self.listing else {
            return;
        };
        match listing.try_recv() {
            Ok((devices, cameras)) => {
                self.devices = devices;
                self.cameras = cameras;
                self.listing = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.listing = None,
        }
    }

    /// Whether the machine has been asked at all. The one thing the frame checks.
    pub fn asked_devices(&self) -> bool {
        self.asked
    }

    /// Throw the device lists away, so the next frame asks for new ones. What *Look for
    /// devices again* does: a headset or a webcam plugged in after the app started is not in
    /// the list, and forgetting is the whole of asking again.
    pub fn forget_devices(&mut self) {
        self.devices.clear();
        self.cameras.clear();
        self.listing = None;
        self.asked = false;
    }

    /// Every audio source on the machine, for the panel's list.
    pub fn devices(&self) -> &[audio::device::Listed] {
        &self.devices
    }

    /// Every capture device on the machine, for the panel's list.
    pub fn cameras(&self) -> &[(String, String)] {
        &self.cameras
    }

    /// Crossings not yet handed out, and the count to remember. See [`Self::delivered`].
    pub fn crossings(&self) -> (Vec<audio::Crossing>, u64) {
        let (edges, dropped) = self.analysis.crossings.since(self.delivered);
        if dropped > 0 {
            log::debug!("main input: {dropped} crossings arrived faster than they were read");
        }
        (edges.collect(), self.analysis.crossings.total)
    }

    /// Mark the crossings up to `total` as delivered.
    pub fn delivered(&mut self, total: u64) {
        self.delivered = total;
    }

    /// Make what is open match what is chosen, then take one frame's worth of everything.
    ///
    /// `resolve` turns an asset reference into a path on this machine; `cache` is the
    /// project's own, so a transcode travels with the project.
    /// Open the video source again even though the choice has not changed. What *Choose
    /// another screen…* needs: the source is still `Screen`, and the ask is the point.
    pub fn reopen_video(&mut self) {
        self.video_wanted = None;
    }

    pub fn reconcile(
        &mut self,
        want: &MainInput,
        resolve: &dyn Fn(&str) -> PathBuf,
        cache: &Path,
        time: &crate::transport::Time,
    ) {
        let video_changed = self.video_wanted.as_ref() != Some(&want.video);
        if video_changed {
            self.open_video(&want.video, resolve, cache);
            self.video_wanted = Some(want.video.clone());
        }
        // The video source's sound is whatever the video source is, so a new video source is a
        // new soundtrack even though the audio choice reads the same.
        if self.audio_wanted.as_ref() != Some(&want.audio)
            || (video_changed && want.audio == AudioSource::Video)
        {
            self.open_audio(&want.audio, &want.video, resolve, cache);
            self.audio_wanted = Some(want.audio.clone());
        }
        if let Some(t) = self.driven {
            self.position.set(t);
        } else {
            // Speed one, on the transport: it pauses with the show and a seek moves it by the
            // jump.
            self.position.step(1.0, time);
        }
        self.tick_video();
        self.tick_audio(want);
    }

    /// Own the file's time: `Some(t)` places the clip and the sound at `t` seconds on the
    /// next reconcile, `None` lets them run from wherever they are.
    pub fn drive(&mut self, t: Option<f64>) {
        self.driven = t;
    }

    /// Is the clip's picture not yet the frame its position names: what a render holds for.
    pub fn waiting(&self) -> bool {
        self.clip_asked.is_some() && self.clip_asked != self.clip_delivered
    }

    /// Where the played file is, in seconds.
    pub fn position(&self) -> f64 {
        self.position.phase()
    }

    /// Put the played file back somewhere, as a render does when it hands the clock back.
    pub fn set_position(&mut self, t: f64) {
        self.position.set(t);
    }

    // --- opening ----------------------------------------------------------------------

    fn open_video(&mut self, want: &VideoSource, resolve: &dyn Fn(&str) -> PathBuf, cache: &Path) {
        // Everything goes first. A device only this reads stops before it is opened again,
        // and the portal session has to end before a second one is asked for.
        self.camera = None;
        self.cast = None;
        self.syphon = None;
        self.ndi = None;
        self.picture = Picture::Idle;
        self.position.set(0.0);
        self.error = None;
        // The last source's last picture goes with it. Keeping it would mean a source that
        // produces nothing — a camera that is not there, a screen whose picker was refused —
        // showing the previous one's frame and looking like it worked.
        self.frame = None;
        self.silence = Some(Silence::new(Instant::now()));
        self.heard = None;
        self.silent = false;
        self.streams = matches!(want, VideoSource::Camera { .. });
        match want {
            VideoSource::None => {}
            VideoSource::Camera { device } => {
                let source = match device.as_str() {
                    "" => Source::Auto,
                    // GStreamer's own color bars. Not a device, and the one source that is
                    // always there: it is how you tell a chain that is wrong from a camera
                    // that is missing, with nothing plugged in and no dialog to answer.
                    "test" => Source::Test,
                    path => Source::Device(path.to_string()),
                };
                match Camera::open(&source, None) {
                    Ok(camera) => self.camera = Some(camera),
                    Err(e) => self.error = Some(e),
                }
            }
            // Always asked, never resumed from a saved token. Which window you are capturing
            // is the rig's, and the rig does not persist: choosing *Screen or window* puts the
            // desktop's picker up, every time, so you are never silently handed last week's.
            VideoSource::Screen => {
                self.cast = Some(Cast::Asking(screen::ask()));
            }
            VideoSource::File { asset } => self.open_clip(&resolve(asset), cache),
            VideoSource::Syphon {
                server,
                flip,
                transparent,
            } => {
                let look = crate::platform::syphon::Look {
                    flip: *flip,
                    transparent: *transparent,
                };
                self.syphon = Some(crate::video::syphon::Receiver::new(server, look));
            }
            VideoSource::Ndi {
                source,
                transparent,
            } => {
                self.ndi = Some(crate::video::ndi::Receiver::new(source, *transparent));
            }
        }
    }

    /// Play a clip from the project's cache, or start making the cache entry.
    fn open_clip(&mut self, source: &Path, cache: &Path) {
        // The same settings the `video` node transcodes with at its default size, so the two
        // share cache entries rather than each writing an all-intra copy of the same file.
        let Some(codec) = *self.codec.get_or_insert_with(clip::Codec::probe) else {
            self.error = Some(format!(
                "no hardware video encoder: {} is needed to play a clip",
                crate::platform::video::CODEC_HINT
            ));
            return;
        };
        let settings = clip::Settings {
            max_height: 1080,
            codec,
        };
        self.picture = match clip::cache_path(cache, source, settings) {
            None => {
                self.error = Some(format!("{}: cannot read", source.display()));
                Picture::Idle
            }
            Some(cached) if cached.is_file() => match clip::Player::open(&cached) {
                Ok(player) => Picture::Playing(Box::new(player)),
                Err(e) => {
                    self.error = Some(e);
                    Picture::Idle
                }
            },
            Some(cached) => Picture::Transcoding(clip::Transcode::start(
                source.to_path_buf(),
                cached,
                settings,
            )),
        };
    }

    fn open_audio(
        &mut self,
        want: &AudioSource,
        video: &VideoSource,
        resolve: &dyn Fn(&str) -> PathBuf,
        cache: &Path,
    ) {
        self.capture = None;
        self.sound = Sound::Silent;
        self.monitor = None;
        self.delivered = 0;
        self.analysis = Analysis::default();
        match want {
            AudioSource::None => {}
            AudioSource::Live { device } => match Capture::open_device(device) {
                Ok(capture) => {
                    capture.set_config(bands::DEFAULT);
                    self.capture = Some(capture);
                }
                Err(e) => self.error = Some(e),
            },
            AudioSource::File { asset } => self.decode(&resolve(asset), cache),
            // The video source's soundtrack: a clip's, or an NDI source's own sound.
            AudioSource::Video => match video {
                VideoSource::File { asset } => self.decode(&resolve(asset), cache),
                VideoSource::Ndi { source, .. } if !source.is_empty() => {
                    match Capture::open_ndi(source) {
                        Ok(capture) => {
                            capture.set_config(bands::DEFAULT);
                            self.capture = Some(capture);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                _ => {
                    self.error = Some(
                        "the video source has no soundtrack: choose a clip, an NDI source, \
                              or the system audio"
                            .to_string(),
                    );
                }
            },
        }
    }

    /// Decode a file's audio on a worker. Blocks for as long as the file takes to read, so it
    /// never happens on the frame thread.
    fn decode(&mut self, source: &Path, cache: &Path) {
        let (tx, rx) = mpsc::channel();
        let source = source.to_path_buf();
        let cache = cache.to_path_buf();
        std::thread::spawn(move || {
            let track = audio::Track::decode(&source, &cache).unwrap_or_default();
            // Nobody may be listening any more, which is not an error.
            let _ = tx.send(track);
        });
        self.sound = Sound::Decoding(rx);
        self.monitor
            .get_or_insert_with(|| audio::Monitor::shared().open());
    }

    // --- one frame's worth ------------------------------------------------------------

    fn tick_video(&mut self) {
        // The picker, if one is up. Answering it opens the pipeline; refusing it says so and
        // leaves the source showing black, which is what a refusal should look like.
        if let Some(Cast::Asking(pending)) = &self.cast {
            match pending.poll() {
                None => {}
                Some(Ok(cast)) => {
                    let source = Source::Screen(cast.stream());
                    match Camera::open(&source, None) {
                        Ok(camera) => {
                            self.camera = Some(camera);
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                    // Held from here on: dropping it closes the portal session and stops the
                    // stream the pipeline above is reading.
                    self.cast = Some(Cast::Running(Box::new(cast)));
                }
                Some(Err(e)) => {
                    self.error = Some(e);
                    self.cast = None;
                }
            }
        }

        if let Picture::Transcoding(job) = &self.picture
            && let Some(done) = job.result()
        {
            self.picture = match done {
                Ok(path) => match clip::Player::open(&path) {
                    Ok(player) => Picture::Playing(Box::new(player)),
                    Err(e) => {
                        self.error = Some(e);
                        Picture::Idle
                    }
                },
                Err(e) => {
                    self.error = Some(e);
                    Picture::Idle
                }
            };
        }

        self.clip_asked = None;
        if let Picture::Playing(player) = &mut self.picture {
            let info = player.info();
            // Looped, and chosen by `video`'s rule, `round(position × frames)` over the
            // clip's own length: a frame index from the position rather than a counter, so a
            // dropped frame is skipped rather than accumulated into a drift. Wrapped, because
            // a render's warm-up runs from before the beginning.
            let frames = info.frames.max(1);
            let length = frames as f64 / info.fps.max(0.001);
            let index = clip::frame_at(
                (self.position.phase() / length).rem_euclid(1.0),
                frames,
                true,
            );
            player.request(index);
            self.clip_asked = Some(index);
            self.clip_delivered = None;
            if let Some((at, frame)) = player.latest() {
                self.clip_delivered = Some(at);
                self.frame = Some(frame);
            }
            if let Some(e) = player.error() {
                self.error = Some(e);
            }
        }

        if let Some(camera) = &mut self.camera {
            if let Some(e) = camera.error() {
                self.error = Some(e);
            }
            if let Some(frame) = camera.latest() {
                self.frame = Some(frame);
            }
        }

        if let Some(syphon) = &mut self.syphon {
            if let Some(frame) = syphon.latest() {
                self.frame = Some(frame);
            }
            self.error = syphon.error();
        }

        if let Some(ndi) = &mut self.ndi {
            if let Some(frame) = ndi.latest() {
                self.frame = Some(frame);
            }
            self.error = ndi.error();
        }

        if self.frame.is_none() {
            self.frame = Some(Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])));
        }
        self.listen(Instant::now());
    }

    /// Whether the video source has gone silent: a frame delivered starts the count again,
    /// and so does a clip transcoding or the screen picker waiting on a hand, since neither
    /// owes a frame yet.
    fn listen(&mut self, now: Instant) {
        let Some(silence) = &mut self.silence else {
            return;
        };
        let waiting = matches!(self.picture, Picture::Transcoding(_))
            || matches!(self.cast, Some(Cast::Asking(_)));
        let fresh = self
            .frame
            .as_ref()
            .filter(|f| f.width > 2)
            .filter(|f| !self.heard.as_ref().is_some_and(|h| Arc::ptr_eq(h, f)))
            .cloned();
        if waiting || fresh.is_some() {
            silence.heard(now);
        }
        if fresh.is_some() {
            self.heard = fresh;
        }
        self.silent = silence.not_responding(now) && (self.heard.is_none() || self.streams);
    }

    fn tick_audio(&mut self, want: &MainInput) {
        if let Sound::Decoding(rx) = &self.sound {
            match rx.try_recv() {
                Ok(track) => {
                    let mut reader = audio::Reader::new(Arc::new(track));
                    reader.analyzer_mut().config = want.bands;
                    self.sound = Sound::Ready(Box::new(reader));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.sound = Sound::Silent,
            }
        }

        if let Some(capture) = &mut self.capture {
            capture.set_config(want.bands);
            self.analysis = capture.latest();
            return;
        }

        if let Sound::Ready(reader) = &mut self.sound {
            let analyzer = reader.analyzer_mut();
            analyzer.config = want.bands;
            // The panel's levels, which `set_levels` hands a capture's audio thread.
            analyzer.thresholds[..bands::BANDS].copy_from_slice(&want.levels);
            let length = reader.track().duration();
            // Looped: a source the whole rig reads should not quietly stop after four minutes.
            let at = if length > 0.0 {
                self.position.phase().rem_euclid(length)
            } else {
                0.0
            };
            let read = reader.advance_to(at);
            self.analysis = read.analysis;
            if let Some(monitor) = &self.monitor {
                monitor.push(reader.last_samples(), audio::track::RATE);
            }
        }
    }

    /// The level each band fires at. Read by the audio thread on its next block, which is
    /// what puts the crossing on the sample it happened on rather than on this frame.
    pub fn set_levels(&self, levels: [f32; bands::BANDS]) {
        if let Some(capture) = &self.capture {
            // The fourth is the whole signal's own threshold, which the Main Input does not
            // offer: infinity is "never", the same as an untouched one.
            capture.set_thresholds([levels[0], levels[1], levels[2], f32::INFINITY]);
        }
    }

    /// How loudly a played sound file comes out of the speakers. Zero is off, as everywhere.
    pub fn set_monitor(&self, volume: f32) {
        if let Some(monitor) = &self.monitor {
            monitor.set_volume(volume);
        }
        if let Some(capture) = &self.capture {
            capture.set_monitor(volume);
        }
    }

    /// What the panel says under the video source: transcoding, waiting on a picker, or what
    /// is actually running.
    ///
    /// A sentence rather than the pipeline. `Camera::description` is the GStreamer launch line
    /// and belongs in the Status box, which is where a person goes to read one; under a
    /// preview it is four hundred characters of `!` that push everything else off the panel.
    pub fn video_status(&self, want: &VideoSource) -> String {
        if let Picture::Transcoding(job) = &self.picture {
            return format!("preparing clip… {:.0}%", job.progress() * 100.0);
        }
        if let Some(Cast::Asking(_)) = &self.cast {
            return "waiting for the screen picker…".to_string();
        }
        let size = self
            .frame
            .as_ref()
            .filter(|f| f.width > 2)
            .map(|f| format!("{}x{}", f.width, f.height));
        let what = match want {
            VideoSource::None => return "nothing open".to_string(),
            VideoSource::Screen => "screen".to_string(),
            // A Mac's project opened where there is no Syphon: why, rather than the server.
            VideoSource::Syphon { .. } if !crate::platform::syphon::available() => {
                return crate::platform::syphon::unavailable()
                    .unwrap_or_default()
                    .to_string();
            }
            VideoSource::Syphon { server, .. } if server.is_empty() => {
                return "no Syphon server chosen".to_string();
            }
            VideoSource::Syphon { server, .. } => server.clone(),
            VideoSource::Ndi { source, .. } if source.is_empty() => {
                return crate::video::ndi::missing()
                    .unwrap_or("no NDI® source chosen")
                    .to_string();
            }
            VideoSource::Ndi { source, .. } => source.clone(),
            VideoSource::File { .. } => "clip".to_string(),
            VideoSource::Camera { device } if device.is_empty() => "camera".to_string(),
            VideoSource::Camera { device } if device == "test" => "test pattern".to_string(),
            // A camera's ID is its name where the ID is one a person never reads.
            VideoSource::Camera { device } if crate::platform::video::NAMED_CAMERAS => self
                .cameras
                .iter()
                .find(|(id, _)| id == device)
                .map_or_else(|| device.clone(), |(_, name)| name.clone()),
            VideoSource::Camera { device } => device.clone(),
        };
        if self.silent {
            return format!("{what}, not responding");
        }
        match size {
            Some(size) => format!("{what}, {size}"),
            // Chosen, open, and nothing has arrived yet. A camera warming up looks like this
            // for a moment and a screen that was refused looks like it for ever, which is why
            // the error line under the panel is the other half of the answer.
            None => format!("{what}, no frame yet"),
        }
    }

    /// What the panel says under the audio source.
    pub fn audio_status(&self, want: &AudioSource) -> String {
        if matches!(self.sound, Sound::Decoding(_)) {
            return "decoding sound…".to_string();
        }
        match (&self.capture, want) {
            (Some(capture), AudioSource::Live { device }) => {
                let name = if device.is_system() {
                    "system audio".to_string()
                } else {
                    capture.device.clone()
                };
                format!("{name}, {} Hz mono", capture.sample_rate)
            }
            (Some(capture), _) => format!("{}, {} Hz", capture.device, capture.sample_rate),
            (None, _) => match &self.sound {
                Sound::Ready(reader) => {
                    format!("playing, {:.0} s", reader.track().duration())
                }
                _ => "nothing open".to_string(),
            },
        }
    }
}

impl crate::synth::Synth {
    /// Keep the Main Input in step with what the panel says, once a frame.
    ///
    /// The only place anything is opened or closed. It runs whether or not any node reads the
    /// input, because the panel's own preview and scope are drawn from it — and because a
    /// source a hand has switched on is on, which is what the panel says it is.
    pub(super) fn tick_main_input(
        &mut self,
        assets: &dyn crate::nodes::Assets,
        want: &MainInput,
        time: &crate::transport::Time,
    ) {
        // The project resolves an asset reference, and it is the one `Assets::resolve` every
        // node goes through: a reference under `assets/` lands in the folder and anything
        // else is a path of its own. The live half never learns where the project is.
        let resolve = |reference: &str| -> PathBuf { assets.resolve(reference) };
        let cache = assets.cache_dir();
        self.main_input.reconcile(want, &resolve, &cache, time);
        // Asking PulseAudio costs a round trip, so it happens once at the start and again
        // only when a hand has thrown the list away: devices do not appear mid-frame. The
        // question is whether it has been *asked*, not whether it answered with anything —
        // see `Live::asked`. The asking is a thread's and the answer lands here.
        if !self.main_input.asked_devices() {
            self.main_input.ask_devices();
        }
        self.main_input.take_devices();
        self.main_input.set_monitor(want.monitor);
        self.main_input.set_levels(want.levels);

        // Collected once for every node that reads them. A frame with no reader still takes
        // them, so a node added later starts on this frame's edges rather than on a backlog.
        let (crossings, total) = self.main_input.crossings();
        self.main_input_crossings = crossings;
        self.main_input.delivered(total);

        if let Some(e) = self.main_input.error() {
            log::debug!("main input: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Beat, Synth};
    use std::sync::Mutex;
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    /// A machine that answers only when the test lets it go, and says which thread asked it:
    /// a USB microphone that does not answer, as the webcam that held a first tick for 10.4 s.
    /// Asked on the ticking thread it answers at once, so a regression fails rather than hangs.
    fn slow_machine(ticking: ThreadId) -> (Lister, mpsc::Receiver<ThreadId>, mpsc::Sender<()>) {
        let (asked, asked_on) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let (asked, released) = (Mutex::new(asked), Mutex::new(released));
        let lister = Lister(Arc::new(move || {
            let here = std::thread::current().id();
            let _ = asked.lock().expect("unpoisoned").send(here);
            if here != ticking {
                let _ = released.lock().expect("unpoisoned").recv();
            }
            let microphone = audio::device::Listed {
                device: audio::device::Device::Pulse {
                    name: "slow.mic".into(),
                },
                label: "Slow microphone".into(),
                monitor: false,
            };
            (
                vec![microphone],
                vec![("/dev/video9".into(), "Slow camera".into())],
            )
        }));
        (lister, asked_on, release)
    }

    fn tick(synth: &mut Synth) {
        synth.step(Beat::Delta(1.0 / 60.0));
    }

    /// Tick until the listing has landed, and say whether it did.
    fn tick_until_listed(synth: &mut Synth) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            tick(synth);
            if !synth.main_input.cameras().is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// **No tick waits for the machine's devices.** The first tick asks, on a thread of its
    /// own, and it and the ticks after it run with empty lists while the machine does not
    /// answer; the answer lands on the first tick after it arrives. Asked once, not per tick,
    /// and again after *Look for devices again*.
    #[test]
    fn a_device_listing_that_does_not_answer_holds_no_tick() {
        let ticking = std::thread::current().id();
        let mut synth = Synth::default();
        let (lister, asked_on, release) = slow_machine(ticking);
        synth.main_input.lister = lister;

        for _ in 0..3 {
            tick(&mut synth);
        }
        let asker = asked_on
            .recv_timeout(Duration::from_secs(10))
            .expect("the first tick asks the machine");
        assert_ne!(asker, ticking, "the listing runs off the ticking thread");
        assert!(asked_on.try_recv().is_err(), "asked once, not once a tick");
        assert!(synth.main_input.asked_devices());
        assert!(
            synth.main_input.devices().is_empty() && synth.main_input.cameras().is_empty(),
            "nothing is listed until the machine answers"
        );

        release.send(()).expect("the listing waits");
        assert!(tick_until_listed(&mut synth), "the answer lands in a tick");
        assert_eq!(synth.main_input.devices()[0].label, "Slow microphone");
        assert_eq!(
            synth.main_input.cameras(),
            [("/dev/video9".to_string(), "Slow camera".to_string())]
        );

        synth.main_input.forget_devices();
        assert!(synth.main_input.cameras().is_empty());
        tick(&mut synth);
        let again = asked_on
            .recv_timeout(Duration::from_secs(10))
            .expect("forgetting is asking again");
        assert_ne!(again, ticking);
        release.send(()).expect("the second listing waits");
        assert!(tick_until_listed(&mut synth), "the second answer lands too");
    }

    /// **A source that delivers nothing for ten seconds is not responding**, where until then
    /// it has no frame yet; a camera that stops for ten seconds after its first is too, and a
    /// clip on a still playhead is not, since a clip does not stream.
    #[test]
    fn ten_silent_seconds_is_not_responding() {
        let opened = Instant::now();
        let at = |s: u64| opened + Duration::from_secs(s);
        let camera = VideoSource::Camera {
            device: "test".to_string(),
        };
        let mut live = Live {
            silence: Some(Silence::new(opened)),
            streams: true,
            ..Live::default()
        };
        live.listen(at(9));
        assert_eq!(live.video_status(&camera), "test pattern, no frame yet");
        live.listen(at(10));
        assert_eq!(live.video_status(&camera), "test pattern, not responding");

        live.frame = Some(Arc::new(Frame::solid(4, 4, [9, 9, 9, 255])));
        live.listen(at(11));
        assert_eq!(live.video_status(&camera), "test pattern, 4x4");
        live.listen(at(20));
        assert_eq!(
            live.video_status(&camera),
            "test pattern, 4x4",
            "the same frame"
        );
        live.listen(at(21));
        assert_eq!(live.video_status(&camera), "test pattern, not responding");

        let clip = VideoSource::File {
            asset: "assets/a.mp4".to_string(),
        };
        live.streams = false;
        live.listen(at(60));
        assert_eq!(live.video_status(&clip), "clip, 4x4");
    }
}
