// SPDX-License-Identifier: AGPL-3.0-or-later

//! Audio capture by name, through PulseAudio — which PipeWire answers.
//!
//! cpal's Linux host is ALSA, and ALSA does not expose a PipeWire **monitor** — the stream of
//! what is already going to the speakers. Capturing that is the whole of silvia's
//! `loopback/` directory, which asks a person to run a script, load two PulseAudio modules,
//! and then re-point every application they wanted to hear at a null sink. It is also what
//! OBS calls Desktop Audio and gets for free, because OBS talks to PulseAudio directly.
//!
//! So does this. GStreamer is already a dependency for cameras and clips, `pulsesrc` is
//! built, and PipeWire answers the PulseAudio protocol, so a named source opens a monitor
//! with no modules, no scripts and nothing to undo afterwards. `@DEFAULT_MONITOR@` is
//! resolved by the server on connect, which means **the loopback follows the default
//! output**: change your output to the headset and what is captured changes with it.

use crate::platform::audio::Source;
use gstreamer as gst;
use gstreamer::prelude::*;

/// Every audio source PulseAudio can see, microphones and monitors together, in server
/// order.
///
/// Empty where there is no PulseAudio server to ask, which is not an error: the panel still
/// offers the default input and the loopback, and the loopback is the one that will then fail
/// with something a person can read.
pub fn sources() -> Vec<Source> {
    if gst::init().is_err() {
        return Vec::new();
    }
    // Ours before any monitor loads a system's NDI plugin in its place: see `video::ndi`.
    let _ = crate::video::ndi::register();
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Audio/Source"), None);
    if monitor.start().is_err() {
        return Vec::new();
    }
    let found = monitor
        .devices()
        .iter()
        .filter_map(|d| {
            let props = d.properties()?;
            // `device.class` is PulseAudio's own word for it: `monitor` on a sink's monitor,
            // `sound` on a capture device. Falling back to the name's suffix covers a server
            // that does not set the property, which is how the name is written anyway.
            let name: String = props
                .get("device.name")
                .or_else(|_| props.get("node.name"))
                .ok()?;
            let class: String = props.get("device.class").unwrap_or_default();
            let monitor = class == "monitor" || name.ends_with(".monitor");
            Some(Source {
                name,
                label: d.display_name().to_string(),
                monitor,
            })
        })
        .collect();
    monitor.stop();
    found
}

/// The source element for a named PulseAudio source, up to the conversion.
///
/// `provide-clock=false` and `do-timestamp=true`: the pipeline exists to hand blocks to an
/// analyzer as they arrive, and nothing downstream of the sink cares what time it is, so
/// there is no reason for the sound card to become the pipeline's clock.
pub fn element(name: &str) -> Result<String, String> {
    Ok(format!(
        "pulsesrc device=\"{name}\" provide-clock=false do-timestamp=true"
    ))
}

/// A loopback read without GStreamer. Never made here: every name, the loopback's included,
/// is a PulseAudio source [`element`] opens.
pub enum Tap {}

impl Tap {
    /// `None`: GStreamer opens every name.
    pub fn open(_name: &str) -> Option<Result<Self, String>> {
        None
    }

    /// Never called.
    pub fn rate(&self) -> u32 {
        match *self {}
    }

    /// Never called.
    pub fn start(&mut self, _analyze: impl FnMut(&[f32]) + Send + 'static) -> Result<(), String> {
        match *self {}
    }
}
