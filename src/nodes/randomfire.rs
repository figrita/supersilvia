// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fires at a random interval set by `temperature`, the Master Gear's stochastic sibling:
//! every firing inside a frame is emitted, stamped with where in the frame it fell.
//!
//! silvia's `fire` output is a bare `'down'` with no paired `'up'` — a pulse, which this
//! graph's event half does not have. **`gate` is invented here** to close it: each cycle is
//! closed, waiting out a fresh random delay, then open for `gate` of the delay that just
//! ended, then closed again — the gate is a fraction of the interval it follows, not one
//! a Master Gear carves out of a fixed interval, since here every interval is a fresh roll.
//! The delay formula, its 0.05s floor and 5s ceiling, and the ±50% jitter are silvia's own
//! `_calculateFireDelay`. `trigger` is this graph's own name for the one output a clock-like
//! node fires — the gears and `clockdivider` all call theirs that — in place of silvia's
//! `fire`.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber};
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Gate, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
};

pub static DEF: NodeDef = NodeDef {
    slug: "randomfire",
    category: Category::Control,
    icon: "🔥",
    label: "Random Fire",
    tooltip: "Fires action events randomly based on Temperature. Higher temperature = more frequent firing.",
    inputs: &[
        InputDef {
            key: "start",
            label: "Start/Stop",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "temperature",
            label: "Temperature",
            ty: UniformNumber,
            control: Control::num_log(1.0, 0.01, 10.0, 0.01, "°"),
        },
        InputDef {
            key: "gate",
            label: "Gate",
            ty: UniformNumber,
            control: Control::num(0.5, 0.01, 1.0, 0.01, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "trigger",
        label: "Trigger",
        ty: Action,
        kind: OutputKind::Action,
        ..OutputDef::EMPTY
    }],
    // The running state, which Start/Stop leaves behind and no throb can show: a press is an
    // event and this is what the press left the node as. The firing itself is the port's, and
    // the port throbs — see `ui::CanvasState::fires_at`.
    regions: &[crate::nodes::Region::Status],
    cpu: Some(CpuDef {
        create: || Box::new(RandomFire::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The most events one frame may produce, however hot the temperature — a gear's own
/// bound, for the same reason: a `tick` never waits and never allocates without one.
const MAX_EVENTS_PER_FRAME: u32 = 64;

struct RandomFire {
    // silvia's `isRunning`, which starts true.
    running: bool,
    start_stop: Gate,
    out: Gate,
    rng: Rng,
    /// Seconds until the next transition — a down while the gate is closed, an up while it
    /// is open. `None` until the first tick, which draws one rather than firing at once.
    countdown: Option<f32>,
}

impl Default for RandomFire {
    fn default() -> Self {
        Self {
            running: true,
            start_stop: Gate::default(),
            out: Gate::default(),
            rng: Rng::default(),
            countdown: None,
        }
    }
}

impl RandomFire {
    /// silvia's `_calculateFireDelay`: higher temperature, shorter delay, a ±50% jitter so
    /// firings do not fall on a metronome.
    fn delay(&mut self, temperature: f32) -> f32 {
        let base = (1.0 / temperature.max(0.01)).clamp(0.05, 5.0);
        base * (0.5 + self.rng.next_f32())
    }
}

impl CpuNode for RandomFire {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// silvia's own pill, in its own words and its own two icons: a flame while it is
    /// running and a snowflake while it is not.
    fn status(&self) -> Option<String> {
        Some(if self.running {
            "\u{1f525} Active".to_string()
        } else {
            "\u{2744} Stopped".to_string()
        })
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "{} {}",
            if self.running { "running" } else { "stopped" },
            if self.out.is_down() {
                "open"
            } else {
                "waiting"
            },
        ))
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);

        if ctx.downs(id, "start", &mut self.start_stop) % 2 == 1 {
            self.running = !self.running;
        }
        if !self.running {
            // A stopped source must not leave a receiver holding a gate it will never close.
            if let Some(edge) = self.out.set(false) {
                ctx.fire_at(id, "trigger", edge, 0.0);
            }
            // So the next run starts with a fresh roll rather than the delay a stop cut off.
            self.countdown = None;
            return;
        }

        let temperature = ctx.input(id, "temperature");
        let gate = ctx.input(id, "gate").clamp(0.01, 1.0);

        let mut remaining = self.countdown.unwrap_or_else(|| self.delay(temperature));
        let mut at = 0.0f32;
        let mut fired = 0u32;
        while remaining <= ctx.dt - at && fired < MAX_EVENTS_PER_FRAME {
            at += remaining;
            let elapsed = remaining;
            remaining = if self.out.is_down() {
                if let Some(edge) = self.out.set(false) {
                    ctx.fire_at(id, "trigger", edge, at);
                }
                self.delay(temperature)
            } else {
                if let Some(edge) = self.out.set(true) {
                    ctx.fire_at(id, "trigger", edge, at);
                }
                gate * elapsed
            };
            fired += 1;
        }
        self.countdown = Some(remaining - (ctx.dt - at));
    }
}
