// SPDX-License-Identifier: AGPL-3.0-or-later

//! The installed font families on Windows, as DirectWrite's system font collection lists them.
//!
//! The Text node's letters are pango's, through GStreamer's `textoverlay`, and pango on
//! Windows finds a family through the same DirectWrite collection — so a name on this list is
//! one it will draw. A family is named in English where it has an English name, which is the
//! name pango matches whatever the desktop's language, and by its first name otherwise.

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
    IDWriteLocalizedStrings,
};
use windows::core::{BOOL, w};

/// Every installed family's name, sorted without regard to case, each once. `None` where
/// DirectWrite cannot be asked.
pub fn installed() -> Option<Vec<String>> {
    // SAFETY: the shared factory is DirectWrite's own, made for any thread, and comes back as
    // the interface it is asked for.
    let factory: IDWriteFactory =
        unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.ok()?;
    let mut collection: Option<IDWriteFontCollection> = None;
    // SAFETY: the out pointer is to a local that outlives the call, which fills it on success.
    unsafe { factory.GetSystemFontCollection(&raw mut collection, false) }.ok()?;
    let collection = collection?;
    // SAFETY: a method of a live collection, taking nothing.
    let count = unsafe { collection.GetFontFamilyCount() };
    let mut names: Vec<String> = (0..count)
        .filter_map(|index| {
            // SAFETY: `index` is below the count the same collection gave.
            let family = unsafe { collection.GetFontFamily(index) }.ok()?;
            // SAFETY: a method of a live family, handing back an interface of its own.
            let names = unsafe { family.GetFamilyNames() }.ok()?;
            english_or_first(&names)
        })
        .collect();
    names.sort();
    names.dedup();
    names.sort_by_key(|n| n.to_lowercase());
    Some(names)
}

/// A family's English name, or its first one where it has none.
fn english_or_first(names: &IDWriteLocalizedStrings) -> Option<String> {
    let mut index = 0;
    let mut exists = BOOL(0);
    // SAFETY: the locale is a NUL-terminated literal, and the two out pointers are to locals
    // that outlive the call.
    unsafe { names.FindLocaleName(w!("en-us"), &raw mut index, &raw mut exists) }.ok()?;
    if !exists.as_bool() {
        index = 0;
    }
    // SAFETY: `index` is one the same list found, or its first, and every list has one.
    let length = unsafe { names.GetStringLength(index) }.ok()?;
    let mut text = vec![0u16; length as usize + 1];
    // SAFETY: the buffer holds the length the list gave and the NUL it writes after it.
    unsafe { names.GetString(index, &mut text) }.ok()?;
    let name = String::from_utf16_lossy(&text[..length as usize]);
    (!name.is_empty()).then_some(name)
}
