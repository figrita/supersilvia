// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Windows backend, for 64-bit Windows 10 and 11: every service of [`crate::platform`],
//! each through the Windows API that replaces the Linux one, and GStreamer's official MSVC
//! release for the media.
//!
//! The file dialogs and the error box are `rfd`'s Win32 ones, the file manager is Explorer,
//! the folders are the shell's Known Folders, MIDI is WinMM through `midir`, the font
//! families are DirectWrite's system collection, the cameras are Media Foundation's through
//! `mfvideosrc`, the named audio inputs and the loopback are WASAPI's through `wasapi2src`,
//! screen capture is `d3d11screencapturesrc` on the primary monitor, and the hardware codecs
//! are NVENC, Quick Sync, AMF and Direct3D 12's through GStreamer. Like Linux it has no native
//! menu bar, no Syphon and no zero-copy path, and like a Mac it opens nothing ahead of the
//! NDI® plugin. The GPU's per-process counters are not read: `gpu` says so.
//!
//! **`unsafe` is allowed file by file**, where each is declared below: `clock`, whose
//! `GetThreadTimes` is `unsafe` by its binding; `dirs`, whose `SHGetKnownFolderPath` hands back
//! a string the caller frees; `check`, whose `RtlGetVersion` fills a structure it is handed;
//! and `fonts`, whose DirectWrite calls are COM methods, `unsafe` by their bindings. Every
//! block carries a `// SAFETY:` line. The crate root denies it everywhere else.

pub mod alert;
pub mod audio;
#[allow(unsafe_code)]
pub mod check;
#[allow(unsafe_code)]
pub mod clock;
#[allow(unsafe_code)]
pub mod dirs;
pub mod filedrop;
pub mod files;
#[allow(unsafe_code)]
pub mod fonts;
pub mod gpu;
pub mod menu;
pub mod midi;
pub mod ndi;
pub mod notices;
pub mod screen;
pub mod syphon;
pub mod video;

/// `value` as a quoted string GStreamer's pipeline parser reads back unchanged: a device's
/// path or ID, which on Windows holds backslashes, braces and `#`.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Media Foundation camera's path survives the parser: its backslashes and its quotes
    /// come back as they went in.
    #[test]
    fn a_quoted_value_is_read_back_as_it_was() {
        gstreamer::init().expect("gstreamer");
        let path = r"\\?\usb#vid_046d&pid_0825&mi_00#7&1a2b3c&0&0000#{e5323777}\global";
        let caps: gstreamer::Caps = format!("video/x-raw,path={}", quoted(path))
            .parse()
            .expect("caps");
        let read = caps.structure(0).and_then(|s| s.get::<String>("path").ok());
        assert_eq!(read.as_deref(), Some(path));
    }
}
