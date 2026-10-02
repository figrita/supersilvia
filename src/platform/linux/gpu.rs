// SPDX-License-Identifier: AGPL-3.0-or-later

//! The process's use of the render engine, from the kernel's per-client DRM counters.
//!
//! Every DRM file descriptor has a `drm-engine-render` line in its `/proc/self/fdinfo`: the
//! nanoseconds the engine has spent on that client's work, cumulative. Summed over the
//! process's clients — each counted once, since a duplicated descriptor is the same client —
//! it sees the synth, the editor's painting and every picture window alike, which no query
//! the renderer places can.

use std::collections::HashMap;

use crate::platform::gpu::Read;

/// The kernel counts the engine per client, so the figure is this process's own.
pub const WHOLE_GPU: bool = false;

/// The descriptors that point at a DRM device, by number: what [`Clients::read`] reads.
#[derive(Debug, Default)]
pub struct Clients {
    fds: Vec<String>,
}

impl Clients {
    /// Search the process's descriptors for DRM ones.
    pub fn scan() -> Self {
        let mut fds = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc/self/fd") else {
            return Self { fds };
        };
        for entry in dir.flatten() {
            if std::fs::read_link(entry.path()).is_ok_and(|p| p.starts_with("/dev/dri")) {
                fds.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        Self { fds }
    }

    /// The render engine's nanoseconds summed over every client, and the clients.
    pub fn read(&self) -> Option<Read> {
        let mut clients = Vec::new();
        let mut total = 0;
        for fd in &self.fds {
            let Ok(text) = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")) else {
                continue;
            };
            let Some((client, ns)) = render_engine(&text) else {
                continue;
            };
            if !clients.contains(&client) {
                clients.push(client);
                total += ns;
            }
        }
        if clients.is_empty() {
            return None;
        }
        clients.sort_unstable();
        Some(Read::EngineNs(total, clients))
    }
}

/// The render engine's nanoseconds this process has used, over every DRM client it holds, each
/// once: every descriptor's fdinfo read, for a figure taken now and again later. Zero where the
/// kernel says nothing.
pub fn render_engine_ns() -> u64 {
    let mut seen = HashMap::new();
    let dir = std::fs::read_dir("/proc/self/fdinfo");
    for entry in dir.into_iter().flatten().flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some((client, ns)) = render_engine(&text) {
            seen.insert(client, ns);
        }
    }
    seen.values().sum()
}

/// A DRM fdinfo's client id and its render engine's nanoseconds.
fn render_engine(fdinfo: &str) -> Option<(u64, u64)> {
    let field = |key: &str| {
        fdinfo.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
            value.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    Some((field("drm-client-id")?, field("drm-engine-render")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drm_fdinfo_names_its_client_and_its_render_time() {
        let fdinfo = "pos:\t0\nflags:\t02100002\nmnt_id:\t26\nino:\t1070\n\
                      drm-driver:\ti915\ndrm-client-id:\t41\ndrm-pdev:\t0000:00:02.0\n\
                      drm-engine-render:\t123456789 ns\ndrm-engine-copy:\t0 ns\n";
        assert_eq!(render_engine(fdinfo), Some((41, 123_456_789)));
        assert_eq!(render_engine("pos:\t0\nflags:\t02\n"), None);
    }
}
