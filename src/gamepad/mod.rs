// SPDX-License-Identifier: AGPL-3.0-or-later

//! A game controller, read on a thread of its own.
//!
//! The shape every device here takes: the device is polled on its own thread, what it said
//! goes into one slot, and the tick reads the newest and never waits. What is different
//! from the camera and the microphone is that a controller has **edges** as well as levels,
//! so the thread keeps a short queue of them beside the axes — each stamped with the
//! instant it was read, which is how `gamepad`'s tick places a press inside the frame the
//! way `audioin` places a threshold crossing.
//!
//! `gilrs` is the crate underneath, agreed in `proposals/gamepad.md`. On Linux it is evdev
//! with its own hotplug watch, so a controller plugged in mid-session appears without
//! anything being asked; **Rescan** is there for the case where it does not.
//!
//! Nothing here opens a device in a test: [`bench`] installs a pad a test drives by hand,
//! and [`Pad::open`] takes it in place of the real one. `scripts/doctor.sh` therefore never
//! has to ask whether a controller is plugged into the box.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How many axes a pad publishes: two sticks and two triggers.
pub const AXES: usize = 6;

/// The axes, in the order they are published and in the order `gamepad`'s outputs are.
pub const AXIS_LABELS: [&str; AXES] = [
    "Left Stick X",
    "Left Stick Y",
    "Right Stick X",
    "Right Stick Y",
    "Left Trigger",
    "Right Trigger",
];

/// How many buttons a pad publishes: silvia's four faces and the four on the d-pad.
pub const BUTTONS: usize = 8;

/// silvia's dead zone: a stick inside a tenth of the way from center reads exactly zero, so
/// a worn stick does not drift a patch sideways all night.
pub const DEAD_ZONE: f32 = 0.1;

/// How long the thread waits for an event before looking at the stop flag again. Short
/// enough that a node being deleted does not hold the tick up, long enough that a pad
/// nobody is touching costs nothing.
const POLL: std::time::Duration = std::time::Duration::from_millis(4);

/// A bound on the queue, so a controller mashed while the editor is away cannot grow it
/// without limit. Two hundred edges is more than a second of anything a hand can do.
const MAX_EVENTS: usize = 200;

/// One button edge, and when the thread read it.
///
/// `at` is an `Instant` rather than a place in a frame, because the thread does not know
/// what a frame is: the tick turns an age into a moment inside its own frame, exactly as
/// `audioin` turns a sample count into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ev {
    pub button: usize,
    pub down: bool,
    pub at: Instant,
}

/// What the device thread has to say, and the queue of edges it has not been asked for yet.
#[derive(Debug, Default)]
struct State {
    axes: [f32; AXES],
    buttons: [bool; BUTTONS],
    events: Vec<Ev>,
    /// The controller's own name, for the node's status line.
    name: Option<String>,
    /// What went wrong, for the same line.
    error: Option<String>,
    /// How many edges were dropped because nothing read the queue.
    dropped: u64,
}

/// The slot between the device thread and the tick.
#[derive(Debug, Default)]
pub struct Shared(Mutex<State>);

/// One read of the slot: the levels now, and every edge since the last read.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Reading {
    pub axes: [f32; AXES],
    pub events: Vec<Ev>,
    pub name: Option<String>,
    pub error: Option<String>,
    pub dropped: u64,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Everything since the last read, and the levels as they stand. The queue is emptied:
    /// an edge is delivered once.
    pub fn take(&self) -> Reading {
        let mut state = self.lock();
        Reading {
            axes: state.axes,
            events: std::mem::take(&mut state.events),
            name: state.name.clone(),
            error: state.error.clone(),
            dropped: std::mem::take(&mut state.dropped),
        }
    }

    /// Move an axis, as the device or a test does. Index is [`AXIS_LABELS`]'s.
    pub fn set_axis(&self, axis: usize, value: f32) {
        let mut state = self.lock();
        if let Some(slot) = state.axes.get_mut(axis) {
            *slot = value;
        }
    }

    /// Press or release a button, as the device or a test does. Nothing is queued where the
    /// button is already in that state, so a device repeating itself is not two edges.
    pub fn press(&self, button: usize, down: bool) {
        self.press_at(button, down, Instant::now());
    }

    /// [`Self::press`], at a known moment — which is what the device thread has.
    pub fn press_at(&self, button: usize, down: bool, at: Instant) {
        let mut state = self.lock();
        let Some(held) = state.buttons.get_mut(button) else {
            return;
        };
        if *held == down {
            return;
        }
        *held = down;
        if state.events.len() >= MAX_EVENTS {
            state.dropped += 1;
            state.events.remove(0);
        }
        state.events.push(Ev { button, down, at });
    }

    fn say(&self, name: Option<String>, error: Option<String>) {
        let mut state = self.lock();
        state.name = name;
        state.error = error;
    }

    /// Every button up and every axis at rest, with the edges that says. What a controller
    /// unplugging means, so nothing is left holding a gate open.
    fn release_all(&self) {
        let held: Vec<usize> = {
            let state = self.lock();
            (0..BUTTONS).filter(|b| state.buttons[*b]).collect()
        };
        for button in held {
            self.press(button, false);
        }
        let mut state = self.lock();
        state.axes = [0.0; AXES];
    }
}

