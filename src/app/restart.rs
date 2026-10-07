// SPDX-License-Identifier: AGPL-3.0-or-later

//! Restart to apply: the app closing as Quit closes it, then started again — the same
//! executable, the same environment, on the project that was open.
//!
//! **The close is Quit's**, `Pending::Restart` through the same confirm, so unsaved edits and
//! the show going out are asked about exactly as on Quit, and a Cancel restarts nothing. The
//! editor only records the request in a [`Restart`] that `main` holds a clone of; `main` starts
//! the new process once eframe has returned — every thread stopped by `App::on_exit`, the
//! window, its display connection and the inspection port gone — and just before the process
//! ends, so the two runs never hold the same thing at once. A test reads [`Restart::asked`] and
//! nothing is started. The log's lock is the one other thing a run holds past its window:
//! `main` writes the closing line and lets go of it first (`crashlog::release`), so the new
//! run takes the main log over rather than writing one beside it.
//!
//! **The executable** is the AppImage itself where one is running (`$APPIMAGE`), since the
//! binary inside it is on a mount that goes with this process; otherwise
//! [`std::env::current_exe`], which on Linux names a binary rebuilt since it started with
//! ` (deleted)` after it, and then the path without that is the one started.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The variable an AppImage's runtime sets to the image's own path.
const APPIMAGE_ENV: &str = "APPIMAGE";

/// Whether the app is to start again once it has closed, and with what arguments. Cloned
/// between the editor, which asks, and `main`, which starts it.
#[derive(Clone, Debug, Default)]
pub struct Restart(Arc<Mutex<Option<Vec<OsString>>>>);

impl Restart {
    /// Start again with `args` once this run has closed.
    pub(super) fn ask(&self, args: Vec<OsString>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(args);
    }

    /// Whether a restart was asked for.
    pub fn asked(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Start the new process, where a restart was asked for: `Ok(false)` where none was.
    /// Called by `main` alone, after eframe has returned and the log is let go of; it logs
    /// nothing, since the new run empties the log.
    ///
    /// # Errors
    /// Where the executable cannot be found or started.
    pub fn relaunch(&self) -> std::io::Result<bool> {
        let Some(args) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return Ok(false);
        };
        std::process::Command::new(executable()?)
            .args(args)
            .spawn()?;
        Ok(true)
    }
}

/// The arguments the new run starts with: the open project's folder, where it is a project on
/// disk, and otherwise none, so it opens on the most recent project as any start does. A path
/// to open is the one argument past the flags (`main`), and the project open now is the one to
/// come back to — not a loose workspace the first start imported, which would be imported
/// again.
pub fn arguments(project: Option<&Path>) -> Vec<OsString> {
    project
        .map(|root| vec![root.as_os_str().to_owned()])
        .unwrap_or_default()
}

/// The executable to start again: the AppImage where one is running, and otherwise this
/// process's own.
fn executable() -> std::io::Result<PathBuf> {
    if let Some(image) = std::env::var_os(APPIMAGE_ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(image));
    }
    Ok(undeleted(std::env::current_exe()?))
}

/// `exe` without the ` (deleted)` Linux puts after a binary replaced since it started, where
/// that is what it carries and the file under the name without it is there.
fn undeleted(exe: PathBuf) -> PathBuf {
    if exe.exists() {
        return exe;
    }
    exe.to_str()
        .and_then(|s| s.strip_suffix(" (deleted)"))
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .unwrap_or(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_is_asked_once_and_taken_by_the_relaunch() {
        let restart = Restart::default();
        let main = restart.clone();
        assert!(!main.asked());
        assert!(!main.relaunch().unwrap(), "nothing asked, nothing started");
        restart.ask(arguments(None));
        assert!(main.asked(), "the clone main holds sees it");
    }

    #[test]
    fn the_new_run_opens_the_project_that_was_open() {
        let root = Path::new("/home/ana/supersilvia/friday");
        assert_eq!(arguments(Some(root)), vec![OsString::from(root)]);
        assert!(arguments(None).is_empty());
    }

    #[test]
    fn a_replaced_binary_is_started_from_the_name_it_had() {
        let dir = std::env::temp_dir().join(format!("ssw-restart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("supersilvia");
        std::fs::write(&exe, b"").unwrap();
        let deleted = PathBuf::from(format!("{} (deleted)", exe.display()));
        assert_eq!(undeleted(deleted), exe);
        assert_eq!(undeleted(exe.clone()), exe);
        let gone = dir.join("gone (deleted)");
        assert_eq!(
            undeleted(gone.clone()),
            gone,
            "nothing there: left as it was"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
