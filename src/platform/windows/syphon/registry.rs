// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spout's directory, to the byte: the list of sender names, each sender's information, the
//! active sender, and the access mutex and frame count a sender and its receivers share. Every
//! one is a named Win32 object in the session's namespace, made by its ANSI name with default
//! security, as Spout 2.007's `SpoutSenderNames`, `SpoutSharedMemory` and `SpoutFrameCount`
//! make them; `notes` in the studio carry the protocol with its citations.
//!
//! **The list** is the map `SpoutSenderNames`, 256 bytes a name, written in byte order with a
//! NUL after the last, behind its `SpoutSenderNames_mutex`. Whoever made the map first fixed
//! its size, so how many names it holds is the smaller of Spout's `MaxSenders` setting and what
//! the map really is.
//!
//! **A sender's information** is a 280-byte map named the sender's name — its texture's
//! legacy share handle, size, DXGI format, a usage of zero, the executable's path and a partner
//! ID of zero — which the sender keeps open for as long as it lives: the map's existence is
//! the sender's. So a receiver opens it, reads it and closes it on every look, and never holds
//! it. A sender's map is made and written before its name is listed, so no receiver finds a
//! name with nothing behind it.
//!
//! **A mutex left abandoned is held.** `WaitForSingleObject` hands an abandoned mutex to its
//! waiter, which then owns it, so it is released like any other; Spout leaves it owned.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::{CString, c_void};
use std::sync::{Mutex, OnceLock, PoisonError};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::System::Memory::{
    CreateFileMappingA, FILE_MAP_ALL_ACCESS, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS,
    MapViewOfFile, OpenFileMappingA, PAGE_READWRITE, UnmapViewOfFile, VirtualQuery,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueA};
use windows::Win32::System::Threading::{
    CreateMutexA, CreateSemaphoreA, GetCurrentProcess, PROCESS_NAME_WIN32,
    QueryFullProcessImageNameA, ReleaseMutex, ReleaseSemaphore, WaitForSingleObject,
};
use windows::core::{PCSTR, PSTR};

/// How long a lock is waited for, in milliseconds: Spout's own.
pub(super) const WAIT_MS: u32 = 67;
/// The list's map.
const LIST: &[u8] = b"SpoutSenderNames";
/// The active sender's map.
const ACTIVE: &[u8] = b"ActiveSenderName";
/// Bytes a name takes in the list, its NUL included.
const SLOT: usize = 256;
/// How many names Spout lists where its setting says nothing.
const MAX_SENDERS: usize = 64;
/// A sender's information, in bytes.
const INFO: usize = 280;
/// The longest name this publishes: a slot, less its NUL, the longest suffix a sender's
/// derived objects add (`_SpoutAccessMutex`) and room for a ` 999` of its own.
pub(super) const LONGEST: usize = SLOT - 1 - 20 - 4;

/// A kernel object's handle, closed when it drops.
pub(super) struct Handle(HANDLE);

// SAFETY: a kernel handle is a process-wide index, used from any thread.
unsafe impl Send for Handle {}
// SAFETY: as above; nothing here mutates through `&Handle`.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle this value owns, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A mutex held, released when it drops.
pub(super) struct Locked<'a>(&'a Handle);

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: a mutex this thread acquired in `acquire`.
        let _ = unsafe { ReleaseMutex(self.0.0) };
    }
}

/// `mutex`, waited for at most `ms` milliseconds; abandoned is held.
fn acquire(mutex: &Handle, ms: u32) -> Option<Locked<'_>> {
    // SAFETY: a live mutex handle.
    let waited = unsafe { WaitForSingleObject(mutex.0, ms) };
    (waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED).then_some(Locked(mutex))
}

/// `name` as the C string the `…A` calls take.
fn c_name(name: &[u8]) -> Result<CString, String> {
    CString::new(name.to_vec()).map_err(|_| "a Spout name holding a NUL".to_owned())
}

/// `name` with `suffix` after it.
fn derived(name: &[u8], suffix: &str) -> Vec<u8> {
    [name, suffix.as_bytes()].concat()
}

