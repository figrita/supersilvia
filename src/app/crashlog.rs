// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a tester can send: the log file, what the last run left in it, and Help ▸ Report a
//! problem….
//!
//! **The log is a file as well as stderr.** `supersilvia.log` in `logs/` under the app's
//! folder in the data directory ([`crate::platform::dirs::data`]): on Linux
//! `~/.local/share/supersilvia/logs/`, on macOS `~/Library/Application
//! Support/supersilvia/logs/`. stderr keeps env_logger's own default, errors alone unless
//! `RUST_LOG` says otherwise; the file takes `RUST_LOG` where it is set and
//! [`FILE_FILTER`] where it is not, with a timestamp on every line. A panic is written there
//! with its backtrace, and so is a lost GPU. The file stops growing at [`CAP`].
//!
//! **A run marks how it ended.** It writes [`CLOSED`] once its window has gone, or once it has
//! said in a box why it could not start, and [`WHY`] before a reason whenever something takes
//! it down: a panic, a lost GPU, a start that cannot go on. The next launch reads the log
//! before it writes a line of its own. A log with no closing line is a run that closed
//! unexpectedly, and the last reason in it is why; none is a run the system ended, or a driver
//! that failed underneath it. That run's log is kept as `previous.log`, and the editor puts
//! the notice up before it asks anything else ([`App::notice_last_run`]).
//!
//! **One log per running app.** The file is held under an exclusive lock for as long as the
//! process lives, and the operating system lets go of it however the process ends. A second
//! supersilvia started beside the first finds it locked, writes `supersilvia-<pid>.log`
//! instead and judges nothing; the next launch to hold the main log deletes those whose
//! process has gone.

use super::App;
use crate::check::{Line, Verdict};
use eframe::egui;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

/// The folder under the app's own in the data directory.
pub const FOLDER: &str = "logs";
/// This run's log.
pub const LOG: &str = "supersilvia.log";
/// The last run's log, kept when this run began.
pub const PREVIOUS: &str = "previous.log";
/// What the file keeps where `RUST_LOG` is not set: what the tester guides asked for on a
/// terminal before there was a file.
pub const FILE_FILTER: &str = "warn,supersilvia=info";
/// The line a run ends on when it ends as it meant to.
pub const CLOSED: &str = "== closed";
/// The start of a line saying why the app went down.
pub const WHY: &str = "!! ";
/// The most the file grows to in one run, in bytes. Past it no record is written; a reason and
/// the closing line still are.
pub const CAP: u64 = 16 << 20;
/// How many of a log's last lines a report carries.
pub const TAIL_LINES: usize = 60;
/// The Discord's bug report channel, which Help ▸ Report a problem… opens.
pub const BUG_CHANNEL: &str = "https://discord.gg/prRuJ6rvy";

/// The last run closed without saying it was closing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unexpected {
    /// The last reason it wrote, or `None` where it wrote none.
    pub why: Option<String>,
    /// Where its log is kept.
    pub log: PathBuf,
}

/// How a run's log says it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    /// There is no run before this one.
    Nothing,
    Closed,
    /// No closing line: the last reason, if one was written.
    Unexpectedly(Option<String>),
}

/// Read how the run that wrote `text` ended.
fn judge(text: &str) -> Ended {
    if text.trim().is_empty() {
        return Ended::Nothing;
    }
    if text.lines().any(|l| l.starts_with(CLOSED)) {
        return Ended::Closed;
    }
    Ended::Unexpectedly(
        text.lines()
            .rev()
            .find_map(|l| l.strip_prefix(WHY))
            .map(|why| why.trim().to_owned()),
    )
}

/// The open file and how much has been written to it.
struct Sink {
    file: File,
    written: u64,
}

/// One run's log: the file, held locked, and what was found of the run before.
pub struct Log {
    sink: Arc<Mutex<Sink>>,
    path: PathBuf,
    last_run: Option<Unexpected>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Log {
    /// Take the log in `dir`, made if it is not there: the main one, with the last run judged
    /// and kept as [`PREVIOUS`], or one of its own where another supersilvia holds that.
    ///
    /// # Errors
    /// Where the folder or the file cannot be made, read or written.
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(LOG);
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Self::take_over(dir, path, file),
            Err(std::fs::TryLockError::WouldBlock) => Self::beside(dir),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }

