// SPDX-License-Identifier: AGPL-3.0-or-later

//! How the synth feeds the one queue: **about one Output per submission, and little queued
//! ahead.**
//!
//! wgpu gives a device one queue, and the editor's paint and every picture window's blit go
//! into it behind whatever the synth has already submitted. Step 0b measured a synth that
//! submitted a whole tick at once starving the editor: its frame waited for the rest of the
//! tick, most of an interval near saturation (`proposals/wgpu.md`, the one-queue section). So
//! the synth submits an Output on its own, or a run of cheap ones together
//! ([`super::SUBMISSION_MS`]), and before each submission [`Throttle::submit`] waits until at
//! most [`QUEUED_AHEAD`] of its earlier submissions are still on the GPU. An editor frame then
//! lands behind the synth submission that is running and at most that many queued, rather
//! than behind a tick. The GPU is kept fed — one submission is queued while the one before it
//! runs — and the synth thread, not the queue, holds the rest of the tick.
//!
//! [`Recording`] is an encoder that knows whether anything went into it, so a phase with
//! nothing to do submits nothing.

use super::gpu::{Gpu, Ticket};
use std::collections::VecDeque;
use std::time::Duration;

/// How many of the synth's earlier submissions may still be on the GPU when it submits the
/// next. One keeps the GPU busy across the gap between two submissions while leaving an
/// editor frame behind about one Output's pass.
pub const QUEUED_AHEAD: usize = 1;

/// How long the throttle waits for one earlier submission before submitting anyway: the
/// renderer's own bound on a GPU that has stopped, [`super::QUEUE_WAIT`].
const WAIT: Duration = super::QUEUE_WAIT;

/// A command encoder, and whether anything has been recorded into it.
pub struct Recording {
    encoder: wgpu::CommandEncoder,
    used: bool,
}

impl Recording {
    pub fn new(gpu: &Gpu, label: &str) -> Self {
        Self {
            encoder: gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) }),
            used: false,
        }
    }

    /// The encoder, to record into: what is recorded is submitted.
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.used = true;
        &mut self.encoder
    }

    /// Whether anything has been asked for the encoder.
    pub fn used(&self) -> bool {
        self.used
    }

    /// The commands, where there are any.
    pub fn finish(self) -> Option<wgpu::CommandBuffer> {
        self.used.then(|| self.encoder.finish())
    }
}

/// The synth's submissions still on the GPU, oldest first, and the bound on how many.
pub struct Throttle {
    pending: VecDeque<Ticket>,
    depth: usize,
    /// Submissions that found [`QUEUED_AHEAD`] earlier ones on the GPU and waited.
    pub waits: u64,
    /// Waits that ran out.
    pub timeouts: u64,
    /// The most earlier submissions ever found still on the GPU at the moment of a submit,
    /// after making room: never more than `depth` unless a wait ran out.
    pub most_ahead: usize,
    /// Submissions made.
    pub submissions: u64,
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new(QUEUED_AHEAD)
    }
}

impl Throttle {
    /// A throttle keeping at most `depth` earlier submissions on the GPU.
    pub fn new(depth: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            depth,
            waits: 0,
            timeouts: 0,
            most_ahead: 0,
            submissions: 0,
        }
    }

    /// Submit what `recording` holds, once at most `depth` earlier submissions are still on
    /// the GPU. `None`, with nothing submitted and nothing waited for, where it holds
    /// nothing.
    pub fn submit(&mut self, gpu: &Gpu, recording: Recording) -> Option<Ticket> {
        let commands = recording.finish()?;
        self.make_room(gpu);
        self.most_ahead = self.most_ahead.max(self.pending.len());
        let ticket = gpu.submit([commands]);
        self.pending.push_back(ticket.clone());
        self.submissions += 1;
        Some(ticket)
    }

    /// Wait until at most `depth` submissions are still on the GPU.
    fn make_room(&mut self, gpu: &Gpu) {
        self.forget_finished(gpu);
        if self.pending.len() <= self.depth {
            return;
        }
        gpu.poll();
        self.forget_finished(gpu);
        while self.pending.len() > self.depth {
            let Some(oldest) = self.pending.pop_front() else {
                break;
            };
            self.waits += 1;
            if gpu.wait(&oldest, WAIT).is_err() {
                self.timeouts += 1;
                if self.timeouts == 1 {
                    log::warn!(
                        "the GPU did not finish a synth submission in {} ms; submitting anyway",
                        WAIT.as_millis()
                    );
                }
            }
            self.forget_finished(gpu);
        }
    }

    fn forget_finished(&mut self, gpu: &Gpu) {
        let completed = gpu.completed();
        self.pending.retain(|t| t.serial > completed);
    }

    /// How many of the synth's submissions are still on the GPU, as of the last poll.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}
