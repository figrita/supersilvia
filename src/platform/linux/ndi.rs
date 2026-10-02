// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NDI® runtime opened by its path, ahead of the plugin, and never closed.
//!
//! `gst-plugin-ndi` opens `libndi.so.6` by bare name through `ld.so`, which searches only the
//! folders it is configured for — and Fedora's are not `/usr/local/lib`, where NDI's installer
//! puts it. A library already in the process is found by a bare-name open that matches its
//! SONAME, which `libndi.so.6`'s is, so opening the file by its full path first and holding it
//! makes the plugin's own open find it. This is the one file in `platform/linux/` allowed
//! `unsafe`: the open runs the library's initialisers.

use crate::platform::ndi::Preload;
use libloading::Library;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The runtime once opened, and where from. Never dropped, so never closed.
static HELD: OnceLock<(PathBuf, Library)> = OnceLock::new();

/// Open the first of `order` that opens — a bare name through the loader's own search, a path
/// as it is — and hold it open for the life of the process. Once held, every later ask
/// answers what was held and opens nothing.
pub fn preload(order: &[PathBuf]) -> Preload {
    if let Some((path, _)) = HELD.get() {
        return Preload::Held(path.clone());
    }
    let mut refused = Vec::new();
    for path in order {
        match open(path) {
            Ok(library) => {
                let held = HELD.get_or_init(|| (path.clone(), library));
                return Preload::Held(held.0.clone());
            }
            Err(why) => refused.push((path.clone(), why)),
        }
    }
    Preload::Refused(refused)
}

/// Open one library, or say why the loader would not: `dlerror`'s own words.
fn open(path: &Path) -> Result<Library, String> {
    // SAFETY: opening `libndi` runs its initialisers, which are the ones the NDI plugin runs
    // when it opens the same runtime in this process; nothing is called through the handle.
    let opened = unsafe { Library::new(path) };
    opened.map_err(|e| {
        std::error::Error::source(&e).map_or_else(|| e.to_string(), ToString::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library that is not there is refused with the loader's reason, naming it, and nothing
    /// is held.
    #[test]
    fn a_library_that_is_not_there_is_refused_with_the_loaders_words() {
        let path = PathBuf::from("/nonexistent/supersilvia/libndi.so.6");
        let Err(why) = open(&path) else {
            panic!("opened a file that is not there");
        };
        assert!(
            why.contains("/nonexistent/supersilvia/libndi.so.6"),
            "{why}"
        );
    }
}
