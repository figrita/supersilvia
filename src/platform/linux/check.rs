// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `--check` asks of a Linux machine: every GStreamer element the app makes, by the plugin
//! set distributions package it in, and the rest of the machine — the session, the libraries
//! the app opens at run time rather than links, the audio server and the MIDI sequencer.
//!
//! A library the binary links is the loader's to find before `main` runs, so a missing one
//! stops the app before it can say anything; `packaging/linux/TESTERS.md` names those. The
//! ones here are opened by name once the app is running — by wgpu, winit and the picture
//! windows — and are looked for where the loader would look: `LD_LIBRARY_PATH`, the loader's
//! cache, and the usual library folders.

use crate::check::{Group, Line, Verdict, el};
use std::path::{Path, PathBuf};

const CORE: &str = "gstreamer";
const BASE: &str = "gst-plugins-base";
const GOOD: &str = "gst-plugins-good";
const BAD: &str = "gst-plugins-bad";
const UGLY: &str = "gst-plugins-ugly";
const LIBAV: &str = "gst-libav";
const PIPEWIRE: &str = "pipewire's GStreamer plugin";

/// Every element the app makes on Linux, by name in `src/` or inside a pipeline string there,
/// and those `decodebin` reaches for the files `nodes::Accepts` lets in — the Linux side of
/// `ELEMENTS` in `packaging/macos/build-app.sh`.
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
        elements: &[el("v4l2src", GOOD)],
    },
    Group {
        what: "microphones",
        required: false,
        elements: &[el("pulsesrc", GOOD)],
    },
    Group {
        what: "screen capture",
        required: false,
        elements: &[el("pipewiresrc", PIPEWIRE)],
    },
    Group {
        what: "zero-copy video",
        required: false,
        elements: &[el("vapostproc", BAD)],
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

/// Libraries opened by name at run time, each with what opens it.
const VULKAN: (&str, &str) = ("libvulkan.so.1", "Vulkan loader");
const WAYLAND: &[&str] = &["libwayland-client.so.0", "libxkbcommon.so.0"];
const X11: &[&str] = &[
    "libX11.so.6",
    "libX11-xcb.so.1",
    "libXcursor.so.1",
    "libXrandr.so.2",
    "libXi.so.6",
    "libxkbcommon-x11.so.0",
];

/// Folders the loader searches on the distributions the AppImage is for, where its cache is
/// not readable.
const LIBRARY_DIRS: &[&str] = &[
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/lib",
    "/lib64",
    "/lib/x86_64-linux-gnu",
    "/lib",
    "/usr/local/lib",
];

/// `Fedora Linux 44 (KDE Plasma), kernel 7.1.10, KDE on wayland`: the distribution as
/// `os-release` names it, the kernel, and the desktop and session as the session says.
pub fn os() -> String {
    let release = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| pretty_name(&text))
        .unwrap_or_else(|| "Linux".to_owned());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".to_owned(), |k| k.trim().to_owned());
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "unknown".to_owned())
    };
    format!(
        "{release}, kernel {kernel}, {} on {}",
        var("XDG_CURRENT_DESKTOP"),
        var("XDG_SESSION_TYPE")
    )
}

/// `PRETTY_NAME` out of an `os-release`, its quotes taken off.
fn pretty_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("PRETTY_NAME=")?;
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// The session, the run-time libraries, the audio server and the MIDI sequencer.
pub fn machine() -> Vec<Line> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    let mut lines = vec![session(wayland, x11)];
    lines.push(match find_library(VULKAN.0) {
        Some(path) => Line::new(Verdict::Pass, VULKAN.1, path.display().to_string()),
        None => Line::new(
            Verdict::Fail,
            VULKAN.1,
            format!(
                "{} not found — install the Vulkan loader and your GPU's Vulkan driver",
                VULKAN.0
            ),
        ),
    });
    // The editor's window is Wayland's in a Wayland session and X11's in any other.
    let window: &[&str] = if wayland || !x11 { WAYLAND } else { X11 };
    let missing: Vec<&str> = window
        .iter()
        .copied()
        .filter(|name| find_library(name).is_none())
        .collect();
    lines.push(if missing.is_empty() {
        Line::new(Verdict::Pass, "window libraries", window.join(", "))
    } else {
        Line::new(
            Verdict::Fail,
            "window libraries",
            format!("{} not found", missing.join(", ")),
        )
    });
    lines.push(audio_server());
    lines.push(if Path::new("/dev/snd/seq").exists() {
        Line::new(Verdict::Pass, "MIDI", "/dev/snd/seq")
    } else {
        Line::new(
            Verdict::Warn,
            "MIDI",
            "no ALSA sequencer at /dev/snd/seq — MIDI devices are not seen (load snd-seq)",
        )
    });
    lines
}

