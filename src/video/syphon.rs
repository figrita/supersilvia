// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture received over Syphon, from a server another app on the Mac publishes: the
//! directory as a menu, and a [`Receiver`] that keeps a client connected to the server a menu
//! names, for the Main Input and the Syphon node alike.
//!
//! **A server is chosen by its label**, "App – Server", which is what a menu shows and what a
//! project saves: the directory's own ID is new on every run of the other app, and its label
//! is not. A receiver whose server is not there says so and looks again every second, so a
//! server that starts later, or restarts, is taken up without a hand.
//!
//! **Frames need no pipeline.** The client ([`crate::platform::syphon::Inlet`]) writes each
//! frame into the one-frame slot a [`Camera`] adopts, as a screen on a Mac does, and the
//! renderer copies each frame's surface into a texture of its own, since the server draws into
//! the same surface again ([`crate::nodes::Redrawn`]). A frame is laid out bottom row first by
//! Syphon's convention and opaque over black unless the receiver's [`Look`] says otherwise.
//!
//! **Nothing here waits.** The directory is read at most twice a second and the client is
//! asked whether its server is there at most once a second; both are a lock taken for a copy.

use super::{Camera, Source};
use crate::nodes::{Choices, Frame};
use crate::platform::syphon::{Inlet, Look, servers};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How old the directory's listing may be before it is read again.
const LISTING: Duration = Duration::from_millis(500);

/// How often a receiver with no server looks for it, and one with a server asks it is there.
const RETRY: Duration = Duration::from_secs(1);

/// What a Syphon menu offers before anything is listed: nothing chosen.
pub const NONE: Choices = &[("", "None")];

/// The directory's labels, as last read, and when.
static LISTED: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

/// The Syphon node's menu as the last listing made it.
static MENU: Mutex<Choices> = Mutex::new(NONE);

/// Every server on the Mac by its label, in the order the directory heard of them, read again
/// where the last reading is older than half a second. Empty off macOS.
pub fn labels() -> Vec<String> {
    let mut listed = LISTED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, labels)) = listed.as_ref()
        && at.elapsed() < LISTING
    {
        return labels.clone();
    }
    let labels: Vec<String> = servers()
        .iter()
        .map(crate::platform::syphon::Described::label)
        .collect();
    *listed = Some((Instant::now(), labels.clone()));
    labels
}

/// The Syphon node's menu: *None*, then every server listed now.
pub fn menu() -> Choices {
    let labels = labels();
    let mut menu = MENU.lock().unwrap_or_else(PoisonError::into_inner);
    let listed = &menu[1..];
    let same = listed.len() == labels.len()
        && listed
            .iter()
            .zip(&labels)
            .all(|(&(value, _), label)| value == label.as_str());
    if !same {
        // Leaked once a listing that differs, which is a server starting or stopping.
        let leak = |s: &str| &*Box::leak(s.to_owned().into_boxed_str());
        let choices: Vec<_> = NONE
            .iter()
            .copied()
            .chain(labels.iter().map(|l| {
                let l = leak(l);
                (l, l)
            }))
            .collect();
        *menu = Box::leak(choices.into_boxed_slice());
    }
    *menu
}

/// A connection to a server: the camera reading the client's slots, and the client. The
/// camera goes first.
struct Link {
    camera: Camera,
    inlet: Inlet,
    /// When the client was last asked whether its server is there.
    asked: Instant,
}

/// A client kept connected to the server a label names.
pub struct Receiver {
    server: String,
    look: Look,
    link: Option<Link>,
    /// When to look for the server next, while there is no link.
    next_try: Instant,
    /// The last frame received, kept up once the server has gone.
    last: Option<Arc<Frame>>,
    error: Option<String>,
}

impl Receiver {
    /// A receiver for the server labelled `server`, each frame read as `look` says. It looks
    /// for it on the first [`Self::latest`].
    pub fn new(server: &str, look: Look) -> Self {
        Self {
            server: server.to_string(),
            look,
            link: None,
            next_try: Instant::now(),
            last: None,
            error: None,
        }
    }

    /// Whether this receives from `server` as `look` says, which a caller whose choice moved
    /// asks before making another.
    pub fn is(&self, server: &str, look: Look) -> bool {
        self.server == server && self.look == look
    }

    /// The newest frame, or the last one received once the server has gone, or `None` before
    /// any. Connects, and connects again, as the server comes and goes. Never waits.
    pub fn latest(&mut self) -> Option<Arc<Frame>> {
        let now = Instant::now();
        let gone = match &mut self.link {
            Some(link) if now.duration_since(link.asked) >= RETRY => {
                link.asked = now;
                (!link.inlet.is_valid()).then(|| link.camera.error())
            }
            _ => None,
        };
        if let Some(why) = gone {
            self.error = why;
            self.link = None;
            self.next_try = now + RETRY;
        }
        if self.link.is_none() && !self.server.is_empty() && now >= self.next_try {
            self.next_try = now + RETRY;
            self.connect(now);
        }
        if let Some(link) = &mut self.link
            && let Some(frame) = link.camera.latest()
        {
            self.last = Some(frame);
        }
        self.last.clone()
    }

    fn connect(&mut self, now: Instant) {
        let Some(server) = servers().into_iter().find(|s| s.label() == self.server) else {
            self.error = Some(format!("no Syphon server {} is running", self.server));
            return;
        };
        let opened = Inlet::open(&server, self.look).and_then(|inlet| {
            let camera = Camera::open(&Source::Syphon(inlet.stream()), None)?;
            Ok(Link {
                camera,
                inlet,
                asked: now,
            })
        });
        match opened {
            Ok(link) => {
                self.link = Some(link);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Whether a client is connected.
    pub fn connected(&self) -> bool {
        self.link.is_some()
    }

    /// Why nothing is being received, where nothing is.
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// The label of the server it receives from.
    pub fn server(&self) -> &str {
        &self.server
    }
}
