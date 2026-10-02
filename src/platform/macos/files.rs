// SPDX-License-Identifier: AGPL-3.0-or-later

//! File dialogs through `NSOpenPanel`, and the Finder and a text editor through `open`: `open`
//! on a folder, `open -R` on a file, which selects it in its folder, and `open -t` on a text
//! file, which opens it in the default text editor rather than whatever claims its extension.
//!
//! [`pick`] is `rfd`'s native macOS backend with its default features off, which are Linux's
//! portal backend and nothing the Mac uses. AppKit runs a panel on the main thread alone, and
//! [`pick`] is only ever called from a thread of its own so the frame never waits: rfd's
//! blocking `FileDialog` hands the panel to the main thread with GCD's `dispatch_sync` onto
//! the main queue, and this thread waits there for the answer. The main thread is in
//! `NSApp`'s `run`, which the winit loop `render::picture::run` starts calls, so the main
//! queue is drained. The panel is application-modal, run by `runModal` from that queue's
//! block rather than from inside a winit callback, and winit's run-loop observers are
//! registered for the common modes, the modal panel's among them, so the editor's loop goes
//! on under it. The picture windows draw on threads of their own.
//!
//! rfd asks one thing of `NSApp` from this thread first, whether it is running, and panics if
//! not: under our loop it always is, and a thread that died without answering reads as a
//! dialog closed without a choice.
//!
//! Not `rfd::AsyncFileDialog`: in rfd 0.15 its future also asks `NSApplication` for the main
//! window and the window list from the calling thread, which AppKit does not promise to
//! answer off the main thread, and then attaches the panel as a sheet to whichever window
//! that is — possibly a borderless picture window filling a projector — where the blocking
//! call puts up a free panel above every window. See `docs/decisions.md`.

use crate::platform::files::Pick;
use std::path::{Path, PathBuf};

/// Put the Mac's open panel up and wait for the answer. `None` is a panel closed without one.
pub fn pick(pick: Pick, start: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    if let Pick::File(label, extensions) = pick
        && !extensions.is_empty()
    {
        dialog = dialog.add_filter(label, extensions);
    }
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    match pick {
        Pick::Folder => dialog.pick_folder(),
        Pick::File(..) => dialog.pick_file(),
    }
}

/// What the file manager is called, for a button that opens it: *Show in Finder*.
pub const MANAGER: &str = "Finder";

/// Show a folder in the Finder. Returns at once.
pub fn reveal(dir: PathBuf) {
    open(&[], dir);
}

/// Show a file in the Finder, selected in the folder holding it. Returns at once.
pub fn reveal_file(file: PathBuf) {
    open(&["-R"], file);
}

/// Open a text file in the default text editor. Returns at once.
pub fn edit_text(file: PathBuf) {
    open(&["-t"], file);
}

/// `open` with these flags on a path, on a thread of its own which then waits, for the reason
/// Linux's does: the frame thread does not wait for the Finder, and a reveal leaves no zombie
/// behind.
fn open(flags: &'static [&'static str], path: PathBuf) {
    std::thread::spawn(move || {
        match std::process::Command::new("open")
            .args(flags)
            .arg(&path)
            .spawn()
        {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => log::warn!("could not open {}: {e}", path.display()),
        }
    });
}
