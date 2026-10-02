// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Input: one video source and one audio source, chosen once for the whole rig.
//!
//! silvia's, and like the mixer it is the rig's rather than a node's. A `maininput` node
//! anywhere in the project reads what is chosen here, so a patch with eight of them opens one
//! camera and analyzes one signal. That is the whole argument for it: **one device, many
//! readers**. Choosing per node is what `camera`, `video` and `audioin` are for, and they stay
//! exactly as they are — this is the other shape, not a replacement for them.
//!
//! What is here is only the *choice*. Opening the device, decoding the file and holding the
//! portal session are [`crate::app::maininput`]'s, the way [`crate::mixer::Mixer`] describes
//! the decks and `render/mixer.rs` runs them.
//!
//! **Saved with the project, never undoable**, as silvia's is. Every save writes it into the
//! manifest and Open reads it back through [`MainInput::restored`], which keeps silvia's
//! rule: a clip, a sound file and the tuning come back, and a camera, a capture device or a
//! screen does not, so opening a project opens no device and puts up no picker. Choosing a
//! source is playing rather than editing, so none of it is a command.
//!
//! What is *not* here, from silvia's panel: the demo video, which was a bundled file for a web
//! page with nothing else to show; and the Gain / Expand / Smooth grid, because a band's level
//! is shaped by the graph here rather than by nine numbers behind the analyzer — `slew` and an
//! exciter do it where they can be seen, which is [decisions.md](../docs/decisions.md)'s call
//! and not a thing to quietly undo in a panel.

use crate::audio::{BANDS, BandConfig, Device, bands};
use serde::{Deserialize, Serialize};

/// Where the Main Input's own picture is uploaded for the panel to draw it.
///
/// A port on node zero, which is an id no graph ever hands out — `Graph::add` increments
/// before it uses one, so the first node is 1. The renderer keys an uploaded frame by port,
/// and the panel's picture is a picture with no node behind it: this is how it gets a key of
/// its own without a special case anywhere in `render/`.
pub const PREVIEW: crate::graph::PortRef =
    crate::graph::PortRef::new(crate::graph::NodeId(0), "frame");

/// Where the Main Input's picture comes from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VideoSource {
    /// Nothing. The node publishes black, and no device is open.
    #[default]
    None,
    /// A clip in the project's assets, played at speed one and looped.
    ///
    /// Deliberately without a transport: a source a whole rig reads wants to be *running*,
    /// and everything a performance does to a clip — speed, scrubbing, a position from a
    /// cable — is what the `video` node is for.
    File { asset: String },
    /// A capture device. Empty is whichever one GStreamer finds first.
    Camera { device: String },
    /// A screen, a window or a region, as chosen through the desktop's own picker.
    ///
    /// Nothing is named here, and nothing *can* be: the portal does the naming, and what it
    /// hands back is one session's. The token for resuming the same pick without asking again
    /// is a preference — it is issued by this desktop to this application and means nothing
    /// on another machine, so a project carrying one would be carrying a dead string.
    ///
    /// It has to stay out of here for a second reason, which cost an afternoon: this enum is
    /// compared against the last one to decide whether to reopen the source. A token written
    /// back into it after a successful pick is a *change*, so the next frame tore the session
    /// down and put the picker up again, for ever.
    Screen,
    /// A picture another app on the Mac publishes over Syphon, by the server's label, "App –
    /// Server", read top row first where `flip` says and with its alpha where `transparent`
    /// does. A server is not a device: it is reconnected on Open, and taken up whenever it
    /// runs.
    Syphon {
        server: String,
        #[serde(default)]
        flip: bool,
        #[serde(default)]
        transparent: bool,
    },
    /// A picture another machine sends over NDI®, by the source's name, `MACHINE (Stream)`,
    /// read with its alpha where `transparent` says. A source is not a device: it is taken up
    /// again on Open, whenever it is on the network.
    Ndi {
        source: String,
        #[serde(default)]
        transparent: bool,
    },
}

/// What the Main Input listens to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AudioSource {
    /// Nothing. Every band reads zero.
    #[default]
    None,
    /// A sound file in the project's assets, played at speed one and looped.
    File { asset: String },
    /// A capture device: a microphone, a line in, or the loopback.
    Live {
        #[serde(default)]
        device: Device,
    },
    /// The video source's own soundtrack, when the video source is a file that has one.
    Video,
}