/// Which session the editor would open in, and what that costs.
fn session(wayland: bool, x11: bool) -> Line {
    let label = "session";
    if wayland {
        Line::new(Verdict::Pass, label, "Wayland")
    } else if x11 {
        Line::new(
            Verdict::Warn,
            label,
            "X11 — the editor opens, but pop-out and fullscreen pictures need a Wayland session",
        )
    } else {
        Line::new(
            Verdict::Warn,
            label,
            "no display here — the editor needs a Wayland session (X11 opens it without \
             picture windows)",
        )
    }
}

/// A PipeWire or PulseAudio socket, which microphones, the loopback and the monitors reach.
fn audio_server() -> Line {
    let label = "audio server";
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let found = runtime.as_deref().and_then(|dir| {
        ["pipewire-0", "pulse/native"]
            .iter()
            .map(|socket| dir.join(socket))
            .find(|path| path.exists())
    });
    match found {
        Some(path) => Line::new(Verdict::Pass, label, path.display().to_string()),
        None => Line::new(
            Verdict::Warn,
            label,
            "no PipeWire or PulseAudio socket — microphones and system audio find nothing",
        ),
    }
}

/// Where the loader would find `soname`: in `LD_LIBRARY_PATH`, in its cache, or in the usual
/// folders. `None` where it is none of them.
pub fn find_library(soname: &str) -> Option<PathBuf> {
    let from_env: Vec<PathBuf> = std::env::var_os("LD_LIBRARY_PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    if let Some(path) = from_env.iter().map(|d| d.join(soname)).find(|p| p.exists()) {
        return Some(path);
    }
    if let Some(path) = std::fs::read("/etc/ld.so.cache")
        .ok()
        .and_then(|cache| in_cache(&cache, soname))
        .filter(|p| p.exists())
    {
        return Some(path);
    }
    LIBRARY_DIRS
        .iter()
        .map(|d| Path::new(d).join(soname))
        .find(|p| p.exists())
}

/// The first path in a loader cache whose file name is `soname`. Both of glibc's cache
/// formats keep every path as a NUL-terminated string, so a path is found by its tail — a `/`,
/// the name, a NUL — and read back to the NUL before it.
fn in_cache(cache: &[u8], soname: &str) -> Option<PathBuf> {
    let tail: Vec<u8> = [b"/", soname.as_bytes(), b"\0"].concat();
    let end = cache
        .windows(tail.len())
        .position(|w| w == tail.as_slice())?;
    let start = cache[..end]
        .iter()
        .rposition(|b| *b == 0)
        .map_or(0, |i| i + 1);
    let path = std::str::from_utf8(&cache[start..end + tail.len() - 1]).ok()?;
    path.starts_with('/').then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_read_out_of_a_loader_cache() {
        let cache = b"glibc-ld.so.cache1.1\0\0\0libfoo.so.1\0/usr/lib64/libfoo.so.1\0libbar.so.2\0/usr/lib/libbar.so.2\0";
        assert_eq!(
            in_cache(cache, "libbar.so.2"),
            Some(PathBuf::from("/usr/lib/libbar.so.2"))
        );
        assert_eq!(
            in_cache(cache, "libfoo.so.1"),
            Some(PathBuf::from("/usr/lib64/libfoo.so.1"))
        );
        assert_eq!(in_cache(cache, "libbaz.so.1"), None);
        assert_eq!(in_cache(cache, "foo.so.1"), None, "a name is matched whole");
    }

    /// This machine's loader finds glibc's own maths library, which every Linux has.
    #[test]
    fn the_loader_finds_libm() {
        assert!(find_library("libm.so.6").is_some());
    }

    #[test]
    fn the_distribution_is_os_release_s_pretty_name() {
        let text = "NAME=\"Fedora Linux\"\nVERSION_ID=44\n\
                    PRETTY_NAME=\"Fedora Linux 44 (KDE Plasma Desktop Edition)\"\n";
        assert_eq!(
            pretty_name(text).as_deref(),
            Some("Fedora Linux 44 (KDE Plasma Desktop Edition)")
        );
        assert_eq!(
            pretty_name("PRETTY_NAME=Arch\n").as_deref(),
            Some("Arch"),
            "unquoted"
        );
        assert_eq!(pretty_name("NAME=x\n"), None);
    }

    #[test]
    fn a_session_without_wayland_says_what_it_costs() {
        assert_eq!(session(true, true).verdict, Verdict::Pass);
        assert!(
            session(false, true)
                .detail
                .contains("need a Wayland session")
        );
        assert_eq!(session(false, false).verdict, Verdict::Warn);
    }
}
