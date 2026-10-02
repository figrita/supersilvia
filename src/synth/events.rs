// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a tick hands the editor **once**: a deck claim, a firing, a value it wrote, what a
//! probe counted, a picture a save or a Snap asked for, a MIDI message.
//!
//! **One ordered log per kind, kept by the synth and never by a buffer.** The snapshot is a
//! double buffer, and a synth that ticks twice between two frames gets back the buffer the
//! editor never took. An event accumulated in the buffers would go out again under a newer
//! `seq`, behind one that happened after it: a stale deck claim landing after a newer one, a
//! recording landing after the Clear that emptied it. So each kind is a [`Log`] on the synth,
//! and each snapshot carries the entries the editor has not acknowledged beside a count of
//! every one there has ever been. The editor applies only the entries past the count it last
//! saw, in order, and nothing twice. `docs/architecture.md` has the argument.
//!
//! **The editor acknowledges by handing a buffer back.** [`super::thread::Mailbox`] marks
//! everything in the buffer the editor returns as seen, and the synth forgets those entries
//! when that buffer comes back to it. A snapshot copies only the entries it does not already
//! hold, so a tick's cost is the events it made and not the length of the log.
//!
//! **A log is bounded.** An editor that stops taking snapshots — minimized, or stalled — lets
//! the oldest go past [`KEPT`], or [`SNAPS`] for the Snaps. Past that the
//! snapshot says how many the editor missed, and the editor logs the loss rather than
//! applying a run with a hole at its front.

use crate::graph::{NodeId, PortRef, Value};
use crate::mixer::Channel;
use std::collections::VecDeque;

/// How many entries a log keeps that the editor has not seen. A tick's firings and a tick's
/// probe counts are one entry each, so this is ticks for those.
pub const KEPT: usize = 1024;

/// How many Snaps a log keeps, each a whole frame at the Output's own size.
pub const SNAPS: usize = 16;

/// One kind of one-shot event, oldest first, beside a count of every one there has been.
#[derive(Debug)]
pub struct Log<T, const KEEP: usize> {
    entries: VecDeque<T>,
    /// Every entry ever pushed. The newest entry's place is `count - 1`.
    count: u64,
    /// How many of them the editor had applied before this snapshot. Written by the mailbox.
    seen: u64,
    /// The place of the first entry that is delivered: every one before it belonged to a
    /// project that has been replaced.
    from: u64,
}

impl<T, const KEEP: usize> Default for Log<T, KEEP> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            count: 0,
            seen: 0,
            from: 0,
        }
    }
}

impl<T: Clone, const KEEP: usize> Log<T, KEEP> {
    /// Add one, letting the oldest go past `KEEP`.
    pub(super) fn push(&mut self, entry: T) {
        if self.entries.len() == KEEP {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.count += 1;
    }

    /// Every entry there has ever been.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// How many entries are held.
    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.entries.len()
    }

    /// The place of the oldest entry held.
    fn first(&self) -> u64 {
        self.count - self.entries.len() as u64
    }

    /// Let go of every entry before place `upto`.
    fn forget(&mut self, upto: u64) {
        let gone = usize::try_from(upto.saturating_sub(self.first())).unwrap_or(usize::MAX);
        self.entries.drain(..gone.min(self.entries.len()));
    }

    /// Deliver none of what is held or was pushed before: it belonged to a project that has
    /// been replaced.
    fn skip(&mut self) {
        self.entries.clear();
        self.from = self.count;
    }

    /// Bring `out`, a copy this log filled earlier, up to it: what the log let go of goes,
    /// and what `out` does not hold yet is appended.
    fn copy_into(&self, out: &mut Self) {
        out.forget(self.first());
        let from = usize::try_from(out.count.saturating_sub(self.first())).unwrap_or(usize::MAX);
        out.entries
            .extend(self.entries.range(from.min(self.entries.len())..).cloned());
        out.count = self.count;
        out.from = self.from;
    }