/// A controller, or the reason there is none. Dropping it stops the thread.
pub struct Pad {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Pad {
    /// Open the controller `which` names — `"auto"` for the first one plugged in, or a
    /// one-based index among the pads gilrs knows about.
    ///
    /// A pad installed by [`bench`] is taken in place of the device, which is how a test
    /// drives a button with nothing plugged into the box.
    pub fn open(which: &str) -> Self {
        if let Some(shared) = bench_pad() {
            return Self {
                shared,
                stop: Arc::new(AtomicBool::new(false)),
                handle: None,
            };
        }
        let shared: Arc<Shared> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (theirs, their_stop) = (Arc::clone(&shared), Arc::clone(&stop));
        let wanted = wanted_index(which);
        let handle = std::thread::Builder::new()
            .name("gamepad".into())
            .spawn(move || run(&theirs, &their_stop, wanted))
            .map_err(|e| {
                shared.say(None, Some(format!("no controller thread: {e}")));
                log::error!("the gamepad thread could not be started: {e}");
            })
            .ok();
        Self {
            shared,
            stop,
            handle,
        }
    }

    /// What the thread has said since the last read.
    pub fn take(&self) -> Reading {
        self.shared.take()
    }

    /// The slot itself, for a test holding both ends.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The one-based index an option names, or `None` for *Auto*.
fn wanted_index(which: &str) -> Option<usize> {
    match which {
        "" | "auto" => None,
        other => other.parse::<usize>().ok().filter(|n| *n >= 1),
    }
}

/// The pad a test installed, if there is one.
fn bench_pad() -> Option<Arc<Shared>> {
    BENCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

static BENCH: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

/// Install a pad every [`Pad::open`] from here on takes in place of a device, and hand it
/// back for the test to drive.
///
/// **For tests.** It is what lets `tests/actions.rs` push a button down and up with nothing
/// plugged into the machine, and what keeps `scripts/doctor.sh` from having to ask for a
/// controller.
pub fn bench() -> Arc<Shared> {
    let shared: Arc<Shared> = Arc::default();
    shared.say(Some("Test Controller".to_string()), None);
    *BENCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&shared));
    shared
}

/// Take the bench pad away again, so a later `Pad::open` looks for a device.
pub fn clear_bench() {
    *BENCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The six axes gilrs reports, in [`AXIS_LABELS`]' order, with silvia's dead zone on the
/// four stick axes and its sign on the two vertical ones.
///
/// **Up is positive on both sticks**, which is worldspace's convention and the opposite of
/// the device's. A trigger is 0 to 1 and has no dead zone: resting is the bottom of its
/// range rather than the middle of it.
pub fn shape(raw: [f32; AXES]) -> [f32; AXES] {
    let dead = |v: f32| if v.abs() < DEAD_ZONE { 0.0 } else { v };
    [
        dead(raw[0]),
        -dead(raw[1]),
        dead(raw[2]),
        -dead(raw[3]),
        raw[4].clamp(0.0, 1.0),
        raw[5].clamp(0.0, 1.0),
    ]
}

/// The thread: gilrs' own event pump, the axes it holds, and the edges it saw.
fn run(shared: &Arc<Shared>, stop: &Arc<AtomicBool>, wanted: Option<usize>) {
    let mut gilrs = match gilrs::Gilrs::new() {
        Ok(g) => g,
        Err(err) => {
            shared.say(None, Some(format!("no controller: {err}")));
            return;
        }
    };
    let mut chosen: Option<gilrs::GamepadId> = None;
    while !stop.load(Ordering::Relaxed) {
        // Drain whatever arrived. gilrs' own connect and disconnect events are in here, so
        // a controller plugged in mid-session is noticed without a rescan.
        while gilrs.next_event_blocking(Some(POLL)).is_some() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
        }
        let picked = pick(&gilrs, wanted);
        if picked != chosen {
            // The pad went, or another one took its place: nothing may be left held.
            shared.release_all();
            chosen = picked;
        }
        match chosen.and_then(|id| gilrs.connected_gamepad(id)) {
            Some(pad) => {
                shared.say(Some(pad.name().to_string()), None);
                let raw = [
                    axis(&pad, gilrs::Axis::LeftStickX),
                    axis(&pad, gilrs::Axis::LeftStickY),
                    axis(&pad, gilrs::Axis::RightStickX),
                    axis(&pad, gilrs::Axis::RightStickY),
                    button_value(&pad, gilrs::Button::LeftTrigger2),
                    button_value(&pad, gilrs::Button::RightTrigger2),
                ];
                for (i, value) in shape(raw).into_iter().enumerate() {
                    shared.set_axis(i, value);
                }
                let now = Instant::now();
                for (i, code) in GILRS_BUTTONS.into_iter().enumerate() {
                    shared.press_at(i, pad.is_pressed(code), now);
                }
            }
            None => {
                shared.say(
                    None,
                    Some("no controller found — plug one in and press a button".to_string()),
                );
            }
        }
    }
    shared.release_all();
}

/// Which pad the option means: the *n*th gilrs knows about, or the first connected one.
fn pick(gilrs: &gilrs::Gilrs, wanted: Option<usize>) -> Option<gilrs::GamepadId> {
    let mut connected = gilrs.gamepads().filter(|(_, pad)| pad.is_connected());
    match wanted {
        None => connected.next().map(|(id, _)| id),
        Some(n) => connected.nth(n - 1).map(|(id, _)| id),
    }
}

fn axis(pad: &gilrs::Gamepad<'_>, code: gilrs::Axis) -> f32 {
    pad.axis_data(code)
        .map_or(0.0, gilrs::ev::state::AxisData::value)
}

fn button_value(pad: &gilrs::Gamepad<'_>, code: gilrs::Button) -> f32 {
    pad.button_data(code)
        .map_or(0.0, gilrs::ev::state::ButtonData::value)
}

/// The eight buttons, in the order `gamepad`'s outputs are: silvia's four faces, then the
/// d-pad clockwise from the top.
const GILRS_BUTTONS: [gilrs::Button; BUTTONS] = [
    gilrs::Button::South,
    gilrs::Button::East,
    gilrs::Button::West,
    gilrs::Button::North,
    gilrs::Button::DPadUp,
    gilrs::Button::DPadDown,
    gilrs::Button::DPadLeft,
    gilrs::Button::DPadRight,
];

#[cfg(test)]
mod tests {
    use super::{Ev, Shared, shape};

