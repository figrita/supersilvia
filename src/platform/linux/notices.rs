// SPDX-License-Identifier: AGPL-3.0-or-later

//! The third-party notices only Linux can give: which Rust crates this binary is built from,
//! and where its GStreamer comes from.
//!
//! The crates are `packaging/linux/rust-crates.txt`, which `scripts/crate-licenses.py` writes
//! for `x86_64-unknown-linux-gnu` and `tests/notices.rs` holds to Cargo.lock, compiled in so
//! a source build, the AppImage and the Flatpak all show the same text with no file to find.

/// Every Rust crate compiled in, its declared licence, and the licence texts their packages
/// carry.
pub const RUST_CRATES: &str = include_str!("../../../packaging/linux/rust-crates.txt");

/// GStreamer, which Linux takes from the system rather than carrying.
pub const GSTREAMER: &str = "\
GStreamer
=========

supersilvia captures, decodes, encodes and plays video through GStreamer, which is free
software under the GNU Lesser General Public License, version 2.1 or later; each of its
plugins is under the LGPL or a more permissive licence.

On Linux supersilvia does not carry GStreamer. It links to the GStreamer the system
provides: a distribution's packages for a source build or an install, the host's own under
the AppImage, and the org.freedesktop.Platform runtime's in the Flatpak. That copy, under the
licence files its packager installed beside it, is the one that runs, and it can be updated,
rebuilt or replaced without touching supersilvia. Its source is at
https://gstreamer.freedesktop.org/src/

Compiled into supersilvia itself are the Rust bindings to GStreamer (gstreamer-rs, MIT or
Apache-2.0) and GStreamer's NDI plugin from gst-plugins-rs (gst-plugin-ndi, MPL-2.0), both
listed with their licences under Rust crates.
";