    /// Every entry the editor had not seen, oldest first, each with its place — or, where
    /// more happened than the log kept, how many that was.
    pub fn fresh(&self) -> Result<impl Iterator<Item = (u64, &T)>, u64> {
        let start = self.seen.max(self.from);
        let unseen = self.count.saturating_sub(start);
        let held = self.entries.len() as u64;
        if unseen > held {
            return Err(unseen);
        }
        let skip = usize::try_from(held - unseen).unwrap_or(usize::MAX);
        Ok((start..).zip(self.entries.range(skip..)))
    }

    /// [`Self::fresh`], with a loss logged and nothing of it applied.
    pub fn unseen(&self, what: &str) -> impl Iterator<Item = (u64, &T)> {
        self.fresh()
            .map_err(|missed| {
                log::warn!(
                    "the editor missed {missed} {what}, more than the {KEEP} the synth keeps; \
                     none of them is applied"
                );
            })
            .ok()
            .into_iter()
            .flatten()
    }
}

/// Every kind of one-shot event, as the synth keeps them and as a snapshot carries them.
#[derive(Debug, Default)]
pub struct Events {
    /// The decks a press or an edge claimed, in the order the ticks claimed them.
    pub decks: Log<(NodeId, Channel), KEPT>,
    /// Every action port one tick saw fire — an output that emitted a down, an input something
    /// fired into, a button a finger went down on. One entry a tick that fired anything. What
    /// it is for is the throb: an action is invisible otherwise, on every node alike.
    pub fired: Log<Vec<PortRef>, KEPT>,
    /// A value a tick wrote onto its own node — `automation`'s recording, when it stops.
    pub values: Log<(NodeId, &'static str, Value), KEPT>,
    /// What one tick's probes counted, each by the Output it probed.
    pub probes: Log<Vec<(NodeId, Vec<u32>)>, KEPT>,
    /// A picture a save asked for, RGBA8 at `render::output::THUMBNAIL`.
    pub thumbnails: Log<(NodeId, Vec<u8>), KEPT>,
    /// A Snap: the Output, its size and its pixels.
    pub snaps: Log<(NodeId, u32, u32, Vec<u8>), SNAPS>,
    /// Every MIDI message the tick read, for the window's monitor and for learning.
    pub midi: Log<crate::midi::Message, KEPT>,
}

/// What [`Events`] asks of each of its logs whatever it holds.
trait Counted {
    fn count(&self) -> u64;
    fn seen(&self) -> u64;
    fn see(&mut self, upto: u64);
    fn forget(&mut self, upto: u64);
}

impl<T: Clone, const KEEP: usize> Counted for Log<T, KEEP> {
    fn count(&self) -> u64 {
        self.count
    }

    fn seen(&self) -> u64 {
        self.seen
    }

    fn see(&mut self, upto: u64) {
        self.seen = upto;
    }

    fn forget(&mut self, upto: u64) {
        Log::forget(self, upto);
    }
}

impl Events {
    fn all(&self) -> [&dyn Counted; 7] {
        [
            &self.decks,
            &self.fired,
            &self.values,
            &self.probes,
            &self.thumbnails,
            &self.snaps,
            &self.midi,
        ]
    }

    fn all_mut(&mut self) -> [&mut dyn Counted; 7] {
        [
            &mut self.decks,
            &mut self.fired,
            &mut self.values,
            &mut self.probes,
            &mut self.thumbnails,
            &mut self.snaps,
            &mut self.midi,
        ]
    }

    /// The editor has applied everything these carry.
    pub(super) fn see_all(&mut self) {
        for log in self.all_mut() {
            log.see(log.count());
        }
    }

    /// The editor had applied everything `before` carries, and nothing of these past it.
    pub(super) fn seen_after(&mut self, before: &Self) {
        for (log, before) in self.all_mut().into_iter().zip(before.all()) {
            log.see(before.count());
        }
    }

    /// The synth's logs let go of what the editor says, in a buffer it handed back, it saw.
    pub(super) fn forget_seen(&mut self, buffer: &Self) {
        for (log, buffer) in self.all_mut().into_iter().zip(buffer.all()) {
            log.forget(buffer.seen());
        }
    }

