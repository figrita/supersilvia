// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Status box: a terminal pane that answers *how fast is it running, and what is holding
//! it back* on its first three lines. See [docs/ui.md](../../docs/ui.md#the-status-box).
//!
//! **One model, two renderings.** [`build`] turns a [`StatusView`] into a [`Pane`] of [`Line`]s
//! of styled [`Span`]s at a width in characters; [`show`] draws them in monospace, and the copy
//! writes the same characters with [`plain`] — every section unfolded and every Output listed —
//! so what is pasted is what was on screen. The frame is box-drawing characters and the bars
//! are block characters, eighths at their ends, because every one of them is one cell of the
//! monospace face and a line's width is a count of its cells — two for a wide character, a
//! workspace named in CJK, as a terminal counts it. The glyph test in `tests/ui.rs` holds the
//! frame's glyphs to one cell.
//!
//! **No reading moves a row.** Every section is as many lines whatever it measures — a list
//! is padded to its slots, a line longer than its column is cut with `…` and whole on its
//! hover, a number sits in a column of its own width — so only a fold or *show all* changes a
//! height. The Nodes section, whose count is the one thing that truly varies, is last.
//!
//! **Waiting is not work.** The CPU section splits the tick by the synth thread's own CPU
//! clock — see [`crate::synth::meter`] — so time blocked in the driver is one row, *waiting on
//! GPU*, and every other row is work. With the sleep they add up to the tick.
//!
//! The pane returns what was asked of it, as every surface in `ui/` does: a node to go to, the
//! folds, the text to copy. `App` does the going, the remembering and the copying.

use crate::graph::NodeId;
use crate::preferences::StatusFolds;
use crate::render::GpuPhase;
use crate::synth::meter::{Reading, Work};
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Color32, FontId, Label, Sense, TextFormat, Ui, WidgetInfo, WidgetType, text::LayoutJob,
};

/// What an Output is doing, beyond rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputState {
    Rendering,
    Linking,
    NothingConnected,
    /// Every workspace showing it is closed.
    Suspended,
    /// Awake and not drawn: nothing on screen, on air or with memory reads it.
    Idle,
    /// No GPU at all: headless.
    NoGpu,
    Error(String),
    Diagnostic(String),
}

/// One Output's row: what it is, what the GPU spent on it, and what it is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputRow {
    pub node: NodeId,
    /// `output22`: the name its link carries in the accessibility tree.
    pub name: String,
    /// What it shows: the label of the node cabled into it, or its own with nothing there.
    pub label: String,
    /// The first workspace it is on.
    pub workspace: String,
    pub resolution: (u32, u32),
    /// The GPU's own time and the worst of the last two seconds. `None` where nothing
    /// measured one, which is drawn as a dash and never as a zero.
    pub gpu_ms: Option<(f32, f32)>,
    pub dropped_per_s: f32,
    pub state: OutputState,
    /// Why it draws on every tick, as a sentence for the hover. Empty where it does not.
    pub why: String,
}

/// One CPU node's own line.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeLine {
    pub node: NodeId,
    /// `phase3`: its slug and id, which is its link's name too.
    pub name: String,
    /// Its kind's label, which the lines are grouped under.
    pub kind: String,
    pub text: String,
}

/// The read-only slice of the app the pane draws from.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusView {
    /// What the last save or load did. Empty when nothing has.
    pub file: String,
    /// Whether that line is about a file it can show — a Snap, a render — with a link.
    pub file_shows: bool,
    /// Whether that line says something failed, which it says in the accent.
    pub file_failed: bool,
    pub tick: u64,
    /// **The editor's frame**, which is only how often the canvas is redrawn: this frame, the
    /// mean of the last second and the worst of the last two, off egui's own clock.
    pub frame_ms: f32,
    pub frame_avg_ms: f32,
    pub frame_worst_ms: f32,
    /// One interval of the display the window is on: the editor's budget.
    pub refresh_ms: f32,
    /// One interval of the tick rate the preference asks for — the display's, or a fixed
    /// rate: the budget the synth's figures are against.
    pub tick_budget_ms: f32,
    pub cpu_ms: f32,
    pub cpu_avg_ms: f32,
    /// **The editor's painting on the GPU**, first paint callback to last: the last frame
    /// read, the mean and the worst of the last two seconds. `None` until one lands, which is
    /// drawn as dashes.
    pub paint_gpu: Option<(f32, f32, f32)>,
    /// Frames dropped in the last second by the Output that dropped most. Not a sum: each
    /// Output counts its own, and nine Outputs each dropping twelve is not a hundred.
    pub dropped_per_s: f32,
    pub uptime_s: f64,
    pub dropped: u64,
    /// Every drop in the project, by cause: a ring every target of which a viewer held, or a
    /// render's wait that ran out. Rare; every other frame is drawn.
    pub drops: Vec<(&'static str, u64)>,
    /// Ticks whose draw found the tick two before it still on the GPU and waited for it, since
    /// the run began: the `TICKS_IN_FLIGHT` wait, which the renderer's own throttle usually
    /// reaches before it.
    pub gpu_waits: u64,
    /// Those in the last second.
    pub gpu_waits_per_s: f32,
    pub drawn: usize,
    pub nodes: usize,
    pub shaders: usize,
    pub uniforms: usize,
    pub undo: usize,
    pub undo_depth: usize,
    pub redo: usize,
    /// Every Output, in render order.
    pub outputs: Vec<OutputRow>,
    /// How many Outputs the synth's last tick drew.
    pub outputs_drawn: usize,
    pub cpu_lines: Vec<NodeLine>,
    /// Where the tick's milliseconds went, on the CPU and the GPU. `None` until the synth has
    /// timed a whole tick with the box open.
    pub phases: Option<crate::synth::Phases>,
    /// Whether there is a GPU to time. With one, the GPU's rows are there
    /// from the first frame, dashes until the first reading lands; headless there are none.
    pub gpu: bool,
    /// Whose use the GPU's busy figure counts: [`crate::platform::gpu::WHOLE_GPU`], so a
    /// Mac's row says it is every app's.
    pub busy_of: BusyOf,
}

/// Whose use of the GPU the busy figure counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyOf {
    /// This process's share of the render engine, as Linux's DRM counters give it.
    Process,
    /// The whole GPU's, every app's, as a Mac's IORegistry gives it.
    WholeGpu,
}

impl BusyOf {
    /// What this machine's counters give.
    pub fn here() -> Self {
        if crate::platform::gpu::WHOLE_GPU {
            Self::WholeGpu
        } else {
            Self::Process
        }
    }
}

/// A section that folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Gpu,
    Cpu,
    Editor,
    Outputs,
    Nodes,
    Project,
}

/// The verdict's GPU row's hover where its figure is the Outputs' alone.
const OUTPUTS_ONLY: &str = "The Outputs drawn this tick, each its own timed pass: this \
     machine's GPU times no whole draw, so the mix, the thumbnails and the rest are not in it.";

/// The GPU section's hover where the busy figure is the whole GPU's.
const WHOLE_GPU_ABOUT: &str = "The synth's draw on the GPU, first command to last, by part: a \
     mean over the last second. The whole GPU is every app's use of it, this one's with the \
     rest, as Activity Monitor shows it: the system keeps no count per process.";

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Gpu => "GPU per tick",
            Self::Cpu => "CPU per tick",
            Self::Editor => "Editor",
            Self::Outputs => "Costliest Outputs",
            Self::Nodes => "Nodes",
            Self::Project => "Project",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Self::Gpu => {
                "The synth's draw on the GPU, first command to last, by part: a mean over the \
                 last second. The whole process is the render engine this process used over \
                 the last second, from the kernel's own counters: the editor's painting and \
                 every picture window with it."
            }
            Self::Cpu => {
                "The synth's tick from the top of one to the top of the next, split by the \
                 thread's own CPU clock: time off the CPU inside a phase is waiting on the \
                 GPU, the rest is work, and the sleep to the deadline is idle. The rows add up \
                 to the tick."
            }
            Self::Editor => {
                "The editor's own redraw, off egui's clock: only how often the canvas is \
                 painted. A minimized or covered window moves these and nothing else. GPU per \
                 frame is the editor's painting on the GPU, every panel, window and picture \
                 blit, from two timestamps on its own context read a few frames later."
            }
            Self::Outputs => {
                "Every Output by the GPU's own time for the last frame it drew: the drawing \
                 ones costliest first, then the idle ones, which are not drawn and keep a \
                 muted figure. A name goes to its node."
            }
            Self::Nodes => "Each CPU node's own line, grouped by kind. A name goes to its node.",
            Self::Project => "The graph, the undo history, the run and the last file action.",
        }
    }

    /// Whether this section is folded, in the folds.
    fn slot(self, folds: &mut StatusFolds) -> &mut bool {
        // The variants by their bare names: `tests/rules.rs` reads a line holding a path to
        // `Nodes` and a `&mut` as a reference to the model.
        use Section::{Cpu, Editor, Gpu, Nodes, Outputs, Project};
        match self {
            Gpu => &mut folds.gpu,
            Cpu => &mut folds.cpu,
            Editor => &mut folds.editor,
            Outputs => &mut folds.outputs,
            Nodes => &mut folds.nodes,
            Project => &mut folds.project,
        }
    }
}

/// Which theme color a span is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Text,
    Muted,
    Heading,
    /// Over budget, dropping, failing: something to look at.
    Accent,
    /// Something to click.
    Link,
    /// The frame and the rules.
    Rule,
    Bar,
    /// A bar's empty cells.
    Empty,
}

/// What clicking a span asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Fold(Section),
    Go(NodeId),
    AllOutputs(bool),
    Copy,
    /// Show the file the status line is about.
    Show,
}

/// A run of characters in one ink, and what clicking it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub ink: Ink,
    pub act: Option<Act>,
    /// The name in the accessibility tree, where it is not the text.
    pub name: Option<String>,
}

