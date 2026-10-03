// SPDX-License-Identifier: AGPL-3.0-or-later

//! The macOS backend, for Apple Silicon: every service of [`crate::platform`], answering the
//! way Linux answers on a machine without the device.
//!
//! Each module names the macOS API its service is written against in a `TODO`, and
//! `proposals/platform.md` holds the plan. `dirs` and `files::reveal` need only the standard
//! library and are written, and so are `files::pick`, through `rfd`'s `NSOpenPanel`, `video`'s
//! cameras and hardware codecs, `audio`'s named inputs and loopback, `screen`, through
//! ScreenCaptureKit, `midi`, CoreMIDI through `midir`, `fonts`, AppKit's font collection,
//! `syphon`, the vendored `Syphon.framework`, and `gpu`, the IORegistry's whole-GPU figure;
//! every service is written. `ndi` opens nothing: dyld already finds the NDI® runtime where its
//! installer puts it.
//!
//! **`unsafe` is allowed file by file**, where each is declared below, beside the one service
//! that needs it: `audio`, which calls Core Audio's process-tap API through `objc2` bindings
//! that mark almost every call `unsafe`; `screen`, whose ScreenCaptureKit calls and
//! Objective-C classes are `unsafe` by their bindings; `pixels`, which reads a decoded
//! buffer's `CVPixelBuffer` through applemedia's meta, and locks a pixel buffer and reads its
//! planes; `gpu`, whose IOKit registry calls are `unsafe` by their bindings and whose
//! property dictionary comes back untyped; `menu`, whose menu items' actions and targets are
//! `unsafe` by their bindings and whose one Objective-C class is every entry's target; and
//! `syphon`, whose framework classes are declared by hand and whose surfaces are taken over
//! from `new` methods and locked to be read. Every block carries a `// SAFETY:` line. The crate root denies it
//! everywhere else.
//! `midi` and `fonts` take no `unsafe`: `midir` and objc2's typed bindings hold it themselves.

pub mod alert;
#[allow(unsafe_code)]
pub mod audio;
pub mod check;
pub mod dirs;
pub mod filedrop;
pub mod files;
pub mod fonts;
#[allow(unsafe_code)]
pub mod gpu;
#[allow(unsafe_code)]
pub mod menu;
pub mod midi;
pub mod ndi;
pub mod notices;
#[allow(unsafe_code)]
mod pixels;
#[allow(unsafe_code)]
pub mod screen;
#[allow(unsafe_code)]
pub mod syphon;
pub mod video;
