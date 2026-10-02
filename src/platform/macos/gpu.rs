// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GPU's use on macOS: the whole GPU's busy share, from the IORegistry.
//!
//! A Mac keeps no per-process engine counter the way DRM fdinfo is one. What it has is the
//! `IOAccelerator` service's `PerformanceStatistics` dictionary, whose `Device Utilization %`
//! is the number Activity Monitor shows: the whole GPU's, every app's work together, this
//! one's with the rest. [`Clients::scan`] finds the service and [`Clients::read`] reads that
//! one key, as a share already, with nothing to difference.
//!
//! **`unsafe`**, allowed where `platform/macos/mod.rs` declares it, because IOKit's matching
//! and property calls are `unsafe fn` in their bindings, and the property dictionary comes back
//! untyped.

use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CFType, Type};
use objc2_io_kit::{
    IO_OBJECT_NULL, IOObjectRelease, IORegistryEntryCreateCFProperty, IOServiceGetMatchingService,
    IOServiceMatching, io_service_t,
};

use crate::platform::gpu::Read;

/// The IORegistry counts the GPU as a whole, so the figure is every app's.
pub const WHOLE_GPU: bool = true;

/// The GPU's `IOAccelerator` service, held until dropped: what [`Clients::read`] reads.
#[derive(Debug, Default)]
pub struct Clients {
    /// `IO_OBJECT_NULL` where none was found.
    service: io_service_t,
}

impl Clients {
    /// Find the GPU's `IOAccelerator` service. An Apple Silicon Mac has one.
    pub fn scan() -> Self {
        // SAFETY: the name is a NUL-terminated literal, alive for the whole call.
        let Some(matching) = (unsafe { IOServiceMatching(c"IOAccelerator".as_ptr()) }) else {
            return Self::default();
        };
        // A mutable dictionary is a dictionary: the call takes the plain type.
        let matching = CFDictionary::retain(&matching);
        // SAFETY: port 0 is `kIOMainPortDefault`, which IOKit documents as the null port, and
        // the matching dictionary is a live one of our own, whose reference the call consumes
        // as the binding's `CFRetained` argument hands it over.
        let service = unsafe { IOServiceGetMatchingService(0, Some(matching)) };
        Self { service }
    }

    /// The whole GPU's `Device Utilization %`, as a share.
    pub fn read(&self) -> Option<Read> {
        if self.service == IO_OBJECT_NULL {
            return None;
        }
        let key = CFString::from_static_str("PerformanceStatistics");
        // SAFETY: the service is a registry entry this value holds a reference on until it is
        // dropped, the key is a live `CFString`, and the allocator `None` is
        // `kCFAllocatorDefault`, which is itself null.
        let stats = unsafe { IORegistryEntryCreateCFProperty(self.service, Some(&key), None, 0) }?
            .downcast::<CFDictionary>()
            .ok()?;
        // SAFETY: every value a CF dictionary holds is a CF object, and the key type only
        // shapes the one key passed in below, a `CFString`; the dictionary is our own copy,
        // which nothing mutates.
        let stats = unsafe { stats.cast_unchecked::<CFString, CFType>() };
        let percent = stats
            .get(&CFString::from_static_str("Device Utilization %"))?
            .downcast::<CFNumber>()
            .ok()?
            .as_i64()?;
        Some(Read::WholeBusy(share(percent)))
    }
}

impl Drop for Clients {
    fn drop(&mut self) {
        if self.service != IO_OBJECT_NULL {
            IOObjectRelease(self.service);
        }
    }
}

/// A percentage as a share, `0..=1`, whatever the driver wrote.
fn share(percent: i64) -> f32 {
    percent.clamp(0, 100) as f32 / 100.0
}

/// Zero: a Mac counts no engine time per process.
pub fn render_engine_ns() -> u64 {
    0
}