    /// The main log, locked: the last run's kept and judged, and the file emptied for this one.
    fn take_over(dir: &Path, path: PathBuf, mut file: File) -> io::Result<Self> {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let previous = dir.join(PREVIOUS);
        let last_run = match judge(&String::from_utf8_lossy(&bytes)) {
            Ended::Nothing => None,
            Ended::Closed => {
                std::fs::write(&previous, &bytes)?;
                None
            }
            Ended::Unexpectedly(why) => {
                std::fs::write(&previous, &bytes)?;
                Some(Unexpected { why, log: previous })
            }
        };
        file.set_len(0)?;
        sweep(dir);
        Ok(Self {
            sink: Arc::new(Mutex::new(Sink { file, written: 0 })),
            path,
            last_run,
        })
    }

    /// A log of this process's own, beside the one another supersilvia holds.
    fn beside(dir: &Path) -> io::Result<Self> {
        let path = dir.join(format!("supersilvia-{}.log", std::process::id()));
        let file = OpenOptions::new().append(true).create(true).open(&path)?;
        file.set_len(0)?;
        // Held so the next main log's sweep knows this process is still running.
        let _ = file.try_lock();
        Ok(Self {
            sink: Arc::new(Mutex::new(Sink { file, written: 0 })),
            path,
            last_run: None,
        })
    }

    /// Where this run's log is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The last run, where it closed unexpectedly.
    pub fn last_run(&self) -> Option<&Unexpected> {
        self.last_run.as_ref()
    }

    /// Write a line as it is, whatever has been written before.
    pub fn line(&self, text: &str) {
        let mut sink = lock(&self.sink);
        let _ = writeln!(sink.file, "{text}");
    }

    /// Write why the app is going down, on one line.
    pub fn why(&self, reason: &str) {
        self.line(&format!(
            "{WHY}{}",
            reason.split_whitespace().collect::<Vec<_>>().join(" ")
        ));
    }

    /// Write that the run is ending as it meant to.
    pub fn closed(&self) {
        self.line(CLOSED);
    }

    /// What env_logger writes records through: the same file, up to [`CAP`].
    fn pipe(&self) -> Pipe {
        Pipe(Arc::clone(&self.sink))
    }
}

/// Delete the logs of second instances that have gone: those whose lock can be had.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("supersilvia-") && name.ends_with(".log")) {
            continue;
        }
        let gone = File::open(entry.path()).is_ok_and(|f| f.try_lock().is_ok());
        if gone {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The file as env_logger's target. Past [`CAP`] a record is dropped.
struct Pipe(Arc<Mutex<Sink>>);

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut sink = lock(&self.0);
        if sink.written >= CAP {
            return Ok(buf.len());
        }
        sink.written += buf.len() as u64;
        sink.file.write_all(buf)?;
        if sink.written >= CAP {
            let _ = writeln!(sink.file, "-- the log stops here, at {} MB", CAP >> 20);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        lock(&self.0).file.flush()
    }
}

/// stderr and the file, each through an env_logger of its own with its own filter.
struct Tee {
    stderr: env_logger::Logger,
    file: Option<env_logger::Logger>,
}

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.stderr.enabled(metadata) || self.file.as_ref().is_some_and(|f| f.enabled(metadata))
    }

    fn log(&self, record: &log::Record<'_>) {
        self.stderr.log(record);
        if let Some(file) = &self.file {
            file.log(record);
        }
    }

    fn flush(&self) {
        self.stderr.flush();
        if let Some(file) = &self.file {
            file.flush();
        }
    }
}

/// This process's log, once [`start`] has made it.
static RUN: OnceLock<Log> = OnceLock::new();
/// The editor's window is up: an exit from here on is the window's to say.
static WINDOW: AtomicBool = AtomicBool::new(false);

/// Start logging: to stderr, as env_logger always did, and to the file. `main` calls it once,
/// before anything else logs and after a command-line flag has had its answer, so a `--check`
/// leaves the last run's log as it was. Where the file cannot be made, stderr alone, with a
/// line on it saying why.
pub fn start() {
    start_in(folder().as_deref());
}

