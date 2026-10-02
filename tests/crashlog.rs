// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the log file, as `main` starts it — a process of its own, since the logger is the
//! process's one. Records reach the file under its own filter with a timestamp, a reason and
//! the closing line are written as the next launch reads them, and that launch says how this
//! one ended.

use std::path::PathBuf;
use supersilvia::app::crashlog::{self, CLOSED, LOG, Log, PREVIOUS, Unexpected};

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-crashlog-run-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

#[test]
fn a_run_logs_to_its_file_and_the_next_launch_reads_how_it_ended() {
    let dir = dir();
    crashlog::start_in(Some(&dir));
    assert_eq!(crashlog::path(), Some(dir.join(LOG).as_path()));
    assert_eq!(crashlog::last_run(), None, "nothing ran here before");

    log::info!(target: "supersilvia::test", "an info line of ours");
    log::warn!(target: "wgpu_core::test", "a warning from a dependency");
    log::info!(target: "wgpu_core::test", "an info line from a dependency");
    crashlog::panicked(
        "thread 'synth' panicked at src/a.rs:1:2:\nboom",
        "   0: supersilvia::a\n   1: std::rt",
    );

    let text = std::fs::read_to_string(dir.join(LOG)).unwrap();
    assert!(
        text.starts_with(&format!("supersilvia {} (pid ", env!("CARGO_PKG_VERSION"))),
        "{text}"
    );
    if std::env::var_os("RUST_LOG").is_none() {
        assert!(text.contains("an info line of ours"), "{text}");
        assert!(text.contains("a warning from a dependency"), "{text}");
        assert!(
            !text.contains("an info line from a dependency"),
            "a dependency's info stays out: {text}"
        );
        let line = text
            .lines()
            .find(|l| l.contains("an info line of ours"))
            .unwrap();
        assert!(
            line.starts_with('[') && line.contains("Z INFO  supersilvia::test]"),
            "a record carries its time and level: {line}"
        );
    }
    assert!(
        text.contains("!! thread 'synth' panicked at src/a.rs:1:2: boom\n   0: supersilvia::a"),
        "a panic's reason is one line, and its backtrace follows: {text}"
    );
    assert!(!text.contains(CLOSED));

    // The next launch, while this process still holds the log, writes one of its own; once
    // this run has let go of it, the run is judged by what it wrote.
    let beside = Log::open(&dir).unwrap();
    assert_ne!(beside.path(), dir.join(LOG));
    drop(beside);

    crashlog::closed();
    let text = std::fs::read_to_string(dir.join(LOG)).unwrap();
    assert!(text.ends_with("== closed\n"), "{text}");

    // Copied as the next launch would find it: this process still holds the original.
    let other = dir.join("next");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join(LOG),
        text.trim_end().trim_end_matches(CLOSED).as_bytes(),
    )
    .unwrap();
    let next = Log::open(&other).unwrap();
    assert_eq!(
        next.last_run(),
        Some(&Unexpected {
            why: Some("thread 'synth' panicked at src/a.rs:1:2: boom".to_owned()),
            log: other.join(PREVIOUS),
        }),
        "without its closing line, the run went down for the reason it wrote"
    );
    drop(next);
    std::fs::remove_dir_all(&dir).ok();
}
