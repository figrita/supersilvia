// SPDX-License-Identifier: AGPL-3.0-or-later

//! The third-party notices Help ▸ Licences shows are the ones this build is made of.
//!
//! Linux's and Windows' crate notices are committed files compiled into the binary, so a
//! Cargo.lock that moves would leave them describing another build.
//! `scripts/crate-licenses.py --check` writes each again from `cargo metadata --offline` and
//! compares, and fails here with the line that rewrites the file. Both are checked on Linux,
//! which is where the lock moves. The `licenses/` folder is compiled in file by file, so a file
//! added there and not to `ui::about::ASSETS` fails too.

use std::path::Path;
use supersilvia::ui::about::ASSETS;

#[cfg(target_os = "linux")]
#[test]
fn the_linux_crate_notices_are_what_cargo_lock_compiles() {
    check_crate_notices(
        "x86_64-unknown-linux-gnu",
        "packaging/linux/rust-crates.txt",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn the_windows_crate_notices_are_what_cargo_lock_compiles() {
    check_crate_notices(
        "x86_64-pc-windows-msvc",
        "packaging/windows/rust-crates.txt",
    );
}

/// `scripts/crate-licenses.py --check` for one target's committed file.
#[cfg(target_os = "linux")]
fn check_crate_notices(target: &str, file: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("python3")
        .arg(root.join("scripts/crate-licenses.py"))
        .args(["--target", target, "--check"])
        .arg(root.join(file))
        .output()
        .expect("python3 runs scripts/crate-licenses.py");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn every_file_in_licenses_is_compiled_in_as_it_is() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("licenses");
    let mut files: Vec<String> = std::fs::read_dir(&folder)
        .expect("licenses/ is there")
        .map(|entry| {
            entry
                .expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    files.sort();
    let compiled: Vec<&str> = ASSETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(files, compiled, "licenses/ and ui::about::ASSETS differ");
    for (name, text) in ASSETS {
        let on_disk = std::fs::read_to_string(folder.join(name)).expect("readable");
        assert_eq!(&on_disk, text, "{name}");
    }
}
