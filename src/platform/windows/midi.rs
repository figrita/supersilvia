// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIDI in through WinMM, by way of `midir`: one connection per source, each handing its bytes
//! to a callback on WinMM's own thread.
//!
//! **WinMM's thread is the reader**, as CoreMIDI's is on a Mac: `MidiInput::connect` calls back
//! on the thread WinMM delivers on, so that callback parses the bytes into a [`Message`] and
//! posts it, and nothing of this app's blocks anywhere. Every connection holds a sender to the
//! one queue, so the synth drains one queue whichever device a message came from.
//!
//! **One connection per source.** `midir`'s `connect` consumes its `MidiInput`, so each source
//! gets a client of its own. A separate client, never consumed, answers what sources exist.
//!
//! **A source is known by its device interface path**, which `midir` hands back as the port's
//! ID and Windows keeps for a device across being unplugged and plugged back in.
//! [`Midi::connect_all`] closes the connection of every source no longer offered, then connects
//! every one offered and not held — so a device that goes away is dropped, and one that comes
//! back is wired in again. A source dropped is said on the queue as [`Wire::Gone`], so the
//! notes it held are let go: its note-offs will never come.
//!
//! **A watcher thread wires in what appears**, every [`WATCH`], as on a Mac, so a device plugged
//! in after opening is heard with no Rescan pressed. Rescan is the same pass run at once.
//!
//! **A WinMM input is one application's at a time.** A source another program holds open is
//! refused, and the watcher tries it again on every pass, so the refusal is logged once per
//! source rather than once a second.

use crate::midi::{Device, Kind, Message, Source, Wire};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// The MIDI input, open.
pub struct Midi {
    /// The frame thread's client, which lists sources. It never connects: `connect` would
    /// consume it.
    browser: midir::MidiInput,
    /// The connections, shared with the watcher.
    wiring: Arc<Wiring>,
}

/// Every source wired in, and the queue their connections post to.
struct Wiring {
    held: Mutex<Vec<Held>>,
    /// The sources whose refusal has been logged, so a held one is not logged on every pass.
    refused: Mutex<Vec<String>>,
    tx: Sender<Wire>,
    /// The next connection's number, which its messages carry as their [`Device`].
    next: std::sync::atomic::AtomicU64,
}

/// A source's open connection. Dropping it closes the connection and its client.
struct Held {
    id: String,
    device: Device,
    _connection: midir::MidiInputConnection<()>,
}

impl Midi {
    /// Open WinMM, wire in every source it offers, and start the watcher.
    ///
    /// `Err` where WinMM will not give this process a client, which the caller reports once
    /// and carries on without MIDI, as Linux does without a sequencer.
    ///
    /// The queue comes back beside the handle rather than inside it: the editor keeps the
    /// handle, for the device list and the wiring, and hands the queue to the synth.
    pub fn open() -> Result<(Self, Receiver<Wire>), String> {
        let browser = client()?;
        let watcher = client()?;
        let (tx, events) = std::sync::mpsc::channel();
        let wiring = Arc::new(Wiring {
            held: Mutex::new(Vec::new()),
            refused: Mutex::new(Vec::new()),
            tx,
            next: std::sync::atomic::AtomicU64::new(0),
        });
        wiring.connect_all(&browser);
        let watched = Arc::downgrade(&wiring);
        std::thread::Builder::new()
            .name("midi watch".into())
            .spawn(move || watch(&watcher, &watched))
            .map_err(|e| e.to_string())?;
        Ok((Self { browser, wiring }, events))
    }

