// SPDX-License-Identifier: AGPL-3.0-or-later

//! `supersilvia --check`: what this machine gives the app, asked without opening a window, and
//! what to install where something is missing. A person runs it first on a machine the app has
//! never run on, and pastes it into a bug report.
//!
//! The binary ships without GStreamer, the GPU driver or the windowing libraries, which are
//! the machine's (`packaging/linux/TESTERS.md`), so what the app can do is decided by what the
//! machine has: every GStreamer element a pipeline makes, by the plugin set a distribution
//! packages it in, a hardware codec pair for video import, the GPU `render::adapter` would
//! pick, the libraries opened at run time, the session, and the NDI® runtime. What differs by
//! machine is `platform::check`'s; the report and its verdicts are here. `--version` and
//! `--help` answer beside it.

use crate::platform;
use gstreamer as gst;
use std::fmt::Write as _;

/// The GStreamer the app is built against: `gstreamer`'s `v1_24` feature.
const GSTREAMER_FLOOR: (u32, u32) = (1, 24);

/// How much a line's finding costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Present.
    Pass,
    /// Missing, and the app runs without it: one feature is off.
    Warn,
    /// Missing, and the app cannot run without it.
    Fail,
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub verdict: Verdict,
    pub label: String,
    pub detail: String,
}

