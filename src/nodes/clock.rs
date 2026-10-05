// SPDX-License-Identifier: AGPL-3.0-or-later

//! The wall clock: what time it actually is, as two numbers.
//!
//! Everything else that moves here is measured from when the patch started. This reads the
//! world instead, which makes it the one node an offline render cannot make deterministic —
//! a render stepped to frame 900 still gets whatever the system clock says at the moment it
//! is asked. That is what `live` on its `CpuDef` declares, and it is the whole of the
//! departure.
//!
//! silvia's arithmetic, kept: seconds carry their fraction, the twelve-hour face runs 0 to 11
//! and the week starts on Sunday. Its one menu and its two outputs are kept too — the raw
//! reading and the same reading as a 0 to 1 fraction, so a sweeping hand and a slow drift
//! come off one node.
//!
//! **Local time is an offset knob rather than the system's zone.** `libc::localtime_r` is the
//! honest clock and `proposals/clock.md` preferred it; it is not a declared dependency here
//! and it is an `unsafe` call outside `render/`, which are two rules this node does not
//! bend. So the node reads UTC and adds an `offset` in hours, which is a knob like any other
//! — cabled, saved, and automatable. See docs/decisions.md.

use crate::graph::PortType::UniformNumber;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};
use std::time::{SystemTime, UNIX_EPOCH};

