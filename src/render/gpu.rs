// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GPU as the wgpu renderer holds it: one instance, the adapter `render::adapter` picks,
//! one device and its one queue — and the **completion serial**, the one number that replaces
//! every GL fence.
//!
//! **A serial per submission.** Vulkan, Metal and Direct3D 12 know completion per submission,
//! not at an arbitrary point in the command stream as a GL fence does. So every submission made
//! through [`Gpu::submit`] is numbered, and `Queue::on_submitted_work_done` stores the number into one
//! `AtomicU64` once the GPU has finished it. "Has this finished?" is then a comparison against
//! [`Gpu::completed`], and a slot, a readback or a returned DMA-BUF carries the serial of the
//! submission that last touched it. The number is taken and the submission made under one
//! lock, so serials are handed out in the order the queue receives them, whichever thread
//! submits; the callback is registered under the same lock, so it covers this submission and
//! no earlier one. A callback runs on whatever thread next polls or submits — the editor's
//! `queue.submit` inside egui_wgpu included — and it only stores into the atomic.
//!
//! **One device and one queue**, shared by the synth, the editor and the picture windows: the
//! editor's latency behind a busy synth is bounded instead by the synth submitting one Output
//! at a time and keeping little queued ahead ([`super::queue`]). Everything that holds a
//! device holds it through a `Gpu`, and the synth asks for its own with [`Gpu::for_synth`],
//! so a synth on a device of its own (`proposals/wgpu.md`, Plan B) would change this file and
//! not its callers. See `proposals/wgpu.md`, sections 1.1, 1.2 and the one-queue measurement.
//!
//! **An error nothing captured is logged, and a lost device is reported, never a panic.**
//! wgpu's defaults panic on both, which ends the process with the unsaved work in it. Every
//! device [`Gpu::open`] makes is [`watch`]ed, and the app's lost-device callback, which saves
//! and says so, is `App::new`'s. There is no recovery: the device and everything made on it
//! would have to be rebuilt, which is what quitting does.

use crate::render::adapter;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Features the device takes where the adapter offers them, and goes without where it does
/// not: pass timestamps for the GPU timer, and the import a decoded frame comes in by.
pub const WANTED_FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    .union(IMPORT_FEATURES);

/// What the import of a decoded frame needs: a DMA-BUF on Vulkan, and on Direct3D 12 an NV12
/// texture, which a decoder's frame is (`render::dmabuf`).
const IMPORT_FEATURES: wgpu::Features = if cfg!(target_os = "windows") {
    wgpu::Features::TEXTURE_FORMAT_NV12
} else {
    wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
};

/// A handle on the one device. Cheap to clone: every clone is the same device, queue and
/// serial.
#[derive(Clone)]
pub struct Gpu {
    inner: Arc<Inner>,
}

struct Inner {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// What the adapter was chosen from, where this `Gpu` chose it.
    choice: Option<adapter::Choice>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The newest serial handed out, locked across the submit it numbers.
    submitted: Mutex<u64>,
    /// The newest serial the GPU is known to have finished.
    completed: Arc<AtomicU64>,
}

/// One submission: its serial, and the index wgpu waits on.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub serial: u64,
    index: wgpu::SubmissionIndex,
}

impl Gpu {
    /// A device on the adapter `render::adapter` picks with what `asked` asks, from an instance
    /// made with no display handle — which none of its backends needs, even to present to a
    /// window: the app's, made in `main` and handed to eframe, `--check`'s, and a bench's.
    pub fn headless(asked: &adapter::Asked) -> Result<Self, String> {
        let (instance, adapter, choice) = adapter::headless(asked)?;
        Self::open_choosing(instance, adapter, Some(choice))
    }

    /// The device eframe paints through — the one `main` handed it — where eframe paints with
    /// wgpu. `None` under a harness that hands the app none, which is egui_kittest.
    pub fn for_eframe(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        cc.wgpu_render_state.as_ref().map(|state| {
            Self::from_parts(
                state.instance.clone(),
                state.adapter.clone(),
                state.device.clone(),
                state.queue.clone(),
            )
        })
    }