/// One line of the box, and what hovering it shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub spans: Vec<Span>,
    pub tip: Option<String>,
}

impl Line {
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn new() -> Self {
        Self::default()
    }

    /// A section's row, begun with its label in the label column.
    fn labeled(label: &str) -> Self {
        Self::new().put(format!("  {}", pad(label, LABEL)), Ink::Text)
    }

    fn put(mut self, text: impl Into<String>, ink: Ink) -> Self {
        self.spans.push(span(text, ink));
        self
    }

    fn act(mut self, text: impl Into<String>, act: Act, name: Option<String>) -> Self {
        self.spans.push(Span {
            text: text.into(),
            ink: Ink::Link,
            act: Some(act),
            name,
        });
        self
    }

    /// What hovering it shows, with what a cut took off put in front.
    fn hover(mut self, text: impl Into<String>) -> Self {
        self.tip = Some(text.into());
        self
    }
}

/// The whole box as text, a line to a line.
pub fn plain(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&line.text());
        out.push('\n');
    }
    out
}

/// What the copy writes: the box at this width with everything open.
pub fn copy_text(view: &StatusView, width: usize) -> String {
    plain(&build(view, &StatusFolds::unfolded(), width).lines())
}

/// The narrowest the box is laid out, in characters: the verdict's bars and figures fit.
pub const MIN_WIDTH: usize = 62;
/// How wide the box opens, in characters.
pub const OPEN_WIDTH: usize = 64;
/// How many Outputs are listed before the rest are counted: the section's slots, filled or
/// not.
pub const TOP_OUTPUTS: usize = 8;
/// The CPU nodes named under their total: the block's slots, filled or not.
pub const TOP_NODES: usize = crate::synth::meter::TOP_NODES;
/// The label column inside a section.
const LABEL: usize = 16;
/// A section's bars, in cells.
const BAR: usize = 16;
/// The verdict's bars, in cells.
const VERDICT_BAR: usize = 22;
/// The verdict's label column.
const VERDICT_LABEL: usize = 15;

/// Which of the two the verdict names.
/// The verdict's GPU figure in milliseconds, and what it measures.
pub type GpuReading = (f32, GpuFigure);

/// What the verdict's GPU figure measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuFigure {
    /// The synth's whole draw, first command to last.
    Draw,
    /// The Outputs drawn, each its own pass: all a Mac can time.
    Outputs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Gpu,
    Cpu,
}

/// The synth's draw on the GPU and its work on the CPU, each a mean per tick, and which is
/// larger. `None` until a tick has been timed; the GPU's figure is `None` where nothing
/// measured one, and then the CPU is what bounds it.
///
/// **Where the draw has no whole reading but its Outputs do**, the GPU's figure is what the
/// Outputs drawing cost, [`GpuFigure::Outputs`]: Metal writes no timestamp for the empty
/// passes that mark a draw's phases, while an Output's pass, which draws, is timed.
pub fn verdict(view: &StatusView) -> Option<(Option<GpuReading>, f32, Bound)> {
    let phases = view.phases.as_ref()?;
    let gpu = phases
        .gpu
        .map(|g| (g[GpuPhase::Whole].avg, GpuFigure::Draw))
        .or_else(|| {
            let mut drawn = view.outputs.iter().filter_map(drawing).peekable();
            (view.gpu && drawn.peek().is_some()).then(|| (drawn.sum(), GpuFigure::Outputs))
        });
    let cpu = work_per_tick(phases);
    let bound = if gpu.is_some_and(|(g, _)| g > cpu) {
        Bound::Gpu
    } else {
        Bound::Cpu
    };
    Some((gpu, cpu, bound))
}

/// Ticks a second, from the timed tick's mean interval, which nothing clamps. `None` until a
/// tick has been timed.
fn tick_rate(view: &StatusView) -> Option<f32> {
    view.phases.as_ref().map(|p| hz(p.tick.avg))
}

/// The tick's work on the CPU: every [`Work`], with the waiting and the sleep left out.
fn work_per_tick(phases: &crate::synth::Phases) -> f32 {
    phases.work.iter().map(|r| r.avg).sum()
}

/// How many cells of a terminal `s` takes: what a line's width is counted in.
fn cells(s: &str) -> usize {
    s.chars().map(cell_width).sum()
}

