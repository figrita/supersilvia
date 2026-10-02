// SPDX-License-Identifier: AGPL-3.0-or-later

//! A per-instance xorshift32 PRNG. silvia's `Math.random()`, made a function of the node's
//! own id rather than of the host's entropy, so a test and a rerun see the same sequence —
//! `oscillator`'s own generator, moved here once a second node needed one.

use crate::graph::NodeId;

/// xorshift32 state, zero until [`Rng::seed`] has run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rng(pub(crate) u32);

impl Rng {
    pub const fn new() -> Self {
        Self(0)
    }

    /// Seed from a node's id, if nothing has seeded this instance yet. Odd and non-zero,
    /// which is all xorshift asks of a seed; idempotent, so a `tick` can call it every
    /// frame rather than only on the first.
    pub fn seed(&mut self, id: NodeId) {
        if self.0 == 0 {
            self.0 = id.0.wrapping_mul(2_654_435_761) | 1;
        }
    }

    /// A generator started from one word rather than a node's id: a press that rolls dice
    /// once, with whatever number the moment of the press gave it. The word is mixed first,
    /// since xorshift's first draws from a small seed are small.
    pub const fn from_word(word: u32) -> Self {
        let x = word.wrapping_mul(2_654_435_761);
        Self((x ^ (x >> 16)) | 1)
    }

    /// One fresh random in 0..1. Reads as zero forever if `seed` was never called.
    pub fn next_f32(&mut self) -> f32 {
        // The top 24 bits, which is the span `Math.random` promises.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// One fresh word, all 32 bits of it: a seed for something that hashes its own.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// One fresh random in `[min, max]`.
    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + self.next_f32() * (max - min)
    }
}
