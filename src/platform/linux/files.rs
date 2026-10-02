// SPDX-License-Identifier: AGPL-3.0-or-later

//! File dialogs through xdg-desktop-portal, and the file manager through `xdg-open` and the
//! portal.
//!
//! A folder is shown by `xdg-open` on it. A *file* is shown by the portal's `OpenURI`
//! `OpenDirectory`, handed the file itself, which opens the folder holding it — and in a file
//! manager that answers `org.freedesktop.FileManager1`, as Dolphin and Nautilus do, with the file
//! selected; where the portal is not there, `xdg-open` on the folder holding it.
//!
//! `rfd`'s `xdg-portal` backend rather than its gtk3 one: on KDE/Wayland the portal puts up
//! the desktop's own dialog in the user's theme, and GTK stays out of the process. See
//! `docs/decisions.md`.
//!
//! rfd's blocking calls are a round trip over D-Bus to the desktop and then as long as the
//! person takes, so [`pick`] is only ever called from a thread of its own, and the caller
//! collects the answer through a channel the frame polls.

use crate::platform::files::Pick;
use std::path::{Path, PathBuf};

/// Put the desktop's dialog up and wait for the answer. `None` is a dialog closed without
/// one.
pub fn pick(pick: Pick, start: Option<&Path>) -> Option<PathBuf> {
    // Inside the shared runtime: `rfd` drives its own futures with `pollster` and brings no
    // runtime, and the D-Bus connection underneath it is the one every portal call in the
    // process shares. See `super::portal`.
    let _guard = super::portal::runtime().enter();
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

/// What the file manager is called, for a button that opens it: *Show in Files*.
pub const MANAGER: &str = "Files";

/// Show a folder in the desktop's file manager. Returns at once.
pub fn reveal(dir: PathBuf) {
    // On a thread of its own, which then waits: a file manager takes as long as it takes,
    // and the frame thread is not allowed to find out how long that is.
    std::thread::spawn(move || xdg_open(&dir));
}

/// Show a file in the desktop's file manager, in the folder holding it and selected where the
/// file manager can. Returns at once.
pub fn reveal_file(file: PathBuf) {
    std::thread::spawn(move || {
        if let Err(e) = open_directory(&file) {
            log::info!(
                "the portal did not show {}: {e}; opening the folder holding it",
                file.display()
            );
            xdg_open(file.parent().unwrap_or(&file));
        }
    });
}

/// Open a text file in whatever the desktop opens it with. Returns at once.
pub fn edit_text(file: PathBuf) {
    std::thread::spawn(move || xdg_open(&file));
}

/// The portal's `OpenDirectory` on a file: the folder holding it, opened by the desktop.
fn open_directory(file: &Path) -> Result<(), String> {
    let handle = std::fs::File::open(file).map_err(|e| e.to_string())?;
    // The request the portal answers with is dropped inside the conversation: see
    // `portal::converse`.
    super::portal::converse(async move {
        ashpd::desktop::open_uri::OpenDirectoryRequest::default()
            .send(&handle)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

/// `xdg-open` on a path, waited for so a reveal leaves no zombie behind for the rest of the
/// session.
fn xdg_open(path: &Path) {
    match std::process::Command::new("xdg-open").arg(path).spawn() {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(e) => log::warn!("could not open {}: {e}", path.display()),
    }
}