/// How many cells `c` takes: two for the East Asian wide and full-width blocks and the emoji,
/// none for a combining mark, a joiner or a variation selector, one for the rest.
fn cell_width(c: char) -> usize {
    match u32::from(c) {
        0x0300..=0x036F | 0x200B..=0x200F | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F => {
            0
        }
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// The start of `s` that fits in `width` cells.
fn take_cells(s: &str, width: usize) -> String {
    let mut used = 0;
    s.chars()
        .take_while(|c| {
            used += cell_width(*c);
            used <= width
        })
        .collect()
}

/// The box, `width` characters a line: the top edge and the verdict, which stay put above
/// the sections; the sections, which scroll; and the bottom edge, which stays put below them.
pub struct Pane {
    pub top: Vec<Line>,
    pub body: Vec<Line>,
    pub foot: Vec<Line>,
}

impl Pane {
    /// Every line, top to bottom.
    pub fn lines(self) -> Vec<Line> {
        let mut lines = self.top;
        lines.extend(self.body);
        lines.extend(self.foot);
        lines
    }
}

/// The box as lines, `width` characters each.
pub fn build(view: &StatusView, folds: &StatusFolds, width: usize) -> Pane {
    let width = width.max(MIN_WIDTH);
    let mut b = Builder {
        lines: Vec::new(),
        width,
    };
    let phases = view.phases.as_ref();
    b.top(view);
    b.verdict(view);
    let top = std::mem::take(&mut b.lines);
    if view.gpu {
        b.gpu(view, phases, *folds);
    }
    b.cpu(view, phases, *folds);
    b.editor(view, *folds);
    b.outputs(view, *folds);
    b.project(view, *folds);
    b.nodes(view, *folds);
    let body = std::mem::take(&mut b.lines);
    b.bottom();
    Pane {
        top,
        body,
        foot: b.lines,
    }
}

struct Builder {
    lines: Vec<Line>,
    width: usize,
}

impl Builder {
    /// Room between `│ ` and ` │`.
    fn inner(&self) -> usize {
        self.width - 4
    }

    /// A row between the two edges, padded to the right one — or cut, where it runs over,
    /// with an ellipsis on the last span that reaches past it, and what was cut whole on its
    /// hover.
    fn boxed(&mut self, row: Line) {
        let inner = self.inner();
        let mut spans = vec![span("│ ", Ink::Rule)];
        let mut used = 0;
        let mut lost = String::new();
        let mut rest = row.spans.into_iter();
        while let Some(mut s) = rest.next() {
            let n = cells(&s.text);
            if used + n <= inner {
                used += n;
                spans.push(s);
                continue;
            }
            let keep = inner - used;
            let trimmed = s.text.trim_end();
            let tail: String = rest.map(|s| s.text).collect();
            if cells(trimmed) <= keep && tail.trim().is_empty() {
                // Only padding runs over: cut it, and there is nothing to mark.
                s.text = take_cells(&s.text, keep);
            } else {
                lost = format!("{}{tail}", s.text).trim().to_string();
                if keep == 0 {
                    // The line is full to the edge: the ellipsis takes the last cell, and
                    // nothing of this span is left to show.
                    if let Some(last) = spans.iter_mut().skip(1).last() {
                        let n = cells(&last.text);
                        last.text = cut(&format!("{}…", last.text), n);
                    }
                    s.text.clear();
                } else {
                    s.text = cut(&s.text, keep);
                }
            }
            spans.push(s);
            // Counted again: a wide character that did not fit leaves a cell short of the cut.
            used = spans.iter().skip(1).map(|s| cells(&s.text)).sum();
            break;
        }
        spans.push(span(format!("{} │", " ".repeat(inner - used)), Ink::Rule));
        let tip = match row.tip {
            tip if lost.is_empty() => tip,
            Some(h) if h.contains(&lost) => Some(h),
            Some(h) => Some(format!("{lost}\n\n{h}")),
            None => Some(lost),
        };
        self.lines.push(Line { spans, tip });
    }

    /// A label and one run of text after it.
    fn row(&mut self, label: &str, text: impl Into<String>, ink: Ink) {
        self.boxed(Line::labeled(label).put(text, ink));
    }

    /// A row whose hover says the whole of it, so what a cut took off is not added to it.
    fn boxed_whole(&mut self, row: Line, tip: String) {
        self.boxed(row);
        if let Some(line) = self.lines.last_mut() {
            line.tip = Some(tip);
        }
    }

    /// The synth's rate against the rate it is asked to tick at: the timed tick's mean
    /// interval, and a dash until a tick has been timed.
    fn top(&mut self, view: &StatusView) {
        let rate = tick_rate(view);
        let want = hz(view.tick_budget_ms);
        let right = match rate {
            Some(rate) => format!(" {rate:>3.0} Hz of {want:.0} "),
            None => format!(" {:>3} Hz of {want:.0} ", "—"),
        };
        let behind = rate.is_some_and(|rate| short_of(rate, want));
        let title = "┌ STATUS ";
        let fill = self.width - cells(title) - cells(&right) - 1;
        self.lines.push(Line {
            spans: vec![
                span(title, Ink::Rule),
                span("─".repeat(fill), Ink::Rule),
                span(right, if behind { Ink::Accent } else { Ink::Heading }),
                span("┐", Ink::Rule),
            ],
            tip: None,
        });
    }

    /// Four rows, whatever is measured: the two bars, the pacing, and how often the tick
    /// waited for the GPU with the drops by cause.
    fn verdict(&mut self, view: &StatusView) {
        let budget = view.tick_budget_ms;
        match verdict(view) {
            None => {
                self.boxed(
                    Line::new()
                        .put(pad("measuring", VERDICT_LABEL), Ink::Text)
                        .put("the first whole tick…", Ink::Muted),
                );
                self.boxed(Line::new());
            }
            Some((gpu, cpu, bound)) => {
                let keeping_up = tick_rate(view).is_some_and(|rate| !short_of(rate, hz(budget)));
                let label = if keeping_up {
                    "busiest"
                } else {
                    "held back by"
                };
                let scale = gpu.map_or(0.0, |(g, _)| g).max(cpu).max(budget);
                let mut rows = [
                    (Bound::Cpu, "CPU", Some((cpu, GpuFigure::Draw))),
                    (Bound::Gpu, "GPU", gpu),
                ];
                rows.sort_by_key(|(b, ..)| *b != bound);
                for (i, (which, name, ms)) in rows.into_iter().enumerate() {
                    let head = Line::new()
                        .put(
                            pad(if i == 0 { label } else { "" }, VERDICT_LABEL),
                            Ink::Text,
                        )
                        .put(
                            format!("{name}  "),
                            if which == bound {
                                Ink::Heading
                            } else {
                                Ink::Muted
                            },
                        );
                    let Some((ms, figure)) = ms else {
                        let why = if view.gpu {
                            "not measured yet"
                        } else {
                            "no GPU"
                        };
                        self.boxed(
                            head.put(pad("—", VERDICT_BAR), Ink::Muted)
                                .put(format!("  {why}"), Ink::Muted),
                        );
                        continue;
                    };
                    let over = ms > budget;
                    let (full, empty) = bar(ms, scale, VERDICT_BAR, true);
                    // The cells past the budget in the accent.
                    let at = ((budget / scale) * VERDICT_BAR as f32).round() as usize;
                    let within: String = full.chars().take(at).collect();
                    let beyond: String = full.chars().skip(at).collect();
                    let row = head
                        .put(within, Ink::Bar)
                        .put(beyond, Ink::Accent)
                        .put(empty, Ink::Empty)
                        .put(
                            format!("  {:>4} ms / {}", short(ms), short(budget)),
                            if over { Ink::Accent } else { Ink::Text },
                        );
                    if figure == GpuFigure::Outputs {
                        self.boxed_whole(row, OUTPUTS_ONLY.to_string());
                    } else {
                        self.boxed(row);
                    }
                }
            }
        }
        // Even while the ninety-ninth percentile is within half again the median.
        let pacing = view.phases.as_ref().map(|p| p.pacing);
        let (shape, p99) = match pacing {
            Some(p) if p.frames == 0 || p.p99 <= p.p50 * 1.5 => (("even", Ink::Text), p.p99),
            Some(p) => (("uneven", Ink::Accent), p.p99),
            None => (("—", Ink::Muted), 0.0),
        };
        let p99 = pacing.map_or_else(|| "—".to_string(), |_| short(p99 * 1000.0));
        let row = Line::new()
            .put(pad("pacing", VERDICT_LABEL), Ink::Text)
            .put(pad(shape.0, 6), shape.1)
            .put(format!(" · p99 {p99:>4} ms · "), Ink::Text)
            .put(
                format!("{} dropped", view.dropped),
                if view.dropped_per_s > 0.0 {
                    Ink::Accent
                } else {
                    Ink::Text
                },
            );
        let window = pacing.map_or_else(
            || "No tick timed since the box opened.".to_string(),
            |p| {
                format!(
                    "The last {} ticks since the box opened: a median of {} ms.",
                    p.frames,
                    short(p.p50 * 1000.0)
                )
            },
        );
        let tip = format!(
            "{window} {} dropped since the run began, {:.0} a second on the worst Output in the \
             last second.",
            view.dropped, view.dropped_per_s
        );
        self.boxed_whole(row, tip);
        let causes = if view.drops.is_empty() {
            "none".to_string()
        } else {
            view.drops
                .iter()
                .map(|(name, n)| format!("{name} {n}"))
                .collect::<Vec<_>>()
                .join(" · ")
        };
        // The share of the last second's ticks whose draw waited for the tick two before it, from
        // the two rates.
        let waited = tick_rate(view).map_or(0, |rate| {
            (view.gpu_waits_per_s / rate * 100.0)
                .round()
                .clamp(0.0, 100.0) as u32
        });
        self.boxed(
            Line::new()
                .put(pad("", VERDICT_LABEL), Ink::Text)
                .put(
                    format!("GPU waits {waited:>3}% of ticks"),
                    if waited > 0 { Ink::Text } else { Ink::Muted },
                )
                .put(format!(" · by cause: {causes}"), Ink::Muted)
                .hover(format!(
                    "Ticks in the last second whose draw found the tick two before it still on \
                     the GPU and waited for it: the two-tick bound on the GPU's queue. The \
                     renderer's throttle, which submits only once at most one earlier \
                     submission is still running, usually holds a tick back before it gets \
                     there, so this can read 0% while the tick waits; waiting on GPU, under CPU \
                     per tick, is the whole of the wait. {} since the run began. A frame drops \
                     only where a viewer holds every target of an Output's ring (held) or a \
                     render's wait for a frame ran out (capture).",
                    view.gpu_waits
                )),
        );
    }

    /// A section's heading rule: its fold, its title, and its figure at the right — or, folded,
    /// its summary after the title. Whether it is folded.
    fn heading(
        &mut self,
        section: Section,
        mut folds: StatusFolds,
        summary: &str,
        right: &str,
    ) -> bool {
        let folded = *section.slot(&mut folds);
        let mark = if folded { "▸" } else { "▾" };
        let title = format!("{mark} {}", section.title());
        let mut spans = vec![span("├ ", Ink::Rule)];
        spans.push(Span {
            text: title.clone(),
            ink: Ink::Heading,
            act: Some(Act::Fold(section)),
            name: Some(format!("{} section", section.title())),
        });
        let mut used = 2 + cells(&title);
        let right = if right.is_empty() || folded {
            String::new()
        } else {
            format!(" {right} ")
        };
        if folded && !summary.is_empty() {
            // The summary cut to what is left before the rule's last three cells.
            let room = self.width.saturating_sub(used + 2 + 1 + 1 + 2);
            let s = format!("  {}", cut(summary, room));
            used += cells(&s);
            spans.push(span(s, Ink::Muted));
        }
        let fill = self
            .width
            .saturating_sub(used + 1 + cells(&right) + 2)
            .max(1);
        spans.push(span(format!(" {}", "─".repeat(fill)), Ink::Rule));
        if !right.is_empty() {
            spans.push(span(right, Ink::Heading));
        }
        spans.push(span("─┤", Ink::Rule));
        self.lines.push(Line {
            spans,
            tip: Some(section.about().to_string()),
        });
        folded
    }

    /// A section's row: a label, a bar against the section's total, the figure — a dash where
    /// nothing measured one — and whatever follows it, on the one line.
    fn measured(
        &mut self,
        label: &str,
        (ms, scale): (Option<f32>, f32),
        warm: bool,
        detail: Option<String>,
    ) {
        let (full, figure) = match ms {
            Some(ms) => (bar(ms, scale, BAR, false).0, fixed(ms)),
            None => (String::new(), "—".to_string()),
        };
        let mut row = Line::labeled(label)
            .put(
                format!("{full:<BAR$}"),
                if warm { Ink::Accent } else { Ink::Bar },
            )
            .put(
                format!("{figure:>6}"),
                if ms.is_none() {
                    Ink::Muted
                } else if warm {
                    Ink::Accent
                } else {
                    Ink::Text
                },
            );
        if let Some(detail) = detail {
            row = row.put(format!("  {detail}"), Ink::Muted);
        }
        self.boxed(row);
    }

    /// Six parts and the whole process — or the whole GPU, where the machine counts only that:
    /// seven rows, dashes until the first reading.
    fn gpu(
        &mut self,
        view: &StatusView,
        phases: Option<&crate::synth::Phases>,
        folds: StatusFolds,
    ) {
        let gpu = phases.and_then(|p| p.gpu.as_ref());
        let total = gpu.map_or(0.0, |g| g[GpuPhase::Whole].avg);
        let figure = gpu.map_or_else(|| "— ms".to_string(), |_| format!("{} ms", fixed(total)));
        let folded = self.heading(Section::Gpu, folds, &figure, &figure);
        if view.busy_of == BusyOf::WholeGpu
            && let Some(line) = self.lines.last_mut()
        {
            line.tip = Some(WHOLE_GPU_ABOUT.to_string());
        }
        if folded {
            return;
        }
        let (drawn, of) = (view.outputs_drawn, view.outputs.len());
        for phase in GpuPhase::ALL {
            let detail = match phase {
                GpuPhase::Whole => continue,
                GpuPhase::Outputs => Some(format!("({drawn} drawn of {of})")),
                _ => None,
            };
            let ms = gpu.map(|g| g[phase].avg);
            self.measured(gpu_label(phase), (ms, total), false, detail);
        }
        let (label, of) = if view.busy_of == BusyOf::WholeGpu {
            ("whole GPU", "· every app")
        } else {
            ("whole process", "of the render engine")
        };
        let busy = phases.and_then(|p| p.busy);
        let (text, ink) = match busy {
            Some(busy) => (
                format!("{:>3.0} % {of}", busy * 100.0),
                if busy >= 0.9 { Ink::Accent } else { Ink::Muted },
            ),
            None => (format!("  — % {of}"), Ink::Muted),
        };
        self.row(label, text, ink);
    }

    /// The waiting, every work, the costliest CPU nodes in their slots under their total, and
    /// the sleep: the same rows whether or not a tick has been timed.
    fn cpu(
        &mut self,
        view: &StatusView,
        phases: Option<&crate::synth::Phases>,
        folds: StatusFolds,
    ) {
        let total = phases.map_or(0.0, |p| p.tick.avg);
        let budget = view.tick_budget_ms;
        let (summary, right) = match phases {
            Some(p) => (
                format!("{} ms · {} work", fixed(total), fixed(work_per_tick(p))),
                format!("{} ms", fixed(total)),
            ),
            None => ("measuring".to_string(), "— ms".to_string()),
        };
        let folded = self.heading(Section::Cpu, folds, &summary, &right);
        if folded {
            return;
        }
        self.measured(
            "waiting on GPU",
            (phases.map(|p| p.waiting.avg.max(0.0)), total),
            false,
            None,
        );
        for work in Work::ALL {
            let r = phases.map(|p| p.work[work as usize]);
            let warm = r.is_some_and(|r| r.avg > budget);
            self.measured(work_label(work), (r.map(|r| r.avg), total), warm, None);
            if work == Work::Nodes {
                for slot in 0..TOP_NODES {
                    self.node_cost(phases.and_then(|p| p.nodes.get(slot)));
                }
            }
        }
        self.measured("idle", (phases.map(|p| p.asleep.avg), total), false, None);
    }

    /// One slot under `CPU nodes`: a node by its mean, in the bar's column with its figure in
    /// the figure's, or a dash where fewer nodes tick.
    fn node_cost(&mut self, node: Option<&(String, Reading)>) {
        let row = Line::labeled("");
        self.boxed(match node {
            Some((name, r)) => {
                let shown = cut(name, BAR - 1);
                let row = row
                    .put(pad(&shown, BAR), Ink::Muted)
                    .put(format!("{:>6}", fixed(r.avg)), Ink::Muted);
                // A name cut to its column is whole on the hover.
                if shown == *name {
                    row
                } else {
                    row.hover(name.clone())
                }
            }
            None => row.put("—", Ink::Muted),
        });
    }

    fn editor(&mut self, view: &StatusView, folds: StatusFolds) {
        let fps = hz(view.frame_avg_ms);
        let folded = self.heading(
            Section::Editor,
            folds,
            &format!("{} ms · {fps:.0} fps", short(view.frame_avg_ms)),
            &format!("{fps:.0} fps"),
        );
        if folded {
            return;
        }
        let (text, ink) = now_avg_worst(Some((
            view.frame_ms,
            view.frame_avg_ms,
            view.frame_worst_ms,
        )));
        self.boxed_whole(
            Line::labeled("frame").put(text, ink),
            format!(
                "The display the editor is on: {:.0} Hz, {:.2} ms a frame.",
                hz(view.refresh_ms),
                view.refresh_ms
            ),
        );
        self.row(
            "CPU per frame",
            format!(
                "{:>6} ms now · {:>6} avg",
                fixed(view.cpu_ms),
                fixed(view.cpu_avg_ms)
            ),
            Ink::Text,
        );
        // Where there is a GPU to time, like the GPU section: dashes until the first reading.
        if view.gpu {
            let (text, ink) = now_avg_worst(view.paint_gpu);
            self.row("GPU per frame", text, ink);
        }
    }

    /// Eight slots and the line that counts them, however many Outputs there are; every
    /// Output and that line on *show all*.
    fn outputs(&mut self, view: &StatusView, folds: StatusFolds) {
        let sorted = by_cost(&view.outputs);
        let all = folds.all_outputs || sorted.len() <= TOP_OUTPUTS;
        let right = if sorted.len() <= TOP_OUTPUTS {
            format!("{}", sorted.len())
        } else if all {
            format!("all {}", sorted.len())
        } else {
            format!("top {TOP_OUTPUTS} of {}", sorted.len())
        };
        // What a tick costs: the Outputs it draws, and not an idle one's last figure.
        let total: f32 = sorted.iter().filter_map(|(_, o)| drawing(o)).sum();
        let folded = self.heading(
            Section::Outputs,
            folds,
            &format!("{} · {} ms", sorted.len(), fixed(total)),
            &right,
        );
        if folded {
            return;
        }
        let shown = if all { sorted.len() } else { TOP_OUTPUTS };
        for (_, o) in &sorted[..shown] {
            self.output(o);
        }
        for _ in shown..TOP_OUTPUTS {
            self.boxed(Line::new());
        }
        let row = if sorted.len() <= TOP_OUTPUTS {
            let count = match sorted.len() {
                0 => "  no Outputs".to_string(),
                1 => format!("  1 Output, {} ms", fixed(total)),
                n => format!("  {n} Outputs, {} ms", fixed(total)),
            };
            Line::new().put(count, Ink::Muted)
        } else if all {
            let fewer = format!("show top {TOP_OUTPUTS}");
            Line::new()
                .put(
                    pad(
                        &format!("  {} Outputs, {} ms", sorted.len(), fixed(total)),
                        self.inner() - cells(&fewer),
                    ),
                    Ink::Muted,
                )
                .act(fewer, Act::AllOutputs(false), None)
        } else {
            let rest = &sorted[TOP_OUTPUTS..];
            // Under what the ones still drawing cost: an idle one's figure is not this tick's.
            let most = rest
                .iter()
                .filter_map(|(_, o)| drawing(o))
                .fold(0.0_f32, f32::max);
            let under = most.floor() + 1.0;
            Line::new()
                .put(
                    pad(
                        &format!("  …{} more under {under:.0} ms", rest.len()),
                        self.inner() - 8,
                    ),
                    Ink::Muted,
                )
                .act(
                    "show all",
                    Act::AllOutputs(true),
                    Some("show all Outputs".into()),
                )
        };
        self.boxed(row);
    }

    /// One Output on one line: what it shows, its workspace, its cost, its link and, after
    /// them, what it is doing — cut to the line, and whole on the hover.
    fn output(&mut self, o: &OutputRow) {
        let ms = o
            .gpu_ms
            .map_or_else(|| "—".to_string(), |(latest, _)| fixed(latest));
        let notes = notes(o);
        let idle = o.state == OutputState::Idle;
        let warm = matches!(o.state, OutputState::Error(_) | OutputState::Diagnostic(_))
            || o.dropped_per_s > 0.0;
        let mut row = Line::new()
            .put("  ", Ink::Text)
            .act(
                pad(&cut(&o.label, 15), 16),
                Act::Go(o.node),
                Some(o.label.clone()),
            )
            .put(pad(&cut(&o.workspace, 15), 16), Ink::Muted)
            .put(
                format!("{ms:>6} ms  "),
                if warm {
                    Ink::Accent
                } else if idle {
                    Ink::Muted
                } else {
                    Ink::Text
                },
            )
            .act("▸ go", Act::Go(o.node), Some(format!("go to {}", o.name)));
        if !notes.is_empty() {
            row = row.put(
                format!("  {notes}"),
                if warm { Ink::Accent } else { Ink::Muted },
            );
        }
        self.boxed_whole(row, output_tip(o));
    }

    /// Last, because how many nodes report a line is the one count that truly varies: each
    /// node one line under its kind, cut to the line and whole on the hover.
    fn nodes(&mut self, view: &StatusView, folds: StatusFolds) {
        let folded = self.heading(
            Section::Nodes,
            folds,
            &format!("({} reporting)", view.cpu_lines.len()),
            &format!("{} reporting", view.cpu_lines.len()),
        );
        if folded {
            return;
        }
        if view.cpu_lines.is_empty() {
            self.boxed(Line::new().put("  no node reports a line", Ink::Muted));
        }
        let mut kinds: Vec<&str> = view.cpu_lines.iter().map(|l| l.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        for kind in kinds {
            let lines: Vec<&NodeLine> = view.cpu_lines.iter().filter(|l| l.kind == kind).collect();
            self.boxed(Line::new().put(format!("  {kind} ×{}", lines.len()), Ink::Heading));
            for line in lines {
                let text = one_line(&line.text);
                let tip = format!("{}: {text}", line.name);
                self.boxed_whole(
                    Line::new()
                        .put("    ", Ink::Text)
                        .act(
                            pad(&cut(&line.name, 13), 14),
                            Act::Go(line.node),
                            Some(format!("go to {}", line.name)),
                        )
                        .put(text, Ink::Text),
                    tip,
                );
            }
        }
    }

    /// Six rows: a live count is always last on its row, so its digits move nothing.
    fn project(&mut self, view: &StatusView, folds: StatusFolds) {
        let folded = self.heading(
            Section::Project,
            folds,
            &format!("{} nodes · up {}", view.nodes, uptime(view.uptime_s)),
            "",
        );
        if folded {
            return;
        }
        let rows = [
            ("nodes", format!("{} · {} drawn", view.nodes, view.drawn)),
            (
                "shaders",
                format!("{} · {} uniforms", view.shaders, view.uniforms),
            ),
            (
                "undo",
                format!("{}/{} · redo {}", view.undo, view.undo_depth, view.redo),
            ),
            ("ticks", format!("{}", view.tick)),
            ("up", uptime(view.uptime_s)),
        ];
        for (label, text) in rows {
            self.row(label, text, Ink::Text);
        }
        let file = one_line(&view.file);
        if file.is_empty() {
            self.row("file", "—", Ink::Muted);
        } else if view.file_shows {
            // The link keeps its place at the end, and the line gives way to it.
            let link = "▸ show";
            let room = self
                .inner()
                .saturating_sub(cells(&Line::labeled("file").text()) + 2 + cells(link));
            let row = Line::labeled("file")
                .put(cut(&file, room), Ink::Text)
                .put("  ", Ink::Text)
                .act(link, Act::Show, Some("show the file".into()));
            self.boxed_whole(row, file);
        } else if view.file_failed {
            self.row("file", file, Ink::Accent);
        } else {
            self.row("file", file, Ink::Text);
        }
    }

    fn bottom(&mut self) {
        let copy = "◰ copy";
        let tail = "─────┘";
        let fill = self.width - 1 - 1 - cells(copy) - 1 - cells(tail);
        self.lines.push(Line {
            spans: vec![
                span(format!("└{} ", "─".repeat(fill)), Ink::Rule),
                Span {
                    text: copy.to_string(),
                    ink: Ink::Link,
                    act: Some(Act::Copy),
                    name: Some("copy the Status box".to_string()),
                },
                span(format!(" {tail}"), Ink::Rule),
            ],
            tip: None,
        });
    }
}

/// A figure's latest, mean and worst, each in a column of its own, or dashes in them where
/// nothing measured one.
fn now_avg_worst(figure: Option<(f32, f32, f32)>) -> (String, Ink) {
    let (cells, ink) = match figure {
        Some((now, avg, worst)) => ([now, avg, worst].map(fixed), Ink::Text),
        None => (["—"; 3].map(str::to_string), Ink::Muted),
    };
    let [now, avg, worst] = cells;
    (
        format!("{now:>6} ms now · {avg:>6} avg · {worst:>6} worst"),
        ink,
    )
}

/// The Outputs with their places in the view, the drawing ones costliest first, then the idle
/// ones by the figure of the last frame each drew, then those nothing measured.
fn by_cost(outputs: &[OutputRow]) -> Vec<(usize, &OutputRow)> {
    let mut sorted: Vec<(usize, &OutputRow)> = outputs.iter().enumerate().collect();
    let rank = |o: &OutputRow| match (o.state == OutputState::Idle, o.gpu_ms) {
        (false, Some((ms, _))) => (0, ms),
        (true, Some((ms, _))) => (1, ms),
        (_, None) => (2, 0.0),
    };
    sorted.sort_by(|(_, a), (_, b)| {
        let ((ra, ca), (rb, cb)) = (rank(a), rank(b));
        ra.cmp(&rb).then(cb.total_cmp(&ca))
    });
    sorted
}

/// What an Output drawn this tick cost: its figure, and none for an idle one.
fn drawing(o: &OutputRow) -> Option<f32> {
    o.gpu_ms
        .filter(|_| o.state != OutputState::Idle)
        .map(|g| g.0)
}

/// What an Output is doing beyond rendering, as its row says it after its `▸ go`.
fn notes(o: &OutputRow) -> String {
    let state = match &o.state {
        OutputState::Rendering => None,
        OutputState::Linking => Some("linking a new shader…".to_string()),
        OutputState::NothingConnected => Some("nothing connected".to_string()),
        OutputState::Suspended => Some("suspended: no open workspace shows it".to_string()),
        OutputState::Idle => Some("idle".to_string()),
        OutputState::NoGpu => Some("no GPU".to_string()),
        OutputState::Error(e) => Some(format!("shader error: {}", one_line(e))),
        OutputState::Diagnostic(d) => Some(format!("diagnostic: {}", one_line(d))),
    };
    let mut notes: Vec<String> = state.into_iter().collect();
    if o.dropped_per_s > 0.0 {
        notes.push(format!("{:.0} dropped/s", o.dropped_per_s));
    }
    notes.join(" · ")
}

/// An Output's hover: what it is, where, what it cost and why it draws.
fn output_tip(o: &OutputRow) -> String {
    let (w, h) = o.resolution;
    let notes = notes(o);
    let drawing = if o.state == OutputState::Idle {
        " Idle: nothing on screen, on air or with memory reads it, so it is not drawn and \
         shows the last frame it drew."
            .to_string()
    } else if o.why.is_empty() {
        String::new()
    } else {
        format!(" Drawn every tick: {}.", o.why)
    };
    format!(
        "{} on {}, {w}×{h}, showing {}. The GPU's own time for the last frame it drew{}.\
         {drawing}{}",
        o.name,
        o.workspace,
        o.label,
        o.gpu_ms.map_or_else(
            || ", which nothing measured".to_string(),
            |(latest, worst)| format!(
                ": {} ms, and {} the worst of two seconds",
                fixed(latest),
                fixed(worst)
            )
        ),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" {notes}.")
        },
    )
}

/// Text on one line: every run of whitespace, a newline among them, one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn gpu_label(phase: GpuPhase) -> &'static str {
    match phase {
        GpuPhase::Uploads => "uploads",
        GpuPhase::Sims => "sims",
        GpuPhase::Outputs => "outputs",
        GpuPhase::Passes => "thumbnails",
        GpuPhase::Probes => "probes",
        GpuPhase::Mix => "mix",
        GpuPhase::Between => "between",
        GpuPhase::Whole => "whole",
    }
}