/// [`start`], with the log kept in `dir`: what a test's process starts with, in a folder of
/// its own. `None` is a machine that names no data directory.
pub fn start_in(dir: Option<&Path>) {
    let stderr = env_logger::Builder::from_default_env().build();
    let (file, trouble) = match dir.map(Log::open) {
        Some(Ok(log)) => {
            log.line(&format!(
                "supersilvia {} (pid {})",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            ));
            let filter = std::env::var("RUST_LOG")
                .ok()
                .filter(|f| !f.trim().is_empty())
                .unwrap_or_else(|| FILE_FILTER.to_owned());
            let logger = env_logger::Builder::new()
                .parse_filters(&filter)
                .format_timestamp_millis()
                .write_style(env_logger::WriteStyle::Never)
                .target(env_logger::Target::Pipe(Box::new(log.pipe())))
                .build();
            let _ = RUN.set(log);
            (Some(logger), None)
        }
        Some(Err(e)) => (
            None,
            Some(format!(
                "no log file in {}: {e}",
                dir.unwrap_or(Path::new("")).display()
            )),
        ),
        None => (None, Some("no log file: set HOME".to_owned())),
    };
    let max = stderr.filter().max(
        file.as_ref()
            .map_or(log::LevelFilter::Off, env_logger::Logger::filter),
    );
    if log::set_boxed_logger(Box::new(Tee { stderr, file })).is_ok() {
        log::set_max_level(max);
    }
    if let Some(trouble) = trouble {
        log::warn!("{trouble}");
    }
}

/// The folder the log is kept in: `logs/` in the app's own folder in the data directory.
/// `None` where the machine names no data directory.
pub fn folder() -> Option<PathBuf> {
    crate::platform::dirs::data().map(|d| d.join("supersilvia").join(FOLDER))
}

/// Where this run's log is, once [`start`] has made one.
pub fn path() -> Option<&'static Path> {
    RUN.get().map(Log::path)
}

/// The last run, where it closed unexpectedly. `None` before [`start`].
pub fn last_run() -> Option<Unexpected> {
    RUN.get().and_then(|log| log.last_run().cloned())
}

/// Write why the app is going down. Nothing before [`start`].
pub fn why(reason: &str) {
    if let Some(log) = RUN.get() {
        log.why(reason);
    }
}

/// Write a panic: its one line as the reason, and the backtrace under it.
pub fn panicked(reason: &str, backtrace: &str) {
    if let Some(log) = RUN.get() {
        log.why(reason);
        log.line(backtrace.trim_end());
    }
}

/// Write that the run is ending as it meant to: its window has gone, or it has said in a box
/// why it could not start.
pub fn closed() {
    if let Some(log) = RUN.get() {
        log.closed();
    }
}

/// The editor's window is up.
pub fn window_open() {
    WINDOW.store(true, Ordering::Relaxed);
}

/// Whether the editor's window has come up yet.
pub fn window_is_open() -> bool {
    WINDOW.load(Ordering::Relaxed)
}

/// Put a box up saying why supersilvia cannot start, with where its log is, and write that it
/// said so. For an exit before the window is up; the main thread only.
pub fn cannot_start(why: &str) {
    let log = path().map_or_else(
        || "There is no log file.".to_owned(),
        |p| format!("The log is {}.", p.display()),
    );
    crate::platform::alert::fatal(
        "supersilvia cannot start",
        &format!(
            "{why}\n\nRunning supersilvia --check in a terminal says what this computer is \
             missing. {log}"
        ),
    );
    closed();
}

/// A log's last `lines` lines, or what stood in the way of reading it.
pub fn tail(path: &Path, lines: usize) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(lines)..].join("\n")
        }
        Err(e) => format!("could not read {}: {e}", path.display()),
    }
}

/// The GPU the editor renders on, as the report's line for it.
pub fn gpu_line(choice: Option<&crate::render::adapter::Choice>) -> Line {
    match choice {
        Some(choice) => Line::new(
            Verdict::Pass,
            "GPU",
            format!(
                "{}, picked by {}",
                crate::render::adapter::describe(choice.in_use()),
                choice.how()
            ),
        ),
        None => Line::new(Verdict::Warn, "GPU", "none was handed to the editor"),
    }
}

