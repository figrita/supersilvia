// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIDI in: a device thread, and the three bytes it hands the frame.
//!
//! **Nothing here ever waits on a device**, which is the same rule the audio thread and every
//! `tick` keep. A reader thread — one blocking on the ALSA sequencer on Linux, CoreMIDI's own
//! on macOS — posts each message into a queue; the **synth** owns the receiving end and
//! empties it at the top of every tick, so a knob moves the world whether or not the editor
//! is painting. [`Midi`] itself is what is left over for the frame thread: which ports exist,
//! and wiring one in. Both are [`crate::platform::midi`]'s, and what is here is the
//! vocabulary they speak.
//!
//! **A queue, not a triple buffer.** The audio analysis hands over its newest reading and
//! drops the rest, because a reading that is one frame stale is a reading. A MIDI message is
//! an event: a note-off dropped is a gate that never closes, so every message is kept and
//! delivered in order.
//!
//! What a message *means* is not here. This module names the wire and nothing else: the map
//! from a message to a control lives in [`crate::project`], applying one is
//! [`crate::synth`], and learning one is `app/midi.rs`.

mod bind;
pub use crate::platform::midi::Midi;
pub use bind::{Binding, Bindings, Saved, SavedTarget, Target, Trigger};

/// What the reader hands the synth: a message from a device, or word that a device went.
///
/// **A device that goes away lets go of what it held.** A note held on a controller that is
/// then unplugged would otherwise keep its action input held for the rest of the run, since
/// the note-off it owes can never arrive. So every message says which device sent it, and
/// the backend says when one is gone: the ALSA sequencer's own announcement on Linux, a
/// source no longer offered on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Message(Device, Message),
    Gone(Device),
}

/// Which device a message came from, in the backend's own numbering and opaque past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Device(pub u64);

impl Device {
    /// The editor's own seam into the queue — a test, an agent — which never goes away.
    pub const EDITOR: Self = Self(u64::MAX);
}

/// One MIDI message, already parsed into the two kinds this reads.
///
/// Everything else on the wire — pitch bend, aftertouch, clock, sysex — is dropped at the
/// reader. Each is a real thing a controller sends and none of them has an address in this
/// editor yet; a message with nowhere to go is noise in a monitor, so it is not carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    /// The MIDI channel, 0 to 15. Part of a binding's address: two controllers on different
    /// channels do not collide, which is the whole reason it is here.
    pub channel: u8,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A continuous controller: which one, and its 0–127 value.
    Control { cc: u8, value: u8 },
    /// A note going down or coming up. A note-on at velocity zero is a note-off, which is
    /// what half the hardware in the world sends, so it arrives here already turned into one.
    Note { note: u8, on: bool },
}

impl Message {
    /// The address half of this message: what a binding is keyed by.
    pub fn trigger(self) -> Trigger {
        match self.kind {
            Kind::Control { cc, .. } => Trigger::Control {
                channel: self.channel,
                cc,
            },
            Kind::Note { note, .. } => Trigger::Note {
                channel: self.channel,
                note,
            },
        }
    }
}

/// One MIDI source the machine is offering, as a person is shown it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// What to show a person: the device's name and the port's, where they differ.
    pub name: String,
    /// Already wired into this editor's own port.
    pub connected: bool,
}
