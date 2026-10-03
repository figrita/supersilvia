// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `--check` asks of a Windows machine: every GStreamer element the app makes, which the
//! folder carries from GStreamer's official release, and the one library past it the app
//! opens by name at run time, the Vulkan loader, which a GPU driver installs.
//!
//! **The version is the kernel's own**, from `RtlGetVersion`, which reports it whatever
//! compatibility the executable declares; `GetVersionEx` reports Windows 8 to an executable
//! whose manifest names nothing later. Windows 11 is a build of 10.0 from 22000 on.

use crate::check::{Group, Line, Verdict, el};
use std::path::PathBuf;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

const CORE: &str = "gstreamer";
const BASE: &str = "gst-plugins-base";
const GOOD: &str = "gst-plugins-good";
const BAD: &str = "gst-plugins-bad";
const UGLY: &str = "gst-plugins-ugly";
const LIBAV: &str = "gst-libav";

/// Every element the app makes on Windows, by name in `src/` or inside a pipeline string there,
/// and those `decodebin` reaches for the files `nodes::Accepts` lets in; the hardware codecs are
/// `platform::video::CODECS`, which `--check` asks after separately.
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
            el("av1parse", BAD),
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
        elements: &[el("mfvideosrc", BAD)],
    },
    Group {
        what: "microphones",
        required: false,
        elements: &[el("wasapi2src", BAD)],
    },
    Group {
        what: "screen capture",
        required: false,
        elements: &[el("d3d11screencapturesrc", BAD), el("d3d11download", BAD)],
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
            el("dav1ddec", BAD),
            el("asfdemux", UGLY),
            el("avdec_h264", LIBAV),
            el("avdec_h265", LIBAV),
            el("avdec_mpeg4", LIBAV),
            el("avdec_mpeg2video", LIBAV),
            el("avdec_aac", LIBAV),
            el("avdec_ac3", LIBAV),
            el("avdec_wmav2", LIBAV),
        ],
    },
];

/// The Vulkan loader every picture is drawn through.
const VULKAN: (&str, &str) = ("vulkan-1.dll", "Vulkan loader");

/// `Windows 11, build 26100, x86_64`: the release the kernel's version says, its build, and
/// the processor.
pub fn os() -> String {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is a local of the size its first field says, which the call fills.
    let read = unsafe { RtlGetVersion(&raw mut info) };
    let arch = std::env::consts::ARCH;
    if read.is_err() {
        return format!("Windows, version unknown, {arch}");
    }
    let release = match (info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber) {
        (10, 0, build) if build >= 22000 => "11".to_owned(),
        (10, 0, _) => "10".to_owned(),
        (major, minor, _) => format!("{major}.{minor}"),
    };
    format!("Windows {release}, build {}, {arch}", info.dwBuildNumber)
}

/// The Vulkan loader, where Windows keeps it.
pub fn machine() -> Vec<Line> {
    let system = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join(VULKAN.0));
    vec![match system.filter(|path| path.exists()) {
        Some(path) => Line::new(Verdict::Pass, VULKAN.1, path.display().to_string()),
        None => Line::new(
            Verdict::Fail,
            VULKAN.1,
            format!(
                "{} not found — install your GPU's driver, which carries Vulkan",
                VULKAN.0
            ),
        ),
    }]
}
