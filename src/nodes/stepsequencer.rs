// SPDX-License-Identifier: AGPL-3.0-or-later

//! silvia's Step Sequencer: four lanes of sixteen cells a hand clicks on and off, and a
//! playhead that walks across them as its Time says, every lit cell opening its lane for the
//! gate length.
//!
//! **The pattern is one of the node's own values**, a `ValueKind::Cells` — silvia's
//! `values.stepStates`, four arrays of sixteen booleans, written as four strings of `x` and
//! `.` — so it is saved with the patch and a click on a cell is one `SetValue`, one step back.
//! Where the playhead is, is a function of its Time, and it is still at rest, as silvia's is.
//!
//! **The clock is `nodes::sequencer`'s**, the one `euclideanrhythm` runs behind: Time and Offset
//! in bars — a Master Gear a bar long cabled into Time is the tempo, with its Hold and Reset —
//! Step as an action row, Gate as a knob with a port where silvia draws an s-number in the
//! body, and four lanes out. The grid is
//! [`Region::Grid`](super::Region::Grid), drawn by `widgets::steps` with the cells Euclidean
//! Rhythm's figure is drawn in, under silvia's Clear.

use crate::graph::{NodeId, Value};
use crate::nodes::sequencer::{self, LANES, Transport};
use crate::nodes::{Category, CpuDef, CpuNode, NodeDef, Region, TickContext, ValueDef, ValueKind};

/// The value the pattern is kept under.
pub const PATTERN: &str = "pattern";

/// silvia's sixteen steps a lane.
pub const STEPS: usize = 16;

pub static DEF: NodeDef = NodeDef {
    slug: "stepsequencer",
    category: Category::Control,
    icon: "🎹",
    label: "Step Sequencer",
    tooltip: "Multi-step sequencer with programmable patterns. Click a cell to light it; a Master Gear a bar long cabled into Time walks the playhead across, a bar a cycle.",
    inputs: sequencer::INPUTS,
    ambient: Some(sequencer::AMBIENT),
    options: &[crate::nodes::SHOW_TIME],
    row_headings: &[crate::nodes::SHOW_TIME.key],
    outputs: sequencer::OUTPUTS,
    values: &[ValueDef {
        key: PATTERN,
        label: "Pattern",
        kind: ValueKind::Cells {
            lanes: LANES as u8,
            steps: STEPS as u8,
        },
    }],
    regions: &[Region::Grid],
    cpu: Some(CpuDef {
        create: || Box::new(StepSequencer::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The pattern as one bitmask a lane, bit `i` set where step `i` is lit.
pub fn masks(pattern: Option<&Value>) -> [u32; LANES] {
    std::array::from_fn(|lane| {
        (0..STEPS)
            .filter(|&step| pattern.is_some_and(|p| p.lit(lane, step)))
            .fold(0, |mask, step| mask | 1 << step)
    })
}

#[derive(Default)]
struct StepSequencer {
    transport: Transport,
}

impl CpuNode for StepSequencer {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(self.transport.debug())
    }

    /// Which column the grid lights: the absolute step, which the grid wraps to its sixteen.
    fn playhead(&self) -> Option<f32> {
        self.transport.playhead(STEPS as i64)
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let masks = masks(ctx.value(id, PATTERN));
        self.transport.tick(id, ctx, |lane, step| {
            masks[lane] >> step.rem_euclid(STEPS as i64) & 1 == 1
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_reads_its_own_string_and_nothing_past_it() {
        let pattern = Value::Cells(vec!["x...x...x...x...".into(), "..x".into(), "?x-X".into()]);
        let m = masks(Some(&pattern));
        assert_eq!(m[0], 0b0001_0001_0001_0001, "four on the floor");
        assert_eq!(m[1], 0b100, "a short lane is unlit past its end");
        assert_eq!(m[2], 0b10, "only `x` is lit");
        assert_eq!(m[3], 0, "a lane the value does not have is unlit");
        assert_eq!(masks(None), [0; LANES], "a new node's grid is empty");
    }

    #[test]
    fn a_grid_written_reads_back_the_same() {
        let written = Value::grid(LANES, STEPS, |lane, step| (lane + step) % 5 == 0);
        for lane in 0..LANES {
            for step in 0..STEPS {
                assert_eq!(written.lit(lane, step), (lane + step) % 5 == 0);
            }
        }
        assert_eq!(
            written.cells().map(|c| c[0].as_str()),
            Some("x....x....x....x")
        );
    }
}
