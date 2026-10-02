// SPDX-License-Identifier: AGPL-3.0-or-later

//! How long a source has delivered nothing.
//!
//! A camera that never delivers looks, on the node and in the panel, exactly like one that is
//! warming up: a black picture and *no frame yet*. A camera warms up in a second or two, so
//! one that has said nothing for [`NOT_RESPONDING`] is not warming up, and is said to be not
//! responding instead — the Logitech that wedges on its first stream after a plug-in, a
//! device another program holds, a cable pulled with no end-of-stream behind it. Wall time,
//! read off a clock the tick hands in, since the tick never waits on the source to find out.

use std::time::{Duration, Instant};

/// How long a source may deliver nothing before it is not responding.
pub const NOT_RESPONDING: Duration = Duration::from_secs(10);

/// What a source that has delivered nothing for [`NOT_RESPONDING`] says.
pub const NOT_RESPONDING_SAYS: &str = "not responding: no frame for 10 s";

/// When a source last delivered, or was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Silence {
    since: Instant,
}

impl Silence {
    /// A source opened at `now`: nothing yet, and nothing owed yet either.
    pub fn new(now: Instant) -> Self {
        Self { since: now }
    }

    /// The source delivered at `now`, or was not yet expected to.
    pub fn heard(&mut self, now: Instant) {
        self.since = now;
    }

    /// Whether the source has delivered nothing for [`NOT_RESPONDING`] up to `now`.
    pub fn not_responding(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= NOT_RESPONDING
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing for ten seconds is not responding; a frame before that, or since, starts the
    /// count again.
    #[test]
    fn ten_seconds_of_nothing_is_not_responding() {
        let opened = Instant::now();
        let at = |s: f32| opened + Duration::from_secs_f32(s);
        let mut silence = Silence::new(opened);
        assert!(!silence.not_responding(at(9.9)));
        assert!(silence.not_responding(at(10.0)));
        silence.heard(at(10.5));
        assert!(!silence.not_responding(at(11.0)), "a frame arrived");
        assert!(!silence.not_responding(at(20.4)));
        assert!(silence.not_responding(at(20.5)), "and stopped again");
        assert!(
            !Silence::new(at(5.0)).not_responding(opened),
            "a clock read before the open is no silence at all"
        );
    }
}
