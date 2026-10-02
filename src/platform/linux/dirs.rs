// SPDX-License-Identifier: AGPL-3.0-or-later

//! This user's folders on Linux: `$XDG_CONFIG_HOME` for preferences, falling back to
//! `$HOME/.config` when it is unset or not absolute, as the base directory specification says;
//! `$XDG_DATA_HOME` for the log, falling back to `$HOME/.local/share` the same way; and the
//! documents folder for projects, which is `XDG_DOCUMENTS_DIR` as
//! `$XDG_CONFIG_HOME/user-dirs.dirs` names it — the file `xdg-user-dirs` writes in the
//! person's own language, `"$HOME/Dokumente"` on a German desktop — and `$HOME/Documents`
//! where it names none.

use std::path::{Path, PathBuf};

/// What a person sets for [`documents`] to answer.
pub const DOCUMENTS_UNSET: &str = "set HOME";

/// What to do about a folder this user is not allowed into, after saying so: nothing more to
/// say here, where it is the folder's own permissions.
pub const DENIED_HINT: Option<&str> = None;

/// What a folder's name cannot hold: the separator, and the byte that ends a C string.
pub const NOT_IN_NAMES: &[char] = &['/', '\0'];

/// `$XDG_CONFIG_HOME`, or `$HOME/.config`. `None` with neither set.
pub fn config() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// `$XDG_DATA_HOME`, or `$HOME/.local/share`. `None` with neither set.
pub fn data() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
}

/// The documents folder: `XDG_DOCUMENTS_DIR` from `user-dirs.dirs`, or `$HOME/Documents`.
/// `None` without `$HOME`.
pub fn documents() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(documents_from(config().as_deref(), &home))
}

/// The documents folder, given the configuration folder `user-dirs.dirs` is read from and the
/// home folder `$HOME` in it stands for: the file's `XDG_DOCUMENTS_DIR`, or `Documents` in the
/// home folder where there is no file or it names none this reads.
fn documents_from(config: Option<&Path>, home: &Path) -> PathBuf {
    config
        .and_then(|dir| std::fs::read_to_string(dir.join("user-dirs.dirs")).ok())
        .and_then(|text| documents_in(&text, home))
        .unwrap_or_else(|| home.join("Documents"))
}

/// `XDG_DOCUMENTS_DIR` out of a `user-dirs.dirs`, the last one set.
///
/// The file is shell assignments, one a line: `XDG_DOCUMENTS_DIR="$HOME/Documents"`. A value
/// is `$HOME` or a path under it, or an absolute path; anything else — a relative path — is
/// not one the specification allows, and is ignored.
fn documents_in(text: &str, home: &Path) -> Option<PathBuf> {
    text.lines().rev().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        if key.trim() != "XDG_DOCUMENTS_DIR" {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value)
            .replace("\\\"", "\"");
        if let Some(rest) = value.strip_prefix("$HOME") {
            return match rest.strip_prefix('/') {
                Some(under) => Some(home.join(under)),
                None => rest.is_empty().then(|| home.to_path_buf()),
            };
        }
        let path = PathBuf::from(value);
        path.is_absolute().then_some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/ana")
    }

    /// What `xdg-user-dirs` writes on a German desktop, comments and all.
    #[test]
    fn the_documents_folder_is_the_one_user_dirs_names() {
        let text = "# This file is written by xdg-user-dirs-update\n\
                    XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
                    XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n\
                    XDG_MUSIC_DIR=\"$HOME/Musik\"\n";
        assert_eq!(
            documents_in(text, &home()),
            Some(PathBuf::from("/home/ana/Dokumente"))
        );
    }

    #[test]
    fn an_absolute_value_is_taken_as_it_is_and_a_relative_one_is_ignored() {
        assert_eq!(
            documents_in("XDG_DOCUMENTS_DIR=\"/data/docs\"", &home()),
            Some(PathBuf::from("/data/docs"))
        );
        assert_eq!(
            documents_in("XDG_DOCUMENTS_DIR=\"Dokumente\"", &home()),
            None
        );
        assert_eq!(
            documents_in("XDG_DOCUMENTS_DIR=$HOME", &home()),
            Some(home()),
            "unquoted, and the home folder itself"
        );
        assert_eq!(
            documents_in("XDG_DOCUMENTS_DIR=\"$HOMEWORK/x\"", &home()),
            None,
            "a variable that only starts like $HOME is not it"
        );
        assert_eq!(documents_in("XDG_MUSIC_DIR=\"$HOME/Musik\"", &home()), None);
    }

    /// No file, or a file naming nothing this reads, is `Documents` in the home folder.
    #[test]
    fn without_a_file_it_is_documents_in_the_home_folder() {
        let dir = std::env::temp_dir().join(format!("ssw-dirs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fallback = home().join("Documents");
        assert_eq!(documents_from(None, &home()), fallback);
        assert_eq!(documents_from(Some(&dir), &home()), fallback, "no file");

        std::fs::write(dir.join("user-dirs.dirs"), "XDG_DOCUMENTS_DIR=\"docs\"\n").unwrap();
        assert_eq!(
            documents_from(Some(&dir), &home()),
            fallback,
            "a relative value"
        );

        std::fs::write(
            dir.join("user-dirs.dirs"),
            "XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n",
        )
        .unwrap();
        assert_eq!(
            documents_from(Some(&dir), &home()),
            home().join("Dokumente")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