impl AudioSource {
    /// The loopback: whatever is going to the default output.
    pub fn system() -> Self {
        Self::Live {
            device: Device::system(),
        }
    }
}

/// The Main Input as the project file carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MainInput {
    pub video: VideoSource,
    pub audio: AudioSource,
    /// Applied to every published level. One number, as silvia has it.
    pub gain: f32,
    /// Where each band listens, dragged on the panel's scope.
    ///
    /// Global rather than per node, which is the point: **one analysis read in eight places
    /// says the same thing in all eight.** It could not be otherwise: the tuning is applied on
    /// the audio thread, inside the one capture every node reads, so a per-node copy would
    /// mean whichever node ticked last decided what all of them saw. A node that wants a
    /// tuning of its own is an `audioin` or a `video`, which own their capture and so can.
    pub bands: [BandConfig; BANDS],
    /// The level each band fires its event at, dragged on the panel's meters.
    ///
    /// Global for exactly the reason the tuning is, and thresholds are crossed on the audio
    /// thread at the sample they happened on — which is the whole advantage of doing it there,
    /// and is not something a per-node copy on the frame thread could keep.
    pub levels: [f32; BANDS],
    /// How loudly the input is heard through the speakers. Zero is off, as everywhere else.
    ///
    /// Off by default, and it matters more here than anywhere: a microphone monitored through
    /// the speakers beside it is a howl, and the loopback monitored back into the output it
    /// came from is the same howl by another route.
    pub monitor: f32,
}

impl Default for MainInput {
    fn default() -> Self {
        Self {
            video: VideoSource::None,
            audio: AudioSource::None,
            gain: 1.0,
            bands: bands::DEFAULT,
            // One, the top of a band's range: nothing fires until a hand moves it down.
            levels: [1.0; BANDS],
            monitor: 0.0,
        }
    }
}

impl MainInput {
    /// The Main Input as Open brings it back from the file: silvia's `deserialize`.
    ///
    /// A clip, a Syphon server, an NDI source, a sound file, the video source's sound, the gain,
    /// the band tuning and the thresholds are kept. A camera, a capture device and a screen are
    /// **None**, because opening one is a device switched on or a picker put up that nobody
    /// asked for tonight; a Syphon server or an NDI source is another app's picture, which
    /// switches nothing on and asks nothing.
    /// The monitor is off, because a sound coming out of the speakers on Open is the same
    /// surprise.
    #[must_use]
    pub fn restored(self) -> Self {
        let video = match self.video {
            VideoSource::File { asset } => VideoSource::File { asset },
            kept @ (VideoSource::Syphon { .. } | VideoSource::Ndi { .. }) => kept,
            VideoSource::None | VideoSource::Camera { .. } | VideoSource::Screen => {
                VideoSource::None
            }
        };
        let audio = match self.audio {
            AudioSource::Live { .. } => AudioSource::None,
            kept => kept,
        };
        Self {
            video,
            audio,
            monitor: 0.0,
            ..self
        }
    }

    /// Is anything open at all? What the panel's header says when it is collapsed.
    pub fn is_idle(&self) -> bool {
        self.video == VideoSource::None && self.audio == AudioSource::None
    }

