// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NDI® runtime on macOS is left to the plugin: dyld's fallback searches `/usr/local/lib`,
//! where the NDI 6 Runtime's installer puts `libndi.dylib`, so the plugin's own open by bare
//! name finds it and nothing is opened ahead of it.

use crate::platform::ndi::Preload;
use std::path::PathBuf;

/// Opens nothing: the plugin finds the runtime itself.
pub fn preload(_order: &[PathBuf]) -> Preload {
    Preload::Left
}
