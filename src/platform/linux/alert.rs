// SPDX-License-Identifier: AGPL-3.0-or-later

//! A box on the desktop that says why the app cannot go on, for an exit before any window of
//! its own is up.
//!
//! **There is no portal for a message.** `rfd`'s `xdg-portal` backend answers a
//! `MessageDialog` by running `zenity`, GNOME's dialog program, and a KDE desktop ships
//! `kdialog` instead. So the box is `zenity` through `rfd` where it is installed, `kdialog`
//! where that is, and nothing past what stderr and the log already say where neither is.

use std::path::Path;

/// Put an error box up with `title` and `text`, and wait for it to be closed. Returns at once
/// where the desktop has no program to put one up with.
pub fn fatal(title: &str, text: &str) {
    if on_path("zenity") {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(title)
            .set_description(text)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    } else if on_path("kdialog") {
        let shown = std::process::Command::new("kdialog")
            .args(["--title", title, "--error", text])
            .status();
        if let Err(e) = shown {
            log::warn!("kdialog did not start: {e}");
        }
    } else {
        log::warn!("neither zenity nor kdialog is installed, so no box was put up");
    }
}

/// Whether a program by this name is in a folder `$PATH` names.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| is_program(&dir.join(program))))
}

fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_found_on_the_path_and_a_made_up_one_is_not() {
        assert!(on_path("sh"), "every Linux has a sh");
        assert!(!on_path("supersilvia-no-such-program"));
    }
}