/// What Help ▸ Report a problem… copies, from what has been gathered.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub os: String,
    pub gpu: String,
    /// The last run, where it closed unexpectedly.
    pub last_run: Option<Unexpected>,
    /// `--check`'s report, as the terminal shows it.
    pub check: String,
    /// Each log's heading and its last lines: this run's, and the last run's where it closed
    /// unexpectedly.
    pub logs: Vec<(String, String)>,
}

impl Report {
    /// Everything [`Report::text`] says, gathered: the operating system, `--check` with the
    /// GPU handed in, and the logs' tails. Reads GStreamer's registry and the files, so it is
    /// called off the frame thread.
    pub fn gather(gpu: Line, last_run: Option<Unexpected>) -> Self {
        let gpu_text = gpu.detail.clone();
        let check = crate::check::report(&crate::check::with_gpu(gpu));
        let mut logs = Vec::new();
        match path() {
            Some(p) => logs.push((
                format!("This run's log, last {TAIL_LINES} lines ({})", p.display()),
                tail(p, TAIL_LINES),
            )),
            None => logs.push((
                "This run's log".to_owned(),
                "there is no log file".to_owned(),
            )),
        }
        if let Some(last) = &last_run {
            logs.push((
                format!(
                    "The last run's log, last {TAIL_LINES} lines ({})",
                    last.log.display()
                ),
                tail(&last.log, TAIL_LINES),
            ));
        }
        Self {
            os: crate::platform::check::os(),
            gpu: gpu_text,
            last_run,
            check,
            logs,
        }
    }

    /// The block a person pastes into the bug channel or an issue: their words, then one
    /// header about this computer, then `--check` and each log in a fenced block, which
    /// Discord and GitHub both show as code.
    pub fn text(&self, words: &Words) -> String {
        use std::fmt::Write as _;
        let said = |text: &str| {
            let text = text.trim();
            if text.is_empty() {
                "_Not filled in._".to_owned()
            } else {
                text.to_owned()
            }
        };
        let mut out = format!("**What's up?**\n{}\n", said(&words.up));
        if !words.name.trim().is_empty() {
            let _ = write!(out, "\n**From:** {}\n", words.name.trim());
        }
        let _ = write!(
            out,
            "\nsupersilvia {}\nOS: {}\nGPU: {}\n",
            env!("CARGO_PKG_VERSION"),
            self.os,
            self.gpu
        );
        if let Some(last) = &self.last_run {
            let _ = writeln!(
                out,
                "Last run: closed unexpectedly{}",
                last.why.as_deref().map_or_else(
                    || ", with no reason written".to_owned(),
                    |w| format!(": {w}")
                )
            );
        }
        let _ = write!(out, "\n--check\n```\n{}```\n", self.check);
        for (heading, tail) in &self.logs {
            let _ = write!(out, "\n{heading}\n```\n{}\n```\n", tail.trim_end());
        }
        out
    }
}

/// What the person wrote in the form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Words {
    pub up: String,
    pub name: String,
}

/// A new GitHub issue on the repository, its title the first line of what's up, cut to 80
/// characters. Only the title: a body with the logs in it would run past what a URL holds.
pub fn issue_url(up: &str) -> String {
    let base = format!("{}/issues/new", env!("CARGO_PKG_REPOSITORY"));
    let title: String = up
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect();
    if title.is_empty() {
        return base;
    }
    format!("{base}?title={}", percent_encoded(&title))
}

