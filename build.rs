// SPDX-License-Identifier: AGPL-3.0-or-later

//! Links the vendored `Syphon.framework` on macOS, and does nothing anywhere else.
//!
//! The framework is `vendor/syphon/`, built from Syphon's source for arm64 (`vendor/README.md`).
//! Two rpaths find it at run time: `@executable_path/../Frameworks`, where the `.app` carries
//! it, and the vendored folder itself, so `cargo run`, `cargo test` and the examples load it
//! from the checkout. A third, `@executable_path/../Frameworks/GStreamer/lib`, is where the
//! `.app` carries GStreamer's libraries, whose install names are `@rpath/<name>` in the
//! official framework the bundle is built against (`packaging/macos/README.md`); a build
//! against Homebrew's links them by absolute path and never reads it.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/syphon/Syphon.framework");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let vendored = manifest.join("vendor").join("syphon");
    println!("cargo:rustc-link-search=framework={}", vendored.display());
    println!("cargo:rustc-link-lib=framework=Syphon");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks/GStreamer/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", vendored.display());
}
