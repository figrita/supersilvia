// SPDX-License-Identifier: AGPL-3.0-or-later

//! Events, and the shape of one. Pure data: an event is a CPU thing and never reaches a
//! uniform.
//!
//! **An action is a gate, not a pulse** (`docs/decisions.md`). An edge fires `Down` when a
//! condition becomes true and `Up` when it stops being true, and a receiver gets both. That
//! is what lets an envelope be held rather than retriggered, a note be released, and a band
//! say when it *stopped* being loud.
//!
//! **An event carries when it happened.** `tick` runs once per frame, but a frame is 16 ms
//! and a beat is not obliged to land on one: a source that knows better than the frame — a
//! clock computing its own phase, an analyzer running at 48 kHz — says exactly where inside
//! the frame the edge fell, and an integrator advances in segments between those moments
//! rather than in one lump per frame. Without it, everything in the event half is quantized
//! to the display, which is audible as jitter on an envelope and visible as a beat landing
//! a frame late.
//!
//! What this does **not** buy is a picture faster than a frame. A `UniformNumber` is
//! sampled into a uniform once per frame however precisely it was computed; sub-frame
//! time removes jitter and fixes phase, and does not make a strobe faster than the display
//! visible.
//!
//! Events are delivered inside `tick`, in the topological order `Synth::tick` already walks, so
//! a producer's events are in the map before its consumer looks. They live for exactly one
//! frame: a consumer that wants to remember something remembers it itself, which is why
//! `Gate` exists here rather than in five nodes.

/// Which way an edge went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The condition became true.
    Down,
    /// The condition stopped being true.
    Up,
}

/// One event on an action port: an edge, and when inside the frame it happened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub edge: Edge,
    /// Seconds after this frame's tick began, in `0..dt`, `dt` being the receiving tick's
    /// own step ([`crate::nodes::TickContext::dt`]): a fraction `at / dt` of the way through
    /// the transport's advance.
    ///
    /// Zero means *as far as this source knows, now* — a hand on a button, a device that
    /// reports a level and not a moment. A source with a phase of its own says where the
    /// crossing actually was, and a consumer that integrates believes it.
    pub at: f32,
}

impl Event {
    /// An event a source could not place more precisely than the frame it arrived in.
    pub const fn now(edge: Edge) -> Self {
        Self { edge, at: 0.0 }
    }

    pub const fn at(edge: Edge, at: f32) -> Self {
        Self { edge, at }
    }

    pub fn is_down(self) -> bool {
        self.edge == Edge::Down
    }
}

/// The level a run of events leaves behind, starting from `level`.
///
/// A consumer that cares about *held* rather than *happened* — an envelope, a button with a
/// cable in it — keeps a bool and passes it through here each frame. Only the last event
/// matters: a source that opened and closed within one frame is closed.
pub fn level_after(level: bool, events: &[Event]) -> bool {
    events.last().map_or(level, |e| e.is_down())
}

/// A level turned into edges.
///
/// Most sources have a *condition* rather than an event — a band above a threshold, a
/// button under a finger, a clock inside its gate. This is the one piece of state that turns
/// one into the other, and holding it here means no node re-derives the transition rule.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    down: bool,
}

impl Gate {
    /// The edge this level change produced, if any. `None` while nothing changed.
    pub fn set(&mut self, level: bool) -> Option<Edge> {
        if level == self.down {
            return None;
        }
        self.down = level;
        Some(if level { Edge::Down } else { Edge::Up })
    }

    /// Is the gate currently held down?
    pub fn is_down(self) -> bool {
        self.down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gate_fires_on_change_and_only_on_change() {
        let mut g = Gate::default();
        assert_eq!(g.set(false), None, "a gate starts up");
        assert_eq!(g.set(true), Some(Edge::Down));
        assert_eq!(g.set(true), None, "held is not retriggered");
        assert_eq!(g.set(false), Some(Edge::Up));
        assert_eq!(g.set(false), None);
    }

    #[test]
    fn a_run_of_events_leaves_the_last_one_standing() {
        assert!(!level_after(false, &[]), "nothing happened");
        assert!(level_after(true, &[]), "and a held gate stays held");
        assert!(level_after(false, &[Event::now(Edge::Down)]));
        // Opened and closed inside one frame: closed.
        assert!(!level_after(
            false,
            &[Event::at(Edge::Down, 0.001), Event::at(Edge::Up, 0.004)]
        ));
    }

    #[test]
    fn a_gate_reports_what_it_is_holding() {
        let mut g = Gate::default();
        assert!(!g.is_down());
        g.set(true);
        assert!(g.is_down());
    }
}
