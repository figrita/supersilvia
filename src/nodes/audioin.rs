// SPDX-License-Identifier: AGPL-3.0-or-later

//! The microphone: level, peak, three bands, and an event when any of them crosses a level.
//!
//! The device is opened on the node's first tick and closed when the node goes. The analysis
//! happens on the audio thread (`audio/`); this tick only reads the newest result, applies
//! the gain and publishes. Smoothing what comes out is `slew`'s job.
//!
//! **The thresholds are crossed where the samples are.** A block is a few milliseconds and a
//! frame is sixteen, so the audio thread finds a crossing at the sample it happened on and
//! this tick places it inside the frame by how long ago that was. silvia checks its
//! thresholds in an animation frame and cannot do better than the frame that noticed.
//!
//! Its outputs are the same bundle `video` publishes, deliberately: a graph built against the
//! microphone plays against a file unchanged.

use crate::audio::{self, Analysis, Capture, bands};
use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "audioin",
    category: Category::Source,
    icon: "🎤",
    label: "Audio In",
    tooltip: "The default input device, analyzed: level, peak, three bands, and an event \
              each time a band crosses the level you set.",
    inputs: crate::audio_inputs![
        InputDef {
            key: "gain",
            label: "Gain",
            ty: UniformNumber,
            control: Control::num(1.0, 0.0, 20.0, 0.01, "x"),
        },
        // The microphone alone thresholds on its own loudness: a file has a mix, a room has
        // a level.
        InputDef {
            key: "volumeLevel",
            label: "Volume At",
            ty: UniformNumber,
            control: Control::num(1.0, 0.0, 1.0, 0.01, ""),
        },
    ],
    hidden: crate::nodes::audio_ports::TUNING,
    outputs: crate::audio_outputs![
        OutputDef {
            key: "level",
            label: "Level",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "peak",
            label: "Peak",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "volumeEvent",
            label: "Volume Event",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        crate::nodes::audio_ports::SHOW_UNIFORMS,
        crate::nodes::audio_ports::SHOW_EVENTS,
        crate::nodes::audio_ports::SHOW_SCOPE,
    ],
    regions: &[crate::nodes::Region::Scope],
    cpu: Some(CpuDef {
        create: || Box::new(AudioIn::new()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// The five published numbers, in output order.
const OUTPUTS: [&str; 5] = ["level", "peak", "bass", "mid", "high"];

/// The action each threshold fires on, indexed the way `audio::Crossing::band` is: the three
/// bands, then volume.
const EVENT_PORTS: [&str; bands::BANDS + 1] = ["bassEvent", "midEvent", "highEvent", "volumeEvent"];

/// The control that sets each threshold, in the same order.
const LEVEL_INPUTS: [&str; bands::BANDS + 1] =
    ["bassLevel", "midLevel", "highLevel", "volumeLevel"];

struct AudioIn {
    capture: Option<Capture>,
    error: Option<String>,
    /// What the last tick read, for the overlay.
    last: Analysis,
    /// How many crossings have been delivered, so none is delivered twice.
    seen: u64,
    /// What the tick last asked the audio thread for, so the scope draws the same tuning the
    /// analysis used rather than reading the controls a second time.
    config: [bands::BandConfig; bands::BANDS],
    levels: [f32; bands::BANDS],
    /// The oscilloscope's frames, reused from tick to tick.
    scope: crate::nodes::audio_ports::Waveform,
}

impl AudioIn {
    /// Turn the audio thread's crossings into events placed inside this frame.
    ///
    /// A crossing carries the sample it happened on, and the analysis carries the sample it
    /// ended on and the instant it was published. So how long ago the crossing was is known:
    /// the samples between it and the end of the block, plus however long the block has been
    /// sitting in the triple buffer. `dt` minus that is where in this frame it belongs.
    ///
    /// It is placed by age rather than by a shared clock because there is no shared clock: a
    /// sound card counts in its own samples and the compositor counts in frames. Age is what
    /// both can agree on, and it is right to within the jitter of one callback.
    fn fire(&mut self, id: NodeId, ctx: &mut TickContext<'_>, analysis: &Analysis) {
        let (edges, dropped) = analysis.crossings.since(self.seen);
        let rate = self
            .capture
            .as_ref()
            .map_or(audio::track::RATE, |c| c.sample_rate) as f32;
        let stale = analysis.age().map_or(0.0, |a| a.as_secs_f32());
        for crossing in edges {
            let Some(port) = EVENT_PORTS.get(crossing.band as usize) else {
                continue;
            };
            let behind = analysis.samples.saturating_sub(crossing.at) as f32 / rate.max(1.0);
            let at = (ctx.dt - (stale + behind)).clamp(0.0, ctx.dt);
            let edge = if crossing.down { Edge::Down } else { Edge::Up };
            ctx.fire_at(id, port, edge, at);
        }
        self.seen = analysis.crossings.total;
        if dropped > 0 {
            log::debug!("audioin{id}: {dropped} crossings arrived faster than they were read");
        }
    }

    fn new() -> Self {
        let (capture, error) = match Capture::open() {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            capture,
            error,
            last: Analysis::default(),
            seen: 0,
            config: bands::DEFAULT,
            levels: [1.0; bands::BANDS],
            scope: crate::nodes::audio_ports::Waveform::default(),
        }
    }
}

impl CpuNode for AudioIn {
    fn reset(&mut self) {
        self.last = Analysis::default();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // The thresholds reach the audio thread before the analysis is read back, so a level
        // moved this frame is the level the next block is measured against.
        let levels: [f32; bands::BANDS + 1] =
            std::array::from_fn(|i| ctx.input(id, LEVEL_INPUTS[i]) as f32);
        self.config = crate::nodes::audio_ports::config(id, ctx);
        self.levels = std::array::from_fn(|b| levels[b]);
        let monitor = ctx.input(id, "monitor") as f32;
        if let Some(capture) = self.capture.as_mut() {
            capture.set_thresholds(levels);
            capture.set_config(self.config);
            capture.set_monitor(monitor);
        }

        let analysis = self
            .capture
            .as_mut()
            .map(Capture::latest)
            .unwrap_or_default();
        self.last = analysis;
        self.fire(id, ctx, &analysis);
        let gain = ctx.input(id, "gain");
        let raw = [
            analysis.rms,
            analysis.peak,
            analysis.bands[0],
            analysis.bands[1],
            analysis.bands[2],
        ];
        ctx.publish_frame(id, "oscilloscope", self.scope.frame(&analysis.waveform));
        for (i, key) in OUTPUTS.iter().enumerate() {
            ctx.publish(id, key, f64::from(raw[i]) * gain);
        }
    }

    fn scope(&self) -> Option<audio::Scope> {
        let capture = self.capture.as_ref()?;
        Some(audio::Scope::new(
            &self.last,
            self.config,
            self.levels,
            capture.sample_rate as f32,
        ))
    }

    fn error(&self) -> Option<String> {
        self.error.clone().or_else(|| {
            audio::Monitor::shared()
                .error()
                .map(|e| format!("monitor: {e}"))
        })
    }

    fn debug(&self) -> Option<String> {
        let capture = self.capture.as_ref()?;
        let age = self.last.age().map_or("no audio yet".to_string(), |a| {
            format!("audio age {:.1} ms", a.as_secs_f32() * 1000.0)
        });
        Some(format!(
            "{age} | {} @ {} Hz x{}",
            capture.device, capture.sample_rate, capture.channels
        ))
    }
}
