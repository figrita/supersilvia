// SPDX-License-Identifier: AGPL-3.0-or-later

//! Screen capture on Windows: the primary monitor, through `d3d11screencapturesrc`.
//!
//! **There is no picker to ask.** Windows lets any application read the screen, and has no
//! desktop dialog for choosing one the way xdg-desktop-portal and ScreenCaptureKit do — the
//! graphics capture picker belongs to WinRT and a window to parent it on. So [`ask`] answers
//! at once, and what it answers is the primary monitor, with the pointer drawn in as Linux asks
//! the portal for it. Choosing another monitor or a window is not offered.
//!
//! **Direct3D 11's, not Direct3D 12's.** Both are in GStreamer's release; the Direct3D 11
//! element is the older, and needs nothing past the desktop duplication every Windows 10
//! machine has. Its frames are Direct3D textures, which `d3d11download` copies into memory for
//! the bytes chain every other source delivers through. A [`Cast`] holds nothing: the pipeline
//! is the capture, and dropping it ends it.

use crate::platform::screen::Head;
use std::sync::mpsc;

/// What a pipeline captures the primary monitor with, and the copy of its frames into memory.
const ELEMENT: &str = "d3d11screencapturesrc show-cursor=true ! d3d11download";

/// A running screen capture, as the rest of the app holds it. There is no session behind it.
pub struct Cast;

impl Cast {
    /// What a pipeline reads this cast by.
    // The signature every backend answers `platform::screen` with, over nothing to read.
    #[allow(clippy::unused_self)]
    pub fn stream(&self) -> Stream {
        Stream
    }
}

/// A cast as a pipeline's source names it: the primary monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream;

impl Stream {
    /// The pipeline's source element for this stream.
    // The signature every backend answers `platform::screen` with, over nothing to read.
    #[allow(clippy::unused_self)]
    pub fn head(&self) -> Head {
        Head::Element(ELEMENT.to_string())
    }
}

/// The answer, given before anyone polls for it.
pub struct Pending(mpsc::Receiver<Result<Cast, String>>);

impl Pending {
    /// The answer, if there is one yet. Never waits; here there always is one.
    pub fn poll(&self) -> Option<Result<Cast, String>> {
        self.0.try_recv().ok()
    }
}

/// The primary monitor, answered at once.
pub fn ask() -> Pending {
    let (answer, pending) = mpsc::channel();
    let _ = answer.send(Ok(Cast));
    Pending(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer is there on the first poll, and the stream names the primary monitor's
    /// capture.
    #[test]
    fn the_primary_monitor_is_answered_at_once() {
        let cast = ask().poll().expect("answered").expect("a cast");
        let Head::Element(element) = cast.stream().head() else {
            panic!("a pipeline element");
        };
        assert!(element.starts_with("d3d11screencapturesrc"), "{element}");
    }
}
