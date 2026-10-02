// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which audio input a capture opens, and what there is to choose from.
//!
//! Two backends, because one of them cannot do the thing that matters most here: hear what is
//! already going to the speakers. cpal opens the default input on every machine, and cannot
//! open a loopback on any of them. A **named source** — a microphone by name, or a monitor of
//! what is playing — goes through GStreamer, by whatever element and whatever names
//! [`crate::platform::audio`] says the machine has: PulseAudio's, which PipeWire answers, on
//! Linux. See that module for why it is not cpal there.
//!
//! cpal stays the default path for a microphone. It is the shorter route to the same samples,
//! it is what the audio thread was written against, and nothing about a microphone needs a
//! name.
//!
//! **The names in a saved file are PulseAudio's**, because that is what this was written
//! against: [`Device::Pulse`] and [`DEFAULT_MONITOR`] are the file's words for a named source
//! and for the loopback, and another machine's backend reads them as that.

/// The PulseAudio name that resolves to the default output's monitor.
///
/// Resolved by the server when the stream connects rather than by us when the list is built,
/// which is what makes it follow the default output instead of pinning the one that happened
/// to be default when the panel was drawn.
pub const DEFAULT_MONITOR: &str = "@DEFAULT_MONITOR@";

/// Where a [`super::Capture`]'s samples come from.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Device {
    /// The default input device, through cpal. A microphone, in practice.
    #[default]
    Default,
    /// A named source, through GStreamer. Any source has a name, but the ones worth naming
    /// are the monitors: a sink's monitor is what that sink is playing.
    Pulse { name: String },
}

impl Device {
    /// The loopback: whatever is playing on the default output.
    pub fn system() -> Self {
        Self::Pulse {
            name: DEFAULT_MONITOR.to_string(),
        }
    }

    /// Is this the loopback, however it was spelled?
    pub fn is_system(&self) -> bool {
        matches!(self, Self::Pulse { name } if name == DEFAULT_MONITOR)
    }
}

/// One thing that can be listened to, as the panel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub device: Device,
    /// What the panel shows. The server's own description, which is the name the desktop's
    /// volume control uses for the same thing.
    pub label: String,
    /// True for a sink's monitor: what is being played rather than what is being said.
    pub monitor: bool,
}

/// Every named audio source the machine can see, microphones and monitors together, in the
/// order it lists them.
///
/// The two entries a person actually wants — the default microphone and the default output's
/// monitor — are not in here: they are not devices but standing instructions, and the panel
/// offers them above this list so that neither one pins a device that may be unplugged.
///
/// Empty where there is nothing to ask, which is not an error: the panel still offers the
/// default input and the loopback, and the loopback is the one that will then fail with
/// something a person can read.
pub fn list() -> Vec<Listed> {
    crate::platform::audio::sources()
        .into_iter()
        .map(|source| Listed {
            device: Device::Pulse { name: source.name },
            label: source.label,
            monitor: source.monitor,
        })
        .collect()
}