    /// silvia's dead zone on the sticks and nowhere else, and up positive on both.
    #[test]
    fn a_resting_stick_reads_zero_and_up_is_positive() {
        let shaped = shape([0.05, -0.05, 0.4, -0.6, 0.25, 1.0]);
        assert_eq!(shaped[0], 0.0, "inside the dead zone");
        assert_eq!(shaped[1], 0.0);
        assert!((shaped[2] - 0.4).abs() < 1e-6, "outside it, untouched");
        assert!((shaped[3] - 0.6).abs() < 1e-6, "the device's down is up");
        assert!(
            (shaped[4] - 0.25).abs() < 1e-6,
            "a trigger has no dead zone"
        );
    }

    /// A button repeating itself is one edge, and each edge is handed over once.
    #[test]
    fn an_edge_is_delivered_once() {
        let shared = Shared::default();
        shared.press(0, true);
        shared.press(0, true);
        shared.press(0, false);
        let read = shared.take();
        let seen: Vec<(usize, bool)> = read
            .events
            .iter()
            .map(|e: &Ev| (e.button, e.down))
            .collect();
        assert_eq!(seen, vec![(0, true), (0, false)]);
        assert!(shared.take().events.is_empty(), "the queue was emptied");
    }

    /// A controller that goes takes its held buttons with it, so nothing downstream is left
    /// holding a gate open.
    #[test]
    fn a_pad_going_away_releases_what_it_held() {
        let shared = Shared::default();
        shared.press(3, true);
        let _ = shared.take();
        shared.release_all();
        let read = shared.take();
        assert_eq!(read.events.len(), 1);
        assert!(!read.events[0].down);
    }
}
