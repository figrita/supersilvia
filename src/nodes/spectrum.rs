// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a spectrum list becomes on the way to a picture: **Bands** gathers it into as many
//! bands as a patch wants, and **Lag** gives each item a rise and a fall.
//!
//! Every audio source publishes its Spectrum as a number list, the analyzer's 128 log-spaced
//! buckets from 20 Hz to 20 kHz as they are this frame (`audio_ports::publish_spectrum`). A
//! spectrum as a field had to hold every fall speed and every band width in one texture,
//! because a field reader has no memory; a list is read on the CPU, so the count is Bands' and
//! the memory is Lag's, each a node on the canvas with its numbers in sight
//! ([media.md](../../docs/media.md#the-spectrum)).
//!
//! **Bands averages, so no bucket is skipped**: a band wider than a bucket is the mean of the
//! buckets under it, the ones its edges cut weighed by how much of them it holds. A band
//! narrower than a bucket reads a window one bucket wide round its middle, which is the
//! spectrum interpolated between bucket centers, so a band is never empty and sixteen bands
//! over the bottom octave rise and fall smoothly rather than in steps.
//!
//! **Lag is Slew's list form**, easing only: an item covers `1 − e^(−dt / Rise)` of the way up
//! each tick, or Fall's on the way down, so Rise and Fall are time constants in seconds and a
//! step is 63% of the way there after one of them. It steps on `dt`, so it holds still while
//! paused and across a jump. **Its state is per item, by index**: items a list keeps through a
//! change of length keep their state, new ones start at their input, and the rest are
//! forgotten.

use crate::graph::NodeId;
use crate::graph::PortType::{ListNumber, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OutputDef, OutputKind, TickContext,
    list::CAP,
};

/// The ends of a spectrum list's range, in hertz: its first bucket starts at one and its last
/// ends at the other, log-spaced between.
const LO_HZ: f64 = crate::audio::SCOPE_LO_HZ as f64;
const HI_HZ: f64 = crate::audio::SCOPE_HI_HZ as f64;