    /// A short line naming what is on, for the collapsed panel and the node's body.
    pub fn summary(&self) -> String {
        let video = match &self.video {
            VideoSource::None => "no video",
            VideoSource::File { .. } => "clip",
            VideoSource::Camera { .. } => "camera",
            VideoSource::Screen => "screen",
            VideoSource::Syphon { .. } => "syphon",
            VideoSource::Ndi { .. } => "ndi",
        };
        let audio = match &self.audio {
            AudioSource::None => "no audio".to_string(),
            AudioSource::File { .. } => "sound file".to_string(),
            AudioSource::Live { device } if device.is_system() => "system audio".to_string(),
            AudioSource::Live { .. } => "input".to_string(),
            AudioSource::Video => "clip audio".to_string(),
        };
        format!("{video}, {audio}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is nothing open, so a new project costs no device.
    #[test]
    fn a_new_main_input_opens_nothing() {
        let input = MainInput::default();
        assert!(input.is_idle());
    }

    /// Saved with the project, so it has to survive the round trip a manifest makes.
    #[test]
    fn a_choice_survives_the_project_file() {
        let input = MainInput {
            video: VideoSource::Camera {
                device: "/dev/video2".to_string(),
            },
            audio: AudioSource::system(),
            ..MainInput::default()
        };
        let json = serde_json::to_string(&input).expect("serializes");
        let back: MainInput = serde_json::from_str(&json).expect("reads back");
        assert_eq!(back, input);
    }

    /// An older project file has no Main Input at all, and must open as one that is off
    /// rather than fail to open.
    #[test]
    fn a_project_without_one_reads_as_idle() {
        let back: MainInput = serde_json::from_str("{}").expect("every field defaults");
        assert!(back.is_idle());
        assert_eq!(back.gain, 1.0);
    }

    /// Open keeps a file and the tuning, and lets go of every device and of the monitor.
    #[test]
    fn a_reopened_choice_keeps_its_files_and_opens_no_device() {
        let clip = VideoSource::File {
            asset: "assets/gumbasia.webm".to_string(),
        };
        let tuned = MainInput {
            video: clip.clone(),
            audio: AudioSource::Video,
            gain: 2.0,
            levels: [0.5; BANDS],
            monitor: 0.7,
            ..MainInput::default()
        };
        let back = tuned.clone().restored();
        assert_eq!(back.video, clip);
        assert_eq!(back.audio, AudioSource::Video);
        assert_eq!((back.gain, back.levels), (2.0, [0.5; BANDS]));
        assert_eq!(back.monitor, 0.0, "nothing is heard on Open");

        for video in [
            VideoSource::Camera {
                device: "/dev/video2".to_string(),
            },
            VideoSource::Screen,
        ] {
            let live = MainInput {
                video,
                audio: AudioSource::system(),
                ..tuned.clone()
            };
            let back = live.restored();
            assert!(back.is_idle(), "{back:?} opens a device");
            assert_eq!(back.gain, 2.0, "the tuning stays with the devices gone");
        }
    }

    /// An NDI source comes back on Open with its Transparent tick, as a Syphon server does, and
    /// a choice with no tick reads it as off.
    #[test]
    fn an_ndi_source_is_kept_on_open() {
        let ndi = VideoSource::Ndi {
            source: "STUDIO (Resolume Arena)".to_string(),
            transparent: true,
        };
        let input = MainInput {
            video: ndi.clone(),
            ..MainInput::default()
        };
        let json = serde_json::to_string(&input).expect("serializes");
        let back: MainInput = serde_json::from_str(&json).expect("reads back");
        assert_eq!(back.restored().video, ndi);
        let bare: VideoSource =
            serde_json::from_str(r#"{"kind":"ndi","source":"A (B)"}"#).expect("reads back");
        assert_eq!(
            bare,
            VideoSource::Ndi {
                source: "A (B)".to_string(),
                transparent: false,
            }
        );
    }

    /// A Syphon server comes back on Open with its two looks, since opening it switches nothing
    /// on; an older file's choice with no looks reads them as off.
    #[test]
    fn a_syphon_server_is_kept_on_open() {
        let syphon = VideoSource::Syphon {
            server: "Arena – Composition".to_string(),
            flip: true,
            transparent: false,
        };
        let input = MainInput {
            video: syphon.clone(),
            ..MainInput::default()
        };
        let json = serde_json::to_string(&input).expect("serializes");
        let back: MainInput = serde_json::from_str(&json).expect("reads back");
        assert_eq!(back.restored().video, syphon);
        let bare: VideoSource =
            serde_json::from_str(r#"{"kind":"syphon","server":"Arena – Composition"}"#)
                .expect("reads back");
        assert_eq!(
            bare,
            VideoSource::Syphon {
                server: "Arena – Composition".to_string(),
                flip: false,
                transparent: false,
            }
        );
    }

    /// The loopback is recognized however it arrived: chosen in the panel, or read back out
    /// of a project file saved last week.
    #[test]
    fn the_loopback_is_recognized_after_a_round_trip() {
        let json = serde_json::to_string(&AudioSource::system()).expect("serializes");
        let back: AudioSource = serde_json::from_str(&json).expect("reads back");
        assert!(matches!(back, AudioSource::Live { device } if device.is_system()));
    }
}
