// SPDX-License-Identifier: AGPL-3.0-or-later

//! This user's folders on Windows, as the shell's Known Folders name them: the roaming
//! `AppData` for preferences, which follows a person to another machine of the same domain;
//! the local `AppData` for the log, which stays on this one; and the Documents folder for
//! projects, wherever the person or OneDrive has moved it.
//!
//! **Asked of the shell, not built from variables.** `%APPDATA%` and `%USERPROFILE%` name the
//! default places, and a Documents folder moved in its Properties, or into OneDrive by its
//! backup, is somewhere else; `SHGetKnownFolderPath` answers where it is.

use std::path::PathBuf;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows::core::GUID;

/// What a person sets for [`documents`] to answer.
pub const DOCUMENTS_UNSET: &str = "give Documents a location in its Properties";

/// What to do about a folder this user is not allowed into, after saying so: on Windows it is
/// usually Controlled folder access, which keeps every app it does not know out of Documents.
pub const DENIED_HINT: Option<&str> = Some(
    "allow supersilvia in Windows Security ▸ Virus & threat protection ▸ Ransomware protection",
);

/// What a folder's name cannot hold: the separators, the characters the shell reserves, and
/// the byte that ends a C string.
pub const NOT_IN_NAMES: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];

/// The roaming `AppData`. `None` where the shell names none.
pub fn config() -> Option<PathBuf> {
    known(&FOLDERID_RoamingAppData)
}

/// The local `AppData`. `None` where the shell names none.
pub fn data() -> Option<PathBuf> {
    known(&FOLDERID_LocalAppData)
}

/// The Documents folder, wherever it has been moved. `None` where the shell names none.
pub fn documents() -> Option<PathBuf> {
    known(&FOLDERID_Documents)
}

/// Where the shell says a Known Folder is, for this user.
fn known(folder: &GUID) -> Option<PathBuf> {
    // SAFETY: `folder` is one of the shell's own folder IDs, alive for the call, and no token
    // is passed, which is the calling user's.
    let path = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) }.ok()?;
    // SAFETY: on success the shell hands back a NUL-terminated wide string it allocated, read
    // here before it is freed below.
    let read = unsafe { path.to_string() };
    // SAFETY: the string is the shell's, allocated with the COM allocator, and freed once,
    // after its last read.
    unsafe { CoTaskMemFree(Some(path.as_ptr().cast())) };
    read.ok().map(PathBuf::from)
}