    /// Another project replaced the one these were made in: none of them is delivered. MIDI's
    /// messages are the run's, and stay.
    pub fn forget_project(&mut self) {
        self.decks.skip();
        self.fired.skip();
        self.values.skip();
        self.probes.skip();
        self.thumbnails.skip();
        self.snaps.skip();
    }

    /// Bring a snapshot's copy up to the synth's logs.
    pub(super) fn copy_into(&self, out: &mut Self) {
        self.decks.copy_into(&mut out.decks);
        self.fired.copy_into(&mut out.fired);
        self.values.copy_into(&mut out.values);
        self.probes.copy_into(&mut out.probes);
        self.thumbnails.copy_into(&mut out.thumbnails);
        self.snaps.copy_into(&mut out.snaps);
        self.midi.copy_into(&mut out.midi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places<const K: usize>(log: &Log<u32, K>) -> Vec<(u64, u32)> {
        log.fresh()
            .expect("nothing missed")
            .map(|(n, e)| (n, *e))
            .collect()
    }

    /// A copy brought up to the log holds exactly what the log does, whichever copy it was
    /// and however far behind: entries are appended once, and what the log let go of goes.
    #[test]
    fn a_copy_follows_the_log() {
        let mut log: Log<u32, 4> = Log::default();
        let mut copy = Log::default();
        for n in 0..3 {
            log.push(n);
        }
        log.copy_into(&mut copy);
        assert_eq!(copy.entries, [0, 1, 2]);
        for n in 3..9 {
            log.push(n);
        }
        log.copy_into(&mut copy);
        assert_eq!(copy.entries, [5, 6, 7, 8], "the oldest went past four");
        assert_eq!(copy.count(), 9);
        log.forget(7);
        log.copy_into(&mut copy);
        assert_eq!(
            copy.entries,
            [7, 8],
            "and what the editor saw goes from both"
        );
    }

    /// The editor sees what is past its count, with each entry's place, and a loss as a loss
    /// rather than as the part of it that is left.
    #[test]
    fn what_is_fresh_is_past_the_count_the_editor_saw() {
        let mut log: Log<u32, 4> = Log::default();
        for n in 10..13 {
            log.push(n);
        }
        log.seen = 1;
        assert_eq!(places(&log), [(1, 11), (2, 12)]);
        log.seen = 3;
        assert!(places(&log).is_empty(), "nothing twice");
        for n in 13..20 {
            log.push(n);
        }
        assert_eq!(
            log.fresh().err(),
            Some(7),
            "seven happened and four are kept"
        );
        assert_eq!(
            log.unseen("numbers").count(),
            0,
            "none of the four is applied"
        );
    }

    #[test]
    fn nothing_from_before_another_project_is_delivered_and_midi_stays() {
        let mut synth = Events::default();
        synth.decks.push((NodeId(1), Channel::A));
        synth
            .values
            .push((NodeId(1), "recording", Value::Points(Vec::new())));
        synth.snaps.push((NodeId(1), 1, 1, vec![1]));
        synth.midi.push(crate::midi::Message {
            channel: 0,
            kind: crate::midi::Kind::Control { cc: 1, value: 1 },
        });
        let mut stale = Events::default();
        synth.copy_into(&mut stale);
        stale.forget_project();
        assert_eq!(stale.decks.fresh().map(Iterator::count), Ok(0));
        assert_eq!(stale.midi.fresh().map(Iterator::count), Ok(1));

        synth.forget_project();
        synth.decks.push((NodeId(2), Channel::B));
        let mut out = Events::default();
        synth.copy_into(&mut out);
        let decks: Vec<_> = out.decks.fresh().expect("nothing missed").collect();
        assert_eq!(decks, [(1, &(NodeId(2), Channel::B))]);
        assert_eq!(out.values.fresh().map(Iterator::count), Ok(0));
        assert_eq!(out.snaps.fresh().map(Iterator::count), Ok(0));
        assert_eq!(out.midi.fresh().map(Iterator::count), Ok(1));
    }
}
