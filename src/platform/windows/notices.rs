// SPDX-License-Identifier: AGPL-3.0-or-later

//! The third-party notices only Windows can give: which Rust crates this binary is built from,
//! and where its GStreamer comes from.
//!
//! The crates are `packaging/windows/rust-crates.txt`, which `scripts/crate-licenses.py` writes
//! for `x86_64-pc-windows-msvc` and `tests/notices.rs` holds to Cargo.lock, compiled in as
//! Linux's are, since Windows has no About panel of its own to show them in.

/// Every Rust crate compiled in, its declared licence, and the licence texts their packages
/// carry.
pub const RUST_CRATES: &str = include_str!("../../../packaging/windows/rust-crates.txt");

/// GStreamer, which the Windows folder carries.
pub const GSTREAMER: &str = "\
GStreamer
=========

supersilvia captures, decodes, encodes and plays video through GStreamer, which is free
software under the GNU Lesser General Public License, version 2.1 or later; each of its
plugins is under the LGPL or a more permissive licence.

On Windows supersilvia carries GStreamer beside it: the DLLs in bin/, lib/gstreamer-1.0/ and
libexec/gstreamer-1.0/ are GStreamer's official MSVC release, from
https://gstreamer.freedesktop.org/download/, unmodified. You may replace them with your own
builds of the same version. Their licences are in licenses/, with the list of every file
carried, and their source is at https://gstreamer.freedesktop.org/src/

Compiled into supersilvia itself are the Rust bindings to GStreamer (gstreamer-rs, MIT or
Apache-2.0) and GStreamer's NDI plugin from gst-plugins-rs (gst-plugin-ndi, MPL-2.0), both
listed with their licences under Rust crates.
";