/// The mutex `name`, made where it does not exist and opened where it does.
fn mutex(name: &[u8]) -> Result<Handle, String> {
    let c = c_name(name)?;
    // SAFETY: a NUL-terminated name that outlives the call, no security attributes, unowned.
    unsafe { CreateMutexA(None, false, PCSTR(c.as_ptr().cast())) }
        .map(Handle)
        .map_err(|e| format!("the mutex {}: {e}", shown(name)))
}

/// A name's bytes as text, one character a byte, for a message.
fn shown(name: &[u8]) -> String {
    name.iter().map(|&b| char::from(b)).collect()
}

/// A named map of shared memory, mapped whole, and the mutex that guards it.
pub(super) struct Memory {
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    /// What the view covers: the map's size rounded up to a page.
    len: usize,
    mutex: Handle,
    _map: Handle,
}

// SAFETY: the view is shared memory every process reads and writes under the map's mutex;
// this value only copies in and out of it, from any thread.
unsafe impl Send for Memory {}
// SAFETY: as above.
unsafe impl Sync for Memory {}

impl Memory {
    /// The map `name` of `size` bytes, made where it does not exist — and then zeroed — and
    /// opened, at the size its maker gave it, where it does; and whether it did.
    fn create(name: &[u8], size: u32) -> Result<(Self, bool), String> {
        let c = c_name(name)?;
        // SAFETY: memory backed by the paging file, no security attributes, and a
        // NUL-terminated name that outlives the call.
        let map = unsafe {
            CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size,
                PCSTR(c.as_ptr().cast()),
            )
        }
        .map_err(|e| format!("the map {}: {e}", shown(name)))?;
        // SAFETY: read straight after the call it describes.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok((Self::mapped(Handle(map), name)?, existed))
    }

    /// The map `name`, or `None` where there is none.
    fn open(name: &[u8]) -> Result<Option<Self>, String> {
        let c = c_name(name)?;
        // SAFETY: a NUL-terminated name that outlives the call.
        match unsafe { OpenFileMappingA(FILE_MAP_ALL_ACCESS.0, false, PCSTR(c.as_ptr().cast())) } {
            Ok(map) => Self::mapped(Handle(map), name).map(Some),
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(None),
            Err(e) => Err(format!("the map {} would not open: {e}", shown(name))),
        }
    }

    /// `map` mapped whole, with its `<name>_mutex`.
    fn mapped(map: Handle, name: &[u8]) -> Result<Self, String> {
        // SAFETY: a live map handle, mapped whole for reading and writing.
        let view = unsafe { MapViewOfFile(map.0, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
        if view.Value.is_null() {
            return Err(format!(
                "the map {} would not map: {}",
                shown(name),
                windows::core::Error::from_thread()
            ));
        }
        let mut region = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: the view just mapped, described into a structure of the size passed.
        let written = unsafe {
            VirtualQuery(
                Some(view.Value.cast_const()),
                &raw mut region,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let memory = Self {
            view,
            len: if written == 0 { 0 } else { region.RegionSize },
            mutex: mutex(&derived(name, "_mutex"))?,
            _map: map,
        };
        Ok(memory)
    }

    /// The map's mutex, waited for as Spout waits.
    fn lock(&self) -> Option<Locked<'_>> {
        acquire(&self.mutex, WAIT_MS)
    }

    /// `len` bytes from `at`, or as many of them as the view holds.
    fn read(&self, at: usize, len: usize) -> Vec<u8> {
        let len = len.min(self.len.saturating_sub(at));
        let mut out = vec![0; len];
        // SAFETY: `len` bytes from `at` lie inside the view, which is mapped until `self`
        // drops; another process may write them, which a byte copy tolerates.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.view.Value.cast::<u8>().add(at),
                out.as_mut_ptr(),
                len,
            );
        }
        out
    }

    /// `bytes` written at `at`, or as many of them as the view holds.
    fn write(&self, at: usize, bytes: &[u8]) {
        let len = bytes.len().min(self.len.saturating_sub(at));
        // SAFETY: `len` bytes from `at` lie inside the view, mapped for writing until `self`
        // drops.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.view.Value.cast::<u8>().add(at),
                len,
            );
        }
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: the view mapped in `mapped`, unmapped once; the handles close after.
        let _ = unsafe { UnmapViewOfFile(self.view) };
    }
}

