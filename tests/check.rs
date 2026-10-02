// SPDX-License-Identifier: AGPL-3.0-or-later

//! `supersilvia --check`, `--version` and `--help`, run as a person runs them: the binary, as a
//! process of its own, on this machine — which `doctor.sh` has already found everything on.

use std::process::Command;
use supersilvia::render::adapter;

/// The binary with `arg`, asked for the integrated GPU unless `SUPERSILVIA_ADAPTER` already
/// names one: `--check` opens a device on the adapter the app would render on, whose default
/// is the strongest GPU, and no test here opens one on a discrete GPU.
fn run(arg: &str) -> (bool, String) {
    let integrated = adapter::Asked::integrated().adapter;
    let out = Command::new(env!("CARGO_BIN_EXE_supersilvia"))
        .arg(arg)
        .env(
            adapter::ADAPTER_ENV,
            integrated.expect("an adapter is named"),
        )
        .env_remove("RUST_LOG")
        .output()
        .expect("the binary runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// The development machine has everything the app needs, so the check passes and names the
/// GStreamer and the GPU it found, and ends saying it can run.
#[test]
fn the_check_passes_here_and_says_what_it_found() {
    let (ok, out) = run("--check");
    assert!(ok, "{out}");
    assert!(out.contains("  PASS  GStreamer "), "{out}");
    assert!(out.contains("  PASS  pipelines "), "{out}");
    assert!(out.contains("  PASS  GPU "), "{out}");
    assert!(!out.contains("FAIL"), "{out}");
    assert!(out.contains("preferences"), "{out}");
    assert!(
        out.lines()
            .last()
            .is_some_and(|l| l.starts_with("Everything") || l.starts_with("supersilvia can run")),
        "{out}"
    );
}

#[test]
fn version_and_help_answer_and_exit() {
    let (ok, out) = run("--version");
    assert!(ok);
    assert_eq!(out, format!("supersilvia {}\n", env!("CARGO_PKG_VERSION")));
    let (ok, out) = run("--help");
    assert!(ok);
    assert!(out.contains("--check"), "{out}");
}