    /// Every MIDI source WinMM offers, sorted by name.
    pub fn sources(&self) -> Vec<Source> {
        let held = self.wiring.held();
        let mut out: Vec<Source> = self
            .browser
            .ports()
            .iter()
            .filter_map(|port| {
                Some(Source {
                    name: self.browser.port_name(port).ok()?,
                    connected: is_held(&held, &port.id()),
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Wire every source WinMM offers into the queue, and let go of every one it no longer
    /// offers. The watcher runs the same pass every [`WATCH`]; this is Rescan's, at once.
    ///
    /// Every one of them, with no device to choose: which box is on the table is a fact about
    /// tonight, and a binding names a channel and a CC rather than a device — so a controller
    /// plugged in mid-set works with nothing to pick.
    pub fn connect_all(&mut self) {
        self.wiring.connect_all(&self.browser);
    }
}

impl Wiring {
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Held>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The one pass, under the lock for all of it, so the watcher and Rescan never both
    /// connect a source.
    fn connect_all(&self, browser: &midir::MidiInput) {
        let ports = browser.ports();
        let offered: Vec<String> = ports.iter().map(midir::MidiInputPort::id).collect();
        let mut held = self.held();
        held.retain(|h| {
            let kept = offered.contains(&h.id);
            if !kept {
                let _ = self.tx.send(Wire::Gone(h.device));
            }
            kept
        });
        for (port, id) in ports.iter().zip(offered) {
            if is_held(&held, &id) {
                continue;
            }
            let device = Device(self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            if let Some(connection) = self.connect(port, &id, device) {
                held.push(Held {
                    id,
                    device,
                    _connection: connection,
                });
            }
        }
    }

    fn connect(
        &self,
        port: &midir::MidiInputPort,
        id: &str,
        device: Device,
    ) -> Option<midir::MidiInputConnection<()>> {
        let input = match client() {
            Ok(input) => input,
            Err(e) => {
                self.refuse(id, &e);
                return None;
            }
        };
        let tx = self.tx.clone();
        // A send that fails is the queue's other end gone, which is `App` going away; the
        // connection closes when `App` drops this handle, so there is nothing to stop here.
        let read = move |_: u64, bytes: &[u8], (): &mut ()| {
            if let Some(message) = parse(bytes) {
                let _ = tx.send(Wire::Message(device, message));
            }
        };
        match input.connect(port, PORT, read, ()) {
            Ok(connection) => {
                self.refused().retain(|r| r != id);
                Some(connection)
            }
            Err(e) => {
                self.refuse(id, &e.to_string());
                None
            }
        }
    }

    fn refused(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        self.refused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Log a source's refusal, the first time since it last connected.
    fn refuse(&self, id: &str, why: &str) {
        let mut refused = self.refused();
        if !refused.iter().any(|r| r == id) {
            log::warn!("midi: could not connect {id}: {why}");
            refused.push(id.to_owned());
        }
    }
}

fn is_held(held: &[Held], id: &str) -> bool {
    held.iter().any(|h| h.id == id)
}

/// The watcher: sleep, wire in what came and let go of what went, until the handle is gone.
fn watch(browser: &midir::MidiInput, wiring: &Weak<Wiring>) {
    loop {
        std::thread::sleep(WATCH);
        let Some(wiring) = wiring.upgrade() else {
            return;
        };
        wiring.connect_all(browser);
    }
}

/// The client name this editor takes in WinMM, and the name of each input port it opens.
const CLIENT: &str = "supersilvia";
const PORT: &str = "MIDI In";

/// How often the watcher looks for sources that came or went.
const WATCH: std::time::Duration = std::time::Duration::from_secs(1);

fn client() -> Result<midir::MidiInput, String> {
    midir::MidiInput::new(CLIENT).map_err(|e| format!("no WinMM client: {e}"))
}

/// One message's bytes as this editor reads them, or `None` for everything else on the wire.
///
/// `midir` hands over one whole message at a time, status byte first, so a Control Change and
/// a Note On or Off are always three bytes here.
fn parse(bytes: &[u8]) -> Option<Message> {
    let &[status, a, b] = bytes else {
        return None;
    };
    if a > 127 || b > 127 {
        return None;
    }
    let channel = status & 0x0f;
    let kind = match status & 0xf0 {
        0xb0 => Kind::Control { cc: a, value: b },
        // A note-on at velocity zero is a note-off. Half the hardware in the world sends one,
        // and a gate that only ever opens is the bug it causes.
        0x90 => Kind::Note { note: a, on: b > 0 },
        0x80 => Kind::Note { note: a, on: false },
        _ => return None,
    };
    Some(Message { channel, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Control Change carries its channel, its number and its value.
    #[test]
    fn a_control_change_is_read() {
        assert_eq!(
            parse(&[0xb3, 74, 127]),
            Some(Message {
                channel: 3,
                kind: Kind::Control { cc: 74, value: 127 }
            })
        );
    }

    /// A note-on at velocity zero arrives as a note-off, and a note-off at any velocity is one.
    #[test]
    fn notes_open_and_close() {
        let note = |on| Kind::Note { note: 60, on };
        assert_eq!(parse(&[0x90, 60, 100]).map(|m| m.kind), Some(note(true)));
        assert_eq!(parse(&[0x90, 60, 0]).map(|m| m.kind), Some(note(false)));
        assert_eq!(parse(&[0x8f, 60, 64]).map(|m| m.kind), Some(note(false)));
        assert_eq!(parse(&[0x8f, 60, 64]).map(|m| m.channel), Some(15));
    }

    /// Everything else on the wire is dropped: pitch bend, a program change, clock, sysex,
    /// and a data byte with its top bit set.
    #[test]
    fn everything_else_is_dropped() {
        for bytes in [
            &[0xe0, 0, 64][..],
            &[0xc0, 5],
            &[0xf8],
            &[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7],
            &[0xb0, 0x80, 0],
            &[],
        ] {
            assert_eq!(parse(bytes), None, "{bytes:02x?}");
        }
    }

    /// Opening needs no device, wires in every source there is, and a second pass wires nothing
    /// twice. WinMM has no virtual source to send from, so what a device sends is the three
    /// tests above.
    #[test]
    fn opening_needs_no_device() {
        let (mut midi, _events) = Midi::open().expect("a WinMM client");
        let held = midi.wiring.held().len();
        midi.connect_all();
        assert_eq!(midi.wiring.held().len(), held);
    }
}
