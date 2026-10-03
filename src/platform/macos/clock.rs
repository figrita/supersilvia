// SPDX-License-Identifier: AGPL-3.0-or-later

//! The calling thread's own CPU clock, `CLOCK_THREAD_CPUTIME_ID`, through `rustix`, whose call
//! is safe.

use std::time::Duration;

/// The calling thread's own CPU time: the clock that stands still while the thread is blocked.
pub fn thread_cpu() -> Duration {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
    Duration::new(
        u64::try_from(t.tv_sec).unwrap_or(0),
        u32::try_from(t.tv_nsec).unwrap_or(0),
    )
}
