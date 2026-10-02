// SPDX-License-Identifier: AGPL-3.0-or-later

//! The one runtime the desktop portal is spoken to on.
//!
//! Everything that talks to **xdg-desktop-portal** — the file dialogs through `rfd`, screen
//! capture through `ashpd` directly — ends up on one D-Bus connection, because `ashpd` caches
//! it in a process-wide `OnceLock`. That connection has a background task that pumps the
//! socket, and under zbus's tokio feature that task lives on whichever tokio runtime happened
//! to be current when the connection was made.
//!
//! **So the runtime that makes it has to outlive every later call**, and nothing may block it.
//! Getting that wrong is not a hang at the call site, which is what makes it worth a module of
//! its own: the connection simply stops being serviced, and then a screen capture that had been
//! running for a minute dies, the next portal call reuses the dead connection and never
//! answers, and everything downstream of it goes black. That was a real afternoon.
//!
//! One runtime, made once, never dropped, with its own worker thread — so it is pumping
//! whether or not anybody is inside `block_on` — and every portal conversation either spawned
//! onto it (screen capture) or run under its guard (`rfd`, which brings no runtime of its own
//! because it drives its futures with `pollster`).

use std::sync::OnceLock;

/// The runtime, built on first use and kept for the life of the process.
///
/// Multi-threaded with a single worker: a `current_thread` runtime only makes progress while
/// someone calls `block_on`, which is exactly the assumption that fails here. One worker is
/// plenty — the total load is a dialog now and then and one D-Bus socket.
pub(super) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("portal")
            .enable_all()
            .build()
            .expect("a tokio runtime for the desktop portal")
    })
}

/// One portal conversation, run to its end on [`runtime`] and with everything it made dropped
/// there too, from any thread that is not the runtime's.
///
/// **What it answers must own nothing of zbus's.** A proxy, a request or a signal stream
/// removes its D-Bus match rule when dropped, and zbus does that by spawning onto the tokio
/// runtime that is current — so one dropped on a plain thread after `block_on` has returned
/// panics with "there is no reactor running", and the stream dropped while that panic unwinds
/// panics again, which aborts the whole app. Map the answer to plain data inside the future.
/// The runtime is also entered for the whole call, so nothing a panic leaves to drop on the
/// way out is outside it either.
pub(super) fn converse<T>(future: impl std::future::Future<Output = T>) -> T {
    let _inside = runtime().enter();
    runtime().block_on(future)
}

#[cfg(test)]
mod tests {
    /// A value whose drop spawns onto the current tokio runtime, as a zbus proxy's and signal
    /// stream's do, made and dropped inside a conversation from a plain thread, does not panic.
    /// Dropped after a bare `block_on` had returned it did, twice, and the app aborted.
    #[test]
    fn what_a_conversation_makes_is_dropped_on_the_runtime() {
        struct SpawnsOnDrop;
        impl Drop for SpawnsOnDrop {
            fn drop(&mut self) {
                tokio::spawn(async {});
            }
        }
        std::thread::spawn(|| {
            super::converse(async {
                let made = SpawnsOnDrop;
                drop(made);
            });
        })
        .join()
        .expect("nothing a conversation made panicked when it was dropped");
        // The shape that aborted the app: the value handed back out of the runtime and
        // dropped on the plain thread.
        let outside = std::thread::spawn(|| drop(super::converse(async { SpawnsOnDrop }))).join();
        assert!(
            outside.is_err(),
            "dropped outside the runtime, it panics: which is why a conversation must not return it"
        );
    }

    /// Two portal conversations in a row, from two different threads, both answering.
    ///
    /// This is the regression. `ashpd` caches one D-Bus connection for the process and its
    /// socket is pumped by a task on the runtime that made it, so a runtime that goes away
    /// between calls — or one a future is blocking — leaves every later call talking to a
    /// connection nobody is reading. It does not fail loudly: the second call simply never
    /// answers. Screen capture died a minute in, *Choose another screen…* did nothing at all,
    /// and everything downstream went black.
    ///
    /// `available_source_types` is the right probe because it is a plain property read: no
    /// dialog, nothing for a person to answer, and it goes over the one connection that
    /// matters. Skipped where there is no portal to ask, the way the DMA-BUF tests skip a
    /// machine with no exporter.
    #[test]
    fn the_portal_answers_twice_from_two_threads() {
        let ask = || {
            super::runtime().block_on(async {
                ashpd::desktop::screencast::Screencast::new()
                    .await
                    .ok()?
                    .available_source_types()
                    .await
                    .ok()
            })
        };
        let Some(first) = std::thread::spawn(ask).join().expect("the thread ran") else {
            eprintln!("no screencast portal here; skipping");
            return;
        };
        let second = std::thread::spawn(ask)
            .join()
            .expect("the thread ran")
            .expect("the second call answers too: the connection is still being pumped");
        assert_eq!(
            first, second,
            "the same portal, over the same cached connection"
        );
    }
}
