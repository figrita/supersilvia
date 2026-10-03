// SPDX-License-Identifier: AGPL-3.0-or-later

//! Audio capture by name on Windows: WASAPI lists the inputs and the outputs, and GStreamer's
//! `wasapi2src` captures one by its endpoint ID.
//!
//! **A loopback is an output read backwards.** WASAPI captures what an output endpoint is
//! playing when the capture is opened in loopback mode on it, with nothing to install, as
//! PipeWire answers a monitor. `wasapi2deviceprovider` lists each output as an `Audio/Source`
//! of its own, marked `wasapi2.device.loopback`. A loopback is named as PulseAudio names a
//! monitor, the endpoint's ID with `.monitor` after it, so a saved file says what it is in the
//! words [`crate::audio::device`] reads; [`DEFAULT_MONITOR`] is `wasapi2src`'s loopback with no
//! device named, which is the default output, and follows it when the default changes.
//!
//! Only `wasapi2`'s devices are listed: the older `wasapi`, DirectSound and kernel-streaming
//! providers list the same endpoints again under other names.

use crate::audio::device::DEFAULT_MONITOR;
use crate::platform::audio::Source;
use gstreamer as gst;
use gstreamer::prelude::*;

/// What a loopback's name ends in, after the output endpoint's ID.
const MONITOR: &str = ".monitor";

/// Every input and every output's loopback WASAPI lists, in its order.
///
/// Empty where GStreamer cannot be asked, which is not an error: the panel still offers the
/// default input and the loopback.
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
            if props.get::<String>("device.api").ok()? != "wasapi2" {
                return None;
            }
            let id: String = props.get("device.id").ok()?;
            let loopback = props
                .get::<bool>("wasapi2.device.loopback")
                .unwrap_or(false);
            Some(Source {
                name: if loopback {
                    format!("{id}{MONITOR}")
                } else {
                    id
                },
                label: d.display_name().to_string(),
                monitor: loopback,
            })
        })
        .collect();
    monitor.stop();
    found
}

/// The source element for an input by its endpoint ID, an output's loopback by the ID with
/// `.monitor` after it, or the default output's by [`DEFAULT_MONITOR`], up to the conversion.
///
/// `provide-clock=false` and `do-timestamp=true`, as Linux's `pulsesrc`: the pipeline hands
/// blocks to an analyzer as they arrive, and nothing downstream of the sink cares what time it
/// is.
pub fn element(name: &str) -> Result<String, String> {
    let device = if name == DEFAULT_MONITOR {
        "loopback=true".to_string()
    } else if let Some(id) = name.strip_suffix(MONITOR) {
        format!("device={} loopback=true", super::quoted(id))
    } else {
        format!("device={}", super::quoted(name))
    };
    Ok(format!(
        "wasapi2src {device} provide-clock=false do-timestamp=true"
    ))
}

/// A loopback read without GStreamer. Never made here: every name, the loopback's included,
/// is a WASAPI endpoint [`element`] opens.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An input is opened by its ID, an output's loopback by the ID before `.monitor`, and the
    /// loopback's own word by the default output.
    #[test]
    fn each_name_opens_what_it_says() {
        let id = "{0.0.0.00000000}.{d3c1d5e2-0000-4b6a-9f1e-123456789abc}";
        let input = element("{0.0.1.00000000}.{aa}").unwrap();
        assert!(
            input.contains("device=\"{0.0.1.00000000}.{aa}\""),
            "{input}"
        );
        assert!(!input.contains("loopback"), "{input}");
        let output = element(&format!("{id}{MONITOR}")).unwrap();
        assert!(
            output.contains(&format!("device=\"{id}\" loopback=true")),
            "{output}"
        );
        let default = element(DEFAULT_MONITOR).unwrap();
        assert!(default.contains("loopback=true"), "{default}");
        assert!(!default.contains("device="), "{default}");
    }
}