/// Every byte but the unreserved ones as `%XX`, as a query value needs.
fn percent_encoded(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The editor's side: the last run's notice, and Help ▸ Report a problem…'s form with what it
/// gathered.
#[derive(Default)]
pub(super) struct Desk {
    /// The last run, where it closed unexpectedly: what the notice says and the report
    /// carries.
    last_run: Option<Unexpected>,
    /// The notice is on screen.
    notice: bool,
    /// The form, while its window is open.
    form: Option<crate::ui::report::ReportState>,
    /// What the form's report says about this computer, once gathered.
    gathered: Option<Report>,
    /// The gathering, on its thread.
    gathering: Option<Receiver<Report>>,
}

impl Desk {
    /// Whether the notice is on screen, which holds every other question back.
    pub(super) fn notice_up(&self) -> bool {
        self.notice
    }
}

impl App {
    /// Say that the last run closed unexpectedly, before anything else is asked: what `main`
    /// hands over from [`last_run`], or a test.
    pub fn notice_last_run(&mut self, last: Option<Unexpected>) {
        self.crashlog.notice = last.is_some();
        self.crashlog.last_run = last;
    }

    /// The notice on screen, if it is.
    pub fn last_run_noticed(&self) -> Option<&Unexpected> {
        self.crashlog
            .last_run
            .as_ref()
            .filter(|_| self.crashlog.notice)
    }

    /// Whether Help ▸ Report a problem…'s window is open.
    pub fn reporting(&self) -> bool {
        self.crashlog.form.is_some()
    }

    /// Whether what the report says about this computer has been gathered.
    pub fn report_gathered(&self) -> bool {
        self.crashlog.gathered.is_some()
    }

    /// Help ▸ Report a problem…: open the form, and gather what it says about this computer on
    /// a thread of its own, afresh, so the log's end is as of now. A form already open keeps
    /// what was typed in it.
    pub(super) fn report_problem(&mut self) {
        self.crashlog.form.get_or_insert_default();
        self.crashlog.gathered = None;
        let gpu = gpu_line(self.gpu_choice.as_ref());
        let last_run = self.crashlog.last_run.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::thread::Builder::new()
            .name("report".into())
            .spawn(move || {
                let _ = tx.send(Report::gather(gpu, last_run));
            });
        match started {
            Ok(_) => self.crashlog.gathering = Some(rx),
            Err(e) => log::warn!("could not start gathering a report: {e}"),
        }
    }

    /// Once a frame: the notice while it is up, a gathering that has landed, and the form.
    pub(super) fn crashlog_frame(&mut self, ui: &egui::Ui) {
        self.last_run_window(ui);
        if let Some(rx) = &self.crashlog.gathering {
            match rx.try_recv() {
                Ok(report) => {
                    self.crashlog.gathered = Some(report);
                    self.crashlog.gathering = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.crashlog.gathering = None,
            }
        }
        self.report_window(ui.ctx());
    }

    /// Help ▸ Report a problem…'s window, while it is open. Copy is the one way to the
    /// clipboard; each link only opens its page.
    fn report_window(&mut self, ctx: &egui::Context) {
        use crate::ui::report::{self, Gathered, ReportAction, ReportView};
        let Some(mut form) = self.crashlog.form.take() else {
            return;
        };
        let words = Words {
            up: form.up.clone(),
            name: form.name.clone(),
        };
        let gathered = self.crashlog.gathered.as_ref();
        let full = form
            .show_full
            .then(|| gathered.map(|g| g.text(&words)))
            .flatten();
        let view = ReportView {
            gathered: gathered.map(|g| Gathered {
                os: &g.os,
                gpu: &g.gpu,
                last_run: g.last_run.is_some(),
            }),
            full: full.as_deref(),
        };
        let actions = report::show(ctx, &mut form, &view, &self.theme);
        let mut open = true;
        for action in actions {
            match action {
                ReportAction::Copy => {
                    if let Some(g) = gathered {
                        ctx.copy_text(g.text(&words));
                        form.copied = true;
                    }
                }
                ReportAction::OpenDiscord => {
                    ctx.open_url(egui::OpenUrl::new_tab(BUG_CHANNEL));
                }
                ReportAction::OpenGitHub => {
                    ctx.open_url(egui::OpenUrl::new_tab(issue_url(&form.up)));
                }
                ReportAction::Close => open = false,
            }
        }
        if open {
            self.crashlog.form = Some(form);
        } else {
            self.crashlog.gathering = None;
            self.crashlog.gathered = None;
        }
    }

    /// *supersilvia closed unexpectedly*: why, as the last run's log says, with Show log and
    /// OK. Modal, like the recovery question, which waits behind it.
    fn last_run_window(&mut self, ui: &egui::Ui) {
        let Some(last) = self.last_run_noticed().cloned() else {
            return;
        };
        let mut ok = false;
        egui::Modal::new(egui::Id::new("closed-unexpectedly")).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.label(egui::RichText::new("supersilvia closed unexpectedly").strong());
            ui.add_space(4.0);
            ui.label(match &last.why {
                Some(why) => format!("The last time it ran, it stopped with: {}", shortened(why)),
                None => "The last time it ran, it wrote no reason. That happens when the \
                         system stops it, or when a driver fails underneath it."
                    .to_owned(),
            });
            ui.add_space(4.0);
            ui.label("Help ▸ Report a problem… puts its log in a bug report.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Show log")
                    .on_hover_text(format!(
                        "{}, in {}",
                        last.log.display(),
                        crate::platform::files::MANAGER
                    ))
                    .clicked()
                {
                    crate::platform::files::reveal_file(last.log.clone());
                }
                if ui.button("OK").clicked() {
                    ok = true;
                }
            });
        });
        if ok {
            self.crashlog.notice = false;
        }
    }
}

