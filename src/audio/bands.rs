// SPDX-License-Identifier: AGPL-3.0-or-later

//! Turning a spectrum into three numbers a graph can play.
//!
//! A band is a **center frequency and a Q**, not a fixed span: that is what lets a band be
//! retuned as one point — X is where it listens, Y is how narrowly — and it is what silvia's
//! band dots drag. The bins under it are averaged with logarithmic weighting, mapped through
//! the same decibel window a browser's `AnalyzerNode` uses, and shaped by a power curve.
//!
//! What comes out is the band's own level and nothing more. **Shaping it is the graph's
//! job**: `slew` smooths any uniform number, and an exciter — a departure from a running
//! median, expanded and soft-clipped — is a node's worth of work that can be metered,
//! patched and put anywhere, which one buried in here could not. See
//! [decisions.md](../../docs/decisions.md).
//!
//! The measurement is ported from silvia's `audioAnalyzer.js`, constants included, because
//! they are tuned and the tuning is the value.

/// Low, mid and high. Three is what a hand can mix.
pub const BANDS: usize = 3;

/// The decibel window a bin is measured in.
///
/// A window, rather than a raw magnitude, because every constant below was tuned against a
/// browser's byte spectrum, which is a decibel domain — a linear magnitude would make all of
/// them mean something else. The **span** is silvia's seventy decibels; the **top** is not.
/// Web Audio's -100..-30 assumes its own bin scaling, where a full-scale sine lands well
/// below zero. `Analyzer::magnitude` normalizes so a full-scale sine on a bin reads exactly
/// one, so the top belongs at 0 dB. Keeping silvia's ceiling here pinned every loud band at
/// one, and a band already at one cannot report that a kick just landed.
const MIN_DB: f32 = -70.0;
const MAX_DB: f32 = 0.0;

/// Per-bin smoothing between analyzes, as `AnalyzerNode::smoothingTimeConstant`.
pub const BIN_SMOOTHING: f32 = 0.3;

/// Where one band listens.
//
// Serialized because the Main Input's tuning is one global set of these, saved with the
// project; a node's own tuning is controls and does not come through here.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BandConfig {
    /// Center frequency in Hz.
    pub freq: f32,
    /// Q. Higher is narrower: the band spans `freq / q` Hz around its center.
    pub q: f32,
    /// Power curve on the averaged band. Below 1 lifts quiet detail.
    pub curve: f32,
}

impl BandConfig {
    /// The bins this band covers, given the width of one bin and how many there are.
    ///
    /// Never bin 0: that is DC, which is not sound.
    pub fn bins(&self, bin_hz: f32, bin_count: usize) -> (usize, usize) {
        if bin_hz <= 0.0 || bin_count < 2 {
            return (1, 1);
        }
        let bandwidth = self.freq / self.q.max(0.01);
        let lo = (self.freq - bandwidth * 0.5).max(0.0);
        let hi = self.freq + bandwidth * 0.5;
        let last = bin_count - 1;
        let lo_bin = ((lo / bin_hz).round() as usize).clamp(1, last);
        let hi_bin = ((hi / bin_hz).round() as usize).clamp(1, last);
        (lo_bin.min(hi_bin), hi_bin.max(lo_bin))
    }
}

/// silvia's tuning, which is the point of porting it rather than inventing one.
pub const DEFAULT: [BandConfig; BANDS] = [
    BandConfig {
        freq: 100.0,
        q: 1.0,
        curve: 0.5,
    },
    BandConfig {
        freq: 1000.0,
        q: 1.0,
        curve: 0.6,
    },
    BandConfig {
        freq: 8000.0,
        q: 1.0,
        curve: 0.7,
    },
];

/// What each band is called, in order, for ports and labels.
pub const NAMES: [&str; BANDS] = ["bass", "mid", "high"];

/// A linear magnitude as the fraction of the decibel window it occupies, 0 at silence.
pub fn to_db_fraction(magnitude: f32) -> f32 {
    if magnitude <= 0.0 || !magnitude.is_finite() {
        return 0.0;
    }
    let db = 20.0 * magnitude.log10();
    ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0)
}

/// One band's value: the log-weighted average of its bins, curved.
///
/// Logarithmic weighting, because a band an octave wide holds many more high bins than low
/// ones and a flat mean would let the top of the band speak for all of it.
pub fn level(bins: &[f32], range: (usize, usize), cfg: &BandConfig) -> f32 {
    let (lo, hi) = range;
    let mut sum = 0.0;
    let mut weights = 0.0;
    let hi = hi.min(bins.len().saturating_sub(1));
    for (i, bin) in bins.iter().enumerate().take(hi + 1).skip(lo) {
        let weight = ((i + 1) as f32).ln();
        sum += bin * weight;
        weights += weight;
    }
    if weights <= 0.0 {
        return 0.0;
    }
    (sum / weights).powf(cfg.curve)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BandConfig {
        DEFAULT[0]
    }

    #[test]
    fn a_band_covers_the_bins_its_q_says() {
        // 100 Hz at Q 1 spans 100 Hz: 50 to 150.
        let (lo, hi) = cfg().bins(10.0, 256);
        assert_eq!((lo, hi), (5, 15));
        // A higher Q is narrower about the same center.
        let narrow = BandConfig { q: 4.0, ..cfg() };
        let (lo, hi) = narrow.bins(10.0, 256);
        assert!(lo > 5 && hi < 15, "got {lo}..{hi}");
    }

    #[test]
    fn a_band_never_reaches_dc_or_past_the_last_bin() {
        let low = BandConfig {
            freq: 5.0,
            q: 0.5,
            ..cfg()
        };
        assert_eq!(low.bins(10.0, 256).0, 1, "bin 0 is DC, which is not sound");
        let high = BandConfig {
            freq: 100_000.0,
            ..cfg()
        };
        assert_eq!(high.bins(10.0, 256).1, 255);
    }

    #[test]
    fn silence_is_the_bottom_of_the_decibel_window() {
        assert_eq!(to_db_fraction(0.0), 0.0);
        assert_eq!(to_db_fraction(-1.0), 0.0, "and so is nonsense");
        assert_eq!(to_db_fraction(1.0), 1.0, "full scale is the top");
        // Half way down the window in decibels is half way along the fraction.
        let mid = to_db_fraction(10f32.powf((MIN_DB / 2.0) / 20.0));
        assert!((mid - 0.5).abs() < 0.01, "got {mid}");
    }

    #[test]
    fn a_band_with_no_bins_under_it_is_silent() {
        // Rather than a division by zero, which would travel into a uniform as a NaN.
        assert_eq!(level(&[], (0, 0), &cfg()), 0.0);
    }
}