pub static BANDS: NodeDef = NodeDef {
    slug: "bands",
    category: Category::List,
    icon: "📊",
    label: "Bands",
    tooltip: "A Spectrum gathered into Count bands, Low to High, each as wide as the last on a \
              musical scale: every band the average of the spectrum under it. Cable in an audio \
              source's Spectrum.",
    inputs: &[
        InputDef {
            key: "spectrum",
            label: "Spectrum",
            ty: ListNumber,
            control: Control::None,
        },
        InputDef {
            key: "low",
            label: "Low",
            ty: UniformNumber,
            control: Control::num_log(20.0, 20.0, 20_000.0, 1.0, "Hz"),
        },
        InputDef {
            key: "high",
            label: "High",
            ty: UniformNumber,
            control: Control::num_log(16_000.0, 20.0, 20_000.0, 1.0, "Hz"),
        },
        InputDef {
            key: "count",
            label: "Count",
            ty: UniformNumber,
            control: Control::num(16.0, 0.0, CAP as f32, 1.0, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "bands",
        label: "Bands",
        ty: ListNumber,
        kind: OutputKind::List,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Bands::default()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// Where `hz` falls on a spectrum of `n` buckets, in buckets from the first one's start: the
/// buckets are log-spaced from [`LO_HZ`] to [`HI_HZ`], and a frequency outside them is held
/// to their ends.
fn bucket_at(hz: f64, n: usize) -> f64 {
    let hz = if hz.is_finite() {
        hz.clamp(LO_HZ, HI_HZ)
    } else {
        LO_HZ
    };
    n as f64 * (hz / LO_HZ).ln() / (HI_HZ / LO_HZ).ln()
}

/// `count` bands of `spectrum` between `low` and `high` hertz into `out`, `sum` a buffer for
/// the running total. A count is rounded and held to `0..=CAP`; an empty spectrum is no bands.
pub fn bands_into(
    spectrum: &[f64],
    low: f64,
    high: f64,
    count: f64,
    sum: &mut Vec<f64>,
    out: &mut Vec<f64>,
) {
    out.clear();
    let n = spectrum.len();
    let count = if count.is_finite() {
        count.round().clamp(0.0, CAP as f64) as usize
    } else {
        0
    };
    if n == 0 || count == 0 {
        return;
    }
    sum.clear();
    sum.push(0.0);
    let mut total = 0.0;
    for level in spectrum {
        total += level;
        sum.push(total);
    }
    // The spectrum's integral from its start to `x` buckets in, each bucket a step.
    let below = |x: f64| {
        let i = (x as usize).min(n - 1);
        sum[i] + (x - i as f64) * spectrum[i]
    };
    let (from, to) = (bucket_at(low, n), bucket_at(high, n));
    let step = (to - from) / count as f64;
    out.extend((0..count).map(|k| {
        let (a, b) = (from + step * k as f64, from + step * (k + 1) as f64);
        let middle = f64::midpoint(a, b);
        let half = 0.5 * (b - a).abs().max(1.0);
        let (a, b) = ((middle - half).max(0.0), (middle + half).min(n as f64));
        (below(b) - below(a)) / (b - a)
    }));
}

/// [`bands_into`], into a list of its own.
pub fn bands(spectrum: &[f64], low: f64, high: f64, count: f64) -> Vec<f64> {
    let mut out = Vec::new();
    bands_into(spectrum, low, high, count, &mut Vec::new(), &mut out);
    out
}

/// The buffers a tick reuses, so the same count allocates nothing.
#[derive(Default)]
struct Bands {
    sum: Vec<f64>,
    out: Vec<f64>,
}

impl CpuNode for Bands {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let (low, high, count) = (
            ctx.input(id, "low"),
            ctx.input(id, "high"),
            ctx.input(id, "count"),
        );
        bands_into(
            &ctx.list(id, "spectrum"),
            low,
            high,
            count,
            &mut self.sum,
            &mut self.out,
        );
        ctx.publish_list(id, "bands", self.out.iter().copied());
    }
}

pub static LAG: NodeDef = NodeDef {
    slug: "lag",
    category: Category::List,
    icon: "🐌",
    label: "Lag",
    tooltip: "Each item of a List eased toward where it is going: Rise is how slowly an item \
              climbs and Fall how slowly it sinks back, in seconds, so a spectrum's bars can leap \
              up and drift down. An item new to the list starts where it is.",
    inputs: &[
        InputDef {
            key: "list",
            label: "List",
            ty: ListNumber,
            control: Control::None,
        },
        InputDef {
            key: "rise",
            label: "Rise",
            ty: UniformNumber,
            control: Control::num(0.02, 0.0, 10.0, 0.01, "s"),
        },
        InputDef {
            key: "fall",
            label: "Fall",
            ty: UniformNumber,
            control: Control::num(0.2, 0.0, 10.0, 0.01, "s"),
        },
    ],
    outputs: &[OutputDef {
        key: "output",
        label: "Output",
        ty: ListNumber,
        kind: OutputKind::List,
        ..OutputDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Lag::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// One step of `items` toward `input` over `dt` seconds, rising with the time constant `rise`
/// and falling with `fall`; a time constant of zero or less lands at once. `items` takes
/// `input`'s length: the items both have are eased, the ones only `input` has start at it.
pub fn lag(items: &mut Vec<f64>, input: &[f64], rise: f64, fall: f64, dt: f64) {
    items.truncate(input.len());
    for (item, &target) in items.iter_mut().zip(input) {
        let lag = if target > *item { rise } else { fall };
        *item = if lag > 0.0 {
            target + (*item - target) * (-dt.max(0.0) / lag).exp()
        } else {
            target
        };
    }
    let kept = items.len();
    items.extend_from_slice(&input[kept..]);
}

/// Where each item is, by index.
#[derive(Default)]
struct Lag {
    items: Vec<f64>,
}

impl CpuNode for Lag {
    fn reset(&mut self) {
        self.items.clear();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let (rise, fall) = (ctx.input(id, "rise"), ctx.input(id, "fall"));
        let dt = f64::from(ctx.dt);
        lag(&mut self.items, &ctx.list(id, "list"), rise, fall, dt);
        ctx.publish_list(id, "output", self.items.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0e-9)
    }

    /// The frequency `x` buckets into a spectrum of `n`.
    fn hz_at(x: f64, n: usize) -> f64 {
        LO_HZ * (HI_HZ / LO_HZ).powf(x / n as f64)
    }

    #[test]
    fn a_flat_spectrum_is_flat_bands() {
        let flat = [0.5; 128];
        let out = bands(&flat, 20.0, 16_000.0, 16.0);
        assert_eq!(out.len(), 16);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1.0e-12), "{out:?}");
    }

    /// The edges are log-spaced from Low to High: the whole range split in two is the
    /// spectrum's two halves, a band across them is their mean, and Low to the middle alone is
    /// the lower half.
    #[test]
    fn bands_split_the_range_on_a_log_scale_and_average_what_they_hold() {
        let mut step = [0.0; 128];
        step[..64].fill(1.0);
        assert!(close(&bands(&step, 20.0, 20_000.0, 2.0), &[1.0, 0.0]));
        assert!(close(
            &bands(&step, 20.0, 20_000.0, 4.0),
            &[1.0, 1.0, 0.0, 0.0]
        ));
        assert!(close(&bands(&step, 20.0, 20_000.0, 1.0), &[0.5]));
        let middle = hz_at(64.0, 128);
        assert!(close(&bands(&step, 20.0, middle, 1.0), &[1.0]));
        // A quarter of a band past the step: three of its four quarters are loud.
        let out = bands(&step, hz_at(56.0, 128), hz_at(72.0, 128), 2.0);
        assert!(close(&out, &[1.0, 0.0]), "{out:?}");
        let out = bands(&step, hz_at(52.0, 128), hz_at(68.0, 128), 1.0);
        assert!(close(&out, &[0.75]), "{out:?}");
    }

    /// A band narrower than a bucket reads the spectrum interpolated between bucket centers,
    /// never nothing: on a ramp, the value at its middle.
    #[test]
    fn a_band_narrower_than_a_bucket_reads_the_spectrum_interpolated() {
        let ramp: Vec<f64> = (0..128).map(f64::from).collect();
        let out = bands(&ramp, hz_at(10.0, 128), hz_at(12.0, 128), 8.0);
        let want: Vec<f64> = (0..8)
            .map(|k| 10.0 + 0.25 * (f64::from(k) + 0.5) - 0.5)
            .collect();
        assert!(close(&out, &want), "{out:?} against {want:?}");
        // At the very bottom the window is cut by the spectrum's start and reads the first
        // bucket alone.
        let out = bands(&ramp, 20.0, 20.0, 3.0);
        assert!(close(&out, &[0.0, 0.0, 0.0]), "{out:?}");
        let many = bands(&ramp, 20.0, 20_000.0, CAP as f64);
        assert_eq!(many.len(), CAP);
        assert!(many.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn no_count_or_no_spectrum_is_no_bands_and_reversed_ends_run_high_to_low() {
        assert!(bands(&[1.0; 128], 20.0, 16_000.0, 0.0).is_empty());
        assert!(bands(&[], 20.0, 16_000.0, 16.0).is_empty());
        assert!(bands(&[1.0; 128], 20.0, 16_000.0, f64::NAN).is_empty());
        let mut step = [0.0; 128];
        step[..64].fill(1.0);
        assert!(close(&bands(&step, 20_000.0, 20.0, 2.0), &[0.0, 1.0]));
        // Ends outside the spectrum's range are held to it.
        assert!(close(&bands(&step, 1.0, 1.0e9, 2.0), &[1.0, 0.0]));
        // A single number is a spectrum of one bucket over the whole range.
        assert!(close(&bands(&[0.3], 20.0, 16_000.0, 3.0), &[0.3, 0.3, 0.3]));
    }

    /// Rise and Fall are time constants: one of them covers 63% of a step, each its own way.
    #[test]
    fn lag_rises_and_falls_at_its_own_rates() {
        let mut items = vec![0.0, 1.0];
        lag(&mut items, &[1.0, 0.0], 0.1, 1.0, 0.1);
        let up = 1.0 - (-1.0f64).exp();
        let down = (-0.1f64).exp();
        assert!(close(&items, &[up, down]), "{items:?}");
        // Nine more tenths of a second: a fall of one second is 63% done after one second.
        for _ in 0..9 {
            lag(&mut items, &[1.0, 0.0], 0.1, 1.0, 0.1);
        }
        assert!((items[1] - (-1.0f64).exp()).abs() < 1.0e-9, "{items:?}");
        // A lag of zero lands at once, and no time is no step.
        lag(&mut items, &[5.0, -5.0], 0.0, 0.0, 0.1);
        assert!(close(&items, &[5.0, -5.0]));
        lag(&mut items, &[0.0, 0.0], 0.1, 0.1, 0.0);
        assert!(close(&items, &[5.0, -5.0]));
    }

    /// Items the list keeps keep their state; new ones start at their input; dropped ones are
    /// forgotten, so one that comes back starts at its input too.
    #[test]
    fn lag_keeps_the_items_a_list_keeps_through_a_change_of_length() {
        let mut items = Vec::new();
        lag(&mut items, &[1.0, 1.0, 1.0], 1.0, 1.0, 0.1);
        assert!(
            close(&items, &[1.0, 1.0, 1.0]),
            "the first tick starts at the input"
        );
        lag(&mut items, &[0.0, 0.0, 0.0, 0.0, 7.0], 1.0, 1.0, 0.1);
        let held = (-0.1f64).exp();
        assert!(close(&items, &[held, held, held, 0.0, 7.0]), "{items:?}");
        lag(&mut items, &[0.0], 1.0, 1.0, 0.1);
        assert!(close(&items, &[held * held]), "{items:?}");
        lag(&mut items, &[0.0, 3.0], 1.0, 1.0, 0.0);
        assert!(close(&items, &[held * held, 3.0]), "{items:?}");
        lag(&mut items, &[], 1.0, 1.0, 0.1);
        assert!(items.is_empty());
    }
}
