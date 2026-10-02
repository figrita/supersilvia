// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIDI in through the ALSA sequencer: a reader thread that blocks on it, and a second client
//! for the frame thread's questions.
//!
//! **The sequencer directly, not `midir`.** cpal links `alsa` 0.11 for the audio device and
//! `alsa-sys` carries `links = "alsa"`, so only one version of it may exist in the binary;
//! `midir` pins `alsa` 0.9 through 0.10, and 0.11 accepts 0.11. On Linux `midir` is a thin
//! shell over `alsa::seq` anyway. See the note on `alsa` in `Cargo.toml`.
//!
//! **Two sequencer clients, not one.** `alsa::Seq` is `Send` and not `Sync`, so the reader
//! thread owns the one it blocks on and the frame thread keeps a second for the two things it
//! asks: what ports exist, and connect that one to ours. Subscribing a third party's port to
//! another client's is an ordinary sequencer operation, so the frame thread's client can wire
//! a device straight into the reader's port without the reader waking to do it.

use crate::midi::{Device, Kind, Message, Source, Wire};
use std::collections::HashSet;
use std::ffi::CString;
use std::sync::mpsc::{Receiver, Sender};

/// The MIDI input, open.
pub struct Midi {
    /// The frame thread's client: enumeration and subscription, never a read.
    seq: alsa::Seq,
    /// This editor's own input port, which every device is subscribed to.
    port: i32,
    client: i32,
}

/// A source as the sequencer addresses it, beside what a person is shown of it.
struct Offered {
    client: i32,
    port: i32,
    source: Source,
}

impl Midi {
    /// Open the sequencer, take an input port, and start the reader.
    ///
    /// `Err` where there is no sequencer at all — a box with no `snd_seq` module, or a
    /// container without `/dev/snd`. That is a machine without MIDI rather than a fault, so
    /// the caller reports it once and carries on with no device.
    ///
    /// The queue comes back beside the handle rather than inside it: the editor keeps the
    /// handle, for the device list and the wiring, and hands the queue to the synth.
    pub fn open() -> Result<(Self, Receiver<Wire>), String> {
        // The reader's client owns the port devices are wired to, because it is the one that
        // blocks on it. It is opened first so its address is known before anything is
        // subscribed to it.
        let reader = alsa::Seq::open(None, Some(alsa::Direction::Capture), false)
            .map_err(|e| format!("no sequencer: {e}"))?;
        reader.set_client_name(&client_name()).ok();
        let port = reader
            .create_simple_port(
                &port_name(),
                alsa::seq::PortCap::WRITE | alsa::seq::PortCap::SUBS_WRITE,
                alsa::seq::PortType::MIDI_GENERIC | alsa::seq::PortType::APPLICATION,
            )
            .map_err(|e| e.to_string())?;
        let client = reader.client_id().map_err(|e| e.to_string())?;
        // The sequencer's own announcements, so the reader hears a device leave: its port or
        // its client going, or its wire to ours being cut. Without them a note held on a
        // controller that is unplugged would hold its action input for the rest of the run.
        let announce = alsa::seq::PortSubscribe::empty().map_err(|e| e.to_string())?;
        announce.set_sender(alsa::seq::Addr::system_announce());
        announce.set_dest(alsa::seq::Addr { client, port });
        if let Err(e) = reader.subscribe_port(&announce) {
            log::warn!("midi: no announcements, so an unplugged device keeps its notes: {e}");
        }

        // The frame thread's own client. It takes no port of its own: it never receives
        // anything, it only asks what exists and wires a sender to the reader's port.
        //
        // **Named apart from the reader's**, because both are visible in anybody's patchbay
        // and two clients under one name — one of them with no ports at all — reads as a bug
        // in this program. This one says what it is for.
        let seq = alsa::Seq::open(None, None, true).map_err(|e| format!("no sequencer: {e}"))?;
        seq.set_client_name(&browser_name()).ok();

        let (tx, events) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("midi".into())
            .spawn(move || read(&reader, alsa::seq::Addr { client, port }, &tx))
            .map_err(|e| e.to_string())?;

        let mut midi = Self { seq, port, client };
        midi.connect_all();
        Ok((midi, events))
    }

    /// Every MIDI source the sequencer offers, this editor's own ports left out.
    pub fn sources(&self) -> Vec<Source> {
        self.offered().into_iter().map(|p| p.source).collect()
    }

