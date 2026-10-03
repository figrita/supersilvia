// SPDX-License-Identifier: AGPL-3.0-or-later

//! Links the vendored `Syphon.framework` on macOS, gives the executable its icon and version on
//! Windows, and does nothing anywhere else.
//!
//! The framework is `vendor/syphon/`, built from Syphon's source for arm64 (`vendor/README.md`).
//! Two rpaths find it at run time: `@executable_path/../Frameworks`, where the `.app` carries
//! it, and the vendored folder itself, so `cargo run`, `cargo test` and the examples load it
//! from the checkout. A third, `@executable_path/../Frameworks/GStreamer/lib`, is where the
//! `.app` carries GStreamer's libraries, whose install names are `@rpath/<name>` in the
//! official framework the bundle is built against (`packaging/macos/README.md`); a build
//! against Homebrew's links them by absolute path and never reads it.
//!
//! On Windows the icon is `assets/icon/supersilvia.ico`, which Explorer shows for the `.exe`
//! file. It goes into a resource script written here, which the resource compiler makes a
//! `.res` and the linker takes as one more input. The compiler is `llvm-rc` when it is on the
//! path, which is what a cross build from Linux has, and `rc`, the Windows SDK's, otherwise.
//! Neither is a crate, so nothing is added to the build of any other machine. With neither, the
//! executable builds without its icon and a warning says so: the window's own icon is
//! `main.rs`'s and does not depend on it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => syphon(&manifest),
        Ok("windows") => icon(&manifest),
        _ => {}
    }
}

fn syphon(manifest: &Path) {
    println!("cargo:rerun-if-changed=vendor/syphon/Syphon.framework");
    let vendored = manifest.join("vendor").join("syphon");
    println!("cargo:rustc-link-search=framework={}", vendored.display());
    println!("cargo:rustc-link-lib=framework=Syphon");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks/GStreamer/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", vendored.display());
}

fn icon(manifest: &Path) {
    println!("cargo:rerun-if-changed=assets/icon/supersilvia.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    let script = out.join("supersilvia.rc");
    let res = out.join("supersilvia.res");
    std::fs::write(&script, resource_script(manifest)).expect("OUT_DIR is writable");

    let compiled = ["llvm-rc", "rc"].iter().find_map(|rc| {
        let status = Command::new(rc)
            .arg("/nologo")
            .arg("/fo")
            .arg(&res)
            .arg(&script)
            .status();
        match status {
            Ok(status) if status.success() => Some(Ok(())),
            Ok(status) => Some(Err(format!("{rc} failed: {status}"))),
            Err(_) => None,
        }
    });
    match compiled {
        Some(Ok(())) => println!("cargo:rustc-link-arg-bins={}", res.display()),
        Some(Err(why)) => panic!("{why}"),
        None => println!(
            "cargo:warning=neither llvm-rc nor rc is on the path: supersilvia.exe has no icon"
        ),
    }
}

/// The icon, and the version block Explorer's Details tab reads, with the numeric version the
/// block needs made from the leading numbers of Cargo's: `0.9.0-alpha.1` is `0,9,0,0`.
fn resource_script(manifest: &Path) -> String {
    let ico = manifest.join("assets").join("icon").join("supersilvia.ico");
    let ico = ico.display().to_string().replace('\\', "\\\\");
    let version = std::env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let numeric = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .chain(std::iter::repeat("0"))
        .take(4)
        .collect::<Vec<_>>()
        .join(",");
    let mut rc = String::new();
    let _ = writeln!(rc, "1 ICON \"{ico}\"");
    let _ = writeln!(rc, "1 VERSIONINFO");
    let _ = writeln!(rc, "FILEVERSION {numeric}");
    let _ = writeln!(rc, "PRODUCTVERSION {numeric}");
    // VOS_NT_WINDOWS32 and VFT_APP, by number: no header is read.
    let _ = writeln!(rc, "FILEOS 0x40004\nFILETYPE 0x1");
    let _ = writeln!(rc, "BEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n  BLOCK \"040904B0\"\n  BEGIN");
    for (key, value) in [
        ("CompanyName", "supersilvia"),
        ("FileDescription", "supersilvia"),
        ("ProductName", "supersilvia"),
        ("OriginalFilename", "supersilvia.exe"),
        ("FileVersion", version.as_str()),
        ("ProductVersion", version.as_str()),
        ("LegalCopyright", "AGPL-3.0-or-later"),
    ] {
        let _ = writeln!(rc, "   VALUE \"{key}\", \"{value}\"");
    }
    let _ = writeln!(rc, "  END\n END\n BLOCK \"VarFileInfo\"\n BEGIN");
    let _ = writeln!(rc, "  VALUE \"Translation\", 0x409, 1200\n END\nEND");
    rc
}
