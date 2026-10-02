// SPDX-License-Identifier: AGPL-3.0-or-later

//! The installed font families on macOS, as AppKit's collection of every available font
//! lists them.
//!
//! **`NSFontCollection`, not `NSFontManager`.** The manager's `availableFontFamilies` is the
//! same list, name for name, but objc2 holds the manager to the main thread, and the menu is
//! first asked for on whichever thread draws it first: the editor's, which is the main thread,
//! or a test's, which is not. The collection and its descriptors may be read on any thread.
//!
//! The Text node's letters are pango's, through GStreamer's `textoverlay`, and pango on macOS
//! finds a family through Core Text, which is the list AppKit reads — so a name on this list
//! is one it will draw. `video/text.rs` draws one to hold that.

use objc2_app_kit::NSFontCollection;
use objc2_foundation::NSString;

/// Every installed family's name, sorted without regard to case, each once. `None` where
/// AppKit hands back no descriptors.
pub fn installed() -> Option<Vec<String>> {
    let descriptors =
        NSFontCollection::fontCollectionWithAllAvailableDescriptors().matchingDescriptors()?;
    // AppKit's `NSFontFamilyAttribute` is an extern static, which Rust reads only in `unsafe`;
    // its value is this string, which is also Core Text's `kCTFontFamilyNameAttribute`.
    let family = NSString::from_str("NSFontFamilyAttribute");
    let mut names: Vec<String> = descriptors
        .iter()
        .filter_map(|d| d.objectForKey(&family))
        .filter_map(|name| name.downcast::<NSString>().ok())
        .map(|name| name.to_string())
        .collect();
    names.sort();
    names.dedup();
    names.sort_by_key(|n| n.to_lowercase());
    Some(names)
}