fn work_label(work: Work) -> &'static str {
    match work {
        Work::Inbox => "editor messages",
        Work::Midi => "MIDI",
        Work::Nodes => "CPU nodes",
        Work::Job => "frame job",
        Work::Draw => "draw submission",
        Work::Publish => "publishing",
        Work::Readbacks => "tap readbacks",
        Work::Snapshot => "snapshot",
        Work::Other => "everything else",
    }
}

fn span(text: impl Into<String>, ink: Ink) -> Span {
    Span {
        text: text.into(),
        ink,
        act: None,
        name: None,
    }
}

/// A bar of `cells` against `scale`: full cells, an eighth-block end, then the rest — `░` with
/// `track`, nothing without. Returned as the filled part and the empty part.
fn bar(value: f32, scale: f32, cells: usize, track: bool) -> (String, String) {
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let fraction = if scale > 0.0 {
        (value / scale).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let eighths = (fraction * (cells * 8) as f32).round() as usize;
    let full = eighths / 8;
    let part = eighths % 8;
    let mut filled = "█".repeat(full);
    let mut used = full;
    if part > 0 {
        filled.push(EIGHTHS[part]);
        used += 1;
    }
    let rest = cells - used;
    let empty = if track {
        "░".repeat(rest)
    } else {
        String::new()
    };
    (filled, empty)
}

/// Hertz from an interval in milliseconds.
/// Whether a synth ticking at `rate` is short of the `want` it is asked for: five per cent
/// short, the line the verdict and the menu bar's meter both turn the accent at.
pub fn short_of(rate: f32, want: f32) -> bool {
    rate < want * 0.95
}

fn hz(ms: f32) -> f32 {
    1000.0 / ms.max(0.001)
}

/// A figure to one decimal, never a negative zero.
fn fixed(ms: f32) -> String {
    format!("{:.1}", ms.max(0.0))
}

/// A figure as briefly as it reads: whole from ten up, one decimal under.
fn short(ms: f32) -> String {
    let ms = ms.max(0.0);
    if ms >= 10.0 {
        format!("{ms:.0}")
    } else {
        format!("{ms:.1}")
    }
}

/// `s` left-aligned in `width` characters.
fn pad(s: &str, width: usize) -> String {
    let n = cells(s);
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

/// Seconds as a clock: `28 s`, `4:07`, `1:02:15`.
fn uptime(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    }
}

/// A name cut to fit its column, with an ellipsis where it did not: at most `width` cells, and
/// one fewer where a wide character would have straddled the edge.
fn cut(name: &str, width: usize) -> String {
    if cells(name) <= width {
        name.to_string()
    } else {
        format!("{}…", take_cells(name, width.saturating_sub(1)))
    }
}

/// What a frame of the pane asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusOut {
    /// A node whose name was clicked.
    pub go: Option<NodeId>,
    /// The folds after this frame's clicks.
    pub folds: StatusFolds,
    /// The text to put on the clipboard.
    pub copy: Option<String>,
    /// The status line's file is to be shown.
    pub show: bool,
}

