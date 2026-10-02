// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Mac's third-party notices are the bundle's: AppKit's About panel shows `Credits.rtf`,
//! and `packaging/macos/build-app.sh` writes every licence in full into
//! `Contents/Resources/licenses`. The Mac's Help menu has no Licences entry, so these are read
//! only where the egui bar stands in for AppKit's — a test harness's thread — and say where
//! the files are.

/// Where the crates' licences are in the `.app`.
pub const RUST_CRATES: &str = "\
Rust crates compiled into supersilvia
=====================================

In supersilvia.app, every Rust crate compiled in and its licence text are in
Contents/Resources/licenses/rust-crates.txt, which scripts/crate-licenses.py writes for
aarch64-apple-darwin when the bundle is built.
";

/// GStreamer, which the `.app` carries.
pub const GSTREAMER: &str = "\
GStreamer
=========

supersilvia.app carries GStreamer and the libraries it uses, from GStreamer's official macOS
release, under the GNU Lesser General Public License 2.1 or later and more permissive
licences. You may replace them with your own builds. Their licences are in
Contents/Resources/licenses, and their source is at https://gstreamer.freedesktop.org/src/
";