    /// A device of its own on `adapter`, with the adapter's own limits and whichever of
    /// [`WANTED_FEATURES`] it offers.
    pub fn open(instance: wgpu::Instance, adapter: wgpu::Adapter) -> Result<Self, String> {
        Self::open_choosing(instance, adapter, None)
    }

    fn open_choosing(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        choice: Option<adapter::Choice>,
    ) -> Result<Self, String> {
        let desc = wgpu::DeviceDescriptor {
            label: Some("supersilvia"),
            required_features: adapter.features() & WANTED_FEATURES,
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };
        let (device, queue) =
            adapter::block_on(adapter.request_device(&desc)).map_err(|e| e.to_string())?;
        watch(&device, |why| log::error!("the GPU device was lost: {why}"));
        Ok(Self::assemble(instance, adapter, device, queue, choice))
    }

    /// A device someone else made: eframe's, handed over through `WgpuSetup::Existing`.
    pub fn from_parts(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Self {
        Self::assemble(instance, adapter, device, queue, None)
    }

    fn assemble(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        choice: Option<adapter::Choice>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                instance,
                adapter,
                choice,
                device,
                queue,
                submitted: Mutex::new(0),
                completed: Arc::new(AtomicU64::new(0)),
            }),
        }
    }

    /// The synth's: the same device and queue.
    #[must_use]
    pub fn for_synth(&self) -> Self {
        self.clone()
    }

    pub fn instance(&self) -> &wgpu::Instance {
        &self.inner.instance
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.inner.adapter
    }

    /// Every adapter the instance offered and which of them this device is on, where
    /// [`Gpu::headless`] chose it.
    pub fn choice(&self) -> Option<&adapter::Choice> {
        self.inner.choice.as_ref()
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.inner.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.inner.queue
    }

    /// Submit, numbered. The serial is [`Gpu::completed`] once the GPU has finished this
    /// submission and every one before it.
    pub fn submit(&self, commands: impl IntoIterator<Item = wgpu::CommandBuffer>) -> Ticket {
        let inner = &self.inner;
        let mut submitted = inner
            .submitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *submitted += 1;
        let serial = *submitted;
        let index = inner.queue.submit(commands);
        let completed = Arc::clone(&inner.completed);
        inner.queue.on_submitted_work_done(move || {
            completed.fetch_max(serial, Ordering::Release);
        });
        drop(submitted);
        Ticket { serial, index }
    }

    /// The newest serial the GPU has finished, as of the last poll or submit on any thread.
    pub fn completed(&self) -> u64 {
        self.inner.completed.load(Ordering::Acquire)
    }

    /// Whether the submission numbered `serial` has finished.
    pub fn is_done(&self, serial: u64) -> bool {
        serial <= self.completed()
    }

    /// Run whatever completion callbacks are due. Never waits.
    pub fn poll(&self) {
        // A poll only fails waiting, and this one does not wait.
        let _ = self.inner.device.poll(wgpu::PollType::Poll);
    }

    /// Wait for `ticket`'s submission to finish, at most `timeout`. Its serial is
    /// [`Gpu::completed`] when this returns `Ok`.
    ///
    /// Stored here as well as by the callback, since a callback registered while another
    /// client's submission raced ours fires only once that later one finishes too.
    pub fn wait(&self, ticket: &Ticket, timeout: Duration) -> Result<(), wgpu::PollError> {
        self.inner.device.poll(wgpu::PollType::Wait {
            submission_index: Some(ticket.index.clone()),
            timeout: Some(timeout),
        })?;
        self.inner
            .completed
            .fetch_max(ticket.serial, Ordering::Release);
        Ok(())
    }
}

/// Log every error on `device` that no error scope captured, rather than panic, and call
/// `lost` with wgpu's reason if the device is lost.
///
/// Setting either again replaces it: a test's device panics on a validation error after
/// this, and the app's lost-device callback is set over this one.
pub fn watch(device: &wgpu::Device, lost: impl Fn(String) + Send + 'static) {
    device.on_uncaptured_error(Arc::new(|e| log::error!("wgpu: {e}")));
    device.set_device_lost_callback(move |reason, message| {
        lost(format!("{reason:?}: {message}"));
    });
}