/// One cell of the monospace face the box is drawn in.
fn cell(ctx: &eframe::egui::Context) -> f32 {
    let font = FontId::monospace(theme::FONT_BASE);
    ctx.fonts_mut(|f| f.glyph_width(&font, '─')).max(1.0)
}

/// The window's inner width that holds a box `columns` characters wide, with room for a
/// scroll bar beside the sections should the style give it room of its own.
pub fn window_width(ctx: &eframe::egui::Context, columns: usize) -> f32 {
    let cells = columns as f32 * cell(ctx);
    cells + ctx.global_style().spacing.scroll.allocated_width()
}

/// How many characters of the monospace face fit across the pane: the inverse of
/// [`window_width`], so a window opened at a width holds exactly that many.
fn columns(ui: &Ui) -> usize {
    let room = ui.available_width() - ui.spacing().scroll.allocated_width();
    // A hair over, so a width that is an exact count of cells is not a cell short of it.
    let n = ((room + 0.01) / cell(ui.ctx())).floor() as usize;
    n.max(MIN_WIDTH)
}

/// Draw the pane: the box built at the pane's width, a line to a row. The top edge and the
/// verdict stay put above the sections, which scroll, and the bottom edge with its copy stays
/// put below them.
pub fn show(ui: &mut Ui, view: &StatusView, theme: &Theme, folds: StatusFolds) -> StatusOut {
    let width = columns(ui);
    let pane = build(view, &folds, width);
    let mut out = StatusOut {
        go: None,
        folds,
        copy: None,
        show: false,
    };
    ui.style_mut().spacing.item_spacing = eframe::egui::vec2(0.0, 0.0);
    let font = FontId::monospace(theme::FONT_BASE);
    let row = ui.fonts_mut(|f| f.row_height(&font));
    let mut draw = |ui: &mut Ui, lines: &[Line]| {
        for line in lines {
            if let Some(act) = draw_line(ui, line, theme, &font) {
                match act {
                    Act::Fold(section) => {
                        let folded = section.slot(&mut out.folds);
                        *folded = !*folded;
                    }
                    Act::Go(node) => out.go = Some(node),
                    Act::AllOutputs(all) => out.folds.all_outputs = all,
                    Act::Copy => out.copy = Some(copy_text(view, width)),
                    Act::Show => out.show = true,
                }
            }
        }
    };
    draw(ui, &pane.top);
    eframe::egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .max_height((ui.available_height() - row).max(row))
        .show(ui, |ui| {
            ui.style_mut().spacing.item_spacing = eframe::egui::vec2(0.0, 0.0);
            draw(ui, &pane.body);
        });
    draw(ui, &pane.foot);
    out
}