/// How many names Spout's setting lists, `HKCU\Software\Leading Edge\Spout\MaxSenders`, or its
/// default.
fn max_senders() -> usize {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a DWORD read into a `u32` of the size passed.
    let read = unsafe {
        RegGetValueA(
            HKEY_CURRENT_USER,
            PCSTR(c"Software\\Leading Edge\\Spout".as_ptr().cast()),
            PCSTR(c"MaxSenders".as_ptr().cast()),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast::<c_void>()),
            Some(&raw mut size),
        )
    };
    if read == ERROR_SUCCESS && (1..=4096).contains(&value) {
        value as usize
    } else {
        MAX_SENDERS
    }
}

/// How many names `list` holds: Spout's setting, cut to the map's own size.
fn capacity(list: &Memory) -> usize {
    max_senders().min(list.len / SLOT)
}

/// The names `list` holds, read as Spout reads them.
fn read_names(list: &Memory, capacity: usize) -> BTreeSet<Vec<u8>> {
    let mut names = BTreeSet::new();
    for i in 0..capacity {
        let slot = list.read(i * SLOT, SLOT);
        if slot.first().is_none_or(|&b| b == 0) {
            break;
        }
        let end = slot.iter().position(|&b| b == 0).unwrap_or(slot.len());
        names.insert(slot[..end].to_vec());
    }
    names
}

/// `names` written into `list` as Spout writes them: in byte order, a NUL after the last.
fn write_names(list: &Memory, capacity: usize, names: &BTreeSet<Vec<u8>>) {
    for (i, name) in names.iter().take(capacity).enumerate() {
        let mut slot = [0u8; SLOT];
        let len = name.len().min(SLOT - 1);
        slot[..len].copy_from_slice(&name[..len]);
        list.write(i * SLOT, &slot);
    }
    if names.len() < capacity {
        list.write(names.len() * SLOT, &[0]);
    }
}

/// The list, made where no Spout app has made it yet.
fn list() -> Result<Memory, String> {
    Memory::create(LIST, (max_senders() * SLOT) as u32).map(|(list, _)| list)
}

/// Every name the list holds, in its order; empty where there is no list.
pub(super) fn names() -> Result<Vec<Vec<u8>>, String> {
    let Some(list) = Memory::open(LIST)? else {
        return Ok(Vec::new());
    };
    let _locked = list.lock().ok_or("Spout's list of senders stayed locked")?;
    Ok(read_names(&list, capacity(&list)).into_iter().collect())
}

/// What a sender's map says of its texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Info {
    /// The texture's legacy share handle, cut to 32 bits; zero for a sender sharing memory.
    pub handle: u32,
    pub width: u32,
    pub height: u32,
    /// The texture's `DXGI_FORMAT`; zero, 21 or 22 for a Direct3D 9 sender.
    pub format: u32,
}

impl Info {
    /// The 280 bytes Spout's `SharedTextureInfo` is.
    fn encode(self) -> [u8; INFO] {
        let mut out = [0u8; INFO];
        for (i, field) in [self.handle, self.width, self.height, self.format, 0]
            .into_iter()
            .enumerate()
        {
            out[i * 4..i * 4 + 4].copy_from_slice(&field.to_le_bytes());
        }
        out[20..276].copy_from_slice(executable());
        out
    }

    fn decode(bytes: &[u8]) -> Self {
        let field = |i: usize| {
            bytes
                .get(i * 4..i * 4 + 4)
                .and_then(|b| b.try_into().ok())
                .map_or(0, u32::from_le_bytes)
        };
        Self {
            handle: field(0),
            width: field(1),
            height: field(2),
            format: field(3),
        }
    }
}

/// This executable's full path, ANSI, NUL-padded to the 256 bytes a sender's map carries it in.
fn executable() -> &'static [u8; 256] {
    static PATH: OnceLock<[u8; 256]> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut path = [0u8; 256];
        let mut size = 255u32;
        // SAFETY: this process's own handle, and a buffer of `size` bytes plus a NUL.
        let read = unsafe {
            QueryFullProcessImageNameA(
                GetCurrentProcess(),
                PROCESS_NAME_WIN32,
                PSTR(path.as_mut_ptr()),
                &raw mut size,
            )
        };
        if read.is_err() {
            path = [0; 256];
        }
        path
    })
}