    /// Every source with its sequencer address, sorted by name.
    fn offered(&self) -> Vec<Offered> {
        let mut out = Vec::new();
        for client in alsa::seq::ClientIter::new(&self.seq) {
            if client.get_client() == self.client || client.get_client() == 0 {
                continue;
            }
            for port in alsa::seq::PortIter::new(&self.seq, client.get_client()) {
                let caps = port.get_capability();
                if !caps.contains(alsa::seq::PortCap::READ)
                    || !caps.contains(alsa::seq::PortCap::SUBS_READ)
                {
                    continue;
                }
                if !port.get_type().contains(alsa::seq::PortType::MIDI_GENERIC) {
                    continue;
                }
                let client_name = client.get_name().unwrap_or("?").to_owned();
                let port_name = port.get_name().unwrap_or("?").to_owned();
                let name = if port_name.starts_with(&client_name) || port_name == client_name {
                    port_name
                } else {
                    format!("{client_name} — {port_name}")
                };
                out.push(Offered {
                    client: port.get_client(),
                    port: port.get_port(),
                    source: Source {
                        name,
                        connected: self.is_connected(port.get_client(), port.get_port()),
                    },
                });
            }
        }
        out.sort_by(|a, b| a.source.name.cmp(&b.source.name));
        out
    }

    /// Wire every source the sequencer offers into this editor's port.
    ///
    /// Every one of them, with no device to choose: which box is on the table is a fact about
    /// tonight, and a binding names a channel and a CC rather than a device — so a controller
    /// plugged in mid-set works the moment Refresh is pressed, with nothing to pick.
    pub fn connect_all(&mut self) {
        for offered in self.offered() {
            if !offered.source.connected {
                self.connect(offered.client, offered.port);
            }
        }
    }

    fn connect(&self, client: i32, port: i32) {
        let subscribe = alsa::seq::PortSubscribe::empty().expect("a subscription");
        subscribe.set_sender(alsa::seq::Addr { client, port });
        subscribe.set_dest(alsa::seq::Addr {
            client: self.client,
            port: self.port,
        });
        if let Err(e) = self.seq.subscribe_port(&subscribe) {
            log::warn!("midi: could not connect {client}:{port}: {e}");
        }
    }

    fn is_connected(&self, client: i32, port: i32) -> bool {
        alsa::seq::PortSubscribeIter::new(
            &self.seq,
            alsa::seq::Addr { client, port },
            alsa::seq::QuerySubsType::READ,
        )
        .any(|s| s.get_dest().client == self.client && s.get_dest().port == self.port)
    }
}

/// The client name this editor takes on the sequencer, and its one input port. `BROWSER` is
/// the second client's, which holds no port and exists only to ask and to connect.
const CLIENT: &str = "supersilvia";
const PORT: &str = "MIDI In";
const BROWSER: &str = "supersilvia (ports)";

fn client_name() -> CString {
    CString::new(CLIENT).expect("a literal with no nul")
}

fn port_name() -> CString {
    CString::new(PORT).expect("a literal with no nul")
}

fn browser_name() -> CString {
    CString::new(BROWSER).expect("a literal with no nul")
}

/// The reader thread: block, parse, post, forever.
///
/// It ends when the queue's other end is dropped, which is `App` going away. A parse that
/// finds nothing this editor reads drops the message rather than posting a kind for it.
fn read(seq: &alsa::Seq, ours: alsa::seq::Addr, tx: &Sender<Wire>) {
    let mut input = seq.input();
    // Every device a message has come from, so a client that exits says which of its ports
    // are gone.
    let mut heard: HashSet<(i32, i32)> = HashSet::new();
    loop {
        // A dropped connection or an interrupted wait is not the end of the thread: the
        // sequencer stays open and the next call blocks again. An error that repeats forever
        // would spin, so it costs a yield.
        let Ok(event) = input.event_input() else {
            std::thread::yield_now();
            continue;
        };
        let wires: Vec<Wire> = match gone(&event, ours) {
            Some(Left::Port(client, port)) => heard
                .take(&(client, port))
                .map(|(c, p)| Wire::Gone(device(c, p)))
                .into_iter()
                .collect(),
            Some(Left::Client(client)) => {
                let ports: Vec<(i32, i32)> = heard
                    .iter()
                    .copied()
                    .filter(|(c, _)| *c == client)
                    .collect();
                ports
                    .into_iter()
                    .map(|(c, p)| {
                        heard.remove(&(c, p));
                        Wire::Gone(device(c, p))
                    })
                    .collect()
            }
            None => {
                let Some(message) = parse(&event) else {
                    continue;
                };
                let from = event.get_source();
                heard.insert((from.client, from.port));
                vec![Wire::Message(device(from.client, from.port), message)]
            }
        };
        for wire in wires {
            if tx.send(wire).is_err() {
                return;
            }
        }
    }
}

