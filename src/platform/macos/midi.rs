// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIDI in through CoreMIDI, by way of `midir`: one connection per source, each handing its
//! bytes to a callback on CoreMIDI's own thread.
//!
//! **CoreMIDI's thread is the reader.** Linux blocks a thread of its own on the sequencer;
//! here `MidiInput::connect` calls back on the thread CoreMIDI delivers on, so that callback
//! parses the bytes into a [`Message`] and posts it, and nothing of this app's blocks
//! anywhere. Every connection holds a sender to the one queue, so the synth drains one queue
//! whichever device a message came from.
//!
//! **One connection per source, not a subscription.** The ALSA sequencer lets a second client
//! wire a device into the reader's port; CoreMIDI has nothing of the kind, and `midir`'s
//! `connect` consumes its `MidiInput`, so each source gets a client and an input port of its
//! own. A separate client, never consumed, answers what sources exist.
//!
//! **A source is known by its CoreMIDI unique ID**, which the system keeps for a device
//! across being unplugged and plugged back in. [`Midi::connect_all`] closes the connection of
//! every source no longer offered, then connects every one offered and not held — so a device
//! that goes away is dropped, and one that comes back is wired in again. A source dropped is
//! said on the queue as [`Wire::Gone`], so the notes it held are let go: its note-offs will
//! never come.
//!
//! **A watcher thread wires in what appears, where Linux waits for Rescan.** CoreMIDI lists a
//! device the moment it arrives, and a device can arrive after launch: macOS asks whether to
//! allow a USB accessory the first time it is plugged in, and holds it back until the answer.
//! The window lists every source live, so without the watcher a device plugged in after
//! opening would show there and stay silent, with only the hollow mark saying it is not
//! wired. The watcher runs [`Midi::connect_all`] every [`WATCH`] on a client of its own,
//! sleeping in between and never on the frame, and ends when the handle is dropped. Rescan is
//! the same pass run at once.

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
    /// Open CoreMIDI, wire in every source it offers, and start the watcher.
    ///
    /// `Err` where CoreMIDI will not give this process a client, which the caller reports once
    /// and carries on without MIDI, as Linux does without a sequencer.
    ///
    /// The queue comes back beside the handle rather than inside it: the editor keeps the
    /// handle, for the device list and the wiring, and hands the queue to the synth.
    pub fn open() -> Result<(Self, Receiver<Wire>), String> {
        // The first client this process creates is the one CoreMIDI tells about setup changes,
        // on the run loop of the thread that created it; this is the main thread's, which
        // runs one.
        let browser = client()?;
        let watcher = client()?;
        let (tx, events) = std::sync::mpsc::channel();
        let wiring = Arc::new(Wiring {
            held: Mutex::new(Vec::new()),
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

    /// Every MIDI source CoreMIDI offers, sorted by name.
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

    /// Wire every source CoreMIDI offers into the queue, and let go of every one it no longer
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
                log::warn!("midi: could not connect {id}: {e}");
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
            Ok(connection) => Some(connection),
            Err(e) => {
                log::warn!("midi: could not connect {id}: {e}");
                None
            }
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

/// The client name this editor takes in CoreMIDI, and the name of each input port it opens.
const CLIENT: &str = "supersilvia";
const PORT: &str = "MIDI In";

/// How often the watcher looks for sources that came or went.
const WATCH: std::time::Duration = std::time::Duration::from_secs(1);

fn client() -> Result<midir::MidiInput, String> {
    midir::MidiInput::new(CLIENT).map_err(|e| format!("no CoreMIDI client: {e}"))
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

    /// Opening needs no device, and wires in every source there is. A source plugged in
    /// after that is wired in with no Rescan pressed, a second pass wires nothing twice, and
    /// what the source sends arrives on the queue the synth drains: a knob's Control Change
    /// and a pad's note, down and up. The source is a CoreMIDI virtual one, so the whole path
    /// is the real one with no device on the desk.
    ///
    /// One test rather than two, because the sources are the process's: a virtual source made
    /// by one test is offered to every other one running beside it.
    #[test]
    fn a_source_plugged_in_later_is_heard() {
        use midir::os::unix::VirtualOutput;
        use std::time::{Duration, Instant};

        let (mut midi, events) = Midi::open().expect("a CoreMIDI client");
        assert!(midi.sources().iter().all(|s| s.connected));

        let name = format!("supersilvia test {}", std::process::id());
        let mut source = midir::MidiOutput::new("supersilvia test")
            .expect("a CoreMIDI client")
            .create_virtual(&name)
            .expect("a virtual source");
        let wired = |midi: &Midi| midi.sources().iter().any(|s| s.name == name && s.connected);
        let since = Instant::now();
        while !wired(&midi) && since.elapsed() < WATCH * 3 {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wired(&midi), "{:?}", midi.sources());
        let held = midi.wiring.held().len();
        midi.connect_all();
        assert_eq!(midi.wiring.held().len(), held);

        source.send(&[0xb0, 70, 99]).expect("a Control Change sent");
        source.send(&[0x99, 36, 127]).expect("a note-on sent");
        source.send(&[0x89, 36, 0]).expect("a note-off sent");
        let mut from = None;
        let heard: Vec<Message> = (0..3)
            .map_while(|_| events.recv_timeout(Duration::from_secs(2)).ok())
            .filter_map(|wire| match wire {
                Wire::Message(device, message) => {
                    from = Some(device);
                    Some(message)
                }
                Wire::Gone(_) => None,
            })
            .collect();
        assert_eq!(
            heard,
            [
                Message {
                    channel: 0,
                    kind: Kind::Control { cc: 70, value: 99 }
                },
                Message {
                    channel: 9,
                    kind: Kind::Note { note: 36, on: true }
                },
                Message {
                    channel: 9,
                    kind: Kind::Note {
                        note: 36,
                        on: false
                    }
                },
            ]
        );

        // Unplugged: the watcher lets go of the source and says so, naming the device its
        // messages came from, so the notes it held are let go.
        drop(source);
        let since = Instant::now();
        let mut gone = None;
        while gone.is_none() && since.elapsed() < WATCH * 3 {
            if let Ok(Wire::Gone(device)) = events.recv_timeout(Duration::from_millis(100)) {
                gone = Some(device);
            }
        }
        assert!(from.is_some());
        assert_eq!(gone, from, "the source that went is the one that sent");
    }
}
