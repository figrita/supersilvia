// SPDX-License-Identifier: AGPL-3.0-or-later

//! This user's folders on macOS: `~/Library/Application Support` for preferences and for the
//! log, which is where Apple asks an application to keep files it manages itself, and
//! `~/Documents` for projects, which are the person's own and where they browse to.

use std::path::PathBuf;

/// What a person sets for [`documents`] to answer.
pub const DOCUMENTS_UNSET: &str = "set HOME";

/// What to do about a folder this user is not allowed into, after saying so: on a Mac it is
/// usually the Documents prompt answered Don't Allow.
pub const DENIED_HINT: Option<&str> =
    Some("allow supersilvia in System Settings ▸ Privacy & Security ▸ Files and Folders");

/// What a folder's name cannot hold: the separator, the byte that ends a C string, and `:`,
/// which the Finder shows as `/` and the older Carbon paths read as a separator.
pub const NOT_IN_NAMES: &[char] = &['/', '\0', ':'];

/// `$HOME/Library/Application Support`. `None` without `$HOME`.
pub fn config() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    })
}

/// `$HOME/Library/Application Support`, as [`config`] is. `None` without `$HOME`.
pub fn data() -> Option<PathBuf> {
    config()
}

/// `$HOME/Documents`. `None` without `$HOME`.
pub fn documents() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents"))
}