/// A device's sequencer address as the synth keys it.
fn device(client: i32, port: i32) -> Device {
    Device((u64::from(client as u32) << 32) | u64::from(port as u32))
}

/// What an announcement says went.
enum Left {
    Port(i32, i32),
    Client(i32),
}

/// The sequencer announcing that a port, a client, or a port's wire into `ours` is gone.
fn gone(event: &alsa::seq::Event<'_>, ours: alsa::seq::Addr) -> Option<Left> {
    match event.get_type() {
        alsa::seq::EventType::PortExit => {
            let addr: alsa::seq::Addr = event.get_data()?;
            Some(Left::Port(addr.client, addr.port))
        }
        alsa::seq::EventType::ClientExit => {
            let addr: alsa::seq::Addr = event.get_data()?;
            Some(Left::Client(addr.client))
        }
        alsa::seq::EventType::PortUnsubscribed => {
            let connect: alsa::seq::Connect = event.get_data()?;
            (connect.dest == ours).then_some(Left::Port(connect.sender.client, connect.sender.port))
        }
        _ => None,
    }
}

/// One sequencer event as this editor reads it, or `None` for everything else on the wire.
fn parse(event: &alsa::seq::Event<'_>) -> Option<Message> {
    match event.get_type() {
        alsa::seq::EventType::Controller => {
            let data: alsa::seq::EvCtrl = event.get_data()?;
            Some(Message {
                channel: data.channel,
                kind: Kind::Control {
                    cc: u8::try_from(data.param).ok()?,
                    value: u8::try_from(data.value.clamp(0, 127)).ok()?,
                },
            })
        }
        alsa::seq::EventType::Noteon | alsa::seq::EventType::Noteoff => {
            let data: alsa::seq::EvNote = event.get_data()?;
            // A note-on at velocity zero is a note-off. Half the hardware in the world sends
            // one, and a gate that only ever opens is the bug it causes.
            let on = event.get_type() == alsa::seq::EventType::Noteon && data.velocity > 0;
            Some(Message {
                channel: data.channel,
                kind: Kind::Note {
                    note: data.note,
                    on,
                },
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The next wire `want` accepts, within two seconds.
    fn wait_for<T>(wires: &Receiver<Wire>, want: impl Fn(Wire) -> Option<T>) -> Option<T> {
        let since = Instant::now();
        while since.elapsed() < Duration::from_secs(2) {
            if let Ok(wire) = wires.recv_timeout(Duration::from_millis(100))
                && let Some(found) = want(wire)
            {
                return Some(found);
            }
        }
        None
    }

    /// **A device that goes away is said gone**, naming the device its messages came from, so
    /// the synth lets go of the notes it held. The device is a sequencer client of the
    /// test's own, wired in by Rescan, sending a note and then closing — the sequencer
    /// announces its port's exit, as it does for a controller pulled out of its socket.
    /// Skipped on a box with no sequencer.
    #[test]
    fn a_source_that_goes_away_is_said_gone() {
        let Ok((mut midi, wires)) = Midi::open() else {
            eprintln!("no sequencer here; skipping");
            return;
        };
        let source = alsa::Seq::open(None, Some(alsa::Direction::Playback), false)
            .expect("a client of the test's own");
        source
            .set_client_name(&CString::new("supersilvia test source").expect("no nul"))
            .ok();
        let port = source
            .create_simple_port(
                &CString::new("out").expect("no nul"),
                alsa::seq::PortCap::READ | alsa::seq::PortCap::SUBS_READ,
                alsa::seq::PortType::MIDI_GENERIC | alsa::seq::PortType::APPLICATION,
            )
            .expect("a port to send from");
        midi.connect_all();

        let mut note = alsa::seq::Event::new(
            alsa::seq::EventType::Noteon,
            &alsa::seq::EvNote {
                channel: 0,
                note: 60,
                velocity: 100,
                off_velocity: 0,
                duration: 0,
            },
        );
        note.set_source(port);
        note.set_subs();
        note.set_direct();
        source
            .event_output_direct(&mut note)
            .expect("the note sent");

        let from = wait_for(&wires, |wire| match wire {
            Wire::Message(device, message)
                if message.kind == (Kind::Note { note: 60, on: true }) =>
            {
                Some(device)
            }
            _ => None,
        })
        .expect("the note arrives, with its device");

        drop(source);
        let gone = wait_for(&wires, |wire| match wire {
            Wire::Gone(device) if device == from => Some(device),
            _ => None,
        });
        assert_eq!(gone, Some(from), "the device that sent is said gone");
    }
}