/// One line: a run of spans with nothing to click is one label, in as many colors as it has,
/// and a span to click is a label of its own. Returns what a click asked for.
fn draw_line(ui: &mut Ui, line: &Line, theme: &Theme, font: &FontId) -> Option<Act> {
    let mut asked = None;
    let row = ui.horizontal(|ui| {
        let mut job = LayoutJob::default();
        for s in &line.spans {
            if let Some(act) = s.act {
                flush(ui, &mut job);
                if clickable(ui, s, theme, font) {
                    asked = Some(act);
                }
            } else {
                job.append(
                    &s.text,
                    0.0,
                    TextFormat::simple(font.clone(), ink(theme, s.ink)),
                );
            }
        }
        flush(ui, &mut job);
    });
    if let Some(tip) = &line.tip {
        row.response.on_hover_ui(|ui| {
            ui.set_max_width(ui.spacing().tooltip_width);
            ui.add(Label::new(tip.as_str()));
        });
    }
    asked
}

/// Draw the run gathered so far, and start another.
fn flush(ui: &mut Ui, job: &mut LayoutJob) {
    if job.is_empty() {
        return;
    }
    let job = std::mem::take(job);
    ui.add(Label::new(job).selectable(false).extend());
}

/// A span that answers a click, named in the accessibility tree by its `name`.
fn clickable(ui: &mut Ui, s: &Span, theme: &Theme, font: &FontId) -> bool {
    let mut job = LayoutJob::default();
    job.append(
        &s.text,
        0.0,
        TextFormat::simple(font.clone(), ink(theme, s.ink)),
    );
    let response = ui
        .add(
            Label::new(job)
                .selectable(false)
                .extend()
                .sense(Sense::click()),
        )
        .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
    let name = s.name.clone().unwrap_or_else(|| s.text.clone());
    let kind = match s.act {
        Some(Act::Go(_)) => WidgetType::Link,
        _ => WidgetType::Button,
    };
    response.widget_info(|| WidgetInfo::labeled(kind, true, name.clone()));
    response.clicked()
}

