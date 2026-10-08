// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bundles the Example Library on every machine, links the vendored `Syphon.framework` on
//! macOS, and gives the executable its icon and version on Windows.
//!
//! **The Example Library** is `assets/examples/<NN-Category>/<NN-Title>.ssw`, workspace files
//! written by `scripts/examples/make.py`. Each one is compiled in with `include_str!` through a
//! list written into `OUT_DIR` that `src/examples.rs` includes, ordered by the two `NN`
//! prefixes, with the category and the title taken from the names less their prefix and the
//! extension. Nothing here reads a file's contents: a title is its file's name. The folder and
//! every file in it are rerun triggers, so adding an example rebuilds.
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
//!
//! **The linker is told not to look for ATL's debug information.** The shader compiler linked
//! in on Windows (`Cargo.toml`) takes the Active Template Library's static library with it,
//! whose objects name a PDB Microsoft does not ship, and `lld-link` warns once per object.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    examples(&manifest);
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => syphon(&manifest),
        Ok("windows") => {
            icon(&manifest);
            println!("cargo:rustc-link-arg=/IGNORE:4099");
        }
        _ => {}
    }
}

/// The Example Library's list, as Rust, into `OUT_DIR/examples.rs`.
fn examples(manifest: &Path) {
    let root = manifest.join("assets").join("examples");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut list = String::from("pub static EXAMPLES: &[Example] = &[\n");
    for (category, folder) in numbered(&root, None) {
        println!("cargo:rerun-if-changed={}", folder.display());
        for (title, file) in numbered(&folder, Some("ssw")) {
            println!("cargo:rerun-if-changed={}", file.display());
            let path = file.display().to_string();
            let _ = writeln!(
                list,
                "    Example {{ category: {category:?}, title: {title:?}, file: include_str!({path:?}) }},"
            );
        }
    }
    list.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    std::fs::write(out.join("examples.rs"), list).expect("OUT_DIR is writable");
}

/// The entries of `dir` named `NN-Name`, folders where `extension` is `None` and files with
/// that extension otherwise, in the order of `NN`, each as its name less the prefix and the
/// extension. Anything else is skipped with a warning; a folder that is not there holds
/// nothing.
fn numbered(dir: &Path, extension: Option<&str>) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let stem = match extension {
            None if path.is_dir() => name.as_str(),
            Some(ext) if path.is_file() => {
                match name.strip_suffix(ext).and_then(|s| s.strip_suffix('.')) {
                    Some(stem) => stem,
                    None => continue,
                }
            }
            _ => continue,
        };
        let parsed = stem
            .split_once('-')
            .and_then(|(n, rest)| Some((n.parse::<u32>().ok()?, rest.to_owned())))
            .filter(|(_, rest)| !rest.is_empty());
        match parsed {
            Some((n, rest)) => found.push((n, rest, path)),
            None => println!(
                "cargo:warning=assets/examples: {} has no NN- prefix and is not bundled",
                path.display()
            ),
        }
    }
    found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    found
        .into_iter()
        .map(|(_, name, path)| (name, path))
        .collect()
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
    let _ = writeln!(
        rc,
        "BEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n  BLOCK \"040904B0\"\n  BEGIN"
    );
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
