// SPDX-License-Identifier: AGPL-3.0-or-later

//! A box on the desktop that says why the app cannot go on, for an exit before any window of
//! its own is up: `rfd`'s, which with no parent window is a `CFUserNotification` alert and
//! needs no running `NSApplication`. Called on the main thread only, since rfd refuses a
//! dialog from any other before the application runs.

/// Put an error box up with `title` and `text`, and wait for it to be closed.
pub fn fatal(title: &str, text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
