// SPDX-License-Identifier: AGPL-3.0-or-later

//! The installed font families, as fontconfig lists them.
//!
//! The Text node's letters are pango's, through GStreamer's `textoverlay`, and pango finds a
//! family through the same fontconfig — so a name on this list is one it will draw.

/// Every installed family's first name, sorted without regard to case, each once. `None`
/// where fontconfig cannot be loaded.
pub fn installed() -> Option<Vec<String>> {
    use fontconfig::{FC_FAMILY, Fontconfig, ObjectSet, Pattern, list_fonts};
    let fc = Fontconfig::new()?;
    let mut objects = ObjectSet::new(&fc).ok()?;
    objects.add(FC_FAMILY).ok()?;
    let fonts = list_fonts(&Pattern::new(&fc).ok()?, Some(&objects)).ok()?;
    let mut names: Vec<String> = fonts
        .iter()
        .filter_map(|p| p.get_string(FC_FAMILY).ok().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names.sort_by_key(|n| n.to_lowercase());
    Some(names)
}