/// What the sender `name` says of its texture, or `None` where its map no longer opens: it has
/// gone. Opened, read under its lock and closed.
pub(super) fn info(name: &[u8]) -> Result<Option<Info>, String> {
    let Some(map) = Memory::open(name)? else {
        return Ok(None);
    };
    let _locked = map
        .lock()
        .ok_or_else(|| format!("the sender {} stayed locked", shown(name)))?;
    Ok(Some(Info::decode(&map.read(0, INFO))))
}

/// The active sender's map, held by this process once it has made a sender active, so the
/// name it wrote stays readable after that sender goes.
static ACTIVE_MAP: Mutex<Option<Memory>> = Mutex::new(None);

/// Make `name` the sender a receiver with no name of its own takes.
fn set_active(name: &[u8]) -> Result<(), String> {
    let mut held = ACTIVE_MAP.lock().unwrap_or_else(PoisonError::into_inner);
    if held.is_none() {
        *held = Some(Memory::create(ACTIVE, SLOT as u32)?.0);
    }
    let map = held.as_ref().expect("made above");
    let _locked = map.lock().ok_or("the active sender stayed locked")?;
    let len = name.len().min(SLOT - 1);
    map.write(0, &[&name[..len], &[0]].concat());
    Ok(())
}

/// The active sender's name, where there is one.
fn active() -> Option<Vec<u8>> {
    let map = Memory::open(ACTIVE).ok()??;
    let _locked = map.lock()?;
    let bytes = map.read(0, SLOT);
    let end = bytes.iter().position(|&b| b == 0)?;
    (end > 0).then(|| bytes[..end].to_vec())
}

/// A sender this process has listed: its map, kept open for as long as it lives, and its name
/// in the list. Dropping it takes both away.
pub(super) struct Entry {
    name: Vec<u8>,
    map: Option<Memory>,
}

impl Entry {
    /// The name it is listed under.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// Write `info` into its map.
    pub fn update(&self, info: Info) -> Result<(), String> {
        let map = self.map.as_ref().expect("held until drop");
        let _locked = map
            .lock()
            .ok_or_else(|| format!("the sender {} stayed locked", shown(&self.name)))?;
        map.write(0, &info.encode());
        Ok(())
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        let unlisted = list().and_then(|list| {
            let _locked = list.lock().ok_or("Spout's list of senders stayed locked")?;
            let was_active = active().as_deref() == Some(self.name.as_slice());
            self.map = None;
            let capacity = capacity(&list);
            let mut names = read_names(&list, capacity);
            names.remove(&self.name);
            write_names(&list, capacity, &names);
            if let Some(first) = names.first()
                && (was_active || names.len() == 1)
            {
                set_active(first)?;
            }
            Ok(())
        });
        if let Err(e) = unlisted {
            log::warn!("spout: {} left listed: {e}", shown(&self.name));
        }
    }
}

/// List a sender showing `info` under `wanted`, or under `wanted 2`, `wanted 3`… where a
/// sender of that name exists, and make it the active one. Its map is made and written before
/// its name is listed.
pub(super) fn announce(wanted: &[u8], info: Info) -> Result<Entry, String> {
    let list = list()?;
    let _locked = list.lock().ok_or("Spout's list of senders stayed locked")?;
    let capacity = capacity(&list);
    let mut names = read_names(&list, capacity);
    for n in 1..1000 {
        let name = if n == 1 {
            wanted.to_vec()
        } else {
            derived(wanted, &format!(" {n}"))
        };
        if names.contains(&name) {
            continue;
        }
        let (map, existed) = Memory::create(&name, INFO as u32)?;
        if existed {
            continue;
        }
        let entry = Entry {
            name: name.clone(),
            map: Some(map),
        };
        entry.update(info)?;
        if names.len() < capacity {
            names.insert(name.clone());
            write_names(&list, capacity, &names);
        } else {
            log::warn!(
                "spout: the list of senders is full at {capacity}; {} is not listed",
                shown(&name)
            );
        }
        if let Err(e) = set_active(&name) {
            log::warn!("spout: {} is not made active: {e}", shown(&name));
        }
        return Ok(entry);
    }
    Err(format!("no free Spout name after {}", shown(wanted)))
}

/// The access mutex and the frame count one sender and its receivers share, by the sender's
/// name.
pub(super) struct Access {
    mutex: Handle,
    count: Handle,
    /// The count a receiver read last.
    last: Cell<i32>,
}

