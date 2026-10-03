// SPDX-License-Identifier: AGPL-3.0-or-later

//! File dialogs through the shell's `IFileOpenDialog`, by way of `rfd`, and Explorer for the
//! rest: `explorer` on a folder opens it, `explorer /select,` on a file opens the folder holding
//! it with the file selected, and `explorer` on a text file opens it with whatever the shell's
//! default verb for its type runs.
//!
//! [`pick`] is `rfd`'s Win32 backend, which needs none of its features. The dialog is modal to
//! no window and runs its own message loop on the calling thread, so [`pick`] is only ever
//! called from a thread of its own, as on the other machines, and the editor's loop goes on
//! under it.
//!
//! **Explorer reads its own command line**, not the C runtime's: a comma separates its
//! switches and a path is only whole inside quotes, so every path is handed over quoted, as
//! written, rather than through `Command::arg`'s quoting, which leaves a path with no space in
//! it bare.

use crate::platform::files::Pick;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};

/// Put the shell's open dialog up and wait for the answer. `None` is a dialog closed without
/// one.
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

/// What the file manager is called, for a button that opens it: *Show in File Explorer*.
pub const MANAGER: &str = "File Explorer";

/// Show a folder in File Explorer. Returns at once.
pub fn reveal(dir: PathBuf) {
    explorer("", dir);
}

/// Show a file in File Explorer, selected in the folder holding it. Returns at once.
pub fn reveal_file(file: PathBuf) {
    explorer("/select,", file);
}

/// Open a text file in whatever the shell opens its type with. Returns at once.
pub fn edit_text(file: PathBuf) {
    explorer("", file);
}

/// `explorer` with a switch and a quoted path, on a thread of its own which then waits, for
/// the reason Linux's does: the frame thread does not wait for Explorer, and a reveal leaves no
/// handle behind.
fn explorer(switch: &'static str, path: PathBuf) {
    std::thread::spawn(move || {
        match std::process::Command::new("explorer")
            .raw_arg(format!("{switch}\"{}\"", path.display()))
            .spawn()
        {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => log::warn!("could not open {}: {e}", path.display()),
        }
    });
}