/// A reason as the notice prints it: at most 300 characters, cut with an ellipsis.
fn shortened(why: &str) -> String {
    const MOST: usize = 300;
    if why.chars().count() <= MOST {
        return why.to_owned();
    }
    let cut: String = why.chars().take(MOST - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of this test's own, empty.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ssv-crashlog-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn a_log_with_its_closing_line_ended_as_it_meant_to() {
        assert_eq!(judge(""), Ended::Nothing);
        assert_eq!(
            judge("supersilvia 0.9\n[..] info\n== closed\n"),
            Ended::Closed
        );
        assert_eq!(
            judge("supersilvia 0.9\n!! thread 'synth' panicked at a.rs:1:2: boom\n  0: frame\n"),
            Ended::Unexpectedly(Some("thread 'synth' panicked at a.rs:1:2: boom".to_owned()))
        );
        assert_eq!(
            judge("supersilvia 0.9\n!! first\n!! second\n"),
            Ended::Unexpectedly(Some("second".to_owned())),
            "the last reason is why"
        );
        assert_eq!(
            judge("supersilvia 0.9\n[..] info\n"),
            Ended::Unexpectedly(None),
            "a run killed from outside writes nothing"
        );
    }

    /// The first run finds nothing; a run that closed leaves nothing to say; a run that went
    /// down says why on the next, and its log is kept as `previous.log`.
    #[test]
    fn each_launch_judges_the_run_before_it() {
        let dir = dir("judges");
        let first = Log::open(&dir).unwrap();
        assert_eq!(first.last_run(), None, "nothing ran before");
        first.line("supersilvia test");
        first.closed();
        drop(first);

        let second = Log::open(&dir).unwrap();
        assert_eq!(second.last_run(), None, "the last run closed");
        assert!(
            std::fs::read_to_string(dir.join(PREVIOUS))
                .unwrap()
                .ends_with("== closed\n"),
            "and its log is kept"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(LOG)).unwrap(),
            "",
            "this run's log starts empty"
        );
        second.line("supersilvia test");
        second.why("The GPU stopped responding.\nYour work was saved.");
        drop(second);

        let third = Log::open(&dir).unwrap();
        assert_eq!(
            third.last_run(),
            Some(&Unexpected {
                why: Some("The GPU stopped responding. Your work was saved.".to_owned()),
                log: dir.join(PREVIOUS),
            }),
            "a reason is one line"
        );
        drop(third);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// While one run holds the log, a second writes its own and judges nothing; once it has
    /// gone, the next to hold the log deletes what it left.
    #[test]
    fn a_second_instance_writes_a_log_of_its_own() {
        let dir = dir("second");
        let held = Log::open(&dir).unwrap();
        held.line("supersilvia test");
        let beside = Log::open(&dir).unwrap();
        assert_ne!(beside.path(), held.path());
        assert_eq!(beside.last_run(), None);
        beside.line("the second");
        assert_eq!(
            std::fs::read_to_string(held.path()).unwrap(),
            "supersilvia test\n",
            "the held log is left alone"
        );
        let own = beside.path().to_path_buf();
        assert!(own.is_file());
        drop(beside);
        held.closed();
        drop(held);

        let next = Log::open(&dir).unwrap();
        assert!(!own.exists(), "a gone instance's log is deleted");
        drop(next);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Records stop at the cap; a reason and the closing line do not.
    #[test]
    fn the_file_stops_growing_at_the_cap() {
        let dir = dir("cap");
        let log = Log::open(&dir).unwrap();
        let mut pipe = log.pipe();
        let block = vec![b'x'; 1 << 20];
        for _ in 0..(CAP >> 20) + 4 {
            pipe.write_all(&block).unwrap();
        }
        log.why("then it went down");
        log.closed();
        let size = std::fs::metadata(log.path()).unwrap().len();
        assert!(size < CAP + 4096, "{size}");
        let end = tail(log.path(), 3);
        assert!(end.contains("the log stops here"), "{end}");
        assert!(end.ends_with("!! then it went down\n== closed"), "{end}");
        drop(log);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_tail_is_the_last_lines() {
        let dir = dir("tail");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.log");
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();
        assert_eq!(tail(&file, 2), "three\nfour");
        assert_eq!(tail(&file, 10), "one\ntwo\nthree\nfour");
        assert!(tail(&dir.join("none"), 2).starts_with("could not read"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The report says the person's words first — what happened, what came before where they
    /// said, who they are where they said — then the version, the OS and the GPU, the last run
    /// where it went down, and `--check` and each log fenced as code.
    #[test]
    fn a_report_is_one_block_to_paste() {
        let report = Report {
            os: "Fedora Linux 44, kernel 7.1.10, KDE on wayland".to_owned(),
            gpu: "Intel Arc".to_owned(),
            last_run: Some(Unexpected {
                why: Some("thread 'main' panicked at a.rs:1:2: boom".to_owned()),
                log: PathBuf::from("/x/previous.log"),
            }),
            check: "supersilvia 0.9 --check\n\n  PASS  GPU  Intel Arc\n".to_owned(),
            logs: vec![("This run's log".to_owned(), "a\nb\n".to_owned())],
        };
        let text = report.text(&Words {
            up: "  The mix went black.\n".to_owned(),
            name: "ana#1".to_owned(),
        });
        assert!(
            text.starts_with(&format!(
                "**What's up?**\nThe mix went black.\n\n**From:** ana#1\n\n\
                 supersilvia {}\nOS: Fedora Linux 44, kernel 7.1.10, KDE on wayland\nGPU: Intel Arc\n\
                 Last run: closed unexpectedly: thread 'main' panicked at a.rs:1:2: boom\n",
                env!("CARGO_PKG_VERSION")
            )),
            "{text}"
        );
        let blank = report.text(&Words::default());
        assert!(
            blank.starts_with("**What's up?**\n_Not filled in._\n\nsupersilvia "),
            "{blank}"
        );
        assert!(
            text.contains(
                "\n--check\n```\nsupersilvia 0.9 --check\n\n  PASS  GPU  Intel Arc\n```\n"
            ),
            "{text}"
        );
        assert!(
            text.ends_with("\nThis run's log\n```\na\nb\n```\n"),
            "{text}"
        );
    }

    /// A GitHub issue's title is the first line said, cut to 80 characters and encoded; none
    /// opens a bare new issue.
    #[test]
    fn an_issue_takes_its_title_from_the_first_line() {
        let base = format!("{}/issues/new", env!("CARGO_PKG_REPOSITORY"));
        assert_eq!(issue_url(""), base);
        assert_eq!(issue_url("\n  \n"), base);
        assert_eq!(
            issue_url("\nThe mix went black & froze\nthen more"),
            format!("{base}?title=The%20mix%20went%20black%20%26%20froze")
        );
        let long = issue_url(&"é".repeat(100));
        assert_eq!(
            long.matches("%C3%A9").count(),
            80,
            "80 characters, not bytes"
        );
    }

    #[test]
    fn a_long_reason_is_cut() {
        assert_eq!(shortened("short"), "short");
        let long = "x".repeat(400);
        assert_eq!(shortened(&long).chars().count(), 300);
        assert!(shortened(&long).ends_with('…'));
    }
}