pub static DEF: NodeDef = NodeDef {
    slug: "clock",
    category: Category::Control,
    icon: "🕐",
    label: "Clock",
    tooltip: "The time of day, as a number: seconds, minutes, hours, or how far through the day, the week or the year it is.",
    inputs: &[InputDef {
        key: "offset",
        label: "Zone",
        ty: UniformNumber,
        // Quarter hours, because a zone can be one: Kathmandu is +5:45, so the step is the
        // finest any of them needs.
        control: Control::num(0.0, -14.0, 14.0, 0.25, "h"),
    }],
    outputs: &[
        OutputDef {
            key: "value",
            label: "Value",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "normalized",
            label: "Normalized",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
    ],
    options: &[OptionDef {
        key: "mode",
        // Named for what it picks rather than silvia's own "Output", which on this node is
        // also what both of its ports are called.
        label: "Mode",
        default: "seconds",
        choices: &MODES,
        // Read by `tick`. Both outputs are uniform numbers, so this node emits no WGSL.
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    cpu: Some(CpuDef {
        create: || Box::new(Clock),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// silvia's eight, in silvia's order.
const MODES: [(&str, &str); 8] = [
    ("seconds", "Seconds (0-59)"),
    ("minutes", "Minutes (0-59)"),
    ("hours", "Hours (0-23)"),
    ("hours12", "Hours 12h (0-11)"),
    ("daySeconds", "Seconds of Day"),
    ("dayProgress", "Day Progress (0-1)"),
    ("weekProgress", "Week Progress (0-1)"),
    ("yearProgress", "Year Progress (0-1)"),
];

const DAY: f64 = 86_400.0;

/// Nothing: the reading is the world's, taken fresh each tick.
struct Clock;

/// How many seconds since the epoch, plus the offset in hours. Non-monotonic by nature — a
/// clock set backwards sets this backwards — which is what a wall clock is.
fn now(offset_hours: f64) -> f64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    since + offset_hours * 3600.0
}

/// The civil date a day number since the epoch falls on, as `(year, days into that year,
/// days in that year)`.
///
/// Howard Hinnant's `civil_from_days`, shifted to an era starting on 1 March so that a leap
/// day is the last day of a year rather than a day in the middle of one.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    // Back to a year starting in January: March is month 0 in that era.
    let mp = (5 * doy + 2) / 153;
    let year = if mp >= 10 { y + 1 } else { y };
    let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let length = if leap(year) { 366 } else { 365 };
    // 1 January of `year` as a day number, from the same map run backwards is more code than
    // walking back from here: the day of the year is what is wanted, and `doy` counts from
    // March.
    let from_march = if mp >= 10 {
        doy - 306
    } else {
        doy + 59 + i64::from(leap(y))
    };
    (year, from_march, length)
}

/// What each mode reads, and what its 0 to 1 form is. silvia's `_getCurrentValue` and
/// `_getNormalizedValue`, to the constant.
fn reading(mode: &str, t: f64) -> (f64, f64) {
    let day_seconds = t.rem_euclid(DAY);
    let days = t.div_euclid(DAY) as i64;
    match mode {
        "minutes" => {
            let m = (day_seconds / 60.0).rem_euclid(60.0).floor();
            (m, m / 60.0)
        }
        "hours" => {
            let h = (day_seconds / 3600.0).floor();
            (h, h / 24.0)
        }
        "hours12" => {
            let h = (day_seconds / 3600.0).floor().rem_euclid(12.0);
            (h, h / 12.0)
        }
        "daySeconds" => (day_seconds, day_seconds / DAY),
        // Already a fraction: silvia hands the same number out of both ports.
        "dayProgress" => {
            let p = day_seconds / DAY;
            (p, p)
        }
        "weekProgress" => {
            // 1 January 1970 was a Thursday, and silvia's week starts on Sunday.
            let weekday = (days + 4).rem_euclid(7) as f64;
            let p = (weekday * DAY + day_seconds) / (7.0 * DAY);
            (p, p)
        }
        "yearProgress" => {
            let (_, into, length) = civil(days);
            let p = (into as f64 * DAY + day_seconds) / (length as f64 * DAY);
            (p, p)
        }
        // "seconds", and anything a file carries that this build does not have.
        _ => {
            let s = day_seconds.rem_euclid(60.0);
            (s, s / 60.0)
        }
    }
}

impl CpuNode for Clock {
    fn reset(&mut self) {}

    fn tick(&mut self, id: crate::graph::NodeId, ctx: &mut TickContext<'_>) {
        let t = now(ctx.input(id, "offset"));
        let (value, normalized) = reading(ctx.option(id, "mode"), t);
        ctx.publish(id, "value", value);
        ctx.publish(id, "normalized", normalized);
    }
}

#[cfg(test)]
mod tests {
    use super::{DAY, civil, reading};

    /// A known instant, read every way silvia reads one. 2 March 2021 was a Tuesday;
    /// 1 614 693 725 is 2021-03-02T14:02:05 UTC, so the week is two days and fourteen hours
    /// in from Sunday and the year sixty days.
    #[test]
    fn every_mode_reads_the_instant_silvia_reads() {
        let t = 1_614_693_725.0 + 0.5;
        let near = |got: f64, want: f64| assert!((got - want).abs() < 1e-3, "{got} != {want}");

        near(reading("seconds", t).0, 5.5);
        near(reading("seconds", t).1, 5.5 / 60.0);
        near(reading("minutes", t).0, 2.0);
        near(reading("hours", t).0, 14.0);
        near(reading("hours12", t).0, 2.0);
        let day = 14.0 * 3600.0 + 2.0 * 60.0 + 5.5;
        near(reading("daySeconds", t).0, day);
        near(reading("dayProgress", t).0, day / DAY);
        // Tuesday is the third day of a week that starts on Sunday.
        near(
            reading("weekProgress", t).0,
            ((2.0 * DAY) + day) / (7.0 * DAY),
        );
        // 31 + 28 whole days before 2 March in a year that is not a leap year, and the day
        // itself part way through.
        near(
            reading("yearProgress", t).0,
            (60.0 * DAY + day) / (365.0 * DAY),
        );
    }

    /// The date arithmetic the year's progress rests on, over a leap year and either side of
    /// it.
    #[test]
    fn the_civil_date_survives_a_leap_year() {
        // 1970-01-01, 2000-02-29, 2021-01-01.
        assert_eq!(civil(0), (1970, 0, 365));
        assert_eq!(civil(11_016), (2000, 59, 366));
        assert_eq!(civil(18_628), (2021, 0, 365));
        // The last day of a leap year is its 366th.
        assert_eq!(civil(11_322), (2000, 365, 366));
    }
}