impl Line {
    pub fn new(verdict: Verdict, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            verdict,
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// A GStreamer element the app makes, and the plugin set it ships in — a distribution's
/// package name, less its prefix.
#[derive(Debug, Clone, Copy)]
pub struct Element {
    pub name: &'static str,
    pub set: &'static str,
}

/// Elements that serve one thing the app does.
#[derive(Debug, Clone, Copy)]
pub struct Group {
    /// What they are for, as the report's label.
    pub what: &'static str,
    /// Whether the app runs without them.
    pub required: bool,
    pub elements: &'static [Element],
}

/// Shorthand for the element tables in `platform/`.
pub const fn el(name: &'static str, set: &'static str) -> Element {
    Element { name, set }
}

/// Whether `arg` is one of the flags [`command_line`] answers, rather than a path.
pub fn is_flag(arg: &str) -> bool {
    matches!(arg, "--version" | "-V" | "--check" | "--help" | "-h")
}

/// Answer a flag on the command line: `Some(exit code)` where `arg` is one of ours and has
/// been answered, `None` where it is a path for the app to open.
pub fn command_line(arg: &str) -> Option<i32> {
    match arg {
        "--version" | "-V" => {
            println!("supersilvia {}", env!("CARGO_PKG_VERSION"));
            Some(0)
        }
        "--check" => {
            let lines = run();
            print!("{}", report(&lines));
            Some(exit_code(&lines))
        }
        "--help" | "-h" => {
            print!("{}", help());
            Some(0)
        }
        _ => None,
    }
}

fn help() -> String {
    format!(
        "supersilvia {} — {}\n\n\
         Usage:\n  \
           supersilvia                 open the most recent project\n  \
           supersilvia <folder>        open a project folder, or the project a path is inside\n  \
           supersilvia <file.ssw>      open a workspace file, as a project of its own\n  \
           supersilvia --check         say what this machine is missing, and exit\n  \
           supersilvia --version       print the version, and exit\n\n\
         It keeps a log, which --check says where to find.\n\
         RUST_LOG=info writes what it does to the terminal as well.\n",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_DESCRIPTION"),
    )
}

/// Every check, in the report's order.
pub fn run() -> Vec<Line> {
    with_gpu(gpu())
}

/// Every check, with the GPU's line handed in rather than found by opening a device of its
/// own: what the running app reports, which renders on one already (Help ▸ Report a
/// problem…).
pub fn with_gpu(gpu: Line) -> Vec<Line> {
    let mut lines = Vec::new();
    let gstreamer = gstreamer();
    let started = gstreamer.verdict != Verdict::Fail;
    lines.push(gstreamer);
    if started {
        lines.extend(elements());
        lines.push(codec());
    }
    lines.push(gpu);
    lines.extend(platform::check::machine());
    lines.push(ndi());
    lines
}

/// Where a person's things are, for the report's foot.
fn folders() -> Vec<(&'static str, String)> {
    let shown = |p: Option<std::path::PathBuf>| {
        p.map_or_else(
            || "nowhere: set HOME".to_string(),
            |p| p.display().to_string(),
        )
    };
    let preferences = crate::preferences::path();
    // The folder the preferences choose, where they choose one.
    let projects = preferences
        .as_deref()
        .filter(|p| p.exists())
        .map_or_else(
            crate::preferences::Preferences::default,
            crate::preferences::Preferences::load,
        )
        .projects_dir();
    vec![
        ("preferences", shown(preferences)),
        ("projects", shown(projects)),
        (
            "log",
            shown(crate::app::crashlog::folder().map(|d| d.join(crate::app::crashlog::LOG))),
        ),
    ]
}

/// The report as the terminal shows it.
pub fn report(lines: &[Line]) -> String {
    let mut out = format!("supersilvia {} --check\n\n", env!("CARGO_PKG_VERSION"));
    let width = lines
        .iter()
        .map(|l| l.label.chars().count())
        .max()
        .unwrap_or(0);
    for line in lines {
        let word = match line.verdict {
            Verdict::Pass => "PASS",
            Verdict::Warn => "WARN",
            Verdict::Fail => "FAIL",
        };
        let mut detail = line.detail.lines();
        let first = detail.next().unwrap_or_default();
        let _ = writeln!(out, "  {word}  {:<width$}  {first}", line.label);
        // A detail of several lines, as a GPU refusal naming every adapter is, keeps to its column.
        for more in detail {
            let _ = writeln!(
                out,
                "{:indent$}{}",
                "",
                more.trim_start(),
                indent = width + 10
            );
        }
    }
    out.push('\n');
    for (what, path) in folders() {
        let _ = writeln!(out, "  {what:<width$}  {path}", width = width + 6);
    }
    out.push('\n');
    let fails = lines.iter().filter(|l| l.verdict == Verdict::Fail).count();
    let warns = lines.iter().filter(|l| l.verdict == Verdict::Warn).count();
    let _ = match (fails, warns) {
        (0, 0) => writeln!(out, "Everything supersilvia uses is here."),
        (0, n) => writeln!(
            out,
            "supersilvia can run here. {n} {} a feature off; each WARN says what to install.",
            if n == 1 { "WARN turns" } else { "WARNs turn" }
        ),
        (n, _) => writeln!(
            out,
            "supersilvia cannot run here until {} fixed; each FAIL says what is missing.",
            if n == 1 {
                "one FAIL is"
            } else {
                "the FAILs are"
            }
        ),
    };
    out
}

/// 1 where anything the app needs is missing, 0 otherwise.
pub fn exit_code(lines: &[Line]) -> i32 {
    i32::from(lines.iter().any(|l| l.verdict == Verdict::Fail))
}

/// GStreamer loads, and is new enough.
fn gstreamer() -> Line {
    if let Err(e) = gst::init() {
        return Line::new(Verdict::Fail, "GStreamer", format!("does not start: {e}"));
    }
    let (major, minor, micro, _) = gst::version();
    let (floor_major, floor_minor) = GSTREAMER_FLOOR;
    let verdict = if (major, minor) >= GSTREAMER_FLOOR {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    let mut detail = format!("{major}.{minor}.{micro}");
    if verdict == Verdict::Fail {
        let _ = write!(detail, " — {floor_major}.{floor_minor} or newer is needed");
    }
    Line::new(verdict, "GStreamer", detail)
}

/// Whether GStreamer's registry has an element by this name.
pub(crate) fn has(element: &str) -> bool {
    gst::ElementFactory::find(element).is_some()
}

/// One line per group: every element there, or the ones missing and the plugin sets they
/// are in.
fn elements() -> Vec<Line> {
    // Ours before anything loads a system's NDI plugin in its place: see `video::ndi`.
    let _ = crate::video::ndi::register();
    platform::check::GROUPS
        .iter()
        .map(|group| {
            let missing: Vec<&Element> = group.elements.iter().filter(|e| !has(e.name)).collect();
            if missing.is_empty() {
                return Line::new(Verdict::Pass, group.what, elements_named(group.elements));
            }
            let verdict = if group.required {
                Verdict::Fail
            } else {
                Verdict::Warn
            };
            Line::new(verdict, group.what, format!("missing {}", by_set(&missing)))
        })
        .collect()
}

/// A group's elements, for a line that passes.
fn elements_named(elements: &[Element]) -> String {
    match elements {
        [one] => one.name.to_string(),
        many => format!("all {}", many.len()),
    }
}

/// Missing elements grouped under the plugin set each ships in: `a, b (gst-plugins-good);
/// c (gst-libav)`.
fn by_set(missing: &[&Element]) -> String {
    let mut sets: Vec<&str> = Vec::new();
    for e in missing {
        if !sets.contains(&e.set) {
            sets.push(e.set);
        }
    }
    sets.iter()
        .map(|set| {
            let names: Vec<&str> = missing
                .iter()
                .filter(|e| e.set == *set)
                .map(|e| e.name)
                .collect();
            format!("{} ({set})", names.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A hardware encoder and decoder pair, which importing a video file transcodes through.
fn codec() -> Line {
    let label = "hardware codec";
    for codec in platform::video::CODECS {
        if has(codec.encoder) && has(codec.decoder) && has(codec.parser) {
            return Line::new(
                Verdict::Pass,
                label,
                format!("{}: {} and {}", codec.name, codec.encoder, codec.decoder),
            );
        }
    }
    Line::new(
        Verdict::Warn,
        label,
        format!(
            "no hardware encoder and decoder pair — video files cannot be imported; \
             needs {}",
            platform::video::CODEC_HINT
        ),
    )
}

/// The GPU the app would render on, by the app's own rule and `SUPERSILVIA_ADAPTER`, with a
/// device opened on it to show it opens.
fn gpu() -> Line {
    match crate::render::Gpu::headless(&crate::render::adapter::Asked::from_env()) {
        Ok(gpu) => Line::new(
            Verdict::Pass,
            "GPU",
            crate::render::adapter::describe(&gpu.adapter().get_info()),
        ),
        Err(why) => Line::new(Verdict::Fail, "GPU", why),
    }
}

/// The NDI® runtime, which only NDI sending and receiving need.
fn ndi() -> Line {
    if crate::video::ndi::runtime() {
        Line::new(Verdict::Pass, "NDI® runtime", "loads")
    } else {
        let why = crate::video::ndi::missing().unwrap_or(crate::video::ndi::MISSING);
        Line::new(
            Verdict::Warn,
            "NDI® runtime",
            format!("{why} (only NDI® sending and receiving need it)"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_elements_are_named_under_their_plugin_sets() {
        let a = el("a", "gst-plugins-good");
        let b = el("b", "gst-libav");
        let c = el("c", "gst-plugins-good");
        assert_eq!(
            by_set(&[&a, &b, &c]),
            "a, c (gst-plugins-good); b (gst-libav)"
        );
    }

    /// A WARN leaves the exit code at 0 and a FAIL makes it 1, and the report's last line
    /// says which.
    #[test]
    fn only_a_fail_fails() {
        let pass = Line::new(Verdict::Pass, "GStreamer", "1.28.7");
        let warn = Line::new(
            Verdict::Warn,
            "cameras",
            "missing v4l2src (gst-plugins-good)",
        );
        let fail = Line::new(Verdict::Fail, "GPU", "no adapter");
        assert_eq!(exit_code(&[pass.clone(), warn.clone()]), 0);
        assert_eq!(exit_code(&[pass.clone(), warn.clone(), fail.clone()]), 1);
        assert!(
            report(std::slice::from_ref(&pass)).ends_with("Everything supersilvia uses is here.\n")
        );
        let warned = report(&[pass.clone(), warn.clone()]);
        assert!(
            warned.contains("  WARN  cameras    missing v4l2src"),
            "{warned}"
        );
        assert!(
            warned.contains("supersilvia can run here. 1 WARN turns"),
            "{warned}"
        );
        let failed = report(&[pass, warn, fail]);
        assert!(
            failed.contains("cannot run here until one FAIL is fixed"),
            "{failed}"
        );
        let long = Line::new(Verdict::Fail, "GPU", "none. Offered:\n  [0] one\n  [1] two");
        let shown = report(&[long]);
        assert!(
            shown.contains("  FAIL  GPU  none. Offered:\n             [0] one\n"),
            "{shown}"
        );
    }

    /// Every element is named once across the groups, so a missing one is reported once.
    #[test]
    fn no_element_is_in_two_groups() {
        let mut seen = std::collections::HashSet::new();
        for group in platform::check::GROUPS {
            for e in group.elements {
                assert!(seen.insert(e.name), "{} is in two groups", e.name);
            }
        }
    }
}
