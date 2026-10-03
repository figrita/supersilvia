// SPDX-License-Identifier: AGPL-3.0-or-later

//! The calling thread's own CPU time on Windows: `GetThreadTimes`' kernel and user times added
//! together, which the scheduler counts in 100-nanosecond units and advances only while the
//! thread runs.

use std::time::Duration;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};

/// The calling thread's own CPU time: the clock that stands still while the thread is blocked.
/// Zero where Windows will not say.
pub fn thread_cpu() -> Duration {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: `GetCurrentThread` is a pseudo-handle that is always valid for the calling
    // thread and needs no closing, and the four pointers are to locals that outlive the call.
    let read = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if read.is_err() {
        return Duration::ZERO;
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let hundreds = ticks(kernel) + ticks(user);
    Duration::new(hundreds / 10_000_000, (hundreds % 10_000_000) as u32 * 100)
}
