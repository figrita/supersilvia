// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `--check` asks of a Mac: every GStreamer element the app makes, which the `.app`
//! carries and a development build takes from Homebrew's `gstreamer`. The rest of the machine
//! — the windows, the audio, MIDI — is the system's own on every Mac the app opens on, so
//! there is nothing further to ask.

use crate::check::{Group, Line, el};

const CORE: &str = "gstreamer";
const BASE: &str = "gst-plugins-base";
const GOOD: &str = "gst-plugins-good";
const BAD: &str = "gst-plugins-bad";
const UGLY: &str = "gst-plugins-ugly";
const LIBAV: &str = "gst-libav";

/// The rows of `packaging/gstreamer-plugins.txt` for macOS, grouped by what each serves; the
/// hardware codecs are `platform::video::CODECS`, which `--check` asks after separately.
pub const GROUPS: &[Group] = &[
    Group {
        what: "pipelines",
        required: true,
        elements: &[
            el("filesrc", CORE),
            el("filesink", CORE),
            el("queue", CORE),
            el("multiqueue", CORE),
            el("capsfilter", CORE),
            el("fakesink", CORE),
            el("typefind", CORE),
            el("appsrc", BASE),
            el("appsink", BASE),
            el("decodebin", BASE),
            el("uridecodebin", BASE),
            el("videoconvert", BASE),
            el("videoscale", BASE),
            el("audioconvert", BASE),
            el("audioresample", BASE),
            el("videotestsrc", BASE),
        ],
    },
    Group {
        what: "video import",
        required: false,
        elements: &[
            el("mp4mux", GOOD),
            el("qtdemux", GOOD),
            el("h264parse", BAD),
            el("h265parse", BAD),
        ],
    },
    Group {
        what: "PNG and JPEG",
        required: false,
        elements: &[el("pngenc", GOOD), el("pngdec", GOOD), el("jpegdec", GOOD)],
    },
    Group {
        what: "Text node",
        required: false,
        elements: &[el("textoverlay", BASE)],
    },
    Group {
        what: "cameras",
        required: false,
        elements: &[el("avfvideosrc", BAD)],
    },
    Group {
        what: "microphones",
        required: false,
        elements: &[el("osxaudiosrc", GOOD)],
    },
    Group {
        what: "clip formats",
        required: false,
        elements: &[
            el("matroskademux", GOOD),
            el("avidemux", GOOD),
            el("wavparse", GOOD),
            el("id3demux", GOOD),
            el("aacparse", GOOD),
            el("mpegaudioparse", GOOD),
            el("flacparse", GOOD),
            el("flacdec", GOOD),
            el("mpg123audiodec", GOOD),
            el("vp8dec", GOOD),
            el("vp9dec", GOOD),
            el("oggdemux", BASE),
            el("opusdec", BASE),
            el("vorbisdec", BASE),
            el("tsdemux", BAD),
            el("aiffparse", BAD),
            el("av1parse", BAD),
            el("dav1ddec", BAD),
            el("asfdemux", UGLY),
            el("avdec_mpeg4", LIBAV),
            el("avdec_mpeg2video", LIBAV),
            el("avdec_aac", LIBAV),
            el("avdec_ac3", LIBAV),
            el("avdec_wmav2", LIBAV),
        ],
    },
];

/// Nothing past GStreamer and the GPU.
pub fn machine() -> Vec<Line> {
    Vec::new()
}

/// `macOS 14.2, aarch64`: the version `sw_vers` reports, and the processor.
pub fn os() -> String {
    let version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "version unknown".to_owned());
    format!("macOS {version}, {}", std::env::consts::ARCH)
}