impl Access {
    /// `<name>_SpoutAccessMutex` and `<name>_Count_Semaphore`, made where they do not exist.
    pub fn new(name: &[u8]) -> Result<Self, String> {
        let count = derived(name, "_Count_Semaphore");
        let c = c_name(&count)?;
        // SAFETY: a NUL-terminated name that outlives the call, no security attributes, and
        // Spout's initial count and maximum.
        let semaphore = unsafe { CreateSemaphoreA(None, 1, i32::MAX, PCSTR(c.as_ptr().cast())) }
            .map_err(|e| format!("the semaphore {}: {e}", shown(&count)))?;
        Ok(Self {
            mutex: mutex(&derived(name, "_SpoutAccessMutex"))?,
            count: Handle(semaphore),
            last: Cell::new(0),
        })
    }

    /// The texture's lock, waited for as Spout waits; `None` skips the frame.
    pub fn lock(&self) -> Option<Locked<'_>> {
        acquire(&self.mutex, WAIT_MS)
    }

    /// A sender's frame counted, inside the lock: one up, as Spout's `SetNewFrame` counts.
    pub fn sent(&self) {
        // SAFETY: a live semaphore; taking one and giving two back is one up.
        unsafe {
            let _ = WaitForSingleObject(self.count.0, 0);
            let _ = ReleaseSemaphore(self.count.0, 2, None);
        }
    }

    /// Whether the sender counts its frames, as of the last [`Self::fresh`].
    pub fn counts(&self) -> bool {
        self.last.get() != 0
    }

    /// Whether the sender has counted a frame since this receiver last looked, inside the
    /// lock: always, for a sender that does not count.
    pub fn fresh(&self) -> bool {
        // SAFETY: a live semaphore; one taken and given back leaves the count as it was.
        let taken = unsafe { WaitForSingleObject(self.count.0, 0) } == WAIT_OBJECT_0;
        if !taken {
            return true;
        }
        let mut count = 0i32;
        // SAFETY: as above, with the count before the release written back.
        if unsafe { ReleaseSemaphore(self.count.0, 1, Some(&raw mut count)) }.is_err() {
            return true;
        }
        if count == 0 {
            self.last.set(0);
            return true;
        }
        let fresh = count != self.last.get();
        self.last.set(count);
        fresh
    }
}

/// `name` made a Spout sender name: printable ASCII kept, a backslash — which no kernel object
/// name may hold — and anything else written `_`, and cut to [`LONGEST`] bytes.
pub(super) fn sanitized(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() || c == ' ') && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    out.truncate(LONGEST);
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// A listed name as text, one character a byte, so [`bytes_of`] gives the bytes back; ASCII
/// reads as itself.
pub(super) fn text_of(name: &[u8]) -> String {
    shown(name)
}

/// The bytes [`text_of`] made `text` from, or `None` for text it did not make.
pub(super) fn bytes_of(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sender's information is Spout's 280 bytes, little-endian, with zero for its usage and
    /// partner, and reads back as it was written.
    #[test]
    fn information_is_spouts_280_bytes() {
        let info = Info {
            handle: 0x8000_1234,
            width: 301,
            height: 167,
            format: 87,
        };
        let bytes = info.encode();
        assert_eq!(&bytes[0..4], &0x8000_1234u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &87u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &[0; 4]);
        assert_eq!(&bytes[276..280], &[0; 4]);
        assert_eq!(Info::decode(&bytes), info);
    }

    /// A name is printable ASCII, a backslash is not, and a name read from the list comes
    /// back to its bytes.
    #[test]
    fn names_are_ascii_and_survive_the_round_trip() {
        assert_eq!(sanitized("supersilvia - Mix"), "supersilvia - Mix");
        assert_eq!(sanitized("a\\b\u{e9}\tc"), "a_b__c");
        assert_eq!(sanitized(""), "_");
        assert_eq!(sanitized(&"x".repeat(400)).len(), LONGEST);
        let raw = [b'O', 0xe9, b'k'];
        assert_eq!(bytes_of(&text_of(&raw)).as_deref(), Some(&raw[..]));
        assert_eq!(bytes_of("\u{263a}"), None);
    }
}