fn ink(theme: &Theme, ink: Ink) -> Color32 {
    match ink {
        Ink::Text => theme.text_primary(),
        Ink::Muted => theme.text_muted(),
        Ink::Heading => theme.text_secondary(),
        Ink::Accent => theme.accent(),
        Ink::Link => theme.primary(),
        Ink::Rule => theme.border_strong(),
        Ink::Bar => theme.primary_muted(),
        Ink::Empty => theme.border_normal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::meter::{GpuPhases, Pacing, Phases};

    /// The box as one list of lines, from the arguments `build` takes.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn built(view: &StatusView, folds: &StatusFolds, width: usize) -> Vec<Line> {
        build(view, folds, width).lines()
    }

    fn reading(avg: f32) -> Reading {
        Reading { now: avg, avg }
    }

    /// The demo's readings, roughly: a GPU-bound synth at 13 Hz of 100, forty-three Outputs.
    fn view(gpu: f32, nodes: f32) -> StatusView {
        let work = Work::ALL.map(|w| {
            reading(match w {
                Work::Nodes => nodes,
                Work::Draw => 1.6,
                Work::Inbox => 0.3,
                _ => 0.1,
            })
        });
        let worked: f32 = work.iter().map(|r| r.avg).sum();
        let tick = gpu.max(worked) + 5.0;
        let phases = Phases {
            tick: reading(tick),
            waiting: reading(tick - worked),
            asleep: reading(0.0),
            work,
            nodes: vec![
                ("lfo4".to_string(), reading(nodes * 0.4)),
                ("slimemold12".to_string(), reading(nodes * 0.2)),
            ],
            gpu: Some(GpuPhases(
                [0.2, 0.0, gpu - 4.2, 0.2, 3.0, 0.8, 0.0, gpu].map(reading),
            )),
            busy: Some(0.97),
            pacing: Pacing {
                p50: tick / 1000.0,
                p99: (tick + 8.0) / 1000.0,
                frames: 600,
            },
        };
        let outputs = (0..43)
            .map(|i: u32| OutputRow {
                node: NodeId(100 + i),
                name: format!("output{}", 100 + i),
                label: ["Kuwahara", "Wavefold", "Mandelbrot", "Slime Mold"][i as usize % 4]
                    .to_string(),
                workspace: ["Effect kernels", "Effect manglers", "Sources"][i as usize % 3]
                    .to_string(),
                resolution: (1280, 720),
                gpu_ms: (i < 42).then(|| (12.8 / (1.0 + i as f32), 14.0)),
                dropped_per_s: 0.0,
                state: if i < 42 {
                    OutputState::Rendering
                } else {
                    OutputState::NothingConnected
                },
                why: "on the workspace being looked at".to_string(),
            })
            .collect();
        StatusView {
            file: "saved 21 September".to_string(),
            file_shows: false,
            file_failed: false,
            tick: 5_120,
            frame_ms: 11.0,
            frame_avg_ms: 11.1,
            frame_worst_ms: 14.2,
            refresh_ms: 10.0,
            tick_budget_ms: 10.0,
            cpu_ms: 2.1,
            cpu_avg_ms: 2.0,
            paint_gpu: Some((3.4, 3.1, 5.2)),
            dropped_per_s: 0.0,
            uptime_s: 247.0,
            dropped: 0,
            drops: Vec::new(),
            gpu_waits: 12,
            gpu_waits_per_s: 0.0,
            drawn: 18,
            nodes: 212,
            shaders: 43,
            uniforms: 1_203,
            undo: 12,
            undo_depth: 100,
            redo: 0,
            outputs,
            outputs_drawn: 42,
            cpu_lines: vec![
                NodeLine {
                    node: NodeId(7),
                    name: "phase7".into(),
                    kind: "Phase".into(),
                    text: "0.42 cycles".into(),
                },
                NodeLine {
                    node: NodeId(9),
                    name: "audioin9".into(),
                    kind: "Audio In".into(),
                    text: "no device: the microphone is not open, so every band reads zero \
                           until one is chosen in the Main Input panel"
                        .into(),
                },
            ],
            phases: Some(phases),
            gpu: true,
            busy_of: BusyOf::Process,
        }
    }

    /// Five per cent short is the line: at it the synth keeps up, past it it is behind.
    #[test]
    fn five_per_cent_short_is_behind() {
        assert!(!short_of(95.0, 100.0));
        assert!(short_of(94.9, 100.0));
        assert!(!short_of(144.0, 60.0));
    }

    #[test]
    fn the_verdict_names_the_gpu_when_its_share_is_larger() {
        let v = view(55.5, 12.8);
        assert_eq!(verdict(&v).map(|v| v.2), Some(Bound::Gpu));
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        let held = text.lines().nth(1).unwrap();
        assert!(held.contains("held back by   GPU"), "{held}");
    }

    #[test]
    fn the_verdict_names_the_cpu_otherwise() {
        let v = view(3.0, 30.0);
        assert_eq!(verdict(&v).map(|v| v.2), Some(Bound::Cpu));
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        assert!(
            text.lines().nth(1).unwrap().contains("held back by   CPU"),
            "{text}"
        );
    }

    /// Where the draw has no whole reading but its Outputs do, as on a Mac, the GPU's figure
    /// is what the Outputs drawing cost, and it is named as that.
    #[test]
    fn with_only_the_outputs_timed_the_gpu_is_their_sum() {
        let mut v = view(55.5, 12.8);
        v.phases.as_mut().unwrap().gpu = None;
        let sum: f32 = v.outputs.iter().filter_map(drawing).sum();
        assert!(sum > 0.0);
        let (gpu, _, _) = verdict(&v).unwrap();
        assert_eq!(gpu, Some((sum, GpuFigure::Outputs)));
        let lines = built(&v, &StatusFolds::default(), 64);
        let text = plain(&lines);
        assert!(!text.contains("not measured"), "{text}");
        assert!(lines.iter().any(|l| l.tip.as_deref() == Some(OUTPUTS_ONLY)));
    }

    /// Without GL there is no GPU reading and no GPU section; the CPU bounds it.
    #[test]
    fn with_no_gpu_reading_the_cpu_is_named() {
        let mut v = view(55.5, 12.8);
        v.phases.as_mut().unwrap().gpu = None;
        v.gpu = false;
        assert_eq!(verdict(&v).map(|v| v.2), Some(Bound::Cpu));
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        assert!(!text.contains("GPU per tick"), "{text}");
    }

    /// The CPU section's rows — the waiting, every work and the sleep — add up to its total,
    /// which is the tick.
    #[test]
    fn the_cpu_rows_add_up_to_the_tick() {
        let v = view(55.5, 12.8);
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        // The rows with a label; a line that only continues the one above has none.
        let section: Vec<&str> = text
            .lines()
            .skip_while(|l| !l.contains("CPU per tick"))
            .skip(1)
            .take_while(|l| !l.starts_with('├'))
            .filter(|l| !l.chars().skip(4).take(LABEL).all(|c| c == ' '))
            .collect();
        let figure = |line: &str| -> Option<f32> {
            // The figure is the six-wide column after the bar.
            let body: String = line.chars().skip(2 + 2 + LABEL + BAR).take(6).collect();
            body.trim().parse().ok()
        };
        let sum: f32 = section.iter().filter_map(|l| figure(l)).sum();
        let tick = v.phases.as_ref().unwrap().tick.avg;
        assert_eq!(section.len(), Work::ALL.len() + 2, "{section:#?}");
        assert!(
            (sum - tick).abs() < 0.1 * section.len() as f32,
            "{sum} against {tick}"
        );
        assert!(section[0].contains("waiting on GPU"));
    }

    /// Every line is exactly as wide as the box, whatever is folded.
    #[test]
    fn every_line_is_the_boxs_width() {
        let v = view(55.5, 12.8);
        for folds in [StatusFolds::default(), StatusFolds::unfolded()] {
            for width in [MIN_WIDTH, 64, 90] {
                for line in built(&v, &folds, width) {
                    assert_eq!(cells(&line.text()), width, "{:?}", line.text());
                }
            }
        }
    }

    /// The copy is the box with every section open and every Output listed, character for
    /// character.
    #[test]
    fn the_copy_is_the_box_unfolded() {
        let v = view(55.5, 12.8);
        let copied = copy_text(&v, 64);
        assert_eq!(copied, plain(&built(&v, &StatusFolds::unfolded(), 64)));
        assert!(
            !copied.lines().any(|l| l.starts_with("├ ▸")),
            "nothing folded:\n{copied}"
        );
        for o in &v.outputs {
            assert!(copied.contains(&o.label), "{} is listed", o.name);
        }
        assert_eq!(
            copied.lines().filter(|l| l.contains("▸ go")).count(),
            v.outputs.len(),
            "every Output has its row"
        );
        let shown = plain(&built(&v, &StatusFolds::default(), 64));
        assert!(shown.contains("…35 more under"), "{shown}");
    }

    /// Each section's line count, the verdict above them as the first, keyed by title.
    fn heights(lines: &[Line]) -> Vec<(String, usize)> {
        let titles = [
            Section::Gpu,
            Section::Cpu,
            Section::Editor,
            Section::Outputs,
            Section::Nodes,
            Section::Project,
        ];
        let mut out = vec![("verdict".to_string(), 0)];
        for line in lines {
            let text = line.text();
            if text.starts_with('├') {
                let title = titles
                    .iter()
                    .find(|s| text.contains(s.title()))
                    .map_or("?", |s| s.title());
                out.push((title.to_string(), 0));
            }
            out.last_mut().unwrap().1 += 1;
        }
        out
    }

    /// The readings a live box goes through, one after another: none yet, no GPU mark yet,
    /// no process figure yet, then 0, 1, 5 and 20 CPU nodes reporting over 3 and 43 Outputs,
    /// with drops, an uneven tick, a failing Output, idle ones and figures of every width.
    fn readings() -> Vec<StatusView> {
        let mut out = Vec::new();
        let mut v = view(55.5, 12.8);
        v.phases = None;
        v.paint_gpu = None;
        out.push(v);
        let mut v = view(55.5, 12.8);
        v.phases.as_mut().unwrap().gpu = None;
        v.phases.as_mut().unwrap().busy = None;
        out.push(v);
        for outputs in [3, 43] {
            for nodes in [0, 1, 5, 20] {
                let mut v = view(if nodes == 5 { 1234.5 } else { 55.5 }, 0.5 + nodes as f32);
                v.outputs.truncate(outputs);
                v.phases.as_mut().unwrap().nodes = (0..nodes)
                    .map(|i| (format!("averylongnodename{i}"), reading(i as f32 * 3.1)))
                    .collect();
                if nodes == 20 {
                    v.drops = vec![("held", 1_234_567), ("capture", 12)];
                    v.gpu_waits_per_s = 18.0;
                    v.phases.as_mut().unwrap().pacing.p99 = 10.0;
                    v.tick = 123_456_789;
                    v.uptime_s = 7_200.0;
                    v.file = format!("rendered 3 frames to /{}", "renders/".repeat(20));
                    v.file_shows = true;
                    v.outputs[1].state =
                        OutputState::Error("0:12: 'x' undeclared\nand more".into());
                    v.outputs[2].dropped_per_s = 12.0;
                    v.paint_gpu = Some((1234.5, 0.01, 99_999.0));
                }
                if nodes == 1 {
                    // Most of them idle, keeping the figures of the last frame each drew.
                    for o in v.outputs.iter_mut().skip(1) {
                        o.state = OutputState::Idle;
                        o.why.clear();
                    }
                    v.outputs_drawn = 1;
                }
                out.push(v);
            }
        }
        // Names wider than their characters: CJK, an emoji and full-width Latin, and a node's
        // line of CJK longer than its row.
        let mut v = view(55.5, 12.8);
        for (o, name) in
            v.outputs
                .iter_mut()
                .zip(["映像ワークスペース", "東京 🎛 live", "Ｆｕｌｌ ｗｉｄｔｈ"])
        {
            o.workspace = name.to_string();
        }
        v.cpu_lines[0].text = "映像 ".repeat(30);
        out.push(v);
        out
    }

    /// **No reading moves a row.** Every section is as many lines whatever the readings are:
    /// none, 1, 5 or 20 CPU nodes reporting, 3 Outputs or 43, a GPU figure or not yet.
    #[test]
    fn every_section_keeps_its_rows_whatever_it_reads() {
        let open = StatusFolds {
            all_outputs: false,
            ..StatusFolds::unfolded()
        };
        for folds in [StatusFolds::default(), open] {
            let all: Vec<Vec<(String, usize)>> = readings()
                .iter()
                .map(|v| heights(&built(v, &folds, 64)))
                .collect();
            for (i, h) in all.iter().enumerate() {
                assert_eq!(h, &all[0], "reading {i} against the first, {folds:?}");
            }
        }
        // And the CPU nodes block is its total and five slots.
        let v = &readings()[2];
        let text = plain(&built(v, &open, 64));
        let block: Vec<&str> = text
            .lines()
            .skip_while(|l| !l.contains("CPU nodes "))
            .take(1 + TOP_NODES + 1)
            .collect();
        assert!(
            block[1..=TOP_NODES].iter().all(|l| l.contains('—')),
            "{text}"
        );
        assert!(block[TOP_NODES + 1].contains("frame job"), "{text}");
    }

    /// **The editor's GPU row** is in the Editor section wherever there is a GPU, as a
    /// row of dashes until a reading lands and the same one line after, and the section is
    /// as tall either way.
    #[test]
    fn the_editor_section_has_a_gpu_row() {
        let open = StatusFolds::unfolded();
        let editor = |v: &StatusView| {
            let text = plain(&built(v, &open, 64));
            let rows: Vec<String> = text
                .lines()
                .skip_while(|l| !l.contains(Section::Editor.title()))
                .skip(1)
                .take_while(|l| !l.starts_with('├'))
                .map(str::to_string)
                .collect();
            rows
        };
        let mut v = view(55.5, 12.8);
        let read = editor(&v);
        let row = read
            .iter()
            .find(|l| l.contains("GPU per frame"))
            .unwrap_or_else(|| panic!("no GPU row in {read:#?}"));
        assert!(
            row.contains("3.4 ms now") && row.contains("3.1 avg") && row.contains("5.2 worst"),
            "{row}"
        );
        v.paint_gpu = None;
        let unread = editor(&v);
        assert_eq!(unread.len(), read.len(), "{unread:#?}");
        let row = unread
            .iter()
            .find(|l| l.contains("GPU per frame"))
            .expect("the row is there before a reading");
        assert!(row.contains('—'), "{row}");
        v.gpu = false;
        assert!(
            editor(&v).iter().all(|l| !l.contains("GPU per frame")),
            "headless there is nothing to time"
        );
    }

    /// **The border is closed.** Every line, a content row as much as a heading, is the box's
    /// width and starts and ends on the frame.
    #[test]
    fn every_line_is_closed_at_the_boxs_width() {
        for v in readings() {
            for folds in [StatusFolds::default(), StatusFolds::unfolded()] {
                for width in [MIN_WIDTH, 64, 90] {
                    for line in built(&v, &folds, width) {
                        let text = line.text();
                        assert_eq!(cells(&text), width, "{text:?}");
                        assert!(text.starts_with(['┌', '├', '│', '└']), "{text:?}");
                        assert!(text.ends_with(['┐', '┤', '│', '┘']), "{text:?}");
                    }
                }
            }
        }
    }

    /// A status longer than its column is cut with an ellipsis, on the one line, and whole on
    /// the line's hover.
    #[test]
    fn a_long_status_is_cut_to_its_line() {
        let v = view(55.5, 12.8);
        let lines = built(&v, &StatusFolds::unfolded(), 64);
        let audio: Vec<&Line> = lines
            .iter()
            .filter(|l| l.text().contains("audioin9"))
            .collect();
        assert_eq!(audio.len(), 1, "one line");
        let text = audio[0].text();
        assert!(text.ends_with("… │"), "{text:?}");
        assert!(text.contains("no device: the microphone"), "{text:?}");
        let tip = audio[0].tip.as_ref();
        assert!(
            tip.is_some_and(|h| h.contains("chosen in the Main Input panel")),
            "{tip:?}"
        );
        let copied = copy_text(&v, 64);
        assert!(!copied.contains("Main Input panel"), "{copied}");
        // Nodes is last: its lines are the last before the bottom edge.
        let last = copied.lines().rev().nth(1).unwrap();
        assert!(last.contains("phase7"), "{copied}");
    }

    /// The waits row says what share of the last second's ticks waited for the tick before, beside the drops by cause, and keeps its one line: whole at the narrowest
    /// width while nothing has dropped.
    #[test]
    fn the_waits_row_is_a_share_of_the_ticks() {
        let mut v = view(20.0, 12.8);
        v.phases.as_mut().unwrap().tick = reading(20.0);
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        assert!(
            text.contains("GPU waits   0% of ticks · by cause: none"),
            "{text}"
        );
        let narrow = built(&v, &StatusFolds::default(), MIN_WIDTH);
        let row = narrow
            .iter()
            .find(|l| l.text().contains("GPU waits"))
            .unwrap();
        assert!(row.text().contains("by cause: none"), "{:?}", row.text());
        let rows = built(&v, &StatusFolds::default(), 64).len();
        // Fifty ticks a second, forty of which waited.
        v.gpu_waits_per_s = 40.0;
        v.drops = vec![("held", 3)];
        let lines = built(&v, &StatusFolds::default(), 64);
        let text = plain(&lines);
        assert!(
            text.contains("GPU waits  80% of ticks · by cause: held 3"),
            "{text}"
        );
        assert_eq!(lines.len(), rows, "no reading moves a row");
    }

    /// The synth is measured against the rate it is asked to tick at, not the display's: at
    /// 60 Hz on a 144 Hz panel it keeps up, and a CPU row under a sixtieth of a second is not
    /// warm.
    #[test]
    fn the_tick_is_measured_against_the_tick_rate() {
        let mut v = view(3.0, 9.0);
        v.refresh_ms = 1000.0 / 144.0;
        v.tick_budget_ms = 1000.0 / 60.0;
        v.phases.as_mut().unwrap().tick = reading(1000.0 / 60.0);
        let lines = built(&v, &StatusFolds::unfolded(), 64);
        let top = &lines[0];
        assert!(top.text().contains(" 60 Hz of 60 "), "{:?}", top.text());
        assert!(
            top.spans.iter().all(|s| s.ink != Ink::Accent),
            "keeping up is not the accent: {top:?}"
        );
        assert!(lines[1].text().contains("busiest"), "{:?}", lines[1].text());
        assert!(lines[1].text().contains("/ 17"), "{:?}", lines[1].text());
        let nodes = lines
            .iter()
            .find(|l| l.text().contains("CPU nodes "))
            .expect("the CPU nodes row");
        assert!(
            nodes.spans.iter().all(|s| s.ink != Ink::Accent),
            "9 ms is within a sixtieth: {nodes:?}"
        );
        // And past it, warm.
        v.tick_budget_ms = v.refresh_ms;
        let lines = built(&v, &StatusFolds::unfolded(), 64);
        let nodes = lines
            .iter()
            .find(|l| l.text().contains("CPU nodes "))
            .unwrap();
        assert!(
            nodes.spans.iter().any(|s| s.ink == Ink::Accent),
            "{nodes:?}"
        );
    }

    /// The rate at the top is the timed tick's, which nothing clamps: it reads a slow synth as
    /// slow, and a dash until a tick has been timed.
    #[test]
    fn the_rate_at_the_top_is_the_timed_ticks() {
        let top = |v: &StatusView| built(v, &StatusFolds::default(), 64)[0].clone();
        let mut v = view(55.5, 12.8);
        v.phases.as_mut().unwrap().tick = reading(250.0);
        let line = top(&v);
        assert!(line.text().contains("   4 Hz of 100 "), "{:?}", line.text());
        assert!(line.spans.iter().any(|s| s.ink == Ink::Accent), "{line:?}");
        v.phases = None;
        let line = top(&v);
        assert!(line.text().contains("   — Hz of 100 "), "{:?}", line.text());
        assert!(line.spans.iter().all(|s| s.ink != Ink::Accent), "{line:?}");
        assert_eq!(cells(&line.text()), 64);
    }

    /// The Outputs drawing this tick come first by cost, then the idle ones by the figure of
    /// the last frame each drew; and the count of the rest is under what the drawing ones cost.
    #[test]
    fn drawing_outputs_come_before_idle_ones() {
        let mut v = view(55.5, 12.8);
        // The ten costliest go idle, keeping their figures.
        for o in v.outputs.iter_mut().take(10) {
            o.state = OutputState::Idle;
        }
        let text = plain(&built(&v, &StatusFolds::default(), 64));
        let rows: Vec<&str> = text.lines().filter(|l| l.contains("▸ go")).collect();
        assert_eq!(rows.len(), TOP_OUTPUTS);
        assert!(
            rows.iter().all(|l| !l.contains("idle")),
            "the drawing ones fill the slots:\n{text}"
        );
        let figure = |l: &str| -> f32 {
            let ms = l.split(" ms").next().unwrap();
            ms.split_whitespace().last().unwrap().parse().unwrap()
        };
        let costs: Vec<f32> = rows.iter().map(|l| figure(l)).collect();
        assert!(costs.windows(2).all(|w| w[0] >= w[1]), "{costs:?}");
        // The ninth drawing Output leads the rest at 12.8 / 19, under a millisecond, while
        // the idle ones among them read up to 12.8.
        assert!(text.contains("…35 more under 1 ms"), "{text}");
        let order: Vec<usize> = by_cost(&v.outputs).into_iter().map(|(i, _)| i).collect();
        assert_eq!(&order[..3], &[10, 11, 12]);
        let idle_from = order.iter().position(|i| *i < 10).unwrap();
        assert_eq!(
            &order[idle_from..idle_from + 10],
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert_eq!(
            order.last(),
            Some(&42),
            "nothing measured it, so it is last"
        );
    }

    /// A span that ends exactly at the edge, with more after it, is marked with an ellipsis in
    /// its last cell and the line stays the box's width.
    #[test]
    fn a_line_full_to_the_edge_is_cut_there() {
        let mut b = Builder {
            lines: Vec::new(),
            width: 64,
        };
        let inner = b.inner();
        b.boxed(
            Line::new()
                .put("x".repeat(inner), Ink::Text)
                .put("more", Ink::Muted),
        );
        let line = &b.lines[0];
        let text = line.text();
        assert_eq!(cells(&text), 64, "{text:?}");
        assert!(text.ends_with("x… │"), "{text:?}");
        assert_eq!(line.tip.as_deref(), Some("more"));
    }

    /// One cell's letter for each ink, under the text it colors.
    fn ink_letter(ink: Ink) -> char {
        match ink {
            Ink::Text => 't',
            Ink::Muted => 'm',
            Ink::Heading => 'H',
            Ink::Accent => 'A',
            Ink::Link => 'L',
            Ink::Rule => '-',
            Ink::Bar => 'B',
            Ink::Empty => '.',
        }
    }

    /// The box as text a reviewer reads: each line, the ink of every cell under it, then
    /// every hover and every click by line number.
    fn described(lines: &[Line]) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let mut tips = String::new();
        let mut acts = String::new();
        for (n, line) in lines.iter().enumerate() {
            out.push_str(&line.text());
            out.push('\n');
            for s in &line.spans {
                out.extend(std::iter::repeat_n(ink_letter(s.ink), cells(&s.text)));
                if let Some(act) = s.act {
                    let name = s.name.as_deref().unwrap_or(&s.text);
                    let _ = writeln!(acts, "{n:>3} {act:?} {name:?}");
                }
            }
            out.push('\n');
            if let Some(tip) = &line.tip {
                let _ = writeln!(tips, "{n:>3} {tip:?}");
            }
        }
        format!("{out}hovers:\n{tips}clicks:\n{acts}")
    }

    /// **The box as it reads**, every reading folded at its narrowest and unfolded at its
    /// widest, with each cell's ink, every hover and every click: what a change to the model
    /// must leave alone or say it changed.
    #[test]
    fn the_box_reads_as_it_did() {
        use std::fmt::Write as _;
        for (i, v) in readings().iter().enumerate() {
            let mut text = String::new();
            for (name, folds, width) in [
                ("default", StatusFolds::default(), MIN_WIDTH),
                ("unfolded", StatusFolds::unfolded(), 90),
            ] {
                let _ = writeln!(text, "=== {name} at {width}");
                text.push_str(&described(&built(v, &folds, width)));
            }
            assert_eq!(
                copy_text(v, 64),
                plain(&built(v, &StatusFolds::unfolded(), 64))
            );
            insta::assert_snapshot!(format!("status_box_reading_{i}"), text);
        }
    }

    /// Where the machine counts only the whole GPU, the row and the hover say so rather than
    /// passing every app's work off as this process's.
    #[test]
    fn a_whole_gpu_figure_says_every_app() {
        let mut v = view(55.5, 12.8);
        v.busy_of = BusyOf::WholeGpu;
        v.phases.as_mut().unwrap().busy = Some(0.37);
        let lines = built(&v, &StatusFolds::unfolded(), MIN_WIDTH);
        let text = plain(&lines);
        assert!(text.contains("whole GPU        37 % · every app"), "{text}");
        assert!(!text.contains("whole process"), "{text}");
        assert!(
            lines
                .iter()
                .any(|l| l.tip.as_deref() == Some(WHOLE_GPU_ABOUT))
        );
    }

    #[test]
    fn a_bar_ends_in_an_eighth() {
        assert_eq!(bar(5.0, 10.0, 4, true), ("██".into(), "░░".into()));
        assert_eq!(bar(5.4, 10.0, 4, false), ("██▏".into(), String::new()));
        assert_eq!(bar(20.0, 10.0, 4, true), ("████".into(), String::new()));
        assert_eq!(bar(0.0, 0.0, 3, true), (String::new(), "░░░".into()));
    }

    #[test]
    fn uptime_reads_as_a_clock() {
        assert_eq!(uptime(28.6), "28 s");
        assert_eq!(uptime(247.0), "4:07");
        assert_eq!(uptime(3735.0), "1:02:15");
    }

    #[test]
    fn a_name_is_cut_to_its_column() {
        assert_eq!(cut("output39", 8), "output39");
        assert_eq!(cut("edgedetection4", 8), "edgedet…");
        assert_eq!(cut("映像ワークスペース", 8), "映像ワ…");
        assert_eq!(cut("映像ワークスペース", 9), "映像ワー…");
        assert_eq!(cells(&cut("a映像ワークスペース", 8)), 8);
    }

    /// **A wide character is two cells.** A workspace named in CJK keeps every line of the box
    /// its width, counted as a terminal counts it, with its right edge closed.
    #[test]
    fn a_wide_workspace_name_keeps_the_frame_closed() {
        assert_eq!(cells("映像"), 4);
        assert_eq!(cells("e\u{301}"), 1, "a combining accent takes no cell");
        let wide = readings().pop().unwrap();
        for folds in [StatusFolds::default(), StatusFolds::unfolded()] {
            for width in [MIN_WIDTH, 64, 90] {
                for line in built(&wide, &folds, width) {
                    let text = line.text();
                    assert_eq!(cells(&text), width, "{text:?}");
                    assert!(text.ends_with(['┐', '┤', '│', '┘']), "{text:?}");
                }
            }
        }
        let text = plain(&built(&wide, &StatusFolds::default(), 64));
        assert!(text.contains("映像ワークスペ… "), "{text}");
    }
}
